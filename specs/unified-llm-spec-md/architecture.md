# Unified LLM Client Implementation Architecture

## Source of Truth

This architecture implements `.spark/spec-implementation/current/spec/source.md` and the requirement ledger in `.spark/spec-implementation/current/spec/requirements.json`. The current repository is a post-M7 implementation with package code, tests, and spec ledgers already present. This run is a remediation pass for `REQ-024`, which belongs to the already-completed M3 high-level generation, streaming, retry, tools, and structured-output scope. It must not introduce a new M8 milestone.

## Canonical Repository Topology

The implementation uses a standard `src/` package layout:

```text
pyproject.toml
src/
  unified_llm/
    __init__.py
    adapters/
      __init__.py
      anthropic.py
      base.py
      gemini.py
      openai.py
      openai_compatible.py  # includes OpenRouterAdapter and LiteLLMAdapter wrappers
    client.py
    data/
      models.json
    defaults.py
    errors.py
    generation.py
    middleware.py
    models.py
    provider_utils/
      __init__.py
      anthropic.py
      errors.py
      gemini.py
      http.py
      media.py
      normalization.py
      openai.py
      openai_compatible.py
      sse.py
    retry.py
    streaming.py
    structured.py
    timeouts.py
    tools.py
    types.py
tests/
  adapters/
    test_anthropic_adapter.py
    test_cross_provider_parity.py
    test_gemini_adapter.py
    test_openai_adapter.py
    test_openai_compatible_adapter.py
  test_adapter_contract.py
  test_client.py
  test_errors.py
  test_generate.py
  test_middleware.py
  test_models.py
  test_provider_utils.py
  test_retry.py
  test_sse.py
  test_stream_high_level.py
  test_streaming.py
  test_structured.py
  test_tool_loop.py
  test_tools.py
```

`pyproject.toml` owns package metadata, the `src` layout, pytest configuration, ruff configuration, and runtime dependencies. Runtime dependencies should be intentionally small: `httpx` for async HTTP and `jsonschema` for JSON Schema validation. Test and lint tooling is exposed through uv with `pytest`, `pytest-asyncio`, and `ruff`.

## Implementation Boundaries

The package follows the four-layer model from the spec.

Layer 1, provider specification and shared types:

- `types.py` contains the unified dataclasses and string enums for messages, content parts, requests, responses, usage, finish reasons, stream events, warnings, and rate-limit metadata.
- `errors.py` contains the SDK error hierarchy and provider error classification types.
- `adapters/base.py` defines the `ProviderAdapter` protocol plus optional lifecycle and capability hook protocols.

Layer 2, provider utilities:

- `provider_utils/` contains HTTP helpers, media preparation, provider option isolation, finish-reason and usage normalization, rate-limit parsing, provider error construction, and SSE parsing.
- `retry.py` contains `RetryPolicy`, delay calculation, and retry wrappers. It is public because applications using low-level client methods may opt in explicitly.
- `streaming.py` contains common stream event accumulation and low-level stream helpers.

Layer 3, core client:

- `client.py` owns provider registration, provider resolution, routing, middleware application, `Client.complete`, `Client.stream`, and lifecycle closing.
- `middleware.py` defines typed middleware call signatures for complete and stream operations.
- `defaults.py` owns lazy module-level default client management.
- The core client does not construct provider-native HTTP payloads. It delegates those details to adapters.

Layer 4, high-level API:

- `generation.py` owns `generate`, `stream`, `GenerateResult`, `StepResult`, `StreamResult`, prompt standardization, cancellation, timeout enforcement, high-level retries, and multi-step orchestration.
- `tools.py` owns tool definitions, tool choices, tool calls/results, handler invocation, argument validation, context injection, and parallel tool execution.
- `structured.py` owns `generate_object`, `stream_object`, response format construction, JSON parsing, incremental parsing support, and JSON Schema validation.
- `models.py` owns `ModelInfo` and catalog lookup helpers backed by `data/models.json`.

Provider adapter modules are integration boundaries. Each adapter translates unified requests to its provider's native API and translates native responses, stream chunks, usage, errors, and provider-specific capabilities back to unified types. The primary OpenAI adapter uses the Responses API. The OpenAI-compatible adapter is separate and targets Chat Completions only.

## Documented Public Interface

The package root `unified_llm` re-exports the stable API:

- Data model: `Message`, `Role`, `ContentPart`, `ContentKind`, media data classes, `Request`, `Response`, `Usage`, `FinishReason`, `ResponseFormat`, `Warning`, `RateLimitInfo`, `StreamEvent`, and `StreamEventType`.
- Client and defaults: `Client`, `set_default_client`, `get_default_client`.
- High-level operations: `generate`, `stream`, `generate_object`, `stream_object`, `GenerateResult`, `StepResult`, and `StreamResult`.
- Tools: `Tool`, `ToolChoice`, `ToolCall`, and `ToolResult`.
- Catalog helpers: `ModelInfo`, `get_model_info`, `list_models`, and `get_latest_model`.
- Errors: all SDK error classes from `errors.py`.
- Adapters: `OpenAIAdapter`, `OpenAICompatibleAdapter`, `OpenRouterAdapter`, `LiteLLMAdapter`, `AnthropicAdapter`, `GeminiAdapter`, and `ProviderAdapter`.

