# CR-2026-0117 result

Status: implementation and review fixes are delivered and validated in the worktree. The overall delivery contract is incomplete: required intermediate commits and validation evidence tied to the final committed state require the authorized parent delivery stage.

## Shipped implementation

1. **Shared model and persistence.** Core runtime, server/client connections, grouped models, provider connections, and agent settings use common types and `config/spark.toml`. Dedicated profile, project, conversation, flow-policy and trigger documents retain their ownership and formats. Scoped writes validate before atomic replacement under cross-process document locks, require opaque revisions, preserve unrelated sections and reject stale edits. Profile reference writers and deletion share the existing workspace reference lock. Core, Desktop/defaults, conversation and browser-layout migrations are versioned, idempotent and backed up.

2. **Runtime integration and captures.** Desktop/server/CLI and native/built-in agent consumers use the shared settings. Provider endpoints, OpenAI identifiers, OpenRouter attribution and credential environment references are configurable. Credential status is public; values remain in transient adapter construction data. Profile credential references resolve against the original environment independently of provider aliases. Agent controls cover binary/runtime/config/seed paths, existing permission/service policies, turn/tool-round limits, command timeouts, output character/line limits, loop detection, subagent depth, environment inheritance and diagnostics. Native home changes retain running values until restart. Model discovery uses the same native configuration and keys its operational cache by those choices.

   Messages and runs capture provider/session/native configuration, startup runtime paths and config location, all LLM-profile definitions, and their resolved model/execution-profile choices. Actual HTTP clients, native launchers and tool sessions consume these captures. Agent CLI requests capture current settings before execution. New work rereads validated saved or file-edited settings; active work, continuations and historical snapshots keep their captures. Existing explicit launch/flow/node precedence, environment/CLI overrides, OAuth ownership, service/permission policies and Desktop remote-access confirmation are preserved.

3. **Inheritance.** Workspace → project → conversation resolution treats provider/profile, model and reasoning as one group. Null clears an override; omission leaves it unchanged. Legacy conversations migrate to inheritance without rewriting transcripts, historical turn metadata, run snapshots or chat/plan mode. Frontend inherited values are not written back as overrides.

4. **API and editor.** Scoped API and CLI reads/validate/set operations cover the implemented sections and existing dedicated document adapters. Provider and agent editors join model, runtime, connection, profile, project and client-preference controls with explicit Save/Discard, validation, pending protection, dirty-navigation protection and retained drafts on conflicts. Reads distinguish stored/effective values, source, credential status and restart fields. The existing live transport invalidates settings in other clients.

5. **Client preferences and cleanup.** Per-client documents own editor mode, widths/splits, advanced/child-flow/graph choices, scopes, run sort/activity/inspector/filter/height choices, and user node positions/edge port choices. Contextual interactions persist on completion; initial graph hydration does not save generated positions as preferences. Accessible legacy layout records are backed up before a revision-checked import; authoritative choices win and a server-side version marker prevents reimport. Original browser records are removed only after success. Superseded layout writers no longer recreate them. Topology stamps, computed routes, viewport restoration, selections, drafts and operational caches retain their separate roles. Desktop uses its app-owned persistent identity across ports; browser identities remain separate.

Documentation: `docs/configuration.md`.

## Review fixes

- Workflow codergen now always supplies the stage trace-path metadata to the native launcher. The launcher remains the single gate using the captured native tracing choice, with the existing environment fallback for uncaptured requests. This fixes saved `agents.native.codex_jsonrpc_trace=true` when `SPARK_DEBUG_CODEX_JSONRPC` is unset without enabling default diagnostics.
- The workflow-to-native regression reads persisted configuration through `read_execution_configuration`, executes the real `CodergenHandler`/Rust backend against the fake Codex app server, and checks actual JSONL artifacts. It covers default-disabled diagnostics, saved tracing without the environment flag, both environment override directions, and enabled/disabled captures surviving later settings and environment changes.

- Reference protection includes the synthesized `native` execution profile while `execution-profiles.toml` is absent. A real HTTP regression registers a project referencing native, rejects a container-only replacement with the project reference identified, verifies the document stays absent, then completes a project workflow with the native snapshot.
- Settings edits, workflow launches, conversation resolution and agent-boundary default resolution reuse the model/provider/reasoning/profile validator in `spark-agent-adapter::config`, without new dependencies or a crate cycle. Invalid file-edited workspace and project groups fail before workflow persistence or agent execution. Existing explicit launch/flow/node precedence is retained.
- HTTP and agent-boundary regressions cover unsupported providers, provider/model mismatches, invalid reasoning, missing profiles and providers requiring explicit models. Valid inheritance, explicit agent selection and existing captures remain covered.

