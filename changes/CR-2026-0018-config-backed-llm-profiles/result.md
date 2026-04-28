---
id: CR-2026-0018-config-backed-llm-profiles
title: Config-Backed LLM Profiles
status: completed
type: feature
changelog: public
---

## Summary

Implemented config-backed LLM profiles so Spark can load named profiles from `$SPARK_HOME/config/llm-profiles.toml`, expose redacted profile metadata through the workspace API, and use profile ids in flows and run requests without storing endpoint URLs or secret values in project files.

## Validation

- `uv run pytest -q` passed with 1724 passed and 26 skipped.
- `npm --prefix frontend run test:unit` passed with 44 files and 312 tests.
- Frontend validation emitted existing React `act(...)` warnings, but Vitest exited successfully.

## Shipped Changes

- Added LLM profile config loading and validation for `openai_compatible` profiles, including base URL requirements, model requirements, optional `api_key_env`, safe public metadata, and adapter construction from configured profile values.
- Added `/attractor/api/llm-profiles` for UI-safe profile metadata that omits base URLs, API key values, and other secret material.
- Added `llm_profile` as a runtime selection field across graph attributes, stylesheet/default resolution, launch payloads, run records, retry, continue, child-run, and project chat paths.
- Updated backend preflight/runtime behavior so configured profiles resolve to their real provider, validate missing or malformed profile state before execution, and preserve the selected profile id in stored run metadata.
- Updated frontend settings, graph/node model controls, execution controls, API clients, store types, and payload builders so users can choose configured profiles and receive profile-specific model suggestions without duplicating provider details.
- Added and updated tests covering config parsing, profile redaction, API metadata, launch preflight, profile precedence, run-record preservation, retry/continue/child-run handling, project chat profile sessions, frontend selectors, and profile-aware payload construction.
