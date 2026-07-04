# Decouple Conversation Turn Acceptance From Assistant Completion

## Summary

Change `POST /workspace/api/conversations/{conversation_id}/turns` so it means “turn accepted and durably started,” not “assistant finished.” The server will synchronously validate and persist the user turn plus pending assistant turn, return that started snapshot immediately, then continue assistant execution in the background with SSE publishing progress and final/failed state.

This removes the need for timeline-level optimistic user messages in the frontend.

## Key Changes

- Backend conversation service:
  - Keep `WorkspaceConversationService::start_turn(...)` as the durable acceptance step.
  - Extract the post-start agent execution portion of `execute_turn_with_progress_payloads(...)` into a reusable completion method that accepts the prepared turn and started snapshot.
  - Preserve existing event emission: started user turn, pending assistant turn, streamed segment/turn updates, final snapshot.

- HTTP route:
  - Update `POST /turns` to:
    - parse request and compute `before_revision`;
    - call `start_turn`;
    - publish the started turn events after `before_revision`;
    - spawn background completion;
    - return the started snapshot immediately.
  - Keep validation, project mismatch, missing model, and active-assistant conflict errors synchronous.
  - Convert backend failures after acceptance into failed assistant turns published through SSE, not failed POST responses.

- Frontend:
  - Remove `optimisticSend` from conversation timeline construction.
  - On send, clear the draft and show only a short non-timeline sending/disabled state until the started snapshot or SSE update arrives.
  - Continue applying snapshots and live SSE events through the existing cache path.
  - Preserve stale-snapshot handling when SSE events arrive before the POST response.

- API contract:
  - `sendConversationTurnValidated(...)` still returns a `ConversationSnapshotResponse`, but callers must treat it as the started snapshot.
  - Tests and docs that expect POST to include completed assistant text must be updated.

## Test Plan

- Rust workspace/service tests:
  - `start_turn` still persists exactly one complete user turn and one pending assistant turn.
  - Extracted completion path finalizes the prepared assistant turn and preserves token usage, segments, raw-log behavior, errors, and Codex thread metadata.

- HTTP tests:
  - `POST /turns` returns quickly with user complete + assistant pending, before final backend output.
  - Validation/conflict errors still return synchronously.
  - Background success publishes assistant progress and final complete state over SSE.
  - Background backend failure publishes a failed assistant turn and final snapshot over SSE.
  - Existing live cursor behavior remains contiguous and stale snapshot handling still works.

- Frontend tests:
  - Sending a message renders the server-created user turn once.
  - No optimistic duplicate appears below the streaming assistant turn.
  - Composer disables/clears appropriately while the accepted turn starts.
  - SSE arriving before POST response does not regress cache state.

- Validation:
  - Run targeted Rust tests for `spark-workspace` and `spark-http`.
  - Run targeted frontend project-panel tests.
  - Run `cargo test --workspace --all-features`.
  - Run `npm run build` in `frontend`.

## Assumptions

- Streaming chat UI behavior is the primary contract; synchronous clients should no longer rely on POST returning the final assistant response.
- Only one active assistant turn per conversation remains allowed.
- No `client_message_id` is required for this change because the timeline optimistic entry is removed; request correlation can be added later if needed.
- Existing SSE infrastructure is the authority for assistant progress after turn acceptance.
