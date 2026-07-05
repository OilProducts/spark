---
id: CR-2026-0065-codex-jsonrpc-debug-trace-mode
title: Codex JSON-RPC Debug Trace Mode For Rust
status: completed
type: feature
changelog: public
---

## Summary

Implemented an explicit Rust debug mode for Codex app-server JSON-RPC tracing. Raw Codex transport lines now write to the `codex-jsonrpc-trace.jsonl` sidecar only when enabled through `SPARK_DEBUG_CODEX_JSONRPC` or `spark-server serve --debug-codex-jsonrpc`, while Spark's durable `events.jsonl` remains limited to semantic runtime events.

## Validation

- `cargo fmt --all -- --check`
- `cargo test -p spark-storage -p spark-workspace -p spark-http -p spark-server -p spark-agent-adapter -p attractor-runtime`
- `cargo test --workspace --all-features`
- `npm --prefix frontend run test:unit`
- `npm --prefix frontend run build`

The recorded evaluate stage completed successfully. Frontend validation passed with existing stderr warnings from React `act(...)` usage and mocked API error paths.

## Shipped Changes

- Added shared `spark-common` debug helpers/constants for `SPARK_DEBUG_CODEX_JSONRPC`, truthy env parsing, trace file naming, and trace path metadata.
- Added `spark-server serve --debug-codex-jsonrpc` and process-wide propagation of the same debug setting used by the env var.
- Updated the Codex app-server adapter to use an optional trace sink that writes JSONL records with `timestamp`, `direction`, and `line`, avoiding disabled-mode buffering and persistence.
- Wired workspace conversation turns, request-user-input answers, and codergen stage runs to pass trace sidecar paths only when debug tracing is enabled.
- Replaced Rust raw trace sidecar naming from `raw-log.jsonl`/`raw-rpc.jsonl` to `codex-jsonrpc-trace.jsonl` on the implemented paths.
- Removed raw Codex JSON-RPC line emission from `CodergenAdapter` runtime events while preserving normalized semantic events, assistant text, token usage, tool lifecycle, and error behavior.
- Updated Rust contract tests for default no-trace behavior, debug trace writes, CLI/env parsing, and regression coverage around raw transport lines not entering `events.jsonl`.
