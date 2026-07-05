---
id: CR-2026-0060-subgraph-scoped-defaults-ui-authoring
title: Subgraph And Scoped Defaults UI Authoring
status: completed
type: feature
changelog: public
---

## Summary

Graph Settings now provides first-class structured authoring for graph-level `node[...]` / `edge[...]` defaults and top-level subgraphs. Users can inspect, create, edit, save, and reopen these DOT constructs without switching to raw DOT for supported fields, while nested subgraphs and unknown extension attrs remain preserved.

## Validation

- `npm --prefix frontend run test:unit -- --run src/features/editor/__tests__/GraphSettings.test.tsx` passed.
- `npm --prefix frontend run test:unit -- --run src/__tests__/ContractBehavior.test.tsx` passed.
- `npm --prefix frontend run test:unit -- --run src/store/__tests__/projectScope.test.ts` passed.
- `npm --prefix frontend run build` passed with the existing large-chunk warning.
- `uv run pytest -q -x --maxfail=1 tests/contracts/frontend/test_canonical_flow_model_contracts.py tests/contracts/frontend/test_raw_dot_baseline_fixtures.py` passed.
- `uv run pytest -q -x --maxfail=1 tests/repo_hygiene/test_rust_rewrite_migration_records.py tests/repo_hygiene/test_rust_rewrite_requirement_decision_coverage.py` passed.
- `uv run pytest -q -x --maxfail=1 tests/compat/api/test_http_route_fixtures.py::test_http_route_fixtures_match_python_oracle tests/compat/packaging/test_packaging_smoke_fixtures.py::test_build_deliverable_smoke_fixture_matches_python_oracle tests/compat/packaging/test_packaging_smoke_fixtures.py::test_package_resource_presence_fixture_matches_python_oracle` passed after refreshing frontend bundle/package compat goldens.
- `uv run pytest -q` passed with 2040 passed, 26 skipped, and 1 warning.

## Shipped Changes

- Added explicit editor state for canonical defaults and subgraphs, with hydrated replacements separated from user edits so autosave only fires on real structured authoring changes.
- Updated editor serialization, raw DOT handoff, preview sync, and save baselines to carry graph-level defaults and subgraphs from structured state.
- Added Graph Settings controls for graph-level node defaults, graph-level edge defaults, subgraph id/label/attrs, node-id membership, scoped node defaults, scoped edge defaults, and nested subgraph inspection.
- Preserved nested subgraphs and unknown attrs while editing exposed top-level subgraph fields.
- Added frontend/store tests for structured defaults and subgraph authoring plus existing contract coverage for raw DOT preservation.
- Updated `src/spark/guides/dot-authoring.md`, `TODO.md`, the migration record, and the coverage review so `subgraph_and_scoped_defaults_ui_authoring` is recorded as implemented instead of an open policy gap.
