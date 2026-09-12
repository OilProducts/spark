**CR-2026-0117 — independent evaluation of the canceled workflow**

**Verdict: substantial implementation, but incomplete and not ready to release.** The work largely follows the requested architecture and includes real persistence, migration, inheritance, snapshot, API, and editor functionality. However, ordinary combinations of those features still violate the request. The workflow's completion claims are too strong; missing commits were not the only remaining problem.

I used the [original request](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/changes/CR-2026-0117-unify-spark-configuration-and-settings/request.md) as the contract, checked the recorded normalization/evaluation instructions, and evaluated the actual files left by `run-18d45752f4ec6770`. The worktree is based on `9839f9f0a2eeed5d5352ab7045b8bcf6564b19f8`, with 199 modified/new files and no implementation commits beyond that base. I did not modify the implementation, merge it, or restart the canceled workflow.

This report uses **Complete** for an implemented requirement supported by source inspection and applicable automated checks; **Partial** where part works but a specific gap remains; **Incomplete** for a missing deliverable; and **Unverified** where the necessary execution was not independently performed. “Complete” is scoped to the particular row, not a claim that every deployment combination has been exercised.

**The most consequential findings**

**F1 — Changing an inherited conversation's reasoning effort can discard its LLM profile. High priority; reproduced through the API using the frontend's exact payload shape.**

The contextual dropdown handler sends the effective provider, model, and effort as legacy scalar fields, without the effective `llm_profile`. The server sees a provider change from the profile-based group and constructs a provider-based override. Changing effort therefore also changes how the model connects and authenticates.

In the reproduction, the conversation initially inherited `{llm_profile: "audit-local", model: "audit-model"}`. Submitting the effort dropdown's payload produced a saved override with `provider: "openai_compatible"` and `llm_profile: null`. The profile's endpoint and credential selection are lost. Both operations returned success.

This fails “choosing an override starts from the currently effective group” and the spirit of grouped selection. Workspace/project group editors and server-side inheritance can be correct while this everyday conversation interaction is wrong. The conversation dropdown also lacks a profile selector corresponding to the full group editor.

Evidence: [frontend writer](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/frontend/src/features/projects/hooks/useProjectsHomeController.ts:430), [server group conversion](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/spark-workspace/src/conversations.rs:564), [API observations](/Users/chris/projects/spark/reports/CR-2026-0117-audit/api-results.txt). The smallest appropriate correction is to use the current complete group when creating/editing an override, preserving its profile unless the user actually changes the provider/profile selection.

**F2 — Desktop loses an environment-selected agent home. High priority; reproduced with the real Desktop bootstrap and capture functions.**

Desktop initializes common settings with an empty environment map. Later execution resolution reads the process environment correctly, but `retain_startup_settings` overwrites the resulting agent-home selection with Desktop's incorrectly captured startup value.

With `SPARK_CLAUDE_CODE_CONFIG_DIR=/tmp/audit-claude-config`, the probe printed:

```text
resolved_before_startup_retention=Some("/tmp/audit-claude-config")
captured_after_startup_retention=None
desktop_startup=None
```

This violates environment-over-persisted precedence and the requirement to retain the actual startup selection for restart-only settings. App-owned Desktop home selection is intentional; ignoring the environment for unrelated agent settings is not. The remedy should preserve the app-owned bootstrap home while resolving the supported environment overrides before capturing startup settings.

Evidence: [Desktop bootstrap](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/apps/spark-desktop/src/desktop_core.rs:214), [startup retention](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/spark-common/src/settings.rs:598), [probe source](/Users/chris/projects/spark/reports/CR-2026-0117-audit/desktop_precedence.rs), [failed assertion](/Users/chris/projects/spark/reports/CR-2026-0117-audit/desktop-precedence-results.txt).

**F3 — Editing a referenced profile can save an invalid workspace configuration. High priority; reproduced.**

A profile can retain its ID while removing the model selected by workspace defaults. Saving the profile returns HTTP 200, but the next `GET /workspace/api/settings` returns HTTP 400: `Select a model supported by the LLM profile.`

The implementation checks references when profile IDs are deleted, but not when an existing profile's contents make those references invalid. This undermines validation-before-write, compatible grouped selection, and the full editor: the aggregate settings read now fails, even though the profile save reported success. New work that validates the same default also cannot resolve it.

Validate affected references against candidate profile contents before writing, under the existing reference lock. Report which selections must be changed first, just as deletion already does.

