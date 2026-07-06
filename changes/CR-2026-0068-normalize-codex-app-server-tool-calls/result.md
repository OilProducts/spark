---
id: CR-2026-0068-normalize-codex-app-server-tool-calls
title: Normalize Codex App-Server Tool Calls
status: completed
type: bugfix
changelog: internal
---

## Summary
Future Codex app-server command execution and file change tool events are normalized into Spark's existing renderable `tool_call` contract before they are emitted or persisted. Existing historical raw persisted tool segments were intentionally left unchanged.

## Validation
- `cargo fmt --all -- --check` - passed
- `cargo test --workspace --all-features` - passed
- `npm --prefix frontend run test:unit` - passed
- `npm --prefix frontend run build` - passed

## Shipped Changes
- Updated `crates/spark-agent-adapter/src/codex_app_server.rs` to normalize `commandExecution` and `fileChange` payloads for item start/completion events and command/file approval requests.
- Updated `crates/spark-agent-adapter/src/bin/spark-agent-fake-codex-app-server.rs` and added a Codex app-server tool notification fixture for contract coverage.
- Extended `crates/spark-agent-adapter/tests/codex_app_server_contracts.rs` and `crates/spark-workspace/tests/conversation_event_normalization_contracts.rs` to verify frontend-renderable tool call fields and persistence through workspace ingestion.
