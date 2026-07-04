# Restore `<proposed_plan>` Plan Card Compatibility

## Summary
Add Rust workspace support for textual `<proposed_plan>...</proposed_plan>` blocks in plan-mode final assistant text. This preserves compatibility with Python behavior and with non-Codex providers such as LiteLLM that may return plan markup as plain text instead of structured Codex Plan events.

## Key Changes
- In `crates/spark-workspace/src/conversations.rs`, add helpers equivalent to Python’s proposed-plan parsing:
  - detect case-insensitive `<proposed_plan>...</proposed_plan>` blocks
  - normalize extracted plan text
  - remove the plan block from the regular assistant final-answer remainder
  - avoid leaking literal tags into conversation segments

- In assistant turn finalization:
  - if `chat_mode == "plan"` and no completed plan segment exists, extract the first `<proposed_plan>` block from `final_assistant_text`
  - materialize that extracted content as a completed `plan` segment
  - call existing `persist_proposed_plan_artifact(...)` so the frontend receives `segment.kind = "plan"` plus a `proposed_plans` sidecar
  - if text remains outside the plan block, persist it as the normal assistant final-answer segment
  - if the response is only the plan block, make the assistant turn content resolve to the plan content and do not create an extra assistant message segment

- Keep the structured Codex Plan event path unchanged:
  - if a completed `plan` segment already exists from `TurnStreamChannel::Plan`, prefer that segment
  - do not duplicate plan artifacts or plan cards

## Test Plan
- Add Rust workspace tests mirroring the Python cases:
  - plan-mode final text containing only `<proposed_plan>` creates one `plan` segment and one pending proposed-plan artifact, with no tag leakage
  - plan-mode final text with prose before/after the block creates a `plan` segment plus one assistant-message remainder segment
  - existing structured plan-channel events still create one plan card/artifact and are not duplicated by final text
  - non-plan chat mode treats `<proposed_plan>` as ordinary assistant text

- Run focused validation:
  - `cargo test -p spark-workspace --test review_artifact_contracts`
  - `cargo test -p spark-workspace --test conversation_event_normalization_contracts`
  - `npm --prefix frontend run test:unit` if frontend behavior is touched indirectly by fixture changes

## Assumptions
- Textual `<proposed_plan>` parsing should apply only in `chat_mode == "plan"`.
- Only the first proposed-plan block should become the plan artifact; surrounding text remains assistant prose.
- The existing frontend green card behavior is correct and should continue to be driven by `segment.kind = "plan"` and `proposed_plans`.
- The Codex structured Plan path is already handled separately through `collaborationMode` and Plan-channel events.
