# Docker Packaging with Container-Native Project Registry

## Summary
Package Spark as a containerized app that stores Spark runtime state in `/spark` and works with projects mounted under `/projects`. Keep the project registry unchanged in principle: it records absolute paths that exist inside the Spark runtime. Add configured project browse roots so Docker users register mounted container paths instead of host paths.

## Key Changes
- Replace the stale backend `Dockerfile` with a packaged image build:
  - build the frontend in a Node stage
  - copy `frontend/dist` into `src/spark/ui_dist`
  - install the Python package from `src/`
  - include runtime dependencies needed for git, Codex CLI, and provider-backed workflows
  - default command: `spark-server serve --host 0.0.0.0 --port 8000 --data-dir /spark`
- Add a packaged Compose file, keeping dev Compose behavior separate:
  - `compose.yaml` remains development-oriented for `just dev-docker`
  - add `compose.package.yaml` for built-image usage
  - mount `spark_data:/spark`
  - mount `${SPARK_PROJECTS_HOST_DIR:-$HOME/projects}:/projects`
  - set `SPARK_HOME=/spark`, `SPARK_PROJECT_ROOTS=/projects`, and `ATTRACTOR_CODEX_RUNTIME_ROOT=/spark/runtime/codex`
- Add `SPARK_PROJECT_ROOTS` to settings as an optional `os.pathsep`-separated list of absolute paths.
- Update `/workspace/api/projects/browse`:
  - default browse path becomes the first configured project root when `SPARK_PROJECT_ROOTS` is set
  - otherwise keep current `$HOME` default
  - include configured browse roots in the response so the frontend can offer root shortcuts
  - continue storing registered project paths exactly as normalized container paths
- Update the Projects UI to show configured browse roots as selectable shortcuts in the project browser/register flow.

## Registry Behavior
- Do not add host/container path translation to project records.
- Do not store host paths in `project.toml`.
- Registered Docker projects should look like `/projects/my-app`, and `project_id` continues to derive from that container path.
- Existing non-Docker installs keep their current behavior unless `SPARK_PROJECT_ROOTS` is set.

## Test Plan
- Backend tests:
  - settings parses unset, single-root, and multiple-root `SPARK_PROJECT_ROOTS`
  - browse defaults to configured root when present
  - browse response includes configured roots
  - project registration persists the container path unchanged
  - current `$HOME` default and explicit `/` browse behavior still work when no roots are configured
- Frontend tests:
  - project browse parser accepts `roots`
  - project browser renders root shortcuts and navigates to selected root
- Packaging checks:
  - Docker image builds
  - packaged container serves `/`, `/workspace/api/projects/browse`, and `/attractor/status`
  - Compose package example can register a project mounted under `/projects`

## Assumptions
- Docker packaged usage is single-container Spark backend serving the built UI.
- The existing development Compose workflow should remain available and not become the packaged deployment interface.
- `/projects` is the default container mount point for user repositories.
- Path translation is intentionally out of scope; users mount projects where Spark can directly access them.
