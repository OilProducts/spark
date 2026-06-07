# Multiplexed Live Event Stream with Resource-Scoped Journals

## Summary
Replace Spark’s browser/operator live-event model with **one browser-facing live SSE stream** that multiplexes updates for the currently observed resources. Do **not** create one giant durable workspace event history. Durable replay remains resource-scoped: run journals stay per run, conversation replay is per conversation, and overview/state resources use targeted snapshots plus live deltas.

This fixes the browser connection starvation bug by removing independent feature-owned long-lived SSE connections while avoiding a monolithic global event store.

The canonical model becomes:
- **One app-shell live transport:** a single EventSource for UI/operator live updates.
- **Many resource-scoped histories:** run journal per run, conversation event/revision history per conversation, trigger/project state through targeted state endpoints.
- **Attractor API remains command/query truth:** run start, status, journal, questions, answers, artifacts, graph, context, checkpoint.
- **Live stream is delivery, not storage:** it fans in resource events and supports resource-scoped catch-up.

## Key API and Data Model Changes
- Add one workspace live stream endpoint, named to avoid implying global durable storage:
  - `GET /workspace/api/live/events`
- The client tells the stream which resources it currently observes. Use query parameters for v1:
  - `project_path=<path>` for scoped overview events.
  - `conversation_id=<id>&conversation_revision=<n>` for active chat replay.
  - `run_id=<id>&run_sequence=<n>` for selected-run journal replay.
  - `include_runs_overview=true` for run-list upserts.
  - Optional `include_triggers=true` if trigger live updates remain needed.
- Event envelopes should be typed and resource-scoped:
  - `type`: e.g. `conversation.turn_upsert`, `conversation.segment_upsert`, `conversation.snapshot`, `run.upsert`, `run.journal_entry`, `run.question_pending`, `run.question_answered`, `trigger.upsert`, `resync_required`.
  - `project_path`: nullable for global resources.
  - `resource`: `{ "kind": "run" | "conversation" | "runs_overview" | "trigger", "id": string | null }`.
  - `cursor`: resource cursor after applying the event, e.g. `{ "kind": "run_sequence", "value": 1201 }` or `{ "kind": "conversation_revision", "value": 42 }`.
  - `payload`: existing resource-specific payload, preserving current shape where practical.
- Avoid a global durable `sequence` as the primary replay contract. If an internal delivery sequence is useful for diagnostics, keep it clearly non-authoritative.
- Add targeted `resync_required` events when replay cannot satisfy a resource cursor:
  - `{ type: "resync_required", resource: { kind, id }, reason }`
  - The client then fetches only that resource’s authoritative endpoint.

## Backend Implementation
- Add a live event multiplexer under the Workspace API layer. It should subscribe internally to existing producers and expose exactly one browser/operator stream.
- Keep resource truth and history where it belongs:
  - Runs: use existing per-run persisted `events.jsonl` and `/attractor/pipelines/{id}/journal` logic for replay.
  - Conversations: add or expose a per-conversation event journal keyed by conversation revision. Current revision numbers already exist; use them for replay instead of broad snapshot recovery.
  - Run overview: hydrate from `GET /attractor/runs`, then live `run.upsert` deltas.
  - Triggers/project metadata: treat as state resources; emit invalidation/upsert events and fetch targeted state as needed.
- Publish all current live-producing paths into the multiplexer:
  - run-list upserts,
  - run journal entries,
  - human-gate pending/answered changes,
  - conversation turn/segment updates,
  - conversation snapshot/sidecar invalidations,
  - trigger changes.
- Remove or stop supporting app-facing per-feature SSE endpoints:
  - `/workspace/api/conversations/{conversation_id}/events`
  - `/attractor/runs/events`
- Replace `/attractor/pipelines/{id}/events` as the Spark UI/operator live-event path. If it cannot be deleted safely in the same pass, mark it deprecated and ensure no frontend or CLI code uses it.
- Do not touch remote worker `/v1/runs/{run_id}/events`; that is a separate worker execution protocol.
- Update specs and docs so the contract is explicit:
  - Attractor owns run commands, run detail, durable journal, artifacts, questions, answers.
  - Workspace live stream owns multiplexed app/operator live delivery.
  - Durable replay is resource-scoped, not one workspace-global log.

