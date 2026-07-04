# Codex JSON-RPC Debug Trace Mode

## Summary
Add an explicit debug mode for recording the full Codex app-server JSON-RPC trace without polluting Spark’s durable runtime event journal. The trace file will be named `codex-jsonrpc-trace.jsonl`, and raw Codex transport lines will no longer be stored in `events.jsonl`.

## Key Changes
- Add debug enablement via:
  - `SPARK_DEBUG_CODEX_JSONRPC=1`
  - `spark-server serve --debug-codex-jsonrpc`
- Treat the server flag as process-wide debug mode for that server instance; it should set or propagate the same internal boolean used by the env var.
- Use truthy env values consistently: `1`, `true`, `TRUE`, `yes`, `YES`, `on`, `ON`.

## Runtime Logging Behavior
- Keep `events.jsonl` as the semantic Spark runtime event journal only.
- Do not emit `codex_app_server_raw_log_line` as `CodergenAdapter` events.
- When debug mode is disabled:
  - do not create `codex-jsonrpc-trace.jsonl`;
  - do not persist raw JSON-RPC lines for workspace conversations or flow/codergen runs;
  - avoid retaining large raw trace vectors just to discard them later.
- When debug mode is enabled:
  - workspace conversations write raw Codex app-server trace to:
    `workspace/projects/<project-id>/conversations/<conversation-id>/codex-jsonrpc-trace.jsonl`
  - flow/codergen stages write raw Codex app-server trace to:
    `attractor/runs/<project-id>/<run-id>/logs/<node-id>/codex-jsonrpc-trace.jsonl`
  - each line is JSONL with at least `timestamp`, `direction`, and `line`.

## Implementation Notes
- Replace the vague Rust conversation raw file name `raw-log.jsonl` with `codex-jsonrpc-trace.jsonl`.
- Replace Python-compatible stage name `raw-rpc.jsonl` in the Rust flow/codergen path with `codex-jsonrpc-trace.jsonl`.
- Introduce a small shared helper for `codex_jsonrpc_trace_enabled()` rather than scattering env parsing.
- Wire the Codex app-server adapter to an optional trace sink/callback/path so raw lines are written only in debug mode.
- Keep normalized semantic events, token usage, assistant text, tool lifecycle, and error events unchanged.

## Test Plan
- Add/update Rust tests proving raw JSON-RPC trace files are not created by default.
- Add/update Rust tests proving `SPARK_DEBUG_CODEX_JSONRPC=1` writes `codex-jsonrpc-trace.jsonl` for:
  - workspace conversation turns;
  - flow/codergen Codex app-server stages.
- Add server CLI tests for `spark-server serve --debug-codex-jsonrpc` parsing and propagation.
- Add regression coverage that Codex raw JSON-RPC lines are not emitted into runtime `events.jsonl`.
- Update existing tests that assert `raw-log.jsonl`, `raw-rpc.jsonl`, or `SPARK_ENABLE_RAW_RPC_LOG` for the Rust implementation.
- Run targeted Rust crate tests for `spark-storage`, `spark-workspace`, `spark-http`, `spark-server`, `spark-agent-adapter`, and `attractor-runtime`.

## Assumptions
- This plan targets the Rust rewrite.
- Existing Python behavior is reference material, not compatibility policy for file names.
- `events.jsonl` remains the semantic event journal name; the new descriptive name applies to the Codex JSON-RPC trace sidecar.
- The server flag is global for the running server process, not per individual `spark run launch` request.
