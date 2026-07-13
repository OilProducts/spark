# Rust Test Topology and Artifact Hygiene Refactor

## Summary

Consolidate the 78 Rust integration-test executables into a small set of grouped targets, reduce debug artifact size while preserving useful backtraces, and add explicit cache-inspection and cleanup commands. Keep standard `cargo test` as the only required runner; do not add nextest.

Success requires observable improvement from clean, isolated before/after measurements, with no warm-test regression or behavioral test changes.

## Implementation Changes

- Set both Cargo `dev` and `test` profiles to line-table-only debug information. Retain incremental compilation and otherwise preserve existing optimization, assertion, and overflow-check behavior.
- Reorganize each crate’s integration tests beneath grouped entrypoints:
  - Use one `contracts` target for ordinary API, state, storage, parsing, and runtime contracts.
  - Use a separate `process_contracts` target only in crates with subprocess, timeout, server-lifecycle, environment-mutation, or live socket tests.
  - Move existing test files beneath module directories so Cargo no longer compiles each file as an independent target; include them as private modules from the grouped entrypoints.
  - Preserve existing test functions and behavioral assertions. Do not merge unrelated test implementations into giant source files.
- Add crate-level test support for process-global state:
  - Use one shared environment lock per grouped executable rather than separate module-local locks.
  - Route every test that mutates process environment through that lock.
  - Continue using ephemeral ports and isolated temporary directories; eliminate fixed-port binding if any executable test still performs it.
  - Keep genuinely process-isolated cases in `process_contracts` rather than disabling parallelism for all contracts.
- Standardize focused triage commands around grouped targets and module filters, for example `cargo test -p spark-agent-adapter --test contracts llm_backend_contracts::case_name`.
- Add explicit Just recipes:
  - `rust-cache-size`: report total `target` usage and its largest immediate subdirectories without changing state.
  - `clean-rust-cache`: print the directory being removed and invoke `cargo clean`; never run automatically from setup, build, dev, or test recipes.
- Update contributor-facing validation and triage documentation, including `AGENTS.md`, to describe grouped targets, module filtering, reduced debug information, and opt-in cleanup. Keep the required validation gate on standard Cargo.

## Measurement and Test Plan

- Before restructuring, benchmark the current commit in an isolated `CARGO_TARGET_DIR`:
  - Clean compile-only wall time and artifact size using `cargo test --workspace --all-features --no-run`.
  - Warm full-test wall time using `cargo test --workspace --all-features`.
  - Count generated workspace integration-test executables.
- Repeat the same commands after restructuring in a separate isolated target directory on the same machine. Record the environment, commands, executable counts, sizes, and timings in the change record.
- Acceptance criteria:
  - Integration-test executable count is materially reduced, with ordinary crates converging on one target and high-risk crates at no more than two.
  - Clean debug artifact size is materially smaller.
  - Warm full-test time does not regress beyond normal measurement noise.
  - No tests are removed, ignored, weakened, or converted into repository-text assertions.
- Run focused grouped targets repeatedly for environment/process-heavy crates to catch concurrency flakiness.
- Run the full repository gate:
  - `cargo fmt --all -- --check`
  - `cargo test --workspace --all-features`
  - `npm --prefix frontend run test:unit`
  - `npm --prefix frontend run build`
- Verify `rust-cache-size` is read-only and `clean-rust-cache` operates only on Cargo’s configured target directory.

## Interfaces and Assumptions

- No product API, runtime behavior, data format, or frontend behavior changes.
- Focused Rust test target names intentionally change; the supported interface becomes grouped target plus module/test filtering.
- `cargo test` remains authoritative locally and in future CI; cargo-nextest is out of scope.
- Cleanup is always explicit and destructive only when the developer invokes `just clean-rust-cache`.
- Performance acceptance uses reproducible before/after evidence rather than a fixed percentage, because linker, filesystem, and CPU differences make absolute thresholds brittle.
