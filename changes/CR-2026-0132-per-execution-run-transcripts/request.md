# Keep each node execution's transcript separate in the Runs view

## Summary

When a flow visits a node more than once (a review loop, a retry, a revisit), the Activity tab merges every visit's transcript into one group, orders it meaninglessly against run events, and lets later visits overwrite earlier ones while the run is live. Give each node execution its own identity in the frontend transcript model so every visit shows as its own group, in time order, with nothing overwritten.

This is the first step of the Runs tab redesign (`reports/runs-tab-redesign/`); the execution ledger and report header follow in their own requests.

## Why

In `run-18d93c056079b9c0`, Implement and Evaluate each ran 4 times and Validate ran twice. The Activity tab shows all 12 executions under one `RESULT_SUMMARY` header. Causes:

- Every execution transcript uses `"turn_id": "response"` (`crates/attractor-runtime/src/handlers.rs`, the transcript rebuild near line 123 and the final-response record near line 1176). Segment ids such as `final-response` are unique only within one execution.
- `transcriptModel.ts` groups segments by `turn_id` alone (near line 99), so all executions of all nodes merge.
- `fetchExecutionSegments` (`frontend/src/lib/api/attractorApi.ts`) keeps `attempt`, which is 0 on every loop revisit, and drops `stage_index`, the visit number. The live path (`AppSessionControllers.tsx`, `spark:run-segment-upsert`) drops it too, although the `conversation.segment_upsert` payload carries it (`crates/spark-workspace/src/live.rs`).
- `RunActivityCard.tsx` sorts transcript groups by `latest_sequence`, an execution's own event number, against run-level event sequence numbers (near line 215).
- `applySegmentUpsert` (`runTranscriptStore.ts`) matches by segment id alone, so a later visit's `final-response` replaces an earlier visit's.

## What exists

An execution is already identified by run id, node id, `stage_index`, and `attempt`: the transcript route is `/pipelines/{run}/executions/{node}/{stage_index}-{attempt}/transcript`, and both the pipeline detail and live envelopes carry all four. Segments carry timestamps.

## Change

- Carry `stage_index` on `RunTranscriptSegment`, from both the fetch path and the live path.
- Identify a segment by its execution (run id, node id, `stage_index`, `attempt`) plus its segment id, and a transcript group by its execution plus turn id. Use this in grouping (`transcriptModel.ts`) and in the live store's upsert match, so executions never merge or overwrite each other, root and child runs included.
- Order Activity rows on one comparable key across transcript groups and run events (for example their timestamps), not on sequence numbers from different streams.
- Label a group for a repeated node with its visit, so the four Evaluate groups are distinguishable. Keep the existing header layout otherwise.
- If the backend's fixed `"response"` turn id or per-execution segment ids stop being needed as keys once the frontend composes identity, leave them; change backend ids only if a consumer still requires globally unique ids.

## Verification

- Unit tests (`transcriptModel`, `runTranscriptStore`): two executions of the same node with the same turn and segment ids, differing only in `stage_index`, produce two groups, and a live upsert for the second leaves the first intact; same for a root and a child run with matching node ids.
- Activity ordering: transcript groups and run events interleave in time order across executions.
- Browser check against a run with a loop (for example `run-18d93c056079b9c0` if it is still present, or a fixture with a repeated node): one group per visit, labeled, in order, before and after a live update.
- `just test`.

## Out of scope

- The execution ledger, trace, and report header proposals, visit counts on the graph, showing the designed Fix loop as something other than "Stage failed", and child-run transcripts under Implement.

## Hand-off

Write `changes/CR-2026-0132-per-execution-run-transcripts/result.md` (frontmatter with id, title, status, type, changelog; sections Summary, Validation, Shipped Changes) when the work is complete.
