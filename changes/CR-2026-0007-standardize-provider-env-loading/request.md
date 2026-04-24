# Standardize Provider Env Loading

## Summary
Make `$SPARK_HOME/config/provider.env` the canonical local provider-secret file across installed service, source-checkout dev, and Docker development. Keep keys out of repo files, run context, flow DOT, and frontend payloads.

## Key Changes
- Update the generated systemd user service in `spark-server service install`.
  - Add `EnvironmentFile=-<resolved SPARK_HOME>/config/provider.env` to `spark.service`.
  - Use the resolved install data dir, so stable installs use `~/.spark/config/provider.env`.
  - Keep the file optional with `-` so installs work before users create credentials.

- Update `just dev-run`.
  - Resolve `spark_home="${SPARK_HOME:-$HOME/.spark-dev}"`.
  - If `${spark_home}/config/provider.env` exists, source/export it before starting the backend.
  - Only the backend needs these values; do not inject them into Vite/frontend.

- Update Docker development.
  - Make `just dev-docker` source `${SPARK_HOME:-$HOME/.spark-dev}/config/provider.env` before `docker compose up --build`.
  - Add pass-through entries in `compose.yaml` for `ANTHROPIC_API_KEY`, `GEMINI_API_KEY`, `GOOGLE_API_KEY`, and supported provider base-url/org/project vars already read by `UnifiedLlmClient.from_env()`.

- Update README.
  - Replace `~/.config/spark/provider.env` with `$SPARK_HOME/config/provider.env`.
  - Document stable path as `~/.spark/config/provider.env`.
  - Document dev path as `~/.spark-dev/config/provider.env` unless `SPARK_HOME` is set.
  - Remove the manual systemd drop-in as the normal path; keep it only as an advanced override if needed.

## Test Plan
- Add/update CLI tests for generated `spark.service` to assert it includes optional `EnvironmentFile=-.../config/provider.env`.
- Add a justfile/README smoke-level check if existing repo hygiene patterns support it; otherwise keep this covered by code review.
- Run:
  - `uv run pytest -q tests/test_cli.py`
  - `uv run pytest -q`
- No frontend unit tests are required unless frontend files change.

## Assumptions
- Provider keys remain process environment variables consumed by `UnifiedLlmClient.from_env()`.
- `$SPARK_HOME/config/provider.env` is local user configuration and should never be checked into a project.
- Stable and dev runtimes should intentionally use separate credential files unless the user points both at the same `SPARK_HOME`.
