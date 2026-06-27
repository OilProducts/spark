use std::any::TypeId;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::json;
use unified_llm_adapter::{
    classify_grpc_code, classify_http_status_code, classify_provider_error_message,
    error_from_grpc_code, error_from_status_code, generate_object_with_policy_and_hooks,
    generate_steps_with_policy_and_hooks, get_default_client, managed_stream, parse_sse_stream,
    retry_after_from_headers, retry_stream_before_first_event_with_hooks, retry_with_hooks,
    set_default_client, stream_events, AdapterError, AdapterErrorKind, AudioData, Client,
    ContentKind, ContentPart, DocumentData, FinishReason, FinishReasonKind, ImageData, LlmRequest,
    LlmResponse, Message, MessageRole, Middleware, ProviderAdapter, ProviderError, RateLimitInfo,
    Request, Response, ResponseFormat, RetryPolicy, Role, SDKError, SDKErrorKind, SseParser,
    StreamAccumulator, StreamEvent, StreamEventType, StreamEvents, ThinkingData, ToolCall,
    ToolCallData, Usage, Warning,
};

static DEFAULT_CLIENT_TEST_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn canonical_public_surface_round_trips_with_legacy_aliases() {
    assert_eq!(TypeId::of::<Request>(), TypeId::of::<LlmRequest>());
    assert_eq!(TypeId::of::<Response>(), TypeId::of::<LlmResponse>());
    assert_eq!(TypeId::of::<Role>(), TypeId::of::<MessageRole>());

    let request = Request {
        model: "gpt-5.2".to_string(),
        provider: Some("OpenAI".to_string()),
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentPart::Text {
                text: "hello".to_string(),
            }],
            name: None,
            tool_call_id: None,
            provider_metadata: BTreeMap::from([("source".to_string(), json!("test"))]),
        }],
        tools: vec![json!({"type": "function", "function": {"name": "lookup"}})],
        tool_choice: Some(json!("auto")),
        response_format: Some(ResponseFormat::JsonObject),
        temperature: Some(0.2),
        top_p: Some(0.9),
        max_tokens: Some(1024),
        stop_sequences: vec!["END".to_string()],
        reasoning_effort: Some("medium".to_string()),
        metadata: BTreeMap::from([("run_id".to_string(), json!("run-1"))]),
        provider_options: BTreeMap::from([("openai".to_string(), json!({"trace": true}))]),
    };
    let legacy_request: LlmRequest = request.clone();

    let request_json = serde_json::to_value(&request).unwrap();
    let legacy_request_json = serde_json::to_value(&legacy_request).unwrap();
    assert_eq!(request_json, legacy_request_json);
    assert_eq!(
        serde_json::from_value::<Request>(legacy_request_json).unwrap(),
        serde_json::from_value::<LlmRequest>(request_json).unwrap(),
    );

    let response = Response {
        id: "resp-1".to_string(),
        model: "gpt-5.2-actual".to_string(),
        provider: "openai".to_string(),
        message: Message::assistant("done"),
        finish_reason: FinishReason::Stop,
        usage: Usage {
            input_tokens: 4,
            output_tokens: 2,
            ..Usage::default()
        },
        raw: Some(json!({"id": "resp-1"})),
        warnings: vec![Warning {
            message: "ignored option".to_string(),
            code: Some("unsupported_option".to_string()),
        }],
        rate_limit: Some(RateLimitInfo {
            requests_remaining: Some(7),
            requests_limit: Some(10),
            tokens_remaining: Some(99),
            tokens_limit: Some(100),
            reset_at: Some("2026-04-21T00:00:00Z".to_string()),
        }),
        ..Response::default()
    };
    let legacy_response: LlmResponse = response.clone();

    let response_json = serde_json::to_value(&response).unwrap();
    let legacy_response_json = serde_json::to_value(&legacy_response).unwrap();
    assert_eq!(response_json, legacy_response_json);
    assert_eq!(
        serde_json::from_value::<Response>(legacy_response_json).unwrap(),
        serde_json::from_value::<LlmResponse>(response_json).unwrap(),
    );
}

#[test]
fn canonical_error_surface_round_trips_with_legacy_aliases() {
    assert_eq!(TypeId::of::<SDKError>(), TypeId::of::<AdapterError>());
    assert_eq!(TypeId::of::<SDKError>(), TypeId::of::<ProviderError>());
    assert_eq!(
        TypeId::of::<SDKErrorKind>(),
        TypeId::of::<AdapterErrorKind>()
    );

    let canonical = SDKError {
        kind: SDKErrorKind::RateLimit,
        message: "slow down".to_string(),
        provider: Some("openai".to_string()),
        status_code: Some(429),
        error_code: Some("rate_limit".to_string()),
        retryable: true,
        retry_after: Some(7.0),
        raw: Some(json!({"error": {"message": "slow down", "code": "rate_limit"}})),
    };
    let legacy: AdapterError = canonical.clone();
    let provider_error: ProviderError = canonical.clone();

    assert_eq!(
        serde_json::to_value(&canonical).unwrap(),
        json!({
            "kind": "rate_limit",
            "message": "slow down",
            "provider": "openai",
            "status_code": 429,
            "error_code": "rate_limit",
            "retryable": true,
            "retry_after": 7.0,
            "raw": {"error": {"message": "slow down", "code": "rate_limit"}},
        })
    );
    assert_eq!(
        serde_json::to_value(&legacy).unwrap(),
        serde_json::to_value(&canonical).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&provider_error).unwrap(),
        serde_json::to_value(&canonical).unwrap()
    );
    assert_eq!(
        serde_json::from_value::<AdapterError>(serde_json::to_value(&canonical).unwrap()).unwrap(),
        canonical
    );
}

#[test]
fn sdk_error_kinds_expose_spec_names_and_retry_defaults() {
    let cases = [
        (SDKErrorKind::Provider, "provider", "ProviderError", true),
        (
            SDKErrorKind::Authentication,
            "authentication",
            "AuthenticationError",
            false,
        ),
        (
            SDKErrorKind::AccessDenied,
            "access_denied",
            "AccessDeniedError",
            false,
        ),
        (SDKErrorKind::NotFound, "not_found", "NotFoundError", false),
        (
            SDKErrorKind::InvalidRequest,
            "invalid_request",
            "InvalidRequestError",
            false,
        ),
        (
            SDKErrorKind::RateLimit,
            "rate_limit",
            "RateLimitError",
            true,
        ),
        (SDKErrorKind::Server, "server", "ServerError", true),
        (
            SDKErrorKind::ContentFilter,
            "content_filter",
            "ContentFilterError",
            false,
        ),
        (
            SDKErrorKind::ContextLength,
            "context_length",
            "ContextLengthError",
            false,
        ),
        (
            SDKErrorKind::QuotaExceeded,
            "quota_exceeded",
            "QuotaExceededError",
            false,
        ),
        (
            SDKErrorKind::RequestTimeout,
            "request_timeout",
            "RequestTimeoutError",
            false,
        ),
        (SDKErrorKind::Abort, "abort", "AbortError", false),
        (SDKErrorKind::Network, "network", "NetworkError", true),
        (SDKErrorKind::Stream, "stream", "StreamError", true),
        (
            SDKErrorKind::InvalidToolCall,
            "invalid_tool_call",
            "InvalidToolCallError",
            false,
        ),
        (
            SDKErrorKind::UnsupportedToolChoice,
            "unsupported_tool_choice",
            "UnsupportedToolChoiceError",
            false,
        ),
        (
            SDKErrorKind::NoObjectGenerated,
            "no_object_generated",
            "NoObjectGeneratedError",
            false,
        ),
        (
            SDKErrorKind::Configuration,
            "configuration",
            "ConfigurationError",
            false,
        ),
    ];

    for (kind, wire_value, spec_name, retryable) in cases {
        assert_eq!(serde_json::to_value(kind).unwrap(), json!(wire_value));
        assert_eq!(kind.spec_error_name(), spec_name);
        assert_eq!(kind.default_retryable(), retryable);
        assert_eq!(SDKError::new(kind, "boom").retryable, retryable);
    }
}

