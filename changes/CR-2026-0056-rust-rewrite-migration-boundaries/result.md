---
id: CR-2026-0056-rust-rewrite-migration-boundaries
title: Rust Rewrite Migration Boundary Records
status: completed
type: docs
changelog: internal
---

## Summary

Recorded the M7 migration boundaries for Rust-backed Spark without changing runtime behavior. The new structured record classifies `agent` and `unified_llm` surfaces as native Rust contracts, Rust-owned adapters, or retained Python modules and ties each boundary to existing compatibility fixtures, Rust tests, Python tests, validation artifacts, or milestone results.

## Shipped Changes

- Added `.spark/rust-rewrite/current/migration-records.json` with bound requirements, contract decisions, adapter classifications, deprecated compatibility surfaces, open policy gaps, explicit non-goals, and future decision candidates.
- Added `docs/rust-rewrite-migration.md` as the human-readable companion record.
- Cross-referenced the migration record from the Rust rewrite architecture.
- Added repo-hygiene coverage for the structured record and evidence references.

## Validation

- `uv run pytest -q -x --maxfail=1 tests/repo_hygiene/test_rust_rewrite_migration_records.py tests/compat/agent tests/compat/providers tests/compat/api` passed with 11 tests.
- `uv run pytest -q tests/repo_hygiene tests/compat` passed with 91 tests.
- `uv run pytest -q` passed with 2015 tests passed, 26 skipped, and 1 warning.
