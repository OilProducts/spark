# Hard-Cut Checkpoint Node Semantics

## Summary
Replace checkpoint `current_node` with explicit checkpoint fields so node state has one meaning everywhere. Runtime/context `current_node` will continue to mean the node currently executing. Checkpoints will no longer serialize or consume `current_node`; they will use `active_node` and `last_completed_node` instead. Existing tests that assert checkpoint `current_node` will be rewritten or removed.

## Key Changes
- Change `Checkpoint` to serialize:
  - `active_node`: node to execute if this checkpoint is resumed; `null` when no further execution is pending.
  - `last_completed_node`: most recent completed node, or `null` before any node completes.
  - `completed_nodes`, `context`, `retry_counts`, `logs`, `timestamp` unchanged.
- Update executor checkpoint writes:
  - Initial/launch checkpoint: `active_node=<start node>`, `last_completed_node=null`, `completed_nodes=[]`.
  - Successful node transition: after routing is known, save `last_completed_node=<node just completed>` and `active_node=<next node>`.
  - Terminal completion/failure: save `last_completed_node=<final processed node>`, `active_node=null`.
  - Retry/pause/goal-gate recovery checkpoints: save `active_node=<node to execute next>` and preserve `last_completed_node` from completed state.
- Update resume logic to use `active_node` directly. Remove the old heuristic that checks whether `checkpoint.current_node in completed_nodes`.
- Update checkpoint-derived API/progress surfaces to expose `active_node` and `last_completed_node` instead of checkpoint `current_node`. Keep runtime/context/result `current_node` only where it means actual current/final execution point, not checkpoint state.
- Update Attractor spec:
  - Context `current_node` remains “currently executing node.”
  - Checkpoint schema uses `active_node` / `last_completed_node`.
  - Resume behavior starts from `active_node`; if it is absent, the checkpoint is terminal/non-resumable.
- Remove or rewrite tests that encode the old checkpoint `current_node` contract, including initial checkpoint, retry checkpoint, status/progress, checkpoint endpoint, continuation, and checkpoint serialization tests.
- Mark the TODO item complete once implementation and spec updates land.

## Test Plan
- Add/adjust checkpoint model tests for serialization and loading with `active_node` and `last_completed_node`; do not keep old `current_node` expectations.
- Add/adjust executor tests for:
  - initial checkpoint has `active_node=start`, no last completed node.
  - normal stage checkpoint records the next active node and last completed node.
  - retry checkpoint records the retried node as active without marking it completed.
  - terminal checkpoint has no active node and records the final completed node.
  - resume executes `active_node` directly without membership heuristics.
- Update API tests for `/checkpoint`, pipeline status/progress, retry, continue, child-run status, and integration smoke expectations.
- Run:
  - `uv run pytest -q -x --maxfail=1 tests/engine/test_checkpointing.py tests/api/test_pipeline_checkpoint_endpoint.py tests/api/test_pipeline_status_endpoint.py`
  - `uv run pytest -q`

## Assumptions
- This is a hard cutover: no fallback loader for old checkpoint JSON and no compatibility aliases in checkpoint API responses.
- Existing persisted runs/checkpoints may be invalid after the change; that is acceptable because there are no in-progress runs and compatibility is explicitly out of scope.
- `current_node` remains valid only for live execution context and result objects, where it means current/final execution point rather than checkpoint resume state.
