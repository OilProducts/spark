---
id: CR-2026-0040-gate-performance-debug-readouts
title: Gate Performance Debug Readouts
status: completed
type: feature
changelog: internal
---

## Summary
Implemented the approved cleanup so editor canvas and run-journal performance diagnostic readouts are hidden from normal UI by default, while remaining available through a developer performance debug flag.

## Validation
- `npm --prefix frontend run test:unit -- src/__tests__/ContractBehavior.test.tsx src/features/editor/__tests__/EditorFlowLoading.test.tsx`
- `npm --prefix frontend run test:unit`
- `uv run pytest -q`

## Shipped Changes
- Added a frontend performance debug helper that enables diagnostics through `debugPerformance=1` or `spark.debug.performance=1`.
- Gated the editor canvas interaction budget and canvas performance profile badges behind the debug helper without changing medium-graph optimization metadata or timing collection.
- Gated the run timeline journal update budget and bounded-window throughput badges behind the same helper while leaving the normal `Live` / `Idle` status visible.
- Updated frontend tests to assert diagnostics are hidden by default and still available when the debug flag is enabled.
- Removed the completed performance-readout cleanup item from `TODO.md`.
