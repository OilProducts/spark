# Add Stable Run Recovery CLI

## Summary
Expose run retry and continuation as stable Spark workspace commands so assistants do not have to call mounted Attractor APIs directly. Add workspace API wrappers that preserve project/conversation scoping and create a dedicated conversation-visible recovery artifact when `--conversation` is supplied.

## Key Changes
- Add workspace API wrappers:
  - `POST /workspace/api/runs/{run_id}/retry`
  - `POST /workspace/api/runs/{run_id}/continue`
- Add CLI commands:
  - `spark run retry --run <id> [--conversation <handle>] [--base-url <url>]`
  - `spark run continue --run <id> --start-node <node> --flow-source-mode snapshot|flow_name [--flow <flow_name>] [--project <path>] [--conversation <handle>] [--model <model>] [--llm-provider <provider>] [--llm-profile <profile>] [--reasoning-effort <effort>] [--base-url <url>]`
- Implement workspace wrappers by delegating to existing Attractor recovery endpoints:
  - retry calls `/attractor/pipelines/{run_id}/retry`
  - continue calls `/attractor/pipelines/{run_id}/continue`
- Add `AttractorApiClient.retry_pipeline()` and `AttractorApiClient.continue_pipeline()` helpers so workspace code does not hand-roll HTTP.
- For `--conversation`, resolve the handle and validate any explicit `--project` matches the conversation’s project.
- For detached continue, require `--project` only when overriding the source run working directory; otherwise let Attractor inherit the source run’s working directory.
- Return JSON by default with stable fields:
  - retry: `ok`, `operation`, `source_run_id`, `run_id`, `status`, optional `conversation_id`, `conversation_handle`, `run_recovery_id`
  - continue: same fields plus `start_node`, `flow_source_mode`, optional `flow_name`, and `continued_from_run_id`
- Add a dedicated conversation artifact model for recovery operations:
  - kind/segment: `run_recovery`
  - fields: `id`, `operation` (`retry` or `continue`), `source_run_id`, `result_run_id`, `status`, `project_path`, `conversation_id`, `source_turn_id`, `start_node`, `flow_source_mode`, `flow_name`, optional model/provider/profile/reasoning overrides, and `recovery_error`
  - retry records same `source_run_id` and `result_run_id`
  - continue records the original run as `source_run_id` and the new run as `result_run_id`
- Update assistant prompt/docs/control-surface text to include:
  - `spark run retry ...`
  - `spark run continue ...`
- Add the new concrete TODO/change request record and remove it from TODO when complete.

## Test Plan
- CLI tests:
  - `spark run retry --run run-1 --conversation amber-otter` posts to `/workspace/api/runs/run-1/retry`
  - `spark run continue --run run-1 --start-node run_milestone --flow-source-mode snapshot --conversation amber-otter --project /repo` posts the expected workspace payload
  - missing `--run`, missing `--start-node`, or missing `--flow-source-mode` returns usage errors
  - `--flow` is only sent for `flow_source_mode=flow_name`
- Workspace API tests:
  - retry delegates to Attractor retry and returns same-run id
  - continue delegates to Attractor continue and returns new run id plus source lineage
  - conversation handle creates a `run_recovery` segment/artifact and publishes the updated snapshot
  - explicit project mismatch with conversation handle returns `400`
  - Attractor validation errors mark the recovery artifact failed
- Existing API tests for `/attractor/pipelines/{id}/retry` and `/continue` remain unchanged.
- Documentation/control-surface tests update stable command lists.
- Run:
  - `uv run pytest -q -x --maxfail=1 tests/test_cli.py tests/api/test_project_chat.py tests/api/test_pipeline_retry_endpoint.py tests/api/test_pipeline_continue_endpoint.py`
  - `uv run pytest -q`

## Assumptions
- Workspace wrappers are the stable assistant-facing path; direct Attractor API calls remain internal/runtime surface.
- `run_recovery` is a new artifact because retry/continue are not semantically the same as launching a flow from scratch.
- Continue defaults to Attractor’s inherited source-run working directory unless `--project` is provided.
- The first supported `flow_source_mode` values are the existing API values: `snapshot` and `flow_name`.
