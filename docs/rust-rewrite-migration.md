# Rust Rewrite Migration Source Of Truth

This committed document is the migration source of truth for Rust-backed Spark. It records adapter ownership, retained Python modules, deprecated compatibility surfaces, explicit non-goals, and future decision candidates using committed repository files: docs, specs, manifests, crates, and tests. Validation of this document uses committed repository files only. It does not change any user command, HTTP route, SSE payload, storage layout, package name, service workflow, or frontend contract.

## Scope

- Public commands remain `spark` and `spark-server`.
- Rust crates own the contracts already implemented and validated through the rewrite milestones.
- Python modules remain valid where this document classifies them as retained behavior.
- Deprecated compatibility routes remain supported until a later contract decision and validation update approve an incompatible change.
- No open policy gaps remain after the post-gap spec/API drift audit. Explicit non-goals and future compatibility-break candidates are still not counted as closed parity.

## Binding Unified LLM Contract Decisions

| Decision | Documentation boundary |
| --- | --- |
| `CD-ULLM-RUST-001` | Normal Spark server and CLI unified LLM execution is Rust-owned through `crates/spark-server`, `crates/spark-cli`, `crates/attractor-runtime`, `crates/spark-agent-adapter`, and `crates/unified-llm-adapter`; it does not import, shell out to, or wrap `src.unified_llm` provider clients, including `src.unified_llm.adapters` or `src.unified_llm.provider_utils`, for normal provider execution. |
| `CD-ULLM-RUST-015` | Python `unified_llm` checks in `tests/compat/providers` and `tests/adapters` are retained oracle or compatibility support; they do not replace Rust-owned behavioral validation through Rust crate tests and `uv run pytest -q`. |
| `CD-ULLM-RUST-016` | Rust-owned surfaces are model catalog/profile resolution, request DTOs, provider routing, native/compatible provider adapters, streaming, errors, retry, tools, structured output, middleware, timeout, usage, launch/profile wiring, and HTTP transport in `crates/unified-llm-adapter` plus Rust consumers. Retained Python surfaces are compatibility tests, oracle fixture generation, historical adapter checks, and package data under `src/unified_llm`. Unsupported/deferred provider capabilities are live SDK/network error-shape parity beyond normalized public errors, persistent provider cache resources, compatible-provider advisory latest-model defaults, Responses-only compatible-provider features, and currently rejected audio/document inputs. Optional live smoke prerequisites are explicit pytest or Rust live-smoke selection plus provider credentials supplied through process environment variables. |

## Agent Boundary

| Surface | Classification | Current owner | Retained Python status | Evidence |
| --- | --- | --- | --- | --- |
| Turn output DTOs, stream event payloads, usage, raw logs, and thread resume failure payloads | `native_rust` | `crates/spark-agent-adapter` | Python can still produce source events, but Rust owns the serialized boundary contract | `crates/spark-agent-adapter/tests/agent_event_contracts.rs`; M3 validation artifacts |
| Codergen prompt selection, context reads, simulation, and prompt/response/status artifacts | `rust_owned_adapter` | `crates/spark-agent-adapter` | Python codergen behavior remains the compatibility oracle while Rust mirrors the adapter boundary | `agent/codergen-prompt-context-artifacts`; `agent/codergen-simulation-artifacts`; `tests/compat/agent/test_codergen_adapter_fixtures.py` |
| Status envelope resolution and repair prompt behavior | `native_rust` | `crates/spark-agent-adapter` | Rust owns the status-envelope contract; Python codergen remains retained until all callers are migrated | `agent/codergen-status-envelope-resolution`; Rust codergen contract tests |
| Agent session loop, tools, environment, project context, provider profiles, and streaming behavior | `retained_python_module` | `src/agent` | Retained Python implementation guarded by the existing agent pytest suite | `tests/agent/test_session.py`; `tests/agent/test_tool_execution.py`; `tests/agent/test_agent_streaming.py` |

## Unified LLM Boundary

