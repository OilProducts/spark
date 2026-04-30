# Conversation Revision Cutover

## Summary
Replace snapshot freshness scoring with an explicit monotonic `revision` contract for project chat conversations. Use a hard schema bump: persisted conversation files without `revision` are unsupported and should fail with the existing “delete/recreate conversation” style error. Remove the interim record/snapshot scoring helpers instead of building on them.

## Key Changes
- Backend conversation state:
  - Bump `CONVERSATION_STATE_SCHEMA_VERSION` from `4` to `5`.
  - Add required `revision: int` to `ConversationState`.
  - `ConversationState.from_dict()` must require schema `5` and a numeric `revision`.
  - `ConversationState.to_dict()` and repository `write_state()` must persist `revision`.

- Revision semantics:
  - New conversations start at `revision = 0`.
  - Every successful conversation mutation that currently calls `touch_conversation_state()` increments revision exactly once.
  - `updated_at` remains display/sort metadata only; it is no longer used for stale snapshot acceptance.
  - Pure reads must not increment revision.
  - `write_state()` must not increment revision by itself; callers mutate state, call `touch_conversation_state()`, then write.

- API/event contract:
  - Add `revision` to conversation snapshots, conversation summaries if needed by existing frontend sorting/cache code, `turn_upsert`, and `segment_upsert` event payloads.
  - Event payloads should use the state revision at the time the payload is built.
  - Snapshot publish paths should include the current persisted revision.

- Frontend cache:
  - Add `revision` to `ConversationSnapshotResponse`, stream event response types, and `NormalizedConversationRecord`.
  - Replace `scoreConversationSnapshotFreshness`, `scoreConversationRecordFreshness`, and related helpers with direct revision comparison.
  - Snapshot apply rule: accept if incoming `revision > cached.revision`; ignore if `revision <= cached.revision`.
  - Stream event rule: apply if `event.revision >= cached.revision` for same-turn/segment incremental updates, and update record revision to `event.revision`; ignore events below cached revision.
  - Remove `conversationRecordToSnapshot()` and any remaining freshness scoring code.

## Test Plan
- Backend API tests:
  - New conversation snapshots include `schema_version: 5` and `revision`.
  - Starting a turn increments revision and all emitted turn/segment events include that revision.
  - Request-user-input answer, settings update, flow request review, proposed plan review, and flow launch artifact updates increment revision once per persisted mutation.
  - Schema `4` conversation state files are rejected.

- Frontend unit tests:
  - Same-`updated_at` snapshots are ordered by `revision`, not event log/content/status scoring.
  - Lower/equal revision snapshots are ignored.
  - Higher revision snapshots replace cached records.
  - Stream events below cached revision are ignored; current/higher revision events update normalized turn/segment state.

- Verification:
  - Run focused frontend state tests.
  - Run `npm run build`.
  - Run full suite with `uv run pytest -q`.

## Assumptions
- Hard schema bump is intentional; no compatibility or migration path for existing local conversation files.
- `revision` is a conversation-level ordering contract, not a per-turn or per-segment sequence.
- The small direct-comparison patch already made should be replaced by this revision-based design, not preserved as a stepping stone.
