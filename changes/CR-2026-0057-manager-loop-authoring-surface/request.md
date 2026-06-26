# Manager-Loop Authoring Surface Completion

## Summary
Make the implemented `stack.manager_loop` authoring surface first-class in Rust-backed Spark wherever users inspect, preview, edit, serialize, and document flow nodes. Runtime support already exists for `manager.steer_cooldown` and `stack.child_autostart`; this change closes the remaining product/editor surface gap without redesigning manager-loop execution.

## Background
The Rust rewrite final conformance record still lists `manager_loop_authoring_surface_completeness` as an open policy gap. Runtime behavior and documentation already recognize:

- `manager.poll_interval`
- `manager.max_cycles`
- `manager.stop_condition`
- `manager.actions`
- `manager.steer_cooldown`
- `stack.child_autostart`

The current frontend/canonical authoring path treats only the first four as first-class manager-loop fields. `manager.steer_cooldown` and `stack.child_autostart` must be promoted to the same level of support.

## Required Behavior
- Flow preview/API payloads expose the full manager-loop authoring surface, including `manager.steer_cooldown` and `stack.child_autostart`.
- Canonical frontend flow modeling preserves and serializes `manager.steer_cooldown` and `stack.child_autostart` for manager-loop nodes.
- Sidebar inspector and canvas task-node authoring controls allow users to inspect and edit:
  - `manager.steer_cooldown` as a duration string.
  - `stack.child_autostart` as a boolean/toggle-style setting.
- Existing manager-loop fields continue to round-trip unchanged.
- Invalid or blank optional values should be omitted consistently with existing manager-loop authoring behavior.
- Documentation/spec surfaces describe the same full authoring contract.
- Once implemented and validated, remove or update TODO/migration references that classify manager-loop authoring surface completeness as an open policy gap.

## Non-Goals
- Do not change manager-loop runtime semantics.
- Do not add new manager-loop attributes beyond the implemented surface above.
- Do not redesign automatic steering, telemetry ingestion, or child-run observation behavior.
- Do not close `manager_loop_telemetry_ingestion`; that is a separate blocker.

## Suggested Target Paths
- `crates/attractor-dsl/src/preview.rs`
- `crates/attractor-dsl/tests/preview_contracts.rs`
- `frontend/src/lib/canonicalFlowModel.ts`
- `frontend/src/features/editor/Sidebar.tsx`
- `frontend/src/features/editor/components/NodeInspectorPanel.tsx`
- `frontend/src/features/workflow-canvas/flowCanvasShared.ts`
- `frontend/src/features/workflow-canvas/TaskNode.tsx`
- `frontend/src/__tests__/ContractBehavior.test.tsx`
- `frontend/src/features/editor/__tests__/InspectorAndNodeAuthoring.test.tsx`
- `src/spark/guides/dot-authoring.md`
- `specs/attractor-spec.md`
- `TODO.md`
- `.spark/rust-rewrite/current/migration-records.json`
- `.spark/rust-rewrite/current/validation/requirement-decision-coverage-review.json`

## Tests
- Add or update backend preview tests proving `manager.steer_cooldown` and `stack.child_autostart` appear in manager-loop preview payloads.
- Add or update frontend tests proving inspector/canvas authoring renders, edits, preserves, and serializes the two fields.
- Add or update migration/coverage tests so `manager_loop_authoring_surface_completeness` no longer counts as an open parity gap after implementation.
- Run focused validation for touched backend/frontend/repo-hygiene tests.
- Final verification: `uv run pytest -q`.

## Acceptance Criteria
- A manager-loop flow containing `manager.steer_cooldown` and `stack.child_autostart` can be previewed, edited in the UI authoring surfaces, serialized back to DOT, and reloaded without dropping either attribute.
- Existing manager-loop authoring behavior remains compatible.
- `manager_loop_authoring_surface_completeness` is removed from open policy gaps and recorded as closed by implementation evidence.
- The change result documents shipped surfaces and validation.
