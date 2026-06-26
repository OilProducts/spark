use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde_json::{json, Value};
use unified_llm_adapter::{
    calculate_retry_delay, classify_provider_error_message, error_from_status_code,
    get_latest_model, get_model_info, list_models, resolve_effective_llm_model,
    resolve_effective_llm_provider, resolve_effective_reasoning_effort, AdapterError,
    AdapterErrorKind, FinishReason, LlmRequest, LlmResolutionInputs, Message, ProviderConfig,
    ProviderEnvironment, ResponseFormat, RetryPolicy, StreamAccumulator, StreamEvent,
    StreamEventType, ToolCall, ToolResult, Usage, RUNTIME_LAUNCH_MODEL_KEY,
    RUNTIME_LAUNCH_PROVIDER_KEY, RUNTIME_LAUNCH_REASONING_EFFORT_KEY,
};

#[test]
fn llm_resolution_preserves_node_launch_fallback_order_and_reasoning_rule() {
    let mut context = BTreeMap::new();
    context.insert(RUNTIME_LAUNCH_MODEL_KEY.to_string(), json!("gpt-launch"));
    context.insert(RUNTIME_LAUNCH_PROVIDER_KEY.to_string(), json!("Anthropic"));
    context.insert(
        RUNTIME_LAUNCH_REASONING_EFFORT_KEY.to_string(),
        json!("Medium"),
    );
    let inputs = LlmResolutionInputs {
        node_model: Some("gpt-node".to_string()),
        node_reasoning_effort: Some("low".to_string()),
        node_reasoning_is_default_placeholder: true,
        fallback_provider: Some("openai".to_string()),
        fallback_reasoning_effort: Some("high".to_string()),
        ..LlmResolutionInputs::default()
    };

    assert_eq!(
        resolve_effective_llm_model(&inputs, &context).as_deref(),
        Some("gpt-node")
    );
    assert_eq!(
        resolve_effective_llm_provider(&inputs, &context),
        "anthropic"
    );
    assert_eq!(
        resolve_effective_reasoning_effort(&inputs, &context).as_deref(),
        Some("medium")
    );
}

#[test]
fn provider_model_env_observation_matches_active_fixture() {
    assert_fixture_observation(
        "model-catalog-env-resolution",
        model_catalog_env_resolution_observation(),
    );
}

#[test]
fn retry_error_usage_stream_observation_matches_active_fixture() {
    assert_fixture_observation(
        "retry-error-usage-stream",
        retry_error_usage_stream_observation(),
    );
}

#[test]
fn request_tool_structured_output_observation_matches_active_fixture() {
    assert_fixture_observation(
        "request-tool-structured-output",
        request_tool_structured_output_observation(),
    );
}

#[test]
fn m6_provider_profile_resource_observation_matches_active_fixture() {
    assert_fixture_observation(
        "m6-provider-profile-resource-parity",
        m6_provider_profile_resource_observation(),
    );
}

fn model_catalog_env_resolution_observation() -> Value {
    let env = BTreeMap::from([
        ("ANTHROPIC_API_KEY".to_string(), "anthropic-key".to_string()),
        ("OPENAI_API_KEY".to_string(), "openai-key".to_string()),
        ("OPENAI_ORG_ID".to_string(), "org-1".to_string()),
    ]);
    let providers = ProviderEnvironment::from_env_map(&env, None);

    json!({
        "sonnet_alias": get_model_info("sonnet"),
        "gemini_models": list_models(Some("gemini"))
            .into_iter()
            .map(|model| model.id)
            .collect::<Vec<_>>(),
        "openai_latest": get_latest_model("openai", None),
        "client_default_provider": providers.default_provider,
        "client_providers": providers.providers.keys().cloned().collect::<Vec<_>>(),
    })
}

