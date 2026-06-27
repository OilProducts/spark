use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use unified_llm_adapter::{
    AdapterError, AdapterErrorKind, Client, ContentPart, FinishReasonKind, ImageData, Message,
    NativeCompleteRequest, NativeCompleteResponse, NativeCompleteTransport, NativeProviderAdapter,
    NativeRequestConfig, ProviderAdapter, Request,
};

#[test]
fn openai_native_adapter_complete_uses_responses_transport_and_normalizes_response() {
    let transport = Arc::new(RecordingTransport::new([Ok(NativeCompleteResponse {
        status: 200,
        headers: BTreeMap::from([
            (
                "x-ratelimit-remaining-requests".to_string(),
                "7".to_string(),
            ),
            ("x-ratelimit-limit-requests".to_string(), "10".to_string()),
        ]),
        body: json!({
            "id": "resp_123",
            "model": "gpt-5.2",
            "status": "completed",
            "output": [
                {"type": "reasoning", "text": "private reasoning"},
                {"type": "output_text", "text": "visible answer"}
            ],
            "usage": {
                "input_tokens": 12,
                "output_tokens": 34,
                "output_tokens_details": {"reasoning_tokens": 5},
                "input_tokens_details": {"cached_tokens": 4}
            }
        }),
    })]));
    let config = NativeRequestConfig {
        api_key: Some("openai-key".to_string()),
        base_url: Some("https://openai.example/custom".to_string()),
        default_headers: BTreeMap::from([("X-Trace".to_string(), "trace-1".to_string())]),
        organization: Some("org-123".to_string()),
        project: Some("project-456".to_string()),
    };
    let native_transport: Arc<dyn NativeCompleteTransport> = transport.clone();
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(NativeProviderAdapter::openai(config, native_transport));
    let client = Client::from_adapters([adapter], Some("openai")).unwrap();

    let response = client
        .complete(Request {
            model: "gpt-5.2".to_string(),
            messages: vec![Message::system("system"), Message::user("hello")],
            reasoning_effort: Some("high".to_string()),
            provider_options: BTreeMap::from([
                (
                    "openai".to_string(),
                    json!({
                        "parallel_tool_calls": false,
                        "unsupportedNativeKey": {"must": "drop"}
                    }),
                ),
                (
                    "anthropic".to_string(),
                    json!({"beta_headers": ["must-not-leak"]}),
                ),
            ]),
            ..Request::default()
        })
        .unwrap();

    let captured = transport.only_request();
    assert_eq!(captured.provider, "openai");
    assert_eq!(captured.method, "POST");
    assert_eq!(captured.url, "https://openai.example/custom/v1/responses");
    assert_eq!(captured.headers["Authorization"], "Bearer openai-key");
    assert_eq!(captured.headers["OpenAI-Organization"], "org-123");
    assert_eq!(captured.headers["OpenAI-Project"], "project-456");
    assert_eq!(captured.headers["X-Trace"], "trace-1");
    assert_eq!(captured.body["model"], json!("gpt-5.2"));
    assert_eq!(captured.body["instructions"], json!("system"));
    assert_eq!(captured.body["reasoning"], json!({"effort": "high"}));
    assert_eq!(captured.body["parallel_tool_calls"], json!(false));
    assert_eq!(
        captured.body["input"],
        json!([{
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "hello"}]
        }])
    );
    assert!(!captured.body.to_string().contains("unsupportedNativeKey"));
    assert!(!captured.body.to_string().contains("must-not-leak"));

    assert_eq!(response.provider, "openai");
    assert_eq!(response.id, "resp_123");
    assert_eq!(response.text(), "visible answer");
    assert_eq!(response.reasoning().as_deref(), Some("private reasoning"));
    assert_eq!(response.finish_reason.reason, FinishReasonKind::Stop);
    assert_eq!(response.usage.input_tokens, 12);
    assert_eq!(response.usage.output_tokens, 34);
    assert_eq!(response.usage.total_tokens, 46);
    assert_eq!(response.usage.reasoning_tokens, Some(5));
    assert_eq!(response.usage.cache_read_tokens, Some(4));
    assert_eq!(response.rate_limit.unwrap().requests_remaining, Some(7));
    assert_eq!(response.raw.as_ref().unwrap()["id"], json!("resp_123"));
}

