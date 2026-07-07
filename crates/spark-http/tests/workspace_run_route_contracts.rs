use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use attractor_core::{CheckpointState, RunRecord};
use attractor_runtime::{CreateRunRequest, RunStore};
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
use spark_http::{build_app, build_app_with_rust_llm_client};
use spark_storage::{ConversationHandleRepository, ConversationRepository, ProjectRegistry};
use tower::ServiceExt;
use unified_llm_adapter::{
    ActiveLlmProfile, AdapterError, Client, FinishReason, Message, ProviderAdapter,
    Request as LlmRequest, Response, StreamEvents, Usage,
};

#[tokio::test]
async fn run_routes_launch_retry_continue_and_preserve_route_validation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    let other_path = temp.path().join("other");
    fs::create_dir_all(&project_path).expect("project dir");
    fs::create_dir_all(&other_path).expect("other dir");
    write_flow(&settings, "ops/run.yaml", simple_flow());
    seed_conversation(
        &settings,
        project_path.to_str().expect("utf-8"),
        "conversation-http-run",
    );
    seed_failed_run(&settings, "run-failed", &project_path);
    seed_completed_run(&settings, "run-source", &project_path);
    let app = build_app(settings.clone());

    let launched = request_json(
        app.clone(),
        "POST",
        "/workspace/api/runs/launch",
        Some(json!({
            "flow_name": "ops/run.yaml",
            "summary": "Launch from route.",
            "conversation_handle": "amber-anchor",
            "project_path": project_path.to_string_lossy(),
            "goal": "Route launch",
            "backend": "legacy-extra-field"
        })),
    )
    .await;
    assert_eq!(launched.0, StatusCode::OK);
    assert_eq!(launched.1["ok"], true);
    assert_eq!(launched.1["conversation_id"], "conversation-http-run");
    assert!(launched.1["flow_launch_id"]
        .as_str()
        .expect("launch id")
        .starts_with("flow-launch-"));

    let missing_launch_field = request_json(
        app.clone(),
        "POST",
        "/workspace/api/runs/launch",
        Some(json!({
            "summary": "Missing the flow name.",
            "project_path": project_path.to_string_lossy()
        })),
    )
    .await;
    assert_eq!(missing_launch_field.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(missing_launch_field.1["detail"][0]["type"], "missing");
    assert_eq!(
        missing_launch_field.1["detail"][0]["loc"],
        json!(["body", "flow_name"])
    );

    write_flow(
        &settings,
        "ops/invalid.yaml",
        "schema_version: '1'\nid: broken\nnodes: [",
    );
    let invalid_launch = request_json(
        app.clone(),
        "POST",
        "/workspace/api/runs/launch",
        Some(json!({
            "flow_name": "ops/invalid.yaml",
            "summary": "Launch invalid YAML.",
            "conversation_handle": "amber-anchor"
        })),
    )
    .await;
    assert_eq!(invalid_launch.0, StatusCode::INTERNAL_SERVER_ERROR);
    let invalid_launch_detail = invalid_launch.1["detail"].as_str().expect("detail");
    assert!(
        invalid_launch_detail.contains("Expected")
            || invalid_launch_detail.contains("parse")
            || invalid_launch_detail.contains("line")
    );

    let malformed_retry = request_raw(
        app.clone(),
        "POST",
        "/workspace/api/runs/run-failed/retry",
        "{",
    )
    .await;
    assert_eq!(malformed_retry.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(malformed_retry.1["detail"][0]["type"], "json_invalid");

    let retry_extra = request_json(
        app.clone(),
        "POST",
        "/workspace/api/runs/run-failed/retry",
        Some(json!({"conversation_handle": "amber-anchor", "extra": true})),
    )
    .await;
    assert_eq!(retry_extra.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(retry_extra.1["detail"][0]["type"], "extra_forbidden");
    assert_eq!(retry_extra.1["detail"][0]["loc"], json!(["body", "extra"]));

    let retry = request_json(
        app.clone(),
        "POST",
        "/workspace/api/runs/run-failed/retry",
        Some(json!({"conversation_handle": "amber-anchor"})),
    )
    .await;
    assert_eq!(retry.0, StatusCode::OK);
    assert_eq!(retry.1["ok"], true);
    assert_eq!(retry.1["operation"], "retry");
    assert_eq!(retry.1["run_id"], "run-failed");

    let continue_extra = request_json(
        app.clone(),
        "POST",
        "/workspace/api/runs/run-source/continue",
        Some(json!({
            "start_node": "task",
            "flow_source_mode": "snapshot",
            "conversation_handle": "amber-anchor",
            "extra": true
        })),
    )
    .await;
    assert_eq!(continue_extra.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(continue_extra.1["detail"][0]["type"], "extra_forbidden");

    let continue_missing_mode = request_json(
        app.clone(),
        "POST",
        "/workspace/api/runs/run-source/continue",
        Some(json!({
            "start_node": "task",
            "conversation_handle": "amber-anchor"
        })),
    )
    .await;
    assert_eq!(continue_missing_mode.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(continue_missing_mode.1["detail"][0]["type"], "missing");
    assert_eq!(
        continue_missing_mode.1["detail"][0]["loc"],
        json!(["body", "flow_source_mode"])
    );

    let mismatch = request_json(
        app.clone(),
        "POST",
        "/workspace/api/runs/run-source/continue",
        Some(json!({
            "start_node": "task",
            "flow_source_mode": "snapshot",
            "conversation_handle": "amber-anchor",
            "project_path": other_path.to_string_lossy()
        })),
    )
    .await;
    assert_eq!(mismatch.0, StatusCode::BAD_REQUEST);
    assert!(mismatch.1["detail"]
        .as_str()
        .expect("detail")
        .contains("does not match"));

    let continued = request_json(
        app.clone(),
        "POST",
        "/workspace/api/runs/run-source/continue",
        Some(json!({
            "start_node": "task",
            "flow_source_mode": "snapshot",
            "conversation_handle": "amber-anchor",
            "project_path": project_path.to_string_lossy(),
            "flow_name": "ignored.yaml"
        })),
    )
    .await;
    assert_eq!(continued.0, StatusCode::OK);
    assert_eq!(continued.1["ok"], true);
    assert_eq!(continued.1["operation"], "continue");
    assert_eq!(continued.1["continued_from_run_id"], "run-source");
    assert!(continued.1.get("flow_name").is_none());

    let snapshot = ConversationRepository::new(&settings.data_dir)
        .read_snapshot(
            "conversation-http-run",
            Some(project_path.to_str().expect("utf-8")),
        )
        .expect("read snapshot")
        .expect("snapshot");
    assert!(snapshot["run_recoveries"]
        .as_array()
        .expect("recoveries")
        .iter()
        .any(|entry| entry["operation"] == "retry"));
    assert!(snapshot["run_recoveries"]
        .as_array()
        .expect("recoveries")
        .iter()
        .any(|entry| entry["operation"] == "continue"));
    assert!(snapshot["flow_launches"]
        .as_array()
        .expect("launches")
        .iter()
        .any(|entry| entry["flow_name"] == "ops/run.yaml" && entry["status"] == "launched"));
    assert!(!snapshot["flow_launches"]
        .as_array()
        .expect("launches")
        .iter()
        .any(|entry| entry["flow_name"] == "ops/invalid.yaml"));
}

#[tokio::test]
async fn run_launch_route_executes_codergen_through_injected_rust_llm_client() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project dir");
    write_flow(&settings, "ops/rust-boundary.yaml", simple_flow());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(RecordingAdapter::new(
        "openai_compatible",
        Arc::clone(&calls),
    ));
    let client = Client::new()
        .with_llm_profile_adapter(
            "frontier",
            ActiveLlmProfile::new("openai_compatible", Some("gpt-route-boundary".to_string())),
            adapter,
        )
        .expect("client");
    let app = build_app_with_rust_llm_client(settings, client);

    let launched = request_json(
        app,
        "POST",
        "/workspace/api/runs/launch",
        Some(json!({
            "flow_name": "ops/rust-boundary.yaml",
            "summary": "Launch through the Rust adapter boundary.",
            "project_path": project_path.to_string_lossy(),
            "model": "gpt-route-boundary",
            "llm_provider": "OpenAI",
            "llm_profile": "frontier",
            "reasoning_effort": "HIGH"
        })),
    )
    .await;

    assert_eq!(launched.0, StatusCode::OK);
    assert_eq!(launched.1["ok"], true);
    assert_eq!(launched.1["status"], "started");
    let requests = calls.lock().expect("calls");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.provider.as_deref(), Some("openai_compatible"));
    assert_eq!(request.model, "gpt-route-boundary");
    assert_eq!(request.messages, vec![Message::user("Write a route note")]);
    assert_eq!(request.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(request.metadata["spark.runtime.source"], json!("codergen"));
    assert_eq!(
        request.metadata["spark.runtime.provider"],
        json!("openai_compatible")
    );
    assert_eq!(
        request.metadata["spark.runtime.model"],
        json!("gpt-route-boundary")
    );
    assert_eq!(
        request.metadata["spark.runtime.llm_profile"],
        json!("frontier")
    );
    assert_eq!(
        request.metadata["spark.runtime.reasoning_effort"],
        json!("high")
    );
}

async fn request_json(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    let request = if let Some(body) = body {
        builder = builder.header("content-type", "application/json");
        builder
            .body(Body::from(body.to_string()))
            .expect("request body")
    } else {
        builder.body(Body::empty()).expect("request body")
    };
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = serde_json::from_slice::<Value>(&bytes).expect("json");
    (status, value)
}

async fn request_raw(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: &str,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request body");
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = serde_json::from_slice::<Value>(&bytes).expect("json");
    (status, value)
}

fn seed_failed_run(settings: &SparkSettings, run_id: &str, project_path: &Path) {
    let mut record = RunRecord::new(run_id, project_path.to_string_lossy());
    record.flow_name = "ops/retry.yaml".to_string();
    record.status = "failed".to_string();
    record.last_error = "previous failure".to_string();
    RunStore::for_settings(settings)
        .create_run(CreateRunRequest {
            record,
            checkpoint: Some(checkpoint("task")),
            flow_source: Some(simple_flow().to_string()),
            flow_definition_json: Some(simple_flow().to_string()),
            ..CreateRunRequest::default()
        })
        .expect("failed run");
}

fn seed_completed_run(settings: &SparkSettings, run_id: &str, project_path: &Path) {
    let mut record = RunRecord::new(run_id, project_path.to_string_lossy());
    record.flow_name = "ops/source.yaml".to_string();
    record.status = "completed".to_string();
    RunStore::for_settings(settings)
        .create_run(CreateRunRequest {
            record,
            checkpoint: Some(checkpoint("task")),
            flow_source: Some(simple_flow().to_string()),
            flow_definition_json: Some(simple_flow().to_string()),
            ..CreateRunRequest::default()
        })
        .expect("completed run");
}

fn checkpoint(current_node: &str) -> CheckpointState {
    CheckpointState {
        timestamp: "2026-06-23T10:00:00Z".to_string(),
        current_node: current_node.to_string(),
        completed_nodes: vec!["start".to_string(), current_node.to_string()],
        context: [
            ("context.seed".to_string(), json!("workspace-route")),
            (
                "_attractor.node_outcomes".to_string(),
                json!({current_node: "fail"}),
            ),
        ]
        .into_iter()
        .collect(),
        retry_counts: Default::default(),
        logs: Vec::new(),
    }
}

fn seed_conversation(settings: &SparkSettings, project_path: &str, conversation_id: &str) {
    let registry = ProjectRegistry::new(&settings.data_dir);
    let project = registry
        .ensure_project_paths(project_path)
        .expect("project");
    write_legacy_conversation_files(
        &settings.data_dir,
        &json!({
            "schema_version": 5,
            "revision": 0,
            "conversation_id": conversation_id,
            "conversation_handle": "amber-anchor",
            "project_path": project_path,
            "chat_mode": "chat",
            "provider": "codex",
            "model": null,
            "llm_profile": null,
            "reasoning_effort": null,
            "title": "Run route thread",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:01Z",
            "turns": [
                {"id": "turn-assistant", "role": "assistant", "content": "Ready.", "timestamp": "2026-01-01T00:00:01Z", "status": "complete", "kind": "message"}
            ],
            "segments": [],
            "event_log": [],
            "flow_run_requests": [],
            "flow_launches": [],
            "run_recoveries": [],
            "proposed_plans": []
        }),
    );
    ConversationHandleRepository::new(&settings.data_dir)
        .ensure_conversation_handle(
            conversation_id,
            &project.project_id,
            project_path,
            "2026-01-01T00:00:00Z",
            Some("amber-anchor"),
        )
        .expect("handle");
}

fn write_flow(settings: &SparkSettings, name: &str, content: &str) {
    let path = settings.flows_dir.join(name);
    fs::create_dir_all(path.parent().expect("flow parent")).expect("flow dir");
    fs::write(path, content).expect("flow");
}

fn simple_flow() -> &'static str {
    "schema_version: '1'\nid: workspace-run-route\ntitle: Workspace Run Route\nnodes:\n  start:\n    kind: start\n  task:\n    kind: agent_task\n    label: Task\n    config:\n      kind: agent_task\n      prompt: Write a route note\n  done:\n    kind: exit\nedges:\n  - from: start\n    to: task\n  - from: task\n    to: done\n"
}

fn settings(root: &Path) -> SparkSettings {
    SparkSettings {
        project_root: root.join("project"),
        data_dir: root.join("spark-home"),
        config_dir: root.join("spark-home/config"),
        runtime_dir: root.join("spark-home/runtime"),
        logs_dir: root.join("spark-home/logs"),
        workspace_dir: root.join("spark-home/workspace"),
        projects_dir: root.join("spark-home/workspace/projects"),
        attractor_dir: root.join("spark-home/attractor"),
        runs_dir: root.join("spark-home/attractor/runs"),
        flows_dir: root.join("flows"),
        ui_dir: None,
        project_roots: Vec::<PathBuf>::new(),
    }
}

struct RecordingAdapter {
    name: &'static str,
    calls: Arc<Mutex<Vec<LlmRequest>>>,
}

impl RecordingAdapter {
    fn new(name: &'static str, calls: Arc<Mutex<Vec<LlmRequest>>>) -> Self {
        Self { name, calls }
    }
}

impl ProviderAdapter for RecordingAdapter {
    fn name(&self) -> &str {
        self.name
    }

    fn complete(&self, request: LlmRequest) -> Result<Response, AdapterError> {
        self.calls.lock().expect("calls").push(request.clone());
        Ok(Response {
            model: request.model.clone(),
            provider: request.provider.clone().unwrap_or_default(),
            message: Message::assistant("route adapter response"),
            finish_reason: FinishReason::Stop,
            usage: Usage {
                input_tokens: 2,
                output_tokens: 3,
                total_tokens: 5,
                ..Usage::default()
            },
            ..Response::default()
        })
    }

    fn stream(&self, _request: LlmRequest) -> Result<StreamEvents, AdapterError> {
        unimplemented!("workspace route codergen uses complete")
    }
}

/// Seed the pre-split legacy conversation layout by hand: core keys in
/// `state.json`, artifact arrays in the project-level sidecar files. The
/// repository migrates these on first read.
fn write_legacy_conversation_files(data_dir: &Path, snapshot: &serde_json::Value) {
    let object = snapshot.as_object().expect("snapshot object");
    let conversation_id = snapshot["conversation_id"]
        .as_str()
        .expect("conversation id");
    let project_path = snapshot["project_path"].as_str().expect("project path");
    let project = ProjectRegistry::new(data_dir)
        .ensure_project_paths(project_path)
        .expect("project paths");
    let root = project.conversations_dir.join(conversation_id);
    fs::create_dir_all(&root).expect("conversation dir");
    let mut core = object.clone();
    let artifact = |key: &str| object.get(key).cloned().unwrap_or_else(|| json!([]));
    for key in [
        "event_log",
        "flow_run_requests",
        "flow_launches",
        "run_recoveries",
        "proposed_plans",
    ] {
        core.remove(key);
    }
    fs::write(
        root.join("state.json"),
        serde_json::to_string_pretty(&serde_json::Value::Object(core)).expect("state json"),
    )
    .expect("state.json");
    for (dir, payload) in [
        (
            &project.flow_run_requests_dir,
            json!({
                "conversation_id": conversation_id,
                "project_id": project.project_id,
                "project_path": project_path,
                "event_log": artifact("event_log"),
                "flow_run_requests": artifact("flow_run_requests"),
            }),
        ),
        (
            &project.flow_launches_dir,
            json!({
                "conversation_id": conversation_id,
                "project_id": project.project_id,
                "project_path": project_path,
                "flow_launches": artifact("flow_launches"),
                "run_recoveries": artifact("run_recoveries"),
            }),
        ),
        (
            &project.proposed_plans_dir,
            json!({
                "conversation_id": conversation_id,
                "project_id": project.project_id,
                "project_path": project_path,
                "proposed_plans": artifact("proposed_plans"),
            }),
        ),
    ] {
        fs::write(
            dir.join(format!("{conversation_id}.json")),
            serde_json::to_string_pretty(&payload).expect("sidecar json"),
        )
        .expect("sidecar");
    }
}
