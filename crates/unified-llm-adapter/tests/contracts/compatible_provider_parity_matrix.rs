use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use unified_llm_adapter::{
    build_openai_compatible_chat_request, generate, ActiveLlmProfile, AdapterError,
    AdapterErrorKind, Client, FinishReasonKind, GenerateRequest, LiteLLMAdapter, Message,
    NativeCompleteRequest, NativeCompleteResponse, NativeCompleteTransport, NativeStreamResponse,
    OpenAICompatibleAdapter, OpenAICompatibleRequestConfig, OpenRouterAdapter, ProviderAdapter,
    ProviderConfig, ProviderEnvironment, Request, Response, ResponseFormat, StreamAccumulator,
    StreamEventType,
};

#[test]
fn compatible_providers_complete_on_chat_completions_path_with_warnings_and_usage() {
    for provider in CompatibleProvider::ALL {
        let transport = Arc::new(RecordingTransport::new([Ok(provider.success_response())]));
        let client =
            Client::from_adapters([provider.adapter(transport.clone())], Some(provider.name()))
                .unwrap();

        let response = client
            .complete(Request {
                model: provider.model().to_string(),
                messages: vec![Message::user("hello")],
                response_format: Some(ResponseFormat::JsonObject),
                reasoning_effort: Some("high".to_string()),
                provider_options: provider_options(provider),
                ..Request::default()
            })
            .unwrap();
        let captured = transport.only_request();

        assert_eq!(captured.provider, provider.name());
        assert_eq!(captured.method, "POST");
        assert_eq!(captured.url, provider.chat_completions_url());
        assert!(captured.url.ends_with("/chat/completions"));
        assert!(!captured.url.contains("/responses"));
        assert_eq!(captured.body["model"], json!(provider.model()));
        assert_eq!(captured.body["parallel_tool_calls"], json!(false));
        assert_eq!(
            captured.body["tools"],
            json!([{
                "type": "function",
                "function": {
                    "name": "matrix_lookup",
                    "description": "Lookup from the parity matrix",
                    "parameters": {"type": "object"},
                },
            }])
        );
        assert!(!captured.body.to_string().contains("web_search_preview"));
        assert!(!captured.body.to_string().contains("previous_response_id"));
        assert!(!captured.body.to_string().contains("conversation"));
        assert!(!captured.body.to_string().contains("max_output_tokens"));
        assert!(!captured.body.to_string().contains("native-responses-only"));
        assert_eq!(captured.headers["X-Matrix-Provider"], provider.name());
        if provider.uses_authorization_header() {
            assert_eq!(
                captured.headers["Authorization"],
                format!("Bearer {}-key", provider.name())
            );
        }

        assert_eq!(response.provider, provider.name());
        assert_eq!(response.id, format!("chatcmpl_{}", provider.name()));
        assert_eq!(response.model, provider.model());
        assert_eq!(response.text(), "compatible complete");
        assert_eq!(response.finish_reason.reason, FinishReasonKind::Stop);
        assert_eq!(response.usage.input_tokens, 8);
        assert_eq!(response.usage.output_tokens, 5);
        assert_eq!(response.usage.total_tokens, 13);
        assert_eq!(response.usage.reasoning_tokens, None);
        assert_eq!(response.usage.cache_read_tokens, Some(3));
        assert_eq!(
            response
                .rate_limit
                .as_ref()
                .and_then(|rate_limit| rate_limit.requests_remaining),
            Some(7)
        );
        assert_warning_codes(
            &response,
            [
                "unsupported_reasoning_effort",
                "unsupported_responses_tool",
                "unsupported_responses_option",
                "unsupported_reasoning_token_visibility",
            ],
        );
    }
}