Python implementation is async-first. Canonical calls are:

```python
response = await client.complete(request)
events = client.stream(request)  # AsyncIterator[StreamEvent]
result = await generate(model="...", prompt="...", provider="...")
stream_result = stream(model="...", prompt="...", provider="...")  # async iterable
object_result = await generate_object(model="...", prompt="...", schema={...})
```

No synchronous duplicate layer is required for this run. Sync wrappers may be added later as convenience APIs if they preserve the async core contract and do not become the primary implementation path.

Provider names are normalized to lowercase strings: `openai`, `anthropic`, `gemini`, `openai_compatible`, `openrouter`, and `litellm`. Model identifiers are never rewritten and are never used to infer providers. A request resolves only through its explicit `provider` or the client's configured default provider. OpenRouter and LiteLLM are first-class OpenAI-compatible providers and require explicit model selection rather than catalog defaults.

High-level operations (`generate`, `stream`, `generate_object`, and `stream_object`) accept `model=None` or an omitted model only after a provider can be resolved from the call's explicit `provider` argument or the selected client's `default_provider`. In that supported omitted-model path, the high-level API calls `get_latest_model(provider).id` and places that provider-native string in every `Request` sent to `Client.complete` or `Client.stream`. Explicit model arguments remain pass-through values and are never replaced by catalog data.

## Data Model and Validation Rules

The shared model uses dataclasses and `str, Enum` values where enum behavior is needed while still allowing provider-specific string extensions. Constructors validate observable invariants:

- `Message.text`, `Response.text`, `Response.tool_calls`, and `Response.reasoning` are derived from content parts.
- Media data validates mutually exclusive URL/data inputs and fills required defaults such as `image/png` for raw image bytes.
- `Usage.__add__` implements the optional-field aggregation rules from the spec.
- Tool names and tool schemas are validated at construction time.
- Error classes avoid names that shadow common Python built-ins.

Validation should reject invalid local inputs before provider calls when the spec defines a deterministic invariant. Provider capability mismatches are represented as SDK errors or warnings, not silent provider-specific fallthrough.

## Provider Adapter Contracts

All native adapters use `httpx.AsyncClient` through an injectable client or transport boundary so tests can mock HTTP deterministically.

OpenAI:

- `OpenAIAdapter` uses `/v1/responses`, not Chat Completions.
- It supports bearer auth, optional base URL, organization, and project headers.
- It maps `reasoning_effort` to Responses API reasoning config and maps reasoning/cache usage from Responses usage fields.
- Built-in tools and other Responses-only features pass through via `provider_options["openai"]`.

OpenAI-compatible:

- `OpenAICompatibleAdapter` is a separate adapter for `/v1/chat/completions`.
- It does not claim Responses-only support. Unsupported features produce warnings or `UnsupportedToolChoiceError` as appropriate.
- `OpenRouterAdapter` and `LiteLLMAdapter` are named wrappers over this path. OpenRouter defaults to `https://openrouter.ai/api/v1` and requires `OPENROUTER_API_KEY`, with optional `OPENROUTER_BASE_URL`, `OPENROUTER_HTTP_REFERER`, and `OPENROUTER_TITLE`. LiteLLM requires `LITELLM_BASE_URL` for a user-operated proxy and allows `LITELLM_API_KEY` to be absent.

Anthropic:

- `AnthropicAdapter` uses `/v1/messages` with `anthropic-version`.
- It extracts system/developer content, merges same-role turns for strict alternation, supports beta headers, preserves thinking signatures and redacted thinking data, and defaults `max_tokens` to 4096 when absent.
- Prompt caching is enabled by adapter support for `cache_control`. Automatic breakpoint injection is on by default for stable agentic prefixes and can be disabled with `provider_options["anthropic"]["auto_cache"] = False`.

Gemini:

- `GeminiAdapter` uses the native Gemini `generateContent` and streaming equivalents, with API key query auth.
- It maps system/developer content to `systemInstruction`, assistant to `model`, and tool results to `functionResponse`.
- It generates synthetic tool call IDs and maintains a concurrency-safe ID-to-function-name mapping for continuation requests, with encoded-name fallback in the generated ID.

Provider options are isolated by provider key. An adapter reads only its own sub-dictionary and ignores other provider keys.

## Generation, Streaming, Tools, and Structured Output

Low-level `Client.complete` and `Client.stream` route to adapters and apply middleware, but never retry automatically.

