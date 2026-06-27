# Rust Rewrite Migration Records

This note summarizes the M7 migration record for Rust-backed Spark. The structured source of truth is `.spark/rust-rewrite/current/migration-records.json`; this page is the human-readable companion. It records adapter ownership, retained Python modules, deprecated compatibility surfaces, explicit non-goals, and future decision candidates. It does not change any user command, HTTP route, SSE payload, storage layout, package name, service workflow, or frontend contract.

## Scope

- Public commands remain `spark` and `spark-server`.
- Rust crates own the contracts already implemented and validated through the rewrite milestones.
- Python modules remain valid where the record classifies them as retained behavior.
- Deprecated compatibility routes remain supported until a later contract decision records an incompatible change.
- No open policy gaps remain after the post-gap spec/API drift audit. Explicit non-goals and future compatibility-break candidates are still not counted as closed parity.

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
| Request DTOs, tool/structured-output payloads, retry, errors, stream events, and usage/cost normalization | `native_rust` | `crates/unified-llm-adapter` | Rust owns the contract for rewrite callers; Python live clients remain retained for actual provider calls | `providers/request-tool-structured-output`; `providers/retry-error-usage-stream`; adapter contract tests |
| Provider adapter bridge from Rust callers to retained provider behavior | `rust_owned_adapter` | `crates/unified-llm-adapter` | Live provider clients and transports remain Python unless a future parity milestone replaces them | M3 and M6 provider/model validation artifacts |
| Provider-specific clients, cross-provider streaming normalization, structured-output fallback handling, middleware, and timeouts | `retained_python_module` | `src/unified_llm` | Retained Python modules guarded by provider and cross-provider tests | `tests/adapters/test_openai_adapter.py`; `tests/adapters/test_anthropic_adapter.py`; `tests/adapters/test_cross_provider_parity.py` |

## M1 Unified LLM Runtime Boundary

M1 establishes `crates/unified-llm-adapter` as the Rust-owned low-level runtime boundary for rewrite callers. The Rust crate owns the public `Request`, `Response`, `Message`, `ContentPart`, `FinishReason`, `Usage`, `StreamEvent`, `ProviderAdapter`, `Client`, default-client functions, provider environment registration, model catalog access, retry/error normalization, and middleware contracts.

Normal Rust-facing complete and stream calls are made through `Client` and registered `ProviderAdapter` trait objects. `Client::from_env` records configured providers in the fixed order `openai`, `anthropic`, `gemini`, `openrouter`, `litellm`, and `openai_compatible`, including the `GOOGLE_API_KEY` fallback for Gemini. OpenAI, Anthropic, and Gemini entries use Rust native provider adapters for non-streaming request/response translation; ordinary environment construction does not install production HTTP transports. OpenRouter, LiteLLM, and OpenAI-compatible entries remain unavailable Rust adapter placeholders instead of importing or delegating to Python `src/unified_llm` provider clients.

The retained Python `src/unified_llm` package remains valid for compatibility tests, oracle fixture generation, live-provider adapter behavior that has not been replaced, and package data such as the model catalog. Existing provider-specific Python clients, cross-provider streaming normalization, structured-output fallbacks, timeout behavior, and live credential handling are retained compatibility surfaces, not normal M1 Rust runtime dependencies.

Deferred provider capabilities include native HTTP transports for OpenAI, Anthropic, Gemini, OpenRouter, LiteLLM, and generic OpenAI-compatible endpoints; provider-specific streaming parsers; live SDK error-shape parity; and optional credential-backed smoke coverage. Catalog defaults remain native-provider only in M1, so OpenRouter, LiteLLM, and OpenAI-compatible endpoints do not receive advisory latest-model defaults from the catalog.

Validation for this boundary is behavioral: `cargo test -p unified-llm-adapter` exercises the Rust public client, adapter, routing, middleware, catalog, DTO, complete, and stream contracts; `uv run pytest tests/compat/providers -q` checks retained Python compatibility observations; `uv run pytest tests/adapters -q` guards retained Python provider behavior; and `uv run pytest -q` remains the full repository completion gate.