#[test]
fn provider_error_classifiers_preserve_metadata_and_raw_payloads() {
    let message_cases = [
        ("model does not exist", None, SDKErrorKind::NotFound),
        ("invalid key provided", None, SDKErrorKind::Authentication),
        ("permission denied", None, SDKErrorKind::AccessDenied),
        (
            "too many tokens in prompt",
            None,
            SDKErrorKind::ContextLength,
        ),
        (
            "content filter safety block",
            None,
            SDKErrorKind::ContentFilter,
        ),
        ("quota exceeded", None, SDKErrorKind::QuotaExceeded),
        ("", Some("insufficient_quota"), SDKErrorKind::QuotaExceeded),
    ];
    for (message, error_code, expected) in message_cases {
        assert_eq!(
            classify_provider_error_message(Some(message), error_code),
            Some(expected)
        );
    }

    let http_cases = [
        (400, SDKErrorKind::InvalidRequest),
        (401, SDKErrorKind::Authentication),
        (403, SDKErrorKind::AccessDenied),
        (404, SDKErrorKind::NotFound),
        (408, SDKErrorKind::RequestTimeout),
        (413, SDKErrorKind::ContextLength),
        (422, SDKErrorKind::InvalidRequest),
        (429, SDKErrorKind::RateLimit),
        (500, SDKErrorKind::Server),
        (599, SDKErrorKind::Server),
    ];
    for (status_code, expected) in http_cases {
        assert_eq!(classify_http_status_code(Some(status_code)), expected);
    }

    let grpc_cases = [
        ("NOT_FOUND", SDKErrorKind::NotFound),
        ("INVALID_ARGUMENT", SDKErrorKind::InvalidRequest),
        ("UNAUTHENTICATED", SDKErrorKind::Authentication),
        ("PERMISSION_DENIED", SDKErrorKind::AccessDenied),
        ("RESOURCE_EXHAUSTED", SDKErrorKind::RateLimit),
        ("UNAVAILABLE", SDKErrorKind::Server),
        ("DEADLINE_EXCEEDED", SDKErrorKind::RequestTimeout),
        ("INTERNAL", SDKErrorKind::Server),
    ];
    for (grpc_code, expected) in grpc_cases {
        assert_eq!(classify_grpc_code(Some(grpc_code)), expected);
    }
    assert_eq!(
        classify_grpc_code(Some("google.rpc.Code.RESOURCE_EXHAUSTED")),
        SDKErrorKind::RateLimit
    );

    let headers = BTreeMap::from([("Retry-After".to_string(), "7".to_string())]);
    let raw = json!({
        "error": {
            "message": "slow down",
            "status": "RESOURCE_EXHAUSTED",
            "code": 429
        }
    });
    let error = error_from_status_code(
        Some(400),
        "",
        Some("gemini"),
        None,
        retry_after_from_headers(&headers),
        Some(raw.clone()),
    );
    assert_eq!(error.kind, SDKErrorKind::RateLimit);
    assert_eq!(error.message, "slow down");
    assert_eq!(error.provider.as_deref(), Some("gemini"));
    assert_eq!(error.status_code, Some(400));
    assert_eq!(error.error_code.as_deref(), Some("RESOURCE_EXHAUSTED"));
    assert_eq!(error.retry_after, Some(7.0));
    assert_eq!(error.raw, Some(raw));
    assert!(error.retryable);

    let content_filter = error_from_grpc_code(
        "INVALID_ARGUMENT",
        "content filter safety block",
        Some("gemini"),
        None,
        None,
    );
    assert_eq!(content_filter.kind, SDKErrorKind::ContentFilter);
    assert_eq!(
        content_filter.error_code.as_deref(),
        Some("INVALID_ARGUMENT")
    );
    assert!(!content_filter.retryable);

    let timeout = error_from_status_code(
        Some(408),
        "request timed out",
        Some("openai"),
        None,
        None,
        None,
    );
    assert_eq!(timeout.kind, SDKErrorKind::RequestTimeout);
    assert!(!timeout.retryable);

    let configuration = SDKError::new(SDKErrorKind::Configuration, "missing API key");
    assert!(!configuration.retryable);
}

#[test]
fn retry_policy_calculates_delays_and_invokes_observable_callbacks() {
    let callback_events = Arc::new(Mutex::new(Vec::new()));
    let policy = RetryPolicy {
        max_retries: 2,
        jitter: true,
        ..RetryPolicy::default()
    }
    .with_on_retry({
        let callback_events = Arc::clone(&callback_events);
        move |error, attempt, delay| {
            callback_events.lock().expect("callback events").push((
                error.kind,
                error.message.clone(),
                attempt,
                delay,
            ));
        }
    });
    let mut calls = 0;
    let mut sleep_durations = Vec::new();

    let result = retry_with_hooks(
        &policy,
        || {
            calls += 1;
            if calls < 3 {
                Err(retryable_rate_limit_error("slow down"))
            } else {
                Ok("ok")
            }
        },
        || 1.0,
        |delay| sleep_durations.push(delay),
    )
    .unwrap();

    assert_eq!(result, "ok");
    assert_eq!(calls, 3);
    assert_eq!(sleep_durations, vec![1.0, 2.0]);
    assert_eq!(
        callback_events.lock().expect("callback events").clone(),
        vec![
            (AdapterErrorKind::RateLimit, "slow down".to_string(), 0, 1.0),
            (AdapterErrorKind::RateLimit, "slow down".to_string(), 1, 2.0),
        ]
    );

    assert_eq!(
        RetryPolicy::default().calculate_delay(0, None, Some(0.1)),
        Some(0.5)
    );
    assert_eq!(
        RetryPolicy::default().calculate_delay(0, None, Some(2.0)),
        Some(1.5)
    );
    assert_eq!(
        RetryPolicy::default().calculate_delay(10, None, Some(1.0)),
        Some(60.0)
    );
}

#[test]
fn retry_policy_preserves_retry_after_cutoffs_and_zero_retry_budget() {
    let policy = RetryPolicy {
        jitter: false,
        ..RetryPolicy::default()
    };
    let mut short_retry_after = retryable_rate_limit_error("retry soon");
    short_retry_after.retry_after = Some(12.5);
    let mut long_retry_after = retryable_rate_limit_error("retry too late");
    long_retry_after.retry_after = Some(61.0);

    assert_eq!(
        policy.calculate_delay(0, Some(&short_retry_after), None),
        Some(12.5)
    );
    assert_eq!(
        policy.calculate_delay(0, Some(&long_retry_after), None),
        None
    );

    let mut cutoff_calls = 0;
    let cutoff_result: Result<(), AdapterError> = retry_with_hooks(
        &policy,
        || {
            cutoff_calls += 1;
            Err(long_retry_after.clone())
        },
        || 1.0,
        |_| panic!("Retry-After above max_delay must not sleep"),
    );
    let cutoff_error = cutoff_result.unwrap_err();
    assert_eq!(cutoff_calls, 1);
    assert_eq!(cutoff_error.retry_after, Some(61.0));

    let zero_budget = RetryPolicy {
        max_retries: 0,
        jitter: false,
        ..RetryPolicy::default()
    };
    let mut zero_budget_calls = 0;
    let zero_budget_result: Result<(), AdapterError> = retry_with_hooks(
        &zero_budget,
        || {
            zero_budget_calls += 1;
            Err(retryable_rate_limit_error("no retry"))
        },
        || 1.0,
        |_| panic!("max_retries=0 must not sleep"),
    );
    assert_eq!(zero_budget_calls, 1);
    assert_eq!(zero_budget_result.unwrap_err().message, "no retry");

    let timeout = AdapterError::new(AdapterErrorKind::RequestTimeout, "timed out");
    assert!(!timeout.retryable);
    let mut timeout_calls = 0;
    let timeout_result: Result<(), AdapterError> = retry_with_hooks(
        &RetryPolicy {
            max_retries: 1,
            jitter: false,
            ..RetryPolicy::default()
        },
        || {
            timeout_calls += 1;
            Err(timeout.clone())
        },
        || 1.0,
        |_| panic!("non-retryable timeout must not sleep"),
    );
    assert_eq!(timeout_calls, 1);
    assert_eq!(
        timeout_result.unwrap_err().kind,
        AdapterErrorKind::RequestTimeout
    );
}

#[test]
fn retry_stream_helper_only_reissues_before_first_yielded_event() {
    let opens = Arc::new(Mutex::new(0));
    let mut stream = retry_stream_before_first_event_with_hooks(
        RetryPolicy {
            max_retries: 1,
            jitter: false,
            ..RetryPolicy::default()
        },
        {
            let opens = Arc::clone(&opens);
            move || {
                let mut opens = opens.lock().expect("open count");
                *opens += 1;
                if *opens == 1 {
                    Ok(stream_events(
                        vec![Err(retryable_rate_limit_error("first event failed"))].into_iter(),
                    ))
                } else {
                    Ok(stream_events(
                        vec![Ok(StreamEvent::text_delta("after retry"))].into_iter(),
                    ))
                }
            }
        },
        || 1.0,
        |_| {},
    )
    .unwrap();
    let events = stream.by_ref().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(events, vec![StreamEvent::text_delta("after retry")]);
    assert_eq!(*opens.lock().expect("open count"), 2);

    let post_partial_opens = Arc::new(Mutex::new(0));
    let mut post_partial_stream = retry_stream_before_first_event_with_hooks(
        RetryPolicy {
            max_retries: 1,
            jitter: false,
            ..RetryPolicy::default()
        },
        {
            let post_partial_opens = Arc::clone(&post_partial_opens);
            move || {
                let mut opens = post_partial_opens.lock().expect("open count");
                *opens += 1;
                Ok(stream_events(
                    vec![
                        Ok(StreamEvent::text_delta("partial")),
                        Err(retryable_rate_limit_error("after partial")),
                    ]
                    .into_iter(),
                ))
            }
        },
        || 1.0,
        |_| panic!("post-partial stream errors must not be retried"),
    )
    .unwrap();

    assert_eq!(
        post_partial_stream.next().expect("partial event").unwrap(),
        StreamEvent::text_delta("partial")
    );
    let error = post_partial_stream
        .next()
        .expect("post-partial stream error")
        .unwrap_err();
    assert_eq!(error.message, "after partial");
    assert!(post_partial_stream.next().is_none());
    assert_eq!(*post_partial_opens.lock().expect("open count"), 1);
}

