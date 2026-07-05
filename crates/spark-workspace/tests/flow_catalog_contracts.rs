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
        "rich.dot",
        r#"digraph rich {
  graph [label="Graph Label", goal="Graph goal", spark.title="Workspace Title", spark.description="Workspace description"];
  start [shape=Mdiamond];
  done [shape=Msquare];
  start -> done;
}
"#,
    );
    write_flow(
        &settings,
        "ops/review/nested.dot",
        r#"digraph nested {
  graph [spark.title="Nested Flow", spark.description="Nested flow description"];
  start [shape=Mdiamond];
  done [shape=Msquare];
  start -> done;
}
"#,
    );

    let flows = WorkspaceFlowService::new(settings)
        .list_flows(Some("human"))
        .expect("list");

    assert_eq!(flows.len(), 2);
    assert_eq!(flows[0].name, "ops/review/nested.dot");
    assert_eq!(flows[0].title, "Nested Flow");
    assert_eq!(flows[0].description, "Nested flow description");
    assert_eq!(flows[1].name, "rich.dot");
    assert_eq!(flows[1].title, "Workspace Title");
    assert_eq!(flows[1].description, "Workspace description");
    assert_eq!(flows[1].graph_label, "Graph Label");
    assert_eq!(flows[1].graph_goal, "Graph goal");
}

#[test]
fn agent_surface_filters_non_requestable_flows_and_hides_detail() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(
        &settings,
        "requestable.dot",
        "digraph requestable { start -> done; }\n",
    );
    write_flow(
        &settings,
        "trigger-only.dot",
        "digraph trigger_only { start -> done; }\n",
    );
    set_flow_launch_policy(
        &settings.config_dir,
        "requestable.dot",
        LAUNCH_POLICY_AGENT_REQUESTABLE,
    )
    .expect("policy");
    set_flow_launch_policy(&settings.config_dir, "trigger-only.dot", "trigger_only")
        .expect("policy");
    let service = WorkspaceFlowService::new(settings);

    let flows = service.list_flows(Some("agent")).expect("list");
    assert_eq!(
        flows
            .iter()
            .map(|flow| flow.name.as_str())
            .collect::<Vec<_>>(),
        ["requestable.dot"]
    );
    let hidden = service
        .describe_flow("trigger-only.dot", Some("agent"))
        .expect_err("hidden");
    assert_eq!(hidden.status_code(), 404);
    assert_eq!(hidden.detail(), "Unknown flow: trigger-only.dot");
}

#[test]
fn describe_raw_validate_and_launch_policy_update_use_catalog_and_dot_sources() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let flow_content = r#"digraph inspectable {
  graph [label="Inspectable Graph", goal="Inspect graph behavior"];
  start [shape=Mdiamond];
  human_review [shape=hexagon];
  manager [shape=house];
  done [shape=Msquare];
  start -> human_review;
  human_review -> manager;
  manager -> done;
}
"#;
    write_flow(&settings, "ops/review/inspectable.dot", flow_content);
    set_flow_launch_policy(
        &settings.config_dir,
        "ops/review/inspectable.dot",
        LAUNCH_POLICY_AGENT_REQUESTABLE,
    )
    .expect("policy");
    let service = WorkspaceFlowService::new(settings.clone());

    let description = service
        .describe_flow("ops/review/inspectable.dot", Some("agent"))
        .expect("describe");
    assert_eq!(description.summary.title, "Inspectable Graph");
    assert_eq!(description.node_count, 4);
    assert_eq!(description.edge_count, 3);
    assert!(description.features.has_human_gate);
    assert!(description.features.has_manager_loop);

    let raw = service
        .raw_flow("ops/review/inspectable.dot", Some("agent"))
        .expect("raw");
    assert_eq!(raw.name, "ops/review/inspectable.dot");
    assert_eq!(raw.content, flow_content);

    let validation = service
        .validate_flow("ops/review/inspectable.dot")
        .expect("validate");
    assert_eq!(validation["name"], "ops/review/inspectable.dot");
    assert_eq!(validation["status"], "ok");
    assert_eq!(validation["diagnostics"], json!([]));

    let response = service
        .update_launch_policy(
            "ops/review/inspectable.dot",
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
    assert_eq!(response.name, "ops/review/inspectable.dot");
    assert_eq!(response.launch_policy.as_deref(), Some("trigger_only"));
    assert_eq!(
        response.execution_lock.as_ref().expect("lock").scope,
        "project"
    );
    assert_eq!(
        read_flow_launch_policy(&settings.config_dir, "ops/review/inspectable.dot")
            .expect("stored")
            .launch_policy
            .as_deref(),
        Some("trigger_only")
    );
}

#[test]
fn validation_endpoint_surfaces_parse_errors_and_path_safety_errors() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings, "broken.dot", "digraph broken { start -> \n");
    let service = WorkspaceFlowService::new(settings);

    let validation = service.validate_flow("broken.dot").expect("parse payload");
    assert_eq!(validation["name"], "broken.dot");
    assert_eq!(validation["status"], "parse_error");
    assert_eq!(validation["diagnostics"][0]["rule_id"], "parse_error");

    let invalid_surface = service.list_flows(Some("invalid")).expect_err("surface");
    assert_eq!(invalid_surface.status_code(), 400);
    assert_eq!(
        invalid_surface.detail(),
        "Flow surface must be 'human' or 'agent'."
    );

    let escaped = service
        .raw_flow("../escape.dot", Some("human"))
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
