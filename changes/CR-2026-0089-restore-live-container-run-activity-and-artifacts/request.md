# Restore Live Container Run Activity and Artifacts

## Summary

Make containerized node execution match native execution for runtime events, live activity notifications, and run-local artifacts. The worker will stream events over its existing JSONL protocol, while the host remains the sole writer of `events.jsonl`. The worker will receive the already-mounted run paths for logs and artifacts without writing the journal itself.

## Implementation Changes

- Add a runtime event sink to `RuntimeHandlerRunner`.
  - Route all operational and Codergen event emission through one internal dispatch path.
  - When an external sink is configured, send the complete `RawRuntimeEvent` to it instead of persisting locally.
  - Otherwise retain native behavior: append under `event_append_lock`, then invoke the run-event observer.
  - Expose a narrow host-ingestion method that performs that same append-and-notify operation so the container executor reuses the existing persistence path.

- Restore worker runtime context.
  - Extend `WorkerNodeRequest` with the run-root metadata needed by `RunRootPaths::from_existing_root`.
  - Reconstruct `RunRootPaths` inside the worker so existing log, trace, response, status, and artifact code receives native-equivalent paths.
  - Configure the worker’s external event sink before execution so reconstructed paths enable artifacts but do not let the worker write `events.jsonl`.
  - Serialize each event as an `EventFrame` through a shared mutex-protected stdout writer, writing one complete JSON line and flushing immediately.
  - Write the final `Result` frame through the same writer after execution; no event may follow it.

- Stream the container transport.
  - Add a streaming method to `ContainerCommandRunner` that supplies complete stdout lines while the child is running and returns exit status plus captured stderr.
  - Implement it with a spawned process, piped stdin/stdout/stderr, incremental buffered stdout reads, and concurrent stderr draining to prevent pipe deadlock.
  - In `execute_container`, parse each line immediately. Persist event frames through the runner’s host-ingestion method; retain the result frame until process exit.
  - Reject malformed frames, duplicate results, and frames after the result. Preserve useful stderr and exit-code diagnostics.
  - Keep ordinary buffered `run()` behavior for container startup and cleanup. No additional host-side event lock is needed beyond the runner’s existing append lock.

## Interface Changes

- `WorkerNodeRequest` gains explicit run-root metadata sufficient to reconstruct `RunRootPaths`; the fields remain optional/defaulted for protocol compatibility with older fixtures.
- `ContainerCommandRunner` gains a streaming execution operation used only for `docker exec`.
- `RuntimeHandlerRunner` gains:
  - configuration for an external raw-event sink;
  - a narrow method for host-owned event persistence and notification.
- `EventFrame` continues carrying the existing event type and payload; preserve the worker event’s run ID and other canonical metadata when converting between `RawRuntimeEvent` and the wire frame, extending the frame if necessary rather than regenerating metadata inconsistently on the host.

## Test Plan

- Worker protocol test: a runner producing lifecycle and Codergen events emits flushed event frames before exactly one terminal result frame.
- Parallel worker test: concurrent branch events remain intact, independently parseable JSON lines.
- Container transport test: fake Docker delivers delayed frames and proves the journal and observer update before process completion.
- Persistence test: streamed events receive ordered journal sequences exactly once and trigger the existing run-event observer.
- Artifact test: a containerized node writes the same expected node logs, trace/status/response files, and declared artifacts as native execution.
- Failure tests: malformed frames, duplicate results, post-result frames, missing result, callback/persistence failure, stderr output, and nonzero worker exit produce deterministic runtime errors without silently losing diagnostics.
- Regression test: native execution retains its current append, sequencing, observer, and artifact behavior.
- Run the focused `attractor-execution`, `attractor-runtime`, `attractor-api` execution-placement, and Spark server worker contract suites.

## Assumptions

- The run root remains bind-mounted at the identical host/container path.
- The host process is the only writer of `events.jsonl`; worker access to run paths is for per-node logs and artifacts.
- Container execution remains one host-side node at a time; parallel event production occurs only inside the worker.
- No journal migration or UI change is required—the activity pane already responds correctly when journal updates are persisted and the observer fires.
