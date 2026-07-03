---
id: CR-2026-0057-decouple-conversation-turn-acceptance-from-assistant-completion
title: Decouple Conversation Turn Acceptance From Assistant Completion
status: completed
type: feature
changelog: public
---

## Summary

`POST /workspace/api/conversations/{conversation_id}/turns` now represents durable turn acceptance instead of assistant completion. The route synchronously validates and persists the user turn plus pending assistant turn, publishes the started events, returns the started snapshot, and continues assistant execution in the background. Post-acceptance backend failures are recorded as failed assistant turns and published through the existing SSE path.

The projects frontend no longer appends timeline-level optimistic user messages. Sending clears the draft and uses a short non-timeline pending-start state until the conversation cache advances from the POST snapshot or live SSE.

## Validation

- `cargo fmt --all -- --check` passed.
- `cargo test --workspace --all-features` passed.
- `npm --prefix frontend run test:unit` passed with existing stderr warnings from React act/schema/mock-request test cases.
- `npm --prefix frontend run build` passed with the existing Vite chunk-size warning.

## Shipped Changes

- `crates/spark-workspace/src/conversations.rs`: added a reusable completion path for prepared started turns and converted agent backend failures after acceptance into durable failed assistant turns with final snapshot events.
- `crates/spark-http/src/workspace.rs`: changed the conversation turn route to call `start_turn`, publish started events, return the started snapshot immediately, and spawn asynchronous completion that keeps SSE state contiguous.
- `crates/spark-http/tests/*conversation*`: updated HTTP and SSE contracts to assert started snapshots, eventual completion, and failed assistant publication through SSE.
- `frontend/src/features/projects` and `frontend/src/state`: replaced optimistic timeline message state with pending conversation turn state, disabled/sent composer affordances while the started snapshot is pending, and preserved snapshot/SSE cache behavior.
