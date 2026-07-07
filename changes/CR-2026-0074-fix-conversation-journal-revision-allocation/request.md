# Fix Conversation Journal Revision Allocation

## Summary
Fix missed live `request_user_input` cards by separating stable transcript placement from durable journal revisions. Keep existing `turn.id`, `segment.id`, and `segment.order` behavior unchanged. Move revision assignment for persisted conversation journal payloads to the final repository persistence boundary so each journaled `turn_upsert`, `segment_upsert`, and `conversation_snapshot` gets a monotonically increasing revision based on the latest committed conversation state.

## Key Changes
- Add a small repository-level batch persistence method for conversation snapshots and journal payloads.
  - It reads the latest persisted conversation revision immediately before writing.
  - It stamps only persisted conversation journal payloads with `latest_revision + 1..N`.
  - It writes the snapshot with the final batch revision.
  - It appends the stamped journal payloads after the snapshot write.
- Replace workspace-local revision stamping in conversation persistence paths with the repository batch method.
  - Keep raw backend/provider streaming events outside this revision system unless they are materialized into a persisted conversation journal payload.
  - Keep segment ids and `segment.order` assigned at materialization time.
  - Keep existing snapshot/event wire shapes unchanged.
  - Do not introduce a new schema version or migration.
- Preserve late updates to earlier transcript items.
  - A later journal event may update an older `turn.id` or `segment.id` with an older `segment.order`.
  - That late update still receives the next newest journal revision.
  - The frontend accepts it by journal revision, then renders it at its stable transcript position by turn order and `segment.order`.
- Make stale in-memory live-turn snapshots safe for journal revisions.
  - A live turn may still hold an old snapshot while request-input answer handling writes a newer one.
  - Its later persisted journal batch must be rebased to the latest committed revision before persistence.
- Add a minimal frontend safety net only if needed after backend tests:
  - On a stale `request_user_input` journal event, trigger a snapshot refresh instead of silently doing nothing.
  - This is defensive recovery, not the primary fix.

## Test Plan
- Add a backend regression test around the existing live request-input flow:
  - Start a turn that emits a pending `request_user_input`.
  - Answer it while the original assistant turn remains live.
  - Have the resumed live turn emit a second pending `request_user_input`.
  - Assert all persisted conversation journal events have strictly increasing revisions.
  - Assert `read_conversation_events_after` from the first answer revision returns the second pending request.
  - Assert the final snapshot contains both the answered first request and pending second request.
- Add a late-update ordering test:
  - Persist a running `tool_call` segment.
  - Persist later assistant text segments with higher `segment.order`.
  - Complete the original tool call afterward.
  - Assert the completion event gets the newest journal revision while keeping the original tool-call `segment.order`.
- Extend repository/storage tests for batch persistence:
  - Given a stale snapshot revision, persist a new journal batch after a newer snapshot already exists.
  - Assert stamped journal event revisions start after the latest persisted revision.
  - Assert snapshot revision equals the last stamped journal event revision.
- Keep existing frontend request-card tests unchanged unless the defensive refresh is added.
- Run the full validation gate before completion:
  - `cargo fmt --all -- --check`
  - `cargo test --workspace --all-features`
  - `npm --prefix frontend run test:unit`
  - `npm --prefix frontend run build`

## Assumptions
- Conversation `revision` is only a durable journal replay/cache cursor, not a transcript placement field.
- Turn and segment placement remain governed by stable ids plus `segment.order`.
- Raw backend/provider stream events do not receive conversation revisions unless materialized into persisted conversation journal payloads.
- Existing broken event logs do not need migration; reloading the current snapshot is enough to recover visible state for affected conversations.
