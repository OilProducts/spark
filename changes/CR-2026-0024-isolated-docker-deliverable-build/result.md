---
id: CR-2026-0024-isolated-docker-deliverable-build
title: Isolated Docker Deliverable Build
status: completed
type: feature
changelog: internal
---

## Summary
Implemented isolated Docker deliverable builds so `just deliverable` builds from source copied into `Dockerfile.wheel` instead of a bind-mounted checkout. The deliverable script now stages from the filesystem without requiring `.git` or `git ls-files`, publishes artifacts to the configured output directory, and applies host ownership only to copied final artifacts.

## Validation
- `just --dry-run deliverable`
- `docker build -f Dockerfile.wheel -t spark-wheel-builder .`
- `just deliverable`
- `find dist -maxdepth 1 -type f -name 'spark-*' -printf '%f %u:%g\n' | sort`
- `uv run pytest -q` passed with 1730 passed and 26 skipped.

## Shipped Changes
- `Dockerfile.wheel` now copies the repository into `/src`, runs the deliverable command from that internal tree, sets `/out` as the default artifact output, and no longer installs git.
- `justfile` now builds the wheel-builder image, creates host `dist/`, and runs the builder with only `$PWD/dist:/out` mounted while passing `HOST_UID` and `HOST_GID` for output ownership.
- `scripts/build_deliverable.py` now validates required source paths, copies the source tree with generated/runtime exclusions, stages the built frontend into package data, verifies wheel contents, publishes one wheel and one sdist, and chowns copied artifacts when host IDs are supplied.
- `.dockerignore` now excludes `.spark`, `dist`, pytest/ruff caches, and other generated directories from Docker build context.
- `README.md` now describes the copied-source deliverable workflow.
- `tests/test_build_deliverable.py` covers filesystem staging exclusions, required source-tree validation, artifact replacement, and host ownership handling.
