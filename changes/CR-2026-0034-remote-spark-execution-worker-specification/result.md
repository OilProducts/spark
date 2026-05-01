---
id: CR-2026-0034-remote-spark-execution-worker-specification
title: Remote Spark Execution Worker Specification
status: completed
type: docs
changelog: internal
---

## Summary

Added a canonical target-state specification for remote Spark execution workers. The document defines remote execution as an execution-profile mode, keeps Spark as the authoritative control plane, assigns worker-local Docker/path/container responsibilities to the remote worker, and explicitly rules out hidden source checkout, sync, or provisioning.

## Validation

- Ran `uv run pytest -q`: 1765 passed, 26 skipped.

## Shipped Changes

- Added `specs/remote-execution-workers.md` covering execution profile configuration, worker/profile validation, root mapping, bearer-token authentication, worker HTTP endpoints, SSE transport semantics, event types, node result shape, human gate and child-run callbacks, runtime metadata, UI placement, failure behavior, and future acceptance scenarios.
- Updated `specs/README.md` to list `remote-execution-workers.md` as a canonical Spark target-state document for prepared remote worker hosts.
