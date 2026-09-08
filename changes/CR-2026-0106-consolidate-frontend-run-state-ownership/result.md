# CR-2026-0106 result

Implemented inspected-run ownership in the existing memory-only Zustand `runDetailSessionsByRunId` map. Scope selection is derived exclusively from `selectedRunIdByScopeKey`; the obsolete inspector slice and foreground copies are removed.

## Review stages

1. **Characterization and selectors:** added stable selected-ID/session selectors and focused reconciliation, scope isolation, resource, stream, launch, and graph race checks. Updated existing tests to use run sessions and selectors.
2. **Record and selection migration:** renamed `summaryRecord` to `record`; centralized status replacement, summary seeding, telemetry enrichment, live/journal patches, and optimistic cancel/retry rollback. Migrated routing, notifications, chat links, launches, continuations, and project transitions to explicit scope keys. Launch acknowledgements do not fabricate records.
3. **Node, gate, and graph migration:** moved live overlays, graph metadata/diagnostics, and inspector wiring into each session. Canvas context identifies its displayed run. Resource request identities, graph cancellation/lifetime checks, and stream cleanup reject obsolete callbacks. Cached graphs survive failed refreshes; variant changes hide incompatible cached graphs. Cached questions remain visible but require fresh ID confirmation in both buttons and submission handlers.
4. **Removal and integration:** deleted foreground snapshot synchronization, equality helpers, the inspector slice, and unused pruning/list mutation interfaces. Added terminal recovery, same-run fetch stability, removal/recreation, and pending-action rollback regression coverage. Updated the observability smoke test to the existing inspector tabs and activity UI, replacing obsolete Advanced evidence/graph-collapse selectors; checkpoint coverage verifies the current resume-node display and refresh on revisit.

The stages above describe implementation areas, not delivered commit boundaries. The suite results below validate working-tree revisions; they do not establish affected-suite results for four independently reviewable commits. No commits were created, per the task instructions.

## Review corrections

Terminal reconciliation now re-arms on nonterminal live statuses for each execution attempt. Runtime events and run upserts share the same coalescing path, independent of record-listener order; a terminal event during recovery schedules one follow-up durable refresh. Delayed status snapshots cannot overwrite newer live record changes.

On revisit, successful status hydration removes cached node/gate overlays that were not updated in the new stream lifetime. Cached overlays remain visible during loading and failure, and newer live nodes and gates survive delayed hydration. Regression coverage uses the real Zustand store, RunStream, HTTP status validator, and node-status model, including A → B → A completion while unselected and same-selection retries through both terminal event sources.

Live list updates now identify their source explicitly. A single-row upsert reconciles only its matching session, without replaying cached summaries for other runs through fresh-list reconciliation. The actual `useRunsList` event-handler regression first reproduced A's telemetry falling from 100 to 5 after B updated; it now verifies A retains telemetry 100, session and record identity, and zero resource-subscriber rerenders. It also verifies fresh list telemetry still updates A and preserves omitted-versus-null semantics for both sources.

The rendered inspector now keeps cached context rows, artifact entries, and ready result bodies alongside refresh/loading and error feedback. Parameterized `RunsPanel` regressions cover each resource through selection revisit, a pending refresh, and HTTP refresh failure; all three reproduced the previous rendering failure before the fix.

Status requests capture source-specific field update identities and pass them to the existing state reconciliation action. Successful hydration always records completed nodes and the fetch timestamp, fills durable detail fields, and preserves live/journal/action fields updated during the request. Intervening list summaries do not gain authority over the status response. This replaces the blanket live-revision rejection without adding fetch loops. The validated HTTP regression starts from a summary, delivers `StageStarted` and live telemetry before status resolves, and verifies durable hydration, newer node/telemetry values, and no fetch restart on ordinary session updates. Existing recovery and terminal refresh regressions remain passing.

## Earlier stage validation

