# Align Failure Routing With Spec

## Summary
Fix Attractor failure routing so a failed node follows matching conditioned failure routes before falling back to `retry_target` / `fallback_retry_target`. This keeps `retry_target` as a generic fallback, not an override for explicit modeled failure routing.

## Key Changes
- Update `PipelineExecutor._select_route_edge` so `FAIL` routing follows the spec order:
  - exact `condition="outcome=fail"` still wins first
  - otherwise, any true conditioned edge selected by normal routing wins next, including `outcome=fail && preferred_label=Rewrite`
  - only then use node `retry_target` / `fallback_retry_target`
  - preserve existing failure termination behavior when only unconditional routes are available
- Keep normal non-`FAIL` edge selection unchanged.
- Keep `error_policy="continue"` behavior unchanged: it may still allow ordinary selected edges after failure when configured.
- Add a completed change record under `changes/` and remove the completed failure-routing item from `TODO.md`.

## Test Plan
- Add a regression test where:
  - a failing node has `retry_target="retry"`
  - it also has `condition="outcome=fail && preferred_label=Rewrite"`
  - the outcome is `FAIL` with `preferred_label="Rewrite"`
  - expected route is the conditioned edge, not `retry_target`
- Add or adjust coverage to show:
  - exact `outcome=fail` still beats other true conditions
  - when no conditioned edge matches, `retry_target` and then `fallback_retry_target` still apply
  - a failing node without a conditioned route or retry target still fails with the stage failure reason
- Run targeted triage first:
  - `uv run pytest -q -x --maxfail=1 tests/engine/test_retry_goal_gate.py`
- Run full validation before completion:
  - `uv run pytest -q`

## Assumptions
- The spec is the intended behavior; this is `code_drift`, not `spec_drift`.
- “Other conditioned failure edges” means true conditioned edges under a failing outcome, using the existing normal condition evaluation and priority rules.
- No public API or DOT syntax changes are required.
