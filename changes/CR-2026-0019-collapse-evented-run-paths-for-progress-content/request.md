# Collapse Evented Run Paths for Progress Content

## Summary
Simplify progress propagation by removing the separate `run_with_events(...)` execution shape. The canonical path should be `run(..., emit_event=None, ...)`; when an event sink is present, LLM progress becomes persisted `LLMContent`, and when absent, execution still works without progress emission. This removes the path mismatch that caused the Runs progress card to sometimes show no content.

## Key Changes
- Update the codergen backend contract so `CodergenBackend.run(...)` accepts optional `emit_event`.
- Remove production `run_with_events(...)` methods from codergen-related backends:
  - Codex app-server backend
  - Unified agent backend
  - Provider router backend
- Remove thread-local progress plumbing such as `_progress_event_state`; pass `emit_event` directly down to Codex turn handling, unified agent session handling, and contract repair calls.
- Change `CodergenHandler` to call `self.backend.run(..., emit_event=runtime.event_emitter, ...)`.
- Simplify runner plumbing:
  - `HandlerRunner` exposes one canonical run path with optional `emit_event`.
  - `BroadcastingRunner` passes `emit_event` through the same canonical path.
  - `PipelineExecutor` should use the canonical event-capable runner path for Spark/Attractor runners.
- Keep plain callable runner support only where it is already a generic engine contract; do not preserve `run_with_events` as a compatibility shim.

## Test Cleanup
- Remove fake test backend `run_with_events(...)` methods that only delegate to `run(...)`.
- Rewrite direct backend progress tests to call `backend.run(..., emit_event=collector, ...)`.
- Remove tests that assert or imply `run_with_events` is a public or preferred interface.
- Keep behavioral coverage that matters:
  - Codex app-server stream events become `LLMContent`.
  - Unified agent session events become `LLMContent`.
  - A pipeline/API execution with an event sink persists `LLMContent` for the Runs progress card.
  - Existing non-progress pipeline tests still pass with simple `run(...)` fakes.

## Acceptance Criteria
- No production references to `run_with_events`.
- No tests define fake `run_with_events` methods just to satisfy the old shape.
- No thread-local progress event state remains in codergen backends.
- Runs progress content is emitted through the same backend call path that produces the node outcome.
- Full suite passes with `uv run pytest -q`.

## Assumptions
- `run_with_events` is not a supported public API we need to preserve.
- Plain callable runner support remains part of the generic engine surface, but it is not the Spark progress-monitoring path.
- The frontend contract stays unchanged: the progress card renders persisted `LLMContent` events from the run timeline.
