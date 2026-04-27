---
id: CR-2026-0001-proposed-plan
title: Preserve Chat Continuity on Resume Failure
status: completed
type: bugfix
changelog: internal
---

## Summary
Spark now treats failed persisted-thread resumes as continuity resets instead of silently starting a replacement durable thread during the same turn. The shipped change preserves resume failure details from the app server, records a failed assistant turn with a stable continuity-reset code, keeps the submitted user turn in history, clears the stale persisted thread id, and leaves prompt-template transcript replay unsupported with clearer error text.

## Validation
- `uv run pytest -q` (`937 passed`)

## Shipped Changes
- `src/spark_common/codex_app_client.py` now returns structured `thread/resume` results that preserve app-server error codes and messages and distinguish missing thread ids from explicit resume failures.
- `src/spark/chat/session.py` now raises `PersistedThreadContinuityResetError` when a persisted backend thread cannot be resumed, and only starts a new durable thread when no persisted thread id exists.
- `src/spark/chat/service.py`, `src/spark/workspace/conversations/models.py`, and `src/spark/workspace/conversations/repository.py` now persist continuity-reset failures with `error_code`, keep the new user turn, clear the stored stale thread id, and append raw debug records without issuing `thread/start` on the failed turn.
- `src/spark/chat/prompt_templates.py` keeps `{{recent_conversation}}` unsupported and explains that Spark does not replay prior transcript text into prompts.
- `tests/api/test_project_chat.py` and `tests/spark_common/test_codex_app_client.py` cover resume failure propagation, continuity-reset persistence, no-fallback-thread behavior, next-message fresh-thread recovery, raw-log/debug output, and the prompt-template contract.
