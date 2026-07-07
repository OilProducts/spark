---
id: CR-2026-0073-linearize-project-chat-snapshot-hydration
title: Linearize Project Chat Snapshot Hydration
status: completed
type: refactor
changelog: internal
---

## Summary
Project chat snapshot hydration now builds the normalized conversation timeline in a bulk pass over ordered turns instead of repeatedly rebuilding timeline state one turn at a time during snapshot load. The change preserves the existing snapshot shape, cache shape, stream-event update behavior, and timeline row ordering.

## Validation
- `npm --prefix frontend run test:unit -- projectsHomeState` passed.
- `npm --prefix frontend run test:unit -- conversationTimeline` passed.
- `cargo fmt --all -- --check` passed.
- `cargo test --workspace --all-features` passed.
- `npm --prefix frontend run test:unit` passed with existing stderr warnings from unrelated React test act/schema/mock-request paths.
- `npm --prefix frontend run build` passed with the existing Vite large chunk warning.

## Shipped Changes
- Updated `frontend/src/features/projects/model/projectsHomeState.ts` so snapshot hydration indexes segments once, sorts grouped segment ids from that map, and constructs timeline ids, timeline entries, and per-turn timeline ids in one bulk helper.
- Kept `rebuildTurnTimelineEntries` available for incremental stream-event updates where rebuilding a single turn remains the intended scope.
- Updated `frontend/src/features/projects/model/__tests__/projectsHomeState.test.ts` to assert hydrated timeline ids and per-turn timeline maps, and added a many-segment ordering regression case for the bulk hydration path.
