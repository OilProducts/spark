use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use attractor_core::ContextMap;
use attractor_core::FlowDefinition;
use attractor_runtime::{
    flow_runtime::node_attrs_for_handler, outgoing_routing_edges, NodeExecutionRequest,
    NodeExecutor, RunRootPaths, RuntimeCodergen, RuntimeHandlerRunner,
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
    let graph = serde_json::from_value::<FlowDefinition>(json!({
        "schema_version": "2",
        "id": "g",
        "title": "G",
        "goal": "Ship",
        "nodes": {
            "task": {
                "kind": "agent_task",
                "config": {
                    "kind": "agent_task",
                    "prompt": "Plan for $goal"
                },
                "routes": {"done": {"to": "done", "description": "Planned."}}
            },
            "done": {"kind": "exit", "config": {"kind": "exit",
                "result": {"name": "completed", "outcome": "success"}}}
        }
    }))
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

    // Simulation completes with the node's single declared decision.
    assert_eq!(execution.outcome.status.as_str(), "completed");
    assert_eq!(execution.outcome.decision, "done");
    assert_eq!(
        std::fs::read_to_string(logs_root.path().join("task/initial-context.txt")).unwrap(),
        "Plan for docs"
    );
    assert_eq!(
        std::fs::read_to_string(logs_root.path().join("task/prompt.md")).unwrap(),
        "Plan for docs\n"
    );
    assert_eq!(
        std::fs::read_to_string(logs_root.path().join("task/response.md"))
            .unwrap()
            .trim(),
        "[Simulated] Response for stage: task"
    );
}

#[test]
fn runtime_codergen_executes_text_only_rust_backend_through_public_api() {
    let graph = serde_json::from_value::<FlowDefinition>(json!({
        "schema_version": "2",
        "id": "g",
        "title": "G",
        "goal": "Ship",
        "nodes": {
            "task": {
                "kind": "agent_task",
                "config": {
                    "kind": "agent_task",
                    "prompt": "Summarize $goal"
                },
                "execution": {
                    "llm_provider": "OpenAI",
                    "llm_model": "gpt-runtime-text",
                    "reasoning_effort": "FutureEffort"
                }
            }
        }
    }))
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

    assert_eq!(execution.outcome.status.as_str(), "completed");
    assert_eq!(
        execution.response_text,
        "runtime backend response for gpt-runtime-text"
    );
    assert_eq!(execution.usage.as_ref().expect("usage").total_tokens, 13);
    assert_eq!(
        std::fs::read_to_string(logs_root.path().join("task/initial-context.txt")).unwrap(),
        "Summarize Rust evidence"
    );
    assert_eq!(
        std::fs::read_to_string(logs_root.path().join("task/prompt.md")).unwrap(),
        "Summarize Rust evidence\n"
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
    assert_eq!(status["status"], json!("completed"));
    assert!(status.get("outcome").is_none(), "{status}");
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
    assert_eq!(request.reasoning_effort.as_deref(), Some("FutureEffort"));
    assert_eq!(request.metadata["spark.runtime.source"], json!("codergen"));
    assert_eq!(
        request.metadata["spark.runtime.provider_selector"],
        json!("openai")
    );
    assert_eq!(request.metadata["test.marker"], json!("runtime-codergen"));
}

#[test]
fn initial_context_persistence_failure_prevents_provider_execution() {
    let graph = serde_json::from_value::<FlowDefinition>(json!({
        "schema_version": "2",
        "id": "g",
        "title": "G",
        "nodes": {
            "task": {
                "kind": "agent_task",
                "config": {
                    "kind": "agent_task",
                    "prompt": "Never submit this"
                },
                "execution": {
                    "llm_provider": "OpenAI",
                    "llm_model": "blocked-model"
                }
            }
        }
    }))
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
    let graph = serde_json::from_value::<FlowDefinition>(json!({
        "schema_version": "2",
        "id": "g",
        "title": "G",
        "nodes": {
            "task": {
                "kind": "agent_task",
                "config": {
                    "kind": "agent_task",
                    "prompt": "Inspect with tools"
                },
                "execution": {
                    "llm_provider": "openai-compatible",
                    "llm_model": "gpt-runtime-agent"
                },
                "extensions": {
                    "codergen.requires_tools": true,
                    "codergen.requires_session_events": true
                }
            }
        }
    }))
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

    assert_eq!(execution.outcome.status.as_str(), "completed");
    assert_eq!(
        execution.response_text,
        "runtime backend response for gpt-runtime-agent"
    );
    assert_eq!(execution.usage.as_ref().expect("usage").total_tokens, 13);
    let status: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(logs_root.path().join("task/status.json")).unwrap(),
    )
    .expect("status json");
    assert_eq!(status["status"], json!("completed"));
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
    assert!(logs_root.path().join("task/prompt.md").is_file());
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
            resume_metadata: Default::default(),
        })
    }
}

#[test]
fn live_sink_receives_prefix_and_execution_keeps_full_event_list() {
    let graph = serde_json::from_value::<FlowDefinition>(json!({
        "schema_version": "2",
        "id": "g",
        "title": "G",
        "goal": "Ship",
        "nodes": {
            "task": {
                "kind": "agent_task",
                "config": {
                    "kind": "agent_task",
                    "prompt": "Plan for $goal"
                }
            }
        }
    }))
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
    let flow = serde_json::from_value::<FlowDefinition>(json!({
        "schema_version": "2",
        "id": "g",
        "nodes": {
            "task": {
                "kind": "agent_task",
                "config": { "kind": "agent_task", "prompt": "Ship it" }
            }
        }
    }))
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
            attempt: 0,
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
    assert!(paths
        .logs_dir()
        .join("task/executions/0-0/response.md")
        .exists());
}

