use std::fs;
use std::path::Path;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
use spark_http::build_app;
use tower::ServiceExt;

#[tokio::test]
async fn workspace_project_routes_persist_records_and_return_json_errors() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_dir = temp.path().join("project");
    fs::create_dir_all(&project_dir).expect("project");
    let app = build_app(settings.clone());

    let register = request_json(
        app.clone(),
        "POST",
        "/workspace/api/projects/register",
        Some(json!({"project_path": project_dir})),
    )
    .await;
    assert_eq!(register.0, StatusCode::OK);
    assert_eq!(
        register.1["project_path"].as_str(),
        Some(project_dir.to_string_lossy().as_ref())
    );
    assert_eq!(register.1["display_name"], "project");

    let list = request_json(app.clone(), "GET", "/workspace/api/projects", None).await;
    assert_eq!(list.0, StatusCode::OK);
    assert_eq!(list.1.as_array().expect("projects").len(), 1);

    let project_file = settings
        .projects_dir
        .join(register.1["project_id"].as_str().expect("project id"))
        .join("project.toml");
    assert!(project_file.exists());

    let missing = request_json(app, "GET", "/workspace/api/missing", None).await;
    assert_eq!(missing.0, StatusCode::NOT_FOUND);
    assert_eq!(missing.1, json!({"detail": "Not Found"}));
}

#[tokio::test]
async fn workspace_browse_and_metadata_routes_match_validation_contracts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("Browse Root");
    fs::create_dir_all(root.join("Alpha")).expect("alpha");
    fs::create_dir_all(root.join("zeta")).expect("zeta");
    fs::write(root.join("notes.txt"), "ignored").expect("file");
    let mut settings = settings(temp.path());
    settings.project_roots = vec![root.clone()];
    let app = build_app(settings);

    let browse = request_json(app.clone(), "GET", "/workspace/api/projects/browse", None).await;
    assert_eq!(browse.0, StatusCode::OK);
    assert_eq!(
        browse.1["current_path"].as_str(),
        Some(root.to_string_lossy().as_ref())
    );
    assert_eq!(browse.1["entries"][0]["name"], "Alpha");
    assert_eq!(browse.1["entries"][1]["name"], "zeta");

    let bad_browse = request_json(
        app.clone(),
        "GET",
        "/workspace/api/projects/browse?path=relative",
        None,
    )
    .await;
    assert_eq!(bad_browse.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        bad_browse.1,
        json!({"detail": "Browse path must be absolute."})
    );

    let metadata = request_json(
        app,
        "GET",
        &format!(
            "/workspace/api/projects/metadata?directory={}",
            root.to_string_lossy().replace(' ', "%20")
        ),
        None,
    )
    .await;
    assert_eq!(metadata.0, StatusCode::OK);
    assert_eq!(metadata.1["name"], "Browse Root");
    assert_eq!(metadata.1["branch"], Value::Null);
    assert_eq!(metadata.1["commit"], Value::Null);
}

#[tokio::test]
async fn workspace_settings_wrap_execution_placement_payload() {
    let temp = tempfile::tempdir().expect("tempdir");
    let response = request_json(
        build_app(settings(temp.path())),
        "GET",
        "/workspace/api/settings",
        None,
    )
    .await;

    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(
        response.1["execution_placement"]["execution_modes"],
        json!(["native", "local_container"])
    );
    assert_eq!(
        response.1["execution_placement"]["profiles"][0]["id"],
        "native"
    );
}

#[tokio::test]
async fn attractor_routes_remain_mounted_when_workspace_routes_are_composed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = build_app(settings(temp.path()));

    let status = request_json(app.clone(), "GET", "/attractor/status", None).await;
    assert_eq!(status.0, StatusCode::OK);
    assert!(status.2.starts_with("application/json"));
    assert_eq!(status.1["status"], "idle");

    let placement = request_json(
        app,
        "GET",
        "/attractor/api/execution-placement-settings",
        None,
    )
    .await;
    assert_eq!(placement.0, StatusCode::OK);
    assert!(placement.2.starts_with("application/json"));
    assert_eq!(
        placement.1["execution_modes"],
        json!(["native", "local_container"])
    );
}

#[tokio::test]
async fn workspace_public_extractor_errors_return_json_envelopes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = build_app(settings(temp.path()));

    let missing_query =
        request_json(app.clone(), "GET", "/workspace/api/projects/metadata", None).await;
    assert_eq!(missing_query.0, StatusCode::BAD_REQUEST);
    assert!(missing_query.2.starts_with("application/json"));
    assert!(missing_query.1["detail"]
        .as_str()
        .expect("detail")
        .contains("directory"));

    let malformed_body = request_raw(
        app,
        "POST",
        "/workspace/api/projects/register",
        "{not-json",
        Some("application/json"),
    )
    .await;
    assert_eq!(malformed_body.0, StatusCode::BAD_REQUEST);
    assert!(malformed_body.2.starts_with("application/json"));
    assert!(malformed_body.1["detail"]
        .as_str()
        .expect("detail")
        .contains("Failed to parse"));
}

#[tokio::test]
async fn project_conversations_route_returns_summary_shape_for_existing_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_dir = temp.path().join("project-a");
    fs::create_dir_all(&project_dir).expect("project dir");
    let app = build_app(settings.clone());

    let register = request_json(
        app.clone(),
        "POST",
        "/workspace/api/projects/register",
        Some(json!({"project_path": project_dir})),
    )
    .await;
    assert_eq!(register.0, StatusCode::OK);
    let project_id = register.1["project_id"].as_str().expect("project id");
    let conversations_dir = settings.projects_dir.join(project_id).join("conversations");
    write_state(
        &conversations_dir,
        "conversation-a",
        json!({
            "schema_version": 5,
            "revision": 2,
            "conversation_id": "conversation-a",
            "conversation_handle": "amber-anchor",
            "project_path": project_dir,
            "title": "Design thread",
            "created_at": "2026-03-07T14:00:00Z",
            "updated_at": "2026-03-07T14:02:00Z",
            "turns": [
                {
                    "id": "turn-a-1",
                    "role": "user",
                    "content": "Design thread preview",
                    "timestamp": "2026-03-07T14:02:00Z",
                    "kind": "message"
                }
            ],
            "segments": []
        }),
    );

    let listed = request_json(
        app,
        "GET",
        &format!(
            "/workspace/api/projects/conversations?project_path={}",
            register.1["project_path"].as_str().expect("project path")
        ),
        None,
    )
    .await;

    assert_eq!(listed.0, StatusCode::OK);
    assert_eq!(listed.1[0]["conversation_id"], "conversation-a");
    assert_eq!(listed.1[0]["conversation_handle"], "amber-anchor");
    assert_eq!(listed.1[0]["title"], "Design thread");
    assert_eq!(listed.1[0]["last_message_preview"], "Design thread preview");
    assert!(listed.1[0].get("turns").is_none());
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
    execute_request(app, request).await
}

async fn request_raw(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: &str,
    content_type: Option<&str>,
) -> (StatusCode, Value, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(content_type) = content_type {
        builder = builder.header("content-type", content_type);
    }
    let request = builder
        .body(Body::from(body.to_string()))
        .expect("request body");
    execute_request(app, request).await
}

async fn execute_request(app: axum::Router, request: Request<Body>) -> (StatusCode, Value, String) {
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
        project_roots: Vec::new(),
    }
}
