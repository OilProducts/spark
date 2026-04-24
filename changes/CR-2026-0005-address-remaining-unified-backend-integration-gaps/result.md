---
id: CR-2026-0005-address-remaining-unified-backend-integration-gaps
title: Address Remaining Unified Backend Integration Gaps
status: completed
type: bugfix
changelog: internal
---

## Summary

Delivered the remaining unified backend integration fixes while preserving the Codex app-server path. The implementation adds durable unified chat history replay, broader assistant failure persistence, unified backend timeout enforcement, isolated model discovery fallback behavior, and consistent `xhigh` reasoning support across backend validation and frontend UI surfaces.

## Validation

- `uv run pytest -q`: 1677 passed, 26 skipped.
- `npm --prefix frontend run test:unit`: 296 passed. Vitest emitted existing React `act(...)` warnings in project/app shell tests, but the suite passed.

## Shipped Changes

- Hydrated `UnifiedAgentChatSession` from completed persisted user/assistant conversation turns before appending the current prompt, avoiding duplicate current-user messages.
- Persisted failed assistant turns for normal exceptions, not only `RuntimeError`, while preserving existing re-raise behavior.
- Wrapped unified backend session execution and repair attempts in node timeout enforcement, returning runtime timeout failures and preserving cleanup.
- Decoupled unified model metadata from Codex app-server model listing so app-server discovery failures no longer fail the whole model endpoint.
- Added `xhigh` reasoning effort support to DOT/default-model validation, stylesheet validation and transforms, frontend stylesheet preview, settings, graph defaults, and the packaged DOT authoring guide.
- Updated the unsupported codergen backend message to include the `provider-router` backend and added regression coverage for the requested backend, chat, model-listing, timeout, and UI validation behavior.
