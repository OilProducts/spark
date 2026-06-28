use std::any::TypeId;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use serde::ser::{Error as SerdeError, Serialize, Serializer};
use serde_json::{json, Value};
use unified_llm_adapter::{
    classify_grpc_code, classify_http_status_code, classify_provider_error_message,
    error_from_grpc_code, error_from_status_code, generate, generate_object,
    generate_object_with_policy_and_hooks, generate_steps_with_policy_and_hooks,
    generate_with_policy_and_hooks, get_default_client, get_latest_model, managed_stream,
    parse_sse_stream, resolve_high_level_provider_and_model, retry_after_from_headers,
    retry_stream_before_first_event_with_hooks, retry_with_hooks, set_default_client, stream,
    stream_events, stream_object, stream_object_with_policy_and_hooks,
    stream_with_policy_and_hooks, AbortController, ActiveLlmProfile, AdapterError,
    AdapterErrorKind, AdapterTimeout, AudioData, Client, ContentKind, ContentPart, DocumentData,
    FinishReason, FinishReasonKind, GenerateRequest, HighLevelLlmResolutionInputs, ImageData,
    LlmRequest, LlmResponse, Message, MessageRole, Middleware, ModelCapabilities,
    NativeRequestConfig, OpenAICompatibleRequestConfig, ProviderAdapter, ProviderError,
    RateLimitInfo, Request, Response, ResponseFormat, RetryPolicy, Role, SDKError, SDKErrorKind,
    SseParser, StopWhen, StreamAccumulator, StreamEvent, StreamEventType, StreamEvents,
    ThinkingData, TimeoutConfig, Tool, ToolCall, ToolCallData, ToolChoice, ToolInvocation,
    ToolRepair, ToolRepairInvocation, Usage, Warning, DEFAULT_CONNECT_TIMEOUT_SECONDS,
    DEFAULT_REQUEST_TIMEOUT_SECONDS, DEFAULT_STREAM_READ_TIMEOUT_SECONDS,
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
        tools: vec![Tool::passive("lookup").unwrap()],
        tool_choice: Some(ToolChoice::auto()),
        response_format: Some(ResponseFormat::JsonObject),
        temperature: Some(0.2),
        top_p: Some(0.9),
        max_tokens: Some(1024),
        stop_sequences: vec!["END".to_string()],
        reasoning_effort: Some("medium".to_string()),
        metadata: BTreeMap::from([("run_id".to_string(), json!("run-1"))]),
        provider_options: BTreeMap::from([("openai".to_string(), json!({"trace": true}))]),
        timeout: None,
        abort_signal: None,
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
    let abandoned_closes = Arc::new(Mutex::new(0));
    let mut stream = retry_stream_before_first_event_with_hooks(
        RetryPolicy {
            max_retries: 1,
            jitter: false,
            ..RetryPolicy::default()
        },
        {
            let opens = Arc::clone(&opens);
            let abandoned_closes = Arc::clone(&abandoned_closes);
            move || {
                let mut opens = opens.lock().expect("open count");
                *opens += 1;
                if *opens == 1 {
                    let abandoned_closes = Arc::clone(&abandoned_closes);
                    Ok(managed_stream(
                        vec![Err(retryable_rate_limit_error("first event failed"))].into_iter(),
                        move || {
                            *abandoned_closes.lock().expect("abandoned close count") += 1;
                            Ok(())
                        },
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
    assert_eq!(*abandoned_closes.lock().expect("abandoned close count"), 1);

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
fn low_level_client_rejects_malformed_named_tool_choice_but_defers_unsupported_modes() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingAdapter::new("default", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], Some("default")).unwrap();

    let error = client
        .complete(Request {
            model: "tool-model".to_string(),
            messages: vec![Message::user("hello")],
            tools: vec![Tool::passive("lookup").unwrap()],
            tool_choice: Some(ToolChoice {
                mode: "named".to_string(),
                tool_name: None,
            }),
            ..Request::default()
        })
        .unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::InvalidRequest);
    assert!(error
        .message
        .contains("named tool_choice requires tool_name"));
    assert_eq!(call_log(&calls), vec!["initialize:default"]);

    let response = client
        .complete(Request {
            model: "tool-model".to_string(),
            messages: vec![Message::user("hello")],
            tool_choice: Some(ToolChoice {
                mode: "provider_native".to_string(),
                tool_name: None,
            }),
            ..Request::default()
        })
        .unwrap();

    assert_eq!(response.text(), "default complete");
    assert_eq!(
        call_log(&calls),
        vec!["initialize:default", "complete:default:tool-model"]
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
fn high_level_generate_does_not_report_request_history_tool_results_or_aggregate_steps() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "high",
        Arc::clone(&calls),
        vec![
            Ok(Response {
                message: Message::tool_result("generated_step_1", json!({"ok": true}), false),
                finish_reason: FinishReason::Stop,
                ..Response::default()
            }),
            Ok(Response {
                message: Message::assistant("final"),
                finish_reason: FinishReason::Stop,
                ..Response::default()
            }),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("high")).unwrap();

    let result = generate_steps_with_policy_and_hooks(
        &client,
        Request {
            model: "step-1".to_string(),
            messages: vec![
                Message::user("continue"),
                Message::tool_result("historical", json!({"from": "caller"}), false),
            ],
            ..Request::default()
        },
        &RetryPolicy::default(),
        |steps| {
            if steps.len() == 1 {
                return Ok(Some(Request {
                    model: "step-2".to_string(),
                    messages: vec![
                        Message::user("continue"),
                        Message::tool_result("historical", json!({"from": "caller"}), false),
                        Message::tool_result("generated_step_1", json!({"ok": true}), false),
                    ],
                    ..Request::default()
                }));
            }
            Ok(None)
        },
        || 1.0,
        |_| {},
    )
    .unwrap();

    assert_eq!(
        result.steps[0].tool_results,
        vec![unified_llm_adapter::ToolResult::success(
            "generated_step_1",
            json!({"ok": true})
        )]
    );
    assert!(result.steps[1].tool_results.is_empty());
    assert!(result.tool_results.is_empty());
    assert_eq!(result.text, "final");
    assert_eq!(
        call_log(&calls),
        vec!["complete:high:step-1:2", "complete:high:step-2:3"]
    );
}

#[test]
fn high_level_generate_honors_zero_retry_budget_and_retry_after_delay() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut retry_after_error = retryable_rate_limit_error("retry after");
    retry_after_error.retry_after = Some(12.5);
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "high",
        Arc::clone(&calls),
        vec![
            Err(retry_after_error),
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
        max_delay: 30.0,
        jitter: false,
        ..RetryPolicy::default()
    };
    let mut sleep_durations = Vec::new();

    let result = generate_with_policy_and_hooks(
        &client,
        Request {
            model: "retry-after-model".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        },
        &policy,
        || 1.0,
        |delay| sleep_durations.push(delay),
    )
    .unwrap();

    assert_eq!(result.text, "done");
    assert_eq!(sleep_durations, vec![12.5]);
    assert_eq!(
        call_log(&calls),
        vec![
            "complete:high:retry-after-model:1",
            "complete:high:retry-after-model:1",
        ]
    );

    let zero_calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "high",
        Arc::clone(&zero_calls),
        vec![
            Err(retryable_rate_limit_error("first error stays visible")),
            Ok(Response {
                message: Message::assistant("late success"),
                finish_reason: FinishReason::Stop,
                ..Response::default()
            }),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("high")).unwrap();
    let zero_policy = RetryPolicy {
        max_retries: 0,
        jitter: false,
        ..RetryPolicy::default()
    };

    let error = generate_with_policy_and_hooks(
        &client,
        Request {
            model: "zero-retry-model".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        },
        &zero_policy,
        || 1.0,
        |_| panic!("max_retries=0 must not sleep"),
    )
    .unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::RateLimit);
    assert_eq!(error.message, "first error stays visible");
    assert_eq!(
        call_log(&zero_calls),
        vec!["complete:high:zero-retry-model:1"]
    );
}

#[test]
fn high_level_generate_normalizes_prompt_system_and_projects_result_fields() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(HighLevelBoundaryAdapter::new("openai", Arc::clone(&calls)));
    let middleware_calls = Arc::new(Mutex::new(Vec::new()));
    let middleware: Arc<dyn Middleware> = Arc::new(RequestRecordingMiddleware {
        calls: Arc::clone(&middleware_calls),
    });
    let client = Client::from_adapters(vec![adapter], Some("openai"))
        .unwrap()
        .with_middleware(vec![middleware]);

    let result = generate(
        &client,
        GenerateRequest {
            prompt: Some("hello".to_string()),
            system: Some("answer tersely".to_string()),
            model: Some("explicit-model".to_string()),
            tools: vec![Tool::passive("lookup").unwrap()],
            ..GenerateRequest::default()
        },
    )
    .unwrap();

    assert_eq!(result.text, "boundary text");
    assert_eq!(result.reasoning.as_deref(), Some("boundary reasoning"));
    assert_eq!(result.tool_calls.len(), 1);
    assert!(result.tool_results.is_empty());
    assert_eq!(result.finish_reason, FinishReason::Stop);
    assert_eq!(result.usage.input_tokens, 3);
    assert_eq!(result.total_usage.total_tokens, 8);
    assert_eq!(result.warnings[0].code.as_deref(), Some("boundary_warning"));
    assert_eq!(result.output, None);
    assert_eq!(result.steps.len(), 1);
    assert_eq!(result.steps[0].text, "boundary text");
    assert_eq!(
        result.steps[0]
            .request
            .messages
            .iter()
            .map(|message| (message.role, message.text()))
            .collect::<Vec<_>>(),
        vec![
            (MessageRole::System, "answer tersely".to_string()),
            (MessageRole::User, "hello".to_string()),
        ]
    );
    assert_eq!(
        call_log(&middleware_calls),
        vec!["middleware-complete:openai:explicit-model"]
    );
    assert_eq!(
        call_log(&calls),
        vec!["complete:openai:explicit-model:system=answer tersely|user=hello",]
    );
}

#[test]
fn high_level_generate_prepends_system_to_messages_without_reordering_history() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(HighLevelBoundaryAdapter::new("openai", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], Some("openai")).unwrap();

    let result = generate(
        &client,
        GenerateRequest {
            messages: Some(vec![
                Message::user("first"),
                Message::assistant("second"),
                Message::user("third"),
            ]),
            system: Some("system first".to_string()),
            model: Some("ordered-model".to_string()),
            ..GenerateRequest::default()
        },
    )
    .unwrap();

    assert_eq!(
        result.steps[0]
            .request
            .messages
            .iter()
            .map(|message| (message.role, message.text()))
            .collect::<Vec<_>>(),
        vec![
            (MessageRole::System, "system first".to_string()),
            (MessageRole::User, "first".to_string()),
            (MessageRole::Assistant, "second".to_string()),
            (MessageRole::User, "third".to_string()),
        ]
    );
    assert_eq!(
        call_log(&calls),
        vec![
            "complete:openai:ordered-model:system=system first|user=first|assistant=second|user=third"
        ]
    );
}

#[test]
fn high_level_generate_executes_active_tools_with_invocation_contract() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let invocation_log = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "high",
        Arc::clone(&calls),
        vec![
            Ok(Response {
                message: Message {
                    role: MessageRole::Assistant,
                    content: vec![
                        ContentPart::Text {
                            text: "checking".to_string(),
                        },
                        ContentPart::ToolCall {
                            tool_call: ToolCall::new(
                                "call_weather",
                                "weather",
                                json!({"city": "Paris"}),
                            ),
                        },
                        ContentPart::ToolCall {
                            tool_call: ToolCall::new(
                                "call_time",
                                "local_time",
                                json!({"city": "Paris"}),
                            ),
                        },
                        ContentPart::ToolCall {
                            tool_call: ToolCall::new("call_echo", "echo", json!({})),
                        },
                    ],
                    ..Message::default()
                },
                finish_reason: FinishReason::ToolCalls,
                ..Response::default()
            }),
            Ok(Response {
                message: Message::assistant("final answer"),
                finish_reason: FinishReason::Stop,
                ..Response::default()
            }),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("high")).unwrap();

    let weather_log = Arc::clone(&invocation_log);
    let weather = Tool::active_with_schema(
        "weather",
        Some("Lookup weather".to_string()),
        Some(json!({
            "type": "object",
            "properties": {"city": {"type": "string"}},
        })),
        move |invocation: ToolInvocation| {
            weather_log.lock().expect("invocation log").push((
                invocation.tool_call_id.clone(),
                invocation.arguments.clone(),
                invocation.messages.clone(),
            ));
            Ok(vec!["sunny", "72F"])
        },
    )
    .unwrap();
    let local_time = Tool::active("local_time", |_invocation: ToolInvocation| {
        Ok(BTreeMap::from([(
            "timezone".to_string(),
            "Europe/Paris".to_string(),
        )]))
    })
    .unwrap();
    let echo = Tool::active("echo", |invocation: ToolInvocation| {
        Ok(format!("echo:{}", invocation.tool_call_id))
    })
    .unwrap();

    let result = generate(
        &client,
        Request {
            model: "tool-model".to_string(),
            messages: vec![Message::user("weather and time?")],
            tools: vec![weather, local_time, echo],
            ..Request::default()
        },
    )
    .unwrap();

    assert_eq!(result.text, "final answer");
    assert_eq!(result.steps.len(), 2);
    assert_eq!(
        result.steps[0].tool_results,
        vec![
            unified_llm_adapter::ToolResult::success("call_weather", json!(["sunny", "72F"]),),
            unified_llm_adapter::ToolResult::success(
                "call_time",
                json!({"timezone": "Europe/Paris"}),
            ),
            unified_llm_adapter::ToolResult::success("call_echo", json!("echo:call_echo")),
        ]
    );
    assert!(result.tool_results.is_empty());
    assert_eq!(
        call_log(&calls),
        vec!["complete:high:tool-model:1", "complete:high:tool-model:5"]
    );

    let invocation_log = invocation_log.lock().expect("invocation log").clone();
    assert_eq!(invocation_log.len(), 1);
    assert_eq!(invocation_log[0].0, "call_weather");
    assert_eq!(invocation_log[0].1, json!({"city": "Paris"}));
    assert_eq!(
        invocation_log[0].2,
        vec![Message::user("weather and time?")]
    );
}

