use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use attractor_core::{
    FlowDefinition, FlowEdge, FlowNode, JournalEntry, LaunchContext, NodeConfig, NodeKind,
    RunRecord,
};
use attractor_runtime::usage::{estimate_model_cost, project_run_usage, RunUsageAccumulator};
use attractor_runtime::{
    ExecuteRunRequest, ExecutionStart, PipelineExecutor, RunStore, RuntimeHandlerRunner,
};
use serde_json::{json, Value};

fn journal_entry(sequence: u64, raw_type: &str, payload: Value) -> JournalEntry {
    JournalEntry {
        id: format!("journal-{sequence}"),
        sequence,
        emitted_at: format!("2026-01-01T00:00:{sequence:02}Z"),
        kind: "other".to_string(),
        raw_type: raw_type.to_string(),
        severity: "info".to_string(),
        summary: String::new(),
        node_id: None,
        stage_index: None,
        source_scope: "root".to_string(),
        source_parent_node_id: None,
        source_flow_name: None,
        question_id: None,
        payload,
    }
}

fn completed_entry(sequence: u64, adapter_event_type: &str, payload: Value) -> JournalEntry {
    journal_entry(
        sequence,
        "CodergenAdapter",
        json!({
            "adapter_event_type": adapter_event_type,
            "payload": payload,
        }),
    )
}

#[test]
fn request_completed_events_aggregate_by_model_across_payload_dialects() {
    let entries = vec![
        // Normalized rust Usage shape.
        completed_entry(
            1,
            "rust_llm_adapter_request_completed",
            json!({
                "model": "gpt-5.3-codex",
                "token_usage": {"input_tokens": 1000, "output_tokens": 200, "total_tokens": 1200},
            }),
        ),
        // Codex camelCase-with-total wrapper, cached input included.
        completed_entry(
            2,
            "codex_app_server_request_completed",
            json!({
                "model": "gpt-5.3-codex",
                "token_usage": {"total": {"inputTokens": 3000, "cachedInputTokens": 2000, "outputTokens": 500}},
            }),
        ),
        // A second model without pricing.
        completed_entry(
            3,
            "rust_agent_adapter_request_completed",
            json!({
                "model": "local-llama",
                "token_usage": {"input_tokens": 400, "output_tokens": 100},
            }),
        ),
        // Non-usage adapter noise is ignored.
        completed_entry(
            4,
            "rust_agent_raw_log_line",
            json!({
                "model": "gpt-5.3-codex",
                "token_usage": {"input_tokens": 999_999},
            }),
        ),
    ];

    let breakdown = project_run_usage(&entries, "fallback-model").expect("usage");
    assert_eq!(breakdown.totals.input_tokens, 4400);
    assert_eq!(breakdown.totals.cached_input_tokens, 2000);
    assert_eq!(breakdown.totals.output_tokens, 800);
    assert_eq!(breakdown.totals.total_tokens, 5200);
    assert_eq!(breakdown.by_model.len(), 2);
    let codex = breakdown
        .by_model
        .get("gpt-5.3-codex")
        .expect("codex bucket");
    assert_eq!(codex.input_tokens, 4000);
    assert_eq!(codex.cached_input_tokens, 2000);
    assert_eq!(codex.output_tokens, 700);

    let cost = estimate_model_cost(&breakdown).expect("cost");
    assert_eq!(cost["currency"], "USD");
    assert_eq!(cost["status"], "partial_unpriced");
    assert_eq!(cost["unpriced_models"], json!(["local-llama"]));
    // gpt-5.3-codex: 2000 uncached * 1.75 + 2000 cached * 0.175 + 700 out * 14.00 per million.
    let expected = (2000.0 * 1.75 + 2000.0 * 0.175 + 700.0 * 14.0) / 1_000_000.0;
    let amount = cost["by_model"]["gpt-5.3-codex"]["amount"]
        .as_f64()
        .expect("amount");
    assert!(
        (amount - expected).abs() < 1e-9,
        "amount {amount} != {expected}"
    );
    assert_eq!(cost["by_model"]["local-llama"]["status"], "unpriced");

    // Wire shape matches what the frontend renders.
    let wire = breakdown.to_value();
    assert_eq!(wire["total_tokens"], 5200);
    assert_eq!(wire["by_model"]["gpt-5.3-codex"]["input_tokens"], 4000);
}

