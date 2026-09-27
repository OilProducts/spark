---
id: CR-2026-0125-reusable-model-chooser
title: Reusable model chooser
status: completed
type: feature
changelog: public
---

## Summary

All model selectors named in the request now use one ModelChooser with ModelSettings value/onChange, project discovery, inheritance labels, fields or compact layout, and disabled controls. Persistence remains with each caller.

## Validation

- `just test` passed: formatting, workspace Rust tests, 83 frontend test files (608 tests), and production frontend build.
- Gatekeeper follow-up: a fresh foreground `just test` exited **0**; full output is saved at `/tmp/cr0125-next-gate.log` and the exit status at `/tmp/cr0125-next-gate.exit`. The historical mission assertion passed in this run; there is no failing command, nonzero gate status, or assertion failure to report. No additional implementation change was justified.
- Validation routing was verified in both the installed `$SPARK_HOME/flows/software-development/implement-change.yaml` and its bundled copy: `validate` failure routes through `count_validate_attempt` before `diagnose_validation`. Every tool execution writes `context.tool.output` and `context.tool.exit_code` (`crates/attractor-runtime/src/handlers.rs`), so the counter replaces the failed gate evidence with `{"attempts":1}` and exit code 0. That supplied counter result cannot establish the gate's outcome; future failures must retain the validate node's own output/status before the counter runs. Flow/runtime changes are outside this model-chooser change.
- The previously reported `mission_routes_enforce_records_revisions_messages_and_controls` assertion did not recur: the test passed within the full gate. No backend changes were needed.
- Eight component tests cover selection values, profile/discovery/suggestion precedence, provider-scoped effort metadata, defaults, custom values, inheritance labels, disabled controls, loading/failure fallbacks, shared fetching, reconnect refresh, and stale responses.
- Affected settings, missions, chat, graph settings, inspector, canvas, and frontend contract suites passed. The focused ModelChooser and ProjectsPanel run passed all 52 tests.
- Chat controller/composer regression coverage enforces `expected_revision` and delays responses. It verifies rapid typing and clearing are coalesced, subsequent provider/model/effort edits persist their final values, and an external revision conflict stops queued saves while retaining the latest draft. It also checks null model preservation, discovered-default send eligibility, and unavailable custom-model blocking.
- After `npm --prefix frontend run build`, all 31 browser smoke tests passed using `PLAYWRIGHT_BROWSERS_PATH=/Users/chris/Library/Caches/ms-playwright npm --prefix frontend run ui:smoke -- settings-editor.spec.ts missions-editor.spec.ts editor-save.spec.ts editor-diagnostics.spec.ts`. The environment override selects the installed browser cache.
- Gatekeeper follow-up smoke run, after the fresh gate's frontend build: the same 31 tests passed with exit code **0**; output is saved at `/tmp/cr0125-next-smoke.log` and status at `/tmp/cr0125-next-smoke.exit`.
- The new real-backend browser regression forwards all settings writes unchanged to Rust and delays their responses. Rapid `my-model` typing, clearing, and subsequent provider/custom-model/effort edits produce six serialized HTTP 200 saves with advancing expected revisions; the final values survive reload.
- `git diff --check` passed.

## Shipped Changes

- Shared provider/profile, model, and reasoning-effort controls across workspace and project settings, mission Model menu, compact chat composer, flow defaults, node inspector, and inline canvas editing.
- Profile models take precedence over discovery; discovery takes precedence over suggestions. Codex and Claude Code use provider-family suggestions while discovery loads or is unavailable. Custom models remain editable and unlisted saved models are marked custom.
- Discovered effort metadata is scoped to the selected provider/model, including profile and provider defaults; missing effort metadata uses the standard list.
- useModelOptions shares one discovery request per mounted project, refreshes on spark:codex-connected, and ignores superseded responses.
- Removed ModelSettingsFields and the chat controller's duplicate option builders and discovery cache. Chat retains its availability message, discovered-default fallback, and send eligibility rules.
- Chat persistence serializes saves per conversation, coalesces pending edits, and uses each acknowledged revision for the next write. Defaults use the same save path. Drafts remain scoped to their conversation, and conflicts retain the latest draft without automatically retrying over another writer.
- Chat passes the editable ModelSettings unchanged to the chooser and displays the discovered Codex default separately. The discovered-default send fallback, availability message, and send eligibility remain unchanged.
- Flow attributes are mapped at call sites. Tests use shared accessible labels; mission browser coverage now exercises the chooser and custom entry.