#[test]
fn high_level_generate_with_active_tools_retries_continuation_without_reexecuting_tool_batch() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "high",
        Arc::clone(&calls),
        vec![
            Ok(Response {
                message: Message {
                    role: MessageRole::Assistant,
                    content: vec![
                        ContentPart::Text {
                            text: "checking".to_string(),
                        },
                        ContentPart::ToolCall {
                            tool_call: ToolCall::new(
                                "call_weather",
                                "weather",
                                json!({"city": "Paris"}),
                            ),
                        },
                        ContentPart::ToolCall {
                            tool_call: ToolCall::new(
                                "call_time",
                                "local_time",
                                json!({"city": "Paris"}),
                            ),
                        },
                    ],
                    ..Message::default()
                },
                finish_reason: FinishReason::ToolCalls,
                ..Response::default()
            }),
            Err(retryable_rate_limit_error("retry continuation only")),
            Ok(Response {
                message: Message::assistant("final after retry"),
                finish_reason: FinishReason::Stop,
                ..Response::default()
            }),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("high")).unwrap();
    let weather_executions = Arc::new(Mutex::new(0usize));
    let time_executions = Arc::new(Mutex::new(0usize));
    let invocation_messages = Arc::new(Mutex::new(Vec::new()));
    let weather = Tool::active("weather", {
        let weather_executions = Arc::clone(&weather_executions);
        let invocation_messages = Arc::clone(&invocation_messages);
        move |invocation: ToolInvocation| {
            *weather_executions.lock().expect("weather executions") += 1;
            invocation_messages
                .lock()
                .expect("invocation messages")
                .push((invocation.tool_call_id.clone(), invocation.messages.len()));
            Ok(json!({"forecast": "sunny"}))
        }
    })
    .unwrap();
    let local_time = Tool::active("local_time", {
        let time_executions = Arc::clone(&time_executions);
        let invocation_messages = Arc::clone(&invocation_messages);
        move |invocation: ToolInvocation| {
            *time_executions.lock().expect("time executions") += 1;
            invocation_messages
                .lock()
                .expect("invocation messages")
                .push((invocation.tool_call_id.clone(), invocation.messages.len()));
            Ok(json!({"timezone": "Europe/Paris"}))
        }
    })
    .unwrap();
    let policy = RetryPolicy {
        max_retries: 1,
        jitter: false,
        ..RetryPolicy::default()
    };
    let mut sleep_durations = Vec::new();

    let result = generate_with_policy_and_hooks(
        &client,
        Request {
            model: "tool-model".to_string(),
            messages: vec![Message::user("weather and time?")],
            tools: vec![weather, local_time],
            ..Request::default()
        },
        &policy,
        || 1.0,
        |delay| sleep_durations.push(delay),
    )
    .unwrap();

    assert_eq!(result.text, "final after retry");
    assert_eq!(result.steps.len(), 2);
    assert_eq!(*weather_executions.lock().expect("weather executions"), 1);
    assert_eq!(*time_executions.lock().expect("time executions"), 1);
    assert_eq!(sleep_durations, vec![1.0]);
    assert_eq!(
        result.steps[0].tool_results,
        vec![
            unified_llm_adapter::ToolResult::success("call_weather", json!({"forecast": "sunny"}),),
            unified_llm_adapter::ToolResult::success(
                "call_time",
                json!({"timezone": "Europe/Paris"}),
            ),
        ]
    );
    assert!(result.steps[1].tool_results.is_empty());
    assert_eq!(
        call_log(&calls),
        vec![
            "complete:high:tool-model:1",
            "complete:high:tool-model:4",
            "complete:high:tool-model:4",
        ]
    );
    let mut invocation_messages = invocation_messages
        .lock()
        .expect("invocation messages")
        .clone();
    invocation_messages.sort();
    assert_eq!(
        invocation_messages,
        vec![
            ("call_time".to_string(), 1),
            ("call_weather".to_string(), 1),
        ]
    );
}

#[test]
fn high_level_generate_turns_active_tool_handler_errors_into_tool_results() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "high",
        Arc::clone(&calls),
        vec![
            Ok(Response {
                message: Message {
                    role: MessageRole::Assistant,
                    content: vec![ContentPart::ToolCall {
                        tool_call: ToolCall::new("call_boom", "boom", json!({})),
                    }],
                    ..Message::default()
                },
                finish_reason: FinishReason::ToolCalls,
                ..Response::default()
            }),
            Ok(Response {
                message: Message::assistant("recovered"),
                finish_reason: FinishReason::Stop,
                ..Response::default()
            }),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("high")).unwrap();
    let boom = Tool::active(
        "boom",
        |_invocation: ToolInvocation| -> Result<Value, AdapterError> {
            Err(AdapterError::new(
                AdapterErrorKind::InvalidToolCall,
                "handler failed",
            ))
        },
    )
    .unwrap();

    let result = generate(
        &client,
        Request {
            model: "tool-model".to_string(),
            messages: vec![Message::user("run tool")],
            tools: vec![boom],
            ..Request::default()
        },
    )
    .unwrap();

    assert_eq!(result.text, "recovered");
    assert_eq!(
        result.steps[0].tool_results,
        vec![unified_llm_adapter::ToolResult::error(
            "call_boom",
            json!("handler failed"),
        )]
    );
    assert!(result.steps[0].tool_results[0].is_error);
    assert_eq!(
        call_log(&calls),
        vec!["complete:high:tool-model:1", "complete:high:tool-model:3"]
    );
}

#[test]
fn high_level_generate_turns_non_serializable_tool_returns_into_error_results() {
    struct FailingSerialize;

    impl Serialize for FailingSerialize {
        fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            Err(S::Error::custom("not JSON serializable"))
        }
    }

    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "high",
        Arc::clone(&calls),
        vec![
            Ok(Response {
                message: Message {
                    role: MessageRole::Assistant,
                    content: vec![ContentPart::ToolCall {
                        tool_call: ToolCall::new("call_bad", "bad_return", json!({})),
                    }],
                    ..Message::default()
                },
                finish_reason: FinishReason::ToolCalls,
                ..Response::default()
            }),
            Ok(Response {
                message: Message::assistant("continued"),
                finish_reason: FinishReason::Stop,
                ..Response::default()
            }),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("high")).unwrap();
    let bad_return = Tool::active("bad_return", |_invocation: ToolInvocation| {
        Ok(FailingSerialize)
    })
    .unwrap();

    let result = generate(
        &client,
        Request {
            model: "tool-model".to_string(),
            messages: vec![Message::user("run tool")],
            tools: vec![bad_return],
            ..Request::default()
        },
    )
    .unwrap();

    assert_eq!(result.text, "continued");
    assert_eq!(result.steps[0].tool_results.len(), 1);
    let tool_result = &result.steps[0].tool_results[0];
    assert_eq!(tool_result.tool_call_id, "call_bad");
    assert!(tool_result.is_error);
    assert!(tool_result
        .content
        .as_str()
        .is_some_and(|message| message.contains("tool handler returned non-serializable content")));
    assert_eq!(
        call_log(&calls),
        vec!["complete:high:tool-model:1", "complete:high:tool-model:3"]
    );
}

#[test]
fn high_level_generate_keeps_successful_siblings_when_argument_parsing_or_validation_fails() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "high",
        Arc::clone(&calls),
        vec![
            Ok(Response {
                message: Message {
                    role: MessageRole::Assistant,
                    content: vec![
                        ContentPart::ToolCall {
                            tool_call: ToolCall::from_raw_arguments(
                                "call_ok",
                                "weather",
                                "{\"city\":\"Paris\"}",
                            ),
                        },
                        ContentPart::ToolCall {
                            tool_call: ToolCall::from_raw_arguments(
                                "call_parse",
                                "weather",
                                "{not json",
                            ),
                        },
                        ContentPart::ToolCall {
                            tool_call: ToolCall::from_raw_arguments(
                                "call_schema",
                                "weather",
                                "{\"city\":7}",
                            ),
                        },
                    ],
                    ..Message::default()
                },
                finish_reason: FinishReason::ToolCalls,
                ..Response::default()
            }),
            Ok(Response {
                message: Message::assistant("continued"),
                finish_reason: FinishReason::Stop,
                ..Response::default()
            }),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("high")).unwrap();
    let executions = Arc::new(Mutex::new(Vec::new()));
    let execution_log = Arc::clone(&executions);
    let weather = Tool::active_with_schema(
        "weather",
        Some("Lookup weather".to_string()),
        Some(json!({
            "type": "object",
            "required": ["city"],
            "properties": {"city": {"type": "string"}},
        })),
        move |invocation: ToolInvocation| {
            execution_log
                .lock()
                .expect("execution log")
                .push(invocation.tool_call_id.clone());
            Ok(invocation.arguments)
        },
    )
    .unwrap();

    let result = generate(
        &client,
        Request {
            model: "tool-model".to_string(),
            messages: vec![Message::user("run tools")],
            tools: vec![weather],
            ..Request::default()
        },
    )
    .unwrap();

    assert_eq!(result.text, "continued");
    assert_eq!(
        result.steps[0].tool_results[0],
        unified_llm_adapter::ToolResult::success("call_ok", json!({"city": "Paris"}))
    );
    assert_eq!(result.steps[0].tool_results[1].tool_call_id, "call_parse");
    assert!(result.steps[0].tool_results[1].is_error);
    assert!(result.steps[0].tool_results[1]
        .content
        .as_str()
        .is_some_and(|message| message.contains("Invalid JSON arguments")));
    assert_eq!(result.steps[0].tool_results[2].tool_call_id, "call_schema");
    assert!(result.steps[0].tool_results[2].is_error);
    let schema_error = result.steps[0].tool_results[2]
        .content
        .as_str()
        .expect("schema validation failure should be reported as text");
    assert!(schema_error.contains("Invalid arguments for tool 'weather'"));
    assert!(schema_error.contains("string"));
    assert_eq!(
        executions.lock().expect("execution log").clone(),
        vec!["call_ok".to_string()]
    );
    assert_eq!(
        call_log(&calls),
        vec!["complete:high:tool-model:1", "complete:high:tool-model:5"]
    );
}

