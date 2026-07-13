---
id: CR-2026-0077-editor-canvas-toolbar-polish
title: Editor Canvas Toolbar Polish
status: completed
type: refactor
changelog: public
---

## Summary

The editor canvas controls were consolidated into a compact grouped toolbar. The shipped UI keeps the existing top-left canvas placement while grouping view, layout, and run actions into a single translucent surface with accessible group labels and pressed-state semantics.

## Validation

Repository tests were updated around observable toolbar behavior, including accessible names, `aria-pressed` states, action visibility in expanded read-only mode, run disabled messaging, and existing editor layout/loading interactions. The request's full validation gate was the intended validation path, but no command output for that gate is recorded in the change-request runtime state read during result recording.

## Shipped Changes

- Added `frontend/src/features/editor/components/EditorCanvasToolbar.tsx` for the structured/YAML, parent/expanded, arrange, reset, add-node, and run controls.
- Updated `frontend/src/features/editor/Editor.tsx` to render the new toolbar and keep performance/read-only notices wrapped below it.
- Added `frontend/src/features/editor/__tests__/EditorCanvasToolbar.test.tsx` for focused toolbar behavior.
- Updated existing editor flow-loading and layout behavior tests to use the shipped accessible labels: `YAML`, `Parent`, `Expanded`, `Arrange`, `Reset`, and `+ Node`.
