# Spark Spawn Workspace Specification

This document is the canonical workspace-layer specification for Spark Spawn.

It defines Spark Spawn-owned domain behavior above [attractor-spec.md](/Users/chris/tinker/sparkspawn/specs/attractor-spec.md) and below the frontend client.

It consolidates and supersedes the split workspace documents:
- `conversation-paradigm.md`
- `conversation-event-contract.md`
- `conversation-state-model.md`
- `storage-boundaries.md`

## 1. Purpose

Spark Spawn is not the Attractor engine.

Spark Spawn is the project-scoped workspace and orchestration layer that:
- manages projects
- manages conversations
- creates and stores review artifacts
- records human approval decisions
- launches Attractor workflows when those decisions require execution
- maintains provenance between conversation artifacts and Attractor runs

Spark Spawn-owned workflow state lives under `SPARKSPAWN_HOME`. Project-owned source content lives in the project repository.

## 2. Scope and Non-Goals

Workspace owns:
- project registration and active project scope
- project conversations
- spec edit proposals
- flow run requests
- execution cards
- human review decisions for workspace artifacts
- provenance links from workspace artifacts to Attractor flows and runs
- project-to-flow associations
- trigger-to-flow bindings

Workspace does not own:
- the Attractor flow definition itself
- pipeline submission semantics
- run lifecycle semantics
- checkpoint, context, or artifact semantics
- project canonical source documents, code, or tests

The frontend is a client of the workspace surface. It is not the durable owner of workspace truth.

## 3. Relationship to Attractor

Attractor remains authoritative for:
- flow definitions and validation
- pipeline submission and execution
- run lifecycle
- checkpoint, context, artifacts, and event streams
- human-gate questions and answers defined by the runtime contract

Spark Spawn remains authoritative for:
- workspace artifacts and review state
- workspace-scoped project identity
- provenance between artifacts and runs
- launch selection and trigger bindings

When both layers run inside one process, they must still remain distinct service surfaces. A modular monolith is acceptable; a mixed undifferentiated API surface is not.

## 4. Core Principles

1. Attractor is the engine of record for execution.
2. Spark Spawn is the workspace of record for conversations and review artifacts.
3. Human approvals are explicit and durable.
4. Provenance is explicit, not inferred from UI state.
5. One user send creates one durable user turn and one active assistant turn.
6. `state.json` is the durable render authority; `raw-log.jsonl` is the wire/debug authority.
7. Spark Spawn workflow state stays in `SPARKSPAWN_HOME`.
8. Project source artifacts remain authoritative in the project repository.

## 5. Workspace Domain Model

### 5.1 Project

A project:
- is bound to one local directory
- resolves to one Git repository context
- scopes conversations, workspace artifacts, and relevant Attractor runs

### 5.2 Conversation

A conversation:
- belongs to exactly one project
- contains ordered user and assistant turns
- may contain inline workspace artifacts
- is the primary collaborative surface for review and planning actions

### 5.3 Spec Edit Proposal

A spec edit proposal is a Spark Spawn review artifact. It contains:
- stable proposal id
- summary
- rationale if present
- one or more concrete changes
- review status
- provenance to the originating conversation turn

### 5.4 Execution Card

An execution card is a planning artifact derived from an approved spec edit proposal. It contains:
- stable execution-card id
- title, summary, objective
- structured work items
- review status
- provenance to the source proposal
- optional references to the intended flow

### 5.5 Flow Run Request

A flow run request is a review artifact for agent-requested Attractor execution. It contains:
- stable request id
- flow name
- summary
- optional goal text
- optional model override
- review status
- provenance to the originating conversation turn
- optional reference to the launched Attractor run

### 5.6 Provenance Record

Spark Spawn must retain explicit durable links:
- conversation -> proposal
- conversation -> flow run request
- proposal -> execution card
- flow run request -> flow reference
- flow run request -> launched Attractor run
- execution card -> flow reference
- execution card -> launched Attractor run
- Attractor run -> produced artifacts

### 5.7 Flow Association

Flow documents remain Attractor resources.

Spark Spawn owns:
- which flows are associated with a project
- which flow a workspace artifact references
- which triggers are bound to which flows for a project