## Final UI review fixes

- Workspace mutations now use the existing navigation-confirmation event before changing the active project or leaving Settings, and persist the route only after acceptance. Switching, clearing, removing, renaming and registry hydration retain the mounted project-model draft when cancelled. Failed project registration also uses this shared transition path. One confirmation applies queued project-and-view transitions together; cancellation applies neither.
- The project execution dialog tracks its saved selection and revision, confirms Escape/outside/Cancel dismissal, and provides Discard and reload. Live settings notifications and window focus refetch clean editors; dirty editors retain their selection and original revision and report external changes. Conflicts retain drafts. Pending saves block dismissal, navigation and duplicate submissions.
- Trigger/editor tab switches retain their existing session-preservation behavior; they do not gain a redundant discard prompt.
- Settings now links to the existing flow-policy and trigger editing surfaces through the guarded view navigation.
- Nine component regressions cover cancelled/confirmed project transitions, dirty execution dismissal, live updates, conflicts, reload and pending protection. Four additional real-server browser scenarios cover project switch/clear cancellation and confirmation, clean/dirty execution updates with Escape/outside dismissal, and scoped-editor links.

## Conversation revision review fix

- Conversation settings updates now require `expected_revision` for `chat_mode` as well as grouped and legacy model selectors. Missing revisions return HTTP 400 and stale revisions return HTTP 409.
- The storage commit lock rejects stale mode/model settings batches before writing transcript, mode-history or journal records. Operational event mutations still rebase.
- The slash-command caller sends its cached conversation revision, and the frontend settings API type requires the revision. The same-mode Rust caller and HTTP contract were updated; configuration documentation now describes the enforced contract.
- Regressions cover missing/stale HTTP revisions, unchanged snapshots after rejection, competing storage writers (one success/one conflict), byte-for-byte preservation of metadata/transcript/events on rejection, preserved historical mode records, and continued operational rebasing. The project-panel test verifies the slash-command revision payload.

## Validation

Previous checks passed after the UI review fixes (the conversation-revision rerun is recorded below):

- `just test`: Rust formatting, all-features workspace tests and doctests, **567 frontend tests across 74 files**, TypeScript, and the production frontend build. This includes the affected storage/migration, HTTP, inheritance, profile/reference, execution-snapshot, agent, CLI and Desktop contracts.
- `npm --prefix frontend run lint`: zero errors; 15 existing warnings.
- `PLAYWRIGHT_BROWSERS_PATH=/Users/chris/Library/Caches/ms-playwright npm --prefix frontend run ui:smoke -- e2e/smoke/settings-editor.spec.ts`: **12 passed**, including real-server settings, migration, port-changing restart, project transitions, execution-dialog live changes/dismissal, and scoped-editor navigation.
- `cargo build --release -p spark-desktop --bin spark-desktop --all-features`: passed with production frontend assets.
- `git diff --check`: passed.

The frontend build retains its existing bundle-size warning; browser tooling retains its Node deprecation warning. The restart browser fixture simulates the native identity response; Rust Desktop contracts verify actual native identity persistence.

## Conversation revision validation rerun

All checks passed on the final implementation working tree after the conversation revision fix:

- `cargo test -p spark-storage -p spark-http -p spark-workspace conversation --all-features` (including competing mode writers, revision rejection, history preservation, and operational rebasing).
- `npm --prefix frontend run test:unit -- src/features/projects/__tests__/ProjectsPanel.test.tsx`: 39 passed.
- `just test`: formatting, all-features workspace tests/doctests, 567 frontend tests across 74 files, TypeScript, production frontend build.
- `npm --prefix frontend run lint`: zero errors, 15 existing warnings.
- `PLAYWRIGHT_BROWSERS_PATH=/Users/chris/Library/Caches/ms-playwright npm --prefix frontend run ui:smoke -- e2e/smoke/settings-editor.spec.ts`: 12 passed using rebuilt frontend assets and the updated server.
- `cargo build --release -p spark-desktop --bin spark-desktop --all-features`: passed with rebuilt production frontend assets.
- `git diff --check`: passed.

Local command logs: `/tmp/cr0117-conversation.log`, `/tmp/cr0117-frontend.log`, `/tmp/cr0117-just-test.log`, `/tmp/cr0117-lint.log`, `/tmp/cr0117-e2e.log`, `/tmp/cr0117-build.log`, `/tmp/cr0117-desktop.log`. These establish working-tree validation, not validation of a final commit that does not yet exist.