- `just test`: passed on full retry, including Rust formatting, workspace tests with all features, all 420 frontend unit tests, and TypeScript/production build. The first attempt stopped in the unchanged Rust HTTP conversation-turn contract with a transcript revision mismatch; an isolated retry timed out waiting for the completed snapshot. The full-gate retry passed that HTTP suite and the long provider-event persistence regression. The frontend unit suite and build also passed independently.
- Review-fix reconciliation/selector/resource/list/panel suites: all 46 tests passed.
- Affected stream/node/inspector and session suites: 120 tests passed; the HTTP-validator revisit regression also passed after its final refinement.
- Targeted ESLint on changed frontend TypeScript files: no errors; five existing hook-dependency warnings. Retained the pre-existing Navbar attention import with a narrowly documented lint exception to avoid the explicitly excluded service cleanup.
- `PLAYWRIGHT_BROWSERS_PATH=/Users/chris/Library/Caches/ms-playwright npm --prefix frontend run ui:smoke -- e2e/smoke/runs-observability.spec.ts`: all six passed. The explicit cache path uses the installed browser because the agent runtime has a different home directory.
- `git diff --check`: passed. Source search confirms no foreground selected record/runtime fields, `summaryRecord`, or inspector slice remain.

Existing dependencies, stores, backend contracts, hashes, validation, journal/transcript stores, and layout worker remain in place. Sessions are not persisted. No event-bus redesign, lazy loading, catalog deduplication, save-toast relocation, backend cleanup, feature flags, or monitoring infrastructure was added.

## Latest review validation

- Cached-resource fix: all 40 affected inspector/resource tests passed after the fix.
- Status-hydration fix: all 45 affected stream, inspector, and reconciliation tests passed after the fix, including recovery and terminal convergence.
- Full frontend suite: all 424 tests passed; TypeScript and production build passed independently.
- Targeted ESLint on every changed frontend TypeScript file: zero errors, five existing hook-dependency warnings.
- Observability smoke suite with the installed Playwright browser cache: all six tests passed.
- `git diff --check`: passed.
- `just test`: passed on full retry, including Rust formatting, all workspace suites, all 424 frontend tests, and TypeScript/production build. The first attempt stopped in the unchanged Rust HTTP conversation-turn contract waiting for its expected snapshot; that contract and the long 50,001-event persistence regression passed in the full retry.


## List/status race correction

Replaced blanket record comparison with per-field event/action update identities in the existing run session. Status hydration replaces summary fields while applying only live, journal, or optimistic/rollback updates received after that request began. Retaining the source update values also protects those updates when a list summary arrives between the event/action and status response. Updates predating the request do not override fresh status. No store, dependency, or backend changes were added.

Two regressions mount RunStream and useRunsList with real validated HTTP responses. They cover a null initial record and an existing summary refreshed while status is pending, asserting authoritative completed status/details, completed node rendering, and identity agreement between the list's displayed record and inspector session. Additional state regressions cover each authoritative update source through an intervening list response, rollback during hydration, and subsequent fresh status replacement.

Validation for this correction:

- Affected run and session suites: 107 tests passed; the final focused race/rollback rerun passed all 21 tests.
- Targeted ESLint on all changed frontend TypeScript files: zero errors, five existing hook-dependency warnings; final state-layer rerun passed.
- Runs-observability smoke using the installed Playwright browser cache: all six passed.
- `just test`: passed on this run, including Rust formatting, workspace tests, all 430 frontend tests, and TypeScript/production build. The long provider-event persistence contract passed. The frontend build also passed independently.
- `git diff --check`: passed; source search confirms the removed foreground record/runtime fields and inspector slice remain absent.

## Latest source-write identity correction

Every intended live, journal, and action field write now receives a fresh tracking identity and retains its value even when it equals the current list summary. Runtime journal handling passes only runtime fields, omitting an absent/null error as before; the full-record runtime merge helper was removed. Optimistic apply and rollback likewise track only the fields they actually patch, preserving conditional rollback without granting copied details authority.

