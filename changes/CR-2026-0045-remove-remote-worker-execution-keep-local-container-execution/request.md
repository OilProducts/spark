# Remove Remote Worker Execution, Keep Local Container Execution

## Summary

Hard-remove Spark’s remote-worker execution path. Keep execution profiles as the public selection mechanism, but limit supported execution modes to `native` and `local_container`. Preserve local container execution through the existing container runner and hidden `spark-server worker run-node` process entrypoint; remove the standalone remote worker HTTP/SSE service.

## Key Changes

- Execution model:
  - Remove `remote_worker` from `EXECUTION_MODES` and `ExecutionMode`.
  - Remove `WorkerProfile`, remote-worker path mapping, remote launch admission, remote client/runner/worker service models, and remote-worker-only metadata.
  - Keep `ExecutionProfile` with `native` and `local_container`; keep `image` required for `local_container`.

- Config and API behavior:
  - Update `execution-profiles.toml` parsing to ignore/remove `[workers.*]` support and reject `mode = "remote_worker"` as an invalid mode.
  - Keep `execution_profile_id` selection for CLI, workspace, chat approvals, and launch APIs.
  - Update execution-placement settings to return only `native` and `local_container` modes, profiles only, and no worker health/protocol payload.
  - Remove remote dispatch branches from pipeline launch and child pipeline launch; local container dispatch remains unchanged.

- CLI and docs:
  - Remove `spark-server worker serve`.
  - Keep hidden `spark-server worker run-node` because local container execution depends on it.
  - Replace or remove remote-worker specs/docs so Spark documents local execution profiles and local container execution only.
  - Update README/help text to avoid advertising remote workers.

- Tests:
  - Delete tests that only validate remote-worker behavior: remote client, remote runner, remote worker app/state/protocol, remote launch admission, remote path mapping, and worker serve CLI.
  - Rewrite execution profile config/settings tests around `native` and `local_container`.
  - Update API/status/context tests to remove remote-worker metadata expectations while preserving local container image/profile metadata.
  - Remove repo-hygiene coverage files or assertions that require remote-worker package boundaries.

## Test Plan

- Run focused tests first:
  - `uv run pytest -q -x --maxfail=1 tests/execution tests/api/test_execution_placement_settings.py tests/test_cli.py`
  - `uv run pytest -q -x --maxfail=1 tests/api/test_pipeline_context_endpoint.py tests/api/test_pipeline_status_endpoint.py`
- Then run the full suite:
  - `uv run pytest -q`

## Assumptions

- This is a hard cut: no compatibility path for configured `remote_worker` profiles.
- Historical remote-worker run records do not need special migration support.
- Local container execution remains supported through execution profiles and the existing containerized node runner.
- `spark-server worker run-node` stays as an internal process-level entrypoint, but `spark-server worker serve` is removed.
