# Result

Implemented fresh same-run Retry with checkpointed per-node execution bases and a
shared artifact-attempt calculation. Explicit retry resets only the target's
automatic-retry count. Existing artifacts, checkpoint logs, working directories,
snapshots, and lineage are preserved; Continue and new child invocations start
with fresh execution metadata. Parallel branch artifacts inherit their owning
execution attempt.

Retry requests now persist a unique ID and the target run/node/stage plus nested
child targets. Preparation writes the adjusted checkpoint before preparing
children or dispatching execution. Restarts finish incomplete preparation without
advancing attempts twice. Applied requests remain recognizable after success or
failure and cannot authorize unrelated node invocations.

Parent Retry validates saved flows, checkpoints, and ancestor/descendant lineage,
then resumes linked failed children in place through the manager/resume path.
Successful children are consumed without replacement. New child failures propagate
without repeating the request. Existing execution ownership rejects overlapping
tree retries, including the acceptance-to-spawn interval; the same ownership guard
is transferred to the execution thread. Custom launchers retain child execution responsibility. Human gates,
recovery pauses outside the prepared invocation, and cancellation remain protected.
Direct child Retry also survives restart while its parent remains failed. Recovery
distinguishes the child's own durable request from inherited manager requests,
validates saved state and lineage, reserves tree ownership before preparation, and
transfers that guard to the existing resume thread. Custom launchers retain recovery
responsibility.

CLI/API request and response shapes and execution-accepted `started` semantics are
unchanged. The permanent recovery contract is in `specs/attractor-spec.md`, with
operator guidance in `spark-operations.md`.

## Regression evidence

Before the fix, the augmented API test
`pipeline_lifecycle_contracts::retry_route_executes_the_prepared_run` failed with
actual status `failed`, expected `completed`, after seeding a valid durable failed
response. The same test passes with the fix.

The added API regression
`direct_child_retry_recovers_all_crash_windows_with_failed_parent` also failed
before the recovery fix: the preparation crash returned `resumed: []` instead of
`["retry-leaf"]`. It now passes crashes during preparation, after preparation before
dispatch, and after the new durable response before checkpoint advancement. It
checks the same child ID, one base advance, unchanged request ID, failed parents,
no siblings, and rejection of overlapping retries and repeated recovery while
a human gate owns execution. The durable-response case opens no interview; the
other cases open exactly one. Separate direct-child recovery cases preserve
cancellation, canceled ancestors, lineage/source validation, and custom launchers.

The review regressions reproduced both remaining blockers before their fixes:
`looping_retry_resumes_the_prepared_visit_and_reuses_its_durable_response` observed
zero task executions instead of one, and
`direct_child_retry_rejects_ambiguous_ancestor_invocations_without_mutation`
received `200 started` instead of 409.

Resume now uses the existing run/node/stage retry target to distinguish the
prepared invocation from earlier completed visits. The real-flow test succeeds,
loops back, fails, and explicitly retries at the original stage with a fresh
attempt. It covers restart during and after preparation, durable-response reuse,
unchanged earlier completion history, and readable old/new artifacts.

Direct-child validation now checks sibling invocation uniqueness at every ancestor
before acceptance. The API regression checks duplicate children at both ancestor
levels, with explicit and absent legacy invocation indexes, and requires the exact
`retry_ambiguous_child_invocation` 409 with unchanged records, checkpoints
(including counters), and events. Unique legacy children remain accepted.

| Required case | Deterministic coverage |
| --- | --- |
| 1. Retry saved failure, preserve artifacts | Augmented API lifecycle test; runtime repeated-retry contract |
| 2. Another explicit retry | Distinct request IDs and artifact attempts 2–6 across three retries |
| 3. Restore automatic allowance | Each exhausted retry gets two automatic attempts; unrelated counter and logs preserved |
| 4. Preparation/response crash windows | Resume prepared checkpoint; reapply saved request; consume only the new durable response |
| 5. Ordinary durable recovery | Existing root/child recovery and malformed/incomplete/contract-invalid response regressions |
| 6. Nested parent recovery | Three-level failed tree completes with original IDs, old/new artifacts, and no siblings |
| 7. Interrupted preparation and successful children | Reconstructed write-ahead crash resumes once; successful, historically acknowledged child checkpoint remains unchanged |
| 8. Protections | Invalid lineage/source/checkpoint, canceled descendant, active ancestor, unanswered human gate, recovery pause, and custom launcher checks |
| 9. Compatibility and Continue | Absent metadata defaults to zero; malformed metadata rejected; legacy marker grants no authority; Continue receives a new ID without retry metadata |

## Validation

- The current revision passed the full `just test` gate with a fresh temporary
  `SPARK_HOME`, without test exclusions. Validation log:
  `/tmp/spark-cr0107-final.5S37Xt/validation.log`.
- Passing affected suites include 82 API contracts (including direct-child recovery and ambiguity
  regressions), 53 runtime contracts, 14 runtime process contracts,
  2 runtime unit tests, and all 47 CLI process contracts.
- All workspace Rust suites passed, including the 50,000-event persistence
  regression and the HTTP conversation test that intermittently failed in earlier
  validation attempts.
- All 445 frontend tests across 58 files and the production frontend build passed.
- Formatting and `git diff --check` passed.

All run fixtures used isolated temporary storage. This complete passing gate
supersedes the earlier failed validation attempts; no conversation implementation
or frontend source was changed to obtain the pass.

No runtime was installed or restarted, and no live workflow records were modified.
The rebuilt runtime must only be installed/restarted after the active workflow
finishes.
