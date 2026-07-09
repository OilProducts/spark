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

## Deferred follow-ups

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
