---
id: CR-2026-0007-standardize-provider-env-loading
title: Standardize Provider Env Loading
status: completed
type: bugfix
changelog: public
---

## Summary

Standardized local provider secret loading on `$SPARK_HOME/config/provider.env` across installed systemd service units, source-checkout development, and Docker development while keeping provider keys out of frontend/Vite environment wiring.

## Validation

- `uv run pytest -q tests/test_cli.py`: 31 passed.
- `uv run pytest -q`: 1679 passed, 26 skipped.

## Shipped Changes

- Added an optional `EnvironmentFile=-<resolved SPARK_HOME>/config/provider.env` line to generated `spark.service` units.
- Updated `just dev-run` to source `${SPARK_HOME:-$HOME/.spark-dev}/config/provider.env` only inside the backend process before starting `spark-server`.
- Updated `just dev-docker` to source the same dev provider env file before `docker compose up --build`.
- Updated `compose.yaml` backend environment pass-through names for OpenAI, Anthropic, and Gemini provider keys and base URL/org/project settings consumed by `UnifiedLlmClient.from_env()`.
- Documented stable and source-checkout provider env paths in the README, with the manual systemd drop-in retained only as an advanced override.