## Delivery

The matching frontend/backend migration remains together in this worktree. This stage's instruction says: “Do not commit, push, create branches or worktrees, or clean anything up; the parent flow evaluates, validates, commits, and releases the workspace.” No such actions were performed.

The change request's intermediate-commit requirement is still outstanding; this report does not waive it or claim it has been satisfied. The authorized parent stage must create the required intermediate implementation commits for the five phases and deliver the completed migration with its matching frontend/backend together before release. Include all new/untracked implementation and test files, as well as this result document, in those commits. HEAD was verified as `9839f9f0` during this review pass; repository history has not yet satisfied this requirement.

## Delivery review handoff

The delivery-only review was inspected without changing implementation or tests. HEAD remains `9839f9f0a2eeed5d5352ab7045b8bcf6564b19f8`; no implementation commit IDs exist to record. `git status --porcelain=v1 -uall` reports 199 changed/new files (the review's 184 entries collapse untracked directories). Existing validation logs confirm the recorded frontend build, lint, 12 browser scenarios and production Desktop build. Both unstaged and staged `git diff --check` pass. No affected checks need rerunning for this report-only update.

For the authorized delivery stage, the current content fingerprint is `e8e248662af262c445e454b08affe3af12da50152059053a13d49da1b710209b` across 860 files. Compute it from the sorted, deduplicated paths returned by `git ls-files -z --cached --others --exclude-standard`, excluding this result document and non-files: SHA-256 of the concatenation of each path's bytes, NUL, the lowercase SHA-256 hex digest of its file bytes, and newline. This records current worktree contents for delivery comparison; it is not proof of committed-tree equivalence or a retrospective fingerprint of the earlier test runs.

Remaining authorized parent delivery actions:

1. Create intermediate commits for the five implementation phases in section 4, including every new implementation and test file. Deliver the matching frontend/backend migration together.
2. Verify the final committed tree matches the validated worktree, including file modes and all new files; rerun affected checks if implementation contents change.
3. Record the five phase commit IDs and final verification evidence here, then mark delivery complete. This stage cannot truthfully mark those actions complete because its explicit no-commit instruction reserves them for the parent flow.

## Gatekeeper delivery routing

Inspected `/tmp/cr0117-gate-test.log`, `/tmp/cr0117-gate-lint.log`, `/tmp/cr0117-gate-e2e.log`, and `/tmp/cr0117-gate-desktop.log`: Rust tests/doctests, 567 frontend tests, TypeScript and production frontend build passed; lint reports zero errors and 15 warnings; all 12 browser scenarios passed; production Desktop build completed. Both staged and unstaged whitespace checks passed again. Implementation contents retain the 860-file fingerprint above; only this report changed, so affected implementation checks were not rerun.

The current content-and-Git-mode fingerprint is `b8b1d9f9a5a17b19411180bc6659d4246f81019996eaf1977e45a14cf725ba1a`, using the same sorted paths and report exclusion above. Prefix each content record with its Git mode and a space (`100755` for executable regular files, `100644` otherwise, `120000` for symlinks; hash symlink target bytes). The delivery stage must compare committed blob contents and modes against this worktree baseline, and separately verify the final report is committed exactly. This is a handoff baseline, not a claim that final commit verification has occurred.

Route to the authorized delivery stage. All five phase commit IDs remain pending: shared model/persistence, runtime integration, inheritance, API/editor, and client preferences/cleanup. The parent must create those commits with all new implementation/test files, deliver matching frontend/backend migration together, and record their actual IDs and final tree verification here. No implementation deletion or simplification is required by this review.

Latest delivery-only review: inspected `/tmp/cr0117-current-gate-test.log`, `/tmp/cr0117-current-gate-lint.log`, `/tmp/cr0117-current-gate-e2e.log`, and `/tmp/cr0117-current-gate-desktop-final.log`. The fresh gate evidence confirms successful validation, zero lint errors (15 warnings), 12 passing browser scenarios, and a completed production Desktop build. Recomputed the content-and-Git-mode fingerprint: it remains `b8b1d9f9a5a17b19411180bc6659d4246f81019996eaf1977e45a14cf725ba1a` over 860 files excluding this report. HEAD and the 199 changed/new-file count remain unchanged. Staged and unstaged whitespace checks pass. Only this report was updated; no implementation changes require affected checks to rerun.

Delivery remains incomplete pending the authorized parent's commits and committed-tree verification. Route this blocker directly to that parent, not back to an implementation stage prohibited from committing. Mark this report complete only after recording actual phase commit IDs and verifying committed contents and modes against the validated worktree.
