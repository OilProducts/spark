# First-Class Codex App-Server Runtime In Rust

## Summary

Implement `provider=codex` as a pure Rust Codex app-server path, using the current Codex App Server documentation as the protocol source of truth and the existing Python client/protocol as the behavioral reference. The first version supports the stdio transport only, matching Spark's current Python behavior and the single-distributable goal.

## Key Changes

- Add a Rust Codex app-server module under the agent/runtime boundary that:
  - launches `codex app-server` over stdio;
  - performs `initialize` plus `initialized` with `clientInfo.name="spark"` and `capabilities.experimentalApi=true`;
  - supports `model/list`, `thread/start`, `thread/resume`, `turn/start`, `turn/steer`, response matching by JSON-RPC id, notification buffering, and server request handling.
- Add Rust app-server event normalization equivalent to the Python behavior:
  - map app-server notifications into existing `TurnStreamEvent`/`SessionEvent` shapes;
  - preserve assistant, plan, reasoning, tool, request-user-input, token usage, context compaction, error, and turn-completed events;
  - carry `source.backend="codex_app_server"` and app-server turn/thread ids where available.
- Route `provider=codex` through the app-server backend, not `openai_compatible`:
  - Workspace conversation turns use app-server for Codex chat/plan mode.
  - Attractor/codergen agent-mode runs use app-server for Codex.
  - Non-Codex providers continue through the Rust unified LLM adapter.
- Preserve Spark runtime behavior:
  - build the Codex runtime environment in Rust using the same semantics as Python: isolated `SPARK_HOME/runtime/codex/.codex`, optional seed from `ATTRACTOR_CODEX_SEED_DIR`, fallback from host `CODEX_HOME`, and first-party tool-bin `PATH` prepending;
  - surface missing `codex`, missing working directory, init failure, timeout, resume failure, and turn failure as structured non-retryable configuration/runtime errors.
- Update docs/tests that currently assert `codex -> openai_compatible`; after this change, `codex` means Codex app-server.

## Protocol Source Requirements

- The implementing agent must first read the current Codex manual section `Codex App Server` and treat it as authoritative for protocol shape.
- It should also generate or inspect version-matched schema from the installed Codex CLI when available:
  - `codex app-server generate-json-schema --out <tmp-schema-dir>`
  - `codex app-server generate-ts --out <tmp-schema-dir>`
- The Python implementation remains a behavior reference for Spark-specific mapping, persistence, request-user-input handling, raw RPC logs, and failure messages, not the protocol authority.

## Test Plan

- Add focused Rust unit tests for:
  - JSON-RPC line parsing and malformed-line handling;
  - request id matching with interleaved notifications;
  - `initialize`, `thread/start`, `thread/resume`, `turn/start`, and `turn/steer` payloads;
  - server-side approval/request-user-input request handling;
  - app-server notification to `TurnStreamEvent` normalization;
  - token usage delta capture and raw RPC log event emission.
- Update Rust integration/contract tests so:
  - `provider=codex` routes to the app-server backend;
  - `provider=openai_compatible` remains the OpenAI-compatible adapter path;
  - Workspace chat persists app-server events and final assistant text;
  - Attractor codergen emits app-server-backed run events and supports child intervention through `turn/steer`.
- Keep Python compatibility tests for the retained Python implementation unless explicitly removed later.
- Validate with:
  - targeted Rust crate tests for `spark-agent-adapter`, `spark-workspace`, `spark-http`, and `attractor-runtime`;
  - `uv run pytest -q`;
  - packaged wheel smoke proving `spark-server` can launch a Codex-backed conversation through `codex app-server`.

## Assumptions

- First version supports stdio transport only.
- The Rust implementation should be the normal runtime path for `provider=codex`; Python is compatibility/reference only.
- WebSocket, Unix socket, and remote app-server listener support are out of scope for this pass.
- Live provider smoke is optional and should not be required for ordinary CI, but a manual packaged smoke with existing Codex auth should be run before declaring the distribution goal satisfied.
