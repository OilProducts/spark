---
id: CR-2026-0057-manager-loop-authoring-surface
title: Manager-Loop Authoring Surface Completion
status: completed
type: feature
changelog: public
---

## Summary
The implemented `stack.manager_loop` authoring surface is now first-class across preview/API payloads, canonical frontend flow modeling, editor controls, serialization, documentation, and Rust rewrite migration evidence. The change covers `manager.steer_cooldown` and `stack.child_autostart` without changing manager-loop runtime semantics or closing the separate `manager_loop_telemetry_ingestion` gap.

## Validation
- `cargo test -p attractor-dsl preview_payload_exposes_full_manager_loop_authoring_surface --all-features`
- `npm --prefix frontend run test:unit -- --run src/features/editor/__tests__/InspectorAndNodeAuthoring.test.tsx src/__tests__/ContractBehavior.test.tsx`
- `uv run pytest -q -x --maxfail=1 tests/contracts/frontend/test_manager_loop_contracts.py tests/contracts/frontend/test_node_handler_roundtrip_contracts.py tests/repo_hygiene/test_rust_rewrite_migration_records.py`
- `cargo test -p attractor-dsl preview_route_graph_payload_matches_http_fixture --all-features`
- `cargo test -p attractor-api preview_service_body_matches_success_fixture --all-features`
- `uv run pytest -q -x --maxfail=1 tests/compat/api/test_http_route_fixtures.py tests/compat/runtime/test_runtime_api_journal_fixtures.py tests/contracts/frontend/test_manager_loop_contracts.py`
- `cargo fmt --check`
- `uv run pytest -q` passed with 2030 passed, 26 skipped, and 1 warning.

## Shipped Changes
- Added `manager.steer_cooldown` and `stack.child_autostart` to Rust preview and Python API graph payloads, with backend preview and fixture coverage.
- Updated the canonical frontend model, canvas hydration, sidebar inspector, and task-node editor toolbar so the two fields are preserved, edited, serialized to DOT, and reloaded alongside the existing manager-loop fields.
- Expanded frontend contract tests, frontend round-trip tests, manager-loop fixtures, and raw DOT baseline coverage for the full manager-loop authoring surface.
- Updated `src/spark/guides/dot-authoring.md`, `specs/attractor-spec.md`, `TODO.md`, migration records, and requirement-decision coverage so `manager_loop_authoring_surface_completeness` is recorded as closed while telemetry ingestion remains open.
