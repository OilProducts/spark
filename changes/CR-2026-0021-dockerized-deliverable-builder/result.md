---
id: CR-2026-0021-dockerized-deliverable-builder
title: Dockerized Deliverable Builder
status: completed
type: feature
changelog: public
---

## Summary
Implemented a dedicated Docker-based deliverable builder for Spark install artifacts. The new builder runs the existing `scripts/build_deliverable.py` packaging path against a bind-mounted checkout and leaves the generated wheel and source distribution in the host `dist/` directory.

The runtime application Docker surfaces were not changed as part of this request.

## Validation
- `just --list`
- `just --dry-run deliverable-docker`
- `docker build -f Dockerfile.wheel -t spark-wheel-builder .`
- `just deliverable-docker`
- Verified `dist/` contains exactly `spark-0.1.0-py3-none-any.whl` and `spark-0.1.0.tar.gz`.
- `uv run pytest -q` passed with 1730 passed and 26 skipped.

## Shipped Changes
- Added `Dockerfile.wheel` as the build-toolchain-only image with Python 3.11, Node 20/npm, git, uv, and minimal OS build dependencies.
- Added `just deliverable-docker`, which builds the builder image, mounts the repository at `/workspace`, and runs `uv run python scripts/build_deliverable.py`.
- Updated `README.md` to document the Dockerized deliverable recipe, the expected wheel and sdist outputs in `dist/`, and the distinction between the builder image and the packaged application runtime Docker files.