| Surface | Classification | Current owner | Retained Python status | Evidence |
| --- | --- | --- | --- | --- |
| Model catalog, provider environment, profile metadata, and installed provider resources | `native_rust` | `crates/unified-llm-adapter` | Rust mirrors the Python oracle and packaged resources | `providers/model-catalog-env-resolution`; `providers/m6-provider-profile-resource-parity`; Rust adapter/profile tests |
| Request DTOs, tool/structured-output payloads, retry, errors, stream events, and usage/cost normalization | `native_rust` | `crates/unified-llm-adapter` | Rust owns the normal Spark server and CLI contract; Python clients are retained for compatibility and oracle fixture support outside normal Spark provider execution | `providers/request-tool-structured-output`; `providers/retry-error-usage-stream`; adapter contract tests |
| Provider adapter routing from Rust callers to provider implementations | `rust_owned_adapter` | `crates/unified-llm-adapter` | Rust callers route through Rust `ProviderAdapter` implementations; Python provider clients are historical compatibility/oracle surfaces, not wrappers used by Spark server or CLI execution | M3 and M6 provider/model validation artifacts |
| Provider-specific clients, cross-provider streaming normalization, structured-output fallback handling, middleware, and timeouts | `retained_python_module` | `src/unified_llm` | Retained Python modules are guarded only as compatibility, oracle, and package-data support; they are not normal provider execution for Rust-backed Spark | `tests/adapters/test_openai_adapter.py`; `tests/adapters/test_anthropic_adapter.py`; `tests/adapters/test_cross_provider_parity.py` |

## M5 Runtime Wiring, Profiles, And Launch Resolution Status

M5 records the normal Rust-owned runtime path for Spark LLM calls. `crates/spark-server` builds an `attractor_runtime::RuntimeHandlerRunner` with a `unified_llm_adapter::Client` loaded from environment and Spark profile config; `crates/attractor-runtime` passes launch and run-record LLM fallback values into node execution; `crates/spark-agent-adapter` turns codergen and agent-turn requests into low-level `unified_llm_adapter::Request` values; and `crates/unified-llm-adapter` owns provider/profile/model/reasoning resolution before dispatching to registered Rust `ProviderAdapter` instances. The CLI path in `crates/spark-cli` normalizes user-supplied launch, continue, conversation-run, provider, profile, model, and reasoning flags into HTTP payloads consumed by the Rust server/runtime path.

Normal M5 Spark server, CLI, Attractor, codergen, agent-turn, profile, and high-level generation paths do not import, shell out to, or wrap `src.unified_llm.adapters` or `src.unified_llm.provider_utils` for provider execution. The retained Python `src/unified_llm` package remains valid only for compatibility tests, oracle fixture generation, and package data such as the model catalog. Those Python surfaces are retained compatibility/oracle/package-data surfaces, not normal provider execution paths for this milestone.

Source-checkout `uv run spark` and `uv run spark-server` commands are development entry points for the public command surface. They do not change the CD-ULLM-RUST-001 runtime ownership boundary: when normal Spark server or CLI execution reaches unified LLM provider work, that execution remains Rust-owned, while retained Python `unified_llm` code remains compatibility/oracle/package-data support.

Spark profile loading is Rust-owned through `llm-profiles.toml` under the Spark config directory. Supported entries are `[profiles.<id>]` tables with `provider = "openai_compatible"`, a non-empty `base_url`, a non-empty `models` list, optional `label`, optional `api_key_env`, and optional `default_model` constrained to the profile's model list. Public profile metadata exposes only `id`, `label`, `provider`, `models`, `default_model`, and `configured`; it redacts `base_url`, `api_key_env`, and secret values. Profiles with `api_key_env` are configured only when the referenced process environment value is non-empty. Profile-backed adapter construction uses the OpenAI-compatible Chat Completions adapter with the profile endpoint, requires an API key only when `api_key_env` is present, permits local no-key profiles with `require_api_key = false`, and surfaces missing or malformed profile fields as configuration errors.

Launch context resolution is Rust-owned for `_attractor.runtime.launch_model`, `_attractor.runtime.launch_provider`, `_attractor.runtime.launch_profile`, and `_attractor.runtime.launch_reasoning_effort`. Node/style values win before launch context, launch context wins before run/backend fallback values, and fallbacks win before defaults. Provider and reasoning values are normalized lowercase. The display model placeholder `codex default (config/profile)` is treated as omitted for model selection, and generated default reasoning placeholders do not override launch-time reasoning because the runtime carries an explicit default-placeholder marker.

