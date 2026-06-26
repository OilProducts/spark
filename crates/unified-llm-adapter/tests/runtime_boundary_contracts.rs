use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use unified_llm_adapter::{
    AdapterError, AdapterErrorKind, Client, FinishReason, Message, ProviderAdapter, Request,
    Response, StreamEvent, StreamEvents,
};

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
fn env_configured_client_defers_provider_transport_without_python_fallback() {
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
    assert!(error
        .message
        .contains("no Rust provider adapter is registered"));
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
        Ok(Box::new(
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
