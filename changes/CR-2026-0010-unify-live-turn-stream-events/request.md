# Unify Live Turn Stream Events

## Summary

Replace `ChatTurnLiveEvent` with a backend-neutral `TurnStreamEvent` model that both the in-process `agent.Session` path and the Codex app-server path normalize into. Keep the first implementation focused on the shared data contract and Home chat behavior; do not build the Runs Progress UI yet, but shape the event/source model so run-node LLM output can later be persisted and rendered with the same chat-canvas components.

## Key Changes

- Add a shared stream contract in a backend-neutral module, e.g. `spark_common.turn_stream`:
  - `TurnStreamEvent`
  - `TurnStreamEventKind`
  - `TurnStreamSource`
  - channels: `assistant`, `reasoning`, `plan`
  - source metadata: `backend`, `session_id`, `app_turn_id`, `item_id`, `response_id`, `summary_index`, `raw_kind`
  - payload fields: `content_delta`, `message`, `tool_call`, `request_user_input`, `token_usage`, `error`, `phase`

- Replace `ChatTurnLiveEvent` usages in Spark chat:
  - remove the dataclass from conversation models
  - update `ProjectChatService` to consume `TurnStreamEvent`
  - update `UnifiedAgentChatSession` and `CodexAppServerChatSession` callbacks to emit `TurnStreamEvent`
  - preserve current persisted `ConversationTurn` and `ConversationSegment` shapes

- Normalize event vocabulary:
  - replace `assistant_delta` / `reasoning_summary` / `plan_delta` with `content_delta` plus `channel`
  - replace `assistant_completed` / `plan_completed` with `content_completed` plus `channel`
  - keep tool, token, input-request, context-compaction, and error events as backend-neutral kinds
  - move Codex-specific IDs out of top-level event fields into `source`

- Adapt both backends into the shared shape:
  - Codex JSON-RPC processing should emit `TurnStreamEvent` instead of `CodexAppServerTurnEvent`
  - `agent.SessionEvent` should be converted at the agent/Spark boundary into `TurnStreamEvent`
  - keep raw backend protocol logs unchanged for debugging

- Make the run-progress future explicit without building the UI:
  - update specs to define `TurnStreamEvent` as the canonical live LLM/node-output stream model
  - update Runs UX spec to say a future read-only Progress view must render persisted turn-stream content using the same markdown/chat rendering rules as Home
  - do not add a Runs Progress panel, endpoint, or persistence table in this change

## Specification Updates

- `specs/spark-workspace.md`:
  - add the shared turn-stream layer between raw backend protocol and durable workspace state
  - state that Home chat materializes `TurnStreamEvent` into conversation segments
  - state that backend-specific correlation belongs in source metadata, not top-level workspace event shape

- `specs/spark-ui-ux.md`:
  - clarify that Home and future Runs Progress render assistant markdown from the same normalized content semantics
  - clarify that Runs Progress is read-only and derived from run/node LLM output, not an editable conversation
  - keep Runs selected-run SSE ownership unchanged

## Test Plan

- Update backend/session tests:
  - Codex app-server event processing emits `TurnStreamEvent`
  - agent session events adapt to the same `TurnStreamEvent` shape
  - assistant, reasoning, plan, tool, token usage, context compaction, and request-user-input cases preserve behavior

- Update project chat tests:
  - streaming assistant markdown still materializes into one assistant segment
  - reasoning and plan segments still persist correctly from channelized content events
  - final completion still avoids duplicate assistant cards
  - token usage updates still update the assistant turn

- Update frontend tests only where event names appear in mocked SSE payloads:
  - preserve existing `turn_upsert` / `segment_upsert` frontend contract
  - verify chat canvas markdown rendering is unchanged
  - no Runs Progress UI tests yet

- Run:
  - targeted backend tests around `project_chat`, `codex_app_client`, and backend invariance
  - targeted frontend tests for project conversation behavior
  - full suite with `uv run pytest -q` before completion

## Assumptions

- `ChatTurnLiveEvent` is removed rather than kept as a compatibility alias.
- `SessionEvent` may remain as the low-level agent runtime/lifecycle event internally, but Spark-facing consumers must use `TurnStreamEvent`.
- Runs Progress UI is intentionally deferred; this change only creates the shared event contract needed to support it cleanly later.