## 6. Project Identity and Registration

A registered project has two co-equal identity attributes:
- canonical absolute project path
- internal Spark Spawn `project-id`

The canonical path is user-facing. The internal `project-id` is storage-facing.

`project.toml` is the source of truth for mapping those identities.

## 7. Conversation Model

Project chat is not a generic message stream. It is the collaborative surface for:
- project-scoped discussion
- specification review
- execution planning handoff
- durable work history

Conversation rules:
- one user send creates one conversational turn
- turns are durable and identity-bearing
- streaming updates mutate the active assistant turn in place
- retries and failures are explicit, not hidden
- tool activity is part of the same timeline when relevant
- project scope and thread scope are both first-class and must not be conflated

## 8. Conversation Turn and Segment Semantics

Turns are the durable conversational containers.

Segments are the primary render units. They exist so Spark Spawn can preserve:
- stable incremental streaming
- reasoning summaries
- tool-call activity
- artifact placement
- restart-safe rendering

Segment identity should preserve upstream item identity where available. Artifact cards are inline anchors linked to durable workspace records rather than inferred from presentation state.

## 9. Conversation Event Normalization Contract

Spark Spawn conversation handling preserves four distinct layers:
- raw app-server notifications
- workspace normalization
- durable `state.json`
- rendered chat cards

The frontend consumes normalized workspace events and snapshots, not raw protocol messages.

The canonical live workspace events are:
- `turn_upsert`
- `segment_upsert`
- `conversation_snapshot`

Rules:
- `segment_upsert` is the granular live content update contract
- `turn_upsert` carries turn lifecycle state, not a substitute content stream
- the frontend must render from normalized segments, not reconstruct from raw protocol notifications
- fallback turn-completion text must not create synthetic duplicate assistant cards

## 10. Durable Conversation State Model

Spark Spawn persists two different conversation authorities:
- `raw-log.jsonl`: exact protocol transcript for debug and reasoning about upstream behavior
- `state.json`: compact, durable, restart-safe render state

`state.json` must contain enough information to restore:
- conversation metadata
- turns
- segments
- inline workspace artifacts
- execution workflow state
- event log

`state.json` is a materialized view, not a raw transcript.

Historical shapes that lack the modern schema or segment model may be rejected rather than heuristically reconstructed.

## 11. Review Artifacts

Spec edit proposals, flow run requests, and execution cards are durable workspace objects, not just render fragments.

Each artifact class must preserve:
- stable identity
- status
- provenance
- inline placement in the conversation timeline

For `flow_run_request` specifically:
- creating the artifact does not start the run
- approval is separate from creation
- once launched, the inline card remains a provenance and approval record rather than a full live mirror of Attractor state

## 12. Approval and Rejection Workflows

The assistant may create candidate artifacts.

The assistant must not auto-approve:
- spec edit proposals
- execution cards
- flow run requests

Those approvals are reserved for human users even when the assistant believes the artifact is correct.

Approval and rejection are durable state transitions, not implied UI gestures.

## 13. Execution Planning and Dispatch

Spark Spawn conversations interact with Attractor through explicit artifacts rather than by treating Attractor as a chat system.

The main touch points are:
- create spec edit proposal
- review spec edit proposal
- create flow run request
- review flow run request
- generate execution card from approved proposal
- review execution card
- launch Attractor workflow from approved execution card
- inspect resulting run and outcomes

### 13.1 Agent-Created Workspace Artifacts

Spark Spawn may expose a narrow first-party CLI surface for agent-created workspace artifacts:

```text
sparkspawn-workspace spec-proposal --conversation <adjective-noun> --json <payload.json>
sparkspawn-workspace flow-run --conversation <adjective-noun> --flow <flow_name> --summary <text> [--goal-file <path>|--goal -] [--model <model>]
sparkspawn-workspace list-flows [--text]
sparkspawn-workspace describe-flow --flow <flow_name> [--text]
sparkspawn-workspace get-flow --flow <flow_name> [--text]
```

The artifact-creation commands create pending review artifacts. They must not approve, reject, or launch downstream execution by themselves.

