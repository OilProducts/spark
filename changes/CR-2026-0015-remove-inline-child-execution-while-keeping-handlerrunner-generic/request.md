# Remove Inline Child Execution While Keeping HandlerRunner Generic

## Summary
Clean up the old embedded child-flow execution path without making `HandlerRunner` non-generic. `HandlerRunner` remains a generic handler invocation adapter that can carry optional per-run capabilities. `ManagerLoopHandler` remains stateless and consumes the child-run capability only when autostarting a child flow. The only supported child-flow execution implementation becomes the first-class child-run launcher supplied by the API/runtime.

## Key Changes
- Remove the inline fallback branch in `ManagerLoopHandler` that builds a nested `HandlerRunner`/`PipelineExecutor` and runs a child graph inside the parent node.
- Keep `child_run_launcher` as an optional runtime capability on `HandlerRunner`/`HandlerRuntime`; do not move it onto `ManagerLoopHandler`.
- In manager child autostart, after path resolution and child graph validation, call the runtime child-run launcher directly.
- If `stack.child_dotfile` autostart is reached without a launcher, allow that to surface as a programmer/wiring error rather than a normal workflow fallback.
- Delete `_run_child_executor_in_workdir()` and remove imports only needed by inline child execution.
- Keep `stack.child_autostart=false` behavior unchanged: manager-loop can still observe/resolve pre-populated child status from context without launching a child.

## Spec And Docs
- Update `specs/attractor-spec.md` Section 4.11 so `start_child_pipeline(...)` is described as launching a first-class child run, not embedded execution.
- Update stale observability wording that implies forwarded child events are the only parent/child model. Parent events may link to child timelines, while child runs own their logs, checkpoints, status, and event stream.
- Do not introduce a new public API or new child status type.

## Test Plan
- Rewrite handler-level manager-loop autostart tests to provide a fake `child_run_launcher` returning `ChildRunResult`.
- Preserve coverage for child path resolution, child graph validation, stale child-state clearing, duplicate running-child suppression, terminal child status resolution, observe/steer actions, and non-autostart behavior.
- Remove inline-specific assertions that inspect `logs/manager/child`, forwarded inline child events, or backend calls made by an embedded child executor.
- Keep API-level first-class child tests as the source of truth for real child run creation, metadata, events, cancellation, retry, and run-list visibility.
- Run:
  - `uv run pytest -q tests/handlers/test_manager_loop_handler.py`
  - `uv run pytest -q tests/api/test_manager_loop_pipeline_api.py tests/api/test_pipeline_events_endpoint.py tests/api/test_pipeline_cancel_endpoint.py tests/api/test_pipeline_retry_endpoint.py`
  - `uv run pytest -q`

## Assumptions
- Child autostart without a runtime child-run launcher is programmer error, not supported runtime behavior.
- `HandlerRunner` should remain generic and reusable; it should not know how to execute child workflows itself.
- `ManagerLoopHandler` should remain a stateless shared handler and should not store per-run launch infrastructure directly.
