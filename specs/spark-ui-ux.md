# Spark UI/UX Specification

This document is the canonical operator-facing client specification for Spark.

## 1. Purpose

The Spark frontend is the operator-facing application for:
- project-scoped collaboration
- flow authoring
- run inspection
- human review and approval of workspace artifacts

The frontend composes workspace data from Spark and execution data from Attractor without becoming either of them.

## 2. Scope and Design Intent

Spark is organized around a home-first operator workflow with an active-project workspace context:
- choose or add the active project
- collaborate in project chat
- review artifacts
- author or inspect flows
- monitor and inspect runs

The active project frames conversations, approvals, execution launch defaults, trigger execution defaults, and default run-history scope.
Flow authoring itself operates on shared workspace flows rather than per-project copies.
Trigger identity remains workspace-global even when trigger execution targets a project.

Story material exists to support implementation, testing, and auditing. It does not override the normative client contract sections below.

## 3. Relationship to Workspace and Attractor

The frontend intentionally targets two distinct backend surfaces:
- Spark Workspace for project, conversation, and review data
- Attractor for flow and run data

Workspace artifacts and Attractor runs remain conceptually separate even when the UI shows them side by side.

## 4. Client Boundaries

### 4.1 The Frontend Is Not the Engine

The frontend does not execute pipelines.

It consumes Attractor through API surfaces for:
- run submission
- status
- event streams
- questions
- checkpoint, context, graph, and artifacts

### 4.2 The Frontend Is Not the Workspace Source of Truth

The frontend does not durably own:
- projects
- conversations
- proposals
- execution cards
- review decisions
- provenance links

### 4.3 No Hidden Local Authority

The frontend must not persist local state that overrides backend truth for:
- which artifacts exist
- whether an artifact was approved
- which run an artifact launched
- which flows workspace triggers resolve to

When local state becomes invalid relative to backend state, the backend wins.

## 5. Data Sources and Service Separation

### 5.1 Workspace Data

The frontend consumes workspace data for:
- project list and active project
- project conversations
- inline conversation artifacts
- proposal and execution-card review state
- workspace trigger definitions and runtime summaries
- provenance references to flows and runs

### 5.2 Attractor Data

The frontend consumes Attractor data for:
- flows
- run list
- run status
- run events
- checkpoint
- context
- graph render
- questions
- artifacts

### 5.3 Separation Rule

The frontend must preserve the conceptual separation between:
- workspace artifacts
- Attractor runs

Workflow launch flow selection comes from workspace trigger configuration or explicit operator overrides. It must not be inferred from whichever flow happens to be open in the editor.

## 6. Primary User Workflows

The primary workflows are:
- Home and project selection
- project-scoped conversation and artifact review
- flow authoring and validation
- direct execution launch
- run inspection

These workflow summaries are normative at the experience level. Detailed story records remain appendix-style support material.

## 7. Navigation Model

Home is the default top-level workspace for:
- project framing
- thread selection
- AI collaboration

The navbar owns project switching and project management.
It must present a persistent active-project switcher cluster with:
- active project identity
- project add
- clear active project
- remove active project

Navigation should preserve project framing as the operator moves from conversation to Execution and Runs whenever that framing still applies.
Execution and Runs are project-framed views by default and should expose a compact project-context indicator near the view header.
The Editor may open with no active project. Execution may also open without a project, but run-start actions must remain locally gated on selecting a project.
Execution is the direct-run launch surface. Runs is the canonical selected-run monitoring and inspection surface.
Triggers are a first-class top-level workspace view because they are global automations rather than project settings; however, new and edited trigger execution targets should default to the active project when that default is meaningful.
Settings remain global and should not imply project ownership.

Editor and Execution are separate in-memory UI sessions.
Switching between them must restore each mode exactly as the operator left it rather than deriving one mode's local state from the other.

## 8. Presentation Surfaces

### 8.1 Workspace Surfaces

Workspace-oriented surfaces include:
- navbar project switcher
- conversation thread list
- project chat
- inline spec proposal cards
- inline flow-run request cards
- inline execution cards
- review controls

### 8.2 Attractor Surfaces

Attractor-oriented surfaces include:
- flow editing and validation
- run history
- run event timeline
- checkpoint view
- context view
- run graph view
- artifact inspection
- human-question answering

