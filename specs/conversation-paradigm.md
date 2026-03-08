# Conversation Paradigm

This document defines the canonical conversation model for Spark Spawn project chat.

It exists to keep chat behavior coherent as the UI, backend, and workflow artifacts evolve.

Source of truth for where conversation data must live:
- `storage-boundaries.md`

## Purpose

Project chat is not a generic message stream. It is the primary collaborative surface for:
- project-scoped discussion
- specification review
- execution planning handoff
- durable work history

The system must preserve conversational continuity without sacrificing explicit review, ordering, or auditability.

## Core Principles

1. One user send creates one conversational turn.
2. A turn is durable and identity-bearing; it must not disappear or be silently replaced.
3. Streaming updates mutate the active assistant turn in place; they do not replace the whole conversation opportunistically.
4. Retries, failures, and workflow transitions are explicit events, not hidden implementation details.
5. Tool activity is part of the same conversational timeline when it is relevant to the assistant response.
6. Project scope and thread scope are both first-class and must never be conflated.

## Conversation Model

A conversation is a durable thread within a single active project.

Each conversation contains ordered turns.

Each turn may contain ordered events.

### Conversation

A conversation:
- belongs to exactly one project
- has a stable `conversation_id`
- persists across app restarts
- resumes the same underlying AI thread when possible

### Turn

A turn is the unit of user intent and assistant response.

Each user submission creates:
- one `user` turn
- one corresponding `assistant` turn

Assistant turns have explicit lifecycle state:
- `pending`
- `streaming`
- `complete`
- `failed`

### Turn Events

Turn events are append-only facts associated with a turn.

Examples:
- assistant text delta
- tool call started
- tool call output delta
- tool call completed
- tool call failed
- assistant completed
- assistant failed
- retry started

The UI may render turn events inline, but the canonical ordering is event order within a stable turn.

## Expected Send Behavior

When a user sends a message, the system should behave as follows:

1. Append the user turn immediately to the active thread.
2. Create one assistant turn in `pending` or `streaming` state.
3. Stream assistant text into that assistant turn as deltas arrive.
4. Append tool-call events in the same turn timeline as they occur.
5. Finalize the assistant turn in place when the response completes.
6. Leave already-rendered history intact.

The system must not:
- remove already-visible turns during normal streaming
- replay the same response as a second assistant turn without an explicit retry event
- reorder old turns because a later snapshot arrived

## Streaming Rules

Streaming is event-driven, not snapshot-driven.

The system may use snapshots for:
- initial thread load
- recovery after reconnect
- persistence checkpoints

But live behavior must follow these rules:
- the active assistant turn is updated incrementally
- tool-call events attach to that active assistant turn
- completion changes turn state; it does not append a duplicate final answer
- reconnects must reconcile with stable turn ids and event ids

## Retry and Failure Rules

Retries are valid, but they must be explicit.

If a turn must retry:
- the original assistant turn remains part of the record
- the retry is recorded as a turn event or explicit system event
- the user can see that a retry occurred

Failures must result in a visible terminal state:
- `failed`
- optional error detail
- optional retry affordance

The system must never silently discard an in-progress response and replay it as though nothing happened.

## Artifact Rules

Artifacts are conversation outputs, but they are not ordinary free-text replies.

### Spec Card

Specification proposals are review artifacts:
- compact
- inline with the conversation
- explicit `apply` / `reject` actions
- durable once created

### Execution Card

Execution cards are downstream workflow artifacts:
- generated from approved spec changes
- posted back into the originating conversation when ready
- reviewed as grouped work packages
- distinct from normal assistant prose

Status-only workflow events that do not require user interaction belong in the workflow event log, not the chat timeline.

## Scope Rules

Project scope:
- determines filesystem, repository, and workflow context

Conversation-thread scope:
- determines which discussion history and AI session are active

Changing project must not leak thread state across projects.

Changing thread within a project must not overwrite or mix histories.

## Invariants

The system must maintain these invariants:

- One send creates one durable user turn.
- One send creates at most one active assistant turn.
- A finalized assistant reply is represented once.
- Visible history is monotonic except for explicit retry/failure state transitions.
- Turn ordering is stable.
- Thread switching is explicit user intent.
- Snapshot recovery must reconcile to stable turn identity, not replace history blindly.

## Implementation Direction

The preferred long-term model is:
- durable conversations
- stable turn ids
- append-only turn events
- event-driven streaming
- snapshots only for load/recovery

This document is the source of truth when streaming behavior, ordering, retry semantics, or artifact placement are ambiguous.
