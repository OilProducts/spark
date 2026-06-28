use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use serde_json::{json, Value};
use unified_llm_adapter::{
    AdapterError, AdapterErrorKind, Client, ContentPart, FinishReasonKind, ImageData, Message,
    MessageRole, NativeCompleteRequest, NativeCompleteResponse, NativeCompleteTransport,
    NativeProviderAdapter, NativeRequestConfig, NativeStreamResponse, ProviderAdapter, Request,
    ResponseFormat, StreamAccumulator, StreamEventType,
};

#[test]
fn native_complete_parity_matrix_covers_text_images_structured_usage_and_provider_options() {
    let cwd = std::env::current_dir().unwrap();
    let temp_dir = tempfile::tempdir_in(&cwd).unwrap();
    let local_image = temp_dir.path().join("diagram.jpg");
    fs::write(&local_image, b"local-image").unwrap();
    let relative_local_image = relative_dot_path(&cwd, &local_image);
    let inline_encoded = encode_base64(b"inline-image");
    let local_encoded = encode_base64(b"local-image");

    for provider in NativeProvider::ALL {
        let transport = Arc::new(RecordingTransport::new([Ok(NativeCompleteResponse {
            status: 200,
            headers: provider.success_headers(),
            body: provider.complete_payload(),
        })]));
        let native_transport: Arc<dyn NativeCompleteTransport> = transport.clone();
        let client = Client::from_adapters(
            [provider.adapter(provider.config(), native_transport)],
            Some(provider.name()),
        )
        .unwrap();

        let response = client
            .complete(parity_request(provider, relative_local_image.clone()))
            .unwrap();
        let captured = transport.only_request();

        assert_eq!(captured.provider, provider.name());
        assert_eq!(captured.method, "POST");
        assert!(captured.url.contains(provider.complete_path_marker()));
        provider.assert_model_selection(&captured);
        assert_no_cross_provider_option_leak(provider, &captured.body);
        provider.assert_active_options_and_images(
            &captured,
            inline_encoded.as_str(),
            local_encoded.as_str(),
        );

        assert_eq!(response.provider, provider.name());
        assert_eq!(response.text(), "visible answer");
        assert_eq!(
            response.reasoning().as_deref(),
            Some(provider.reasoning_text())
        );
        assert_eq!(response.finish_reason.reason, FinishReasonKind::Stop);
        assert_eq!(response.usage.input_tokens, 11);
        assert_eq!(response.usage.output_tokens, 13);
        assert_eq!(response.usage.total_tokens, 24);
        assert_eq!(
            response.usage.reasoning_tokens,
            Some(provider.reasoning_tokens())
        );
        assert_eq!(
            response.usage.cache_read_tokens,
            Some(provider.cache_read_tokens())
        );
        assert_eq!(
            response.raw.as_ref().and_then(|raw| provider.raw_id(raw)),
            Some(provider.id())
        );
        assert_eq!(
            response
                .rate_limit
                .as_ref()
                .and_then(|rate_limit| rate_limit.requests_remaining),
            Some(7)
        );
    }
}

