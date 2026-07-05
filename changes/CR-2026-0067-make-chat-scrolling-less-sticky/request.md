# Make Chat Scrolling Less Sticky

## Summary
Reduce the aggressive bottom-locking behavior while preserving expected chat behavior: follow output only while the user is genuinely at the bottom, stop following when they scroll up, restore each conversation’s previous reading position when switching back, and keep the existing “Jump to bottom” escape hatch.

## Key Changes
- In `useHomeSidebarLayout`, keep per-conversation `isPinnedToBottom` and `scrollTop` state, but make restoration run only when `activeConversationId` changes.
- Remove the dependency that lets ordinary session-map updates retrigger restoration and overwrite active user scrolling.
- Tighten the bottom threshold from `24px` to a smaller tolerance, such as `4px`, so intentional upward scrolls exit pinned mode sooner.
- Avoid redundant conversation session updates when the computed pinned state and saved `scrollTop` have not changed.
- Keep streaming auto-scroll in `useProjectsHomeController`, but only while the conversation remains pinned.
- Keep `scrollConversationToBottom` as the explicit repin path for the “Jump to bottom” button.

## Public Interfaces
- No backend, API, schema, or persisted data shape changes.
- No new UI controls.
- Existing visible behavior remains except that scrolling up should feel less sticky and the “Jump to bottom” button should appear sooner.

## Test Plan
- Update the existing conversation scroll test to verify:
  - pinned conversations follow new streamed output;
  - scrolling up beyond the smaller threshold unpins the conversation;
  - unpinned conversations do not move when new streamed output arrives;
  - clicking “Jump to bottom” repins and scrolls to the latest bottom.
- Add or extend coverage for conversation switching:
  - switching back restores that conversation’s saved `scrollTop`;
  - active scroll updates do not cause restoration to rerun and override the current viewport.
- Run the full validation gate:
  - `cargo fmt --all -- --check`
  - `cargo test --workspace --all-features`
  - `npm --prefix frontend run test:unit`
  - `npm --prefix frontend run build`

## Assumptions
- Restore each conversation’s previous scroll position when returning to it.
- Treat explicit “Jump to bottom” and sending a new message as valid reasons to resume following the bottom.
- Keep the fix scoped to the chat conversation pane; do not alter sidebar resizing or unrelated layout behavior.
