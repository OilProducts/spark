use std::fs;
use std::path::{Path, PathBuf};

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
use spark_http::build_app;
use spark_storage::ProjectRegistry;
use tower::ServiceExt;

#[tokio::test]
async fn conversation_routes_return_snapshot_tool_output_settings_and_delete_contracts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = "/projects/http-app";
    let project = ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths(project_path)
        .expect("project");
    write_state(
        &project.conversations_dir,
        "conversation-http",
        json!({
            "schema_version": 5,
            "revision": 1,
            "conversation_id": "conversation-http",
            "conversation_handle": "amber-anchor",
            "project_path": project_path,
            "chat_mode": "chat",
            "provider": "codex",
            "model": null,
            "reasoning_effort": null,
            "title": "HTTP thread",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:01Z",
            "turns": [],
            "segments": [
                {
                    "id": "segment-tool",
                    "turn_id": "turn-a",
                    "order": 1,
                    "kind": "tool_call",
                    "role": "assistant",
                    "status": "complete",
                    "timestamp": "2026-01-01T00:00:01Z",
                    "updated_at": "2026-01-01T00:00:01Z",
                    "content": "",
                    "source": {},
                    "tool_call": {
                        "id": "tool-a",
                        "kind": "command_execution",
                        "status": "completed",
                        "title": "Command",
                        "output": "full output",
                        "file_paths": []
                    }
                }
            ]
        }),
    );
    fs::write(
        project.flow_run_requests_dir.join("conversation-http.json"),
        serde_json::to_string_pretty(&json!({
            "conversation_id": "conversation-http",
            "project_id": project.project_id,
            "project_path": project_path,
            "event_log": [],
            "flow_run_requests": [{"id": "request-http", "created_at": "2026-01-01T00:00:01Z", "updated_at": "2026-01-01T00:00:01Z", "flow_name": "flow.dot", "summary": "Run", "project_path": project_path, "conversation_id": "conversation-http", "source_turn_id": "turn-a"}]
        }))
        .expect("json"),
    )
    .expect("sidecar");
    let app = build_app(settings.clone());

    let snapshot = request_json(
        app.clone(),
        "GET",
        "/workspace/api/conversations/conversation-http?project_path=/projects/http-app",
        None,
    )
    .await;
    assert_eq!(snapshot.0, StatusCode::OK);
    assert_eq!(snapshot.1["conversation_id"], "conversation-http");
    assert_eq!(snapshot.1["llm_profile"], Value::Null);
    assert_eq!(snapshot.1["flow_run_requests"][0]["id"], "request-http");

    let output = request_json(
        app.clone(),
        "GET",
        "/workspace/api/conversations/conversation-http/segments/segment-tool/tool-output?project_path=/projects/http-app",
        None,
    )
    .await;
    assert_eq!(output.0, StatusCode::OK);
    assert_eq!(
        output.1,
        json!({"output": "full output", "output_size": 11})
    );

    let settings_response = request_json(
        app.clone(),
        "PUT",
        "/workspace/api/conversations/conversation-http/settings",
        Some(json!({"project_path": "/projects/http-app", "chat_mode": "plan"})),
    )
    .await;
    assert_eq!(settings_response.0, StatusCode::OK);
    assert_eq!(settings_response.1["chat_mode"], "plan");
    // Mode-change turn entry plus settings journal entry.
    assert_eq!(settings_response.1["revision"], 3);

    let deprecated = request_text(
        app.clone(),
        "GET",
        "/workspace/api/conversations/conversation-http/events?project_path=/projects/http-app",
        "",
        None,
    )
    .await;
    assert_eq!(deprecated.0, StatusCode::GONE);
    assert_eq!(deprecated.2, "text/plain; charset=utf-8");
    assert_eq!(
        deprecated.1,
        "Deprecated. Use /workspace/api/live/events with conversation_id and conversation_revision."
    );

    let deleted = request_json(
        app,
        "DELETE",
        "/workspace/api/conversations/conversation-http?project_path=/projects/http-app",
        None,
    )
    .await;
    assert_eq!(deleted.0, StatusCode::OK);
    assert_eq!(
        deleted.1,
        json!({
            "status": "deleted",
            "conversation_id": "conversation-http",
            "project_path": "/projects/http-app"
        })
    );
    assert!(!project.conversations_dir.join("conversation-http").exists());
}

