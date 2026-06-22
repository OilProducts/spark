---
id: CR-2026-0051-fix-runs-overview-live-upserts
title: Fix Runs Overview Live Upserts
status: completed
type: bugfix
changelog: public
---

## Summary
The runs overview live-update path now normalizes `run.upsert` payloads through the same run-record parser used by `/attractor/runs`. Invalid or incomplete live payloads are ignored, while valid scoped upserts merge into Run History and refresh the selected run summary without throwing.

## Validation
- `npm --prefix frontend run test:unit -- src/lib/api/__tests__/attractorApi.test.ts src/features/runs/__tests__/RunsPanel.test.tsx` passed with 2 files and 21 tests.
- `npm --prefix frontend run test:unit` passed with 47 files and 344 tests.
- `uv run pytest -q` passed with 1918 passed and 26 skipped.

The runtime state's `request.validation_command` was empty, so validation followed the approved request test plan and repository test policy.

## Shipped Changes
- `frontend/src/lib/api/attractorApi.ts` exports `parseRunRecordPayload(...)`, backed by the existing `parseRunRecord(...)` normalization logic, and `parseRunsListResponse(...)` now calls that shared export.
- `frontend/src/lib/attractorClient.ts` re-exports the single-run parser for existing facade imports.
- `frontend/src/features/runs/hooks/useRunsList.ts` parses live upsert payloads with the shared parser, ignores null parses, applies project scoping, merges valid runs into the list, and updates selected-run status, outcome, error, token usage, and estimated-cost fields.
- `frontend/src/lib/api/__tests__/attractorApi.test.ts` covers single-run parsing parity with list parsing and incomplete payload rejection.
- `frontend/src/features/runs/__tests__/RunsPanel.test.tsx` covers ignored incomplete upserts plus live selected-run status, outcome, and list updates.
