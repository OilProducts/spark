---
id: CR-2026-0031-remove-obsolete-snapshot-timeline-builder
title: Remove Obsolete Snapshot Timeline Builder
status: completed
type: refactor
changelog: internal
---

## Summary
Removed the obsolete snapshot-shaped `buildConversationTimelineEntries(snapshot, optimisticSend)` export from the Home conversation timeline model. Timeline materialization now remains on the normalized conversation path through `buildConversationTimelineEntriesForTurn`, with optimistic send behavior left in the Home view model.

## Validation
- `rg "buildConversationTimelineEntries" frontend/src/features/projects -n`
- `npm --prefix frontend run test:unit -- conversationTimeline projectsHomeState projectsHomeViewModel`
- `npm --prefix frontend run test:unit -- ProjectConversationHistory ProjectsPanel`
- `uv run pytest -q`

All validation passed. The broader Home rendering check still printed an existing React `act(...)` warning from `ProjectsPanel.test.tsx`.

## Shipped Changes
- `frontend/src/features/projects/model/conversationTimeline.ts`: removed the snapshot-level timeline builder and its now-unused snapshot/optimistic state imports.
- `frontend/src/features/projects/model/__tests__/conversationTimeline.test.ts`: migrated coverage to hydrate snapshots into normalized Home conversation records and read entries via `getConversationTimelineEntries`, covering tool/final separator ordering, mode changes, plan segments, context compaction, and pending/expired `request_user_input` entries.
