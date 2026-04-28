---
id: CR-2026-0017-add-openrouter-and-litellm-provider-support
title: Add OpenRouter and LiteLLM Provider Support
status: completed
type: feature
changelog: public
---

## Summary
Implemented OpenRouter and LiteLLM as first-class OpenAI-compatible inference providers for Spark and Attractor. OpenRouter uses `OPENROUTER_API_KEY` with a hosted default base URL and optional attribution headers; LiteLLM uses an explicit `LITELLM_BASE_URL` with optional `LITELLM_API_KEY`. Both providers require explicit model selection and route through the existing OpenAI-compatible Chat Completions adapter path.

## Validation
- `uv run pytest -q tests/test_client.py tests/adapters/test_openai_compatible_adapter.py tests/agent/test_provider_profiles.py`
- `uv run pytest -q tests/api/test_backend_invariance.py tests/api/test_project_chat.py tests/api/test_manager_loop_pipeline_api.py`
- `npm --prefix frontend run test:unit -- --run src/lib/__tests__/llmSuggestions.test.ts`
- `docker compose config --quiet`
- `uv run pytest -q`
- `npm --prefix frontend run test:unit`

All recorded validation commands passed.

## Shipped Changes
- Added `OpenRouterAdapter` and `LiteLLMAdapter` wrappers over the OpenAI-compatible Chat Completions adapter, including provider identities, base URL handling, OpenRouter attribution headers, and LiteLLM optional API key behavior.
- Registered `openrouter` and `litellm` in `Client.from_env()` and exported them through unified LLM adapter/package surfaces.
- Added conservative OpenAI-compatible agent profile factories for both providers without OpenAI Responses-only reasoning options.
- Extended Attractor backend routing, workflow preflight, and project chat session handling to accept `openrouter` and `litellm`, reject missing required provider configuration, and enforce explicit models.
- Updated frontend provider/model suggestions and project provider labels for the new provider IDs.
- Updated README, Docker Compose environment pass-through, and unified LLM specs to document the new providers, environment variables, and explicit-model rule.
- Added backend, agent, API, project chat, and frontend unit coverage for the new provider behavior.
