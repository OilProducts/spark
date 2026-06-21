---
id: CR-2026-0048-fix-steer-endpoint-validation-and-add-local-container-api-steering
title: Fix Steer Endpoint Validation and Add Local-Container API Steering
status: completed
type: feature
changelog: public
---

## Summary
Implemented the approved steer endpoint and local-container steering changes. `POST /pipelines/{pipeline_id}/steer` now validates the URL pipeline ID before target selection or event publication, so unknown parent IDs return `404` with `{"detail": "Unknown pipeline"}` and no intervention event. Known inactive or unsupported steering paths continue to return structured `200` rejection results.

Local-container execution now supports parent-initiated API steering while a worker process is active. The parent runner forwards intervention requests into the active container worker, the worker routes matching active-run requests to its in-process backend, and inactive, unsupported, invalid, timed-out, or failed paths return structured rejected intervention results instead of failing the run.

## Validation
- `uv run pytest -q`
- Result: 1911 passed, 26 skipped in 28.22s.

## Shipped Changes
- Updated `src/attractor/api/server.py` to apply the existing known-pipeline validation before steer request handling.
- Extended `src/attractor/handlers/execution_container.py` with local-container `request_child_intervention` support, thread-safe worker stdin writes, request ID correlation for concurrent control responses, a worker-side protocol bridge, active backend steering dispatch, timeout handling, and no-active-worker rejection behavior.
- Preserved the worker-originated callback protocol for human gates, child runs, child status checks, and child intervention requests while moving worker stdin handling behind a single protocol bridge.
- Added API coverage in `tests/api/test_pipeline_steer_endpoint.py` for unknown parent steer requests returning `404` without publishing events, and adjusted empty-message coverage to use a known pipeline.
- Added local-container coverage in `tests/handlers/test_execution_container.py` for parent intervention forwarding, concurrent response routing by request ID, inactive-worker rejection, and worker-side active-backend steering.
