# Runs View: Child-Run Legibility, Scope, and Live Usage

## Summary

Child (subflow) runs are second-class citizens in the Runs view. Five defects, all confirmed against the code: child runs vanish from the "Active project" scope because their record's `project_path` is the worktree; the nested sidebar cards crush their own text to two characters; the Run Graph header toolbar overflows its card; graph nodes without an authored label all render the literal string "Task Node"; and the run header's Tokens / Est. cost stay "—" for the entire duration of a long agent node because usage only rolls up on node completion. Fix all five.

## Root causes (current state, main branch)

1. **Scope**: `crates/attractor-runtime/src/manager_loop.rs:415` sets the child `RunRecord.project_path = working_directory` (the isolated worktree, e.g. `.spark/checkouts/<run-id>`). The runs list endpoint (`attractor-api/src/lib.rs::list_run_records`) filters on `project_path`, so children never match the active project and only appear under "All projects".
2. **Sidebar cards**: `frontend/src/features/runs/components/RunList.tsx` `renderRunRow` gives the right-hand chip group (`Child` + status) `shrink-0` while the text column is `min-w-0 flex-1`. In the 256px (`w-64`) panel, minus the `ml-4 pl-2` nesting indent and card padding, the chips starve the text column to ~40px — title and each meta item truncate to almost nothing.
3. **Graph toolbar**: `RunGraphCard.tsx` header puts "Open in Editor", "Refresh", and `ChildFlowExpansionToggle` ("Parent Only" / "Expanded Child Flow", `size="sm"` `px-3`) in a non-wrapping `flex items-center gap-2` container, so at narrower card widths the controls overflow past the card edge. The canvas zoom/fullscreen controls can also sit clipped at the canvas bottom edge.
4. **Node labels**: `frontend/src/features/workflow-canvas/TaskNode.tsx:124` falls back to the literal `'Task Node'` when a node has no `label`. The worker flows (`workers/implement-task.yaml`, `workers/resolve-merge-conflicts.yaml`) author no node labels, so every node in a child-run graph reads "Task Node". Relatedly, `manager_loop.rs:246` derives the child run's `flow_name` from the YAML file name, so the run header shows "implement-task.yaml" instead of the flow's `title` ("Implement Task").
5. **Usage rollup**: `attractor-runtime/src/executor.rs::refresh_record_usage` recomputes the record's token usage from the journal only after a node completes (executor.rs:722) and at run end. `usage.rs::project_run_usage` counts only request-completed events. A run like implement-task is one long `agent_task` node, so Tokens/Est. cost stay "—" until the run is essentially over — even though the journal is receiving `codex_app_server_session_event` entries whose `token_usage` payload carries cumulative turn totals (`/total/totalTokens`) the whole time.

## Requirements

### 1. Child runs join the parent's project scope

- When spawning a child run, set `RunRecord.project_path` to the **parent run record's** `project_path`; `working_directory` keeps the child worktree path.
- Verify nothing derives the child's on-disk run location, workdir, or context from `record.project_path` before relying on it (child workdir flows through `internal.run_workdir` / the spawn request today; run storage uses the parent's `internal.runs_dir`).
- Acceptance: with the parent project active, a running child appears nested under its parent in the "Active project" scope (the frontend nesting already keys off `parent_run_id` being present in the filtered list), and remains visible after completion. Pre-existing child records keep their old `project_path`; no migration.

### 2. Legible child cards in the run sidebar

- Rework `renderRunRow` in `RunList.tsx` so the flow-name column keeps usable width at the default `w-64` panel for nesting depth 1 and 2: drop the separate `Child` text chip (nesting indent + guide border already convey it) and keep only the status chip, which may move onto its own line if needed.
- The meta line (duration · short run id · root id/project when shown) renders as one wrapping line, not one truncated fragment per line.
- Acceptance: at 256px panel width, depth-1 child card shows at least the first 12 characters of the flow name plus a readable duration and short run id.

### 3. Run Graph controls stay inside the card