High-level `generate` standardizes prompt/messages input, prepends system messages, applies per-call retries, executes active tools, aggregates usage, and returns detailed steps. Tool loops execute multiple tool calls concurrently, wait for all results, preserve result order, and append one assistant tool-call message plus all tool-result messages before the next LLM call. Tool handler exceptions and unknown tool calls become error `ToolResult` values so the model can recover.

High-level model selection is resolved once during generation configuration. If the caller omits `model`, the implementation first selects the `Client` object, then resolves a provider from the explicit high-level `provider` argument or that client's `default_provider`, and then asks the advisory model catalog for `get_latest_model(provider)`. If no provider can be resolved or the catalog has no latest entry for that provider, generation raises `ConfigurationError` before constructing a provider request. This preserves explicit provider routing while satisfying the spec's latest-model defaulting rule.

High-level `stream` returns a `StreamResult` that is async iterable, exposes `text_stream`, tracks `partial_response`, and returns the final accumulated response after completion. When active tools create multiple model steps, the stream emits the spec extension event type string `step_finish` between steps.

`StreamAccumulator` reconstructs a `Response` from start/delta/end stream events and is shared by provider streaming tests and high-level stream results.

`generate_object` uses provider-native structured output where available: OpenAI `json_schema`, Gemini `responseSchema`, and Anthropic schema instructions. Final parsing and validation failures raise `NoObjectGeneratedError` and are not treated as transient retryable failures.

## Validation Strategy

The deterministic validation gate is:

```text
uv run pytest -q
uv run ruff check .
```

Tests are behavior-first and avoid assertions against source, prompt, documentation, or spec strings. Coverage is organized by public API behavior, fake adapters, mocked HTTP transports, synthetic provider payloads, filesystem effects for local media preparation, stream event sequences, state transitions, and logging/error outcomes.

REQ-024 remediation tests must cover observable `Request.model` values seen by fake clients or fake adapters for `generate`, `stream`, `generate_object`, and `stream_object`. Each high-level API needs both explicit-provider and default-provider omitted-model coverage, explicit model pass-through coverage, and failure coverage for unresolved provider or absent latest catalog entry.

Provider adapter tests do not require live API keys. They validate outbound native payloads, headers, error translation, streaming normalization, and response normalization with mocked transports.

Optional live smoke tests are marked and skipped unless the matching provider API keys are present. They follow the spec's Definition of Done scenarios for generation, streaming, tools, image input, structured output, errors, reasoning usage, caching usage, and provider option pass-through.

Failure triage should use:

```text
uv run pytest -q -x --maxfail=1 <path-or-nodeid>
```

Before reporting completion of any code change, run the full deterministic suite with `uv run pytest -q` and lint with `uv run ruff check .`.

## Repository Hygiene Expectations

- Library code uses standard library `logging` with module-level loggers and never prints directly.
- Exceptions that are converted to SDK values are logged only where useful for diagnostics. Unexpected parsing, close, middleware, and provider classification failures are logged at an appropriate level before re-raising or preserving actionable error data.
- Tests must not depend on private source text, doc strings, prompt wording, or deprecated behavior.
- No API keys, credentials, local absolute paths, or live network assumptions are committed.
- Compatibility shims are not used as primary provider implementations. Native adapters own native protocol support.
- Shared utilities are used for cross-provider behavior to avoid drift in errors, usage, rate-limit parsing, media preparation, provider options, and SSE handling.

## Requirement Dependencies and Milestone Flow

Milestone M1 creates the package foundation: `REQ-001` scaffold, `REQ-002` message/content types, `REQ-003` request/response/usage/stream types, and `REQ-004` errors.

Milestone M2 builds reusable infrastructure on M1: `REQ-005` adapter protocol, `REQ-006` client/default client routing, `REQ-007` middleware, `REQ-008` model catalog, `REQ-009` provider utilities, and `REQ-010` SSE/stream accumulation.

Milestone M3 builds high-level behavior. Implement `REQ-015` tools, `REQ-016` tool loop, and `REQ-017` retry before finalizing `REQ-012` generate and `REQ-013` high-level stream. `REQ-011` low-level operations can land after M2 and before high-level orchestration. `REQ-014` structured output depends on `REQ-012`. `REQ-024` is a post-M7 audit remediation of this same M3 surface: omitted high-level model values must default through `get_latest_model(provider).id` when the provider is explicit or available from the selected client's default provider.

Milestone M4 implements OpenAI: `REQ-018` native Responses adapter, then `REQ-019` OpenAI-compatible Chat Completions adapter.

Milestone M5 implements `REQ-020` Anthropic Messages with thinking and prompt caching.

Milestone M6 implements `REQ-021` Gemini native API with synthetic tool IDs and Gemini usage normalization.

Milestone M7 validates parity and completion: `REQ-022` cross-provider multimodal/reasoning/caching parity and `REQ-023` deterministic and optional live test coverage.
