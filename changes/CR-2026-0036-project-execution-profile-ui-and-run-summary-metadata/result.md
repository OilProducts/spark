---
id: CR-2026-0036-project-execution-profile-ui-and-run-summary-metadata
title: Project Execution Profile UI And Run Summary Metadata
status: completed
type: feature
changelog: public
---

## Summary

Implemented a project-scoped settings dialog for selecting a default execution profile and added execution placement details to run summaries when runs include execution metadata.

## Validation

- `uv run pytest -q`
- `uv run pytest -q -x --maxfail=1 tests/repo_hygiene/test_execution_runtime_boundaries.py`
- `npm run test:unit -- --run src/app/__tests__/AppShell.test.tsx src/features/runs/components/__tests__/RunSummaryCard.test.tsx`

## Shipped Changes

- Added a navbar Project Settings entry point and dialog for active projects.
- Loaded workspace execution profiles, showed only enabled profiles plus workspace default, and blocked saving on invalid workspace execution settings.
- Persisted project default profile changes through `PATCH /workspace/api/projects/state`, including `null` for workspace default.
- Updated project registry state from the returned project record.
- Added run summary execution metadata display for profile, mode, image, worker, mapped project path, and worker runtime root while omitting the section for legacy runs.
- Added behavior tests for project settings and run summary execution metadata.
