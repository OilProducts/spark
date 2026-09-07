# Prevent live-event replay and recovery storms

## Summary

Fix the confirmed replay vulnerability and overlapping recovery requests. Success means repeated workflow updates cannot replay previously published history, and recovery traffic cannot build a backlog that blocks project switching.

Keep existing chat and navigation behavior. Do not repair stored workflow statuses in this patch.

## Implementation

- **Preserve publication progress.** In `crates/spark-http/src/lib.rs`, update each run’s last published sequence independently of its usage accumulator. Evict terminal-run usage data as today, but retain the sequence for the publisher’s lifetime. Preserve the previous sequence when publication fails or returns no sequence. This deliberately retains one small cursor per observed run until shutdown.
- **Unify run-list fetching.** In `frontend/src/features/runs/hooks/useRunsList.ts`, route initial loading, reconnects, manual refreshes, and recovery notices through the same request path. Allow one active request per scope; notices received during it set one pending-refresh flag. After successful completion, perform at most one trailing refresh, using the same rule for subsequent notices.
- **Isolate scopes.** Ignore recovery notices for other projects. Abort the old request and discard its pending refresh when scope changes or synchronization stops. Guard response and cleanup writes by request identity so an old request cannot overwrite the new project’s state.
- **Handle failure without another loop.** On request failure, retain the existing error presentation and clear the pending refresh. Retry only on a subsequent recovery notice or explicit reconnect. Intentional cancellation must not show an error.
- Add an optional `AbortSignal` to the existing run-list API helper. No HTTP endpoints, event schemas, dependencies, buffer sizes, or durable formats change.

## Verification

- Extend publisher tests to reproduce a run marked `failed` that continues producing updates. Publish more than 256 historical events once; repeated notifications must not republish them, and a subsequent new event must appear once.
- Verify terminal usage eviction still works and publication errors do not reset the cursor.
- Add frontend tests with deferred responses: a burst of recovery notices produces one active request and one trailing refresh; failures do not retry automatically.
- Switch projects during recovery and verify cancellation, rejection of stale responses, and successful loading of the new scope. Verify unrelated-project notices cause no fetch.
- Run the affected Rust and frontend suites, frontend type checking, and the existing project-switching checks.

## Assumptions and rollout

- The captured overflow’s exact publisher batches remain uninstrumented; the regression must reproduce the identified cursor-loss mechanism before the fix.
- Incorrect run status is a separate investigation. Stream correctness must not depend on that status being accurate.
- Preserve unrelated workspace edits and existing run records. Build the desktop artifact after checks; arrange restart separately because workflows are active. After rollout, verify that repeated recovery notices no longer create concurrent run-list traffic.
