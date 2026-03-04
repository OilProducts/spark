# Attractor UI Implementation Checklist

Companion to `/Users/chris/tinker/sparkspawn/ui-spec.md`.

Use this as the execution plan and verification ledger for full UI spec coverage. Tasks are ordered to match the UI spec section/subsection sequence.

Status key:
- `[ ]` not started
- `[~]` in progress
- `[x]` complete

---

## 1. Overview and Goals

### 1.1 Problem Statement
- [x] [1.1-01] Document all places where users must leave the UI to raw DOT for required configuration. (See `ui-raw-dot-required-config.md`.)
- [x] [1.1-02] Add a parity-risk report that identifies current behavior-loss and hidden-config failure modes. (See `ui-parity-risk-report.md`.)
- [x] [1.1-03] Add a baseline fixture set of spec-valid flows that currently require raw DOT edits.

### 1.2 Product Goal
- [x] [1.2-01] Define the parity-complete user journey (project-select, author, execute, inspect) as an end-to-end acceptance script.
- [x] [1.2-03] Add release criteria that fail when any required spec feature is unavailable in UI.

### 1.3 Non-Goals
- [x] [1.3-01] Document runtime/parser boundaries so UI work does not alter engine semantics.
- [x] [1.3-02] Add safeguards that prevent invalid DOT synthesis from form states.
- [x] [1.3-03] Keep advanced feature access enabled (no simplified mode removing required spec controls).

---

## 2. Design Principles

- [x] [2-01] Implement spec-first behavior mapping for each UI control with direct spec references. (See `ui-spec-first-behavior-map.md`.)
- [x] [2-02] Add no-silent-loss save protections and user-visible failure states.
- [x] [2-03] Ensure progressive disclosure for advanced fields while preserving full editability.
- [x] [2-04] Add explainability views for routing, retry, and failure decisions.
- [x] [2-05] Add confirmations and clear status transitions for destructive/operational actions.
- [x] [2-06] Standardize graph/node/edge inspector interaction patterns.

---

## 3. Users and Primary Workflows

### 3.1 User Roles
- [x] [3.1-01] Capture author/operator/reviewer/project-owner persona scenarios with concrete UI success criteria. (See `ui-role-persona-scenarios.md`.)

### 3.2 Primary Workflows
- Deferred to `Deferred Tasks` until project-scoped conversation/spec/plan/build surfaces exist.

---

## 4. Information Architecture

- [x] [4-01] Ensure Projects, Editor, Execution, Runs, and Settings are first-class areas with stable navigation.
- [x] [4-03] Add route restoration behavior on refresh/reopen.

### 4.1 Global Regions
- [x] [4.1-01] Keep top navigation persistent across modes with active-project, active-flow, and run action context.
- [x] [4.1-02] Keep canvas workspace as primary interaction zone in editor/execution modes.
- [x] [4.1-03] Keep inspector panel context-driven for graph/node/edge selection.
- [x] [4.1-04] Keep execution footer/stream visible and consistent during active runs.

### 4.2 Project Scope and Invariants
- [x] [4.2-01] Implement project registry keyed by unique directory path and reject duplicate registrations.
- [x] [4.2-03] Enforce exactly one active project for authoring/execution actions.
- [x] [4.2-04] Scope conversations/specs/plans/runs/artifacts to active project boundaries.
- [x] [4.2-05] Prevent cross-project context and file leakage across navigation and run transitions.
- [x] [4.2-06] Expose project-scoped conversation/spec/plan entry points in the Projects area.

### 4.3 Projects Workspace Requirements
- [x] [4.3-01] Implement create/register project UX from local directory path.
- [x] [4.3-02] Implement duplicate-path prevention on project create/update.
- [x] [4.3-04] Implement persistent active-project indicator in top navigation.
- [x] [4.3-05] Implement recent/favorite project switching UX.
- [x] [4.3-06] Implement glanceable project metadata (`name`, `directory`, current branch, last activity timestamp).
- [x] [4.3-07] Add deep-link and restore tests for active-project identity.

---

## 5. Core Interaction Model

### 5.1 Selection and Editing
- [x] [5.1-01] Enforce single-select semantics for nodes/edges across canvas and inspector.
- [x] [5.1-02] Sync selection state bidirectionally between canvas and inspector.
- [x] [5.1-03] Reflect unsaved edits immediately in local graph model and diagnostics.

### 5.2 Editing Modalities
- [x] [5.2-01] Implement direct manipulation for node move/connect/select with persistence hooks.
- [x] [5.2-02] Implement structured form editing for graph/node/edge properties.
- [x] [5.2-03] Implement raw DOT mode with safe handoff back to structured editing.

### 5.3 Save Semantics
- [x] [5.3-01] Implement explicit save state indicator (saved/saving/error/conflict).
- [x] [5.3-02] Surface save failures with actionable remediation and no silent drop.
- [x] [5.3-03] Add semantic-equivalence checks for output DOT where user behavior did not change.

### 5.4 Project-Scoped Context and Isolation
- [x] [5.4-01] Require an active project for mutating flow edits and run start actions.
- [x] [5.4-03] Reset flow/run/conversation selections correctly on project switch with no hidden state carryover.

### 5.5 AI Conversation and Spec Authoring Loop
- [x] [5.5-01] Implement project-scoped AI conversation surface for start/continue flows.
- [x] [5.5-02] Persist and render conversation history per project.
- [x] [5.5-03] Implement explicit proposal UX for AI-generated spec edits (preview before apply).
- [x] [5.5-04] Require explicit user confirmation before applying proposed spec edits.
- [x] [5.5-05] Ensure reject actions do not mutate spec files.
- [x] [5.5-06] Add tests proving proposal artifacts and conversation context remain project-isolated.

---

## 6. Authoring Surface Requirements

### 6.1 Graph-Level Attribute Editing
- [x] [6.1-01] Add graph editor fields for all required graph attrs (including `stack.child_*`, `tool_hooks.*`).
- [x] [6.1-02] Add type-aware validation and normalization for graph attrs.
- [x] [6.1-03] Add inline help and precedence notes for graph attrs.
- [x] [6.1-04] Add tests that graph-level edits serialize correctly to DOT and rehydrate cleanly.

### 6.2 Node Authoring and Editing
- [x] [6.2-01] Add full handler type support including `stack.manager_loop` authoring controls.
- [x] [6.2-02] Add complete advanced node attributes editor for all spec node attrs.
- [x] [6.2-03] Support shape-derived defaults with explicit `type` override behavior.
- [x] [6.2-04] Add tests for node attribute round-trip across all handler types.

### 6.3 Edge Authoring and Editing
- [x] [6.3-01] Add complete edge attributes editor (`label`, `condition`, `weight`, `fidelity`, `thread_id`, `loop_restart`).
- [x] [6.3-02] Add condition syntax hints and preview validation feedback.
- [x] [6.3-03] Add tests for edge attr serialization and execution-side effect visibility.