#[test]
fn high_level_generate_repairs_invalid_tool_arguments_before_ordered_batched_continuation() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "high",
        Arc::clone(&calls),
        vec![
            Ok(Response {
                message: Message {
                    role: MessageRole::Assistant,
                    content: vec![
                        ContentPart::ToolCall {
                            tool_call: ToolCall::from_raw_arguments(
                                "call_weather",
                                "weather",
                                "{not json",
                            ),
                        },
                        ContentPart::ToolCall {
                            tool_call: ToolCall::from_raw_arguments(
                                "call_time",
                                "local_time",
                                "{\"city\":\"Paris\"}",
                            ),
                        },
                    ],
                    ..Message::default()
                },
                finish_reason: FinishReason::ToolCalls,
                ..Response::default()
            }),
            Ok(Response {
                message: Message::assistant("continued after repair"),
                finish_reason: FinishReason::Stop,
                ..Response::default()
            }),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("high")).unwrap();
    let repair_observations = Arc::new(Mutex::new(Vec::new()));
    let repair_log = Arc::clone(&repair_observations);
    let repair_tool_call = ToolRepair::new(move |invocation: ToolRepairInvocation| {
        repair_log.lock().expect("repair log").push((
            invocation.tool_call_id.clone(),
            invocation.tool_definition.name.clone(),
            invocation.messages.len(),
            invocation.validation_error.clone(),
        ));
        Ok(json!({"city": "Paris"}))
    });
    let weather = Tool::active_with_schema(
        "weather",
        Some("Lookup weather".to_string()),
        Some(json!({
            "type": "object",
            "required": ["city"],
            "properties": {"city": {"type": "string"}},
        })),
        |invocation: ToolInvocation| Ok(json!({"weather_city": invocation.arguments["city"]})),
    )
    .unwrap();
    let local_time = Tool::active_with_schema(
        "local_time",
        Some("Lookup time".to_string()),
        Some(json!({
            "type": "object",
            "required": ["city"],
            "properties": {"city": {"type": "string"}},
        })),
        |invocation: ToolInvocation| Ok(json!({"time_city": invocation.arguments["city"]})),
    )
    .unwrap();

    let result = generate(
        &client,
        GenerateRequest {
            model: Some("tool-model".to_string()),
            messages: Some(vec![Message::user("run repaired tools")]),
            tools: vec![weather, local_time],
            repair_tool_call: Some(repair_tool_call),
            ..GenerateRequest::default()
        },
    )
    .unwrap();

    assert_eq!(result.text, "continued after repair");
    assert_eq!(
        result.steps[0].tool_results,
        vec![
            unified_llm_adapter::ToolResult::success(
                "call_weather",
                json!({"weather_city": "Paris"}),
            ),
            unified_llm_adapter::ToolResult::success("call_time", json!({"time_city": "Paris"}),),
        ]
    );
    let repairs = repair_observations.lock().expect("repair log").clone();
    assert_eq!(repairs.len(), 1);
    assert_eq!(repairs[0].0, "call_weather");
    assert_eq!(repairs[0].1, "weather");
    assert_eq!(repairs[0].2, 1);
    assert!(repairs[0].3.contains("Invalid JSON arguments"));
    assert_eq!(
        call_log(&calls),
        vec!["complete:high:tool-model:1", "complete:high:tool-model:4"]
    );
}

#[test]
fn high_level_generate_returns_passive_tool_calls_without_auto_execution() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "high",
        Arc::clone(&calls),
        vec![Ok(Response {
            message: Message {
                role: MessageRole::Assistant,
                content: vec![ContentPart::ToolCall {
                    tool_call: ToolCall::new("call_lookup", "lookup", json!({"query": "Paris"})),
                }],
                ..Message::default()
            },
            finish_reason: FinishReason::ToolCalls,
            ..Response::default()
        })],
    ));
    let client = Client::from_adapters(vec![adapter], Some("high")).unwrap();

    let result = generate(
        &client,
        Request {
            model: "tool-model".to_string(),
            messages: vec![Message::user("lookup")],
            tools: vec![Tool::passive("lookup").unwrap()],
            ..Request::default()
        },
    )
    .unwrap();

    assert_eq!(result.steps.len(), 1);
    assert_eq!(result.tool_calls.len(), 1);
    assert_eq!(result.tool_calls[0].name, "lookup");
    assert!(result.tool_results.is_empty());
    assert_eq!(call_log(&calls), vec!["complete:high:tool-model:1"]);
}

#[test]
fn high_level_generate_executes_active_tool_calls_concurrently_and_batches_ordered_results() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "high",
        Arc::clone(&calls),
        vec![
            Ok(Response {
                message: Message {
                    role: MessageRole::Assistant,
                    content: vec![
                        ContentPart::ToolCall {
                            tool_call: ToolCall::new("call_slow", "slow", json!({})),
                        },
                        ContentPart::ToolCall {
                            tool_call: ToolCall::new("call_fast", "fast", json!({})),
                        },
                    ],
                    ..Message::default()
                },
                finish_reason: FinishReason::ToolCalls,
                ..Response::default()
            }),
            Ok(Response {
                message: Message::assistant("done"),
                finish_reason: FinishReason::Stop,
                ..Response::default()
            }),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("high")).unwrap();
    let rendezvous = Arc::new((Mutex::new(0usize), Condvar::new()));

    let slow_rendezvous = Arc::clone(&rendezvous);
    let slow = Tool::active("slow", move |_invocation: ToolInvocation| {
        let (lock, cvar) = &*slow_rendezvous;
        let mut started = lock.lock().expect("rendezvous lock");
        *started += 1;
        cvar.notify_all();
        let (started, _) = cvar
            .wait_timeout_while(started, Duration::from_secs(2), |started| *started < 2)
            .expect("rendezvous wait");
        Ok(json!({"tool": "slow", "saw_parallel_peer": *started >= 2}))
    })
    .unwrap();
    let fast_rendezvous = Arc::clone(&rendezvous);
    let fast = Tool::active("fast", move |_invocation: ToolInvocation| {
        let (lock, cvar) = &*fast_rendezvous;
        let mut started = lock.lock().expect("rendezvous lock");
        *started += 1;
        cvar.notify_all();
        let (started, _) = cvar
            .wait_timeout_while(started, Duration::from_secs(2), |started| *started < 2)
            .expect("rendezvous wait");
        Ok(json!({"tool": "fast", "saw_parallel_peer": *started >= 2}))
    })
    .unwrap();

    let result = generate(
        &client,
        Request {
            model: "tool-model".to_string(),
            messages: vec![Message::user("run both")],
            tools: vec![slow, fast],
            ..Request::default()
        },
    )
    .unwrap();

    assert_eq!(result.text, "done");
    assert_eq!(
        result.steps[0].tool_results,
        vec![
            unified_llm_adapter::ToolResult::success(
                "call_slow",
                json!({"tool": "slow", "saw_parallel_peer": true}),
            ),
            unified_llm_adapter::ToolResult::success(
                "call_fast",
                json!({"tool": "fast", "saw_parallel_peer": true}),
            ),
        ]
    );
    assert_eq!(
        call_log(&calls),
        vec!["complete:high:tool-model:1", "complete:high:tool-model:4"]
    );
}

#[test]
fn high_level_generate_converts_unknown_tool_calls_to_error_results_and_continues() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "high",
        Arc::clone(&calls),
        vec![
            Ok(Response {
                message: Message {
                    role: MessageRole::Assistant,
                    content: vec![ContentPart::ToolCall {
                        tool_call: ToolCall::new("call_missing", "missing", json!({})),
                    }],
                    ..Message::default()
                },
                finish_reason: FinishReason::ToolCalls,
                ..Response::default()
            }),
            Ok(Response {
                message: Message::assistant("continued after tool error"),
                finish_reason: FinishReason::Stop,
                ..Response::default()
            }),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("high")).unwrap();

    let result = generate(
        &client,
        Request {
            model: "tool-model".to_string(),
            messages: vec![Message::user("run missing")],
            ..Request::default()
        },
    )
    .unwrap();

    assert_eq!(result.text, "continued after tool error");
    assert_eq!(
        result.steps[0].tool_results,
        vec![unified_llm_adapter::ToolResult::error(
            "call_missing",
            json!("Unknown tool 'missing'"),
        )]
    );
    assert_eq!(
        call_log(&calls),
        vec!["complete:high:tool-model:1", "complete:high:tool-model:3"]
    );
}

#[test]
fn high_level_generate_respects_max_tool_rounds_zero_and_multiple_rounds() {
    let zero_calls = Arc::new(Mutex::new(Vec::new()));
    let zero_adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "high",
        Arc::clone(&zero_calls),
        vec![Ok(Response {
            message: Message {
                role: MessageRole::Assistant,
                content: vec![ContentPart::ToolCall {
                    tool_call: ToolCall::new("call_lookup", "lookup", json!({})),
                }],
                ..Message::default()
            },
            finish_reason: FinishReason::ToolCalls,
            ..Response::default()
        })],
    ));
    let zero_client = Client::from_adapters(vec![zero_adapter], Some("high")).unwrap();
    let zero_executions = Arc::new(Mutex::new(0usize));
    let zero_execution_log = Arc::clone(&zero_executions);
    let zero_tool = Tool::active("lookup", move |_invocation: ToolInvocation| {
        *zero_execution_log.lock().expect("execution lock") += 1;
        Ok("should not run")
    })
    .unwrap();

    let zero_result = generate(
        &zero_client,
        GenerateRequest {
            model: Some("tool-model".to_string()),
            messages: Some(vec![Message::user("lookup")]),
            tools: vec![zero_tool],
            max_tool_rounds: 0,
            ..GenerateRequest::default()
        },
    )
    .unwrap();

    assert_eq!(zero_result.steps.len(), 1);
    assert_eq!(zero_result.tool_calls.len(), 1);
    assert!(zero_result.tool_results.is_empty());
    assert_eq!(*zero_executions.lock().expect("execution lock"), 0);
    assert_eq!(call_log(&zero_calls), vec!["complete:high:tool-model:1"]);

    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "high",
        Arc::clone(&calls),
        vec![
            Ok(Response {
                message: Message {
                    role: MessageRole::Assistant,
                    content: vec![ContentPart::ToolCall {
                        tool_call: ToolCall::new("call_first", "lookup", json!({})),
                    }],
                    ..Message::default()
                },
                finish_reason: FinishReason::ToolCalls,
                ..Response::default()
            }),
            Ok(Response {
                message: Message {
                    role: MessageRole::Assistant,
                    content: vec![ContentPart::ToolCall {
                        tool_call: ToolCall::new("call_second", "lookup", json!({})),
                    }],
                    ..Message::default()
                },
                finish_reason: FinishReason::ToolCalls,
                ..Response::default()
            }),
            Ok(Response {
                message: Message::assistant("done after two rounds"),
                finish_reason: FinishReason::Stop,
                ..Response::default()
            }),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("high")).unwrap();
    let tool = Tool::active("lookup", |invocation: ToolInvocation| {
        Ok(invocation.tool_call_id)
    })
    .unwrap();

    let result = generate(
        &client,
        GenerateRequest {
            model: Some("tool-model".to_string()),
            messages: Some(vec![Message::user("lookup twice")]),
            tools: vec![tool],
            max_tool_rounds: 2,
            ..GenerateRequest::default()
        },
    )
    .unwrap();

    assert_eq!(result.text, "done after two rounds");
    assert_eq!(result.steps.len(), 3);
    assert_eq!(
        result.steps[0].tool_results,
        vec![unified_llm_adapter::ToolResult::success(
            "call_first",
            json!("call_first"),
        )]
    );
    assert_eq!(
        result.steps[1].tool_results,
        vec![unified_llm_adapter::ToolResult::success(
            "call_second",
            json!("call_second"),
        )]
    );
    assert_eq!(
        call_log(&calls),
        vec![
            "complete:high:tool-model:1",
            "complete:high:tool-model:3",
            "complete:high:tool-model:5",
        ]
    );
}

#[test]
fn high_level_generate_respects_stop_when_after_recording_tool_results() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "high",
        Arc::clone(&calls),
        vec![
            Ok(Response {
                message: Message {
                    role: MessageRole::Assistant,
                    content: vec![ContentPart::ToolCall {
                        tool_call: ToolCall::new("call_lookup", "lookup", json!({})),
                    }],
                    ..Message::default()
                },
                finish_reason: FinishReason::ToolCalls,
                ..Response::default()
            }),
            Ok(Response {
                message: Message::assistant("should not be requested"),
                finish_reason: FinishReason::Stop,
                ..Response::default()
            }),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("high")).unwrap();
    let stop_observations = Arc::new(Mutex::new(Vec::new()));
    let stop_log = Arc::clone(&stop_observations);
    let tool = Tool::active("lookup", |_invocation: ToolInvocation| Ok("tool result")).unwrap();

    let result = generate(
        &client,
        GenerateRequest {
            model: Some("tool-model".to_string()),
            messages: Some(vec![Message::user("lookup")]),
            tools: vec![tool],
            max_tool_rounds: 2,
            stop_when: Some(StopWhen::new(move |steps| {
                stop_log.lock().expect("stop log").push(steps.len());
                steps.len() == 1
            })),
            ..GenerateRequest::default()
        },
    )
    .unwrap();

    assert_eq!(result.steps.len(), 1);
    assert_eq!(
        result.tool_results,
        vec![unified_llm_adapter::ToolResult::success(
            "call_lookup",
            json!("tool result"),
        )]
    );
    assert_eq!(*stop_observations.lock().expect("stop log"), vec![1]);
    assert_eq!(call_log(&calls), vec!["complete:high:tool-model:1"]);
}

#[test]
fn high_level_generate_and_stream_reject_prompt_and_messages_before_client_call() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(HighLevelBoundaryAdapter::new("openai", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], Some("openai")).unwrap();

    let error = generate(
        &client,
        GenerateRequest {
            prompt: Some("hello".to_string()),
            messages: Some(vec![Message::user("hello")]),
            model: Some("gpt-5.2".to_string()),
            ..GenerateRequest::default()
        },
    )
    .unwrap_err();
    let stream_error = match stream(
        &client,
        GenerateRequest {
            prompt: Some("hello".to_string()),
            messages: Some(vec![Message::user("hello")]),
            model: Some("gpt-5.2".to_string()),
            ..GenerateRequest::default()
        },
    ) {
        Ok(_) => panic!("stream should reject prompt and messages together"),
        Err(error) => error,
    };

    assert_eq!(error.kind, AdapterErrorKind::InvalidRequest);
    assert_eq!(stream_error.kind, AdapterErrorKind::InvalidRequest);
    assert!(error.message.contains("either prompt or messages"));
    assert!(stream_error.message.contains("either prompt or messages"));
    assert!(call_log(&calls).is_empty());
}

