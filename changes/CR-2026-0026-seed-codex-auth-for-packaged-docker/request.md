# Seed Codex Auth For Packaged Docker

## Summary
Make `just run-docker` / `scripts/run-docker.sh` prepare the packaged Docker Spark home so Codex-backed runs work without manual file copying. The launcher will copy missing Codex auth/config files from the host Codex home into the host-visible Docker Spark home before starting Compose.

## Key Changes
- Update `scripts/run-docker.sh` to resolve:
  - Docker Spark home: `${SPARK_DOCKER_HOME:-$HOME/.spark-docker}`
  - Host Codex home: `${CODEX_HOME:-$HOME/.codex}`
  - Docker Codex home: `${SPARK_DOCKER_HOME:-$HOME/.spark-docker}/runtime/codex/.codex`
- Before `docker compose -f compose.package.yaml up --build`, create the Docker Codex home and copy these files if they exist on the host and are missing in the Docker home:
  - `auth.json`
  - `config.toml`
- Do not overwrite existing Docker Codex files. This lets packaged Docker keep separate auth/config once initialized.
- Set restrictive permissions on copied files where supported, using best-effort `chmod 600`.
- Keep provider env behavior unchanged: `run-docker.sh` still sources `${SPARK_DOCKER_HOME:-$HOME/.spark-docker}/config/provider.env`.
- Update README packaged Docker docs to say:
  - Codex auth is seeded from `${CODEX_HOME:-$HOME/.codex}` on first `just run-docker`.
  - Seeded files live at `~/.spark-docker/runtime/codex/.codex/`.
  - Existing Docker Codex auth/config is preserved.
  - Raw `docker compose -f compose.package.yaml up --build` does not perform host-side seeding; `just run-docker` is the supported launcher.

## Test Plan
- Add focused tests for the launcher script behavior, preferably through a shell-level test that runs `scripts/run-docker.sh` with a fake `docker` executable earlier on `PATH`.
- Cover:
  - Missing Docker Codex home is created.
  - Existing host `auth.json` and `config.toml` are copied when destination files are absent.
  - Existing destination files are not overwritten.
  - Missing host Codex files do not fail startup.
  - Existing provider env sourcing still works.
- Run:
  - `bash -n scripts/run-docker.sh scripts/package-entrypoint.sh`
  - Targeted new tests
  - Full suite: `uv run pytest -q`

## Assumptions
- `just run-docker` is the supported packaged Docker startup path.
- The fix should not make raw Compose startup responsible for seeding host secrets.
- Copy-if-missing is intentional; explicit refresh can be added later if needed.