Evidence: [profile update boundary](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/spark-workspace/src/profile_settings.rs:126), [aggregate settings validation](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/spark-workspace/src/settings.rs:73), [API observations](/Users/chris/projects/spark/reports/CR-2026-0117-audit/api-results.txt).

**F4 — Captured settings are not fully adapted to container execution. High priority for container users; code finding, not a live Docker reproduction.**

There are two concrete boundary problems:

- Capture resolves native agent binaries on the host. Container dispatch copies that captured context unchanged, and the agent uses the captured binary. A host-specific absolute macOS path will not name the Linux image's installed binary. Standard container mounts do not make that executable portable.
- Container environment forwarding uses a fixed list of conventional provider variable names. A supported credential reference such as `TEAM_OPENAI_KEY` is absent from that list, so a captured profile/provider referring to it cannot resolve the host credential inside a standard container.

These need execution-aware handling. Snapshotting configuration is valuable, but it cannot make host-specific paths valid in another execution environment. Forward the necessary configured credential references through the existing environment boundary without persisting their values. Confirm both paths with a real container run before marking execution consistency complete.

Evidence: [native binary capture](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/spark-agent-adapter/src/config.rs:20), [unchanged container context](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/attractor-execution/src/container.rs:242), [container mounts](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/attractor-execution/src/container.rs:334), [environment allowlist](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/attractor-execution/src/container.rs:432), [captured binary consumer](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/spark-agent-adapter/src/codex_app_server.rs:415).

**F5 — Navigation can offer “Discard and leave” while a save is still running. Medium priority; reproduced in a component test.**

The shared navigation hook accepts separate `dirty` and `pending` arguments and blocks navigation when `pending` is true. Most settings hooks call it as `useSettingsNavigationProtection(dirty || pending)`, leaving its actual pending argument false.

In a test of the real Runtime editor, I held its save request unresolved, attempted navigation, and selected “Discard and leave.” Navigation proceeded while the save was still active. Discard cannot undo that submitted request; it may finish after the user leaves. Save buttons themselves correctly disable duplicate submissions, but the navigation behavior is misleading.

Pass dirty and pending separately at the existing hook call sites, and cover this pending state in the editor tests.

Evidence: [navigation hook](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/frontend/src/features/settings/hooks/useSettingsNavigationProtection.ts:4), [Runtime call site](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/frontend/src/features/settings/hooks/useRuntimeSettingsEditor.ts:12), [component reproduction](/Users/chris/projects/spark/reports/CR-2026-0117-audit/pending-navigation.test.tsx), [failed assertion](/Users/chris/projects/spark/reports/CR-2026-0117-audit/pending-navigation-results.txt). The pattern also appears in several other settings, flow-policy, and trigger editors; project execution's separate-argument call demonstrates that the intended mechanism already exists.

**F6 — `validate` and `set` do not use equivalent validation. Medium priority; reproduced.**

For the same request deleting a referenced LLM profile, `/settings/validate` returned HTTP 200 with `{"valid": true}`, while the actual save returned HTTP 400 identifying the reference. The preflight only parses the candidate profiles; the save also checks references. The CLI inherits this difference because it uses that API.

Validation need not reserve a revision or guarantee a future save against races. It should, however, detect a reference that already exists when validation runs. Share the relevant domain checks between preflight and save, retaining the locked recheck at write time.

Evidence: [preflight dispatch](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/spark-workspace/src/settings.rs:143), [API observations](/Users/chris/projects/spark/reports/CR-2026-0117-audit/api-results.txt).

**F7 — Effective-value and source reporting is incomplete. Medium priority; API/source verified.**

Model inheritance has useful source information. Runtime, connection, provider, and agent views do not identify whether values came from CLI, environment, storage, or built-in defaults. Stored/effective differences alone do not satisfy the requested provenance contract.

Connection reporting also retains the server's startup `client_api_base_url` after a save. The reproduction saved `http://127.0.0.1:4987`; the read showed that stored value but `http://127.0.0.1:8000` as effective. New CLI invocations read the saved target, and the target is not listed as restart-only. The response needs to distinguish actual running-server state from the target used by a newly launched client. Calling both “effective” without consumer/timing context is misleading.

Read shapes are uneven: some sections lack effective/source or validation-error fields; invalid core/model configuration can fail the aggregate read instead of giving the editor a usable scoped error view. F3 demonstrates the practical impact.

