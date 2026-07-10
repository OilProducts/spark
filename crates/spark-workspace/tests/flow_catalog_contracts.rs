use std::fs;
use std::path::Path;

use serde_json::json;
use spark_common::settings::SparkSettings;
use spark_storage::{
    read_flow_launch_policy, set_flow_launch_policy, LAUNCH_POLICY_AGENT_REQUESTABLE,
};
use spark_workspace::{WorkspaceFlowLaunchPolicyUpdate, WorkspaceFlowService};

#[test]
fn human_list_preserves_metadata_fallbacks_and_nested_names() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(
        &settings,
        "rich.yaml",
        "schema_version: '1'\nid: rich\ntitle: Workspace Title\ndescription: Workspace description\ngoal: Graph goal\nnodes:\n  start:\n    kind: start\n  done:\n    kind: exit\nedges:\n  - from: start\n    to: done\n",
    );
    write_flow(
        &settings,
        "ops/review/nested.yaml",
        "schema_version: '1'\nid: nested\ntitle: Nested Flow\ndescription: Nested flow description\nnodes:\n  start:\n    kind: start\n  done:\n    kind: exit\nedges:\n  - from: start\n    to: done\n",
    );

    let flows = WorkspaceFlowService::new(settings)
        .list_flows(Some("human"))
        .expect("list");

    assert_eq!(flows.len(), 2);
    assert_eq!(flows[0].name, "ops/review/nested.yaml");
    assert_eq!(flows[0].title, "Nested Flow");
    assert_eq!(flows[0].description, "Nested flow description");
    assert_eq!(flows[1].name, "rich.yaml");
    assert_eq!(flows[1].title, "Workspace Title");
    assert_eq!(flows[1].description, "Workspace description");
    assert_eq!(flows[1].graph_label, "Workspace Title");
    assert_eq!(flows[1].graph_goal, "Graph goal");
}

#[test]
fn agent_surface_filters_non_requestable_flows_and_hides_detail() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(
        &settings,
        "requestable.yaml",
        "schema_version: '1'\nid: requestable\ntitle: Requestable\nnodes:\n  start:\n    kind: start\n  done:\n    kind: exit\nedges:\n  - from: start\n    to: done\n",
    );
    write_flow(
        &settings,
        "trigger-only.yaml",
        "schema_version: '1'\nid: trigger-only\ntitle: Trigger Only\nnodes:\n  start:\n    kind: start\n  done:\n    kind: exit\nedges:\n  - from: start\n    to: done\n",
    );
    set_flow_launch_policy(
        &settings.config_dir,
        "requestable.yaml",
        LAUNCH_POLICY_AGENT_REQUESTABLE,
    )
    .expect("policy");
    set_flow_launch_policy(&settings.config_dir, "trigger-only.yaml", "trigger_only")
        .expect("policy");
    let service = WorkspaceFlowService::new(settings);

    let flows = service.list_flows(Some("agent")).expect("list");
    assert_eq!(
        flows
            .iter()
            .map(|flow| flow.name.as_str())
            .collect::<Vec<_>>(),
        ["requestable.yaml"]
    );
    let hidden = service
        .describe_flow("trigger-only.yaml", Some("agent"))
        .expect_err("hidden");
    assert_eq!(hidden.status_code(), 404);
    assert_eq!(hidden.detail(), "Unknown flow: trigger-only.yaml");
}

#[test]
fn describe_raw_validate_and_launch_policy_update_use_catalog_and_yaml_sources() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let flow_content = "schema_version: '1'\nid: inspectable\ntitle: Inspectable Graph\ngoal: Inspect graph behavior\nnodes:\n  start:\n    kind: start\n  human_review:\n    kind: human_gate\n    label: Human Review\n    config:\n      kind: human_gate\n      prompt: Review this flow\n  manager:\n    kind: subflow\n    label: Manager\n    config:\n      kind: subflow\n      flow_ref: ops/review/child.yaml\n  done:\n    kind: exit\nedges:\n  - from: start\n    to: human_review\n  - from: human_review\n    to: manager\n  - from: manager\n    to: done\n";
    write_flow(&settings, "ops/review/inspectable.yaml", flow_content);
    set_flow_launch_policy(
        &settings.config_dir,
        "ops/review/inspectable.yaml",
        LAUNCH_POLICY_AGENT_REQUESTABLE,
    )
    .expect("policy");
    let service = WorkspaceFlowService::new(settings.clone());

    let description = service
        .describe_flow("ops/review/inspectable.yaml", Some("agent"))
        .expect("describe");
    assert_eq!(description.summary.title, "Inspectable Graph");
    assert_eq!(description.node_count, 4);
    assert_eq!(description.edge_count, 3);
    assert!(description.features.has_human_gate);
    assert!(description.features.has_manager_loop);

    let raw = service
        .raw_flow("ops/review/inspectable.yaml", Some("agent"))
        .expect("raw");
    assert_eq!(raw.name, "ops/review/inspectable.yaml");
    assert_eq!(raw.content, flow_content);

    let validation = service
        .validate_flow("ops/review/inspectable.yaml")
        .expect("validate");
    assert_eq!(validation["name"], "ops/review/inspectable.yaml");
    assert_eq!(validation["status"], "ok");
    assert_eq!(validation["diagnostics"], json!([]));

    let response = service
        .update_launch_policy(
            "ops/review/inspectable.yaml",
            WorkspaceFlowLaunchPolicyUpdate {
                launch_policy: "trigger_only".to_string(),
                execution_lock: Some(json!({
                    "scope": "project",
                    "key": "main-worktree-integration",
                    "conflict_policy": "queue",
                })),
            },
        )
        .expect("policy update");
    assert_eq!(response.name, "ops/review/inspectable.yaml");
    assert_eq!(response.launch_policy.as_deref(), Some("trigger_only"));
    assert_eq!(
        response.execution_lock.as_ref().expect("lock").scope,
        "project"
    );
    assert_eq!(
        read_flow_launch_policy(&settings.config_dir, "ops/review/inspectable.yaml")
            .expect("stored")
            .launch_policy
            .as_deref(),
        Some("trigger_only")
    );
}

#[test]
fn validation_endpoint_surfaces_flow_definition_errors_and_path_safety_errors() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(
        &settings,
        "broken.yaml",
        "schema_version: '1'\nid: broken\nnodes: {\n",
    );
    let service = WorkspaceFlowService::new(settings);

    let validation = service.validate_flow("broken.yaml").expect("parse payload");
    assert_eq!(validation["name"], "broken.yaml");
    assert_eq!(validation["status"], "validation_error");
    assert_eq!(validation["diagnostics"][0]["rule_id"], "parse_error");

    let invalid_surface = service.list_flows(Some("invalid")).expect_err("surface");
    assert_eq!(invalid_surface.status_code(), 400);
    assert_eq!(
        invalid_surface.detail(),
        "Flow surface must be 'human' or 'agent'."
    );

    let escaped = service
        .raw_flow("../escape.yaml", Some("human"))
        .expect_err("escape");
    assert_eq!(escaped.status_code(), 400);
    assert_eq!(
        escaped.detail(),
        "Flow name must be a relative path inside flows_dir."
    );
}

fn write_flow(settings: &SparkSettings, name: &str, content: &str) {
    let path = settings.flows_dir.join(name);
    fs::create_dir_all(path.parent().expect("flow parent")).expect("flow parent");
    fs::write(path, content).expect("flow");
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