#[test]
fn response_defaults_are_required_in_public_serialization_contract() {
    let response = Response::default();
    assert_eq!(response.finish_reason, FinishReason::Other);
    assert_eq!(response.usage, Usage::default());

    let serialized = serde_json::to_value(&response).unwrap();
    assert!(serialized.get("finish_reason").is_some());
    assert!(serialized.get("usage").is_some());
    assert_eq!(serialized["finish_reason"]["reason"], "other");
    assert_eq!(serialized["usage"]["total_tokens"], 0);

    let deserialized: Response = serde_json::from_value(json!({
        "id": "resp-with-defaults",
        "message": {
            "role": "assistant",
            "content": []
        }
    }))
    .unwrap();
    assert_eq!(deserialized.finish_reason, FinishReason::Other);
    assert_eq!(deserialized.usage, Usage::default());

    let legacy: LlmResponse = serde_json::from_value(serialized.clone()).unwrap();
    assert_eq!(serde_json::to_value(&legacy).unwrap(), serialized);
}

#[test]
fn stream_event_uses_canonical_wire_fields_and_preserves_custom_types() {
    let event = StreamEvent {
        r#type: StreamEventType::Custom("provider.chunk".to_string()),
        delta: Some("visible".to_string()),
        text_id: Some("text-1".to_string()),
        reasoning_delta: Some("private".to_string()),
        thinking: None,
        tool_call: Some(ToolCall {
            id: "call-1".to_string(),
            name: "lookup".to_string(),
            arguments: json!({"city": "NYC"}),
            raw_arguments: Some("{\"city\":\"NYC\"}".to_string()),
            r#type: "function".to_string(),
        }),
        finish_reason: Some(FinishReason::Stop),
        usage: Some(Usage {
            input_tokens: 3,
            output_tokens: 4,
            total_tokens: 7,
            ..Usage::default()
        }),
        response: Some(Response::from_text("done")),
        error: Some(unified_llm_adapter::AdapterError::new(
            unified_llm_adapter::AdapterErrorKind::Stream,
            "stream failed",
        )),
        raw: Some(json!({"provider": true})),
    };

    let serialized = serde_json::to_value(&event).unwrap();
    assert_eq!(serialized["type"], "provider.chunk");
    assert_eq!(serialized["delta"], "visible");
    assert_eq!(serialized["text_id"], "text-1");
    assert_eq!(serialized["reasoning_delta"], "private");
    assert!(serialized.get("event_type").is_none());
    assert!(serialized.get("text").is_none());
    assert!(serialized.get("reasoning").is_none());

    let round_tripped: StreamEvent = serde_json::from_value(serialized).unwrap();
    assert_eq!(round_tripped, event);
}

#[test]
fn shared_sse_parser_preserves_provider_neutral_records() {
    let records = parse_sse_stream(
        ": keepalive\n\
         retry: 1500\n\
         event: sse.event.name\n\
         data: {\"type\":\"json.event.name\",\n\
         data: \"message\":\"hello\"}\n\
         \n\
         \n\
         data: [DONE]\n\
         \n\
         event: provider.error\n\
         data: {not-json}\n\
         \n",
    );

    assert_eq!(records.len(), 3);
    assert_eq!(records[0].sse_event.as_deref(), Some("sse.event.name"));
    assert_eq!(records[0].json_event.as_deref(), Some("json.event.name"));
    assert_eq!(records[0].event.as_deref(), Some("json.event.name"));
    assert_eq!(records[0].retry, Some(1500));
    assert_eq!(
        records[0].data,
        "{\"type\":\"json.event.name\",\n\"message\":\"hello\"}"
    );
    assert_eq!(
        records[0].payload.as_ref().unwrap()["message"],
        json!("hello")
    );
    assert!(records[0].payload_error.is_none());

    assert!(records[1].done);
    assert_eq!(records[1].data, "[DONE]");
    assert!(records[1].payload.is_none());
    assert!(records[1].payload_error.is_none());

    assert_eq!(records[2].event.as_deref(), Some("provider.error"));
    assert_eq!(records[2].data, "{not-json}");
    assert!(records[2].payload.is_none());
    let payload_error = records[2]
        .payload_error
        .as_ref()
        .expect("malformed JSON error");
    assert_eq!(payload_error.raw, "{not-json}");
    assert!(!payload_error.message.is_empty());
}

#[test]
fn shared_sse_parser_handles_split_chunks_and_provider_json_event_fields() {
    let mut parser = SseParser::default();
    let mut records = parser.push_str("event: sse.name\n");
    records.extend(parser.push_str("data: {\"event\":\"payload.name\","));
    records.extend(parser.push_str("\"ok\":true}\n\n"));
    records.extend(parser.finish());

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].sse_event.as_deref(), Some("sse.name"));
    assert_eq!(records[0].json_event.as_deref(), Some("payload.name"));
    assert_eq!(records[0].event.as_deref(), Some("payload.name"));
    assert_eq!(records[0].payload.as_ref().unwrap()["ok"], json!(true));
}

#[test]
fn stream_accumulator_consumes_canonical_event_fields() {
    let tool_call = ToolCall {
        id: "call-1".to_string(),
        name: "weather".to_string(),
        arguments: json!({"city": "NYC"}),
        raw_arguments: None,
        r#type: "function".to_string(),
    };
    let mut tool_event = StreamEvent::new(StreamEventType::ToolCallEnd);
    tool_event.tool_call = Some(tool_call.clone());

    let mut accumulator = StreamAccumulator::default();
    accumulator.push(StreamEvent::provider_event(json!({"kind": "provider"})));
    accumulator.push(StreamEvent::text_delta("hello "));
    accumulator.push(StreamEvent::text_delta("world"));
    accumulator.push(StreamEvent::reasoning_delta("private "));
    accumulator.push(StreamEvent::reasoning_delta("plan"));
    accumulator.push(tool_event);
    accumulator.push(StreamEvent::finish(
        FinishReason::Stop,
        Some(Usage {
            input_tokens: 2,
            output_tokens: 5,
            ..Usage::default()
        }),
    ));

    assert_eq!(accumulator.final_text, "hello world");
    assert_eq!(accumulator.reasoning_text, "private plan");
    assert_eq!(accumulator.tool_calls, vec![tool_call]);
    assert_eq!(
        accumulator.raw_provider_events,
        vec![json!({"kind": "provider"})]
    );
    assert_eq!(accumulator.finish_reason, Some(FinishReason::Stop));
    assert_eq!(accumulator.usage.unwrap().total_tokens, 7);
}