Evidence: [public views](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/spark-workspace/src/settings.rs:73), [API observations](/Users/chris/projects/spark/reports/CR-2026-0117-audit/api-results.txt).

**F8 — Cleanup, documentation, and delivery are unfinished. Lower implementation priority, explicit acceptance gap.**

The old `ui-defaults.json` is backed up and marked imported, but remains at its original path after successful migration. The obsolete commands and active defaults autosave machinery are removed; this is a leftover source file, not evidence of ongoing dual writes. Still, the request expressly called for removing the extra file after migration.

The configuration documentation still says that other split/sort choices, graph layouts, and browser preference migration remain outside the implementation, while later sections describe their implementation. That stale paragraph should be reconciled.

There are no implementation commits beyond the workflow base, so intermediate commits and a finished delivery are absent. The original request requires intermediate commits; it does **not** require exactly five commits. I would not treat an evaluator's demand for exactly one commit per phase as an additional user requirement. The worker was also explicitly prohibited from committing and expected its parent to handle that step. That conflict explains the stalled workflow, but does not establish that the code was otherwise finished.

Evidence: [migration source retention](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/spark-storage/src/settings.rs:348), [stale documentation](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/docs/configuration.md:94), original request and workflow instructions, and `git log <base>..HEAD` returning no commits.

**Point-by-point assessment: 1. Ownership and storage**

| Requirement | Status | Assessment |
|---|---|---|
| Core `config/spark.toml` plus existing scoped authorities | Complete | Core sections use the shared store; profiles, policies, triggers, project metadata, conversation metadata, and client documents retain their respective roles. |
| Runtime paths, roots, server binding, client target, Desktop remote access | Partial | All are represented and editable. Desktop environment resolution and connection effective reporting have F2/F7 gaps. Derived paths and actual address are separately represented. |
| Model provider/profile/model/effort as one group | Partial | The shared type and pure resolution implement this. The contextual conversation writer breaks the grouping in F1. |
| Provider endpoints, organization/project, attribution, credential references | Complete | Typed provider settings and dedicated controls exist; validation prevents credential-bearing endpoint forms and stores references rather than key values. Container transport is separately incomplete in F4. |
| Existing LLM profile document, formats, and supported-provider validation | Complete | Existing parser and document are reused. The supported profile provider remains the existing `openai_compatible` contract. Cross-reference-safe editing is separately defective in F3. |
| Execution profiles: default, capabilities, image, metadata, mounts | Complete | Existing execution graph and mount validators are reused; the editor covers these fields. |
| Agent binary/config/seed locations, policies, limits, output, loops, environment, diagnostics | Partial | Existing session controls and native paths are represented and consumed. Existing permission/service policies are preserved. F2/F4 prevent claiming correct consumption everywhere. |
| Flow launch permission and execution-lock policy | Complete | Existing catalog remains authoritative; dedicated editor and validation paths are retained. |
| Trigger schedules, polling, webhooks, actions, allowlists | Complete | Trigger definitions retain these fields and their dedicated editing surface; runtime trigger records remain separate. |
| Optional project model group alongside execution override | Complete | Added without replacing the execution override; operational updates preserve both and extension fields. |
| Optional conversation group; conversation-specific chat/plan mode | Complete | Metadata and nullable inheritance exist; migration preserves mode. F1 concerns how the UI edits that group. |
| Per-client `config/clients/<id>.toml` | Complete | Shared client preference store and distinct client identities are implemented. |
| `spark-common`: shared types and pure rules; domain execution remains in domain crates | Complete | Model resolution and shared settings types live in common; execution behavior stays in the existing agent/execution crates. |
| `spark-storage`: document IO, locking, atomic writes, migration | Complete | Shared revision-checked document boundary and migration helpers are used. |
| `spark-workspace`: operations and coordination of existing validators | Partial | This boundary is present. F3/F6 show missing coordination across profile contents/references and preflight/save. |
| Common resolved configuration; separate persisted/effective/runtime public values | Partial | Common configuration and execution snapshots are substantial. Scoped model/profile carriers remain separate, which fits the scoped-file design. The actual shortfall is incomplete public provenance/effective semantics in F7. |
| Existing profile/policy/trigger/project/conversation operations use common ownership; editing surfaces/formats preserved | Complete | Existing resource routes delegate into the shared storage/domain operations instead of establishing a second writer. |
| Authored flow/node settings stay in YAML | Complete | The work retains authoring and policy distinctions. Graph-authored metadata is not incorrectly treated as UI defaults. |
| Transcripts, runs, tasks, checkpoints, drafts, approvals, caches, restoration outside settings | Complete | No relocation of these operational records into the settings store was found. Conversation settings coexist with metadata while history remains intact. |
| No credential store; existing environment/OAuth ownership; references/status only | Complete | Responses/snapshots inspected contain references and configured/missing state. Secret resolution happens at execution. Redaction contract tests pass. This does not cure F4's missing transport of custom references. |

