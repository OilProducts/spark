---
id: CR-2026-0046-fix-frontend-checkpoint-node-semantics
title: Fix Frontend Checkpoint Node Semantics
status: completed
type: bugfix
changelog: public
---

## Summary

Frontend run status and checkpoint detail handling now follows the backend checkpoint model. Status progress reads `active_node`, `last_completed_node`, and `completed_count`, while the UI-local `current_node` value is derived from `active_node`. Checkpoint summaries use `checkpoint.active_node` as the current/resume node and do not fall back to `last_completed_node` for terminal checkpoints.

## Validation

- `npm run test:unit -- --run src/lib/api/__tests__/attractorApi.test.ts src/features/runs/model/__tests__/runDetailsModel.test.ts` passed: 2 files, 6 tests.
- `npm run test:unit -- --run src/__tests__/ContractBehavior.test.tsx src/features/runs/__tests__/RunsPanel.test.tsx` passed: 2 files, 70 tests. Existing console stderr from degraded endpoint scenarios and React act warnings was emitted.
- `uv run pytest -q -x --maxfail=1 tests/api/test_pipeline_status_endpoint.py tests/api/test_pipeline_checkpoint_endpoint.py tests/api/test_pipeline_events_endpoint.py` passed: 22 tests.
- `uv run pytest -q` passed: 1865 passed, 26 skipped.

## Shipped Changes

- Updated `frontend/src/lib/api/attractorApi.ts` to parse backend-shaped progress payloads with `active_node`, `last_completed_node`, and `completed_count`, deriving the frontend run record's `current_node` from `active_node`.
- Updated `frontend/src/features/runs/model/runDetailsModel.ts` so checkpoint summaries source the current/resume node from `active_node` and show the empty placeholder when terminal checkpoints have no active node.
- Reworked frontend API, run-details, runs-panel, contract, and smoke-test fixtures/assertions to use `active_node` and `last_completed_node` instead of wire-level `current_node` for checkpoint and progress payloads.
