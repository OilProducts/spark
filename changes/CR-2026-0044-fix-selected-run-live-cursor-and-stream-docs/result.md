---
id: CR-2026-0044-fix-selected-run-live-cursor-and-stream-docs
title: Fix Selected-Run Live Cursor and Stream Docs
status: completed
type: bugfix
changelog: public
---

## Summary

Delivered the selected-run live cursor fix by moving browser-facing live updates onto the shared Workspace live stream and withholding `run_id` until selected-run detail and durable journal hydration establish an initial cursor. Loaded journals now live-tail with `run_sequence=<newest loaded sequence>`, while loaded empty journals use `run_sequence=0`; reconnects reuse the latest cursor held outside the memoized stream identity.

The change also documents the resource-scoped live stream contract: conversations require `conversation_project_path`, runs overview uses optional `runs_project_path`, triggers use `triggers_project_path`, and `conversation_revision` plus `run_sequence` are reconnect/catch-up cursors rather than steady-state stream identity.

## Validation

- `uv run pytest -q`: 2000 passed, 26 skipped.
- `npm run test:unit`: 47 files passed, 341 tests passed.
- `npm run build`: completed successfully; Vite reported the existing large-chunk warning.

## Shipped Changes

- Frontend app-shell live delivery now uses one Workspace live controller for conversations, runs overview, selected-run journal updates, and triggers.
- Selected-run live subscription is gated on `runJournalStore` readiness through `resolveRunJournalLiveCursor`, preventing full journal replay before the durable cursor is known.
- Run, conversation, trigger, and project hooks/tests were updated to consume resource-scoped live envelopes instead of feature-specific SSE routes.
- Workspace API now exposes `/workspace/api/live/events` with resource filters and cursor catch-up behavior; deprecated conversation and runs-overview feature streams now return `410`, while the pipeline event route remains deprecated.
- Backend tests, frontend unit tests, smoke coverage, README, UI/spec docs, Attractor spec, and Spark operations guide were updated to match the new stream shape and selected-run hydration behavior.
