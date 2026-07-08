use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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
async fn review_routes_create_by_handle_and_review_flow_run_requests() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project dir");
    write_flow(&settings, "ops/review.dot", simple_flow());
    seed_conversation(
        &settings,
        project_path.to_str().expect("utf-8"),
        "conversation-http-review",
    );
    let app = build_app(settings.clone());

    let created = request_json(
        app.clone(),
        "POST",
        "/workspace/api/conversations/by-handle/amber-anchor/flow-run-requests",
        Some(json!({
            "flow_name": "ops/review.dot",
            "summary": "Run implementation.",
            "goal": "Run the tiny flow.",
            "launch_context": {"context.review": "approved"}
        })),
    )
    .await;
    assert_eq!(created.0, StatusCode::OK);
    assert_eq!(created.1["ok"], true);
    assert_eq!(created.1["conversation_id"], "conversation-http-review");
    let request_id = created.1["flow_run_request_id"]
        .as_str()
        .expect("request id");
    let project_paths = ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths(project_path.to_str().expect("utf-8"))
        .expect("paths");
    let sidecar = settings
        .projects_dir
        .join(project_paths.project_id)
        .join("flow-run-requests/conversation-http-review.json");
    assert!(sidecar.exists());

    let duplicate = request_json(
        app.clone(),
        "POST",
        "/workspace/api/conversations/by-handle/amber-anchor/flow-run-requests",
        Some(json!({
            "flow_name": "ops/review.dot",
            "summary": "Run implementation.",
            "goal": "Run the tiny flow.",
            "launch_context": {"context.review": "approved"}
        })),
    )
    .await;
    assert_eq!(duplicate.0, StatusCode::CONFLICT);

    let reviewed = request_json(
        app.clone(),
        "POST",
        &format!(
            "/workspace/api/conversations/conversation-http-review/flow-run-requests/{request_id}/review"
        ),
        Some(json!({
            "project_path": project_path.to_string_lossy(),
            "disposition": "approved",
            "message": "Approved."
        })),
    )
    .await;
    assert_eq!(reviewed.0, StatusCode::OK);
    let request = reviewed.1["flow_run_requests"]
        .as_array()
        .expect("requests")
        .iter()
        .find(|entry| entry["id"] == request_id)
        .expect("request");
    assert_eq!(request["status"], "launched");
    assert_eq!(request["review_message"], "Approved.");

    let unknown = request_json(
        app.clone(),
        "POST",
        "/workspace/api/conversations/by-handle/missing-handle/flow-run-requests",
        Some(json!({"flow_name": "ops/review.dot", "summary": "Run"})),
    )
    .await;
    assert_eq!(unknown.0, StatusCode::NOT_FOUND);
    assert!(unknown.1["detail"]
        .as_str()
        .expect("detail")
        .contains("Unknown conversation handle"));

    let missing_flow = request_json(
        app.clone(),
        "POST",
        "/workspace/api/conversations/by-handle/amber-anchor/flow-run-requests",
        Some(json!({"flow_name": "missing.dot", "summary": "Run"})),
    )
    .await;
    assert_eq!(missing_flow.0, StatusCode::NOT_FOUND);

    let malformed = request_text(
        app,
        "POST",
        "/workspace/api/conversations/by-handle/amber-anchor/flow-run-requests",
        "{not-json",
        Some("application/json"),
    )
    .await;
    assert_eq!(malformed.0, StatusCode::BAD_REQUEST);
    let malformed_body: Value = serde_json::from_str(&malformed.1).expect("json");
    assert!(malformed_body["detail"]
        .as_str()
        .expect("detail")
        .contains("Failed to parse"));
}

