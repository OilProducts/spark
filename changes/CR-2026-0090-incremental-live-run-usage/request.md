# Incremental Live Run Usage

## Summary

Replace repeated full-journal usage scans during agent execution with an incremental server-side accumulator. Each usage report will update the current aggregate in O(1), and the existing `run.upsert` live envelope will carry the new token and cost totals immediately. Durable `run.json` updates remain at stage and terminal boundaries.

## Implementation Changes

- Add a reusable `RunUsageAccumulator` beside the existing usage projection code.
  - Track completed request totals by model.
  - Track the latest cumulative in-flight snapshot by node, replacing older snapshots instead of adding them.
  - Retain legacy `LLMTokenUsage` totals only as the existing fallback when no completed event contains usage.
  - Make `project_run_usage` use the same reducer so incremental and full replay results cannot diverge.

- Move live aggregation into the run-event publisher.
  - Extend each live publication result to expose the journal entries already read for that cursor; do not reread the journal.
  - Maintain one accumulator per active run in the publisher loop and apply only newly published entries.
  - On the first publication, initialize from the complete available journal window. If the bounded cache cannot provide history from sequence zero, perform one full journal projection to seed the accumulator, then continue incrementally.
  - Evict accumulator state when the run becomes terminal, alongside the existing live-cache eviction.

- Publish immediate usage through the existing `run.upsert` contract.
  - Overlay the accumulator’s `token_usage`, `token_usage_breakdown`, and `estimated_model_cost` onto the full run record used for that upsert.
  - Keep the frontend unchanged; its existing upsert merge already updates both run-list and selected-run usage fields.
  - Remove the five-second Codergen handler scan/write path and its `usage_record_writes` state.
  - Keep stage, failure, pause, and terminal `refresh_record_usage` calls as durable reconciliation points and restart-safe sources of truth.

## Interface Changes

- `RunLivePublication` gains the newly read journal entries used during that publication cycle.
- Add an internal run-upsert helper that accepts an optional usage overlay; the public SSE envelope remains `run.upsert` with the existing run-record shape.
- No persisted schema, API response, frontend type, migration, or new live-event type is introduced.

## Test Plan

- Verify incremental accumulation exactly matches `project_run_usage` for:
  - repeated cumulative in-flight snapshots;
  - multiple nodes and models;
  - request completion replacing an in-flight snapshot without double-counting;
  - multiple completed requests;
  - legacy token events and mixed modern/legacy journals.
- Verify a usage-bearing journal event produces an updated `run.upsert` immediately while `run.json` remains unchanged mid-stage.
- Verify successive publisher cycles process only new entries and do not rescan or double-count earlier usage.
- Verify an incomplete bounded window performs one full initialization and subsequent events remain incremental.
- Verify reconnect/restart initialization yields the same live totals as a full journal projection.
- Verify stage and terminal writes still persist final token totals and estimated costs.
- Run focused runtime usage, workspace live-publication, HTTP SSE, and Runs UI upsert tests, followed by the full test suite.

## Assumptions

- Live usage is a transient overlay; durability is required only at the existing stage and terminal boundaries.
- Provider usage reports retain their current semantics: session reports are cumulative snapshots and request-completed reports are authoritative per-request totals.
- The server remains the canonical accounting implementation; the frontend only renders the existing run fields.