#[test]
fn high_level_generate_and_stream_require_prompt_or_messages_before_client_call() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(HighLevelBoundaryAdapter::new("openai", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], Some("openai")).unwrap();

    let generate_error = generate(
        &client,
        GenerateRequest {
            model: Some("gpt-5.2".to_string()),
            ..GenerateRequest::default()
        },
    )
    .unwrap_err();
    let stream_error = match stream(
        &client,
        GenerateRequest {
            model: Some("gpt-5.2".to_string()),
            ..GenerateRequest::default()
        },
    ) {
        Ok(_) => panic!("stream should reject missing prompt/messages"),
        Err(error) => error,
    };

    assert_eq!(generate_error.kind, AdapterErrorKind::InvalidRequest);
    assert_eq!(stream_error.kind, AdapterErrorKind::InvalidRequest);
    assert!(generate_error
        .message
        .contains("either prompt or messages must be provided"));
    assert!(stream_error
        .message
        .contains("either prompt or messages must be provided"));
    assert!(call_log(&calls).is_empty());
}

#[test]
fn high_level_model_resolution_handles_profiles_capabilities_and_compatible_omissions() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingAdapter::new("openai", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], Some("openai")).unwrap();

    let defaulted = generate(
        &client,
        GenerateRequest {
            prompt: Some("hello".to_string()),
            ..GenerateRequest::default()
        },
    )
    .unwrap();
    assert_eq!(
        defaulted.steps[0].request.provider.as_deref(),
        Some("openai")
    );
    assert_eq!(
        defaulted.steps[0].request.model,
        get_latest_model("openai", None).unwrap().id
    );

    let reasoning_defaulted = generate(
        &client,
        GenerateRequest {
            prompt: Some("think briefly".to_string()),
            reasoning_effort: Some("medium".to_string()),
            ..GenerateRequest::default()
        },
    )
    .unwrap();
    assert_eq!(
        reasoning_defaulted.steps[0].request.model,
        get_latest_model("openai", Some("reasoning")).unwrap().id
    );

    let combined_capabilities =
        ModelCapabilities::reasoning().union(ModelCapabilities::structured_output());
    assert!(combined_capabilities.reasoning);
    assert!(combined_capabilities.structured_output);
    let resolved = resolve_high_level_provider_and_model(&HighLevelLlmResolutionInputs {
        provider: Some("openai".to_string()),
        required_capabilities: combined_capabilities,
        ..HighLevelLlmResolutionInputs::default()
    })
    .unwrap();
    assert_eq!(
        resolved.model,
        get_latest_model("openai", Some("reasoning")).unwrap().id
    );

    let explicit_unknown = generate(
        &client,
        GenerateRequest {
            prompt: Some("hello".to_string()),
            model: Some("not-in-catalog-2026-06-27".to_string()),
            ..GenerateRequest::default()
        },
    )
    .unwrap();
    assert_eq!(
        explicit_unknown.steps[0].request.model,
        "not-in-catalog-2026-06-27"
    );

    let explicit_provider_ignores_other_profile_default =
        resolve_high_level_provider_and_model(&HighLevelLlmResolutionInputs {
            provider: Some("openai".to_string()),
            active_profile: Some(ActiveLlmProfile::new(
                "openai_compatible",
                Some("local-large".to_string()),
            )),
            ..HighLevelLlmResolutionInputs::default()
        })
        .unwrap();
    assert_eq!(
        explicit_provider_ignores_other_profile_default.provider,
        "openai"
    );
    assert_eq!(
        explicit_provider_ignores_other_profile_default.model,
        get_latest_model("openai", None).unwrap().id
    );

    let compatible_with_other_profile_default =
        resolve_high_level_provider_and_model(&HighLevelLlmResolutionInputs {
            provider: Some("openrouter".to_string()),
            active_profile: Some(ActiveLlmProfile::new("openai", Some("gpt-5.2".to_string()))),
            ..HighLevelLlmResolutionInputs::default()
        })
        .unwrap_err();
    assert_eq!(
        compatible_with_other_profile_default.kind,
        AdapterErrorKind::Configuration
    );
    assert!(compatible_with_other_profile_default
        .message
        .contains("No model configured"));

    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(RecordingAdapter::new(
        "openai_compatible",
        Arc::clone(&calls),
    ));
    let client = Client::from_adapters(vec![adapter], None).unwrap();
    let profiled = generate(
        &client,
        GenerateRequest {
            prompt: Some("hello".to_string()),
            active_profile: Some(ActiveLlmProfile::new(
                "openai_compatible",
                Some("local-large".to_string()),
            )),
            ..GenerateRequest::default()
        },
    )
    .unwrap();
    assert_eq!(
        profiled.steps[0].request.provider.as_deref(),
        Some("openai_compatible")
    );
    assert_eq!(profiled.steps[0].request.model, "local-large");

    for provider in ["openrouter", "litellm", "openai_compatible"] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let adapter: Arc<dyn ProviderAdapter> =
            Arc::new(HighLevelBoundaryAdapter::new(provider, Arc::clone(&calls)));
        let client = Client::from_adapters(vec![adapter], Some(provider)).unwrap();
        let error = generate(
            &client,
            GenerateRequest {
                prompt: Some("hello".to_string()),
                ..GenerateRequest::default()
            },
        )
        .unwrap_err();
        assert_eq!(error.kind, AdapterErrorKind::Configuration);
        assert!(error.message.contains("No model configured"));
        assert!(call_log(&calls).is_empty());
    }

    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingAdapter::new("gemini", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], Some("gemini")).unwrap();
    let vision = generate(
        &client,
        GenerateRequest {
            messages: Some(vec![Message {
                role: MessageRole::User,
                content: vec![ContentPart::Image {
                    image: ImageData::url("https://example.test/image.png"),
                }],
                ..Message::default()
            }]),
            ..GenerateRequest::default()
        },
    )
    .unwrap();
    assert_eq!(
        vision.steps[0].request.model,
        get_latest_model("gemini", Some("vision")).unwrap().id
    );

    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingAdapter::new("anthropic", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], Some("anthropic")).unwrap();
    let structured = generate(
        &client,
        GenerateRequest {
            prompt: Some("json please".to_string()),
            response_format: Some(ResponseFormat::JsonObject),
            ..GenerateRequest::default()
        },
    )
    .unwrap();
    assert_eq!(
        structured.steps[0].request.model,
        get_latest_model("anthropic", None).unwrap().id
    );
}

#[test]
fn high_level_stream_routes_through_middleware_and_reconstructs_response() {
    fn assert_async_stream<T: futures_core::Stream<Item = Result<StreamEvent, AdapterError>>>() {}
    assert_async_stream::<unified_llm_adapter::StreamResult>();

    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(HighLevelBoundaryAdapter::new("openai", Arc::clone(&calls)));
    let middleware_calls = Arc::new(Mutex::new(Vec::new()));
    let middleware: Arc<dyn Middleware> = Arc::new(RequestRecordingMiddleware {
        calls: Arc::clone(&middleware_calls),
    });
    let client = Client::from_adapters(vec![adapter], Some("openai"))
        .unwrap()
        .with_middleware(vec![middleware]);

    let mut result = stream(
        &client,
        GenerateRequest {
            prompt: Some("stream please".to_string()),
            model: Some("stream-model".to_string()),
            ..GenerateRequest::default()
        },
    )
    .unwrap();
    assert_eq!(
        result.next().unwrap().unwrap(),
        StreamEvent::text_delta("hel")
    );
    assert_eq!(result.partial_response().text(), "hel");
    let response = result.response().unwrap();
    assert_eq!(response.text(), "hello");
    assert_eq!(response.finish_reason, FinishReason::Stop);

    let mut text_result = stream(
        &client,
        GenerateRequest {
            prompt: Some("stream please".to_string()),
            model: Some("stream-model".to_string()),
            ..GenerateRequest::default()
        },
    )
    .unwrap();
    let chunks = text_result
        .text_stream()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(chunks, vec!["hel".to_string(), "lo".to_string()]);
    assert_eq!(text_result.response().unwrap().text(), "hello");

    assert_eq!(
        call_log(&middleware_calls),
        vec![
            "middleware-stream:openai:stream-model",
            "middleware-stream:openai:stream-model",
        ]
    );
    assert_eq!(
        call_log(&calls),
        vec![
            "stream:openai:stream-model:user=stream please",
            "stream:openai:stream-model:user=stream please",
        ]
    );
}

#[test]
fn high_level_stream_continues_active_tool_loop_with_step_finish_boundary() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter = Arc::new(ScriptedStreamAdapter::new(
        "streaming",
        Arc::clone(&calls),
        vec![
            Ok(vec![
                Ok(StreamEvent::text_delta("Need tools")),
                Ok(tool_call_stream_event(
                    StreamEventType::ToolCallEnd,
                    ToolCall::from_raw_arguments("call_weather", "weather", "{\"city\":\"Paris\"}"),
                )),
                Ok(tool_call_stream_event(
                    StreamEventType::ToolCallEnd,
                    ToolCall::from_raw_arguments("call_time", "local_time", "{\"city\":\"Paris\"}"),
                )),
                Ok(StreamEvent::finish(FinishReason::ToolCalls, None)),
            ]),
            Ok(vec![
                Ok(StreamEvent::text_delta("final answer")),
                Ok(StreamEvent::finish(FinishReason::Stop, None)),
            ]),
        ],
    ));
    let request_log = adapter.requests();
    let adapter: Arc<dyn ProviderAdapter> = adapter;
    let client = Client::from_adapters(vec![adapter], Some("streaming")).unwrap();
    let weather = Tool::active_with_schema(
        "weather",
        Some("Lookup weather".to_string()),
        Some(json!({
            "type": "object",
            "required": ["city"],
            "properties": {"city": {"type": "string"}},
        })),
        |invocation: ToolInvocation| Ok(json!({"weather_city": invocation.arguments["city"]})),
    )
    .unwrap();
    let local_time = Tool::active("local_time", |invocation: ToolInvocation| {
        Ok(json!({"time_city": invocation.arguments["city"]}))
    })
    .unwrap();

    let mut result = stream(
        &client,
        GenerateRequest {
            prompt: Some("what should I do?".to_string()),
            model: Some("stream-model".to_string()),
            tools: vec![weather, local_time],
            ..GenerateRequest::default()
        },
    )
    .unwrap();

    let events = result.by_ref().collect::<Result<Vec<_>, _>>().unwrap();
    let response = result.response().unwrap();

    assert_eq!(
        events
            .iter()
            .map(|event| event.r#type.clone())
            .collect::<Vec<_>>(),
        vec![
            StreamEventType::TextDelta,
            StreamEventType::ToolCallEnd,
            StreamEventType::ToolCallEnd,
            StreamEventType::Finish,
            StreamEventType::Custom("step_finish".to_string()),
            StreamEventType::TextDelta,
            StreamEventType::Finish,
        ]
    );
    assert_eq!(
        events[4].response.as_ref().expect("step response").text(),
        "Need tools"
    );
    assert_eq!(
        events[4]
            .finish_reason
            .as_ref()
            .expect("step finish reason")
            .reason,
        FinishReasonKind::ToolCalls
    );
    assert_eq!(response.text(), "final answer");
    assert_eq!(result.partial_response().text(), "final answer");
    assert_eq!(
        call_log(&calls),
        vec![
            "stream:streaming:stream-model:1",
            "stream:streaming:stream-model:4",
        ]
    );

    let requests = request_log.lock().expect("request log").clone();
    assert!(requests[0]
        .tool_choice
        .as_ref()
        .is_some_and(ToolChoice::is_auto));
    assert_eq!(
        requests[1]
            .messages
            .iter()
            .map(|message| message.role)
            .collect::<Vec<_>>(),
        vec![
            MessageRole::User,
            MessageRole::Assistant,
            MessageRole::Tool,
            MessageRole::Tool,
        ]
    );
    assert_eq!(
        requests[1].messages[2].tool_call_id.as_deref(),
        Some("call_weather")
    );
    assert_eq!(
        requests[1].messages[3].tool_call_id.as_deref(),
        Some("call_time")
    );
}

