---
id: CR-2026-0029-incremental-home-chat-model
title: Incremental Home Chat Model
status: completed
type: refactor
changelog: internal
---

## Summary
Implemented the Home chat cache refactor so the frontend stores normalized conversation records keyed by conversation id instead of snapshot-shaped active cache entries. Snapshot API responses remain accepted at the boundary, then hydrate normalized records; SSE turn and segment upserts now mutate those records incrementally.

## Validation
- `npm run test:unit`
- `npm run build -- --mode test`
- `uv run pytest -q`

## Shipped Changes
- Replaced `snapshotsByConversationId` with `conversationsById` across Home session/cache state and related project scope transitions.
- Added normalized conversation record hydration with ordered turn ids, turn and segment maps, artifact maps/order, event logs, and ordered timeline entry ids/data.
- Added incremental `turn_upsert` and `segment_upsert` handling that rebuilds timeline entries for the affected turn and updates conversation summaries from normalized state.
- Updated Home controllers and view model consumers to use `activeConversationRecord` and derive active history from normalized timeline entries.
- Removed the timeline object stabilization layer and identity-focused tests, while keeping observable timeline rendering coverage.
- Updated project cleanup, rename, delete, select, and registered-project removal paths to prune or reuse normalized Home conversation records, summaries, and scroll sessions.
