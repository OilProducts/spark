use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use unified_llm_adapter::{
    AdapterError, AdapterErrorKind, Client, ContentPart, FinishReasonKind, ImageData, Message,
    NativeCompleteRequest, NativeCompleteResponse, NativeCompleteTransport, NativeProviderAdapter,
    NativeRequestConfig, NativeStreamResponse, ProviderAdapter, Request, StreamAccumulator,
    StreamEventType,
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
fn openai_native_adapter_stream_uses_responses_stream_and_enriches_finish_event() {
    let stream_chunks = vec![
        json!({
            "type": "response.created",
            "response": {"id": "resp_stream", "model": "gpt-5.2"}
        }),
        json!({"type": "response.output_text.delta", "delta": "Hel"}),
        json!({"type": "response.output_text.delta", "delta": "lo"}),
        json!({
            "type": "response.completed",
            "response": {
                "id": "resp_stream",
                "model": "gpt-5.2",
                "status": "completed",
                "output": [{"type": "output_text", "text": "Hello"}],
                "usage": {
                    "input_tokens": 3,
                    "output_tokens": 2,
                    "output_tokens_details": {"reasoning_tokens": 1},
                    "input_tokens_details": {"cached_tokens": 2}
                }
            }
        }),
    ];
    let transport = Arc::new(RecordingTransport::new_with_streams(
        std::iter::empty(),
        [Ok(NativeStreamResponse::ok(stream_chunks.clone()))],
    ));
    let native_transport: Arc<dyn NativeCompleteTransport> = transport.clone();
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(NativeProviderAdapter::openai(
        NativeRequestConfig::new("openai-key"),
        native_transport,
    ));
    let client = Client::from_adapters([adapter], Some("openai")).unwrap();

    let events = client
        .stream(Request {
            model: "gpt-5.2".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let accumulator = StreamAccumulator::from_events(events.clone());
    let response = accumulator.response;

    let captured = transport.only_request();
    assert_eq!(captured.provider, "openai");
    assert_eq!(captured.method, "POST");
    assert_eq!(captured.url, "https://api.openai.com/v1/responses");
    assert_eq!(captured.headers["Authorization"], "Bearer openai-key");
    assert_eq!(captured.body["stream"], json!(true));
    assert_eq!(
        events
            .iter()
            .map(|event| event.r#type.clone())
            .collect::<Vec<_>>(),
        vec![
            StreamEventType::StreamStart,
            StreamEventType::TextStart,
            StreamEventType::TextDelta,
            StreamEventType::TextDelta,
            StreamEventType::TextEnd,
            StreamEventType::Finish,
        ]
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.r#type == StreamEventType::TextDelta)
            .filter_map(|event| event.delta.as_deref())
            .collect::<String>(),
        "Hello"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event.r#type,
                StreamEventType::TextStart | StreamEventType::TextDelta | StreamEventType::TextEnd
            ))
            .map(|event| event.text_id.as_deref())
            .collect::<Vec<_>>(),
        vec![
            Some("text_0"),
            Some("text_0"),
            Some("text_0"),
            Some("text_0")
        ]
    );
    assert_eq!(
        events.last().unwrap().response.as_ref().unwrap().text(),
        "Hello"
    );
    assert_eq!(response.text(), "Hello");
    assert_eq!(response.finish_reason.reason, FinishReasonKind::Stop);
    assert_eq!(response.usage.total_tokens, 5);
    assert_eq!(response.usage.reasoning_tokens, Some(1));
    assert_eq!(response.usage.cache_read_tokens, Some(2));
    let finish_event = events.last().expect("finish event");
    assert_eq!(finish_event.r#type, StreamEventType::Finish);
    assert_eq!(
        finish_event.usage.as_ref().unwrap().reasoning_tokens,
        Some(1)
    );
    assert_eq!(
        finish_event.usage.as_ref().unwrap().cache_read_tokens,
        Some(2)
    );
    assert_eq!(response.raw, Some(Value::Array(stream_chunks)));
}

#[test]
fn openai_native_adapter_stream_accepts_raw_sse_and_done_termination() {
    let stream_payload = concat!(
        ": keepalive\n",
        "retry: 2500\n",
        "event: ignored.sse.name\n",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_sse\",\"model\":\"gpt-5.2\"}}\n",
        "\n",
        "event: ignored.sse.name\n",
        "data: {\"type\":\"response.output_text.delta\",\"item_id\":\"text_1\",\"delta\":\"Hi\"}\n",
        "\n",
        "data: [DONE]\n",
        "\n"
    );
    let transport = Arc::new(RecordingTransport::new_with_streams(
        std::iter::empty(),
        [Ok(NativeStreamResponse::sse(stream_payload))],
    ));
    let native_transport: Arc<dyn NativeCompleteTransport> = transport.clone();
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(NativeProviderAdapter::openai(
        NativeRequestConfig::new("openai-key"),
        native_transport,
    ));
    let client = Client::from_adapters([adapter], Some("openai")).unwrap();

    let events = client
        .stream(Request {
            model: "gpt-5.2".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let accumulator = StreamAccumulator::from_events(events.clone());
    let response = accumulator.response;

    assert_eq!(
        events
            .iter()
            .map(|event| event.r#type.clone())
            .collect::<Vec<_>>(),
        vec![
            StreamEventType::StreamStart,
            StreamEventType::TextStart,
            StreamEventType::TextDelta,
            StreamEventType::TextEnd,
            StreamEventType::Finish,
        ]
    );
    assert_eq!(response.id, "resp_sse");
    assert_eq!(response.text(), "Hi");
    assert_eq!(
        events
            .iter()
            .filter(|event| event.r#type == StreamEventType::TextDelta)
            .map(|event| event.text_id.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("text_1")]
    );
    assert_eq!(transport.request_count(), 1);
}

#[test]
fn openai_native_adapter_stream_maps_function_call_argument_lifecycle() {
    let stream_chunks = vec![
        json!({
            "type": "response.created",
            "response": {"id": "resp_tools", "model": "gpt-5.2"}
        }),
        json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": {
                "type": "function_call",
                "id": "fc_1",
                "call_id": "call_lookup",
                "name": "lookup",
                "arguments": ""
            }
        }),
        json!({
            "type": "response.function_call_arguments.delta",
            "item_id": "fc_1",
            "output_index": 0,
            "delta": "{\"city\":"
        }),
        json!({
            "type": "response.function_call_arguments.delta",
            "item_id": "fc_1",
            "output_index": 0,
            "delta": "\"Paris\"}"
        }),
        json!({
            "type": "response.output_item.done",
            "output_index": 0,
            "item": {
                "type": "function_call",
                "id": "fc_1",
                "call_id": "call_lookup",
                "name": "lookup",
                "arguments": "{\"city\":\"Paris\"}"
            }
        }),
        json!({
            "type": "response.completed",
            "response": {
                "id": "resp_tools",
                "model": "gpt-5.2",
                "status": "completed",
                "output": [{
                    "type": "function_call",
                    "id": "fc_1",
                    "call_id": "call_lookup",
                    "name": "lookup",
                    "arguments": "{\"city\":\"Paris\"}"
                }],
                "usage": {"input_tokens": 5, "output_tokens": 4}
            }
        }),
    ];
    let transport = Arc::new(RecordingTransport::new_with_streams(
        std::iter::empty(),
        [Ok(NativeStreamResponse::ok(stream_chunks))],
    ));
    let native_transport: Arc<dyn NativeCompleteTransport> = transport.clone();
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(NativeProviderAdapter::openai(
        NativeRequestConfig::new("openai-key"),
        native_transport,
    ));
    let client = Client::from_adapters([adapter], Some("openai")).unwrap();

    let events = client
        .stream(Request {
            model: "gpt-5.2".to_string(),
            messages: vec![Message::user("lookup Paris")],
            tools: vec![tool("lookup")],
            ..Request::default()
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let response = StreamAccumulator::from_events(events.clone()).response;

    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event.r#type,
                StreamEventType::ToolCallStart
                    | StreamEventType::ToolCallDelta
                    | StreamEventType::ToolCallEnd
            ))
            .map(|event| event.r#type.clone())
            .collect::<Vec<_>>(),
        vec![
            StreamEventType::ToolCallStart,
            StreamEventType::ToolCallDelta,
            StreamEventType::ToolCallDelta,
            StreamEventType::ToolCallEnd,
        ]
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event.r#type,
                StreamEventType::ToolCallStart
                    | StreamEventType::ToolCallDelta
                    | StreamEventType::ToolCallEnd
            ))
            .map(|event| event.tool_call.as_ref().unwrap().id.as_str())
            .collect::<Vec<_>>(),
        vec!["call_lookup", "call_lookup", "call_lookup", "call_lookup"]
    );
    assert_eq!(response.finish_reason.reason, FinishReasonKind::ToolCalls);
    let tool_calls = response.tool_calls();
    assert_eq!(tool_calls.len(), 1);
    let tool_call = &tool_calls[0];
    assert_eq!(tool_call.id, "call_lookup");
    assert_eq!(tool_call.name, "lookup");
    assert_eq!(tool_call.arguments, json!({"city": "Paris"}));
    assert_eq!(
        tool_call.raw_arguments.as_deref(),
        Some("{\"city\":\"Paris\"}")
    );
    assert_eq!(transport.request_count(), 1);
}

#[test]
fn openai_native_adapter_stream_emits_provider_event_for_unrecognized_output_item_done() {
    let passthrough_chunk = json!({
        "type": "response.output_item.done",
        "item": {
            "type": "web_search_call",
            "id": "search_1",
            "status": "completed",
            "results": [{"title": "opaque provider result"}]
        }
    });
    let stream_chunks = vec![
        json!({
            "type": "response.created",
            "response": {"id": "resp_passthrough", "model": "gpt-5.2"}
        }),
        passthrough_chunk.clone(),
        json!({
            "type": "response.completed",
            "response": {
                "id": "resp_passthrough",
                "model": "gpt-5.2",
                "status": "completed",
                "output": [],
                "usage": {"input_tokens": 3, "output_tokens": 1}
            }
        }),
    ];
    let transport = Arc::new(RecordingTransport::new_with_streams(
        std::iter::empty(),
        [Ok(NativeStreamResponse::ok(stream_chunks.clone()))],
    ));
    let native_transport: Arc<dyn NativeCompleteTransport> = transport.clone();
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(NativeProviderAdapter::openai(
        NativeRequestConfig::new("openai-key"),
        native_transport,
    ));
    let client = Client::from_adapters([adapter], Some("openai")).unwrap();

    let events = client
        .stream(Request {
            model: "gpt-5.2".to_string(),
            messages: vec![Message::user("search")],
            ..Request::default()
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let response = StreamAccumulator::from_events(events.clone()).response;

    assert_eq!(
        events
            .iter()
            .map(|event| event.r#type.clone())
            .collect::<Vec<_>>(),
        vec![
            StreamEventType::StreamStart,
            StreamEventType::ProviderEvent,
            StreamEventType::Finish,
        ]
    );
    let provider_event = events
        .iter()
        .find(|event| event.r#type == StreamEventType::ProviderEvent)
        .expect("provider event");
    assert_eq!(provider_event.raw.as_ref(), Some(&passthrough_chunk));
    assert_eq!(response.raw, Some(Value::Array(stream_chunks)));
    assert_eq!(transport.request_count(), 1);
}

#[test]
fn native_sse_stream_malformed_payload_surfaces_stream_error_with_raw_data() {
    let transport = Arc::new(RecordingTransport::new_with_streams(
        std::iter::empty(),
        [Ok(NativeStreamResponse::sse(
            "event: response.output_text.delta\ndata: {not-json}\n\n",
        ))],
    ));
    let native_transport: Arc<dyn NativeCompleteTransport> = transport.clone();
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(NativeProviderAdapter::openai(
        NativeRequestConfig::new("openai-key"),
        native_transport,
    ));
    let client = Client::from_adapters([adapter], Some("openai")).unwrap();

    let mut stream = client
        .stream(Request {
            model: "gpt-5.2".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap();
    let error = stream.next().expect("stream error").unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::Stream);
    assert!(error.message.contains("Malformed provider stream payload"));
    assert_eq!(error.raw.as_ref().unwrap()["data"], json!("{not-json}"));
    assert_eq!(
        error.raw.as_ref().unwrap()["sse_event"],
        json!("response.output_text.delta")
    );
    assert!(stream.next().is_none());
    assert_eq!(transport.request_count(), 1);
}

#[test]
fn native_stream_unknown_provider_payloads_emit_provider_event_with_raw_payload() {
    let provider_chunk = json!({
        "vendorEvent": "cache_hit",
        "payload": {"opaque": true}
    });
    let transport = Arc::new(RecordingTransport::new_with_streams(
        std::iter::empty(),
        [Ok(NativeStreamResponse::ok([provider_chunk.clone()]))],
    ));
    let native_transport: Arc<dyn NativeCompleteTransport> = transport.clone();
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(NativeProviderAdapter::gemini(
        NativeRequestConfig::new("gemini-key"),
        native_transport,
    ));
    let client = Client::from_adapters([adapter], Some("gemini")).unwrap();

    let events = client
        .stream(Request {
            model: "gemini-3.1-pro-preview".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let response = StreamAccumulator::from_events(events.clone()).response;

    assert_eq!(
        events
            .iter()
            .map(|event| event.r#type.clone())
            .collect::<Vec<_>>(),
        vec![
            StreamEventType::StreamStart,
            StreamEventType::ProviderEvent,
            StreamEventType::Finish,
        ]
    );
    let provider_event = events
        .iter()
        .find(|event| event.r#type == StreamEventType::ProviderEvent)
        .expect("provider event");
    assert_eq!(provider_event.raw.as_ref(), Some(&provider_chunk));
    assert_eq!(response.raw, Some(provider_chunk));
    assert_eq!(transport.request_count(), 1);
}

#[test]
fn gemini_native_adapter_stream_uses_stream_endpoint_and_stateless_tool_call_ids() {
    let first_chunk = json!({
        "responseId": "gemini_stream",
        "modelVersion": "gemini-3.1-pro-preview",
        "candidates": [{
            "content": {"parts": [{"text": "Hello"}]}
        }]
    });
    let second_chunk = json!({
        "responseId": "gemini_stream",
        "modelVersion": "gemini-3.1-pro-preview",
        "candidates": [{
            "content": {
                "parts": [
                    {"text": "private thought", "thought": true, "thoughtSignature": "sig-gemini-stream"},
                    {"functionCall": {"name": "lookup", "args": {"city": "Paris"}}}
                ]
            },
            "finishReason": "STOP"
        }],
        "usageMetadata": {
            "promptTokenCount": 4,
            "candidatesTokenCount": 3,
            "totalTokenCount": 7,
            "thoughtsTokenCount": 2,
            "cachedContentTokenCount": 1
        }
    });
    let stream_chunks = vec![first_chunk.clone(), second_chunk.clone()];
    let transport = Arc::new(RecordingTransport::new_with_streams(
        std::iter::empty(),
        [Ok(NativeStreamResponse::ok(stream_chunks.clone()))],
    ));
    let native_transport: Arc<dyn NativeCompleteTransport> = transport.clone();
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(NativeProviderAdapter::gemini(
        NativeRequestConfig::new("gemini-key"),
        native_transport,
    ));
    let client = Client::from_adapters([adapter], Some("gemini")).unwrap();

    let events = client
        .stream(Request {
            model: "gemini-3.1-pro-preview".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let accumulator = StreamAccumulator::from_events(events.clone());
    let response = accumulator.response;

    let captured = transport.only_request();
    assert_eq!(captured.provider, "gemini");
    assert_eq!(captured.method, "POST");
    assert_eq!(
        captured.url,
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.1-pro-preview:streamGenerateContent?alt=sse&key=gemini-key"
    );
    assert_eq!(
        captured.body["contents"],
        json!([{"role": "user", "parts": [{"text": "hello"}]}])
    );
    assert_eq!(
        events
            .iter()
            .map(|event| event.r#type.clone())
            .collect::<Vec<_>>(),
        vec![
            StreamEventType::StreamStart,
            StreamEventType::TextStart,
            StreamEventType::TextDelta,
            StreamEventType::TextEnd,
            StreamEventType::ReasoningStart,
            StreamEventType::ReasoningDelta,
            StreamEventType::ReasoningEnd,
            StreamEventType::ToolCallStart,
            StreamEventType::ToolCallEnd,
            StreamEventType::Finish,
        ]
    );
    assert_eq!(response.text(), "Hello");
    assert_eq!(response.reasoning().as_deref(), Some("private thought"));
    assert_eq!(response.finish_reason.reason, FinishReasonKind::ToolCalls);
    assert_eq!(response.usage.total_tokens, 7);
    assert_eq!(response.usage.reasoning_tokens, Some(2));
    assert_eq!(response.usage.cache_read_tokens, Some(1));
    assert_eq!(response.raw, Some(Value::Array(stream_chunks)));
    match response
        .message
        .content
        .iter()
        .find(|part| matches!(part, ContentPart::Thinking { .. }))
        .expect("gemini thought part")
    {
        ContentPart::Thinking { thinking } => {
            assert_eq!(thinking.signature.as_deref(), Some("sig-gemini-stream"));
            assert_eq!(thinking.source_provider.as_deref(), Some("gemini"));
        }
        other => panic!("expected gemini thinking block, got {other:?}"),
    }
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event.r#type,
                StreamEventType::TextStart | StreamEventType::TextDelta | StreamEventType::TextEnd
            ))
            .map(|event| event.text_id.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("text_0"), Some("text_0"), Some("text_0")]
    );
    let tool_call = &response.tool_calls()[0];
    assert_eq!(tool_call.name, "lookup");
    assert!(tool_call.id.starts_with("gemini_call_lookup_c0_p1_"));
    let tool_events = events
        .iter()
        .filter(|event| {
            matches!(
                event.r#type,
                StreamEventType::ToolCallStart | StreamEventType::ToolCallEnd
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(tool_events[0].tool_call.as_ref().unwrap().id, tool_call.id);
    assert_eq!(tool_events[1].tool_call.as_ref().unwrap().id, tool_call.id);
}

#[test]
fn gemini_native_adapter_stream_accepts_json_array_payload_chunks() {
    let first_chunk = json!({
        "responseId": "gemini_json_array_stream",
        "modelVersion": "gemini-3.1-pro-preview",
        "candidates": [{
            "content": {"parts": [{"text": "Hello"}]}
        }]
    });
    let second_chunk = json!({
        "responseId": "gemini_json_array_stream",
        "modelVersion": "gemini-3.1-pro-preview",
        "candidates": [{
            "content": {"parts": [{"text": "Hello world"}]},
            "finishReason": "STOP"
        }],
        "usageMetadata": {
            "promptTokenCount": 2,
            "candidatesTokenCount": 3,
            "totalTokenCount": 5,
            "thoughtsTokenCount": 1,
            "cachedContentTokenCount": 1
        }
    });
    let stream_chunks = vec![first_chunk.clone(), second_chunk.clone()];
    let payload = serde_json::to_string(&Value::Array(stream_chunks.clone())).unwrap();
    let transport = Arc::new(RecordingTransport::new_with_streams(
        std::iter::empty(),
        [Ok(NativeStreamResponse {
            status: 200,
            headers: BTreeMap::from([("content-type".to_string(), "application/json".to_string())]),
            body: vec![Ok(Value::String(payload))],
        })],
    ));
    let native_transport: Arc<dyn NativeCompleteTransport> = transport.clone();
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(NativeProviderAdapter::gemini(
        NativeRequestConfig::new("gemini-key"),
        native_transport,
    ));
    let client = Client::from_adapters([adapter], Some("gemini")).unwrap();

    let events = client
        .stream(Request {
            model: "gemini-3.1-pro-preview".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let response = StreamAccumulator::from_events(events.clone()).response;

    assert_eq!(
        events
            .iter()
            .map(|event| event.r#type.clone())
            .collect::<Vec<_>>(),
        vec![
            StreamEventType::StreamStart,
            StreamEventType::TextStart,
            StreamEventType::TextDelta,
            StreamEventType::TextDelta,
            StreamEventType::TextEnd,
            StreamEventType::Finish,
        ]
    );
    assert_eq!(response.text(), "Hello world");
    assert_eq!(response.finish_reason.reason, FinishReasonKind::Stop);
    assert_eq!(response.usage.total_tokens, 5);
    assert_eq!(response.usage.reasoning_tokens, Some(1));
    assert_eq!(response.usage.cache_read_tokens, Some(1));
    assert_eq!(response.raw, Some(Value::Array(stream_chunks)));
    assert_eq!(transport.request_count(), 1);
}

#[test]
fn anthropic_native_adapter_stream_correlates_interleaved_blocks_by_ids() {
    let stream_chunks = vec![
        json!({
            "type": "message_start",
            "message": {
                "id": "msg_stream",
                "model": "claude-sonnet-4-5",
                "role": "assistant",
                "content": [],
                "stop_reason": null,
                "usage": {"input_tokens": 4, "output_tokens": 0}
            }
        }),
        json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {"type": "text"}
        }),
        json!({
            "type": "content_block_start",
            "index": 1,
            "content_block": {
                "type": "tool_use",
                "id": "call_weather",
                "name": "weather",
                "input": {}
            }
        }),
        json!({
            "type": "content_block_start",
            "index": 2,
            "content_block": {
                "type": "tool_use",
                "id": "call_search",
                "name": "search",
                "input": {}
            }
        }),
        json!({
            "type": "content_block_delta",
            "index": 1,
            "delta": {"type": "input_json_delta", "partial_json": "{\"city\":"}
        }),
        json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "text_delta", "text": "Hello "}
        }),
        json!({
            "type": "content_block_delta",
            "index": 2,
            "delta": {"type": "input_json_delta", "partial_json": "{\"query\":"}
        }),
        json!({
            "type": "content_block_delta",
            "index": 1,
            "delta": {"type": "input_json_delta", "partial_json": "\"Paris\"}"}
        }),
        json!({"type": "content_block_stop", "index": 1}),
        json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "text_delta", "text": "world"}
        }),
        json!({"type": "content_block_stop", "index": 0}),
        json!({
            "type": "content_block_delta",
            "index": 2,
            "delta": {"type": "input_json_delta", "partial_json": "\"rust\"}"}
        }),
        json!({"type": "content_block_stop", "index": 2}),
        json!({
            "type": "message_delta",
            "delta": {"stop_reason": "tool_use"},
            "usage": {"output_tokens": 7}
        }),
        json!({"type": "message_stop"}),
    ];
    let transport = Arc::new(RecordingTransport::new_with_streams(
        std::iter::empty(),
        [Ok(NativeStreamResponse::ok(stream_chunks.clone()))],
    ));
    let native_transport: Arc<dyn NativeCompleteTransport> = transport.clone();
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(NativeProviderAdapter::anthropic(
        NativeRequestConfig::new("anthropic-key"),
        native_transport,
    ));
    let client = Client::from_adapters([adapter], Some("anthropic")).unwrap();

    let events = client
        .stream(Request {
            model: "claude-sonnet-4-5".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let accumulator = StreamAccumulator::from_events(events.clone());
    let response = accumulator.response;

    assert_eq!(response.text(), "Hello world");
    assert_eq!(response.finish_reason.reason, FinishReasonKind::ToolCalls);
    assert_eq!(response.usage.input_tokens, 4);
    assert_eq!(response.usage.output_tokens, 7);
    assert_eq!(response.usage.total_tokens, 11);
    let finish_event = events.last().expect("finish event");
    assert_eq!(finish_event.r#type, StreamEventType::Finish);
    assert_eq!(finish_event.usage.as_ref().unwrap().input_tokens, 4);
    assert_eq!(finish_event.usage.as_ref().unwrap().output_tokens, 7);
    assert_eq!(finish_event.usage.as_ref().unwrap().total_tokens, 11);
    assert_eq!(
        finish_event.response.as_ref().unwrap().usage,
        response.usage
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event.r#type,
                StreamEventType::TextStart | StreamEventType::TextDelta | StreamEventType::TextEnd
            ))
            .map(|event| event.text_id.as_deref())
            .collect::<Vec<_>>(),
        vec![
            Some("text_0"),
            Some("text_0"),
            Some("text_0"),
            Some("text_0")
        ]
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event.r#type,
                StreamEventType::ToolCallStart
                    | StreamEventType::ToolCallDelta
                    | StreamEventType::ToolCallEnd
            ))
            .map(|event| event.tool_call.as_ref().unwrap().id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "call_weather",
            "call_search",
            "call_weather",
            "call_search",
            "call_weather",
            "call_weather",
            "call_search",
            "call_search"
        ]
    );
    let tool_calls = response.tool_calls();
    assert_eq!(tool_calls.len(), 2);
    assert_eq!(tool_calls[0].id, "call_weather");
    assert_eq!(
        tool_calls[0].raw_arguments.as_deref(),
        Some("{\"city\":\"Paris\"}")
    );
    assert_eq!(tool_calls[1].id, "call_search");
    assert_eq!(
        tool_calls[1].raw_arguments.as_deref(),
        Some("{\"query\":\"rust\"}")
    );
}

#[test]
fn anthropic_native_adapter_stream_preserves_thinking_and_redacted_thinking_blocks() {
    let stream_chunks = vec![
        json!({
            "type": "message_start",
            "message": {
                "id": "msg_thinking_stream",
                "model": "claude-sonnet-4-5",
                "role": "assistant",
                "content": [],
                "stop_reason": null,
                "usage": {"input_tokens": 6, "output_tokens": 0}
            }
        }),
        json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {"type": "thinking", "thinking": "plan "}
        }),
        json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "thinking_delta", "thinking": "ahead"}
        }),
        json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "signature_delta", "signature": "sig-stream"}
        }),
        json!({"type": "content_block_stop", "index": 0}),
        json!({
            "type": "content_block_start",
            "index": 1,
            "content_block": {"type": "redacted_thinking", "data": "opaque"}
        }),
        json!({"type": "content_block_stop", "index": 1}),
        json!({
            "type": "content_block_start",
            "index": 2,
            "content_block": {"type": "text"}
        }),
        json!({
            "type": "content_block_delta",
            "index": 2,
            "delta": {"type": "text_delta", "text": "visible"}
        }),
        json!({"type": "content_block_stop", "index": 2}),
        json!({
            "type": "message_delta",
            "delta": {"stop_reason": "end_turn"},
            "usage": {
                "output_tokens": 5,
                "reasoning_tokens": 3,
                "cache_read_input_tokens": 2,
                "cache_creation_input_tokens": 1
            }
        }),
        json!({"type": "message_stop"}),
    ];
    let transport = Arc::new(RecordingTransport::new_with_streams(
        std::iter::empty(),
        [Ok(NativeStreamResponse::ok(stream_chunks))],
    ));
    let native_transport: Arc<dyn NativeCompleteTransport> = transport.clone();
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(NativeProviderAdapter::anthropic(
        NativeRequestConfig::new("anthropic-key"),
        native_transport,
    ));
    let client = Client::from_adapters([adapter], Some("anthropic")).unwrap();

    let events = client
        .stream(Request {
            model: "claude-sonnet-4-5".to_string(),
            messages: vec![Message::user("think")],
            ..Request::default()
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let response = StreamAccumulator::from_events(events.clone()).response;

    assert_eq!(response.text(), "visible");
    assert_eq!(response.reasoning().as_deref(), Some("plan aheadopaque"));
    assert_eq!(response.finish_reason.reason, FinishReasonKind::Stop);
    assert_eq!(response.usage.input_tokens, 6);
    assert_eq!(response.usage.output_tokens, 5);
    assert_eq!(response.usage.reasoning_tokens, Some(3));
    assert_eq!(response.usage.cache_read_tokens, Some(2));
    assert_eq!(response.usage.cache_write_tokens, Some(1));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event.r#type,
                StreamEventType::ReasoningStart
                    | StreamEventType::ReasoningDelta
                    | StreamEventType::ReasoningEnd
            ))
            .map(|event| event.r#type.clone())
            .collect::<Vec<_>>(),
        vec![
            StreamEventType::ReasoningStart,
            StreamEventType::ReasoningDelta,
            StreamEventType::ReasoningDelta,
            StreamEventType::ReasoningDelta,
            StreamEventType::ReasoningEnd,
            StreamEventType::ReasoningStart,
            StreamEventType::ReasoningDelta,
            StreamEventType::ReasoningEnd,
        ]
    );
    match &response.message.content[0] {
        ContentPart::Text { text } => assert_eq!(text, "visible"),
        other => panic!("expected text block, got {other:?}"),
    }
    match &response.message.content[1] {
        ContentPart::Thinking { thinking } => {
            assert_eq!(thinking.text, "plan ahead");
            assert_eq!(thinking.signature.as_deref(), Some("sig-stream"));
            assert_eq!(thinking.source_provider.as_deref(), Some("anthropic"));
            assert_eq!(thinking.source_model.as_deref(), Some("claude-sonnet-4-5"));
        }
        other => panic!("expected thinking block, got {other:?}"),
    }
    match &response.message.content[2] {
        ContentPart::RedactedThinking { thinking } => {
            assert_eq!(thinking.text, "opaque");
            assert!(thinking.redacted);
            assert_eq!(thinking.source_provider.as_deref(), Some("anthropic"));
        }
        other => panic!("expected redacted thinking block, got {other:?}"),
    }
    assert_eq!(transport.request_count(), 1);
}

