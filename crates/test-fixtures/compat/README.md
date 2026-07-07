# Compatibility Fixtures

This directory is the reviewed golden fixture root for the Rust rewrite M0
compatibility harness. Fixtures here are intentional oracle records from the
isolated rewrite worktree and must name their requirement and contract-decision
coverage.

The initial scaffold reserves `harness/self-check` for verifying the capture
manifest format. Domain fixtures for CLI, HTTP, SSE, DSL, runtime, frontend,
and packaging are added by later M0 items.

## M0-I02 CLI And Filesystem Fixtures

- `cli/*.json` records local Python `spark` and `spark-server` process
  observations for help, flow validate/format, usage errors, and source
  checkout guards.
- `cli/server-backed/*.json` records representative `spark` commands against a
  real local `spark-server serve` process, including flow, trigger, conversation
  run-request, and run recovery error surfaces.
- `filesystem/*.json` records normalized `SPARK_HOME`, flow catalog,
  conversation, run-request, trigger definition, trigger state, and file rewrite
  effects captured through real commands.

These fixtures intentionally normalize temporary paths, localhost ports,
timestamps, generated ids, conversation handles, project ids, webhook
credentials, file hashes, and file sizes. Committed manifests use stable tokens
for local worktree, temp, API, and runtime paths so the reviewed oracle files do
not depend on ignored workflow state or one developer machine.

## M0-I03 HTTP And SSE Fixtures

- `http/*.json` records Python product, Workspace, Attractor, deprecated,
  trigger, webhook, static asset, guard, validation, and not-found route
  observations through real HTTP requests against `spark-server serve`.
- `sse/*.json` records app-shell `/workspace/api/live/events` observations for
  conversation snapshots/replay/resync, trigger snapshots and upsert/delete
  events, invalid cursor and unknown run guards, and bounded keepalive comments.

HTTP fixtures include request method/path/query/body, selected request headers,
status code, selected response headers, response body kind, route provenance,
and requirement/decision coverage. SSE fixtures include the same route
provenance plus decoded event-stream frames with JSON envelope data or comment
frames. The comparison harness normalizes temporary homes and project paths,
ephemeral localhost ports, generated trigger/run ids, webhook credentials,
timestamps, file hashes, and file sizes into stable reviewed oracle values.

## M0-I04 FlowDefinition And Runtime Fixtures

- `dsl/*.json` keeps current flow-name path-safety compatibility coverage for
  typed YAML flow names.
- `runtime/*.json` contains only reviewed runtime snapshots that are still
  aligned with typed `FlowDefinition` execution.

Retired parser, formatter, preview, and graph-shape snapshots are intentionally
absent from this fixture root. Current runtime tests construct or parse typed
YAML `FlowDefinition` documents directly.
