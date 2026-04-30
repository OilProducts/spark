# Remove Obsolete Snapshot Timeline Builder

## Summary
Remove the production-dead snapshot-shaped `buildConversationTimelineEntries(snapshot, optimisticSend)` path and migrate its remaining tests to the normalized Home conversation model. Keep current Home behavior unchanged: snapshots still hydrate into normalized records, stream updates still rebuild affected turn rows, and optimistic send remains handled in the Home view model.

## Key Changes
- In `conversationTimeline.ts`, delete the snapshot-level `buildConversationTimelineEntries` export and its imports of `ConversationSnapshotResponse` and `OptimisticSendState`.
- Keep `buildConversationTimelineEntriesForTurn` as the sole timeline materialization helper used by normalized record hydration and per-turn rebuilds.
- Preserve `OptimisticSendState` and optimistic message behavior in `projectsHomeViewModel.ts`; do not move or remove it in this cleanup.
- Replace `conversationTimeline.test.ts` with normalized-model coverage using `hydrateConversationRecordFromSnapshot` and `getConversationTimelineEntries`, covering:
  - tool row, worked separator, and assistant message ordering from a hydrated snapshot
  - mode-change entry ordering
  - plan segment rendering
  - context-compaction segment rendering
  - pending and expired `request_user_input` segment rendering
- Remove the obsolete test case for `buildConversationTimelineEntries(null, optimisticSend)` because optimistic message composition now belongs to `projectsHomeViewModel`; rely on the existing view-model test for that behavior.

## Test Plan
- Run targeted frontend unit tests:
  - `npm --prefix frontend run test:unit -- conversationTimeline projectsHomeState projectsHomeViewModel`
- Run a broader Home chat rendering check:
  - `npm --prefix frontend run test:unit -- ProjectConversationHistory ProjectsPanel`
- Before reporting implementation completion, run the required full suite:
  - `uv run pytest -q`

## Assumptions
- No production code should call the removed snapshot-level builder; current search confirms only `conversationTimeline.test.ts` imports it.
- This task intentionally does not address unknown-conversation stream fallback, render-count tests, or the “Worked for …” product question beyond preserving current behavior.
