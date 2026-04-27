---
id: CR-2026-0012-runs-progress-active-node-first
title: "Runs Progress: Active Node First"
status: completed
type: feature
changelog: public
---

## Summary
Delivered a read-only Runs Progress view that prioritizes the selected run's current-node LLM output before recent LLM output. The progress projection now exposes an active current-node entry, recent entries excluding that active stream, and newest-first node filter options. The selected progress filter is stored per run detail session, defaults to `current`, and supports current-node, recent, and concrete-node views.

## Validation
- `cd frontend && npm run test:unit -- --run src/features/runs/__tests__/RunsPanel.test.tsx src/features/runs/model/__tests__/timelineModel.test.ts`: passed, 2 files and 27 tests.
- `cd frontend && npm run build`: passed; Vite reported the existing large chunk size warning.
- `uv run pytest -q`: passed, 1690 tests with 26 skipped.

## Shipped Changes
- Updated the Runs timeline model and hook to build a structured progress projection from loaded run journal entries, pass in the selected run's current node, and keep `LLMContent` out of the summary card's latest-journal fact.
- Added the `RunProgressCard` UI between pinned questions and Run Journal with a compact header, native node filter select, current-node labeling, and markdown rendering through the existing project conversation renderer.
- Added run-local session state for progress collapse and node filter persistence across tab switches and selected-run re-renders.
- Extended Runs Panel and timeline model tests for current-node priority, recent fallback/filtering, placement, default filter behavior, bounded recent display, and summary exclusion of LLM content.
- Updated `specs/spark-ui-ux.md` to document the delivered monitoring hierarchy and Progress contract.
- Preserved the selected-run SSE/journal path for backend `LLMContent` emission and kept backend invariance coverage for Codex and unified-agent streaming progress events.
