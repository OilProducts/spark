# Spark Workspace Specification

This document is the canonical workspace-layer specification for Spark.

It defines Spark-owned domain behavior above [attractor-spec.md](attractor-spec.md) and below the frontend client.

It consolidates and supersedes the split workspace documents:
- `conversation-paradigm.md`
- `conversation-event-contract.md`
- `conversation-state-model.md`
- `storage-boundaries.md`

## 1. Purpose

Spark is not the Attractor engine.

Spark is the workspace and orchestration layer over registered projects. It:
- manages projects
- exposes shared flow assets that live under `SPARK_HOME`
- manages conversations
- creates and stores flow-review artifacts
- records human approval decisions
- launches Attractor workflows when those decisions require execution
- maintains provenance between conversation artifacts and Attractor runs

Spark-owned workflow state lives under `SPARK_HOME`. Project-owned source content lives in the project repository.

## 2. Scope and Non-Goals

Workspace owns:
- project registration and active project scope
- project conversations
- flow run requests
- flow launches recorded from the conversation surface
- human review decisions for workspace artifacts
- provenance links from workspace artifacts to Attractor flows and runs
- flow references for workspace artifacts
- workspace-owned trigger definitions and trigger runtime state

Workspace does not own:
- the Attractor flow definition itself
- pipeline submission semantics
- run lifecycle semantics
- checkpoint, context, or artifact semantics
- project canonical source documents, code, or tests

The frontend is a client of the workspace surface. It is not the durable owner of workspace truth.
Top-level frontend view-session state is client-ephemeral only; it is not part of workspace durable state.
That includes `homeSession`, `editorSession`, `executionSession`, `runsSession`, and `triggersSession`.
Hidden-tab live synchronization is a frontend session behavior, not a workspace durability requirement.

## 3. Relationship to Attractor

Attractor remains authoritative for:
- flow definitions and validation
- pipeline submission and execution
- run lifecycle and workflow outcome
- checkpoint, context, artifacts, and event streams
- human-gate questions and answers defined by the runtime contract

Spark remains authoritative for:
- workspace artifacts and review state
- workspace-scoped project identity
- provenance between artifacts and runs
- launch selection and workspace-owned triggers

When both layers run inside one process, they must still remain distinct service surfaces. A modular monolith is acceptable; a mixed undifferentiated API surface is not.

## 4. Core Principles

1. Attractor is the engine of record for execution.
2. Spark is the workspace of record for conversations and review artifacts.
3. Human approvals are explicit and durable.
4. Provenance is explicit, not inferred from UI state.
5. One user send creates one durable user turn and one active assistant turn.
6. `state.json` is the durable render authority; `raw-log.jsonl` is the wire/debug authority.
7. Spark workflow state stays in `SPARK_HOME`.
8. Project source artifacts remain authoritative in the project repository.

## 5. Workspace Domain Model

### 5.1 Project

A project:
- is bound to one local directory
- may expose Git repository context when available
- scopes conversations, review artifacts, and relevant Attractor runs

### 5.2 Conversation

A conversation:
- belongs to exactly one project
- contains ordered user and assistant turns
- may contain inline workspace artifacts
- is the primary collaborative surface for review and planning actions

### 5.3 Flow Run Request

A flow run request is a review artifact for agent-requested Attractor execution. It contains:
- stable request id
- flow name
- summary
- optional goal text
- optional model override
- review status
- provenance to the originating conversation turn
- optional reference to the launched Attractor run

### 5.4 Flow Launch

A flow launch is a conversation artifact for an immediately launched Attractor run. It contains:
- stable launch id
- flow name
- summary
- optional goal text
- optional model override
- launch status
- provenance to the originating conversation turn
- reference to the launched Attractor run when available

### 5.5 Provenance Record

Spark must retain explicit durable links:
- conversation -> flow run request
- conversation -> flow launch
- flow run request -> flow reference
- flow run request -> launched Attractor run
- Attractor run -> produced artifacts

### 5.6 Flow Association

Flow documents remain Attractor resources.
They are shared workspace/global authoring assets rather than per-project copies.

Spark owns:
- which flow a workspace artifact references
- which workspace triggers launch which flows
- whether a trigger action carries a project target

## 6. Project Identity and Registration

A registered project has two co-equal identity attributes:
- canonical absolute project path
- internal Spark `project-id`

The canonical path is user-facing. The internal `project-id` is storage-facing.

`project.toml` is the source of truth for mapping those identities.

## 7. Conversation Model

