# Minimal Justfile After Dockerized Deliverables

## Summary
Make `justfile` a small set of real project workflows. `deliverable` becomes the Dockerized wheel/sdist build; there is no separate local packaging target. Remove GitHub Actions entirely. Remove UI smoke as a justfile workflow and rely on the normal Python/frontend test suites for routine verification.

## Public Just Targets
Keep only these visible targets:

- `setup`: install local Python and frontend dependencies for source development.
- `dev-run`: run the source checkout backend plus Vite frontend.
- `dev-docker`: run the development compose stack.
- `run-docker`: run the packaged-app compose stack.
- `test`: run the normal local Python suite plus frontend unit tests.
- `deliverable`: build `Dockerfile.wheel`, run the builder container, and write wheel plus sdist to `dist/`.
- `install`: build the deliverable, install the wheel into `~/.spark/venv`, and seed the stable runtime.
- `install-systemd`: build/install the wheel and register/start the user service.

## Remove / Refactor
- Remove `ui-smoke`; delete the just target and stop presenting it as a supported workflow.
- Remove `deliverable-docker`; its behavior becomes `deliverable`.
- Remove `build`; no packaging alias.
- Remove `clean`, `dev-init`, `frontend-unit`, `frontend-build`, `stop`, `logs`, and `restart`.
- Keep private helper recipes only where they remove duplication from public workflows, especially:
  - `frontend-deps`
  - `install-wheel`
- Inline install logic into private just recipes and delete the install shell wrappers:
  - `scripts/install-wheel.sh`
  - `scripts/install.sh`
  - `scripts/install-systemd.sh`
- Delete `scripts/ui-smoke.sh` if nothing else uses it after removing CI.
- Keep orchestration scripts that are still real process wrappers:
  - `scripts/dev-run.sh`
  - `scripts/dev-docker.sh`
  - `scripts/run-docker.sh`
  - `scripts/build_deliverable.py`

## CI and Tests
- Delete `.github/workflows/ci.yml`.
- Remove repo-hygiene tests that assert CI workflow wiring or justfile command presence.
- Keep underlying behavior covered by the normal suite:
  - Python: `uv run pytest -q`
  - Frontend unit: `npm --prefix frontend run test:unit`
- Do not add a replacement smoke target unless a concrete gap is identified later.

## Documentation
- Update README command lists and packaging docs:
  - `just deliverable` is now explicitly Dockerized.
  - Remove `just deliverable-docker`, `just build`, `just clean`, `just dev-init`, `just frontend-unit`, and `just ui-smoke`.
  - Replace `just dev-init` examples with `SPARK_HOME=~/.spark-dev uv run spark-server init`.
  - Remove GitHub CI and UI smoke as documented project workflows.

## Test Plan
- Run `just --list` and confirm only the eight public targets are visible.
- Run `just --dry-run deliverable`, `just --dry-run install`, and `just --dry-run install-systemd`.
- Run `rg` for removed target names and `.github/workflows` references.
- Run `uv run pytest -q`.
- Run `npm --prefix frontend run test:unit`.