#[test]
fn compatible_providers_stream_chat_completions_events_and_usage() {
    for provider in CompatibleProvider::ALL {
        let transport = Arc::new(RecordingTransport::new_with_streams(
            std::iter::empty(),
            [Ok(NativeStreamResponse {
                status: 200,
                headers: BTreeMap::from([(
                    "x-ratelimit-remaining-requests".to_string(),
                    "6".to_string(),
                )]),
                body: vec![Ok(Value::String(provider.stream_payload()))],
            })],
        ));
        let client =
            Client::from_adapters([provider.adapter(transport.clone())], Some(provider.name()))
                .unwrap();

        let events = client
            .stream(Request {
                model: provider.model().to_string(),
                messages: vec![Message::user("hello")],
                reasoning_effort: Some("medium".to_string()),
                ..Request::default()
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let captured = transport.only_request();
        let accumulated = StreamAccumulator::from_events(events.clone()).response;

        assert_eq!(captured.provider, provider.name());
        assert_eq!(captured.url, provider.chat_completions_url());
        assert!(!captured.url.contains("/responses"));
        assert_eq!(captured.body["stream"], json!(true));
        assert_eq!(
            events.iter().map(|event| &event.r#type).collect::<Vec<_>>(),
            vec![
                &StreamEventType::StreamStart,
                &StreamEventType::TextStart,
                &StreamEventType::TextDelta,
                &StreamEventType::TextDelta,
                &StreamEventType::TextEnd,
                &StreamEventType::Finish,
            ]
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event.r#type == StreamEventType::TextDelta)
                .filter_map(|event| event.delta.as_deref())
                .collect::<String>(),
            "streamed compatible"
        );
        assert_eq!(accumulated.provider, provider.name());
        assert_eq!(accumulated.model, provider.model());
        assert_eq!(accumulated.text(), "streamed compatible");
        assert_eq!(accumulated.finish_reason.reason, FinishReasonKind::Stop);
        assert_eq!(accumulated.usage.input_tokens, 10);
        assert_eq!(accumulated.usage.output_tokens, 4);
        assert_eq!(accumulated.usage.total_tokens, 14);
        assert_eq!(accumulated.usage.reasoning_tokens, None);
        assert_eq!(
            accumulated
                .rate_limit
                .as_ref()
                .and_then(|rate_limit| rate_limit.requests_remaining),
            Some(6)
        );
        assert_warning_codes(
            &accumulated,
            [
                "unsupported_reasoning_effort",
                "unsupported_reasoning_token_visibility",
            ],
        );
    }
}

#[test]
fn compatible_providers_classify_authentication_and_rate_limit_errors() {
    for provider in CompatibleProvider::ALL {
        assert_provider_error(
            provider,
            NativeCompleteResponse {
                status: 401,
                headers: BTreeMap::new(),
                body: json!({
                    "error": {
                        "message": "bad key",
                        "code": "invalid_api_key",
                    },
                }),
            },
            AdapterErrorKind::Authentication,
            false,
            None,
            "invalid_api_key",
        );
        assert_provider_error(
            provider,
            NativeCompleteResponse {
                status: 429,
                headers: BTreeMap::from([("Retry-After".to_string(), "4".to_string())]),
                body: json!({
                    "error": {
                        "message": "slow down",
                        "code": "rate_limit",
                    },
                }),
            },
            AdapterErrorKind::RateLimit,
            true,
            Some(4.0),
            "rate_limit",
        );
    }
}

#[test]
fn compatible_provider_model_resolution_requires_explicit_or_profile_default_models() {
    for provider in CompatibleProvider::ALL {
        let missing_model_transport = Arc::new(RecordingTransport::new(std::iter::empty()));
        let missing_model_client = Client::from_adapters(
            [provider.adapter(missing_model_transport.clone())],
            Some(provider.name()),
        )
        .unwrap();
        let error = generate(
            &missing_model_client,
            GenerateRequest {
                prompt: Some("hello".to_string()),
                ..GenerateRequest::default()
            },
        )
        .unwrap_err();

        assert_eq!(error.kind, AdapterErrorKind::Configuration);
        assert!(error.message.contains("No model configured"));
        assert_eq!(missing_model_transport.request_count(), 0);

        let explicit_model_transport =
            Arc::new(RecordingTransport::new([Ok(provider.success_response())]));
        let explicit_model_client = Client::from_adapters(
            [provider.adapter(explicit_model_transport.clone())],
            Some(provider.name()),
        )
        .unwrap();
        let generated = generate(
            &explicit_model_client,
            GenerateRequest {
                prompt: Some("hello".to_string()),
                model: Some(provider.model().to_string()),
                ..GenerateRequest::default()
            },
        )
        .unwrap();
        let captured = explicit_model_transport.only_request();

        assert_eq!(generated.steps[0].request.model, provider.model());
        assert_eq!(captured.body["model"], json!(provider.model()));
        assert_eq!(captured.url, provider.chat_completions_url());
    }

    let profile_transport = Arc::new(RecordingTransport::new([Ok(NativeCompleteResponse::ok(
        json!({
            "id": "chatcmpl_profile",
            "model": "profile-default-model",
            "choices": [{
                "message": {"role": "assistant", "content": "profile ok"},
                "finish_reason": "stop",
            }],
        }),
    ))]));
    let profile_adapter: Arc<dyn ProviderAdapter> =
        Arc::new(OpenAICompatibleAdapter::openai_compatible(
            OpenAICompatibleRequestConfig {
                base_url: Some("https://profiles.example/custom/responses".to_string()),
                ..OpenAICompatibleRequestConfig::default()
            },
            profile_transport.clone(),
        ));
    let profile_client = Client::new()
        .with_llm_profile_adapter(
            "local_profile",
            ActiveLlmProfile::new(
                "openai_compatible",
                Some("profile-default-model".to_string()),
            ),
            profile_adapter,
        )
        .unwrap()
        .with_default_provider(Some("local_profile"))
        .unwrap();

    let generated = generate(
        &profile_client,
        GenerateRequest {
            prompt: Some("hello".to_string()),
            ..GenerateRequest::default()
        },
    )
    .unwrap();
    let captured = profile_transport.only_request();

    assert_eq!(
        generated.steps[0].request.provider.as_deref(),
        Some("local_profile")
    );
    assert_eq!(generated.steps[0].request.model, "profile-default-model");
    assert_eq!(
        generated.steps[0].request.metadata["spark.runtime.provider"],
        json!("openai_compatible")
    );
    assert_eq!(
        generated.steps[0].request.metadata["spark.runtime.model"],
        json!("profile-default-model")
    );
    assert_eq!(
        generated.steps[0].request.metadata["spark.runtime.llm_profile"],
        json!("local_profile")
    );
    assert_eq!(captured.provider, "openai_compatible");
    assert_eq!(
        captured.url,
        "https://profiles.example/custom/v1/chat/completions"
    );
    assert_eq!(captured.body["model"], json!("profile-default-model"));
}

#[test]
fn compatible_provider_environment_resolution_uses_rust_adapter_configs() {
    let env = BTreeMap::from([
        (
            "OPENROUTER_API_KEY".to_string(),
            "openrouter-key".to_string(),
        ),
        (
            "OPENROUTER_BASE_URL".to_string(),
            "https://router.example/root/responses".to_string(),
        ),
        (
            "OPENROUTER_HTTP_REFERER".to_string(),
            "https://spark.example".to_string(),
        ),
        ("OPENROUTER_TITLE".to_string(), "Spark".to_string()),
        (
            "LITELLM_BASE_URL".to_string(),
            "https://litellm.example/proxy/chat/completions".to_string(),
        ),
        ("LITELLM_API_KEY".to_string(), "litellm-key".to_string()),
        (
            "OPENAI_COMPATIBLE_BASE_URL".to_string(),
            "https://compatible.example/custom/responses".to_string(),
        ),
        (
            "OPENAI_COMPATIBLE_API_KEY".to_string(),
            "compatible-key".to_string(),
        ),
    ]);
    let environment = ProviderEnvironment::from_env_map(&env, None);

    assert_eq!(environment.default_provider.as_deref(), Some("openrouter"));
    assert_eq!(
        environment
            .providers
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["litellm", "openai_compatible", "openrouter"]
    );
    assert_compatible_config(
        environment.providers.get("openrouter").unwrap(),
        "openrouter",
        Some("openrouter-key"),
        Some("https://router.example/root/v1"),
    );
    assert_eq!(
        environment.providers["openrouter"]
            .options
            .get("HTTP-Referer")
            .map(String::as_str),
        Some("https://spark.example")
    );
    assert_eq!(
        environment.providers["openrouter"]
            .options
            .get("X-Title")
            .map(String::as_str),
        Some("Spark")
    );
    assert_compatible_config(
        environment.providers.get("litellm").unwrap(),
        "litellm",
        Some("litellm-key"),
        Some("https://litellm.example/proxy/v1"),
    );
    assert_compatible_config(
        environment.providers.get("openai_compatible").unwrap(),
        "openai_compatible",
        Some("compatible-key"),
        Some("https://compatible.example/custom/v1"),
    );

    let client = Client::from_env_map(&env, Some("litellm")).unwrap();
    assert_eq!(client.default_provider(), Some("litellm"));
    assert_eq!(
        client.provider_names().collect::<Vec<_>>(),
        vec!["openrouter", "litellm", "openai_compatible"]
    );

    let prepared = build_openai_compatible_chat_request(
        "openai_compatible",
        &Request {
            model: "explicit-compatible-model".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        },
        environment.providers.get("openai_compatible").unwrap(),
    )
    .unwrap();
    assert_eq!(
        prepared.url,
        "https://compatible.example/custom/v1/chat/completions"
    );
    assert_eq!(prepared.headers["Authorization"], "Bearer compatible-key");
    assert_eq!(prepared.body["model"], json!("explicit-compatible-model"));
}

#[derive(Clone, Copy)]
enum CompatibleProvider {
    OpenAICompatible,
    OpenRouter,
    LiteLLM,
}

impl CompatibleProvider {
    const ALL: [Self; 3] = [Self::OpenAICompatible, Self::OpenRouter, Self::LiteLLM];

    fn name(self) -> &'static str {
        match self {
            Self::OpenAICompatible => "openai_compatible",
            Self::OpenRouter => "openrouter",
            Self::LiteLLM => "litellm",
        }
    }

    fn model(self) -> &'static str {
        match self {
            Self::OpenAICompatible => "local/compatible-model",
            Self::OpenRouter => "anthropic/claude-sonnet-4.5",
            Self::LiteLLM => "litellm/team-model",
        }
    }

    fn chat_completions_url(self) -> &'static str {
        match self {
            Self::OpenAICompatible => "https://compatible.example/custom/v1/chat/completions",
            Self::OpenRouter => "https://openrouter.ai/api/v1/chat/completions",
            Self::LiteLLM => "https://litellm.example/proxy/v1/chat/completions",
        }
    }

    fn uses_authorization_header(self) -> bool {
        true
    }

    fn adapter(self, transport: Arc<RecordingTransport>) -> Arc<dyn ProviderAdapter> {
        let transport: Arc<dyn NativeCompleteTransport> = transport;
        match self {
            Self::OpenAICompatible => Arc::new(OpenAICompatibleAdapter::openai_compatible(
                OpenAICompatibleRequestConfig {
                    api_key: Some(format!("{}-key", self.name())),
                    base_url: Some("https://compatible.example/custom/responses".to_string()),
                    require_api_key: true,
                    ..OpenAICompatibleRequestConfig::default()
                },
                transport,
            )),
            Self::OpenRouter => Arc::new(
                OpenRouterAdapter::new(
                    OpenAICompatibleRequestConfig {
                        api_key: Some(format!("{}-key", self.name())),
                        require_api_key: true,
                        ..OpenAICompatibleRequestConfig::default()
                    },
                    transport,
                )
                .unwrap(),
            ),
            Self::LiteLLM => Arc::new(
                LiteLLMAdapter::new(
                    OpenAICompatibleRequestConfig {
                        api_key: Some(format!("{}-key", self.name())),
                        base_url: Some("https://litellm.example/proxy/responses".to_string()),
                        ..OpenAICompatibleRequestConfig::default()
                    },
                    transport,
                )
                .unwrap(),
            ),
        }
    }

    fn success_response(self) -> NativeCompleteResponse {
        NativeCompleteResponse {
            status: 200,
            headers: BTreeMap::from([(
                "x-ratelimit-remaining-requests".to_string(),
                "7".to_string(),
            )]),
            body: json!({
                "id": format!("chatcmpl_{}", self.name()),
                "model": self.model(),
                "choices": [{
                    "message": {"role": "assistant", "content": "compatible complete"},
                    "finish_reason": "stop",
                }],
                "usage": {
                    "prompt_tokens": 8,
                    "completion_tokens": 5,
                    "total_tokens": 13,
                    "prompt_tokens_details": {"cached_tokens": 3},
                    "completion_tokens_details": {"reasoning_tokens": 2},
                },
            }),
        }
    }

    fn stream_payload(self) -> String {
        [
            sse(json!({
                "id": format!("chatcmpl_stream_{}", self.name()),
                "model": self.model(),
                "choices": [{"index": 0, "delta": {"role": "assistant"}}],
            })),
            sse(json!({
                "id": format!("chatcmpl_stream_{}", self.name()),
                "model": self.model(),
                "choices": [{"index": 0, "delta": {"content": "streamed "}}],
            })),
            sse(json!({
                "id": format!("chatcmpl_stream_{}", self.name()),
                "model": self.model(),
                "choices": [{
                    "index": 0,
                    "delta": {"content": "compatible"},
                    "finish_reason": "stop",
                }],
                "usage": {
                    "prompt_tokens": 10,
                    "completion_tokens": 4,
                    "total_tokens": 14,
                    "completion_tokens_details": {"reasoning_tokens": 1},
                },
            })),
            "data: [DONE]\n\n".to_string(),
        ]
        .join("")
    }
}

