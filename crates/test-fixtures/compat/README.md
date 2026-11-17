# Compatibility fixtures

The Rust tests still consume these captured contracts:

- `http/attractor-status.json`: idle status response.
- `agent/codex-app-server-tool-notifications.jsonl`: Codex tool notifications.
- `providers/*.json`: provider configuration, request, retry, and usage contracts.
- `sse/live-conversation-*.json`: conversation snapshot, replay, and resync envelopes.

The `frontend/` records are kept outside the Rust test cleanup.
