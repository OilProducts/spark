# Fix chat Markdown rendering

## Summary

Upgrade the shared renderer for assistant replies, expanded thinking, and existing plan/result cards in Home and Runs.

Include tables, task lists, strikethrough, syntax highlighting, images, math, and Mermaid. Web links open externally; local file references offer **Copy path**. Images load automatically, and valid Mermaid diagrams render during streaming.

## Rendering changes

- Keep `react-markdown`; add `remark-gfm`, `remark-math`, `rehype-katex`, KaTeX styles, and `rehype-highlight`. Use their standard syntax: `$…$` for inline math and `$$…$$` for display math. Unknown code languages remain plain code. These integrations follow the [existing renderer’s documented plugin approach](https://github.com/remarkjs/react-markdown).
- Preserve ordered-list start numbers. Style tables with header cells, alignment, and horizontal scrolling. Task checkboxes are read-only.
- Give headings a descending visual hierarchy without forced uppercase. Restore paragraph spacing inside lists and blockquotes.
- Preserve code indentation and long lines with horizontal scrolling. Apply inline-code styling only to inline code; retain exact source copying for fenced blocks.
- Recognize attribute-free `<br>`, `<br/>`, and `<br />` as line breaks through an AST transformation. Continue skipping other raw HTML. Preserve CommonMark soft-line-break behavior; do not rewrite code contents.
- Keep all wide content inside the message bubble. Scope styles so they do not override KaTeX, syntax tokens, or diagram output.

## Links, images, and diagrams

- Render HTTP/HTTPS links as accessible links. In browsers, open a new tab with `noopener noreferrer`; in desktop, use Tauri’s [opener plugin](https://v2.tauri.app/plugin/opener/) with permission restricted to HTTP/HTTPS URLs.
- Show local absolute and relative file references as labels with the existing copy control. Copy the path as written, including a supplied line suffix; do not resolve or open files. Preserve message-local anchors for footnotes, with IDs unique to each renderer instance. Unsupported URL schemes remain noninteractive.
- Load HTTP/HTTPS images inline using native lazy loading, bounded width, preserved aspect ratio, and no referrer. On failure, show alt text and the source link. Local image references show alt text and **Copy path**; this change adds no filesystem-serving endpoint.
- Lazy-load Mermaid for `mermaid` fences. Use its [strict security mode](https://mermaid.js.org/config/schema-docs/config-properties-securitylevel.html), disable automatic document scanning, and suppress injected error diagrams.
- Attempt rendering whenever diagram source changes, including incomplete fences during streaming. Allow one render in flight per component, then process the latest pending source. Ignore stale results and results after unmount.
- Show source while the current diagram is pending or invalid; replace it when valid. Keep source available in a disclosure after rendering. Preserve existing copy eligibility.
- Render invalid math as readable source/error text without crashing the message. Keep KaTeX trust disabled.

## Thinking and shared behavior

- Apply the changes through `ProjectConversationMarkdown` and the shared transcript rows.
- Preserve recognized leading bold thinking headings. Otherwise, use a neutral “Thinking” header and make the full content expandable. Render the expanded body through the shared Markdown renderer.
- Keep thinking collapsed by default. User messages and raw tool output remain literal.
- Preserve memoization of unchanged messages, streaming updates, scroll behavior, and existing copy feedback.
- Update the spec and tests that currently require links and images to be suppressed. No persisted-message schema or HTTP API changes are needed; desktop gains only the scoped URL-opening integration.

## Validation and delivery

- Add focused renderer tests covering tables, task lists, strikethrough, list starts, nested code, links/path copying, footnotes, image failures, math, and safe line breaks.
- Test thinking with and without a bold heading, plus shared Home/Run rendering.
- Test Mermaid transitions from invalid to valid, rapid updates, stale completion, multiple diagrams, and unmounts.
- Verify exact code/message copying, unsafe URL rejection, and HTML/script suppression.
- Add a browser smoke scenario using real math and Mermaid rendering, checking wide-content containment and streaming behavior. Manually verify desktop links open the system browser.
- Run relevant Vitest tests, the smoke scenario, frontend build/lint, and desktop checks.

The earlier pending run request is superseded by this plan and must not be launched. No implementation or new run request occurs during planning.
