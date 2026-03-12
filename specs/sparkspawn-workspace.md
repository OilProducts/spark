# Spark Spawn Workspace Specification

This document defines the Spark Spawn workspace layer that sits above a compliant Attractor implementation.

It is intentionally separate from:
- [attractor-spec.md](/Users/chris/tinker/sparkspawn/specs/attractor-spec.md), which defines the workflow engine
- [ui-spec.md](/Users/chris/tinker/sparkspawn/specs/ui-spec.md), which currently mixes presentation and workspace concerns

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
- execution cards
- human review decisions for Spark Spawn artifacts
- provenance links from workspace artifacts to Attractor flows and runs

These concepts are outside the scope of the base Attractor spec.

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

### 4.5 Provenance Record

Spark Spawn MUST retain explicit provenance links:
- conversation -> proposal
- proposal -> execution card
- execution card -> flow reference
- execution card -> launched Attractor run
- Attractor run -> produced artifacts

These links MUST be durable and queryable without reconstructing them from rendered UI history.

---

## 5. Workflow Model

### 5.1 Conversation Touch Points

Spark Spawn conversations interact with Attractor through explicit artifacts rather than by treating Attractor as a chat system.

The main conversational touch points are:
- create spec edit proposal
- review spec edit proposal
- generate execution card from approved proposal
- review execution card
- launch Attractor workflow from approved execution card
- inspect resulting run and outcomes

### 5.2 Approval Rules

The assistant MAY create candidate artifacts.

The assistant MUST NOT auto-approve:
- spec edit proposals
- execution cards

Those approvals are reserved for human users, even when the assistant believes the artifact is correct.

### 5.3 Run Launch

When a workspace artifact launches execution:
- Spark Spawn chooses or references a concrete Attractor flow
- Spark Spawn calls the Attractor execution surface
- Spark Spawn stores the resulting run id as provenance on the relevant artifact
- Spark Spawn does not reinterpret the Attractor run model into a separate execution model

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
- spec edit proposals
- execution cards
- artifact review decisions
- provenance metadata linking workspace artifacts to Attractor runs and flows

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

This document is the source of truth when ownership, provenance, approval boundaries, or workspace-vs-engine responsibilities are ambiguous.