fn m6_provider_profile_resource_observation() -> Value {
    let env = BTreeMap::from([
        ("OPENAI_API_KEY".to_string(), "openai-key".to_string()),
        (
            "OPENAI_BASE_URL".to_string(),
            "https://openai.example/responses".to_string(),
        ),
        ("OPENAI_ORG_ID".to_string(), "org-1".to_string()),
        ("OPENAI_PROJECT_ID".to_string(), "project-1".to_string()),
        ("ANTHROPIC_API_KEY".to_string(), "anthropic-key".to_string()),
        (
            "ANTHROPIC_BASE_URL".to_string(),
            "https://anthropic.example/messages".to_string(),
        ),
        ("GOOGLE_API_KEY".to_string(), "google-key".to_string()),
        (
            "GEMINI_BASE_URL".to_string(),
            "https://gemini.example/v1".to_string(),
        ),
        (
            "OPENROUTER_API_KEY".to_string(),
            "openrouter-key".to_string(),
        ),
        (
            "OPENROUTER_HTTP_REFERER".to_string(),
            "https://spark.example".to_string(),
        ),
        ("OPENROUTER_TITLE".to_string(), "Spark".to_string()),
        (
            "LITELLM_BASE_URL".to_string(),
            "https://litellm.example/api".to_string(),
        ),
        ("LITELLM_API_KEY".to_string(), "".to_string()),
        (
            "OPENAI_COMPATIBLE_BASE_URL".to_string(),
            "https://compatible.example/custom/chat/completions".to_string(),
        ),
        ("OPENAI_COMPATIBLE_API_KEY".to_string(), "".to_string()),
    ]);
    let automatic = ProviderEnvironment::from_env_map(&env, None);
    let explicit = ProviderEnvironment::from_env_map(&env, Some("OpenRouter"));

    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        temp.path().join("llm-profiles.toml"),
        r#"
[profiles.local]
provider = "openai_compatible"
base_url = "http://127.0.0.1:4000/v1"
models = ["local-small", "local-large"]
label = "Local"
api_key_env = "LOCAL_PROFILE_API_KEY"
default_model = "local-large"

[profiles.no_key]
provider = "openai_compatible"
base_url = "http://127.0.0.1:5000/v1"
models = ["no-key-model"]
"#,
    )
    .expect("write profiles");
    let absent_profile_env = BTreeMap::<String, String>::new();
    let present_profile_env = BTreeMap::from([(
        "LOCAL_PROFILE_API_KEY".to_string(),
        "profile-key".to_string(),
    )]);
    let absent_profiles =
        unified_llm_adapter::public_llm_profiles_with_env(temp.path(), &absent_profile_env)
            .expect("absent profiles");
    let present_profiles =
        unified_llm_adapter::public_llm_profiles_with_env(temp.path(), &present_profile_env)
            .expect("present profiles");
    let public_profile_text = serde_json::to_string(&present_profiles).expect("profiles json");
    assert!(!public_profile_text.contains("127.0.0.1"));
    assert!(!public_profile_text.contains("LOCAL_PROFILE_API_KEY"));
    assert!(!public_profile_text.contains("profile-key"));

    let provider_keys = explicit.providers.keys().cloned().collect::<Vec<_>>();
    let providers = explicit
        .providers
        .iter()
        .map(|(name, config)| (name.clone(), provider_config_observation(config)))
        .collect::<BTreeMap<_, _>>();
    let provider_counts = ["openai", "anthropic", "gemini", "openrouter", "litellm"]
        .into_iter()
        .map(|provider| (provider.to_string(), list_models(Some(provider)).len()))
        .collect::<BTreeMap<_, _>>();
    let codex_alias = get_model_info("codex").map(|model| model.id);

    json!({
        "provider_environment": {
            "automatic_default_provider": automatic.default_provider,
            "explicit_default_provider": explicit.default_provider,
            "provider_keys": provider_keys,
            "providers": providers,
        },
        "profiles": {
            "without_process_env": absent_profiles,
            "with_process_env": present_profiles,
        },
        "model_catalog": {
            "codex_alias": codex_alias,
            "provider_counts": provider_counts,
        },
    })
}

fn provider_config_observation(config: &ProviderConfig) -> Value {
    let mut config_payload = serde_json::Map::new();
    config_payload.insert("api_key".to_string(), json!(config.api_key));
    config_payload.insert("base_url".to_string(), json!(config.base_url));
    if let Some(value) = config.options.get("organization") {
        config_payload.insert("organization".to_string(), json!(value));
    }
    if let Some(value) = config.options.get("project") {
        config_payload.insert("project".to_string(), json!(value));
    }

    let default_headers = ["HTTP-Referer", "X-Title"]
        .into_iter()
        .filter_map(|key| {
            config
                .options
                .get(key)
                .map(|value| (key.to_string(), json!(value)))
        })
        .collect::<serde_json::Map<_, _>>();
    let require_api_key = match config.provider.as_str() {
        "openrouter" => Some(true),
        "litellm" | "openai_compatible" => Some(false),
        _ => None,
    };

    json!({
        "config": Value::Object(config_payload),
        "default_headers": Value::Object(default_headers),
        "require_api_key": require_api_key,
    })
}

