# Result

Publication cursors now survive terminal usage eviction for the publisher lifetime. Failed or absent publication sequences leave the prior cursor intact. Run-list initial loads, manual refreshes, reconnects, and recovery notices share one scope-owned request and one pending-refresh flag. Scope cleanup aborts requests and discards pending work; request identity guards responses and cleanup. Failures preserve the error presentation and discard pending retries. Unrelated-project recovery notices are ignored.

## Verification

- Before the fix, the failed-run regression published 303 historical journal entries again on the second notification (expected zero). After the fix, history publishes once, repeated notifications publish no journal entries, and one appended event publishes once. Corrupt and empty journal publications preserve progress. Existing terminal usage eviction coverage passes.
- Four deferred-response frontend tests cover burst coalescing across initial/manual/reconnect/recovery requests, trailing refreshes, failure without automatic retry, explicit retry, scope cancellation, stale responses and cleanup, new-scope request URLs and returned project records, synchronization stop, and unrelated-project filtering.
- `just test` passes: Rust formatting, `cargo test --workspace --all-features` (including the affected `spark-http` suite), frontend unit tests, TypeScript checking, and the production frontend build.
- All 391 frontend tests pass, including existing AppShell and contract project-switching checks. `npm --prefix frontend run build` passes TypeScript checking and production compilation.

- After checks, `cargo build --release -p spark-desktop --bin spark-desktop --all-features` passes. Desktop artifact: `target/release/spark-desktop` (optimized macOS executable).

## Rollout handoff

Restart must be scheduled separately after active workflows can safely tolerate it. No restart or deployment was performed in this implementation worktree. After rollout, use the desktop network inspector with a delayed run-list response and repeated recovery notices: verify at most one active `/attractor/runs` request, one trailing refresh after success, no automatic retry after failure, and prompt cancellation/loading on project switch.

The exact captured overflow batches remain uninstrumented. Correctness does not depend on accurate terminal status. Existing run records, stored workflow statuses, endpoints, event schemas, dependencies, buffer sizes, and durable formats are unchanged.
