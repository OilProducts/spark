---
id: CR-2026-0039-flow-run-results
title: Flow Run Results
status: completed
type: feature
changelog: public
---

## Summary

Delivered a first-class flow run result surface. Completed runs can now expose a selected or inferred source node response as `result/result.json` and `result/result.md`, optionally summarize that response, and serve the result through `GET /attractor/pipelines/{pipeline_id}/result`.

## Validation

- `uv run pytest -q` passed with 1976 tests passing, 26 skipped, and 2 warnings.
- Added backend coverage for explicit result node selection, fallback source inference, raw and summarized results, unavailable states, and invalid explicit sources.
- Added frontend coverage for result card states, conversation launch result access, and graph attribute contracts.

## Shipped Changes

- Backend result materialization and API response handling were added under `src/attractor/api`, including pending, unavailable, raw, summary, and error metadata behavior.
- Run details, result fetching, and result display were wired into the frontend Runs view and conversation flow-launch artifacts.
- Graph settings now expose result node selection, summary enablement, and an optional summary prompt, with the new graph attributes included in validation and store types.
- DOT authoring guidance and Spark flow extension specs now document `spark.result_node`, `spark.result_summary_enabled`, and `spark.result_summary_prompt`.
- Tests were added or updated across API, frontend component behavior, conversation history behavior, and graph attribute contracts.
