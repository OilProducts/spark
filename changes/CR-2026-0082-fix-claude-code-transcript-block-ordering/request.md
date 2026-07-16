# Fix Claude Code Transcript Block Ordering

## Summary
Turns from the Claude Code backend that interleave assistant text with tool calls (text → tool → text → tool → final text) lose narration and render out of order. Two distinct mechanisms, both rooted in the adapter emitting anonymous events:

1. **Chat mode — segment collapse.** The claude_code adapter never sets `source.item_id` on assistant text/thinking events, so `assistant_segment_id()` falls back to the shared `segment-assistant-{turn_id}`: each `ContentCompleted` replaces the previous block's content while keeping the segment's original `order`. The final answer displays at the first text block's position — before the tool calls — and intermediate narration is overwritten.
2. **Plan mode — narration swallowed entirely.** The adapter also never sets `phase`, and `is_final_answer_phase(None)` is true, so in plan mode `conversations.rs` routes *every* assistant text event into the single `buffered_plan_assistant_event` slot (each overwriting the last) instead of materializing it. No text segment exists until `finalize_agent_turn_output` creates one from `final_assistant_text` after all tool segments. Narration is dropped without trace, and nothing about assistant text reaches the UI until turn completion.

Fix by giving every text/thinking block a distinct, stable identity and an explicit non-final phase, so blocks materialize as their own segments in stream order in both modes.

## Key Changes
- Give each assistant content block a unique identity in the claude_code adapter (`crates/spark-agent-adapter/src/claude_code.rs`).
  - Add a per-turn block counter to the turn ingestor.
  - In `ingest_assistant`, set `event.source.item_id` (e.g. `block-{n}`) on every `text` and `thinking` block event, incrementing per block.
  - Use a counter rather than the API `message.id`: the fake CLI emits no message id, the real CLI may split one API message across multiple `assistant` events, and this is a single-pass pipeline with no re-ingest, so a monotonic counter is unique under both shapes.
  - Tool events are unchanged — they already key on the `toolu_*` id.

- Mark streamed text/thinking events with an explicit non-final phase in the claude_code adapter (e.g. `commentary`, the codex convention).
  - Rationale: `is_final_answer_phase(None)` is true, so phase-less text is treated as a final answer everywhere it matters. In plan mode (`conversations.rs` event loop) that buffers every text block into `buffered_plan_assistant_event` — narration never materializes and each buffered event overwrites the last.
  - With a non-final phase, narration bypasses the plan buffer and materializes as ordinary segments in both chat and plan mode. The true final answer segment continues to be created by `finalize_agent_turn_output` from `output.final_assistant_text`, which lands after the tool segments — the correct position.
  - Interplay to preserve: `finalized_assistant_segment()` treats a completed phase-less `assistant_message` as the final answer. Narration segments must carry the non-final phase so finalize still creates the real final-answer segment and the missing-final-answer failure check stays accurate.
  - Known behavior change: the chat-mode mid-stream `turn.content` update (`conversations.rs` event loop, final-answer-phase branch) stops firing for narration blocks; `turn.content` is set at finalize instead. No user-visible loss — the claude_code backend currently delivers all events in one batch at turn completion anyway.

- Accept `item_id` without `app_turn_id` in segment identity (`crates/spark-common/src/segments.rs`).
  - Add a match arm to `assistant_segment_id`: `(None, Some(item_id))` → `segment-assistant-{turn_id}-{item_id}`.
  - `reasoning_segment_id` needs no change — it already incorporates `item_id`; it collapses today only because the adapter never sets one.
  - `plan_segment_id` is out of scope (the claude_code backend never emits the Plan channel).
  - No other backend is affected: codex backfills `app_turn_id` on every event (`codex_app_server.rs`), so the new arm only fires for claude_code events after this change.

- Move the `Worked for …` separator to the final assistant message (`frontend/src/features/projects/model/conversationTimeline.ts`).
  - Today the separator inserts before the first `assistant_message`/`plan` segment that follows tool activity. Once interim narration survives as separate segments, that would place "Worked for Ns" above a mid-turn preamble instead of the final answer.
  - Compute the index of the last `assistant_message`/`plan` segment in the turn up front and insert the separator there when prior tool activity exists.

- Extend the fake CLI fixture (`crates/spark-agent-adapter/src/bin/spark-agent-fake-claude-code.rs`) so the default script exercises the interleaved shape: narration text, tool_use/tool_result, narration text, tool_use/tool_result, final text — enough that the pre-fix behavior (overwritten narration, misordered tool calls) would be caught by the contract tests.

## Test Plan
- Rust unit tests (`crates/spark-common/src/segments.rs`):
  - Feeding text → tool_call started/completed → text events for one turn yields three distinct segments with ascending `order`.
  - The first text segment's content is preserved after the second text event materializes (regression for the overwrite).
  - Distinct thinking blocks in one turn yield distinct reasoning segments.
- Contract tests (`crates/spark-agent-adapter/tests/process_contracts/claude_code_contracts.rs`):
  - Assistant text and thinking events carry distinct `source.item_id` values in emission order and a non-final `phase`.
  - End-to-end over the fake CLI: the materialized turn contains narration, tool call, and final answer segments in stream order.
- Workspace tests (`crates/spark-workspace`):
  - Plan mode: an interleaved claude_code turn materializes narration segments (they are not swallowed by the plan buffer), and the final answer segment is created at finalize, ordered after the tool segments.
  - Chat mode: `finalize_agent_turn_output` still creates the final answer segment from `final_assistant_text` when narration segments (non-final phase) exist, and still fails the turn when no final answer exists at all.
  - Codex plan-mode behavior is unchanged (its `final_answer`-phased events still buffer; its `commentary` events still materialize).
- Frontend tests (`frontend/src/features/projects/model/__tests__/conversationTimeline.test.ts`):
  - An interleaved fixture (narration → tool → narration → tool → answer) renders timeline entries in that order.
  - The `Worked for …` separator appears immediately before the final assistant message, not before interim narration.
  - Existing single-message turns keep their current separator behavior.
- Full validation:
  - `cargo fmt --all -- --check`
  - `cargo test --workspace --all-features`
  - `npm --prefix frontend run test:unit`
  - `npm --prefix frontend run build`

## Assumptions
- No migration for existing conversations: turns already materialized with a collapsed assistant segment stay as stored; only newly ingested turns get per-block segments.
- The claude_code adapter has no streaming-delta path (it emits only `ContentCompleted` per block), so no `ContentDelta` identity handling is needed for this backend.
- Segment `order` remains the sole transcript ordering key; no timestamp-based ordering is introduced.
- Placing the worked-duration separator before the last assistant message is the desired UX for interleaved turns.
