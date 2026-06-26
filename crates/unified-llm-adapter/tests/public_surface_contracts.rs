use std::any::TypeId;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::json;
use unified_llm_adapter::{
    get_default_client, set_default_client, AudioData, Client, ContentKind, ContentPart,
    DocumentData, FinishReason, FinishReasonKind, ImageData, LlmRequest, LlmResponse, Message,
    MessageRole, Middleware, ProviderAdapter, RateLimitInfo, Request, Response, ResponseFormat,
    Role, StreamAccumulator, StreamEvent, StreamEventType, StreamEvents, ThinkingData, ToolCall,
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
        Ok(Box::new(
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
        Ok(Box::new(
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
        Ok(Box::new(stream.map(move |result| match result {
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
        Ok(Box::new(
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
        Ok(Box::new(
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
