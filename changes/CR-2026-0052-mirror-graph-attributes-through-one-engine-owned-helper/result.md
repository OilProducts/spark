---
id: CR-2026-0052-mirror-graph-attributes-through-one-engine-owned-helper
title: Mirror Graph Attributes Through One Engine-Owned Helper
status: completed
type: bugfix
changelog: public
---

## Summary
Direct `PipelineExecutor` runs now seed the same `graph.*` context namespace as API-launched runs. Graph attribute mirroring is centralized in `attractor.graph_prep.graph_attr_context_seed(graph)`, preserving existing defaults for missing `graph.goal` and normalized integer `graph.default_max_retries`.

## Validation
- `uv run pytest -q`
- Result: `1919 passed, 26 skipped in 28.66s`

## Shipped Changes
- Added the shared `graph_attr_context_seed(graph)` helper in `src/attractor/graph_prep.py`.
- Updated the API run bootstrap in `src/attractor/api/pipeline_runs.py` to import and use the shared helper instead of owning a duplicate implementation.
- Updated `PipelineExecutor._mirror_graph_attrs(...)` in `src/attractor/engine/executor.py` to apply the shared graph attribute seed to engine contexts.
- Added direct engine coverage in `tests/engine/test_executor.py` for arbitrary graph attributes, missing `graph.goal`, and typed `graph.default_max_retries`.
- Marked the matching `graph.*` TODO item complete in `TODO.md`.
