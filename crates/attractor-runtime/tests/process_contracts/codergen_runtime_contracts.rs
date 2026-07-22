use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use attractor_core::ContextMap;
use attractor_core::FlowDefinition;
use attractor_runtime::{
    codergen_events_for_journal, flow_runtime::node_attrs_for_handler, outgoing_routing_edges,
    NodeExecutionRequest, NodeExecutor, RunRootPaths, RuntimeCodergen, RuntimeHandlerRunner,
};
use serde_json::json;
use spark_agent_adapter::RustLlmCodergenBackend;
use tempfile::tempdir;
use unified_llm_adapter::{
    stream_events, AdapterError, Client, FinishReason, Message, ProviderAdapter,
    Request as LlmRequest, Response, StreamEvent, StreamEvents, Usage,
};

#[test]
fn runtime_codergen_wrapper_writes_stage_artifacts_and_maps_events() {
    let graph = FlowDefinition::from_yaml_str(
        r#"
schema_version: '1'
id: g
title: G
goal: Ship
nodes:
  task:
    kind: agent_task
    config:
      kind: agent_task
      prompt: Plan for $goal
"#,
    )
    .expect("flow parses")
    .to_runtime_dot_graph();
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
        std::fs::read_to_string(logs_root.path().join("task/initial-context.txt")).unwrap(),
        "Plan for docs"
    );
    assert!(!logs_root.path().join("task/prompt.md").exists());
    assert_eq!(
        std::fs::read_to_string(logs_root.path().join("task/response.md"))
            .unwrap()
            .trim(),
        "[Simulated] Response for stage: task"
    );

    let events = codergen_events_for_journal("run-1", "task", &execution);
    assert_eq!(events[0].event_type, "LLMRequestStarted");
    assert_eq!(events[0].payload["node_id"], json!("task"));
}

#[test]
fn runtime_codergen_executes_text_only_rust_backend_through_public_api() {
    let graph = FlowDefinition::from_yaml_str(
        r#"
schema_version: '1'
id: g
title: G
goal: Ship
nodes:
  task:
    kind: agent_task
    config:
      kind: agent_task
      prompt: Summarize $goal
    execution:
      llm_provider: OpenAI
      llm_model: gpt-runtime-text
      reasoning_effort: HIGH
"#,
    )
    .expect("flow parses")
    .to_runtime_dot_graph();
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
        std::fs::read_to_string(logs_root.path().join("task/initial-context.txt")).unwrap(),
        "Summarize Rust evidence"
    );
    assert!(!logs_root.path().join("task/prompt.md").exists());
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
    assert!(journal_events
        .iter()
        .all(|event| event.payload["node_id"] == json!("task")));
    let request_started = journal_events
        .iter()
        .find(|event| event.event_type == "LLMRequestStarted")
        .expect("request started journal event");
    assert_eq!(
        request_started.payload["payload"]["runtime_mode"]["mode"],
        json!("text_only")
    );
    assert!(journal_events.iter().any(|event| {
        event.event_type == "LLMRequestCompleted"
            && event.payload["payload"]["runtime_mode"]["mode"] == json!("text_only")
    }));
}

#[test]
fn initial_context_persistence_failure_prevents_provider_execution() {
    let graph = FlowDefinition::from_yaml_str(
        r#"
schema_version: '1'
id: g
title: G
nodes:
  task:
    kind: agent_task
    config:
      kind: agent_task
      prompt: Never submit this
    execution:
      llm_provider: OpenAI
      llm_model: blocked-model
"#,
    )
    .expect("flow parses")
    .to_runtime_dot_graph();
    let logs_root = tempdir().unwrap();
    std::fs::create_dir_all(logs_root.path().join("task/initial-context.txt"))
        .expect("blocking directory");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingProviderAdapter::new("openai", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], None).expect("client");
    let mut codergen = RuntimeCodergen::with_backend(
        graph,
        Some(logs_root.path().to_path_buf()),
        RustLlmCodergenBackend::new(client),
    );

    let error = codergen
        .execute("task", ContextMap::new())
        .expect_err("capture must fail");

    assert!(matches!(
        error,
        spark_agent_adapter::CodergenError::Artifact(_)
    ));
    assert!(calls.lock().expect("calls").is_empty());
}

