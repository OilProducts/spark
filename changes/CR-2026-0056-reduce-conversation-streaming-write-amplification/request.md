# Reduce Conversation Streaming Write Amplification

## Summary

Change the Python Codex app-server chat path so text deltas are still available to live clients but are no longer persisted to disk on every tiny stream event. Persist assistant/reasoning/plan text only when the segment completes, and gate raw RPC logging behind an explicit environment variable.

## Key Changes

- In `src/spark/chat/service.py`, update `persist_live_event` so `content_delta` events for `assistant`, `reasoning`, and `plan` do not call `_write_state()` or append `segment_upsert` events.
- Keep final persistence on:
  - `content_completed` for assistant/reasoning/plan;
  - `token_usage_updated`;
  - tool start/update/completion events;
  - request-user-input events;
  - context compaction events;
  - failure/final assistant turn persistence.
- Preserve real-time UI streaming by still forwarding incoming deltas through the live/progress callback path, but do not append those delta-derived payloads to `events.jsonl`.
- Gate raw RPC logging:
  - add `SPARK_ENABLE_RAW_RPC_LOG=1` as the opt-in switch;
  - when unset, do not bind `set_raw_rpc_logger` in the Python chat execution path;
  - leave raw RPC append behavior unchanged once enabled.
- Keep completed segment persistence semantically identical: completed assistant, plan, and reasoning segments should still appear in `state.json` and `events.jsonl` after completion.

## Test Plan

- Add/update Python tests for Codex app-server chat streaming:
  - multiple `content_delta` events do not write repeated `segment_upsert` events;
  - `content_completed` writes exactly one completed segment payload;
  - live progress callback still receives streamed deltas;
  - final assistant turn content and token usage are persisted.
- Add raw RPC logging tests:
  - default behavior does not create/append `raw-log.jsonl`;
  - `SPARK_ENABLE_RAW_RPC_LOG=1` preserves current append-only raw log behavior.
- Run targeted tests first with:
  - `uv run pytest -q -x --maxfail=1 tests/api/test_project_chat.py`
  - any affected workspace conversation tests.
- Before completion, run:
  - `uv run pytest -q`

## Assumptions

- Mid-turn reconnect replay does not need every text delta persisted; completed segment state is sufficient.
- Raw RPC logs are diagnostic artifacts and should be opt-in by default.
- This is a mitigation for the current Python app-server path; the future Rust app-server implementation should adopt the same persistence policy from the start.
