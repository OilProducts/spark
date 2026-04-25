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
- flow-run requests
- conversation-linked flow launches
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
- flow-run request review state
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

Stateful top-level surfaces use separate client-ephemeral view sessions.
At minimum the frontend must model:
- `homeSession`
- `editorSession`
- `executionSession`
- `runsSession`
- `triggersSession`

Switching away from any stateful top-level surface must restore that surface exactly as the operator left it rather than deriving one surface's local state from another.
Exact restoration includes drafts, selections, filters, expansion state, layout state, and scroll position for long-scroll operator surfaces.
If a hidden surface has an active live transport, hiding it must not terminate that transport. Visibility is not session destruction.

## 8. Presentation Surfaces

### 8.1 Workspace Surfaces

Workspace-oriented surfaces include:
- navbar project switcher
- conversation thread list
- project chat
- inline flow-run request cards
- inline flow-launch cards
- review controls

### 8.2 Attractor Surfaces

Attractor-oriented surfaces include:
- flow editing and validation
- run history
- run journal browser
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
- continuation configuration and submission
- a collapsible launch-planning graph card below the launch inputs and notice stack

Runs owns:
- selected run summary and actions
- pinned pending questions
- run journal
- an `Advanced` disclosure for graph, checkpoint, context, and artifacts

For selected-run inspection, Runs treats backend run-detail state as authoritative.
The selected run summary, lifecycle badge, and detail surfaces must reconcile against `GET /pipelines/{id}` rather than relying on the runs list row or event-stream-local status.
The runs list hydrates from `GET /runs` and then updates from `GET /runs/events`; it remains an overview/navigation surface rather than a second authority for the selected run.
Selected-run inspection hydrates authoritative run detail from `GET /pipelines/{id}`, durable history from `GET /pipelines/{id}/journal`, and then live tail updates from `GET /pipelines/{id}/events`.
The run summary should fold in current monitoring facts rather than relying on a separate `Run Activity` card.
The `Advanced` disclosure must be collapsed by default.
Run Journal must use durable selected-run history from the journal endpoint, not only events observed while the tab was open.
`GET /pipelines/{id}/events` is a live-tail SSE channel with optional `after_sequence` gap-fill; it is not the primary full-history API.
Steady-state polling is not part of the Runs contract.
The runs list must not show polling-era stale-data warnings or background-refresh affordances.
Runs must own exactly one per-run SSE transport for the selected run; summary monitoring facts, journal live tail, node status, and pending human-gate state all share that transport.

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
- inline workspace artifacts such as flow-run requests and flow launches

Conversation UX commitments include:
- an assistant row should appear promptly after the user sends a message
- progressive assistant output should stream into that row when available
- streaming updates in long project-chat threads should stay localized enough that existing history and composer input remain responsive
- tool-call rows may appear before, between, or after assistant text updates without breaking timeline order
- a placeholder such as `Thinking...` may remain until the first assistant text arrives
- final completion should finalize the active assistant row rather than append a duplicate
- failure should convert the active assistant row into an explicit failure state or remove it cleanly
- switching away from Home and back must preserve the selected thread, visible history, draft text, layout state, and expansion state
- background assistant activity must continue to reconcile into the active thread session even while Home is hidden
- returning to Home must not transiently drop the thread list or show a false `No threads` state while session-backed data is still restoring
- normal assistant text rows in Home chat render markdown as a frontend presentation rule over existing string content
- user text, thinking summaries, tool rows, and inline flow artifact rows keep their literal or custom rendering paths
- markdown links render as non-interactive label text until dedicated link handling exists

The UI must not reconstruct chat cards from raw protocol notifications.

## 11. Review and Approval UX

Flow-run request approval is an explicit human action.

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

### 12.4 Continuation Launches

Runs initiates continuation, but Execution owns continuation configuration and launch.

The continuation UX should work as follows:
- Runs exposes `Continue from node` only for inactive runs
- clicking that action immediately switches to Execution
- Execution loads a continuation draft prefilled from the source run
- the default graph source is the stored source-run snapshot
- selecting an installed flow in the Execution sidebar switches continuation into installed-flow override mode
- restart-node selection happens in Execution by clicking the graph
- Execution warns that arbitrary restart nodes may depend on upstream state and may fail when restarted cold
- launching a continuation returns the operator to Runs with the derived run selected

Execution may render the graph card for direct launches as a read-only preview, but that card is still a launch-planning surface rather than a live run monitor.

## 13. Run Inspection UX

The run inspector is a frontend for Attractor runs, not for workspace artifacts.

It should render:
- run summary
- run-level actions
- pinned pending questions when present
- Run Journal
- an `Advanced` disclosure for checkpoint, run graph based on the stored run snapshot rather than the current named flow, context, and artifacts

