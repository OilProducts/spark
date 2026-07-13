use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::{json, Value};
use unified_llm_adapter::{
    AdapterErrorKind, AdapterTimeout, Client, Message, NativeCompleteRequest,
    NativeCompleteTransport, NativeHttpTransport, NativeProviderAdapter, NativeRequestConfig,
    OpenAICompatibleAdapter, OpenAICompatibleRequestConfig, ProviderAdapter, Request,
    StreamAccumulator, StreamEvent, StreamEventType,
};

#[test]
fn http_transport_executes_prepared_complete_request_and_returns_json_with_headers() {
    let server = TestServer::new(TestHttpResponse {
        status: 200,
        headers: BTreeMap::from([
            ("Content-Type".to_string(), "application/json".to_string()),
            ("X-Response-ID".to_string(), "resp-header".to_string()),
        ]),
        body: br#"{"id":"resp_123","ok":true}"#.to_vec(),
        delay_before_response: None,
    });
    let transport = NativeHttpTransport::new();

    let response = transport
        .complete(NativeCompleteRequest {
            provider: "openai".to_string(),
            method: "POST".to_string(),
            url: server.url("/v1/responses?trace=1"),
            headers: BTreeMap::from([
                (
                    "Authorization".to_string(),
                    "Bearer request-key".to_string(),
                ),
                ("X-Trace".to_string(), "trace-1".to_string()),
            ]),
            timeout: AdapterTimeout::new(1.0, 2.0, 3.0),
            abort_signal: None,
            body: json!({
                "model": "gpt-5.2",
                "input": [{"role": "user", "content": "hello"}],
            }),
        })
        .unwrap();

    assert_eq!(response.status, 200);
    assert_eq!(
        header_value(&response.headers, "x-response-id"),
        Some("resp-header")
    );
    assert_eq!(response.body["id"], json!("resp_123"));
    assert_eq!(response.body["ok"], json!(true));

    let captured = server.captured();
    assert_eq!(captured.method, "POST");
    assert_eq!(captured.path, "/v1/responses?trace=1");
    assert_eq!(
        header_value(&captured.headers, "authorization"),
        Some("Bearer request-key")
    );
    assert_eq!(header_value(&captured.headers, "x-trace"), Some("trace-1"));
    assert_eq!(
        serde_json::from_str::<Value>(&captured.body).unwrap(),
        json!({
            "model": "gpt-5.2",
            "input": [{"role": "user", "content": "hello"}],
        })
    );
}

