---
id: CR-2026-0059-acceptance-workflow-harness
title: Acceptance Workflow Harness
status: completed
type: feature
changelog: internal
---

## Summary

Delivered an executable pytest acceptance harness for the existing `tests/acceptance/agent-workflows` markdown workflows. The harness preserves the markdown files as human-readable assets while mapping each workflow to deterministic, black-box executable checks for the requested Spark product journeys. The `acceptance_workflow_harness` migration gap is now recorded as closed with implementation evidence.

## Validation

- `uv run pytest -q tests/acceptance/agent-workflows` passed.
- `uv run pytest -q -x --maxfail=1 tests/acceptance/agent-workflows` passed.
- `uv run pytest -q -x --maxfail=1 tests/repo_hygiene/test_rust_rewrite_migration_records.py tests/repo_hygiene/test_rust_rewrite_requirement_decision_coverage.py` passed.
- `uv run pytest -q -x --maxfail=1 tests/repo_hygiene/test_rust_rewrite_final_validation_artifacts.py` passed.
- `uv run pytest -q` passed with 2040 passed, 26 skipped, and 1 warning.

## Shipped Changes

- Added the runnable agent-workflow acceptance harness in `tests/acceptance/agent-workflows/harness.py`, with pytest wiring in `conftest.py` and `test_agent_workflows.py`.
- Bound all five existing workflow assets to executable cases covering project selection/context, project-scoped Home conversation visibility, flow authoring/editing/validation/save/reopen persistence, launch/monitor/cancel-or-terminal-state visibility, run history/detail inspection, events, checkpoint/context, artifacts, flow-run request review, and direct-launch traceability.
- Kept harness execution local and deterministic, without external LLM or provider calls.
- Updated `tests/acceptance/agent-workflows/README.md` with harness run instructions and workflow coverage.
- Updated migration, TODO, coverage, validation, and milestone records so `acceptance_workflow_harness` is no longer an open policy gap while unrelated gaps remain open.
