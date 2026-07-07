# Linearize Project Chat Snapshot Hydration

## Summary
Refactor project chat snapshot hydration so loading a conversation builds the normalized record and timeline in one linear pass, without changing API responses, stored state shape, stream-event behavior, or rendered output.

## Key Changes
- Replace the snapshot-only use of `rebuildTurnTimelineEntries` with a bulk timeline builder used by `hydrateConversationRecordFromSnapshot`.
- Build `segmentsById` once before sorting grouped segment ids, and sort each turn’s segment ids by looking up in that map instead of calling `snapshot.segments.find(...)` inside the comparator.
- Construct `timelineEntryIds`, `timelineEntriesById`, and `timelineEntryIdsByTurnId` directly while iterating `orderedTurnIds` once.
- Keep `rebuildTurnTimelineEntries` for incremental `turn_upsert` and `segment_upsert` stream events, where rebuilding one turn is still the right scope.
- Preserve existing timeline ordering and row generation exactly: user messages, assistant messages, reasoning, plans, tool calls, worked separator, request-user-input, flow launch/request rows, mode changes, and context compaction.

## Tests
- Update/add unit coverage in `projectsHomeState.test.ts` to assert hydration still produces the same ordered timeline and artifact maps.
- Add a regression test with many turns and segments that verifies hydration completes through the bulk path and produces stable ordering; do not assert brittle timing, but structure the test so it would catch accidental per-turn rebuild behavior if practical.
- Run focused tests first:
  - `npm --prefix frontend run test:unit -- projectsHomeState`
  - `npm --prefix frontend run test:unit -- conversationTimeline`
- Then run the repo validation gate:
  - `cargo fmt --all -- --check`
  - `cargo test --workspace --all-features`
  - `npm --prefix frontend run test:unit`
  - `npm --prefix frontend run build`

## Assumptions
- This pass is frontend-only.
- No endpoint shape, conversation snapshot schema, cache shape, or rendering behavior changes.
- This pass does not add virtualization/windowing or partial snapshot loading; those are separate follow-up optimizations after hydration is no longer quadratic.
