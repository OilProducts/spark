use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use attractor_core::ContextMap;
use attractor_dsl::parse_dot;
use attractor_runtime::{codergen_events_for_journal, RuntimeCodergen};
use serde_json::json;
use spark_agent_adapter::RustLlmCodergenBackend;
use tempfile::tempdir;
use unified_llm_adapter::{
    stream_events, AdapterError, Client, FinishReason, Message, ProviderAdapter,
    Request as LlmRequest, Response, StreamEvent, StreamEvents, Usage,
};

#[test]
fn runtime_codergen_wrapper_writes_stage_artifacts_and_maps_events() {
    let graph = parse_dot(
        r#"
        digraph G {
          graph [goal="Ship"];
          task [shape=box, prompt="Plan for $goal"];
        }
        "#,
    )
    .expect("dot parses");
    let logs_root = tempdir().unwrap();
    let mut codergen = RuntimeCodergen::simulation(graph, Some(logs_root.path().to_path_buf()));

    let execution = codergen
        .execute(
            "task",
            ContextMap::from([("graph.goal".to_string(), json!("docs"))]),
        )
        .expect("codergen executes");

    assert_eq!(execution.outcome.status.as_str(), "success");
    assert_eq!(
        std::fs::read_to_string(logs_root.path().join("task/prompt.md"))
            .unwrap()
            .trim(),
        "Plan for docs"
    );
    assert_eq!(
        std::fs::read_to_string(logs_root.path().join("task/response.md"))
            .unwrap()
            .trim(),
        "[Simulated] Response for stage: task"
    );

    let events = codergen_events_for_journal("run-1", "task", &execution);
    assert_eq!(events[0].event_type, "CodergenAdapter");
    assert_eq!(events[0].payload["node_id"], json!("task"));
    assert_eq!(
        events[0].payload["adapter_event_type"],
        json!("codergen_backend_request_started")
    );
}

#[test]
fn runtime_codergen_executes_text_only_rust_backend_through_public_api() {
    let graph = parse_dot(
        r#"
        digraph G {
          graph [goal="Ship"];
          task [
            shape=box,
            prompt="Summarize $goal",
            llm_provider="OpenAI",
            llm_model="gpt-runtime-text",
            reasoning_effort="HIGH"
          ];
        }
        "#,
    )
    .expect("dot parses");
    let logs_root = tempdir().unwrap();
    let project = tempdir().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingProviderAdapter::new("openai", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], None).expect("client");
    let backend = RustLlmCodergenBackend::new(client);
    let mut codergen =
        RuntimeCodergen::with_backend(graph, Some(logs_root.path().to_path_buf()), backend)
            .with_runtime_context(
                Some(project.path().to_path_buf()),
                BTreeMap::from([("test.marker".to_string(), json!("runtime-codergen"))]),
            );

    let execution = codergen
        .execute(
            "task",
            ContextMap::from([("graph.goal".to_string(), json!("Rust evidence"))]),
        )
        .expect("codergen executes");

    assert_eq!(execution.outcome.status.as_str(), "success");
    assert_eq!(
        execution.response_text,
        "runtime backend response for gpt-runtime-text"
    );
    assert_eq!(execution.usage.as_ref().expect("usage").total_tokens, 13);
    assert_eq!(
        std::fs::read_to_string(logs_root.path().join("task/prompt.md"))
            .unwrap()
            .trim(),
        "Summarize Rust evidence"
    );
    assert_eq!(
        std::fs::read_to_string(logs_root.path().join("task/response.md"))
            .unwrap()
            .trim(),
        "runtime backend response for gpt-runtime-text"
    );
    let status: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(logs_root.path().join("task/status.json")).unwrap(),
    )
    .expect("status json");
    assert_eq!(status["outcome"], json!("success"));
    assert_eq!(status["usage"]["total_tokens"], json!(13));

    let requests = calls.lock().expect("calls");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.provider.as_deref(), Some("openai"));
    assert_eq!(request.model, "gpt-runtime-text");
    assert_eq!(
        request.messages,
        vec![Message::user("Summarize Rust evidence")]
    );
    assert_eq!(request.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(request.metadata["spark.runtime.source"], json!("codergen"));
    assert_eq!(
        request.metadata["spark.runtime.provider_selector"],
        json!("openai")
    );
    assert_eq!(request.metadata["test.marker"], json!("runtime-codergen"));

    let journal_events = codergen_events_for_journal("run-text", "task", &execution);
    assert!(journal_events.iter().all(|event| {
        event.event_type == "CodergenAdapter" && event.payload["node_id"] == json!("task")
    }));
    let request_started = journal_events
        .iter()
        .find(|event| {
            event.payload["adapter_event_type"] == json!("codergen_backend_request_started")
        })
        .expect("request started journal event");
    assert_eq!(
        request_started.payload["payload"]["runtime_mode"]["mode"],
        json!("text_only")
    );
    assert!(journal_events.iter().any(|event| {
        event.payload["adapter_event_type"] == json!("rust_llm_adapter_request_completed")
            && event.payload["payload"]["runtime_mode"]["mode"] == json!("text_only")
    }));
    assert!(journal_events.iter().any(|event| {
        event.payload["adapter_event_type"] == json!("codergen_backend_response_accepted")
    }));
}

