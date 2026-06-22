# Fix Runs Overview Live Upserts

## Summary
Repair the frontend runs overview live-update path so `run.upsert` events are normalized through the same parser as `/attractor/runs` responses, then merged into the runs list without throwing. This fixes the full frontend unit failure and restores live status/token/cost updates in Run History.

## Key Changes
- Export a proper single-run parser from `frontend/src/lib/api/attractorApi.ts`, backed by the existing private `parseRunRecord(...)` logic; name it something explicit like `parseRunRecordPayload`.
- Re-export that parser from `frontend/src/lib/attractorClient.ts` if `useRunsList.ts` continues to import API helpers through that facade.
- Replace the missing `asRunRecord(...)` call in `useRunsList.ts` with `parseRunRecordPayload(detail?.run)`.
- Treat invalid or incomplete live upsert payloads as ignored events, not thrown errors.
- Keep `/attractor/runs` and `run.upsert` normalization identical so list fetches and live updates cannot drift.

## Test Plan
- Add or update a focused parser test proving `parseRunRecordPayload(...)` accepts the same run shape used by `parseRunsListResponse`.
- Add or update `RunsPanel` coverage for `run.upsert` events:
  - a scoped live upsert adds/replaces a run in the list.
  - a selected run receives live status/outcome updates.
  - token usage and estimated model cost from live upserts reach the selected summary.
- Run:
  - `npm --prefix frontend run test:unit -- src/lib/api/__tests__/attractorApi.test.ts src/features/runs/__tests__/RunsPanel.test.tsx`
  - `npm --prefix frontend run test:unit`
  - `uv run pytest -q`

## Assumptions
- The server `run.upsert.payload.run` shape is intentionally the same as `/attractor/runs[].run`, because both are produced from `RunRecord.to_dict()`.
- No backend API change is needed.
- This should be a narrow frontend bugfix, separate from checkpoint cleanup.
