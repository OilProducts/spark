# Spark Spawn Workspace Specification

This document defines the Spark Spawn workspace layer that sits above a compliant Attractor implementation.

It is intentionally separate from:
- [attractor-spec.md](/Users/chris/tinker/sparkspawn/specs/attractor-spec.md), which defines the workflow engine
- [sparkspawn-frontend.md](/Users/chris/tinker/sparkspawn/specs/sparkspawn-frontend.md), which defines the frontend-client behavior above the workspace and Attractor contracts

This document is the source of truth for the domain model and service behavior on the Spark Spawn side of Attractor.

---

## 1. Purpose

Spark Spawn is not the Attractor engine.

Spark Spawn is the project-scoped workspace and orchestration layer that:
- manages projects
- manages conversations
- creates and stores review artifacts
- records human approval decisions
- launches Attractor workflows when those decisions require execution
- maintains provenance between conversation artifacts and Attractor runs

Attractor remains a standalone workflow runtime.

Spark Spawn remains a standalone workspace product.

In the current deployment model, Spark Spawn Workspace and Attractor MAY run inside one backend process, but they MUST remain distinct service surfaces. A modular monolith is acceptable; a mixed undifferentiated API surface is not.

The canonical Workspace HTTP surface is mounted under:
- `/workspace/...`

Workspace APIs MUST NOT depend on duplicate root-path aliases.

---

## 2. Layer Boundaries

### 2.1 Attractor

Attractor owns:
- flow definitions and validation
- pipeline submission and execution
- run lifecycle
- checkpoint, context, artifacts, and event streams
- human-gate questions and answers defined by the Attractor runtime contract

Spark Spawn MUST NOT redefine or shadow these concepts.

### 2.2 Spark Spawn Workspace

Spark Spawn owns:
- project registration and active project scope
- project conversations
- spec edit proposals
- flow run requests
- execution cards
- human review decisions for Spark Spawn artifacts
- provenance links from workspace artifacts to Attractor flows and runs
- project-to-flow associations
- trigger-to-flow bindings

These concepts are outside the scope of the base Attractor spec.

The canonical Workspace service surface SHOULD be exposed separately from the Attractor surface, even when both are hosted by one FastAPI/ASGI deployment.

### 2.3 Frontend

The frontend is a client of:
- the Spark Spawn workspace service for project/conversation/review data
- a compliant Attractor API for flows, runs, events, checkpoint, context, and graph inspection

The frontend is not the durable owner of workspace or Attractor domain state.

---

## 3. Core Principles

1. Attractor is the engine of record for execution.
2. Spark Spawn is the workspace of record for conversations and review artifacts.
3. Human approvals are explicit and durable.
4. Provenance is explicit, not inferred from UI state.
5. UI-local state is ephemeral unless intentionally persisted by the workspace or Attractor service.
6. Spark Spawn may use Codex app-server, MCP, or other model integrations internally, but those are implementation details rather than frontend contracts.

---

## 4. Domain Model

### 4.1 Project

A project:
- is bound to one local directory
- resolves to one Git repository context
- scopes conversations, workspace artifacts, and relevant Attractor runs

Project identity is a Spark Spawn concern.

### 4.2 Conversation

A conversation:
- belongs to exactly one project
- contains user turns and assistant turns
- may contain inline workspace artifacts
- is the conversational touch point for workspace-driven actions

Conversation storage and eventing are Spark Spawn concerns, not Attractor concerns.

### 4.3 Spec Edit Proposal

A spec edit proposal is a Spark Spawn review artifact.

It contains:
- stable proposal id
- summary
- rationale if present
- one or more concrete changes
- review status (`pending`, `approved`, `rejected`, `revision_requested`)
- provenance to the originating conversation turn

Approving a spec edit proposal is a human decision point.

### 4.4 Execution Card

An execution card is a Spark Spawn planning artifact derived from an approved spec edit proposal.

It contains:
- stable execution card id
- title, summary, objective
- structured work items
- review status (`pending`, `approved`, `rejected`, `revision_requested`)
- provenance to the source proposal
- optional references to the flow intended for execution

Approving an execution card is a human decision point.

### 4.5 Flow Run Request

