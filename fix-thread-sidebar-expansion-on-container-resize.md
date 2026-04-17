# Fix Thread Sidebar Expansion on Container Resize

## Summary
Change the Home sidebar split from a persisted pixel height to a persisted proportional split so the threads pane and event log both resize with the sidebar container. This fixes the current bug where the thread card stays stuck at its prior pixel height when the containing element grows. The chosen behavior is `Keep proportion`.

## Implementation Changes
- Replace `HomeProjectSessionState.sidebarPrimaryHeight: number` with a proportional value representing the primary-pane share of available split space.
- Keep the current default behavior equivalent to the existing layout by deriving the initial ratio from the current default height and the first measured container height, bounded by the existing min-primary, min-secondary, and handle-height constraints.
- Update `useHomeSidebarLayout` so all drag and keyboard resize interactions:
  - read the current container height,
  - convert the stored ratio into an actual pixel height for rendering,
  - clamp against the same min pane sizes,
  - write back the new ratio instead of a pixel height.
- Update container-resize synchronization so it recomputes the rendered pixel height from the stored ratio whenever the sidebar stack height changes, not just on `window.resize`.
- Use element-level observation for the sidebar stack (`ResizeObserver`) so the split updates when the element grows due to layout changes, not only viewport changes.
- Keep the rendered inline height on `home-sidebar-primary-surface`, but make it a derived value from the ratio and current container height rather than persisted state.
- Preserve existing narrow/mobile behavior and existing min-size constraints for both panes.

## Public Interfaces / Types
- Frontend session state changes:
  - `HomeProjectSessionState.sidebarPrimaryHeight` becomes a proportional split field, named clearly enough to indicate ratio/relative sizing.
- No backend API or workspace contract changes.

## Test Plan
- Update the existing desktop resize test to assert that dragging still changes the split and persists the chosen proportion.
- Add a behavior test where:
  - the sidebar is resized manually,
  - the container height then increases,
  - the thread pane height increases proportionally instead of remaining fixed.
- Add a complementary test where container height decreases and the split remains proportional while respecting min pane sizes.
- Add a test that simulates non-window layout-driven container growth via the observed sidebar element so the fix is not limited to `window.resize`.
- Run the frontend test coverage for the projects panel behavior, then run the full suite with `uv run pytest -q` before reporting completion.

## Assumptions
- The intended UX is that a user-chosen split represents balance between panes, not an absolute pixel lock.
- Existing per-project session persistence should continue to remember the split, but now as a ratio.
- No migration is needed beyond tolerating missing/new session state and falling back to the default proportional split.