#[tokio::test]
async fn project_conversation_list_allocates_missing_summary_handles() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = "/projects/http-list";
    let project = ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths(project_path)
        .expect("project");
    write_state(
        &project.conversations_dir,
        "conversation-no-handle",
        json!({
            "schema_version": 5,
            "revision": 1,
            "conversation_id": "conversation-no-handle",
            "project_path": project_path,
            "chat_mode": "chat",
            "provider": "codex",
            "model": null,
            "reasoning_effort": null,
            "title": "Needs handle",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:01Z",
            "turns": [],
            "segments": []
        }),
    );
    let app = build_app(settings.clone());

    let listed = request_json(
        app,
        "GET",
        "/workspace/api/projects/conversations?project_path=/projects/http-list",
        None,
    )
    .await;

    assert_eq!(listed.0, StatusCode::OK);
    let handle = listed.1[0]["conversation_handle"].as_str().expect("handle");
    assert_eq!(handle.split('-').count(), 2);
    let index: Value = serde_json::from_str(
        &fs::read_to_string(settings.workspace_dir.join("conversation-handles.json"))
            .expect("handles"),
    )
    .expect("json");
    assert_eq!(
        index["conversation_ids"]["conversation-no-handle"].as_str(),
        Some(handle)
    );
}

#[tokio::test]
async fn conversation_routes_return_json_errors_for_bad_inputs_and_unsupported_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths("/projects/http-errors")
        .expect("project");
    write_state(
        &project.conversations_dir,
        "conversation-old",
        json!({
            "schema_version": 4,
            "revision": 1,
            "conversation_id": "conversation-old",
            "project_path": "/projects/http-errors",
            "segments": []
        }),
    );
    let app = build_app(settings);

    let unsupported = request_json(
        app.clone(),
        "GET",
        "/workspace/api/conversations/conversation-old?project_path=/projects/http-errors",
        None,
    )
    .await;
    assert_eq!(unsupported.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        unsupported.1,
        json!({"detail": "Unsupported conversation state schema. Delete the local conversation and recreate it."})
    );

    let unknown = request_json(
        app.clone(),
        "GET",
        "/workspace/api/conversations/missing",
        None,
    )
    .await;
    assert_eq!(unknown.0, StatusCode::NOT_FOUND);
    assert_eq!(
        unknown.1,
        json!({"detail": "Unknown conversation: missing"})
    );

    let malformed = request_text(
        app.clone(),
        "PUT",
        "/workspace/api/conversations/conversation-old/settings",
        "{not-json",
        Some("application/json"),
    )
    .await;
    assert_eq!(malformed.0, StatusCode::BAD_REQUEST);
    assert!(malformed.2.starts_with("application/json"));
    let malformed_body: Value = serde_json::from_str(&malformed.1).expect("json");
    assert!(malformed_body["detail"]
        .as_str()
        .expect("detail")
        .contains("Failed to parse"));

    let missing_delete_query = request_json(
        app,
        "DELETE",
        "/workspace/api/conversations/conversation-old",
        None,
    )
    .await;
    assert_eq!(missing_delete_query.0, StatusCode::BAD_REQUEST);
    assert!(missing_delete_query.1["detail"]
        .as_str()
        .expect("detail")
        .contains("project_path"));
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

fn write_state(conversations_dir: &Path, conversation_id: &str, payload: Value) {
    let state_path = conversations_dir.join(conversation_id).join("state.json");
    fs::create_dir_all(state_path.parent().expect("state parent")).expect("state parent");
    fs::write(
        state_path,
        serde_json::to_string_pretty(&payload).expect("json"),
    )
    .expect("state");
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
