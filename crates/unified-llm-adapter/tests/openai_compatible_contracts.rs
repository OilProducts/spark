use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use unified_llm_adapter::{
    build_openai_compatible_chat_request, AdapterError, AdapterErrorKind, Client, ContentPart,
    FinishReasonKind, ImageData, LiteLLMAdapter, Message, MessageRole, NativeCompleteRequest,
    NativeCompleteResponse, NativeCompleteTransport, NativeStreamResponse, OpenAICompatibleAdapter,
    OpenAICompatibleRequestConfig, OpenRouterAdapter, ProviderAdapter, ProviderEnvironment,
    Request, ResponseFormat, StreamAccumulator, StreamEventType, ToolCall,
};

#[test]
fn compatible_request_uses_chat_completions_and_active_provider_options_only() {
    let image_bytes = b"image-bytes".to_vec();
    let request = Request {
        model: "team-model".to_string(),
        messages: vec![
            Message::system("system instructions"),
            Message::developer("developer instructions"),
            Message {
                role: MessageRole::User,
                content: vec![
                    ContentPart::Text {
                        text: "show weather".to_string(),
                    },
                    ContentPart::Image {
                        image: ImageData::url("https://example.test/image.png"),
                    },
                    ContentPart::Image {
                        image: ImageData::data(image_bytes, Some("image/png".to_string())),
                    },
                ],
                ..Message::default()
            },
            Message {
                role: MessageRole::Assistant,
                content: vec![
                    ContentPart::Text {
                        text: "I need a tool".to_string(),
                    },
                    ContentPart::ToolCall {
                        tool_call: tool_call(
                            "call_123",
                            "lookup_weather",
                            json!({"city": "Paris"}),
                        ),
                    },
                ],
                ..Message::default()
            },
            Message::tool_result("call_123", json!({"temperature": 72, "unit": "F"}), false),
        ],
        tools: vec![tool("lookup_weather")],
        tool_choice: Some(json!({"mode": "named", "tool_name": "lookup_weather"})),
        response_format: Some(ResponseFormat::JsonSchema {
            json_schema: json!({
                "type": "object",
                "properties": {"answer": {"type": "string"}},
            }),
            strict: true,
        }),
        temperature: Some(0.2),
        top_p: Some(0.9),
        max_tokens: Some(256),
        stop_sequences: vec!["END".to_string()],
        provider_options: BTreeMap::from([
            (
                "openai_compatible".to_string(),
                json!({
                    "parallel_tool_calls": false,
                    "headers": {"X-Request-ID": "req-123"},
                }),
            ),
            (
                "openai".to_string(),
                json!({"reasoning": {"effort": "high"}}),
            ),
            (
                "anthropic".to_string(),
                json!({"beta_headers": ["must-not-leak"]}),
            ),
        ]),
        ..Request::default()
    };
    let config = OpenAICompatibleRequestConfig {
        api_key: Some("compatible-key".to_string()),
        base_url: Some("https://compatible.example/api/responses".to_string()),
        default_headers: BTreeMap::from([
            ("Authorization".to_string(), "wrong".to_string()),
            ("X-Custom".to_string(), "value".to_string()),
        ]),
        require_api_key: true,
    };

    let prepared =
        build_openai_compatible_chat_request("openai_compatible", &request, config).unwrap();

    assert_eq!(prepared.method, "POST");
    assert_eq!(
        prepared.url,
        "https://compatible.example/api/v1/chat/completions"
    );
    assert!(!prepared.url.contains("/responses"));
    assert_eq!(prepared.headers["Authorization"], "Bearer compatible-key");
    assert_eq!(prepared.headers["X-Custom"], "value");
    assert_eq!(prepared.headers["X-Request-ID"], "req-123");
    assert_eq!(prepared.body["model"], json!("team-model"));
    assert_eq!(prepared.body["parallel_tool_calls"], json!(false));
    assert!(!prepared.body.to_string().contains("must-not-leak"));
    assert!(!prepared.body.to_string().contains("reasoning"));
    assert_eq!(
        prepared.body["tool_choice"],
        json!({"type": "function", "function": {"name": "lookup_weather"}})
    );
    assert_eq!(
        prepared.body["response_format"],
        json!({
            "type": "json_schema",
            "json_schema": {
                "type": "object",
                "properties": {"answer": {"type": "string"}},
            },
            "strict": true,
        })
    );
    assert_eq!(
        prepared.body["tools"],
        json!([{
            "type": "function",
            "function": {
                "name": "lookup_weather",
                "description": "Lookup a fact",
                "parameters": {
                    "type": "object",
                    "properties": {"query": {"type": "string"}},
                },
            },
        }])
    );
    assert_eq!(
        prepared.body["messages"],
        json!([
            {"role": "system", "content": "system instructions"},
            {"role": "developer", "content": "developer instructions"},
            {
                "role": "user",
                "content": [
                    {"type": "text", "text": "show weather"},
                    {"type": "image_url", "image_url": {"url": "https://example.test/image.png"}},
                    {"type": "image_url", "image_url": {"url": "data:image/png;base64,aW1hZ2UtYnl0ZXM="}},
                ],
            },
            {
                "role": "assistant",
                "content": "I need a tool",
                "tool_calls": [{
                    "id": "call_123",
                    "type": "function",
                    "function": {
                        "name": "lookup_weather",
                        "arguments": "{\"city\":\"Paris\"}",
                    },
                }],
            },
            {
                "role": "tool",
                "tool_call_id": "call_123",
                "content": "{\"temperature\":72,\"unit\":\"F\"}",
            },
        ])
    );
    assert_eq!(
        request.provider_options["openai"]["reasoning"]["effort"],
        "high"
    );
}