Key evidence: [common settings](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/spark-common/src/settings.rs), [shared storage](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/spark-storage/src/settings.rs), [workspace operations](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/spark-workspace/src/settings.rs), [profile operations/tests](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/spark-workspace/tests/profile_settings.rs), [agent settings](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/spark-common/src/agent_settings.rs), [provider settings](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/spark-common/src/provider_settings.rs).

**Point-by-point assessment: 2. Resolution, updates, and migration**

| Requirement | Status | Assessment |
|---|---|---|
| Workspace defaults represent a complete selection group | Complete | Workspace group is the root authority; omitted optional values are resolved according to provider/profile rules. |
| Project/conversation inherit the whole group or explicitly replace it | Complete | Pure resolution and stored nullable groups implement whole-group precedence, rather than mixing individual inherited fields. |
| Starting an override copies the effective group; provider/profile changes require compatibility | Partial | Dedicated group editing/validation exists, but F1 drops an inherited profile and F3 permits invalidation through profile editing. |
| Provider/profile mutually exclusive; omitted model/effort means selected provider's default | Complete | Common/domain validation enforces exclusivity and native/profile defaults. Providers with no implicit model retain their existing requirement for an explicit model. |
| “Use defaults” clears the override rather than copying current values | Complete | Explicit `null` is sent and stored; tested hierarchy restoration works. |
| Resolve immediately before each message; no inherited frontend writeback on send | Complete | `start_turn` rereads defaults and captures the selected group; ordinary message submission no longer pins inherited selectors. F1 is a separate explicit-edit path. |
| Return effective model group and workspace/project/conversation source | Complete | Model views expose source and UI labels distinguish inheritance levels. |
| Preserve explicit launch/flow/node precedence; fill only unspecified defaults | Complete | Launch resolution retains explicit choices and fills missing selectors from project/workspace settings; applicable contracts pass. Container consumption remains a separate F4 gap. |
| Preserve execution-profile precedence | Complete | Existing profile selection remains in place, with selected profile capture for active work/recovery. |
| CLI > environment > persisted > built-in precedence | Partial | Server/common/CLI paths implement it; Desktop agent-home capture fails F2. |
| Bootstrap home stays external | Complete | No self-referential home setting was introduced; source-checkout safeguards remain. Desktop retains app-owned bootstrap home. |
| Show configured/effective values under overrides | Partial | Most sections show both, but source provenance and client-target timing are incomplete in F7. |
| Capture resolved execution settings and needed profile contents at message/run start | Partial | Provider/session settings, LLM profile definitions, selected execution profile, continuation/child context are captured. F2 captures incorrect startup values; F4 assumes host-specific captures work inside containers. |
| New work reads current validated configuration | Partial | Rereads and native/provider snapshot tests pass. F3 allows a saved profile to poison new resolution; F4 leaves another execution environment inconsistent. |
| Binding/runtime paths/agent homes require restart and are labeled | Partial | Restart fields and startup retention exist. Agent-home retention must first capture the correct Desktop environment value (F2). |
| File edits observed on next read/new work; no watcher required | Partial | Byte revisions and normal rereads implement this. Non-startup connection effective reporting remains stale in F7. No watcher was added unnecessarily. |
| Save one section/document, preserving unrelated fields | Complete | Shared section patching and preservation tests cover core, profile, project, and conversation paths. |
| Existing atomic writes plus per-document cross-process lock | Complete | File locks and atomic replacement are used; multiprocess conflict tests pass. Reference-changing writes also share a reference lock. |
| Opaque revisions required; stale writes conflict; drafts retained | Complete | Save routes reject stale/missing revisions; editors retain dirty state and report conflicts. Pending navigation has the separate F5 defect. |
| Validate before writing; actionable invalid-file errors | Partial | Local shape/domain validation and redacted parse errors are implemented. Candidate reference compatibility and usable scoped read errors are incomplete (F3/F7). |
| Reject referenced profile deletion and identify references | Complete | Save scans authored configuration, including defaults/projects/conversations/flows/triggers. Historical snapshots correctly do not prevent deletion. Preflight parity is a separate F6 gap. |
| Preserve policy defaults and remote-access confirmation | Complete | Existing policies remain; Desktop remote enabling still requires confirmation. Relevant contracts pass. |
| Versioned, idempotent migration with original backups | Complete | Storage tests cover retry, backup failures, authority, and unsupported future versions. |
| Import Desktop remote-access settings and earlier defaults file | Complete | Core/late Desktop bootstrap import paths preserve existing authority and source backups. |
| Browser defaults import only without authoritative workspace choice | Complete | Import operation checks authority under the shared boundary; frontend retries conflicts and removes the old browser source only after success. |
| All legacy conversations become inherited; preserve mode/history/snapshots | Complete | Migration enumerates stored conversations, including those without readable project registration, clears legacy overrides, and backs up metadata without rewriting history. Tests cover this. |
| Preserve project execution overrides, profiles, policies, triggers | Complete | Existing dedicated files and project fields remain authoritative and are not bulk-rewritten into core settings. |
| Import accessible browser UI preferences for current client | Complete | Legacy persisted layout choices are imported with backups and authority checks. Inaccessible prior origins are explicitly outside automatic recovery, as allowed. |
| Remove extra defaults file/commands/autosave; remove “Save to current conversation” | Partial | Active obsolete commands, queue, and button are removed. The imported source `ui-defaults.json` remains (F8). Preference interaction autosave is intentional and required. |
| One-way version transition, no supported concurrent old/new writers | Complete in implementation | Version markers/future-version rejection establish the new contract. No prolonged dual writer was introduced. Coordinated release itself is still pending. |

