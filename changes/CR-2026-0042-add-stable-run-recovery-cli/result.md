---
id: CR-2026-0042-add-stable-run-recovery-cli
title: Add Stable Run Recovery CLI
status: completed
type: feature
changelog: public
---

## Summary
Delivered stable Spark run recovery surfaces for retrying and continuing runs through workspace-scoped commands and API wrappers, so assistants can avoid direct Attractor recovery calls. Conversation-scoped recovery now records a dedicated `run_recovery` artifact and publishes updated conversation snapshots.

## Validation
- `uv run pytest -q -x --maxfail=1 tests/test_cli.py tests/api/test_project_chat.py tests/api/test_pipeline_retry_endpoint.py tests/api/test_pipeline_continue_endpoint.py`
- `uv run pytest -q -x --maxfail=1 tests/api/test_workspace_flows_endpoint.py`
- `uv run pytest -q` (1991 passed, 26 skipped, 2 warnings)

## Shipped Changes
- Added `spark run retry` and `spark run continue` CLI commands that post JSON payloads to workspace recovery endpoints.
- Added `POST /workspace/api/runs/{run_id}/retry` and `POST /workspace/api/runs/{run_id}/continue`, including conversation handle resolution, explicit project mismatch validation, stable response fields, and failed-artifact recording for Attractor validation errors.
- Added `AttractorApiClient.retry_pipeline()` and `AttractorApiClient.continue_pipeline()` helpers that delegate to the existing internal Attractor retry and continue endpoints.
- Added `RunRecovery` conversation model storage, repository serialization, creation, and result update paths for `run_recovery` segments/artifacts.
- Updated assistant prompt/control-surface text, README API and command guidance, and the packaged Spark operations guide.
- Added CLI, workspace API, conversation artifact, and Attractor client tests for the new recovery behavior.