#[test]
fn runtime_codergen_executes_agent_required_rust_session_backend_through_public_api() {
    let graph = FlowDefinition::from_yaml_str(
        r#"
schema_version: '1'
id: g
title: G
nodes:
  task:
    kind: agent_task
    config:
      kind: agent_task
      prompt: Inspect with tools
    execution:
      llm_provider: openai-compatible
      llm_model: gpt-runtime-agent
    extensions:
      codergen.requires_tools: true
      codergen.requires_session_events: true
"#,
    )
    .expect("flow parses")
    .to_runtime_dot_graph();
    let logs_root = tempdir().unwrap();
    let project = tempdir().unwrap();
    std::fs::write(
        project.path().join("AGENTS.md"),
        "Keep the captured project instructions byte-exact.",
    )
    .expect("project instructions");
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
    let captured = std::fs::read_to_string(logs_root.path().join("task/initial-context.txt"))
        .expect("initial context");
    assert_eq!(
        captured,
        request
            .messages
            .iter()
            .map(Message::text)
            .collect::<String>()
    );
    assert!(captured.contains("Keep the captured project instructions byte-exact."));
    assert_eq!(captured.matches("<tools>").count(), 1);
    assert!(captured.ends_with("Inspect with tools"));
    assert!(!captured.ends_with('\n'));
    assert!(!captured.contains("\n---\n"));
    assert!(!logs_root.path().join("task/prompt.md").exists());
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
        event.event_type == "LLMRequestStarted"
            && event.payload["payload"]["runtime_mode"]["mode"] == json!("agent")
    }));
    assert!(journal_events.iter().any(|event| {
        event.event_type == "LLMTokenUsage"
            && event.payload["token_usage"]["total_tokens"] == json!(13)
    }));
    assert!(journal_events.iter().any(|event| {
        event.event_type == "LLMRequestCompleted"
            && event.payload["payload"]["runtime_mode"]["mode"] == json!("agent")
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

struct StreamingFakeBackend;

impl spark_agent_adapter::CodergenBackend for StreamingFakeBackend {
    fn run(
        &mut self,
        request: spark_agent_adapter::CodergenBackendRequest,
    ) -> Result<spark_agent_adapter::CodergenBackendOutput, spark_agent_adapter::CodergenError>
    {
        self.run_with_event_sink(request, None)
    }

    fn run_with_event_sink(
        &mut self,
        request: spark_agent_adapter::CodergenBackendRequest,
        event_sink: Option<spark_agent_adapter::CodergenEventSink>,
    ) -> Result<spark_agent_adapter::CodergenBackendOutput, spark_agent_adapter::CodergenError>
    {
        // Prefix contract: streamed events lead the returned batch, in order;
        // the terminal event is batch-only.
        let streamed = [
            spark_agent_adapter::CodergenEvent::new(
                "fake_stream_event",
                BTreeMap::from([
                    ("node_id".to_string(), json!(request.node_id.clone())),
                    ("step".to_string(), json!(1)),
                ]),
            ),
            spark_agent_adapter::CodergenEvent::new(
                "fake_stream_event",
                BTreeMap::from([
                    ("node_id".to_string(), json!(request.node_id.clone())),
                    ("step".to_string(), json!(2)),
                ]),
            ),
        ];
        let mut events = Vec::new();
        for event in streamed {
            if let Some(sink) = &event_sink {
                sink(event.clone());
            }
            events.push(event);
        }
        events.push(spark_agent_adapter::CodergenEvent::new(
            "fake_request_completed",
            BTreeMap::from([("node_id".to_string(), json!(request.node_id))]),
        ));
        Ok(spark_agent_adapter::CodergenBackendOutput {
            response: spark_agent_adapter::CodergenBackendResponse::Text("done".to_string()),
            events,
            usage: None,
        })
    }
}

#[test]
fn live_sink_receives_prefix_and_execution_keeps_full_event_list() {
    let graph = FlowDefinition::from_yaml_str(
        r#"
schema_version: '1'
id: g
title: G
goal: Ship
nodes:
  task:
    kind: agent_task
    config:
      kind: agent_task
      prompt: Plan for $goal
"#,
    )
    .expect("flow parses")
    .to_runtime_dot_graph();
    let logs_root = tempdir().unwrap();
    let mut codergen = RuntimeCodergen::with_boxed_backend(
        graph,
        Some(logs_root.path().to_path_buf()),
        Box::new(StreamingFakeBackend),
    );

    let streamed = Arc::new(Mutex::new(Vec::new()));
    let sink_events = Arc::clone(&streamed);
    let execution = codergen
        .execute_with_event_sink(
            "task",
            ContextMap::new(),
            Some(Arc::new(
                move |event: spark_agent_adapter::CodergenEvent| {
                    sink_events.lock().expect("streamed").push(event);
                },
            )),
        )
        .expect("codergen executes");

    let streamed = streamed.lock().expect("streamed").clone();
    let streamed_types: Vec<&str> = streamed
        .iter()
        .map(|event| event.event_type.as_str())
        .collect();
    let execution_types: Vec<&str> = execution
        .events
        .iter()
        .map(|event| event.event_type.as_str())
        .collect();

    // Everything the execution records was streamed exactly once, in order —
    // request lifecycle, backend prefix, batch-only tail, acceptance.
    assert_eq!(streamed_types, execution_types);
    assert_eq!(
        execution_types,
        vec![
            "codergen_backend_request_started",
            "fake_stream_event",
            "fake_stream_event",
            "fake_request_completed",
            "codergen_backend_response_accepted",
        ],
    );
    assert_eq!(streamed.len(), execution.events.len());
}

#[test]
fn codergen_external_sink_wins_over_reconstructed_run_paths() {
    let flow = FlowDefinition::from_yaml_str(
        r#"
schema_version: '1'
id: g
nodes:
  task:
    kind: agent_task
    config:
      kind: agent_task
      prompt: Ship it
"#,
    )
    .expect("flow parses");
    let node = flow.nodes.get("task").expect("task").clone();
    let temp = tempdir().unwrap();
    let paths = RunRootPaths::new(temp.path(), "project", "run-1").unwrap();
    std::fs::create_dir_all(paths.logs_dir()).unwrap();
    let streamed = Arc::new(Mutex::new(Vec::new()));
    let sink_events = Arc::clone(&streamed);
    let mut runner = RuntimeHandlerRunner::new()
        .with_codergen_backend_factory(|| Box::new(StreamingFakeBackend))
        .with_external_event_sink(move |event| {
            sink_events.lock().unwrap().push(event);
            Ok(())
        });

    runner
        .execute(NodeExecutionRequest {
            node_id: "task".to_string(),
            stage_index: 0,
            context: ContextMap::new(),
            prompt: "Ship it".to_string(),
            node_attrs: node_attrs_for_handler("task", &node),
            node,
            flow: flow.clone(),
            outgoing_edges: outgoing_routing_edges(&flow, "task").unwrap(),
            run_paths: Some(paths.clone()),
            run_workdir: temp.path().to_path_buf(),
            run_id: "run-1".to_string(),
            fallback_model: None,
            fallback_provider: None,
            fallback_profile: None,
            fallback_reasoning_effort: None,
        })
        .expect("codergen executes");

    let events = streamed.lock().unwrap();
    assert!(!events.is_empty());
    assert!(events.iter().all(|event| event.run_id == "run-1"));
    assert!(events
        .iter()
        .all(|event| event.payload["node_id"] == json!("task")));
    assert!(!paths.events_jsonl().exists(), "worker must not journal");
    assert!(paths.logs_dir().join("task/response.md").exists());
}
