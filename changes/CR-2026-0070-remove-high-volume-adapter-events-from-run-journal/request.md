# Remove High-Volume Adapter Events From Run Journal

## Summary
Stop writing streaming LLM deltas and raw adapter/session events to `events.jsonl`. Keep `events.jsonl` as low-volume durable run history only. Live UI may still receive deltas while a run is active, but reload/history must come from coalesced transcript state, not replayed deltas. Existing old runs may lose legacy transcript reconstruction; no migration.

## Key Changes
- Replace blanket `CodergenAdapter` journal persistence with explicit routing:
  - `events.jsonl`: lifecycle, stage, checkpoint, human-gate, child-run, cancellation, retry, and normal runtime summary events only.
  - debug trace/log sidecars: raw JSON-RPC, raw provider/session events, reasoning deltas, text deltas, tool-output deltas, and other high-volume diagnostics.
  - run transcript store: coalesced renderable entries for assistant text, reasoning, plan, tool calls, request-user-input, context compaction, and node boundaries.

- Remove `CodergenAdapter` as the generic persistence bridge:
  - Do not append each `CodergenEvent` from `codergen_events_for_journal` to `events.jsonl`.
  - Introduce domain runtime events only where the run journal needs durable operational history, for example node LLM request started/completed/failed and token usage summary.
  - Keep those events low-volume and independent of backend implementation names like Codex, unified LLM, or Codergen.

- Add run-local transcript persistence:
  - Use the existing conversation turn/segment semantics as the durable render shape, stored under the run root rather than project Home conversations.
  - Materialize streaming deltas into upserted segments while live.
  - Persist final/coalesced segment content; do not persist the delta sequence as the reload source.
  - Add explicit run/node boundary entries so a multi-node flow still reads as one transcript with clear workflow structure.

- Update run APIs/live behavior:
  - Existing journal endpoints continue serving operational history from `events.jsonl`.
  - Add or switch selected-run transcript loading to the run transcript store.
  - Live SSE may send transient delta/upsert events for active rendering, but completed hydration uses transcript state.
  - The Runs tab no longer projects transcript rows from raw journal `CodergenAdapter` payloads.

- Debug logging:
  - Reuse `codex-jsonrpc-trace.jsonl` for Codex JSON-RPC when debug tracing is enabled.
  - Add an equivalent debug-only raw/session trace path for unified LLM/agent streams if one is not already present.
  - Debug traces are not consumed by normal UI rendering.

## Tests
- Rust runtime tests:
  - New codex/unified LLM runs do not write `ContentDelta`, reasoning deltas, tool-output deltas, raw session events, or `CodergenAdapter` wrappers to `events.jsonl`.
  - Journal still records lifecycle/stage/checkpoint/human-gate/child-run events and low-volume LLM summary events.
  - Debug mode writes raw traces to sidecar files only.

- Transcript tests:
  - Deltas stream into a run transcript segment and coalesce into final content.
  - Completed content replaces accumulated streaming text.
  - Tool calls, plans, reasoning, request-user-input, context compaction, and node boundaries persist as renderable transcript entries.
  - Reloading a completed run does not require parsing raw adapter payloads.

- Frontend tests:
  - Runs tab loads transcript from transcript state, not journal `CodergenAdapter` rows.
  - No raw journal payloads, adapter names, `turn_stream_event`, or delta event names render in normal or advanced run details.
  - Old runs with only legacy `CodergenAdapter` transcript data may show reduced transcript output without migration.

- Validation gate:
  - `cargo fmt --all -- --check`
  - `cargo test --workspace --all-features`
  - `npm --prefix frontend run test:unit`
  - `npm --prefix frontend run build`

## Assumptions
- No migration for existing high-volume journals; legacy transcript reconstruction can be removed or allowed to degrade.
- `events.jsonl` remains the durable operational journal, not the render transcript and not the debug stream.
- Backend-specific details may appear in debug traces and transcript source metadata, but not as first-class journal event types.