#[test]
fn native_streaming_parity_matrix_deltas_match_finish_response_text() {
    for provider in NativeProvider::ALL {
        let transport = Arc::new(RecordingTransport::new_with_streams(
            std::iter::empty(),
            [Ok(NativeStreamResponse::ok(provider.stream_payloads()))],
        ));
        let native_transport: Arc<dyn NativeCompleteTransport> = transport.clone();
        let client = Client::from_adapters(
            [provider.adapter(provider.config(), native_transport)],
            Some(provider.name()),
        )
        .unwrap();

        let events = client
            .stream(Request {
                model: provider.model().to_string(),
                messages: vec![Message::user("hello")],
                ..Request::default()
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let captured = transport.only_request();
        let deltas = events
            .iter()
            .filter(|event| event.r#type == StreamEventType::TextDelta)
            .filter_map(|event| event.delta.as_deref())
            .collect::<String>();
        let finish_event = events
            .iter()
            .find(|event| event.r#type == StreamEventType::Finish)
            .expect("stream should finish");
        let finish_text = finish_event.response.as_ref().unwrap().text();
        let accumulated = StreamAccumulator::from_events(events).response;

        assert_eq!(captured.provider, provider.name());
        assert!(captured.url.contains(provider.stream_path_marker()));
        assert_eq!(deltas, "Hello");
        assert_eq!(deltas, finish_text);
        assert_eq!(deltas, accumulated.text());
        assert_eq!(
            accumulated.usage.reasoning_tokens,
            Some(provider.reasoning_tokens())
        );
        assert_eq!(accumulated.finish_reason.reason, FinishReasonKind::Stop);
    }
}

#[test]
fn native_error_parity_matrix_classifies_authentication_and_rate_limits() {
    for provider in NativeProvider::ALL {
        let raw_auth = provider.authentication_error_payload();
        assert_provider_error(
            provider,
            NativeCompleteResponse {
                status: 401,
                headers: BTreeMap::new(),
                body: raw_auth.clone(),
            },
            AdapterErrorKind::Authentication,
            false,
            None,
            provider.authentication_error_code(),
            raw_auth,
        );

        let raw_rate_limit = provider.rate_limit_error_payload();
        assert_provider_error(
            provider,
            NativeCompleteResponse {
                status: provider.rate_limit_status(),
                headers: BTreeMap::from([("Retry-After".to_string(), "4.5".to_string())]),
                body: raw_rate_limit.clone(),
            },
            AdapterErrorKind::RateLimit,
            true,
            Some(4.5),
            provider.rate_limit_error_code(),
            raw_rate_limit,
        );
    }
}

fn assert_provider_error(
    provider: NativeProvider,
    response: NativeCompleteResponse,
    expected_kind: AdapterErrorKind,
    retryable: bool,
    retry_after: Option<f64>,
    error_code: &str,
    raw: Value,
) {
    let transport = Arc::new(RecordingTransport::new([Ok(response.clone())]));
    let native_transport: Arc<dyn NativeCompleteTransport> = transport;
    let client = Client::from_adapters(
        [provider.adapter(provider.config(), native_transport)],
        Some(provider.name()),
    )
    .unwrap();

    let error = client
        .complete(Request {
            model: provider.model().to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap_err();

    assert_eq!(error.kind, expected_kind);
    assert_eq!(error.provider.as_deref(), Some(provider.name()));
    assert_eq!(error.status_code, Some(response.status));
    assert_eq!(error.error_code.as_deref(), Some(error_code));
    assert_eq!(error.retryable, retryable);
    assert_eq!(error.retry_after, retry_after);
    assert_eq!(error.raw, Some(raw));
}

fn parity_request(provider: NativeProvider, local_image: String) -> Request {
    Request {
        model: provider.model().to_string(),
        messages: vec![
            Message::system("system instructions"),
            Message {
                role: MessageRole::User,
                content: vec![
                    ContentPart::Text {
                        text: "inspect images".to_string(),
                    },
                    ContentPart::Image {
                        image: ImageData::url("https://example.test/cat.png"),
                    },
                    ContentPart::Image {
                        image: ImageData::data(
                            b"inline-image".to_vec(),
                            Some("image/png".to_string()),
                        ),
                    },
                    ContentPart::Image {
                        image: ImageData {
                            url: Some(local_image),
                            data: None,
                            media_type: None,
                            detail: Some("high".to_string()),
                        },
                    },
                ],
                ..Message::default()
            },
        ],
        response_format: Some(ResponseFormat::JsonSchema {
            json_schema: json!({
                "type": "object",
                "properties": {"answer": {"type": "string"}},
                "required": ["answer"],
            }),
            strict: true,
        }),
        reasoning_effort: Some("high".to_string()),
        provider_options: BTreeMap::from([
            (
                "openai".to_string(),
                json!({
                    "parallel_tool_calls": false,
                    "reasoning": {"summary": "auto"},
                    "tools": [{"type": "web_search_preview"}],
                    "unsupportedNativeKey": {"must": "drop"}
                }),
            ),
            (
                "anthropic".to_string(),
                json!({
                    "auto_cache": false,
                    "beta_headers": ["matrix-beta"],
                    "thinking": {"type": "enabled", "budget_tokens": 256},
                    "unsupportedNativeKey": {"must": "drop"}
                }),
            ),
            (
                "gemini".to_string(),
                json!({
                    "safetySettings": [{"category": "HARM_CATEGORY_DANGEROUS_CONTENT"}],
                    "thinkingConfig": {"includeThoughts": true},
                    "topK": 32,
                    "unsupportedNativeKey": {"must": "drop"}
                }),
            ),
        ]),
        ..Request::default()
    }
}

fn assert_no_cross_provider_option_leak(provider: NativeProvider, body: &Value) {
    let body = body.to_string();
    assert!(!body.contains("unsupportedNativeKey"));
    match provider {
        NativeProvider::OpenAi => {
            assert!(!body.contains("matrix-beta"));
            assert!(!body.contains("safetySettings"));
        }
        NativeProvider::Anthropic => {
            assert!(!body.contains("parallel_tool_calls"));
            assert!(!body.contains("safetySettings"));
        }
        NativeProvider::Gemini => {
            assert!(!body.contains("parallel_tool_calls"));
            assert!(!body.contains("matrix-beta"));
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeProvider {
    OpenAi,
    Anthropic,
    Gemini,
}

impl NativeProvider {
    const ALL: [Self; 3] = [Self::OpenAi, Self::Anthropic, Self::Gemini];

    fn name(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
        }
    }

    fn model(self) -> &'static str {
        match self {
            Self::OpenAi => "gpt-5.2",
            Self::Anthropic => "claude-sonnet-4-5",
            Self::Gemini => "gemini-3.1-pro-preview",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::OpenAi => "resp_matrix_openai",
            Self::Anthropic => "msg_matrix_anthropic",
            Self::Gemini => "resp_matrix_gemini",
        }
    }

    fn config(self) -> NativeRequestConfig {
        NativeRequestConfig::new(format!("{}-key", self.name()))
    }

    fn adapter(
        self,
        config: NativeRequestConfig,
        transport: Arc<dyn NativeCompleteTransport>,
    ) -> Arc<dyn ProviderAdapter> {
        match self {
            Self::OpenAi => Arc::new(NativeProviderAdapter::openai(config, transport)),
            Self::Anthropic => Arc::new(NativeProviderAdapter::anthropic(config, transport)),
            Self::Gemini => Arc::new(NativeProviderAdapter::gemini(config, transport)),
        }
    }

    fn complete_path_marker(self) -> &'static str {
        match self {
            Self::OpenAi => "/v1/responses",
            Self::Anthropic => "/v1/messages",
            Self::Gemini => ":generateContent",
        }
    }

    fn stream_path_marker(self) -> &'static str {
        match self {
            Self::OpenAi => "/v1/responses",
            Self::Anthropic => "/v1/messages",
            Self::Gemini => ":streamGenerateContent",
        }
    }

    fn success_headers(self) -> BTreeMap<String, String> {
        match self {
            Self::OpenAi | Self::Gemini => BTreeMap::from([
                (
                    "x-ratelimit-remaining-requests".to_string(),
                    "7".to_string(),
                ),
                ("x-ratelimit-limit-requests".to_string(), "10".to_string()),
            ]),
            Self::Anthropic => BTreeMap::from([
                (
                    "anthropic-ratelimit-requests-remaining".to_string(),
                    "7".to_string(),
                ),
                (
                    "anthropic-ratelimit-requests-limit".to_string(),
                    "10".to_string(),
                ),
            ]),
        }
    }

    fn complete_payload(self) -> Value {
        match self {
            Self::OpenAi => json!({
                "id": self.id(),
                "model": self.model(),
                "status": "completed",
                "output": [
                    {"type": "reasoning", "text": self.reasoning_text()},
                    {"type": "output_text", "text": "visible answer"}
                ],
                "usage": {
                    "input_tokens": 11,
                    "output_tokens": 13,
                    "output_tokens_details": {"reasoning_tokens": self.reasoning_tokens()},
                    "input_tokens_details": {"cached_tokens": self.cache_read_tokens()}
                }
            }),
            Self::Anthropic => json!({
                "id": self.id(),
                "type": "message",
                "model": self.model(),
                "content": [
                    {
                        "type": "thinking",
                        "thinking": self.reasoning_text(),
                        "signature": "sig-anthropic-matrix"
                    },
                    {"type": "text", "text": "visible answer"}
                ],
                "stop_reason": "end_turn",
                "usage": {
                    "input_tokens": 11,
                    "output_tokens": 13,
                    "reasoning_tokens": self.reasoning_tokens(),
                    "cache_read_input_tokens": self.cache_read_tokens()
                }
            }),
            Self::Gemini => json!({
                "responseId": self.id(),
                "modelVersion": self.model(),
                "candidates": [{
                    "content": {
                        "role": "model",
                        "parts": [
                            {
                                "text": self.reasoning_text(),
                                "thought": true,
                                "thoughtSignature": "sig-gemini-matrix"
                            },
                            {"text": "visible answer"}
                        ]
                    },
                    "finishReason": "STOP"
                }],
                "usageMetadata": {
                    "promptTokenCount": 11,
                    "candidatesTokenCount": 13,
                    "totalTokenCount": 24,
                    "thoughtsTokenCount": self.reasoning_tokens(),
                    "cachedContentTokenCount": self.cache_read_tokens()
                }
            }),
        }
    }

    fn stream_payloads(self) -> Vec<Value> {
        match self {
            Self::OpenAi => vec![
                json!({
                    "type": "response.created",
                    "response": {"id": self.id(), "model": self.model()}
                }),
                json!({"type": "response.output_text.delta", "delta": "Hel"}),
                json!({"type": "response.output_text.delta", "delta": "lo"}),
                json!({
                    "type": "response.completed",
                    "response": {
                        "id": self.id(),
                        "model": self.model(),
                        "status": "completed",
                        "output": [
                            {"type": "reasoning", "text": self.reasoning_text()},
                            {"type": "output_text", "text": "Hello"}
                        ],
                        "usage": {
                            "input_tokens": 11,
                            "output_tokens": 13,
                            "output_tokens_details": {"reasoning_tokens": self.reasoning_tokens()}
                        }
                    }
                }),
            ],
            Self::Anthropic => vec![
                json!({
                    "type": "message_start",
                    "message": {
                        "id": self.id(),
                        "model": self.model(),
                        "content": [],
                        "stop_reason": null,
                        "usage": {"input_tokens": 11, "output_tokens": 0}
                    }
                }),
                json!({
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {"type": "text"}
                }),
                json!({
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {"type": "text_delta", "text": "Hel"}
                }),
                json!({
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {"type": "text_delta", "text": "lo"}
                }),
                json!({"type": "content_block_stop", "index": 0}),
                json!({
                    "type": "content_block_start",
                    "index": 1,
                    "content_block": {"type": "thinking", "thinking": self.reasoning_text()}
                }),
                json!({"type": "content_block_stop", "index": 1}),
                json!({
                    "type": "message_delta",
                    "delta": {"stop_reason": "end_turn"},
                    "usage": {
                        "output_tokens": 13,
                        "reasoning_tokens": self.reasoning_tokens()
                    }
                }),
                json!({"type": "message_stop"}),
            ],
            Self::Gemini => vec![
                json!({
                    "responseId": self.id(),
                    "modelVersion": self.model(),
                    "candidates": [{
                        "content": {"role": "model", "parts": [{"text": "Hel"}]}
                    }]
                }),
                json!({
                    "responseId": self.id(),
                    "modelVersion": self.model(),
                    "candidates": [{
                        "content": {
                            "role": "model",
                            "parts": [
                                {"text": "Hello"},
                                {"text": self.reasoning_text(), "thought": true}
                            ]
                        },
                        "finishReason": "STOP"
                    }],
                    "usageMetadata": {
                        "promptTokenCount": 11,
                        "candidatesTokenCount": 13,
                        "totalTokenCount": 24,
                        "thoughtsTokenCount": self.reasoning_tokens()
                    }
                }),
            ],
        }
    }

    fn assert_active_options_and_images(
        self,
        captured: &NativeCompleteRequest,
        inline_encoded: &str,
        local_encoded: &str,
    ) {
        match self {
            Self::OpenAi => {
                assert_eq!(captured.body["parallel_tool_calls"], json!(false));
                assert_eq!(
                    captured.body["reasoning"],
                    json!({"effort": "high", "summary": "auto"})
                );
                assert_eq!(
                    captured.body["response_format"]["type"],
                    json!("json_schema")
                );
                assert_eq!(
                    captured.body["input"][0]["content"],
                    json!([
                        {"type": "input_text", "text": "inspect images"},
                        {
                            "type": "input_image",
                            "image_url": "https://example.test/cat.png"
                        },
                        {
                            "type": "input_image",
                            "image_url": format!("data:image/png;base64,{inline_encoded}")
                        },
                        {
                            "type": "input_image",
                            "image_url": format!("data:image/jpeg;base64,{local_encoded}"),
                            "detail": "high"
                        }
                    ])
                );
            }
            Self::Anthropic => {
                assert_eq!(captured.headers["anthropic-beta"], "matrix-beta");
                assert_eq!(
                    captured.body["thinking"],
                    json!({"type": "enabled", "budget_tokens": 256})
                );
                assert_eq!(
                    captured.body["tool_choice"],
                    json!({"type": "tool", "name": "structured_output"})
                );
                assert_eq!(
                    captured.body["messages"][0]["content"],
                    json!([
                        {"type": "text", "text": "inspect images"},
                        {
                            "type": "image",
                            "source": {
                                "type": "url",
                                "url": "https://example.test/cat.png"
                            }
                        },
                        {
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": "image/png",
                                "data": inline_encoded
                            }
                        },
                        {
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": "image/jpeg",
                                "data": local_encoded
                            }
                        }
                    ])
                );
            }
            Self::Gemini => {
                assert_eq!(
                    captured.body["safetySettings"],
                    json!([{"category": "HARM_CATEGORY_DANGEROUS_CONTENT"}])
                );
                assert_eq!(captured.body["generationConfig"]["topK"], json!(32));
                assert_eq!(
                    captured.body["generationConfig"]["thinkingConfig"],
                    json!({"includeThoughts": true})
                );
                assert_eq!(
                    captured.body["generationConfig"]["responseMimeType"],
                    json!("application/json")
                );
                assert!(captured.body["generationConfig"]
                    .as_object()
                    .unwrap()
                    .contains_key("responseSchema"));
                assert_eq!(
                    captured.body["contents"][0]["parts"],
                    json!([
                        {"text": "inspect images"},
                        {
                            "fileData": {
                                "fileUri": "https://example.test/cat.png",
                                "mimeType": "image/png"
                            }
                        },
                        {
                            "inlineData": {
                                "data": inline_encoded,
                                "mimeType": "image/png"
                            }
                        },
                        {
                            "inlineData": {
                                "data": local_encoded,
                                "mimeType": "image/jpeg"
                            }
                        }
                    ])
                );
            }
        }
    }

    fn assert_model_selection(self, captured: &NativeCompleteRequest) {
        match self {
            Self::OpenAi | Self::Anthropic => {
                assert_eq!(captured.body["model"], json!(self.model()));
            }
            Self::Gemini => {
                assert!(captured
                    .url
                    .contains("/v1beta/models/gemini-3.1-pro-preview:generateContent"));
            }
        }
    }

    fn reasoning_text(self) -> &'static str {
        match self {
            Self::OpenAi => "openai reasoning",
            Self::Anthropic => "anthropic thinking",
            Self::Gemini => "gemini thought",
        }
    }

    fn reasoning_tokens(self) -> u64 {
        match self {
            Self::OpenAi => 3,
            Self::Anthropic => 5,
            Self::Gemini => 7,
        }
    }

    fn cache_read_tokens(self) -> u64 {
        match self {
            Self::OpenAi => 2,
            Self::Anthropic => 4,
            Self::Gemini => 6,
        }
    }

    fn raw_id(self, raw: &Value) -> Option<&str> {
        match self {
            Self::OpenAi | Self::Anthropic => raw.get("id").and_then(Value::as_str),
            Self::Gemini => raw.get("responseId").and_then(Value::as_str),
        }
    }

    fn authentication_error_payload(self) -> Value {
        match self {
            Self::OpenAi => json!({
                "error": {
                    "message": "invalid API key",
                    "code": "invalid_api_key",
                    "type": "authentication_error"
                }
            }),
            Self::Anthropic => json!({
                "type": "error",
                "error": {
                    "type": "authentication_error",
                    "message": "invalid x-api-key"
                }
            }),
            Self::Gemini => json!({
                "error": {
                    "code": 401,
                    "message": "API key not valid",
                    "status": "UNAUTHENTICATED"
                }
            }),
        }
    }

    fn rate_limit_error_payload(self) -> Value {
        match self {
            Self::OpenAi => json!({
                "error": {
                    "message": "too many requests",
                    "code": "rate_limit_exceeded",
                    "type": "rate_limit_error"
                }
            }),
            Self::Anthropic => json!({
                "type": "error",
                "error": {
                    "type": "rate_limit_error",
                    "message": "rate limited"
                }
            }),
            Self::Gemini => json!({
                "error": {
                    "code": 429,
                    "message": "resource exhausted",
                    "status": "RESOURCE_EXHAUSTED"
                }
            }),
        }
    }

    fn authentication_error_code(self) -> &'static str {
        match self {
            Self::OpenAi => "invalid_api_key",
            Self::Anthropic => "authentication_error",
            Self::Gemini => "UNAUTHENTICATED",
        }
    }

    fn rate_limit_error_code(self) -> &'static str {
        match self {
            Self::OpenAi => "rate_limit_exceeded",
            Self::Anthropic => "rate_limit_error",
            Self::Gemini => "RESOURCE_EXHAUSTED",
        }
    }

    fn rate_limit_status(self) -> u16 {
        match self {
            Self::OpenAi | Self::Anthropic => 429,
            Self::Gemini => 400,
        }
    }
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
        let requests = self.requests.lock().expect("recorded native requests");
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
            .expect("recorded native requests")
            .push(request);
        self.responses
            .lock()
            .expect("native responses")
            .pop_front()
            .expect("native response queued")
    }

    fn stream(&self, request: NativeCompleteRequest) -> Result<NativeStreamResponse, AdapterError> {
        self.requests
            .lock()
            .expect("recorded native requests")
            .push(request);
        self.stream_responses
            .lock()
            .expect("native stream responses")
            .pop_front()
            .expect("native stream response queued")
    }
}

fn relative_dot_path(cwd: &Path, path: &Path) -> String {
    format!(
        "./{}",
        path.strip_prefix(cwd)
            .expect("test temp file should be under current directory")
            .display()
    )
}

fn encode_base64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}