#[test]
fn stream_accumulator_rebuilds_lifecycle_response_and_enriches_finish_event() {
    let tool_call = ToolCall {
        id: "call-weather".to_string(),
        name: "weather".to_string(),
        arguments: json!({"city": "Paris"}),
        raw_arguments: Some("{\"city\":\"Paris\"}".to_string()),
        r#type: "function".to_string(),
    };
    let events = vec![
        StreamEvent {
            r#type: StreamEventType::StreamStart,
            raw: Some(json!({"event": "start"})),
            ..StreamEvent::new(StreamEventType::StreamStart)
        },
        StreamEvent::new(StreamEventType::TextStart),
        StreamEvent::text_delta("Hel"),
        StreamEvent::text_delta("lo"),
        StreamEvent {
            r#type: StreamEventType::TextEnd,
            delta: Some("Hello".to_string()),
            ..StreamEvent::new(StreamEventType::TextEnd)
        },
        StreamEvent::new(StreamEventType::ReasoningStart),
        StreamEvent::reasoning_delta("private "),
        StreamEvent {
            r#type: StreamEventType::ReasoningEnd,
            reasoning_delta: Some("private plan".to_string()),
            ..StreamEvent::new(StreamEventType::ReasoningEnd)
        },
        StreamEvent {
            r#type: StreamEventType::ToolCallStart,
            tool_call: Some(tool_call.clone()),
            ..StreamEvent::new(StreamEventType::ToolCallStart)
        },
        StreamEvent {
            r#type: StreamEventType::ToolCallEnd,
            tool_call: Some(tool_call.clone()),
            ..StreamEvent::new(StreamEventType::ToolCallEnd)
        },
        StreamEvent {
            r#type: StreamEventType::ProviderEvent,
            raw: Some(json!({"event": "provider"})),
            ..StreamEvent::new(StreamEventType::ProviderEvent)
        },
        StreamEvent::finish(
            FinishReason::Stop,
            Some(Usage {
                input_tokens: 4,
                output_tokens: 2,
                ..Usage::default()
            }),
        ),
    ];

    let accumulator = StreamAccumulator::from_events(events.clone());

    assert_eq!(accumulator.final_text, "Hello");
    assert_eq!(accumulator.reasoning_text, "private plan");
    assert_eq!(accumulator.tool_calls, vec![tool_call.clone()]);
    assert_eq!(accumulator.finish_reason, Some(FinishReason::Stop));
    assert_eq!(accumulator.usage.as_ref().unwrap().total_tokens, 6);
    assert_eq!(accumulator.response.text(), "Hello");
    assert_eq!(
        accumulator.response.reasoning().as_deref(),
        Some("private plan")
    );
    assert_eq!(accumulator.response.tool_calls(), vec![tool_call]);
    assert_eq!(
        accumulator.raw_provider_events,
        vec![json!({"event": "start"}), json!({"event": "provider"})]
    );
    let finish_event = accumulator.finish_event.expect("finish event");
    assert_eq!(finish_event.r#type, StreamEventType::Finish);
    assert_eq!(
        finish_event.response.expect("finish response").text(),
        "Hello"
    );
    assert_eq!(finish_event.usage.expect("finish usage").total_tokens, 6);

    let only_deltas = events
        .iter()
        .filter(|event| event.r#type == StreamEventType::TextDelta)
        .filter_map(|event| event.delta.as_deref())
        .collect::<String>();
    assert_eq!(only_deltas, "Hello");
}

#[test]
fn stream_accumulator_merges_split_response_usage_and_updates_stored_finish_event() {
    let events = vec![
        StreamEvent {
            r#type: StreamEventType::StreamStart,
            response: Some(Response {
                usage: Usage {
                    input_tokens: 4,
                    cache_read_tokens: Some(2),
                    ..Usage::default()
                },
                ..Response::default()
            }),
            ..StreamEvent::new(StreamEventType::StreamStart)
        },
        StreamEvent::text_delta("complete text"),
        StreamEvent {
            r#type: StreamEventType::Finish,
            finish_reason: Some(FinishReason::Stop),
            response: Some(Response {
                usage: Usage {
                    output_tokens: 7,
                    reasoning_tokens: Some(3),
                    ..Usage::default()
                },
                ..Response::default()
            }),
            ..StreamEvent::new(StreamEventType::Finish)
        },
    ];

    let accumulator = StreamAccumulator::from_events(events);
    let usage = accumulator.usage.as_ref().expect("merged usage");

    assert_eq!(usage.input_tokens, 4);
    assert_eq!(usage.output_tokens, 7);
    assert_eq!(usage.total_tokens, 11);
    assert_eq!(usage.cache_read_tokens, Some(2));
    assert_eq!(usage.reasoning_tokens, Some(3));
    assert_eq!(accumulator.response.usage, *usage);

    let finish_event = accumulator.finish_event.as_ref().expect("finish event");
    assert_eq!(finish_event.usage.as_ref(), Some(usage));
    assert_eq!(finish_event.response.as_ref().unwrap().usage, *usage);
    let stored_finish = accumulator.events.last().expect("stored finish event");
    assert_eq!(stored_finish.r#type, StreamEventType::Finish);
    assert_eq!(stored_finish.usage.as_ref(), Some(usage));
    assert_eq!(stored_finish.response.as_ref().unwrap().usage, *usage);
}

#[test]
fn stream_accumulator_correlates_interleaved_lifecycles_by_stable_ids() {
    let mut events = Vec::new();
    events.push(StreamEvent {
        r#type: StreamEventType::TextStart,
        text_id: Some("text-main".to_string()),
        ..StreamEvent::new(StreamEventType::TextStart)
    });
    events.push(StreamEvent {
        text_id: Some("text-main".to_string()),
        ..StreamEvent::text_delta("Hel")
    });
    events.push(StreamEvent {
        r#type: StreamEventType::ToolCallStart,
        tool_call: Some(ToolCall {
            id: "call-weather".to_string(),
            name: "weather".to_string(),
            arguments: json!(""),
            raw_arguments: Some(String::new()),
            r#type: "function".to_string(),
        }),
        ..StreamEvent::new(StreamEventType::ToolCallStart)
    });
    events.push(StreamEvent {
        r#type: StreamEventType::ToolCallStart,
        tool_call: Some(ToolCall {
            id: "call-search".to_string(),
            name: "search".to_string(),
            arguments: json!(""),
            raw_arguments: Some(String::new()),
            r#type: "function".to_string(),
        }),
        ..StreamEvent::new(StreamEventType::ToolCallStart)
    });
    events.push(StreamEvent {
        r#type: StreamEventType::ToolCallDelta,
        tool_call: Some(ToolCall {
            id: "call-weather".to_string(),
            name: String::new(),
            arguments: json!("{\"city\":"),
            raw_arguments: Some("{\"city\":".to_string()),
            r#type: "function".to_string(),
        }),
        ..StreamEvent::new(StreamEventType::ToolCallDelta)
    });
    events.push(StreamEvent {
        text_id: Some("text-main".to_string()),
        ..StreamEvent::text_delta("lo ")
    });
    events.push(StreamEvent {
        r#type: StreamEventType::ToolCallDelta,
        tool_call: Some(ToolCall {
            id: "call-search".to_string(),
            name: String::new(),
            arguments: json!("{\"query\":"),
            raw_arguments: Some("{\"query\":".to_string()),
            r#type: "function".to_string(),
        }),
        ..StreamEvent::new(StreamEventType::ToolCallDelta)
    });
    events.push(StreamEvent {
        r#type: StreamEventType::ToolCallDelta,
        tool_call: Some(ToolCall {
            id: "call-weather".to_string(),
            name: String::new(),
            arguments: json!("\"Paris\"}"),
            raw_arguments: Some("\"Paris\"}".to_string()),
            r#type: "function".to_string(),
        }),
        ..StreamEvent::new(StreamEventType::ToolCallDelta)
    });
    events.push(StreamEvent {
        text_id: Some("text-main".to_string()),
        ..StreamEvent::text_delta("world")
    });
    events.push(StreamEvent {
        r#type: StreamEventType::ToolCallEnd,
        tool_call: Some(ToolCall {
            id: "call-weather".to_string(),
            name: String::new(),
            arguments: json!(""),
            raw_arguments: Some(String::new()),
            r#type: "function".to_string(),
        }),
        ..StreamEvent::new(StreamEventType::ToolCallEnd)
    });
    events.push(StreamEvent {
        r#type: StreamEventType::ToolCallDelta,
        tool_call: Some(ToolCall {
            id: "call-search".to_string(),
            name: String::new(),
            arguments: json!("\"rust\"}"),
            raw_arguments: Some("\"rust\"}".to_string()),
            r#type: "function".to_string(),
        }),
        ..StreamEvent::new(StreamEventType::ToolCallDelta)
    });
    events.push(StreamEvent {
        r#type: StreamEventType::TextEnd,
        text_id: Some("text-main".to_string()),
        ..StreamEvent::new(StreamEventType::TextEnd)
    });
    events.push(StreamEvent {
        r#type: StreamEventType::ToolCallEnd,
        tool_call: Some(ToolCall {
            id: "call-search".to_string(),
            name: String::new(),
            arguments: json!(""),
            raw_arguments: Some(String::new()),
            r#type: "function".to_string(),
        }),
        ..StreamEvent::new(StreamEventType::ToolCallEnd)
    });
    events.push(StreamEvent::finish(FinishReason::ToolCalls, None));

    let accumulator = StreamAccumulator::from_events(events.clone());
    let only_text_deltas = events
        .iter()
        .filter(|event| event.r#type == StreamEventType::TextDelta)
        .filter_map(|event| event.delta.as_deref())
        .collect::<String>();

    assert_eq!(only_text_deltas, "Hello world");
    assert_eq!(accumulator.final_text, only_text_deltas);
    assert_eq!(accumulator.response.text(), "Hello world");
    assert_eq!(accumulator.tool_calls.len(), 2);
    assert_eq!(accumulator.tool_calls[0].id, "call-weather");
    assert_eq!(
        accumulator.tool_calls[0].raw_arguments.as_deref(),
        Some("{\"city\":\"Paris\"}")
    );
    assert_eq!(accumulator.tool_calls[1].id, "call-search");
    assert_eq!(
        accumulator.tool_calls[1].raw_arguments.as_deref(),
        Some("{\"query\":\"rust\"}")
    );
}

#[test]
fn stream_accumulator_reasoning_lifecycle_is_sequential_without_ids() {
    let events = vec![
        StreamEvent::new(StreamEventType::ReasoningStart),
        StreamEvent::reasoning_delta("first "),
        StreamEvent {
            r#type: StreamEventType::ReasoningEnd,
            reasoning_delta: Some("first thought".to_string()),
            ..StreamEvent::new(StreamEventType::ReasoningEnd)
        },
        StreamEvent::new(StreamEventType::ReasoningStart),
        StreamEvent::reasoning_delta(" second"),
        StreamEvent::new(StreamEventType::ReasoningEnd),
        StreamEvent::finish(FinishReason::Stop, None),
    ];

    let accumulator = StreamAccumulator::from_events(events);

    assert_eq!(accumulator.reasoning_text, "first thought second");
    assert_eq!(
        accumulator.response.reasoning().as_deref(),
        Some("first thought second")
    );
}

#[test]
fn managed_streams_can_be_explicitly_closed_or_closed_on_drop() {
    let explicit_close_calls = Arc::new(Mutex::new(0));
    let mut stream = managed_stream(std::iter::empty(), {
        let explicit_close_calls = Arc::clone(&explicit_close_calls);
        move || {
            *explicit_close_calls.lock().expect("close count") += 1;
            Ok(())
        }
    });

    stream.close().unwrap();
    drop(stream);
    assert_eq!(*explicit_close_calls.lock().expect("close count"), 1);

    let drop_close_calls = Arc::new(Mutex::new(0));
    {
        let _stream = managed_stream(std::iter::empty(), {
            let drop_close_calls = Arc::clone(&drop_close_calls);
            move || {
                *drop_close_calls.lock().expect("drop close count") += 1;
                Ok(())
            }
        });
    }
    assert_eq!(*drop_close_calls.lock().expect("drop close count"), 1);
}

#[test]
fn response_format_uses_spec_wire_values() {
    assert_eq!(
        serde_json::to_value(ResponseFormat::Text).unwrap(),
        json!({"type": "text"})
    );
    assert_eq!(
        serde_json::to_value(ResponseFormat::JsonObject).unwrap(),
        json!({"type": "json"})
    );
    assert_eq!(
        serde_json::to_value(ResponseFormat::JsonSchema {
            json_schema: json!({"name": "Decision", "schema": {"type": "object"}}),
            strict: true,
        })
        .unwrap(),
        json!({
            "type": "json_schema",
            "json_schema": {"name": "Decision", "schema": {"type": "object"}},
            "strict": true
        })
    );

    assert_eq!(
        serde_json::from_value::<ResponseFormat>(json!({"type": "json"})).unwrap(),
        ResponseFormat::JsonObject
    );
    assert_eq!(
        serde_json::from_value::<ResponseFormat>(json!({"type": "json_object"})).unwrap(),
        ResponseFormat::JsonObject
    );
}

#[test]
fn crate_root_client_and_provider_adapter_surface_are_exercisable() {
    let _default_client_guard = DEFAULT_CLIENT_TEST_LOCK
        .lock()
        .expect("default client test lock");
    let previous_default_client = get_default_client();
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(EchoAdapter);
    let client = Client::from_providers(
        BTreeMap::from([("FAKE".to_string(), adapter)]),
        Some("fake"),
    )
    .unwrap();

    let request = Request {
        model: "test-model".to_string(),
        messages: vec![Message::user("hello")],
        ..empty_request()
    };

    let response = client.complete(request.clone()).unwrap();
    assert_eq!(response.text(), "fake:test-model");
    assert_eq!(response.finish_reason, FinishReason::Stop);

    let events = client
        .stream(request)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(events, vec![StreamEvent::text_delta("fake-stream")]);
    assert!(client.supports_tool_choice("auto", None).unwrap());

    set_default_client(Some(client.clone()));
    assert_eq!(get_default_client().default_provider(), Some("fake"));
    set_default_client(Some(previous_default_client));
}

#[test]
fn provider_adapter_registration_uses_rust_trait_objects_and_lifecycle_hooks() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let default_adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingAdapter::new("default", Arc::clone(&calls)));
    let explicit_adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingAdapter::new("explicit", Arc::clone(&calls)));
    let client = Client::from_adapters(
        vec![Arc::clone(&default_adapter), Arc::clone(&explicit_adapter)],
        Some("DEFAULT"),
    )
    .unwrap();

    let names = client.provider_names().collect::<Vec<_>>();
    assert_eq!(names, vec!["default", "explicit"]);
    assert_eq!(
        call_log(&calls),
        vec!["initialize:default", "initialize:explicit"]
    );

    let response = client
        .complete(Request {
            model: "model-default".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap();
    assert_eq!(response.provider, "default");
    assert_eq!(response.model, "model-default");
    assert_eq!(response.text(), "default complete");

    let stream = client
        .stream(Request {
            model: "model-explicit".to_string(),
            provider: Some("EXPLICIT".to_string()),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(stream, vec![StreamEvent::text_delta("explicit stream")]);
    assert!(client
        .supports_tool_choice("auto", Some("explicit"))
        .unwrap());
    assert!(!client
        .supports_tool_choice("required", Some("explicit"))
        .unwrap());

    client.close().unwrap();
    assert_eq!(
        call_log(&calls),
        vec![
            "initialize:default",
            "initialize:explicit",
            "complete:default:model-default",
            "stream:explicit:model-explicit",
            "supports_tool_choice:explicit:auto",
            "supports_tool_choice:explicit:required",
            "close:explicit",
            "close:default",
        ]
    );
}

#[test]
fn low_level_client_complete_and_stream_do_not_retry_retryable_adapter_errors() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(RetryableErrorAdapter {
        calls: Arc::clone(&calls),
    });
    let client = Client::from_adapters(vec![adapter], Some("retryable")).unwrap();
    let request = Request {
        model: "retry-model".to_string(),
        messages: vec![Message::user("hello")],
        ..Request::default()
    };

    let complete_error = client.complete(request.clone()).unwrap_err();
    let stream_error = match client.stream(request) {
        Ok(_) => panic!("low-level stream must surface the first adapter error"),
        Err(error) => error,
    };

    assert_eq!(complete_error.kind, AdapterErrorKind::RateLimit);
    assert_eq!(stream_error.kind, AdapterErrorKind::RateLimit);
    assert_eq!(
        call_log(&calls),
        vec![
            "complete:retryable:retry-model",
            "stream:retryable:retry-model"
        ]
    );
}

#[test]
fn high_level_generate_retries_each_llm_step_without_restarting_completed_steps() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "high",
        Arc::clone(&calls),
        vec![
            Ok(Response {
                message: Message::assistant("needs tool"),
                finish_reason: FinishReason::ToolCalls,
                ..Response::default()
            }),
            Err(retryable_rate_limit_error("retry second step")),
            Ok(Response {
                message: Message::assistant("done"),
                finish_reason: FinishReason::Stop,
                ..Response::default()
            }),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("high")).unwrap();
    let policy = RetryPolicy {
        max_retries: 1,
        jitter: false,
        ..RetryPolicy::default()
    };
    let mut tool_rounds_executed = 0;
    let mut sleep_durations = Vec::new();

    let result = generate_steps_with_policy_and_hooks(
        &client,
        Request {
            model: "step-1".to_string(),
            messages: vec![Message::user("what is the weather?")],
            ..Request::default()
        },
        &policy,
        |steps| {
            if steps.len() == 1 {
                tool_rounds_executed += 1;
                return Ok(Some(Request {
                    model: "step-2".to_string(),
                    messages: vec![
                        steps[0].request.messages[0].clone(),
                        steps[0].response.message.clone(),
                        Message::tool_result("call_weather", json!({"city": "Paris"}), false),
                    ],
                    ..Request::default()
                }));
            }
            Ok(None)
        },
        || 1.0,
        |delay| sleep_durations.push(delay),
    )
    .unwrap();

    assert_eq!(result.text, "done");
    assert_eq!(result.steps.len(), 2);
    assert_eq!(tool_rounds_executed, 1);
    assert_eq!(sleep_durations, vec![1.0]);
    assert_eq!(
        call_log(&calls),
        vec![
            "complete:high:step-1:1",
            "complete:high:step-2:3",
            "complete:high:step-2:3",
        ]
    );
}

#[test]
fn high_level_generate_object_retries_provider_errors_then_parses_successful_response() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "structured",
        Arc::clone(&calls),
        vec![
            Err(retryable_rate_limit_error("retry object call")),
            Ok(Response {
                message: Message::assistant("{\"answer\":\"yes\"}"),
                finish_reason: FinishReason::Stop,
                ..Response::default()
            }),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("structured")).unwrap();
    let policy = RetryPolicy {
        max_retries: 1,
        jitter: false,
        ..RetryPolicy::default()
    };

    let result = generate_object_with_policy_and_hooks(
        &client,
        Request {
            model: "object-model".to_string(),
            messages: vec![Message::user("answer as JSON")],
            ..Request::default()
        },
        object_schema(),
        &policy,
        || 1.0,
        |_| {},
    )
    .unwrap();

    assert_eq!(result.value, json!({"answer": "yes"}));
    assert_eq!(result.raw_text, "{\"answer\":\"yes\"}");
    assert_eq!(
        call_log(&calls),
        vec![
            "complete:structured:object-model:1",
            "complete:structured:object-model:1",
        ]
    );
}

#[test]
fn high_level_generate_object_does_not_retry_no_object_generated_failures() {
    for (response_text, expected_message) in [
        ("not json", "failed to parse structured output as JSON"),
        (
            "{\"answer\":7}",
            "structured output did not match the provided JSON Schema",
        ),
    ] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
            "structured",
            Arc::clone(&calls),
            vec![
                Ok(Response {
                    message: Message::assistant(response_text),
                    finish_reason: FinishReason::Stop,
                    ..Response::default()
                }),
                Ok(Response {
                    message: Message::assistant("{\"answer\":\"late success\"}"),
                    finish_reason: FinishReason::Stop,
                    ..Response::default()
                }),
            ],
        ));
        let client = Client::from_adapters(vec![adapter], Some("structured")).unwrap();
        let policy = RetryPolicy {
            max_retries: 2,
            jitter: false,
            ..RetryPolicy::default()
        };

        let error = generate_object_with_policy_and_hooks(
            &client,
            Request {
                model: "object-model".to_string(),
                messages: vec![Message::user("answer as JSON")],
                ..Request::default()
            },
            object_schema(),
            &policy,
            || 1.0,
            |_| panic!("NoObjectGenerated failures must not sleep or retry"),
        )
        .unwrap_err();

        assert_eq!(error.kind, AdapterErrorKind::NoObjectGenerated);
        assert_eq!(error.message, expected_message);
        assert!(!error.retryable);
        assert_eq!(
            error
                .raw
                .as_ref()
                .and_then(|raw| raw.get("raw_text"))
                .and_then(serde_json::Value::as_str),
            Some(response_text)
        );
        assert_eq!(call_log(&calls), vec!["complete:structured:object-model:1"]);
    }
}

