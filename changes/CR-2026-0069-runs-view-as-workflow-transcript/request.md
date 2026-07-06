# Runs View as Workflow Transcript

## Summary
Redesign the selected run detail view as one chat-like transcript with explicit workflow boundaries. Reuse the home chat rendering primitives, but feed them with a run-specific transcript projection instead of raw journal rows.

## Key Changes
- Refactor the home chat renderer into reusable conversation timeline primitives.
  - Keep markdown, message bubbles, thinking/plan blocks, tool-call cards, and request-user-input cards visually consistent with the Home tab.
  - Avoid coupling the shared renderer to project conversation cache, conversation IDs, or conversation segment transport.
  - Add a run-only boundary entry type for node execution metadata: node name, attempt/retry, status, timing, model, and child-flow source when relevant.

- Replace the current large summary/progress/journal stack with:
  - A compact run header for flow name, status, current node, elapsed time, token/cost summary, and actions.
  - One primary transcript surface for the run.
  - Advanced details for graph, checkpoint, context, artifacts, and result only.

- Add a run transcript projection derived from the run journal.
  - Build transcript entries in execution order.
  - Use node boundaries to differentiate the transcript.
  - Coalesce `LLMContent` / `CodergenAdapter` stream content into one visible message per node/channel/source item.
  - Prefer final completed content over accumulated deltas when both exist.
  - Map human gates into the shared request-user-input rendering flow.

- Define visible transcript rules.
  - Show assistant/final output with the same markdown renderer used in Home.
  - Show plan/reasoning only when it is already intended for user display.
  - Show failures as readable boundary states plus meaningful evaluator/failure output.
  - Never render raw `CodergenAdapter` rows, raw journal rows, adapter deltas, token-update fragments, checkpoint noise, or raw JSON payloads anywhere in normal or advanced run details.

- Keep live behavior bounded.
  - Keep latest-page hydration and “load older” pagination.
  - Include the bounded SSE replay fix so node transitions cannot replay full run history.
  - Live stream updates should update the active transcript message or boundary, not append visible rows for each delta.

## Tests
- Shared renderer tests:
  - Home chat still renders messages, thinking, plans, tool calls, and request-user-input cards correctly.
  - Runs can render the same shared message/tool/input primitives plus run node boundaries.

- Run transcript projection tests:
  - Multiple nodes become one ordered transcript with clear boundaries.
  - Retries create distinct attempts.
  - Stream deltas coalesce into a single visible message.
  - Completed content replaces accumulated streaming text.
  - Raw adapter/journal events are filtered out of visible transcript output.

- UI tests:
  - Selected run shows compact header plus transcript as the primary surface.
  - LLM output renders markdown like Home chat.
  - Human-gate questions remain answerable inline.
  - No raw journal rows or `CodergenAdapter` deltas render anywhere in normal or advanced run details.
  - Loading older history preserves transcript grouping.

- Validation gate:
  - `cargo fmt --all -- --check`
  - `cargo test --workspace --all-features`
  - `npm --prefix frontend run test:unit`
  - `npm --prefix frontend run build`

## Assumptions
- The run list/sidebar remains.
- The graph/checkpoint/context/artifacts/result panels remain available as advanced details.
- No backend schema migration in this pass; transcript projection is derived from existing persisted journal data.
