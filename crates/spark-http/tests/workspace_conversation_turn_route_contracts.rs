use std::path::{Path, PathBuf};

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use spark_agent_adapter::AgentTurnOutput;
use spark_common::events::{TurnStreamEvent, TurnStreamEventKind, TurnStreamSource};
use spark_common::settings::SparkSettings;
use spark_http::build_app;
use spark_workspace::{ConversationTurnRequest, WorkspaceConversationService};
use tower::ServiceExt;

#[tokio::test]
async fn conversation_turn_route_persists_initial_turns_and_errors() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let app = build_app(settings.clone());

    let response = request_json(
        app.clone(),
        "POST",
        "/workspace/api/conversations/conversation-http-turn/turns",
        Some(json!({
            "project_path": "/projects/http-turn",
            "message": "Ship it",
            "provider": "openai",
            "model": "gpt-5",
            "chat_mode": "chat"
        })),
    )
    .await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(response.1["conversation_id"], "conversation-http-turn");
    assert_eq!(response.1["provider"], "openai");
    assert_eq!(response.1["model"], "gpt-5");
    assert_eq!(response.1["turns"].as_array().expect("turns").len(), 2);
    assert_eq!(response.1["turns"][0]["role"], "user");
    assert_eq!(response.1["turns"][1]["role"], "assistant");
    assert_eq!(response.1["turns"][1]["status"], "pending");

    let project = spark_storage::ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths("/projects/http-turn")
        .expect("project");
    assert!(project
        .conversations_dir
        .join("conversation-http-turn/state.json")
        .exists());
    assert!(project
        .conversations_dir
        .join("conversation-http-turn/events.jsonl")
        .exists());

    let conflict = request_json(
        app.clone(),
        "POST",
        "/workspace/api/conversations/conversation-http-turn/turns",
        Some(json!({"project_path": "/projects/http-turn", "message": "Again"})),
    )
    .await;
    assert_eq!(conflict.0, StatusCode::CONFLICT);
    assert!(conflict.1["detail"]
        .as_str()
        .expect("detail")
        .contains("assistant turn is still in progress"));

    let empty = request_json(
        app.clone(),
        "POST",
        "/workspace/api/conversations/conversation-empty/turns",
        Some(json!({"project_path": "/projects/http-turn", "message": "   "})),
    )
    .await;
    assert_eq!(empty.0, StatusCode::BAD_REQUEST);
    assert_eq!(empty.1, json!({"detail": "Message is required."}));

    let missing_model = request_json(
        app.clone(),
        "POST",
        "/workspace/api/conversations/conversation-model/turns",
        Some(json!({"project_path": "/projects/http-turn", "message": "Hi", "provider": "openrouter"})),
    )
    .await;
    assert_eq!(missing_model.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        missing_model.1,
        json!({"detail": "Provider openrouter requires an explicit model."})
    );

    let malformed = request_text(
        app,
        "POST",
        "/workspace/api/conversations/conversation-http-turn/turns",
        "{not-json",
        Some("application/json"),
    )
    .await;
    assert_eq!(malformed.0, StatusCode::BAD_REQUEST);
    assert!(malformed.2.starts_with("application/json"));
}

#[tokio::test]
async fn request_user_input_answer_route_expires_pending_requests() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = WorkspaceConversationService::new(settings.clone());
    let (prepared, _) = service
        .start_turn(
            "conversation-http-input",
            ConversationTurnRequest {
                project_path: "/projects/http-input".to_string(),
                message: "Need input".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start");
    service
        .ingest_agent_turn_output(
            "conversation-http-input",
            "/projects/http-input",
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![request_user_input_event()],
                ..AgentTurnOutput::default()
            },
        )
        .expect("pending request");
    let app = build_app(settings);

    let answered = request_json(
        app.clone(),
        "POST",
        "/workspace/api/conversations/conversation-http-input/request-user-input/decision/answer",
        Some(json!({
            "project_path": "/projects/http-input",
            "answers": {"decision": "Approve"}
        })),
    )
    .await;
    assert_eq!(answered.0, StatusCode::OK);
    let segment = answered.1["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .find(|segment| segment["kind"] == "request_user_input")
        .expect("request segment");
    assert_eq!(segment["request_user_input"]["status"], "expired");
    assert_eq!(segment["status"], "failed");

    let changed = request_json(
        app.clone(),
        "POST",
        "/workspace/api/conversations/conversation-http-input/request-user-input/decision/answer",
        Some(json!({
            "project_path": "/projects/http-input",
            "answers": {"decision": "Reject"}
        })),
    )
    .await;
    assert_eq!(changed.0, StatusCode::CONFLICT);

    let missing = request_json(
        app,
        "POST",
        "/workspace/api/conversations/conversation-http-input/request-user-input/missing/answer",
        Some(json!({
            "project_path": "/projects/http-input",
            "answers": {"decision": "Approve"}
        })),
    )
    .await;
    assert_eq!(missing.0, StatusCode::NOT_FOUND);
    assert_eq!(
        missing.1,
        json!({"detail": "Unknown conversation input request: missing"})
    );
}

async fn request_json(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value, String) {
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
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = serde_json::from_slice::<Value>(&bytes).expect("json");
    (status, value, content_type)
}

async fn request_text(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: &str,
    content_type: Option<&str>,
) -> (StatusCode, String, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(content_type) = content_type {
        builder = builder.header("content-type", content_type);
    }
    let request = builder
        .body(Body::from(body.to_string()))
        .expect("request body");
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (
        status,
        String::from_utf8(bytes.to_vec()).expect("utf-8"),
        content_type,
    )
}

fn request_user_input_event() -> TurnStreamEvent {
    TurnStreamEvent {
        kind: TurnStreamEventKind::RequestUserInputRequested,
        channel: None,
        source: TurnStreamSource {
            app_turn_id: Some("app-turn".to_string()),
            item_id: Some("input-1".to_string()),
            ..TurnStreamSource::default()
        },
        content_delta: None,
        message: None,
        tool_call: None,
        request_user_input: Some(json!({
            "itemId": "input-1",
            "questions": [
                {
                    "id": "decision",
                    "header": "Approve",
                    "question": "Approve this change?",
                    "options": [
                        {"label": "Approve", "description": "Continue"},
                        {"label": "Reject", "description": "Stop"}
                    ]
                }
            ]
        })),
        token_usage: None,
        error: None,
        phase: None,
        status: None,
    }
}

fn settings(root: &Path) -> SparkSettings {
    SparkSettings {
        project_root: root.join("source"),
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