A flow run request is a Spark Spawn review artifact for agent-requested Attractor execution.

It contains:
- stable flow run request id
- flow name
- summary
- optional goal/prompt
- optional model override
- review status (`pending`, `approved`, `rejected`, `launch_failed`, `launched`)
- provenance to the originating conversation turn
- optional reference to the launched Attractor run

Creating a flow run request does not start execution.

Approving a flow run request is a human decision point.

### 4.6 Provenance Record

Spark Spawn MUST retain explicit provenance links:
- conversation -> proposal
- conversation -> flow run request
- proposal -> execution card
- flow run request -> flow reference
- flow run request -> launched Attractor run
- execution card -> flow reference
- execution card -> launched Attractor run
- Attractor run -> produced artifacts

These links MUST be durable and queryable without reconstructing them from rendered UI history.

### 4.7 Flow Association

Flow documents remain Attractor resources.

Spark Spawn does not own the flow definition itself, even when Spark Spawn created or edited that flow through Attractor APIs.

Spark Spawn does own:
- which flows are associated with a project
- which flow a workspace artifact references
- which triggers are bound to which flows for a project

This means Spark Spawn stores flow usage metadata, while Attractor remains the source of truth for the flow document and execution semantics.

---

## 5. Workflow Model

### 5.1 Conversation Touch Points

Spark Spawn conversations interact with Attractor through explicit artifacts rather than by treating Attractor as a chat system.

The main conversational touch points are:
- create spec edit proposal
- review spec edit proposal
- create flow run request
- review flow run request
- generate execution card from approved proposal
- review execution card
- launch Attractor workflow from approved execution card
- inspect resulting run and outcomes

### 5.1.1 Agent-Created Workspace Artifacts

Spark Spawn MAY expose a narrowly-scoped first-party CLI surface for agent-created workspace artifacts.

This CLI surface exists to provide a stable, Spark Spawn-owned mutation boundary for agent workflows without relying on experimental runtime-specific tool transports.

The initial supported mutating commands are:

```text
sparkspawn-workspace spec-proposal --conversation <adjective-noun> --json <payload.json>
sparkspawn-workspace flow-run --conversation <adjective-noun> --flow <flow_name> --summary <text> [--goal-file <path>|--goal -] [--model <model>]
```

These commands create pending review artifacts within an existing project conversation.

They MUST NOT:
- approve a proposal
- reject a proposal
- apply the proposal to the repository
- approve or reject a flow run request
- launch downstream execution by itself

Those actions remain separate human or workflow decisions.

Rationale:
- the assistant needs one reliable way to create workspace artifacts
- proposal creation and flow-run requests are workspace concerns, not raw Attractor concerns
- the command surface should stay minimal until additional agent-facing actions are clearly justified

### 5.1.2 `sparkspawn-workspace spec-proposal`

The command input contract is JSON-based.

The command line MUST include:
- `--conversation <handle>`

The payload MUST include:
- `summary`
- `changes`

The payload MAY include:
- `rationale`

Example shape:

```json
{
  "summary": "Clarify approval before execution planning.",
  "changes": [
    {
      "path": "spec/home-chat.md#review-flow",
      "before": "Planning begins immediately.",
      "after": "Planning begins only after explicit spec approval."
    }
  ],
  "rationale": "Grounded in the current review flow and approval boundary."
}
```

Field intent:
- `--conversation`
  - identifies the conversation where the artifact should appear
  - uses the stable external `adjective-noun` handle, for example `amber-otter`
- `summary`
  - concise description of the concrete proposed edit
- `changes`
  - minimal grounded `before` / `after` edits
  - this is not a place for speculative feature ideation or broad whole-file rewrites by default
- `rationale`
  - optional explanation of why the proposed change is justified

The payload MUST NOT include:
- `conversation_id`
- `project_path`
- `turn_id`

The command help text MUST describe both:
- the payload shape
- the semantic intent of each field

The command help SHOULD also instruct the agent to:
- prefer `--json -` via stdin
- use a temporary file outside the repository only when stdin is not practical
- avoid leaving proposal payload files in the project working tree

