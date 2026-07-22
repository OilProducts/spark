# Restore Local-Container Workflow Dispatch

## Summary

Wire the already-resolved execution profile into pipeline execution so `local_container` runs use `ContainerizedNodeExecutor` instead of only recording container metadata. Preserve native behavior and fail closed when a resumed run’s placement no longer matches its original mode/image.

## Implementation Changes

- Replace the API’s native-only executor builder with one that accepts `ExecutionProfileSelection` and returns `ContainerizedNodeExecutor`; its existing mode switch will keep native profiles in-process and route container profiles through Docker.
- Pass the selected profile through both synchronous and detached fresh-run paths. Keep one executor instance for the pipeline so existing cleanup and error propagation semantics remain unchanged.
- For continue, retry, and startup resume:
  - Reload the recorded `execution_profile_id` explicitly from `execution-profiles.toml`.
  - Reject execution if the profile is missing, disabled, invalid, or its mode/image differs from the run record.
  - Persist the resulting error through the existing failed-run handling; never fall back to native execution.
- Do not change the worker protocol, Docker command construction, run-record schema, profile format, or public API payloads.

## Test Plan

- Add an API-level dispatch regression test using a fake `docker` executable:
  - A `local_container` launch issues `docker run` followed by `docker exec … spark-server worker run-node`.
  - Worker protocol output reaches the pipeline and completes the run.
  - The recorded mode, profile, and image match the executor actually used.
- Confirm a native profile completes without invoking Docker.
- Cover both waited and detached launch paths.
- Cover continue/retry profile reconstruction:
  - Matching recorded mode/image resumes through Docker.
  - Missing profile, changed mode, or changed image fails with a clear placement error and does not execute natively.
- Run the focused `attractor-execution` and `attractor-api` contract suites.

## Assumptions

- Resume placement is fail-closed based on profile ID, mode, and image. Current profile mounts and capabilities are used because the full profile is not persisted.
- Existing per-executor container lifecycle and cleanup behavior remain out of scope.
- No migration is required for existing runs that still reference a matching configured profile.
