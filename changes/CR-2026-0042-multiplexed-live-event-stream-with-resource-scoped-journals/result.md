---
id: CR-2026-0042-multiplexed-live-event-stream-with-resource-scoped-journals
title: Multiplexed Live Event Stream with Resource-Scoped Journals
status: completed
type: feature
changelog: public
---

## Summary
Delivered the workspace live-event model requested in CR-2026-0042: Spark now exposes a single browser/operator SSE transport at `GET /workspace/api/live/events` while keeping durable replay scoped to runs and conversations. The stream accepts observed-resource query parameters, emits typed resource envelopes, replays run journal and conversation events from resource cursors, sends targeted `resync_required` events for replay gaps, and hydrates overview-style resources through snapshots or upserts.

The old browser/operator stream paths are no longer used by the frontend or CLI. `/workspace/api/conversations/{conversation_id}/events` and `/attractor/runs/events` now return deprecation responses, and `/attractor/pipelines/{pipeline_id}/events` is marked deprecated while the remote worker run-event protocol remains untouched.

## Validation
- `uv run pytest -q` passed: 1995 passed, 26 skipped, 2 warnings.
- `npm run test:unit` in `frontend/` passed: 47 files passed, 341 tests passed.
- `npm run build` in `frontend/` passed. Vite reported the existing large chunk warning.

## Shipped Changes
- Backend API: added the workspace live-event multiplexer in `src/spark/workspace/api.py`, including run overview, selected-run journal replay, conversation replay, trigger snapshots/upserts, SSE keepalives, malformed cursor rejection, and targeted resync envelopes.
- Resource journals: added per-conversation event journal persistence and replay helpers in `src/spark/workspace/conversations/repository.py`; run replay continues to use the existing run journal/event history.
- Attractor API: deprecated app-facing run stream endpoints, removed app use of `/attractor/runs/events`, and publishes human-gate answer updates into the run event path.
- Frontend: added one app-shell `WorkspaceLiveEventsController` in `AppSessionControllers`, removed feature-owned live transports for chat/runs/triggers, and routes live envelopes into the existing conversation, run list, selected-run journal, pending-question, and trigger state handlers.
- CLI/operator surface: added `spark run events <run_id> [--after <sequence>] [--json]` using `/workspace/api/live/events`.
- Documentation/specs/tests: updated README/spec/operations docs and rewrote backend, frontend, CLI, and static-contract tests around the multiplexed stream and resource-scoped replay contract.
