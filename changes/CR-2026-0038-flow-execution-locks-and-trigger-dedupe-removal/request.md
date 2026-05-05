# Flow Execution Locks and Trigger Dedupe Removal

## Summary
Add a workspace-managed execution-lock system for serializing runs that mutate shared resources, and remove the current trigger dedupe behavior entirely. Execution locks live in the flow catalog/config layer, not DOT, so they describe workspace launch admission policy rather than flow semantics.

## Key Changes
- Extend `flow-catalog.toml` per-flow entries with optional `execution_lock` config:
  - `scope`: initially `project`
  - `key`: named shared resource, such as `main-worktree-integration`
  - `conflict_policy`: initially `queue`
- Multiple flows may declare the same lock key. Spark resolves the runtime lock identity as `scope + project_id + key`; only one active run may hold that identity.
- On launch:
  - if the lock is free, launch normally and mark the run as holder
  - if held, persist the full attempted launch payload into that lock’s FIFO queue
  - when the holder reaches terminal status, release the lock and launch the next queued attempt
- Queued launch payloads must preserve the full initialization surface: flow, project/working directory, launch context, model/provider/profile/reasoning, execution profile, goal, spec/plan ids, and source metadata where available.
- Remove trigger dedupe state and behavior:
  - remove `dedupe_keys`, `seen_item_ids`, request-id suppression, and “trigger already running” skip logic
  - keep trigger recent history for actual launch success/failure records, but no dedupe key field

## UI Exposure
- Add execution-lock controls next to Launch Policy in Graph Settings, backed by the workspace flow catalog:
  - lock disabled/enabled
  - scope display/select, initially project
  - lock key text field
  - conflict policy display/select, initially queue
  - note that this is workspace config, not DOT
- Show lock metadata during manual launch when the selected flow has a lock.
- Show lock state in Runs:
  - active holder: “Holding execution lock”
  - queued attempts grouped by lock identity, not by flow

## Test Plan
- Backend tests:
  - flow catalog reads/writes launch policy plus execution lock config
  - locked flow launches immediately when no holder exists
  - second launch with same project/key is queued, not started
  - terminal holder completion starts the next queued payload FIFO
  - different flows sharing the same key serialize behind one lock
  - different projects with the same key do not block each other
  - trigger runtime no longer suppresses webhook request ids, poll item ids, flow-event run ids, or concurrent trigger firings
- Frontend tests:
  - Graph Settings loads/saves execution lock config
  - manual launch surface shows configured lock metadata
  - Runs UI renders lock holder and queued attempts
- Run full suite with `uv run pytest -q`.

## Assumptions
- V1 supports only `scope="project"` and `conflict_policy="queue"`.
- Execution lock config is operational workspace config, so it belongs in `flow-catalog.toml`, not DOT.
- FIFO queueing is required; blocked launches are not coalesced.
- Manual, trigger, agent, and approved-conversation launches all use the same admission path and lock behavior.
