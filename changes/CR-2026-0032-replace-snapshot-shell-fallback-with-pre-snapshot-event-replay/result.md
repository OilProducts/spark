---
id: CR-2026-0032-replace-snapshot-shell-fallback-with-pre-snapshot-event-replay
title: Replace Snapshot Shell Fallback With Pre-Snapshot Event Replay
status: completed
type: bugfix
changelog: internal
---

## Summary
Replaced synthetic empty conversation snapshot creation with explicit handling for stream events that arrive before the first real snapshot. Unknown-conversation stream events now report `missing_record`, are buffered by the active conversation stream, and are replayed after a real snapshot only when their revision is newer than the snapshot.

## Validation
- `npm --prefix frontend run test:unit -- projectsHomeState ProjectsPanel` passed: 2 files, 49 tests.
- `uv run pytest -q` passed: 1748 passed, 26 skipped.

## Shipped Changes
- Removed `ensureConversationSnapshotShell` from the project conversation state model.
- Added `ApplyConversationStreamEventResult` statuses for applied, stale, and missing-record stream events.
- Updated project conversation cache hooks and app controller types to return and propagate stream-event application status.
- Added active-stream pre-snapshot buffering, single in-flight snapshot fetch reuse, revision-filtered replay, and artifact snapshot refresh after replay in `useConversationStream`.
- Updated project panel and project home state tests to cover pre-snapshot replay behavior and the missing-record cache contract, while keeping existing snapshot, stale-event, and artifact refresh coverage aligned with real snapshots.
