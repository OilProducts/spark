# Claude Code Conversations Get the Spark Control Surface Environment

## Summary
Workspace-assistant conversations on the `claude-code` backend are spawned without the control-surface environment their system prompt promises. `conversations.rs` (~line 3128) tells the agent that `SPARK_HOME` and `SPARK_API_BASE_URL` are set and the `spark` CLI is available; the codex backend delivers this (`build_codex_runtime_environment` injects both vars and prepends first-party tool bin dirs to `PATH`), but `claude_code.rs::run_once` spawns the CLI with the parent's raw environment plus only `CLAUDE_CONFIG_DIR`. Reuse the codex mechanism in the claude-code spawn — same crate, same statics — and make the desktop dev flow actually build the `spark` binary the `PATH` prepend points at.

## Observed defect
From a live workspace-assistant conversation (claude-code backend, `just dev-desktop-release`, project = this source checkout): `spark` not on `PATH`, `SPARK_HOME`/`SPARK_API_BASE_URL` unset, and leaked `CARGO_MANIFEST_DIR`/`OUT_DIR`/`TAURI_*` confirming raw env inheritance from the desktop parent process. Because the project is a source checkout, the `spark-common::source_checkout` guards make the CLI refuse default fallbacks — so the missing env is a hard block on the control surface (no `spark flow list`, no `spark convo run-request`), not a degraded default.

## Key Changes

1. **`crates/spark-agent-adapter/src/claude_code.rs::run_once`** — before spawn, set:
   - `SPARK_HOME` from the existing `CODEX_SPARK_HOME` static when configured (desktop already calls `configure_codex_spark_home` at startup),
   - `SPARK_API_BASE_URL` from the existing `CODEX_API_BASE_URL` static when configured (desktop and server already call `configure_codex_api_base_url`),
   - `PATH` with the existing `first_party_tool_bin_dirs()` prepended.
   Expose those three existing items from `codex_app_server.rs` as `pub(crate)` (accessors for the two statics; the helper as-is). No new module, no renames, no aliases — the "codex"-named globals are now documented as adapter-wide via a short comment at their definition.
   `list_available_claude_code_models` stays untouched (project-independent probe). Do not route claude-code through `build_codex_runtime_environment`: its HOME/CODEX_HOME/XDG rewiring and codex auth seeding are codex-specific and must not apply to claude spawns — only the three keys above.

2. **`justfile` `dev-desktop` / `dev-desktop-release`** — build the CLI into the same target dir before launching (`cargo build -p spark-cli --bin spark`, matching profile), so `first_party_tool_bin_dirs` (the running executable's directory) actually contains `spark` in desktop dev runs. Today `target/release` holds only `spark-desktop`, so the `PATH` fix alone would point at a directory without the binary.

## Test Plan
- Extend `crates/spark-agent-adapter/tests/process_contracts/claude_code_contracts.rs` (existing stub harness via `SPARK_CLAUDE_CODE_BIN`): configure the globals, run a turn against an env-dumping stub, assert the recorded env has the configured `SPARK_HOME`, `SPARK_API_BASE_URL`, and a `PATH` starting with the first-party tool bin dirs.
- Existing codex contracts stay green (`codex_app_server_contracts.rs`).
- `cargo test -p spark-agent-adapter`.
- Manual acceptance — the blocked real flow: `just dev-desktop-release`, open a workspace conversation on this checkout, assistant runs `spark flow list` successfully with no shell-profile help.

## Skipped (add when the condition arrives)
- Tauri sidecar bundling of `spark` for packaged desktop builds — add when installed desktop bundles are a deployment path; dev runs don't need it.
- `configure_codex_spark_home` call in headless `spark-server` startup — env inheritance covers headless installs today; add if a headless install reports the same gap.
- Startup diagnostic warning when no `spark` binary resolves — add if this recurs after the fix; the contract test guards the spawn env itself.
- Scrubbing inherited build vars (`CARGO_*`, `OUT_DIR`, `TAURI_*`) from agent env — cosmetic.
- Conditionalizing the conversations.rs prompt promise — the promise becomes true instead.