Added validated delayed-HTTP regressions through RunStream and useRunsList for completed → list running → running events from both live and journal sources. They assert final running status, fresh durable details, and list/inspector record identity. State regressions cover the exact first-fetch captured-identity sequence for both sources and verify rollback leaves intervening field identities intact while allowing durable detail hydration.

- Affected reconciliation, stream, list, session-race, and inspector suites: all 65 tests passed.
- Targeted ESLint on all changed frontend TypeScript files: zero errors, five existing hook-dependency warnings.
- Runs-observability smoke with the installed Playwright browser cache: all six tests passed.
- `just test`: passed, including Rust formatting, workspace tests, all 435 frontend unit tests, and TypeScript/production build. The long provider-event persistence regression also passed.
- `git diff --check`: passed; removed foreground record names and inspector slice remain absent from frontend source.

## Workspace transport callback correction

Workspace message callbacks now reject closed effect lifetimes, obsolete EventSource connections, and removed/replaced selected-session lifetimes before parsing envelopes, updating cursors, or dispatching events. Connection errors invalidate the connection immediately; captured obsolete error callbacks cannot close its replacement or schedule another reconnect. The existing effect and session lifecycle machinery remains in use.

Added regressions mounting WorkspaceLiveEventsController with RunStream and the real run, journal, and transcript stores. They capture A's original callback, revisit A through B, remove/recreate A, and replace the connection after an error. Obsolete runtime, question, transcript, and recovery messages leave session records/nodes/gates, journal/transcript state, and recovery request counts unchanged. Reconnecting verifies cursor preservation; current callbacks still update the record, node, gate, journal, and transcript and request recovery. The regressions reproduced the stale record/node/gate writes before the fix.

Validation: all 48 affected AppShell, stream, and race tests passed; the final focused regression rerun passed all 14 tests. Targeted ESLint across changed frontend TypeScript files reported zero errors and five existing warnings. All six runs-observability smoke tests passed using the installed Playwright browser cache.

The full frontend suite passed all 438 tests; TypeScript and the production build passed independently. The final changed-test ESLint check and `git diff --check` passed. The first two `just test` attempts stopped in the unchanged Rust `conversation_turn_route_uses_rust_llm_client_backend_for_openai_compatible_profile` contract: first a pending assistant snapshot, then a transcript revision mismatch (expected 6, got 5).

The third `just test` attempt passed completely: Rust formatting, all workspace tests (including the previously intermittent conversation-turn contract and long provider-event persistence regression), all 438 frontend tests, and TypeScript/production build. No backend changes or test exclusions were made.

## Selected-inspector subscription boundary

Moved the list hook into a local `RunsSidebar` component. `RunsPanel` now subscribes only to list metadata, whether rows exist, and its selected summary, in addition to the existing selected-run session subscriptions. Cached updates to another listed run and its live list snapshot no longer render the selected inspector or graph. No safety guards or services changed.

An integrated `RunsPanel` regression uses the real store, sidebar, inspector, and graph with render spies. With A selected and A/B cached and listed, it verifies B’s row updates without inspector/graph renders, A’s updates render, and A → B → A selection changes render the right run and preserve A’s inspector tab. Restoring the broad list subscription reproduced the inspector-render failure; the isolated boundary passes.

Validation: all 195 affected tests passed, targeted ESLint across changed frontend TypeScript files reported zero errors and five existing warnings, and all six runs-observability smoke tests passed with the installed browser cache. Final repository-gate result follows below.

Delivery limitation: the required four staged commits have **not** been delivered. The current-stage instruction explicitly prohibits commits and reserves committing for the parent flow. Earlier prose stage descriptions and suite runs are not a substitute for independently validated staged commits. The implementation remains an uncommitted diff for the parent flow; this delivery requirement remains outstanding.

