## Restore `raw-log.jsonl` as Pure Protocol Transcript

### Summary
Revert the continuity-reset diagnostic write out of `raw-log.jsonl` and move that diagnostic into normalized durable conversation state.

After this change:
- `raw-log.jsonl` contains only exact app-server traffic captured by the raw RPC logger.
- continuity-reset details remain fully durable and queryable through conversation state.
- failed resume behavior stays as implemented: no silent fallback thread, visible failed turn, preserved resume details.

### Implementation Changes
- Remove synthetic debug-log writes from the continuity-reset path in `src/spark/chat/service.py`.
  - Delete `_append_raw_debug_log()` or stop using it for conversation diagnostics.
  - Keep the raw RPC logger path untouched so only actual app-server ingress/egress lines reach `raw-log.jsonl`.
- Extend `WorkflowEvent` in `src/spark/workspace/conversations/models.py` beyond plain message-only text.
  - Add optional structured fields for normalized event typing and details.
  - Minimum fields for this case: `kind`, `error_code`, `details`.
  - `details` should carry `persisted_thread_id` and structured `resume_failure` (`kind`, `code`, `message`).
- In the continuity-reset handler in `src/spark/chat/service.py`, append a normalized `WorkflowEvent` instead of writing a fake raw-log entry.
  - Event kind should be specific, e.g. `continuity_reset`.
  - Preserve the same detail payload currently emitted into the debug raw-log record.
- Keep existing assistant turn/segment failure persistence unchanged.
  - The assistant turn should still fail with `CONTINUITY_RESET_ERROR_CODE`.
  - `session.json` should still clear the stale `thread_id`.
- Keep prompt-template behavior unchanged.
  - `recent_conversation` remains unsupported.
  - The prompt-template error text added in this change can stay.

### Test Plan
- Update the continuity-reset tests in `tests/api/test_project_chat.py`.
  - Stop expecting a `"direction":"debug"` entry in `raw-log.jsonl`.
  - Assert `raw-log.jsonl` contains only the outgoing `thread/resume` request and incoming protocol error response.
  - Assert no synthetic non-protocol entries are present.
- Add assertions that the continuity reset is now present in durable normalized state.
  - Read the conversation snapshot/state and assert an `event_log` entry with kind `continuity_reset`.
  - Assert the event includes `error_code`, `persisted_thread_id`, and structured `resume_failure`.
- Add or update model serialization tests if needed.
  - Verify `WorkflowEvent` round-trips the new structured fields.
- Run the full suite with `uv run pytest -q`.

### Public/API Effects
- `raw-log.jsonl` regains its strict meaning as exact app-server protocol traffic only.
- Conversation snapshots/state will now expose continuity-reset diagnostics through normalized `event_log`.
- No change to the supported prompt variables or resume-failure user-facing turn error.

### Assumptions
- `event_log` is the preferred normalized durable home for conversation-level diagnostics.
- We do not need a new `debug-log.jsonl` file for this scope.
- Structured `WorkflowEvent` expansion is acceptable as the smallest durable fix for this regression.
