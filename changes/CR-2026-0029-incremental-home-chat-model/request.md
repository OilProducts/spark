# Incremental Home Chat Model

## Summary
Replace the Home project chat cache’s snapshot-shaped active state with a normalized, incremental conversation model. Initial loads and API responses still accept `ConversationSnapshotResponse`, but the Home UI stores conversations as records keyed by id, applies SSE updates by turn/segment id, and maintains ordered timeline row ids so streaming updates patch only the affected conversation block. Remove the timeline object stabilization layer and its identity tests.

## Key Changes
- Change `HomeConversationCacheState` from `snapshotsByConversationId` to `conversationsById`, where each conversation record stores metadata, ordered turn ids, turns by id, segments grouped by turn, artifact maps/order, event log, and ordered timeline entry ids/data.
- Add model functions to hydrate/replace a conversation from a full snapshot, apply `turn_upsert`, apply `segment_upsert`, rebuild only the affected turn’s timeline entries, and update summaries from normalized conversation state.
- Keep backend/API wire types unchanged. Snapshot responses from load/send/settings/review/request-input/artifact-refresh become normalized records immediately; SSE events mutate normalized records directly.
- Update Home controllers/view model to read `activeConversationRecord` instead of `activeConversationSnapshot`, pass `activeConversationHistory` from the record’s ordered timeline entries, and remove `stabilizeConversationTimelineEntries`, `previousConversationHistoryScopeRef`, and schema-wide rendered-field equality.
- Preserve existing scroll behavior using the existing narrow `conversationHistoryRevisionKey`, now computed from normalized timeline entries. Do not add virtualization, stream coalescing, or backend changes in this refactor.
- Update project cleanup/rename/delete/select paths to operate on `conversationsById`. Selecting a cached thread should activate the existing normalized record without reapplying a snapshot-shaped cache entry.

## Data Flow
- On conversation open/reconnect: fetch full snapshot, normalize it, and replace the cached record only if it is fresher than the current record.
- On `turn_upsert`: upsert the turn by id, update conversation metadata, insert the turn into ordered turn ids if new, rebuild timeline entries for that one turn, and update the project summary.
- On `segment_upsert`: upsert the segment by id, update that turn’s ordered segment ids, rebuild timeline entries for that segment’s turn, and update the project summary.
- Artifact segment events keep the current refresh behavior: trigger a full snapshot fetch so artifact maps are refreshed from authoritative backend state.
- Timeline separator ids should be deterministic, for example based on the turn id and first final-message/plan segment id, so row keys remain stable across affected-turn rebuilds.

## Test Plan
- Replace `conversationTimeline` stabilization/object-identity tests with normalized model tests for snapshot hydration, turn upsert append/update, segment upsert streaming content update, mode-change chat mode update, artifact snapshot refresh hydration, stale snapshot rejection, and project rename/delete cache behavior.
- Keep timeline rendering tests focused on observable UI: streaming markdown updates, request-user-input status updates, artifact rows render from refreshed artifacts, and scroll pinning follows live-edge updates without pulling the user when scrolled away.
- Remove tests whose only assertion is object identity or render-count containment from the old stabilizer design.
- Run targeted frontend tests for projects/home chat, then the required full suite with `uv run pytest -q`.

## Assumptions
- “Full replacement” means no snapshot-shaped Home chat cache remains; snapshots are accepted only at API boundaries and immediately normalized.
- Backend conversation snapshot and SSE payload schemas remain unchanged.
- The full snapshot remains the reconciliation source after reconnects, reviews, settings changes, request-user-input submits, and artifact refreshes.