Final validation for the subscription correction: `just test` passed on full retry, including Rust formatting, all workspace tests, all 439 frontend unit tests, and TypeScript/production build. The first attempt stopped in the previously intermittent Rust conversation-turn contract while waiting for its expected snapshot; that contract and the long provider-event persistence regression passed in the retry. The complete frontend suite and build also passed independently. Final changed-file ESLint and `git diff --check` passed. No backend modifications or test exclusions were made.

## Staged-delivery handoff (2026-09-07)

The delivery review remains unresolved. HEAD is `3ec1f16c5a1b074deb40bac74a89b6396c00de76`; the implementation is still uncommitted. This node is explicitly prohibited from committing, creating branches/worktrees, or cleaning up. No implementation source was changed during this handoff.

The commit-authorized parent must deliver these independently reviewable commits, preserving the validated final implementation, and record each commit ID with its actual affected-suite command and result:

1. Characterization tests and selectors.
2. Record and selection migration.
3. Node, gate, and graph migration.
4. Removal and integration checks, including this result document.

Fresh validation of the current **working tree**, not staged commits: `just test` passed on the first attempt (Rust formatting, workspace tests, full frontend unit suite, and TypeScript/production build); targeted ESLint across changed and untracked frontend TypeScript files passed with zero errors and five warnings; all six runs-observability smoke tests passed with `PLAYWRIGHT_BROWSERS_PATH=/Users/chris/Library/Caches/ms-playwright`. Logs are `/tmp/cr-0106-parent-handoff-just-test.log`, `/tmp/cr-0106-parent-handoff-eslint.log`, and `/tmp/cr-0106-parent-handoff-smoke.log`.

After staged delivery, the parent must rerun `just test`, targeted ESLint, and `npm --prefix frontend run ui:smoke -- e2e/smoke/runs-observability.spec.ts` against the final committed tree. The working-tree results above do not satisfy that final committed-tree requirement or replace validation at each stage.

## Mandatory review routing: fresh verification

Preserved the implementation and reran validation at HEAD `3ec1f16c5a1b074deb40bac74a89b6396c00de76` with the existing uncommitted implementation. Only this result document was edited during this attempt.

- `just test`: exit 0 on the first attempt; Rust formatting and workspace tests passed, all 439 frontend tests in 58 files passed, and TypeScript/production build passed. The previously reported `conversation_turn_route_executes_injected_backend_and_preserves_validation` failure did not recur. The long provider-event persistence test also passed.
- Changed-file ESLint: exit 0, zero errors and five warnings. Invoked `./node_modules/.bin/eslint` from `frontend` with all existing modified/untracked `.ts` and `.tsx` files returned by `git ls-files --modified --others --exclude-standard -z`. The initial invocation included the deleted inspector slice and failed file discovery; excluding deleted paths allowed the complete applicable file set to run. Exact file arguments and output: `/tmp/cr-0106-routing-eslint.log`.
- `PLAYWRIGHT_BROWSERS_PATH=/Users/chris/Library/Caches/ms-playwright npm --prefix frontend run ui:smoke -- e2e/smoke/runs-observability.spec.ts`: exit 0, all six tests passed. Output: `/tmp/cr-0106-routing-smoke.log`.
- Inspected the reported Rust error boundary: `ActivityRepository::append_transcript` rejects a revision that does not follow the durable transcript tail; `commit_conversation` allocates revisions before individual appends. This inspection does not establish the intermittent failure's cause. No Rust code, validation, security protections, or test exclusions changed.

**Route to the commit-authorized parent.** Four independently reviewable stages, actual stage commit IDs, and affected-suite commands/results remain outstanding. This node cannot deliver commits under its explicit no-commit instruction. These fresh working-tree passes are not a substitute for staged delivery or rerunning the required gates against the final delivered tree. Do not route this unchanged delivery gap back to another commit-prohibited implementation attempt.
