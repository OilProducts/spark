# Context Compaction Visibility Plan

## Summary

Make app-server context compaction visible inside Home Project Chat as a durable inline system artifact.

- when the Codex app server compacts conversation context during a turn, Spark should record that event in the conversation timeline
- the compaction row should render inline with the rest of the conversation, not as a transient toast or hidden debug detail
- the persisted artifact should survive reloads and snapshots
- duplicate app-server notifications for the same compaction should collapse into one conversation row

This plan is intentionally narrow. It does not change compaction behavior, token budgeting, or prompt content. It only makes the existing compaction event visible in Project Chat.

## Problem

Spark currently receives evidence that compaction happened, but it drops that information before it reaches the persisted conversation timeline.

Observed repository facts:

- the raw app-server log shows `item/started` and `item/completed` for `contextCompaction`, followed by `thread/compacted`
- `process_turn_message()` in `src/spark_common/codex_app_server.py` normalizes tool, assistant, reasoning, command-output, token-usage, error, and turn-complete events, but it does not normalize compaction events
- `ChatTurnLiveEvent` does not currently have compaction event kinds
- `CodexAppServerChatSession.turn()` only emits live events for assistant, reasoning, and tool activity
- `ProjectChatService._materialize_segment_for_live_event()` only persists reasoning, assistant-message, and tool-call segments
- the frontend conversation API parser and timeline model reject unknown segment kinds, so even a backend segment would currently be dropped before rendering

As a result, the user gets no inline indication that the thread compacted, even though the app server explicitly reported it.

## Goals

- Surface compaction inline in Project Chat using the same durable conversation-artifact model used for other system-visible events.
- Persist compaction as a conversation segment so it remains visible after reload.
- Show one row per compaction occurrence, not duplicate rows for both `contextCompaction` and `thread/compacted`.
- Keep the visual treatment minimal and consistent with other system rows.

## Non-Goals

- No change to when or why the Codex app server performs compaction.
- No new banner, toast, modal, or debug-only inspector.
- No change to prompt construction, plan-mode rules, or run-launch behavior.
- No attempt to expose a detailed token-budget explanation unless the app server already provides one.

## Implementation Plan

### 1. Normalize compaction from the app-server event stream

Extend the app-server turn-event adapter so compaction becomes an explicit normalized event instead of a raw log-only detail.

Target change:

- teach `process_turn_message()` to recognize `item/started` with `item.type == "contextCompaction"` as compaction start
- teach `process_turn_message()` to recognize `item/completed` with `item.type == "contextCompaction"` as compaction completion
- recognize `thread/compacted` as a completion fallback signal for the current turn

Why:

- the Codex app server is already the source of truth for whether compaction happened
- Spark should not infer compaction indirectly from token usage or missing history

Implementation notes:

- keep the normalized event payload small: app turn id, item id when present, and a simple event kind
- do not create a separate visible row for both completion signals when they refer to the same compaction

### 2. Thread compaction through the chat session as live events

Update the session layer so normalized compaction events become `ChatTurnLiveEvent`s consumed by project-chat persistence.

Target change:

- add live-event kinds such as `context_compaction_started` and `context_compaction_completed`
- emit them from `CodexAppServerChatSession.turn()` with the current app turn id and item id

Why:

- this preserves the existing layering, where the session translates app-server events into chat-domain live events and the service persists them
- it avoids teaching the persistence layer about raw JSON-RPC method names

Implementation notes:

- if `thread/compacted` arrives after an already-completed compaction item, treat it as a no-op
- if only `thread/compacted` arrives, use it to complete the in-flight compaction segment for that turn

### 3. Persist a durable compaction segment in conversation state

Extend the live-event materialization path so compaction becomes a system segment in the same conversation turn.

Target change:

- add a new conversation segment kind, `context_compaction`
- create or update one segment per compaction occurrence using a stable segment id derived from app turn id and item id when available
- mark the segment `running` while compaction is in progress and `complete` when it finishes

