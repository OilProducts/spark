---
id: CR-2026-0127-provider-native-reasoning-effort
title: Provider-native reasoning effort (phase 1)
status: completed
type: feature
changelog: public
---

## Summary

Kept the stored `reasoning_effort` string and native CLI adapters. API models now expose ordered efforts and optional defaults; unknown native models use visibly unverified provider fallback levels. Models without declared levels offer only Default without rewriting saved effort.

## Validation

Final review fixes on 2026-09-27:

- Anthropic discovery now reads the existing resolved provider configuration from `spark.toml` and uses its `execution_environment` mapping before feeding the existing cache. This preserves execution's environment precedence and credential-reference handling.
- The captured-chat socket harness explicitly restores blocking mode immediately after accept, before setting the read timeout and cloning. Existing deadlines and assertions remain unchanged, matching the captured-workflow harness fix.
- A deterministic subprocess integration test clears inherited environment variables and configures a local `base_url` plus custom `api_key_env`, with no `ANTHROPIC_API_KEY`. It verifies authenticated discovery, live chat-models metadata, absence of the secret in that response, cached metadata after the endpoint closes, and cache invalidation after endpoint and credential configuration changes.

Final validation (all exited 0):

- `cargo test --workspace --all-features --test settings_execution`: all 7 tests passed, including both captured-chat cases.
- `cargo test --workspace --all-features --test contracts chat_models_discovery_uses_resolved_configuration_and_invalidates_cache`: the new integration test passed.
- `cargo test --workspace --all-features --test contracts discovery`: discovery checks passed, including live mapping, pagination, configuration-based caching, no-key/failure fallback, and the new configured-endpoint integration test.
- Foreground `just test`: formatting, all Rust suites, all 617 frontend tests across 83 files, and the production frontend build passed. Log: `/tmp/spark-cr-0127-just-test.log`. The build emitted a Node deprecation warning and its large-chunk advisory.
- `git diff --check`: passed.

The full gate includes catalog compatibility, native request fields and omission, compatible-provider declarations, endpoint metadata, picker behavior, and mixed-case effort preservation through chat and workflow dispatch. Existing catalog fields were compared against the original catalog and preserved in the earlier implementation review.

Provider documentation reverified on 2026-09-27:

- [OpenAI reasoning](https://developers.openai.com/api/docs/guides/reasoning), [GPT-6 Astra](https://developers.openai.com/api/docs/models/gpt-6-astra), [Sol](https://developers.openai.com/api/docs/models/gpt-6-sol), [Luna](https://developers.openai.com/api/docs/models/gpt-6-luna), [GPT-5.5](https://developers.openai.com/api/docs/models/gpt-5.5), [GPT-5.2](https://developers.openai.com/api/docs/models/gpt-5.2), [GPT-5.2 Codex](https://developers.openai.com/api/docs/models/gpt-5.2-codex).
- [Gemini model levels and defaults](https://ai.google.dev/gemini-api/docs/thinking), [generateContent ThinkingConfig](https://ai.google.dev/api/generate-content#ThinkingConfig).
- [Anthropic model discovery](https://platform.claude.com/docs/en/api/models/list), [effort support and defaults](https://platform.claude.com/docs/en/build-with-claude/effort).

Existing catalog fields and model ordering are retained. The legacy `gpt-5.2-mini` entry has no verified effort declaration (its official model page returned 404). Defaults not established by the fetched documentation remain absent rather than guessed.

## Shipped Changes

- Catalog `reasoning_efforts` replaces `supports_reasoning`; capability checks derive from non-empty levels. Added current GPT-6, GPT-5.5, and Gemini Flash entries and catalog-owned native provider fallback lists.
- Anthropic models and effort capabilities are discovered with authenticated, paginated GET requests. Discovery uses resolved `spark.toml` provider settings and the existing execution environment mapping. Success and catalog fallback results are cached for five minutes, keyed by the resolved provider configuration. Missing keys make no request.
- Anthropic sends `output_config.effort`, OpenAI retains `reasoning.effort`, and Gemini sends `generationConfig.thinkingConfig.thinkingLevel`. Existing thinking options remain intact. Unlisted values reach the provider unchanged.
- `llm-profiles.toml` supports `reasoning_efforts = ["low", "high"]` for `openai_compatible`, `openrouter`, and `litellm` profiles. Declared profiles send Chat Completions `reasoning_effort`; explicit effort without a declaration fails with an error naming the setting.
- Chat-models exposes levels, defaults, provider fallback lists, and profile identity. Workspace settings can fetch the same metadata without a project. The picker keeps profiles distinct, preserves provider level names, and labels unverified fallback levels.
- Chat, workflow node/launch/fallback resolution, and CLI launch payloads preserve mixed-case API effort strings. Regression coverage captures compatible-provider HTTP request bodies with declared levels and an unlisted `FutureEffort`, through chat and both workflow execution modes. Codex and Claude Code retain their existing execution normalization.
- No thinking budgets, reasoning mode, summaries, thinking-off controls, dependencies, or stored model-settings fields were added.