fn retry_error_usage_stream_observation() -> Value {
    let policy = RetryPolicy::default();
    let retry_after = AdapterError {
        kind: AdapterErrorKind::RateLimit,
        message: "slow".to_string(),
        provider: Some("openai".to_string()),
        status_code: Some(429),
        error_code: None,
        retryable: true,
        retry_after: Some(12.5),
        raw: None,
    };
    let server_error = error_from_status_code(
        Some(503),
        "temporary failure",
        Some("openai"),
        None,
        None,
        None,
    );

    let mut accumulator = StreamAccumulator::default();
    accumulator.push(StreamEvent {
        event_type: StreamEventType::ProviderEvent,
        text: None,
        reasoning: None,
        tool_call: None,
        finish_reason: None,
        usage: None,
        error: None,
        raw: Some(json!({"kind": "provider"})),
    });
    accumulator.push(StreamEvent::text_delta("hello "));
    accumulator.push(StreamEvent::text_delta("world"));
    accumulator.push(StreamEvent::finish(
        FinishReason::Stop,
        Some(Usage {
            input_tokens: 10,
            output_tokens: 5,
            ..Usage::default()
        }),
    ));
    let usage = Usage {
        input_tokens: 10,
        output_tokens: 5,
        total_tokens: 15,
        ..Usage::default()
    };
    let cost = usage
        .cost_for_model(&get_model_info("gpt-5.2").unwrap())
        .unwrap();

    json!({
        "retry_policy": policy,
        "retry_delays": {
            "attempt_0_no_jitter": calculate_retry_delay(&policy, 0, None, Some(1.0)).unwrap(),
            "attempt_1_no_jitter": calculate_retry_delay(&policy, 1, None, Some(1.0)).unwrap(),
            "retry_after": calculate_retry_delay(&policy, 0, Some(&retry_after), None).unwrap(),
        },
        "errors": {
            "server": {
                "type": adapter_error_type(server_error.kind),
                "message": server_error.message,
                "provider": server_error.provider,
                "status_code": server_error.status_code,
                "retryable": server_error.retryable,
                "retry_after": server_error.retry_after,
            },
            "not_found_classifier": adapter_error_type(
                classify_provider_error_message(Some("model does not exist"), None).unwrap()
            ),
        },
        "stream": {
            "text": accumulator.final_text,
            "finish_reason": accumulator.finish_reason.unwrap(),
            "usage": accumulator.usage.unwrap(),
            "raw_events": accumulator.raw_provider_events,
            "event_types": accumulator.events
                .iter()
                .map(|event| serde_json::to_value(event.event_type).unwrap())
                .collect::<Vec<_>>(),
        },
        "usage_cost": cost,
    })
}

fn request_tool_structured_output_observation() -> Value {
    let request = LlmRequest {
        model: "gpt-5.2".to_string(),
        provider: Some("openai".to_string()),
        messages: vec![Message::user("hello")],
        reasoning_effort: Some("medium".to_string()),
        response_format: Some(ResponseFormat::JsonSchema {
            json_schema: json!({"name": "Decision", "schema": {"type": "object"}}),
            strict: true,
        }),
        tools: Vec::new(),
        tool_choice: None,
        provider_options: BTreeMap::from([("trace".to_string(), json!("enabled"))]),
        metadata: BTreeMap::from([("run_id".to_string(), json!("run-1"))]),
    };
    let tool_call = ToolCall {
        id: "call-1".to_string(),
        name: "weather".to_string(),
        arguments: json!({"city": "NYC"}),
        raw_arguments: None,
        r#type: "function".to_string(),
    };
    let tool_result = ToolResult {
        tool_call_id: tool_call.id.clone(),
        content: json!({"forecast": "sunny"}),
        is_error: false,
    };
    let response_format = serde_json::to_value(request.response_format.as_ref().unwrap()).unwrap();
    let _round_tripped_response_format: ResponseFormat =
        serde_json::from_value(response_format.clone()).unwrap();

    json!({
        "request": {
            "model": request.model,
            "provider": request.provider,
            "messages": request.messages.iter().map(message_observation).collect::<Vec<_>>(),
            "response_format": response_format,
            "reasoning_effort": request.reasoning_effort,
            "provider_options": request.provider_options,
            "metadata": request.metadata,
        },
        "tool_call": tool_call,
        "tool_result": tool_result,
    })
}

