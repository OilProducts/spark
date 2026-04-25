# Memoized Conversation History Rendering

## Summary
Improve long-thread streaming responsiveness by keeping Markdown streaming intact while preventing unchanged conversation history rows from rerendering/reparsing. Add a concise UI/UX spec note that active chat streaming should remain responsive in long threads without prescribing React implementation details.

## Key Changes
- Update `specs/spark-ui-ux.md` section 10 with one requirement: streaming updates in long project-chat threads should stay localized enough that existing history and composer input remain responsive.
- Add structural sharing for derived conversation timeline entries:
  - Keep `buildConversationTimelineEntries` as the pure timeline builder.
  - Add a pure stabilizer that reuses previous `ConversationTimelineEntry` objects when their rendered fields are unchanged.
  - Scope reuse by active conversation id so entries from different threads are never cross-reused.
- Split `ProjectConversationHistory` rendering into memoized row components:
  - At minimum memoize message/thinking rows, plan rows, tool-call rows, request-user-input rows, and artifact rows.
  - Pass row-specific resolved props rather than broad Maps/records to every row.
  - Make callbacks used by rows stable where needed, especially `onOpenFlowRun` and plan review-note updates.
- Preserve Markdown while streaming:
  - Assistant and plan content continues to render through `ProjectConversationMarkdown` during `streaming`.
  - Memoize `ProjectConversationMarkdown` so unchanged Markdown content is not reparsed when sibling rows update.
  - The currently streaming row still rerenders immediately for each content update.

## Test Plan
- Add model tests for timeline structural sharing:
  - unchanged entries reuse object identity across rebuilds
  - the changed streaming entry gets a new object
  - switching conversation scope does not reuse prior thread entries
- Add focused frontend component tests:
  - rerendering history with only the latest assistant Markdown changed does not rerender/reparse an older unchanged assistant Markdown row
  - the latest streaming assistant Markdown row still updates and renders Markdown semantics
  - existing plan Markdown behavior remains unchanged
- Keep existing ProjectsPanel streaming tests unchanged unless imports/component factoring require small fixture adjustments.
- Run:
  - `npm --prefix frontend run test:unit -- ProjectConversationHistory conversationTimeline`
  - `npm --prefix frontend run build`
  - `uv run pytest -q`

## Assumptions
- No virtualization and no stream-event coalescing.
- No backend API, SSE payload, or conversation storage changes.
- Render-count tests are acceptable here because the requested behavior is specifically a UI performance containment guarantee, not a source-text assertion.
