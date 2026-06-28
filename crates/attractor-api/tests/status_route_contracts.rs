use std::fs;
use std::path::{Path, PathBuf};

use attractor_api::{handle_attractor_request, AttractorApiService};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;

#[test]
fn idle_status_payload_matches_fixture() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = AttractorApiService::new(settings(temp.path()));
    let fixture = fixture_json("http/attractor-status.json");

    let response = service.get_status();

    assert_eq!(response.status_code, 200);
    assert_eq!(response.content_type, "application/json");
    assert_eq!(response.body, fixture["response"]["body"]["json"]);
}

#[test]
fn fresh_run_store_returns_empty_runs_payload() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = AttractorApiService::new(settings(temp.path()));

    let response = service.list_runs();

    assert_eq!(response.status_code, 200);
    assert_eq!(response.body, json!({"runs": []}));
}

#[test]
fn deprecated_runs_events_route_returns_plain_text_410() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = AttractorApiService::new(settings(temp.path()));
    let fixture = fixture_json("http/deprecated-attractor-runs-events.json");

    let response = service.deprecated_runs_events();

    assert_eq!(response.status_code, 410);
    assert_eq!(response.content_type, "text/plain; charset=utf-8");
    assert_eq!(
        response.body.as_str().unwrap(),
        fixture["response"]["body"]["text"].as_str().unwrap()
    );
}

#[test]
fn route_dispatch_keeps_root_and_api_subroutes_distinct() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).expect("config dir");

    let status = handle_attractor_request("GET", "/attractor/status", "", settings.clone());
    assert_eq!(status.status_code, 200);
    assert_eq!(status.body["status"], json!("idle"));

    let profiles =
        handle_attractor_request("GET", "/attractor/api/llm-profiles", "", settings.clone());
    assert_eq!(profiles.status_code, 200);
    assert_eq!(profiles.body, json!({"profiles": []}));

    let moved = handle_attractor_request("GET", "/attractor/llm-profiles", "", settings);
    assert_eq!(moved.status_code, 404);
    assert_eq!(moved.body, json!({"detail": "Not Found"}));
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
