use std::path::Path;

use attractor_api::{
    handle_attractor_request, handle_preview_request, preview, preview_with_flows_dir,
    PreviewRequest, PreviewServiceConfig,
};
use serde_json::json;
use spark_common::settings::SparkSettings;

#[test]
fn preview_service_body_reports_flow_definition_graph() {
    let response = preview(PreviewRequest {
        flow_content: simple_flow(),
        flow_name: None,
        expand_children: false,
    });

    assert_eq!(response.status_code, 200);
    assert_eq!(response.content_type, "application/json");
    assert_eq!(response.body["status"], json!("ok"));
    assert!(response.body["graph"]["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .any(|node| node["id"] == json!("start")));
}

#[test]
fn preview_service_body_reports_yaml_parse_errors() {
    let response = preview(PreviewRequest {
        flow_content: "nodes: [".to_string(),
        flow_name: None,
        expand_children: false,
    });

    assert_eq!(response.status_code, 200);
    assert_eq!(response.content_type, "application/json");
    assert_eq!(response.body["status"], json!("validation_error"));
    assert_eq!(
        response.body["errors"][0]["rule_id"],
        json!("flow_definition")
    );
}

#[test]
fn preview_service_reports_validation_errors_with_http_success_status() {
    let response = preview(PreviewRequest {
        flow_content: r#"schema_version: "1"
id: broken
nodes:
  start:
    kind: start
  done:
    kind: exit
edges:
  - from: start
    to: missing
"#
        .to_string(),
        flow_name: None,
        expand_children: false,
    });

    assert_eq!(response.status_code, 200);
    assert_eq!(response.body["status"], json!("validation_error"));
    assert_eq!(
        response.body["errors"][0]["rule_id"],
        json!("flow_definition")
    );
}

#[test]
fn preview_service_accepts_flow_name_and_expand_children_flags() {
    let temp = tempfile::tempdir().expect("tempdir");
    let flows_dir = temp.path().join("flows");
    let response = preview_with_flows_dir(
        PreviewRequest {
            flow_content: simple_flow(),
            flow_name: Some("nested/parent.yaml".to_string()),
            expand_children: true,
        },
        &flows_dir,
    );

    assert_eq!(response.status_code, 200);
    assert_eq!(response.body["status"], json!("ok"));
}

#[test]
fn preview_service_expands_subflow_children_from_flows_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let flows_dir = temp.path().join("flows");
    let nested_dir = flows_dir.join("nested");
    std::fs::create_dir_all(&nested_dir).expect("flows dir");
    std::fs::write(nested_dir.join("child.yaml"), child_flow()).expect("child flow");

    let response = preview_with_flows_dir(
        PreviewRequest {
            flow_content: r#"schema_version: "1"
id: parent
title: Parent
nodes:
  start:
    kind: start
  child:
    kind: subflow
    label: Child
    config:
      kind: subflow
      flow_ref: child.yaml
  done:
    kind: exit
edges:
  - from: start
    to: child
  - from: child
    to: done
"#
            .to_string(),
            flow_name: Some("nested/parent.yaml".to_string()),
            expand_children: true,
        },
        &flows_dir,
    );

    assert_eq!(response.status_code, 200);
    assert_eq!(response.body["status"], json!("ok"));
    assert_eq!(
        response.body["graph"]["child_previews"]["child"]["flow_name"],
        json!("nested/child.yaml")
    );
    assert!(
        response.body["graph"]["child_previews"]["child"]["graph"]["nodes"]
            .as_array()
            .expect("child nodes")
            .iter()
            .any(|node| node["kind"] == json!("human_gate"))
    );
}

#[test]
fn preview_route_wrapper_accepts_post_preview_json() {
    let body = json!({
        "flow_content": simple_flow(),
    })
    .to_string();

    let response =
        handle_preview_request("POST", "/preview", &body, &PreviewServiceConfig::default());

    assert_eq!(response.status_code, 200);
    assert_eq!(response.content_type, "application/json");
    assert_eq!(response.body["status"], json!("ok"));
}

#[test]
fn preview_route_wrapper_accepts_mounted_attractor_preview_path() {
    let body = json!({
        "flow_content": simple_flow(),
    })
    .to_string();

    let response = handle_preview_request(
        "POST",
        "/attractor/preview",
        &body,
        &PreviewServiceConfig::default(),
    );

    assert_eq!(response.status_code, 200);
    assert_eq!(response.content_type, "application/json");
    assert_eq!(response.body["status"], json!("ok"));
}

#[test]
fn mounted_route_dispatcher_returns_json_404_for_unknown_api_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let response =
        handle_attractor_request("GET", "/attractor/api/missing", "", settings(temp.path()));

    assert_eq!(response.status_code, 404);
    assert_eq!(response.content_type, "application/json");
    assert_eq!(response.body, json!({"detail": "Not Found"}));
}

#[test]
fn preview_route_wrapper_rejects_non_preview_routes_without_server_scope() {
    let response = handle_preview_request("GET", "/status", "{}", &PreviewServiceConfig::default());

    assert_eq!(response.status_code, 404);
    assert_eq!(response.body, json!({"detail": "Not Found"}));
}

fn simple_flow() -> String {
    r#"schema_version: "1"
id: preview
title: Preview
nodes:
  start:
    kind: start
  task:
    kind: agent_task
    label: Task
    config:
      kind: agent_task
      prompt: Preview task
  done:
    kind: exit
edges:
  - from: start
    to: task
  - from: task
    to: done
"#
    .to_string()
}

fn child_flow() -> String {
    r#"schema_version: "1"
id: child
title: Child
nodes:
  start:
    kind: start
  human:
    kind: human_gate
    label: Human
    config:
      kind: human_gate
      prompt: Review
  done:
    kind: exit
edges:
  - from: start
    to: human
  - from: human
    to: done
"#
    .to_string()
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
        flows_dir: root.join("spark-home/flows"),
        ui_dir: None,
        project_roots: Vec::new(),
    }
}
