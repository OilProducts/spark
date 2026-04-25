---
id: CR-2026-0008-memoized-conversation-history-rendering
title: Memoized Conversation History Rendering
status: completed
type: feature
changelog: public
---

## Summary
Delivered localized project-chat streaming updates for long conversation histories. Conversation timeline entries now preserve object identity for unchanged rendered rows within the same active conversation, and the history renderer uses memoized row components so stable message, plan, tool-call, request-user-input, and artifact rows avoid unnecessary rerendering while the active streaming Markdown row continues to update.

## Validation
- `npm --prefix frontend run test:unit -- ProjectConversationHistory conversationTimeline` passed: 2 files, 29 tests.
- `npm --prefix frontend run build` passed with the existing large chunk size warning.
- `uv run pytest -q` passed: 1679 passed, 26 skipped.

## Shipped Changes
- Added `stabilizeConversationTimelineEntries` with same-conversation structural sharing and model tests for unchanged reuse, streaming-entry refresh, and conversation-scope isolation.
- Refactored `ProjectConversationHistory` into memoized row components and narrowed row props so sibling streaming changes do not force stable rows to rerender.
- Memoized `ProjectConversationMarkdown` while preserving Markdown rendering for streaming assistant and plan content.
- Stabilized project-chat review and flow-run callbacks used by conversation rows.
- Added focused component coverage for stable Markdown rows, streaming Markdown semantics, and stable plan/artifact rows during sibling streaming updates.
- Updated `specs/spark-ui-ux.md` with the long-thread streaming responsiveness requirement.