Project chat is not a generic message stream. It is the collaborative surface for:
- project-scoped discussion
- run-request review
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

Segments are the primary render units. They exist so Spark can preserve:
- stable incremental streaming
- reasoning summaries
- tool-call activity
- artifact placement
- restart-safe rendering

Segment identity should preserve upstream item identity where available. Artifact cards are inline anchors linked to durable workspace records rather than inferred from presentation state.
Conversation turns and segments continue to persist plain string content. Markdown interpretation, when used, is a frontend-only rendering rule applied to assistant message segments. That presentation boundary does not introduce a workspace HTTP, API, schema, or persisted-state change.

## 9. Conversation Event Normalization Contract

Spark conversation handling preserves four distinct layers:
- raw app-server notifications
- shared `TurnStreamEvent` normalization
- durable `state.json`
- rendered chat cards

The frontend consumes normalized workspace events and snapshots, not raw protocol messages.

`TurnStreamEvent` is the canonical live LLM and node-output stream model between raw backend protocols and durable workspace state. Both in-process agent sessions and Codex app-server turns normalize into this backend-neutral shape before Home chat materializes content into conversation segments.

`TurnStreamEvent` content uses `content_delta` or `content_completed` plus a `channel` of `assistant`, `reasoning`, or `plan`. Backend-specific correlation such as app turn IDs, item IDs, response IDs, summary indexes, and raw event kind belongs in `source` metadata rather than top-level workspace event fields.

`SessionEvent` is the generic agent runtime stream. Spark consumes `SessionEvent` from in-process agent sessions and converts it at the Spark boundary into `TurnStreamEvent`; Spark chat rendering does not adapt `unified_llm.StreamEvent` directly. Codex app-server JSON-RPC notifications are a peer runtime path and normalize directly into `TurnStreamEvent` with `source.backend = "codex_app_server"`.

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

Spark persists two different conversation authorities:
- `raw-log.jsonl`: exact protocol transcript for debug and reasoning about upstream behavior
- `state.json`: compact, durable, restart-safe render state

`state.json` must contain enough information to restore:
- conversation metadata
- turns
- segments
- inline workspace artifacts
- flow run request and flow launch records
- event log

It does not need to persist frontend-only view sessions such as:
- editor canvas selection, viewport, or draft DOT mode
- execution canvas selection, inspected run focus, or launch-form draft values
- Home thread-view state such as thread-list cache, draft text, expansion state, or layout state
- Runs inspection filters, artifact/context selection, or pending freeform gate-answer drafts
- Triggers selection, unsaved create/edit drafts, or revealed-secret UI state

`state.json` is a materialized view, not a raw transcript.

Durable workspace state is the render, provenance, and restart authority for the workspace surface. It is not required to be the primary continuity mechanism for the upstream model runtime on every turn.

Spark may preserve backend conversational continuity through a reused runtime thread or session that is tracked alongside durable workspace state.

Historical shapes that lack the modern schema or segment model may be rejected rather than heuristically reconstructed.

## 11. Review Artifacts

Flow run requests and flow launches are durable workspace objects, not just render fragments.

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
- flow run requests

Those approvals are reserved for human users even when the assistant believes the artifact is correct.

Approval and rejection are durable state transitions, not implied UI gestures.

## 13. Conversation Launch and Dispatch

Spark conversations interact with Attractor through explicit artifacts rather than by treating Attractor as a chat system.

The main touch points are:
- create flow run request
- review flow run request
- launch Attractor workflow directly from the conversation surface
- inspect resulting run and outcomes

### 13.1 Spark Agent Control Surface

Spark must expose a first-class Spark agent control surface.

Inside the assistant runtime, the canonical interface is the bare `spark` CLI. The runtime must make that surface available on `PATH` without requiring a user-global install.

Human source-checkout shell instructions are a separate operational concern. In a source checkout, human docs should use `uv run spark ...` rather than treating bare `spark ...` as the default shell entrypoint.

Stable read-only flow inspection and validation commands:

```text
spark flow list [--text]
spark flow describe --flow <flow_name> [--text]
spark flow get --flow <flow_name> [--text]
spark flow validate --file <path> --text
```

Stable mutating workspace and run commands:

```text
spark convo run-request --conversation <adjective-noun> --flow <flow_name> --summary <text> [--goal-file <path>|--goal -] [--model <model>]
spark run launch --flow <flow_name> --summary <text> [--conversation <adjective-noun>] [--project <path>]
```

`spark convo run-request` creates a pending review artifact. It must not approve or launch downstream execution by itself.

