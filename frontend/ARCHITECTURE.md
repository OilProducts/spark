# Frontend Architecture

## Goals

- Keep feature logic inside feature modules instead of accumulating in `src/components`.
- Use `src/ui` as the canonical shared primitive layer for non-canvas UI.
- Keep React Flow node, edge, and canvas rendering custom under `src/features/workflow-canvas`.

## Directory Boundaries

- `src/ui`
  - Shared non-canvas primitives and surface helpers.
  - Safe to import from any feature.
- `src/features/<feature>/components`
  - Presentational feature components.
  - May import `@/ui/*`, local feature model types, and store selectors.
  - Must not call API clients directly.
- `src/features/<feature>/hooks`
  - Orchestration hooks, loaders, and feature-local behavior.
  - The default place for API calls and effectful coordination.
- `src/features/<feature>/model`
  - Pure transforms, selectors, reducers, helpers, and feature-local types.
  - No React.
- `src/features/workflow-canvas`
  - Canvas-specific node, edge, layout, and rendering code.
  - Exempt from the shared primitive requirement because canvas rendering is intentionally custom.

## Shared State

- Global Zustand slices are reserved for cross-feature session state:
  - route/view mode
  - project identity and project sessions
  - editor session
  - execution session
- Derived presentation state should live in feature model modules rather than large controller hooks.
- Feature-local UI state should stay local unless another feature truly needs it.

## Import Rules

- Presentational feature components should not import API clients directly.
- Non-canvas feature components should use `@/ui/*` rather than `@/components/ui/*`.
- Top-level `src/components/*` files are compatibility shims or app-shell composition only. New implementation code belongs under `src/features/*` or `src/ui`.
