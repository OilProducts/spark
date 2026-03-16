# Conversation State Model

This document defines the canonical durable state model for Spark Spawn project conversations.

It is intentionally separate from:
- [conversation-event-contract.md](/Users/chris/tinker/sparkspawn/specs/conversation-event-contract.md), which defines how raw model/runtime events are normalized into workspace conversation events
- [conversation-paradigm.md](/Users/chris/tinker/sparkspawn/specs/conversation-paradigm.md), which defines the product-level meaning of turns, retries, artifacts, and conversation behavior
- [sparkspawn-workspace.md](/Users/chris/tinker/sparkspawn/specs/sparkspawn-workspace.md), which defines workspace-owned domain concepts and ownership boundaries

This document is the source of truth for:
- what durable conversation files exist on disk
- what each file is for
- how streaming data is collapsed into render-ready state
- how conversation state must be reconstructed after application restart
- which raw protocol identities must be preserved to render streaming segments correctly

---

## 1. Purpose

Spark Spawn conversations need to satisfy two different requirements at the same time:

1. preserve enough protocol fidelity to debug and reason about upstream behavior
2. persist a compact, durable, restart-safe representation that the UI can render exactly enough without replaying raw transport logs

Those requirements must not be collapsed into one file.

The system therefore distinguishes between:
- exact protocol transcript
- normalized render state

This split exists to prevent four recurring failure modes:
- losing the identity needed to keep streaming updates attached to the correct card
- keeping every tiny delta forever in the primary state file
- rebuilding the UI from heuristics after restart instead of from stable durable state
- confusing raw app-server behavior with Spark Spawn’s own workspace model

---

## 2. Design Principles

1. `raw-log.jsonl` preserves exact transport facts.
2. `state.json` preserves durable render state.
3. `state.json` is authoritative for restart/reload rendering.
4. `raw-log.jsonl` is authoritative for debugging and protocol audits.
5. The durable state model must preserve enough identity to determine:
   - whether a new visual segment should be created
   - whether an incoming delta should append to an existing segment
6. Durable state should collapse token-like deltas into materialized segment content once the segment is known.
7. Conversation rendering must be reconstructable without re-running protocol heuristics over the raw log.
8. A separate `session.json` is not part of the default architecture.

Rationale:
- The raw log already preserves exact transport history.
- A separate session file adds storage and lifecycle complexity without clear value unless a concrete resumable-session requirement appears later.
- If resume-only metadata is needed in the future, it should first be considered as a narrowly-scoped `runtime` section inside `state.json` rather than as a third primary representation.

---

## 3. Durable Files

For each conversation, Spark Spawn persists:

- `state.json`
- `raw-log.jsonl`

The workspace also persists a global active conversation-handle index at:
- `SPARKSPAWN_HOME/workspace/conversation-handles.json`

Spark Spawn MUST NOT require any additional per-conversation durable file to reconstruct the conversation UI.

### 3.1 `raw-log.jsonl`

Purpose:
- exact protocol/debug log
- parser evolution
- failure triage
- wire-level audits

Properties:
- append-only
- exact raw JSON-RPC request/response/notification lines as received or sent
- not optimized for rendering
- may remain verbose

The UI MUST NOT render directly from `raw-log.jsonl`.

### 3.2 `state.json`

Purpose:
- durable normalized conversation state
- compact restart-safe render model
- workspace-owned artifacts and statuses
- stable segment identities and finalized segment content

`state.json` MUST be sufficient to:
- reconstruct the visible conversation timeline after restart
- preserve meaningful streaming boundaries such as multiple reasoning cards
- preserve tool-call identity and terminal status
- preserve inline artifact placement

---

## 4. What `state.json` Represents

`state.json` is a materialized view over the conversation, not a raw event transcript.

It stores:
- conversation metadata
- workspace turns
- materialized render segments
- workspace artifacts
- workflow/provenance metadata
- compact event log entries that matter at the workspace level

It does not store:
- every token-level or delta-level protocol event forever
- every transport message verbatim
- presentation heuristics such as “currently open thinking box”

---

## 5. Top-Level Schema

The canonical top-level shape is:

