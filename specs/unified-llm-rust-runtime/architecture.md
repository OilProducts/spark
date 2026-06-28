# Unified LLM Rust Runtime Architecture

This architecture implements the Unified LLM Client Specification in the Rust rewrite worktree. The product behavior and external contract come from the committed runtime source specification at `specs/unified-llm-rust-runtime/source.md`, backed by the upstream committed source specification at `specs/unified-llm-spec-md/source.md`. The extracted requirement ledgers are `specs/unified-llm-rust-runtime/requirements.json` and `specs/unified-llm-spec-md/requirements.json`.

The implementation target is a Rust-owned normal runtime path for Spark server and CLI LLM execution. The existing Python `src/unified_llm` package may remain as an oracle, historical compatibility surface, and package data location, but normal Spark server and CLI execution must not import or delegate provider calls to `src.unified_llm.adapters` or `src.unified_llm.provider_utils`.

## Canonical Repository Topology

`crates/unified-llm-adapter` is the canonical Rust implementation of the unified LLM runtime. It must not become a thin Python bridge. Its public crate API owns the request, response, provider, streaming, retry, tool, structured-output, and provider-environment contracts used by Spark runtime code.

Recommended crate topology:

| Path | Ownership |
| --- | --- |
| `crates/unified-llm-adapter/src/lib.rs` | Public exports only. Re-export stable API names and compatibility aliases where explicitly retained. |
| `crates/unified-llm-adapter/src/request.rs` or `src/types.rs` | Layer 1 provider specification DTOs: `Request`, `Response`, `Message`, `Role`, `ContentPart`, media data, tools, response formats, finish reasons, warnings, rate limits. Existing `LlmRequest` and `LlmResponse` names may remain as aliases during migration, but they are not separate contracts. |
| `crates/unified-llm-adapter/src/usage.rs` | `Usage`, usage aggregation, cost estimates, cache and reasoning token accounting. |
| `crates/unified-llm-adapter/src/events.rs` | `StreamEvent`, `StreamEventType`, `StreamAccumulator`, `StreamResult` support types, and final response accumulation. |
| `crates/unified-llm-adapter/src/errors.rs` | Unified error taxonomy, HTTP/gRPC/message classifiers, retryability flags, raw provider error retention. |
| `crates/unified-llm-adapter/src/catalog.rs` | Advisory model catalog lookup and capability filtering. Runtime loads JSON through Rust resource APIs, not by importing Python. |
| `crates/unified-llm-adapter/src/env.rs` | Provider environment resolution for OpenAI, Anthropic, Gemini, OpenRouter, LiteLLM, and explicit OpenAI-compatible endpoints. |
| `crates/unified-llm-adapter/src/profiles.rs` and `src/resolution.rs` | Spark-specific profile/resource metadata and launch-time provider/model/reasoning resolution used by Rust callers. |
| `crates/unified-llm-adapter/src/client.rs` | Layer 3 `Client`, provider registry, deterministic routing, default provider handling, lifecycle, and concurrency-safe adapter storage. |
| `crates/unified-llm-adapter/src/defaults.rs` | Module-level default client with lazy environment initialization and `set_default_client`. |
| `crates/unified-llm-adapter/src/middleware.rs` | Complete and stream middleware traits and onion execution helpers. |
| `crates/unified-llm-adapter/src/provider_utils/` | Layer 2 shared utilities: HTTP transport, SSE parser, media normalization, provider error parsing, finish-reason and usage normalization, retry helpers, JSON schema helpers. |
| `crates/unified-llm-adapter/src/adapters/` | Layer 1 implementations: `openai`, `anthropic`, `gemini`, `openai_compatible`, `openrouter`, and `litellm`. Native providers use their native APIs; OpenAI-compatible providers use Chat Completions in a separate adapter path. |
| `crates/unified-llm-adapter/src/generation.rs` | Layer 4 high-level `generate` and `stream` orchestration, retries, prompt normalization, timeouts, abort handling, and step results. |
| `crates/unified-llm-adapter/src/tools.rs` | Tool definitions, tool choice, tool call extraction, active/passive tool loop helpers, argument validation, repair hooks. |
| `crates/unified-llm-adapter/src/structured.rs` | `generate_object`, `stream_object`, provider structured-output request setup, local JSON parsing and validation. |
| `crates/unified-llm-adapter/src/timeouts.rs` | Timeout config and adapter timeout defaults. |

