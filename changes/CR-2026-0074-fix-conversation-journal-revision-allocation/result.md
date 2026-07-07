---
id: CR-2026-0074-fix-conversation-journal-revision-allocation
title: Fix Conversation Journal Revision Allocation
status: completed
type: bugfix
changelog: internal
---

## Summary

Conversation journal revision assignment now happens at the storage repository persistence boundary instead of in workspace-local snapshot handling. Persisted conversation journal payloads are stamped from the latest committed snapshot revision, stale live-turn snapshots are rebased before writing turn and segment upserts, and snapshot payloads are refreshed before they are journaled.

## Validation

- `cargo fmt --all -- --check`
- `cargo test --workspace --all-features`
- `npm --prefix frontend run test:unit`
- `npm --prefix frontend run build`

All validation commands completed successfully. The frontend unit test run still emitted existing React `act(...)`, API schema, and unhandled mock request stderr diagnostics, but the suite passed.

## Shipped Changes

- Added `ConversationRepository::persist_snapshot_with_events` to re-read the latest persisted revision, optionally rebase stale turn and segment updates, stamp only durable conversation journal payloads, write the snapshot at the final batch revision, and append the stamped events.
- Updated workspace conversation persistence paths to use the repository batch persistence helper for turn, segment, and snapshot journal writes.
- Preserved stable transcript placement by leaving turn ids, segment ids, and segment order assignment outside the revision allocation change.
- Added storage and workspace regression coverage for stale snapshot rebasing, strictly increasing request-user-input journal revisions, replay after an answered request revision, and late segment updates that retain transcript order while receiving newer journal revisions.
