---
id: CR-2026-0015-remove-inline-child-execution-while-keeping-handlerrunner-generic
title: Remove Inline Child Execution While Keeping HandlerRunner Generic
status: completed
type: refactor
changelog: internal
---

## Summary

Removed the manager-loop inline child-flow execution fallback. Child autostart now validates and prepares the resolved child graph, then launches only through the runtime `child_run_launcher` capability carried by `HandlerRunner`/`HandlerRuntime`.

`ManagerLoopHandler` remains stateless and no longer builds nested `HandlerRunner` or `PipelineExecutor` instances. Autostart without a child-run launcher now surfaces as a wiring/programmer error. Non-autostart child status observation remains unchanged.

## Validation

- `uv run pytest -q` passed: 1698 passed, 26 skipped.

## Shipped Changes

- Updated `src/attractor/handlers/builtin/manager_loop.py` to remove embedded child execution, forwarded inline child event handling, inline child workdir execution, and imports used only by that path.
- Reworked `tests/handlers/test_manager_loop_handler.py` to exercise autostart through fake `ChildRunResult` launchers while preserving coverage for path resolution, validation, stale state clearing, terminal child handling, parent control propagation, observe/steer actions, and non-autostart behavior.
- Updated `tests/engine/test_executor.py` to provide a child-run launcher for manager-loop executor coverage.
- Updated `specs/attractor-spec.md` and `specs/spark-ui-ux.md` so parent/child observability describes first-class child runs, linked child timelines, and child-owned logs/checkpoints/status/events rather than embedded inline child execution.
