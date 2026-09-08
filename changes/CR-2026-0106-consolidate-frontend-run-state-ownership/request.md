# Consolidate frontend run-state ownership

## Summary

Make the existing per-run session map the sole owner of inspected run data. Selection remains an ID stored per project/scope; components derive the selected session instead of restoring and synchronizing foreground copies.

Include status, node state, human gates, and graph metadata. Deliver in staged commits within one refactor, preserving existing navigation and streaming behavior.

Returning to a run shows cached details immediately while refreshing. Cached human questions remain non-actionable until confirmed.

## Ownership and interfaces

- Keep the existing Zustand store and `runDetailSessionsByRunId`. Do not introduce another store, repository layer, or dependency.
- Rename the session’s `summaryRecord` to `record`, since it holds the inspected run’s authoritative record.
- Move status synchronization/error state, live node statuses, human gate, graph attributes, and diagnostics into that run’s session. Keep existing completed-node snapshots, fetched timestamp, resources, and inspector preferences there.
- Remove the separate runtime status/outcome fields; use the run record’s fields. Do not manufacture a complete record from a launch acknowledgement or partial event.
- Keep `selectedRunIdByScopeKey` as the only stored selection. Add selectors for the effective selected ID and session; use stable empty defaults.
- Replace implicit “update the selected run” actions with actions taking an explicit `runId`. Selection actions take an explicit scope key. Async callbacks never discover their destination from the current selection.
- Remove the foreground run fields and their setters, the bidirectional synchronization effects, and record-equality helpers used only to maintain those copies.

These are internal TypeScript changes. Backend APIs, event formats, and URL hashes remain unchanged.

## Implementation

### 1. Establish one path for record updates

Keep the list response as a separate summary snapshot because it has different completeness and freshness from run details. Centralize reconciliation in the runs state layer:

| Source | Update rule |
|---|---|
| Status response | Replace the inspected record and completed-node snapshot; record the fetch time. |
| List response before a status fetch | Seed or refresh the cached record from the summary. |
| List response after a status fetch | Update available token/cost telemetry; preserve authoritative status and detail fields. |
| Live run upsert | Apply the existing supported status, outcome, completion, error, and telemetry fields to the matching cached record. |
| Journal runtime/node event | Patch the matching run’s record and live node/gate state. |
| Cancel/retry action | Apply the optimistic change to the list and matching cached record together. Roll back only the operation’s unchanged optimistic fields on failure. |

Use selectors to display cached authoritative details over list summaries where available, so the list row and inspector agree. Rendering must not write merged records back through effects.

Preserve omitted-versus-null behavior in the existing source-specific merges. Keep event ordering, retry-stage handling, and terminal reconciliation rules.

### 2. Migrate selection and synchronization

Replace the synchronization machinery in `AppSessionControllers.tsx` with selection-derived wiring for the existing list and detail synchronization hooks.

Update every selection entry point: list clicks, hash routing, chat links, notifications, launches, and continuations.

- Preserve independent remembered selections for each project and the all-projects scope.
- An explicitly empty selection stays empty; remove the fallback to a global selected ID.
- Project switching changes scope without copying or clearing another run’s data.
- A launch response updates the scope captured for that launch, so switching projects while launching cannot select the new run in the wrong project.
- A selected-run 404 clears that run’s cached detail and references to its selection, without clearing a different run selected meanwhile.
- Project removal prunes its known run sessions and selection references. Late requests cannot recreate removed sessions.

Keep synchronization limited to the currently inspected run, as today. Scope each stream lifetime’s cursors, recovery guard, and callbacks to its captured run ID. Cleanup invalidates obsolete callbacks, including an A → B → A sequence.

Preserve coalesced recovery and terminal durable refreshes. Their correctness must not depend on which event listener updates the record first.

### 3. Scope graph and cached interactions

Store graph metadata and diagnostics alongside the nodes and edges for that run. Extend the existing canvas context to identify the displayed run, so shared nodes and edges read that session rather than foreground globals. Leave the broader canvas/editor component redesign for later.

On revisiting a run:

- Render its cached record, graph, resources, and inspector preferences immediately.
- Refresh through the existing loaders; keep cached content visible on refresh failure with existing error feedback.
- Preserve stale-response guards and the committed artifact-preview request identity.
- Keep cached questions visible but disable answering until a fresh questions response confirms their IDs. Enforce this in the submit handler as well as the buttons; a failed refresh leaves them disabled.
- Reset live overlays when rebuilding after a stream gap; derive the displayed node state from durable data using the existing node-status model.

Retain graph cancellation and layout guards. Cache reuse must respect the child-expansion setting; never display one graph variant as another.

### 4. Remove transitional code

Delete foreground snapshot preservation/restoration from project transitions and remove the obsolete run-inspector slice once its callers are migrated.

Use run-specific subscriptions so updates to unrelated cached runs do not rerender the selected inspector. Keep the journal and transcript stores, layout worker, and existing API validation.

Leave event-bus redesign, lazy resource loading, catalog-fetch deduplication, save-toast relocation, and unrelated service cleanup outside this effort.

## Tests and acceptance

Add focused tests before migration, then update existing tests to assert behavior through selectors rather than removed globals.

Cover:

- Project A → B → A and active → all → active restore independent selections and preferences without leaking records, nodes, gates, or diagnostics.
- Cached details remain visible during loading and failure; confirmed questions become actionable, stale ones do not.
- Delayed status, graph, resource, and error responses cannot affect another selection or resurrect removed sessions.
- A 404 cannot deselect a newer run; a delayed launch response cannot overwrite another project’s selection.
- Status responses, list telemetry, live upserts, terminal events, cancel, and retry reconcile correctly.
- Same-run updates do not restart detail fetches; recovery bursts remain bounded.
- Existing journal pagination/deduplication, deep links, graph expansion, and artifact-preview regression tests remain passing.

Run the affected suites during each stage, then the full frontend unit suite, TypeScript/build checks, targeted lint, and the existing runs-observability smoke test.

Completion requires no stored foreground run copies, no effects synchronizing equivalent snapshots, and explicit run IDs on asynchronous writes.

## Delivery and defaults

Use four reviewable stages: characterization tests and selectors; record/selection migration; node/gate/graph migration; removal and integration checks. Temporary compatibility code may exist between commits but must be gone from the final result.

Run sessions remain memory-only, so no persisted-data migration or backend deployment is required. Ship through the normal frontend build; no feature flag or new monitoring infrastructure.
