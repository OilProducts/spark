---
id: CR-2026-0045-remove-remote-worker-execution-keep-local-container-execution
title: Remove Remote Worker Execution, Keep Local Container Execution
status: completed
type: refactor
changelog: public
---

## Summary

Spark's standalone remote-worker execution path has been removed. Execution profiles remain the public selection mechanism, but the supported modes are now only `native` and `local_container`. Local container execution is preserved through the existing container runner and hidden `spark-server worker run-node` process entrypoint.

## Validation

- `uv run pytest -q` passed with 1865 tests and 26 skipped.

## Shipped Changes

- Removed the `remote_worker` execution mode, worker profile model, remote path mapping, remote launch dispatch, remote client/runner modules, standalone worker HTTP/SSE service modules, and remote-worker-only metadata.
- Updated execution profile parsing and execution-placement responses so `[workers.*]` tables are ignored, `mode = "remote_worker"` is rejected, only `native` and `local_container` are reported, and local container profiles still require `image`.
- Kept `execution_profile_id` selection across CLI/API/workspace surfaces and kept local container node execution routed through `spark-server worker run-node`; removed the public `spark-server worker serve` command.
- Updated frontend execution settings, run metadata handling, API schema types, README/spec documentation, Docker packaging helpers, and regression tests to reflect local-only execution profiles and the removal of remote-worker surfaces.
- Deleted remote-worker-only tests, specs, repo-hygiene coverage data, and assertions that described the removed remote worker package boundaries.