Key evidence: [inheritance and active-turn tests](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/spark-workspace/tests/settings_execution.rs), [document/concurrency/migration tests](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/spark-storage/tests/settings_documents.rs), [client migration tests](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/spark-storage/tests/client_preferences_migration.rs), [workflow launch integration](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/attractor-api/src/lib.rs:1708), [agent boundary capture](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/spark-agent-adapter/src/boundary_cli.rs:116).

**Point-by-point assessment: 3. Interfaces and editor**

| Requirement | Status | Assessment |
|---|---|---|
| Scoped reads: stored/effective/source/revision/errors/restart | Partial | Workspace/project/conversation/client reads exist. The complete response contract is inconsistent across sections (F7). |
| Typed section updates identify scope and expected revision | Complete | Tagged typed payloads and scoped identifiers route to common operations. |
| Omitted override unchanged; explicit null inherits | Complete | Nullable patch deserialization distinguishes both and existing adapters use it. |
| Existing resource endpoints retained as common-operation adapters; old read fields retained | Complete | Existing resource responses are augmented with settings information rather than wholly replaced. |
| `spark settings get`, `validate`, `set`, common validation and target/home safeguards | Partial | Commands and safeguards are implemented. Validate does not perform all save-time domain checks (F6). |
| Existing live transport publishes changes; refetch; dirty draft survives external change | Complete | Notifications/refetch and dirty-editor warning paths exist; component and Desktop live-event contracts pass. |
| Native Desktop platform boundary retained, persistence shared | Complete | Native bootstrap/remote controls delegate storage rather than maintaining a separate settings writer. Runtime precedence still needs F2. |
| Stable Desktop identity across ports; browser-generated persistent identity | Complete | Desktop identity is persisted independently of server port; browser identity is stored locally. Contract/unit coverage passes. |
| Models/providers: defaults, connections, credential refs, LLM-profile CRUD | Partial | Full workspace controls exist. Profile CRUD has F3; contextual conversation profile editing has F1. |
| Execution: profiles/defaults/capabilities/images/mounts/validated metadata | Complete | Dedicated profile controls and structured metadata editing use existing validators. |
| Agents: binary/home, policy, limits/timeouts/output/diagnostics | Partial | Controls and policy visibility exist. F2/F4 prevent assurance that configured values reach every actual execution target correctly. Preserving fixed existing policies is appropriate; adding new policy choices was not necessary. |
| Runtime/Desktop: paths/roots/connections/remote/effective/restart | Partial | Controls, confirmation, and labels exist; effective/source and Desktop startup gaps remain (F2/F7). |
| Preferences: mode, widths/splits, advanced controls, child expansion, graph, scope/filter/sort/layout | Complete | These reusable preferences are represented and wired into interactions, with client-scoped persistence. |
| Scoped configuration: project model/execution, conversation inheritance, flow/trigger links | Partial | Project editors, inherited conversation source/reset, and dedicated-editor links exist. Conversation overrides still use the incomplete scalar dropdown path (F1). |
| Dedicated controls; validated structured metadata; no generic schema form framework | Complete | Implementation uses concrete editors and the existing component stack. |
| Each section Save/Discard, field validation, success/error feedback | Partial | Editors provide these states and keep failed drafts. Profile saves can claim success for an invalid resulting selection (F3); response/error shapes need F7. |
| Navigation protection for unsaved edits | Partial | Dirty-edit protection works in covered cases. Pending Save is incorrectly treated as discardable dirty state in F5. |
| Disable duplicate pending submissions; request state local | Complete | Section save buttons track local pending state. This does not imply pending navigation is correct. |
| Contextual dropdowns commit explicit conversation overrides | Partial | They do commit overrides, but can unintentionally switch off a profile (F1). |
| Layout interactions persist on completion without Settings Save | Complete | Completion callbacks and preference controller persist reusable changes outside the explicit settings form flow. |
| Keep selection/scroll/individual transcript expansion/drafts/topology/computed routes operational | Complete | Reusable presentation settings are persisted; per-record/session/cache state retains its existing role. |

