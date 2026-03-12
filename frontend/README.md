# Spark Spawn Frontend

This directory contains the React 19 + Vite frontend for Spark Spawn.

The UI is responsible for:

- project registration and project-scoped Home workflows
- conversation threads, spec-edit review, and execution-card review
- DOT authoring in both structured and raw modes
- execution controls, live run streaming, and run inspection
- settings, diagnostics, and responsive shell layout

## Development

Install frontend dependencies:

```bash
npm install
```

Run the frontend in dev mode:

```bash
npm run dev
```

By default, the Vite dev server proxies API requests to `http://127.0.0.1:8000`.
Override that with `VITE_BACKEND_URL` if needed.

From the repo root, `just run` is the normal full-stack workflow.

## Scripts

- `npm run dev`: start the Vite dev server
- `npm run build`: type-check and produce a production build in `dist/`
- `npm run lint`: run ESLint
- `npm run preview`: serve the production build locally
- `npm run test:unit`: run Vitest unit tests
- `npm run test:unit:watch`: run Vitest in watch mode
- `npm run ui:smoke`: run Playwright smoke checks

## Build Output

Production assets are emitted to `frontend/dist/`.
To bundle them into the Python package, sync them into [attractor/ui_dist/](/Users/chris/tinker/sparkspawn/attractor/ui_dist) from the repo root:

```bash
./scripts/sync_ui_dist.sh
```

Or use:

```bash
just sync-ui-dist
```

## Related Docs

- Root project overview: [README.md](/Users/chris/tinker/sparkspawn/README.md)
- UI workflow acceptance assets: [tests/acceptance/agent-workflows/README.md](/Users/chris/tinker/sparkspawn/tests/acceptance/agent-workflows/README.md)
