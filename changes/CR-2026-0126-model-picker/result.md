---
id: CR-2026-0126-model-picker
title: Model picker
status: completed
type: feature
changelog: public
---

## Summary

Replaced ModelChooser's three selects with one accessible, searchable Radix Popover picker. The trigger displays explicit or resolved inherited model and effort values across settings, missions, chat, graph defaults, the node inspector, and canvas node editing.

## Validation

- Effort-reset follow-up: all 99 affected component and caller tests passed (picker, settings, project protection, chat, and missions). Log: `/tmp/model-picker-effort-targeted.log`.
- Follow-up frontend build passed before browser smoke. All 38 requested browser checks passed across the suite and one targeted rerun. The first run passed 37 checks; the workspace/project regression successfully saved but expected a null profile field that project storage omits. After correcting that test expectation, the regression passed, including both HTTP 200 saves, persisted values, and reloads. Logs: `/tmp/model-picker-effort-build.log`, `/tmp/model-picker-effort-smoke.log`, `/tmp/model-picker-effort-smoke-recheck.log`.
- New component regressions cover atomic effort commits after clearing with discovered Codex defaults, explicit inherited models, non-Codex providers, and LLM profiles, plus preserving inheritance when choosing Default effort. Real-backend regressions cover Use default → reopen → High → Save for workspace and non-Codex project settings, mission saving, and an inherited-profile chat effort edit queued behind a pending reset. The existing chat revision/conflict coverage remains green.

- Foreground `just test` passed: Rust formatting, workspace Rust tests with all features, all 83 frontend test files (619 tests), and the production frontend build. Log: `/tmp/model-picker-effort-just-test.log`.
- Sixteen component tests cover resolved labels, search across providers and profiles, atomic model/profile selection, default reset, custom entry without keystroke saves, supported and custom efforts, loading/unavailable discovery, shared refresh and stale responses, keyboard navigation, focus return, disabled controls, and invalid-model feedback.
- Real-backend browser regressions use deliberately different saved overrides and inherited defaults. They assert the “Use default” model and effort, the cleared draft trigger, and the persisted result for workspace, project, conversation, and mission resets. The conversation regression holds the reset request pending to verify the optimistic label before persistence. Chat revision queue coverage also passed.
- Updated caller coverage includes chat send eligibility, revision-checked queued saves and conflicts, settings draft protection, missions, graph settings, inspector edits, and canvas model/effort persistence. Regression coverage verifies an effort choice cannot replace a just-committed custom model with another row.
- Regenerated and visually reviewed chat composer and editor inspector screenshots at widths 1440 and 390 in both themes. Eight files are in `frontend/artifacts/ui-smoke/`, named `model-picker-{chat,inspector}-{light,dark}-{1440,390}.png`. Browser assertions check inspector popover viewport bounds and keyboard focus.
- `git diff --check` passed.

## Shipped Changes

- Fixed effort commits after Use default: the picker now reuses the displayed effective selection, preserving its inherited provider/profile and model when committing an explicit effort. Provider discovery defaults retain a null model, and choosing Default effort on a cleared selection keeps inheritance. Each choice still emits one atomic onChange; mission/chat share this fix without changing the chat save queue or API reset normalization.
- Single compact trigger or full-width labeled field, preserving disabled and invalid states.
- Provider/profile groups retain profile models, discovered display names and defaults, suggestion fallbacks, and saved custom models through the existing useModelOptions hook.
- Search supports model identifiers, display names, provider names, and profile names. Custom entries commit explicitly for the provider/profile in context.
- Model clicks commit provider/profile and model together while leaving the picker open; effort choices and Enter close it. Search never saves. Effort controls reflect discovery metadata and retain unsupported saved efforts as custom.
- Cleared picker settings serialize to the existing null reset for conversations and projects, and the explicit Codex default group required by workspace settings. Serialization is shared at the API boundary; chat’s save queue is preserved.
- Explicit groups with null effort resolve the provider default without borrowing saved inherited effort. Known empty effort support shows only Default and any saved custom effort; unknown support retains standard choices. Tests cover clearing saved High and changing providers with inherited settings present.
- Listbox/option semantics, search autofocus, arrow navigation, Escape focus return, and viewport-constrained scrolling support keyboard and narrow-screen use.
- Reset inheritance follows the scope being restored: workspace resets explicitly resolve Codex discovery defaults; project resets read workspace settings; conversation and mission resets read project settings (including workspace fallback). A shared read hook refreshes parent settings on settings events and focus and discards responses after project changes or unmount. Saved overrides are never used as the reset default. Chat send rules and the save queue remain unchanged. No backend or security configuration changes, dependencies, or provider sign-in actions were added.
