use std::fs;
use std::path::{Path, PathBuf};

use attractor_api::{
    list_logical_flow_names, preview_named_flow_source, read_named_flow_source,
    resolve_logical_flow_path, AttractorApiService, SaveFlowRequest,
};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;

const VALID_DOT: &str = r#"digraph Workflow {
  start [shape=Mdiamond];
  task [shape=box, prompt="Do work"];
  done [shape=Msquare];
  start -> task;
  task -> done;
}
"#;

#[test]
fn list_and_get_flows_use_normalized_attractor_payloads() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let nested = settings.flows_dir.join("examples");
    fs::create_dir_all(&nested).expect("flows dir");
    fs::write(nested.join("simple-linear.dot"), VALID_DOT).expect("write flow");
    fs::write(settings.flows_dir.join("root.dot"), VALID_DOT).expect("write root");
    let service = AttractorApiService::new(settings);

    let list = service.list_flows();
    assert_eq!(list.status_code, 200);
    assert_eq!(list.body, json!(["examples/simple-linear.dot", "root.dot"]));

    let get = service.get_flow("examples/simple-linear.dot");
    assert_eq!(get.status_code, 200);
    assert_eq!(get.body["name"], json!("examples/simple-linear.dot"));
    assert_eq!(get.body["content"], json!(VALID_DOT));
}

#[test]
fn get_and_delete_missing_flow_match_error_contract() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = AttractorApiService::new(settings(temp.path()));

    let get = service.get_flow("missing.dot");
    assert_eq!(get.status_code, 404);
    assert_eq!(get.body, json!({"detail": "Flow not found."}));

    let delete = service.delete_flow("missing.dot");
    assert_eq!(delete.status_code, 404);
    assert_eq!(delete.body, json!({"detail": "Flow not found."}));
}

#[test]
fn flow_name_safety_errors_are_exposed_through_api_responses() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = AttractorApiService::new(settings(temp.path()));

    assert_eq!(
        service.get_flow("").body,
        json!({"detail": "Flow name is required."})
    );
    assert_eq!(
        service.get_flow("/tmp/escape.dot").body,
        json!({"detail": "Flow name must be a relative path inside flows_dir."})
    );
    assert_eq!(
        service.get_flow("../escape.dot").body,
        json!({"detail": "Flow name must be a relative path inside flows_dir."})
    );
    assert_eq!(
        service.get_flow("nested/").body,
        json!({"detail": "Flow name must reference a file."})
    );
}

#[test]
fn public_flow_source_helpers_match_route_normalization_and_preview_contracts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    fs::create_dir_all(settings.flows_dir.join("nested")).expect("flows dir");
    fs::write(settings.flows_dir.join("nested/helper.dot"), VALID_DOT).expect("flow");

    assert_eq!(
        list_logical_flow_names(&settings.flows_dir).expect("names"),
        vec!["nested/helper.dot"]
    );
    assert_eq!(
        resolve_logical_flow_path(&settings.flows_dir, "nested/helper")
            .expect("path")
            .file_name()
            .and_then(|value| value.to_str()),
        Some("helper.dot")
    );
    let source = read_named_flow_source(&settings.flows_dir, "nested/helper").expect("source");
    assert_eq!(source.name, "nested/helper.dot");
    assert_eq!(source.content, VALID_DOT);

    let preview = preview_named_flow_source(&settings.flows_dir, &source.name, &source.content);
    assert_eq!(preview.status_code, 200);
    assert_eq!(preview.body["status"], "ok");
    assert_eq!(preview.body["graph"]["nodes"][0]["id"], "start");
}

#[test]
fn save_parse_error_matches_python_fixture_detail_shape() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = AttractorApiService::new(settings(temp.path()));
    let fixture = fixture_json("http/attractor-flow-save-parse-error.json");

    let response = service.save_flow(SaveFlowRequest {
        name: "bad.dot".to_string(),
        content: "digraph Workflow { start -> }\n".to_string(),
        expect_semantic_equivalence: false,
    });

    assert_eq!(response.status_code, 422);
    assert_eq!(response.body, fixture["response"]["body"]["json"]);
}

#[test]
fn save_validation_error_reports_diagnostics_without_writing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = AttractorApiService::new(settings.clone());

    let response = service.save_flow(SaveFlowRequest {
        name: "broken.dot".to_string(),
        content: r#"digraph Broken {
  start [shape=Mdiamond];
  done [shape=Msquare];
  start -> missing;
}
"#
        .to_string(),
        expect_semantic_equivalence: false,
    });

    assert_eq!(response.status_code, 422);
    assert_eq!(response.body["detail"]["status"], json!("validation_error"));
    assert!(!settings.flows_dir.join("broken.dot").exists());
}

#[test]
fn save_canonicalizes_nested_flow_and_reports_semantic_equivalence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = AttractorApiService::new(settings.clone());

    let created = service.save_flow(SaveFlowRequest {
        name: "nested/new-flow".to_string(),
        content: VALID_DOT.to_string(),
        expect_semantic_equivalence: false,
    });
    assert_eq!(created.status_code, 200);
    assert_eq!(
        created.body,
        json!({"status": "saved", "name": "nested/new-flow.dot"})
    );
    let flow_path = settings.flows_dir.join("nested/new-flow.dot");
    assert!(flow_path.exists());
    assert!(fs::read_to_string(&flow_path)
        .expect("saved flow")
        .starts_with("digraph Workflow"));

    let equivalent = service.save_flow(SaveFlowRequest {
        name: "nested/new-flow.dot".to_string(),
        content: VALID_DOT.to_string(),
        expect_semantic_equivalence: true,
    });
    assert_eq!(equivalent.status_code, 200);
    assert_eq!(
        equivalent.body["semantic_equivalent_to_existing"],
        json!(true)
    );
}

#[test]
fn save_semantic_mismatch_conflicts_when_requested() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = AttractorApiService::new(settings(temp.path()));
    assert_eq!(
        service
            .save_flow(SaveFlowRequest {
                name: "flow.dot".to_string(),
                content: VALID_DOT.to_string(),
                expect_semantic_equivalence: false,
            })
            .status_code,
        200
    );

    let changed = VALID_DOT.replace("Do work", "Do different work");
    let response = service.save_flow(SaveFlowRequest {
        name: "flow.dot".to_string(),
        content: changed,
        expect_semantic_equivalence: true,
    });

    assert_eq!(response.status_code, 409);
    assert_eq!(
        response.body,
        json!({
            "detail": {
                "status": "semantic_mismatch",
                "error": "semantic equivalence check failed: output DOT would change flow behavior",
            }
        })
    );
}

#[test]
fn delete_flow_removes_existing_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.flows_dir).expect("flows dir");
    fs::write(settings.flows_dir.join("delete-me.dot"), VALID_DOT).expect("write flow");
    let service = AttractorApiService::new(settings.clone());

    let response = service.delete_flow("delete-me.dot");

    assert_eq!(response.status_code, 200);
    assert_eq!(response.body, json!({"status": "deleted"}));
    assert!(!settings.flows_dir.join("delete-me.dot").exists());
}

fn fixture_json(relative: &str) -> Value {
    let path = workspace_root()
        .join("crates")
        .join("test-fixtures")
        .join("compat")
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
