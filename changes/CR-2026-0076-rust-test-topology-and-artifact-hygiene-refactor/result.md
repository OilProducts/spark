---
id: CR-2026-0076-rust-test-topology-and-artifact-hygiene-refactor
title: Rust Test Topology and Artifact Hygiene Refactor
status: completed
type: refactor
changelog: internal
---

## Summary

Completed the approved Rust test-topology refactor. The shipped change groups crate integration tests behind `contracts` and `process_contracts` entrypoints, reduces Cargo debug artifact size with line-table-only debug info for development and test profiles, and adds explicit Cargo target cache inspection and cleanup recipes.

The recorded isolated measurements show workspace integration-test executables reduced from 78 to 21, clean target size reduced from 18,069,904,488 bytes to 5,594,060,178 bytes, clean compile-only time reduced from 37.64 s to 32.66 s, and warm full-test time reduced from 19.41 s to 12.80 s on the measured Linux host.

## Validation

- Baseline and refactor isolated runs used separate initially absent `CARGO_TARGET_DIR` directories and `cargo test --workspace --all-features --no-run`, followed by `cargo test --workspace --all-features`.
- Both isolated full Rust test runs passed with ignored-test behavior unchanged.
- `just rust-cache-size` was verified as read-only against an isolated configured target directory.
- `just clean-rust-cache` was verified to print and remove the configured Cargo target directory through `cargo clean`.
- The repository validation gate was recorded as passed during implementation: `cargo fmt --all -- --check`, `cargo test --workspace --all-features`, `npm --prefix frontend run test:unit`, and `npm --prefix frontend run build`.

## Shipped Changes

- Cargo workspace `dev` and `test` profiles now use line-table-only debug information.
- Rust integration tests across workspace crates were moved from standalone test executables into private modules under grouped `contracts` and `process_contracts` targets.
- Process-global or live-resource tests were kept in `process_contracts`; ordinary contract tests use `contracts`.
- Shared process-environment locking was added where grouped tests mutate environment state.
- `justfile` now includes `rust-cache-size` and `clean-rust-cache` recipes for explicit artifact hygiene.
- Contributor documentation in `AGENTS.md` and `README.md` now documents the grouped target topology, focused module filtering, reduced debug information, and opt-in cache cleanup.
- Supporting Rust source/test changes were made where needed for the grouped topology, including adapter server test support and workspace model test behavior.