### 6.4 Subgraphs and Default Blocks
- Deferred to `Deferred Tasks` until subgraph/default-scope structures are represented in the frontend flow model and DOT serializer.

### 6.5 Stylesheet Authoring
- [x] [6.5-01] Implement stylesheet editor with syntax highlighting.
- [x] [6.5-02] Add parse/lint diagnostics for stylesheet grammar and selectors.
- [x] [6.5-03] Add selector matching preview and effective per-node value preview.
- [x] [6.5-04] Add tests for precedence rendering (node attr > stylesheet > graph default).

### 6.6 Tool Hook Authoring
- [x] [6.6-01] Add UI for `tool_hooks.pre` and `tool_hooks.post` at graph scope.
- [x] [6.6-02] Add node-level override UX if supported by parser/runtime contract.
- [x] [6.6-03] Add validation and warnings for malformed hook command values.
- [x] [6.6-04] Add save/load tests for hook definitions.

### 6.7 Manager Loop Authoring
- [x] [6.7-01] Expose `house` shape and `stack.manager_loop` type as selectable options.
- [x] [6.7-02] Add manager fields (`manager.poll_interval`, `manager.max_cycles`, `manager.stop_condition`, `manager.actions`).
- [x] [6.7-03] Link manager node config to `stack.child_*` graph attributes in UI workflow.
- [x] [6.7-04] Add fixtures and tests for manager-loop authoring round-trip.

---

## 7. Validation and Diagnostics UX

### 7.1 Diagnostic Surfaces
- [x] [7.1-01] Implement centralized validation panel with filtering/sorting.
- [x] [7.1-02] Implement inline node and edge diagnostic badges.
- [x] [7.1-03] Implement field-level diagnostic mapping in inspectors.

### 7.2 Blocking Rules
- [x] [7.2-01] Block run start on error-level diagnostics.
- [x] [7.2-02] Allow run start on warning-only state with explicit warning banner.
- [x] [7.2-03] Add tests for blocking/unblocking transitions as diagnostics change.

### 7.3 Navigability
- [x] [7.3-01] Clicking a diagnostic must select and focus corresponding graph entity.
- [x] [7.3-02] Add fallback UX for diagnostics without direct element mapping.
- [x] [7.3-03] Add integration tests for diagnostic-to-canvas navigation.

---

## 8. Execution and Control UX

### 8.1 Run Initiation
- [x] [8.1-01] Ensure run form captures active project identity, flow source, working directory, backend/model.
- [x] [8.1-02] Ensure payload parity with `/pipelines` request contract.
- [x] [8.1-03] Add failure handling UI for rejected run start requests.
- [x] [8.1-04] Default working directory to active project directory unless user overrides.
- [x] [8.1-05] Add run-start policy gate/warning path for project Git-state violations.

### 8.2 Runtime Controls
- [x] [8.2-01] Provide Start and Cancel controls for supported backend behavior.
- Deferred to `Deferred Tasks` until backend runtime control API exposes pause/resume capability.
- [x] [8.2-03] Show disabled reason text for unsupported controls.
- [x] [8.2-04] Add tests for control enable/disable state transitions.

### 8.3 Live State Updates
- [x] [8.3-01] Stream and render live node status transitions from runtime events.
- [x] [8.3-02] Resolve event ordering/race handling for rapid transitions.
- [x] [8.3-03] Add tests for state reset and run switching behavior.

### 8.4 Execution Footer
- [x] [8.4-01] Keep execution controls/status visible in canvas footer during active runs.
- [x] [8.4-02] Ensure footer reflects current run identity and terminal state.

### 8.5 Spec -> Plan -> Build Workflow Orchestration
- [x] [8.5-01] Implement plan-generation workflow launch from approved project spec state.
- Deferred to `Deferred Tasks` until project-scoped plan artifact persistence/retrieval APIs exist.
- [x] [8.5-03] Implement plan gate controls (`approve`, `reject`, `request-revision`) with clear state transitions.
- [x] [8.5-04] Enforce build workflow launch from approved plan state only.
- [x] [8.5-05] Implement planning/build failure diagnostics with rerun affordances.
  - Evidence: Evaluator marked this `pass` with UI-smoke visual QA covering enabled/disabled rerun states for planning and build failures (`20a`-`20d` screenshots).
- [x] [8.5-06] Ensure live status/log/artifact surfaces support both planning and build workflows.
  - Evidence: Shared execution-footer status/log/artifact surfaces now serve planning + build launches (`tests/integration/test_ui_plan_build_live_surfaces.py`), with UI-smoke visual evidence in `frontend/artifacts/ui-smoke/06-execution-panel.png`.

---

## 9. Run Inspector Requirements

### 9.1 Summary
- [x] [9.1-01] Build run summary panel with status/result/time/model/working-dir/project/git-metadata/error/tokens.
  - Evidence (2026-03-03): Evaluator `pass`; populated summary runtime assertions added in UI smoke and visual QA captured in `frontend/artifacts/ui-smoke/08b-runs-panel-populated-summary.png`.
- [x] [9.1-02] Add run metadata refresh behavior and stale-state indicators.
  - Evidence (2026-03-03): Evaluator `pass`; `RunsPanel` now surfaces freshness/stale indicators with refresh wiring, covered by helper + panel integration tests and UI-smoke scenario `08c-runs-panel-refresh-stale-indicator.png`.

### 9.2 Checkpoint View
- [x] [9.2-01] Add checkpoint viewer backed by `/pipelines/{id}/checkpoint`.
  - Evidence (2026-03-03): Evaluator `pass`; `RunsPanel` now fetches and renders checkpoint payloads with refresh control, covered by integration guard tests and UI-smoke screenshot `frontend/artifacts/ui-smoke/08d-runs-panel-checkpoint-viewer.png`.
- [x] [9.2-02] Render current node, completed nodes, retry counters, and raw checkpoint payload.
  - Evidence (2026-03-03): Evaluator `pass`; `RunsPanel` now renders checkpoint progress fields (`current_node`, `completed_nodes`, `retry_counts`) above raw payload JSON, covered by integration guard test `test_runs_panel_renders_checkpoint_progress_fields_item_9_2_02` and UI-smoke visual QA in `frontend/artifacts/ui-smoke/08d-runs-panel-checkpoint-viewer.png`.
- [x] [9.2-03] Add missing/unavailable checkpoint error handling UX.
  - Evidence (2026-03-03): Evaluator `pass`; `RunsPanel` now renders explicit degraded-state UX for missing/unavailable checkpoints with refresh retry, covered by `tests/integration/test_ui_run_checkpoint_panel.py::test_runs_panel_adds_checkpoint_unavailable_error_handling_item_9_2_03` and UI-smoke screenshot `frontend/artifacts/ui-smoke/08e-runs-panel-checkpoint-unavailable.png`.

