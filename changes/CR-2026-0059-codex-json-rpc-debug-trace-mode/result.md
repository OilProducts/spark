---
id: CR-2026-0059-codex-json-rpc-debug-trace-mode
title: Codex JSON-RPC Debug Trace Mode
status: completed
type: feature
changelog: internal
---

## Summary
Implemented an explicit Codex app-server JSON-RPC trace debug mode for the current Python server/workspace/attractor implementation. Raw Codex transport lines are now written only when `SPARK_DEBUG_CODEX_JSONRPC` is truthy or `spark-server serve --debug-codex-jsonrpc` is used, and the trace sidecar is named `codex-jsonrpc-trace.jsonl`.

## Validation
Ran `uv run pytest -q`.

Result: 1966 passed, 26 skipped.

## Shipped Changes
- Added shared debug helpers and constants for `SPARK_DEBUG_CODEX_JSONRPC`, truthy env parsing, and `codex-jsonrpc-trace.jsonl`.
- Added `spark-server serve --debug-codex-jsonrpc`, which enables the same process-wide debug env used by the runtime.
- Updated workspace conversation raw trace paths from `raw-log.jsonl` to `codex-jsonrpc-trace.jsonl` and kept raw JSON-RPC trace persistence disabled by default.
- Updated attractor Codex app-server stage trace paths from `raw-rpc.jsonl` to `codex-jsonrpc-trace.jsonl` and avoided binding raw RPC loggers when debug tracing is disabled.
- Added regression coverage for disabled-by-default trace behavior, truthy debug env values, server flag propagation, stage trace creation, and keeping raw JSON-RPC lines out of conversation `events.jsonl`.
- Updated workspace specification references from the old raw log names to the new debug-only trace file name.
