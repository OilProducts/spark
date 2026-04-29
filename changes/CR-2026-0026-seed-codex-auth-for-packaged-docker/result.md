---
id: CR-2026-0026-seed-codex-auth-for-packaged-docker
title: Seed Codex Auth For Packaged Docker
status: completed
type: bugfix
changelog: public
---

## Summary

Packaged Docker startup through `just run-docker` now seeds missing Codex auth/config files into the host-visible Docker Spark home before starting Compose. The launcher copies `auth.json` and `config.toml` from `${CODEX_HOME:-$HOME/.codex}` into `${SPARK_DOCKER_HOME:-$HOME/.spark-docker}/runtime/codex/.codex/` only when the corresponding destination file is absent, preserving separate packaged Docker credentials after initialization.

## Validation

- `bash -n scripts/run-docker.sh scripts/package-entrypoint.sh`
- `uv run pytest -q tests/test_run_docker.py` (`3 passed`)
- `uv run pytest -q` (`1733 passed, 26 skipped`)

## Shipped Changes

- Updated `scripts/run-docker.sh` to resolve host and packaged Docker Codex homes, create the packaged Docker Codex directory, copy missing `auth.json` and `config.toml` from the host Codex home, and apply best-effort `chmod 600` to copied files.
- Preserved existing provider env sourcing from `${SPARK_DOCKER_HOME:-$HOME/.spark-docker}/config/provider.env` and the existing `docker compose -f compose.package.yaml up --build` startup path.
- Added shell-level launcher tests in `tests/test_run_docker.py` covering seeding, permission tightening, preservation of existing packaged Docker Codex files, tolerance for missing host Codex files, provider env sourcing, and Compose invocation.
- Updated the packaged Docker README section to document first-launch Codex seeding, destination paths, preservation behavior, and the requirement to use `just run-docker` for host-side seeding.
