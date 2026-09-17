# Preserve containers for one run execution

## Summary

Implement the container-lifetime correction from fresh, committed `main`. Reuse one container across nodes, node retries, and result summarization within an uninterrupted execution. Explicitly clean it up when that execution ends, with cleanup failures recorded.

Use the old branch and audit tests as references; do not merge its unrelated history.

## Implementation

- Create an isolated checkout from current committed `main`. Preserve the active checkout’s uncommitted Desktop packaging, icon, and UI-audit work.
- Enable `keep_container_open()` in the shared production executor factory so synchronous launches, background launches, retries, continuations, and recovery use the same behavior.
- Add a default no-op finalization method to `NodeExecutor`, implemented by the container executor using `close()`.
- Wrap `PipelineExecutor` execution with explicit finalization on every normal return, including success, failure, cancellation, pause, and runtime errors. Complete cleanup before returning to the API and releasing the execution lock. Keep destructor cleanup as an exceptional-exit fallback.
- Record Docker command failures and nonzero removal exits through the existing `cleanup_error` field and event. Preserve the execution outcome; a cleanup failure must not overwrite the original node failure. Reuse existing persistence and notification mechanisms.
- Make finalization idempotent so explicit cleanup and destruction do not issue duplicate removal commands.
- Preserve current settings snapshots, container-side path discovery, credential forwarding, mounts, and native execution behavior. No new settings, HTTP endpoints, or persistence schema.

## Tests and validation

Port the useful lifecycle tests into current suites, updating fixtures for `SparkSettings`, target-path discovery, and current activity storage.

Verify:

- Container-local state survives across nodes and node retries; result summarization shares that container.
- Separate executions use separate containers.
- Success, node failure, runtime error, cancellation, and pause attempt final cleanup exactly once.
- Cleanup failures—both command-launch errors and nonzero exits—are recorded without changing the execution outcome.
- Retry, continuation, and recovery create a new container while retaining the existing execution configuration.
- Native runs never invoke Docker.

Run focused API/runtime/container tests first, then the repository validation gate. Add a real Docker smoke check for cross-node state and cleanup using the existing worker image, without paid model calls. If Docker is unavailable, report that validation gap explicitly.

## Boundaries and delivery

- Cancellation retains the existing “stop after the current node” behavior.
- Pausing ends the current execution; continuing creates a new container. Only mounted storage is durable across executions or process restarts.
- Do not add crash-orphan scavenging, persistent container reattachment, or automatic cleanup retries.
- Keep implementation and validation evidence together in one focused change. Report actual commits and remaining validation gaps without imposing a multi-commit sequence.
- Submit implementation through an agent-requestable Spark flow from fresh state. This session is currently in Plan Mode, and `spark flow list` failed because `spark` is unavailable; no workflow request has been created.