fn provider_options(provider: CompatibleProvider) -> BTreeMap<String, Value> {
    BTreeMap::from([
        (
            provider.name().to_string(),
            json!({
                "headers": {"X-Matrix-Provider": provider.name()},
                "parallel_tool_calls": false,
                "tools": [
                    {"type": "web_search_preview"},
                    {
                        "type": "function",
                        "function": {
                            "name": "matrix_lookup",
                            "description": "Lookup from the parity matrix",
                            "parameters": {"type": "object"},
                        },
                    },
                ],
                "previous_response_id": "resp_native",
                "conversation": "native-responses-only-conversation",
                "max_output_tokens": 256,
            }),
        ),
        (
            "openai".to_string(),
            json!({
                "reasoning": {"effort": "high"},
                "tools": [{"type": "web_search_preview"}],
            }),
        ),
        (
            "anthropic".to_string(),
            json!({"beta_headers": ["must-not-leak"]}),
        ),
    ])
}

fn assert_provider_error(
    provider: CompatibleProvider,
    response: NativeCompleteResponse,
    expected_kind: AdapterErrorKind,
    retryable: bool,
    retry_after: Option<f64>,
    error_code: &str,
) {
    let transport = Arc::new(RecordingTransport::new([Ok(response.clone())]));
    let client =
        Client::from_adapters([provider.adapter(transport.clone())], Some(provider.name()))
            .unwrap();

    let error = client
        .complete(Request {
            model: provider.model().to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap_err();

    assert_eq!(
        transport.only_request().url,
        provider.chat_completions_url()
    );
    assert_eq!(error.kind, expected_kind);
    assert_eq!(error.provider.as_deref(), Some(provider.name()));
    assert_eq!(error.status_code, Some(response.status));
    assert_eq!(error.error_code.as_deref(), Some(error_code));
    assert_eq!(error.retryable, retryable);
    assert_eq!(error.retry_after, retry_after);
    assert_eq!(error.raw, Some(response.body));
}

fn assert_warning_codes<const N: usize>(response: &Response, expected: [&str; N]) {
    let codes = response
        .warnings
        .iter()
        .filter_map(|warning| warning.code.as_deref())
        .collect::<BTreeSet<_>>();
    for code in expected {
        assert!(
            codes.contains(code),
            "missing warning code {code:?}; saw {codes:?}"
        );
    }
}

fn assert_compatible_config(
    config: &ProviderConfig,
    provider: &str,
    api_key: Option<&str>,
    base_url: Option<&str>,
) {
    assert_eq!(config.provider, provider);
    assert_eq!(config.api_key.as_deref(), api_key);
    assert_eq!(config.base_url.as_deref(), base_url);
}

struct RecordingTransport {
    requests: Mutex<Vec<NativeCompleteRequest>>,
    responses: Mutex<VecDeque<Result<NativeCompleteResponse, AdapterError>>>,
    stream_responses: Mutex<VecDeque<Result<NativeStreamResponse, AdapterError>>>,
}

impl RecordingTransport {
    fn new(
        responses: impl IntoIterator<Item = Result<NativeCompleteResponse, AdapterError>>,
    ) -> Self {
        Self::new_with_streams(responses, std::iter::empty())
    }

    fn new_with_streams(
        responses: impl IntoIterator<Item = Result<NativeCompleteResponse, AdapterError>>,
        stream_responses: impl IntoIterator<Item = Result<NativeStreamResponse, AdapterError>>,
    ) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(responses.into_iter().collect()),
            stream_responses: Mutex::new(stream_responses.into_iter().collect()),
        }
    }

    fn only_request(&self) -> NativeCompleteRequest {
        let requests = self.requests.lock().expect("recorded compatible requests");
        assert_eq!(requests.len(), 1);
        requests[0].clone()
    }

    fn request_count(&self) -> usize {
        self.requests
            .lock()
            .expect("recorded compatible requests")
            .len()
    }
}

impl NativeCompleteTransport for RecordingTransport {
    fn complete(
        &self,
        request: NativeCompleteRequest,
    ) -> Result<NativeCompleteResponse, AdapterError> {
        self.requests
            .lock()
            .expect("recorded compatible requests")
            .push(request);
        self.responses
            .lock()
            .expect("compatible responses")
            .pop_front()
            .expect("compatible response queued")
    }

    fn stream(&self, request: NativeCompleteRequest) -> Result<NativeStreamResponse, AdapterError> {
        self.requests
            .lock()
            .expect("recorded compatible requests")
            .push(request);
        self.stream_responses
            .lock()
            .expect("compatible stream responses")
            .pop_front()
            .expect("compatible stream response queued")
    }
}

fn sse(payload: Value) -> String {
    format!("data: {}\n\n", serde_json::to_string(&payload).unwrap())
}
