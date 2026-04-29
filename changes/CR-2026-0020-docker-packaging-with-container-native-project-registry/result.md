---
id: CR-2026-0020-docker-packaging-with-container-native-project-registry
title: Docker Packaging with Container-Native Project Registry
status: completed
type: feature
changelog: public
---

## Summary
Delivered packaged Docker support for Spark as a single container that serves the built UI, stores runtime state under `/spark`, and defaults project browsing to container-visible project roots when configured. Project registration remains container-native: registered paths are normalized and persisted exactly as Spark sees them, such as `/projects/my-app`, with no host/container path translation added.

## Validation
- `uv run pytest -q` passed with 1730 passed and 26 skipped.
- `npm run build` passed.
- `npm run test:unit -- src/lib/api/__tests__/projectsApi.test.ts src/app/__tests__/AppShell.test.tsx` passed.
- `npm run test:unit -- --run src/__tests__/ContractBehavior.test.tsx` passed.
- `docker build -t spark-package-test .` passed.
- Packaged container smoke checks covered `/`, `/workspace/api/projects/browse`, `/attractor/status`, and registering `/projects/my-app`.

## Shipped Changes
- Replaced the root `Dockerfile` with a multi-stage packaged image build that builds the frontend, copies `frontend/dist` into `src/spark/ui_dist`, installs the Python package, includes runtime tooling, exposes port 8000, and starts `spark-server serve --host 0.0.0.0 --port 8000 --data-dir /spark`.
- Added `compose.package.yaml` for built-image usage with `/spark` runtime state, `/projects` project mounts, and packaged runtime environment variables.
- Kept `compose.yaml` focused on development by installing the mounted checkout editable before running the reload server.
- Added `SPARK_PROJECT_ROOTS` settings parsing as an optional `os.pathsep`-separated absolute path list.
- Updated `/workspace/api/projects/browse` to default to the first configured project root, include `roots` in responses, and preserve existing `$HOME` and explicit-root browsing behavior when no roots are configured.
- Updated frontend project browse API parsing and project browser UI to display configured root shortcuts.
- Added backend and frontend coverage for project roots parsing, browse response roots, root shortcut navigation, and unchanged container path persistence.