struct ThinkingBackend;

impl spark_agent_adapter::CodergenBackend for ThinkingBackend {
    fn run(
        &mut self,
        _request: spark_agent_adapter::CodergenBackendRequest,
    ) -> Result<spark_agent_adapter::CodergenBackendOutput, spark_agent_adapter::CodergenError>
    {
        unreachable!("the handler streams")
    }

    fn run_with_event_sink(
        &mut self,
        _request: spark_agent_adapter::CodergenBackendRequest,
        event_sink: Option<spark_agent_adapter::CodergenEventSink>,
    ) -> Result<spark_agent_adapter::CodergenBackendOutput, spark_agent_adapter::CodergenError>
    {
        let sink = event_sink.expect("event sink");
        let thinking = |active, tokens| {
            spark_agent_adapter::CodergenEvent::new(
                "turn_event",
                BTreeMap::from([(
                    "turn_stream_event".to_string(),
                    json!(spark_common::events::TurnStreamEvent::thinking_progress(
                        active, tokens
                    )),
                )]),
            )
        };
        let recorded = spark_agent_adapter::CodergenEvent::new(
            "after_thinking",
            BTreeMap::from([("step".to_string(), json!(1))]),
        );
        sink(thinking(true, Some(50)));
        sink(thinking(false, Some(186)));
        sink(recorded.clone());
        // Not streamed: the live-only events must not shift the prefix count.
        let terminal = spark_agent_adapter::CodergenEvent::new(
            "batch_only",
            BTreeMap::from([("step".to_string(), json!(2))]),
        );
        let mut output = spark_agent_adapter::CodergenBackendOutput::text("Done");
        output.events = vec![recorded, terminal];
        Ok(output)
    }
}

#[test]
fn node_thinking_reaches_the_publisher_without_touching_the_run_record() {
    let flow = serde_json::from_value::<FlowDefinition>(json!({
        "schema_version": "2",
        "id": "g",
        "nodes": {
            "task": {
                "kind": "agent_task",
                "config": { "kind": "agent_task", "prompt": "Ship it" }
            }
        }
    }))
    .expect("flow parses");
    let node = flow.nodes.get("task").expect("task").clone();
    let temp = tempdir().unwrap();
    let paths = RunRootPaths::new(temp.path(), "project", "run-thinking").unwrap();
    std::fs::create_dir_all(paths.logs_dir()).unwrap();
    let seen_at_notice = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&seen_at_notice);
    let mut runner = RuntimeHandlerRunner::new()
        .with_codergen_backend_factory(|| Box::new(ThinkingBackend))
        .with_run_event_observer(Arc::new(move |run_id: &str| {
            observed
                .lock()
                .unwrap()
                .extend(attractor_runtime::events::take_node_thinking(run_id));
        }));

    runner
        .execute(NodeExecutionRequest {
            node_id: "task".to_string(),
            stage_index: 3,
            attempt: 1,
            context: ContextMap::new(),
            prompt: "Ship it".to_string(),
            node_attrs: node_attrs_for_handler("task", &node),
            node,
            flow: flow.clone(),
            outgoing_edges: outgoing_routing_edges(&flow, "task").unwrap(),
            run_paths: Some(paths.clone()),
            run_workdir: temp.path().to_path_buf(),
            run_id: "run-thinking".to_string(),
            fallback_model: None,
            fallback_provider: None,
            fallback_profile: None,
            fallback_reasoning_effort: None,
        })
        .expect("codergen executes");

    let thinking = seen_at_notice.lock().unwrap();
    assert_eq!(thinking.len(), 2);
    assert!(thinking[0].active);
    assert_eq!(
        (
            thinking[0].node_id.as_str(),
            thinking[0].stage_index,
            thinking[0].attempt,
            thinking[0].estimated_tokens
        ),
        ("task", 3, 1, Some(50))
    );
    assert!(!thinking[1].active);
    assert_eq!(thinking[1].estimated_tokens, Some(186));

    // Nothing about thinking was recorded for the run, and the backend's
    // batch-only event was still delivered after the live-only ones.
    let mut recorded = String::new();
    let mut pending = vec![paths.root.clone()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            if entry.path().is_dir() {
                pending.push(entry.path());
            } else if entry.path().extension().is_some_and(|ext| ext == "jsonl") {
                recorded.push_str(&std::fs::read_to_string(entry.path()).unwrap_or_default());
            }
        }
    }
    assert!(!recorded.contains("thinking_progress"), "{recorded}");
    assert!(recorded.contains("batch_only"), "{recorded}");
}

