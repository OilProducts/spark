---
id: CR-2026-0033-run-scoped-execution-containers-for-flow-runs
title: Run-Scoped Execution Containers for Flow Runs
status: completed
type: feature
changelog: public
---

## Summary

Implemented container execution mode for workflow runs while preserving native execution as the default. Flow launches can now resolve an execution image from an explicit run override or project default, persist the selected profile in run metadata, and expose it in run context for observability.

## Validation

- `uv run pytest -q`
- Result recorded for this change: 1764 passed, 26 skipped.

## Shipped Changes

- Added `execution_container_image` propagation through project metadata, conversation flow-run requests, workspace launches, Attractor `PipelineStartRequest`, and `spark run launch --execution-container`.
- Added execution-profile resolution, run metadata fields, context seeding, and API responses for native/container mode.
- Added `ContainerizedHandlerRunner`, Docker-backed run-scoped container lifecycle management, provider env forwarding, path mapping for packaged Docker, and cleanup/cancel handling.
- Added the hidden `spark-server worker run-node` command and JSONL worker protocol for handler execution, events, human gates, manager child-run delegation, and child status lookup.
- Updated packaged Docker support so the Spark image includes the Docker CLI and Compose mounts/passes the Docker socket, `SPARK_DOCKER_HOME`, and `SPARK_PROJECTS_HOST_DIR`.
- Added unit/API/CLI coverage for execution-profile resolution, launch payload propagation, metadata persistence, container transport behavior, cancellation, and worker protocol behavior.
