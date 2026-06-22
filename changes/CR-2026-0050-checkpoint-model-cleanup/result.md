---
id: CR-2026-0050-checkpoint-model-cleanup
title: Checkpoint Model Cleanup
status: completed
type: refactor
changelog: public
---

## Summary
Spark checkpoints now use `current_node` as the canonical persisted node field. The old checkpoint `active_node` and `last_completed_node` shape has been removed from checkpoint serialization, executor checkpoint events, API checkpoint progress payloads, result materialization, and active tests.

## Validation
- `uv run pytest -q`: 1918 passed, 26 skipped.
- Focused frontend validation passed: `npm --prefix frontend run test:unit -- src/lib/api/__tests__/attractorApi.test.ts src/features/runs/model/__tests__/runDetailsModel.test.ts`.
- Full frontend unit validation was attempted with `npm --prefix frontend run test:unit`; it is blocked by an unrelated existing `ReferenceError: asRunRecord is not defined` in `frontend/src/features/runs/hooks/useRunsList.ts`, with 1 failed file and 3 failed tests in `frontend/src/features/runs/__tests__/RunsPanel.test.tsx`.

## Shipped Changes
- Replaced the `Checkpoint` model with `current_node`, `completed_nodes`, `context`, `retry_counts`, `logs`, and `timestamp`.
- Updated executor checkpoint save/resume/finalize behavior so terminal checkpoints keep a non-empty `current_node`, post-stage pause checkpoints stay on the completed node, and resume advances past an already completed `current_node`.
- Updated API status/progress, checkpoint journal summaries, retry preparation, child-run snapshots, and result materialization to consume and emit `current_node`.
- Updated frontend API parsing and run checkpoint summaries to use `current_node` checkpoint payloads.
- Updated the Attractor spec to document canonical `current_node` checkpoint, resume, terminal, and event semantics.
- Rewrote backend, frontend, and smoke test fixtures away from the old checkpoint field names and added coverage for completed-node resume advancement.
