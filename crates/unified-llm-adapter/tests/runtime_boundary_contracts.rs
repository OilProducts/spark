use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::json;
use unified_llm_adapter::{
    resolve_effective_llm_model, resolve_effective_llm_profile, resolve_effective_llm_provider,
    resolve_effective_reasoning_effort, resolve_high_level_provider_and_model, stream_events,
    ActiveLlmProfile, AdapterError, AdapterErrorKind, Client, FinishReason,
    HighLevelLlmResolutionInputs, LlmResolutionInputs, Message, ModelCapabilities, ProviderAdapter,
    Request, Response, StreamEvent, StreamEvents, DISPLAY_MODEL_PLACEHOLDER,
    RUNTIME_LAUNCH_MODEL_KEY, RUNTIME_LAUNCH_PROFILE_KEY, RUNTIME_LAUNCH_PROVIDER_KEY,
    RUNTIME_LAUNCH_REASONING_EFFORT_KEY,
};

#[test]
fn llm_resolution_applies_launch_before_fallback_and_omits_model_placeholder() {
    let context = BTreeMap::from([
        (
            RUNTIME_LAUNCH_MODEL_KEY.to_string(),
            json!(DISPLAY_MODEL_PLACEHOLDER),
        ),
        (RUNTIME_LAUNCH_PROVIDER_KEY.to_string(), json!("Gemini")),
        (
            RUNTIME_LAUNCH_PROFILE_KEY.to_string(),
            json!("launch-profile"),
        ),
        (
            RUNTIME_LAUNCH_REASONING_EFFORT_KEY.to_string(),
            json!("MEDIUM"),
        ),
    ]);
    let inputs = LlmResolutionInputs {
        node_model: Some(DISPLAY_MODEL_PLACEHOLDER.to_string()),
        node_reasoning_effort: Some("HIGH".to_string()),
        node_reasoning_is_default_placeholder: true,
        fallback_model: Some("backend-model".to_string()),
        fallback_provider: Some("OpenAI".to_string()),
        fallback_profile: Some("backend-profile".to_string()),
        fallback_reasoning_effort: Some("LOW".to_string()),
        ..LlmResolutionInputs::default()
    };

    assert_eq!(
        resolve_effective_llm_model(&inputs, &context).as_deref(),
        Some("backend-model")
    );
    assert_eq!(resolve_effective_llm_provider(&inputs, &context), "gemini");
    assert_eq!(
        resolve_effective_llm_profile(&inputs, &context).as_deref(),
        Some("launch-profile")
    );
    assert_eq!(
        resolve_effective_reasoning_effort(&inputs, &context).as_deref(),
        Some("medium")
    );
}

#[test]
fn high_level_resolution_omits_display_model_placeholder_before_request_building() {
    let resolved = resolve_high_level_provider_and_model(&HighLevelLlmResolutionInputs {
        provider: Some("openai_compatible".to_string()),
        model: Some(DISPLAY_MODEL_PLACEHOLDER.to_string()),
        active_profile: Some(ActiveLlmProfile::new(
            "openai_compatible",
            Some("profile-default".to_string()),
        )),
        client_default_provider: None,
        required_capabilities: ModelCapabilities::default(),
    })
    .expect("resolved model");

    assert_eq!(resolved.provider, "openai_compatible");
    assert_eq!(resolved.model, "profile-default");
}

#[test]
fn client_complete_and_stream_execute_through_registered_rust_adapter() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RuntimeBoundaryAdapter::new("m1_rust", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], Some("M1_RUST")).unwrap();

    let response = client
        .complete(Request {
            model: "complete-model".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap();
    assert_eq!(response.provider, "m1_rust");
    assert_eq!(response.model, "complete-model");
    assert_eq!(response.text(), "complete:m1_rust:complete-model");

    let events = client
        .stream(Request {
            model: "stream-model".to_string(),
            provider: Some("M1_RUST".to_string()),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        events,
        vec![
            StreamEvent::text_delta("stream:m1_rust:stream-model"),
            StreamEvent::finish(FinishReason::Stop, None),
        ]
    );

    assert_eq!(
        calls.lock().expect("runtime boundary calls").as_slice(),
        [
            "complete:m1_rust:complete-model",
            "stream:m1_rust:stream-model",
        ]
    );
}

#[test]
fn env_configured_native_client_defers_http_transport_without_python_fallback() {
    let env = BTreeMap::from([("OPENAI_API_KEY".to_string(), "test-key".to_string())]);
    let client = Client::from_env_map(&env, None).unwrap();
    assert_eq!(client.provider_names().collect::<Vec<_>>(), vec!["openai"]);
    assert_eq!(client.default_provider(), Some("openai"));

    let error = client
        .complete(Request {
            model: "gpt-5.2".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::Configuration);
    assert_eq!(error.provider.as_deref(), Some("openai"));
    assert!(error.message.contains("no HTTP transport is configured"));
}

struct RuntimeBoundaryAdapter {
    name: &'static str,
    calls: Arc<Mutex<Vec<String>>>,
}

impl RuntimeBoundaryAdapter {
    fn new(name: &'static str, calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self { name, calls }
    }

    fn record(&self, request: &Request, operation: &str) {
        self.calls
            .lock()
            .expect("runtime boundary calls")
            .push(format!(
                "{operation}:{}:{}",
                request.provider.as_deref().unwrap_or_default(),
                request.model
            ));
    }
}

impl ProviderAdapter for RuntimeBoundaryAdapter {
    fn name(&self) -> &str {
        self.name
    }

    fn complete(&self, request: Request) -> Result<Response, AdapterError> {
        self.record(&request, "complete");
        Ok(Response {
            model: request.model.clone(),
            provider: request.provider.clone().unwrap_or_default(),
            message: Message::assistant(format!(
                "complete:{}:{}",
                request.provider.unwrap_or_default(),
                request.model
            )),
            finish_reason: FinishReason::Stop,
            ..Response::default()
        })
    }

    fn stream(&self, request: Request) -> Result<StreamEvents, AdapterError> {
        self.record(&request, "stream");
        Ok(stream_events(
            vec![
                Ok(StreamEvent::text_delta(format!(
                    "stream:{}:{}",
                    request.provider.unwrap_or_default(),
                    request.model
                ))),
                Ok(StreamEvent::finish(FinishReason::Stop, None)),
            ]
            .into_iter(),
        ))
    }
}
