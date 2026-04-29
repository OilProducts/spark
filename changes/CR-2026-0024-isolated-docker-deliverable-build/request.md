# Isolated Docker Deliverable Build

## Summary
Make `just deliverable` build from source copied into the builder image, not from a bind-mounted checkout. Remove the packaging script’s incidental dependency on `.git` / `git ls-files`. The host should only provide Docker build context and receive final artifacts in `dist/`.

## Key Changes
- Update `Dockerfile.wheel` to copy the project source into the image and run the deliverable build from that internal source tree.
- Change `just deliverable` to:
  - build `Dockerfile.wheel`
  - create host `dist/`
  - run the builder with only `dist/` mounted as `/out`
  - pass host UID/GID only for final artifact ownership, not as the build user
- Run the container build as root inside the image so npm/uv can write inside the copied `/src` tree without host-user home/cache hacks.
- After the build, copy `dist/spark-*` from the image to `/out` and `chown` those copied artifacts to the host UID/GID when provided.

## Build Script Refactor
- Replace the git-specific source checks with source-tree checks: require `pyproject.toml`, `src/`, `frontend/package-lock.json`, and `scripts/build_deliverable.py`.
- Replace `git ls-files` staging with filesystem staging that copies the source tree while excluding generated/runtime directories.
- Exclude at least: `.git`, `.venv`, `.spark`, `dist`, `frontend/node_modules`, `frontend/dist`, `.pytest_cache`, `.ruff_cache`, `__pycache__`, and Python bytecode.
- Keep existing behavior that runs `npm ci` when frontend deps are absent, runs `npm --prefix frontend run build`, stages `frontend/dist` into `src/spark/ui_dist`, runs `uv build`, verifies wheel contents, and publishes exactly one wheel plus one sdist.

## Docker Context and Docs
- Update `.dockerignore` so the builder context does not send generated/runtime directories such as `dist` and `.spark`.
- Update README packaging text to say `just deliverable` builds from source copied into the builder image and mounts only `dist/` for artifact output.
- Remove references saying the builder bind-mounts the checkout or depends on `.git` / `git ls-files`.

## Test Plan
- Run `just --dry-run deliverable` and confirm it mounts only `dist/`, not the repo root.
- Run `docker build -f Dockerfile.wheel -t spark-wheel-builder .`.
- Run `just deliverable` and verify host `dist/` contains exactly one `spark-*.whl` and one `spark-*.tar.gz`.
- Confirm the wheel verification in `scripts/build_deliverable.py` passes.
- Run `uv run pytest -q`.

## Assumptions
- The deliverable builder should not use host `frontend/node_modules`, host `.git`, host uv state, or host npm state.
- It is acceptable for the container to build as root internally as long as copied output artifacts are owned by the invoking host user.
- `scripts/build_deliverable.py` should remain usable outside Docker, but it no longer needs to require a git checkout.
