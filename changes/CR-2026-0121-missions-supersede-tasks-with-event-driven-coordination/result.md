---
id: CR-2026-0121-missions-supersede-tasks-with-event-driven-coordination
title: Missions replace tasks with event-driven coordination
status: completed
type: feature
changelog: public
---

## Summary

Replaced project tasks with missions that retain the manual six-stage board and add explicit execution, owned work runs, a durable event inbox, ordered hooks, and one reaction run at a time. Delivered the first coordination slice; trigger delivery modes, outbound hooks, domain-specific reaction flows, and cost caps remain deferred.

## Validation

- `cargo test -p spark-workspace --test contracts mission_contracts`: 11 passed. Coverage includes legacy task loading, revision conflicts, project scope, idempotent events, hook precedence, reaction batching and directive ordering, budgets, cancellation, signals, terminal-result gating, recovery, and coordination through closure in Review.
- From `frontend`, `npm run test:unit -- src/features/missions/__tests__/MissionsPanel.test.tsx`: 27 passed; React `act(...)` warnings occurred in the draft-cancel test.
- `git diff --check`: passed.
- Inspected the approved request, runtime state, implementation, and added HTTP, CLI, live-stream, and browser contracts. Those additional suites, the frontend build, and `just test` were not run for this result; no recorded passing evidence for them was available in the change-request runtime directory.
- This record describes the current working-tree implementation; it does not establish a release or deployment.

## Shipped Changes

- Storage and workspace services: mission records and JSONL inboxes, adoption of existing task storage and IDs, revision-checked edits, explicit start/pause/resume/cancel/close, execution substates, budgets, run rosters, hook processing, reaction directives, and restart recovery.
- Runtime and HTTP: run-event delivery and notification after result materialization; mission CRUD, event and control routes; mission attention entries; project-scoped `mission.upsert` live updates. Task routes were replaced.
- CLI and bundled assets: `spark mission list|get|create|update|start|send|events`, the generic `missions/react.yaml` reaction flow, and updated operations guidance.
- Frontend: Missions navigation and board, retained editing/draft/conflict behavior, execution chips and run counts, state and roster views, event messaging, YAML hooks/budget editing, controls, and links into Runs. Live updates replace board polling.
- Verification surfaces: mission service contracts, renamed HTTP/CLI contracts, SSE coverage, frontend unit tests, and browser smoke scenarios for light/dark themes at wide and narrow widths.

The implementation deliberately deduplicates waiting events per run/status and retains a waiting roster status until terminal completion; repeated gates within one run are not separately tracked. Signal delivery compares checkpoint values, so consecutive identical signal values are deduplicated.