#[test]
fn in_flight_session_usage_is_latest_per_node_until_completion() {
    let event = |sequence, node: &str, event_type: &str, usage: Value| {
        journal_entry(
            sequence,
            "CodergenAdapter",
            json!({
                "node_id": node,
                "adapter_event_type": event_type,
                "payload": {"model": "gpt-5", "token_usage": usage},
            }),
        )
    };
    let entries = vec![
        event(
            1,
            "work",
            "codex_app_server_session_event",
            json!({"total": {"inputTokens": 10, "outputTokens": 2, "totalTokens": 12}}),
        ),
        event(
            2,
            "work",
            "codex_app_server_session_event",
            json!({"total": {"inputTokens": 20, "cachedInputTokens": 5, "outputTokens": 4, "totalTokens": 24}}),
        ),
        event(
            3,
            "other",
            "rust_agent_session_event",
            json!({"input_tokens": 3, "output_tokens": 1}),
        ),
    ];
    let live = project_run_usage(&entries, "fallback").expect("live usage");
    assert_eq!(live.totals.total_tokens, 28);
    assert_eq!(live.totals.cached_input_tokens, 5);

    let mut completed = entries;
    completed.push(event(4, "work", "codex_app_server_request_completed", json!({"total": {"inputTokens": 20, "cachedInputTokens": 5, "outputTokens": 4, "totalTokens": 24}})));
    let final_usage = project_run_usage(&completed, "fallback").expect("completed usage");
    assert_eq!(final_usage.totals.total_tokens, 28);
}

#[test]
fn legacy_token_usage_events_only_count_when_no_completed_event_carried_usage() {
    let legacy_only = vec![
        journal_entry(
            1,
            "LLMTokenUsage",
            json!({
                "node_id": "build",
                "token_usage": {"input_tokens": 100, "output_tokens": 50},
            }),
        ),
        journal_entry(
            2,
            "LLMTokenUsage",
            json!({
                "node_id": "verify",
                "token_usage": {"input_tokens": 30, "output_tokens": 10},
            }),
        ),
    ];
    let breakdown = project_run_usage(&legacy_only, "compat-model").expect("usage");
    assert_eq!(breakdown.totals.total_tokens, 190);
    assert_eq!(
        breakdown.by_model.keys().collect::<Vec<_>>(),
        vec!["compat-model"]
    );

    // With an authoritative completed event present, the duplicated legacy
    // usage events are ignored.
    let mixed = vec![
        journal_entry(
            1,
            "LLMTokenUsage",
            json!({
                "token_usage": {"input_tokens": 100, "output_tokens": 50},
            }),
        ),
        journal_entry(
            2,
            "LLMRequestCompleted",
            json!({
                "node_id": "build",
                "payload": {
                    "model": "gpt-5",
                    "token_usage": {"input_tokens": 100, "output_tokens": 50},
                },
            }),
        ),
    ];
    let breakdown = project_run_usage(&mixed, "compat-model").expect("usage");
    assert_eq!(breakdown.totals.total_tokens, 150);
    assert_eq!(breakdown.by_model.keys().collect::<Vec<_>>(), vec!["gpt-5"]);

    assert!(project_run_usage(&[], "compat-model").is_none());
}

#[test]
fn incremental_usage_matches_full_projection_at_every_cursor() {
    let event = |sequence, node: &str, event_type: &str, model: &str, total| {
        journal_entry(
            sequence,
            "CodergenAdapter",
            json!({
                "node_id": node,
                "adapter_event_type": event_type,
                "payload": {"model": model, "token_usage": {"input_tokens": total - 1, "output_tokens": 1}}
            }),
        )
    };
    let entries = vec![
        event(1, "build", "codex_app_server_session_event", "gpt-5", 10),
        event(2, "build", "codex_app_server_session_event", "gpt-5", 20),
        event(3, "verify", "rust_agent_session_event", "gpt-5.4", 7),
        event(
            4,
            "build",
            "codex_app_server_request_completed",
            "gpt-5",
            20,
        ),
        event(5, "build", "codex_app_server_request_completed", "gpt-5", 3),
        event(
            6,
            "verify",
            "rust_agent_adapter_request_completed",
            "gpt-5.4",
            7,
        ),
    ];
    let mut incremental = RunUsageAccumulator::new("fallback");
    for end in 1..=entries.len() {
        incremental.apply(&entries[end - 1..end]);
        assert_eq!(
            incremental.breakdown(),
            project_run_usage(&entries[..end], "fallback"),
            "cursor {end}"
        );
    }
}