High-level provider and model resolution integrates explicit request values, profile selectors, client defaults, native latest-model fallback, and OpenAI-compatible explicit-model requirements. Profile IDs supplied through provider selectors are preserved in request metadata and session identity while the low-level adapter call routes to the profile provider, currently `openai_compatible`. Native providers `openai`, `anthropic`, and `gemini` may use catalog latest-model fallback when the requested capabilities match; compatible providers `openrouter`, `litellm`, and `openai_compatible` still require an explicit request model or active profile default.

M5 validation evidence is behavioral and credential-free. Rust-facing coverage includes `cargo test -p unified-llm-adapter --test llm_profile_contracts`, `cargo test -p unified-llm-adapter --test runtime_boundary_contracts`, the high-level profile/model tests in `cargo test -p unified-llm-adapter --test public_surface_contracts`, `cargo test -p spark-agent-adapter --test llm_backend_contracts`, Attractor launch/profile coverage in `cargo test -p attractor-runtime --test core_handler_contracts`, and CLI/server shell contract tests in `cargo test -p spark-cli --test cli_shell_contracts` and `cargo test -p spark-server --test server_shell_contracts`. Retained Python compatibility/oracle coverage remains `uv run pytest tests/compat/providers -q` and `uv run pytest tests/adapters -q`. The repository completion gate for this milestone remains `uv run pytest -q`.

M7 closes the M5-adjacent production transport gap for normal Rust provider execution. Remaining adjacent capabilities are broader live SDK/network error-shape parity, keeping credential-backed smoke execution outside ordinary validation, and future provider feature expansion beyond the current Rust adapters.

## M6 Unified LLM Boundary Closure

M6 closes the unified LLM migration documentation for the currently delivered runtime boundary. Normal Spark server and CLI execution is Rust-owned: `spark-server` and `spark-cli` send launch/profile/model/provider inputs into `crates/attractor-runtime`, `crates/spark-agent-adapter` lowers generation requests to `unified_llm_adapter::Request`, and `crates/unified-llm-adapter` performs provider/profile/model resolution, request translation, stream translation, error normalization, retry policy support, usage accounting, tools, structured output, middleware, timeouts, and provider dispatch through Rust `ProviderAdapter` implementations. That path does not import, shell out to, or wrap `src.unified_llm` provider clients for normal Spark provider execution.

The retained Python `src/unified_llm` package remains intentionally present in three roles. First, `tests/compat/providers` uses it as an oracle for fixture-backed parity observations such as model catalog/profile resources, request/tool/structured-output DTOs, retry/error/usage/stream behavior, and package-data compatibility. Second, `tests/adapters` guards historical provider adapter behavior as compatibility evidence, including optional comparison smoke runs. Third, `src/unified_llm/data/models.json` and related loaders remain package-data support while Rust mirrors the model catalog and provider profile behavior. These retained surfaces are not the binding normal runtime path for Spark server or CLI provider execution.

Rust-facing validation is behavioral. `cargo test -p unified-llm-adapter --test native_provider_parity_matrix` covers OpenAI, Anthropic, and Gemini provider routing, request translation, image URL/data/local-path inputs, structured output, provider option isolation, response translation, streaming delta/final-text parity, usage, reasoning/thinking, cache accounting, authentication errors, and rate-limit errors. `cargo test -p unified-llm-adapter --test compatible_provider_parity_matrix` covers OpenAI-compatible, OpenRouter, and LiteLLM Chat Completions routing, explicit model/profile resolution, environment configuration, unsupported feature warnings, usage, errors, and streaming without treating those providers as native OpenAI Responses implementations.

Additional Rust behavior coverage comes from `cargo test -p unified-llm-adapter --test public_surface_contracts` for public DTOs, low-level routing, retry boundaries, high-level retries, active/passive tools, ordered batched tool continuation, streaming tool loops, structured-output generation/streaming, middleware, timeout, usage, and concurrency contracts; `cargo test -p unified-llm-adapter --test native_request_contracts` for provider-native request/response translation, prompt caching, thinking continuation, cache usage, media handling, and explicitly rejected audio/document inputs; `cargo test -p unified-llm-adapter --test openai_compatible_contracts` for Chat Completions request/response/error/SSE compatibility; and `cargo test -p unified-llm-adapter --test runtime_boundary_contracts` plus `cargo test -p unified-llm-adapter --test adapter_contracts` for runtime resolution and Python-oracle fixture parity. Retained Python checks remain `uv run pytest tests/compat/providers -q` and `uv run pytest tests/adapters -q`, while `uv run pytest -q` is the repository completion gate.

