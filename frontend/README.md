# Spark Frontend

This directory contains the React 19 + Vite frontend for Spark.

The UI is responsible for:

- project registration and project-scoped Home workflows
- conversation threads, flow-run request review, and direct flow launches
- DOT authoring in both structured and raw modes
- execution controls, live run streaming, and run inspection
- settings, diagnostics, and responsive shell layout

## Development

From the repo root, `just setup` prepares the whole checkout.
If you only need the frontend toolchain in this directory, install dependencies with:

```bash
npm ci
```

Run the frontend in dev mode:

```bash
npm run dev
```

By default, the Vite dev server proxies API requests to `http://127.0.0.1:8000`.
Override that with `VITE_BACKEND_URL` if needed.

From the repo root, `just dev-run` is the normal full-stack workflow.

## Shared UI Boundary

- The shadcn CLI source of truth is `src/components/ui`.
- `src/components/app` is limited to the shared dialog controller and `FlowTree`.
- Generic presentation patterns such as cards, notices, field groupings, and empty states should come from `src/components/ui` or stay local to the owning feature.
- Import shared UI by direct module path under `@/components/ui/*` or `@/components/app/*`, not through a mixed barrel.

## Scripts

- `npm run dev`: start the Vite dev server
- `npm run build`: type-check and produce a production build in `dist/`
- `npm run lint`: run ESLint
- `npm run preview`: serve the production build locally
- `npm run shadcn:verify`: verify `components.json` and the shadcn-managed primitives against the pinned repo-local baseline, including `native-select`, without depending on live `shadcn@latest` network calls at evaluation time
- `npm run test:unit`: run Vitest unit tests
- `npm run test:unit:watch`: run Vitest in watch mode

## Build Output

Production assets are emitted to `frontend/dist/`.
From the repo root, use `just deliverable` to build the distributable wheel and sdist with the bundled UI:

```bash
just deliverable
```

## Related Docs

- Root project overview: [README.md](../README.md)
- UI workflow acceptance assets: [tests/acceptance/agent-workflows/README.md](../tests/acceptance/agent-workflows/README.md)
