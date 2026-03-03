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
| Run initiation payload and policy banners | `frontend/src/components/Navbar.tsx` | Shows project, flow source, working directory, backend/model, plus explicit warning/error banners before and during run launch. | `ui-spec.md` 4.1, 8.1, 8.2 |
| Flow create/delete/select controls | `frontend/src/components/Sidebar.tsx` | Create/select/delete flow with explicit user actions; deletion requires confirmation. | `ui-spec.md` 3.2, 5.2, 2 (principle 5) |
| Add Node button | `frontend/src/components/Editor.tsx` | Adds a new editable node in editor mode and persists graph changes through the same save path used for other edits. | `ui-spec.md` 5.2, 5.3, 6.2 |
| Raw DOT mode toggle and handoff diagnostics | `frontend/src/components/Editor.tsx` | Allows structured/raw editing mode switches while surfacing parse/handoff errors before applying raw changes back to graph state. | `ui-spec.md` 5.2, 5.3, 7.1 |
| Graph settings drawer | `frontend/src/components/GraphSettings.tsx` | Edits graph/run attributes through structured fields and persists via save path; advanced settings remain available in UI. | `ui-spec.md` 5.2, 6.1, 6.5, 2 (principles 1 and 3) |
| Graph/node tool hook fields | `frontend/src/components/GraphSettings.tsx`, `frontend/src/components/TaskNode.tsx` | Exposes `tool_hooks.pre` and `tool_hooks.post` fields with warnings for malformed values while preserving explicit override behavior. | `ui-spec.md` 6.6, 7.1 |
| Apply To Nodes button | `frontend/src/components/GraphSettings.tsx` | Applies current flow-level LLM defaults to all nodes only when editor context is active and a flow is selected. | `ui-spec.md` 5.1, 5.2, 6.2 |
| Reset From Global button | `frontend/src/components/GraphSettings.tsx` | Resets flow-scoped LLM defaults from global settings without mutating runtime semantics outside configured default fields. | `ui-spec.md` 4, 11.5, 12 |
| Node inspector fields | `frontend/src/components/Sidebar.tsx`, `frontend/src/components/TaskNode.tsx` | Node properties are editable with progressive disclosure for advanced fields; edits are reflected in local model and persisted. | `ui-spec.md` 5.1, 5.2, 6.2, 2 (principles 3 and 6) |
| Node quick-edit controls | `frontend/src/components/TaskNode.tsx` | Supports inline node label edit and a focused node detail editor (`Edit` control) with the same node attribute semantics as sidebar editing. | `ui-spec.md` 5.1, 5.2, 6.2 |
| Manager loop configuration controls | `frontend/src/components/Sidebar.tsx`, `frontend/src/components/TaskNode.tsx` | Supports manager-loop field authoring (`manager.*`) and child-linkage guidance tied to graph-level `stack.child_*` attributes. | `ui-spec.md` 6.7, 5.2 |
| Human default choice controls | `frontend/src/components/Sidebar.tsx`, `frontend/src/components/TaskNode.tsx` | Exposes `human.default_choice` in node editing so timeout/default behavior remains explicit and reviewable in authoring flows. | `ui-spec.md` 6.2, 10.3 |
| Edge inspector fields | `frontend/src/components/Sidebar.tsx` | Edge attributes (`label`, `condition`, `weight`, `fidelity`, `thread_id`, `loop_restart`) remain editable and persisted. | `ui-spec.md` 5.2, 6.3, Appendix A.3 |
| Subgraph/default block controls | `frontend/src/components/Editor.tsx`, `frontend/src/components/Sidebar.tsx` | Subgraph membership, labels, and scoped node/edge defaults must be first-class editable controls rather than raw DOT-only operations. | `ui-spec.md` 6.4, 5.2 |
| Validation edge diagnostic badge | `frontend/src/components/ValidationEdge.tsx` | Mirrors edge-level diagnostic severity directly on canvas edges so operators can inspect issues without leaving graph context. | `ui-spec.md` 7.1, 7.3 |
| Validation panel entries | `frontend/src/components/ValidationPanel.tsx` | Diagnostics are visible, severity-tagged, and selectable to focus graph entities; error diagnostics block run start. | `ui-spec.md` 7.1, 7.2, 7.3 |
| Inspector empty state scaffold | `frontend/src/components/InspectorScaffold.tsx` | Provides deterministic empty-state and loading shells so graph/node/edge inspector behavior stays consistent across selection changes. | `ui-spec.md` 4.1, 5.1, 5.2 |
| Human gate discoverability indicators | `frontend/src/components/Sidebar.tsx`, `frontend/src/components/RunsPanel.tsx` | Keeps pending-human status discoverable from authoring and run views so operators can locate blocked work without raw logs. | `ui-spec.md` 10.1, 9.6 |
| Explainability panel controls | `frontend/src/components/ExplainabilityPanel.tsx` | Shows routing/retry/failure reasoning from live runtime events with clear fallback messaging when no events are available. | `ui-spec.md` 2 (principle 4), 9.4 |
| Canvas controls (pan/zoom/fit/minimap) | `frontend/src/components/Editor.tsx` | Maintains direct-manipulation canvas navigation controls with consistent graph context while editing and inspecting. | `ui-spec.md` 4.1, 5.2 |
| Stylesheet editor controls | `frontend/src/components/StylesheetEditor.tsx` | Supports graph stylesheet authoring with lint/diagnostic feedback and preview-oriented editing affordances. | `ui-spec.md` 6.5, 7.1 |
| Run history refresh/open/cancel actions | `frontend/src/components/RunsPanel.tsx` | Supports run list refresh, run selection/open, and cancel requests with explicit operator action. | `ui-spec.md` 8.2, 9.1, 9.6 |
| Run checkpoint viewer controls | `frontend/src/components/RunsPanel.tsx` | Exposes checkpoint retrieval and rendering controls for current/completed nodes and retry state with missing-data error messaging. | `ui-spec.md` 9.2, 9.1 |
| Run context inspector controls | `frontend/src/components/RunsPanel.tsx` | Provides searchable project/run context inspection with typed scalar/object rendering and copy/export affordances. | `ui-spec.md` 9.3, 9.1 |
| Run artifact browser controls | `frontend/src/components/RunsPanel.tsx` | Lists run artifacts, supports graph render/file view actions, and surfaces partial/missing artifact states without hidden failures. | `ui-spec.md` 9.5, 9.6 |
| Run stream panel controls | `frontend/src/components/RunStream.tsx` | Renders live event/log stream with stable run association during active execution and run inspection. | `ui-spec.md` 8.3, 8.4, 9.4 |
| Execution footer cancel control | `frontend/src/components/ExecutionControls.tsx` | Keeps runtime status/control visible during execution and requires confirmation before cancel request. | `ui-spec.md` 8.2, 8.4, 2 (principle 5) |
| Execution footer unsupported pause/resume reason | `frontend/src/components/ExecutionControls.tsx` | Shows explicit disabled-state reason text for unsupported pause/resume controls so unavailable runtime actions are non-silent. | `ui-spec.md` 8.2, 8.4 |
| Terminal clear action | `frontend/src/components/Terminal.tsx` | Allows explicit operator clearing of runtime log stream without changing runtime state. | `ui-spec.md` 8.4, 9.4 |
| Global default settings controls | `frontend/src/components/SettingsPanel.tsx` | Maintains global default model/provider/reasoning settings for new flow snapshots. | `ui-spec.md` 4, 11.5, 12 |
| Projects workspace controls | `frontend/src/components/ProjectsPanel.tsx` | Maintains project registration/selection context and project-scoped workflow entry points without cross-project leakage. | `ui-spec.md` 4.2, 4.3, 5.4 |
| Project AI conversation controls | `frontend/src/components/ProjectsPanel.tsx` | Supports start/continue conversation actions and visible project-scoped history for iterative spec collaboration. | `ui-spec.md` 5.5, 4.2 |
| Project spec proposal review controls | `frontend/src/components/ProjectsPanel.tsx` | Provides preview/apply/reject actions for proposed spec edits so mutation is always explicit and auditable. | `ui-spec.md` 5.5, 2 (principle 5) |
| Project plan generation controls | `frontend/src/components/ProjectsPanel.tsx` | Gates plan-generation launch behind approved spec state and surfaces planning status/error outcomes in project scope. | `ui-spec.md` 8.5, 4.2 |
| Human prompt question-type controls | `frontend/src/components/RunStream.tsx`, `frontend/src/components/RunsPanel.tsx` | Supports explicit render/response affordances for `MULTIPLE_CHOICE`, `YES_NO`, `CONFIRMATION`, and `FREEFORM` prompt types. | `ui-spec.md` 10.2, 10.1 |
| Grouped multi-question/inform controls | `frontend/src/components/RunStream.tsx`, `frontend/src/components/RunsPanel.tsx` | Handles grouped question payloads and informational prompt events with clear per-question status and operator actions. | `ui-spec.md` 10.4, 10.1 |

## Spec references used during control behavior decisions

When control behavior is ambiguous, use this order:

1. `ui-spec.md` section-specific requirement for the active area.
2. `ui-spec.md` section `2` design principles.
3. `attractor-spec.md` for runtime/parser semantics.

Implementation and tests should cite the section(s) above when adding or modifying controls.
