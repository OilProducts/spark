# Move Codex Runtime Home Under Spark Home

## Summary

Change Spark’s Codex runtime environment builder so the default runtime root is derived from Spark’s configured home, not `/codex-runtime` with a `/tmp` fallback. The canonical default path should be:

`<SPARK_HOME>/runtime/codex`

With no `SPARK_HOME` set, that resolves to:

`~/.spark/runtime/codex`

This keeps one shared Codex runtime per Spark home, reused across node startups and projects, which should eliminate the repeated “Refusing to create helper binaries under temporary dir /tmp” warning in normal installed usage.

## Key Changes

- Update `spark_common.codex_runtime.build_codex_runtime_environment()` to resolve the runtime root with this precedence:
  - `ATTRACTOR_CODEX_RUNTIME_ROOT` if explicitly set
  - `<SPARK_HOME>/runtime/codex` if `SPARK_HOME` is set
  - `~/.spark/runtime/codex` otherwise
- Keep `CODEX_HOME`, `XDG_CONFIG_HOME`, and `XDG_DATA_HOME` rooted under that resolved runtime root unless those env vars are explicitly overridden.
- Preserve the existing temp-dir fallback only as a last-resort recovery path when the chosen persistent runtime root cannot be created.
- Preserve current auth/config seeding behavior from the user’s existing Codex home into the runtime `CODEX_HOME`.
- Keep the change centralized in the runtime builder so both existing call sites inherit it automatically:
  - `spark_common.codex_app_client`
  - `spark.chat.session`
- Do not introduce per-project Codex homes. The shared runtime is the intended model.
- Do not hardcode `~/.spark/codex-runtime`. Align with Spark’s existing runtime-dir convention instead.

## Public Interface / Behavior Changes

- Default runtime-root behavior changes from:
  - `current: /codex-runtime, then /tmp/spark-codex-runtime`
  - `new: <SPARK_HOME>/runtime/codex, then temp only if creation fails`
- Supported override contract remains:
  - `ATTRACTOR_CODEX_RUNTIME_ROOT` still forces a custom runtime root
  - explicit `CODEX_HOME`, `XDG_CONFIG_HOME`, and `XDG_DATA_HOME` still win if set
- Operational expectation:
  - repeated Codex node startups should reuse the same persistent runtime state for a given Spark home
  - helper-binary/PATH initialization should no longer be blocked by a temp-dir `CODEX_HOME` in standard installed operation

## Test Plan

- Add unit tests for runtime-root resolution covering:
  - no env overrides → `~/.spark/runtime/codex`
  - `SPARK_HOME` set → `<SPARK_HOME>/runtime/codex`
  - `ATTRACTOR_CODEX_RUNTIME_ROOT` set → exact override path
  - explicit `CODEX_HOME` / `XDG_*` env vars remain respected
- Add a failure-path test where the chosen persistent runtime root cannot be created and the builder falls back to the temp runtime root.
- Add a regression test that verifies the returned env does not place `CODEX_HOME` under `/tmp` in the normal installed-case default.
- Add a smoke-style test for repeated builder calls or repeated client startup showing they converge on the same persistent runtime root rather than generating per-node temp homes.

## Assumptions

- This is planned as an upstream Spark change, not a machine-local hotfix to the installed `~/.spark/venv`.
- `spark_common` should remain self-contained; derive the default from `SPARK_HOME` semantics directly rather than introducing a dependency on `spark.settings`.
- Temp fallback remains necessary as an escape hatch, but it is no longer the normal path.
