---
id: CR-2026-0062-first-class-codex-app-server-runtime-in-rust
title: First-Class Codex App-Server Runtime In Rust
status: completed
type: feature
changelog: internal
---

## Summary

Implemented `provider=codex` as a Rust Codex app-server stdio runtime path. Codex chat, request-user-input resume, and codergen/agent-mode runs now route through the app-server backend instead of the OpenAI-compatible adapter when no explicit LLM profile is selected.

## Validation

- `cargo test -p spark-agent-adapter --test codex_app_server_contracts -- --nocapture`
- `cargo test -p spark-agent-adapter --test llm_backend_contracts codex -- --nocapture`
- `cargo test -p spark-agent-adapter -p spark-workspace -p spark-http -p attractor-runtime`
- `uv run pytest -q` passed with 2087 passed, 29 skipped, 1 warning.

Packaged live Codex smoke was not run in this pass because it requires a packaged install plus live Codex auth/model access. Deterministic fake app-server tests cover stdio launch, `initialize`/`initialized`, `thread/start`, `thread/resume`, `turn/start`, `turn/steer`, `model/list`, server-side request handling, event normalization, raw RPC logging, runtime environment setup, and Codex-backed routing.

## Shipped Changes

- Added `crates/spark-agent-adapter/src/codex_app_server.rs` and exported its Rust app-server client, backend, JSON-RPC parsing, event normalization, token-usage mapping, and isolated Codex runtime environment setup.
- Routed Codex workspace turns, request-user-input answers, and codergen agent-mode runs through the Codex app-server backend while keeping profiled and `openai_compatible` selectors on the unified LLM adapter path.
- Normalized Codex app-server notifications into existing Spark turn/session events, including assistant text, plan/reasoning deltas, tool events, request-user-input requests, context compaction, token usage, raw RPC lines, errors, turn completion, and app-server thread/turn ids.
- Aligned app-server request payloads with the generated v2 schema, including `effort` for `turn/start`, schema-backed `model/list`, and `answers` responses for `item/tool/requestUserInput`.
- Updated Rust contract tests, workspace/http normalization tests, Python compatibility fixtures, and the Rust runtime contract decision so `provider=codex` no longer means `openai_compatible`.
