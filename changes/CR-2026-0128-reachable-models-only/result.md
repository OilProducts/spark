---
id: CR-2026-0128-reachable-models-only
title: List only reachable models, cached for the life of the process
status: completed
type: feature
changelog: public
---

## Summary

The picker gets available API models from configured providers instead of the bundled catalog. Unconfigured API providers are absent; discovery failures show the provider error without suggested catalog models. Codex, Claude Code, and configured profiles retain their discovery sources. Discovery successes and failures persist until configuration or authentication changes.

## Validation

- Local HTTP fixtures verify all five API providers, Spark credential references and endpoints, optional LiteLLM authentication, unavailable errors, model filtering, reasoning metadata, cache reuse across projects, and exactly one rediscovery after provider/profile/native settings saves and Codex authentication changes.
- The route fixture populates the native model cache before the first connection status read, then verifies initial and repeated unchanged reads cause no extra model/list calls or settings events. Login completion and observed sign-out each produce exactly one event and one rediscovery; subsequent reads reuse the cache.
- Adapter contracts verify Anthropic/Gemini pagination, malformed responses, cached failures, credential redaction, changed configurations, live/catalog/unverified reasoning precedence, Gemini thinking support, costs, and callable aliases.
- Frontend tests verify provider status parsing, no fallback suggestions, unavailable errors, unverified levels, and one shared fetch per project per affected settings event.
- Final foreground `just test`: passed (exit 0), including formatting, all Rust suites, 619 frontend tests across 83 files, and the production frontend build. Log: `/tmp/cr128-review-fix-just-test.log`. The build emitted its existing Node deprecation and large-chunk advisories.
- `git diff --check`: passed.

Documentation checked on 2026-09-27:

- [OpenAI Model resource](https://developers.openai.com/api/reference/resources/models) and [model families](https://developers.openai.com/api/docs/models): the list lacks capability metadata, so documented non-Responses ID families are filtered with the required `ponytail:` comment.
- [Gemini Model resource](https://ai.google.dev/api/models): discovery follows pagination, requires `generateContent`, and only offers reasoning levels when `thinking` is true.
- [OpenRouter reasoning metadata](https://openrouter.ai/docs/guides/best-practices/reasoning-tokens): live supported/default efforts take precedence; documented gateway levels supply the unverified fallback.
- [LiteLLM routing](https://docs.litellm.ai/docs/routing), [model management](https://docs.litellm.ai/docs/proxy/model_management), and [the proxy's model-list implementation](https://github.com/BerriAI/litellm/blob/main/litellm/proxy/proxy_server.py): `/v1/models` lists callable public names in OpenAI-compatible form; detailed management metadata belongs to `/model/info`. The standard list does not document effort levels. Spark consumes explicit reasoning metadata if supplied and otherwise retains profile-declared efforts without guessing from proxy aliases.

## Shipped Changes

- Shared API discovery uses Spark's resolved provider configuration and execution environment mapping; no direct credential environment lookups or catalog availability fallback.
- Catalog notes enrich listed IDs with reasoning levels/defaults, costs, and aliases while preserving each provider's callable ID. Explicit empty live efforts and Gemini non-thinking models remain empty. Provider fallback efforts are marked unverified.
- Configuration-keyed caches retain successes and failures for the process lifetime. Discovery and invalidation are serialized so concurrent requests reuse results. Native and API caches invalidate by affected settings section; The first Codex status observation establishes a baseline without invalidation or an event. Codex login completion publishes immediately, and later status checks avoid duplicate refreshes while observed account changes invalidate the cache. Observed external sign-out also invalidates Codex.
- Existing settings live events refresh shared picker requests. Removed duplicate browser-only Codex/profile save notifications. The picker no longer adds catalog suggestions or saved-but-undiscovered models to provider lists.
- No existing picker Refresh action or Codex sign-out endpoint was present. Login cancellation still preserves credentials. Phase 2 controls and Claude Code sign-in remain out of scope.