Suggested content:

- running: `Compacting conversation context…`
- complete: `Context compacted to continue the turn.`

Why:

- the user should be able to see that the turn compacted even after the page reloads
- the segment model is already the durable mechanism for reasoning, tool calls, and other inline turn details

Implementation notes:

- use `role="system"` so the row is clearly metadata, not assistant prose
- avoid generic notice plumbing; this is a single concrete artifact type

### 4. Extend the conversation API and frontend timeline unions

Update the frontend response parser and timeline model so the new segment kind survives parsing and reaches the renderer.

Target change:

- allow `context_compaction` in `ConversationSegmentResponse`
- allow a corresponding timeline entry kind in the project chat model
- map persisted `context_compaction` segments into a dedicated system timeline row

Why:

- today the frontend drops unknown segment kinds during response parsing
- adding backend persistence alone would not make the event visible

### 5. Render compaction inline with existing system rows

Add a compact inline renderer in Project Chat that matches the existing visual language for mode-change and final-separator rows.

Target change:

- render compaction as a centered, subdued system row in the conversation history
- distinguish `running` and `complete` text, but keep both lightweight

Why:

- compaction is timeline metadata, not a tool call and not assistant content
- reusing the existing system-row look keeps the UI consistent and low-noise

Implementation notes:

- do not render this as a collapsible card
- do not render it as Markdown assistant content

### 6. Add regression tests across the actual failure boundaries

Add focused tests that prove compaction survives the end-to-end path from app-server event to inline conversation row.

Required coverage:

- backend unit test: app-server event parsing recognizes `contextCompaction` start and completion
- session unit test: `CodexAppServerChatSession.turn()` emits compaction live events
- project-chat test: compaction live events persist a single `context_compaction` segment that transitions from running to complete
- project-chat test: duplicate completion signals do not create duplicate segments
- frontend API parser test: `context_compaction` segments are accepted
- frontend timeline/rendering test: a persisted compaction segment renders as an inline system row

Test style constraints:

- assert on observable behavior through normalized events, persisted segments, API snapshots, and rendered UI
- do not assert on prompt strings or incidental internal wording outside the artifact content itself

### 7. Keep the change isolated from concurrent chat work

There are already unrelated in-flight edits in the worktree touching project chat and planning UI. The implementation should:

- preserve unrelated modifications
- avoid mixing compaction visibility work with `request_user_input` or plan-mode composer work
- keep the change scoped to event normalization, conversation persistence, API parsing, and inline rendering

## Target Paths

- `src/spark_common/codex_app_server.py`
- `src/spark/chat/session.py`
- `src/spark/chat/service.py`
- `src/spark/workspace/conversations/models.py`
- `frontend/src/lib/api/conversationsApi.ts`
- `frontend/src/features/projects/model/types.ts`
- `frontend/src/features/projects/model/conversationTimeline.ts`
- `frontend/src/features/projects/components/ProjectConversationHistory.tsx`
- backend and frontend tests closest to those surfaces

## Acceptance Criteria

- When the Codex app server reports context compaction during a project-chat turn, the conversation snapshot includes a durable `context_compaction` segment.
- Project Chat renders that compaction inline in the same conversation history after reload.
- A single compaction occurrence produces one visible row even if both `item/completed` and `thread/compacted` are received.
- The row is rendered as a system artifact, not as an assistant message or tool-call card.
- Existing assistant, reasoning, tool-call, mode-change, and flow-launch behavior remains unchanged.

## Validation

- focused backend tests for compaction event parsing and segment persistence
- focused frontend tests for parser, timeline mapping, and history rendering
- `uv run pytest -q`

## Out of Scope Follow-Up

If this lands cleanly, a later planning task can decide whether Spark should expose richer compaction metadata when available, such as:

- a more specific reason if the app server ever provides one
- token-usage context before and after compaction
- a user-facing explanation for how compaction affects what the assistant still remembers