`crates/spark-agent-adapter`, `crates/spark-cli`, and `crates/spark-server` are consumers of the Rust-owned interface. Normal execution must route through the Rust client or Rust adapter boundary, not through Python provider clients.

`src/unified_llm` remains a compatibility and oracle package while the Rust migration is underway. It can continue to host Python compatibility tests and historical package data, including `src/unified_llm/data/models.json` while that file is loaded by Rust through `spark-assets`. Runtime behavior must not depend on importing Python code from this package.

`tests/adapters` and `tests/compat/providers` remain behavioral validation surfaces. Tests may compare Python oracle fixtures to Rust observations, but final runtime ownership is proven through Rust-facing interfaces and mocked or recorded provider interactions.

`docs/rust-rewrite-migration.md` records the migration state, retained compatibility surfaces, validation evidence, and any intentionally deferred provider capabilities.

## Implementation Boundaries

The four specification layers map to the repository as follows:

| Spec layer | Rust implementation |
| --- | --- |
| Layer 1: Provider specification | Public DTOs plus `ProviderAdapter`. Adding a provider must require implementing the trait and registration, not changing shared DTO semantics. |
| Layer 2: Provider utilities | Reusable HTTP, SSE, media, normalization, error, retry, and schema helpers under `provider_utils/`. These helpers must be callable by adapters without depending on high-level orchestration. |
| Layer 3: Core client | `Client` stores `Arc<dyn ProviderAdapter + Send + Sync>`, resolves the target provider, applies middleware, and calls adapters. It holds no mutable per-request state. |
| Layer 4: High-level API | `generate`, `stream`, `generate_object`, and `stream_object` build requests, apply retries, run active tool loops, aggregate usage, manage cancellation and timeouts, and return high-level results. |

Low-level `Client.complete` and `Client.stream` never retry automatically. Retry behavior lives in high-level APIs and in standalone retry utilities.

Provider adapters own all provider-specific HTTP details. They must preserve raw provider JSON in responses and raw provider errors in errors. They must also preserve unknown provider event payloads as `PROVIDER_EVENT` stream events where the event cannot be normalized.

## Public Interface

The documented public Rust surface is:

- `ProviderAdapter`: name, `complete`, `stream`, and optional `initialize`, `close`, and `supports_tool_choice`.
- `Client`: programmatic construction, `from_env`, provider registry, middleware registration, `complete`, `stream`, and `close`.
- Default-client functions: lazy environment-backed default client, `set_default_client`, and per-call explicit client overrides.
- Core data types: `Request`, `Response`, `Message`, `Role`, `ContentPart`, `ContentKind`, `ImageData`, `AudioData`, `DocumentData`, `ToolCallData`, `ToolResultData`, `ThinkingData`, `Tool`, `ToolChoice`, `ToolCall`, `ToolResult`, `ResponseFormat`, `FinishReason`, `Usage`, `Warning`, `RateLimitInfo`, `StreamEvent`, and `StreamEventType`.
- High-level APIs: `generate`, `stream`, `generate_object`, `stream_object`, `StreamAccumulator`, `GenerateResult`, `StepResult`, and `StreamResult`.
- Environment and catalog APIs: `ProviderEnvironment`, `ProviderConfig`, `get_model_info`, `list_models`, and `get_latest_model`.
- Error and retry APIs: canonical `SDKError`, `ProviderError`, `SDKErrorKind`, provider error classifiers, `RetryPolicy`, and delay calculation helpers. Existing Rust-facing `AdapterError` and `AdapterErrorKind` names may remain as compatibility aliases or re-exports only if they expose the same data, classification, serialization, and retryability behavior as the canonical names.

Rust type names should match the specification where practical. If existing Spark callers already use `LlmRequest` or `LlmResponse`, those names may be kept as type aliases to the canonical `Request` and `Response` types. They must not fork behavior or serialization.