The primary monitoring hierarchy is:
- summary
- pinned pending questions
- Run Journal
- `Advanced`

The selected-run summary is the primary answer to `what is happening now?` and should reconcile against authoritative `GET /pipelines/{id}` detail state.
Run Journal is the durable operational history browser. It should carry logs, child-flow activity, and question audit history inline rather than splitting those concerns into separate primary surfaces.
When the graph lacks live state overlays, it is a secondary reference surface rather than a primary monitoring surface.
The `Advanced` disclosure remains collapsed by default.

Run history and run inspection must render lifecycle status separately from workflow outcome.
Examples:
- `Status: Completed`, `Outcome: Success`
- `Status: Completed`, `Outcome: Failure`
- `Status: Failed`, `Outcome: —`

When the user reaches run inspection from a workspace surface, the UI should preserve project framing by default rather than dropping into an unscoped global run context.

Run-inspection state is run-local and may be shared across tabs, but it is distinct from the execution launch session.
The currently selected execution flow, selected run, launch form draft, and runtime inspection context must not be inferred from the Editor session.
Runs session restoration must preserve:
- run-list scope mode
- selected run
- journal filters
- artifact and context inspection selection
- pending freeform human-gate answers

Runs is the canonical run-monitoring surface.
Execution may show compact handoff notices for the currently selected run, such as a launch-success message or pending human-gate reminder, and may render a launch-planning graph card. It must not own the primary run journal, artifact, or question surfaces.

`Completed + failure outcome` is not a runtime failure surface.
It should be presented as a completed workflow that reached a negative business conclusion, optionally with a reason code or reason message supplied by Attractor.

Journal and lifecycle summaries for `PipelineCompleted` should include:
- workflow outcome
- optional outcome reason code
- optional outcome reason message

Run summary monitoring facts and journal summaries should prefer a backend-provided failure reason over a generic lifecycle label when one is available.
Run Journal is the durable selected-run history browser.
Nested child-flow activity should appear inline and clearly labeled in the Run Journal when the parent run executed child flows through `stack.manager_loop`.

## 14. Frontend State Model

The frontend treats backend-provided state as authoritative for:
- project identity
- conversation identity
- artifact status
- run identity
- run lifecycle status
- run workflow outcome
- provenance links

For run inspection specifically:
- the selected run detail surface is authoritative on backend run-detail state from `GET /pipelines/{id}`
- the runs list is an overview cache and must not outrank selected-run detail state
- event streams are additive live-update channels for logs, node status, human gates, and journal live-tail enrichment
- event streams must reconcile with authoritative per-run detail state so missed terminal events do not leave the UI stale
- run-list live updates come from `/runs/events`, not from steady-state timer refreshes
- selected-run live updates come from `/pipelines/{id}/events` after initial `GET /pipelines/{id}` hydration
- the selected run may have only one live per-run stream subscription at a time
- selected-run journal history must survive reselecting the run later; the UI must not depend on the run having been open live
- selected-run journal history may include inline child-flow events when the backend marks them with child-source metadata

The frontend may keep local ephemeral state for:
- selected tab
- expanded or collapsed cards
- `homeSession`
- `editorSession`
- `executionSession`
- `runsSession`
- `triggersSession`
- local filter or sort controls
- draft message text

Run inspection state should remain separate from `executionSession` even when both surfaces can reference the same selected run id.
Trigger selection and unsaved create/edit drafts must remain in `triggersSession` rather than disappearing on top-level navigation.
Home thread cache, draft state, and conversation UI affordances must remain in `homeSession` rather than on component mount lifetime.

These sessions are client-ephemeral only.
They are not backend-authoritative, and in v1 they do not need to survive a full reload or restart.

Empty-state rendering must be status-aware.
`[]` must never mean both `not yet loaded` and `loaded empty`.
Collection or detail empty states may render only after the relevant resource reaches `ready` with zero items.

## 15. Error, Degraded, and Partial-Availability UX

When a backend surface is unavailable or incompatible:
- the affected UI should enter an explicit degraded state
- unaffected surfaces should remain usable where feasible
- the client must not fabricate missing authority from stale local state
- save and launch paths should remain non-destructive
- Runs transport degradation must surface as one Runs-tab reconnect notice summarizing whether the run-list stream, the selected-run stream, or both are degraded
- Runs reconnect must rehydrate `GET /runs` and, when a run is selected, `GET /pipelines/{id}` before reopening the corresponding SSE streams
- selected-run stream degradation belongs to Runs, not to the app-wide save indicator

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
- project-scoped AI conversation, flow-run request review, and direct flow launches
- execution governance, traceability, and auditability
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
