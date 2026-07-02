# Spark Desktop Smoke

This smoke validates the Tauri desktop surface required by
`CR-2026-0064-tauri-desktop-app-for-rust-spark`.

## Deterministic Headless Checks

Run these from the repository root:

```sh
npm --prefix frontend run build
cargo test -p spark-desktop desktop_server_serves_web_ui_from_in_process_loopback_url
cargo build -p spark-desktop --bin spark-desktop --all-features
```

The desktop server test starts the same in-process Spark HTTP server used by
the Tauri entrypoint, confirms the chosen URL is `http://127.0.0.1:<port>/`,
and verifies that the React web UI loads from that URL.

## Codex App-Server Conversation Smoke

For a deterministic Codex-backed conversation without live credentials, first
build the repo fake app-server binary, then run the ignored desktop smoke:

```sh
cargo build -p spark-agent-adapter --bin spark-agent-fake-codex-app-server
SPARK_CODEX_APP_SERVER_BIN="$PWD/target/debug/spark-agent-fake-codex-app-server" \
  cargo test -p spark-desktop desktop_server_codex_smoke_persists_codex_app_server_backend_marker -- --ignored
```

That test starts the desktop in-process server, posts a conversation turn through
`/workspace/api/conversations/<id>/turns` with `provider: "codex"`, and confirms
the persisted conversation events include `source.backend: "codex_app_server"`.

## GUI Launch Smoke

On a machine with a desktop session and WebKit/Tauri runtime dependencies:

```sh
npm --prefix frontend run build
cargo run -p spark-desktop --bin spark-desktop --all-features
```

Confirm the Spark window opens, the Projects UI renders, and Settings exposes
the desktop server section. The server URL displayed there should match the
in-process loopback URL. Enabling remote access must show the warning and, after
confirmation, persist `remote_access_enabled: true` in the app config while
indicating that restart is required for the bind-host change.
