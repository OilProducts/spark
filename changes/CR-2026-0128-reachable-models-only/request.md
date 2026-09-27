# List only reachable models, cached for the life of the process

## Summary

The model picker lists only models Spark can actually use right now: providers Spark is configured for, with models taken from each provider's own list. The bundled catalog stops being a source of available models; it keeps supplying costs, aliases, and reasoning-level notes for models a provider actually lists. Model discovery is cached for the life of the process and refreshed only by the Spark events that change what is reachable.

## Why

- `public_unified_chat_models()` (`crates/spark-workspace/src/models.rs`) lists catalog models for OpenAI, Anthropic, Gemini, OpenRouter, and LiteLLM whether or not Spark is configured for them, and Anthropic discovery falls back to the catalog on failure. The picker offers models Spark cannot call.
- Discovery results expire every five minutes (`CODEX_MODELS_CACHE_TTL` and `CLAUDE_CODE_MODELS_CACHE_TTL` in `models.rs`; the Anthropic cache in `unified-llm-adapter/src/model_discovery.rs`), re-running discovery for information that changes only when Spark's configuration does.

## Availability

A provider appears in the picker only when Spark can reach it:

| Provider | Reachable when | Models from |
|---|---|---|
| Codex | the existing discovery succeeds | Codex `model/list` (unchanged) |
| Claude Code | the existing discovery succeeds | the CLI catalog (unchanged) |
| Anthropic API | an API key resolves through Spark's provider configuration | `GET /v1/models` |
| OpenAI API | an API key resolves | `GET /v1/models` |
| Gemini API | an API key resolves | `models.list`, keeping models whose `supportedGenerationMethods` include `generateContent` |
| OpenRouter | an API key resolves | `GET /api/v1/models` |
| LiteLLM | a base URL resolves | the proxy's model list endpoint |
| LLM profiles | configured | the profile's configured models (unchanged) |

- Resolve credentials and endpoints the way Phase 1's Anthropic discovery does: Spark's provider configuration and execution environment mapping, never a bare environment lookup.
- A provider that is configured but whose list request fails appears with an unavailable status and the error, like Codex today, with no models. Remove the catalog fallback.
- An unconfigured provider does not appear.
- OpenAI's model list includes models that cannot run a Responses turn (embeddings, moderation, speech, transcription, image, realtime). Exclude those families by their documented ID prefixes, with a `ponytail:` comment noting that OpenAI's model object has no capability field to filter on.

## Reasoning levels

For each listed model, in order of preference:

1. The provider's own per-model data: Anthropic `capabilities.effort` (Phase 1), OpenRouter `reasoning.supported_efforts` and `reasoning.default_effort`, and Codex and Claude Code as today.
2. The bundled notes for that model ID (the Phase 1 `reasoning_efforts` and `default_reasoning_effort` fields in `models.json`), used for OpenAI and Gemini, whose model lists do not report levels. OpenAI's Model object has only `id`, `created`, `object`, `owned_by`, and `shutdown_date`; Gemini's Model resource has only a boolean `thinking`.
3. The provider's documented level set, marked unverified (Phase 1 behaviour). For Gemini, only when the model reports `thinking: true`; otherwise none.

LiteLLM: if its model endpoint reports reasoning support per model, use it; otherwise profile-declared levels as in Phase 1. Verify against current LiteLLM documentation.

The notes never add a model to the list. Catalog costs and aliases keep working for models that are listed.

## Caching and refresh

- Cache each provider's discovery result, success or failure, for the life of the process. Remove the time-based expiry.
- Keep keying caches by the resolved configuration they used, so a changed configuration misses the cache.
- Clear the affected provider's cache, and publish the existing settings live event so open pickers reload, when:
  - Codex sign-in or sign-out completes (`crates/spark-http/src/codex_auth.rs`);
  - provider settings, LLM profiles, or native agent settings are saved;
  - the user presses the existing Refresh in the picker's discovery-unavailable state, if one exists; do not add one.
- The frontend (`useModelOptions`) keeps one fetch per project and reloads on those events. Drop any timer-based refetch.

## Verification

- Unconfigured providers are absent from `/workspace/api/projects/chat-models`; configured ones list only what their endpoint returns (use local fixture servers, as Phase 1's Anthropic discovery test does).
- A configured provider whose endpoint fails reports unavailable with the error and no models.
- OpenAI filtering excludes the non-Responses families; Gemini keeps only `generateContent` models.
- Reasoning levels follow the order above, including OpenRouter's live `reasoning` data and Gemini's `thinking: false`.
- Discovery runs once per provider per process; saving provider settings, LLM profiles, or native agent settings, and Codex sign-in, each trigger exactly one new discovery for the affected provider.
- Costs and aliases still resolve for listed models.
- Picker: only configured providers appear; an unavailable provider shows its status.
- `just test`.

## Out of scope

- Phase 2 reasoning controls (thinking budgets, OpenAI `reasoning.mode`, summaries).
- Claude Code sign-in (tracked as a separate mission). When it lands, sign-in joins the refresh events.

## Hand-off

Write `changes/CR-2026-0128-reachable-models-only/result.md` (frontmatter with id, title, status, type, changelog; sections Summary, Validation, Shipped Changes) when the work is complete.