### 9.3 Context View
- [x] [9.3-01] Add searchable context key/value inspector backed by `/pipelines/{id}/context`.
  - Evidence (2026-03-03): Evaluator `pass`; context table now exposes searchable key/value rows with runtime-tested type-aware rendering hooks (`run-context-row-type`/`run-context-row-value`) covered by `tests/integration/test_ui_run_context_panel.py` and UI smoke scenario/screenshot `frontend/artifacts/ui-smoke/08f-runs-panel-context-viewer.png`.
- [x] [9.3-02] Add typed rendering for scalar and structured values.
  - Evidence (2026-03-03): Evaluator `pass`; `RunsPanel` now formats context values into scalar (`string`/`number`/`boolean`/`null`) and structured (`object`/`array`) render paths with dedicated test hooks (`run-context-row-value-scalar` and `run-context-row-value-structured`), covered by `tests/integration/test_ui_run_context_panel.py` and UI smoke scenario/screenshot `frontend/artifacts/ui-smoke/08g-runs-panel-context-typed-rendering.png`.
- [x] [9.3-03] Add copy/export actions for context inspection.
  - Evidence (2026-03-03): Evaluator `pass`; `RunsPanel` now exposes filtered-context `Copy JSON` + `Export JSON` actions (clipboard and download) with coverage in `tests/integration/test_ui_run_context_panel.py` and UI-smoke visual QA at `frontend/artifacts/ui-smoke/08h-runs-panel-context-copy-export.png`.

### 9.4 Event Timeline
- [x] [9.4-01] Render typed lifecycle/stage/parallel/interview/checkpoint events from SSE history + stream.
  - Evidence (2026-03-03): Evaluator `pass`; `RunsPanel` now streams typed timeline rows from `/pipelines/{id}/events` with lifecycle/stage/parallel/interview/checkpoint categories, covered by `tests/integration/test_ui_run_event_timeline_panel.py` and UI-smoke visual QA in `frontend/artifacts/ui-smoke/08i-runs-panel-event-timeline.png`.
- [x] [9.4-02] Add filters by event type, node/stage, and severity/category.
  - Evidence (2026-03-03): Evaluator `pass`; `RunsPanel` now supports event type/node-stage/category/severity filtering with UI smoke coverage and visual QA in `frontend/artifacts/ui-smoke/08j-runs-panel-event-timeline-filters.png`.
- [x] [9.4-03] Add timeline grouping/correlation for retries and interview sequences.
  - Evidence (2026-03-03): Evaluator `pass`; `RunsPanel` now groups correlated retry/interview timeline sequences with per-row correlation context, covered by `tests/integration/test_ui_run_event_timeline_panel.py` and UI-smoke visual QA in `frontend/artifacts/ui-smoke/08k-runs-panel-event-timeline-grouping-correlation.png`.
- [x] [9.4-04] Add tests for timeline replay and live append behavior.
  - Evidence (2026-03-03): Evaluator `pass`; added replay/live-append timeline smoke scenario with count/order assertions and screenshot `frontend/artifacts/ui-smoke/08l-runs-panel-event-timeline-replay-live-append.png`, plus guard coverage in `tests/integration/test_ui_run_event_timeline_panel.py`.

### 9.5 Artifact Browser
- [x] [9.5-01] Add artifact listing and file viewer/download actions for run outputs.
  - Evidence (2026-03-03): Evaluator `pass`; added backend artifact list/file endpoints with traversal guards, plus `RunsPanel` artifact table + view/download UX, covered by `tests/api/test_pipeline_artifact_endpoints.py`, `tests/integration/test_ui_run_artifact_panel.py`, and UI-smoke visual QA `frontend/artifacts/ui-smoke/08m-runs-panel-artifact-browser.png`.
- [x] [9.5-02] Add Graphviz render viewer for `/pipelines/{id}/graph`.
  - Evidence (2026-03-03): Evaluator `pass`; `RunsPanel` now fetches and renders `/pipelines/{id}/graph` SVG output with refresh + degraded-state messaging, covered by `tests/integration/test_ui_run_artifact_panel.py` and UI-smoke screenshot `frontend/artifacts/ui-smoke/08n-runs-panel-graphviz-viewer.png`.
- [x] [9.5-03] Add graceful handling for missing artifact files and partial runs.
  - Evidence (2026-03-03): Evaluator `pass`; `RunsPanel` now surfaces partial/pruned artifact warning states and explicit missing-file preview handling, covered by `tests/integration/test_ui_run_artifact_panel.py` and UI-smoke screenshot `frontend/artifacts/ui-smoke/08o-runs-panel-artifact-missing-partial.png`.

### 9.6 Project Run History and Traceability
- [x] [9.6-01] Implement durable run history listing filterable by project.
  - Evidence (2026-03-03): Evaluator `pass`; `/runs` now supports project-scoped filtering via `project_path` with normalized scope matching against durable run metadata (`project_path`/`working_directory`), covered by `tests/api/test_runs_endpoint.py::test_list_runs_filters_durable_history_by_project_item_9_6_01`.
- [x] [9.6-02] Include project identity and available Git metadata on run records.
  - Evidence (2026-03-03): Evaluator `pass`; `RunsPanel` run-history rows now render per-record `Project`, `Branch`, and `Commit` metadata with dedicated test hooks, covered by `tests/integration/test_ui_run_history_traceability.py` and UI-smoke visual QA screenshot `frontend/artifacts/ui-smoke/08p-runs-panel-run-history-traceability.png`.
- [x] [9.6-03] Link run records to associated spec/plan artifacts when available.
  - Evidence (2026-03-03): Evaluator `pass`; run-start payload and durable run metadata now carry `spec_id`/`plan_id`, and `RunsPanel` run-history rows render clickable spec/plan artifact links, covered by `tests/api/test_runs_endpoint.py`, `tests/integration/test_ui_execution_run_initiation.py`, `tests/integration/test_ui_run_history_traceability.py`, and ui-smoke screenshot `frontend/artifacts/ui-smoke/08q-runs-panel-run-history-spec-plan-links.png`.
- [x] [9.6-04] Add audit-focused tests for timestamp completeness and historical reconstruction.
  - Evidence (2026-03-03): Evaluator `pass`; `/runs` now backfills missing `started_at`/`ended_at` from `run.log` for durable history reconstruction, covered by `tests/api/test_runs_endpoint.py::test_list_runs_backfills_missing_timestamps_from_run_log_item_9_6_04` and `tests/api/test_runs_endpoint.py::test_list_runs_reconstructs_timestamp_ordering_from_run_logs_item_9_6_04`.

---

## 10. Human-in-the-Loop UX

