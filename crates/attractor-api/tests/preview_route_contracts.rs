use std::fs;
use std::path::{Path, PathBuf};

use attractor_api::{
    handle_attractor_request, handle_preview_request, preview, preview_with_flows_dir,
    PreviewRequest, PreviewServiceConfig,
};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;

#[test]
fn preview_service_body_matches_success_fixture() {
    let fixture = fixture_json("http/attractor-preview-success.json");
    let response = preview(PreviewRequest {
        flow_content: fixture["request"]["body"]["json"]["flow_content"]
            .as_str()
            .unwrap()
            .to_string(),
        flow_name: None,
        expand_children: false,
    });

    assert_eq!(response.status_code, 200);
    assert_eq!(response.content_type, "application/json");
    assert_eq!(response.body, fixture["response"]["body"]["json"]);
}

#[test]
fn preview_service_body_matches_parse_error_fixture() {
    let fixture = fixture_json("http/attractor-preview-parse-error.json");
    let response = preview(PreviewRequest {
        flow_content: fixture["request"]["body"]["json"]["flow_content"]
            .as_str()
            .unwrap()
            .to_string(),
        flow_name: None,
        expand_children: false,
    });

    assert_eq!(response.status_code, 200);
    assert_eq!(response.content_type, "application/json");
    assert_eq!(response.body, fixture["response"]["body"]["json"]);
}

#[test]
fn preview_service_reports_validation_errors_with_http_success_status() {
    let flow = r#"
    digraph Broken {
      start [shape=Mdiamond];
      done [shape=Msquare];
      start -> missing;
    }
    "#;

    let response = preview(PreviewRequest {
        flow_content: flow.to_string(),
        flow_name: None,
        expand_children: false,
    });

    assert_eq!(response.status_code, 200);
    assert_eq!(response.body["status"], "validation_error");
    assert!(response.body["graph"].is_object());
    let error_rules = response.body["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| diagnostic["rule"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert!(error_rules.contains(&"edge_target_exists".to_string()));
}

#[test]
fn preview_service_uses_flow_name_parent_as_child_preview_base_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let flows_dir = temp.path().join("flows");
    let nested_dir = flows_dir.join("nested");
    fs::create_dir_all(&nested_dir).expect("nested dir");
    let child_path = nested_dir.join("child.dot");
    fs::write(
        &child_path,
        r#"
        digraph Child {
          start [shape=Mdiamond];
          done [shape=Msquare];
          start -> done;
        }
        "#,
    )
    .expect("write child");

    let parent = r#"
    digraph Parent {
      graph [stack.child_dotfile="child.dot"];
      start [shape=Mdiamond];
      manager [shape=house, prompt="Manage"];
      done [shape=Msquare];
      start -> manager -> done;
    }
    "#;

    let response = preview_with_flows_dir(
        PreviewRequest {
            flow_content: parent.to_string(),
            flow_name: Some("nested/parent.dot".to_string()),
            expand_children: true,
        },
        &flows_dir,
    );

    assert_eq!(response.status_code, 200);
    assert_eq!(
        response.body["graph"]["child_previews"]["manager"]["flow_path"],
        child_path.to_string_lossy().to_string()
    );
}

#[test]
fn preview_route_wrapper_accepts_post_preview_json() {
    let fixture = fixture_json("http/attractor-preview-success.json");
    let body = json!({
        "flow_content": fixture["request"]["body"]["json"]["flow_content"],
    })
    .to_string();

    let response =
        handle_preview_request("POST", "/preview", &body, &PreviewServiceConfig::default());

    assert_eq!(response.status_code, 200);
    assert_eq!(response.content_type, "application/json");
    assert_eq!(response.body, fixture["response"]["body"]["json"]);
}

#[test]
fn preview_route_wrapper_accepts_mounted_attractor_preview_path() {
    let fixture = fixture_json("http/attractor-preview-success.json");
    let body = json!({
        "flow_content": fixture["request"]["body"]["json"]["flow_content"],
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
    assert_eq!(response.body, fixture["response"]["body"]["json"]);
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

fn fixture_json(relative: &str) -> Value {
    let path = workspace_root()
        .join("tests")
        .join("compat")
        .join("fixtures")
        .join(relative);
    serde_json::from_str(
        &fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("unable to read fixture {}: {error}", path.display())),
    )
    .expect("fixture json")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}
