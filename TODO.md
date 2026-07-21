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

- [ ] **Run event logs are always parsed eagerly and in full; live paths pay
  O(log size) per cycle and memory high-water grows unbounded with run
  length.**

  *The problem, present tense.* There is no way to read part of a run's
  event log: `RunBundle` (`crates/attractor-runtime/src/store.rs`) always
  JSON-parses every line of `events.jsonl` into `raw_events` and derives
  the full `journal`, on every `read_run_bundle`/`list_child_run_bundles`
  call. So each of these triggers pays a cost proportional to the whole
  log, however small the piece it actually needs:
  - each publisher cycle (`run_event_publisher_loop`, ~120 ms coalesced
    while a run streams) — needs only the entries appended since the last
    cycle;
  - each live-stream (SSE) connect and each paged-journal fetch — needs
    the window after/around a cursor;
  - each manager observation of a child (`resolve_child_result` in
    `manager_loop.rs`, every poll cycle) — needs only a count and a
    timestamp;
  - each human-gate answer poll (`journaled_gate_answer` in
    `handlers.rs`, every 250 ms for as long as a gate blocks — and gates
    block until answered, by design) — needs only "did an answer event
    appear since the last poll".
  On a 46 MB / 36k-event log each such build costs ~0.8–4.4 s and a large
  transient allocation; the allocator keeps the high-water mark, so
  desktop RSS grows with the longest run seen (26.6 GB after an overnight
  streaming run, 2026-07-21). Runs only get longer; per-trigger cost and
  memory scale with them.

  *What the first round already fixed* (branch
  `spark/live-run-perf-fixes`, for context — do not re-fix): the same full
  parse used to be paid per *event append* (quadratic — a streaming agent
  paid it tens of times per second; 1.23 TB cumulative reads and a pinned
  core measured 2026-07-19), four times per publish cycle instead of once,
  and on record-only paths (run upsert, milestones, terminal triggers).
  Those fixes collapsed the *number* of full builds; the *cost of each*
  build is untouched and is what remains below.

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
  2. *Incremental journal tailing.* Consumer inventory (2026-07-21): no
     recurring consumer needs more than "entries appended since an offset"
     plus projections maintainable from them — publisher, gate poll,
     pending-questions, and cursor replay all fit; the manager needs less
     (stat + tail-read); full/raw history is needed only by cold one-shots
     (transcript view, deep page-back, raw-events routes, artifact
     capture-kind scan, end-of-run summarizer), where a from-disk rebuild
     is acceptable. So the cache must NOT retain each run's full parsed
     journal (that would institutionalize the RSS high-water): per active
     run keep byte offsets per source file, a bounded recent-entry ring,
     and small incremental projection state (segments, pending questions,
     latest sequence), evicted when the run goes terminal; on notify, seek
     to the stored offset and parse only appended lines; cold deep-history
     requests rebuild from disk explicitly. Invalidation: file truncation
     or a shrunk offset (crash rewrite) falls back to a full rebuild. A
     sidecar index or SQLite would buy durable multi-process cursors the
     single-server-process design does not need. The combined journal's
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

- [x] Build an executable harness for the existing agent-driven acceptance workflow assets under `tests/acceptance/agent-workflows`.
- [x] Add first-class structured UI authoring for subgraphs and scoped `node[...]` / `edge[...]` defaults so these no longer require raw DOT editing.
- [x] Audit Spark against the original Attractor spec/API to identify any remaining runtime/editor contract drift versus intentional product-layer extensions after the known drift candidates above are handled.
