---
id: CR-2026-0019-collapse-evented-run-paths-for-progress-content
title: Collapse Evented Run Paths for Progress Content
status: completed
type: bugfix
changelog: public
---

## Summary
Collapsed codergen progress propagation onto the canonical `run(..., emit_event=...)` path. Backends now receive an optional event sink directly through `run`, so LLM progress content is emitted through the same backend call that produces the node outcome and can be persisted for the Runs progress card.

## Validation
- `uv run python -m compileall -q src tests`
- `uv run pytest -q -x --maxfail=1 tests/api/test_backend_invariance.py tests/handlers/test_wait_human_handler.py tests/handlers/test_manager_loop_handler.py`
- `uv run pytest -q -x --maxfail=1 tests/engine/test_executor.py tests/api/test_pipeline_events_endpoint.py tests/api/test_manager_loop_pipeline_api.py tests/api/test_pipeline_retry_endpoint.py tests/integration/test_integration_smoke_pipeline.py tests/integration/test_cross_feature_matrix.py`
- `uv run pytest -q -x --maxfail=1 tests/api/test_pipeline_events_endpoint.py::test_pipeline_api_persists_llm_content_emitted_from_canonical_run_path`
- `uv run pytest -q` completed with `1725 passed, 26 skipped`.
- Repository search found no remaining `run_with_events` or `_progress_event_state` references under `src` or `tests`.

## Shipped Changes
- Updated codergen backend contracts and implementations to accept `emit_event` on `run`.
- Removed production `run_with_events(...)` methods and thread-local progress plumbing from codergen backend paths.
- Updated `CodergenHandler`, `HandlerRunner`, `BroadcastingRunner`, and `PipelineExecutor` to pass optional event emitters through the canonical run path.
- Cleaned up test fakes and backend progress tests so they no longer define or call the removed `run_with_events` shape.
- Added API/pipeline coverage proving `LLMContent` emitted from backend `run(..., emit_event=...)` is persisted and readable through the run journal used by timeline/progress UI surfaces.