fn message_observation(message: &Message) -> Value {
    let text = message
        .content
        .iter()
        .find_map(|part| match part {
            unified_llm_adapter::ContentPart::Text { text } => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default();
    json!({
        "role": message.role,
        "text": text,
    })
}

fn assert_fixture_observation(fixture_name: &str, actual: Value) {
    let fixture = fixture(fixture_name);
    let expected = fixture
        .get("observation")
        .unwrap_or_else(|| panic!("{fixture_name} fixture must include observation"));
    assert_json_matches_fixture(&actual, expected, fixture_name);
}

fn fixture(fixture_name: &str) -> Value {
    let path = fixture_path(fixture_name);
    serde_json::from_str(&fs::read_to_string(&path).unwrap_or_else(|source| {
        panic!("read active provider fixture {}: {source}", path.display())
    }))
    .unwrap_or_else(|source| panic!("parse active provider fixture {}: {source}", path.display()))
}

fn fixture_path(fixture_name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.spark/rust-rewrite/current/compat-fixtures/providers")
        .join(format!("{fixture_name}.json"))
}

fn assert_json_matches_fixture(actual: &Value, expected: &Value, path: &str) {
    match (actual, expected) {
        (Value::Object(actual), Value::Object(expected)) => {
            assert_eq!(
                actual.keys().collect::<Vec<_>>(),
                expected.keys().collect::<Vec<_>>(),
                "object keys differ at {path}"
            );
            for (key, expected_value) in expected {
                let next_path = format!("{path}.{key}");
                assert_json_matches_fixture(&actual[key], expected_value, &next_path);
            }
        }
        (Value::Array(actual), Value::Array(expected)) => {
            assert_eq!(
                actual.len(),
                expected.len(),
                "array length differs at {path}"
            );
            for (index, (actual_value, expected_value)) in actual.iter().zip(expected).enumerate() {
                let next_path = format!("{path}[{index}]");
                assert_json_matches_fixture(actual_value, expected_value, &next_path);
            }
        }
        (Value::Number(actual), Value::Number(expected))
            if actual.as_f64().is_some() || expected.as_f64().is_some() =>
        {
            let actual = actual.as_f64().unwrap();
            let expected = expected.as_f64().unwrap();
            assert!(
                (actual - expected).abs() <= 1e-12,
                "number differs at {path}: actual={actual:?}, expected={expected:?}"
            );
        }
        _ => assert_eq!(actual, expected, "value differs at {path}"),
    }
}

fn adapter_error_type(kind: AdapterErrorKind) -> &'static str {
    match kind {
        AdapterErrorKind::Provider => "ProviderError",
        AdapterErrorKind::Authentication => "AuthenticationError",
        AdapterErrorKind::AccessDenied => "AccessDeniedError",
        AdapterErrorKind::NotFound => "NotFoundError",
        AdapterErrorKind::InvalidRequest => "InvalidRequestError",
        AdapterErrorKind::RateLimit => "RateLimitError",
        AdapterErrorKind::Server => "ServerError",
        AdapterErrorKind::ContentFilter => "ContentFilterError",
        AdapterErrorKind::ContextLength => "ContextLengthError",
        AdapterErrorKind::QuotaExceeded => "QuotaExceededError",
        AdapterErrorKind::RequestTimeout => "RequestTimeoutError",
        AdapterErrorKind::Abort => "AbortError",
        AdapterErrorKind::Network => "NetworkError",
        AdapterErrorKind::Stream => "StreamError",
        AdapterErrorKind::InvalidToolCall => "InvalidToolCallError",
        AdapterErrorKind::UnsupportedToolChoice => "UnsupportedToolChoiceError",
        AdapterErrorKind::NoObjectGenerated => "NoObjectGeneratedError",
        AdapterErrorKind::Configuration => "ConfigurationError",
    }
}