#[test]
fn complete_middleware_uses_onion_order_and_transforms_responses() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RoutingAdapter::new("explicit", Arc::clone(&calls)));
    let first: Arc<dyn Middleware> = Arc::new(CompleteTransformMiddleware::new(
        "first",
        Arc::clone(&calls),
    ));
    let second: Arc<dyn Middleware> = Arc::new(CompleteTransformMiddleware::new(
        "second",
        Arc::clone(&calls),
    ));
    let client = Client::from_adapters(vec![adapter], Some("explicit"))
        .unwrap()
        .with_middleware(vec![first, second]);

    let response = client
        .complete(Request {
            model: "base".to_string(),
            provider: Some("EXPLICIT".to_string()),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap();

    assert_eq!(response.provider, "explicit");
    assert_eq!(response.model, "base-first-second");
    assert_eq!(response.text(), "explicit:base-first-second|second|first");
    assert_eq!(
        call_log(&calls),
        vec![
            "enter:first:explicit:base",
            "enter:second:explicit:base-first",
            "adapter:complete:explicit:base-first-second",
            "exit:second:explicit:base-first-second",
            "exit:first:explicit:base-first-second|second",
        ]
    );
}

#[test]
fn middleware_request_provider_mutation_does_not_bypass_resolved_routing() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let default_adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RoutingAdapter::new("default", Arc::clone(&calls)));
    let explicit_adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RoutingAdapter::new("explicit", Arc::clone(&calls)));
    let override_provider: Arc<dyn Middleware> = Arc::new(ProviderOverrideMiddleware {
        provider: "explicit",
    });
    let client = Client::from_adapters(vec![default_adapter, explicit_adapter], Some("default"))
        .unwrap()
        .with_middleware(vec![override_provider]);

    let default_response = client
        .complete(Request {
            model: "default-route".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap();
    assert_eq!(default_response.provider, "default");
    assert_eq!(default_response.text(), "default:default-route");

    let explicit_response = client
        .complete(Request {
            model: "explicit-route".to_string(),
            provider: Some("EXPLICIT".to_string()),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap();
    assert_eq!(explicit_response.provider, "explicit");
    assert_eq!(explicit_response.text(), "explicit:explicit-route");

    assert_eq!(
        call_log(&calls),
        vec![
            "adapter:complete:default:default-route",
            "adapter:complete:explicit:explicit-route",
        ]
    );
}

#[test]
fn stream_middleware_wraps_events_and_final_stream_errors() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(StreamErrorAdapter::new("default", Arc::clone(&calls)));
    let first: Arc<dyn Middleware> =
        Arc::new(StreamTransformMiddleware::new("first", Arc::clone(&calls)));
    let second: Arc<dyn Middleware> =
        Arc::new(StreamTransformMiddleware::new("second", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], Some("default"))
        .unwrap()
        .with_middleware(vec![first, second]);

    let mut stream = client
        .stream(Request {
            model: "stream-model".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap();

    let event = stream.next().expect("first event").unwrap();
    assert_eq!(event, StreamEvent::text_delta("seed|second|first"));
    let error = stream.next().expect("stream error").unwrap_err();
    assert_eq!(error.kind, unified_llm_adapter::AdapterErrorKind::Stream);
    assert_eq!(error.message, "boom|second|first");
    assert!(stream.next().is_none());

    assert_eq!(
        call_log(&calls),
        vec![
            "stream-enter:first:default:stream-model",
            "stream-enter:second:default:stream-model",
            "adapter:stream:default:stream-model",
            "stream-event:second:seed",
            "stream-event:first:seed|second",
            "stream-error:second:boom",
            "stream-error:first:boom|second",
        ]
    );
}

#[test]
fn absent_middleware_preserves_adapter_outputs_and_error_semantics() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RoutingAdapter::new("default", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], Some("default")).unwrap();

    let response = client
        .complete(Request {
            model: "plain".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap();
    assert_eq!(response.provider, "default");
    assert_eq!(response.text(), "default:plain");

    let events = client
        .stream(Request {
            model: "plain".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(events, vec![StreamEvent::text_delta("default:plain")]);

    let observed = Arc::new(Mutex::new(Vec::new()));
    let observer: Arc<dyn Middleware> = Arc::new(RequestRecordingMiddleware {
        calls: Arc::clone(&observed),
    });
    let client = client.with_middleware(vec![observer]);
    let error = client
        .complete(Request {
            model: "plain".to_string(),
            provider: Some("missing".to_string()),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap_err();
    assert_eq!(
        error.kind,
        unified_llm_adapter::AdapterErrorKind::Configuration
    );
    assert!(error.message.contains("Unknown provider"));
    assert_eq!(call_log(&observed), Vec::<String>::new());
}

#[test]
fn client_from_env_map_registers_providers_in_fixed_order_and_selects_default() {
    let env = BTreeMap::from([
        ("ANTHROPIC_API_KEY".to_string(), "anthropic-key".to_string()),
        ("GEMINI_API_KEY".to_string(), "gemini-key".to_string()),
        (
            "LITELLM_BASE_URL".to_string(),
            "https://litellm.example".to_string(),
        ),
        ("OPENAI_API_KEY".to_string(), "openai-key".to_string()),
        (
            "OPENAI_COMPATIBLE_BASE_URL".to_string(),
            "https://compatible.example".to_string(),
        ),
        (
            "OPENROUTER_API_KEY".to_string(),
            "openrouter-key".to_string(),
        ),
    ]);

    let client = Client::from_env_map(&env, None).unwrap();
    assert_eq!(
        client.provider_names().collect::<Vec<_>>(),
        vec![
            "openai",
            "anthropic",
            "gemini",
            "openrouter",
            "litellm",
            "openai_compatible",
        ]
    );
    assert_eq!(client.default_provider(), Some("openai"));

    let explicit = Client::from_env_map(&env, Some("OpenRouter")).unwrap();
    assert_eq!(explicit.default_provider(), Some("openrouter"));

    let mut without_openai = env.clone();
    without_openai.remove("OPENAI_API_KEY");
    let without_openai = Client::from_env_map(&without_openai, None).unwrap();
    assert_eq!(
        without_openai.provider_names().collect::<Vec<_>>(),
        vec![
            "anthropic",
            "gemini",
            "openrouter",
            "litellm",
            "openai_compatible",
        ]
    );
    assert_eq!(without_openai.default_provider(), Some("anthropic"));

    let google_fallback = Client::from_env_map(
        &BTreeMap::from([("GOOGLE_API_KEY".to_string(), "google-key".to_string())]),
        None,
    )
    .unwrap();
    assert_eq!(
        google_fallback.provider_names().collect::<Vec<_>>(),
        vec!["gemini"]
    );
    assert_eq!(google_fallback.default_provider(), Some("gemini"));

    let error = match Client::from_env_map(&env, Some("missing")) {
        Ok(_) => panic!("missing default provider should fail"),
        Err(error) => error,
    };
    assert_eq!(
        error.kind,
        unified_llm_adapter::AdapterErrorKind::Configuration
    );
    assert!(error.message.contains("Unknown default provider"));
}

#[test]
fn client_errors_when_provider_cannot_resolve_without_model_inference() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingAdapter::new("openai", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], None).unwrap();

    let error = client
        .complete(Request {
            model: "openai:gpt-5.2".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap_err();

    assert_eq!(
        error.kind,
        unified_llm_adapter::AdapterErrorKind::Configuration
    );
    assert!(error.message.contains("No provider configured"));
    assert_eq!(call_log(&calls), vec!["initialize:openai"]);

    let adapter: Arc<dyn ProviderAdapter> = Arc::new(EchoAdapter);
    let error = match Client::from_adapters(vec![adapter], Some("missing")) {
        Ok(_) => panic!("missing default provider should fail"),
        Err(error) => error,
    };
    assert_eq!(
        error.kind,
        unified_llm_adapter::AdapterErrorKind::Configuration
    );
    assert!(error.message.contains("Unknown default provider"));
}

#[test]
fn client_is_send_sync_and_does_not_mutate_requests_across_concurrent_calls() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Client>();
    assert_send_sync::<Arc<dyn Middleware>>();

    let calls = Arc::new(Mutex::new(Vec::new()));
    let middleware_calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingAdapter::new("fake", Arc::clone(&calls)));
    let middleware: Arc<dyn Middleware> = Arc::new(RequestRecordingMiddleware {
        calls: Arc::clone(&middleware_calls),
    });
    let client = Arc::new(
        Client::from_adapters(vec![adapter], Some("FAKE"))
            .unwrap()
            .with_middleware(vec![middleware]),
    );

    let handles = (0..12)
        .map(|index| {
            let client = Arc::clone(&client);
            thread::spawn(move || {
                let request = Request {
                    model: format!("model-{index}"),
                    provider: (index % 2 == 0).then(|| "FAKE".to_string()),
                    messages: vec![Message::user("hello")],
                    ..Request::default()
                };
                let original_provider = request.provider.clone();

                if index % 3 == 0 {
                    let events = client
                        .stream(request.clone())
                        .unwrap()
                        .collect::<Result<Vec<_>, _>>()
                        .unwrap();
                    assert_eq!(events, vec![StreamEvent::text_delta("fake stream")]);
                } else {
                    let response = client.complete(request.clone()).unwrap();
                    assert_eq!(response.provider, "fake");
                    assert_eq!(response.text(), "fake complete");
                }

                assert_eq!(request.provider, original_provider);
            })
        })
        .collect::<Vec<_>>();

    for handle in handles {
        handle.join().expect("client worker thread");
    }

    let calls = call_log(&calls);
    assert_eq!(calls.len(), 13);
    assert!(calls.contains(&"initialize:fake".to_string()));
    for index in 0..12 {
        assert!(
            calls
                .iter()
                .any(|call| call.ends_with(&format!(":model-{index}"))),
            "missing call for model-{index}: {calls:?}"
        );
    }
    for call in calls.iter().filter(|call| !call.starts_with("initialize:")) {
        assert!(call.contains(":fake:model-"));
    }
    let middleware_calls = call_log(&middleware_calls);
    assert_eq!(middleware_calls.len(), 12);
    for index in 0..12 {
        assert!(
            middleware_calls
                .iter()
                .any(|call| call.ends_with(&format!(":fake:model-{index}"))),
            "missing middleware call for model-{index}: {middleware_calls:?}"
        );
    }
}

#[test]
fn module_default_client_override_replaces_lazy_env_backed_default() {
    let _default_client_guard = DEFAULT_CLIENT_TEST_LOCK
        .lock()
        .expect("default client test lock");
    let previous_default_client = get_default_client();
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(EchoAdapter);
    let override_client = Client::from_providers(
        BTreeMap::from([("fake".to_string(), adapter)]),
        Some("fake"),
    )
    .unwrap();

    set_default_client(Some(override_client));
    assert_eq!(get_default_client().default_provider(), Some("fake"));

    set_default_client(None);
    let lazy_client = get_default_client();
    assert_ne!(lazy_client.default_provider(), Some("fake"));
    assert_eq!(
        get_default_client().default_provider(),
        lazy_client.default_provider()
    );

    set_default_client(Some(previous_default_client));
}

#[test]
fn core_dtos_preserve_content_accessors_and_extensible_payloads() {
    let mixed = Message {
        role: Role::Assistant,
        content: vec![
            ContentPart::Text {
                text: "hello ".to_string(),
            },
            ContentPart::Thinking {
                thinking: ThinkingData {
                    text: "private plan".to_string(),
                    signature: Some("sig-1".to_string()),
                    redacted: false,
                    source_provider: Some("anthropic".to_string()),
                    source_model: Some("claude-sonnet-4-5".to_string()),
                },
            },
            ContentPart::Text {
                text: "world".to_string(),
            },
            ContentPart::ToolCall {
                tool_call: ToolCallData {
                    id: "call-1".to_string(),
                    name: "lookup".to_string(),
                    arguments: json!({"term": "rust"}),
                    raw_arguments: Some("{\"term\":\"rust\"}".to_string()),
                    r#type: "function".to_string(),
                },
            },
            ContentPart::Raw {
                kind: "anthropic_beta_block".to_string(),
                raw: json!({"provider": "anthropic", "payload": true}),
            },
        ],
        ..Message::default()
    };
    assert_eq!(mixed.text(), "hello world");
    assert_eq!(
        mixed.content[4].kind(),
        ContentKind::Custom("anthropic_beta_block".to_string())
    );
    mixed.validate().unwrap();

    let tool_message = Message::tool_result("call-1", json!({"ok": true}), false);
    assert_eq!(tool_message.role, Role::Tool);
    assert_eq!(tool_message.tool_call_id.as_deref(), Some("call-1"));
    tool_message.validate().unwrap();

    let response = Response {
        message: mixed,
        finish_reason: FinishReason::from_provider(FinishReasonKind::ToolCalls, "tool_use"),
        ..Response::default()
    };
    assert_eq!(response.text(), "hello world");
    assert_eq!(response.tool_calls().len(), 1);
    assert_eq!(response.reasoning().as_deref(), Some("private plan"));
    assert_eq!(response.finish_reason.raw.as_deref(), Some("tool_use"));

    let serialized = serde_json::to_value(ContentPart::Custom {
        kind: "provider/new_content".to_string(),
        raw: json!({"opaque": ["kept"]}),
    })
    .unwrap();
    assert_eq!(serialized["kind"], "provider/new_content");
    assert_eq!(serialized["raw"], json!({"opaque": ["kept"]}));
    assert_eq!(
        serde_json::from_value::<ContentPart>(serialized).unwrap(),
        ContentPart::Custom {
            kind: "provider/new_content".to_string(),
            raw: json!({"opaque": ["kept"]}),
        }
    );
}

#[test]
fn core_dto_validation_rejects_known_invalid_payload_boundaries() {
    let image = ContentPart::Image {
        image: ImageData {
            url: Some("https://example.test/image.png".to_string()),
            data: Some(vec![1, 2, 3]),
            media_type: Some("image/png".to_string()),
            detail: None,
        },
    };
    let image_error = Message {
        role: Role::User,
        content: vec![image],
        ..Message::default()
    }
    .validate()
    .unwrap_err();
    assert!(image_error.contains("exactly one of url or data"));

    let audio_error = Message {
        role: Role::Assistant,
        content: vec![ContentPart::Audio {
            audio: AudioData {
                url: Some("https://example.test/audio.wav".to_string()),
                data: None,
                media_type: Some("audio/wav".to_string()),
            },
        }],
        ..Message::default()
    }
    .validate()
    .unwrap_err();
    assert!(audio_error.contains("audio content is not allowed"));

    let document_error = Message {
        role: Role::Assistant,
        content: vec![ContentPart::Document {
            document: DocumentData {
                url: Some("https://example.test/file.pdf".to_string()),
                data: None,
                media_type: Some("application/pdf".to_string()),
                file_name: Some("file.pdf".to_string()),
            },
        }],
        ..Message::default()
    }
    .validate()
    .unwrap_err();
    assert!(document_error.contains("document content is not allowed"));
}

#[test]
fn low_level_client_rejects_empty_model_before_provider_call() {
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(EchoAdapter);
    let client =
        Client::from_providers(BTreeMap::from([("fake".to_string(), adapter)]), None).unwrap();

    let error = client
        .complete(Request {
            messages: vec![Message::user("hello")],
            provider: Some("fake".to_string()),
            ..Request::default()
        })
        .unwrap_err();

    assert_eq!(
        error.kind,
        unified_llm_adapter::AdapterErrorKind::InvalidRequest
    );
    assert!(error.message.contains("request model"));
}

#[test]
fn usage_addition_preserves_none_as_absent_for_optional_token_fields() {
    let empty_optional = Usage {
        input_tokens: 1,
        output_tokens: 2,
        total_tokens: 3,
        ..Usage::default()
    } + Usage {
        input_tokens: 4,
        output_tokens: 5,
        total_tokens: 9,
        ..Usage::default()
    };
    assert_eq!(empty_optional.reasoning_tokens, None);
    assert_eq!(empty_optional.cache_read_tokens, None);
    assert_eq!(empty_optional.cache_write_tokens, None);

    let populated_optional = empty_optional
        + Usage {
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            reasoning_tokens: Some(7),
            cache_read_tokens: Some(8),
            cache_write_tokens: Some(9),
            raw: Some(json!({"usage": "raw"})),
        };
    assert_eq!(populated_optional.reasoning_tokens, Some(7));
    assert_eq!(populated_optional.cache_read_tokens, Some(8));
    assert_eq!(populated_optional.cache_write_tokens, Some(9));
    assert_eq!(populated_optional.raw, Some(json!({"usage": "raw"})));
}

struct RoutingAdapter {
    name: &'static str,
    calls: Arc<Mutex<Vec<String>>>,
}

impl RoutingAdapter {
    fn new(name: &'static str, calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self { name, calls }
    }

    fn record(&self, call: impl Into<String>) {
        self.calls.lock().expect("call log lock").push(call.into());
    }
}

impl ProviderAdapter for RoutingAdapter {
    fn name(&self) -> &str {
        self.name
    }

    fn complete(&self, request: Request) -> Result<Response, unified_llm_adapter::AdapterError> {
        let provider = request.provider.unwrap_or_default();
        self.record(format!("adapter:complete:{provider}:{}", request.model));
        Ok(Response {
            model: request.model.clone(),
            provider: provider.clone(),
            message: Message::assistant(format!("{provider}:{}", request.model)),
            finish_reason: FinishReason::Stop,
            ..Response::default()
        })
    }

    fn stream(&self, request: Request) -> Result<StreamEvents, unified_llm_adapter::AdapterError> {
        let provider = request.provider.unwrap_or_default();
        self.record(format!("adapter:stream:{provider}:{}", request.model));
        Ok(stream_events(
            vec![Ok(StreamEvent::text_delta(format!(
                "{provider}:{}",
                request.model
            )))]
            .into_iter(),
        ))
    }
}

struct StreamErrorAdapter {
    name: &'static str,
    calls: Arc<Mutex<Vec<String>>>,
}

impl StreamErrorAdapter {
    fn new(name: &'static str, calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self { name, calls }
    }

    fn record(&self, call: impl Into<String>) {
        self.calls.lock().expect("call log lock").push(call.into());
    }
}

impl ProviderAdapter for StreamErrorAdapter {
    fn name(&self) -> &str {
        self.name
    }

    fn complete(&self, _request: Request) -> Result<Response, unified_llm_adapter::AdapterError> {
        Ok(Response::default())
    }

    fn stream(&self, request: Request) -> Result<StreamEvents, unified_llm_adapter::AdapterError> {
        let provider = request.provider.unwrap_or_default();
        self.record(format!("adapter:stream:{provider}:{}", request.model));
        Ok(stream_events(
            vec![
                Ok(StreamEvent::text_delta("seed")),
                Err(unified_llm_adapter::AdapterError::new(
                    unified_llm_adapter::AdapterErrorKind::Stream,
                    "boom",
                )),
            ]
            .into_iter(),
        ))
    }
}

struct CompleteTransformMiddleware {
    name: &'static str,
    calls: Arc<Mutex<Vec<String>>>,
}

impl CompleteTransformMiddleware {
    fn new(name: &'static str, calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self { name, calls }
    }

    fn record(&self, call: impl Into<String>) {
        self.calls.lock().expect("call log lock").push(call.into());
    }
}

impl Middleware for CompleteTransformMiddleware {
    fn complete(
        &self,
        request: Request,
        next: &unified_llm_adapter::CompleteNext<'_>,
    ) -> Result<Response, unified_llm_adapter::AdapterError> {
        self.record(format!(
            "enter:{}:{}:{}",
            self.name,
            request.provider.as_deref().unwrap_or_default(),
            request.model
        ));
        let mut request = request;
        request.model = format!("{}-{}", request.model, self.name);

        let mut response = next(request)?;
        self.record(format!("exit:{}:{}", self.name, response.text()));
        response.message = Message::assistant(format!("{}|{}", response.text(), self.name));
        Ok(response)
    }
}

struct ProviderOverrideMiddleware {
    provider: &'static str,
}

impl Middleware for ProviderOverrideMiddleware {
    fn complete(
        &self,
        mut request: Request,
        next: &unified_llm_adapter::CompleteNext<'_>,
    ) -> Result<Response, unified_llm_adapter::AdapterError> {
        request.provider = Some(self.provider.to_string());
        next(request)
    }
}

struct StreamTransformMiddleware {
    name: &'static str,
    calls: Arc<Mutex<Vec<String>>>,
}

impl StreamTransformMiddleware {
    fn new(name: &'static str, calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self { name, calls }
    }

    fn record(&self, call: impl Into<String>) {
        self.calls.lock().expect("call log lock").push(call.into());
    }
}

impl Middleware for StreamTransformMiddleware {
    fn stream(
        &self,
        request: Request,
        next: &unified_llm_adapter::StreamNext<'_>,
    ) -> Result<StreamEvents, unified_llm_adapter::AdapterError> {
        self.record(format!(
            "stream-enter:{}:{}:{}",
            self.name,
            request.provider.as_deref().unwrap_or_default(),
            request.model
        ));
        let stream = next(request)?;
        let calls = Arc::clone(&self.calls);
        let name = self.name;
        Ok(stream_events(stream.map(move |result| match result {
            Ok(mut event) => {
                let previous = event.delta.clone().unwrap_or_default();
                calls
                    .lock()
                    .expect("call log lock")
                    .push(format!("stream-event:{name}:{previous}"));
                if event.r#type == unified_llm_adapter::StreamEventType::TextDelta {
                    event.delta = Some(format!("{previous}|{name}"));
                }
                Ok(event)
            }
            Err(mut error) => {
                let previous = error.message.clone();
                calls
                    .lock()
                    .expect("call log lock")
                    .push(format!("stream-error:{name}:{previous}"));
                error.message = format!("{previous}|{name}");
                Err(error)
            }
        })))
    }
}

struct RequestRecordingMiddleware {
    calls: Arc<Mutex<Vec<String>>>,
}

impl RequestRecordingMiddleware {
    fn record(&self, phase: &str, request: &Request) {
        self.calls.lock().expect("call log lock").push(format!(
            "{phase}:{}:{}",
            request.provider.as_deref().unwrap_or_default(),
            request.model
        ));
    }
}

impl Middleware for RequestRecordingMiddleware {
    fn complete(
        &self,
        request: Request,
        next: &unified_llm_adapter::CompleteNext<'_>,
    ) -> Result<Response, unified_llm_adapter::AdapterError> {
        self.record("middleware-complete", &request);
        next(request)
    }

    fn stream(
        &self,
        request: Request,
        next: &unified_llm_adapter::StreamNext<'_>,
    ) -> Result<StreamEvents, unified_llm_adapter::AdapterError> {
        self.record("middleware-stream", &request);
        next(request)
    }
}

struct RetryableErrorAdapter {
    calls: Arc<Mutex<Vec<String>>>,
}

impl RetryableErrorAdapter {
    fn record(&self, call: impl Into<String>) {
        self.calls.lock().expect("call log lock").push(call.into());
    }
}

impl ProviderAdapter for RetryableErrorAdapter {
    fn name(&self) -> &str {
        "retryable"
    }

    fn complete(&self, request: Request) -> Result<Response, unified_llm_adapter::AdapterError> {
        self.record(format!(
            "complete:{}:{}",
            request.provider.as_deref().unwrap_or_default(),
            request.model
        ));
        Err(retryable_rate_limit_error("complete failed"))
    }

    fn stream(&self, request: Request) -> Result<StreamEvents, unified_llm_adapter::AdapterError> {
        self.record(format!(
            "stream:{}:{}",
            request.provider.as_deref().unwrap_or_default(),
            request.model
        ));
        Err(retryable_rate_limit_error("stream failed"))
    }
}

struct ScriptedCompleteAdapter {
    name: &'static str,
    calls: Arc<Mutex<Vec<String>>>,
    results: Mutex<VecDeque<Result<Response, unified_llm_adapter::AdapterError>>>,
}

impl ScriptedCompleteAdapter {
    fn new(
        name: &'static str,
        calls: Arc<Mutex<Vec<String>>>,
        results: Vec<Result<Response, unified_llm_adapter::AdapterError>>,
    ) -> Self {
        Self {
            name,
            calls,
            results: Mutex::new(VecDeque::from(results)),
        }
    }

    fn record(&self, call: impl Into<String>) {
        self.calls.lock().expect("call log lock").push(call.into());
    }
}

impl ProviderAdapter for ScriptedCompleteAdapter {
    fn name(&self) -> &str {
        self.name
    }

    fn complete(&self, request: Request) -> Result<Response, unified_llm_adapter::AdapterError> {
        self.record(format!(
            "complete:{}:{}:{}",
            request.provider.as_deref().unwrap_or_default(),
            request.model,
            request.messages.len()
        ));
        let result = self
            .results
            .lock()
            .expect("scripted result lock")
            .pop_front()
            .unwrap_or_else(|| {
                Err(unified_llm_adapter::AdapterError::new(
                    unified_llm_adapter::AdapterErrorKind::Configuration,
                    "no scripted complete response",
                ))
            });
        result.map(|mut response| {
            if response.provider.is_empty() {
                response.provider = request.provider.unwrap_or_default();
            }
            if response.model.is_empty() {
                response.model = request.model;
            }
            response
        })
    }

    fn stream(&self, _request: Request) -> Result<StreamEvents, unified_llm_adapter::AdapterError> {
        Err(unified_llm_adapter::AdapterError::new(
            unified_llm_adapter::AdapterErrorKind::Configuration,
            "scripted adapter does not implement stream",
        ))
    }
}

fn empty_request() -> Request {
    Request {
        model: String::new(),
        provider: None,
        messages: Vec::new(),
        tools: Vec::new(),
        tool_choice: None,
        response_format: None,
        temperature: None,
        top_p: None,
        max_tokens: None,
        stop_sequences: Vec::new(),
        reasoning_effort: None,
        metadata: BTreeMap::new(),
        provider_options: BTreeMap::new(),
    }
}

struct EchoAdapter;

impl ProviderAdapter for EchoAdapter {
    fn name(&self) -> &str {
        "fake"
    }

    fn complete(&self, request: Request) -> Result<Response, unified_llm_adapter::AdapterError> {
        Ok(Response {
            message: Message::assistant(format!(
                "{}:{}",
                request.provider.unwrap_or_default(),
                request.model
            )),
            finish_reason: FinishReason::Stop,
            ..Response::default()
        })
    }

    fn stream(&self, _request: Request) -> Result<StreamEvents, unified_llm_adapter::AdapterError> {
        Ok(stream_events(
            vec![Ok(StreamEvent::text_delta("fake-stream"))].into_iter(),
        ))
    }

    fn supports_tool_choice(&self, mode: &str) -> bool {
        mode == "auto"
    }
}

struct RecordingAdapter {
    name: &'static str,
    calls: Arc<Mutex<Vec<String>>>,
}

impl RecordingAdapter {
    fn new(name: &'static str, calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self { name, calls }
    }

    fn record(&self, call: impl Into<String>) {
        self.calls.lock().expect("call log lock").push(call.into());
    }
}

impl ProviderAdapter for RecordingAdapter {
    fn name(&self) -> &str {
        self.name
    }

    fn initialize(&self) -> Result<(), unified_llm_adapter::AdapterError> {
        self.record(format!("initialize:{}", self.name));
        Ok(())
    }

    fn complete(&self, request: Request) -> Result<Response, unified_llm_adapter::AdapterError> {
        self.record(format!(
            "complete:{}:{}",
            request.provider.as_deref().unwrap_or_default(),
            request.model
        ));
        Ok(Response {
            model: request.model,
            provider: request.provider.unwrap_or_default(),
            message: Message::assistant(format!("{} complete", self.name)),
            finish_reason: FinishReason::Stop,
            ..Response::default()
        })
    }

    fn stream(&self, request: Request) -> Result<StreamEvents, unified_llm_adapter::AdapterError> {
        self.record(format!(
            "stream:{}:{}",
            request.provider.as_deref().unwrap_or_default(),
            request.model
        ));
        Ok(stream_events(
            vec![Ok(StreamEvent::text_delta(format!("{} stream", self.name)))].into_iter(),
        ))
    }

    fn close(&self) -> Result<(), unified_llm_adapter::AdapterError> {
        self.record(format!("close:{}", self.name));
        Ok(())
    }

    fn supports_tool_choice(&self, mode: &str) -> bool {
        self.record(format!("supports_tool_choice:{}:{mode}", self.name));
        mode == "auto"
    }
}

fn call_log(calls: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    calls.lock().expect("call log lock").clone()
}

fn retryable_rate_limit_error(message: impl Into<String>) -> AdapterError {
    AdapterError::new(AdapterErrorKind::RateLimit, message)
}

fn object_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "required": ["answer"],
        "properties": {
            "answer": {"type": "string"}
        },
        "additionalProperties": false
    })
}
