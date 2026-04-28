# Clean Up `_build_codergen_backend` Usage Callback Contract

## Summary
Remove the fake-driven compatibility probing around `_build_codergen_backend()` and make `on_usage_update` a normal explicit part of the backend-construction contract. Keep `on_usage_update` itself, because it carries live token usage from Codex/unified backends into run metadata, run-list updates, and cost reporting.

## Key Changes
- In `src/attractor/api/server.py`, remove the `inspect` import and all `inspect.signature(_build_codergen_backend)` branches.
- At each backend construction site, pass `on_usage_update` directly:
  - normal pipeline launch uses the run’s live usage handler
  - first-class child launch uses the child run’s live usage handler
  - retry runner helper passes through its optional usage handler
- Keep `_build_codergen_backend(..., on_usage_update=None)` and `build_codergen_backend(..., on_usage_update=None)` as the explicit contract.
- Fix `_build_pipeline_runner_for_run`’s return annotation to match its actual five-value return shape.
- Update the remaining test fake builders in `tests/api/test_backend_invariance.py` so they accept `on_usage_update=None`; do not preserve production compatibility code for obsolete fake signatures.

## Test Plan
- Run focused tests that exercise affected launch paths:
  - `uv run pytest -q tests/api/test_backend_invariance.py`
  - `uv run pytest -q tests/api/test_manager_loop_pipeline_api.py tests/api/test_pipeline_retry_endpoint.py`
  - `uv run pytest -q tests/api/test_runs_endpoint.py`
- Run the full suite before reporting completion:
  - `uv run pytest -q`

## Assumptions
- `on_usage_update` remains required as a supported builder parameter, while the callback value may be `None`.
- Tests should model the production backend-builder contract rather than forcing production code to tolerate legacy fake-only signatures.
- No new source-text tests should be added for the absence of `inspect.signature`; behavior-level tests are sufficient.
