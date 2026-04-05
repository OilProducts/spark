# Frontend Architecture

## Intent

- Keep implementation organized around feature modules, not a generic shared component grab bag.
- Use `src/ui` as the canonical shared primitive and surface layer for non-canvas UI.
- Keep workflow canvas rendering custom under `src/features/workflow-canvas`.
- Treat this as an internal frontend architecture document. It does not change backend APIs, persisted contracts, or product behavior.

## Top-Level Layout

- `src/ui`
  - Canonical shared non-canvas primitives and reusable surface helpers.
  - Safe to import from any feature.
  - Shared dialog flows should use `DialogProvider` and `useDialogController`, not browser `alert`, `confirm`, or `prompt`.
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

There is no generic `src/components` layer. New implementation code belongs in `src/features/*` or `src/ui`.

## Feature Internals

- `src/features/<feature>/components`
  - Presentational feature components and local composition surfaces.
  - May import `@/ui/*`, local feature model types, and store selectors.
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

- Non-canvas feature UI should compose `@/ui/*` primitives instead of hand-rolled raw controls.
- Shared primitives cover controls such as buttons, inputs, selects, tabs, badges, cards, scroll areas, textareas, labels, separators, dialogs, tooltips, switches, and checkboxes.
- Shared surface helpers such as `Panel`, `SectionHeader`, `FieldRow`, `InlineNotice`, and `EmptyState` are the default building blocks for repeated non-canvas surfaces. When a pattern repeats across features, formalize it in `src/ui` instead of re-implementing it locally.
- Canvas renderers are the exception. Inspector chrome, sidebars, forms, and other non-canvas surfaces are not exempt.

## Import Boundaries

- Presentational feature components must not import API clients directly.
- API client usage is limited to feature hooks and services.
- Presentational components may read store selectors, but store mutation and cross-feature coordination should stay in hooks, services, or top-level composition files.
- Non-canvas feature components should import shared controls from `@/ui/*`.

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
