---
id: CR-2026-0064-tauri-desktop-app-for-rust-spark
title: Tauri Desktop App For Rust Spark
status: proposed
type: feature
changelog: public
---

## Summary

Create a first Tauri v2 desktop app for the Rust Spark runtime. The app should embed the existing React frontend and run Spark's Rust HTTP server in-process, not as a sidecar. It should default to local-only access on `127.0.0.1`, with an explicit remote-access toggle that can bind the in-process server to `0.0.0.0` after warning the user.

The existing Cargo CLI and server binaries remain supported. This change adds a desktop distribution surface and should reuse the same Rust router, runtime, and Codex app-server path already proven by the installed-binary smoke.

## Key Requirements

- Add a Tauri v2 desktop app under `apps/spark-desktop/` and include its Rust crate in the Cargo workspace.
- Configure the app to build the existing `frontend` and use `frontend/dist` as the Tauri frontend assets.
- Start Spark's Axum HTTP app in-process from the Tauri Rust entrypoint:
  - resolve an app-owned Spark data directory by default using Tauri app data/config paths;
  - initialize Spark runtime directories and packaged flows on first launch;
  - construct the Rust unified LLM client from the resolved Spark settings;
  - serve `spark_http::build_app_with_rust_llm_client(...)` on an available local port;
  - load the desktop webview at the in-process server URL;
  - shut down the server cleanly when the app exits.
- Add a desktop settings bridge for server binding:
  - default host is `127.0.0.1`;
  - remote toggle host is `0.0.0.0`;
  - persist the choice in app config;
  - require an explicit warning/confirmation before enabling remote access.
- Keep existing `spark` and `spark-server` Cargo install behavior unchanged.
- Do not reintroduce Python, `uv`, wheel metadata, or old package paths.

## Implementation Guidance

- Prefer extracting reusable helpers from `spark-server` where needed instead of shelling out to the `spark-server` binary.
- Reuse existing Spark settings, asset loading, packaged flow seeding, HTTP router, and Rust LLM client behavior.
- Keep the web UI on the same HTTP origin as the API by loading the Tauri webview from the in-process server URL rather than from a separate static webview origin.
- Remote access is opt-in only for this pass. Do not add tunneling, public internet exposure, updater support, signing, or store packaging in this change.

## Test Plan

- Add Rust tests for:
  - app/server bootstrap choosing an app-owned data directory by default;
  - first launch seeding packaged flows into the app runtime;
  - remote toggle mapping to `127.0.0.1` versus `0.0.0.0`;
  - frontend URL selection using the in-process server port.
- Run the existing validation gate:
  - `npm --prefix frontend run build`
  - `cargo test --workspace --all-features`
- Add or document a desktop smoke:
  - build the Tauri app;
  - launch the app and confirm the UI loads;
  - run a Codex-backed conversation turn through the desktop app server;
  - confirm persisted conversation events include `source.backend: "codex_app_server"`.

## Assumptions

- First implementation targets Tauri v2.
- The desktop app runs Spark's HTTP server in-process.
- The app defaults to local-only access but includes a guarded remote-access toggle.
- Existing frontend remains the canonical UI.
- Existing CLI/server Cargo install path remains supported alongside the desktop app.
