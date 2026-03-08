# Spark Spawn UI Story Records

This document is the canonical detailed record for tracked UI user stories.

Use it for implementation, testing, auditing, refactoring, and spec cross-reference work.

The intended split is:
- `ui-user-stories.md`: concise story catalog and stable IDs
- `ui-story-records.md`: detailed story records with rationale, acceptance criteria, non-goals, implementation intent, and references

---

## Story Record Template

### STORY-ID

**Summary**

As a ...

**Rationale**

- ...

**Acceptance Criteria**

- ...

**Non-Goals**

- ...

**Implementation Intent**

- ...

**References**

- `ui-user-stories.md#...`
- `ui-spec.md#...`

---

## US-CONV-12

**Summary**

As a user, when I send a message in project chat, I want to see the assistant reply stream into the conversation as it is generated so that I can tell the model is actively responding and follow the answer before it completes.

**Rationale**

- `Thinking...` alone is too opaque for longer turns.
- The Codex desktop app establishes the expectation that assistant text appears progressively.
- Progressive assistant output improves perceived latency and makes tool usage feel connected to the emerging answer.

**Acceptance Criteria**

- After the user sends a message, an assistant row appears immediately in the active thread.
- As assistant text is generated, that same row updates incrementally instead of waiting for final completion.
- Tool-call rows can appear before, between, or after assistant text updates without breaking timeline order.
- If the assistant emits no text yet, the placeholder remains `Thinking...` until the first streamed text arrives.
- When the turn completes, the live assistant row is finalized into the persisted assistant message without duplicating the final response.
- If the turn fails, the live assistant row is removed or converted into an explicit error state instead of lingering indefinitely.
- Streaming updates remain attached to the active thread even when backend path canonicalization differs from the client-side project key.
- If the user is already at the bottom of the thread, the viewport follows streamed updates; if the user has scrolled away, the viewport stays put and the existing jump-to-bottom affordance remains available.

**Non-Goals**

- Exposing raw internal reasoning.
- Guaranteeing token-perfect character-by-character streaming for every turn.
- Replacing tool-call observability; assistant streaming is additive to that behavior.

**Implementation Intent**

- Preserve the current SSE conversation-snapshot model if possible.
- Treat assistant delta text as first-class live conversation state rather than only as a final parsed artifact.
- Final assistant completion should promote the live assistant row into durable conversation state rather than append a second assistant row.
- Backend handling should prefer progressive assistant deltas when available, while remaining robust when the app-server only emits sparse updates or a final message.
- Tests should verify observable progressive updates in the chat timeline rather than exact protocol event names.

**References**

- `ui-user-stories.md#2-project-scoped-ai-conversation-and-spec-editing`
- `ui-spec.md#55-ai-conversation-and-spec-authoring-loop`
- `conversation-paradigm.md`
