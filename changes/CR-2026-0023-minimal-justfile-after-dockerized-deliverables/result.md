---
id: CR-2026-0023-minimal-justfile-after-dockerized-deliverables
title: Minimal Justfile After Dockerized Deliverables
status: completed
type: internal
changelog: internal
---

## Summary

Delivered the approved justfile simplification. The public `just` surface now contains only `setup`, `dev-run`, `dev-docker`, `run-docker`, `test`, `deliverable`, `install`, and `install-systemd`. `just deliverable` now runs the Dockerized `Dockerfile.wheel` builder, and the install flows build from that deliverable before installing or registering the packaged app.

Removed the obsolete GitHub Actions workflow, UI smoke just workflow, local packaging alias/workflow targets, and install shell wrappers that were replaced by private just recipes.

## Validation

- `just --list` showed only the eight approved public recipes.
- `just --dry-run deliverable` showed the Dockerized builder path.
- `just --dry-run install` and `just --dry-run install-systemd` showed both install flows depending on the Dockerized deliverable and then running the inline install/service commands.
- `rg` for removed workflow names and `.github/workflows` references only found the approved change request text.
- `uv run pytest -q` passed: 1727 passed, 26 skipped.
- `npm --prefix frontend run test:unit` passed: 44 files and 313 tests passed. Existing React `act(...)` warnings were emitted during frontend tests.

## Shipped Changes

- `justfile`: reduced to the approved public workflows, made `deliverable` Dockerized, kept private helper recipes for frontend dependency installation and wheel installation, and inlined install/systemd logic.
- `Dockerfile.wheel`: present as the Dockerized deliverable builder image used by `just deliverable`.
- `.github/workflows/ci.yml`: removed.
- `scripts/install-wheel.sh`, `scripts/install.sh`, `scripts/install-systemd.sh`, and `scripts/ui-smoke.sh`: removed.
- `README.md`, `frontend/README.md`, and `tests/README.md`: updated to document the smaller workflow surface and Dockerized deliverable path while removing CI/UI-smoke workflow references.
- `tests/repo_hygiene/test_dot_format_lint.py`: removed repository-hygiene assertions that required CI wiring or deleted justfile guard targets.