#[test]
fn current_generation_models_are_priced() {
    // gpt-5.5: $5.00 input / $0.50 cached / $30.00 output per million,
    // per the official OpenAI pricing page (checked 2026-07-09).
    let entries = vec![completed_entry(
        1,
        "codex_app_server_request_completed",
        json!({
            "model": "gpt-5.5",
            "token_usage": {"total": {"inputTokens": 1_000_000, "cachedInputTokens": 400_000, "outputTokens": 100_000}},
        }),
    )];
    let breakdown = project_run_usage(&entries, "fallback").expect("usage");
    let cost = estimate_model_cost(&breakdown).expect("cost");
    assert_eq!(cost["status"], "estimated");
    // 600k uncached * 5.00 + 400k cached * 0.50 + 100k output * 30.00 per million.
    let expected = (600_000.0 * 5.00 + 400_000.0 * 0.50 + 100_000.0 * 30.00) / 1_000_000.0;
    let amount = cost["amount"].as_f64().expect("amount");
    assert!(
        (amount - expected).abs() < 1e-9,
        "amount {amount} != {expected}"
    );
}

struct UsageEmittingBackend;

impl spark_agent_adapter::CodergenBackend for UsageEmittingBackend {
    fn run(
        &mut self,
        request: spark_agent_adapter::CodergenBackendRequest,
    ) -> Result<spark_agent_adapter::CodergenBackendOutput, spark_agent_adapter::CodergenError>
    {
        Ok(spark_agent_adapter::CodergenBackendOutput {
            response: spark_agent_adapter::CodergenBackendResponse::Text(
                "{\"outcome\":\"success\"}".to_string(),
            ),
            events: vec![spark_agent_adapter::CodergenEvent::new(
                "rust_llm_adapter_request_completed",
                BTreeMap::from([
                    ("node_id".to_string(), json!(request.node_id)),
                    ("model".to_string(), json!("gpt-5")),
                    (
                        "token_usage".to_string(),
                        json!({"input_tokens": 800, "output_tokens": 200}),
                    ),
                ]),
            )],
            usage: None,
        })
    }
}

struct LiveUsageBackend {
    store: RunStore,
    observations: Arc<Mutex<Vec<(Option<u64>, String)>>>,
    calls: Arc<AtomicUsize>,
}

impl spark_agent_adapter::CodergenBackend for LiveUsageBackend {
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
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let run_id = request.metadata["spark.runtime.run_id"]
            .as_str()
            .expect("run id");
        if call > 0 {
            self.store
                .update_run_record(run_id, |record| {
                    record.status = "concurrent-status".to_string()
                })
                .expect("concurrent record update");
        }
        let event = spark_agent_adapter::CodergenEvent::new(
            "codex_app_server_session_event",
            BTreeMap::from([
                ("node_id".to_string(), json!(request.node_id)),
                ("model".to_string(), json!("gpt-5")),
                (
                    "token_usage".to_string(),
                    json!({"total": {"inputTokens": 8, "outputTokens": 2, "totalTokens": 10}}),
                ),
            ]),
        );
        if let Some(sink) = &event_sink {
            sink(event.clone());
        }
        if call == 0 {
            self.store
                .update_run_record(run_id, |record| {
                    record.status = "concurrent-status".to_string()
                })
                .expect("concurrent record update");
        }
        let record = self
            .store
            .read_run_bundle(run_id)
            .expect("read run")
            .and_then(|bundle| bundle.record)
            .expect("record");
        self.observations
            .lock()
            .expect("observations")
            .push((record.token_usage, record.status));
        Ok(spark_agent_adapter::CodergenBackendOutput {
            response: spark_agent_adapter::CodergenBackendResponse::Text(
                "{\"outcome\":\"success\"}".to_string(),
            ),
            events: vec![event],
            usage: None,
        })
    }
}

fn agent_flow() -> FlowDefinition {
    FlowDefinition {
        schema_version: "1".to_string(),
        id: "usage-contract".to_string(),
        title: "Usage Contract".to_string(),
        nodes: [
            (
                "start".to_string(),
                FlowNode {
                    kind: NodeKind::Start,
                    config: Some(NodeConfig::Start {}),
                    ..FlowNode::default()
                },
            ),
            (
                "build".to_string(),
                FlowNode {
                    kind: NodeKind::AgentTask,
                    config: Some(NodeConfig::AgentTask {
                        prompt: "build".to_string(),
                    }),
                    ..FlowNode::default()
                },
            ),
            (
                "done".to_string(),
                FlowNode {
                    kind: NodeKind::Exit,
                    config: Some(NodeConfig::Exit {
                        result_summary: false,
                        result_summary_prompt: None,
                    }),
                    ..FlowNode::default()
                },
            ),
        ]
        .into_iter()
        .collect(),
        edges: vec![
            FlowEdge {
                from: "start".to_string(),
                to: "build".to_string(),
                ..FlowEdge::default()
            },
            FlowEdge {
                from: "build".to_string(),
                to: "done".to_string(),
                ..FlowEdge::default()
            },
        ],
        ..FlowDefinition::default()
    }
}

