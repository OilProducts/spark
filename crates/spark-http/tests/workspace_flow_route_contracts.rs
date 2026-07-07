use std::fs;
use std::path::Path;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
use spark_http::build_app;
use spark_storage::{
    read_flow_launch_policy, set_flow_launch_policy, LAUNCH_POLICY_AGENT_REQUESTABLE,
};
use tower::ServiceExt;

#[tokio::test]
async fn workspace_flow_list_detail_raw_validate_and_policy_routes_match_contracts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let flow_content = r#"schema_version: '1'
id: inspectable
title: Inspectable Graph
goal: Inspect graph behavior
nodes:
  start:
    kind: start
  human_review:
    kind: human_gate
    label: Human Review
    config:
      kind: human_gate
      prompt: Review the result
  done:
    kind: exit
edges:
  - from: start
    to: human_review
  - from: human_review
    to: done
"#;
    write_flow(&settings, "ops/review/inspectable.yaml", flow_content);
    write_flow(&settings, "disabled.yaml", &simple_flow("disabled"));
    set_flow_launch_policy(
        &settings.config_dir,
        "ops/review/inspectable.yaml",
        LAUNCH_POLICY_AGENT_REQUESTABLE,
    )
    .expect("policy");
    let app = build_app(settings.clone());

    let human = request_json(
        app.clone(),
        "GET",
        "/workspace/api/flows?surface=human",
        None,
    )
    .await;
    assert_eq!(human.0, StatusCode::OK);
    assert_eq!(human.1.as_array().expect("flows").len(), 2);

    let agent = request_json(
        app.clone(),
        "GET",
        "/workspace/api/flows?surface=agent",
        None,
    )
    .await;
    assert_eq!(agent.0, StatusCode::OK);
    assert_eq!(agent.1.as_array().expect("flows").len(), 1);
    assert_eq!(agent.1[0]["name"], "ops/review/inspectable.yaml");

    let detail = request_json(
        app.clone(),
        "GET",
        "/workspace/api/flows/ops/review/inspectable.yaml?surface=agent",
        None,
    )
    .await;
    assert_eq!(detail.0, StatusCode::OK);
    assert_eq!(detail.1["title"], "Inspectable Graph");
    assert_eq!(detail.1["features"]["has_human_gate"], true);

    let raw = request_text(
        app.clone(),
        "GET",
        "/workspace/api/flows/ops/review/inspectable.yaml/raw?surface=agent",
        None,
    )
    .await;
    assert_eq!(raw.0, StatusCode::OK);
    assert!(raw.2.starts_with("text/yaml"));
    assert_eq!(raw.3, Some("ops/review/inspectable.yaml".to_string()));
    assert_eq!(raw.1, flow_content);

    let validation = request_json(
        app.clone(),
        "GET",
        "/workspace/api/flows/ops/review/inspectable.yaml/validate",
        None,
    )
    .await;
    assert_eq!(validation.0, StatusCode::OK);
    assert_eq!(validation.1["name"], "ops/review/inspectable.yaml");
    assert_eq!(validation.1["status"], "ok");

    let policy = request_json(
        app,
        "PUT",
        "/workspace/api/flows/ops/review/inspectable.yaml/launch-policy",
        Some(json!({"launch_policy": "trigger_only"})),
    )
    .await;
    assert_eq!(policy.0, StatusCode::OK);
    assert_eq!(policy.1["name"], "ops/review/inspectable.yaml");
    assert_eq!(policy.1["launch_policy"], "trigger_only");
    assert_eq!(
        read_flow_launch_policy(&settings.config_dir, "ops/review/inspectable.yaml")
            .expect("stored")
            .launch_policy
            .as_deref(),
        Some("trigger_only")
    );
}

#[tokio::test]
async fn workspace_flow_routes_return_json_errors_for_surface_missing_safety_and_json() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings, "hidden.yaml", &simple_flow("hidden"));
    set_flow_launch_policy(&settings.config_dir, "hidden.yaml", "trigger_only").expect("policy");
    let app = build_app(settings);

    let invalid_surface = request_json(
        app.clone(),
        "GET",
        "/workspace/api/flows?surface=invalid",
        None,
    )
    .await;
    assert_eq!(invalid_surface.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        invalid_surface.1,
        json!({"detail": "Flow surface must be 'human' or 'agent'."})
    );

    let missing = request_json(
        app.clone(),
        "GET",
        "/workspace/api/flows/missing.yaml",
        None,
    )
    .await;
    assert_eq!(missing.0, StatusCode::NOT_FOUND);
    assert_eq!(missing.1, json!({"detail": "Unknown flow: missing.yaml"}));

    let escaped = request_json(
        app.clone(),
        "GET",
        "/workspace/api/flows/../escape.yaml/raw",
        None,
    )
    .await;
    assert_eq!(escaped.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        escaped.1,
        json!({"detail": "Flow name must be a relative path inside flows_dir."})
    );

    let hidden = request_json(
        app.clone(),
        "GET",
        "/workspace/api/flows/hidden.yaml/raw?surface=agent",
        None,
    )
    .await;
    assert_eq!(hidden.0, StatusCode::NOT_FOUND);
    assert_eq!(hidden.1, json!({"detail": "Unknown flow: hidden.yaml"}));

    let malformed = request_text(
        app,
        "PUT",
        "/workspace/api/flows/hidden.yaml/launch-policy",
        Some("{not-json"),
    )
    .await;
    assert_eq!(malformed.0, StatusCode::BAD_REQUEST);
    let payload: Value = serde_json::from_str(&malformed.1).expect("json");
    assert!(payload["detail"]
        .as_str()
        .expect("detail")
        .contains("Failed to parse"));
}

async fn request_json(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value, String) {
    let body_text = body.map(|value| value.to_string());
    let response = request_text(app, method, uri, body_text.as_deref()).await;
    (
        response.0,
        serde_json::from_str(&response.1).expect("json"),
        response.2,
    )
}

async fn request_text(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<&str>,
) -> (StatusCode, String, String, Option<String>) {
    let mut builder = Request::builder().method(method).uri(uri);
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    let request = builder
        .body(Body::from(body.unwrap_or_default().to_string()))
        .expect("request body");
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let headers = response.headers().clone();
    let content_type = headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let flow_name = headers
        .get("x-spark-flow-name")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (
        status,
        String::from_utf8_lossy(&bytes).into_owned(),
        content_type,
        flow_name,
    )
}

fn write_flow(settings: &SparkSettings, name: &str, content: &str) {
    let path = settings.flows_dir.join(name);
    fs::create_dir_all(path.parent().expect("flow parent")).expect("flow parent");
    fs::write(path, content).expect("flow");
}

fn simple_flow(id: &str) -> String {
    format!(
        "schema_version: '1'\nid: {id}\ntitle: {id}\nnodes:\n  start:\n    kind: start\n  done:\n    kind: exit\nedges:\n  - from: start\n    to: done\n"
    )
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