Execution owns:
- flow selection for direct runs
- launch-input form rendering from flow metadata
- launch diagnostics and validation blockers
- direct-run submission controls
- post-launch navigation choice

Runs owns:
- selected run summary and actions
- live run graph
- terminal/log stream
- run event timeline
- checkpoint, context, artifacts, and questions

### 8.3 Bridge Surfaces

Bridge surfaces show how workspace artifacts connect to Attractor execution.

These are provenance views, not ownership transfers.

## 9. Project Selection UX

The UI should support:
- one active project at a time
- visible active project identity in the navbar and in project-framed views
- duplicate-path prevention
- short project labels with full paths visible enough for safe selection
- recent or favorite project switching when supported
- active-project add, clear, and remove actions without requiring a trip back to Home

Project identity itself remains workspace truth even though the UI presents it.

### 9.1 Project Context Propagation

The active project is a shared workspace context, not a Home-only decoration.

That context should propagate as follows:
- Home shows threads for the active project rather than a standalone project list
- Execution defaults direct runs to the active project and makes that target explicit in launch copy
- Runs defaults to active-project scope and provides a one-click `All projects` escape hatch
- Triggers remain a global registry, but trigger execution-target controls should default to the active project and make project targeting visible in list rows and detail panels
- Editor and Settings remain visually and conceptually global

## 10. Conversation UX

The frontend renders project chat from the workspace conversation contract rather than from raw model protocol messages.

The conversation surface should show:
- user turns
- assistant turns
- thinking or reasoning blocks
- tool activity
- inline workspace artifacts such as spec proposals, flow-run requests, and execution cards

Conversation UX commitments include:
- an assistant row should appear promptly after the user sends a message
- progressive assistant output should stream into that row when available
- tool-call rows may appear before, between, or after assistant text updates without breaking timeline order
- a placeholder such as `Thinking...` may remain until the first assistant text arrives
- final completion should finalize the active assistant row rather than append a duplicate
- failure should convert the active assistant row into an explicit failure state or remove it cleanly

The UI must not reconstruct chat cards from raw protocol notifications.

## 11. Review and Approval UX

Spec edit proposal approval, flow-run request approval, and execution-card approval are explicit human actions.

The UI must present these as review controls, not as automatic assistant continuations.

When approval causes downstream execution, the UI should display:
- what artifact was approved
- what flow was launched
- what Attractor run id was created
- an affordance to open the resulting run

The UI must not imply approval unless the backend has recorded it explicitly.

## 12. Flow Authoring UX

Flow authoring is a client experience layered on top of Attractor contracts.
It operates on shared workspace flows, not project-local flow copies.

The UI may provide:
- structured editing
- raw DOT editing
- validation visibility
- preview and save workflows
- degraded-state feedback when API or validation calls fail

This section is about the authoring experience, not the DOT DSL itself.

Authoring session state is editor-local.
The frontend must not infer editor selection, inspector scope, draft DOT, validation state, or save state from whichever flow is currently open in Execution.

### 12.1 Flow Contract Authoring

Spark authoring surfaces should make the flow's launch and context contract visible without forcing raw DOT edits for common cases.

The structured graph inspector should support:
- graph title and description
- graph launch input declarations
- graph defaults and advanced attrs

Launch input declarations should:
- edit the persisted `spark.launch_inputs` metadata
- define the `context.*` values Spark should collect before a direct run
- make required vs optional inputs explicit
- render stable user-facing labels and descriptions instead of raw key names alone

### 12.2 Node Context Contract Authoring

The structured node inspector should support node-level context contract declarations.

At minimum, Spark should expose:
- `Reads Context`
- `Writes Context`

These fields should edit the persisted `spark.reads_context` and `spark.writes_context` metadata rather than inventing hidden local-only state.

The primary UX purpose is to:
- document what launch or prior-stage state a node expects
- document what state a node is expected to produce for later stages
- help operators understand retry loops and multi-stage feedback flows

These declarations do not, by themselves, create runtime behavior. They are authoring-surface contract metadata.

### 12.3 Launch Form Generation

When a flow declares launch inputs, direct-run launch surfaces should render a launch form from that metadata.

That launch form should:
- appear before run submission
- collect values using the declared field types
- validate required inputs
- convert the submitted values into Attractor `launch_context`
- surface invalid launch-schema problems as explicit launch blockers rather than failing silently

