---
id: CR-2026-0025-host-visible-docker-spark-home
title: Host-Visible Docker Spark Home
status: completed
type: feature
changelog: public
---

## Summary
The packaged Docker runtime now uses a host-visible Docker-specific Spark home while keeping the container's internal `SPARK_HOME` at `/spark`. By default, Compose bind-mounts `${SPARK_DOCKER_HOME:-$HOME/.spark-docker}` to `/spark`, so packaged flows, config, logs, and runtime state are inspectable from the host without colliding with native `~/.spark` or source-checkout `~/.spark-dev` state.

## Validation
- `docker compose -f compose.package.yaml config` passed and showed `/spark` resolved as a bind mount from the active shell's default `.spark-docker` host path.
- `bash -n scripts/run-docker.sh scripts/package-entrypoint.sh` passed.
- `uv run pytest -q` passed with 1730 passed and 26 skipped.

## Shipped Changes
- `compose.package.yaml` now bind-mounts `${SPARK_DOCKER_HOME:-$HOME/.spark-docker}` to `/spark`, keeps `${SPARK_PROJECTS_HOST_DIR:-$HOME/projects}` mounted to `/projects`, and no longer declares the old `spark_data` named volume.
- `scripts/run-docker.sh` now loads packaged Docker provider environment from `${SPARK_DOCKER_HOME:-$HOME/.spark-docker}/config/provider.env` while the container still receives `SPARK_HOME=/spark`.
- `README.md` documents the packaged Docker state location, seeded flow visibility, provider env path, native/source runtime home separation, project mount, and `SPARK_DOCKER_HOME=/some/path just run-docker` override.