Unsupported or intentionally deferred provider capabilities remain explicit non-goals or future work rather than silent gaps. Credential-backed smoke execution in ordinary validation is an explicit non-goal; default validation remains credential-free. Exact live SDK/network error-shape parity beyond normalized public errors, provider-specific persistent cache resource management, OpenRouter/LiteLLM/OpenAI-compatible advisory latest-model defaults, and compatible-provider support for Responses-only options or built-in Responses tools are future work. Audio and document content for native providers is currently rejected with provider-scoped errors. Compatible providers continue to surface unsupported Responses-only behavior through warnings and keep requiring an explicit request model or profile default.

Optional real-key checks are operational smoke tests, not the binding migration contract. They are gated by explicit pytest selection and provider credentials as described in the M6 gated live provider smoke plan below.

## M6 Gated Live Provider Smoke Plan

M6 keeps ordinary validation credential-free while documenting opt-in real-key checks for retained Python compatibility smoke behavior. `uv run pytest -q` does not make OpenAI, Anthropic, Gemini, or OpenAI-compatible API calls: tests marked `live` or `live_smoke` are skipped by default unless the operator explicitly selects them with `--run-live`, `-m live`, or the agent live-smoke gate. Missing provider credentials skip the selected live tests instead of failing default local runs.

The adapter smoke entry point is `tests/adapters/test_cross_provider_parity.py`. Run it explicitly with `uv run pytest -q -m live tests/adapters/test_cross_provider_parity.py` or `uv run pytest -q --run-live tests/adapters/test_cross_provider_parity.py`. It covers OpenAI, Anthropic, and Gemini native adapters for basic generation, streaming, image input, structured output, tool calling with tool-result replay, and provider error conversion. It also covers the OpenAI-compatible Chat Completions path against OpenAI when `OPENAI_API_KEY` is present.

Operational prerequisites are process environment variables only. OpenAI native and OpenAI-compatible smoke tests require `OPENAI_API_KEY`; Anthropic smoke tests require `ANTHROPIC_API_KEY`; Gemini smoke tests require `GEMINI_API_KEY` or `GOOGLE_API_KEY`. Optional model overrides are `UNIFIED_LLM_LIVE_OPENAI_MODEL`, `UNIFIED_LLM_LIVE_ANTHROPIC_MODEL`, `UNIFIED_LLM_LIVE_GEMINI_MODEL`, and `UNIFIED_LLM_LIVE_CHAT_MODEL`; `OPENAI_BASE_URL` can redirect the OpenAI-compatible adapter when needed.

The live smoke plan complements mocked and fixture-backed parity coverage in `tests/adapters` and `tests/compat/providers`; it is not the only proof of provider behavior and is not required for the milestone completion gate. Live runs must not add credential files, captured API logs, local response artifacts, or generated smoke outputs as deliverables.

## M7 Production Rust Provider Transport Status

M7 installs `NativeHttpTransport` for environment and profile configured providers in `crates/unified-llm-adapter`. Normal Spark server, CLI, and agent unified LLM runtime execution is Rust-owned and transport-backed for configured providers: `Client::from_env`, `Client::from_env_map`, and `Client::from_env_map_and_profiles` now construct Rust transport-backed adapters for OpenAI, Anthropic, Gemini, OpenRouter, LiteLLM, and OpenAI-compatible endpoints. Profile-backed OpenAI-compatible adapters use the same transport, preserve local no-key profiles with `require_api_key = false`, and keep public profile metadata redacted.

The transport executes prepared `NativeCompleteRequest` values without rewriting provider adapter intent: method, URL, headers, JSON body, connect timeout, request timeout, and stream-read timeout are forwarded to HTTP. Complete calls return status, normalized headers, and parsed JSON to existing response translators. Streaming calls yield provider chunks incrementally through the shared SSE/JSON stream translators, release network resources on explicit close or drop, and keep post-partial stream errors visible to callers instead of transparently retrying them.