The serialized JSON form is part of the contract for tests and cross-language compatibility. Rust may use idiomatic enum or struct internals, but external serialization must preserve the spec fields: for example `FinishReason` carries both unified `reason` and provider `raw`, and `StreamEvent` carries a `type` discriminator plus optional text, reasoning, tool, finish, usage, response, error, and raw fields.

## Provider Routing And Configuration

`Client.from_env` registers only configured providers. Registration and default-provider selection follow this deterministic order: `openai`, `anthropic`, `gemini`, `openrouter`, `litellm`, `openai_compatible`. Explicit default providers are normalized case-insensitively and must refer to a configured provider.

Provider-specific environment handling:

- OpenAI: `OPENAI_API_KEY`; optional `OPENAI_BASE_URL`, `OPENAI_ORG_ID`, `OPENAI_PROJECT_ID`.
- Anthropic: `ANTHROPIC_API_KEY`; optional `ANTHROPIC_BASE_URL`.
- Gemini: `GEMINI_API_KEY`, falling back to `GOOGLE_API_KEY`; optional `GEMINI_BASE_URL`.
- OpenRouter: `OPENROUTER_API_KEY`; optional `OPENROUTER_BASE_URL`, `OPENROUTER_HTTP_REFERER`, `OPENROUTER_TITLE`.
- LiteLLM: `LITELLM_BASE_URL`; optional `LITELLM_API_KEY`.
- Explicit OpenAI-compatible local endpoint: `OPENAI_COMPATIBLE_BASE_URL`; optional `OPENAI_COMPATIBLE_API_KEY`.

Requests route by explicit `request.provider` when present. Otherwise they use `client.default_provider`. If neither is available, the client raises `ConfigurationError`. The client never infers provider from the model string.

The model catalog is advisory. Unknown model IDs pass through to providers. `get_latest_model` may return a default model only for native providers with catalog entries; OpenRouter and LiteLLM require explicit model selection and do not use catalog defaults.

## Model, Profile, And Launch Resolution

Low-level `Client.complete` and `Client.stream` receive a fully resolved `Request`; `Request.model` is required and must be non-empty at that boundary. This preserves the serialized provider contract and prevents low-level calls from hiding ambiguous provider/model selection.

High-level entry points may accept an omitted model to honor the source spec's latest-model default behavior. Before constructing a `Request`, `generate`, `stream`, `generate_object`, and `stream_object` resolve provider and model in this order:

1. Resolve provider from explicit call parameter or request field, then an active Spark LLM profile, then the client default provider.
2. Resolve model from an explicit call parameter, then the active Spark LLM profile `default_model`, then the provider catalog latest model for native `openai`, `anthropic`, or `gemini`.
3. If the resolved provider is `openrouter`, `litellm`, or `openai_compatible`, require an explicit model or profile `default_model`; these providers do not use catalog latest defaults.
4. If provider or model still cannot be resolved, raise `ConfigurationError`.

Capability-aware APIs such as `generate_object` and calls with image/tool requirements may ask `get_latest_model(provider, capability)` for a native-provider default that satisfies the needed capability. Unknown explicit model strings continue to pass through and are not rejected by the advisory catalog.

Spark-specific profile resolution is Rust-owned runtime behavior, not only metadata. `llm-profiles.toml` is loaded from Spark's config directory and currently supports configured OpenAI-compatible profiles:

- `[profiles.<id>]` requires `provider = "openai_compatible"`, `base_url`, and a non-empty `models` list.
- Optional `label`, `api_key_env`, and `default_model` are accepted. `default_model`, when present, must be listed in `models`.
- Public profile metadata exposes `id`, `label`, `provider`, `models`, `default_model`, and `configured`; it must not expose `base_url` or secret values.
- A profile with `api_key_env` is configured only when that environment variable is non-empty. A missing key blocks profile-backed adapter construction with a configuration error; profiles without `api_key_env` are valid local endpoints.
- A profile-backed request uses the OpenAI-compatible adapter with the profile base URL and optional profile API key. The runtime may normalize a profile id supplied through a provider selector to the profile's provider, but the selected profile id remains part of runtime metadata and session identity.