The flow-discovery commands are read-only. They expose only workspace-requestable flows to the agent surface by default, while the workspace retains separate global policy for flows that are trigger-only or disabled.

### 13.2 Automatic Placement

The agent must not supply a workspace `turn_id`.

Instead, the workspace service must:
1. resolve the conversation handle
2. resolve the appropriate assistant turn automatically
3. attach the artifact to that turn
4. create the corresponding inline segment and workspace event

### 13.3 Conversation Handles

Each conversation has:
- an internal storage id
- a stable external `adjective-noun` handle

The external handle:
- is immutable for the life of the conversation
- is unique across the workspace
- is surfaced in summaries and snapshots
- is the conversation identifier used by Spark Spawn agent tooling

### 13.4 Prompt Framing

Prompt assembly is split into:
- a fixed non-user-editable system frame
- a user-editable guidance template

The fixed system frame owns:
- role and scope
- invariant workflow boundaries
- tool-use boundaries
- runtime-bound values Spark Spawn must always inject

The editable template owns:
- project-specific guidance
- tone and emphasis
- explicitly allowed runtime variables

## 14. Provenance Model

Spark Spawn must retain queryable provenance without reconstructing it from UI history.

That provenance model must support:
- opening a run from an approved artifact
- inspecting downstream run outcomes from a conversation
- tracing review decisions back to their source turn and project context

## 15. Project-to-Flow Associations and Trigger Bindings

Trigger bindings are workspace resources, not Attractor flow resources.

Each binding is:
- project-scoped
- keyed by a stable trigger name
- resolved to a flow name owned by Attractor

Initial trigger set:
- `spec_edit_approved`
- `execution_card_approved`
- `execution_card_rejected`
- `execution_card_revision_requested`

Launch flow resolution order:
1. explicit request override
2. project trigger binding
3. built-in workspace fallback

## 16. Storage Boundaries

Core rule:
- Spark Spawn owns workflow state.
- The project owns canonical content and outputs.

Spark Spawn-owned durable data includes:
- conversation threads
- normalized conversation state and raw transport logs
- workflow logs
- spec proposals
- execution cards
- flow run requests
- review decisions
- run metadata linkage
- project registry metadata
- trigger bindings and provenance metadata

Project-owned data includes:
- specifications
- user stories
- intent/design documents
- code
- tests
- generated outputs intended as part of the project record

Spark Spawn workflow state must not be written into the target project repository.

## 17. On-Disk Layout

Canonical `SPARKSPAWN_HOME` layout:

```text
SPARKSPAWN_HOME/
  config/
  runtime/
  logs/
  workspace/
    projects/
      <project-id>/
        project.toml
        conversations/
        workflow/
        proposals/
        execution-cards/
  attractor/
    runs/
      <project-id>/
        <run-id>/
```

Conversation-specific durable files include:
- `state.json`
- `raw-log.jsonl`

Repo-owned material should remain organized by content, not by Spark Spawn as a tool.

## 18. Workspace HTTP Surface

The canonical workspace HTTP surface is mounted under:
- `/workspace/...`

Workspace API categories should cover:
- projects
- conversations
- conversation events and snapshots
- review artifacts
- artifact review actions
- trigger bindings
- provenance lookups

Workspace events and Attractor events should remain separate at the service boundary even if the frontend presents them together.

## 19. Model and Tool Integration

Spark Spawn may use model/tool infrastructure to create workspace artifacts. Examples include:
- Codex app-server
- MCP tools
- local-model adapters

These are implementation details of the workspace layer.

The stable contract is:
1. an artifact creation request is made
2. Spark Spawn validates and persists the artifact
3. Spark Spawn emits the corresponding workspace conversation event

The frontend must not depend on one specific model runtime.

## 20. Invariants and Compatibility Constraints

Spark Spawn must maintain these invariants:
- workspace workflow state is authoritative in `SPARKSPAWN_HOME`
- project source artifacts are authoritative in the project repository
- opening one project must not write Spark Spawn workflow state into another repository
- a conversation belongs to exactly one project
- no synthetic duplicate assistant cards may be created from fallback text
- turn and segment identity must remain stable across restart
- approvals and provenance must remain explicit and durable
- unsupported historical shapes may be rejected rather than guessed
