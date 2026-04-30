# Reduce Conversation Snapshot Payloads

## Summary
Fix the Safari freeze by making conversation API/UI payloads lightweight while preserving full persisted conversation state and LLM/runtime behavior. The browser should receive bounded tool-output previews, and full tool output should load only on demand.

## Key Changes
- Add a UI-facing conversation serializer on the backend for `GET /workspace/api/conversations/{id}` and conversation SSE `conversation_snapshot` events.
- In that serializer, truncate only `segment.tool_call.output`:
  - keep command/tool metadata intact
  - include a preview string capped at `8KB`
  - add metadata such as `output_truncated: true` and `output_size`
  - do not mutate `state.json`
- Add a backend endpoint to fetch full output for one segment:
  - `GET /workspace/api/conversations/{conversation_id}/segments/{segment_id}/tool-output?project_path=...`
  - return `{ output, output_size }`
  - 404 if the conversation or segment is missing, or if the segment has no tool output
- Stop duplicate initial full loads:
  - keep the initial `fetchConversationSnapshotValidated`
  - change the SSE endpoint so it does not immediately send another full `conversation_snapshot`
  - continue sending incremental `turn_upsert` and `segment_upsert` events
- Update frontend types/parsers to understand truncated output metadata.
- Update the tool-call UI so expanded historical tool calls can request full output on demand when `output_truncated` is true.
- Keep live `segment_upsert` events bounded with the same serializer so a single large completed tool call cannot freeze the page.

## Test Plan
- Backend tests:
  - UI snapshot truncates large `tool_call.output` and preserves persisted full state.
  - Small tool outputs are unchanged and marked untruncated.
  - Full-output endpoint returns the original full output.
  - SSE connect no longer emits an initial `conversation_snapshot`.
  - `turn_upsert` / `segment_upsert` events still work.
- Frontend tests:
  - Parser accepts `output_truncated` and `output_size`.
  - Tool-call UI shows preview output and can load full output for truncated segments.
  - Conversation stream no longer relies on an SSE initial snapshot.
- Validation:
  - `uv run pytest -q`
  - frontend test command already used by the repo, if separate from pytest

## Assumptions
- UI truncation is API-only and must not affect persisted state, Codex app-server threads, unified-agent sessions, or LLM context.
- `8KB` preview per tool output is the default; it is enough for scanning while keeping large conversations responsive.
- Historical pagination/windowing and storage migration to separate artifact files are deferred.
