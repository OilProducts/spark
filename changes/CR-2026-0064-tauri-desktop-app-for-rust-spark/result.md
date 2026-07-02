---
id: CR-2026-0064-tauri-desktop-app-for-rust-spark
title: Tauri Desktop App For Rust Spark
status: completed
type: feature
changelog: public
---

## Summary

Delivered a first Tauri v2 desktop app for Spark under `apps/spark-desktop`, with the app included in the Cargo workspace. The desktop entrypoint resolves app-owned data and config paths, initializes Spark runtime directories and packaged flows, builds the Rust unified LLM client from the resolved Spark settings, starts Spark's Axum HTTP app in-process, and opens the webview at the in-process server URL.

The desktop server defaults to `127.0.0.1`. Remote access is persisted as an opt-in desktop setting, maps the bind host to `0.0.0.0`, and requires explicit warning confirmation before it can be enabled. Existing `spark` and `spark-server` install behavior remains supported, and no Python, `uv`, wheel metadata, or old package paths were added.

## Validation

- `cargo fmt --all -- --check`
- `cargo test --workspace --all-features`
- `npm --prefix frontend run test:unit`
- `npm --prefix frontend run build`
- `npm --prefix frontend run build && cargo test --workspace --all-features`
- `cargo build -p spark-desktop --bin spark-desktop --all-features`
- `cargo test -p spark-desktop`
- `cargo build -p spark-agent-adapter --bin spark-agent-fake-codex-app-server`
- `SPARK_CODEX_APP_SERVER_BIN="$PWD/target/debug/spark-agent-fake-codex-app-server" cargo test -p spark-desktop desktop_server_codex_smoke_persists_codex_app_server_backend_marker -- --ignored`

The deterministic desktop smoke starts the same in-process server used by Tauri, confirms the React UI is served from `http://127.0.0.1:<port>/`, and verifies a Codex-backed conversation through the desktop server persists `source.backend: "codex_app_server"`. Native GUI launch remains documented as a manual smoke in `apps/spark-desktop/SMOKE.md` for machines with a desktop session and WebKit/Tauri runtime dependencies.

## Shipped Changes

- Added the `spark-desktop` Tauri v2 crate, Tauri config, capabilities, icon asset, smoke documentation, and desktop contract tests under `apps/spark-desktop`.
- Added the desktop crate to the Cargo workspace and updated `Cargo.lock` for the Tauri dependency graph.
- Exposed reusable `spark-server` runtime initialization and Rust LLM client construction helpers so the desktop app can run the existing HTTP router in-process instead of shelling out to the server binary.
- Added Tauri IPC commands for reading desktop server settings and persisting the guarded remote-access toggle.
- Updated the React Settings panel to show the desktop server section only when Tauri IPC is available.