The CLI help is the implementation-facing source of truth for command usage. Prompts may instruct an agent to use the command, but prompts MUST NOT be the only specification of the payload contract.

### 5.1.3 `sparkspawn-workspace flow-run`

The command line MUST include:
- `--conversation <handle>`
- `--flow <flow_name>`
- `--summary <text>`

The command SHOULD include:
- `--goal-file <path>` or `--goal -` when the selected flow requires launch context

The command MAY include:
- `--model <model>`

The command creates a pending `flow_run_request` artifact. It does not start the Attractor run immediately.

The flow-run request artifact itself MUST persist:
- a concise human-readable summary of why the run is being requested
- the selected flow name
- optional goal text
- optional model override
- review status
- run id only after approval and successful launch

If the user approves the request, Spark Spawn launches the Attractor flow and updates the artifact with the resulting run id. If the launch fails, Spark Spawn records `launch_failed` and the error.

The workspace service SHOULD expose:

```text
POST /workspace/api/conversations/by-handle/{handle}/flow-run-requests
POST /workspace/api/conversations/{conversation_id}/flow-run-requests/{request_id}/review
```

The create request MUST include:
- `flow_name`
- `summary`

The create request MAY include:
- `goal`
- `model`

The review request MUST include:
- `project_path`
- `disposition`

The review request MAY include:
- `message`
- `flow_name` when the reviewer intentionally overrides the originally requested flow
- `model`

Review dispositions are:
- `approved`
- `rejected`

On approval, Spark Spawn launches the Attractor run, records the resulting `run_id`, and updates the linked `flow_run_request` artifact. On rejection, Spark Spawn records the rejection without launching anything.

### 5.1.4 Automatic Placement

The agent MUST NOT supply a workspace `turn_id`.

Instead, the workspace service MUST:
1. resolve the referenced conversation handle
2. resolve the appropriate assistant turn automatically
3. attach the artifact to that turn
4. create the corresponding inline conversation segment and workspace event

If no valid assistant turn exists for placement, the command MUST fail with a clear validation error.

Rationale:
- `turn_id` is internal placement state
- requiring the agent to supply it creates unnecessary coupling to workspace internals
- automatic placement lets Spark Spawn preserve a stable artifact-rendering model even if turn-level internals evolve

### 5.1.5 Conversation Handles

Each conversation has:
- an internal storage id
- a stable external conversation handle

The external handle:
- MUST use lowercase `adjective-noun` form
- MUST be generated by the workspace service when the conversation is created
- MUST be immutable for the life of the conversation
- MUST be unique across the workspace
- MUST be surfaced in conversation summaries and conversation snapshots
- MUST be the conversation identifier used by Spark Spawn agent tooling

The external handle is not the user-editable title.

Workspace ordering and sorting SHOULD use `created_at`, not the handle.

### 5.1.6 Prompt Framing

Prompt assembly for the chat agent is split into:
- a fixed non-user-editable system frame
- a user-editable guidance template

The fixed system frame owns:
- role and scope
- invariant workflow boundaries
- tool-use boundaries
- runtime-bound values that Spark Spawn must always inject

The user-editable guidance template owns:
- project-specific guidance
- tone and emphasis
- optional use of explicitly allowed runtime variables

The allowed runtime variables for the chat prompt are:
- `conversation_handle`
- `project_path`
- `recent_conversation`
- `latest_user_message`

The allowed runtime variables for the execution-planning prompt are:
- `project_path`
- `approved_spec_edit_proposal`
- `recent_conversation`
- `review_feedback`

Rationale:
- Spark Spawn needs a small invariant frame so tooling and workflow rules remain stable
- users still need meaningful control over the editable guidance portion
- the variable set must remain explicit so prompt templating does not become an ad hoc data transport channel

### 5.2 Approval Rules

The assistant MAY create candidate artifacts.

The assistant MUST NOT auto-approve:
- spec edit proposals
- execution cards
- flow run requests

Those approvals are reserved for human users, even when the assistant believes the artifact is correct.

### 5.3 Run Launch

When a workspace artifact launches execution:
- Spark Spawn chooses or references a concrete Attractor flow
- Spark Spawn calls the Attractor execution surface
- Spark Spawn stores the resulting run id as provenance on the relevant artifact
- Spark Spawn does not reinterpret the Attractor run model into a separate execution model

