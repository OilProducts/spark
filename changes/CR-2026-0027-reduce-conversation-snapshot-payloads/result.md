---
id: CR-2026-0027-reduce-conversation-snapshot-payloads
title: Reduce Conversation Snapshot Payloads
status: completed
type: bugfix
changelog: public
---

## Summary

Implemented lightweight UI conversation payloads so large historical tool outputs no longer get sent wholesale in conversation snapshots or live segment updates. Persisted conversation state remains unchanged, and full tool output is still available through an on-demand API.

## Validation

- `uv run pytest -q` passed with 1746 passed and 26 skipped.
- `npm run test:unit` passed with 315 passed.
- `npm run build` passed.

## Shipped Changes

- Added backend UI serialization that caps `segment.tool_call.output` previews at 8KB and annotates tool calls with `output_truncated` and `output_size`.
- Applied bounded serialization to conversation snapshot responses, published snapshots, mutation responses, and `segment_upsert` SSE payloads without mutating persisted `state.json`.
- Added `GET /workspace/api/conversations/{conversation_id}/segments/{segment_id}/tool-output` to return the original full tool output and size for one segment.
- Removed the duplicate initial `conversation_snapshot` event from the conversation SSE stream while preserving incremental turn and segment events.
- Updated frontend conversation API parsing, workspace client wiring, timeline types, and conversation history UI so truncated historical tool calls fetch full output when expanded.
- Covered the backend API/SSE behavior and frontend parser/UI behavior with tests.
