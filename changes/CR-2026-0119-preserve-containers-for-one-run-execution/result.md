# CR-2026-0119 result

Implemented in the parent flow's isolated worktree, based on committed main
`d6a73529fbcce5731551051451add05877b8706a`. No commits, merges, branch changes,
or edits to the active checkout were made by this implementation stage. The
parent Spark flow owns validation, committing, and workspace release.

The shared production factory now keeps one container for each uninterrupted
execution, including node retries and result summarization. `NodeExecutor` has
a default no-op `finalize`; `PipelineExecutor` calls it before returning on all
ordinary success/error paths, including cancellation and pause. Container
finalization uses the existing idempotent `close`, with destructor cleanup
retained for exceptional exits. Both removal launch errors and nonzero exits
populate `cleanup_error` and its existing event without replacing the execution
result or original failure. Persistence failures are reported to stderr without
replacing that result.

Settings capture, target-side path discovery, credentials, mounts, and native
execution retain their existing paths. Retry, continuation, and recovery still
construct new executors from retained execution configuration. Cancellation
remains stop-after-current-node; pause ends the execution. Only mounted storage
survives execution boundaries. No settings, endpoints, schema changes, cleanup
retries, reattachment, or orphan scavenging were added.

Validation:

- Focused API/runtime/container suites passed before the repository gate, then
  passed again after strengthening retained-configuration fixtures: 176 tests
  passed, two intentionally ignored (including the opt-in Docker smoke test).
- Added checks cover cross-node/retry/summary container reuse, separate execution
  identities, lifecycle finalization, both cleanup failure types and outcome
  preservation, idempotence through destruction, and retained configuration on
  retry/continuation/recovery. Existing native-no-Docker and current target-path
  and activity-storage contracts remain in use.
- Repository gate: `just test` passed on the final rerun (Rust formatting,
  workspace tests with all features: 952 passed and seven ignored; 577 frontend
  tests in 77 files; and the
  production frontend build). The first run encountered a mock Codex
  `turn/start` timeout in the unchanged agent-adapter suite and eight consequent
  environment-lock poison failures. The initiating test passed in isolation;
  the subsequent full gate passed without concurrent test runs.
- Added the opt-in `real_docker_run_lifetime` API test. It uses an existing worker
  image (default `spark:package`, override `SPARK_WORKER_IMAGE`), two tool nodes
  sharing an unmounted `/tmp` file, two isolated executions, and verifies removal.
  No model calls are used. Its explicit invocation failed at Docker availability:
  `/var/run/docker.sock` is absent. Real Docker behavior remains unverified here.

Commands (the process had no configured default Rust toolchain):

```sh
RUSTUP_HOME=/Users/chris/.rustup RUSTUP_TOOLCHAIN=stable cargo test -p attractor-api -p attractor-runtime -p attractor-execution
RUSTUP_HOME=/Users/chris/.rustup RUSTUP_TOOLCHAIN=stable just test
RUSTUP_HOME=/Users/chris/.rustup RUSTUP_TOOLCHAIN=1.97.0 cargo test -p attractor-api real_docker_run_lifetime -- --ignored
```

No implementation commit exists yet; this report and the implementation are
submitted together for the parent flow's single focused commit.
