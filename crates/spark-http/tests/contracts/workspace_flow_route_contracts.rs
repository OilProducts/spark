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

#[tokio::test]
async fn workspace_org_routes_show_layers_history_copies_and_memory_read_only() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let bundled = settings.bundled_dir();
    write_flow(&settings, "shared", &simple_flow("shared"));
    write_flow(&settings, "plain", &simple_flow("plain"));
    let role = "---\ntype: Role\n---\n\nYou review.\n";
    std::fs::create_dir_all(bundled.join("roles")).expect("roles dir");
    std::fs::write(bundled.join("roles/reviewer.md"), role).expect("role");
    std::fs::write(bundled.join("roles/coordinator.md"), role).expect("role");

    let project = temp.path().join("project");
    let org = spark_common::project::ensure_org_repo(&project).expect("org");
    let git = |args: &[&str]| {
        spark_common::project::run_git(
            &org,
            &[
                &[
                    "-c",
                    "user.name=Org Test",
                    "-c",
                    "user.email=org@example.com",
                ],
                args,
            ]
            .concat(),
        )
        .expect("git")
    };
    // `shared` is copied unmodified, then bundled moves on; `local` is the
    // project's own and uses the bundled reviewer role.
    let write_org_flow = |name: &str, content: &str| {
        attractor_dsl::write_flow_dir(
            &org.join("flows").join(name),
            &attractor_dsl::parse_flow_definition(content).expect("valid flow"),
        )
        .expect("write org flow")
    };
    write_org_flow("shared", &simple_flow("shared"));
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "Copy shared from bundled"]);
    write_org_flow(
        "local",
        "schema_version: '1'\nid: local\ntitle: local\nnodes:\n  start:\n    kind: start\n  review:\n    kind: agent_task\n    role: reviewer\n    config:\n      kind: agent_task\n      prompt: Review.\n  done:\n    kind: exit\nedges:\n  - from: start\n    to: review\n  - from: review\n    to: done\n",
    );
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "Add local"]);
    write_flow(
        &settings,
        "shared",
        &simple_flow("shared").replace("title: shared", "title: Shared v2"),
    );
    std::fs::create_dir_all(org.join("roles")).expect("org roles");
    std::fs::write(org.join("roles/helper.md"), role).expect("untracked role");
    let memory = org.join("memory/coordinator");
    std::fs::create_dir_all(memory.join("notes")).expect("memory");
    std::fs::write(memory.join("index.md"), "# Known\n").expect("index");
    std::fs::write(memory.join("notes/a.md"), "note a\n").expect("note");
    let head = git(&["rev-parse", "HEAD"]);

    let app = build_app(settings.clone());
    let query = format!("project_path={}", project.display());
    let get = |uri: String| {
        let app = app.clone();
        async move { request_json(app, "GET", &uri, None).await }
    };

    let (status, list, _) = get(format!("/workspace/api/org?{query}")).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let items = list["items"].as_array().expect("items");
    let find = |kind: &str, name: &str| {
        items
            .iter()
            .find(|item| item["kind"] == kind && item["name"] == name)
            .unwrap_or_else(|| panic!("{kind} {name} in {list}"))
            .clone()
    };
    assert_eq!(find("flow", "shared")["layer"], "project_copy");
    assert_eq!(find("flow", "shared")["path"], "flows/shared");
    assert_eq!(find("flow", "plain")["layer"], "bundled");
    assert_eq!(find("flow", "local")["layer"], "project");
    assert_eq!(find("flow", "local")["uncommitted"], false);
    let helper = find("role", "helper");
    assert_eq!(helper["layer"], "project");
    assert_eq!(helper["uncommitted"], true);
    assert_eq!(helper["standing"], Value::Null);
    let reviewer = find("role", "reviewer");
    assert_eq!(reviewer["layer"], "bundled");
    assert_eq!(
        reviewer["usage"],
        json!([{"flow": "local", "node": "review"}])
    );
    let coordinator = find("role", "coordinator");
    assert_eq!(coordinator["usage"], json!([]));
    assert_eq!(coordinator["standing"]["state"], "idle");
    assert_eq!(coordinator["standing"]["budget"]["runs_per_day"], 20);
    assert!(coordinator["standing"]["conversation_id"]
        .as_str()
        .expect("conversation")
        .ends_with("-coordinator"));

    // A project copy carries its diff from today's bundled version and
    // whether bundled changed since the copy's first commit.
    let (status, copy, _) = get(format!(
        "/workspace/api/org/item?{query}&kind=flow&name=shared"
    ))
    .await;
    assert_eq!(status, StatusCode::OK, "{copy}");
    assert_eq!(copy["bundled_changed"], true);
    assert!(copy["bundled_diff"]
        .as_str()
        .expect("diff")
        .contains("-title: Shared v2"));
    let flow_file = copy["files"]
        .as_array()
        .expect("files")
        .iter()
        .find(|file| file["name"] == "flow.md")
        .expect("flow.md")
        .clone();
    assert!(flow_file["content"]
        .as_str()
        .expect("content")
        .contains("title: shared"));
    assert_eq!(flow_file["status"], Value::Null);
    let (_, plain, _) = get(format!(
        "/workspace/api/org/item?{query}&kind=flow&name=plain"
    ))
    .await;
    assert_eq!(plain["layer"], "bundled");
    assert_eq!(plain["bundled_changed"], Value::Null);
    let (_, helper, _) = get(format!(
        "/workspace/api/org/item?{query}&kind=role&name=helper"
    ))
    .await;
    assert_eq!(
        helper["files"],
        json!([{"name": "helper.md", "content": role, "status": "untracked"}])
    );
    // Copying shared again makes bundled match the first commit.
    write_flow(&settings, "shared", &simple_flow("shared"));
    let (_, same, _) = get(format!(
        "/workspace/api/org/item?{query}&kind=flow&name=shared"
    ))
    .await;
    assert_eq!(same["bundled_changed"], false);
    assert_eq!(same["bundled_diff"], "");

    let (status, history, _) = get(format!(
        "/workspace/api/org/history?{query}&path=flows/shared"
    ))
    .await;
    assert_eq!(status, StatusCode::OK, "{history}");
    let commits = history["commits"].as_array().expect("commits");
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0]["message"], "Copy shared from bundled");
    assert_eq!(commits[0]["author"], "Org Test");
    assert!(commits[0]["diff"]
        .as_str()
        .expect("diff")
        .contains("+title: shared"));
    let elsewhere = temp.path().join("no-org");
    let (status, empty, _) = get(format!(
        "/workspace/api/org/history?project_path={}&path=flows/shared",
        elsewhere.display()
    ))
    .await;
    assert_eq!((status, empty), (StatusCode::OK, json!({"commits": []})));

    let (status, files, _) = get(format!(
        "/workspace/api/org/memory?{query}&role=coordinator"
    ))
    .await;
    assert_eq!(status, StatusCode::OK, "{files}");
    assert_eq!(files["files"], json!(["index.md", "notes/a.md"]));
    let (_, note, _) = get(format!(
        "/workspace/api/org/memory?{query}&role=coordinator&file=notes/a.md"
    ))
    .await;
    assert_eq!(note["content"], "note a\n");

    // Paths stay inside the org repo and the bundled root.
    for uri in [
        format!("/workspace/api/org/history?{query}&path=../outside"),
        format!("/workspace/api/org/history?{query}&path=/etc/passwd"),
        format!(
            "/workspace/api/org/memory?{query}&role=coordinator&file=../../flows/shared/flow.md"
        ),
        format!("/workspace/api/org/memory?{query}&role=..&file=flows/shared/flow.md"),
        format!("/workspace/api/org/item?{query}&kind=flow&name=../shared"),
        format!("/workspace/api/org/item?{query}&kind=role&name=../roles/reviewer"),
    ] {
        let (status, body, _) = get(uri.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {body}");
    }
    // Nothing above wrote to the org repo.
    assert_eq!(git(&["rev-parse", "HEAD"]), head);
    assert_eq!(git(&["status", "--porcelain"]), "?? memory/\n?? roles/");
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
