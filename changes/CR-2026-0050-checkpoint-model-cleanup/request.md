# Checkpoint Model Cleanup

## Summary
Replace Spark’s split checkpoint model (`active_node` + `last_completed_node`) with a single canonical `current_node` model, matching the clean Metalshop direction. This is a hard cutover: remove compatibility code and tests for the old checkpoint shape rather than preserving old run/checkpoint support.

## Key Changes
- Change `Checkpoint` to persist:
  - `current_node: str`
  - `completed_nodes`
  - `context`
  - `retry_counts`
  - `logs`
  - `timestamp`
  - optionally `next_node_visit_sequence` if the implementation also adds runner visit-index support in the same pass.
- Remove `active_node` and `last_completed_node` from checkpoint serialization, deserialization, executor events, API progress payloads, and tests.
- Update executor checkpoint semantics:
  - Save `current_node` as the node that is currently resumable from.
  - Terminal checkpoints keep `current_node` as the terminal/current result node, not `None`.
  - Pause after a node completion should pause at the completed node unless the pause happens before node execution.
  - Resume should read `checkpoint.current_node`; if that node is already completed, resolve the next edge before executing so the completed node is not re-run.
- Update API/status surfaces that currently expose checkpoint progress:
  - Use `current_node` and `completed_nodes`.
  - Remove `active_node` / `last_completed_node` fields from checkpoint-derived payloads.
  - Update event/journal copy from “active/terminal checkpoint” language to “checkpoint saved at current node”.
- Update result materialization code to use `checkpoint.current_node` wherever it currently uses `checkpoint.active_node`.

## Tests
- Rewrite checkpoint unit tests to assert `current_node` only; delete tests whose purpose is old `active_node=None` terminal behavior.
- Add/keep coverage for:
  - initial checkpoint stores the start node as `current_node`.
  - in-progress checkpoint stores the next resumable node.
  - terminal completed/failed checkpoint still has a non-empty `current_node`.
  - pause after a completed stage records the completed stage as `current_node`.
  - resume from a checkpoint whose `current_node` is already in `completed_nodes` advances to the routed next node rather than re-running it.
  - API status/progress/checkpoint endpoints no longer emit `active_node` or `last_completed_node`.
  - continuation/retry code derives its start node from `current_node`.
- Run `uv run pytest -q`.

## Assumptions
- This is a hard cutover. Existing persisted checkpoints using `active_node` / `last_completed_node` do not need migration compatibility.
- Run records may keep unrelated fields like `current_node` in result/status payloads, but checkpoint-derived public payloads should not expose the old names.
- Do not bundle unrelated Metalshop ports such as failure result materialization, execution capability matching, graph attr seeding, or manager-loop child invocation metadata into this change.