### 10.1 Prompt Handling
- [x] [10.1-01] Ensure pending human gates are discoverable in execution and run views.
  - Evidence (2026-03-04): Evaluator `pass`; `RunsPanel` now surfaces pending gates from both `InterviewStarted` and `human_gate` events with duplicate suppression, covered by frontend contract `CID:10.1.01` (`frontend/src/components/__tests__/ContractBehavior.test.tsx`, `tests/contracts/frontend/test_human_gate_discoverability_contracts.py`) and UI smoke visual QA screenshots `frontend/artifacts/ui-smoke/10a-human-gate-discoverability.png` and `frontend/artifacts/ui-smoke/10b-human-gate-discoverability-runs.png`.
- [x] [10.1-02] Ensure operator can answer prompts without leaving UI.
  - Evidence (2026-03-04): Evaluator `pass`; `RunsPanel` now renders in-panel answer actions for pending human gates and submits selections to `/pipelines/{id}/questions/{question_id}/answer`, covered by frontend contract `CID:10.1.02` (`frontend/src/components/__tests__/ContractBehavior.test.tsx`, `tests/contracts/frontend/test_human_gate_discoverability_contracts.py`) and UI smoke visual QA screenshot `frontend/artifacts/ui-smoke/10b-human-gate-discoverability-runs.png`.

### 10.2 Question Types
- [x] [10.2-01] Render `MULTIPLE_CHOICE` interactions with option metadata.
  - Evidence (2026-03-04): Evaluator `pass`; `RunsPanel` now parses/render `MULTIPLE_CHOICE` option metadata (`key`/`description`) for pending human gates, covered by frontend contract `CID:10.2.01` (`frontend/src/components/__tests__/ContractBehavior.test.tsx`, `tests/contracts/frontend/test_human_gate_discoverability_contracts.py`) and UI smoke visual QA screenshot `frontend/artifacts/ui-smoke/10b-human-gate-discoverability-runs.png`.
- [x] [10.2-02] Render `YES_NO` and `CONFIRMATION` interactions with explicit semantics.
  - Evidence (2026-03-04): Evaluator `pass`; `RunsPanel` now renders explicit `Sends YES/NO` semantics for `YES_NO` and `CONFIRMATION` gate options, covered by frontend contract `CID:10.2.02` (`frontend/src/components/__tests__/ContractBehavior.test.tsx`, `tests/contracts/frontend/test_human_gate_discoverability_contracts.py`) and UI-smoke visual QA screenshot `frontend/artifacts/ui-smoke/10c-human-gate-yes-no-confirmation-semantics.png`.
- [x] [10.2-03] Render `FREEFORM` input interactions when backend exposes them.
  - Evidence (2026-03-04): Evaluator `pass`; `RunsPanel` now renders FREEFORM text-input + submit interactions for pending human gates and posts typed answers through `/pipelines/{id}/questions/{question_id}/answer`, covered by frontend contract `CID:10.2.03` (`frontend/src/components/__tests__/ContractBehavior.test.tsx`, `tests/contracts/frontend/test_human_gate_discoverability_contracts.py`) and UI-smoke visual QA screenshot `frontend/artifacts/ui-smoke/10d-human-gate-freeform-interaction.png`.
- [x] [10.2-04] Add contract tests for each supported question type.
  - Evidence (2026-03-04): Evaluator `pass`; added frontend contract `CID:10.2.04` plus contract bridge coverage so `MULTIPLE_CHOICE`, `YES_NO`, `CONFIRMATION`, and `FREEFORM` each have explicit behavioral assertions (`frontend/src/components/__tests__/ContractBehavior.test.tsx`, `tests/contracts/frontend/test_human_gate_discoverability_contracts.py`) with screenshot-based visual QA references `frontend/artifacts/ui-smoke/10b-human-gate-discoverability-runs.png`, `frontend/artifacts/ui-smoke/10c-human-gate-yes-no-confirmation-semantics.png`, and `frontend/artifacts/ui-smoke/10d-human-gate-freeform-interaction.png`.

### 10.3 Timeout/Default Semantics
- [x] [10.3-01] Expose `human.default_choice` authoring and visibility in node inspector.
  - Evidence (2026-03-04): Evaluator `pass`; node inspector now shows explicit timeout-default guidance for `human.default_choice` on `wait.human` nodes in `frontend/src/components/Sidebar.tsx`, with frontend contract `CID:10.3.01` (`frontend/src/components/__tests__/ContractBehavior.test.tsx`, `tests/contracts/frontend/test_human_gate_discoverability_contracts.py`) and screenshot-based visual QA review using `frontend/artifacts/ui-smoke/10a-human-gate-discoverability.png`, `frontend/artifacts/ui-smoke/10b-human-gate-discoverability-runs.png`, `frontend/artifacts/ui-smoke/10c-human-gate-yes-no-confirmation-semantics.png`, and `frontend/artifacts/ui-smoke/10d-human-gate-freeform-interaction.png`.
- [x] [10.3-02] Display timeout/default-applied/skipped outcome provenance in run timeline.
  - Evidence (2026-03-04): Evaluator `pass`; `RunsPanel` now renders provenance-aware interview timeline summaries (`accepted`, `skipped`, `timeout_default_applied`, `timeout_no_default`) sourced from runtime event payloads, with frontend contract `CID:10.3.02` and backend handler coverage in `tests/handlers/test_wait_human_handler.py`, plus screenshot-based visual QA review of run timeline surfaces in `frontend/artifacts/ui-smoke/08i-runs-panel-event-timeline.png`, `frontend/artifacts/ui-smoke/08j-runs-panel-event-timeline-filters.png`, and `frontend/artifacts/ui-smoke/08k-runs-panel-event-timeline-grouping-correlation.png`.
- [x] [10.3-03] Add tests for timeout fallback and explicit answer branches.
  - Evidence (2026-03-04): Evaluator `pass`; added frontend contract `CID:10.3.03` to assert timeout/default and explicit-answer summary fallbacks when `outcome_provenance` is omitted, with fallback inference implemented in `RunsPanel` and contract bridge coverage in `tests/contracts/frontend/test_human_gate_discoverability_contracts.py`.

### 10.4 Multi-Question and Inform Messages
- [x] [10.4-01] Support grouped multi-question prompts when emitted by backend.
  - Evidence (2026-03-04): Evaluator `pass`; `RunsPanel` now retains multiple pending prompts per stage entity and renders grouped stage cards in the pending-human-gates panel, covered by frontend contract `CID:10.4.01` (`frontend/src/components/__tests__/ContractBehavior.test.tsx`, `tests/contracts/frontend/test_human_gate_discoverability_contracts.py`) plus screenshot-based visual QA using `frontend/artifacts/ui-smoke/10b-human-gate-discoverability-runs.png`, `frontend/artifacts/ui-smoke/10c-human-gate-yes-no-confirmation-semantics.png`, and `frontend/artifacts/ui-smoke/10d-human-gate-freeform-interaction.png`.
