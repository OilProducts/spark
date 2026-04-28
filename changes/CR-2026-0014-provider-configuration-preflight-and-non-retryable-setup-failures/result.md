---
id: CR-2026-0014-provider-configuration-preflight-and-non-retryable-setup-failures
title: Provider Configuration Preflight And Non-Retryable Setup Failures
status: completed
type: feature
changelog: public
---

## Summary
Implemented launch-time provider configuration preflight for LLM-backed pipeline runs. Launches now fail before scheduling when the effective provider is missing required local credential material for OpenAI, Anthropic, Gemini, or Codex. Non-LLM flows skip provider credential validation.

Provider setup and authentication failures that still occur during execution are now marked non-retryable, including unsupported providers, missing Codex runtime/binary material, and API credential/configuration failures surfaced by the unified backend.

## Validation
Ran `uv run pytest -q`.

Result: 1697 passed, 26 skipped.

## Shipped Changes
Changed `src/attractor/api/server.py` to resolve effective launch providers for LLM-backed nodes, validate required local provider configuration before parent launches, retry/continue launches, and first-class child pipeline launches, and return validation errors without scheduling runs when material is missing.

Changed `src/attractor/api/codex_backends.py` to classify provider setup/auth/configuration failures as non-retryable runtime failures.

Changed `src/attractor/handlers/builtin/manager_loop.py` so first-class child provider validation failures are treated as ordinary failed child runs and preserve the specific provider failure reason on the parent.

Changed API test fixtures and coverage in `tests/api/conftest.py`, `tests/api/test_backend_invariance.py`, `tests/api/test_manager_loop_pipeline_api.py`, and `tests/api/test_pipeline_retry_endpoint.py` to cover configured defaults, missing provider credential rejection, missing Codex auth rejection, non-LLM preflight bypass, inherited retry provider validation, first-class child provider validation failure propagation, and non-retryable setup failures.
