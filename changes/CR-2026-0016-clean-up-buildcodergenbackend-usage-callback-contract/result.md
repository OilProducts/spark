---
id: CR-2026-0016-clean-up-buildcodergenbackend-usage-callback-contract
title: Clean Up `_build_codergen_backend` Usage Callback Contract
status: completed
type: refactor
changelog: internal
---

## Summary

Removed the `inspect.signature` compatibility probing around `_build_codergen_backend()` and made `on_usage_update` a direct explicit argument at each backend construction site. The usage callback remains supported with `None` as an allowed value, preserving the live token usage path for run metadata, run-list updates, and cost reporting.

## Validation

- `uv run pytest -q` passed with `1698 passed, 26 skipped`.

## Shipped Changes

- Updated `src/attractor/api/server.py` to remove the `inspect` import, pass `on_usage_update` directly for normal launches, first-class child launches, and retry runner helper construction, and correct `_build_pipeline_runner_for_run`'s return annotation to its five-value return shape.
- Updated backend-builder fakes in `tests/api/test_backend_invariance.py` and `tests/api/test_pipeline_events_endpoint.py` to accept the explicit `on_usage_update=None` contract.