#[test]
fn anthropic_native_adapter_complete_uses_messages_transport_and_preserves_cache_and_thinking() {
    let transport = Arc::new(RecordingTransport::new([Ok(NativeCompleteResponse {
        status: 200,
        headers: BTreeMap::from([(
            "anthropic-ratelimit-tokens-remaining".to_string(),
            "88".to_string(),
        )]),
        body: json!({
            "id": "msg_123",
            "type": "message",
            "model": "claude-sonnet-4-5",
            "content": [
                {"type": "thinking", "thinking": "private reasoning", "signature": "sig-123"},
                {"type": "redacted_thinking", "data": "opaque"},
                {"type": "text", "text": "visible answer"}
            ],
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 11,
                "output_tokens": 13,
                "reasoning_tokens": 6,
                "cache_read_input_tokens": 5,
                "cache_creation_input_tokens": 2
            }
        }),
    })]));
    let native_transport: Arc<dyn NativeCompleteTransport> = transport.clone();
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(NativeProviderAdapter::anthropic(
        NativeRequestConfig::new("anthropic-key"),
        native_transport,
    ));
    let client = Client::from_adapters([adapter], Some("anthropic")).unwrap();

    let response = client
        .complete(Request {
            model: "claude-sonnet-4-5".to_string(),
            messages: vec![Message::system("system"), Message::user("hello")],
            tools: vec![tool("lookup")],
            provider_options: BTreeMap::from([
                (
                    "anthropic".to_string(),
                    json!({"beta_headers": ["custom-beta", "custom-beta"]}),
                ),
                ("openai".to_string(), json!({"parallel_tool_calls": false})),
            ]),
            ..Request::default()
        })
        .unwrap();

    let captured = transport.only_request();
    assert_eq!(captured.provider, "anthropic");
    assert_eq!(captured.method, "POST");
    assert_eq!(captured.url, "https://api.anthropic.com/v1/messages");
    assert_eq!(captured.headers["x-api-key"], "anthropic-key");
    assert_eq!(captured.headers["anthropic-version"], "2023-06-01");
    assert_eq!(
        captured.headers["anthropic-beta"],
        "custom-beta,prompt-caching-2024-07-31"
    );
    assert_eq!(captured.body["model"], json!("claude-sonnet-4-5"));
    assert_eq!(
        captured.body["messages"],
        json!([{
            "role": "user",
            "content": [{"type": "text", "text": "hello"}]
        }])
    );
    assert!(captured.body.to_string().contains("cache_control"));
    assert!(!captured.body.to_string().contains("parallel_tool_calls"));

    assert_eq!(response.provider, "anthropic");
    assert_eq!(response.text(), "visible answer");
    assert_eq!(
        response.reasoning().as_deref(),
        Some("private reasoningopaque")
    );
    assert_eq!(response.finish_reason.reason, FinishReasonKind::Stop);
    assert_eq!(response.usage.reasoning_tokens, Some(6));
    assert_eq!(response.usage.cache_read_tokens, Some(5));
    assert_eq!(response.usage.cache_write_tokens, Some(2));
    assert_eq!(response.rate_limit.unwrap().tokens_remaining, Some(88));
    match &response.message.content[0] {
        ContentPart::Thinking { thinking } => {
            assert_eq!(thinking.signature.as_deref(), Some("sig-123"));
            assert_eq!(thinking.source_provider.as_deref(), Some("anthropic"));
        }
        other => panic!("expected thinking block, got {other:?}"),
    }
    match &response.message.content[1] {
        ContentPart::RedactedThinking { thinking } => {
            assert!(thinking.redacted);
            assert_eq!(thinking.source_provider.as_deref(), Some("anthropic"));
        }
        other => panic!("expected redacted thinking block, got {other:?}"),
    }
    assert_eq!(response.raw.as_ref().unwrap()["id"], json!("msg_123"));
}