#[test]
fn http_transport_errors_flow_through_provider_status_translation_with_raw_body_and_headers() {
    let server = TestServer::new(TestHttpResponse {
        status: 429,
        headers: BTreeMap::from([
            ("Retry-After".to_string(), "4.5".to_string()),
            (
                "X-RateLimit-Remaining-Requests".to_string(),
                "0".to_string(),
            ),
        ]),
        body: b"rate limited plain text".to_vec(),
        delay_before_response: None,
    });
    let adapter: std::sync::Arc<dyn ProviderAdapter> =
        std::sync::Arc::new(OpenAICompatibleAdapter::openai_compatible(
            OpenAICompatibleRequestConfig {
                api_key: Some("compatible-key".to_string()),
                base_url: Some(server.base_url.clone()),
                require_api_key: true,
                timeout: AdapterTimeout::new(1.0, 2.0, 3.0),
                ..OpenAICompatibleRequestConfig::default()
            },
            std::sync::Arc::new(NativeHttpTransport::new()),
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
    assert_eq!(error.status_code, Some(429));
    assert_eq!(error.retry_after, Some(4.5));
    assert_eq!(error.message, "rate limited plain text");
    assert_eq!(
        error.raw,
        Some(Value::String("rate limited plain text".to_string()))
    );

    let captured = server.captured();
    assert_eq!(captured.path, "/v1/chat/completions");
    assert_eq!(
        header_value(&captured.headers, "authorization"),
        Some("Bearer compatible-key")
    );
}

#[test]
fn http_transport_malformed_success_preserves_raw_body_and_response_headers() {
    let server = TestServer::new(TestHttpResponse {
        status: 200,
        headers: BTreeMap::from([
            ("Retry-After".to_string(), "7".to_string()),
            ("X-Trace".to_string(), "malformed-response".to_string()),
        ]),
        body: b"this is not json".to_vec(),
        delay_before_response: None,
    });
    let transport = NativeHttpTransport::new();

    let error = transport
        .complete(NativeCompleteRequest {
            provider: "openai".to_string(),
            method: "POST".to_string(),
            url: server.url("/v1/responses"),
            headers: BTreeMap::new(),
            timeout: AdapterTimeout::new(1.0, 2.0, 3.0),
            abort_signal: None,
            body: json!({"model": "gpt-5.2"}),
        })
        .unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::Provider);
    assert_eq!(error.provider.as_deref(), Some("openai"));
    assert_eq!(error.status_code, Some(200));
    assert_eq!(error.retry_after, Some(7.0));
    let raw = error.raw.as_ref().expect("raw malformed response");
    assert_eq!(raw["status"], json!(200));
    assert_eq!(raw["body"], json!("this is not json"));
    assert_eq!(raw["headers"]["x-trace"], json!("malformed-response"));

    let captured = server.captured();
    assert_eq!(captured.path, "/v1/responses");
}

#[test]
fn env_configured_client_uses_http_transport_without_python_fallback() {
    let server = TestServer::new(TestHttpResponse {
        status: 200,
        headers: BTreeMap::from([(
            "X-RateLimit-Remaining-Requests".to_string(),
            "11".to_string(),
        )]),
        body: br#"{
            "id": "resp_env",
            "model": "gpt-5.2",
            "status": "completed",
            "output": [{"type": "output_text", "text": "env transport ok"}]
        }"#
        .to_vec(),
        delay_before_response: None,
    });
    let client = Client::from_env_map(
        &BTreeMap::from([
            ("OPENAI_API_KEY".to_string(), "env-key".to_string()),
            ("OPENAI_BASE_URL".to_string(), server.base_url.clone()),
        ]),
        None,
    )
    .unwrap();

    let response = client
        .complete(Request {
            model: "gpt-5.2".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap();

    assert_eq!(response.provider, "openai");
    assert_eq!(response.id, "resp_env");
    assert_eq!(response.text(), "env transport ok");
    assert_eq!(response.rate_limit.unwrap().requests_remaining, Some(11));

    let captured = server.captured();
    assert_eq!(captured.path, "/v1/responses");
    assert_eq!(
        header_value(&captured.headers, "authorization"),
        Some("Bearer env-key")
    );
}

#[test]
fn env_configured_complete_calls_use_provider_native_and_compatible_http_endpoints() {
    assert_env_complete_uses_http_endpoint(
        "openai",
        "gpt-5.2",
        |base_url| {
            BTreeMap::from([
                ("OPENAI_API_KEY".to_string(), "openai-key".to_string()),
                ("OPENAI_BASE_URL".to_string(), base_url.to_string()),
            ])
        },
        json!({
            "id": "resp_http",
            "model": "gpt-5.2",
            "status": "completed",
            "output": [{"type": "output_text", "text": "openai transport ok"}],
        }),
        "/v1/responses",
        Some(("authorization", "Bearer openai-key")),
        "openai transport ok",
    );

    assert_env_complete_uses_http_endpoint(
        "anthropic",
        "claude-sonnet-4-5",
        |base_url| {
            BTreeMap::from([
                ("ANTHROPIC_API_KEY".to_string(), "anthropic-key".to_string()),
                ("ANTHROPIC_BASE_URL".to_string(), base_url.to_string()),
            ])
        },
        json!({
            "id": "msg_http",
            "type": "message",
            "model": "claude-sonnet-4-5",
            "content": [{"type": "text", "text": "anthropic transport ok"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 1, "output_tokens": 2},
        }),
        "/v1/messages",
        Some(("x-api-key", "anthropic-key")),
        "anthropic transport ok",
    );

    assert_env_complete_uses_http_endpoint(
        "gemini",
        "gemini-3.1-pro-preview",
        |base_url| {
            BTreeMap::from([
                ("GEMINI_API_KEY".to_string(), "gemini-key".to_string()),
                ("GEMINI_BASE_URL".to_string(), base_url.to_string()),
            ])
        },
        json!({
            "responseId": "gemini_http",
            "modelVersion": "gemini-3.1-pro-preview",
            "candidates": [{
                "content": {"parts": [{"text": "gemini transport ok"}]},
                "finishReason": "STOP",
            }],
            "usageMetadata": {
                "promptTokenCount": 1,
                "candidatesTokenCount": 2,
                "totalTokenCount": 3,
            },
        }),
        "/v1beta/models/gemini-3.1-pro-preview:generateContent?key=gemini-key",
        None,
        "gemini transport ok",
    );

    assert_env_complete_uses_http_endpoint(
        "openrouter",
        "anthropic/claude-sonnet-4.5",
        |base_url| {
            BTreeMap::from([
                (
                    "OPENROUTER_API_KEY".to_string(),
                    "openrouter-key".to_string(),
                ),
                ("OPENROUTER_BASE_URL".to_string(), base_url.to_string()),
            ])
        },
        chat_completions_response("chat_http", "openrouter transport ok"),
        "/v1/chat/completions",
        Some(("authorization", "Bearer openrouter-key")),
        "openrouter transport ok",
    );

    assert_env_complete_uses_http_endpoint(
        "litellm",
        "team-model",
        |base_url| {
            BTreeMap::from([
                ("LITELLM_BASE_URL".to_string(), base_url.to_string()),
                ("LITELLM_API_KEY".to_string(), "litellm-key".to_string()),
            ])
        },
        chat_completions_response("chat_http", "litellm transport ok"),
        "/v1/chat/completions",
        Some(("authorization", "Bearer litellm-key")),
        "litellm transport ok",
    );

    assert_env_complete_uses_http_endpoint(
        "openai_compatible",
        "team-model",
        |base_url| {
            BTreeMap::from([
                (
                    "OPENAI_COMPATIBLE_BASE_URL".to_string(),
                    base_url.to_string(),
                ),
                (
                    "OPENAI_COMPATIBLE_API_KEY".to_string(),
                    "compatible-key".to_string(),
                ),
            ])
        },
        chat_completions_response("chat_http", "compatible transport ok"),
        "/v1/chat/completions",
        Some(("authorization", "Bearer compatible-key")),
        "compatible transport ok",
    );
}

#[test]
fn http_transport_streams_openai_responses_incrementally_before_response_end() {
    let server = StreamingTestServer::gated(vec![
        sse_chunk(json!({"type": "response.output_text.delta", "delta": "Hel"})),
        [
            sse_chunk(json!({"type": "response.output_text.delta", "delta": "lo"})),
            sse_chunk(json!({
                "type": "response.completed",
                "response": {
                    "id": "resp_stream_http",
                    "model": "gpt-5.2",
                    "status": "completed",
                    "output": [{"type": "output_text", "text": "Hello"}],
                    "usage": {"input_tokens": 1, "output_tokens": 2, "total_tokens": 3}
                }
            })),
        ]
        .concat(),
    ]);
    let client = Client::from_env_map(
        &BTreeMap::from([
            ("OPENAI_API_KEY".to_string(), "stream-key".to_string()),
            ("OPENAI_BASE_URL".to_string(), server.base_url.clone()),
        ]),
        Some("openai"),
    )
    .unwrap();

    let (event_sender, event_receiver) = mpsc::channel();
    let stream_thread = thread::spawn(move || {
        let mut stream = client
            .stream(Request {
                model: "gpt-5.2".to_string(),
                messages: vec![Message::user("hello")],
                ..Request::default()
            })
            .unwrap();
        for _ in 0..3 {
            event_sender
                .send(stream.next().expect("stream event").unwrap())
                .expect("send early stream event");
        }
        stream.collect::<Result<Vec<_>, _>>()
    });

    server.wait_for_first_chunk();
    let early_events = [
        recv_stream_event(&event_receiver),
        recv_stream_event(&event_receiver),
        recv_stream_event(&event_receiver),
    ];
    assert_eq!(early_events[0].r#type, StreamEventType::StreamStart);
    assert_eq!(early_events[1].r#type, StreamEventType::TextStart);
    assert_eq!(early_events[2].r#type, StreamEventType::TextDelta);
    assert_eq!(early_events[2].delta.as_deref(), Some("Hel"));

    server.continue_stream();
    let mut events = early_events.to_vec();
    events.extend(
        stream_thread
            .join()
            .expect("stream thread joined")
            .expect("stream completion"),
    );
    let response = StreamAccumulator::from_events(events).response;
    assert_eq!(response.text(), "Hello");
    assert_eq!(response.usage.total_tokens, 3);

    let captured = server.captured();
    assert_eq!(captured.path, "/v1/responses");
    assert_eq!(
        header_value(&captured.headers, "authorization"),
        Some("Bearer stream-key")
    );
}

#[test]
fn http_transport_streams_openai_compatible_chat_sse_until_done() {
    let body = [
        sse_text(json!({
            "id": "chatcmpl_http",
            "model": "team-model",
            "choices": [{"index": 0, "delta": {"role": "assistant"}}]
        })),
        sse_text(json!({
            "id": "chatcmpl_http",
            "model": "team-model",
            "choices": [{"index": 0, "delta": {"content": "Hi"}}]
        })),
        sse_text(json!({
            "id": "chatcmpl_http",
            "model": "team-model",
            "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
        })),
        "data: [DONE]\n\n".to_string(),
    ]
    .join("");
    let server = TestServer::new(TestHttpResponse {
        status: 200,
        headers: BTreeMap::from([("Content-Type".to_string(), "text/event-stream".to_string())]),
        body: body.into_bytes(),
        delay_before_response: None,
    });
    let adapter: std::sync::Arc<dyn ProviderAdapter> =
        std::sync::Arc::new(OpenAICompatibleAdapter::openai_compatible(
            OpenAICompatibleRequestConfig {
                api_key: Some("compatible-key".to_string()),
                base_url: Some(server.base_url.clone()),
                timeout: AdapterTimeout::new(1.0, 2.0, 3.0),
                ..OpenAICompatibleRequestConfig::default()
            },
            std::sync::Arc::new(NativeHttpTransport::new()),
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

    assert_eq!(
        events.last().expect("finish event").r#type,
        StreamEventType::Finish
    );
    assert_eq!(response.text(), "Hi");
    assert_eq!(response.usage.total_tokens, 2);

    let captured = server.captured();
    assert_eq!(captured.path, "/v1/chat/completions");
    assert_eq!(
        header_value(&captured.headers, "authorization"),
        Some("Bearer compatible-key")
    );
}

#[test]
fn http_transport_stream_status_errors_preserve_retry_after_and_raw_body() {
    let server = TestServer::new(TestHttpResponse {
        status: 429,
        headers: BTreeMap::from([("Retry-After".to_string(), "7".to_string())]),
        body: br#"{"error":{"message":"stream rate limited","code":"rate_limit_exceeded"}}"#
            .to_vec(),
        delay_before_response: None,
    });
    let adapter: std::sync::Arc<dyn ProviderAdapter> =
        std::sync::Arc::new(OpenAICompatibleAdapter::openai_compatible(
            OpenAICompatibleRequestConfig {
                api_key: Some("compatible-key".to_string()),
                base_url: Some(server.base_url.clone()),
                timeout: AdapterTimeout::new(1.0, 2.0, 3.0),
                ..OpenAICompatibleRequestConfig::default()
            },
            std::sync::Arc::new(NativeHttpTransport::new()),
        ));
    let client = Client::from_adapters([adapter], Some("openai_compatible")).unwrap();

    let error = match client.stream(Request {
        model: "team-model".to_string(),
        messages: vec![Message::user("hello")],
        ..Request::default()
    }) {
        Ok(_) => panic!("stream status error should fail before yielding events"),
        Err(error) => error,
    };

    assert_eq!(error.kind, AdapterErrorKind::RateLimit);
    assert_eq!(error.status_code, Some(429));
    assert_eq!(error.retry_after, Some(7.0));
    assert_eq!(error.message, "stream rate limited");
    assert_eq!(
        error.raw.as_ref().unwrap()["error"]["code"],
        "rate_limit_exceeded"
    );

    let captured = server.captured();
    assert_eq!(captured.path, "/v1/chat/completions");
}

#[test]
fn http_transport_stream_read_timeout_after_partial_event_surfaces_iterator_error() {
    let server = StreamingTestServer::delayed(
        vec![
            sse_chunk(json!({"type": "response.output_text.delta", "delta": "partial"})),
            sse_chunk(json!({"type": "response.output_text.delta", "delta": "late"})),
        ],
        Duration::from_millis(300),
    );
    let adapter: std::sync::Arc<dyn ProviderAdapter> =
        std::sync::Arc::new(NativeProviderAdapter::openai(
            NativeRequestConfig {
                api_key: Some("stream-key".to_string()),
                base_url: Some(server.base_url.clone()),
                timeout: AdapterTimeout::new(1.0, 0.1, 0.05),
                ..NativeRequestConfig::default()
            },
            std::sync::Arc::new(NativeHttpTransport::new()),
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
    let delta = stream.next().expect("text delta").unwrap();
    assert_eq!(delta.r#type, StreamEventType::TextDelta);
    assert_eq!(delta.delta.as_deref(), Some("partial"));

    let error = stream
        .next()
        .expect("post-partial stream read error")
        .unwrap_err();
    assert_eq!(error.kind, AdapterErrorKind::RequestTimeout, "{error:?}");
    assert_eq!(error.status_code, Some(200));
    assert_eq!(error.raw.as_ref().unwrap()["scope"], json!("stream_read"));
    assert_eq!(error.raw.as_ref().unwrap()["timeout"], json!(0.05));

    let captured = server.captured();
    assert_eq!(captured.path, "/v1/responses");
}

#[test]
fn http_transport_request_timeout_reports_request_scope_and_supplied_timeout() {
    let server = TestServer::new(TestHttpResponse {
        status: 200,
        headers: BTreeMap::new(),
        body: br#"{"ok":true}"#.to_vec(),
        delay_before_response: Some(Duration::from_millis(250)),
    });
    let transport = NativeHttpTransport::new();

    let error = transport
        .complete(NativeCompleteRequest {
            provider: "openai".to_string(),
            method: "POST".to_string(),
            url: server.url("/slow"),
            headers: BTreeMap::new(),
            timeout: AdapterTimeout::new(1.0, 0.05, 3.0),
            abort_signal: None,
            body: json!({"model": "gpt-5.2"}),
        })
        .unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::RequestTimeout);
    assert_eq!(error.provider.as_deref(), Some("openai"));
    let raw = error.raw.as_ref().expect("timeout raw metadata");
    assert_eq!(raw["scope"], json!("request"));
    assert_eq!(raw["timeout"], json!(0.05));

    let captured = server.captured();
    assert_eq!(captured.path, "/slow");
}

#[test]
fn http_transport_network_errors_preserve_request_metadata_without_response_status() {
    let transport = NativeHttpTransport::new();
    let url = format!("{}/unreachable", unused_base_url());

    let error = transport
        .complete(NativeCompleteRequest {
            provider: "openai".to_string(),
            method: "POST".to_string(),
            url: url.clone(),
            headers: BTreeMap::from([("X-Trace".to_string(), "network-error".to_string())]),
            timeout: AdapterTimeout::new(0.2, 0.2, 0.2),
            abort_signal: None,
            body: json!({"model": "gpt-5.2"}),
        })
        .unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::Network, "{error:?}");
    assert_eq!(error.provider.as_deref(), Some("openai"));
    assert_eq!(error.status_code, None);
    let raw = error.raw.as_ref().expect("network raw metadata");
    assert_eq!(raw["scope"], json!("network"));
    assert_eq!(raw["method"], json!("POST"));
    assert_eq!(raw["url"], json!(url));
}

#[test]
fn profile_backed_local_openai_compatible_endpoint_uses_http_transport_without_api_key() {
    let server = TestServer::new(TestHttpResponse {
        status: 200,
        headers: BTreeMap::from([("Content-Type".to_string(), "application/json".to_string())]),
        body: serde_json::to_vec(&chat_completions_response(
            "chat_profile",
            "profile transport ok",
        ))
        .unwrap(),
        delay_before_response: None,
    });
    let temp = tempfile::tempdir().expect("profile tempdir");
    std::fs::write(
        temp.path().join("llm-profiles.toml"),
        format!(
            r#"
[profiles.local]
provider = "openai_compatible"
base_url = "{}/local"
models = ["local-model"]
default_model = "local-model"
"#,
            server.base_url
        ),
    )
    .expect("write profile config");
    let client =
        Client::from_env_map_and_profiles(&BTreeMap::new(), temp.path(), Some("local")).unwrap();

    let response = client
        .complete(Request {
            model: "local-model".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap();

    assert_eq!(response.provider, "openai_compatible");
    assert_eq!(response.text(), "profile transport ok");

    let captured = server.captured();
    assert_eq!(captured.method, "POST");
    assert_eq!(captured.path, "/local/v1/chat/completions");
    assert_eq!(header_value(&captured.headers, "authorization"), None);
    assert_eq!(
        serde_json::from_str::<Value>(&captured.body).unwrap()["model"],
        json!("local-model")
    );
}

#[test]
fn http_transport_stream_close_releases_network_response_before_remainder_is_read() {
    let server = CloseAwareStreamingTestServer::new(sse_chunk(json!({
        "type": "response.output_text.delta",
        "delta": "partial"
    })));
    let adapter: std::sync::Arc<dyn ProviderAdapter> =
        std::sync::Arc::new(NativeProviderAdapter::openai(
            NativeRequestConfig {
                api_key: Some("stream-key".to_string()),
                base_url: Some(server.base_url.clone()),
                timeout: AdapterTimeout::new(1.0, 2.0, 3.0),
                ..NativeRequestConfig::default()
            },
            std::sync::Arc::new(NativeHttpTransport::new()),
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

    stream.close().unwrap();
    server.wait_for_client_close();
    let captured = server.captured();
    assert_eq!(captured.path, "/v1/responses");
}

fn assert_env_complete_uses_http_endpoint(
    provider: &str,
    model: &str,
    env_for_base_url: impl FnOnce(&str) -> BTreeMap<String, String>,
    response_body: Value,
    expected_path: &str,
    expected_auth_header: Option<(&str, &str)>,
    expected_text: &str,
) {
    let server = TestServer::new(TestHttpResponse {
        status: 200,
        headers: BTreeMap::from([("Content-Type".to_string(), "application/json".to_string())]),
        body: serde_json::to_vec(&response_body).expect("serialize provider response"),
        delay_before_response: None,
    });
    let client = Client::from_env_map(&env_for_base_url(&server.base_url), Some(provider)).unwrap();

    let response = client
        .complete(Request {
            model: model.to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap();

    assert_eq!(response.provider, provider);
    assert_eq!(response.text(), expected_text);

    let captured = server.captured();
    assert_eq!(captured.method, "POST");
    assert_eq!(captured.path, expected_path);
    assert_eq!(
        header_value(&captured.headers, "content-type"),
        Some("application/json")
    );
    if let Some((name, value)) = expected_auth_header {
        assert_eq!(header_value(&captured.headers, name), Some(value));
    }
}

fn chat_completions_response(id: &str, content: &str) -> Value {
    json!({
        "id": id,
        "object": "chat.completion",
        "model": "team-model",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop",
        }],
        "usage": {
            "prompt_tokens": 1,
            "completion_tokens": 2,
            "total_tokens": 3,
        },
    })
}

#[derive(Debug)]
struct TestHttpResponse {
    status: u16,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
    delay_before_response: Option<Duration>,
}

#[derive(Debug)]
struct CapturedHttpRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: String,
}

struct TestServer {
    base_url: String,
    receiver: Receiver<CapturedHttpRequest>,
    handle: Option<JoinHandle<()>>,
}

impl TestServer {
    fn new(response: TestHttpResponse) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test HTTP listener");
        let address = listener.local_addr().expect("test HTTP address");
        let (sender, receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("test HTTP accept");
            let captured = read_http_request(&mut stream);
            sender.send(captured).expect("send captured request");
            if let Some(delay) = response.delay_before_response {
                thread::sleep(delay);
            }
            let _ = write_http_response(&mut stream, response);
        });
        Self {
            base_url: format!("http://{address}"),
            receiver,
            handle: Some(handle),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn captured(mut self) -> CapturedHttpRequest {
        let captured = self
            .receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("captured HTTP request");
        self.handle
            .take()
            .unwrap()
            .join()
            .expect("test server joined");
        captured
    }
}

enum StreamPause {
    Wait(mpsc::Receiver<()>),
    Delay(Duration),
}

struct StreamingTestServer {
    base_url: String,
    receiver: Receiver<CapturedHttpRequest>,
    first_chunk_receiver: Receiver<()>,
    continue_sender: Option<mpsc::Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl StreamingTestServer {
    fn gated(chunks: Vec<Vec<u8>>) -> Self {
        let (continue_sender, continue_receiver) = mpsc::channel();
        Self::new(
            chunks,
            StreamPause::Wait(continue_receiver),
            Some(continue_sender),
        )
    }

    fn delayed(chunks: Vec<Vec<u8>>, delay: Duration) -> Self {
        Self::new(chunks, StreamPause::Delay(delay), None)
    }

    fn new(
        chunks: Vec<Vec<u8>>,
        pause: StreamPause,
        continue_sender: Option<mpsc::Sender<()>>,
    ) -> Self {
        assert!(!chunks.is_empty(), "streaming response requires chunks");
        let listener = TcpListener::bind("127.0.0.1:0").expect("test streaming HTTP listener");
        let address = listener.local_addr().expect("test streaming HTTP address");
        let (sender, receiver) = mpsc::channel();
        let (first_chunk_sender, first_chunk_receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("test streaming HTTP accept");
            let captured = read_http_request(&mut stream);
            sender.send(captured).expect("send captured stream request");
            write_streaming_response_headers(&mut stream).expect("write stream headers");
            stream
                .write_all(&chunks[0])
                .expect("write first stream chunk");
            stream.flush().expect("flush first stream chunk");
            first_chunk_sender
                .send(())
                .expect("send first stream chunk signal");
            match pause {
                StreamPause::Wait(receiver) => {
                    let _ = receiver.recv_timeout(Duration::from_secs(5));
                }
                StreamPause::Delay(delay) => thread::sleep(delay),
            }
            for chunk in chunks.into_iter().skip(1) {
                if stream.write_all(&chunk).is_err() {
                    return;
                }
                let _ = stream.flush();
            }
        });
        Self {
            base_url: format!("http://{address}"),
            receiver,
            first_chunk_receiver,
            continue_sender,
            handle: Some(handle),
        }
    }

    fn wait_for_first_chunk(&self) {
        self.first_chunk_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("first stream chunk sent");
    }

    fn continue_stream(&self) {
        if let Some(sender) = &self.continue_sender {
            sender.send(()).expect("continue streaming response");
        }
    }

    fn captured(mut self) -> CapturedHttpRequest {
        let captured = self
            .receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("captured streaming HTTP request");
        self.handle
            .take()
            .unwrap()
            .join()
            .expect("streaming test server joined");
        captured
    }
}

struct CloseAwareStreamingTestServer {
    base_url: String,
    receiver: Receiver<CapturedHttpRequest>,
    client_close_receiver: Receiver<()>,
    handle: Option<JoinHandle<()>>,
}

impl CloseAwareStreamingTestServer {
    fn new(first_chunk: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("close-aware HTTP listener");
        let address = listener.local_addr().expect("close-aware HTTP address");
        let (sender, receiver) = mpsc::channel();
        let (client_close_sender, client_close_receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("close-aware HTTP accept");
            let captured = read_http_request(&mut stream);
            sender.send(captured).expect("send captured request");
            write_streaming_response_headers(&mut stream).expect("write stream headers");
            stream
                .write_all(&first_chunk)
                .expect("write first stream chunk");
            stream.flush().expect("flush first stream chunk");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("set close read timeout");
            let mut buffer = [0_u8; 1];
            match stream.read(&mut buffer) {
                Ok(0) => client_close_sender
                    .send(())
                    .expect("send client close signal"),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::BrokenPipe
                            | std::io::ErrorKind::UnexpectedEof
                    ) =>
                {
                    client_close_sender
                        .send(())
                        .expect("send client close signal");
                }
                Ok(_) | Err(_) => {}
            }
        });
        Self {
            base_url: format!("http://{address}"),
            receiver,
            client_close_receiver,
            handle: Some(handle),
        }
    }

    fn wait_for_client_close(&self) {
        self.client_close_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("client closed streaming response");
    }

    fn captured(mut self) -> CapturedHttpRequest {
        let captured = self
            .receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("captured close-aware HTTP request");
        self.handle
            .take()
            .unwrap()
            .join()
            .expect("close-aware test server joined");
        captured
    }
}

fn read_http_request(stream: &mut TcpStream) -> CapturedHttpRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set read timeout");
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    let header_end = loop {
        let count = stream.read(&mut buffer).expect("read request bytes");
        assert!(count > 0, "client closed before sending headers");
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(index) = find_header_end(&bytes) {
            break index;
        }
    };

    let header_text = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().expect("request line");
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap_or_default().to_string();
    let path = request_parts.next().unwrap_or_default().to_string();
    let mut headers = BTreeMap::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        headers.insert(name.to_string(), value.trim().to_string());
    }

    let content_length = header_value(&headers, "content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let body_start = header_end + 4;
    while bytes.len().saturating_sub(body_start) < content_length {
        let count = stream.read(&mut buffer).expect("read request body");
        assert!(count > 0, "client closed before sending full body");
        bytes.extend_from_slice(&buffer[..count]);
    }
    let body = String::from_utf8_lossy(&bytes[body_start..body_start + content_length]).into();

    CapturedHttpRequest {
        method,
        path,
        headers,
        body,
    }
}

fn write_streaming_response_headers(stream: &mut TcpStream) -> std::io::Result<()> {
    write!(stream, "HTTP/1.1 200 OK\r\n")?;
    write!(stream, "Content-Type: text/event-stream\r\n")?;
    write!(stream, "Connection: close\r\n")?;
    write!(stream, "\r\n")?;
    stream.flush()
}

fn write_http_response(stream: &mut TcpStream, response: TestHttpResponse) -> std::io::Result<()> {
    let reason = match response.status {
        200..=299 => "OK",
        429 => "Too Many Requests",
        _ => "Provider Response",
    };
    write!(stream, "HTTP/1.1 {} {}\r\n", response.status, reason)?;
    let has_content_length = response
        .headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case("content-length"));
    let has_connection = response
        .headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case("connection"));
    for (name, value) in response.headers {
        write!(stream, "{name}: {value}\r\n")?;
    }
    if !has_content_length {
        write!(stream, "Content-Length: {}\r\n", response.body.len())?;
    }
    if !has_connection {
        write!(stream, "Connection: close\r\n")?;
    }
    write!(stream, "\r\n")?;
    stream.write_all(&response.body)?;
    stream.flush()
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn header_value<'a>(headers: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn sse_chunk(payload: Value) -> Vec<u8> {
    sse_text(payload).into_bytes()
}

fn sse_text(payload: Value) -> String {
    format!("data: {payload}\n\n")
}

fn recv_stream_event(receiver: &Receiver<StreamEvent>) -> StreamEvent {
    receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("incremental stream event")
}

fn unused_base_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("unused local listener");
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    url
}