The read-only flow commands expose only workspace-requestable flows to the agent surface by default, while the workspace retains separate global policy for flows that are trigger-only or disabled.

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
- is the conversation identifier used by Spark agent tooling

### 13.4 Prompt Framing

Prompt assembly is split into:
- a fixed non-user-editable system frame
- a user-editable guidance template

The fixed system frame owns:
- role and scope
- invariant workflow boundaries
- tool-use boundaries
- runtime-bound values Spark must always inject
- the stable authoring and operations guidance references Spark requires for flow editing and run control

The editable template owns:
- project-specific guidance
- tone and emphasis
- explicitly allowed runtime variables

Project-chat continuity should normally come from the reused backend thread or session rather than from replaying prior workspace transcript text into each turn prompt.

Rules:
- the current user message is always injected as turn input through `latest_user_message`
- supported editable-template runtime variables are explicitly allowlisted by Spark rather than being open-ended
- `recent_conversation` is not a supported prompt variable
- local prompt templates that still reference `{{recent_conversation}}` may be rejected rather than silently rendered with stale semantics
- if a persisted backend thread cannot be resumed, Spark may start a fresh backend thread for the turn instead of reconstructing recent transcript history from durable workspace state

## 14. Provenance Model

Spark must retain queryable provenance without reconstructing it from UI history.

That provenance model must support:
- opening a run from an approved artifact
- inspecting downstream run outcomes from a conversation
- tracing review decisions back to their source turn and project context

## 15. Workspace Trigger Subsystem

Triggers are workspace resources, not project records and not Attractor flow resources.

Each trigger definition includes:
- stable trigger id
- name
- enabled/disabled state
- protected/non-protected status
- source configuration
- `launch_flow` action configuration

The action may optionally include a project target. Project context is therefore an execution input, not the owner of trigger identity.

Persisted definitions live under:
- `SPARK_HOME/config/triggers/<id>.toml`

Persisted runtime state lives under:
- `SPARK_HOME/workspace/trigger-state/<id>.json`

V1 source types are:
- `schedule`
- `poll`
- `webhook`
- `flow_event`

Webhook ingress is shared under:
- `POST /workspace/api/webhooks`

Webhook routing is resolved by per-trigger credentials rather than by per-trigger URL paths.

## 16. Storage Boundaries

Core rule:
- Spark owns workflow state.
- The project owns canonical content and outputs.

Spark-owned durable data includes:
- conversation threads
- normalized conversation state and raw transport logs
- workflow logs
- flow run requests
- review decisions
- run metadata linkage
- project registry metadata
- trigger definitions, trigger runtime state, and provenance metadata

Project-owned data includes:
- specifications
- user stories
- intent/design documents
- code
- tests
- generated outputs intended as part of the project record

Spark workflow state must not be written into the target project repository.

## 17. On-Disk Layout

Canonical `SPARK_HOME` layout:

```text
SPARK_HOME/
  config/
    triggers/
  runtime/
  logs/
  workspace/
    trigger-state/
    projects/
      <project-id>/
        project.toml
        conversations/
  attractor/
    runs/
      <project-id>/
        <run-id>/
```

Conversation-specific durable files include:
- `state.json`
- `raw-log.jsonl`

Repo-owned material should remain organized by content, not by Spark as a tool.

## 18. Workspace HTTP Surface

The canonical workspace HTTP surface is mounted under:
- `/workspace/...`

Workspace API categories should cover:
- projects
- conversations
- conversation events and snapshots
- review artifacts
- artifact review actions
- triggers and webhook ingress
- provenance lookups

Workspace events and Attractor events should remain separate at the service boundary even if the frontend presents them together.

## 19. Model and Tool Integration

Spark may use model/tool infrastructure to create workspace artifacts. Examples include:
- Codex app-server
- MCP tools
- local-model adapters

These are implementation details of the workspace layer.

The stable contract is:
1. an artifact creation request is made
2. Spark validates and persists the artifact
3. Spark emits the corresponding workspace conversation event

The frontend must not depend on one specific model runtime.

## 20. Invariants and Compatibility Constraints

Spark must maintain these invariants:
- workspace workflow state is authoritative in `SPARK_HOME`
- project source artifacts are authoritative in the project repository
- opening one project must not write Spark workflow state into another repository
- a conversation belongs to exactly one project
- no synthetic duplicate assistant cards may be created from fallback text
- turn and segment identity must remain stable across restart
- approvals and provenance must remain explicit and durable
- unsupported historical shapes may be rejected rather than guessed
