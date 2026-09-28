---
id: CR-2026-0132-per-execution-run-transcripts
title: Keep each node execution's transcript separate in the Runs view
status: completed
type: fix
changelog: public
---

## Summary

The Runs view Activity tab now shows one transcript group per node execution. A node that runs more than once (a review loop, a revisit) gets one group per visit, labeled `evaluate — visit 1`, `evaluate — visit 2`, and so on. Groups and run events are ordered by time, and live updates for a later visit no longer overwrite an earlier visit. Nodes that run once keep their existing header. The backend's fixed `"response"` turn id and per-execution segment ids are unchanged, because the frontend now builds identity from the execution.

## Validation

- `transcriptModel.test.ts`: two executions of one node that differ only in `stage_index`, with the same turn and segment ids, produce two groups in visit order with visit labels. A root and a child run with matching node ids produce two unlabeled groups.
- The same file tests `runTranscriptStore.applySegmentUpsert`: a live upsert for the second visit updates only that visit and leaves the first intact. A child-run upsert leaves the matching root segment intact.
- `RunActivityCard.test.tsx`: transcript groups and journal events interleave in time order, even though the execution-local sequence numbers are larger than the journal's.
- Browser check: a new smoke test, `run activity shows one labeled transcript group per visit...` in `e2e/smoke/runs-observability.spec.ts`, uses a fixture with `evaluate` visited at stage 1 and stage 3. It checks for two labeled groups interleaved with stage events, both before and after a live `conversation.segment_upsert` for the second visit. The smoke test ran after `npm --prefix frontend run build`; all 7 tests in `runs-observability` passed.
- `just test`: passed.

## Shipped Changes

- `RunTranscriptSegment` carries `stage_index` from both the fetch path (`fetchExecutionSegments`, `parseRunTranscriptSegment`) and the live path (`spark:run-segment-upsert` in `AppSessionControllers.tsx`).
- `transcriptModel.ts` adds `runTranscriptExecutionKey` (run id, node id, `stage_index`, attempt) and `runTranscriptSegmentKey`. It groups by execution plus turn id, computes a visit number for nodes with more than one execution, and records each group's latest segment time.
- `runTranscriptStore.applySegmentUpsert` matches segments by execution plus segment id.
- `RunActivityCard.tsx` orders rows by timestamp (segment time for groups, emitted time for events), keeps journal sequence as the tie-break between events, and keys groups by their execution identity.
- `RunTranscriptGroups.tsx` adds a `data-stage-index` attribute to each group.
