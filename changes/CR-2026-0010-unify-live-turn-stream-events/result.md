---
id: CR-2026-0010-unify-live-turn-stream-events
title: Unify Live Turn Stream Events
status: completed
type: refactor
changelog: internal
---

## Summary

Implemented a backend-neutral `TurnStreamEvent` contract in `spark_common.turn_stream` and migrated Spark chat plus Codex app-server turn processing to emit and consume that shared stream shape. The existing persisted conversation turn and segment models remain the durable chat state, while live assistant, reasoning, plan, tool, token usage, context compaction, request-user-input, and error updates now pass through the normalized event vocabulary.

## Validation

- `uv run pytest -q` passed with 1681 passed and 26 skipped.
- Targeted backend coverage passed for project chat behavior and Codex app-client stream normalization, including command output, request-user-input resume, and backend invariance cases.
- Targeted frontend unit coverage passed for project conversation behavior where mocked SSE payloads needed to remain compatible with the existing `turn_upsert` and `segment_upsert` contract.

## Shipped Changes

- Added `src/spark_common/turn_stream.py` with `TurnStreamEvent`, `TurnStreamEventKind`, `TurnStreamSource`, channelized content events, backend source metadata, and normalized payload fields.
- Removed the Spark-facing `ChatTurnLiveEvent` model and updated chat service/session code to materialize conversation segments from `TurnStreamEvent`.
- Updated Codex app-client/server stream handling so Codex-specific JSON-RPC events are adapted into the shared turn-stream contract while raw protocol logging remains separate.
- Updated backend and frontend tests around chat streaming, assistant/reasoning/plan materialization, token usage, request-user-input, and mocked project chat SSE behavior.
- Updated workspace and UI/UX specs to define `TurnStreamEvent` as the canonical live LLM/node-output stream model and to document the deferred read-only Runs Progress direction without adding a Runs Progress UI or persistence surface.
