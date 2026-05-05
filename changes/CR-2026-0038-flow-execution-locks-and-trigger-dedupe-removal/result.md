---
id: CR-2026-0038-flow-execution-locks-and-trigger-dedupe-removal
title: Flow Execution Locks and Trigger Dedupe Removal
status: completed
type: feature
changelog: internal
---

## Summary
Delivered workspace-managed execution locks for flow launches and removed trigger dedupe suppression. Flows can now declare an `execution_lock` in the workspace flow catalog, launches with the same project-scoped lock key queue FIFO behind the active holder, queued runs retain visible run records and lock metadata, and trigger firings no longer skip duplicate webhook request ids, poll item ids, or repeated flow events.

## Validation
- `uv run pytest -q` passed with `1971 passed`, `26 skipped`, and `2 warnings` in `47.12s`.
- The passing suite includes the new backend lock/queue coverage in `tests/api/test_workspace_flows_endpoint.py`, trigger repeat-fire coverage in `tests/api/test_triggers_endpoint.py`, and the UI regressions in the relevant frontend unit tests.

## Shipped Changes
- Workspace flow catalog and API: `src/spark/workspace/flow_catalog.py` and `src/spark/workspace/api.py` now read, validate, persist, and expose per-flow `execution_lock` config plus the allowed scope/conflict-policy enums; `frontend/src/lib/api/flowsApi.ts` and `frontend/src/features/editor/services/graphLaunchPolicy.ts` were updated to carry that schema.
- Run admission and runtime state: `src/attractor/api/server.py` now resolves project-scoped lock identities, persists holder/queue state in runtime storage, queues conflicting launches, starts the next queued run on terminal completion, and returns queued launch responses; `src/attractor/api/run_records.py` and `src/attractor/api/pipeline_runs.py` now persist `execution_lock` metadata on run records.
- Operator UI: `frontend/src/features/editor/GraphSettings.tsx` and `frontend/src/features/editor/components/graph-settings/GraphSettingsSections.tsx` added execution-lock controls next to Launch Policy, `frontend/src/features/execution/ExecutionControls.tsx` shows lock metadata during manual launch and handles queued launch responses, and the Runs surfaces in `frontend/src/features/runs/*` now display lock holders, queue position, and queued groups by lock identity.
- Trigger behavior and regression coverage: `src/spark/workspace/triggers.py` removed request-id/item-id/run-id dedupe state while keeping recent execution history, `frontend/src/lib/api/triggersApi.ts` dropped the old dedupe-key expectation, and request-aligned coverage was added in `tests/api/test_workspace_flows_endpoint.py`, `tests/api/test_triggers_endpoint.py`, `frontend/src/features/editor/__tests__/GraphSettings.test.tsx`, `frontend/src/features/execution/__tests__/ExecutionControls.test.tsx`, and `frontend/src/features/runs/__tests__/RunsPanel.test.tsx`.