- [x] [10.4-02] Display interviewer inform messages in context of originating stage.
  - Evidence (2026-03-04): Evaluator `pass`; `RunsPanel` now classifies `InterviewInform` events, renders inform-message summaries, and places inform prompts inside stage-scoped pending-gate groups using `message` fallback handling, covered by frontend contract `CID:10.4.02` (`frontend/src/components/__tests__/ContractBehavior.test.tsx`, `tests/contracts/frontend/test_human_gate_discoverability_contracts.py`) with screenshot-based visual QA references `frontend/artifacts/ui-smoke/10b-human-gate-discoverability-runs.png`, `frontend/artifacts/ui-smoke/10c-human-gate-yes-no-confirmation-semantics.png`, `frontend/artifacts/ui-smoke/10d-human-gate-freeform-interaction.png`, `frontend/artifacts/ui-smoke/08i-runs-panel-event-timeline.png`, and `frontend/artifacts/ui-smoke/08k-runs-panel-event-timeline-grouping-correlation.png`.
- [x] [10.4-03] Preserve order and auditability of grouped interactions.
  - Evidence (2026-03-04): Evaluator `pass`; `RunsPanel` now preserves grouped interaction order by timeline sequence (group order + in-group order) and renders per-item audit metadata (`Order #`, `Question ID`, `Received`), covered by frontend contract `CID:10.4.03` (`frontend/src/components/__tests__/ContractBehavior.test.tsx`, `tests/contracts/frontend/test_human_gate_discoverability_contracts.py`) with screenshot-based visual QA references `frontend/artifacts/ui-smoke/10b-human-gate-discoverability-runs.png`, `frontend/artifacts/ui-smoke/10c-human-gate-yes-no-confirmation-semantics.png`, and `frontend/artifacts/ui-smoke/10d-human-gate-freeform-interaction.png`.

---

## 11. Persistence and DOT Serialization

### 11.1 Canonical Model
- [x] [11.1-01] Define and implement canonical frontend flow model covering all spec constructs.
  - Evidence (2026-03-04): Evaluator `pass`; added canonical flow model ingestion/serialization (`frontend/src/lib/canonicalFlowModel.ts`) and routed editor hydration + DOT generation through the canonical mapping (`frontend/src/components/Editor.tsx`, `frontend/src/lib/dotUtils.ts`), covered by contract tests `tests/contracts/frontend/test_canonical_flow_model_contracts.py` and UI-smoke visual QA screenshots `frontend/artifacts/ui-smoke/01-editor-shell.png`, `frontend/artifacts/ui-smoke/19a-semantic-equivalence-mismatch-blocked.png`, and `frontend/artifacts/ui-smoke/19b-semantic-equivalence-round-trip-saved.png`.
- [x] [11.1-02] Ensure canonical model can represent subgraph/default scopes and extension attrs.
  - Evidence (2026-03-04): Evaluator `pass`; canonical DOT generation now serializes top-level defaults, nested subgraph scopes, and unknown-valid extension attrs (`frontend/src/lib/canonicalFlowModel.ts`), covered by contract test `tests/contracts/frontend/test_canonical_flow_model_contracts.py::test_canonical_flow_model_serializes_scopes_and_extension_attrs_item_11_1_02` plus UI-smoke visual QA screenshots `frontend/artifacts/ui-smoke/01-editor-shell.png`, `frontend/artifacts/ui-smoke/19a-semantic-equivalence-mismatch-blocked.png`, and `frontend/artifacts/ui-smoke/19b-semantic-equivalence-round-trip-saved.png`.

### 11.2 Round-Trip Requirements
- [x] [11.2-01] Add no-op save semantic-equivalence tests for spec-valid fixtures.
  - Evidence (2026-03-04): Evaluator `pass`; added fixture-based no-op semantic-equivalence coverage in `tests/api/test_flow_save_validation.py::test_save_flow_no_op_semantic_equivalence_for_spec_valid_fixtures_item_11_2_01`, and updated canonical preview round-trip handling (`frontend/src/lib/canonicalFlowModel.ts`, `frontend/src/components/Editor.tsx`) to preserve explicit-vs-implicit label semantics across no-op saves, with visual QA screenshots `frontend/artifacts/ui-smoke/19-semantic-equivalence-noop-save.png`, `frontend/artifacts/ui-smoke/19a-semantic-equivalence-mismatch-blocked.png`, and `frontend/artifacts/ui-smoke/19b-semantic-equivalence-round-trip-saved.png`.
- [x] [11.2-02] Add open/edit/save/re-open tests for advanced attribute coverage.
  - Evidence (2026-03-04): Evaluator `pass`; added open/edit/save/re-open API contract `tests/api/test_flow_save_validation.py::test_save_flow_open_edit_save_reopen_preserves_advanced_attrs_item_11_2_02` plus canonical serializer fix preserving explicit `false` advanced booleans in `frontend/src/lib/canonicalFlowModel.ts`, validated by `just test` and `uv run pytest -q`.
- [x] [11.2-03] Add regression tests for previously lossy edit paths.
  - Evidence (2026-03-04): Evaluator `pass`; added API regression coverage in `tests/api/test_flow_save_validation.py::test_save_flow_open_edit_save_reopen_preserves_label_regressions_item_11_2_03` for a previously lossy semicolon-less explicit-label edit path, plus a minimal canonical-label detection fix in `frontend/src/lib/canonicalFlowModel.ts`, validated by `just test` and UI-smoke visual QA screenshots `frontend/artifacts/ui-smoke/19-semantic-equivalence-noop-save.png`, `frontend/artifacts/ui-smoke/19a-semantic-equivalence-mismatch-blocked.png`, and `frontend/artifacts/ui-smoke/19b-semantic-equivalence-round-trip-saved.png`.

### 11.3 Mixed-Mode Editing
- [x] [11.3-01] Implement robust transitions between raw DOT and structured UI modes.
  - Evidence (2026-03-04): Evaluator `pass`; `Editor` now enforces single-flight raw→structured handoff (re-entry guard + disabled Structured toggle while handoff is in-flight) in `frontend/src/components/Editor.tsx`, with contract coverage `CID:11.3.01` in `frontend/src/components/__tests__/ContractBehavior.test.tsx` bridged by `tests/contracts/frontend/test_mixed_mode_editing_contracts.py`, and screenshot-based visual QA references `frontend/artifacts/ui-smoke/19a-semantic-equivalence-mismatch-blocked.png` and `frontend/artifacts/ui-smoke/19b-semantic-equivalence-round-trip-saved.png`.
- [x] [11.3-02] Preserve unsurfaced data through both editing paths.
  - Evidence (2026-03-04): Evaluator `pass`; frontend contract `CID:11.3.02` now proves unknown graph/default/subgraph/node/edge attrs survive structured saves, raw-mode drafts, and raw→structured handoff saves (`frontend/src/components/__tests__/ContractBehavior.test.tsx`, `tests/contracts/frontend/test_mixed_mode_editing_contracts.py`), implemented via canonical context carry-through and hydrated attribute passthrough in `frontend/src/lib/dotUtils.ts` and `frontend/src/components/Editor.tsx`, with screenshot-based visual QA on `frontend/artifacts/ui-smoke/01-editor-shell.png`, `frontend/artifacts/ui-smoke/19-semantic-equivalence-noop-save.png`, `frontend/artifacts/ui-smoke/19a-semantic-equivalence-mismatch-blocked.png`, and `frontend/artifacts/ui-smoke/19b-semantic-equivalence-round-trip-saved.png`.