Spark launch context resolution uses these keys: `_attractor.runtime.launch_model`, `_attractor.runtime.launch_provider`, `_attractor.runtime.launch_profile`, and `_attractor.runtime.launch_reasoning_effort`. Pipeline launch prepares those keys from explicit launch parameters and graph defaults such as `ui_default_llm_model`, `ui_default_llm_provider`, and `ui_default_llm_profile`. Node execution then resolves effective values in this order: node attributes produced by explicit node settings or model stylesheet, runtime launch context, backend fallback, then the applicable client/provider default. Provider and reasoning effort values are normalized lowercase. The display placeholder `codex default (config/profile)` is treated as omitted for model resolution.

Reasoning effort has one special compatibility rule: generated default placeholder node attributes do not override a launch-time reasoning effort. Rust resolution helpers must carry an explicit marker, such as `node_reasoning_is_default_placeholder`, rather than inferring this from source text.

## Provider Adapter Architecture

OpenAI native adapter:

- Uses the Responses API at `/v1/responses`.
- Does not use Chat Completions for native OpenAI.
- Extracts system/developer instructions into `instructions`.
- Maps `reasoning_effort` to `reasoning.effort`.
- Supports Responses API built-in tools and other body features through `provider_options.openai`.
- Parses output items, tool calls, usage, reasoning tokens, cache read tokens, rate-limit headers, errors, and streaming events.

Anthropic adapter:

- Uses the Messages API at `/v1/messages`.
- Extracts system messages and merges developer instructions into the system parameter.
- Enforces Anthropic user/assistant alternation by merging consecutive same-role messages.
- Defaults `max_tokens` to 4096 when omitted.
- Translates tool results into user-role `tool_result` content blocks.
- Preserves `thinking` and `redacted_thinking` blocks, including signatures and opaque redacted data.
- Joins `provider_options.anthropic.beta_headers` into `anthropic-beta`.
- Implements prompt caching and automatic cache-control injection unless disabled.

Gemini adapter:

- Uses Gemini native `generateContent` and streaming endpoints under `/v1beta/models/*`.
- Sends the API key as the provider expects and uses `systemInstruction`.
- Maps assistant to Gemini `model` role and tool results to `functionResponse`.
- Generates synthetic stable tool call IDs for function calls and recovers the function name from message history when sending follow-up tool results.
- Passes Gemini body options such as safety settings, grounding, thinking config, cached content, and generation config through `provider_options.gemini`.

OpenAI-compatible adapter:

- Uses `/v1/chat/completions`, not `/v1/responses`.
- Exists only for third-party compatibility protocols.
- Documents unsupported Responses-only features such as reasoning token visibility, built-in OpenAI Responses tools, and server-side conversation state.
- `OpenRouterAdapter` and `LiteLLMAdapter` are named configurations over this adapter, with their own environment and header behavior.
- The compatible adapter may be scaffolded during provider translation, but full `REQ-022` completion is not claimed until Chat Completions response translation, error translation, and streaming translation are validated after the M3 error and streaming foundation exists.

## Data Translation Contracts

The unified message model supports `SYSTEM`, `USER`, `ASSISTANT`, `TOOL`, and `DEVELOPER`. `Message.text` concatenates all text content parts. Direction constraints are enforced at construction where possible and at provider translation boundaries where the provider-specific context is needed.

`ContentPart` supports text, image, audio, document, tool call, tool result, thinking, redacted thinking, and custom provider-specific kinds. Custom kinds are represented as explicit provider/custom variants with raw JSON payloads so new provider features do not require changing the enum for every release.

Image inputs support URL, raw bytes, and local path convenience. `ImageData` validates that exactly one source is provided after local-path normalization. Raw image data defaults to `image/png` when no media type is supplied. Local paths beginning with `/`, `./`, or `~` are read by provider utilities, MIME type is inferred from extension, and bytes are base64 encoded into the provider's inline image format. Unsupported media kinds are rejected with provider-specific errors or warnings without corrupting other content.

