---
id: CR-2026-0129-provider-native-reasoning-controls
title: Provider-native reasoning controls (phase 2)
status: completed
type: feature
changelog: public
---

## Summary

Added nullable thinking, token budget, reasoning mode, and reasoning summary settings. Model pickers show provider-supported controls, preserve inherited-choice locking, and require a budget of at least 1024 tokens. Saved selections and flow settings reach native provider requests through the existing captured execution settings.

## Validation

- Contract tests cover older settings documents, null defaults, numeric/type/range errors, all settings scopes, mission turn snapshots, provider capability precedence, request omission and serialization, Anthropic override precedence, arbitrary stylesheet efforts, and flow-to-session propagation.
- Review regressions exercise the actual flow parser through Codergen and the session request: stylesheet `thinking: budget; thinking_budget_tokens: 2048` plus a node budget of 4096 produces an effective request budget of 4096. Invalid resolved combinations (missing budget, budget without budget mode, and explicit off with a budget) remain rejected. Invalid declared fields remain rejected even when overridden.
- CLI regressions exercise `flow validate --file`: invalid stylesheet thinking, budget, mode, and summary controls produce validation diagnostics; inherited budgets pass. The CLI and parser now call the same shared validator.
- Picker and editor tests cover capability gating, inherited locking, budget selection, preservation across edits, and numeric YAML serialization and clearing.
- Apply to Nodes regressions inspect saved node YAML with workspace thinking=budget and budget=2048: graph off/adaptive omit the node budget; default clears an existing node budget; resolved budget inherits 2048 and an explicit graph budget overrides it. Targeted `npm --prefix frontend run test:unit -- src/features/editor/__tests__/GraphSettings.test.tsx`: passed (25 tests).
- Final foreground `just test` after the Apply to Nodes review fix: passed (exit 0), including formatting, all Rust suites, 628 frontend tests across 83 files, and the production frontend build. Log: `/tmp/reasoning-apply-defaults-gate.log`. The build emitted its existing Node deprecation and bundle-size advisories.
- `git diff --check`: passed. Targeted reasoning contracts and budget-editing checks also passed.

Provider evidence checked on 2026-09-27:

- [Anthropic model capabilities](https://platform.claude.com/docs/en/api/models/list) expose adaptive/enabled thinking types, with no documented disabled indicator. Live types take precedence; explicit per-model notes supply off. [Thinking](https://platform.claude.com/docs/en/build-with-claude/thinking) documents models that prohibit disabling, including Opus 5.5, and Opus 5's effort-dependent restriction. [Extended thinking](https://platform.claude.com/docs/en/build-with-claude/extended-thinking) documents the fixed budget and 1024 minimum. Unsupported model/effort combinations remain provider errors.
- [OpenAI reasoning guide](https://developers.openai.com/api/docs/guides/reasoning), consulted using the OpenAI Docs skill, documents GPT-5.6/GPT-6 standard/pro modes and model-dependent summaries. Notes enrich only discovered IDs; they do not add available models.
- [OpenRouter reasoning documentation](https://openrouter.ai/docs/guides/best-practices/reasoning-tokens) documents `mandatory`, `supports_max_tokens`, `reasoning.enabled`, and `reasoning.max_tokens`. No summary selector was verified; none is advertised. Google budget controls remain excluded.
- Installed `codex-cli 0.154.0`: `codex app-server generate-json-schema --out /tmp/spark-reasoning-schema` confirms `v2/TurnStartParams.json` accepts optional `summary`, referencing `ReasoningSummary` with `auto`, `concise`, and `detailed` (plus `none`, intentionally outside this contract). Codex requests now send the selected summary and omit it for null instead of forcing detailed summaries. The fake app-server process test checks the serialized selection.

## Shipped Changes

- Shared stored-setting validation and backward-compatible defaults for workspace, project, conversation, and mission settings; field-specific HTTP errors without exposing submitted values.
- Native Anthropic thinking payloads, OpenAI Responses reasoning mode/summary, OpenRouter off/budget controls, and verified Codex summaries. Anthropic `provider_options.thinking` retains precedence.
- Live capabilities followed by Spark notes, with no added catalog models. Claude Code, Gemini, LiteLLM, and compatible profiles gain no unverified picker controls.
- Field validation runs on declarations; thinking/budget consistency runs after node, stylesheet, and graph precedence resolves. CLI file validation shares the save/launch parser’s validation path.
- Flow execution/default attributes, stylesheet validation and resolution, run records, captured inherited defaults, and fan-in request fields. Stylesheets accept arbitrary effort strings. Existing legacy effort spelling normalization remains compatible.
- Picker controls and flow editor mappings, including Apply to Nodes and Reset From Global, preserve all four fields and clear obsolete budgets when leaving budget mode.
- Apply to Nodes resolves thinking before copying its budget, populating node budgets only for resolved budget mode and clearing them for off, adaptive, or default.
- Launch-time flow-run overrides, Gemini budgets, and unverified controls remain out of scope.