- [x] [11.3-03] Add conflict handling when raw edit invalidates structured assumptions.
  - Evidence (2026-03-04): Evaluator `pass`; raw→structured handoff now blocks when preview reports validation/conflict payloads (`status=validation_error` or non-empty `errors`) and keeps the editor in raw mode with explicit handoff guidance in `frontend/src/components/Editor.tsx`, covered by frontend contract `CID:11.3.03` (`frontend/src/components/__tests__/ContractBehavior.test.tsx`, `tests/contracts/frontend/test_mixed_mode_editing_contracts.py`) and screenshot-based visual QA references `frontend/artifacts/ui-smoke/19a-semantic-equivalence-mismatch-blocked.png`, `frontend/artifacts/ui-smoke/19b-semantic-equivalence-round-trip-saved.png`, and `frontend/artifacts/ui-smoke/01-projects-shell.png`.

### 11.4 Extension Attributes
- [x] [11.4-01] Implement generic advanced key/value editor for non-core attributes.
  - Evidence (2026-03-04): Evaluator `pass`; added reusable non-core key/value editor component (`frontend/src/components/AdvancedKeyValueEditor.tsx`) wired into graph/node/edge inspectors (`frontend/src/components/GraphSettings.tsx`, `frontend/src/components/Sidebar.tsx`) with extension-attr filtering helper (`frontend/src/lib/extensionAttrs.ts`), covered by frontend contract `CID:11.4.01` (`frontend/src/components/__tests__/ContractBehavior.test.tsx`, `tests/contracts/frontend/test_extension_attribute_editor_contracts.py`) and validated by `just test`.
- [ ] [11.4-02] Preserve unknown-valid attributes during all save operations.
- [ ] [11.4-03] Add tests proving extension attribute stability across edits.

### 11.5 Project Workspace Persistence
- [ ] [11.5-01] Persist project registry across sessions with unique-directory enforcement.
- [ ] [11.5-02] Persist and restore project-scoped conversation/spec/plan linkage.
- [ ] [11.5-03] Rehydrate last active project context safely on reopen.

### 11.6 Spec and Plan Artifact Provenance
- [ ] [11.6-01] Store spec/plan provenance metadata or references for workflow-generated artifacts.
- [ ] [11.6-02] Ensure provenance captures run linkage and timestamps, plus branch/commit when available.
- [ ] [11.6-03] Persist and restore plan status lifecycle (`draft`, `approved`, `rejected`, `revision-requested`).

---

## 12. API Integration Contract

### 12.1 Required Endpoints
- [ ] [12.1-01] Verify UI coverage for all required endpoints listed in UI spec section 12.1.
- [ ] [12.1-02] Add typed client adapters and runtime schema validation for endpoint responses.
- [ ] [12.1-03] Add endpoint-level integration tests for happy path and common error cases.

### 12.2 Contract Drift Handling
- [ ] [12.2-01] Add degraded-state UX when endpoints are unavailable or incompatible.
- [ ] [12.2-02] Ensure non-dependent UI surfaces remain functional under partial API failure.
- [ ] [12.2-03] Ensure save paths remain non-destructive during API contract drift.

### 12.3 Project Scope Contract
- [ ] [12.3-01] Persist and restore active-project identity in UI client state.
- [ ] [12.3-02] Ensure execution payload/project identity resolves to concrete working-directory context.
- [ ] [12.3-03] Ensure conversation/spec/plan retrieval is keyed by project identity.

### 12.4 Workflow Orchestration Contract
- [ ] [12.4-01] Integrate project-scoped conversation turn/history contract in UI client adapters.
- [ ] [12.4-02] Integrate spec-edit proposal/apply/reject contract with schema validation.
- [ ] [12.4-03] Integrate plan-generation invocation/status contract with degraded-state handling.
- [ ] [12.4-04] Integrate plan approval/rejection/revision transition contract.
- [ ] [12.4-05] Integrate build invocation-from-approved-plan contract and error paths.

---

## 13. Accessibility, Responsiveness, and Performance

### 13.1 Accessibility
- [ ] [13.1-01] Add keyboard navigation coverage for core project-management/authoring/execution flows.
- [ ] [13.1-02] Add focus-visible and semantic label audit across interactive controls.
- [ ] [13.1-03] Verify diagnostic/status color contrast against accessibility standards.

### 13.2 Responsiveness
- [ ] [13.2-01] Add responsive layout behavior for inspector, timeline, and diagnostics.
- [ ] [13.2-02] Ensure mobile and narrow viewport usability for core project and operational tasks.
- [ ] [13.2-03] Add viewport-based regression tests/screenshots.

### 13.3 Performance
- [ ] [13.3-01] Define performance budgets for canvas interaction and timeline updates.
- [ ] [13.3-02] Add profiling and optimization pass for medium-sized graphs.
- [ ] [13.3-03] Add stress tests for sustained SSE event throughput.

---

## 14. Parity Program Breakdown

### 14.0 E0 - Project Workspace Foundation (P0)
- [ ] [14.0-01] Implement Projects area, registration, and active-project selection UX.
- [ ] [14.0-02] Implement unique-directory + Git-repo invariant enforcement and tests.
- [ ] [14.0-03] Implement project-scoped conversation/session state and deep-linking.
- [ ] [14.0-04] Add isolation tests proving no cross-project context/file leakage.

### 14.1 E1 - Data Model Parity Foundation (P0)
- [ ] [14.1-01] Finalize canonical UI graph model schema for all spec constructs.
- [ ] [14.1-02] Implement parser-to-UI model mapping without dropping data.
- [ ] [14.1-03] Implement UI model-to-DOT serialization with semantic round-trip tests.

### 14.2 E2 - Graph Attribute Completeness (P0)
- [ ] [14.2-01] Implement missing graph attribute forms and validation.
- [ ] [14.2-02] Add compatibility tests for `stack.child_*` and `tool_hooks.*` attrs.
- [ ] [14.2-03] Add help text and usage guidance for advanced graph attrs.

### 14.3 E3 - Node Handler Completeness (P0)
- [ ] [14.3-01] Implement full handler selection matrix and type override behavior.
- [ ] [14.3-02] Implement handler-specific inspector sections for all supported handlers.
- [ ] [14.3-03] Add fixtures/tests for each handler editor path.

### 14.4 E4 - Subgraph and Defaults Authoring (P0)
- [ ] [14.4-01] Implement subgraph CRUD and membership UI.
- [ ] [14.4-02] Implement scoped default blocks with inheritance previews.
- [ ] [14.4-03] Add round-trip tests for subgraph/default block integrity.