Key evidence: [HTTP settings routes](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/spark-http/src/workspace.rs:937), [CLI implementation](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/crates/spark-cli/src/lib.rs), [editor organization](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/frontend/src/features/settings/SettingsPanel.tsx), [preference persistence controller](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/frontend/src/features/settings/ClientPreferencesController.tsx), [Desktop contracts](/Users/chris/projects/spark/.spark/checkouts/run-18d45752f4ec6770/apps/spark-desktop/tests/desktop_contracts.rs:288).

**Point-by-point assessment: 4. Implementation sequence and delivery**

| Planned phase/deliverable | Status | Assessment |
|---|---|---|
| 1. Shared model and persistence | Partial | Main infrastructure is implemented and well tested. Cross-reference validation and consistent public views still need F3/F6/F7. |
| 2. Runtime integration and snapshots | Partial | Substantial common runtime integration exists. Desktop startup precedence and container boundaries need F2/F4. |
| 3. Inheritance and removal of default writeback | Partial | Core inheritance, migration, and send-time resolution are implemented. Explicit conversation editing needs F1. |
| 4. Scoped API and full editor | Partial | All major areas have an implementation. F3/F5/F6/F7 keep this from satisfying the behavior contract. |
| 5. Client preferences, cleanup, docs/contracts | Partial | Preference persistence/migration and many contracts exist. Source-file cleanup and contradictory docs remain; tests miss several boundary cases. |
| Intermediate commits | Incomplete | No implementation commits beyond the base. Exactly five commits was not a literal request. |
| Release matching frontend/backend migration together | Incomplete as delivery | Matching changes are together in the worktree, but have not been committed/merged/released. No evidence that mixed versions were intentionally shipped. |
| Avoid prolonged dual writes | Complete in implementation | Superseded active writers were removed; leftover migration source files are passive. |

The final diff demonstrates coverage of all five planned areas. With no implementation commit history, it does not independently prove that they were carried out in the requested chronological order. More importantly, satisfying the sequence would not cure the behavior failures above.

**Point-by-point assessment: 5. Acceptance and verification**