Provider options are an explicit escape hatch. Each adapter reads only its provider namespace and either applies, ignores, or warns about unsupported keys. Options for other providers must remain unmodified on the request and must not leak into the active provider body or headers.

Prompt caching:

- OpenAI and Gemini automatic cache statistics are mapped into `Usage.cache_read_tokens`.
- Anthropic supports explicit `cache_control` content annotations.
- Anthropic automatic caching is enabled by default. The heuristic marks, in priority order and within provider limits: the merged system prompt, tool definitions, and the stable conversation prefix before the current user turn. Explicit user-provided cache controls are preserved and take precedence.
- `provider_options.anthropic.auto_cache = false` disables automatic injection.
- Whenever Anthropic cache controls are present, `prompt-caching-2024-07-31` is included in `anthropic-beta` unless already present.

Reasoning and thinking:

- OpenAI reasoning token accounting comes from Responses API usage details; visible reasoning text is not expected.
- Anthropic thinking and redacted thinking content round-trip as content parts. Reasoning token counts are estimated from thinking content when exact counts are unavailable.
- Gemini `thoughtsTokenCount` maps to `Usage.reasoning_tokens`; thought parts are normalized to thinking content when returned.

## Streaming, Errors, And Retry

The stream model follows start/delta/end lifecycles for text, reasoning, and tool calls. Each successful stream terminates with `FINISH` carrying finish reason, usage, and the complete accumulated response. Consumers filtering only `TEXT_DELTA` must be able to concatenate deltas into the same text carried by the final response.

The shared SSE parser must handle event lines, multi-line data, retry lines, comments, and blank event boundaries. Gemini JSON chunks are handled through provider utilities and normalized into the same `StreamEvent` model.

Initial streaming connection failures are ordinary provider errors and can be retried by high-level `stream` before any data is delivered. Once any stream event has been yielded, the runtime must not silently retry. Later failures surface as an `ERROR` event or stream error and terminate the stream.

Errors use the unified taxonomy. Public Rust APIs expose canonical spec names `SDKError` and `ProviderError`; `AdapterError` is only a compatibility alias or re-export when retained. Provider errors preserve provider name, status code, provider error code, retryability, retry-after, and raw body. HTTP, Gemini gRPC, and message-based classifiers are shared utilities. Unknown provider errors default to retryable.

Retry policy implements exponential backoff with optional jitter and `Retry-After` override. `Retry-After` above `max_delay` prevents retry and returns the original error. High-level APIs retry individual LLM calls, not whole multi-step operations. `generate_object` does not retry schema validation failures.

## High-Level Generation And Tools

`generate` accepts either `prompt` or `messages`, never both. `prompt` becomes a single user message, and `system` is prepended as a system message. The function may accept an omitted model only at this high-level boundary; model omission must be resolved before building the low-level `Request`. The function returns `GenerateResult` with final-step usage and aggregate `total_usage`.

`stream` accepts the same generation inputs and returns `StreamResult`, including text-only streaming, partial response state, and final accumulated response.

Rust tool execution uses an explicit invocation object instead of Python-style signature introspection. A tool handler receives parsed arguments plus context fields such as current messages, abort signal, and tool call ID. This preserves the spec's injected-context behavior in a Rust-native API.

Active tools have handlers and participate in automatic loops. Passive tools have no handlers and return tool calls to the caller. Multiple tool calls from one model step are executed concurrently, all results are awaited, ordering follows the original tool call order, and all results are sent in a single continuation request. Unknown tools and handler failures become `is_error = true` tool results rather than aborting sibling calls.

`stream` with active tools emits model stream events, step-finish boundaries between model calls, and subsequent model events as one continuous stream.

## Structured Output

`ResponseFormat` supports text, JSON, and JSON Schema with strict mode. `generate_object` and `stream_object` always perform local parsing and schema validation before returning structured output.

Provider strategies:

- OpenAI: native Responses API JSON Schema response format where supported.
- Gemini: native `responseMimeType: application/json` and `responseSchema`.
- Anthropic: tool-based extraction with a forced/named tool when tool choice is supported; otherwise schema instructions are injected and the output is parsed locally.

