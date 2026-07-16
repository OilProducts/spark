# Fix Claude Code Delta/Completion Segment Correlation

## Summary
Since CR-2026-0083, live transcripts duplicate every claude_code text output. The adapter correlates partial-stream deltas to completed blocks through `partial_block_ids`, keyed by the block's index in the API message content array — and clears that map after every `assistant` event. The real CLI emits **one `assistant` event per content block** (single-element content arrays), so the completed block's enumerate index is always 0 and the map is cleared between blocks of the same message: only the first block of each message correlates, and every later block mints a fresh item id. The delta-accumulated segment and the completed segment then coexist with identical content — every text block renders twice in the live view. `last_text_item_id` also points at the miscorrelated completed id, so final-answer promotion targets the duplicate and the delta-built copy stays `commentary`. The durable path is unaffected (persisted transcripts are clean; a reload clears the duplicates). The contract tests passed because the fake CLI emits multi-block single `assistant` events — exactly the shape the code wrongly assumes — instead of the real CLI's per-block events.

Fix the correlation so a block's deltas and its completion always resolve to one identity, under both event shapes, and make the fake CLI emit the real shape so the fixture cannot drift on this property again.

## Key Changes
- Correlate by channel and arrival order, not array index (`crates/spark-agent-adapter/src/claude_code.rs`).
  - Track pending partial-stream block ids as per-channel FIFO queues (Assistant text, Reasoning thinking): a `content_block_delta` creates-or-reuses the pending id for its stream index as today, but records it in the queue for its channel.
  - When `ingest_assistant` completes a text or thinking block, consume the oldest pending id for the matching channel; mint a new id only when no pending id exists. This is correct for the real per-block events, for hypothetical multi-block events (in-order consumption), and degrades to CR-2026-0082 behavior when no partial stream is present (older CLI, `--include-partial-messages` absent).
  - Stop clearing `partial_block_ids` per `assistant` event. Reset the stream-index map on `message_start` (the stream index space is per-message); pending queues are consumed by completions, not cleared.
  - `last_text_item_id` must end up equal to the id the block's deltas used, so result promotion promotes the delta-built segment in place.
- Make the fake CLI emit the real event shape (`crates/spark-agent-adapter/src/bin/spark-agent-fake-claude-code.rs`).
  - One `assistant` event per content block with a single-element content array, with `stream_event` partial deltas preceding each block, for both thinking and text — matching observed real CLI output.
  - Keep one scenario with a multi-block `assistant` event to pin the FIFO consumption path.

## Test Plan
- Adapter contract tests (`crates/spark-agent-adapter/tests/process_contracts/claude_code_contracts.rs`):
  - Over the per-block fake CLI script: each block's `ContentDelta` events and its `ContentCompleted` carry the same `item_id`; the set of item ids equals the set of blocks (no orphan or extra ids).
  - The trailing `final_answer` event reuses the same id as the last text block's deltas.
  - Multi-block scenario: two blocks in one `assistant` event consume pending ids in order.
  - No-partials scenario (script without `stream_event` lines): completions mint ids and behave as before CR-2026-0083's delta work.
- Workspace contract tests (`crates/spark-workspace`): a streamed claude_code turn materializes exactly one assistant segment per text block in the live path (count assertion on the working view), with the final text promoted in place — no duplicate-content segments.
- Full validation:
  - `cargo fmt --all -- --check`
  - `cargo test --workspace --all-features`
  - `npm --prefix frontend run test:unit`
  - `npm --prefix frontend run build`

## Assumptions
- The real CLI emits one `assistant` event per content block; the correlation must nevertheless tolerate multi-block events, and the FIFO consumption rule covers both.
- Tool-use blocks are out of scope: they keep their `toolu_*` identity and never enter the pending queues (their `input_json_delta` stream events remain ignored).
- No migration: durable snapshots were never duplicated; live views showing duplicates self-heal on reload.