```json
{
  "schema_version": 4,
  "conversation_id": "conversation-...",
  "conversation_handle": "amber-otter",
  "project_path": "/abs/project/path",
  "title": "New thread",
  "created_at": "2026-03-13T13:42:05Z",
  "updated_at": "2026-03-13T13:43:53Z",
  "turns": [],
  "segments": [],
  "event_log": [],
  "spec_edit_proposals": [],
  "flow_run_requests": [],
  "execution_cards": [],
  "execution_workflow": {}
}
```

Required top-level metadata fields:
- `schema_version`
- `conversation_id`
- `conversation_handle`
- `project_path`
- `title`
- `created_at`
- `updated_at`

### 5.1 `turns`

Turns are durable conversational containers.

Turns answer questions like:
- who initiated this conversational unit
- what assistant turn corresponds to a user send
- what terminal status did the assistant turn reach

Turns are not the primary render unit for detailed streaming reconstruction.

### 5.2 `segments`

Segments are the primary render unit.

A segment corresponds to one meaningful visible block in the conversation timeline.

Examples:
- one assistant message bubble
- one reasoning summary card
- one tool-call row
- one system separator
- one inline artifact anchor

### 5.3 `spec_edit_proposals`, `flow_run_requests`, and `execution_cards`

Artifacts are durable workspace objects.

They are not just render fragments.

Initial artifact collections:
- `spec_edit_proposals`
- `flow_run_requests`
- `execution_cards`

Each `flow_run_request` record MUST persist at least:
- `id`
- `created_at`
- `status`
- `flow_name`
- `summary`
- `project_path`
- `conversation_id`
- `source_turn_id`
- `source_segment_id` when known
- `goal` when provided
- `model` when provided
- `run_id` when launched
- `launch_error` when launch failed

### 5.4 `execution_workflow`

Stores workspace-level workflow status relevant to the conversation, not the full Attractor run model.

### 5.5 `event_log`

Stores coarse durable workflow/conversation events that matter at the workspace level.

This is not a substitute for segment content or raw logs.

---

## 6. Turn Model

Turns remain important, but they become lighter-weight than the current event-centric rendering approach.

Each turn MUST include at least:
- `id`
- `role`
- `status`
- `started_at`
- `updated_at`

Assistant turns SHOULD also include:
- `completed_at`
- `parent_turn_id`
- `app_turn_id` when known
- summary references to relevant segments

Example:

```json
{
  "id": "turn-abc",
  "role": "assistant",
  "status": "failed",
  "started_at": "2026-03-13T13:42:05Z",
  "updated_at": "2026-03-13T13:43:53Z",
  "completed_at": null,
  "parent_turn_id": "turn-user-123",
  "app_turn_id": "019ce76e-d10e-7b01-9962-c9d43ca329f1",
  "summary": {
    "final_message_segment_id": "segment-msg-1",
    "has_reasoning": true,
    "has_tool_calls": true,
    "has_artifacts": false
  }
}
```

## 6.1 Unsupported Historical Shapes

Historical `state.json` payloads without:
- `schema_version: 4`
- `segments`

are unsupported.

Spark Spawn does not reconstruct the current timeline from historical `turn_events` payloads anymore. Old local conversations may be deleted instead of migrated.

Turn identity remains the durable grouping for user intent and assistant response, but the detailed visual structure lives in segments.

---

## 7. Segment Model

Segments are the core of the architecture.

### 7.1 Segment Purpose

Each segment represents exactly one renderable timeline block.

That means:
- multiple reasoning sections in one assistant turn become multiple reasoning segments
- a tool call remains one stable segment as it updates
- one assistant message item remains one stable segment as its text grows
- one assistant turn may contain multiple assistant message segments, including multiple `commentary` items and one `final_answer`
- inline artifacts are attached to the segment that produced them or represented by a dedicated artifact segment

### 7.2 Required Segment Fields

Each segment MUST include:
- `id`
- `turn_id`
- `kind`
- `status`
- `order`
- `started_at`
- `updated_at`
- `source`
- `content`

Optional:
- `completed_at`
- `artifact_id`
- `phase`
- `metadata`

Example:

