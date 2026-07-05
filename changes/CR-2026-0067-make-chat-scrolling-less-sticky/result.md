---
id: CR-2026-0067-make-chat-scrolling-less-sticky
title: Make Chat Scrolling Less Sticky
status: completed
type: bugfix
changelog: public
---

## Summary

Chat conversation scrolling now stops following the bottom sooner when the user scrolls upward, while preserving the expected pinned-to-bottom behavior for active streaming output and the explicit "Jump to bottom" repin action.

## Validation

- `cargo fmt --all -- --check` passed.
- `cargo test --workspace --all-features` passed.
- `npm --prefix frontend run test:unit` passed. The run emitted existing React `act(...)`, mocked request, and schema-validation stderr output in unrelated tests, but completed with 47 test files and 353 tests passing.
- `npm --prefix frontend run build` passed. Vite reported the existing large chunk size warning.

## Shipped Changes

- Updated `frontend/src/features/projects/hooks/useHomeSidebarLayout.ts` to reduce the conversation bottom threshold from 24px to 4px.
- Changed conversation session updates to read the latest store state and skip redundant writes when pinned state and saved `scrollTop` are unchanged.
- Limited conversation scroll restoration to active conversation changes, so unrelated session-map updates no longer overwrite the active reading position.
- Restored pinned conversations to the bottom and unpinned conversations to their saved `scrollTop` when switching threads.
- Extended `frontend/src/features/projects/__tests__/ProjectsPanel.test.tsx` coverage for the smaller unpin threshold, unpinned streaming behavior, jump-to-bottom repinning, per-thread scroll restoration, and avoiding active-scroll override after unrelated session updates.