#[tokio::test]
async fn flow_run_request_review_executes_codergen_through_injected_rust_llm_client() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project dir");
    write_flow(&settings, "ops/review-rust-boundary.dot", codergen_flow());
    seed_conversation(
        &settings,
        project_path.to_str().expect("utf-8"),
        "conversation-http-review-boundary",
    );
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(RecordingAdapter::new(
        "openai_compatible",
        Arc::clone(&calls),
    ));
    let client = Client::new()
        .with_llm_profile_adapter(
            "frontier",
            ActiveLlmProfile::new("openai_compatible", Some("gpt-review-boundary".to_string())),
            adapter,
        )
        .expect("client");
    let app = build_app_with_rust_llm_client(settings, client);

    let created = request_json(
        app.clone(),
        "POST",
        "/workspace/api/conversations/by-handle/amber-anchor/flow-run-requests",
        Some(json!({
            "flow_name": "ops/review-rust-boundary.dot",
            "summary": "Run with Rust adapter.",
            "model": "gpt-review-boundary",
            "llm_provider": "OpenAI",
            "llm_profile": "frontier",
            "reasoning_effort": "HIGH"
        })),
    )
    .await;
    assert_eq!(created.0, StatusCode::OK);
    let request_id = created.1["flow_run_request_id"]
        .as_str()
        .expect("request id");

    let reviewed = request_json(
        app,
        "POST",
        &format!(
            "/workspace/api/conversations/conversation-http-review-boundary/flow-run-requests/{request_id}/review"
        ),
        Some(json!({
            "project_path": project_path.to_string_lossy(),
            "disposition": "approved",
            "message": "Approved."
        })),
    )
    .await;

    assert_eq!(reviewed.0, StatusCode::OK);
    let request_record = reviewed.1["flow_run_requests"]
        .as_array()
        .expect("requests")
        .iter()
        .find(|entry| entry["id"] == request_id)
        .expect("request");
    assert_eq!(request_record["status"], "launched");
    // The review response returns at run start; the codergen invocation
    // happens on the detached execution thread.
    wait_for_codergen_calls(&calls, 1);
    let requests = calls.lock().expect("calls");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.provider.as_deref(), Some("openai_compatible"));
    assert_eq!(request.model, "gpt-review-boundary");
    assert_eq!(
        request.messages,
        vec![Message::user("Write review route note")]
    );
    assert_eq!(request.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(request.metadata["spark.runtime.source"], json!("codergen"));
    assert_eq!(
        request.metadata["spark.runtime.provider"],
        json!("openai_compatible")
    );
    assert_eq!(
        request.metadata["spark.runtime.model"],
        json!("gpt-review-boundary")
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

#[tokio::test]
async fn proposed_plan_review_route_records_rejection() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project dir");
    seed_proposed_plan(
        &settings,
        project_path.to_str().expect("utf-8"),
        "conversation-plan-http",
    );
    let app = build_app(settings);

    let response = request_json(
        app,
        "POST",
        "/workspace/api/conversations/conversation-plan-http/proposed-plans/proposed-plan-inline/review",
        Some(json!({
            "project_path": project_path.to_string_lossy(),
            "disposition": "rejected",
            "review_note": "Needs work."
        })),
    )
    .await;

    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(response.1["proposed_plans"][0]["status"], "rejected");
    assert_eq!(
        response.1["proposed_plans"][0]["review_note"],
        "Needs work."
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

async fn request_text(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: &str,
    content_type: Option<&str>,
) -> (StatusCode, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(content_type) = content_type {
        builder = builder.header("content-type", content_type);
    }
    let request = builder
        .body(Body::from(body.to_string()))
        .expect("request body");
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8(bytes.to_vec()).expect("utf-8"))
}

fn seed_conversation(settings: &SparkSettings, project_path: &str, conversation_id: &str) {
    let registry = ProjectRegistry::new(&settings.data_dir);
    let project = registry
        .ensure_project_paths(project_path)
        .expect("project");
    let state_path = project
        .conversations_dir
        .join(conversation_id)
        .join("state.json");
    fs::create_dir_all(state_path.parent().expect("state parent")).expect("state dir");
    fs::write(
        state_path,
        serde_json::to_string_pretty(&json!({
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
            "title": "Review thread",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:01Z",
            "turns": [
                {"id": "turn-user", "role": "user", "content": "Run it.", "timestamp": "2026-01-01T00:00:00Z", "status": "complete", "kind": "message"},
                {"id": "turn-assistant", "role": "assistant", "content": "I can request that.", "timestamp": "2026-01-01T00:00:01Z", "status": "complete", "kind": "message"}
            ],
            "segments": []
        }))
        .expect("json"),
    )
    .expect("write state");
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

fn seed_proposed_plan(settings: &SparkSettings, project_path: &str, conversation_id: &str) {
    ConversationRepository::new(&settings.data_dir)
        .write_snapshot(&json!({
            "schema_version": 5,
            "revision": 0,
            "conversation_id": conversation_id,
            "conversation_handle": "",
            "project_path": project_path,
            "chat_mode": "plan",
            "provider": "codex",
            "model": null,
            "llm_profile": null,
            "reasoning_effort": null,
            "title": "Plan review",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:01Z",
            "turns": [
                {"id": "turn-user-plan", "role": "user", "content": "Draft plan.", "timestamp": "2026-01-01T00:00:00Z", "status": "complete", "kind": "message"},
                {"id": "turn-assistant-plan", "role": "assistant", "content": "Plan.", "timestamp": "2026-01-01T00:00:01Z", "status": "complete", "kind": "message"}
            ],
            "segments": [
                {
                    "id": "segment-plan-inline",
                    "turn_id": "turn-assistant-plan",
                    "order": 1,
                    "kind": "plan",
                    "role": "assistant",
                    "status": "complete",
                    "timestamp": "2026-01-01T00:00:01Z",
                    "updated_at": "2026-01-01T00:00:01Z",
                    "completed_at": "2026-01-01T00:00:01Z",
                    "content": "# Proposed Plan\n\nDo the work.",
                    "artifact_id": "proposed-plan-inline",
                    "source": {}
                }
            ],
            "event_log": [],
            "flow_run_requests": [],
            "flow_launches": [],
            "run_recoveries": [],
            "proposed_plans": [
                {
                    "id": "proposed-plan-inline",
                    "created_at": "2026-01-01T00:00:01Z",
                    "updated_at": "2026-01-01T00:00:01Z",
                    "title": "Proposed Plan",
                    "content": "# Proposed Plan\n\nDo the work.",
                    "project_path": project_path,
                    "conversation_id": conversation_id,
                    "source_turn_id": "turn-assistant-plan",
                    "status": "pending_review",
                    "source_segment_id": "segment-plan-inline"
                }
            ]
        }))
        .expect("write proposed plan");
}

fn write_flow(settings: &SparkSettings, name: &str, content: &str) {
    let path = settings.flows_dir.join(name);
    fs::create_dir_all(path.parent().expect("flow parent")).expect("flow dir");
    fs::write(path, content).expect("flow");
}

fn simple_flow() -> &'static str {
    r#"
    digraph Review {
      start [shape=Mdiamond]
      done [shape=Msquare]
      start -> done
    }
    "#
}

fn codergen_flow() -> &'static str {
    r#"
    digraph ReviewRustBoundary {
      start [shape=Mdiamond]
      task [shape=box, prompt="Write review route note"]
      done [shape=Msquare]
      start -> task -> done
    }
    "#
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
            message: Message::assistant("review route adapter response"),
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
        unimplemented!("workspace review route codergen uses complete")
    }
}

fn wait_for_codergen_calls<T>(calls: &std::sync::Arc<std::sync::Mutex<Vec<T>>>, expected: usize) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if calls.lock().expect("calls").len() >= expected {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "codergen backend was never invoked by the detached run",
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
