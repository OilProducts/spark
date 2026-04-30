---
id: CR-2026-0030-conversation-revision-cutover
title: Conversation Revision Cutover
status: completed
type: internal
changelog: internal
---

## Summary
Implemented the schema-5 conversation revision contract for project chat conversations. Conversation state now requires and persists a numeric `revision`, new conversations start at revision `0`, and successful mutations that touch conversation state advance the revision through the existing mutation path.

Frontend conversation cache freshness now uses direct revision comparison instead of snapshot scoring. Snapshots are accepted only when their revision is newer than the cached record, while stream events below the cached revision are ignored and same-or-newer revision events update the normalized record.

## Validation
- `uv run pytest -q` passed: 1748 passed, 26 skipped.
- `npm run build` passed for the frontend production build; Vite reported the existing large chunk size warning.
- `npm run test:unit` passed: 44 files, 324 tests.
- An attempted `npm run test:unit -- --runInBand` failed before running tests because Vitest does not support the Jest `--runInBand` flag; it was rerun successfully without that flag.

## Shipped Changes
- Backend conversation model and repository state handling now use schema version `5`, require `revision`, persist it, and reject schema-4 state files.
- Backend project chat snapshots, summaries, `turn_upsert`, and `segment_upsert` payloads include revision values from the current conversation state.
- Frontend API response types and parsers require revision on snapshots, summaries, and stream events.
- Frontend project conversation state removed the prior freshness scoring helpers and `conversationRecordToSnapshot()` replacement path in favor of direct revision comparisons.
- Tests were updated and added across backend API coverage, frontend API parsing, project cache behavior, app shell behavior, and project panel flows to cover revision ordering and stale-event handling.
