---
id: CR-2026-0066-block-codex-app-server-request-user-input-until-user-answer
title: Block Codex App-Server Request-User-Input Until User Answer
status: completed
type: bugfix
changelog: public
---

## Summary
Implemented the Rust Codex app-server request-user-input bridge so `item/tool/requestUserInput` emits a pending request card and keeps the JSON-RPC request open until a user answer is submitted. The old empty-answer behavior was removed, and live answers are delivered back to the pending app-server request with the expected `{ "answers": { "<question_id>": { "answers": ["..."] } } }` payload.

## Validation
- `cargo fmt --all -- --check`
- `cargo test -p spark-agent-adapter --test codex_app_server_contracts`
- `cargo test -p spark-workspace --test conversation_event_normalization_contracts`
- `cargo test -p spark-http --test workspace_conversation_turn_route_contracts`
- `cargo test --workspace --all-features`
- `npm --prefix frontend run test:unit`
- `npm --prefix frontend run build`

## Shipped Changes
- Updated `crates/spark-agent-adapter/src/codex_app_server.rs` to register pending request-user-input JSON-RPC calls, wait for submitted answers, send the live response, and report recoverable failures for missing, mismatched, expired, or no-longer-live requests.
- Updated `crates/spark-workspace/src/conversations.rs` so a delivered live answer updates the request card while leaving the original assistant turn streaming until Codex completes it.
- Updated Codex app-server contract tests, workspace normalization tests, and fake app-server fixtures to cover blocking behavior, delivered answer payloads, and preservation of the pending turn lifecycle.
- Removed the stale test expectation that Codex app-server request-user-input calls are answered immediately with empty answers.