#[test]
fn anthropic_native_adapter_stream_estimates_reasoning_tokens_when_usage_omits_them() {
    let stream_chunks = vec![
        json!({
            "type": "message_start",
            "message": {
                "id": "msg_estimated_thinking_stream",
                "model": "claude-sonnet-4-5",
                "role": "assistant",
                "content": [],
                "stop_reason": null,
                "usage": {"input_tokens": 6, "output_tokens": 0}
            }
        }),
        json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {"type": "thinking", "thinking": "plan "}
        }),
        json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "thinking_delta", "thinking": "ahead"}
        }),
        json!({"type": "content_block_stop", "index": 0}),
        json!({
            "type": "content_block_start",
            "index": 1,
            "content_block": {"type": "redacted_thinking", "data": "opaque"}
        }),
        json!({"type": "content_block_stop", "index": 1}),
        json!({
            "type": "content_block_start",
            "index": 2,
            "content_block": {"type": "text"}
        }),
        json!({
            "type": "content_block_delta",
            "index": 2,
            "delta": {"type": "text_delta", "text": "visible"}
        }),
        json!({"type": "content_block_stop", "index": 2}),
        json!({
            "type": "message_delta",
            "delta": {"stop_reason": "end_turn"},
            "usage": {
                "output_tokens": 5,
                "cache_read_input_tokens": 2,
                "cache_creation_input_tokens": 1
            }
        }),
        json!({"type": "message_stop"}),
    ];
    let transport = Arc::new(RecordingTransport::new_with_streams(
        std::iter::empty(),
        [Ok(NativeStreamResponse::ok(stream_chunks))],
    ));
    let native_transport: Arc<dyn NativeCompleteTransport> = transport.clone();
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(NativeProviderAdapter::anthropic(
        NativeRequestConfig::new("anthropic-key"),
        native_transport,
    ));
    let client = Client::from_adapters([adapter], Some("anthropic")).unwrap();

    let events = client
        .stream(Request {
            model: "claude-sonnet-4-5".to_string(),
            messages: vec![Message::user("think")],
            ..Request::default()
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let response = StreamAccumulator::from_events(events.clone()).response;
    let finish_event = events.last().expect("finish event");

    assert_eq!(response.reasoning().as_deref(), Some("plan aheadopaque"));
    assert_eq!(response.usage.reasoning_tokens, Some(5));
    assert_eq!(response.usage.cache_read_tokens, Some(2));
    assert_eq!(response.usage.cache_write_tokens, Some(1));
    assert_eq!(
        finish_event
            .response
            .as_ref()
            .unwrap()
            .usage
            .reasoning_tokens,
        Some(5)
    );
    assert_eq!(
        finish_event.usage.as_ref().unwrap().cache_read_tokens,
        Some(2)
    );
    assert_eq!(
        finish_event.usage.as_ref().unwrap().cache_write_tokens,
        Some(1)
    );
    assert_eq!(transport.request_count(), 1);
}

#[test]
fn native_stream_errors_after_partial_events_surface_without_transparent_retry() {
    let stream_error = AdapterError::provider(
        AdapterErrorKind::Stream,
        "after partial",
        Some("openai".to_string()),
    );
    let transport = Arc::new(RecordingTransport::new_with_streams(
        std::iter::empty(),
        [Ok(NativeStreamResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: vec![
                Ok(json!({"type": "response.output_text.delta", "delta": "partial"})),
                Err(stream_error.clone()),
            ],
        })],
    ));
    let native_transport: Arc<dyn NativeCompleteTransport> = transport.clone();
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(NativeProviderAdapter::openai(
        NativeRequestConfig::new("openai-key"),
        native_transport,
    ));
    let client = Client::from_adapters([adapter], Some("openai")).unwrap();

    let mut stream = client
        .stream(Request {
            model: "gpt-5.2".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap();
    assert_eq!(
        stream.next().expect("stream start").unwrap().r#type,
        StreamEventType::StreamStart
    );
    assert_eq!(
        stream.next().expect("text start").unwrap().r#type,
        StreamEventType::TextStart
    );
    assert_eq!(
        stream.next().expect("text delta").unwrap().delta.as_deref(),
        Some("partial")
    );
    let error = stream.next().expect("stream error").unwrap_err();
    assert_eq!(error, stream_error);
    assert!(stream.next().is_none());
    assert_eq!(transport.request_count(), 1);
}

#[test]
fn native_adapter_classifies_gemini_grpc_errors_and_preserves_retry_metadata() {
    let raw = json!({
        "error": {
            "message": "slow down",
            "status": "RESOURCE_EXHAUSTED",
            "code": 429
        }
    });
    let transport = Arc::new(RecordingTransport::new([Ok(NativeCompleteResponse {
        status: 400,
        headers: BTreeMap::from([("Retry-After".to_string(), "7".to_string())]),
        body: raw.clone(),
    })]));
    let native_transport: Arc<dyn NativeCompleteTransport> = transport;
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(NativeProviderAdapter::gemini(
        NativeRequestConfig::new("gemini-key"),
        native_transport,
    ));
    let client = Client::from_adapters([adapter], Some("gemini")).unwrap();

    let error = client
        .complete(Request {
            model: "gemini-3.1-pro-preview".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::RateLimit);
    assert_eq!(error.message, "slow down");
    assert_eq!(error.provider.as_deref(), Some("gemini"));
    assert_eq!(error.status_code, Some(400));
    assert_eq!(error.error_code.as_deref(), Some("RESOURCE_EXHAUSTED"));
    assert!(error.retryable);
    assert_eq!(error.retry_after, Some(7.0));
    assert_eq!(error.raw, Some(raw));
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

    fn request_count(&self) -> usize {
        self.requests
            .lock()
            .expect("recorded native requests")
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
