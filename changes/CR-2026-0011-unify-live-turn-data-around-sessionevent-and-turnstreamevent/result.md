---
id: CR-2026-0011-unify-live-turn-data-around-sessionevent-and-turnstreamevent
title: Unify Live Turn Data Around SessionEvent and TurnStreamEvent
status: completed
type: refactor
changelog: internal
---

## Summary

Implemented the live-turn stream boundary around `agent.SessionEvent` and strict `TurnStreamEvent.source` metadata. The agent stream now preserves reasoning, model-proposed tool calls, finish usage, and assistant text response IDs as `SessionEvent` data, while Spark converts those session events into `TurnStreamEvent` at the chat boundary. Codex remains a peer runtime path that normalizes JSON-RPC notifications directly into `TurnStreamEvent` with Codex source metadata.

## Validation

- `uv run pytest -q tests/spark_common/test_codex_app_client.py tests/api/test_project_chat.py -x --maxfail=1`
- `uv run pytest -q tests/api/test_backend_invariance.py -x --maxfail=1`
- `uv run pytest -q tests/agent/test_events.py tests/agent/test_agent_streaming.py -x --maxfail=1`
- `uv run pytest -q`

Final full suite result: 1682 passed, 26 skipped.

## Shipped Changes

- Expanded `src/agent/events.py` and `src/agent/session.py` so streamed reasoning, provider/model tool-call lifecycle events, usage updates, and assistant text completion response IDs are emitted as generic `SessionEvent` values.
- Tightened the Spark live-turn contract around `spark_common.turn_stream.TurnStreamEvent`, with correlation fields read from `TurnStreamEvent.source` instead of compatibility top-level fields.
- Updated Spark chat service/session materialization so assistant, reasoning, plan, tool execution, token usage, context compaction, request-user-input, and error updates consume normalized `TurnStreamEvent` data.
- Updated Codex app-server client/server normalization to emit the strict `TurnStreamEvent` shape directly from JSON-RPC notifications.
- Removed the old Spark-facing `ChatTurnLiveEvent` model and updated agent, Spark chat, backend invariance, and Codex tests to cover the new event boundary.
- Updated coding-agent and workspace specs to document `SessionEvent` as the agent runtime stream, `TurnStreamEvent` as the Spark live-turn/materialization boundary, and the absence of a direct `unified_llm.StreamEvent` to Spark chat rendering path.
