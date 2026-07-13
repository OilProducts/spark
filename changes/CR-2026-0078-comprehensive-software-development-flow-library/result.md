---
id: CR-2026-0078-comprehensive-software-development-flow-library
title: Comprehensive Software-Development Flow Library
status: completed
type: feature
changelog: public
---

## Summary

Added the ten-launcher software-development flow catalog, hidden reusable workers, and product-owned runtime primitives for task normalization, canonical Git repository identity, repository-scoped integration locking, committed-base worktree isolation, durable task/result state, validation evidence, commit finalization, cleanup, and guarded explicit-merge integration.

## Validation

- `cargo fmt --all -- --check`
- `cargo test --workspace --all-features`
- `npm --prefix frontend run test:unit` (373 tests passed)
- `npm --prefix frontend run build`
- Focused software-development contracts cover launcher-to-runtime policy enforcement, normalization equivalence, concurrent isolation, dirty-checkout exclusion, linked-worktree locking, durable failure preservation, successful cleanup, explicit merge commits, dirty targets, target movement, source-worktree removal, and validation-failure preservation.

## Shipped Changes

- Packaged and enabled ten intent-oriented launchers while keeping implementation workers non-requestable.
- Composed Implement Change from reusable worker subflows and retained Implement Change Request as a delegating compatibility launcher with its durable artifact lifecycle.
- Preserved Implement Spec Program milestone dispatch and compatibility metadata.
- Added observable Rust contracts for the shared Git and durable-state primitives and corrected launcher context references to the declared `context.request` namespace.
- Wired launcher workspace-policy metadata into pipeline startup so agent execution actually moves to managed committed-state worktrees, read-only runs use non-writable detached checkouts, integration runs cross the locked merge boundary, and terminal outcomes are durably recorded under `.spark/software-development/runs/<run-id>/` before successful cleanup.
