## Run Reporting Redesign

### Summary
- Replace the current separate `Run Activity` and `Event Timeline` surfaces with one durable `Run Journal` browser in Runs.
- Keep the run summary at the top. Keep Runs as the canonical monitoring surface. Keep `Run Graph`, `Context`, `Artifacts`, and `Checkpoint` user-visible, but move them under one top-level `Advanced` disclosure that is always collapsed by default.
- Show pending human-gate questions in a pinned actionable panel near the top of the inspector. Their audit history stays in the journal.
- Redesign the data path so long runs are browsed from durable backend history, not replayed into one giant hot Zustand array.

### Implementation Changes
- Backend:
  - Add `GET /attractor/pipelines/{id}/journal` as the primary history-read API.
  - Query contract: `limit` with newest-first ordering, plus optional `before_sequence` for explicit `Load older`.
  - Keep `GET /attractor/pipelines/{id}` as the authoritative run-detail surface for status/outcome/progress.
  - Keep `GET /attractor/pipelines/{id}/events` for live tail only, with optional `after_sequence` so the client can bridge the race between initial journal load and stream subscription without full-history replay.
  - Preserve existing `checkpoint`, `context`, `artifacts`, and `questions` endpoints.
  - Treat logs as durable journal entries of kind `log`. Keep `run.log` as exact-text artifact/download, not as the primary in-app reporting model.
  - Reuse existing durable event persistence (`events.jsonl`) as the storage base in v1; do not require a storage migration.

- Frontend:
  - Replace the current activity/timeline pair with a `RunJournalBrowser` that shows the newest journal slice first and supports explicit `Load older`.
  - Fold the “what is happening now?” facts into the top summary/overview instead of keeping a separate `Run Activity` card.
  - Keep pending questions in a pinned panel above the journal. Answered/timeout/default provenance remains visible in journal history.
  - Add one `Advanced` disclosure below the journal. Inside it, keep the existing detail cards for graph, context, artifacts, and checkpoint.
  - Introduce a dedicated run-journal store keyed by run id, backed by paged segments and sequence cursors, outside the hot Zustand session state.
  - Keep only session state in Zustand: selected run, filters, loaded range metadata, advanced open/closed state, selected artifact/context item, pending question draft state, and similar operator state.
  - Use a virtualized/windowed list for the journal so rendered DOM stays bounded regardless of loaded history.
  - Keep Runs canonical. Do not broaden Execution into a second full monitoring surface.

### Public Interfaces And Contract Changes
- Add `RunJournalEntry` as a backend/frontend contract type with these stable fields:
  - `id`, `sequence`, `emitted_at`, `kind`, `raw_type`, `severity`, `summary`, `node_id`, `stage_index`, `source_scope`, `source_parent_node_id`, `source_flow_name`, `question_id`, `payload`
- Add `RunJournalPageResponse`:
  - `pipeline_id`, `entries`, `oldest_sequence`, `newest_sequence`, `has_older`
- Revise `/pipelines/{id}/events` semantics so the frontend no longer depends on “replay full durable history, then tail live” as the main history model.
- Update `specs/spark-ui-ux.md`:
  - Replace separate `Run Activity` and `Event Timeline` requirements with one durable `Run Journal` browser.
  - Keep summary as the primary monitoring entry point.
  - Keep Runs as the canonical monitoring surface.
  - Replace the current “cap timeline to 200 items” performance contract with a bounded hot-state + virtualized browsing contract.
- Update the Attractor API spec to document the new journal endpoint and revised SSE usage.

### Test Plan
- Backend API tests:
  - journal returns newest-first durable history
  - `before_sequence` paginates older history correctly
  - log entries are present in journal history
  - child-flow metadata and human-gate provenance survive in journal pages
  - `/events?after_sequence=` only delivers gap-fill/live-tail entries after the requested sequence
- Frontend unit/integration tests:
  - summary remains topmost and reflects authoritative selected-run detail state
  - pinned pending-questions panel renders and submits without leaving Runs
  - unified journal renders mixed event/log history correctly
  - `Load older` appends older slices without duplicates
  - advanced section is collapsed by default and reveals graph/context/artifacts/checkpoint
  - journal list remains virtualized/bounded under sustained live updates
- Contract/spec tests:
  - rewrite the current sustained-SSE contract away from “trim to 200” and toward bounded client state plus durable browsing
  - rewrite human-gate discoverability contracts to target the pinned questions panel + journal audit history
  - update runs observability smoke coverage for the new journal-driven inspector
- Verification:
  - run the repo-standard suite with `uv run pytest -q`
  - run frontend unit tests
  - run Playwright smoke checks for Runs

### Assumptions And Defaults
- History browsing is newest-first with explicit `Load older`, not infinite scroll or page-number navigation.
- Logs are part of the unified journal model in-app. Exact raw-text access is via artifact/download, not a dedicated primary log panel in v1.
- The advanced evidence area is always collapsed by default.
- Filters apply to loaded journal history in v1; loading older expands the filterable window.
- No client-side persistence across full browser reload is required beyond backend rehydration from authoritative APIs.

