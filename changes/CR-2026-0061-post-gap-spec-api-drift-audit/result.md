---
id: CR-2026-0061-post-gap-spec-api-drift-audit
title: Post-Gap Spec/API Drift Audit
status: completed
type: docs
changelog: internal
---

## Summary

Completed the post-gap audit against the original Attractor contract and current Spark product specs. The durable audit artifact records the audited DSL, runtime, API, frontend/editor, migration, coverage, and final-validation surfaces, and classifies the remaining differences as intentional Spark product behavior, explicit non-goals, or future decision candidates. No remaining unintended compatibility drift or blockers were found.

The audit closes `post_gap_spec_api_drift_audit`. It also verifies that the acceptance workflow harness and subgraph/scoped-defaults UI authoring gaps were already closed with evidence, while deprecated event route removal remains a future contract decision and retained Python modules plus remote worker reintroduction remain explicit non-goals.

## Validation

- `uv run pytest -q -x --maxfail=1 tests/repo_hygiene/test_rust_rewrite_migration_records.py tests/repo_hygiene/test_rust_rewrite_requirement_decision_coverage.py` passed with 17 tests.
- `uv run pytest -q -x --maxfail=1 tests/compat/dsl/test_dsl_fixture_oracle.py tests/compat/runtime/test_runtime_execution_fixtures.py tests/compat/api/test_http_route_fixtures.py tests/contracts/frontend/test_canonical_flow_model_contracts.py tests/acceptance/agent-workflows/test_agent_workflows.py` passed with 14 tests.
- `uv run pytest -q` passed with 2040 tests, 26 skipped, and 1 warning.

## Shipped Changes

- Added `.spark/rust-rewrite/current/validation/post-gap-spec-api-drift-audit.md` as the durable audit artifact.
- Moved `post_gap_spec_api_drift_audit` from `policy_gaps` to `closed_policy_gaps` in `.spark/rust-rewrite/current/migration-records.json`.
- Regenerated requirement/decision coverage evidence so `post_gap_spec_api_drift_audit` is a passing closed-policy-gap note rather than an open policy gap.
- Marked the final audit item complete in `TODO.md`.
- Updated migration/status docs and repo-hygiene tests to reflect the closed audit gap.
