# Manager-Loop Steering Contract Cleanup

## Summary
Keep Spark’s existing intervention plumbing, but make the automatic steering contract explicit and bounded: automatic manager steering is triggered only by concrete child failure context, never by stall/progress heuristics, and repeated automatic steering for the same child point is capped.

## Key Changes
- Replace the stale TODO/spec direction about progress-aware stall steering with the chosen model:
  - observe ambiguous progress telemetry
  - do not auto-steer from elapsed time, unchanged active stage, missing artifacts, or long-running work
  - auto-steer only when failure context is present
- Add an automatic steering repeat guard in `stack.manager_loop`:
  - key automatic attempts by `(child_run_id, target_node_id, failure_reason)`
  - default limit: `1` delivered or attempted automatic intervention per key per manager invocation
  - `manager.steer_cooldown` remains only a time throttle, not a progress detector
  - human/API steering is unaffected and does not consume this automatic cap

## Implementation
- In the manager-loop handler, track automatic steering keys during the manager invocation before requesting child intervention.
- When the same key exceeds the limit, skip the request and record a clear suppressed-intervention result:
  - status/reason should make it obvious, e.g. `skipped` with `auto_steer_limit_reached`
  - include the child run id, target node, and failure reason in the artifact/event payload where available
- Preserve current behavior for:
  - no failure context: no automatic intervention
  - missing active child run: rejected intervention result
  - unsupported backend: rejected intervention result
  - API/human steering through `/pipelines/{pipeline_id}/steer`
- Update docs:
  - `TODO.md`: remove/rewrite the progress-aware heuristic item
  - Attractor spec: document explicit-signal automatic steering and the repeat guard
  - Spark DOT authoring guide: document `manager.steer_cooldown` as supported, with the clarification that it only spaces attempts

## Tests
- Add manager-loop handler tests for:
  - failure context triggers one automatic intervention
  - repeated same `(child_run_id, target_node_id, failure_reason)` is suppressed on later cycles
  - different target node allows a new automatic intervention
  - different failure reason allows a new automatic intervention
  - cooldown still spaces eligible attempts
  - no failure context still does not steer
- Keep existing API steering tests unchanged, and add one assertion if needed that human/API steering is not limited by the automatic repeat guard.
- Run focused tests:
  - `uv run pytest -q -x --maxfail=1 tests/handlers/test_manager_loop_handler.py tests/api/test_pipeline_steer_endpoint.py`
- Final verification after implementation:
  - `uv run pytest -q`

## Assumptions
- The repeat limit is intentionally not configurable for this pass; use the clean default of one automatic intervention per failure point.
- The limit is scoped to a single manager-loop invocation, not persisted across retries or separate parent runs.
- Existing uncommitted parallel-join authoring work should be left intact and not mixed conceptually with this change unless the user explicitly asks to commit both together.
