# TODO

## Known issues

- [ ] **Turn-failure journaling contracts fail on main.**
  `conversation_turn_route_persists_structured_backend_error_output` (flaky) and
  `conversation_turn_route_persists_thread_resume_failure_details` (usually fails) in
  `crates/spark-http/tests/workspace_conversation_turn_route_contracts.rs`.
  The failed-turn path marks the assistant turn/segments failed in the snapshot
  (snapshot assertions pass), but the journal read through `read_events_after`
  intermittently lacks the failed `segment_upsert` and the slim
  `conversation_snapshot_ref` entries. The commit path in
  `crates/spark-workspace/src/conversations.rs` needs to journal the mutations it
  already applies to the snapshot. Pre-existing on `origin/main` (693faacd);
  unrelated to the 2026-07 rewrite reland or the macOS path fixes.

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

- [ ] **Run event logs are always parsed eagerly and in full; live paths pay
  O(log size) per cycle and memory high-water grows unbounded with run
  length.** `RunBundle` (`crates/attractor-runtime/src/store.rs`)
  materializes `raw_events` (every event JSON-parsed into
  `RawRuntimeEvent`) and a derived `journal` on every
  `read_run_bundle`/`list_child_run_bundles` call, so every consumer pays a
  full parse of `events.jsonl` even when it needs only the record or a
  cursor delta. Measured on a 46 MB / 36k-event log (loam MS-002 child,
  2026-07-19): ~0.8–4.4 s per combined build, 1.23 TB cumulative page-cache
  reads and an 88%-pinned core before the first round of fixes; after them
  (tail-read appends, one build per publish cycle, record-only reads —
  branch `spark/live-run-perf-fixes`) the steady-state cost is one full
  combined build per publisher cycle per notifying run, plus one per SSE
  connect, plus one per paged-journal fetch, plus one per manager
  observation cycle (`resolve_child_result` in `manager_loop.rs` parses the
  entire child log to report `event_count`/`latest_event_at`). Allocator
  high-water from the transient parse trees reached 8.5 GB (2026-07-19),
  ~10 GB, then 26.6 GB RSS (2026-07-21, overnight MS-004 run) on the
  desktop process — bounded per run but growing with every longer run.
  Remaining fix, two independent layers:
  1. *Lazy bundles.* Split `RunBundle` so `record` (and `checkpoint`) load
     without touching `events.jsonl` — either `raw_events:
     OnceCell<Vec<RawRuntimeEvent>>` populated on first access, or a
     separate `RunBundleMeta` type for the record-only callers that remain
     (`list_run_bundles` for the runs list, `next_child_invocation_index`,
     `update_run_record`, trigger/milestone/upsert paths already converted).
     `list_child_run_bundles` should stop reading full child logs when the
     caller (e.g. `child_result_from_bundle`) needs only counts — offer a
     metadata-level child summary that stats the file and tail-reads the
     last event instead.
  2. *Incremental journal tailing.* The publisher
     (`run_event_publisher_loop` in `crates/spark-http/src/lib.rs`) and the
     SSE replay re-derive the combined journal from zero every time. Keep a
     per-run in-memory cache keyed by run id holding the parsed journal
     plus each source file's byte offset; on notify, seek to the stored
     offset, parse only appended lines, extend the cache, and re-emit from
     the cursor. Invalidation: file truncation or a shrunk offset (crash
     rewrite) falls back to a full rebuild. The combined journal's
     re-sequencing (`resequence_combined_journal` in
     `crates/attractor-runtime/src/journals.rs`) stays stable under this
     because merge order is `(emitted_at, source rank, sequence, id)` and
     appends sort after existing entries — assert that invariant in tests
     rather than assuming it, since a child event emitted with a clock
     earlier than already-merged parent entries would re-number history and
     must instead trigger a resync envelope.
  Acceptance: publisher cycles and SSE connects for a run with a 50 MB
  event log parse only bytes appended since the last cycle (steady-state
  cost proportional to delta, not log size); the runs list and manager
  observation loops read no event bytes at all; desktop RSS stays flat
  across a multi-hour streaming run; existing journal/SSE/pagination
  contract tests pass unchanged, plus a new contract covering the
  out-of-order-append resync path.

- [ ] **Playwright smoke specs** predate the runs-tab overhaul; re-record the runs
  smoke flows against the Activity stream / inspector layout.
- [ ] **Manual desktop verification** of blocking human gates and live chat/run
  streaming (`just dev-desktop`).
- [ ] **Parent-run usage rollup**: run token usage/cost is per-record; child-run
  usage does not aggregate into the parent (matches old Python behavior). Roll
  up over the combined journal if parent totals should include children.
- [ ] **Human gate wait timeout**: gates now block in production; add a
  `wait_timeout_seconds` node attribute so unattended runs can time out instead
  of waiting forever.

## Done

- [x] Build an executable harness for the existing agent-driven acceptance workflow assets under `tests/acceptance/agent-workflows`.
- [x] Add first-class structured UI authoring for subgraphs and scoped `node[...]` / `edge[...]` defaults so these no longer require raw DOT editing.
- [x] Audit Spark against the original Attractor spec/API to identify any remaining runtime/editor contract drift versus intentional product-layer extensions after the known drift candidates above are handled.