## M2 Native Provider Translation Status

M2 adds Rust-owned, non-streaming native provider adapters in `crates/unified-llm-adapter` for OpenAI, Anthropic, and Gemini. The adapters implement `ProviderAdapter.complete` by preparing OpenAI Responses API requests, Anthropic Messages API requests, and Gemini `generateContent` requests with provider-native method, URL, headers, authentication, body shape, provider-option isolation, media handling, prompt caching, reasoning/thinking continuation data, and tool-result replay data. Responses returned by an injected Rust transport are normalized back into the unified `Response`, preserving raw payloads and separating visible output tokens from reasoning and cache token accounting.

This is adapter and mock-transport coverage, not a production live-transport replacement. `Client::from_env` registers native OpenAI, Anthropic, and Gemini adapter entries, but ordinary environment construction does not install live HTTP transports. The retained Python `src/unified_llm` provider clients remain the live-provider compatibility surface for streaming, retries around real HTTP calls, provider SDK error-shape parity, credential-backed smoke behavior, and high-level generation flows until later rewrite items replace those paths. OpenRouter, LiteLLM, and generic `openai_compatible` endpoints remain separate compatibility surfaces; OpenAI-compatible Chat Completions scaffolding is distinct from native OpenAI Responses behavior and does not count as REQ-022 completion.

M2 validation evidence is behavioral and credential-free. `cargo test -p unified-llm-adapter` exercises `Client.complete` and `ProviderAdapter.complete` with mock native transports for endpoint paths, headers, authentication, request bodies, response parsing, rate-limit mapping, raw payload preservation, active-provider option isolation, Anthropic beta header joining, prompt caching, reasoning/thinking preservation, usage/cost fields, local filesystem media inputs, and Gemini synthetic ID replay. `uv run pytest tests/compat/providers -q` and `uv run pytest tests/adapters -q` continue to guard the retained Python compatibility boundary, while `uv run pytest -q` remains the full repository completion gate.

Deferred M2-adjacent capabilities include production native Rust HTTP transport installation, provider streaming parsers, live credential smoke tests, OpenAI-compatible Chat Completions completion parity, OpenRouter/LiteLLM native compatibility completion, provider-specific persistent cache resource management, and broader high-level generate/tool-loop orchestration.

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

The structured record now carries all prior policy gaps as closed with implementation or audit evidence:

- Acceptance workflow assets are covered by an executable pytest harness.
- First-class subgraph and scoped `node[...]` / `edge[...]` default authoring is available through Graph Settings.
- The post-gap spec/API drift audit found no remaining unintended runtime, editor, or API drift and records all remaining differences as intentional policy decisions, explicit non-goals, or future compatibility-break candidates.

The structured record carries these as explicit non-goals, with `counts_as_closed_parity` set to `false`:

- M7 does not remove Python `agent` or `unified_llm` modules.
- M7 does not reintroduce remote-worker execution; native and local-container execution remain the supported modes.

## Approved Policy Decisions

- Trigger-launched flows may run against and mutate the trigger action's configured project repository by default. This preserves the current trigger launch behavior: `project_path` on the trigger action is the launched run's project target, trigger static context and payload are injected into the run checkpoint, trigger provenance is recorded, and missing `project_path` falls back to Spark home/data-dir behavior.
- Manager-loop authoring is first-class in Rust-backed preview payloads and frontend authoring for the supported manager-loop attributes.
- Manager-loop observe cycles ingest linked child-run telemetry into the runtime-owned `context.stack.child.*` snapshot without adding stall or progress heuristics.
- The non-exit outgoing-edge validator rule is intended behavior: every non-exit node must declare at least one outgoing edge, and intentional completion routes through the single terminal node.

## Future Decision Candidates

Any removal of deprecated event routes, manager-loop contract change, or validator rule relaxation/tightening requires a new contract decision before implementation depends on it.
