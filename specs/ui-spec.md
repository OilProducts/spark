# Spark Spawn UI Specification

Natural-language specification for the Spark Spawn web UI. This document defines the target behavior, information architecture, feature coverage, and delivery slices for full parity with `attractor-spec.md` plus project-scoped workflow requirements defined here.

---

## Table of Contents

1. [Overview and Goals](#1-overview-and-goals)
2. [Design Principles](#2-design-principles)
3. [Users and Primary Workflows](#3-users-and-primary-workflows)
4. [Information Architecture](#4-information-architecture)
5. [Core Interaction Model](#5-core-interaction-model)
6. [Authoring Surface Requirements](#6-authoring-surface-requirements)
7. [Validation and Diagnostics UX](#7-validation-and-diagnostics-ux)
8. [Execution and Control UX](#8-execution-and-control-ux)
9. [Run Inspector Requirements](#9-run-inspector-requirements)
10. [Human-in-the-Loop UX](#10-human-in-the-loop-ux)
11. [Persistence and DOT Serialization](#11-persistence-and-dot-serialization)
12. [API Integration Contract](#12-api-integration-contract)
13. [Accessibility, Responsiveness, and Performance](#13-accessibility-responsiveness-and-performance)
14. [Parity Program Breakdown](#14-parity-program-breakdown)
15. [Definition of Done](#15-definition-of-done)
16. [Appendix A: Attribute Coverage Matrix](#appendix-a-attribute-coverage-matrix)
17. [Appendix B: Construct Coverage Matrix](#appendix-b-construct-coverage-matrix)
18. [Appendix C: User Story Coverage Map](#appendix-c-user-story-coverage-map)

---

## 1. Overview and Goals

The Spark Spawn UI is the project-scoped operator and authoring surface for DOT-defined pipelines.

The UI MUST support three top-level use cases:

1. **Project Management:** create/select project scope anchored to a unique local directory and Git repository.
2. **Authoring:** create, edit, validate, and save spec-valid pipeline definitions.
3. **Operations:** execute, monitor, diagnose, and inspect pipeline runs.

All authoring and operations workflows MUST be scoped to an active project.

The UI target state is **full spec parity** for all features defined in `attractor-spec.md` that are user-visible or user-configurable, plus complete project-scoped workflow coverage from this UI spec.

### 1.1 Problem Statement

Without full parity, users are forced to switch between visual editing and raw DOT for core features, and edits can lose configuration fidelity.

### 1.2 Product Goal

A user MUST be able to author, run, inspect, and troubleshoot any spec-valid pipeline entirely in the UI, without behavior loss and without hidden required configuration.

For project-first operation, the UI MUST additionally support a complete project-scoped loop: select project -> collaborate on spec -> review execution cards derived from approved spec edits -> run build workflows -> inspect outcomes.

### 1.3 Non-Goals

- Replacing the DOT runtime parser or executor.
- UI interpretation of DOT parsing and execution behavior MUST not diverge from `attractor-spec.md` or the runtime/parser implementation.
- Supporting arbitrary invalid DOT inputs.
- Hiding advanced features in favor of a simplified-only mode.

---

## 2. Design Principles

1. **Spec-first behavior:** when uncertain, UI behavior MUST follow `attractor-spec.md`.
2. **No silent data loss:** any save path MUST preserve behavior and config.
3. **Progressive disclosure:** advanced fields MAY be collapsed but MUST remain available.
4. **Explainability:** routing, retries, and failures MUST be inspectable in UI.
5. **Operational safety:** destructive actions require confirmation; state changes are visible.
6. **Consistency:** editing patterns are uniform across graph, node, and edge scopes.

---

## 3. Users and Primary Workflows

### 3.1 User Roles

- **Pipeline Author:** defines workflow structure and stage configuration.
- **Operator:** executes and monitors runs in real time.
- **Reviewer/Auditor:** inspects run artifacts, context, and decisions after execution.
- **Project Owner/Planner:** selects project scope and drives project-scoped spec-to-execution-card-to-build loops.

### 3.2 Primary Workflows

1. Open `Home`.
2. Create/register project from a unique local directory and verify Git invariant.
3. Select active project from the Home sidebar.
4. Open or continue a project-scoped AI conversation in Home.
5. Draft/refine project specification via explicit AI edit proposals.
6. Accept a spec edit proposal to apply the spec change.
7. Automatically trigger an execution-planning workflow from an accepted spec edit.
8. Review the resulting execution card with `approve`, `reject`, or `request-revision`.
9. Launch build workflows from approved execution-card state when supported by the active flow.
10. Create/open global reusable flows for use within project context.
11. Edit graph, nodes, edges, subgraph/default behavior.
12. Validate and resolve diagnostics.
13. Start run with model and project-scoped working directory.
14. Monitor live state, events, and human-gate prompts.
15. Inspect checkpoint, context, artifacts, graph render.
16. Iterate and re-run within the same project scope.

---

## 4. Information Architecture

The UI MUST provide these first-class areas:

1. **Home**
2. **Editor**
3. **Execution**
4. **Runs**
5. **Settings**

Each area MUST have deterministic, deep-linkable state (project/flow/run/conversation selection + panel state).

### 4.1 Global Regions

- **Left navigation rail:** top-level area switching.
- **Top navigation/header:** product identity, top-level tabs, and active project identity.
- **Home project sidebar:** compact project/thread navigator at top plus a running workflow event log at bottom.
- **Home main workspace:** project-scoped AI conversation with inline spec-edit cards, execution cards, and review controls.
- **Canvas workspace:** graph and contextual overlays.
- **Inspector panel:** graph/node/edge properties.
- **Footer/stream area:** execution controls and runtime output.

### 4.2 Project Scope and Invariants

The UI MUST treat project scope as a first-class operational boundary.

- A project is uniquely identified by directory path.
- Duplicate project registrations for the same directory MUST be rejected.
- Project directory MUST resolve to a Git repository before workflows can run.
- Exactly one project is active at a time for authoring/execution actions.
- Flow definitions are global reusable workflow assets rather than project-owned documents.
- User-configured event or trigger bindings for flows are a future orchestration concern and are out of scope for this document.
- Conversations, project-authored specs, execution-card workflow state, runs, and artifacts MUST be project-scoped.
- Cross-project context/file leakage MUST be prevented by default.
- Home workspace SHOULD provide direct access to project-scoped conversation, spec-edit, and execution-card artifacts.

### 4.3 Home Workspace Requirements

The Home area MUST provide:

- Create/register from local directory path.
- Native directory picker support when the runtime can provide it.
- Duplicate-path prevention at registration/update time.
- Git repository verification for selected directory at registration/activation time.
- Active project selection in a compact left sidebar tree with persistent display in top navigation.
- Per-project conversation thread selection, creation, and deletion from the Home sidebar.
- Project-scoped AI conversation in the main Home pane.
- Agent-emitted proposal/review/apply controls for spec edits inline within the conversation pane (no user-triggered proposal-generation button).
- Inline execution-card review controls in the conversation pane after planning completes.
- Automatic workflow trigger wiring from accepted spec edit proposals to execution-card generation.
- Deterministic deep-link state including active project identity and active conversation.

---

## 5. Core Interaction Model

### 5.1 Selection and Editing

- Single-select node and edge behavior MUST be supported.
- Selection context MUST drive inspector content.
- Unsaved edits MUST be reflected immediately in the canvas model.

### 5.2 Editing Modalities

The UI MUST support:

- Direct manipulation on canvas (move/connect/select).
- Structured form editing in inspectors.
- Raw DOT view/edit fallback for advanced or extension use cases.

### 5.3 Save Semantics

- Save MUST be explicit or autosave with visible status.
- Save failures MUST be surfaced as actionable errors.
- Saved output MUST maintain semantic equivalence with prior DOT unless user changed behavior.

### 5.4 Project-Scoped Context and Isolation

- Actions that mutate flow state or start runs MUST require an active project.
- AI conversation context MUST include active project directory and repository metadata.
- Flow definitions remain global reusable assets even when they are authored or executed from within an active project context.
- Project switching from the Home sidebar MUST reset selection context to the target project (flow/run/conversation) and MUST NOT carry hidden state across projects.

### 5.5 AI Conversation and Spec Authoring Loop

The UI MUST provide a project-scoped AI conversation surface that supports iterative specification authoring.

Source of truth for conversation lifecycle, streaming, retry semantics, and artifact placement:
- `conversation-paradigm.md`

- Users MUST be able to start/continue a conversation within active project context from Home.
- Conversation history MUST persist per project and remain discoverable.
- Project-scoped conversations MUST survive app restart and SHOULD resume the same underlying AI thread when the runtime can restore it.
- The UI MUST support multiple conversation threads per project.
- Users MUST be able to start a new thread from the Home conversation surface without changing active project.
- The Home sidebar SHOULD expose the active project's conversation threads as a compact selectable list adjacent to project navigation.
- Switching threads within a project MUST replace the visible chat history, inline artifacts, and resumed AI session with the selected thread's state.
- Creating a new thread MUST start with an empty visible conversation history while preserving active project scope, repository metadata, and project event log context.
- Chat history in the conversation surface MUST represent user/assistant turns; workflow/system events MUST render in the sidebar event log, not as chat cards.
- Assistant responses SHOULD stream progressively into the conversation surface while a turn is in progress.
- Tool calls and tool output SHOULD render inline in chronological order within the conversation timeline.
- The conversation surface SHOULD auto-follow new content only while the user remains at the live edge, and SHOULD expose an explicit jump-to-bottom affordance when the user scrolls away.
- AI-proposed spec edits MUST be emitted by the assistant/agent turn pipeline and presented as inline conversation cards with explicit, reviewable before/after changes before apply.
- Proposal cards SHOULD render Git-like diff styling and MUST support collapsed/expanded viewing for long diffs.
- Applying spec edits MUST require explicit user confirmation.
- Rejected proposals MUST not mutate spec files.
- Conversation context and proposal artifacts MUST remain isolated to the originating project.
- Accepted spec-edit proposals MUST trigger an execution-planning workflow in project scope.

---

## 6. Authoring Surface Requirements

### 6.1 Graph-Level Attribute Editing

The UI MUST expose all spec-defined graph attributes (see Appendix A), including:

- `goal`, `label`, `model_stylesheet`, `default_max_retry`, `default_fidelity`, `retry_target`, `fallback_retry_target`
- `stack.child_dotfile`, `stack.child_workdir`
- `tool_hooks.pre`, `tool_hooks.post`

Each field MUST include:

- Type-aware validation (string/int/duration/enum where applicable)
- Inline help text with precedence and effect
- Clear default/empty behavior

### 6.2 Node Authoring and Editing

The UI MUST support full node configuration for all spec attributes and handler-specific settings:

- `start`, `exit`, `codergen`, `wait.human`, `conditional`, `parallel`, `parallel.fan_in`, `tool`, `stack.manager_loop`
- Shape-based defaults and explicit `type` override
- Complete advanced attributes (`max_retries`, `goal_gate`, `retry_target`, `fallback_retry_target`, `fidelity`, `thread_id`, `class`, `timeout`, `llm_model`, `llm_provider`, `reasoning_effort`, `auto_status`, `allow_partial`)

### 6.3 Edge Authoring and Editing

The UI MUST support all spec edge attributes:

- `label`, `condition`, `weight`, `fidelity`, `thread_id`, `loop_restart`

Condition fields SHOULD include syntax hints and quick validation feedback.

### 6.4 Subgraphs and Default Blocks

The UI MUST provide first-class authoring for:

- Subgraph creation and label assignment
- Scoped `node [ ... ]` and `edge [ ... ]` defaults
- Visual membership of nodes within subgraphs
- Derived class behavior from subgraph labels

The user MUST be able to choose between explicit per-node attrs and inherited defaults.

### 6.5 Stylesheet Authoring

For `model_stylesheet`, UI MUST provide:

- Syntax-highlighted editor
- Parse/lint feedback
- Selector support guidance (`*`, `.class`, `#id`)
- Effective resolution preview per node

### 6.6 Tool Hook Authoring

For `tool_hooks.pre` and `tool_hooks.post`, UI MUST provide:

- Graph-level editor fields
- Optional node-level overrides where supported by DSL/engine
- Validation and warning when command appears invalid

### 6.7 Manager Loop Authoring

For manager loop support, UI MUST expose:

- Node shape/type selection for `house` / `stack.manager_loop`
- Manager control fields (`manager.poll_interval`, `manager.max_cycles`, `manager.stop_condition`, `manager.actions`)
- Child pipeline linkage via `stack.child_*` attrs

---

## 7. Validation and Diagnostics UX

### 7.1 Diagnostic Surfaces

Diagnostics MUST be shown in three places:

1. Global validation panel (sortable/filterable)
2. Inline node/edge badges
3. Field-level messages in inspectors

### 7.2 Blocking Rules

- Error diagnostics MUST block run start.
- Warning diagnostics SHOULD allow run start with clear warning state.

### 7.3 Navigability

Clicking a diagnostic MUST focus and select the related graph element when available.

---

## 8. Execution and Control UX

### 8.1 Run Initiation

Run form MUST include:

- Active project identity
- Flow source
- Working directory
- Backend/model selection

Working directory SHOULD default to the active project directory.

Run start SHOULD be blocked or explicitly warned when project Git policy checks fail.

The run submission payload MUST align with `/pipelines` contract.

### 8.2 Runtime Controls

UI MUST support run controls according to backend capability:

- Start
- Cancel
- Pause/Resume when available in control API

Unsupported controls MUST be visibly disabled with reason text.

### 8.3 Live State Updates

The canvas MUST reflect node runtime status transitions in near real time.

### 8.4 Execution Footer

Execution controls and status MUST stay visible in canvas footer during active runs.

### 8.5 Spec -> Execution Card -> Build Workflow Orchestration

The UI MUST support workflow chaining from project specification to implementation execution through reviewable execution cards.

- Accepted spec edit proposals MUST trigger a workflow execution that generates an execution card in the originating conversation.
- Execution-card generation MUST run asynchronously with visible workflow-event-log progress.
- Execution cards MUST represent grouped work derived from the approved spec edit and MUST support `approve`, `reject`, and `request-revision`.
- `reject` and `request-revision` actions MUST capture reviewer feedback for the next planning pass.
- Approved execution cards are intended to feed a future work tracker, but tracker ingestion mechanics are out of scope for this document.
- Build/implementation workflows MUST be launchable from approved execution-card state when supported by the active flow.
- Failed planning/build runs MUST expose actionable diagnostics and rerun options.
- Live status/log/artifact visibility MUST be available for planning, review, and build workflows.

---

## 9. Run Inspector Requirements

The UI MUST provide a run inspector for any selected run ID in project context.

### 9.1 Summary

Required fields:

- Status, result, started/ended timestamps, duration
- Working directory, model, flow name
- Project identifier/path and Git branch/commit metadata (when available)
- Last error and token usage

### 9.2 Checkpoint View

From `/pipelines/{id}/checkpoint` show:

- Current node
- Completed nodes
- Retry counters
- Serialized checkpoint metadata

### 9.3 Context View

From `/pipelines/{id}/context` show:

- Searchable key/value table
- Type-aware rendering (string/number/bool/object)

### 9.4 Event Timeline

The timeline MUST render typed events, including:

- Pipeline lifecycle (`PipelineStarted`, `PipelineCompleted`, `PipelineFailed`)
- Stage lifecycle (`StageStarted`, `StageCompleted`, `StageFailed`, `StageRetrying`)
- Parallel and interview events
- Checkpoint events (`CheckpointSaved`)

Timeline MUST support filtering by event type and node.

### 9.5 Artifact Browser

The UI MUST expose run artifact directories/files where available:

- `manifest.json`
- `checkpoint.json`
- Per-node `prompt.md`, `response.md`, `status.json`
- `artifacts/*`
- Graphviz render output

### 9.6 Project Run History and Traceability

- Run history MUST be durable and filterable by project.
- Each run record MUST link to project identity and available Git metadata (branch/commit).
- Where available, run records SHOULD link associated spec and execution-card artifacts.
- Timeline and summary surfaces MUST preserve timestamps sufficient for audit reconstruction.

---

## 10. Human-in-the-Loop UX

### 10.1 Prompt Handling

Human-gate prompts MUST be operable from web controls without CLI dependency.

### 10.2 Question Types

UI MUST support rendering semantics for:

- `MULTIPLE_CHOICE`
- `YES_NO`
- `CONFIRMATION`
- `FREEFORM` (when exposed by backend)

### 10.3 Timeout/Default Semantics

UI MUST represent timeout and default behavior clearly:

- Configurable/default path (`human.default_choice`)
- Distinguish accepted answer vs timeout fallback vs skipped

### 10.4 Multi-Question and Inform Messages

If backend emits multi-question or inform patterns, UI SHOULD group them by stage and preserve order.

---

## 11. Persistence and DOT Serialization

Source of truth for app-owned versus project-owned storage:
- `storage-boundaries.md`

### 11.1 Canonical Model

UI editor state MUST map to a canonical flow model that can represent all spec constructs, not only currently rendered controls.

### 11.2 Round-Trip Requirements

For any spec-valid DOT input:

1. Open in UI
2. Perform no-op save
3. Re-parse and compare semantics

Result MUST remain semantically equivalent.

### 11.3 Mixed-Mode Editing

When users edit in raw DOT and visual mode interchangeably, the UI MUST avoid destructive rewrites.

### 11.4 Extension Attributes

Attributes outside core spec SHOULD be preserved and editable through a generic advanced key/value editor.

### 11.5 Project Workspace Persistence

- Project registry data MUST persist across sessions with unique-directory enforcement.
- Project-scoped conversation history and spec-edit/execution-card artifacts MUST remain linked to the originating project.
- Restore-on-reopen behavior MUST rehydrate the last active project and active thread context safely.
- Home sidebar project/thread selection state SHOULD persist across sessions where practical.

### 11.6 Spec and Execution Artifact Provenance

- Spec-edit and execution-card artifacts produced through UI workflows MUST include or reference provenance metadata.
- Provenance MUST include run linkage and timestamps, and SHOULD include available branch/commit context.
- Execution-card status transitions (draft/approved/rejected/revision-requested/superseded) MUST be persisted and recoverable.

---

## 12. API Integration Contract

### 12.1 Required Endpoints

UI integrations MUST cover:

- `/api/flows`, `/api/flows/{name}`
- `/preview`
- `/pipelines`, `/pipelines/{id}`
- `/pipelines/{id}/events`
- `/pipelines/{id}/cancel`
- `/pipelines/{id}/graph`
- `/pipelines/{id}/questions`
- `/pipelines/{id}/questions/{qid}/answer`
- `/pipelines/{id}/checkpoint`
- `/pipelines/{id}/context`
- `/api/projects/metadata`
- `/api/projects/pick-directory`
- `/api/projects/conversations`
- `/api/conversations/{id}`
- `/api/conversations/{id}/events`
- `/api/conversations/{id}/turns`
- `/api/conversations/{id}/spec-edit-proposals/{proposalId}/approve`
- `/api/conversations/{id}/spec-edit-proposals/{proposalId}/reject`
- `/api/conversations/{id}/execution-cards/{executionCardId}/review`
- `/runs`, `/status`

### 12.2 Contract Drift Handling

If an endpoint is unavailable or shape changes, UI MUST:

- Show a clear degraded-state message
- Keep non-dependent areas functional
- Avoid destructive save behavior

### 12.3 Project Scope Contract

- Project selection and active-project identity MUST be persisted by the UI.
- Project identity passed to execution surfaces MUST resolve to a concrete working directory.
- Conversation/spec-edit/execution-card state MUST be retrievable by project identity.

### 12.4 Workflow Orchestration Contract

UI integrations MUST cover backend contracts for:

- Project-scoped conversation turns and history retrieval.
- Proposal of spec edits and explicit apply/reject actions.
- Triggering execution-planning workflows on accepted spec edits.
- Execution-card generation and status retrieval.
- Human approval/rejection/revision transitions for execution cards.
- Build workflow invocation from approved execution-card state.

---

## 13. Accessibility, Responsiveness, and Performance

### 13.1 Accessibility

UI MUST meet practical WCAG-oriented requirements:

- Keyboard navigable core flows
- Focus-visible states
- Semantic labels for form controls
- Color contrast suitable for diagnostics/status states

### 13.2 Responsiveness

UI MUST remain usable at mobile and desktop breakpoints.

Minimum:

- Inspector sections collapse gracefully
- Event timeline and diagnostics remain reachable

### 13.3 Performance

Targets:

- Canvas interactions remain responsive for medium-sized graphs
- Event timeline rendering handles sustained stream updates
- Inspector operations do not block canvas interactions

---

## 14. Parity Program Breakdown

Project + parity delivery is intentionally large and is decomposed into implementation elements.

### 14.0 E0 - Home Workspace Foundation (P0)

Scope:

- Add Home as a first-class top-level area.
- Move project registration and active-project selection into Home.
- Enforce unique-directory and Git-repository invariants.
- Establish compact Home project/thread tree navigation.
- Establish project-scoped conversation/session state and deep-linking.
- Enforce cross-project context isolation for authoring/execution surfaces.

User story:

- As a user, I can choose an active project from Home and safely run the spec-to-execution-card-to-build loop without context leakage.

### 14.1 E1 - Data Model Parity Foundation (P0)

Scope:

- Extend frontend flow model to include all graph/node/edge attrs and subgraph/default constructs.

User story:

- As an author, I can load any spec-valid flow and see/edit all required configuration.

### 14.2 E2 - Graph Attribute Completeness (P0)

Scope:

- Add missing graph attrs (`stack.child_*`, `tool_hooks.*`) with typed fields.

User story:

- As an author, I can configure manager and tool-hook behavior without raw DOT edits.

### 14.3 E3 - Node Handler Completeness (P0)

Scope:

- Expose full handler selection and handler-specific fields, including manager-loop attrs.

User story:

- As an author, I can configure every handler type from the UI.

### 14.4 E4 - Subgraph and Defaults Authoring (P0)

Scope:

- Author/visualize subgraphs and scoped node/edge defaults.

User story:

- As an author, I can model inheritance/scoping explicitly.

### 14.5 E5 - Condition and Routing Tooling (P1)

Scope:

- Better condition authoring assistance and edge-selection explainability.

User story:

- As an author/operator, I can predict and debug route decisions.

### 14.6 E6 - Stylesheet Tooling (P1)

Scope:

- Lint + selector preview + effective model resolution UI.

User story:

- As an author, I can trust model routing before run time.

### 14.7 E7 - Human Gate Advanced UX (P1)

Scope:

- Question type semantics, timeout/default-path semantics, answer provenance.

User story:

- As an operator, I can handle all interviewer scenarios confidently.

### 14.8 E8 - Run Inspector Shell (P0)

Scope:

- Structured run detail page/panel.

User story:

- As an operator, I can inspect full run metadata from one place.

### 14.9 E9 - Checkpoint and Context Inspector (P0)

Scope:

- Dedicated checkpoint/context UI with search/filter.

User story:

- As a debugger, I can inspect state evolution without opening files manually.

### 14.10 E10 - Event Timeline Completeness (P0)

Scope:

- Full typed event rendering from SSE with filters and correlations.

User story:

- As an operator, I can identify failures/retries quickly.

### 14.11 E11 - Artifact Browser and Graph Render Viewer (P0)

Scope:

- Browse/download run artifacts and view rendered pipeline graph.

User story:

- As an auditor, I can verify prompts, outputs, and outcomes.

### 14.12 E12 - No-Loss Serialization Guarantees (P0)

Scope:

- Semantic round-trip tests and preservation of extension attrs.

User story:

- As an author, I can safely edit flows visually without corruption.

### 14.13 E13 - Accessibility and Mobile Hardening (P2)

Scope:

- Keyboard coverage, mobile usability pass, focus/state consistency.

### 14.14 E14 - End-to-End Parity Certification (P0)

Scope:

- Test suite and checklist proving full UI parity with spec tables.

### 14.15 E15 - Spec Authoring Conversation Workflow (P0)

Scope:

- Home-based project-scoped AI conversation for iterative spec drafting.
- Explicit proposal/review/apply model for spec edits.
- Durable conversation history and artifact linkage by project.
- Trigger execution-card planning automatically on accepted spec edits.
- Stream assistant/tool activity directly in the conversation surface.

User story:

- As a project author, I can collaborate with AI on spec drafting, accept explicit edits, and automatically trigger execution planning without cross-project leakage.

### 14.16 E16 - Execution Card Governance and Build Launch (P0)

Scope:

- Execution-card workflow execution from accepted spec edits.
- Human review for execution cards (`approve`, `reject`, `request-revision`).
- Build launch from approved execution-card state and failure-recovery UX.

User story:

- As an operator/reviewer, I can gate execution-card approval and run builds from approved work packages with full traceability.

---

## 15. Definition of Done

A parity-complete UI release is done when all conditions hold:

1. Every item in Appendix A and Appendix B is marked supported in UI.
2. Any spec-valid flow can be authored, saved, and re-opened without behavior loss.
3. Validation blocks run on error diagnostics.
4. Run inspector exposes status, timeline, checkpoint, context, graph render, and artifacts.
5. Human gates are fully operable from UI for supported question types.
6. Full frontend build/lint/test passes.
7. End-to-end parity tests pass against representative fixtures.
8. Home is first-class in IA with integrated project sidebar, enforces unique-directory + Git invariants, and isolates context across projects.
9. Project-scoped conversation/spec-edit/execution-card/build loop is fully operable with explicit spec-edit and execution-card approval gates.
10. Per-project run history/provenance supports audit reconstruction of spec-edit/execution-card/build outcomes.

---

## Appendix A: Attribute Coverage Matrix

The UI MUST support these attributes as first-class fields or generated structured controls.

### A.1 Graph Attributes

- `goal`
- `label`
- `model_stylesheet`
- `default_max_retry`
- `default_fidelity`
- `retry_target`
- `fallback_retry_target`
- `stack.child_dotfile`
- `stack.child_workdir`
- `tool_hooks.pre`
- `tool_hooks.post`

### A.2 Node Attributes

- `label`
- `shape`
- `type`
- `prompt`
- `tool_command`
- `max_retries`
- `goal_gate`
- `retry_target`
- `fallback_retry_target`
- `fidelity`
- `thread_id`
- `class`
- `timeout`
- `llm_model`
- `llm_provider`
- `reasoning_effort`
- `auto_status`
- `allow_partial`
- `join_policy`
- `error_policy`
- `max_parallel`
- `manager.poll_interval`
- `manager.max_cycles`
- `manager.stop_condition`
- `manager.actions`
- `human.default_choice`

### A.3 Edge Attributes

- `label`
- `condition`
- `weight`
- `fidelity`
- `thread_id`
- `loop_restart`

---

## Appendix B: Construct Coverage Matrix

The UI MUST provide authoring and inspection for:

1. Directed graph with valid node IDs
2. Chained edges
3. Node default blocks
4. Edge default blocks
5. Subgraphs with scoped defaults
6. Subgraph-derived class behavior
7. Shape-to-handler mapping with explicit `type` override
8. Retry and goal-gate configuration
9. Fidelity and thread resolution controls
10. Model stylesheet selectors and precedence
11. Human gate defaults and timeout behavior
12. Parallel fan-out and fan-in configuration
13. Tool handler command configuration
14. Manager loop supervision configuration
15. Runtime checkpoint/context/events/artifact inspection

---

## Appendix C: User Story Coverage Map

The following map identifies where implementing this spec satisfies each story in `ui-user-stories.md`.

- `US-PROJ-01`: Sections 3.2, 4.3
- `US-PROJ-02`: Sections 4.2, 4.3
- `US-PROJ-03`: Sections 3.2, 4.2, 4.3, 8.1
- `US-PROJ-04`: Sections 4.1, 4.3
- `US-PROJ-05`: Section 4.3
- `US-PROJ-06`: Section 4.3
- `US-HOME-01`: Sections 3.2, 4, 4.1, 4.3
- `US-HOME-02`: Sections 3.2, 4.1, 4.3, 5.4
- `US-HOME-03`: Sections 4.1, 4.3
- `US-CONV-01`: Sections 3.2, 5.5
- `US-CONV-02`: Sections 5.4, 5.5
- `US-CONV-03`: Sections 3.2, 5.5
- `US-CONV-04`: Sections 5.5, 12.4
- `US-CONV-05`: Sections 5.5, 11.5
- `US-CONV-06`: Sections 4.2, 5.4, 5.5
- `US-WORK-01`: Sections 3.2, 5.5, 8.5, 12.4
- `US-WORK-02`: Sections 8.5, 11.6, 12.4
- `US-WORK-03`: Sections 8.5, 10, 12.4
- `US-WORK-04`: Sections 3.2, 8.5, 11.6, 12.4
- `US-WORK-05`: Sections 8.5, 12.4
- `US-WORK-06`: Sections 7, 8.5, 9.1, 9.4, 9.5
- `US-WORK-07`: Sections 7, 8.5, 9.4, 9.5
- `US-GOV-01`: Sections 4.2, 5.4, 8.1
- `US-GOV-02`: Sections 8.1, 12.3
- `US-GOV-03`: Sections 9.1, 9.6, 11.6
- `US-GOV-04`: Sections 9.6, 11.5, 11.6
- `US-GOV-05`: Sections 2, 5.3, 11.3, 12.2
- `US-IA-01`: Sections 4, 4.1, 4.3
- `US-IA-02`: Sections 4, 4.3, 12.3
- `US-IA-03`: Sections 3.2, 4.1, 4.3, 8.5, 9
