use std::collections::BTreeMap;
use std::path::Path;

use attractor_core::{
    FlowDefinition, FlowEdge, FlowNode, JournalEntry, LaunchContext, NodeConfig, NodeKind,
    RunRecord,
};
use attractor_runtime::usage::{estimate_model_cost, project_run_usage};
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
            "rust_agent_session_event",
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
                    config: Some(NodeConfig::Exit {}),
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