fn record(run_id: &str, project_path: &Path) -> RunRecord {
    let mut record = RunRecord::new(run_id, project_path.to_string_lossy());
    record.flow_name = "usage-contract".to_string();
    record.model = "compat-model".to_string();
    record.started_at = "2026-07-09T10:00:00Z".to_string();
    record
}

#[test]
fn executed_runs_persist_token_usage_and_estimated_cost_on_the_record() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("spark-home/attractor/runs"));
    let project_path = temp.path().join("project");
    std::fs::create_dir_all(&project_path).expect("project dir");

    let runner = RuntimeHandlerRunner::new()
        .with_codergen_backend_factory(|| Box::new(UsageEmittingBackend));
    let mut executor = PipelineExecutor::new(runner);
    let result = executor
        .execute(ExecuteRunRequest {
            store: store.clone(),
            record: record("run-usage", &project_path),
            flow: agent_flow(),
            flow_source: None,
            flow_definition_json: None,
            launch_context: LaunchContext::empty(),
            runtime_context: Default::default(),
            max_steps: None,
            start: ExecutionStart::Fresh,
        })
        .expect("execute");
    assert_eq!(result.status, "completed");

    let paths = store
        .run_root(&project_path.to_string_lossy(), "run-usage")
        .expect("run root");
    let persisted = store
        .read_run_record(&paths)
        .expect("read record")
        .expect("record");
    assert_eq!(persisted.token_usage, Some(1000));
    let breakdown = persisted.token_usage_breakdown.expect("breakdown");
    assert_eq!(breakdown["total_tokens"], 1000);
    assert_eq!(breakdown["by_model"]["gpt-5"]["input_tokens"], 800);
    let cost = persisted.estimated_model_cost.expect("cost");
    assert_eq!(cost["status"], "estimated");
    // gpt-5: 800 input * 1.25 + 200 output * 10.00 per million.
    let expected = (800.0 * 1.25 + 200.0 * 10.0) / 1_000_000.0;
    let amount = cost["amount"].as_f64().expect("amount");
    assert!(
        (amount - expected).abs() < 1e-9,
        "amount {amount} != {expected}"
    );
}

#[test]
fn live_usage_does_not_write_run_record_mid_stage() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let project_path = temp.path().join("project");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let observations = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::new(AtomicUsize::new(0));
    let backend_store = store.clone();
    let backend_observations = Arc::clone(&observations);
    let backend_calls = Arc::clone(&calls);
    let runner = RuntimeHandlerRunner::new().with_codergen_backend_factory(move || {
        Box::new(LiveUsageBackend {
            store: backend_store.clone(),
            observations: Arc::clone(&backend_observations),
            calls: Arc::clone(&backend_calls),
        })
    });
    let mut flow = agent_flow();
    flow.nodes.insert(
        "verify".to_string(),
        FlowNode {
            kind: NodeKind::AgentTask,
            config: Some(NodeConfig::AgentTask {
                prompt: "verify".to_string(),
            }),
            ..FlowNode::default()
        },
    );
    flow.edges = vec![
        FlowEdge {
            from: "start".to_string(),
            to: "build".to_string(),
            ..Default::default()
        },
        FlowEdge {
            from: "build".to_string(),
            to: "verify".to_string(),
            ..Default::default()
        },
        FlowEdge {
            from: "verify".to_string(),
            to: "done".to_string(),
            ..Default::default()
        },
    ];
    PipelineExecutor::new(runner)
        .execute(ExecuteRunRequest {
            store,
            record: record("run-live-usage", &project_path),
            flow,
            flow_source: None,
            flow_definition_json: None,
            launch_context: LaunchContext::empty(),
            runtime_context: Default::default(),
            max_steps: None,
            start: ExecutionStart::Fresh,
        })
        .expect("execute");
    let observations = observations.lock().expect("observations");
    assert_eq!(observations.len(), 2);
    assert_eq!(observations[0], (None, "concurrent-status".to_string()));
    assert_eq!(observations[1], (Some(10), "concurrent-status".to_string()));
}