#[test]
fn runtime_codergen_executes_agent_required_rust_session_backend_through_public_api() {
    let graph = parse_dot(
        r#"
        digraph G {
          task [
            shape=box,
            prompt="Inspect with tools",
            codergen.requires_tools=true,
            codergen.requires_session_events=true,
            llm_provider="openai-compatible",
            llm_model="gpt-runtime-agent"
          ];
        }
        "#,
    )
    .expect("dot parses");
    let logs_root = tempdir().unwrap();
    let project = tempdir().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(RecordingProviderAdapter::new(
        "openai_compatible",
        Arc::clone(&calls),
    ));
    let client = Client::from_adapters(vec![adapter], None).expect("client");
    let backend = RustLlmCodergenBackend::new(client);
    let mut codergen =
        RuntimeCodergen::with_backend(graph, Some(logs_root.path().to_path_buf()), backend)
            .with_runtime_context(
                Some(project.path().to_path_buf()),
                BTreeMap::from([
                    ("spark.runtime.run_id".to_string(), json!("run-agent")),
                    ("spark.runtime.root_run_id".to_string(), json!("root-agent")),
                ]),
            );

    let execution = codergen
        .execute("task", ContextMap::new())
        .expect("codergen executes");

    assert_eq!(execution.outcome.status.as_str(), "success");
    assert_eq!(
        execution.response_text,
        "runtime backend response for gpt-runtime-agent"
    );
    assert_eq!(execution.usage.as_ref().expect("usage").total_tokens, 13);
    let status: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(logs_root.path().join("task/status.json")).unwrap(),
    )
    .expect("status json");
    assert_eq!(status["outcome"], json!("success"));
    assert_eq!(status["usage"]["input_tokens"], json!(5));
    assert_eq!(status["usage"]["output_tokens"], json!(8));
    assert_eq!(
        std::fs::read_to_string(logs_root.path().join("task/response.md"))
            .unwrap()
            .trim(),
        "runtime backend response for gpt-runtime-agent"
    );

    let requests = calls.lock().expect("calls");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.provider.as_deref(), Some("openai_compatible"));
    assert_eq!(request.model, "gpt-runtime-agent");
    assert_eq!(
        request.messages.last(),
        Some(&Message::user("Inspect with tools"))
    );
    assert!(!request.tools.is_empty());
    assert_eq!(
        request.metadata["spark.runtime.source"],
        json!("agent_turn")
    );
    assert_eq!(
        request.metadata["spark.runtime.project_path"],
        json!(project.path().to_string_lossy().to_string())
    );
    assert_eq!(
        request.metadata["spark.runtime.codergen.node_id"],
        json!("task")
    );
    assert_eq!(
        request.metadata["spark.runtime.codergen.runtime_mode"]["mode"],
        json!("agent")
    );

    let journal_events = codergen_events_for_journal("run-agent", "task", &execution);
    assert!(journal_events.iter().any(|event| {
        event.payload["adapter_event_type"] == json!("codergen_backend_request_started")
            && event.payload["payload"]["runtime_mode"]["mode"] == json!("agent")
    }));
    assert!(journal_events.iter().any(|event| {
        event.payload["adapter_event_type"] == json!("rust_agent_session_event")
            && event.payload["payload"]["kind"] == json!("model_usage_update")
            && event.payload["payload"]["token_usage"]["total_tokens"] == json!(13)
    }));
    assert!(journal_events.iter().any(|event| {
        event.payload["adapter_event_type"] == json!("rust_agent_adapter_request_completed")
            && event.payload["payload"]["runtime_mode"]["mode"] == json!("agent")
    }));
    assert!(journal_events.iter().any(|event| {
        event.payload["adapter_event_type"] == json!("codergen_backend_response_accepted")
    }));
}

struct RecordingProviderAdapter {
    name: &'static str,
    calls: Arc<Mutex<Vec<LlmRequest>>>,
}

impl RecordingProviderAdapter {
    fn new(name: &'static str, calls: Arc<Mutex<Vec<LlmRequest>>>) -> Self {
        Self { name, calls }
    }
}

impl ProviderAdapter for RecordingProviderAdapter {
    fn name(&self) -> &str {
        self.name
    }

    fn complete(&self, request: LlmRequest) -> Result<Response, AdapterError> {
        self.calls.lock().expect("calls").push(request.clone());
        Ok(recorded_response(&request))
    }

    fn stream(&self, request: LlmRequest) -> Result<StreamEvents, AdapterError> {
        self.calls.lock().expect("calls").push(request.clone());
        Ok(stream_events(
            vec![
                Ok(StreamEvent::text_delta(recorded_text(&request))),
                Ok(StreamEvent::finish(
                    FinishReason::Stop,
                    Some(recorded_usage()),
                )),
            ]
            .into_iter(),
        ))
    }
}

fn recorded_response(request: &LlmRequest) -> Response {
    Response {
        model: request.model.clone(),
        provider: request.provider.clone().unwrap_or_default(),
        message: Message::assistant(recorded_text(request)),
        finish_reason: FinishReason::Stop,
        usage: recorded_usage(),
        ..Response::default()
    }
}

fn recorded_text(request: &LlmRequest) -> String {
    format!("runtime backend response for {}", request.model)
}

fn recorded_usage() -> Usage {
    Usage {
        input_tokens: 5,
        output_tokens: 8,
        total_tokens: 13,
        cache_read_tokens: Some(2),
        ..Usage::default()
    }
}
