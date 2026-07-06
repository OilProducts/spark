# Align Run Transcript Persistence With Canonical Render State

## Summary
Fix the run transcript work by making the transcript a run-local durable render state, not a second journal projection. `events.jsonl` remains the low-volume operational journal. Raw provider/session/Codex data remains debug-only. The Runs tab renders the canonical run transcript plus journal-derived operational status where needed, without reconstructing transcript rows from raw journal payloads.

## Key Changes
- Define the run transcript contract in `specs/attractor-spec.md`.
  - Add `GET /pipelines/{id}/transcript` as the durable render-state endpoint for selected-run transcript inspection.
  - State that the transcript uses Spark conversation turn/segment semantics adapted to run scope.
  - State that reload/history uses coalesced transcript state, not delta replay or raw adapter/session events.
  - State that `/journal` is operational history only: lifecycle, stage, checkpoint, human-gate, child-run, cancellation, retry, LLM request summary, usage, and similar low-volume events.

- Replace the custom `RunTranscriptEntry` model with a run-scoped reuse of the existing renderable conversation segment shape where possible.
  - Persist assistant, reasoning, and plan content as upserted segments keyed by stable upstream item identity when available.
  - Persist final/completed content as the retained render value; deltas may update live state but must not be the reload source.
  - Persist renderable tool-call segments using the same fields the existing chat renderer expects.
  - Persist request-user-input entries in the same renderable shape as Home chat input requests.
  - Persist context compaction and notice-style output as renderable segments, not raw backend events.

- Represent workflow structure as explicit transcript boundaries.
  - Add run/node/stage boundary records to the transcript materialized state so multi-node flows read as one continuous transcript with clear workflow separators.
  - Boundary identity must include source scope, parent node id, child flow name, node id, stage index, and attempt.
  - Boundary upserts must preserve original start time and only set end time/status from terminal events.
  - Boundaries are transcript structure, not a duplicate implementation of frontend journal projection.

- Rework live and completed Runs tab behavior around the canonical transcript.
  - Initial selected-run hydration loads `/pipelines/{id}/transcript` for renderable transcript content and `/journal` for operational history/cursors.
  - Live LLM content updates should flow as transcript/segment upserts or an equivalent normalized render update, not only as `run.journal_entry`.
  - The `Load older` control must not imply transcript paging unless transcript paging exists. For this pass, either load the complete transcript once or rename/scope the older button to operational journal history only.
  - Remove normal UI rendering paths that project assistant/tool/plan/reasoning transcript rows from `CodergenAdapter`, raw `turn_stream_event`, provider deltas, or journal payload internals.

- Keep debug and context concerns separate.
  - Codex JSON-RPC continues to use `codex-jsonrpc-trace.jsonl` only when debug tracing is enabled.
  - Unified LLM/agent raw streams use an equivalent debug-only trace sidecar if needed.
  - LLM generation context is not the same as render transcript. Codex resume may own Codex context; unified LLM backend context must be persisted separately when implemented.

## Test Plan
- Rust runtime tests:
  - New Codex and unified LLM runs persist coalesced transcript render state without storing deltas/raw adapter wrappers in `events.jsonl`.
  - Transcript segment upserts preserve stable identity and final content replaces accumulated streaming text.
  - Tool calls, request-user-input, reasoning, plan, assistant text, notices, and boundaries persist in frontend-renderable shape.
  - Boundary records preserve `started_at`, set terminal `ended_at`, and do not collapse across source scope, child flow, stage index, or attempt.

- API tests:
  - `/pipelines/{id}/journal` returns only operational journal entries.
  - `/pipelines/{id}/transcript` returns canonical renderable transcript state.
  - Completed run reload does not require parsing raw adapter/session payloads.

- Frontend tests:
  - Runs tab renders transcript content from `/transcript`.
  - Live transcript updates appear during active runs after journal deltas are removed.
  - Normal and advanced run views do not render raw adapter names, `turn_stream_event`, JSON-RPC method names, or delta event names.
  - The older-history control behavior matches its actual data source.

- Full validation:
  - `cargo fmt --all -- --check`
  - `cargo test --workspace --all-features`
  - `npm --prefix frontend run test:unit`
  - `npm --prefix frontend run build`

## Assumptions
- No migration is required for old runs that only have high-volume journal transcript data.
- Existing Home chat segment semantics are the source of truth for renderable LLM output shape.
- `events.jsonl` is not the transcript and not the debug trace.
- Debug trace files are never parsed by normal Runs tab rendering.
