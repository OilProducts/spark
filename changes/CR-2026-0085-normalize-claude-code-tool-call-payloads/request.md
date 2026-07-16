# Normalize Claude Code Tool Call Payloads

## Summary
Claude Code tool calls render in the transcript with the generic title "Tool call" and, worse, a detail line taken from the first line of the tool's *output* (e.g. "===", "no changes added to commit…"). Two causes, both adapter-shape problems. First, the started payload never maps the canonical fields: `ToolCallStarted` carries `{id, name, input}`, but `normalize_tool_call_payload` derives `command` only from a top-level `command` key (codex's `commandExecution` shape) — claude_code's command sits at `input.command` and is never lifted. Second, completion erases what exists: the materializer replaces the segment's whole `tool_call` object with the completed payload, which for claude_code is only `{id, output, is_error}` — codex tolerates the replace because its completion events re-carry the full item; claude_code's don't, so `name`/`input` vanish and the renormalized title degrades to "Tool call". The frontend's `summarizeToolCallDetail` then falls through `command` (absent) → `filePaths` (empty) → first output line.

Fix in the adapter, per the normalization principle (CR-2026-0083): emit complete, canonical tool_call payloads on both the started and completed events. No consumer changes — the replace semantics stay as codex expects.

## Key Changes
- Emit canonical fields on `ToolCallStarted` (`crates/spark-agent-adapter/src/claude_code.rs`).
  - `Bash`: `kind: "command_execution"`, `command` from `input.command`, `title` from `input.description` when present (Claude Code Bash calls carry a short human-written description), else the first line of the command, else the tool name.
  - File tools (`Read`, `Write`, `Edit`, `NotebookEdit`): `file_paths` from `input.file_path` / `input.notebook_path`; title = tool name.
  - Search tools (`Glob`, `Grep`): title = tool name; include `input.path` in `file_paths` when present.
  - All other tools (including MCP tools): `kind: "dynamic_tool"`, `title` = tool name.
  - Emitting `title` directly is honored by `normalize_tool_call_payload` (it early-returns when `title` is present) — no shared-code change needed.
- Re-carry the canonical fields on completion.
  - Track started tool calls by `toolu_*` id in the turn state; `ToolCallCompleted`/`ToolCallFailed` payloads merge the remembered `kind`/`title`/`command`/`file_paths` with `output`/`is_error`.
  - A tool_result with no remembered started call (defensive) keeps today's minimal payload.
- Explicitly out of scope: `materialize_segment_for_event`'s replace semantics and `normalize_tool_call_payload` stay unchanged (codex depends on them); no frontend changes (`summarizeToolCallDetail`'s fallback order is correct once `command`/`file_paths` are populated); no migration of previously persisted segments.

## Test Plan
- Adapter contract tests (`crates/spark-agent-adapter/tests/process_contracts/claude_code_contracts.rs`):
  - A Bash tool_use with `description` yields a started event with `kind: command_execution`, the command, and the description as title — and a completed event that re-carries all three plus `output`/`is_error`.
  - A Bash tool_use without `description` titles from the command's first line.
  - A `Read` tool_use yields `file_paths: [input.file_path]` on started and completed events.
  - An unknown tool name yields `kind: dynamic_tool`, `title` = tool name, preserved through completion.
- Fake CLI (`crates/spark-agent-adapter/src/bin/spark-agent-fake-claude-code.rs`): extend the script so its tool_use blocks carry `description` and a file-tool example, with matching tool_results.
- Workspace contract test (`crates/spark-workspace`): after a claude_code turn completes, the persisted tool_call segment retains `command` and the descriptive title (not "Tool call"), with output present.
- Full validation:
  - `cargo fmt --all -- --check`
  - `cargo test --workspace --all-features`
  - `npm --prefix frontend run test:unit`
  - `npm --prefix frontend run build`

## Assumptions
- Codex behavior is untouched: its payloads already carry canonical fields on both events, and the consumer replace semantics are unchanged.
- `input.description` is optional on Bash tool calls; the fallback chain (description → command first line → name) covers its absence.
- Previously persisted segments keep their generic labels; only newly ingested turns get the canonical payloads.
