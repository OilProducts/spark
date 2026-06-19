# Fix Frontend Checkpoint Node Semantics

## Summary

Update the frontend and its tests to match the backend checkpoint model: `active_node` is the next/resumable node, and `last_completed_node` is the most recent completed or terminal node. Remove remaining frontend assumptions that checkpoint/progress payloads use `current_node`.

## Key Changes

- Update frontend API types and parsing in `frontend/src/lib/api/attractorApi.ts`:
  - Accept `progress.active_node`, `progress.last_completed_node`, and `progress.completed_count`.
  - Derive the UI-facing run `current_node` from `progress.active_node` for active/resumable progress.
  - Stop depending on `progress.current_node` except, at most, as a deleted-test cleanup path if no production compatibility is desired.

- Update checkpoint detail summarization in `frontend/src/features/runs/model/runDetailsModel.ts`:
  - Read `checkpoint.active_node` as the primary “current/resume node” display value.
  - Optionally expose/display `last_completed_node` if the existing checkpoint card can do so cleanly; otherwise keep the card’s current label but source it from `active_node`.
  - For terminal checkpoints where `active_node` is null, show `—` for current/resume node rather than falling back to the completed node.

- Rewrite stale frontend tests and fixtures:
  - Replace checkpoint fixtures using `current_node` with `active_node` and `last_completed_node`.
  - Replace status progress fixtures using `progress.current_node` with `progress.active_node`.
  - Update smoke-test checkpoint payload assertions to expect `"active_node"` instead of `"current_node"`.
  - Remove tests that assert the old checkpoint/progress shape, rather than preserving compatibility with it.

## Test Plan

- Run targeted frontend unit tests:
  - `cd frontend && npm run test:unit -- --run src/lib/api/__tests__/attractorApi.test.ts src/features/runs/model/__tests__/runDetailsModel.test.ts`
  - Add/update assertions proving new backend-shaped payloads populate run status and checkpoint summaries correctly.

- Run the affected broader frontend tests if changed:
  - `cd frontend && npm run test:unit -- --run src/__tests__/ContractBehavior.test.tsx src/features/runs/__tests__/RunsPanel.test.tsx`

- Run backend focused tests to ensure the backend contract remains unchanged:
  - `uv run pytest -q -x --maxfail=1 tests/api/test_pipeline_status_endpoint.py tests/api/test_pipeline_checkpoint_endpoint.py tests/api/test_pipeline_events_endpoint.py`

- Before completion, run the full backend suite per repo policy:
  - `uv run pytest -q`

## Assumptions

- This is a hard cutover: no frontend compatibility branch for old `current_node` checkpoint/progress payloads.
- The backend shape is authoritative: progress contains `active_node`, `last_completed_node`, and `completed_count`; checkpoint JSON contains `active_node`, `last_completed_node`, `completed_nodes`, context, retry counts, logs, and timestamp.
- The frontend’s existing `RunRecord.current_node` field may remain as UI-local derived state, but it should be populated from backend `active_node`, not from a wire-level `current_node`.
