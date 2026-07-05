# Rust Rewrite Migration Boundary Records

## Summary

Create an auditable M7 migration record for Rust-backed Spark that classifies `agent` and `unified_llm` boundaries as native Rust contracts, Rust-owned adapters, or retained Python modules. The record must also carry deprecated compatibility surfaces, current policy gaps, explicit non-goals, and future compatibility-break candidates without changing runtime behavior.

## Key Changes

- Add `.spark/rust-rewrite/current/migration-records.json` as the structured source of truth for M7 migration boundaries.
- Add `docs/rust-rewrite-migration.md` as the human-readable companion.
- Cross-reference the migration record from the Rust rewrite architecture.
- Add repo-hygiene coverage that validates structured fields, evidence references, deprecated-surface preservation, and open-gap/non-goal statuses.

## Non-Goals

- Do not retire Python `agent` or `unified_llm` behavior.
- Do not remove or reshape deprecated compatibility routes.
- Do not change public CLI, HTTP, SSE, storage, frontend, packaging, service, Docker, trigger, provider, agent, or runtime behavior.
- Do not close TODO policy gaps by documentation assertion alone.

## Validation

- `uv run pytest -q tests/repo_hygiene/test_rust_rewrite_migration_records.py tests/compat/agent tests/compat/providers tests/compat/api`
- `uv run pytest -q tests/repo_hygiene tests/compat`
- `uv run pytest -q`
