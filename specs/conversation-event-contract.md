# Conversation Event Contract

This document defines the live conversation contract between the workspace backend and the frontend.

It exists to keep four layers distinct:
- raw Codex app-server JSON-RPC notifications
- workspace normalization
- durable `state.json`
- rendered chat cards

Related specs:
- [conversation-state-model.md](/Users/chris/tinker/sparkspawn/specs/conversation-state-model.md)
- [conversation-paradigm.md](/Users/chris/tinker/sparkspawn/specs/conversation-paradigm.md)
- [storage-boundaries.md](/Users/chris/tinker/sparkspawn/specs/storage-boundaries.md)

## Purpose

Spark Spawn chat follows this pipeline:

1. app-server raw notifications are captured in `raw-log.jsonl`
2. workspace normalizes only the documented `item/*` and `turn/*` notifications it needs
3. normalized item identity is materialized into canonical conversation segments
4. the frontend renders directly from those segments

The frontend must not reconstruct chat cards from raw protocol messages or turn-level fallback text.

## Layer 1: Raw App-Server Notifications

Raw notifications are backend-internal inputs. The render-relevant ones are:
- `item/started`
- `item/completed`
- `item/agentMessage/delta`
- `item/reasoning/summaryPartAdded`
- `item/reasoning/summaryTextDelta`
- `item/commandExecution/outputDelta`
- approval-request notifications when present
- `turn/started`
- `turn/completed`
- `error`

Legacy `codex/event/*` item mirrors are not part of Spark Spawn’s render contract. The app-server session should opt out of them where supported, and the workspace must not treat them as a second render source.

## Layer 2: Workspace Live Events

The frontend may consume only these workspace SSE events for chat rendering:
- `turn_upsert`
- `segment_upsert`
- `conversation_snapshot`

### `turn_upsert`

`turn_upsert` carries turn lifecycle metadata:
- turn identity
- role
- status
- parent linkage
- summary content for turn-level previews

`turn_upsert` is not a primary content stream. The frontend must not use it to create or extend assistant/reasoning/tool cards while a turn is active.

### `segment_upsert`

`segment_upsert` is the only granular live chat update contract.

Each `segment_upsert` carries the full current snapshot of one canonical segment. The frontend must upsert by `segment.id`.

Segment kinds:
- `assistant_message`
- `reasoning`
- `tool_call`
- `spec_edit_proposal`
- `flow_run_request`
- `execution_card`

One upstream item becomes one segment.

Workspace-created review artifacts may also produce `segment_upsert` updates even when they do not originate from app-server `item/*` notifications. Those segments are still canonical workspace render units and must be upserted by `segment.id`.

Identity rules:
- assistant segments are keyed by `app_turn_id + item_id` when upstream identity exists
- reasoning segments are keyed by `app_turn_id + item_id + summary_index`
- tool segments are keyed by stable tool/item identity for that call

Rules:
- a new item creates a new segment
- later deltas update the matching segment
- completion updates the same segment instead of creating another one
- multiple assistant commentary items in one turn remain distinct cards
- the final answer remains its own distinct assistant card
- artifact review actions update the existing artifact segment rather than creating duplicate approval/result cards

### `conversation_snapshot`

Snapshots are used for:
- initial load
- reconnect recovery
- durable refresh after mutations

Snapshots are not a second live event grammar. They contain the same canonical turn and segment model that the live stream updates incrementally.

## Normalization Rules

### Authoritative Render Source

Renderable chat artifacts come from `item/*` notifications, not from turn-completion fallbacks.

That means:
- assistant cards come from assistant message items
- reasoning cards come from reasoning items/summaries
- tool rows come from tool/command items

`turn/completed` may finalize turn status and error metadata, but it must never create or repeat a renderable assistant segment.

### Assistant Messages

Raw inputs:
- `item/agentMessage/delta`
- `item/completed` for `AgentMessage`

Rules:
- assistant text is accumulated into an `assistant_message` segment
- the segment keeps `phase` when known, including `commentary` and `final_answer`
- one assistant turn may contain multiple assistant message segments
- no turn-level synthetic assistant segment may be created after the fact if an item-backed segment already exists

If a turn finishes without a final-answer assistant item, Spark Spawn must treat that as an incomplete/failed turn condition rather than synthesizing a visible final assistant card from backup text.

### Reasoning

Raw inputs:
- `item/reasoning/summaryPartAdded`
- `item/reasoning/summaryTextDelta`

Rules:
- reasoning renders as `reasoning` segments only
- reasoning never overwrites assistant text
- if multiple reasoning notifications refer to the same `app_turn_id + item_id + summary_index`, they update the same segment
- normalization may dedupe mirrored reasoning updates by stable reasoning identity, not by arrival order

### Tool Calls

Raw inputs:
- command/tool item deltas and completion
- approval lifecycle events when exposed as item-scoped tool work

Rules:
- tool output mutates the existing `tool_call` segment for that item/call
- repeated output updates the same segment
- completion/failure updates the same segment instead of creating a new row

### Flow Run Requests

`flow_run_request` is a workspace artifact segment, not an app-server item stream.

Rules:
- creating a flow run request emits a pending inline artifact card
- approval, rejection, launch success, and launch failure update that same artifact segment and linked artifact record
- the conversation card is not a live Attractor status mirror
- the card may expose a link or affordance to inspect the launched run elsewhere in the UI once `run_id` exists

## Frontend Rendering Contract

The frontend renders from canonical segments.

Rules:
- initial timeline comes from `conversation_snapshot.segments`
- live updates patch that timeline through `segment_upsert`
- ordering is determined by durable segment order inside a turn, not by reinterpreting raw event sequences on the client
- the temporary “Thinking...” placeholder is ephemeral UI state only and disappears once the first real segment for that assistant turn exists

The frontend must not:
- reconstruct cards from raw protocol messages
- reconstruct cards from historical `turn_events`
- invent assistant cards from `turn_upsert.content`

## Snapshot Requirements

Every conversation snapshot must include:
- `schema_version`
- `conversation_id`
- `project_path`
- `turns`
- `segments`
- workflow/artifact state as defined by the state-model spec

Historical `turn_events`-only snapshot shapes are unsupported. Spark Spawn may reject them instead of attempting reconstruction.

## Failure Semantics

If the app-server produces malformed or incomplete item lifecycles:
- the raw transcript remains in `raw-log.jsonl`
- the turn may fail or remain incomplete
- Spark Spawn must not manufacture extra assistant cards just to “complete” the transcript visually

This is intentional. The render model should be simpler and stricter than the raw transport, not more synthetic.