For agent-requested flow execution, the relevant artifact is the approved `flow_run_request`, not an implicit chat action.

---

## 6. Data Ownership and Storage

### 6.1 Attractor-Owned Durable Data

Attractor owns:
- flow definitions
- run records
- checkpoint data
- context state
- run artifacts
- pipeline event streams

### 6.2 Spark Spawn-Owned Durable Data

Spark Spawn owns:
- project registry
- conversations
- conversation events
- conversation handle index
- spec edit proposals
- execution cards
- flow run requests
- artifact review decisions
- project-to-flow associations
- trigger-to-flow bindings
- provenance metadata linking workspace artifacts to Attractor runs and flows

Workspace storage lives under:
- `SPARKSPAWN_HOME/workspace/projects/...`
- `SPARKSPAWN_HOME/workspace/conversation-handles.json`

The conversation-handle index:
- is workspace-global
- stores active conversations only
- supports handle to conversation-id lookup
- supports conversation-id to handle lookup

Delete means delete:
- deleting a conversation removes its conversation files
- deleting a conversation removes its handle mapping
- Spark Spawn does not preserve handle tombstones

### 6.4 Trigger Binding Model

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

The workspace service MUST resolve launch flow selection in this order:
1. explicit request override
2. project trigger binding
3. built-in workspace fallback

The workspace service MUST persist only the trigger-to-flow reference. It MUST NOT duplicate or take ownership of the underlying flow document.

### 6.3 Frontend-Owned State

The frontend may own only ephemeral view state such as:
- current tab
- expanded/collapsed panels
- currently inspected run
- draft input text

The frontend MUST NOT be the durable source of truth for:
- project identity
- conversation identity
- artifact review state
- provenance
- Attractor run state

---

## 7. Service API Expectations

The workspace service API SHOULD expose resource-oriented operations for:
- projects
- conversations
- conversation events
- spec edit proposals
- execution cards
- flow run requests
- artifact review actions
- provenance lookups

The Attractor API remains separately responsible for:
- flows
- pipeline submission
- run status
- run events
- questions
- checkpoint/context/graph/artifact inspection

The frontend should be able to swap Attractor implementations as long as they satisfy the Attractor contract.

---

## 8. Eventing Model

Workspace conversation eventing is a Spark Spawn concern.

Attractor run eventing is an Attractor concern.

These streams SHOULD remain separate at the service boundary, even if the frontend presents them together in one product.

### 8.1 Workspace Events

Examples:
- turn created
- assistant delta
- reasoning summary
- tool call lifecycle
- spec edit proposal created
- flow run request created
- flow run request approved
- flow run request rejected
- flow run request launch failed
- execution card created
- review decision recorded

### 8.2 Attractor Events

Examples:
- pipeline started
- stage completed
- checkpoint saved
- human question opened
- pipeline completed or failed

Spark Spawn MAY mirror selected Attractor outcomes into workspace provenance or status surfaces, but it MUST NOT replace Attractor’s own run event stream.

---

## 9. Model and Tool Integration

Spark Spawn may use model/tool infrastructure to create workspace artifacts.

Examples:
- Codex app-server
- MCP tools
- local-model adapters

These are implementation details of the workspace layer.

The stable contract is:
- an artifact creation request is made
- Spark Spawn validates and persists the artifact
- Spark Spawn emits the corresponding workspace conversation event

The frontend should not depend on one specific model runtime.

---

## 10. Recommended Direction

The preferred long-term architecture is:

1. Attractor remains a clean engine implementation of [attractor-spec.md](/Users/chris/tinker/sparkspawn/specs/attractor-spec.md).
2. Spark Spawn workspace remains a separate orchestration/product layer.
3. The frontend talks to both layers through explicit contracts rather than one blended API.
4. Conversation artifacts are the primary bridge from collaborative planning into Attractor execution.
5. Agent-requested flow launches must become explicit review artifacts before they become Attractor runs.

This document is the source of truth when ownership, provenance, approval boundaries, or workspace-vs-engine responsibilities are ambiguous.
