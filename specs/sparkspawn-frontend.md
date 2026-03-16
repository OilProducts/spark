# Spark Spawn Frontend Specification

This document defines the Spark Spawn frontend as an independent client application.

It is intentionally separate from:
- [attractor-spec.md](/Users/chris/tinker/sparkspawn/specs/attractor-spec.md), which defines the execution engine and run APIs
- [sparkspawn-workspace.md](/Users/chris/tinker/sparkspawn/specs/sparkspawn-workspace.md), which defines project/conversation/artifact domain behavior

This document focuses on presentation, interaction, client-side state boundaries, and how the frontend consumes backend surfaces.

---

## 1. Purpose

The Spark Spawn frontend is the operator-facing application for:
- project-scoped collaboration
- flow authoring
- run inspection
- human review and approval of workspace artifacts

The frontend must be able to talk to:
- a Spark Spawn workspace service
- any compliant Attractor implementation

It MUST NOT require the frontend to own or emulate engine behavior.

When both backend surfaces are hosted by one deployable server, the frontend should still treat them as distinct API surfaces rather than as one undifferentiated backend.

---

## 2. Client Boundaries

### 2.1 The Frontend Is Not the Engine

The frontend does not execute pipelines.

The frontend consumes Attractor through its API surfaces:
- run submission
- status
- event stream
- questions
- checkpoint/context/graph/artifacts

### 2.2 The Frontend Is Not the Workspace Source of Truth

The frontend does not durably own:
- projects
- conversations
- proposals
- execution cards
- review decisions
- provenance links

These must come from backend services.

### 2.3 The Frontend Is a Composition Layer

The frontend’s main responsibility is to present and coordinate:
- workspace data from Spark Spawn
- run and flow data from Attractor

without forcing either backend to adopt the other’s domain model.

---

## 3. Data Sources

### 3.1 Workspace Data

The frontend consumes Spark Spawn workspace data for:
- project list and active project
- project conversations
- inline conversation artifacts
- proposal and execution-card review state
- project-to-flow associations
- trigger-to-flow bindings
- provenance references to flows and runs

Trigger bindings are explicit workspace data. The frontend MUST NOT infer workflow launch flow selection from whichever flow is currently open in an editor or inspector.

### 3.2 Attractor Data

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

### 3.3 Contract Separation

The frontend MUST preserve the conceptual separation between:
- workspace artifacts
- Attractor runs

The UI may present them in adjacent surfaces, but it must not collapse them into one implied backend model.

If one backend process exposes both surfaces, it SHOULD do so through explicit mounted route boundaries so the frontend can target Workspace and Attractor intentionally.

---

## 4. Presentation Surfaces

### 4.1 Workspace Surfaces

Workspace-oriented surfaces include:
- project selection
- conversation list
- project chat
- inline spec proposal cards
- inline flow-run request cards
- inline execution cards
- proposal and card review controls

### 4.2 Attractor Surfaces

Attractor-oriented surfaces include:
- flow editing and validation
- run history
- run event timeline
- checkpoint view
- context view
- graph view
- artifact inspection
- human-question answering

When the user reaches run inspection from a project workspace surface, the frontend SHOULD preserve project framing by default rather than dropping the user into an unscoped global run context.

### 4.3 Bridge Surfaces

Bridge surfaces show how workspace artifacts connect to Attractor execution.

Examples:
- an approved execution card references the flow it will launch
- an approved flow-run request references the Attractor run it launched
- a project surface can show which flows are associated with the project or bound to triggers
- a launched execution card shows the resulting Attractor run id
- a proposal or execution card may link to downstream run outcomes

These are provenance views, not ownership transfers.

---

## 5. Frontend State Model

### 5.1 Durable Backend State

The frontend must treat backend-provided state as authoritative for:
- project and conversation identity
- artifact status
- run identity and run status
- provenance links

### 5.2 Ephemeral UI State

The frontend may keep local ephemeral state for:
- selected tab
- expanded/collapsed cards
- active inspector panel
- actively inspected run
- draft message text
- local filter/sort controls

### 5.3 Prohibition on Hidden Authority

The frontend MUST NOT persist local state that can override backend truth for:
- which proposals exist
- whether an artifact was approved
- which runs belong to a project
- which run an artifact launched
- which flow a workflow trigger resolves to

When local UI state becomes invalid relative to backend state, the backend wins.

The frontend SHOULD rely on workspace-managed trigger bindings for workflow launches and only send an explicit flow override when the user is intentionally overriding the configured binding.

---

## 6. Conversation Rendering Model

The frontend renders project chat from the workspace conversation contract rather than from raw model protocol messages.

The conversation surface should show:
- user turns
- assistant turns
- thinking blocks
- tool activity
- inline workspace artifacts such as spec proposals, flow-run requests, and execution cards

The frontend should not require knowledge of whether an artifact was created through:
- Codex app-server
- MCP
- another model backend

It only needs the normalized workspace conversation event contract.

---

## 7. Review and Approval UX

### 7.1 Human Review

Spec edit proposal approval, flow-run request approval, and execution-card approval are explicit human actions.

The frontend MUST present these as review controls, not as automatic assistant continuations.

### 7.2 Visible Provenance

When a human approval causes downstream execution, the frontend SHOULD display:
- what artifact was approved
- what flow was launched
- what Attractor run id was created
- an affordance to open that run in the project-scoped execution or run-inspection UI

For `flow_run_request` cards, the frontend does not need to mirror ongoing Attractor run status inline after launch. The inline card may remain a provenance/approval record once `run_id` exists.

### 7.3 No Implicit Approval

The frontend MUST NOT imply that an assistant-created artifact is already accepted unless the backend records that approval explicitly.

---

## 8. Run Inspection UX

The run inspector is a frontend for Attractor runs, not for workspace artifacts.

The inspector must render:
- run summary
- event timeline
- checkpoint
- context
- graph
- artifacts
- pending questions when present

Workspace state may link the user into a specific run, but the inspector still operates on the Attractor run contract.

When entered from a project conversation or another project-scoped workspace surface, the run inspector SHOULD preserve that project scope in surrounding navigation and run-list context where practical.

---

## 9. Integration Rules

### 9.1 Engine Independence

The frontend SHOULD be able to work with any Attractor implementation that satisfies [attractor-spec.md](/Users/chris/tinker/sparkspawn/specs/attractor-spec.md).

The canonical engine-facing HTTP surface is mounted under:
- `/attractor/...`

### 9.2 Workspace Independence

The frontend SHOULD consume Spark Spawn workspace behavior through explicit workspace APIs rather than direct filesystem assumptions or browser-owned durable state.

The canonical workspace-facing HTTP surface is mounted under:
- `/workspace/...`

The frontend MUST treat those mounted surfaces as canonical and MUST NOT depend on duplicate root-path aliases.

### 9.3 Model Runtime Independence

The frontend SHOULD NOT depend on one specific model backend such as Codex app-server.

If the workspace service changes its internal model/tool runtime, the frontend contract should remain stable as long as:
- the workspace conversation APIs remain stable
- Attractor APIs remain stable

---

## 10. Recommended Direction

The preferred long-term direction is:

1. Keep the frontend cleanly split between workspace-facing and Attractor-facing data sources.
2. Keep workspace artifacts as conversational touch points into execution.
3. Keep Attractor runs and run inspection on the Attractor contract.
4. Keep client-local state limited to view behavior, not durable business truth.

This document is the source of truth when frontend responsibilities, client-side storage, or presentation boundaries are ambiguous.
