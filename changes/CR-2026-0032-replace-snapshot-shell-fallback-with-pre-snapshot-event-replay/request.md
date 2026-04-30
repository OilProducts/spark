# Replace Snapshot Shell Fallback With Pre-Snapshot Event Replay

## Summary
Remove fake empty snapshot creation while preserving the real race-safety contract: stream updates that arrive before the initial snapshot must not be lost. Unknown-conversation granular events will be held until the real snapshot is applied, then replayed if they are newer than that snapshot.

## Key Changes
- Remove `ensureConversationSnapshotShell` and stop hydrating synthetic snapshots for missing conversations.
- Change stream-event application to report whether an event was `applied`, `stale`, or `missing_record`.
- Add a short-lived pre-snapshot event buffer scoped to the active conversation stream.
  - On `missing_record`, buffer the event and ensure one snapshot fetch is in flight.
  - After applying a real snapshot, replay buffered events with `event.revision > snapshot.revision`.
  - Clear pending events when the active conversation/project changes or replay completes.
- Keep current behavior for known cached records: apply granular updates, reject stale revisions, and refresh snapshots for artifact segments.

## Tests
- Add one behavioral race test: granular events arrive before the snapshot, then a real snapshot applies, and the final conversation contains both snapshot history and newer stream updates with no duplicates.
- Remove or rewrite any existing test that expects a stream event to create a conversation from an empty fake snapshot.
- Keep existing snapshot hydration, known-record stream update, stale-event rejection, and artifact refresh tests.
- Validation:
  - `npm --prefix frontend run test:unit -- projectsHomeState ProjectsPanel`
  - `uv run pytest -q`

## Assumptions
- Snapshots are authoritative through their `revision`.
- Stream event revisions are monotonic within a conversation.
- Buffering is only needed before the first real snapshot for the active conversation; after that, granular events apply directly to the normalized cache.
