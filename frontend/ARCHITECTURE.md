# Frontend Architecture

Historical evidence in tests and notes may reference superseded shared UI paths. The current source of truth is the boundary described below.

## Intent

- Keep implementation organized around feature modules with an explicit shared UI boundary.
- Use `src/components/ui` for shadcn-managed primitives.
- Limit `src/components/app` to Spark-owned shared dialog orchestration and the shared flow tree.
- Keep workflow canvas rendering custom under `src/features/workflow-canvas`.
- Treat this as an internal frontend architecture document. It does not change backend APIs, persisted contracts, or product behavior.

## Top-Level Layout

- `src/components/ui`
  - Canonical shared non-canvas primitives managed through the shadcn CLI.
  - Safe to import from any non-canvas feature.
  - Import primitives directly from `@/components/ui/*`. Do not add or use a barrel.
- `src/components/app`
  - Limited to `dialog-controller` and `flow-tree`.
  - Shared dialog flows should use `DialogProvider` and `useDialogController`, not browser `alert`, `confirm`, or `prompt`.
  - `FlowTree` remains shared because editor and execution both depend on the same Spark-specific file-tree behavior.
  - Import composites directly from `@/components/app/*`. Do not add or use a barrel.
- `src/features/projects`
- `src/features/runs`
- `src/features/editor`
- `src/features/execution`
- `src/features/triggers`
- `src/features/workflow-canvas`
  - Owns node renderers, edge renderers, canvas frames, and other React Flow-specific rendering concerns.
  - Exempt from the shared primitive requirement because canvas visuals are intentionally custom.
- `src/app`
  - App-shell composition only.
- `src/App.tsx`
  - Top-level wiring between the app shell and feature entrypoints.

## Feature Internals

- `src/features/<feature>/components`
  - Presentational feature components and local composition surfaces.
  - May import `@/components/ui/*`, `@/components/app/*`, local feature model types, and store selectors.
  - Must not call API clients directly.
- `src/features/<feature>/hooks`
  - Orchestration hooks, loaders, subscriptions, and other effectful feature behavior.
  - Default place for API calls and cross-surface coordination inside a feature.
- `src/features/<feature>/services`
  - Transport helpers, API adapters, and stateful integrations.
  - May import API clients and validated client modules.
- `src/features/<feature>/model`
  - Pure reducers, selectors, transforms, helpers, and feature-local types.
  - No React and no direct network calls.

## Shared UI Rules

- Non-canvas feature UI should compose primitives from `@/components/ui/*` instead of hand-rolled raw controls.
- Shared primitives cover controls such as buttons, inputs, selects, tabs, badges, cards, scroll areas, textareas, labels, separators, dialogs, tooltips, switches, and checkboxes.
- Generic presentation patterns such as cards, field groupings, alerts, and empty states should be composed directly from `@/components/ui/*` or owned locally by the feature.
- Shared Spark-specific composites belong in `src/components/app` only when they are Spark-specific and truly cross-feature.
- Feature-specific presentation code stays in the owning feature instead of moving into shared folders just because it looks visually similar.
- `FlowTree` and dialog orchestration are the only remaining shared Spark-managed composites under `src/components/app`.
- Canvas renderers are the exception. Inspector chrome, sidebars, forms, and other non-canvas surfaces are not exempt.

## Import Boundaries

- Presentational feature components must not import API clients directly.
- API client usage is limited to feature hooks and services.
- Presentational components may read store selectors, but store mutation and cross-feature coordination should stay in hooks, services, or top-level composition files.
- Import shadcn primitives directly from `@/components/ui/*`, not from a barrel.
- Import Spark-managed shared composites directly from `@/components/app/*`, not from a retired mixed shared layer.
- Runtime code must not import deleted shared wrapper modules such as panels, field rows, inline notices, empty states, section headers, or project chips from `src/components/app`.

## Shared State

- Global Zustand slices are reserved for cross-feature session state:
  - route and view mode
  - project identity and project sessions
  - home session
  - editor session
  - execution session
  - runs session
  - triggers session
- Project-scope transitions should be centralized in dedicated transition helpers rather than spread throughout slice methods.
- Derived presentation state should live in feature model modules instead of growing controller hooks.
- Feature-local UI state should remain local only when it is not operator-meaningful across top-level navigation.
- If losing that state on tab switch would be a UX regression, it belongs in an explicit session slice rather than component-local mount state.

## Guardrails

- Enforce boundaries with import and usage rules, not arbitrary file-size limits.
- Keep behavior-neutral refactors behavior-neutral: reorganize ownership and layering without changing backend contracts or user-visible requirements.