Credential-free deterministic Rust transport coverage is in `crates/unified-llm-adapter/tests/http_transport_contracts.rs`. It covers successful complete and streaming calls, exact request forwarding, native and compatible endpoint selection, profile-backed local endpoints, HTTP status errors, malformed JSON, refused-connection network errors, request timeouts, stream read timeouts, explicit stream close, Retry-After headers, rate-limit headers, raw error bodies, and provider error metadata. Runtime integration checks include `crates/unified-llm-adapter/tests/runtime_boundary_contracts.rs`, `crates/spark-agent-adapter/tests/llm_backend_contracts.rs`, and the CLI/server shell contract tests that exercise launch/profile/provider payload wiring into the Rust runtime path. Retry boundary coverage remains in `crates/unified-llm-adapter/tests/public_surface_contracts.rs`: low-level `Client.complete` and `Client.stream` do not retry automatically, while high-level generation and streaming retry only individual LLM calls and only before a stream has yielded caller-visible events.

The gated Rust live smoke harness is `crates/unified-llm-adapter/tests/live_smoke_contracts.rs`. It is inert by default and runs only with `UNIFIED_LLM_ADAPTER_RUN_LIVE=1`; `UNIFIED_LLM_ADAPTER_LIVE_PROVIDERS` can restrict providers. It supports OpenAI, Anthropic, Gemini, OpenRouter, and OpenAI-compatible complete and stream calls, with provider credentials supplied only through process environment variables and optional model overrides. The credential-free full-suite command and milestone completion gate is `uv run pytest -q`.

## M1 Unified LLM Runtime Boundary

M1 establishes `crates/unified-llm-adapter` as the Rust-owned low-level runtime boundary for rewrite callers. The Rust crate owns the public `Request`, `Response`, `Message`, `ContentPart`, `FinishReason`, `Usage`, `StreamEvent`, `ProviderAdapter`, `Client`, default-client functions, provider environment registration, model catalog access, retry/error normalization, and middleware contracts.

Normal Rust-facing complete and stream calls are made through `Client` and registered `ProviderAdapter` trait objects. `Client::from_env` records configured providers in the fixed order `openai`, `anthropic`, `gemini`, `openrouter`, `litellm`, and `openai_compatible`, including the `GOOGLE_API_KEY` fallback for Gemini. OpenAI, Anthropic, and Gemini entries use Rust native provider adapters for provider-native request/response translation; OpenRouter, LiteLLM, and OpenAI-compatible entries use the separate Rust Chat Completions compatibility adapter path. As of M7, ordinary environment construction installs the Rust HTTP transport for these providers.

The retained Python `src/unified_llm` package remains valid only for compatibility tests, oracle fixture generation, and package data such as the model catalog. Existing provider-specific Python clients, cross-provider streaming normalization, structured-output fallbacks, timeout behavior, and credential handling are retained compatibility/oracle/package-data surfaces, not normal M1 Rust runtime dependencies.

Remaining provider capabilities include broader live SDK error-shape parity and optional credential-backed smoke coverage outside ordinary validation. Catalog defaults remain native-provider only, so OpenRouter, LiteLLM, and OpenAI-compatible endpoints do not receive advisory latest-model defaults from the catalog.

Validation for this boundary is behavioral: `cargo test -p unified-llm-adapter` exercises the Rust public client, adapter, routing, middleware, catalog, DTO, complete, and stream contracts; `uv run pytest tests/compat/providers -q` checks retained Python compatibility observations; `uv run pytest tests/adapters -q` guards retained Python provider behavior; and `uv run pytest -q` remains the full repository completion gate.

## M2 Native Provider Translation Status

M2 adds Rust-owned, non-streaming native provider adapters in `crates/unified-llm-adapter` for OpenAI, Anthropic, and Gemini. The adapters implement `ProviderAdapter.complete` by preparing OpenAI Responses API requests, Anthropic Messages API requests, and Gemini `generateContent` requests with provider-native method, URL, headers, authentication, body shape, provider-option isolation, media handling, prompt caching, reasoning/thinking continuation data, and tool-result replay data. Responses returned by an injected Rust transport are normalized back into the unified `Response`, preserving raw payloads and separating visible output tokens from reasoning and cache token accounting.