| Acceptance criterion | Status | Evidence and remaining work |
|---|---|---|
| Persistence across Desktop port changes/server restart; unrelated sections preserved | Complete in automated contracts | Desktop restart/different-port contract and storage preservation tests passed. I did not independently launch/restart the native GUI. |
| Migration sources/authority/retry/backups/all legacy chats | Complete for functional migration | Tests cover these cases, including orphaned registration and backup failures. Removal of the extra source file remains F8. |
| Workspace/project/conversation inheritance on next message; clearing restores inheritance | Partial overall | Resolution and clearing tests pass. An explicit contextual edit can corrupt the intended group (F1). |
| UI/CLI/agents/triggers consistent defaults; active/history unchanged | Partial | Shared launch/turn paths and snapshot tests provide substantial evidence. F1/F2/F4 disprove a blanket consistency claim; live container verification remains outstanding. |
| Profiles/formats/mounts/deletion/policies/remote confirmation | Partial overall | Round-trip, invalid mounts, deletion locks, policies and remote confirmation tests pass. Referenced profile edits still invalidate defaults (F3), and preflight is inconsistent (F6). |
| Concurrent edits cannot silently overwrite; failure keeps drafts/no false success | Partial overall | Document/HTTP/component conflict tests pass. F3 can report success for an invalid combined configuration; F5 offers misleading discard during an active save. |
| Client isolation; Desktop identity stable across ports | Complete in contracts | Client identity/isolation, migrations, interaction persistence, and Desktop port-change tests passed. |
| UI Save/Discard/navigation/validation/source/CRUD/restart end to end | Partial; browser recheck unverified | Existing component coverage passed, but focused pending-save test failed and API checks exposed group/CRUD/provenance defects. I could not rerun browser scenarios. |
| Secrets absent from responses/errors/migration logs | Complete in inspected paths/tests | Environment refs/status, safe parsing, endpoint/reference validation, and redaction tests passed. No new credential store or OAuth owner was introduced. |
| No crate cycle; operational state outside settings | Complete | Full workspace/all-features compilation/tests passed; inspected ownership respects operational-state boundaries. |

Fresh checks run against this worktree:

| Check | Result |
|---|---|
| `SPARK_HOME=<isolated writable temporary home> just test` | Passed. This runs formatting, workspace Rust tests with all features, frontend unit/component tests, and production frontend build. Some intentionally ignored Rust tests remain ignored. |
| Frontend tests | 74 files, 567 tests passed. |
| Frontend lint | Passed with 0 errors and 15 warnings. |
| TypeScript and production frontend build | Passed. Vite reported large-chunk warnings. |
| Release Desktop build with all features | Passed. This is a build, not an installed native-app smoke test. |
| Independent disposable-server API probes | Reproduced F1, F3, F6, F7; no model calls. |
| Independent Desktop bootstrap/capture probe | Failed its correctness assertion as described in F2. |
| Independent pending-navigation component test | Failed its correctness assertion as described in F5. |
| Browser/native UI and live Docker rechecks | Not independently completed. Earlier workflow browser results are historical evidence, not a fresh audit pass. |

The first unqualified `just test` attempt failed two worker-process protocol tests under the sandbox: resolving the default home attempted a settings lock under `~/.spark`, and the resulting warning was written to stdout before the worker JSON response. An isolated writable `SPARK_HOME` made the full gate pass. The stdout logging behavior pre-existed this change; settings initialization exposes it on this failure path. I do not count it as evidence that ordinary writable installations fail, but worker stdout should remain machine-readable when configuration initialization fails. The test run should also state its required home explicitly.

The [evidence directory](/Users/chris/projects/spark/reports/CR-2026-0117-audit/README.md) includes reproduction sources, observed results, and rerun instructions. The API probe creates and removes its own home/server and does not mutate the user's running Spark instance. The focused failing tests were kept outside the implementation worktree.

**Does the completed work fit the spirit of the request?**

**At the architecture and feature-coverage level, largely yes. At the user-behavior and delivery level, not yet.** The change is more than a cosmetic settings screen. It establishes shared storage, useful concurrency guarantees, backed-up migrations, scoped inheritance, actual execution capture, and separate client preferences. It reuses existing domain parsers and file formats, keeps operational records separate, preserves secret ownership and remote confirmation, and avoids a new generic form framework. Those choices fit both the request and the project's preference for existing mechanisms.

The spirit was that a saved choice should mean the same thing throughout Spark, and that explicit Save should be trustworthy. F1–F7 show where that promise breaks. Retaining a scalar conversation-edit path undermines the new group model. Capturing a setting without respecting Desktop startup environment or container execution context makes a consistent-looking snapshot unreliable. Separate preflight/save checks and misused navigation-hook arguments create inconsistent editor behavior. The missing tests are chiefly combinations across boundaries, not missing tests for individual structs or controls.

I would keep the implementation and finish it, rather than discard the work. Before acceptance, correct F1–F7, add focused checks for the reproduced combinations, finish F8 cleanup/docs, then rerun the browser/restart and relevant real-container scenarios and deliver matching frontend/backend changes together. The work merits preservation, but it does not merit a “complete” verdict in its current state.

Browser verification limitation: automatic approval review rejected opening the isolated audit server because browser access was denied. I did not bypass that rejection; the report distinguishes API/component evidence from browser verification.