### 14.5 E5 - Condition and Routing Tooling (P1)
- [ ] [14.5-01] Add condition builder/assist UX and syntax guidance.
- [ ] [14.5-02] Add route explanation panel showing routing inputs and selected edge.
- [ ] [14.5-03] Add deterministic routing preview tests.

### 14.6 E6 - Stylesheet Tooling (P1)
- [ ] [14.6-01] Implement stylesheet linter + diagnostics in graph settings.
- [ ] [14.6-02] Implement selector simulation preview against current graph.
- [ ] [14.6-03] Implement per-node resolved model stack inspector.

### 14.7 E7 - Human Gate Advanced UX (P1)
- [ ] [14.7-01] Support all exposed interviewer question modes in UI.
- [ ] [14.7-02] Show timeout/default provenance in timeline and node status.
- [ ] [14.7-03] Add deterministic tests for human gate branching outcomes.

### 14.8 E8 - Run Inspector Shell (P0)
- [ ] [14.8-01] Implement run detail container with section navigation.
- [ ] [14.8-02] Wire summary/status refresh behavior across runs.
- [ ] [14.8-03] Add run switching UX with preserved filters/context.

### 14.9 E9 - Checkpoint and Context Inspector (P0)
- [ ] [14.9-01] Implement checkpoint panel and context panel in run inspector.
- [ ] [14.9-02] Add search/filter and copy tools for context keys.
- [ ] [14.9-03] Add no-data/degraded-state handling for missing checkpoint/context.

### 14.10 E10 - Event Timeline Completeness (P0)
- [ ] [14.10-01] Implement full typed event timeline model and renderer.
- [ ] [14.10-02] Implement replay from history plus live stream continuation.
- [ ] [14.10-03] Add timeline filters and event correlation views.

### 14.11 E11 - Artifact Browser and Graph Render Viewer (P0)
- [ ] [14.11-01] Implement artifact file listing and viewer/download support.
- [ ] [14.11-02] Implement graph render embed/view UX.
- [ ] [14.11-03] Add artifact availability checks and fallback UX.

### 14.12 E12 - No-Loss Serialization Guarantees (P0)
- [ ] [14.12-01] Add parity fixture corpus for no-loss serialization checks.
- [ ] [14.12-02] Add semantic diff harness between input and saved DOT.
- [ ] [14.12-03] Gate releases on no-loss serialization pass criteria.

### 14.13 E13 - Accessibility and Mobile Hardening (P2)
- [ ] [14.13-01] Complete accessibility audit and remediation pass.
- [ ] [14.13-02] Complete mobile interaction audit and remediation pass.
- [ ] [14.13-03] Add regression tests for accessibility/mobile hardening.

### 14.14 E14 - End-to-End Parity Certification (P0)
- [ ] [14.14-01] Build parity certification matrix from UI spec + appendices.
- [ ] [14.14-02] Add CI parity job that fails on uncovered checklist items.
- [ ] [14.14-03] Publish parity certification evidence bundle for release readiness.

### 14.15 E15 - Spec-to-Plan Conversation Workflow (P0)
- [ ] [14.15-01] Implement project-scoped AI conversation and spec-drafting UX end-to-end.
- [ ] [14.15-02] Implement explicit proposal/review/apply flow for AI spec edits.
- [ ] [14.15-03] Add isolation and durability tests for conversation history + proposal artifacts per project.

### 14.16 E16 - Plan Governance and Build Launch (P0)
- [ ] [14.16-01] Implement plan-generation and persisted plan artifact workflow.
- [ ] [14.16-02] Implement plan governance state machine and operator controls.
- [ ] [14.16-03] Implement approved-plan build launch path with traceability and recovery UX.

---

## 15. Definition of Done

- [ ] [15-01] Verify Appendix A graph/node/edge attributes are all UI-supported and tested.
- [ ] [15-02] Verify Appendix B constructs are all authorable and inspectable in UI.
- [ ] [15-03] Verify spec-valid flow open/save/reopen has no behavior loss across fixtures.
- [ ] [15-04] Verify run inspector exposes summary, timeline, checkpoint, context, graph, artifacts.
- [ ] [15-05] Verify human gates are fully operable for all supported question types.
- [ ] [15-06] Verify frontend build/lint/tests and parity e2e suite are green.
- [ ] [15-07] Verify Projects is first-class and unique-directory + Git invariants + project isolation are enforced.
- [ ] [15-08] Verify project-scoped AI conversation/spec-authoring loop with explicit apply/reject gates is fully operable.
- [ ] [15-09] Verify spec->plan->build orchestration with plan approval gates is fully operable.
- [ ] [15-10] Verify per-project history/provenance supports audit reconstruction of spec/plan/build outcomes.

---

## Appendix A: Attribute Coverage Matrix

### A.1 Graph Attributes
- [ ] [A1-01] Implement + test `goal`.
- [ ] [A1-02] Implement + test `label`.
- [ ] [A1-03] Implement + test `model_stylesheet`.
- [ ] [A1-04] Implement + test `default_max_retry`.
- [ ] [A1-05] Implement + test `default_fidelity`.
- [ ] [A1-06] Implement + test `retry_target`.
- [ ] [A1-07] Implement + test `fallback_retry_target`.
- [ ] [A1-08] Implement + test `stack.child_dotfile`.
- [ ] [A1-09] Implement + test `stack.child_workdir`.
- [ ] [A1-10] Implement + test `tool_hooks.pre`.
- [ ] [A1-11] Implement + test `tool_hooks.post`.

### A.2 Node Attributes
- [ ] [A2-01] Implement + test `label`.
- [ ] [A2-02] Implement + test `shape`.
- [ ] [A2-03] Implement + test `type`.
- [ ] [A2-04] Implement + test `prompt`.
- [ ] [A2-05] Implement + test `tool_command`.
- [ ] [A2-06] Implement + test `max_retries`.
- [ ] [A2-07] Implement + test `goal_gate`.
- [ ] [A2-08] Implement + test `retry_target`.
- [ ] [A2-09] Implement + test `fallback_retry_target`.
- [ ] [A2-10] Implement + test `fidelity`.
- [ ] [A2-11] Implement + test `thread_id`.
- [ ] [A2-12] Implement + test `class`.
- [ ] [A2-13] Implement + test `timeout`.
- [ ] [A2-14] Implement + test `llm_model`.
- [ ] [A2-15] Implement + test `llm_provider`.
- [ ] [A2-16] Implement + test `reasoning_effort`.
- [ ] [A2-17] Implement + test `auto_status`.
- [ ] [A2-18] Implement + test `allow_partial`.
- [ ] [A2-19] Implement + test `join_policy`.
- [ ] [A2-20] Implement + test `error_policy`.
- [ ] [A2-21] Implement + test `max_parallel`.
- [ ] [A2-22] Implement + test `manager.poll_interval`.
- [ ] [A2-23] Implement + test `manager.max_cycles`.
- [ ] [A2-24] Implement + test `manager.stop_condition`.
- [ ] [A2-25] Implement + test `manager.actions`.
- [ ] [A2-26] Implement + test `human.default_choice`.