/// The manager loop's intervention path reaches a running Claude Code node
/// as a live message in its turn.
#[cfg(unix)]
#[test]
fn manager_loop_intervention_reaches_a_running_claude_code_node() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempdir().unwrap();
    let bin = temp.path().join("claude");
    std::fs::write(
        &bin,
        r#"#!/usr/bin/env python3
import json, sys
def send(x):
    print(json.dumps(x), flush=True)
sys.stdin.readline()
send({'type':'system','subtype':'init','session_id':'s1','model':'m','tools':[],'capabilities':['interrupt_receipt_v1']})
open('started','w').close()
m = json.loads(sys.stdin.readline())
assert m['type'] == 'user' and 'priority' not in m, m
open('steered','w').write(m['message']['content'])
send({'type':'result','subtype':'success','is_error':False,'result':'Steered','session_id':'s1','user_message_uuids':[m['uuid']]})
"#,
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("SPARK_CLAUDE_CODE_BIN", &bin);
    let graph = serde_json::from_value::<FlowDefinition>(json!({
        "schema_version": "2",
        "id": "g",
        "title": "G",
        "nodes": {
            "task": {
                "kind": "agent_task",
                "config": {
                    "kind": "agent_task",
                    "prompt": "Implement it"
                },
                "execution": {
                    "llm_provider": "claude-code",
                    "llm_model": "test-model"
                }
            }
        }
    }))
    .expect("flow parses")
    .to_runtime_dot_graph();
    let logs_root = tempdir().unwrap();
    let project = tempdir().unwrap();
    let backend = RustLlmCodergenBackend::with_intervention_broker(
        Client::new(),
        spark_agent_adapter::CodergenSessionInterventionBroker::shared(),
    );
    let mut codergen =
        RuntimeCodergen::with_backend(graph, Some(logs_root.path().to_path_buf()), backend)
            .with_runtime_context(
                Some(project.path().to_path_buf()),
                BTreeMap::from([
                    ("spark.runtime.run_id".to_string(), json!("child-claude")),
                    (
                        "spark.runtime.root_run_id".to_string(),
                        json!("root-claude"),
                    ),
                ]),
            );
    // The manager loop runs beside the node, as it does for a child run.
    let started_file = project.path().join("started");
    let manager = std::thread::spawn(move || {
        let started = std::time::Instant::now();
        while !started_file.exists() {
            assert!(started.elapsed() < std::time::Duration::from_secs(10));
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        RuntimeHandlerRunner::new()
            .with_rust_llm_client(Client::new())
            .request_child_intervention(attractor_runtime::ChildInterventionRequest {
                child_run_id: "child-claude".to_string(),
                message: "Fix the failing test.".to_string(),
                parent_run_id: "parent".to_string(),
                parent_node_id: "manager".to_string(),
                root_run_id: "root-claude".to_string(),
                reason: "child failed".to_string(),
                source: "manager_loop".to_string(),
                cycle: Some(1),
                target_node_id: Some("task".to_string()),
            })
    });
    let execution = codergen
        .execute("task", ContextMap::new())
        .expect("node executes");
    let result = manager.join().unwrap();

    assert_eq!(result.status, "delivered", "{result:?}");
    assert_eq!(execution.response_text, "Steered");
    assert_eq!(
        std::fs::read_to_string(project.path().join("steered")).unwrap(),
        "Fix the failing test."
    );
}

/// Streams an agent warning, a finished compaction and messages sent into
/// the turn, as adapters report them.
struct StatusEventsBackend;

impl spark_agent_adapter::CodergenBackend for StatusEventsBackend {
    fn run(
        &mut self,
        request: spark_agent_adapter::CodergenBackendRequest,
    ) -> Result<spark_agent_adapter::CodergenBackendOutput, spark_agent_adapter::CodergenError>
    {
        self.run_with_event_sink(request, None)
    }

    fn run_with_event_sink(
        &mut self,
        _request: spark_agent_adapter::CodergenBackendRequest,
        event_sink: Option<spark_agent_adapter::CodergenEventSink>,
    ) -> Result<spark_agent_adapter::CodergenBackendOutput, spark_agent_adapter::CodergenError>
    {
        let events = [
            json!({"kind": "warning", "message": "API error 529 (overloaded); retrying in 4s (attempt 2 of 10)"}),
            json!({"kind": "context_compaction_completed", "message": "Context compacted (21.6k \u{2192} 1.9k tokens)."}),
            json!({"kind": "session_start", "message": "session started"}),
            json!({"kind": "tool_call_completed", "tool_call": {"id": "toolu_sub_grep", "kind": "dynamic_tool",
                "title": "Grep", "file_paths": [], "parent_call_id": "toolu_agent"}}),
            json!({"kind": "user_message_delivered", "message": "Use the docs.", "source": {"item_id": "steer-1"}}),
            json!({"kind": "user_message_delivered", "message": "Too late.", "source": {"item_id": "steer-2"}}),
            json!({"kind": "user_message_not_delivered", "message": "Not sent: the turn ended before the agent read it.",
                "source": {"item_id": "steer-2"}}),
        ]
        .into_iter()
        .map(|event| {
            spark_agent_adapter::CodergenEvent::new(
                "turn_stream_event",
                BTreeMap::from([("turn_stream_event".to_string(), event)]),
            )
        })
        .collect::<Vec<_>>();
        if let Some(sink) = &event_sink {
            events.iter().cloned().for_each(|event| sink(event));
        }
        Ok(spark_agent_adapter::CodergenBackendOutput {
            response: spark_agent_adapter::CodergenBackendResponse::Text("done".to_string()),
            events,
            usage: None,
            resume_metadata: Default::default(),
        })
    }
}

#[test]
fn node_transcripts_keep_agent_events_and_messages_sent_into_the_turn() {
    let flow = serde_json::from_value::<FlowDefinition>(json!({
        "schema_version": "2",
        "id": "g",
        "nodes": {
            "task": {
                "kind": "agent_task",
                "config": { "kind": "agent_task", "prompt": "Ship it" }
            }
        }
    }))
    .expect("flow parses");
    let node = flow.nodes.get("task").expect("task").clone();
    let temp = tempdir().unwrap();
    let paths = RunRootPaths::new(temp.path(), "project", "run-1").unwrap();
    std::fs::create_dir_all(paths.logs_dir()).unwrap();
    RuntimeHandlerRunner::new()
        .with_codergen_backend_factory(|| Box::new(StatusEventsBackend))
        .with_external_event_sink(|_| Ok(()))
        .execute(NodeExecutionRequest {
            node_id: "task".to_string(),
            stage_index: 0,
            attempt: 0,
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

    let records =
        spark_storage::ActivityRepository::new(paths.logs_dir().join("task/executions/0-0"))
            .read_transcript_records()
            .unwrap();
    let segments = records
        .iter()
        .filter_map(|record| match record {
            spark_storage::TranscriptRecord::SegmentUpsert { segment, .. } => Some((
                segment.kind.as_str(),
                segment.content.as_str(),
                segment
                    .extra
                    .get("category")
                    .and_then(|value| value.as_str()),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    // Session start/end stay out of the transcript.
    assert_eq!(
        &segments[..2],
        [
            (
                "agent_event",
                "API error 529 (overloaded); retrying in 4s (attempt 2 of 10)",
                Some("warning")
            ),
            (
                "context_compaction",
                "Context compacted (21.6k \u{2192} 1.9k tokens).",
                None
            ),
        ]
    );
    assert!(!segments
        .iter()
        .any(|segment| segment.1 == "session started"));
    // A subagent's call keeps the call it nests under.
    let nested = records
        .iter()
        .find_map(|record| match record {
            spark_storage::TranscriptRecord::SegmentUpsert { segment, .. } => {
                segment.tool_call.clone()
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(nested["parent_call_id"], "toolu_agent");
    // A message sent into the turn shows as the user's; one the turn never
    // read is marked failed in place.
    let messages = records
        .iter()
        .filter_map(|record| match record {
            spark_storage::TranscriptRecord::SegmentUpsert { segment, .. }
                if segment.kind == "user_message" =>
            {
                Some((segment.id.as_str(), segment))
            }
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let read = messages["segment-user-message-steer-1"];
    assert_eq!(
        (
            read.role.as_str(),
            read.status.as_str(),
            read.content.as_str()
        ),
        ("user", "complete", "Use the docs.")
    );
    let unread = messages["segment-user-message-steer-2"];
    assert_eq!(
        (unread.status.as_str(), unread.content.as_str()),
        ("failed", "Too late.")
    );
    assert_eq!(
        unread.error.as_deref(),
        Some("Not sent: the turn ended before the agent read it.")
    );
    // It keeps the place it was sent.
    let sent_order = records
        .iter()
        .find_map(|record| match record {
            spark_storage::TranscriptRecord::SegmentUpsert { segment, .. }
                if segment.id == "segment-user-message-steer-2" =>
            {
                Some(segment.order)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(unread.order, sent_order);
    assert!(read.order < unread.order);
}

#[test]
fn bundled_implement_change_evaluate_prompt_lists_its_decisions() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../spark-assets/assets/flows/software-development/implement-change");
    let flow = attractor_dsl::read_flow_dir(&path).unwrap();
    let graph = attractor_runtime::flow_runtime::flow_graph_for_handler_compat(&flow);
    let logs_root = tempdir().unwrap();
    let mut codergen = RuntimeCodergen::simulation(graph, Some(logs_root.path().to_path_buf()));

    let execution = codergen
        .execute("evaluate", ContextMap::new())
        .expect("codergen executes");
    // Simulation never guesses among several declared decisions.
    assert!(execution.outcome.is_error(), "{:?}", execution.outcome);

    let prompt = std::fs::read_to_string(logs_root.path().join("evaluate/prompt.md")).unwrap();
    assert!(
        prompt.contains("Decisions (choose exactly one):"),
        "{prompt}"
    );
    let route = |decision: &str| {
        let route = flow.route("evaluate", decision).expect("declared route");
        let target = &flow.nodes[&route.to].label;
        format!(
            "- {}: goes to {}. {}",
            json!(decision),
            json!(target),
            route.description
        )
    };
    for decision in ["blocked", "fix", "ready", "unfinished"] {
        assert!(prompt.contains(&route(decision)), "{decision}\n{prompt}");
    }
    assert!(
        prompt.contains("- \"fix\": goes to \"Implement\"."),
        "{prompt}"
    );
    assert!(
        prompt.contains("- \"ready\": goes to \"Validate\"."),
        "{prompt}"
    );
}

/// Records the Claude Code session id each node is asked to resume and
/// hands out a new one per call (`h-<call>`). `check` answers `again` once,
/// then `done`; `resolve` answers `rereview` until its prompt marks that
/// target as out of visits (always, with `ignore_visit_budget`). A
/// design-change reviewer records [`lens_findings`]; `synthesize` forwards
/// every branch's findings from `parallel.results`. The first call of
/// `flaky` fails retryably. Every other node answers `done`.
#[derive(Clone, Default)]
struct SessionBackend {
    calls: Arc<Mutex<Vec<(String, Option<String>)>>>,
    prompts: Arc<Mutex<Vec<String>>>,
    ignore_visit_budget: bool,
}

const VISITS_USED: &str = "its target has used all its visits";

impl spark_agent_adapter::CodergenBackend for SessionBackend {
    fn run(
        &mut self,
        request: spark_agent_adapter::CodergenBackendRequest,
    ) -> Result<spark_agent_adapter::CodergenBackendOutput, spark_agent_adapter::CodergenError>
    {
        let key = "spark.runtime.claude_code.session_id";
        self.prompts.lock().unwrap().push(request.prompt.clone());
        let mut calls = self.calls.lock().unwrap();
        calls.push((
            request.node_id.clone(),
            request
                .metadata
                .get(key)
                .and_then(|v| v.as_str())
                .map(str::to_string),
        ));
        let checks = calls.iter().filter(|(node, _)| node == "check").count();
        let flaky = calls.iter().filter(|(node, _)| node == "flaky").count();
        let envelope = |decision: &str, updates: serde_json::Value| {
            spark_agent_adapter::CodergenBackendResponse::Text(
                json!({"decision": decision, "context_updates": updates}).to_string(),
            )
        };
        let response = if request.node_id == "flaky" && flaky == 1 {
            // The first attempt fails and is retried automatically.
            spark_agent_adapter::CodergenBackendResponse::Outcome(attractor_core::Outcome::error(
                attractor_core::FailureKind::Runtime,
                "transient",
                Some(true),
            ))
        } else if request.node_id == "a" {
            // A forged session handle would be a contract violation (see
            // agent_written_runtime_sessions_are_contract_violations_not_dropped_keys);
            // the runtime alone records the handle the backend resumes.
            envelope("done", json!({}))
        } else if DESIGN_REVIEWERS.contains(&request.node_id.as_str()) {
            envelope(
                "done",
                json!({"context.review.lens_findings": lens_findings(&request.node_id)}),
            )
        } else if request.node_id == "synthesize" {
            let findings = request.context["parallel.results"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|branch| branch["context"].get("context.review.lens_findings"))
                .flat_map(|findings| findings.as_array().unwrap().clone())
                .collect::<Vec<_>>();
            envelope("done", json!({"context.review.findings": findings}))
        } else {
            let decision = match (request.node_id.as_str(), checks) {
                ("check", 1) => "again",
                ("resolve", _)
                    if self.ignore_visit_budget || !request.prompt.contains(VISITS_USED) =>
                {
                    "rereview"
                }
                _ => "done",
            };
            spark_agent_adapter::CodergenBackendResponse::Outcome(
                attractor_core::Outcome::completed(decision),
            )
        };
        Ok(spark_agent_adapter::CodergenBackendOutput {
            response,
            events: Vec::new(),
            usage: None,
            resume_metadata: BTreeMap::from([(
                key.to_string(),
                json!(format!("h-{}", calls.len())),
            )]),
        })
    }
}

const DESIGN_REVIEWERS: [&str; 5] = [
    "codebase_check",
    "critic",
    "naive_reader",
    "readiness",
    "zoom_out",
];

/// Two findings, a Blocker and a Major, whose evidence ends well past the
/// 200 characters `last_response` keeps.
fn lens_findings(lens: &str) -> serde_json::Value {
    let padding = "the cited line does not match the code ".repeat(6);
    json!([
        format!("Blocker, Approach: {lens} problem. Evidence: {padding}{lens}-tail-1"),
        format!("Major, Risks: {lens} problem. Evidence: {padding}{lens}-tail-2"),
    ])
}

fn session_flow(nodes: serde_json::Value) -> FlowDefinition {
    attractor_dsl::validated_flow(
        serde_json::from_value(json!({"schema_version": "2", "id": "sessions", "nodes": nodes}))
            .expect("flow json"),
    )
    .expect("valid flow")
}

fn routes(routes: &[(&str, &str)]) -> serde_json::Value {
    routes
        .iter()
        .map(|(decision, to)| {
            (
                decision.to_string(),
                json!({"to": to, "description": format!("Choose {decision}.")}),
            )
        })
        .collect::<serde_json::Map<_, _>>()
        .into()
}

fn start_to(to: &str) -> serde_json::Value {
    json!({"kind": "start", "routes": routes(&[("next", to)])})
}

fn exit_node() -> serde_json::Value {
    json!({"kind": "exit", "config": {"kind": "exit",
        "result": {"name": "completed", "outcome": "success"}}})
}

/// An agent task whose single decision `done` goes to `to`.
fn agent_in(session: Option<&str>, to: &str) -> serde_json::Value {
    agent_routes(session, &[("done", to)])
}

fn agent_routes(session: Option<&str>, decisions: &[(&str, &str)]) -> serde_json::Value {
    json!({"kind": "agent_task", "config": {"kind": "agent_task", "prompt": "work"},
        "contracts": {"response": "decision_envelope"},
        "runtime": session.map(|s| json!({"session": s})).unwrap_or(json!({})),
        "routes": routes(decisions)})
}

fn run_sessions(
    temp: &std::path::Path,
    run_id: &str,
    flow: FlowDefinition,
    backend: &SessionBackend,
    start: attractor_runtime::ExecutionStart,
) -> attractor_runtime::PipelineExecutionResult {
    let backend = backend.clone();
    let runner = RuntimeHandlerRunner::new()
        .with_codergen_backend_factory(move || Box::new(backend.clone()));
    attractor_runtime::PipelineExecutor::new(runner)
        .execute(attractor_runtime::ExecuteRunRequest {
            store: attractor_runtime::RunStore::for_runs_dir(temp.join("runs")),
            record: attractor_core::RunRecord::new(run_id, temp.to_string_lossy()),
            flow,
            flow_definition_json: None,
            launch_context: attractor_core::LaunchContext::empty(),
            runtime_context: ContextMap::from([(
                "internal.bundled_root".to_string(),
                json!(temp.to_string_lossy()),
            )]),
            max_steps: None,
            start,
        })
        .expect("run executes")
}

#[test]
fn nodes_sharing_a_session_continue_it_across_nodes_loops_and_checkpoints() {
    let temp = tempdir().unwrap();
    let mut nodes = json!({
        "start": start_to("a"), "done": exit_node(),
        "a": agent_in(Some("s"), "b"), "b": agent_in(Some("s"), "check"),
        "check": agent_routes(None, &[("again", "a"), ("done", "c")]),
        "c": agent_in(Some("s"), "done"),
    });
    nodes["c"]["execution"] = json!({"llm_provider": "other-provider"});
    let flow = session_flow(nodes);
    let backend = SessionBackend::default();
    let result = run_sessions(
        temp.path(),
        "run-sessions",
        flow.clone(),
        &backend,
        Default::default(),
    );

    let handle = |value: &str| Some(value.to_string());
    assert_eq!(
        *backend.calls.lock().unwrap(),
        vec![
            ("a".to_string(), None),
            ("b".to_string(), handle("h-1")),
            ("check".to_string(), None),
            ("a".to_string(), handle("h-2")),
            ("b".to_string(), handle("h-4")),
            ("check".to_string(), None),
        ]
    );
    // `c` resolves another provider than the session started with.
    assert_eq!(result.status, "failed");
    assert!(
        result
            .failure_reason
            .contains("session 's' was started with provider"),
        "{}",
        result.failure_reason
    );
    let sessions = result.context["_attractor.runtime.sessions"].clone();
    assert_eq!(
        sessions["named"]["s"]["resume"]["spark.runtime.claude_code.session_id"],
        "h-5"
    );

    // A run restored from that checkpoint context continues the session.
    backend.calls.lock().unwrap().clear();
    let checkpoint = attractor_core::CheckpointState {
        timestamp: attractor_runtime::events::utc_timestamp(),
        current_node: "b".to_string(),
        completed_nodes: vec!["start".to_string(), "a".to_string()],
        context: result.context.clone(),
        retry_counts: BTreeMap::new(),
        visit_counts: Default::default(),
        logs: Vec::new(),
    };
    run_sessions(
        temp.path(),
        "run-sessions-continued",
        flow,
        &backend,
        attractor_runtime::ExecutionStart::FromCheckpoint {
            start_node: "b".to_string(),
            checkpoint,
            flow_files: None,
        },
    );
    assert_eq!(
        backend.calls.lock().unwrap().first(),
        Some(&("b".to_string(), handle("h-5")))
    );
}

fn bundled_design_change() -> FlowDefinition {
    let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../spark-assets/assets");
    attractor_dsl::resolve_flow_roles(
        &attractor_dsl::read_flow_dir(&assets.join("flows/software-development/design-change"))
            .unwrap(),
        &attractor_dsl::FlowRoots::new(&assets, None),
    )
    .unwrap()
}

#[test]
fn bundled_design_change_resolves_in_the_drafters_session_until_review_visits_run_out() {
    let flow = bundled_design_change();
    let review_visits = attractor_runtime::routing::max_visits(&flow, "review") as usize;
    let temp = tempdir().unwrap();
    let backend = SessionBackend::default();
    // `resolve` asks for a re-review until its prompt says `review` is out
    // of visits, then chooses `done`.
    let result = run_sessions(
        temp.path(),
        "run-design",
        flow,
        &backend,
        Default::default(),
    );
    assert_eq!(result.status, "completed", "{}", result.failure_reason);
    assert_eq!(result.current_node, "done");

    let calls = backend.calls.lock().unwrap().clone();
    let handles = |id: &str| {
        calls
            .iter()
            .enumerate()
            .filter(|(_, (node, _))| node == id)
            .map(|(index, (_, handle))| (format!("h-{}", index + 1), handle.clone()))
            .collect::<Vec<_>>()
    };
    // `resolve` continues `draft`'s session, then its own on later passes.
    let draft = handles("draft");
    let resolve = handles("resolve");
    assert_eq!(draft.len(), 1);
    assert_eq!(resolve.len(), review_visits);
    assert_eq!(resolve[0].1, Some(draft[0].0.clone()));
    for pass in 1..resolve.len() {
        assert_eq!(resolve[pass].1, Some(resolve[pass - 1].0.clone()));
    }
    // The review runs once per allowed visit; every reviewer call starts a
    // fresh session.
    assert_eq!(handles("synthesize").len(), review_visits);
    for id in DESIGN_REVIEWERS {
        let runs = handles(id);
        assert_eq!(runs.len(), review_visits, "{id} {calls:?}");
        assert!(runs.iter().all(|(_, handle)| handle.is_none()), "{id}");
    }
    let prompts = backend.prompts.lock().unwrap().clone();
    let prompts_of = |id: &str| {
        calls
            .iter()
            .zip(&prompts)
            .filter(|((node, _), _)| node == id)
            .map(|(_, prompt)| prompt.clone())
            .collect::<Vec<_>>()
    };
    // Every reviewer's complete findings, Blockers included, reach
    // `synthesize` and then `resolve` in every round.
    for id in DESIGN_REVIEWERS {
        for finding in lens_findings(id).as_array().unwrap() {
            let finding = finding.as_str().unwrap();
            assert!(finding.find(&format!("{id}-tail")).unwrap() > 200);
            for prompt in prompts_of("synthesize")
                .iter()
                .chain(&prompts_of("resolve"))
            {
                assert!(prompt.contains(finding), "{finding}\n{prompt}");
            }
        }
    }
    // Only the last `resolve` sees `rereview` marked out of visits.
    let resolve_prompts = prompts_of("resolve");
    let marked = format!("- \"rereview\": goes to \"Review Design\" ({VISITS_USED}");
    let (last, earlier) = resolve_prompts.split_last().unwrap();
    assert!(last.contains(&marked), "{last}");
    for prompt in earlier {
        assert!(!prompt.contains(&marked), "{prompt}");
    }
}

#[test]
fn choosing_an_exhausted_route_fails_the_run_instead_of_rerouting() {
    let temp = tempdir().unwrap();
    let flow = bundled_design_change();
    let review_visits = attractor_runtime::routing::max_visits(&flow, "review") as usize;
    let backend = SessionBackend {
        ignore_visit_budget: true,
        ..SessionBackend::default()
    };
    let result = run_sessions(
        temp.path(),
        "run-design-exhausted",
        flow,
        &backend,
        Default::default(),
    );

    assert_eq!(result.status, "failed");
    assert!(
        result.failure_reason.starts_with("visit_limit_reached"),
        "{}",
        result.failure_reason
    );
    let calls = backend.calls.lock().unwrap().clone();
    let count = |id: &str| calls.iter().filter(|(node, _)| node == id).count();
    assert_eq!(count("resolve"), review_visits);
    assert_eq!(count("synthesize"), review_visits);
    let store = attractor_runtime::RunStore::for_runs_dir(temp.path().join("runs"));
    let bundle = store
        .read_run_bundle("run-design-exhausted")
        .unwrap()
        .unwrap();
    let record = bundle.record.unwrap();
    assert_eq!(record.status, "failed");
    assert!(
        record.last_error.contains("visit_limit_reached"),
        "{}",
        record.last_error
    );
    // No `done` transition was substituted for the exhausted decision.
    assert!(!bundle
        .raw_events
        .iter()
        .any(|event| event.event_type == "StageTransition"
            && event.payload["node_id"] == "resolve"
            && event.payload["decision"] == "done"));
}

#[test]
fn sessions_started_in_parallel_branches_continue_after_fan_in_but_stay_hidden() {
    let temp = tempdir().unwrap();
    let mut nodes = json!({
        "start": start_to("fork"), "done": exit_node(),
        "fork": {"kind": "parallel",
            "config": {"kind": "parallel", "branches": ["aa", "b"], "join": "join"}},
        "join": {"kind": "fan_in", "config": {"kind": "fan_in"}, "routes": routes(&[("next", "c")])},
        "aa": agent_in(None, "join"), "b": agent_in(Some("s"), "join"),
        "c": agent_in(Some("s"), "done"),
    });
    nodes["c"]["contracts"]["reads_context"] = json!(["parallel.results"]);
    let flow = session_flow(nodes);
    let backend = SessionBackend::default();
    let result = run_sessions(
        temp.path(),
        "run-parallel-sessions",
        flow,
        &backend,
        Default::default(),
    );

    assert_eq!(result.status, "completed", "{}", result.failure_reason);
    let calls = backend.calls.lock().unwrap().clone();
    let b_handle = calls.iter().position(|(node, _)| node == "b").unwrap() + 1;
    assert_eq!(
        calls.last(),
        Some(&("c".to_string(), Some(format!("h-{b_handle}"))))
    );
    // Branch payloads, and the prompts that read them, carry no handles.
    let results = result.context["parallel.results"].to_string();
    assert!(results.contains("route_trace"), "{results}");
    assert!(
        !results.contains("_attractor.runtime.sessions"),
        "{results}"
    );
    let prompt = backend.prompts.lock().unwrap().last().unwrap().clone();
    assert!(prompt.contains("parallel.results"), "{prompt}");
    assert!(!prompt.contains("_attractor.runtime.sessions"), "{prompt}");
    assert!(!prompt.contains(&format!("h-{b_handle}")), "{prompt}");
}

#[test]
fn retries_and_crash_recovery_continue_the_failed_attempts_session() {
    let temp = tempdir().unwrap();
    let mut nodes = json!({
        "start": start_to("flaky"), "done": exit_node(),
        "flaky": agent_in(Some("s"), "b"), "b": agent_in(Some("s"), "done"),
    });
    nodes["flaky"]["retry"] = json!({"max_retries": 1});
    let flow = session_flow(nodes);
    let backend = SessionBackend::default();
    let runner = || {
        let backend = backend.clone();
        RuntimeHandlerRunner::new().with_codergen_backend_factory(move || Box::new(backend.clone()))
    };
    let store = attractor_runtime::RunStore::for_runs_dir(temp.path().join("runs"));
    let runtime_context = ContextMap::from([(
        "internal.bundled_root".to_string(),
        json!(temp.path().to_string_lossy()),
    )]);
    // Stop after `flaky`, as if the process died before `b` ran.
    let paused = attractor_runtime::PipelineExecutor::new(runner())
        .execute(attractor_runtime::ExecuteRunRequest {
            store: store.clone(),
            record: attractor_core::RunRecord::new("run-recover", temp.path().to_string_lossy()),
            flow: flow.clone(),
            flow_definition_json: None,
            launch_context: attractor_core::LaunchContext::empty(),
            runtime_context: runtime_context.clone(),
            max_steps: Some(2),
            start: Default::default(),
        })
        .expect("run executes");
    assert_ne!(paused.status, "completed", "{}", paused.failure_reason);
    let handle = |value: &str| Some(value.to_string());
    // The retried attempt resumes the failed attempt's handle.
    assert_eq!(
        *backend.calls.lock().unwrap(),
        vec![
            ("flaky".to_string(), None),
            ("flaky".to_string(), handle("h-1"))
        ]
    );

    // Recovery from the persisted checkpoint continues the session.
    let bundle = store.read_run_bundle("run-recover").unwrap().unwrap();
    let recovered = attractor_runtime::PipelineExecutor::new(runner())
        .execute(attractor_runtime::ExecuteRunRequest {
            store: store.clone(),
            record: bundle.record.unwrap(),
            flow,
            flow_definition_json: None,
            launch_context: attractor_core::LaunchContext::empty(),
            runtime_context,
            max_steps: None,
            start: attractor_runtime::ExecutionStart::Resume {
                paths: bundle.paths,
                checkpoint: bundle.checkpoint.unwrap(),
            },
        })
        .expect("run recovers");
    assert_eq!(
        recovered.status, "completed",
        "{}",
        recovered.failure_reason
    );
    assert_eq!(
        backend.calls.lock().unwrap().last(),
        Some(&("b".to_string(), handle("h-2")))
    );
}

#[test]
fn child_sessions_continue_per_subflow_node_across_reentries() {
    let temp = tempdir().unwrap();
    attractor_dsl::write_flow_dir(
        &temp.path().join("flows/child"),
        &session_flow(json!({
            "start": start_to("work"), "work": agent_in(Some("s"), "done"), "done": exit_node()
        })),
    )
    .expect("child flow");
    let subflow = |to: &str| {
        json!({"kind": "subflow",
            "config": {"kind": "subflow", "flow_ref": "child", "results": {"completed": "done"}},
            "manager": {"max_cycles": 1, "child_autostart": true},
            "routes": routes(&[("done", to)])})
    };
    let parent = session_flow(json!({
        "start": start_to("one"), "done": exit_node(),
        "one": subflow("check"), "two": subflow("done"),
        "check": agent_routes(None, &[("again", "one"), ("done", "two")]),
    }));
    let backend = SessionBackend::default();
    let result = run_sessions(
        temp.path(),
        "run-parent",
        parent,
        &backend,
        Default::default(),
    );
    assert_eq!(result.status, "completed", "{}", result.failure_reason);

    let work = backend
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(node, _)| node == "work")
        .map(|(_, handle)| handle.clone())
        .collect::<Vec<_>>();
    // `one`'s second child continues its first child's session; `two`'s
    // child starts its own.
    assert_eq!(work, vec![None, Some("h-1".to_string()), None]);
    let sessions = &result.context["_attractor.runtime.sessions"];
    assert!(sessions.get("named").is_none(), "{sessions}");
    assert_eq!(
        sessions["subflows"]["one"]["named"]["s"]["resume"]["spark.runtime.claude_code.session_id"],
        "h-3"
    );
    assert_eq!(
        sessions["subflows"]["two"]["named"]["s"]["resume"]["spark.runtime.claude_code.session_id"],
        "h-5"
    );
}

/// Records the `project_path` each agent call runs in.
#[derive(Clone, Default)]
struct WorkdirBackend {
    paths: Arc<Mutex<Vec<Option<std::path::PathBuf>>>>,
}

impl spark_agent_adapter::CodergenBackend for WorkdirBackend {
    fn run(
        &mut self,
        request: spark_agent_adapter::CodergenBackendRequest,
    ) -> Result<spark_agent_adapter::CodergenBackendOutput, spark_agent_adapter::CodergenError>
    {
        self.paths
            .lock()
            .unwrap()
            .push(request.project_path.clone());
        Ok(spark_agent_adapter::CodergenBackendOutput {
            response: spark_agent_adapter::CodergenBackendResponse::Outcome(
                attractor_core::Outcome::completed("done"),
            ),
            events: Vec::new(),
            usage: None,
            resume_metadata: BTreeMap::new(),
        })
    }
}

#[test]
fn agent_task_runs_in_the_directory_named_by_runtime_workdir_from() {
    let temp = tempdir().unwrap();
    std::fs::create_dir(temp.path().join("ws")).unwrap();
    let run = |workdir_from: Option<&str>, ws: Option<&str>| {
        let mut task = agent_in(None, "done");
        if let Some(key) = workdir_from {
            task["runtime"] = json!({"workdir_from": key});
        }
        let flow =
            session_flow(json!({"start": start_to("task"), "done": exit_node(), "task": task}));
        let launch = ws
            .map(|ws| ContextMap::from([("context.ws".to_string(), json!(ws))]))
            .unwrap_or_default();
        let backend = WorkdirBackend::default();
        let runner = RuntimeHandlerRunner::new().with_codergen_backend_factory({
            let backend = backend.clone();
            move || Box::new(backend.clone())
        });
        let result = attractor_runtime::PipelineExecutor::new(runner)
            .execute(attractor_runtime::ExecuteRunRequest {
                store: attractor_runtime::RunStore::for_runs_dir(temp.path().join("runs")),
                record: attractor_core::RunRecord::new(
                    format!("run-workdir-{}", ws.unwrap_or("none")),
                    temp.path().to_string_lossy(),
                ),
                flow,
                flow_definition_json: None,
                launch_context: attractor_core::LaunchContext::new(launch).unwrap(),
                runtime_context: ContextMap::new(),
                max_steps: None,
                start: Default::default(),
            })
            .expect("run executes");
        let paths = backend.paths.lock().unwrap().clone();
        (result, paths)
    };

    // The resolved, canonical directory becomes the agent's project path.
    let (result, paths) = run(Some("context.ws"), Some("ws"));
    assert_eq!(result.status, "completed", "{}", result.failure_reason);
    assert_eq!(
        paths,
        vec![Some(std::fs::canonicalize(temp.path().join("ws")).unwrap())]
    );

    // Without the field the agent runs in the run's working directory.
    let (result, paths) = run(None, Some("ws"));
    assert_eq!(result.status, "completed", "{}", result.failure_reason);
    assert_eq!(paths, vec![Some(temp.path().to_path_buf())]);

    // A missing key or directory fails the node before the agent runs.
    for ws in [None, Some("missing")] {
        let (result, paths) = run(Some("context.ws"), ws);
        assert_eq!(result.status, "failed");
        assert!(
            result.failure_reason.contains("runtime.workdir_from"),
            "{}",
            result.failure_reason
        );
        assert!(paths.is_empty());
    }
}