#[test]
fn high_level_stream_retries_only_before_first_delivered_event() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedStreamAdapter::new(
        "streaming",
        Arc::clone(&calls),
        vec![
            Err(retryable_rate_limit_error("open failed")),
            Ok(vec![Ok(StreamEvent::text_delta("after retry"))]),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("streaming")).unwrap();
    let policy = RetryPolicy {
        max_retries: 2,
        jitter: false,
        ..RetryPolicy::default()
    };
    let sleep_durations = Arc::new(Mutex::new(Vec::new()));

    let mut result = stream_with_policy_and_hooks(
        &client,
        Request {
            model: "stream-model".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        },
        &policy,
        || 1.0,
        {
            let sleep_durations = Arc::clone(&sleep_durations);
            move |delay| sleep_durations.lock().expect("sleep durations").push(delay)
        },
    )
    .unwrap();

    let events = result.by_ref().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(events, vec![StreamEvent::text_delta("after retry")]);
    assert_eq!(result.response().unwrap().text(), "after retry");
    assert_eq!(
        sleep_durations.lock().expect("sleep durations").clone(),
        vec![1.0]
    );
    assert_eq!(
        call_log(&calls),
        vec![
            "stream:streaming:stream-model:1",
            "stream:streaming:stream-model:1",
        ]
    );

    let first_event_error_calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedStreamAdapter::new(
        "streaming",
        Arc::clone(&first_event_error_calls),
        vec![
            Ok(vec![Err(retryable_rate_limit_error("first event failed"))]),
            Ok(vec![Ok(StreamEvent::text_delta("late success"))]),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("streaming")).unwrap();
    let first_event_sleeps = Arc::new(Mutex::new(Vec::new()));
    let mut result = stream_with_policy_and_hooks(
        &client,
        Request {
            model: "stream-model".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        },
        &policy,
        || 1.0,
        {
            let first_event_sleeps = Arc::clone(&first_event_sleeps);
            move |delay| {
                first_event_sleeps
                    .lock()
                    .expect("first event sleeps")
                    .push(delay)
            }
        },
    )
    .unwrap();

    let events = result.by_ref().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(events, vec![StreamEvent::text_delta("late success")]);
    assert_eq!(
        first_event_sleeps
            .lock()
            .expect("first event sleeps")
            .clone(),
        vec![1.0]
    );
    assert_eq!(
        call_log(&first_event_error_calls),
        vec![
            "stream:streaming:stream-model:1",
            "stream:streaming:stream-model:1",
        ]
    );

    let zero_calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedStreamAdapter::new(
        "streaming",
        Arc::clone(&zero_calls),
        vec![
            Err(retryable_rate_limit_error("zero retry stream open")),
            Ok(vec![Ok(StreamEvent::text_delta("late success"))]),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("streaming")).unwrap();
    let zero_policy = RetryPolicy {
        max_retries: 0,
        jitter: false,
        ..RetryPolicy::default()
    };
    let error = match stream_with_policy_and_hooks(
        &client,
        Request {
            model: "stream-model".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        },
        &zero_policy,
        || 1.0,
        |_| panic!("max_retries=0 must not sleep"),
    ) {
        Ok(_) => panic!("max_retries=0 must surface the opening error"),
        Err(error) => error,
    };
    assert_eq!(error.kind, AdapterErrorKind::RateLimit);
    assert_eq!(error.message, "zero retry stream open");
    assert_eq!(
        call_log(&zero_calls),
        vec!["stream:streaming:stream-model:1"]
    );

    let partial_calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedStreamAdapter::new(
        "streaming",
        Arc::clone(&partial_calls),
        vec![Ok(vec![
            Ok(StreamEvent::text_delta("partial")),
            Err(retryable_rate_limit_error("after partial")),
        ])],
    ));
    let client = Client::from_adapters(vec![adapter], Some("streaming")).unwrap();
    let partial_sleeps = Arc::new(Mutex::new(Vec::new()));
    let mut result = stream_with_policy_and_hooks(
        &client,
        Request {
            model: "stream-model".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        },
        &policy,
        || 1.0,
        {
            let partial_sleeps = Arc::clone(&partial_sleeps);
            move |delay| partial_sleeps.lock().expect("partial sleeps").push(delay)
        },
    )
    .unwrap();

    assert_eq!(
        result.next().expect("partial event").unwrap(),
        StreamEvent::text_delta("partial")
    );
    let error = result
        .next()
        .expect("post-partial stream error")
        .unwrap_err();
    assert_eq!(error.kind, AdapterErrorKind::RateLimit);
    assert_eq!(error.message, "after partial");
    assert!(result.next().is_none());
    assert!(partial_sleeps.lock().expect("partial sleeps").is_empty());
    assert_eq!(
        call_log(&partial_calls),
        vec!["stream:streaming:stream-model:1"]
    );
}

#[test]
fn high_level_stream_closes_provider_resources_on_close_drop_and_terminal_error() {
    let explicit_close_calls = Arc::new(Mutex::new(0));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ClosableStreamAdapter::new(
        "streaming",
        Arc::clone(&explicit_close_calls),
        vec![
            Ok(StreamEvent::text_delta("partial")),
            Ok(StreamEvent::text_delta("unused")),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("streaming")).unwrap();
    let mut result = stream(
        &client,
        GenerateRequest {
            prompt: Some("hello".to_string()),
            model: Some("stream-model".to_string()),
            ..GenerateRequest::default()
        },
    )
    .unwrap();
    assert_eq!(
        result.next().expect("partial event").unwrap(),
        StreamEvent::text_delta("partial")
    );
    result.close().unwrap();
    assert!(result.next().is_none());
    result.close().unwrap();
    assert_eq!(*explicit_close_calls.lock().expect("close count"), 1);

    let drop_close_calls = Arc::new(Mutex::new(0));
    {
        let adapter: Arc<dyn ProviderAdapter> = Arc::new(ClosableStreamAdapter::new(
            "streaming",
            Arc::clone(&drop_close_calls),
            vec![
                Ok(StreamEvent::text_delta("partial")),
                Ok(StreamEvent::text_delta("unused")),
            ],
        ));
        let client = Client::from_adapters(vec![adapter], Some("streaming")).unwrap();
        let mut result = stream(
            &client,
            GenerateRequest {
                prompt: Some("hello".to_string()),
                model: Some("stream-model".to_string()),
                ..GenerateRequest::default()
            },
        )
        .unwrap();
        assert_eq!(
            result.next().expect("partial event").unwrap(),
            StreamEvent::text_delta("partial")
        );
    }
    assert_eq!(*drop_close_calls.lock().expect("drop close count"), 1);

    let error_close_calls = Arc::new(Mutex::new(0));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ClosableStreamAdapter::new(
        "streaming",
        Arc::clone(&error_close_calls),
        vec![
            Ok(StreamEvent::text_delta("partial")),
            Err(AdapterError::new(AdapterErrorKind::Stream, "stream broke")),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("streaming")).unwrap();
    let mut result = stream(
        &client,
        GenerateRequest {
            prompt: Some("hello".to_string()),
            model: Some("stream-model".to_string()),
            ..GenerateRequest::default()
        },
    )
    .unwrap();
    assert_eq!(
        result.next().expect("partial event").unwrap(),
        StreamEvent::text_delta("partial")
    );
    let error = result.next().expect("terminal stream error").unwrap_err();
    assert_eq!(error.kind, AdapterErrorKind::Stream);
    assert_eq!(error.message, "stream broke");
    assert_eq!(*error_close_calls.lock().expect("error close count"), 1);
    assert_eq!(result.response().unwrap_err().message, "stream broke");
}

#[test]
fn high_level_generate_checks_abort_before_and_between_llm_steps() {
    let pre_aborted_calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(HighLevelBoundaryAdapter::new(
        "high",
        Arc::clone(&pre_aborted_calls),
    ));
    let client = Client::from_adapters(vec![adapter], Some("high")).unwrap();
    let controller = AbortController::new();
    controller.abort("caller cancelled");

    let error = generate(
        &client,
        GenerateRequest {
            prompt: Some("hello".to_string()),
            model: Some("abort-model".to_string()),
            abort_signal: Some(controller.signal()),
            ..GenerateRequest::default()
        },
    )
    .unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::Abort);
    assert_eq!(error.message, "caller cancelled");
    assert!(call_log(&pre_aborted_calls).is_empty());

    let between_step_calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "high",
        Arc::clone(&between_step_calls),
        vec![
            Ok(Response {
                message: Message::assistant("first"),
                finish_reason: FinishReason::Stop,
                ..Response::default()
            }),
            Ok(Response {
                message: Message::assistant("second should not run"),
                finish_reason: FinishReason::Stop,
                ..Response::default()
            }),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("high")).unwrap();
    let controller = AbortController::new();
    let signal = controller.signal();
    let error = generate_steps_with_policy_and_hooks(
        &client,
        Request {
            model: "step-1".to_string(),
            messages: vec![Message::user("hello")],
            abort_signal: Some(signal),
            ..Request::default()
        },
        &RetryPolicy::default(),
        |steps| {
            if steps.len() == 1 {
                controller.abort("stop before continuation");
                return Ok(Some(Request {
                    model: "step-2".to_string(),
                    messages: vec![Message::user("continue")],
                    ..Request::default()
                }));
            }
            Ok(None)
        },
        || 1.0,
        |_| {},
    )
    .unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::Abort);
    assert_eq!(error.message, "stop before continuation");
    assert_eq!(
        call_log(&between_step_calls),
        vec!["complete:high:step-1:1"]
    );
}

#[test]
fn high_level_stream_abort_closes_provider_resources_once() {
    let close_calls = Arc::new(Mutex::new(0));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ClosableStreamAdapter::new(
        "streaming",
        Arc::clone(&close_calls),
        vec![
            Ok(StreamEvent::text_delta("partial")),
            Ok(StreamEvent::text_delta("late")),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("streaming")).unwrap();
    let controller = AbortController::new();
    let mut result = stream(
        &client,
        GenerateRequest {
            prompt: Some("hello".to_string()),
            model: Some("stream-model".to_string()),
            abort_signal: Some(controller.signal()),
            ..GenerateRequest::default()
        },
    )
    .unwrap();

    assert_eq!(
        result.next().expect("first event").unwrap(),
        StreamEvent::text_delta("partial")
    );
    controller.abort("stop stream");
    let error = result.next().expect("abort event").unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::Abort);
    assert_eq!(error.message, "stop stream");
    assert_eq!(*close_calls.lock().expect("close count"), 1);
    assert!(result.next().is_none());
    assert_eq!(result.response().unwrap_err().message, "stop stream");
    result.close().unwrap();
    assert_eq!(*close_calls.lock().expect("close count"), 1);
}

#[test]
fn total_timeout_fails_high_level_generation_and_stream_without_provider_calls() {
    let generate_calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(HighLevelBoundaryAdapter::new(
        "timed",
        Arc::clone(&generate_calls),
    ));
    let client = Client::from_adapters(vec![adapter], Some("timed")).unwrap();

    let error = generate(
        &client,
        GenerateRequest {
            prompt: Some("hello".to_string()),
            model: Some("timed-model".to_string()),
            timeout: Some(TimeoutConfig::total(0.0)),
            ..GenerateRequest::default()
        },
    )
    .unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::RequestTimeout);
    assert!(error.message.contains("generation timed out"));
    assert!(call_log(&generate_calls).is_empty());

    let stream_calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(HighLevelBoundaryAdapter::new(
        "timed",
        Arc::clone(&stream_calls),
    ));
    let client = Client::from_adapters(vec![adapter], Some("timed")).unwrap();
    let stream_error = match stream(
        &client,
        GenerateRequest {
            prompt: Some("hello".to_string()),
            model: Some("timed-model".to_string()),
            timeout: Some(TimeoutConfig::total(0.0)),
            ..GenerateRequest::default()
        },
    ) {
        Ok(_) => panic!("total timeout should fail before stream open"),
        Err(error) => error,
    };

    assert_eq!(stream_error.kind, AdapterErrorKind::RequestTimeout);
    assert!(stream_error.message.contains("stream timed out"));
    assert!(call_log(&stream_calls).is_empty());
}