M2 originally established adapter and mock-transport coverage. As of M7, `Client::from_env` registers native OpenAI, Anthropic, Gemini, OpenRouter, LiteLLM, and generic OpenAI-compatible adapter entries with the Rust HTTP transport. The retained Python `src/unified_llm` provider clients remain compatibility/oracle surfaces for historical adapter behavior and optional comparison smoke runs. OpenAI-compatible Chat Completions remains distinct from native OpenAI Responses behavior.

M2 validation evidence is behavioral and credential-free. `cargo test -p unified-llm-adapter` exercises `Client.complete` and `ProviderAdapter.complete` with mock native transports for endpoint paths, headers, authentication, request bodies, response parsing, rate-limit mapping, raw payload preservation, active-provider option isolation, Anthropic beta header joining, prompt caching, reasoning/thinking preservation, usage/cost fields, local filesystem media inputs, and Gemini synthetic ID replay. `uv run pytest tests/compat/providers -q` and `uv run pytest tests/adapters -q` continue to guard the retained Python compatibility boundary, while `uv run pytest -q` remains the full repository completion gate.

Remaining M2-adjacent capabilities include optional live credential smoke operation, provider-specific persistent cache resource management, and broader high-level generate/tool-loop orchestration.

## M3 OpenAI-Compatible, OpenRouter, And LiteLLM Translation Status

M3 adds a Rust-owned Chat Completions compatibility adapter in `crates/unified-llm-adapter`. `OpenAICompatibleAdapter`, `OpenRouterAdapter`, and `LiteLLMAdapter` build `POST /v1/chat/completions` requests and never route compatible providers through native OpenAI's `POST /v1/responses` path. The adapters translate unified messages, image URL/data parts, tool calls and tool results, function tools, tool choice, generation options, response formats, active-provider `provider_options`, provider headers, Chat Completions responses, usage, rate-limit headers, raw payloads, provider errors, and SSE streams into the unified Rust DTOs and stream lifecycle.

Only the active provider namespace is read from `provider_options`: `openai_compatible`, `openrouter`, or `litellm`. OpenRouter defaults to `https://openrouter.ai/api/v1`, requires `OPENROUTER_API_KEY` for environment configuration, and forwards configured `OPENROUTER_HTTP_REFERER` and `OPENROUTER_TITLE` headers. LiteLLM requires `LITELLM_BASE_URL` and accepts optional `LITELLM_API_KEY`. These compatible providers still require explicit request models; catalog latest-model defaults remain native-provider only.

The compatibility adapter reports unsupported Responses-only behavior through `Response.warnings` rather than claiming native Responses parity. Unsupported features include reasoning-token visibility on compatible usage payloads, built-in OpenAI Responses tools such as web search tools, and server-side Responses conversation state options such as `previous_response_id`, `conversation`, `prompt`, `instructions`, and `store`.

M3 also completes the Rust-owned error, retry, and streaming contracts for rewrite callers. `SDKError`, `ProviderError`, `SDKErrorKind`, and retained `AdapterError` aliases expose retryability, Retry-After, provider metadata, status codes, error codes, and raw payload retention. `RetryPolicy` owns deterministic delay math, jitter bounds, Retry-After override and cutoff handling, callbacks, and high-level retry helpers, while low-level `Client.complete` and `Client.stream` continue to surface adapter failures without automatic retry. `StreamEvent`, `StreamEventType`, `StreamAccumulator`, managed stream closing, and the shared SSE parser own provider-neutral stream lifecycle behavior for text, reasoning, tool calls, final accumulated responses, provider-event passthrough, malformed stream payloads, and post-partial stream errors.

M3 validation evidence is behavioral and credential-free. `cargo test -p unified-llm-adapter` exercises the Rust-owned public error classifiers, RetryPolicy, low-level no-retry client boundary, before-first-event stream retry helper, post-partial stream error behavior, explicit stream closing, stream accumulation, OpenAI Responses streaming, Anthropic Messages streaming, Gemini SSE and JSON stream chunks, OpenAI-compatible Chat Completions response/error/request/stream translation, OpenRouter environment and header behavior, LiteLLM environment behavior, explicit compatible-provider model requirements, raw payload retention, and unsupported-feature warnings. `uv run pytest tests/compat/providers -q` and `uv run pytest tests/adapters -q` remain retained Python compatibility checks, but they do not replace the Rust-owned runtime validation; `uv run pytest -q` remains the full repository completion gate.