Parse and validation failures raise `NoObjectGeneratedError` and are not retried by default.

## Validation Strategy

The repository validation command for this run is `uv run pytest -q`. Milestone validation may add targeted Rust checks such as `cargo test -p unified-llm-adapter` or `cargo test --workspace --all-targets` as supplemental evidence, but final completion must still pass `uv run pytest -q` unless the user explicitly changes the policy.

Validation must be behavioral:

- Unit tests cover data model accessors, validation, usage aggregation, error classification, retry delay calculation, provider routing, and middleware ordering.
- Mock-transport provider tests cover request translation, response translation, streaming translation, provider options, rate-limit headers, prompt caching, reasoning tokens, and errors for OpenAI, Anthropic, Gemini, OpenAI-compatible, OpenRouter, and LiteLLM.
- Cross-provider parity tests cover text, streaming, image URL/base64/local path, tools, structured output, reasoning, provider options, usage, and caching behavior.
- Tool-loop tests cover passive tools, active tools, parallel execution, ordered batched continuation, unknown tools, handler errors, repair behavior, round limits, streaming step boundaries, cancellation, and timeouts.
- Structured-output tests cover native OpenAI/Gemini strategies, Anthropic fallback/tool strategy, local JSON Schema validation, stream partial parsing, and `NoObjectGeneratedError`.
- Live provider smoke tests are optional and gated by explicit credentials and markers; they must not fail ordinary local test runs.
- Python compatibility fixtures may remain as oracle tests, but Rust runtime tests must exercise Rust-owned interfaces.

Tests must not assert source text, prompt wording, or deprecated implementation details when observable behavior is unchanged.

## Repository Hygiene

Implementation must stay in the Rust rewrite worktree and in the paths needed to support the runtime contract. Do not add test-only bootstrap paths, environment-specific hacks, shell wrappers around Python provider clients, or duplicate delivery layers.

Generated caches, `__pycache__`, build output, credential files, live API logs, and local smoke-test artifacts are not deliverables. Secrets must not be committed.

Dependency additions must be justified by runtime need. HTTP and async dependencies should use existing workspace-compatible choices where possible, such as `reqwest` with Rustls for provider HTTP and `futures-util` for stream composition.

Documentation updates in `docs/rust-rewrite-migration.md` must identify Rust-owned runtime surfaces, retained Python compatibility status, validation evidence, and any explicit non-goals or future work.

## Requirement Dependencies And Milestones

| Milestone | Requirement IDs | Dependency summary |
| --- | --- | --- |
| M1 core contract | REQ-001, REQ-002, REQ-003, REQ-004, REQ-005, REQ-007 | Establish Rust crate ownership, provider contract, routing, model catalog, middleware, content model, request/response/usage metadata. |
| M2 provider translation | REQ-006, REQ-008, REQ-009, REQ-010, REQ-011, REQ-012 | Depends on M1 types. Implements media handling, native provider request/response translation, options, caching, and reasoning. May add OpenAI-compatible request/response scaffolding, but does not close REQ-022. |
| M3 retry streaming | REQ-013, REQ-014, REQ-015, REQ-016, REQ-022 | Depends on M1/M2 response and error types. Implements error taxonomy, retry policy, unified streaming model, provider streaming parsers, and completes OpenAI-compatible Chat Completions streaming/error behavior. |
| M4 high-level tools | REQ-017, REQ-018, REQ-019, REQ-020, REQ-021 | Depends on routing, retry, streaming, and provider translation. Implements generate/stream orchestration, cancellation/timeouts, tools, loops, and structured output. |
| M5 migration validation | REQ-023, REQ-024 | Depends on provider and high-level behavior. Proves cross-provider parity, documents Rust ownership, and records validation evidence. |

Downstream implementation items must bind to the relevant committed contract decisions in `specs/unified-llm-rust-runtime/contract-decisions.json`, backed by upstream decisions in `specs/unified-llm-spec-md/contract-decisions.json`. A material spec-versus-architecture conflict must be resolved there before implementation depends on it.
