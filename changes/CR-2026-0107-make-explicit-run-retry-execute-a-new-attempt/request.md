# Make explicit run retry execute a new attempt

## Summary

Fix retry so it executes failed work again under the same run ID, while ordinary crash recovery continues to reuse durable outcomes.

Retrying a parent will recover its linked failed child in place and then continue the parent. Each explicit retry receives a fresh configured automatic-retry allowance. Preserve existing checkouts, snapshots, lineage, and prior attempt logs.

Keep this separate from the running frontend refactor. Do not document a temporary workaround.

## Implementation

### Separate execution identity from retry allowance

The current executor identifies saved work by run, node, stage, and attempt. Explicit retry removes the failed node from the completed list but leaves that identity unchanged, allowing the executor to reload its saved failure.

- Persist a per-node execution-attempt base in reserved checkpoint context. Missing entries default to zero for compatibility.
- Calculate the artifact attempt as `execution base + automatic retry count`, using one shared helper in execution and all durable-outcome recovery checks.
- On explicit retry, advance the failed node’s base beyond its previous attempt and reset its automatic retry count. Preserve other nodes’ counters.
- Persist the adjusted checkpoint before dispatching execution. Never delete or overwrite the previous attempt’s artifacts.
- Keep durable-outcome reuse enabled. After a crash, recovery must reuse an outcome belonging to the **new attempt**, without falling back to an older attempt.
- Audit consumers of attempt numbers so artifact lookup, transcripts, and recovery use execution identity; backoff and retry limits continue using the automatic retry count.

This avoids both replaying the old failure and disabling crash-recovery protection.

### Make retry authorization durable and specific

Replace the unused run-ID-only retry marker with a structured internal retry request containing a unique request ID, target node, and linked-child target when applicable.

- Persist the request with the new attempt identity.
- Treat repeated preparation with the same request ID as idempotent: do not advance attempts or reset allowances again.
- Keep the request tied to its target node; it must not authorize retries of later, unrelated nodes.
- Retain enough information to recognize an already-applied request after a restart. A subsequent user retry gets a new request ID.
- Recover interrupted preparation from the saved request rather than creating another attempt.
- Reject overlapping retry requests while the affected run tree already has an executor. Use the existing execution ownership mechanism.

Continue supporting old checkpoints without the new fields. The legacy marker alone must not authorize additional execution.

### Recover the linked child from parent Retry

Use the existing manager and child-resume paths rather than adding another scheduler.

- Before accepting a parent retry, validate the linked child’s parent/node/root lineage, checkpoint, and saved flow. Follow the linked failed-child chain for nested managers.
- Persist the child target with the parent’s retry request before preparing child execution.
- Apply that request idempotently to each failed child, then resume the existing child run. Keep its run ID and working directory.
- For this explicit recovery, the linked child takes precedence over the manager’s normal “historical invocation” replacement behavior.
- If the child already succeeded, consume its result and continue the parent without rerunning it or creating a sibling.
- If the child fails again, propagate that new failure. Do not repeatedly retry it under the same request.
- Preserve human-gate waiting and recovery-pause semantics. Do not automatically answer questions or override canceled runs.
- Reject missing, ambiguous, or inconsistent lineage with a specific error. Custom child launchers remain responsible for their execution; do not silently replace their children with native runs.

Direct child retry remains supported.

## Interfaces and documentation

- Keep the existing retry CLI/API request and response shapes. Retry still returns the same run ID; Continue still creates a new run.
- Add only internal checkpoint metadata, with backward-compatible defaults and validation.
- Keep `started` meaning that execution was accepted, not that it succeeded.
- Update the permanent recovery contract and relevant code comments to describe fresh attempts, restored retry allowance, and parent-to-child recovery. Supersede the old child-first-only behavior.
- No new dependency, generic recovery framework, or frontend redesign.

## Verification

Use deterministic fake executors and the existing runtime/API contract suites.

Required regression cases:

1. A persisted failed outcome is followed by explicit retry: the executor runs again, the same run ID succeeds, and both attempts’ artifacts remain readable.
2. A second failure followed by another explicit retry receives another distinct execution identity.
3. Exhausted automatic retries receive a fresh allowance on explicit retry; subsequent automatic attempts use distinct artifact locations.
4. Crash after retry preparation resumes the prepared attempt. Crash after its durable response but before checkpoint advancement reuses that response without executing again.
5. Ordinary crash recovery still consumes valid durable outcomes; malformed or incomplete artifacts remain rejected.
6. Parent Retry recovers its failed child and nested descendant in place, then completes the parent without creating siblings.
7. Restart during parent/child preparation does not apply the same retry twice. An already-successful child is consumed without reexecution.
8. Invalid lineage, overlapping retries, human gates, and recovery pauses retain their required protections.
9. Existing checkpoints lacking the new metadata still load; Continue behavior remains unchanged.

Demonstrate the principal failure before the fix and success afterward. Run affected runtime/API suites, CLI contracts, formatting checks, and the repository’s required checks.

## Delivery

Implement as one focused backend change with regression tests and permanent contract updates. Validate using isolated test runs, without modifying the frontend refactor’s live run records.

The fix requires a rebuilt runtime to take effect. Install/restart it only after the currently running workflow finishes; merging source alone does not update the active desktop process.
