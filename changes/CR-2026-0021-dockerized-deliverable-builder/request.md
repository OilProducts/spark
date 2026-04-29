# Dockerized Deliverable Builder

## Summary
Add a dedicated Docker-based build environment for Spark’s install artifacts. This will not change the packaged runtime container. It will build the same canonical deliverable artifacts the repo already supports and write them back into the project’s `dist/` directory.

Also remove the earlier mistaken runtime-container startup edits from the current worktree: restore `Dockerfile` and delete `scripts/package-entrypoint.sh`.

## Key Changes
- Add a separate builder Dockerfile, e.g. `Dockerfile.wheel`, whose image contains the build toolchain only: Python 3.11, Node 20/npm, git, uv, and minimal OS dependencies.
- Do not bake the source tree into the builder image. Run the builder image with the repo bind-mounted at `/workspace` so `scripts/build_deliverable.py` can use the real `.git` checkout and `git ls-files`.
- Add a `just` recipe, e.g. `deliverable-docker`, that:
  - builds the builder image
  - runs it with `"$PWD:/workspace"` mounted
  - executes `uv run python scripts/build_deliverable.py`
  - leaves the resulting wheel and sdist in host `dist/`
- Keep `just deliverable` as the local canonical path and make the Docker recipe a containerized way to run that same path, not a competing packaging implementation.

## Documentation
- Update the repository commands section to mention the new Dockerized deliverable recipe.
- Document that `dist/` receives both artifacts: `spark-*.whl` and `spark-*.tar.gz`.
- Clarify that the builder container is for reproducible package construction, while `Dockerfile` / `compose.package.yaml` are for running the packaged app.

## Test Plan
- Run `just --list` and `just --dry-run deliverable-docker` to verify the recipe is exposed and syntactically valid.
- Run `docker build -f Dockerfile.wheel -t spark-wheel-builder .`.
- Run the new `just deliverable-docker` recipe and verify `dist/` contains exactly one `spark-*.whl` and one `spark-*.tar.gz`.
- Inspect the wheel contents through the existing `scripts/build_deliverable.py` verification, including bundled UI, flows, and guides.
- Run `uv run pytest -q`.

## Assumptions
- The Dockerized builder should output both wheel and sdist because that is the current deliverable contract.
- The builder should be exposed through both a dedicated Dockerfile and a `just` recipe.
- The earlier runtime Docker startup seeding change should be removed from this work so this task stays focused on build packaging.
