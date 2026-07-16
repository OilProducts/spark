# Normalize The Claude Code Event Stream To The Canonical Adapter Contract

## Summary
`TurnStreamEvent` is the normalization boundary between backend adapters and everything downstream (live sink, durable replay, segment materializer, finalizer, frontend). The codex adapter satisfies that contract; the claude_code adapter still emits under-specified events, and consumers carry claude-shaped compensations as a result. Every transcript bug fixed by CR-2026-0082 (segment collapse, plan-mode narration swallowing) was a symptom of this, and CR-2026-0082 itself added two more compensations: a claude-only segment-id match arm and a hard-coded `commentary` phase with the true final answer synthesized post-hoc by the finalizer. Finish the job from first principles: make the claude_code adapter emit fully-normalized events — turn identity, honest phases, a real final-answer event, incremental deltas — then delete the consumer-side compensations that only existed because it didn't.

Remaining conformance gaps (codex vs claude_code):
- `app_turn_id`: codex backfills it on every event and returns it on `AgentTurnOutput`; claude_code sets `None` everywhere.
- `phase`: codex emits `commentary`/`final_answer` from its protocol; claude_code hard-codes `commentary` on everything, so the final answer never exists as a stream event and `finalize_agent_turn_output` synthesizes it from `final_assistant_text`.
- Streaming: codex emits incremental `ContentDelta`; claude_code emits only whole-block `ContentCompleted`, so live chat shows nothing for a text block until it is complete.

## Key Changes
- Stamp `app_turn_id` on every claude_code event and on `AgentTurnOutput` (`crates/spark-agent-adapter/src/claude_code.rs`).
  - The CLI has no turn concept; synthesize one identifier per turn invocation (uuid) and backfill it on every emitted event, mirroring the codex adapter's backfill.
  - With this in place, claude_code events reach the `(Some(app_turn_id), Some(item_id))` segment-id arms and `apply_assistant_turn_app_server_ids` records turn threading like codex.

- Emit the final answer as a real stream event (`crates/spark-agent-adapter/src/claude_code.rs`).
  - When the `result` message arrives, emit a `ContentCompleted` on the Assistant channel with `phase: "final_answer"` and content `result.result`, **reusing the last text block's `item_id`** so the segment upsert promotes that segment in place (phase and content update; no duplicate bubble).
  - If the turn produced no text blocks, emit the event with the next counter `item_id`. If `result.result` is empty, emit nothing — the finalizer's missing-final-answer failure check covers it.
  - The adapter owns the knowledge of which text was final; it must express it in the canonical event language instead of leaving `final_assistant_text` as a side channel for the finalizer to interpret.

- Stream incremental content (`crates/spark-agent-adapter/src/claude_code.rs`).
  - Run the CLI with `--include-partial-messages` and translate its `stream_event` deltas into `ContentDelta` events on the Assistant/Reasoning channels, carrying the same `item_id` as the block's eventual `ContentCompleted` (materializer semantics: deltas append, completion sets the full text — the completion is authoritative).
  - Keep per-block `ContentCompleted` emission unchanged. Live chat then streams claude_code text as it is generated, including plan-mode narration, matching codex behavior.

- Tighten the consumers once no producer emits anonymous content (`crates/spark-workspace/src/conversations.rs`, `crates/spark-common/src/segments.rs`).
  - Flip `is_final_answer_phase(None)` to `false`: phase-less content is no longer treated as a final answer anywhere (plan-mode buffering, chat-mode turn-content updates, `finalized_assistant_segment`).
  - Delete the finalize synthesis path: `fallback_assistant_segment_id` and the `complete_existing_assistant_segment_with_text` / synthesized-event fallback in `finalize_agent_turn_output`. Keep the missing-final-answer failure check.
  - Delete the `(None, Some(item_id))` arm added by CR-2026-0082 in `assistant_segment_id` — dead once claude_code stamps `app_turn_id`. The bare `segment-assistant-{turn_id}` fallback remains as the last resort.
  - Producer conformance and consumer tightening land together in this one change; the ordering constraint (producer first) only matters if the work is split.

## Test Plan
- Adapter contract tests (`crates/spark-agent-adapter/tests/process_contracts/claude_code_contracts.rs`):
  - Every emitted event carries the same synthesized `app_turn_id`, and `AgentTurnOutput.app_turn_id` matches it.
  - The final text block's segment identity is reused by a trailing `final_answer` `ContentCompleted` whose content equals `result.result`.
  - With the fake CLI emitting `stream_event` partial messages: `ContentDelta` events precede each block's `ContentCompleted` and share its `item_id`.
  - A result-only turn (no text blocks) emits a `final_answer` event with a fresh `item_id`; an empty result emits none.
- Fake CLI (`crates/spark-agent-adapter/src/bin/spark-agent-fake-claude-code.rs`): extend the script to emit `stream_event` partial-message lines for at least one text and one thinking block.
- Workspace contract tests (`crates/spark-workspace`):
  - Chat and plan mode: narration streams as `commentary` segments in the live path; the final answer arrives by in-place promotion of the last text segment, ordered after the tool segments; no synthesized final-answer event is created by the finalizer.
  - A claude_code turn whose result is empty fails the turn via the missing-final-answer check.
  - Codex behavior unchanged (its phases and item identities flow exactly as before).
- Segment unit tests (`crates/spark-common/src/segments.rs`): `ContentDelta` events append to the segment keyed by `(app_turn_id, item_id)` and the following `ContentCompleted` replaces content in full; a later `final_answer` `ContentCompleted` with the same `item_id` updates phase and content in place without creating a new segment.
- Full validation:
  - `cargo fmt --all -- --check`
  - `cargo test --workspace --all-features`
  - `npm --prefix frontend run test:unit`
  - `npm --prefix frontend run build`

## Assumptions
- Single CR: producer conformance and consumer tightening are implemented and validated together; no intermediate state ships where consumers are tightened before the adapter conforms.
- The installed Claude Code CLI supports `--include-partial-messages` stream-json output; the fake CLI is the contract fixture for the exact `stream_event` shape consumed.
- No migration of persisted conversations: turns materialized under the old event shapes stay as stored; only newly ingested turns get the normalized identities and phases.
- Flipping `is_final_answer_phase(None)` is safe because after this change no backend emits phase-less assistant content: codex sets phases from its protocol, claude_code sets `commentary`/`final_answer` explicitly, and the run-node projection (`llm_backend.rs`) only re-wraps adapter events.
- Segment ids for new claude_code turns change shape (they now include the synthesized `app_turn_id`); segment ids are opaque to consumers, so this is not a compatibility concern.