#[test]
fn gemini_native_adapter_complete_uses_generate_content_transport_and_replays_synthetic_tool_ids() {
    let transport = Arc::new(RecordingTransport::new([Ok(NativeCompleteResponse {
        status: 200,
        headers: BTreeMap::new(),
        body: json!({
            "responseId": "gemini_123",
            "modelVersion": "gemini-3.1-pro-preview",
            "candidates": [{
                "content": {
                    "parts": [
                        {"text": "private thought", "thought": true, "thoughtSignature": "sig-gemini"},
                        {"functionCall": {"name": "lookup", "args": {"city": "Paris"}}},
                        {"text": "visible answer"}
                    ]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 12,
                "candidatesTokenCount": 34,
                "totalTokenCount": 46,
                "thoughtsTokenCount": 5,
                "cachedContentTokenCount": 4
            }
        }),
    })]));
    let config = NativeRequestConfig {
        api_key: Some("gemini key".to_string()),
        base_url: Some("https://gemini.example/root".to_string()),
        ..NativeRequestConfig::default()
    };
    let native_transport: Arc<dyn NativeCompleteTransport> = transport.clone();
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(NativeProviderAdapter::gemini(config, native_transport));
    let client = Client::from_adapters([adapter], Some("gemini")).unwrap();

    let response = client
        .complete(Request {
            model: "models/gemini-3.1-pro-preview".to_string(),
            messages: vec![Message {
                role: unified_llm_adapter::MessageRole::User,
                content: vec![
                    ContentPart::Text {
                        text: "hello".to_string(),
                    },
                    ContentPart::Image {
                        image: ImageData::data(b"image".to_vec(), Some("image/png".to_string())),
                    },
                ],
                ..Message::default()
            }],
            provider_options: BTreeMap::from([
                (
                    "gemini".to_string(),
                    json!({
                        "safetySettings": [{"category": "HARM_CATEGORY_DANGEROUS_CONTENT"}],
                        "thinkingConfig": {"includeThoughts": true},
                        "unsupportedNativeKey": {"must": "drop"}
                    }),
                ),
                (
                    "anthropic".to_string(),
                    json!({"beta_headers": ["must-not-leak"]}),
                ),
            ]),
            ..Request::default()
        })
        .unwrap();

    let captured = transport.only_request();
    assert_eq!(captured.provider, "gemini");
    assert_eq!(captured.method, "POST");
    assert_eq!(
        captured.url,
        "https://gemini.example/root/v1beta/models/gemini-3.1-pro-preview:generateContent?key=gemini+key"
    );
    assert_eq!(
        captured.body["contents"][0]["parts"],
        json!([
            {"text": "hello"},
            {"inlineData": {"data": "aW1hZ2U=", "mimeType": "image/png"}}
        ])
    );
    assert_eq!(
        captured.body["generationConfig"],
        json!({"thinkingConfig": {"includeThoughts": true}})
    );
    assert_eq!(
        captured.body["safetySettings"],
        json!([{"category": "HARM_CATEGORY_DANGEROUS_CONTENT"}])
    );
    assert!(!captured.body.to_string().contains("unsupportedNativeKey"));
    assert!(!captured.body.to_string().contains("must-not-leak"));

    assert_eq!(response.provider, "gemini");
    assert_eq!(response.id, "gemini_123");
    assert_eq!(response.text(), "visible answer");
    assert_eq!(response.reasoning().as_deref(), Some("private thought"));
    assert_eq!(response.finish_reason.reason, FinishReasonKind::ToolCalls);
    assert_eq!(response.usage.reasoning_tokens, Some(5));
    assert_eq!(response.usage.cache_read_tokens, Some(4));
    let tool_call = &response.tool_calls()[0];
    assert_eq!(tool_call.name, "lookup");
    assert!(tool_call.id.starts_with("gemini_call_lookup_c0_p1_"));
    assert_eq!(
        response.raw.as_ref().unwrap()["responseId"],
        json!("gemini_123")
    );
}

#[test]
fn native_adapter_rejects_openai_compatible_as_req022_native_provider() {
    let error = match NativeProviderAdapter::without_transport(
        "openai_compatible",
        NativeRequestConfig::default(),
    ) {
        Ok(_) => panic!("openai_compatible must not be a native provider adapter"),
        Err(error) => error,
    };
    assert_eq!(error.kind, AdapterErrorKind::Configuration);
    assert_eq!(error.provider.as_deref(), Some("openai_compatible"));
    assert!(error.message.contains("Unsupported native provider"));

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
    assert!(error
        .message
        .contains("no Rust provider adapter is registered"));
}

struct RecordingTransport {
    requests: Mutex<Vec<NativeCompleteRequest>>,
    responses: Mutex<VecDeque<Result<NativeCompleteResponse, AdapterError>>>,
}

impl RecordingTransport {
    fn new(
        responses: impl IntoIterator<Item = Result<NativeCompleteResponse, AdapterError>>,
    ) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(responses.into_iter().collect()),
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