### A.3 Edge Attributes
- [ ] [A3-01] Implement + test `label`.
- [ ] [A3-02] Implement + test `condition`.
- [ ] [A3-03] Implement + test `weight`.
- [ ] [A3-04] Implement + test `fidelity`.
- [ ] [A3-05] Implement + test `thread_id`.
- [ ] [A3-06] Implement + test `loop_restart`.

---

## Appendix B: Construct Coverage Matrix

- [ ] [B-01] Implement + test directed graph authoring with valid node IDs.
- [ ] [B-02] Implement + test chained edge authoring/serialization.
- [ ] [B-03] Implement + test node default blocks.
- [ ] [B-04] Implement + test edge default blocks.
- [ ] [B-05] Implement + test subgraphs with scoped defaults.
- [ ] [B-06] Implement + test subgraph-derived class behavior.
- [ ] [B-07] Implement + test shape-to-handler mapping and explicit `type` override.
- [ ] [B-08] Implement + test retry and goal-gate configuration UX.
- [ ] [B-09] Implement + test fidelity and thread resolution controls.
- [ ] [B-10] Implement + test model stylesheet selectors and precedence tooling.
- [ ] [B-11] Implement + test human gate defaults and timeout behavior UX.
- [ ] [B-12] Implement + test parallel fan-out and fan-in configuration UX.
- [ ] [B-13] Implement + test tool handler command and hook configuration UX.
- [ ] [B-14] Implement + test manager loop supervision configuration UX.
- [ ] [B-15] Implement + test runtime checkpoint/context/events/artifact inspection UX.
- [ ] [B-16] Implement + test project registration with unique-directory enforcement.
- [ ] [B-17] Implement + test Git-repository gating for project-scoped workflow starts.
- [ ] [B-18] Implement + test active-project deep-link and restoration behavior.
- [ ] [B-19] Implement + test project-scoped conversation/spec/plan/run linkage.
- [ ] [B-20] Implement + test cross-project context/file isolation guarantees.

---

## Appendix C: User Story Coverage Map

- [ ] [C-01] Add a maintained story-to-spec-to-checklist trace matrix for all `US-*` IDs.
- [ ] [C-02] Add acceptance tests that collectively cover every story cluster (project selection, conversation/spec, plan/build governance, traceability).
- [ ] [C-03] Add CI gate failing when any `US-*` story lacks mapped implementation evidence.

---

## Deferred Tasks

- [ ] [1.2-02] Add CI acceptance checks proving the full journey works without raw DOT fallback.
  Deferred because the required full-journey UI surfaces (project registration/selection and project-scoped conversation/spec/plan flow) are not yet implemented, so a CI proof would be non-representative.
- [ ] [3.1-02] Add role-oriented smoke tests (project onboarding, authoring, live operation, post-run audit).
  Deferred because project onboarding and active-project workflow surfaces from `ui-spec.md` sections 4.2 and 4.3 are not yet implemented, so role smoke tests would be partial and misleading.
- [ ] [3.2-01] Implement workflow guardrails from create/open to iterate/re-run as explicit state machine transitions.
  Deferred because the required project-scoped workflow surfaces (project registry/Git gating, conversation/spec loop, and spec->plan->build chain) are not yet implemented in the UI.
- [ ] [3.2-02] Add test coverage for the full 12-step primary workflow sequence (project registration/selection through re-run).
  Deferred because the project registration/Git gating and project-scoped conversation/spec/plan/run surfaces required by `ui-spec.md` section 3.2 are not yet implemented for behavioral end-to-end coverage.
- [ ] [3.2-03] Ensure every step has a first-class UI surface (no hidden or CLI-only transition).
  Deferred because the required project-scoped conversation/spec/plan/build surfaces are not implemented yet, so the workflow still has unavoidable non-UI transitions.
- [ ] [3.2-04] Add explicit workflow coverage for project-scoped AI conversation -> spec refinement -> plan generation/approval -> build execution chain.
  Deferred because the project-scoped conversation/spec/plan/build chain is not fully implemented, so coverage would only validate placeholders instead of the real workflow.
- [ ] [4-02] Add deterministic deep-link state for active project/flow/run/conversation and panel selection.
  Deferred because active-project identity and project-scoped conversation state are not implemented yet, so full deep-link determinism cannot be validated end-to-end.
- [ ] [4.2-02] Enforce Git-repository requirement before workflow execution (with explicit initialize path when user confirms).
  Deferred because the current API/UI contract has no project Git verification/initialize endpoint path yet, so the required enforcement and explicit initialization flow cannot be implemented correctly.
- [ ] [4.3-03] Implement Git verification UI with explicit initialize action path.
  Deferred because the frontend currently has no project Git verification/initialize API contract to drive real status checks or an explicit initialize action flow.
- [ ] [5.4-02] Inject active project directory and repository metadata into AI conversation context.
  Deferred because the request pipeline for real AI conversation payloads is still not implemented, so there is no concrete backend-bound conversation context path to inject this metadata into yet.
- [ ] [6.4-01] Add subgraph creation, labeling, and membership editing UI.
  Deferred because the current frontend/API flow model only returns flattened nodes/edges and lacks subgraph/default-scope structures needed for first-class membership editing.
- [ ] [6.4-02] Add scoped `node[...]` and `edge[...]` defaults authoring controls.
  Deferred because the current frontend/API payload and DOT serializer do not model scoped default blocks, so this control cannot be implemented without first adding subgraph/default-scope primitives.
- [ ] [6.4-03] Visualize inherited vs explicit attrs at node/edge level.
  Deferred because inherited-versus-explicit visualization depends on subgraph membership and scoped defaults that are currently flattened out by the frontend/API model and serializer.
- [ ] [6.4-04] Add support for derived class behavior from subgraph labels.
  Deferred because derived class behavior depends on subgraph creation/membership and scoped default infrastructure (`6.4-01` to `6.4-03`) that is not yet modeled in the frontend.
- [ ] [6.4-05] Add round-trip tests preserving subgraph/default-block semantics.
  Deferred because the current frontend/API model and serializer flatten subgraph/default scopes, so the test would fail on known model loss instead of catching regressions.
- [ ] [8.2-02] Provide Pause/Resume controls when backend/API supports them.
  Deferred because the current UI/API contract only exposes `/pipelines/{id}/cancel` with no pause/resume endpoint or capability metadata to drive those controls safely.
- [ ] [8.5-02] Persist generated implementation plans to project files with visible status/provenance.
  Deferred because the current API/UI contract has no project-scoped plan artifact write/read endpoint (only flow save and pipeline launch), so file persistence with visible status/provenance cannot be implemented correctly yet.
