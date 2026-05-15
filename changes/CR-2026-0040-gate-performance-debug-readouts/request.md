# Gate Performance Debug Readouts

## Summary
Hide the editor and run-journal performance/profiling readouts from normal production UI while keeping the underlying medium-graph optimizations and bounded rendering behavior unchanged. Expose those readouts only when a developer debug flag is enabled.

## Key Changes
- Add a small frontend debug helper using the existing pattern:
  - query param: `debugPerformance=1`
  - localStorage key: `spark.debug.performance=1`
- Gate the visible performance readouts behind that helper:
  - editor canvas interaction budget badge
  - editor canvas performance profile/optimization/timing badge
  - run timeline journal update budget badge
  - run timeline loaded/rendered bounded-window diagnostic badge
- Keep normal user-facing status UI intact, such as the run timeline `Live` / `Idle` indicator.
- Keep medium-graph behavior unchanged:
  - node threshold detection
  - visible-only rendering
  - preview debounce increase
  - layout/preview timing collection if still useful for debug mode
- Remove the first completed item from `TODO.md` after the cleanup is implemented.

## Test Plan
- Update frontend behavior tests so normal UI asserts these debug readouts are absent by default.
- Add or update frontend tests that enable `spark.debug.performance=1` and assert the debug readouts, data attributes, and medium-graph optimization metadata are still available.
- Adjust editor flow-loading tests that currently use the performance badge as an observable by enabling the debug flag in those tests, or switch them to a non-debug observable if cleaner.
- Run focused frontend unit coverage:
  - `npm --prefix frontend run test:unit -- src/__tests__/ContractBehavior.test.tsx src/features/editor/__tests__/EditorFlowLoading.test.tsx`
- Run full validation before completion:
  - `npm --prefix frontend run test:unit`
  - `uv run pytest -q`

## Assumptions
- “Clean that up” means hide these diagnostics from normal UI, not remove the diagnostic capability entirely.
- One general performance debug flag is preferable to separate canvas and timeline flags because these readouts are part of the same profiling/debug surface.
- No backend API, persisted state, or public Spark CLI behavior changes are needed.
