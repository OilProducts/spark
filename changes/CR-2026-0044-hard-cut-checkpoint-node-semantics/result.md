---
id: CR-2026-0044-hard-cut-checkpoint-node-semantics
title: Hard-Cut Checkpoint Node Semantics
status: completed
type: bugfix
changelog: public
---

## Summary

Implemented the hard cutover from checkpoint `current_node` to explicit `active_node` and `last_completed_node` fields. Checkpoint persistence, resume, retry, pause, terminal handling, status/progress payloads, checkpoint events, and continuation paths now use the new checkpoint semantics while runtime/result `current_node` remains scoped to actual execution state.

## Validation

- `uv run pytest -q -x --maxfail=1 tests/engine/test_checkpointing.py tests/api/test_pipeline_checkpoint_endpoint.py tests/api/test_pipeline_status_endpoint.py`
- `uv run pytest -q`

## Shipped Changes

- Updated `Checkpoint` serialization/loading to emit `active_node` and `last_completed_node` without `current_node`.
- Updated executor checkpoint writes so initial, routed, retry, pause, failure, and terminal checkpoints record the next resumable node separately from the last processed node.
- Updated resume logic to execute `active_node` directly and treat checkpoints without an active node as terminal/non-resumable.
- Updated API/status/progress, child-run, retry, journal, checkpoint endpoint, and run-result surfaces that derive state from checkpoints.
- Updated the Attractor spec and TODO item to reflect the new checkpoint contract.
- Reworked engine, API, backend invariance, and integration tests around observable `active_node` and `last_completed_node` behavior.
