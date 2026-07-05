---
id: CR-2026-0058-manager-loop-telemetry-ingestion
title: Manager-Loop Telemetry Ingestion
status: completed
type: feature
changelog: public
---

## Summary
Rust-backed manager-loop observe cycles now ingest authoritative linked child-run telemetry into runtime-owned `context.stack.child.*` keys. The shipped behavior preserves existing autostart and conservative steering semantics: running child state suppresses duplicate autostart, stale terminal child state can be replaced by fresh autostart, and ambiguous progress telemetry does not create failure context.

## Validation
- `cargo test -p attractor-runtime manager_loop -- --nocapture`
- `cargo test -p attractor-runtime`
- `uv run pytest -q -x --maxfail=1 tests/handlers/test_manager_loop_handler.py tests/repo_hygiene/test_rust_rewrite_migration_records.py tests/repo_hygiene/test_starter_flow_context_contracts.py`
- `uv run pytest -q -x --maxfail=1 tests/api/test_manager_loop_pipeline_api.py tests/handlers/test_execution_container.py`
- `uv run pytest -q -x --maxfail=1 tests/repo_hygiene/test_rust_rewrite_requirement_decision_coverage.py`
- `uv run pytest -q`

## Shipped Changes
- `crates/attractor-runtime/src/manager_loop.rs` now resolves linked child run state from runtime status resolvers or run-store bundles and records child status, outcome reason fields, active stage, completed nodes, route trace, failure reason, retry counters, artifact/event counts, checkpoint/latest-event timestamps, and run start/end timestamps.
- `src/attractor/handlers/builtin/manager_loop.py` now mirrors the expanded child telemetry ingestion through the retained Python manager-loop path and includes the additional observability fields in manager telemetry artifacts.
- Runtime and Python manager-loop tests cover telemetry ingestion from child status records, terminal failure outcome ingestion, stop-condition behavior, autostart freshness, and no automatic steering from telemetry-only progress signals.
- `specs/attractor-spec.md`, `TODO.md`, `.spark/rust-rewrite/current/migration-records.json`, and `.spark/rust-rewrite/current/validation/requirement-decision-coverage-review.json` now describe the closed `manager_loop_telemetry_ingestion` gap and the supported runtime-owned child telemetry contract.
