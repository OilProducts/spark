# Provider-native reasoning effort (phase 1)

## Summary

Make reasoning effort work the way each provider actually offers it. For every model Spark lists, report the effort levels that model accepts and its default, taken live from the provider where it publishes them and from a curated catalog where it does not. Send the chosen effort to each provider in that provider's own field. The model picker then shows the right levels for each model.

Phase 1 covers effort only and keeps the stored `reasoning_effort` string. Thinking budgets, OpenAI's `reasoning.mode` (`pro`), and reasoning summaries need new stored fields and are phase 2.

Codex and Claude Code already work this way (Codex discovery; Claude Code's catalog and `--effort`, commit ae1e40c4) and stay as they are.

## Why

Today the picker offers one fixed list (`low` to `ultra`) for every provider except Codex and Claude Code:

- the OpenAI API receives it unchecked, although its levels are model-dependent;
- the Anthropic API ignores it entirely;
- Gemini ignores it;
- OpenAI-compatible profiles fail the request with `unsupported_reasoning_effort`.

## What each provider offers

Verified against current provider documentation while writing this request. Re-verify any value you rely on; update the catalog, not the code, when a provider changes.

- **Anthropic API.** `output_config: {effort}` with levels from `low`, `medium`, `high`, `xhigh`, `max`, depending on the model; some older models (for example Haiku 4.5) have no effort. `GET /v1/models` returns `capabilities.effort.<level>.supported` for every model.
- **OpenAI API (Responses).** `reasoning: {effort}` with levels from `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`, depending on the model (for example GPT-6 Astra rejects `none`). OpenAI does not publish per-model levels through an API, so they come from the curated catalog.
- **Gemini API.** A thinking level from `minimal`, `low`, `medium`, `high`, depending on the model, with model-specific defaults. The Model resource reports only a `thinking` boolean, so levels come from the curated catalog. Confirm the exact `generateContent` request field (the thinking level inside the thinking config) against current Gemini documentation and the existing Gemini adapter before implementing.
- **OpenAI-compatible, OpenRouter, and LiteLLM (Chat Completions).** The standard field is `reasoning_effort`; what a server honours varies. Levels come from configuration, not guesses.

## Model metadata

- In the curated catalog (`crates/spark-assets/assets/unified_llm/data/models.json`, read by `unified-llm-adapter/src/catalog.rs`), replace `supports_reasoning: bool` with `reasoning_efforts: [..]` (ordered, empty for none) and `default_reasoning_effort` (optional).
- Refresh the OpenAI and Gemini entries to current models with their documented levels and defaults. Keep existing entries' other fields.
- Add a per-provider fallback level list to the catalog for models it does not list, such as a custom model name: the provider's full documented set. Report it as a fallback so the picker can mark it as unverified for that model.
- Keep anything that used `supports_reasoning` (for example `resolution.rs` capability checks) working by deriving it from a non-empty `reasoning_efforts`.
- Anthropic: when an Anthropic API key is configured, list models from `GET /v1/models`, taking each model's effort levels from `capabilities.effort`. Cache it like the Codex and Claude Code discovery caches. Fall back to the curated catalog when the call fails or no key is set.
- OpenAI-compatible LLM profiles, OpenRouter, and LiteLLM: add an optional `reasoning_efforts` list to the profile configuration. With none declared, the model offers no effort levels.
- `/workspace/api/projects/chat-models` reports these through the existing `supported_reasoning_efforts` and `default_reasoning_effort` fields, plus the provider fallback list.

## Requests

- Anthropic adapter: send `output_config: {effort}` when an effort is set. Leave `thinking` configuration as it is today (the existing `provider_options.thinking` passthrough).
- OpenAI adapter: keep `reasoning: {effort}`.
- Gemini adapter: send the thinking level when an effort is set.
- OpenAI-compatible adapter (also OpenRouter and LiteLLM): send `reasoning_effort` when an effort is set and the profile or provider declares levels, replacing the `unsupported_reasoning_effort` rejection. With no declared levels, keep rejecting an explicit effort with a message that names the profile setting.
- An effort the selected model does not list is sent unchanged; the provider's own error is surfaced as it is today.

## Picker

- Remove the frontend's fixed effort fallback list (`standardEfforts` in `ModelChooser.tsx`). Effort levels come only from the backend, per model, with the provider fallback for unlisted models.
- Show the provider's level names as they are (for example "None" and "Minimal" where offered).
- Mark fallback levels for an unlisted model as unverified (for example with a short note under the effort row).

## Verification

- Catalog: loads the new fields; `supports_reasoning` users still behave the same.
- Anthropic discovery: maps `capabilities.effort` into levels, falls back to the catalog on failure or without a key, and caches.
- Adapter request tests for each provider: the effort appears in that provider's field when set and is absent otherwise; OpenAI-compatible sends `reasoning_effort` only with declared levels.
- Chat-models endpoint reports per-model levels and defaults for every provider, and the provider fallback.
- Picker: shows each model's levels, marks fallback levels, and shows only Default for models with none.
- `just test`.

## Out of scope (phase 2)

- Thinking budgets (`budget_tokens`) for budget-only Anthropic models, OpenAI `reasoning.mode` (`pro`), reasoning summaries, and turning thinking off. These need new stored settings fields.

## Hand-off

Write `changes/CR-2026-0127-provider-native-reasoning-effort/result.md` (frontmatter with id, title, status, type, changelog; sections Summary, Validation, Shipped Changes) when the work is complete.
