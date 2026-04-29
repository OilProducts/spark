# Host-Visible Docker Spark Home

## Summary
Change the packaged Docker runtime from an opaque named volume to a host-visible Docker-specific Spark home. The container will still use `/spark` internally, but Compose will bind-mount `${SPARK_DOCKER_HOME:-$HOME/.spark-docker}` there so flows, config, logs, and runtime state are inspectable on the host without colliding with native `~/.spark`.

## Key Changes
- Update `compose.package.yaml`:
  - replace `spark_data:/spark` with `${SPARK_DOCKER_HOME:-$HOME/.spark-docker}:/spark`
  - remove the unused `volumes: spark_data` declaration
  - keep `/projects` mounted from `${SPARK_PROJECTS_HOST_DIR:-$HOME/projects}`
- Update `scripts/run-docker.sh`:
  - load provider env from `${SPARK_DOCKER_HOME:-$HOME/.spark-docker}/config/provider.env`
  - keep the container’s internal `SPARK_HOME=/spark`
- Keep `scripts/package-entrypoint.sh` unchanged:
  - it seeds packaged flows into `/spark/flows`
  - with the new bind mount, those appear on the host at `${SPARK_DOCKER_HOME:-$HOME/.spark-docker}/flows`

## Documentation
- Update README Docker runtime docs to explain:
  - packaged Docker state lives under `~/.spark-docker` by default
  - seeded flows are visible at `~/.spark-docker/flows`
  - provider env for packaged Docker belongs at `~/.spark-docker/config/provider.env`
  - native install state remains under `~/.spark`
  - source dev state remains under `~/.spark-dev`
- Mention `SPARK_DOCKER_HOME=/some/path just run-docker` as the override.

## Test Plan
- Run `docker compose -f compose.package.yaml config` and verify `/spark` source resolves to the default host `.spark-docker` path.
- Run `bash -n scripts/run-docker.sh scripts/package-entrypoint.sh`.
- Optionally smoke with a temporary `SPARK_DOCKER_HOME`:
  - `SPARK_DOCKER_HOME="$(mktemp -d)" docker compose -f compose.package.yaml up --build`
  - confirm flows appear under `$SPARK_DOCKER_HOME/flows`
- Run `uv run pytest -q`.

## Assumptions
- Host inspectability is more important than keeping packaged Docker state in an opaque Docker named volume.
- `~/.spark` remains reserved for native installed Spark, and `~/.spark-dev` remains reserved for source-checkout development.
- Existing users with the old `spark_spark_data` named volume may need to manually copy or reseed state; the new default intentionally moves Docker runtime state to `~/.spark-docker`.