#[test]
fn timeout_config_applies_to_each_llm_step_and_retry_policy_can_opt_in() {
    let timeout = TimeoutConfig {
        total: Some(30.0),
        per_step: Some(5.0),
        stream_read: Some(2.0),
    };
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(TimeoutRecordingAdapter::new(
        "timed",
        Arc::clone(&calls),
        vec![
            Ok(Response {
                message: Message::assistant("first"),
                finish_reason: FinishReason::Stop,
                ..Response::default()
            }),
            Ok(Response {
                message: Message::assistant("done"),
                finish_reason: FinishReason::Stop,
                ..Response::default()
            }),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("timed")).unwrap();

    let result = generate_steps_with_policy_and_hooks(
        &client,
        Request {
            model: "step-1".to_string(),
            messages: vec![Message::user("hello")],
            timeout: Some(timeout),
            ..Request::default()
        },
        &RetryPolicy::default(),
        |steps| {
            if steps.len() == 1 {
                return Ok(Some(Request {
                    model: "step-2".to_string(),
                    messages: vec![Message::user("continue")],
                    ..Request::default()
                }));
            }
            Ok(None)
        },
        || 1.0,
        |_| {},
    )
    .unwrap();

    assert_eq!(result.text, "done");
    assert_eq!(
        calls.lock().expect("timeout calls").clone(),
        vec![
            ("step-1".to_string(), Some(timeout)),
            ("step-2".to_string(), Some(timeout)),
        ]
    );

    let zero_timeout_calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(TimeoutRecordingAdapter::new(
        "timed",
        Arc::clone(&zero_timeout_calls),
        vec![Ok(Response {
            message: Message::assistant("should not run"),
            finish_reason: FinishReason::Stop,
            ..Response::default()
        })],
    ));
    let client = Client::from_adapters(vec![adapter], Some("timed")).unwrap();
    let error = generate(
        &client,
        GenerateRequest {
            prompt: Some("hello".to_string()),
            model: Some("timed-model".to_string()),
            timeout: Some(TimeoutConfig::per_step(0.0)),
            ..GenerateRequest::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.kind, AdapterErrorKind::RequestTimeout);
    assert!(error.message.contains("generation step timed out"));
    assert!(zero_timeout_calls
        .lock()
        .expect("zero timeout calls")
        .is_empty());

    let timeout_error = unified_llm_adapter::timeout_error("generation step", Some(1.0));
    assert!(!timeout_error.retryable);
    let default_retry_calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "timed",
        Arc::clone(&default_retry_calls),
        vec![
            Err(timeout_error.clone()),
            Ok(Response {
                message: Message::assistant("late success"),
                finish_reason: FinishReason::Stop,
                ..Response::default()
            }),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("timed")).unwrap();
    let error = generate_with_policy_and_hooks(
        &client,
        Request {
            model: "timed-model".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        },
        &RetryPolicy {
            max_retries: 1,
            jitter: false,
            ..RetryPolicy::default()
        },
        || 1.0,
        |_| panic!("timeouts must not retry by default"),
    )
    .unwrap_err();
    assert_eq!(error.kind, AdapterErrorKind::RequestTimeout);
    assert_eq!(
        call_log(&default_retry_calls),
        vec!["complete:timed:timed-model:1"]
    );

    let opt_in_retry_calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "timed",
        Arc::clone(&opt_in_retry_calls),
        vec![
            Err(timeout_error),
            Ok(Response {
                message: Message::assistant("retried"),
                finish_reason: FinishReason::Stop,
                ..Response::default()
            }),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("timed")).unwrap();
    let sleeps = Arc::new(Mutex::new(Vec::new()));
    let result = generate_with_policy_and_hooks(
        &client,
        Request {
            model: "timed-model".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        },
        &RetryPolicy {
            max_retries: 1,
            jitter: false,
            retry_timeouts: true,
            ..RetryPolicy::default()
        },
        || 1.0,
        {
            let sleeps = Arc::clone(&sleeps);
            move |delay| sleeps.lock().expect("timeout sleeps").push(delay)
        },
    )
    .unwrap();
    assert_eq!(result.text, "retried");
    assert_eq!(sleeps.lock().expect("timeout sleeps").clone(), vec![1.0]);
    assert_eq!(
        call_log(&opt_in_retry_calls),
        vec![
            "complete:timed:timed-model:1",
            "complete:timed:timed-model:1",
        ]
    );
}

#[test]
fn stream_timeout_and_adapter_timeout_defaults_are_observable_without_live_providers() {
    let close_calls = Arc::new(Mutex::new(0));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ClosableStreamAdapter::new(
        "streaming",
        Arc::clone(&close_calls),
        vec![Ok(StreamEvent::text_delta("should not read"))],
    ));
    let client = Client::from_adapters(vec![adapter], Some("streaming")).unwrap();
    let mut result = stream(
        &client,
        GenerateRequest {
            prompt: Some("hello".to_string()),
            model: Some("stream-model".to_string()),
            timeout: Some(TimeoutConfig::default().with_stream_read(Some(0.0))),
            ..GenerateRequest::default()
        },
    )
    .unwrap();

    let error = result.next().expect("stream timeout").unwrap_err();
    assert_eq!(error.kind, AdapterErrorKind::RequestTimeout);
    assert!(error.message.contains("stream_read timed out"));
    assert_eq!(*close_calls.lock().expect("close count"), 1);

    let adapter_timeout = AdapterTimeout::default();
    assert_eq!(adapter_timeout.connect, DEFAULT_CONNECT_TIMEOUT_SECONDS);
    assert_eq!(adapter_timeout.request, DEFAULT_REQUEST_TIMEOUT_SECONDS);
    assert_eq!(
        adapter_timeout.stream_read,
        DEFAULT_STREAM_READ_TIMEOUT_SECONDS
    );
    assert_eq!(NativeRequestConfig::default().timeout, adapter_timeout);
    assert_eq!(
        OpenAICompatibleRequestConfig::default().timeout,
        adapter_timeout
    );
}

#[test]
fn tool_invocation_carries_same_abort_signal_without_keyword_introspection() {
    let controller = AbortController::new();
    let signal = controller.signal();
    let tool_call = ToolCall {
        id: "call-1".to_string(),
        name: "lookup".to_string(),
        arguments: json!({"city": "Paris"}),
        raw_arguments: None,
        r#type: "function".to_string(),
    };
    let invocation = ToolInvocation::new(
        tool_call.clone(),
        vec![Message::user("weather?")],
        Some(signal.clone()),
    );

    assert_eq!(invocation.tool_call, tool_call);
    assert_eq!(invocation.tool_call_id, "call-1");
    assert_eq!(invocation.arguments, json!({"city": "Paris"}));
    assert_eq!(invocation.messages, vec![Message::user("weather?")]);
    assert_eq!(invocation.abort_signal, Some(signal));
    assert!(invocation.check_abort().is_ok());

    controller.abort("tool cancelled");
    let error = invocation.check_abort().unwrap_err();
    assert_eq!(error.kind, AdapterErrorKind::Abort);
    assert_eq!(error.message, "tool cancelled");
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
    assert_eq!(result.generation.output, Some(json!({"answer": "yes"})));
    assert_eq!(
        call_log(&calls),
        vec![
            "complete:structured:object-model:1",
            "complete:structured:object-model:1",
        ]
    );
}

#[test]
fn high_level_generate_object_preserves_zero_retry_budget() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "structured",
        Arc::clone(&calls),
        vec![
            Err(retryable_rate_limit_error("object call should not retry")),
            Ok(Response {
                message: Message::assistant("{\"answer\":\"late success\"}"),
                finish_reason: FinishReason::Stop,
                ..Response::default()
            }),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("structured")).unwrap();
    let policy = RetryPolicy {
        max_retries: 0,
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
        |_| panic!("max_retries=0 must not sleep"),
    )
    .unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::RateLimit);
    assert_eq!(error.message, "object call should not retry");
    assert_eq!(call_log(&calls), vec!["complete:structured:object-model:1"]);
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
fn high_level_generate_object_validates_full_json_schema_keywords_locally() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "structured",
        Arc::clone(&calls),
        vec![Ok(Response {
            message: Message::assistant("{\"answer\":\"no\"}"),
            finish_reason: FinishReason::Stop,
            ..Response::default()
        })],
    ));
    let client = Client::from_adapters(vec![adapter], Some("structured")).unwrap();

    let error = generate_object(
        &client,
        Request {
            model: "object-model".to_string(),
            messages: vec![Message::user("answer as JSON")],
            ..Request::default()
        },
        json!({
            "type": "object",
            "required": ["answer"],
            "properties": {
                "answer": {"type": "string", "minLength": 3}
            }
        }),
    )
    .unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::NoObjectGenerated);
    assert_eq!(
        error.message,
        "structured output did not match the provided JSON Schema"
    );
    assert_eq!(call_log(&calls), vec!["complete:structured:object-model:1"]);
}

#[test]
fn high_level_generate_object_accepts_structured_output_tool_arguments() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "structured",
        Arc::clone(&calls),
        vec![Ok(Response {
            message: Message {
                role: MessageRole::Assistant,
                content: vec![ContentPart::ToolCall {
                    tool_call: ToolCall {
                        id: "call_structured".to_string(),
                        name: "structured_output".to_string(),
                        arguments: json!({"answer": "yes"}),
                        raw_arguments: Some("{\"answer\":\"yes\"}".to_string()),
                        r#type: "function".to_string(),
                    },
                }],
                ..Message::default()
            },
            finish_reason: FinishReason::ToolCalls,
            ..Response::default()
        })],
    ));
    let client = Client::from_adapters(vec![adapter], Some("structured")).unwrap();

    let result = generate_object(
        &client,
        Request {
            model: "object-model".to_string(),
            messages: vec![Message::user("answer as JSON")],
            tools: vec![Tool::passive_with_schema(
                "structured_output",
                None::<String>,
                Some(object_schema()),
            )
            .unwrap()],
            ..Request::default()
        },
        object_schema(),
    )
    .unwrap();

    assert_eq!(result.value, json!({"answer": "yes"}));
    assert_eq!(result.raw_text, "{\"answer\":\"yes\"}");
    assert_eq!(result.generation.output, Some(json!({"answer": "yes"})));
    assert_eq!(call_log(&calls), vec!["complete:structured:object-model:1"]);
}

#[test]
fn high_level_generate_object_prefers_structured_output_tool_over_mixed_text() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "structured",
        Arc::clone(&calls),
        vec![Ok(Response {
            message: Message {
                role: MessageRole::Assistant,
                content: vec![
                    ContentPart::Text {
                        text: "I found a valid object.".to_string(),
                    },
                    ContentPart::ToolCall {
                        tool_call: ToolCall::from_raw_arguments(
                            "call_structured",
                            "structured_output",
                            "{\"answer\":\"yes\"}",
                        ),
                    },
                ],
                ..Message::default()
            },
            finish_reason: FinishReason::ToolCalls,
            ..Response::default()
        })],
    ));
    let client = Client::from_adapters(vec![adapter], Some("structured")).unwrap();

    let result = generate_object(
        &client,
        Request {
            model: "object-model".to_string(),
            messages: vec![Message::user("answer as JSON")],
            ..Request::default()
        },
        object_schema(),
    )
    .unwrap();

    assert_eq!(result.value, json!({"answer": "yes"}));
    assert_eq!(result.raw_text, "{\"answer\":\"yes\"}");
    assert_eq!(result.generation.output, Some(json!({"answer": "yes"})));
    assert_eq!(result.generation.text, "I found a valid object.");
    assert_eq!(call_log(&calls), vec!["complete:structured:object-model:1"]);
}

#[test]
fn high_level_generate_object_treats_injected_structured_output_tool_as_terminal() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedCompleteAdapter::new(
        "structured",
        Arc::clone(&calls),
        vec![Ok(Response {
            message: Message {
                role: MessageRole::Assistant,
                content: vec![ContentPart::ToolCall {
                    tool_call: ToolCall::from_raw_arguments(
                        "call_structured",
                        "structured_output",
                        "{\"answer\":\"yes\"}",
                    ),
                }],
                ..Message::default()
            },
            finish_reason: FinishReason::ToolCalls,
            ..Response::default()
        })],
    ));
    let client = Client::from_adapters(vec![adapter], Some("structured")).unwrap();

    let result = generate_object(
        &client,
        Request {
            model: "object-model".to_string(),
            messages: vec![Message::user("answer as JSON")],
            ..Request::default()
        },
        object_schema(),
    )
    .unwrap();

    assert_eq!(result.value, json!({"answer": "yes"}));
    assert_eq!(result.raw_text, "{\"answer\":\"yes\"}");
    assert_eq!(result.generation.steps.len(), 1);
    assert!(result.generation.steps[0].tool_results.is_empty());
    assert_eq!(result.generation.output, Some(json!({"answer": "yes"})));
    assert_eq!(call_log(&calls), vec!["complete:structured:object-model:1"]);
}

#[test]
fn high_level_stream_object_treats_injected_structured_output_tool_as_terminal() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter = Arc::new(ScriptedStreamAdapter::new(
        "structured",
        Arc::clone(&calls),
        vec![Ok(vec![
            Ok(tool_call_stream_event(
                StreamEventType::ToolCallEnd,
                ToolCall::from_raw_arguments(
                    "call_structured",
                    "structured_output",
                    "{\"answer\":\"yes\"}",
                ),
            )),
            Ok(StreamEvent::finish(FinishReason::ToolCalls, None)),
        ])],
    ));
    let request_log = adapter.requests();
    let adapter: Arc<dyn ProviderAdapter> = adapter;
    let client = Client::from_adapters(vec![adapter], Some("structured")).unwrap();

    let mut result = stream_object(
        &client,
        Request {
            model: "object-stream-model".to_string(),
            messages: vec![Message::user("answer as JSON")],
            ..Request::default()
        },
        object_schema(),
    )
    .unwrap();

    assert_eq!(result.object().unwrap(), json!({"answer": "yes"}));
    assert_eq!(result.partial_object(), Some(json!({"answer": "yes"})));
    assert_eq!(result.response().unwrap().tool_calls().len(), 1);
    assert_eq!(
        call_log(&calls),
        vec!["stream:structured:object-stream-model:1"]
    );

    let requests = request_log.lock().expect("request log").clone();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].tools.is_empty());
}

