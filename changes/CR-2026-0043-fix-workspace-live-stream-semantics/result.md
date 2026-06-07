---
id: CR-2026-0043-fix-workspace-live-stream-semantics
title: Fix Workspace Live Stream Semantics
status: completed
type: bugfix
changelog: internal
---

## Summary
Implemented the workspace live stream semantics fix so the multiplexed EventSource uses independent resource scopes and stable stream identity. The backend now parses resource-specific scope parameters, keeps legacy `project_path` as a constrained compatibility alias, requires `conversation_project_path` for conversation streams, and avoids applying conversation scope to runs overview or selected-run delivery. Conversation and selected-run catch-up now subscribe before replaying durable state, then dedupe live events already emitted from replay.

The app shell now owns a single workspace live stream for Home conversation events, runs overview, selected runs, and triggers. Cursor values are tracked in refs and used only when opening or reconnecting the stream; advancing conversation revisions or run sequences no longer changes the React URL identity.

## Validation
- `uv run pytest -q` passed: 2000 passed, 26 skipped, 2 warnings.
- `npm run build` passed in `frontend`.
- `npm run test:unit -- src/app/__tests__/AppShell.test.tsx src/features/runs/__tests__/RunsPanel.test.tsx src/features/projects/__tests__/ProjectsPanel.test.tsx` passed: 3 files, 80 tests.

## Shipped Changes
- Updated `/workspace/api/live/events` in `src/spark/workspace/api.py` for resource-specific scopes, cursor validation, subscribe-before-replay ordering, replay gap handling, and duplicate suppression.
- Added conversation repository support in `src/spark/workspace/conversations/repository.py` for replaying journal-backed conversation revisions.
- Added `WorkspaceLiveEventsController` and supporting frontend live-events utilities under `frontend/src/app/AppSessionControllers.tsx` and `frontend/src/features/workspace/`.
- Updated runs, conversation, and trigger hooks to consume the central workspace live events instead of creating separate stream URLs with advancing cursors.
- Expanded backend and frontend coverage for independent scopes, all-project runs overview behavior, stable EventSource URLs, reconnect cursor use, and replay/live duplicate handling.
- Updated API/client contracts, documentation, and smoke coverage to reflect the workspace live stream resource-scope model.