The frontend should not require the operator to hand-author raw JSON for routine launch input cases if the flow already declares them.

Launch-form state is execution-local.
Selecting or editing a flow in the Editor must not overwrite the currently selected execution flow, launch draft values, or launch failure state.
Execution should provide an explicit post-launch navigation choice such as `Open in Runs after launch`.
That control defaults to unchecked. When unchecked, Execution remains visible after a successful launch and shows a compact success notice with a `View run` affordance.

## 13. Run Inspection UX

The run inspector is a frontend for Attractor runs, not for workspace artifacts.

It should render:
- run summary
- run-level actions
- run graph based on the stored run snapshot rather than the current named flow
- terminal/log stream
- event timeline
- checkpoint
- context
- artifacts
- pending questions when present

Run history and run inspection must render lifecycle status separately from workflow outcome.
Examples:
- `Status: Completed`, `Outcome: Success`
- `Status: Completed`, `Outcome: Failure`
- `Status: Failed`, `Outcome: —`

When the user reaches run inspection from a workspace surface, the UI should preserve project framing by default rather than dropping into an unscoped global run context.

Run-inspection state is run-local and may be shared across tabs, but it is distinct from the execution launch session.
The currently selected execution flow, selected run, launch form draft, and runtime inspection context must not be inferred from the Editor session.

Runs is the canonical run-monitoring surface.
Execution may show compact handoff notices for the currently selected run, such as a launch-success message or pending human-gate reminder, but it must not own the primary run graph or terminal surface.

`Completed + failure outcome` is not a runtime failure surface.
It should be presented as a completed workflow that reached a negative business conclusion, optionally with a reason code or reason message supplied by Attractor.

Timeline and lifecycle summaries for `PipelineCompleted` should include:
- workflow outcome
- optional outcome reason code
- optional outcome reason message

Terminal and run-log summaries should prefer a backend-provided failure reason over a generic lifecycle label when one is available.

## 14. Frontend State Model

The frontend treats backend-provided state as authoritative for:
- project identity
- conversation identity
- artifact status
- run identity
- run lifecycle status
- run workflow outcome
- provenance links

The frontend may keep local ephemeral state for:
- selected tab
- expanded or collapsed cards
- editor session state
- execution session state
- run inspection session state
- draft message text
- local filter or sort controls
- trigger execution-target mode for the editor form
- runs scope mode (`active project` vs `all projects`)

At minimum, the frontend should model two independent mode-local sessions:
- `editorSession`
- `executionSession`

Run inspection state should remain separate from `executionSession` even when both surfaces can reference the same selected run id.

These sessions are client-ephemeral only.
They are not backend-authoritative, and in v1 they do not need to survive a full reload or restart.

## 15. Error, Degraded, and Partial-Availability UX

When a backend surface is unavailable or incompatible:
- the affected UI should enter an explicit degraded state
- unaffected surfaces should remain usable where feasible
- the client must not fabricate missing authority from stale local state
- save and launch paths should remain non-destructive

Degraded behavior is part of the operator contract, not an implementation accident.

## 16. Accessibility and Responsiveness Expectations

The UI should preserve core workflows across:
- desktop and narrow/mobile layouts
- keyboard-accessible review and navigation flows where supported
- readable project framing and status visibility in constrained layouts

Only durable, testable UX expectations belong here.

## 17. User Story Catalog

The current stable story groups are:
- Home workspace and project selection
- project-scoped AI conversation and spec editing
- spec -> plan -> work tracker -> build chain
- governance, safety, and auditability
- UX and information architecture implications

Story IDs remain useful for traceability and test planning, but the catalog does not create independent backend truth.

## 18. Detailed Story Records

Detailed story records remain useful for:
- rationale
- acceptance criteria
- non-goals
- implementation intent
- references

Those records are appendix-style support material. They should clarify or trace the normative UX sections, not replace them.

## 19. Traceability to Workspace and Attractor Contracts

Each substantial UI obligation should be traceable back to:
- a workspace contract section
- an Attractor contract section
- or a clearly identified client-only UX requirement

That traceability exists to prevent:
- UX sections from quietly redefining backend truth
- stories from becoming a second source of authority
- implementation intent from being mistaken for normative contract language