#[test]
fn high_level_stream_object_retries_provider_errors_but_not_local_parse_or_schema_failures() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedStreamAdapter::new(
        "structured",
        Arc::clone(&calls),
        vec![
            Err(retryable_rate_limit_error("retry object stream")),
            Ok(vec![Ok(StreamEvent::text_delta("{\"answer\":\"yes\"}"))]),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("structured")).unwrap();
    let policy = RetryPolicy {
        max_retries: 1,
        jitter: false,
        ..RetryPolicy::default()
    };
    let sleeps = Arc::new(Mutex::new(Vec::new()));

    let mut result = stream_object_with_policy_and_hooks(
        &client,
        Request {
            model: "object-stream-model".to_string(),
            messages: vec![Message::user("answer as JSON")],
            ..Request::default()
        },
        object_schema(),
        &policy,
        || 1.0,
        {
            let sleeps = Arc::clone(&sleeps);
            move |delay| sleeps.lock().expect("sleeps").push(delay)
        },
    )
    .unwrap();

    assert_eq!(result.object().unwrap(), json!({"answer": "yes"}));
    assert_eq!(result.partial_object(), Some(json!({"answer": "yes"})));
    assert_eq!(sleeps.lock().expect("sleeps").clone(), vec![1.0]);
    assert_eq!(
        call_log(&calls),
        vec![
            "stream:structured:object-stream-model:1",
            "stream:structured:object-stream-model:1",
        ]
    );

    for (response_text, expected_message) in [
        ("not json", "failed to parse structured output as JSON"),
        (
            "{\"answer\":7}",
            "structured output did not match the provided JSON Schema",
        ),
    ] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedStreamAdapter::new(
            "structured",
            Arc::clone(&calls),
            vec![
                Ok(vec![Ok(StreamEvent::text_delta(response_text))]),
                Ok(vec![Ok(StreamEvent::text_delta(
                    "{\"answer\":\"late success\"}",
                ))]),
            ],
        ));
        let client = Client::from_adapters(vec![adapter], Some("structured")).unwrap();
        let mut result = stream_object_with_policy_and_hooks(
            &client,
            Request {
                model: "object-stream-model".to_string(),
                messages: vec![Message::user("answer as JSON")],
                ..Request::default()
            },
            object_schema(),
            &RetryPolicy {
                max_retries: 2,
                jitter: false,
                ..RetryPolicy::default()
            },
            || 1.0,
            |_| panic!("local structured parse failures must not sleep or retry"),
        )
        .unwrap();

        let error = result.object().unwrap_err();
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
        assert_eq!(
            call_log(&calls),
            vec!["stream:structured:object-stream-model:1"]
        );
    }
}

#[test]
fn high_level_stream_object_yields_partial_updates_before_complete_json() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedStreamAdapter::new(
        "structured",
        Arc::clone(&calls),
        vec![Ok(vec![
            Ok(StreamEvent::text_delta("{\"answer\":\"hel")),
            Ok(StreamEvent::text_delta("lo\"}")),
        ])],
    ));
    let client = Client::from_adapters(vec![adapter], Some("structured")).unwrap();

    let mut result = stream_object(
        &client,
        Request {
            model: "object-stream-model".to_string(),
            messages: vec![Message::user("answer as JSON")],
            ..Request::default()
        },
        object_schema(),
    )
    .unwrap();

    let first = result.next().unwrap().unwrap();
    assert_eq!(first, json!({"answer": "hel"}));
    assert_eq!(result.partial_object(), Some(json!({"answer": "hel"})));
    assert_eq!(result.partial_response().text(), "{\"answer\":\"hel");

    let second = result.next().unwrap().unwrap();
    assert_eq!(second, json!({"answer": "hello"}));
    assert_eq!(result.object().unwrap(), json!({"answer": "hello"}));
    assert_eq!(
        call_log(&calls),
        vec!["stream:structured:object-stream-model:1"]
    );
}

#[test]
fn high_level_stream_object_yields_partial_updates_from_structured_output_tool_deltas() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedStreamAdapter::new(
        "structured",
        Arc::clone(&calls),
        vec![Ok(vec![
            Ok(StreamEvent::text_delta("The object is coming.")),
            Ok(tool_call_stream_event(
                StreamEventType::ToolCallStart,
                ToolCall::from_raw_arguments("call_structured", "structured_output", ""),
            )),
            Ok(tool_call_stream_event(
                StreamEventType::ToolCallDelta,
                ToolCall::from_raw_arguments("call_structured", "", "{\"answer\":\"he"),
            )),
            Ok(tool_call_stream_event(
                StreamEventType::ToolCallDelta,
                ToolCall::from_raw_arguments("call_structured", "", "llo\"}"),
            )),
            Ok(StreamEvent::finish(FinishReason::ToolCalls, None)),
        ])],
    ));
    let client = Client::from_adapters(vec![adapter], Some("structured")).unwrap();

    let mut result = stream_object(
        &client,
        Request {
            model: "object-stream-model".to_string(),
            messages: vec![Message::user("answer as JSON")],
            ..Request::default()
        },
        object_schema(),
    )
    .unwrap();

    let first = result.next().unwrap().unwrap();
    assert_eq!(first, json!({"answer": "he"}));
    assert_eq!(result.partial_object(), Some(json!({"answer": "he"})));
    assert_eq!(result.partial_response().text(), "The object is coming.");
    assert_eq!(
        result.partial_response().tool_calls()[0]
            .raw_arguments
            .as_deref(),
        Some("{\"answer\":\"he")
    );

    let second = result.next().unwrap().unwrap();
    assert_eq!(second, json!({"answer": "hello"}));
    assert_eq!(result.object().unwrap(), json!({"answer": "hello"}));
    assert_eq!(result.partial_object(), Some(json!({"answer": "hello"})));
    assert_eq!(
        call_log(&calls),
        vec!["stream:structured:object-stream-model:1"]
    );
}

#[test]
fn high_level_stream_object_preserves_zero_retry_budget() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ScriptedStreamAdapter::new(
        "structured",
        Arc::clone(&calls),
        vec![
            Err(retryable_rate_limit_error("object stream should not retry")),
            Ok(vec![Ok(StreamEvent::text_delta(
                "{\"answer\":\"late success\"}",
            ))]),
        ],
    ));
    let client = Client::from_adapters(vec![adapter], Some("structured")).unwrap();
    let policy = RetryPolicy {
        max_retries: 0,
        jitter: false,
        ..RetryPolicy::default()
    };

    let error = match stream_object_with_policy_and_hooks(
        &client,
        Request {
            model: "object-stream-model".to_string(),
            messages: vec![Message::user("answer as JSON")],
            ..Request::default()
        },
        object_schema(),
        &policy,
        || 1.0,
        |_| panic!("max_retries=0 must not sleep"),
    ) {
        Ok(_) => panic!("max_retries=0 should surface the first stream open error"),
        Err(error) => error,
    };

    assert_eq!(error.kind, AdapterErrorKind::RateLimit);
    assert_eq!(error.message, "object stream should not retry");
    assert_eq!(
        call_log(&calls),
        vec!["stream:structured:object-stream-model:1"]
    );
}

#[test]
fn high_level_generate_object_resolves_models_before_low_level_request_construction() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(StructuredRecordingAdapter::new(
        "openai",
        Arc::clone(&calls),
    ));
    let client = Client::from_adapters(vec![adapter], Some("openai")).unwrap();
    let result = generate_object(
        &client,
        GenerateRequest {
            prompt: Some("answer as JSON".to_string()),
            ..GenerateRequest::default()
        },
        object_schema(),
    )
    .unwrap();
    let structured_openai = get_latest_model("openai", Some("structured_output"))
        .expect("openai structured default")
        .id;

    assert_eq!(result.value, json!({"answer": "yes"}));
    assert_eq!(result.generation.steps[0].request.model, structured_openai);
    assert!(matches!(
        result.generation.steps[0].request.response_format.as_ref(),
        Some(ResponseFormat::JsonSchema { strict: true, .. })
    ));
    assert_eq!(
        call_log(&calls),
        vec![format!(
            "complete:openai:{}:1:json_schema_strict",
            result.generation.steps[0].request.model
        )]
    );

    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(StructuredRecordingAdapter::new(
        "openai_compatible",
        Arc::clone(&calls),
    ));
    let client = Client::from_adapters(vec![adapter], None).unwrap();
    let profiled = generate_object(
        &client,
        GenerateRequest {
            prompt: Some("answer as JSON".to_string()),
            active_profile: Some(ActiveLlmProfile::new(
                "openai_compatible",
                Some("local-large".to_string()),
            )),
            ..GenerateRequest::default()
        },
        object_schema(),
    )
    .unwrap();
    assert_eq!(
        profiled.generation.steps[0].request.provider.as_deref(),
        Some("openai_compatible")
    );
    assert_eq!(profiled.generation.steps[0].request.model, "local-large");
    assert_eq!(
        call_log(&calls),
        vec!["complete:openai_compatible:local-large:1:json_schema_strict"]
    );

    for provider in ["openrouter", "litellm", "openai_compatible"] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let adapter: Arc<dyn ProviderAdapter> = Arc::new(StructuredRecordingAdapter::new(
            provider,
            Arc::clone(&calls),
        ));
        let client = Client::from_adapters(vec![adapter], Some(provider)).unwrap();
        let error = generate_object(
            &client,
            GenerateRequest {
                prompt: Some("answer as JSON".to_string()),
                ..GenerateRequest::default()
            },
            object_schema(),
        )
        .unwrap_err();

        assert_eq!(error.kind, AdapterErrorKind::Configuration);
        assert!(error.message.contains("No model configured"));
        assert!(call_log(&calls).is_empty());
    }

    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(StructuredRecordingAdapter::new(
        "openai",
        Arc::clone(&calls),
    ));
    let client = Client::from_adapters(vec![adapter], Some("openai")).unwrap();
    let explicit_unknown = generate_object(
        &client,
        GenerateRequest {
            prompt: Some("answer as JSON".to_string()),
            model: Some("unknown-structured-model-2026-06-27".to_string()),
            ..GenerateRequest::default()
        },
        object_schema(),
    )
    .unwrap();
    assert_eq!(
        explicit_unknown.generation.steps[0].request.model,
        "unknown-structured-model-2026-06-27"
    );
    assert_eq!(
        call_log(&calls),
        vec!["complete:openai:unknown-structured-model-2026-06-27:1:json_schema_strict"]
    );
}

#[test]
fn high_level_stream_object_resolves_models_before_low_level_request_construction() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(StructuredRecordingAdapter::new(
        "gemini",
        Arc::clone(&calls),
    ));
    let client = Client::from_adapters(vec![adapter], Some("gemini")).unwrap();
    let mut result = stream_object(
        &client,
        GenerateRequest {
            prompt: Some("answer as JSON".to_string()),
            ..GenerateRequest::default()
        },
        object_schema(),
    )
    .unwrap();
    let structured_gemini = get_latest_model("gemini", Some("structured_output"))
        .expect("gemini structured default")
        .id;
    assert_eq!(result.object().unwrap(), json!({"answer": "yes"}));
    assert_eq!(
        call_log(&calls),
        vec![format!(
            "stream:gemini:{structured_gemini}:1:json_schema_strict"
        )]
    );

    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(StructuredRecordingAdapter::new(
        "openai_compatible",
        Arc::clone(&calls),
    ));
    let client = Client::from_adapters(vec![adapter], None).unwrap();
    let mut profiled = stream_object(
        &client,
        GenerateRequest {
            prompt: Some("answer as JSON".to_string()),
            active_profile: Some(ActiveLlmProfile::new(
                "openai_compatible",
                Some("local-large".to_string()),
            )),
            ..GenerateRequest::default()
        },
        object_schema(),
    )
    .unwrap();
    assert_eq!(profiled.object().unwrap(), json!({"answer": "yes"}));
    assert_eq!(
        call_log(&calls),
        vec!["stream:openai_compatible:local-large:1:json_schema_strict"]
    );

    for provider in ["openrouter", "litellm", "openai_compatible"] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let adapter: Arc<dyn ProviderAdapter> = Arc::new(StructuredRecordingAdapter::new(
            provider,
            Arc::clone(&calls),
        ));
        let client = Client::from_adapters(vec![adapter], Some(provider)).unwrap();
        let error = match stream_object(
            &client,
            GenerateRequest {
                prompt: Some("answer as JSON".to_string()),
                ..GenerateRequest::default()
            },
            object_schema(),
        ) {
            Ok(_) => panic!("stream_object should reject an omitted compatible model"),
            Err(error) => error,
        };

        assert_eq!(error.kind, AdapterErrorKind::Configuration);
        assert!(error.message.contains("No model configured"));
        assert!(call_log(&calls).is_empty());
    }

    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(StructuredRecordingAdapter::new(
        "openai",
        Arc::clone(&calls),
    ));
    let client = Client::from_adapters(vec![adapter], Some("openai")).unwrap();
    let mut explicit_unknown = stream_object(
        &client,
        GenerateRequest {
            prompt: Some("answer as JSON".to_string()),
            model: Some("unknown-structured-stream-model-2026-06-27".to_string()),
            ..GenerateRequest::default()
        },
        object_schema(),
    )
    .unwrap();
    assert_eq!(explicit_unknown.object().unwrap(), json!({"answer": "yes"}));
    assert_eq!(
        call_log(&calls),
        vec!["stream:openai:unknown-structured-stream-model-2026-06-27:1:json_schema_strict"]
    );
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

    let response_format_error = Request {
        model: "model".to_string(),
        messages: vec![Message::user("hello")],
        response_format: Some(ResponseFormat::JsonSchema {
            json_schema: json!("not a schema object"),
            strict: true,
        }),
        ..Request::default()
    }
    .validate_for_client()
    .unwrap_err();
    assert!(response_format_error.contains("root JSON Schema object"));
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
fn low_level_client_rejects_invalid_json_schema_response_format_before_provider_call() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingAdapter::new("fake", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], Some("fake")).unwrap();

    let error = client
        .complete(Request {
            model: "model".to_string(),
            messages: vec![Message::user("hello")],
            response_format: Some(ResponseFormat::JsonSchema {
                json_schema: json!(["not", "a", "schema", "object"]),
                strict: true,
            }),
            ..Request::default()
        })
        .unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::InvalidRequest);
    assert!(error.message.contains("root JSON Schema object"));
    assert!(!call_log(&calls)
        .iter()
        .any(|call| call.starts_with("complete:")));
}

