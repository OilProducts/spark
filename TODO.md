# TODO

## Known issues

- [ ] **Human steer endpoint records interventions but never delivers them.**
  `POST /attractor/pipelines/{id}/steer` (`steer_pipeline_route` in
  `crates/attractor-api/src/lib.rs`) resolves the correct target — an explicit
  `target_run_id`/`target_node_id`, else the active child from
  `context.stack.child.run_id` and its `active_stage` via `intervention_target`
  — then unconditionally appends a `HumanInterventionRequested` event with
  `status: "rejected", delivery_mode: "none", reason: "no_active_child_run"`
  and returns it. No delivery is ever attempted, regardless of run state.
  The delivery machinery exists but is wired only to the *automatic* path:
  the manager loop's `request_child_intervention`
  (`crates/attractor-runtime/src/manager_loop.rs`) composes a steer message
  from child failure context and delivers it through the
  `codergen_intervention_broker`
  (`spark_agent_adapter::CodergenSessionInterventionBroker`, wired in
  `crates/attractor-runtime/src/handlers.rs`), which injects text into the
  live codex app-server session for the target node's in-flight agent turn.
  That path only fires when the child has a failure reason, is rate-limited
  by `AUTO_STEER_ATTEMPT_LIMIT` and `steer_cooldown`, and takes no human
  input.
  Field evidence (2026-07-20, loam build): a running milestone agent
  repeatedly gamed its render benchmark; with no working human-steer channel
  the correction had to be delivered by writing an `AGENTS.md` into the
  project repo, which only takes effect when the *next* agent session starts
  — mid-turn correction was impossible.
  Proposed fix: route the human steer request through the same broker the
  automatic path uses. `steer_pipeline_route` should (1) resolve the target
  as today, (2) look up the live session for `(target_run_id,
  target_node_id)` via the broker, (3) deliver the message and journal
  `HumanInterventionRequested` with the real `status`
  (`delivered`/`rejected`) and `delivery_mode` (`session_injection`), and
  (4) fall back to a durable queued mode — persist the message under the
  run root and have the agent-task handler drain pending interventions at
  its next turn boundary — so a steer sent between turns is not lost.
  Frontend already maps `HumanInterventionRequested` journal entries to
  `run.question_pending` envelopes (`crates/spark-workspace/src/live.rs`),
  so delivered steers surface in the timeline without UI work; a small
  compose box on the run detail view is the only missing surface.
  Acceptance: steering a run whose child agent is mid-turn lands the
  message in that turn's session and journals `delivered`; steering between
  turns queues and drains at the next turn; steering a run with no active
  child still journals `rejected` with the current reason; the automatic
  manager path's behavior and rate limits are unchanged.

- [ ] **Playwright smoke specs** predate the runs-tab overhaul; re-record the runs
  smoke flows against the Activity stream / inspector layout. Confirmed stale
  2026-07-21: `runs-observability.spec.ts` asserts `run-advanced-panel` and
  `run-advanced-toggle-button`, which no longer exist anywhere in
  `frontend/src`.
- [ ] **Manual desktop verification of blocking human gates**
  (`just dev-desktop`). Narrowed 2026-07-21: live chat streaming and live run
  streaming were verified extensively in production desktop use during the
  2026-07-19..21 loam build (conversation nimble-stream, multi-hour milestone
  runs); blocking `human_gate` nodes were never exercised on the desktop in
  that period and still need a manual pass.
- [ ] **Parent-run usage rollup**: run token usage/cost is per-record; child-run
  usage does not aggregate into the parent (matches old Python behavior). Roll
  up over the combined journal if parent totals should include children.

## Done

- [x] **Run event logs: metadata-only reads and incremental journal tailing**
  (implemented 2026-07-21, branch `spark/live-run-perf-fixes`, commits
  `a3357ec`/`9eb385d`/`f045773`). RunMeta reads records/checkpoints without
  touching events.jsonl (runs list, control routes, record updates); the
  manager observes children from metadata plus a tail-read; the human-gate
  wait loop scans only appended bytes per poll; and the publisher/cursor
  replay are served by a bounded per-run incremental cache (byte offsets +
  recent-entry ring + incremental segment projection) that contract tests
  hold byte-equal to the cold rebuild, including the out-of-order-append
  rebuild path. Cold paths (transcript, deep page-back, raw events)
  intentionally still rebuild from disk. Remaining verification: confirm
  desktop RSS stays flat across a multi-hour streaming run after the next
  rebuild+restart.

- [x] Build an executable harness for the existing agent-driven acceptance workflow assets under `tests/acceptance/agent-workflows`.
- [x] Add first-class structured UI authoring for subgraphs and scoped `node[...]` / `edge[...]` defaults so these no longer require raw DOT editing.
- [x] Audit Spark against the original Attractor spec/API to identify any remaining runtime/editor contract drift versus intentional product-layer extensions after the known drift candidates above are handled.
