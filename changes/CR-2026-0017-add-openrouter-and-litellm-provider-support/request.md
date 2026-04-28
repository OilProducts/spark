# Add OpenRouter and LiteLLM Provider Support

## Summary
Add `openrouter` and `litellm` as first-class Spark/Attractor inference providers while reusing the existing OpenAI-compatible Chat Completions adapter path. OpenRouter gets a known hosted default base URL and API-key preflight. LiteLLM represents a user-operated proxy, so it requires `LITELLM_BASE_URL`, allows missing `LITELLM_API_KEY`, and requires explicit model selection.

External API assumptions are grounded in OpenRouter’s Chat Completions/auth docs and LiteLLM’s OpenAI-compatible proxy docs:
- OpenRouter: `https://openrouter.ai/api/v1/chat/completions`, bearer `OPENROUTER_API_KEY`, optional attribution headers.
- LiteLLM: OpenAI-compatible proxy base URL, commonly a user-hosted `/v1/chat/completions` endpoint.

## Key Changes
- In `unified_llm`, add `OpenRouterAdapter` and `LiteLLMAdapter` as thin `OpenAICompatibleAdapter` wrappers with provider identities `openrouter` and `litellm`.
- Register both in `Client.from_env()`:
  - `openrouter` when `OPENROUTER_API_KEY` is present, default base URL `https://openrouter.ai/api/v1`, optional `OPENROUTER_BASE_URL`, `OPENROUTER_HTTP_REFERER`, and `OPENROUTER_TITLE`.
  - `litellm` when `LITELLM_BASE_URL` is present, optional `LITELLM_API_KEY`, no localhost default.
- Add conservative agent profiles for `openrouter` and `litellm` that reuse OpenAI-compatible tools but do not inject OpenAI Responses-only provider options.
- Extend Attractor and project chat provider validation/routing so `llm_provider=openrouter` and `llm_provider=litellm` run through `UnifiedAgentBackend`.
- Keep model selection explicit for both new providers. Do not add model catalog defaults for OpenRouter or LiteLLM in this pass; frontend suggestions may include example OpenRouter model IDs but should not force defaults.
- Update frontend provider suggestion lists to include `openrouter` and `litellm`, including Settings, graph/node LLM fields, project chat provider selector, and model suggestion tests.
- Update provider-enumerating specs/docs to mention the new OpenAI-compatible providers, their env vars, and the explicit-model rule.

## Test Plan
- Backend/unit:
  - `uv run pytest -q tests/test_client.py tests/adapters/test_openai_compatible_adapter.py`
  - Add/adjust tests for env registration, OpenRouter headers/base URL, LiteLLM optional API key, and provider identity in normalized responses/errors.
- Agent/Attractor/workspace:
  - `uv run pytest -q tests/api/test_backend_invariance.py tests/api/test_project_chat.py tests/api/test_manager_loop_pipeline_api.py`
  - Cover missing `OPENROUTER_API_KEY`, missing `LITELLM_BASE_URL`, routing through `ProviderRouterBackend`, and project chat session creation for both providers.
- Frontend:
  - Run targeted Vitest coverage for provider/model suggestions and provider selectors.
  - Run `npm --prefix frontend run test:unit -- --run` if that is the repo’s accepted frontend unit command.
- Final gate:
  - `uv run pytest -q`
  - `npm --prefix frontend run test:unit` if frontend files changed.

## Assumptions
- `LITELLM_API_KEY` is optional; only `LITELLM_BASE_URL` is required.
- OpenRouter and LiteLLM are first-class provider IDs, not anonymous `openai_compatible` aliases.
- OpenRouter/LiteLLM models are deployment- or account-specific enough that flows should provide `llm_model` explicitly.
- No UI redesign is needed; existing free-text provider/model inputs and datalist suggestions are sufficient.