## Frontend Implementation
- Add one app-shell live event controller mounted once in `AppSessionControllers` or a neighboring app-level module.
- Remove all feature-owned `new EventSource(...)` calls for Home chat, Runs list, and selected RunStream.
- Replace them with store-level event application:
  - conversation events call the existing conversation cache reducers;
  - run overview events update runs-list state;
  - run journal events update selected-run journal state;
  - human-gate events update pending question state;
  - trigger events update trigger list/detail state.
- The app-level controller computes the observed resource set from store state:
  - active project path,
  - active conversation id and cached revision,
  - selected run id and newest loaded run sequence,
  - whether runs overview/triggers are currently needed.
- On observed resource changes, reconnect the single SSE stream with the new resource query. Keep this one connection, not one connection per resource.
- On `resync_required`, fetch only the relevant resource:
  - conversation snapshot for one conversation,
  - run status/journal for one run,
  - runs list for one project/all scope,
  - trigger list/detail as applicable.
- Keep hidden UI state cached, but do not let hidden panels own network transports. Visibility may affect the observed resource set, but it must not destroy local session state.

## CLI and Operator Surface
- Add a first-class CLI command, for example:
  - `spark run events <run_id> [--after <sequence>]`
  - optional `--json` for machine-readable output.
- The CLI uses `/workspace/api/live/events` with `run_id` and `run_sequence`, not `/attractor/pipelines/{id}/events`.
- Update `spark-operations.md`:
  - Replace curl live-tail examples with `spark run events`.
  - Keep curl examples for durable query endpoints: `/pipelines/{id}`, `/journal`, `/questions`, `/artifacts`, etc.
- If a raw HTTP live-tail example is still documented, point it at `/workspace/api/live/events` with resource filters, not Attractor per-run SSE.

## Test Plan
- Backend tests:
  - live endpoint returns SSE headers and keepalives;
  - endpoint accepts resource filters and rejects malformed cursor values;
  - run replay emits missed per-run journal entries after `run_sequence`;
  - conversation replay emits missed per-conversation events after `conversation_revision`;
  - run overview emits current upserts through the live stream;
  - human-gate answer emits a gate answered/update event without opening a second stream;
  - replay gaps emit targeted `resync_required`;
  - deprecated/removed app-facing SSE endpoints are either gone or explicitly no longer referenced by frontend contracts.
- Frontend tests:
  - app opens exactly one EventSource during normal Home/Runs operation;
  - Home chat receives turn/segment updates through the live controller;
  - Runs list receives run upserts through the live controller;
  - selected-run journal receives live entries through the live controller;
  - answering a pending human gate does not create another EventSource and does not stall subsequent chat sends;
  - resource changes reconnect the single stream with updated filters;
  - `resync_required` triggers targeted fetches rather than broad snapshots.
- CLI tests:
  - `spark run events <run_id>` builds the expected `/workspace/api/live/events` request;
  - `--after` maps to `run_sequence`;
  - JSON and text output handle replayed entries and live entries.
- Remove/rewrite tests that preserve accidental complexity:
  - stream-count tests expecting one runs-list stream plus one selected-run stream;
  - tests expecting conversation components to own their own EventSource;
  - frontend static contract tests requiring direct use of `/workspace/api/conversations/{id}/events`, `/attractor/runs/events`, or `/attractor/pipelines/{id}/events`.
- Full validation:
  - `uv run pytest -q`
  - frontend unit tests
  - frontend build

## Assumptions and Defaults
- “Workspace live stream” means one app/operator transport, not one global durable event log.
- Resource-scoped durability is the durable contract:
  - run history by run sequence,
  - conversation history by conversation revision,
  - overview resources by targeted snapshot plus deltas.
- The implementation should favor deleting old browser/operator stream paths over preserving compatibility shims.
- If a temporary shim is unavoidable, it must be deprecated, unreferenced by frontend/CLI, and covered only by migration/deprecation tests.
- Remote worker SSE stays unchanged.
