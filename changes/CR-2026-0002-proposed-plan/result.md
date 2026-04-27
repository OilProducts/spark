---
id: CR-2026-0002-proposed-plan
title: Restore `raw-log.jsonl` as Pure Protocol Transcript
status: completed
type: bugfix
changelog: internal
---

## Summary

Delivered the requested continuity-reset fix. `raw-log.jsonl` now stays a pure app-server protocol transcript, while failed persisted-thread resume diagnostics move into durable normalized conversation state as structured workflow events.

The shipped implementation also preserves continuity-reset details on failed assistant turns and segments via `error_code`, clears the stale persisted `thread_id` from `session.json`, and starts a fresh durable thread only on the next explicit user message after the reset. Snapshot parsing on the frontend was updated so the new workflow event fields survive round-tripping.

## Validation

- `uv run pytest -q`
- Result: `938 passed in 16.63s`

## Shipped Changes

- Backend chat/session flow in `src/spark/chat/` now raises and persists a structured continuity-reset failure instead of writing synthetic debug entries into `raw-log.jsonl`.
- Conversation models and repository handling in `src/spark/workspace/conversations/` now support structured `WorkflowEvent` fields (`kind`, `error_code`, `details`) and explicit session thread clearing.
- Codex app client resume handling in `src/spark_common/codex_app_client.py` now preserves structured resume-failure details instead of collapsing them to `None`.
- Frontend snapshot parsing in `frontend/src/lib/api/` now preserves structured workflow event fields for conversation snapshots.
- Regression coverage was updated in `tests/api/test_project_chat.py` and `tests/spark_common/test_codex_app_client.py`, including assertions that `raw-log.jsonl` contains only protocol traffic and that continuity-reset diagnostics are durable in snapshot state.
