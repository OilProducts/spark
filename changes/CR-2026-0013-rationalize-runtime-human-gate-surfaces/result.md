---
id: CR-2026-0013-rationalize-runtime-human-gate-surfaces
title: Rationalize Runtime Human Gate Surfaces
status: completed
type: refactor
changelog: internal
---

## Summary
Implemented the runtime human-gate surface rationalization so pending runtime gates remain answerable from the Runs pinned pending-questions panel, while Execution, run graph nodes, summaries, and journal surfaces serve only as context, status, or audit surfaces.

## Validation
- `npm run test:unit -- src/features/workflow-canvas/__tests__/TaskNode.test.tsx src/features/execution/__tests__/ExecutionControls.test.tsx src/__tests__/ContractBehavior.test.tsx`
- `uv run pytest -q -x --maxfail=1 tests/docs_traceability/test_ui_spec_artifacts.py tests/contracts/frontend/test_human_gate_discoverability_contracts.py`
- `uv run pytest -q`

All recorded validation passed.

## Shipped Changes
- Removed the Execution pending human-gate banner and `View run` handoff from `ExecutionControls`.
- Removed Execution sidebar use of global `humanGate` state for flow indicators.
- Removed run-canvas node answer controls while preserving waiting-state/highlight behavior.
- Updated existing frontend contract and TaskNode tests to assert duplicate Execution and node-toolbar answer surfaces are absent.
- Updated `specs/spark-ui-ux.md` to state that Runs is the only runtime human-gate operating surface and that summary, graph, journal, and Execution surfaces must not own answers.
