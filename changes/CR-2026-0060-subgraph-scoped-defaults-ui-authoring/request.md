# Subgraph And Scoped Defaults UI Authoring

## Summary
Add first-class structured UI authoring for Attractor subgraphs and scoped `node[...]` / `edge[...]` defaults so users can inspect, create, edit, and preserve these DOT constructs without falling back to raw DOT editing. This closes the `subgraph_and_scoped_defaults_ui_authoring` migration gap.

## Background
The Rust rewrite already parses, previews, transforms, and serializes subgraphs and defaults through the canonical flow model. Existing frontend contracts prove unsurfaced subgraphs/defaults are preserved through structured and raw edit paths. The remaining gap is product/editor completeness: the structured UI does not yet provide first-class authoring controls for:

- graph-level node defaults
- graph-level edge defaults
- subgraph creation/editing
- subgraph graph attributes such as `label`
- subgraph membership via node IDs
- scoped subgraph `node[...]` defaults
- scoped subgraph `edge[...]` defaults
- nested subgraph inspection/preservation

## Required Behavior
- Add structured editor controls for graph-level scoped defaults:
  - `node[...]` defaults
  - `edge[...]` defaults
- Add structured editor controls for subgraphs:
  - list existing subgraphs
  - create a subgraph with a stable id
  - edit subgraph id and graph attributes such as `label`
  - edit subgraph node membership
  - edit scoped node defaults and edge defaults
  - inspect nested subgraphs without dropping them
- Preserve existing canonical behavior:
  - raw DOT subgraphs/defaults round-trip through structured edits
  - unknown extension attributes remain preserved
  - nested subgraphs remain preserved even if the first pass only exposes limited controls for nesting
  - generated DOT remains valid and stable
- Make the UI behavior clear and bounded:
  - controls should live in existing editor/inspector surfaces where they fit naturally
  - avoid requiring users to edit raw DOT for the supported subgraph/default fields
  - validation diagnostics should still surface if users produce invalid graph structure
- Update docs to describe the new structured authoring surface.
- Update migration/TODO/coverage records so `subgraph_and_scoped_defaults_ui_authoring` is no longer an open policy gap when implementation is complete.

## Non-Goals
- Do not change DOT parser, transform, or runtime semantics unless a bug is found while wiring the UI.
- Do not implement the final post-gap spec/API drift audit.
- Do not remove deprecated compatibility routes.
- Do not redesign the whole editor layout.
- Do not require raw DOT editing for the supported fields above, but keep raw DOT mode available.

## Suggested Target Paths
- `frontend/src/lib/canonicalFlowModel.ts`
- `frontend/src/lib/dotUtils.ts`
- `frontend/src/features/editor/Editor.tsx`
- `frontend/src/features/editor/Sidebar.tsx`
- `frontend/src/features/editor/components/GraphInspectorPanel.tsx`
- `frontend/src/features/editor/components/graph-settings/GraphSettingsSections.tsx`
- `frontend/src/features/workflow-canvas/flowCanvasShared.ts`
- `frontend/src/__tests__/ContractBehavior.test.tsx`
- `frontend/src/features/editor/__tests__/*`
- `tests/contracts/frontend/test_canonical_flow_model_contracts.py`
- `tests/contracts/frontend/test_raw_dot_baseline_fixtures.py`
- `tests/fixtures/reference-1.1-03-subgraph-defaults.dot`
- `src/spark/guides/dot-authoring.md`
- `TODO.md`
- `.spark/rust-rewrite/current/migration-records.json`
- `.spark/rust-rewrite/current/validation/requirement-decision-coverage-review.json`

## Tests
- Add frontend unit tests proving users can edit graph-level node/edge defaults through structured controls and save valid DOT.
- Add frontend unit tests proving users can create/edit a subgraph, set label/attrs, assign node membership, edit scoped node/edge defaults, save, and reopen without data loss.
- Add regression coverage proving existing raw DOT subgraph/default preservation still works.
- Add contract tests proving canonical model serialization still preserves nested subgraphs and unknown attrs.
- Add migration/coverage hygiene tests so the gap closes only when executable UI evidence exists.
- Run focused validation:
  - `npm --prefix frontend run test:unit -- --run src/__tests__/ContractBehavior.test.tsx`
  - `uv run pytest -q -x --maxfail=1 tests/contracts/frontend/test_canonical_flow_model_contracts.py tests/contracts/frontend/test_raw_dot_baseline_fixtures.py`
  - `uv run pytest -q -x --maxfail=1 tests/repo_hygiene/test_rust_rewrite_migration_records.py tests/repo_hygiene/test_rust_rewrite_requirement_decision_coverage.py`
- Final verification: `uv run pytest -q`.

## Acceptance Criteria
- Users can author graph-level `node[...]` and `edge[...]` defaults from structured UI controls.
- Users can author at least one subgraph, including id, label/attrs, node membership, and scoped node/edge defaults, from structured UI controls.
- Existing subgraphs/defaults loaded from DOT are visible in structured UI and persist across save/reopen.
- Nested subgraphs and unknown extension attributes are preserved even if only top-level/narrow controls are exposed initially.
- `subgraph_and_scoped_defaults_ui_authoring` is removed from open policy gaps and recorded as closed by implementation evidence.
- The change result documents shipped behavior and validation.
