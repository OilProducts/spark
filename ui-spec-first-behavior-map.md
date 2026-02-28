# Spec-First Behavior Mapping for UI Controls

Checklist item: [2-01]

This document maps current UI controls to their expected behavior with direct references to `ui-spec.md` and `attractor-spec.md`. If implementation and spec ever diverge, spec behavior wins.

## Source of Truth

- `ui-spec.md` section `2. Design Principles`:
  - Principle 1: spec-first behavior.
  - Principle 5: operational safety.
  - Principle 6: consistency across graph/node/edge editing.
- `ui-spec.md` sections `4` through `10` define required UI behavior by area.
- `attractor-spec.md` defines DOT/runtime semantics that the UI must preserve.

## Control-to-Spec Behavior Map

| UI control | Current location | Expected behavior | Spec references |
| --- | --- | --- | --- |
| Top navigation mode switch (Editor/Execution/Settings/Runs) | `frontend/src/components/Navbar.tsx` | Switches first-class area without losing global app shell context; active mode remains visible. | `ui-spec.md` 4, 4.1 |
| Execute button | `frontend/src/components/Navbar.tsx` | Starts pipeline only when an active flow is selected and validation is not erroring; surfaces visible failure on run start errors. | `ui-spec.md` 5.3, 7.2, 8.1 |
| Flow create/delete/select controls | `frontend/src/components/Sidebar.tsx` | Create/select/delete flow with explicit user actions; deletion requires confirmation. | `ui-spec.md` 3.2, 5.2, 2 (principle 5) |
| Add Node button | `frontend/src/components/Editor.tsx` | Adds a new editable node in editor mode and persists graph changes through the same save path used for other edits. | `ui-spec.md` 5.2, 5.3, 6.2 |
| Graph settings drawer | `frontend/src/components/GraphSettings.tsx` | Edits graph/run attributes through structured fields and persists via save path; advanced settings remain available in UI. | `ui-spec.md` 5.2, 6.1, 6.5, 2 (principles 1 and 3) |
| Apply To Nodes button | `frontend/src/components/GraphSettings.tsx` | Applies current flow-level LLM defaults to all nodes only when editor context is active and a flow is selected. | `ui-spec.md` 5.1, 5.2, 6.2 |
| Reset From Global button | `frontend/src/components/GraphSettings.tsx` | Resets flow-scoped LLM defaults from global settings without mutating runtime semantics outside configured default fields. | `ui-spec.md` 4, 11.5, 12 |
| Node inspector fields | `frontend/src/components/Sidebar.tsx`, `frontend/src/components/TaskNode.tsx` | Node properties are editable with progressive disclosure for advanced fields; edits are reflected in local model and persisted. | `ui-spec.md` 5.1, 5.2, 6.2, 2 (principles 3 and 6) |
| Node quick-edit controls | `frontend/src/components/TaskNode.tsx` | Supports inline node label edit and a focused node detail editor (`Edit` control) with the same node attribute semantics as sidebar editing. | `ui-spec.md` 5.1, 5.2, 6.2 |
| Edge inspector fields | `frontend/src/components/Sidebar.tsx` | Edge attributes (`label`, `condition`, `weight`, `fidelity`, `thread_id`, `loop_restart`) remain editable and persisted. | `ui-spec.md` 5.2, 6.3, Appendix A.3 |
| Validation panel entries | `frontend/src/components/ValidationPanel.tsx` | Diagnostics are visible, severity-tagged, and selectable to focus graph entities; error diagnostics block run start. | `ui-spec.md` 7.1, 7.2, 7.3 |
| Canvas controls (pan/zoom/fit/minimap) | `frontend/src/components/Editor.tsx` | Maintains direct-manipulation canvas navigation controls with consistent graph context while editing and inspecting. | `ui-spec.md` 4.1, 5.2 |
| Run history refresh/open/cancel actions | `frontend/src/components/RunsPanel.tsx` | Supports run list refresh, run selection/open, and cancel requests with explicit operator action. | `ui-spec.md` 8.2, 9.1, 9.6 |
| Execution footer cancel control | `frontend/src/components/ExecutionControls.tsx` | Keeps runtime status/control visible during execution and requires confirmation before cancel request. | `ui-spec.md` 8.2, 8.4, 2 (principle 5) |
| Terminal clear action | `frontend/src/components/Terminal.tsx` | Allows explicit operator clearing of runtime log stream without changing runtime state. | `ui-spec.md` 8.4, 9.4 |
| Global default settings controls | `frontend/src/components/SettingsPanel.tsx` | Maintains global default model/provider/reasoning settings for new flow snapshots. | `ui-spec.md` 4, 11.5, 12 |

## Spec references used during control behavior decisions

When control behavior is ambiguous, use this order:

1. `ui-spec.md` section-specific requirement for the active area.
2. `ui-spec.md` section `2` design principles.
3. `attractor-spec.md` for runtime/parser semantics.

Implementation and tests should cite the section(s) above when adding or modifying controls.
