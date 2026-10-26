use std::path::Path;

use attractor_api::AttractorApiService;
use attractor_api::{
    handle_attractor_request, handle_preview_request, preview, preview_with_roots, PreviewRequest,
    PreviewServiceConfig,
};
use attractor_core::FlowDefinition;
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;

#[test]
fn preview_service_body_reports_flow_definition_graph() {
    let response = preview(PreviewRequest {
        flow: Some(simple_flow()),
        flow_name: None,
        expand_children: false,
        project_path: None,
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
fn preview_route_wrapper_rejects_a_flow_that_is_not_a_flow_definition() {
    let body = json!({"flow": "nodes: ["}).to_string();

    let response =
        handle_preview_request("POST", "/preview", &body, &PreviewServiceConfig::default());

    assert_eq!(response.status_code, 400);
    assert!(response.body["detail"]
        .as_str()
        .expect("detail")
        .starts_with("Invalid preview request JSON"));
}

#[test]
fn preview_without_a_flow_or_flow_name_is_a_bad_request() {
    let response = preview(PreviewRequest::default());

    assert_eq!(response.status_code, 400);
}

#[test]
fn preview_service_reports_validation_errors_with_http_success_status() {
    let response = preview(PreviewRequest {
        flow: Some(flow(json!({
            "schema_version": "1",
            "id": "broken",
            "nodes": {"start": {"kind": "start"}, "done": {"kind": "exit"}},
            "edges": [{"from": "start", "to": "missing"}]
        }))),
        flow_name: None,
        expand_children: false,
        project_path: None,
    });

    assert_eq!(response.status_code, 200);
    assert_eq!(response.body["status"], json!("validation_error"));
    assert_eq!(response.body["errors"][0]["rule_id"], json!("edge_target"));
    assert_eq!(
        response.body["errors"][0]["edge"],
        json!(["start", "missing"])
    );
}

#[test]
fn preview_service_accepts_flow_name_and_expand_children_flags() {
    let temp = tempfile::tempdir().expect("tempdir");
    let response = preview_with_roots(
        PreviewRequest {
            flow: Some(simple_flow()),
            flow_name: Some("nested/parent".to_string()),
            expand_children: true,
            project_path: None,
        },
        &attractor_dsl::FlowRoots::new(temp.path(), None),
    );

    assert_eq!(response.status_code, 200);
    assert_eq!(response.body["status"], json!("ok"));
}

#[test]
fn preview_service_expands_subflow_children_by_name_through_the_roots() {
    let temp = tempfile::tempdir().expect("tempdir");
    attractor_dsl::write_flow_dir(&temp.path().join("flows/nested/child"), &child_flow())
        .expect("child flow");
    let project = temp.path().join("project");
    let mut project_child = child_flow();
    project_child.title = "Project Child".to_string();
    attractor_dsl::write_flow_dir(
        &spark_common::project::org_dir(&project).join("flows/nested/child"),
        &project_child,
    )
    .expect("project child flow");
    let request = |project: Option<&Path>| {
        preview_with_roots(
            parent_request(),
            &attractor_dsl::FlowRoots::new(temp.path(), project),
        )
    };

    let response = request(None);
    assert_eq!(response.status_code, 200);
    assert_eq!(response.body["status"], json!("ok"));
    assert_eq!(
        response.body["graph"]["child_previews"]["child"]["flow_name"],
        json!("nested/child")
    );
    assert_eq!(
        response.body["graph"]["child_previews"]["child"]["flow_label"],
        json!("Child")
    );
    assert!(
        response.body["graph"]["child_previews"]["child"]["graph"]["nodes"]
            .as_array()
            .expect("child nodes")
            .iter()
            .any(|node| node["kind"] == json!("human_gate"))
    );
    // A project copy of the child wins.
    assert_eq!(
        request(Some(&project)).body["graph"]["child_previews"]["child"]["flow_label"],
        json!("Project Child")
    );
}

fn parent_request() -> PreviewRequest {
    PreviewRequest {
        flow: Some(flow(json!({
            "schema_version": "1",
            "id": "parent",
            "title": "Parent",
            "nodes": {
                "start": {"kind": "start"},
                "child": {
                    "kind": "subflow",
                    "label": "Child",
                    "config": {"kind": "subflow", "flow_ref": "child"}
                },
                "done": {"kind": "exit"}
            },
            "edges": [
                {"from": "start", "to": "child"},
                {"from": "child", "to": "done"}
            ]
        }))),
        flow_name: Some("nested/parent".to_string()),
        expand_children: true,
        project_path: None,
    }
}

#[test]
fn preview_route_wrapper_accepts_post_preview_json() {
    let body = json!({
        "flow": simple_flow(),
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
        "flow": simple_flow(),
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
fn preview_route_previews_a_saved_flow_by_name_resolving_org_first() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = temp.path().join("project");
    let mut bundled = serde_json::to_value(simple_flow()).expect("flow json");
    bundled["title"] = json!("Bundled Preview");
    let name = crate::install_flow(&settings, bundled.clone());
    let service = AttractorApiService::new(settings);
    let preview_saved = |project_path: Option<&Path>| {
        service.dispatch(
            "POST",
            "/preview",
            &json!({
                "flow_name": name,
                "project_path": project_path.map(|path| path.to_string_lossy().to_string()),
            })
            .to_string(),
        )
    };

    let response = preview_saved(Some(&project));
    assert_eq!(response.status_code, 200);
    assert_eq!(response.body["status"], json!("ok"));
    assert_eq!(response.body["graph"]["title"], json!("Bundled Preview"));

    let mut org = bundled;
    org["title"] = json!("Org Preview");
    assert_eq!(crate::install_org_flow(&project, org), name);
    assert_eq!(
        preview_saved(Some(&project)).body["graph"]["title"],
        json!("Org Preview")
    );
    // Without a project only the bundled library resolves.
    assert_eq!(
        preview_saved(None).body["graph"]["title"],
        json!("Bundled Preview")
    );
}

#[test]
fn preview_route_accepts_a_json_flow_definition() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = AttractorApiService::new(settings(temp.path()));

    let response = service.dispatch(
        "POST",
        "/preview",
        &json!({"flow": simple_flow()}).to_string(),
    );

    assert_eq!(response.status_code, 200);
    assert_eq!(response.body["status"], json!("ok"));
    assert_eq!(response.body["graph"]["id"], json!("preview"));
    assert_eq!(
        response.body["flow"]["nodes"]["task"]["config"]["prompt"],
        json!("Preview task")
    );
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

fn flow(value: Value) -> FlowDefinition {
    serde_json::from_value(value).expect("flow definition JSON")
}

fn simple_flow() -> FlowDefinition {
    flow(json!({
        "schema_version": "1",
        "id": "preview",
        "title": "Preview",
        "nodes": {
            "start": {"kind": "start"},
            "task": {
                "kind": "agent_task",
                "label": "Task",
                "config": {"kind": "agent_task", "prompt": "Preview task"}
            },
            "done": {"kind": "exit"}
        },
        "edges": [
            {"from": "start", "to": "task"},
            {"from": "task", "to": "done"}
        ]
    }))
}

fn child_flow() -> FlowDefinition {
    flow(json!({
        "schema_version": "1",
        "id": "child",
        "title": "Child",
        "nodes": {
            "start": {"kind": "start"},
            "human": {
                "kind": "human_gate",
                "label": "Human",
                "config": {"kind": "human_gate", "prompt": "Review"}
            },
            "done": {"kind": "exit"}
        },
        "edges": [
            {"from": "start", "to": "human"},
            {"from": "human", "to": "done"}
        ]
    }))
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
        ui_dir: None,
        project_roots: Vec::new(),
    }
}
