# Acceptance Workflow Harness

## Summary
Build an executable acceptance workflow harness for the existing agent-workflow assets under `tests/acceptance/agent-workflows`, then close the `acceptance_workflow_harness` migration gap. The harness should turn the current high-level markdown workflows into runnable, black-box acceptance checks that verify complete Spark product journeys instead of only isolated API/component behavior.

## Background
The Rust rewrite migration record still lists `acceptance_workflow_harness` as an open policy gap. The current assets are durable manual workflow descriptions:

- `tests/acceptance/agent-workflows/project-select-author-execute-inspect.md`
- `tests/acceptance/agent-workflows/pipeline-author-workflow.md`
- `tests/acceptance/agent-workflows/operator-run-workflow.md`
- `tests/acceptance/agent-workflows/reviewer-auditor-workflow.md`
- `tests/acceptance/agent-workflows/project-owner-workflow.md`

The README says these workflows are intended to become executable agent-driven acceptance tests once the required harness exists. The final validation ledger has been `prerequisite_limited` because that executable harness was missing.

## Required Behavior
- Add an executable harness for `tests/acceptance/agent-workflows` that can be run by `uv run pytest`.
- Keep the workflows black-box and outcome-oriented:
  - verify user-visible product goals and observable outcomes
  - avoid asserting prompt/source/doc strings
  - avoid depending on private implementation details when a stable UI/API outcome is available
- Preserve the markdown workflow files as human-readable acceptance assets, but bind each one to an executable test case.
- The harness should cover, at minimum, the existing workflow goals:
  - project selection/registration and active project context
  - project-scoped Home conversation visibility and flow-run request/direct-launch traceability where feasible
  - structured flow authoring/editing, validation, save, and reopen persistence
  - execution launch/monitor/cancel-or-terminal-state visibility
  - run history/detail inspection, including events, checkpoint/context, and artifacts when available
- Prefer existing frontend smoke/API infrastructure where it gives a stable black-box check. It is acceptable for the harness to:
  - use Playwright-backed flows when live UI behavior is required
  - use HTTP/API fixtures or a local test server for deterministic setup
  - mark narrowly unavailable browser/computer-use dependencies with explicit pytest skip conditions, but the harness itself must exist and be runnable
- Add structured harness output or metadata that maps each executable case back to its markdown workflow asset.
- Update documentation in `tests/acceptance/agent-workflows/README.md` to describe how to run the harness and what coverage each workflow provides.
- Update migration/TODO/coverage records so `acceptance_workflow_harness` is no longer an open policy gap when implementation is complete.

## Non-Goals
- Do not implement `subgraph_and_scoped_defaults_ui_authoring`.
- Do not perform the final post-gap spec/API drift audit.
- Do not redesign product UI or introduce new UX requirements outside what is needed to make the existing workflows executable.
- Do not require real external LLM/provider calls for acceptance execution.
- Do not make tests depend on exact prose in specs, prompts, docs, or workflow markdown beyond stable workflow identifiers.

## Suggested Target Paths
- `tests/acceptance/agent-workflows/README.md`
- `tests/acceptance/agent-workflows/*.md`
- `tests/acceptance/agent-workflows/test_*.py`
- `tests/acceptance/agent-workflows/harness.py`
- `frontend/e2e/smoke/*.spec.ts`
- `frontend/e2e/smoke/*`
- `tests/api/*`
- `tests/repo_hygiene/test_rust_rewrite_migration_records.py`
- `tests/repo_hygiene/test_rust_rewrite_requirement_decision_coverage.py`
- `TODO.md`
- `.spark/rust-rewrite/current/migration-records.json`
- `.spark/rust-rewrite/current/validation/requirement-decision-coverage-review.json`

## Tests
- Add pytest coverage proving every markdown workflow asset is registered in the executable harness.
- Add executable acceptance tests for the workflow outcomes, using deterministic local state and no external provider calls.
- Add a regression test that the harness can be invoked through `uv run pytest -q tests/acceptance/agent-workflows`.
- Update repo-hygiene tests so `acceptance_workflow_harness` is closed only when executable evidence exists.
- Run focused validation:
  - `uv run pytest -q -x --maxfail=1 tests/acceptance/agent-workflows`
  - `uv run pytest -q -x --maxfail=1 tests/repo_hygiene/test_rust_rewrite_migration_records.py tests/repo_hygiene/test_rust_rewrite_requirement_decision_coverage.py`
- Final verification: `uv run pytest -q`.

## Acceptance Criteria
- `uv run pytest -q tests/acceptance/agent-workflows` executes the acceptance workflow harness successfully in a clean local test environment.
- Each existing markdown workflow is mapped to at least one executable test case with observable pass/fail outcomes.
- The harness verifies complete product journeys rather than only static file presence.
- No acceptance test requires real external LLM/provider calls.
- `acceptance_workflow_harness` is removed from open policy gaps and recorded as closed by implementation evidence.
- The change result documents shipped harness behavior and validation.
