use std::fs;
use std::path::Path;

use attractor_api::{
    list_logical_flow_names, preview_named_flow_source, read_named_flow_source,
    resolve_logical_flow_path, AttractorApiService, SaveFlowRequest,
};
use attractor_dsl::{parse_flow_definition, write_flow_dir};
use serde_json::json;
use spark_common::settings::SparkSettings;

const VALID_FLOW: &str = r#"schema_version: "1"
id: workflow
title: Workflow
goal: Do work
nodes:
  start:
    kind: start
    label: Start
  task:
    kind: agent_task
    label: Task
    config:
      kind: agent_task
      prompt: Do work
  done:
    kind: exit
    label: Done
edges:
  - from: start
    to: task
  - from: task
    to: done
"#;

#[test]
fn list_and_get_flows_use_normalized_attractor_payloads() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings.flows_dir.join("examples/simple-linear"));
    write_flow(&settings.flows_dir.join("root"));
    // Retired YAML files are not flows.
    fs::write(settings.flows_dir.join("legacy.yaml"), VALID_FLOW).expect("write yaml");
    let service = AttractorApiService::new(settings);

    let list = service.list_flows();
    assert_eq!(list.status_code, 200);
    assert_eq!(list.body, json!(["examples/simple-linear", "root"]));

    let get = service.get_flow("examples/simple-linear");
    assert_eq!(get.status_code, 200);
    assert_eq!(get.body["name"], json!("examples/simple-linear"));
    assert_eq!(
        parse_flow_definition(get.body["content"].as_str().expect("content")).expect("flow"),
        parse_flow_definition(VALID_FLOW).expect("flow")
    );
}

#[test]
fn get_and_delete_missing_flow_match_error_contract() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = AttractorApiService::new(settings(temp.path()));

    let get = service.get_flow("missing");
    assert_eq!(get.status_code, 404);
    assert_eq!(get.body, json!({"detail": "Flow not found."}));

    let delete = service.delete_flow("missing");
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
        service.get_flow("/tmp/escape.yaml").body,
        json!({"detail": "Flow name must be a relative path inside flows_dir."})
    );
    assert_eq!(
        service.get_flow("../escape.yaml").body,
        json!({"detail": "Flow name must be a relative path inside flows_dir."})
    );
    assert_eq!(
        service.get_flow("nested/").body,
        json!({"detail": "Flow name must not end with '/'."})
    );
    assert_eq!(
        service.get_flow("nested/flow.yaml").body,
        json!({"detail": "Flow names name a flow directory; drop the file extension."})
    );
}

#[test]
fn public_flow_source_helpers_match_route_normalization_and_preview_contracts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings.flows_dir.join("nested/helper"));

    assert_eq!(
        list_logical_flow_names(&settings.flows_dir).expect("names"),
        vec!["nested/helper"]
    );
    assert_eq!(
        resolve_logical_flow_path(&settings.flows_dir, "nested/helper").expect("path"),
        settings.flows_dir.join("nested/helper")
    );
    let source = read_named_flow_source(&settings.flows_dir, "nested/helper").expect("source");
    assert_eq!(source.name, "nested/helper");
    assert_eq!(
        parse_flow_definition(&source.content).expect("flow"),
        source.flow
    );

    let preview = preview_named_flow_source(&settings.flows_dir, &source.name, &source.content);
    assert_eq!(preview.status_code, 200);
    assert_eq!(preview.body["status"], "ok");
    assert!(preview.body["graph"]["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .any(|node| node["id"] == json!("start")));
}

#[test]
fn save_parse_error_reports_flow_definition_diagnostics() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = AttractorApiService::new(settings(temp.path()));

    let response = service.save_flow(SaveFlowRequest {
        name: "bad".to_string(),
        content: "nodes: [\n".to_string(),
    });

    assert_eq!(response.status_code, 422);
    assert_eq!(response.body["detail"]["status"], json!("validation_error"));
    assert_eq!(
        response.body["detail"]["errors"][0]["rule_id"],
        json!("parse_error")
    );
}

#[test]
fn save_validation_error_reports_diagnostics_without_writing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = AttractorApiService::new(settings.clone());

    let response = service.save_flow(SaveFlowRequest {
        name: "broken".to_string(),
        content: r#"schema_version: "1"
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
    });

    assert_eq!(response.status_code, 422);
    assert_eq!(response.body["detail"]["status"], json!("validation_error"));
    assert_eq!(
        response.body["detail"]["errors"][0]["rule_id"],
        json!("edge_target")
    );
    assert_eq!(
        response.body["detail"]["errors"][0]["edge"],
        json!(["start", "missing"])
    );
    assert!(!settings.flows_dir.join("broken").exists());
}

#[test]
fn save_splits_nested_flow_into_a_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = AttractorApiService::new(settings.clone());

    let created = service.save_flow(SaveFlowRequest {
        name: "nested/new-flow".to_string(),
        content: VALID_FLOW.to_string(),
    });
    assert_eq!(created.status_code, 200);
    assert_eq!(
        created.body,
        json!({"status": "saved", "name": "nested/new-flow"})
    );
    let flow_dir = settings.flows_dir.join("nested/new-flow");
    let mut files = fs::read_dir(&flow_dir)
        .expect("flow dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .into_string()
                .expect("name")
        })
        .collect::<Vec<_>>();
    files.sort();
    assert_eq!(files, ["done.md", "flow.md", "start.md", "task.md"]);
    assert!(fs::read_to_string(flow_dir.join("flow.md"))
        .expect("flow.md")
        .starts_with("---\ntype: Flow\n"));

    // A folder of flows, or a path inside a flow, is not a flow directory.
    for name in ["nested", "nested/new-flow/inner"] {
        let refused = service.save_flow(SaveFlowRequest {
            name: name.to_string(),
            content: VALID_FLOW.to_string(),
        });
        assert_eq!(refused.status_code, 409, "{name}: {}", refused.body);
    }
    assert!(!settings.flows_dir.join("nested/flow.md").exists());
    assert!(!flow_dir.join("inner").exists());
}

#[test]
fn delete_flow_removes_its_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings.flows_dir.join("delete-me"));
    let service = AttractorApiService::new(settings.clone());

    let response = service.delete_flow("delete-me");

    assert_eq!(response.status_code, 200);
    assert_eq!(response.body, json!({"status": "deleted"}));
    assert!(!settings.flows_dir.join("delete-me").exists());
}

fn write_flow(dir: &Path) {
    write_flow_dir(dir, &parse_flow_definition(VALID_FLOW).expect("valid flow")).expect("write");
}

fn settings(root: &Path) -> SparkSettings {
    SparkSettings {
        startup_sources: Default::default(),
        connections: Default::default(),
        providers: Default::default(),
        agents: Default::default(),
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