Remaining M3-adjacent work includes credential-backed live provider smoke operation, exact live SDK/network error-shape parity beyond the normalized public contract, and any future provider capability expansion that would turn compatible-provider warnings into native feature support.

## M2 Anthropic Prompt Caching And Cache Accounting

Anthropic request translation preserves explicit `cache_control` annotations on provider-native system blocks, tool definitions, and message content blocks. Automatic Anthropic prompt caching is enabled by default: when no explicit `provider_options.anthropic.cache_control` is supplied, the adapter uses `{"type":"ephemeral"}` and marks the merged system prompt, the last tool definition, and the stable conversation prefix before the current user turn, while staying within Anthropic's four-breakpoint limit.

`provider_options.anthropic.auto_cache = false` disables only automatic breakpoint injection. Explicit annotations remain intact, and only the Anthropic provider namespace controls Anthropic caching behavior. Whenever any Anthropic request body contains `cache_control`, the adapter adds `prompt-caching-2024-07-31` to `anthropic-beta` unless it is already present.

Cache accounting is normalized without mixing it into visible output or reasoning token fields: Anthropic `cache_read_input_tokens` maps to `Usage.cache_read_tokens`, Anthropic `cache_creation_input_tokens` maps to `Usage.cache_write_tokens`, OpenAI `usage.input_tokens_details.cached_tokens` maps to `Usage.cache_read_tokens`, and Gemini `usageMetadata.cachedContentTokenCount` maps to `Usage.cache_read_tokens`.

Deferred cache behavior remains provider-specific persistent cache resource management, such as creating or reusing Gemini `cachedContent` resources beyond pass-through request options.

## Deprecated Compatibility Surfaces

The following routes are compatibility surfaces, not cleanup leftovers. They remain preserved until a later contract decision, migration note, and validation update approve a different contract.

| Surface | Current status | Preferred replacement | Evidence |
| --- | --- | --- | --- |
| `GET /attractor/runs/events` | Preserved compatibility route | `GET /workspace/api/live/events` | `http/deprecated-attractor-runs-events`; M5 deprecated route evidence |
| `GET /workspace/api/conversations/{conversation_id}/events` | Preserved compatibility route | `GET /workspace/api/live/events` | `http/deprecated-workspace-conversation-events`; M5 deprecated route evidence |
| `GET /attractor/pipelines/{id}/events` | Preserved compatibility route | `GET /attractor/pipelines/{id}/journal` or `GET /workspace/api/live/events?run_id=<id>` | `crates/spark-http/tests/deprecated_route_error_contracts.rs`; `tests/api/test_pipeline_contract_section_95.py` |

## Closed Gaps, Non-Goals, And Audit Result

This document treats prior policy gaps as closed only where implementation or audit evidence is listed:

- Acceptance workflow assets are covered by an executable pytest harness.
- First-class subgraph and scoped `node[...]` / `edge[...]` default authoring is available through Graph Settings.
- The post-gap spec/API drift audit found no remaining unintended runtime, editor, or API drift and records all remaining differences as intentional policy decisions, explicit non-goals, or future compatibility-break candidates.

The following remain explicit non-goals and are not counted as closed parity:

- M7 does not remove Python `agent` or `unified_llm` modules.
- M7 does not reintroduce remote-worker execution; native and local-container execution remain the supported modes.

## Approved Policy Decisions

- Trigger-launched flows may run against and mutate the trigger action's configured project repository by default. This preserves the current trigger launch behavior: `project_path` on the trigger action is the launched run's project target, trigger static context and payload are injected into the run checkpoint, trigger provenance is recorded, and missing `project_path` falls back to Spark home/data-dir behavior.
- Manager-loop authoring is first-class in Rust-backed preview payloads and frontend authoring for the supported manager-loop attributes.
- Manager-loop observe cycles ingest linked child-run telemetry into the runtime-owned `context.stack.child.*` snapshot without adding stall or progress heuristics.
- The non-exit outgoing-edge validator rule is intended behavior: every non-exit node must declare at least one outgoing edge, and intentional completion routes through the single terminal node.

## Future Decision Candidates

Any removal of deprecated event routes, manager-loop contract change, or validator rule relaxation/tightening requires a new contract decision before implementation depends on it.