```json
{
  "id": "segment-rsn-1",
  "turn_id": "turn-abc",
  "kind": "reasoning",
  "status": "complete",
  "order": 12,
  "started_at": "2026-03-13T13:42:10Z",
  "updated_at": "2026-03-13T13:42:16Z",
  "completed_at": "2026-03-13T13:42:16Z",
  "source": {
    "protocol": "codex-app-server",
    "app_thread_id": "019ce76e-d0eb-7821-a873-f3efde0caabe",
    "app_turn_id": "019ce76e-d10e-7b01-9962-c9d43ca329f1",
    "item_id": "rs_00b3214b80ef97f90169b4142e53d0819cbc635a5eb6c9c329",
    "summary_index": 0,
    "section_index": 0,
    "call_id": null
  },
  "content": {
    "text": "**Planning** Review the current CLI spec and propose a minimal addition.",
    "heading": "Planning"
  },
  "artifact_id": null,
  "metadata": {}
}
```

### 7.3 Segment Kinds

Initial canonical segment kinds:
- `assistant_message`
- `reasoning`
- `tool_call`
- `flow_run_request`
- `artifact_anchor`
- `system_separator`

Additional kinds may be introduced later, but each must correspond to a meaningful durable render unit, not a transport-specific delta type.

---

## 8. Segment Identity Rules

The state model MUST preserve enough identity to distinguish:
- new segment
- append to existing segment
- finalize existing segment

This identity is derived from raw protocol fields but persisted in normalized workspace form.

### 8.1 Reasoning Segments

A reasoning segment MUST be keyed from:
- `app_turn_id`
- `item_id`
- `summary_index`
- `section_index`

Why:
- one assistant turn may have multiple reasoning items
- one reasoning item may have multiple summary indexes
- one summary may contain multiple visible sections

`section_index` increments when the upstream runtime indicates a reasoning section break.

This identity is what allows the system to know whether to:
- append another reasoning delta to the same visible box
- create a new reasoning box

### 8.2 Assistant Message Segments

An assistant message segment MUST be keyed from:
- `app_turn_id`
- `item_id`

This distinguishes separate message items in one turn.

Assistant message segments MUST also persist a `phase` field when known.

Initial canonical assistant phases:
- `commentary`
- `final_answer`
- `unknown`

The phase does not affect segment identity. It affects presentation and durable meaning.

Examples:
- multiple deltas for one `commentary` item update the same assistant segment
- a later `final_answer` item creates a second assistant segment rather than replacing the commentary segment

### 8.3 Tool-Call Segments

For command/file items:
- `app_turn_id`
- `item_id`

For call-id keyed tool requests such as MCP:
- `app_turn_id`
- `call_id`

### 8.4 Artifact Linkage

Artifacts SHOULD NOT be identified by transport ids.

They become durable workspace objects when persisted and receive:
- `artifact_id`

The segment that originated the artifact MUST store or link:
- `artifact_id`
- origin metadata

---

## 9. Streaming and Compaction Rules

### 9.1 Live Streaming

While a turn is active:
- deltas append into the matching materialized segment
- the segment remains `status = streaming`
- `updated_at` advances

The in-memory live model MAY temporarily track more fine-grained updates than are written to disk, but the persisted state must remain semantically correct if the app stops at any time.

### 9.2 Durable Compaction

Once a segment has enough identity and content to be meaningful, `state.json` SHOULD store the current accumulated segment content rather than every raw delta event.

Examples:
- reasoning summary deltas collapse into one `reasoning` segment text field
- assistant text deltas collapse into one `assistant_message` segment text field per assistant message item
- tool output deltas collapse into the current tool-call output field

This is intentional.

Rationale:
- the raw log already preserves every fine-grained delta
- the durable state should optimize for exact render reconstruction, not token-level history

### 9.3 Completion

When a segment completes:
- `status` becomes `complete` or `failed`
- `completed_at` is set when known
- the final collapsed content remains

The state model MUST NOT require replay of every delta to display the final segment after restart.

---

## 10. Artifact Model and Inline Placement

Artifacts remain first-class workspace objects with their own lifecycle.

They are not merely chunks of assistant text.

### 10.1 Artifact Creation

When an artifact is created, `state.json` MUST persist:
- the artifact object itself
- a durable link from the originating turn/segment to that artifact

