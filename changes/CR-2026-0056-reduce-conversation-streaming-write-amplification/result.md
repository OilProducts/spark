---
id: CR-2026-0056-reduce-conversation-streaming-write-amplification
title: Reduce Conversation Streaming Write Amplification
status: completed
type: bugfix
changelog: internal
---

## Summary

Delivered the Python app-server chat persistence change so assistant, reasoning, and plan `content_delta` events are forwarded to live clients as transient progress payloads without writing each delta to durable conversation state or `events.jsonl`. Completed content, token usage, tools, request-user-input, compaction, and final turn persistence remain durable. Raw JSON-RPC logging is now opt-in through `SPARK_ENABLE_RAW_RPC_LOG=1`.

## Validation

- `uv run pytest -q` passed: 1951 passed, 26 skipped.

## Shipped Changes

- `src/spark/chat/service.py`: added transient streaming segment payloads for text deltas, stopped durable delta writes for assistant/reasoning/plan streams, and gated raw RPC logger binding behind `SPARK_ENABLE_RAW_RPC_LOG=1`.
- `src/spark/chat/session.py` and `src/spark_common/codex_app_protocol.py`: track reasoning summary text per item and summary index, then emit a completed reasoning segment at turn completion so durable reasoning persistence remains complete.
- `src/spark/workspace/api.py`: prevents transient live events from advancing the durable conversation revision cursor or being dropped as stale duplicate revisions.
- `tests/api/test_project_chat.py`: added regression coverage for transient delta streaming, completed segment persistence, token usage persistence, reasoning summary completion, and raw RPC logging default/opt-in behavior.
