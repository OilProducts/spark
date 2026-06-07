# Fix Workspace Live Stream Semantics

## Summary
Correct the multiplexed live stream so it behaves as one stable browser/operator stream with independent resource scopes. The fix should address three issues: all-project runs being filtered by the Home project, cursor changes causing EventSource reconnect churn, and conversation catch-up having a subscribe-after-snapshot race.

## Key Changes
- Change `/workspace/api/live/events` from a shared top-level `project_path` model to resource-specific scopes:
  - `conversation_project_path` applies only to `conversation_id`.
  - `runs_project_path` applies only when `include_runs_overview=true`; omit it for all-project runs.
  - `triggers_project_path` applies only to trigger delivery if triggers remain project-scoped.
  - Keep accepting legacy `project_path` only as a compatibility alias when no resource-specific scope is supplied, but do not let it accidentally filter unrelated resources.
- Make the app-shell EventSource URL stable for the selected resource set:
  - Include resource identities and scope flags in the URL: conversation id/project, selected run id, runs overview scope, triggers scope.
  - Do not include advancing cursors like `conversation_revision` or `run_sequence` in the React dependency-driven URL.
  - Track latest cursors in refs inside `WorkspaceLiveEventsController`; use them only when opening/reopening the stream.
  - On `onerror`, close and reconnect with the latest cursor refs, with a small bounded retry delay.
- Fix backend catch-up ordering:
  - Subscribe to requested live hubs before reading snapshots/journals.
  - Then read durable state/journal and emit replay from the requested cursor.
  - Continue consuming the subscribed queue afterward, dropping already-emitted cursor values.
  - Apply this ordering for conversation and selected-run resources.
- Preserve existing durable query APIs:
  - `GET /pipelines/{id}`, `/journal`, `/result`, etc. remain authoritative query APIs.
  - `/workspace/api/live/events` remains live delivery and catch-up, not a replacement for full durable inspection APIs.
  - Deprecated old SSE endpoints may remain deprecated/410 as currently implemented.

## Frontend Behavior
- `WorkspaceLiveEventsController` should compute one URL for the currently observed resources without cursor values:
  - Home active conversation: `conversation_id` + `conversation_project_path`.
  - Runs active scope: `include_runs_overview=true` + `runs_project_path=<activeProjectPath>`.
  - Runs all scope: `include_runs_overview=true` with no runs project path.
  - Selected run: `run_id=<selectedRunId>`.
- Maintain mutable refs:
  - `latestConversationRevisionById`.
  - `latestRunSequenceById`.
- When a live envelope arrives, update the matching ref from `cursor.value` before dispatching the internal browser event.
- When the selected resource identity or scope changes, open a fresh stream using the current ref cursor for that resource.
- When only cursor values advance, do not reconnect.

## Backend Behavior
- Update live endpoint parameter parsing and validation:
  - Require `conversation_project_path` when `conversation_id` is present.
  - Allow `runs_project_path` to be absent for all-project overview.
  - Validate cursors as non-negative integers, but treat them as reconnect/catch-up hints only.
- Ensure independent resource filtering:
  - Conversation replay/live only uses `conversation_project_path`.
  - Runs overview replay/live only uses `runs_project_path`.
  - Selected-run replay/live should not inherit any project filter unless a future explicit run project scope is added.
- For conversation replay:
  - Subscribe first.
  - Read snapshot and journal after the requested revision.
  - Emit contiguous journal events when available.
  - Emit targeted `resync_required` when the journal cannot replay from the requested cursor.
- For run replay:
  - Subscribe first.
  - Read persisted + in-memory run events after requested sequence.
  - Emit contiguous entries and targeted `resync_required` on gaps.
  - Continue with live queue, deduping by sequence.

## Tests
- Add/adjust backend tests for independent scopes:
  - One request with `conversation_id + conversation_project_path=A + include_runs_overview=true` and no `runs_project_path` returns all-project runs, not just project A.
  - Same request with `runs_project_path=A` filters runs overview to project A.
  - `conversation_id` without `conversation_project_path` returns 400.
- Add backend race coverage:
  - Simulate an event published after subscription but before replay completes; assert it is delivered once.
  - Simulate replay plus live duplicate sequence/revision; assert duplicate is dropped.
- Add frontend tests:
  - Advancing a run event sequence does not create a new `EventSource`.
  - Advancing a conversation revision does not create a new `EventSource`.
  - Switching runs or changing runs scope does create a new `EventSource` with the latest known cursor.
  - All-project runs remain all-project even while Home has an active conversation.
- Keep existing full suite expectations:
  - `uv run pytest -q`.
  - `npm run build`.
  - Run the relevant frontend unit slice for `AppShell`, `RunsPanel`, and `ProjectsPanel`.

## Assumptions
- The central app-shell stream remains the only browser-created live stream for Home, runs overview, selected run, and triggers.
- Cursor values are reconnect/catch-up state, not part of the steady-state stream identity.
- Resource scopes should be independent; one stream request may observe resources with different scopes.
