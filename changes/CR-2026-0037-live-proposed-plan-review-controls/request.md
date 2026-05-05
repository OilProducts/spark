# Live Proposed Plan Review Controls

## Summary
Fix the refresh-only Approve/Disapprove behavior by sending the durable artifact record with the live artifact-backed segment update. The plan card should become reviewable as soon as the completed plan segment arrives, without depending on a follow-up full snapshot.

## Key Changes
- Extend `segment_upsert` stream events to optionally include artifact sidecars for the segment being updated:
  - plan segment: matching `proposed_plans`
  - flow run request segment: matching `flow_run_requests`
  - flow launch segment: matching `flow_launches`
- Update backend segment event construction to attach only the matching artifact record when the segment has `artifact_id`.
  - Leave non-artifact segment events unchanged.
  - Leave full conversation snapshots unchanged.
- Update frontend stream parsing/types and cache merge logic.
  - Merge optional artifact sidecars into normalized artifact maps/lists.
  - Rebuild the affected turn timeline after merging the segment.
  - Preserve stale-event handling and pre-snapshot replay behavior.
- Keep snapshot refresh as a recovery fallback, but stop relying on it as the main path for showing plan review controls.

## Test Plan
- Replace the workaround-shaped project panel test:
  - Remove the test that expects a second snapshot fetch before plan review buttons appear.
  - Add a live-stream test where completed plan `segment_upsert` includes `proposed_plans`, and Approve/Disapprove appear immediately.
- Trim the pre-snapshot replay test so it only verifies event replay after the initial snapshot.
  - Do not require artifact review buttons to come from a refreshed snapshot.
- Add reducer/parser coverage:
  - `segment_upsert` with `proposed_plans` populates `proposedPlansById` and keeps the plan timeline row linked to the artifact.
  - Optional `flow_run_requests` and `flow_launches` sidecars parse and merge consistently.
- Keep existing snapshot artifact hydration and component rendering tests, because snapshots remain valid recovery/reload sources.
- Run focused frontend tests plus `uv run pytest -q`.

## Assumptions
- The durable artifact record remains the source of truth for review actions; the frontend must not synthesize proposed plan records from segment text.
- A segment event should include only artifact records needed to render that segment, not a full artifact snapshot.
- Existing full snapshot refresh behavior may remain as fallback, but tests should not assert it as the primary success path.