### 10.2 Inline Rendering

The conversation timeline may render an artifact:
- as a dedicated artifact segment
- or as an artifact linked from the segment that created it

The chosen representation must preserve stable ordering after restart.

### 10.3 Proposal Example

A spec proposal created by MCP or another tool should follow this durable flow:

1. tool/MCP segment begins
2. tool/MCP segment streams or updates
3. proposal artifact is persisted with `artifact_id`
4. the originating segment gains `artifact_id`, or an adjacent artifact segment is created
5. the frontend renders the proposal card inline at that exact position

This avoids the current failure mode where a proposal card cannot appear until after unrelated heuristics or turn finalization.

### 10.4 Flow Run Request Example

An agent-requested flow launch should follow this durable flow:

1. the assistant creates a `flow_run_request` artifact
2. Spark Spawn persists the artifact with `status = pending`
3. Spark Spawn creates or updates the inline `flow_run_request` segment at the originating conversation position
4. a human review action updates the artifact to `approved` or `rejected`
5. if approved, Spark Spawn launches the Attractor run
6. on successful launch, the artifact stores `run_id` and `status = launched`
7. on launch failure, the artifact stores `status = launch_failed` and `launch_error`

The artifact is the durable conversation object. The Attractor run remains a separate engine object linked by provenance.

---

## 11. Rendering Rules

The frontend MUST render from `state.json` materialized segments, not from replay heuristics over raw turn events.

### 11.1 Ordering

Segments are ordered by durable `order`.

`order` MUST be stable across restart and must reflect normalized segment creation order within the conversation.

### 11.2 Grouping

The frontend MUST group or update segments by durable `segment_id`.

It MUST NOT decide segment boundaries by transient local pointer heuristics such as:
- “current thinking index”
- “append to whichever reasoning box is currently open”

### 11.3 Restart Behavior

After application restart, the frontend MUST be able to:
- load `state.json`
- render the same set of reasoning boxes, message bubbles, tool rows, and inline artifact placements
- resume streaming into the correct in-flight segment if the backend continues the turn

This is the primary reason the segment identity model exists.

---

## 12. `raw-log.jsonl` vs `state.json`

The two files are complementary and intentionally different.

### `raw-log.jsonl`

Use for:
- debugging upstream behavior
- protocol audits
- postmortem analysis
- parser evolution

Do not use as the primary render model.

### `state.json`

Use for:
- UI reconstruction
- restart-safe rendering
- workspace artifact/state inspection
- provenance resolution

Do not try to preserve every raw delta forever here.

---

## 13. No Default `session.json`

Spark Spawn conversation persistence does not require a separate `session.json` as part of the canonical design.

Reasons:
- the raw transcript already preserves transport history
- the durable snapshot should already preserve renderable truth
- a separate session file creates a third representation with unclear ownership

If resumable runtime metadata becomes necessary later, it MUST first be justified by a concrete behavior requirement.

Only then may Spark Spawn consider one of:
- a narrow `runtime` section inside `state.json`
- a clearly-scoped ephemeral sidecar

That future decision must not be made preemptively.

---

## 14. Historical State

Historical `state.json` payloads that do not satisfy this schema are unsupported.

Spark Spawn may reject them instead of attempting migration or best-effort reconstruction.

For records written under this schema, precise segment reconstruction is mandatory.

---

## 15. Decision Summary

Spark Spawn intentionally adopts the following choices:

1. Keep exact transport history in `raw-log.jsonl`.
2. Keep compact, render-ready normalized state in `state.json`.
3. Do not keep a separate default `session.json`.
4. Render and reconstruct from materialized segments, not turn-event heuristics.
5. Preserve enough upstream identity to know when to create a new visible segment versus append to an existing one.
6. Collapse delta streams into materialized segment content in durable state.
7. Keep artifacts as first-class workspace objects linked into the timeline, not as ad hoc sibling turns or purely textual output.

These choices are intended to keep the system:
- debuggable
- compact
- restart-safe
- renderer-friendly
- adaptable to future runtime changes such as MCP-backed artifact creation

This document is the source of truth when the durable conversation storage model, restart behavior, or render reconstruction model is ambiguous.