#[test]
fn low_level_client_rejects_empty_model_after_middleware_before_provider_call() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingAdapter::new("fake", Arc::clone(&calls)));
    let middleware: Arc<dyn Middleware> = Arc::new(EmptyModelMiddleware);
    let client = Client::from_adapters(vec![adapter], Some("fake"))
        .unwrap()
        .with_middleware(vec![middleware]);

    let complete_error = client
        .complete(Request {
            model: "valid-before-middleware".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap_err();
    let stream_error = match client.stream(Request {
        model: "valid-before-middleware".to_string(),
        messages: vec![Message::user("hello")],
        ..Request::default()
    }) {
        Ok(_) => panic!("stream should reject an empty post-middleware model"),
        Err(error) => error,
    };

    assert_eq!(complete_error.kind, AdapterErrorKind::InvalidRequest);
    assert_eq!(stream_error.kind, AdapterErrorKind::InvalidRequest);
    assert!(complete_error.message.contains("request model"));
    assert!(stream_error.message.contains("request model"));
    assert_eq!(call_log(&calls), vec!["initialize:fake"]);
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

struct EmptyModelMiddleware;

impl Middleware for EmptyModelMiddleware {
    fn complete(
        &self,
        mut request: Request,
        next: &unified_llm_adapter::CompleteNext<'_>,
    ) -> Result<Response, unified_llm_adapter::AdapterError> {
        request.model.clear();
        next(request)
    }

    fn stream(
        &self,
        mut request: Request,
        next: &unified_llm_adapter::StreamNext<'_>,
    ) -> Result<StreamEvents, unified_llm_adapter::AdapterError> {
        request.model.clear();
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

struct HighLevelBoundaryAdapter {
    name: &'static str,
    calls: Arc<Mutex<Vec<String>>>,
}

impl HighLevelBoundaryAdapter {
    fn new(name: &'static str, calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self { name, calls }
    }

    fn record(&self, operation: &str, request: &Request) {
        self.calls.lock().expect("call log lock").push(format!(
            "{operation}:{}:{}:{}",
            request.provider.as_deref().unwrap_or_default(),
            request.model,
            message_summary(&request.messages)
        ));
    }
}

impl ProviderAdapter for HighLevelBoundaryAdapter {
    fn name(&self) -> &str {
        self.name
    }

    fn complete(&self, request: Request) -> Result<Response, unified_llm_adapter::AdapterError> {
        self.record("complete", &request);
        Ok(Response {
            model: request.model,
            provider: request.provider.unwrap_or_default(),
            message: Message {
                role: MessageRole::Assistant,
                content: vec![
                    ContentPart::Text {
                        text: "boundary text".to_string(),
                    },
                    ContentPart::Thinking {
                        thinking: ThinkingData {
                            text: "boundary reasoning".to_string(),
                            signature: None,
                            redacted: false,
                            source_provider: None,
                            source_model: None,
                        },
                    },
                    ContentPart::ToolCall {
                        tool_call: ToolCall {
                            id: "call_lookup".to_string(),
                            name: "lookup".to_string(),
                            arguments: json!({"city": "Paris"}),
                            raw_arguments: None,
                            r#type: "function".to_string(),
                        },
                    },
                ],
                ..Message::default()
            },
            finish_reason: FinishReason::Stop,
            usage: Usage {
                input_tokens: 3,
                output_tokens: 5,
                total_tokens: 8,
                ..Usage::default()
            },
            warnings: vec![Warning {
                message: "boundary warning".to_string(),
                code: Some("boundary_warning".to_string()),
            }],
            ..Response::default()
        })
    }

    fn stream(&self, request: Request) -> Result<StreamEvents, unified_llm_adapter::AdapterError> {
        self.record("stream", &request);
        Ok(stream_events(
            vec![
                Ok(StreamEvent::text_delta("hel")),
                Ok(StreamEvent::text_delta("lo")),
                Ok(StreamEvent::finish(FinishReason::Stop, None)),
            ]
            .into_iter(),
        ))
    }
}

struct StructuredRecordingAdapter {
    name: &'static str,
    calls: Arc<Mutex<Vec<String>>>,
}

impl StructuredRecordingAdapter {
    fn new(name: &'static str, calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self { name, calls }
    }

    fn record(&self, operation: &str, request: &Request) {
        self.calls.lock().expect("call log lock").push(format!(
            "{operation}:{}:{}:{}:{}",
            request.provider.as_deref().unwrap_or_default(),
            request.model,
            request.messages.len(),
            response_format_label(request.response_format.as_ref())
        ));
    }
}

impl ProviderAdapter for StructuredRecordingAdapter {
    fn name(&self) -> &str {
        self.name
    }

    fn complete(&self, request: Request) -> Result<Response, unified_llm_adapter::AdapterError> {
        self.record("complete", &request);
        Ok(Response {
            model: request.model,
            provider: request.provider.unwrap_or_default(),
            message: Message::assistant("{\"answer\":\"yes\"}"),
            finish_reason: FinishReason::Stop,
            ..Response::default()
        })
    }

    fn stream(&self, request: Request) -> Result<StreamEvents, unified_llm_adapter::AdapterError> {
        self.record("stream", &request);
        Ok(stream_events(
            vec![Ok(StreamEvent::text_delta("{\"answer\":\"yes\"}"))].into_iter(),
        ))
    }
}

fn response_format_label(response_format: Option<&ResponseFormat>) -> &'static str {
    match response_format {
        Some(ResponseFormat::JsonSchema { strict: true, .. }) => "json_schema_strict",
        Some(ResponseFormat::JsonSchema { .. }) => "json_schema",
        Some(ResponseFormat::JsonObject) => "json",
        Some(ResponseFormat::Text) => "text",
        None => "none",
    }
}

fn message_summary(messages: &[Message]) -> String {
    messages
        .iter()
        .map(|message| format!("{}={}", role_label(message.role), message.text()))
        .collect::<Vec<_>>()
        .join("|")
}

fn role_label(role: MessageRole) -> &'static str {
    match role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
        MessageRole::Developer => "developer",
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

struct TimeoutRecordingAdapter {
    name: &'static str,
    calls: Arc<Mutex<Vec<(String, Option<TimeoutConfig>)>>>,
    results: Mutex<VecDeque<Result<Response, unified_llm_adapter::AdapterError>>>,
}

impl TimeoutRecordingAdapter {
    fn new(
        name: &'static str,
        calls: Arc<Mutex<Vec<(String, Option<TimeoutConfig>)>>>,
        results: Vec<Result<Response, unified_llm_adapter::AdapterError>>,
    ) -> Self {
        Self {
            name,
            calls,
            results: Mutex::new(VecDeque::from(results)),
        }
    }
}

impl ProviderAdapter for TimeoutRecordingAdapter {
    fn name(&self) -> &str {
        self.name
    }

    fn complete(&self, request: Request) -> Result<Response, unified_llm_adapter::AdapterError> {
        self.calls
            .lock()
            .expect("timeout call log")
            .push((request.model.clone(), request.timeout));
        self.results
            .lock()
            .expect("timeout recording result lock")
            .pop_front()
            .unwrap_or_else(|| {
                Err(unified_llm_adapter::AdapterError::new(
                    unified_llm_adapter::AdapterErrorKind::Configuration,
                    "no timeout recording response",
                ))
            })
            .map(|mut response| {
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
            "timeout recording adapter does not implement stream",
        ))
    }
}

struct ScriptedStreamAdapter {
    name: &'static str,
    calls: Arc<Mutex<Vec<String>>>,
    requests: Arc<Mutex<Vec<Request>>>,
    results: Mutex<VecDeque<Result<Vec<Result<StreamEvent, AdapterError>>, AdapterError>>>,
}

impl ScriptedStreamAdapter {
    fn new(
        name: &'static str,
        calls: Arc<Mutex<Vec<String>>>,
        results: Vec<Result<Vec<Result<StreamEvent, AdapterError>>, AdapterError>>,
    ) -> Self {
        Self {
            name,
            calls,
            requests: Arc::new(Mutex::new(Vec::new())),
            results: Mutex::new(VecDeque::from(results)),
        }
    }

    fn requests(&self) -> Arc<Mutex<Vec<Request>>> {
        Arc::clone(&self.requests)
    }

    fn record(&self, call: impl Into<String>) {
        self.calls.lock().expect("call log lock").push(call.into());
    }
}

impl ProviderAdapter for ScriptedStreamAdapter {
    fn name(&self) -> &str {
        self.name
    }

    fn complete(&self, _request: Request) -> Result<Response, unified_llm_adapter::AdapterError> {
        Err(unified_llm_adapter::AdapterError::new(
            unified_llm_adapter::AdapterErrorKind::Configuration,
            "scripted stream adapter does not implement complete",
        ))
    }

    fn stream(&self, request: Request) -> Result<StreamEvents, unified_llm_adapter::AdapterError> {
        self.requests
            .lock()
            .expect("scripted stream request lock")
            .push(request.clone());
        self.record(format!(
            "stream:{}:{}:{}",
            request.provider.as_deref().unwrap_or_default(),
            request.model,
            request.messages.len()
        ));
        self.results
            .lock()
            .expect("scripted stream result lock")
            .pop_front()
            .unwrap_or_else(|| {
                Err(unified_llm_adapter::AdapterError::new(
                    unified_llm_adapter::AdapterErrorKind::Configuration,
                    "no scripted stream response",
                ))
            })
            .map(|events| stream_events(events.into_iter()))
    }
}

struct ClosableStreamAdapter {
    name: &'static str,
    close_calls: Arc<Mutex<usize>>,
    events: Vec<Result<StreamEvent, AdapterError>>,
}

impl ClosableStreamAdapter {
    fn new(
        name: &'static str,
        close_calls: Arc<Mutex<usize>>,
        events: Vec<Result<StreamEvent, AdapterError>>,
    ) -> Self {
        Self {
            name,
            close_calls,
            events,
        }
    }
}

impl ProviderAdapter for ClosableStreamAdapter {
    fn name(&self) -> &str {
        self.name
    }

    fn complete(&self, _request: Request) -> Result<Response, unified_llm_adapter::AdapterError> {
        Err(unified_llm_adapter::AdapterError::new(
            unified_llm_adapter::AdapterErrorKind::Configuration,
            "closable stream adapter does not implement complete",
        ))
    }

    fn stream(&self, _request: Request) -> Result<StreamEvents, unified_llm_adapter::AdapterError> {
        let close_calls = Arc::clone(&self.close_calls);
        Ok(managed_stream(self.events.clone().into_iter(), move || {
            *close_calls.lock().expect("close call count") += 1;
            Ok(())
        }))
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
        timeout: None,
        abort_signal: None,
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

fn tool_call_stream_event(event_type: StreamEventType, tool_call: ToolCall) -> StreamEvent {
    StreamEvent {
        r#type: event_type.clone(),
        tool_call: Some(tool_call),
        ..StreamEvent::new(event_type)
    }
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