#[test]
fn compatible_adapter_complete_translates_response_usage_errors_and_warnings() {
    let transport = Arc::new(RecordingTransport::new([Ok(NativeCompleteResponse {
        status: 200,
        headers: BTreeMap::from([
            (
                "x-ratelimit-remaining-requests".to_string(),
                "7".to_string(),
            ),
            ("x-ratelimit-remaining-tokens".to_string(), "99".to_string()),
        ]),
        body: json!({
            "id": "chatcmpl_123",
            "model": "team-model",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Hello",
                    "tool_calls": [{
                        "id": "call_123",
                        "type": "function",
                        "function": {
                            "name": "lookup_weather",
                            "arguments": "{\"city\":\"Paris\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {
                "prompt_tokens": 12,
                "completion_tokens": 34,
                "total_tokens": 46,
                "completion_tokens_details": {"reasoning_tokens": 5}
            }
        }),
    })]));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(OpenAICompatibleAdapter::openai_compatible(
        OpenAICompatibleRequestConfig::new("compatible-key"),
        transport.clone(),
    ));
    let client = Client::from_adapters([adapter], Some("openai_compatible")).unwrap();

    let response = client
        .complete(Request {
            model: "team-model".to_string(),
            messages: vec![Message::user("hello")],
            reasoning_effort: Some("high".to_string()),
            provider_options: BTreeMap::from([(
                "openai_compatible".to_string(),
                json!({
                    "tools": [{"type": "web_search_preview"}],
                    "previous_response_id": "resp_123"
                }),
            )]),
            ..Request::default()
        })
        .unwrap();

    let captured = transport.only_request();
    assert_eq!(captured.url, "https://api.openai.com/v1/chat/completions");
    assert!(!captured.body.to_string().contains("web_search_preview"));
    assert!(!captured.body.to_string().contains("previous_response_id"));
    assert_eq!(response.provider, "openai_compatible");
    assert_eq!(response.id, "chatcmpl_123");
    assert_eq!(response.model, "team-model");
    assert_eq!(response.text(), "Hello");
    assert_eq!(response.finish_reason.reason, FinishReasonKind::ToolCalls);
    assert_eq!(response.usage.input_tokens, 12);
    assert_eq!(response.usage.output_tokens, 34);
    assert_eq!(response.usage.total_tokens, 46);
    assert_eq!(response.usage.reasoning_tokens, None);
    assert_eq!(
        response.rate_limit.as_ref().unwrap().tokens_remaining,
        Some(99)
    );
    assert_eq!(
        response.tool_calls(),
        vec![tool_call(
            "call_123",
            "lookup_weather",
            json!({"city": "Paris"})
        )]
    );
    let warning_codes = response
        .warnings
        .iter()
        .filter_map(|warning| warning.code.as_deref())
        .collect::<Vec<_>>();
    assert!(warning_codes.contains(&"unsupported_reasoning_effort"));
    assert!(warning_codes.contains(&"unsupported_responses_tool"));
    assert!(warning_codes.contains(&"unsupported_responses_option"));
    assert!(warning_codes.contains(&"unsupported_reasoning_token_visibility"));
}

#[test]
fn compatible_adapter_complete_preserves_http_error_metadata() {
    let transport = Arc::new(RecordingTransport::new([Ok(NativeCompleteResponse {
        status: 429,
        headers: BTreeMap::from([("Retry-After".to_string(), "7".to_string())]),
        body: json!({
            "error": {
                "message": "slow down",
                "code": "rate_limit",
            }
        }),
    })]));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(OpenAICompatibleAdapter::openai_compatible(
        OpenAICompatibleRequestConfig::new("compatible-key"),
        transport,
    ));
    let client = Client::from_adapters([adapter], Some("openai_compatible")).unwrap();

    let error = client
        .complete(Request {
            model: "team-model".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::RateLimit);
    assert_eq!(error.provider.as_deref(), Some("openai_compatible"));
    assert_eq!(error.message, "slow down");
    assert_eq!(error.status_code, Some(429));
    assert_eq!(error.error_code.as_deref(), Some("rate_limit"));
    assert_eq!(error.retry_after, Some(7.0));
    assert_eq!(error.raw.as_ref().unwrap()["error"]["message"], "slow down");
}

#[test]
fn compatible_stream_translates_text_tool_call_usage_and_done_lifecycle() {
    let payload = [
        sse(json!({
            "id": "chatcmpl_stream",
            "model": "team-model",
            "choices": [{"index": 0, "delta": {"role": "assistant"}}]
        })),
        sse(json!({
            "id": "chatcmpl_stream",
            "model": "team-model",
            "choices": [{"index": 0, "delta": {"content": "Hel"}}]
        })),
        sse(json!({
            "id": "chatcmpl_stream",
            "model": "team-model",
            "choices": [{"index": 0, "delta": {"content": "lo"}}]
        })),
        sse(json!({
            "id": "chatcmpl_stream",
            "model": "team-model",
            "choices": [{
                "index": 0,
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_123",
                        "type": "function",
                        "function": {
                            "name": "lookup_weather",
                            "arguments": "{\"city\":"
                        }
                    }]
                }
            }]
        })),
        sse(json!({
            "id": "chatcmpl_stream",
            "model": "team-model",
            "choices": [{
                "index": 0,
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "function": {"arguments": "\"Paris\"}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {
                "prompt_tokens": 4,
                "completion_tokens": 5,
                "total_tokens": 9
            }
        })),
        "data: [DONE]\n\n".to_string(),
    ]
    .join("");
    let transport = Arc::new(RecordingTransport::new_with_streams(
        std::iter::empty(),
        [Ok(NativeStreamResponse {
            status: 200,
            headers: BTreeMap::from([(
                "x-ratelimit-remaining-requests".to_string(),
                "3".to_string(),
            )]),
            body: vec![Ok(Value::String(payload))],
        })],
    ));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(OpenAICompatibleAdapter::openai_compatible(
        OpenAICompatibleRequestConfig::new("stream-key"),
        transport.clone(),
    ));
    let client = Client::from_adapters([adapter], Some("openai_compatible")).unwrap();

    let events = client
        .stream(Request {
            model: "team-model".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let response = StreamAccumulator::from_events(events.clone()).response;

    let captured = transport.only_request();
    assert_eq!(captured.method, "POST");
    assert_eq!(captured.url, "https://api.openai.com/v1/chat/completions");
    assert_eq!(captured.body["stream"], json!(true));
    assert_eq!(
        events.iter().map(|event| &event.r#type).collect::<Vec<_>>(),
        vec![
            &StreamEventType::StreamStart,
            &StreamEventType::TextStart,
            &StreamEventType::TextDelta,
            &StreamEventType::TextDelta,
            &StreamEventType::ToolCallStart,
            &StreamEventType::ToolCallDelta,
            &StreamEventType::ToolCallDelta,
            &StreamEventType::TextEnd,
            &StreamEventType::ToolCallEnd,
            &StreamEventType::Finish,
        ]
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.r#type == StreamEventType::TextDelta)
            .filter_map(|event| event.delta.as_deref())
            .collect::<Vec<_>>(),
        vec!["Hel", "lo"]
    );
    let final_tool_call = events
        .iter()
        .find(|event| event.r#type == StreamEventType::ToolCallEnd)
        .and_then(|event| event.tool_call.clone())
        .expect("tool call end");
    assert_eq!(final_tool_call.id, "call_123");
    assert_eq!(final_tool_call.name, "lookup_weather");
    assert_eq!(final_tool_call.arguments, json!({"city": "Paris"}));
    assert_eq!(response.text(), "Hello");
    assert_eq!(response.finish_reason.reason, FinishReasonKind::ToolCalls);
    assert_eq!(response.usage.total_tokens, 9);
    assert_eq!(response.rate_limit.unwrap().requests_remaining, Some(3));
    assert_eq!(response.raw.as_ref().unwrap()["usage"]["total_tokens"], 9);
}

#[test]
fn compatible_stream_surfaces_provider_error_payloads_as_error_events() {
    let payload = sse(json!({
        "error": {
            "message": "provider stream failed",
            "code": "server_error"
        }
    }));
    let transport = Arc::new(RecordingTransport::new_with_streams(
        std::iter::empty(),
        [Ok(NativeStreamResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: vec![Ok(Value::String(payload))],
        })],
    ));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(OpenAICompatibleAdapter::openai_compatible(
        OpenAICompatibleRequestConfig::new("stream-key"),
        transport,
    ));
    let client = Client::from_adapters([adapter], Some("openai_compatible")).unwrap();

    let events = client
        .stream(Request {
            model: "team-model".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(
        events.iter().map(|event| &event.r#type).collect::<Vec<_>>(),
        vec![&StreamEventType::StreamStart, &StreamEventType::Error]
    );
    let error_event = events.last().expect("error event");
    let error = error_event.error.as_ref().expect("stream error");
    assert_eq!(error.kind, AdapterErrorKind::Stream);
    assert_eq!(error.provider.as_deref(), Some("openai_compatible"));
    assert_eq!(error.message, "provider stream failed");
    assert_eq!(
        error_event.response.as_ref().unwrap().finish_reason.reason,
        FinishReasonKind::Error
    );
}

#[test]
fn openrouter_and_litellm_wrappers_enforce_configuration_and_headers() {
    let missing_openrouter_key =
        match OpenRouterAdapter::without_transport(OpenAICompatibleRequestConfig::default()) {
            Ok(_) => panic!("OpenRouter without an API key should fail by default"),
            Err(error) => error,
        };
    assert_eq!(missing_openrouter_key.kind, AdapterErrorKind::Configuration);
    assert_eq!(
        missing_openrouter_key.provider.as_deref(),
        Some("openrouter")
    );
    assert!(missing_openrouter_key
        .message
        .contains("OPENROUTER_API_KEY"));

    let missing_litellm_base =
        match LiteLLMAdapter::without_transport(OpenAICompatibleRequestConfig::default()) {
            Ok(_) => panic!("LiteLLM without a base URL should fail"),
            Err(error) => error,
        };
    assert_eq!(missing_litellm_base.kind, AdapterErrorKind::Configuration);
    assert_eq!(missing_litellm_base.provider.as_deref(), Some("litellm"));
    assert!(missing_litellm_base.message.contains("LITELLM_BASE_URL"));

    let openrouter_transport = Arc::new(RecordingTransport::new([Ok(NativeCompleteResponse::ok(
        json!({
            "id": "chatcmpl_openrouter",
            "model": "anthropic/claude-sonnet-4.5",
            "choices": [{"message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}]
        }),
    ))]));
    let openrouter: Arc<dyn ProviderAdapter> = Arc::new(
        OpenRouterAdapter::new(
            OpenAICompatibleRequestConfig {
                api_key: Some("openrouter-key".to_string()),
                default_headers: BTreeMap::from([
                    (
                        "HTTP-Referer".to_string(),
                        "https://spark.example".to_string(),
                    ),
                    ("X-Title".to_string(), "Spark".to_string()),
                ]),
                require_api_key: true,
                ..OpenAICompatibleRequestConfig::default()
            },
            openrouter_transport.clone(),
        )
        .unwrap(),
    );
    let client = Client::from_adapters([openrouter], Some("openrouter")).unwrap();
    let response = client
        .complete(Request {
            model: "anthropic/claude-sonnet-4.5".to_string(),
            messages: vec![Message::user("hello")],
            provider_options: BTreeMap::from([(
                "openrouter".to_string(),
                json!({"headers": {"X-Request-ID": "req-1"}}),
            )]),
            ..Request::default()
        })
        .unwrap();
    let captured = openrouter_transport.only_request();
    assert_eq!(response.provider, "openrouter");
    assert_eq!(
        captured.url,
        "https://openrouter.ai/api/v1/chat/completions"
    );
    assert_eq!(captured.headers["Authorization"], "Bearer openrouter-key");
    assert_eq!(captured.headers["HTTP-Referer"], "https://spark.example");
    assert_eq!(captured.headers["X-Title"], "Spark");
    assert_eq!(captured.headers["X-Request-ID"], "req-1");

    let litellm_transport = Arc::new(RecordingTransport::new([Ok(NativeCompleteResponse::ok(
        json!({
            "id": "chatcmpl_litellm",
            "model": "team-model",
            "choices": [{"message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}]
        }),
    ))]));
    let litellm: Arc<dyn ProviderAdapter> = Arc::new(
        LiteLLMAdapter::new(
            OpenAICompatibleRequestConfig {
                base_url: Some("https://litellm.example/proxy".to_string()),
                ..OpenAICompatibleRequestConfig::default()
            },
            litellm_transport.clone(),
        )
        .unwrap(),
    );
    let client = Client::from_adapters([litellm], Some("litellm")).unwrap();
    let response = client
        .complete(Request {
            model: "team-model".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap();
    let captured = litellm_transport.only_request();
    assert_eq!(response.provider, "litellm");
    assert_eq!(
        captured.url,
        "https://litellm.example/proxy/v1/chat/completions"
    );
    assert!(!captured.headers.contains_key("Authorization"));
}

#[test]
fn compatible_provider_environment_preserves_headers_and_requires_explicit_models() {
    let env = BTreeMap::from([
        (
            "OPENROUTER_API_KEY".to_string(),
            "openrouter-key".to_string(),
        ),
        (
            "OPENROUTER_BASE_URL".to_string(),
            "https://router.example/api/chat/completions".to_string(),
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
    ]);
    let environment = ProviderEnvironment::from_env_map(&env, None);

    assert_eq!(environment.default_provider.as_deref(), Some("openrouter"));
    let openrouter = environment
        .providers
        .get("openrouter")
        .expect("openrouter provider config");
    assert_eq!(openrouter.api_key.as_deref(), Some("openrouter-key"));
    assert_eq!(
        openrouter.base_url.as_deref(),
        Some("https://router.example/api/v1")
    );
    assert_eq!(
        openrouter.options.get("HTTP-Referer").map(String::as_str),
        Some("https://spark.example")
    );
    assert_eq!(
        openrouter.options.get("X-Title").map(String::as_str),
        Some("Spark")
    );

    let litellm = environment
        .providers
        .get("litellm")
        .expect("litellm provider config");
    assert_eq!(litellm.api_key.as_deref(), Some("litellm-key"));
    assert_eq!(
        litellm.base_url.as_deref(),
        Some("https://litellm.example/proxy/v1")
    );

    let client = Client::from_env_map(&env, None).unwrap();
    assert_eq!(
        client.provider_names().collect::<Vec<_>>(),
        vec!["openrouter", "litellm"]
    );
    let missing_model = client
        .complete(Request {
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap_err();
    assert_eq!(missing_model.kind, AdapterErrorKind::InvalidRequest);
    assert!(missing_model.message.contains("model"));

    let openrouter_missing_transport = client
        .complete(Request {
            model: "anthropic/claude-sonnet-4.5".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap_err();
    assert_eq!(
        openrouter_missing_transport.kind,
        AdapterErrorKind::Configuration
    );
    assert_eq!(
        openrouter_missing_transport.provider.as_deref(),
        Some("openrouter")
    );
    assert!(openrouter_missing_transport
        .message
        .contains("no HTTP transport is configured"));

    let litellm_client = Client::from_env_map(&env, Some("litellm")).unwrap();
    let litellm_missing_transport = litellm_client
        .complete(Request {
            model: "team-model".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap_err();
    assert_eq!(
        litellm_missing_transport.kind,
        AdapterErrorKind::Configuration
    );
    assert_eq!(
        litellm_missing_transport.provider.as_deref(),
        Some("litellm")
    );
    assert!(litellm_missing_transport
        .message
        .contains("no HTTP transport is configured"));
}

#[test]
fn env_configured_compatible_client_uses_real_adapter_boundary_without_placeholder() {
    let client = Client::from_env_map(
        &BTreeMap::from([(
            "OPENAI_COMPATIBLE_BASE_URL".to_string(),
            "https://compatible.example/v1".to_string(),
        )]),
        None,
    )
    .unwrap();
    assert_eq!(
        client.provider_names().collect::<Vec<_>>(),
        vec!["openai_compatible"]
    );

    let error = client
        .complete(Request {
            model: "compat-model".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::Configuration);
    assert_eq!(error.provider.as_deref(), Some("openai_compatible"));
    assert!(error.message.contains("no HTTP transport is configured"));
    assert!(!error
        .message
        .contains("no Rust provider adapter is registered"));
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

fn tool(name: &str) -> Value {
    json!({
        "name": name,
        "description": "Lookup a fact",
        "parameters": {
            "type": "object",
            "properties": {"query": {"type": "string"}},
        },
    })
}

fn tool_call(id: &str, name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        raw_arguments: Some(serde_json::to_string(&arguments).expect("arguments json")),
        arguments,
        r#type: "function".to_string(),
    }
}

fn sse(payload: Value) -> String {
    format!("data: {}\n\n", serde_json::to_string(&payload).unwrap())
}
