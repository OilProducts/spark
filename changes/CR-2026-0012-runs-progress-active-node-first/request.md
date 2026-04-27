# Runs Progress: Active Node First

## Summary
Refine the first-pass Runs Progress view so it answers “what is happening right now?” before it acts as a recent-output archive. Keep the existing backend `LLMContent` journal event path and selected-run SSE ownership; do not add a second stream or broaden backend APIs in this step.

## Key Changes
- Update the Progress model to return a structured projection with:
  - `activeEntry`: newest LLM stream whose `nodeId` matches the selected run’s current node.
  - `recentEntries`: newest remaining LLM streams, excluding `activeEntry`.
  - `nodeOptions`: distinct nodes that have loaded LLM content, newest first.
- Add run-local Progress view state:
  - `progressNodeFilter: 'current' | 'recent' | string`
  - Default: `'current'`.
  - `'current'` shows the active node stream when available, otherwise falls back to recent streams with an empty-state note that the current node has not emitted LLM content.
  - `'recent'` shows the bounded recent streams exactly as the first-pass view does today.
  - A concrete node id shows only streams for that node.
- Update `RunProgressCard` UI:
  - Header stays compact and read-only.
  - Add a small native select with options: `Current node`, `Recent`, then loaded node ids.
  - Render the active/current stream first and visually label it `Current node`.
  - Keep markdown rendering through `ProjectConversationMarkdown`.
  - Keep the card between pinned questions and Run Journal.
- Update Runs tab wiring:
  - Pass `selectedRun.current_node` into the progress projection.
  - Store the selected progress filter in `RunDetailSessionState` so it survives tab switches and selected-run re-renders.
  - Keep `LLMContent` excluded from the summary card’s latest-journal fact.
- Update `specs/spark-ui-ux.md`:
  - Replace the “future Runs Progress view” language with the actual contract.
  - Document the monitoring hierarchy as: Summary, pinned questions, Progress, Run Journal, Advanced.
  - State that Progress is read-only, derived from selected-run journal/SSE data, prioritizes current-node LLM output, and uses the same markdown rendering semantics as Home assistant output.

## Test Plan
- Extend `timelineModel` unit tests:
  - Current-node stream is selected as `activeEntry`.
  - Recent streams exclude the active stream.
  - Missing current-node content falls back to recent entries.
  - Node filter returns only matching node streams.
- Extend `RunsPanel` tests:
  - Progress appears between pinned questions and Run Journal.
  - `Current node` is the default filter.
  - Current node output is shown before other recent output.
  - Switching to `Recent` shows bounded recent streams.
  - Summary latest-journal still ignores `LLMContent`.
- Keep backend tests from the first pass for Codex and unified-agent `LLMContent` emission.
- Verification commands:
  - `cd frontend && npm run test:unit -- --run src/features/runs/__tests__/RunsPanel.test.tsx src/features/runs/model/__tests__/timelineModel.test.ts`
  - `cd frontend && npm run build`
  - `uv run pytest -q`

## Assumptions
- Treat the current uncommitted first-pass Progress implementation as the baseline.
- No backend schema or endpoint changes in this step.
- No full historical LLM-output paging yet; Progress works from loaded journal entries plus live tail.
- Non-LLM node activity remains in Summary and Run Journal for now.
