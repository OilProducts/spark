use std::path::Path;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
use spark_http::build_app;
use tower::ServiceExt;

#[tokio::test]
async fn workspace_flow_list_detail_raw_and_validate_routes_layer_the_project_org() {
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
    write_flow(&settings, "ops/review/inspectable", flow_content);
    write_flow(&settings, "shared", &simple_flow("shared"));
    // The project's copy of `shared` replaces the bundled one, with a lock.
    let project = temp.path().join("project");
    let org_flows = spark_common::project::org_dir(&project).join("flows");
    attractor_dsl::write_flow_dir(
        &org_flows.join("shared"),
        &attractor_dsl::parse_flow_definition(&format!(
            "{}execution_lock: {{ scope: project, key: integration }}\n",
            simple_flow("shared").replace("title: shared", "title: Project Shared")
        ))
        .expect("valid flow"),
    )
    .expect("write org flow");
    let app = build_app(settings.clone());
    let query = format!("?project_path={}", project.display());

    let bundled = request_json(app.clone(), "GET", "/workspace/api/flows", None).await;
    assert_eq!(bundled.0, StatusCode::OK);
    assert_eq!(bundled.1.as_array().expect("flows").len(), 2);
    assert_eq!(bundled.1[1]["title"], "shared");
    assert_eq!(bundled.1[1]["execution_lock"], Value::Null);

    let layered = request_json(
        app.clone(),
        "GET",
        &format!("/workspace/api/flows{query}"),
        None,
    )
    .await;
    assert_eq!(layered.0, StatusCode::OK);
    assert_eq!(layered.1.as_array().expect("flows").len(), 2);
    assert_eq!(layered.1[1]["name"], "shared");
    assert_eq!(layered.1[1]["title"], "Project Shared");
    assert_eq!(
        layered.1[1]["execution_lock"],
        json!({"scope": "project", "key": "integration", "conflict_policy": "queue"})
    );
    assert!(layered.1[1].get("effective_launch_policy").is_none());

    let detail = request_json(
        app.clone(),
        "GET",
        &format!("/workspace/api/flows/ops/review/inspectable{query}"),
        None,
    )
    .await;
    assert_eq!(detail.0, StatusCode::OK);
    assert_eq!(detail.1["title"], "Inspectable Graph");
    assert_eq!(detail.1["features"]["has_human_gate"], true);

    let raw = request_text(
        app.clone(),
        "GET",
        "/workspace/api/flows/ops/review/inspectable/raw",
        None,
    )
    .await;
    assert_eq!(raw.0, StatusCode::OK);
    assert!(raw.2.starts_with("text/yaml"));
    assert_eq!(raw.3, Some("ops/review/inspectable".to_string()));
    assert_eq!(
        attractor_dsl::parse_flow_definition(&raw.1).expect("raw flow"),
        attractor_dsl::read_flow_dir(
            &settings
                .bundled_dir()
                .join("flows")
                .join("ops/review/inspectable")
        )
        .expect("stored flow")
    );

    let validation = request_json(
        app.clone(),
        "GET",
        "/workspace/api/flows/ops/review/inspectable/validate",
        None,
    )
    .await;
    assert_eq!(validation.0, StatusCode::OK);
    assert_eq!(validation.1["name"], "ops/review/inspectable");
    assert_eq!(validation.1["status"], "ok");

    let project_raw = request_text(
        app,
        "GET",
        &format!("/workspace/api/flows/shared/raw{query}"),
        None,
    )
    .await;
    assert_eq!(project_raw.0, StatusCode::OK);
    assert!(
        project_raw.1.contains("Project Shared"),
        "{}",
        project_raw.1
    );
}

#[tokio::test]
async fn workspace_flow_routes_return_json_errors_for_missing_and_unsafe_names() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let app = build_app(settings);

    let missing = request_json(app.clone(), "GET", "/workspace/api/flows/missing", None).await;
    assert_eq!(missing.0, StatusCode::NOT_FOUND);
    assert_eq!(missing.1, json!({"detail": "Unknown flow: missing"}));

    let escaped = request_json(
        app.clone(),
        "GET",
        "/workspace/api/flows/../escape/raw",
        None,
    )
    .await;
    assert_eq!(escaped.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        escaped.1,
        json!({"detail": "Flow name must be a relative path inside the flows directory."})
    );

    let removed = request_text(
        app,
        "PUT",
        "/workspace/api/flows/missing/launch-policy",
        Some("{}"),
    )
    .await;
    assert_eq!(removed.0, StatusCode::METHOD_NOT_ALLOWED);
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
    attractor_dsl::write_flow_dir(
        &settings.bundled_dir().join("flows").join(name),
        &attractor_dsl::parse_flow_definition(content).expect("valid flow"),
    )
    .expect("write flow");
}

fn simple_flow(id: &str) -> String {
    format!(
        "schema_version: '1'\nid: {id}\ntitle: {id}\nnodes:\n  start:\n    kind: start\n  done:\n    kind: exit\nedges:\n  - from: start\n    to: done\n"
    )
}

fn settings(root: &Path) -> SparkSettings {
    SparkSettings {
        startup_sources: Default::default(),
        connections: Default::default(),
        providers: Default::default(),
        agents: Default::default(),
        project_root: root.join("source"),
        data_dir: root.join("spark-home"),
        config_dir: root.join("spark-home/config"),
        runtime_dir: root.join("spark-home/runtime"),
        logs_dir: root.join("spark-home/logs"),
        workspace_dir: root.join("spark-home/workspace"),
        projects_dir: root.join("spark-home/workspace/projects"),
        attractor_dir: root.join("spark-home/attractor"),
        runs_dir: root.join("spark-home/attractor/runs"),
        ui_dir: None,
        project_roots: Vec::new(),
    }
}