- The Run Graph header controls must never overflow the card horizontally: allow the controls container to wrap (`flex-wrap`, `min-w-0`) and compact the expansion toggle (`size="xs"`, labels "Parent only" / "Child flows"). `ChildFlowExpansionToggle` is shared with the editor canvas — keep the editor rendering acceptable (compact labels are fine there too, or accept a size prop).
- The canvas zoom/fullscreen controls render fully inside the canvas viewport at the default pane height.

### 4. Real node and flow names

- `TaskNode.tsx`: when `data.label` is empty, fall back to the node id, never the literal "Task Node".
- Author `label:` values for every node in `workers/implement-task.yaml` (e.g. Start / Implement / Done) and `workers/resolve-merge-conflicts.yaml`.
- Child run `flow_name`: use the parsed child flow's `title` when non-empty, falling back to the file name (`manager_loop.rs` already parses the child source before spawning). The run header and sidebar then show "Implement Task" instead of "implement-task.yaml".

### 5. Live token and cost rollup during long nodes

- Extend `project_run_usage` (usage.rs) to count in-flight usage: for `CodergenAdapter` journal entries of type `rust_agent_session_event`, `codex_app_server_session_event`, or `claude_code_session_event` whose payload carries `token_usage`, track the **latest** snapshot per node id. A node's session snapshots count toward the rollup only until a request-completed event for that node id follows them in the journal; completed events remain authoritative and must never be double-counted with session snapshots of the same request. Final totals for completed runs must be byte-identical to today's (completed-events-only) sums.
- Map the codex cumulative snapshot shape (`total.totalTokens` etc.) into the existing `TokenUsageBucket` fields; snapshots that don't parse are skipped.
- Fold the refreshed usage into the persisted run record when the executor journals a usage-bearing codergen event, throttled to at most one record write per ~5 seconds per run, using the existing merge-don't-clobber pattern from executor.rs:723 (the record doubles as the cancel/pause control channel).
- Est. cost continues to derive from the breakdown via the existing `estimate_model_cost`.
- Acceptance: while an implement-task child executes its codex turn, the child run's header Tokens and Est. cost populate within seconds of the first `token_count` session event and increase monotonically; a claude-code node (usage arrives only in its final result event) still populates at node end as today.

## Non-goals

- Rolling child-run usage up into the **parent** run's totals (separate concern; parent telemetry ingestion exists but its usage semantics are not changed here).
- Artifact preview scroll/clipping polish and the "N runs · M running" summary-vs-group-count consistency.
- Migrating `project_path` on historical child run records.

## Test Plan

- Rust: unit tests for `project_run_usage` in-flight semantics — session snapshots count latest-wins per node, stop counting once the node's completed event lands, never double-count, and completed-run totals are unchanged against existing fixtures; codex snapshot shape maps correctly. Manager-loop test asserting the child record inherits the parent `project_path`, keeps the worktree `working_directory`, and uses the flow title as `flow_name`.
- Rust: runs-list contract asserting a child run appears when filtering by the parent's project path.
- Frontend unit tests: `TaskNode` renders the node id when no label is authored; `RunList` child rows omit the Child chip and keep the status chip; expansion toggle renders the compact labels.
- Manual/visual: Runs view at default sidebar width with a nested running child — legible cards, no toolbar overflow, live token counter moving during the child's agent turn.
- Run the full repository validation gate: `just test` (Rust formatting check, workspace tests with all features, frontend unit tests, frontend build).

## Assumptions

- Codex `token_count` session payloads are cumulative per turn (confirmed: the adapter records `last_token_total` from `/total/totalTokens`), so latest-wins per in-flight node is exact, not additive.
- One in-flight codergen request per node id at a time; parallel branches are distinct node ids, so per-node grouping remains correct.
- Dropping the "Child" chip is acceptable since nesting is conveyed structurally; if a textual marker is still desired, it belongs in the meta line, not a width-reserving chip.
- The test gate has no known baseline flakes (the formerly load-sensitive webhook dispatch contract was rewritten in 978530bf); treat every gate failure as real.
