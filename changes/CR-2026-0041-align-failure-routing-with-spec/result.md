---
id: CR-2026-0041-align-failure-routing-with-spec
title: Align Failure Routing With Spec
status: completed
type: bugfix
changelog: internal
---

## Summary
Updated failed-node routing so explicit failure routes still win first, then other matching conditioned edges are honored before node retry fallbacks.

## Validation
- `uv run pytest -q -x --maxfail=1 tests/engine/test_retry_goal_gate.py` passed with 34 tests.
- `uv run pytest -q` passed with 1983 tests, 26 skipped, and 2 warnings.

## Shipped Changes
- Adjusted `PipelineExecutor._select_route_edge` to prefer normal selected conditioned failure routes before `retry_target` and `fallback_retry_target`.
- Added regression coverage for conditioned failure routing, retry fallback, fallback retry target behavior, exact `outcome=fail` priority, and terminal failure without a routed recovery.
- Removed the completed failure-routing drift item from `TODO.md`.
