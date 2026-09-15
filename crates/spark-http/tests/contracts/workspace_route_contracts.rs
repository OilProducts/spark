use std::fs;
use std::path::Path;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
use spark_http::build_app;
use tower::ServiceExt;

#[tokio::test]
async fn codex_connection_routes_validate_login_input_and_cancel_without_changing_files() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let app = build_app(settings.clone());
    for body in [
        json!({"method": "unknown"}),
        json!({"method": "browser", "access_token": "unexpected"}),
        json!({}),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/workspace/api/codex/login")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/workspace/api/codex/login")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(body["status"], "disconnected");
    assert!(body["login_url"].is_null());
    assert!(!settings.runtime_dir.join("codex/.codex/auth.json").exists());
}

#[tokio::test]
async fn workspace_project_routes_persist_records_and_return_json_errors() {
    let temp = tempfile::tempdir().expect("tempdir");
    // Canonicalize: registered project paths come back canonical (macOS /var -> /private/var).
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("project");
    let app = build_app(settings.clone());

    let register = request_json(
        app.clone(),
        "POST",
        "/workspace/api/projects/register",
        Some(json!({"project_path": project_dir})),
    )
    .await;
    assert_eq!(register.0, StatusCode::OK);
    assert_eq!(
        register.1["project_path"].as_str(),
        Some(project_dir.to_string_lossy().as_ref())
    );
    assert_eq!(register.1["display_name"], "project");

    let list = request_json(app.clone(), "GET", "/workspace/api/projects", None).await;
    assert_eq!(list.0, StatusCode::OK);
    assert_eq!(list.1.as_array().expect("projects").len(), 1);

    let project_file = settings
        .projects_dir
        .join(register.1["project_id"].as_str().expect("project id"))
        .join("project.toml");
    assert!(project_file.exists());

    let missing = request_json(app, "GET", "/workspace/api/missing", None).await;
    assert_eq!(missing.0, StatusCode::NOT_FOUND);
    assert_eq!(missing.1, json!({"detail": "Not Found"}));
}

#[tokio::test]
async fn workspace_browse_and_metadata_routes_match_validation_contracts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("Browse Root");
    fs::create_dir_all(root.join("Alpha")).expect("alpha");
    fs::create_dir_all(root.join("zeta")).expect("zeta");
    fs::write(root.join("notes.txt"), "ignored").expect("file");
    let mut settings = settings(temp.path());
    settings.project_roots = vec![root.clone()];
    let app = build_app(settings);

    let browse = request_json(app.clone(), "GET", "/workspace/api/projects/browse", None).await;
    assert_eq!(browse.0, StatusCode::OK);
    assert_eq!(
        browse.1["current_path"].as_str(),
        Some(root.to_string_lossy().as_ref())
    );
    assert_eq!(browse.1["entries"][0]["name"], "Alpha");
    assert_eq!(browse.1["entries"][1]["name"], "zeta");

    let bad_browse = request_json(
        app.clone(),
        "GET",
        "/workspace/api/projects/browse?path=relative",
        None,
    )
    .await;
    assert_eq!(bad_browse.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        bad_browse.1,
        json!({"detail": "Browse path must be absolute."})
    );

    let metadata = request_json(
        app,
        "GET",
        &format!(
            "/workspace/api/projects/metadata?directory={}",
            root.to_string_lossy().replace(' ', "%20")
        ),
        None,
    )
    .await;
    assert_eq!(metadata.0, StatusCode::OK);
    assert_eq!(metadata.1["name"], "Browse Root");
    assert_eq!(metadata.1["branch"], Value::Null);
    assert_eq!(metadata.1["commit"], Value::Null);
}

#[tokio::test]
async fn workspace_settings_wrap_execution_placement_payload() {
    let temp = tempfile::tempdir().expect("tempdir");
    let response = request_json(
        build_app(settings(temp.path())),
        "GET",
        "/workspace/api/settings",
        None,
    )
    .await;

    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(
        response.1["execution_placement"]["execution_modes"],
        json!(["native", "local_container"])
    );
    assert_eq!(
        response.1["execution_placement"]["profiles"][0]["id"],
        "native"
    );
}

#[tokio::test]
async fn attractor_routes_remain_mounted_when_workspace_routes_are_composed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = build_app(settings(temp.path()));

    let status = request_json(app.clone(), "GET", "/attractor/status", None).await;
    assert_eq!(status.0, StatusCode::OK);
    assert!(status.2.starts_with("application/json"));
    assert_eq!(status.1["status"], "idle");

    let placement = request_json(
        app,
        "GET",
        "/attractor/api/execution-placement-settings",
        None,
    )
    .await;
    assert_eq!(placement.0, StatusCode::OK);
    assert!(placement.2.starts_with("application/json"));
    assert_eq!(
        placement.1["execution_modes"],
        json!(["native", "local_container"])
    );
}

#[tokio::test]
async fn workspace_public_extractor_errors_return_json_envelopes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = build_app(settings(temp.path()));

    let missing_query =
        request_json(app.clone(), "GET", "/workspace/api/projects/metadata", None).await;
    assert_eq!(missing_query.0, StatusCode::BAD_REQUEST);
    assert!(missing_query.2.starts_with("application/json"));
    assert!(missing_query.1["detail"]
        .as_str()
        .expect("detail")
        .contains("directory"));

    let malformed_body = request_raw(
        app,
        "POST",
        "/workspace/api/projects/register",
        "{not-json",
        Some("application/json"),
    )
    .await;
    assert_eq!(malformed_body.0, StatusCode::BAD_REQUEST);
    assert!(malformed_body.2.starts_with("application/json"));
    assert!(malformed_body.1["detail"]
        .as_str()
        .expect("detail")
        .contains("Failed to parse"));
}

#[tokio::test]
#[ignore = "legacy state.json fixture is unsupported after the conversation hard cutover"]
async fn project_conversations_route_returns_summary_shape_for_existing_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_dir = temp.path().join("project-a");
    fs::create_dir_all(&project_dir).expect("project dir");
    let app = build_app(settings.clone());

    let register = request_json(
        app.clone(),
        "POST",
        "/workspace/api/projects/register",
        Some(json!({"project_path": project_dir})),
    )
    .await;
    assert_eq!(register.0, StatusCode::OK);
    let project_id = register.1["project_id"].as_str().expect("project id");
    let conversations_dir = settings.projects_dir.join(project_id).join("conversations");
    write_state(
        &conversations_dir,
        "conversation-a",
        json!({
            "schema_version": 5,
            "revision": 2,
            "conversation_id": "conversation-a",
            "conversation_handle": "amber-anchor",
            "project_path": project_dir,
            "title": "Design thread",
            "created_at": "2026-03-07T14:00:00Z",
            "updated_at": "2026-03-07T14:02:00Z",
            "turns": [
                {
                    "id": "turn-a-1",
                    "role": "user",
                    "content": "Design thread preview",
                    "timestamp": "2026-03-07T14:02:00Z",
                    "kind": "message"
                }
            ],
            "segments": []
        }),
    );

    let listed = request_json(
        app,
        "GET",
        &format!(
            "/workspace/api/projects/conversations?project_path={}",
            register.1["project_path"].as_str().expect("project path")
        ),
        None,
    )
    .await;

    assert_eq!(listed.0, StatusCode::OK);
    assert_eq!(listed.1[0]["conversation_id"], "conversation-a");
    assert_eq!(listed.1[0]["conversation_handle"], "amber-anchor");
    assert_eq!(listed.1[0]["title"], "Design thread");
    assert_eq!(listed.1[0]["last_message_preview"], "Design thread preview");
    assert!(listed.1[0].get("turns").is_none());
}

async fn request_json(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    let request = if let Some(body) = body {
        builder = builder.header("content-type", "application/json");
        builder
            .body(Body::from(body.to_string()))
            .expect("request body")
    } else {
        builder.body(Body::empty()).expect("request body")
    };
    execute_request(app, request).await
}

async fn request_raw(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: &str,
    content_type: Option<&str>,
) -> (StatusCode, Value, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(content_type) = content_type {
        builder = builder.header("content-type", content_type);
    }
    let request = builder
        .body(Body::from(body.to_string()))
        .expect("request body");
    execute_request(app, request).await
}

async fn execute_request(app: axum::Router, request: Request<Body>) -> (StatusCode, Value, String) {
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = serde_json::from_slice::<Value>(&bytes).expect("json");
    (status, value, content_type)
}

fn write_state(conversations_dir: &Path, conversation_id: &str, payload: Value) {
    let state_path = conversations_dir.join(conversation_id).join("state.json");
    fs::create_dir_all(state_path.parent().expect("state parent")).expect("state parent");
    fs::write(
        state_path,
        serde_json::to_string_pretty(&payload).expect("json"),
    )
    .expect("state");
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
        flows_dir: root.join("flows"),
        ui_dir: None,
        project_roots: Vec::new(),
    }
}

#[tokio::test]
async fn task_routes_enforce_minimal_records_and_revision_protection() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project = temp.path().join("tasks-project");
    fs::create_dir_all(&project).unwrap();
    let app = build_app(settings);
    let list_uri = format!("/workspace/api/tasks?project_path={}", project.display());
    let created = request_json(
        app.clone(),
        "POST",
        &list_uri,
        Some(
            json!({"fields":{"title":"Quick capture"},"actor":"assistant","note":"Captured issue"}),
        ),
    )
    .await;
    assert_eq!(created.0, StatusCode::OK);
    assert_eq!(
        created.1["fields"],
        json!({"title":"Quick capture","description":"","stage":"backlog","archived":false})
    );
    assert_eq!(created.1["activity"][0]["actor"], "assistant");
    let uri = format!(
        "/workspace/api/tasks/{}?project_path={}",
        created.1["id"].as_str().unwrap(),
        project.display()
    );
    let updated = request_json(
        app.clone(),
        "PATCH",
        &uri,
        Some(json!({"revision":1,"fields":{"stage":"done"}})),
    )
    .await;
    assert_eq!(updated.0, StatusCode::OK);
    let stale = request_json(
        app.clone(),
        "PATCH",
        &uri,
        Some(json!({"revision":1,"fields":{"title":"Stale"}})),
    )
    .await;
    assert_eq!(stale.0, StatusCode::CONFLICT);
    for key in [
        "priority",
        "acceptance_criteria",
        "next_action",
        "blocked",
        "needs_input",
        "conversations",
        "artifacts",
        "runs",
    ] {
        let rejected = request_json(
            app.clone(),
            "PATCH",
            &uri,
            Some(json!({"revision":2,"fields":{key:null}})),
        )
        .await;
        assert_eq!(rejected.0, StatusCode::BAD_REQUEST, "{key}: {}", rejected.1);
    }
    let rejected = request_json(
        app.clone(),
        "POST",
        &list_uri,
        Some(json!({"fields":{"title":"No provenance"},"conversation_id":"old"})),
    )
    .await;
    assert_eq!(rejected.0, StatusCode::BAD_REQUEST);
    let listed = request_json(app.clone(), "GET", &list_uri, None).await;
    assert_eq!(listed.1, json!({"tasks":[updated.1]}));
    let fetched = request_json(app.clone(), "GET", &uri, None).await;
    assert_eq!(fetched.1, updated.1);
    let other = temp.path().join("other");
    fs::create_dir(&other).unwrap();
    let other_uri = format!(
        "/workspace/api/tasks/{}?project_path={}",
        created.1["id"].as_str().unwrap(),
        other.display()
    );
    assert_eq!(
        request_json(app, "GET", &other_uri, None).await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn runtime_settings_http_preserve_sections_reject_stale_writes_and_scope_invalid_values() {
    let temp = tempfile::tempdir().unwrap();
    let config = settings(temp.path());
    fs::create_dir_all(&config.config_dir).unwrap();
    let path = config.config_dir.join("spark.toml");
    fs::write(
        &path,
        "[desktop]\nremote_access_enabled = false\n[extension]\nkeep = 12\n",
    )
    .unwrap();
    let app = build_app(config.clone());
    let first = request_json(app.clone(), "GET", "/workspace/api/settings", None).await;
    let revision = first.1["runtime"]["revision"].clone();
    let payload = json!({"expected_revision": revision, "section": "runtime", "value": {"flows_dir": "/new/flows", "project_roots": []}});
    let valid = request_json(
        app.clone(),
        "POST",
        "/workspace/api/settings/validate",
        Some(payload.clone()),
    )
    .await;
    assert_eq!(valid.0, StatusCode::OK, "{}", valid.1);
    assert_eq!(
        first.1["runtime"]["revision"],
        request_json(app.clone(), "GET", "/workspace/api/settings", None)
            .await
            .1["runtime"]["revision"]
    );
    let saved = request_json(
        app.clone(),
        "PATCH",
        "/workspace/api/settings",
        Some(payload.clone()),
    )
    .await;
    assert_eq!(saved.0, StatusCode::OK, "{}", saved.1);
    assert_eq!(saved.1["runtime"]["stored"]["flows_dir"], "/new/flows");
    assert_eq!(
        saved.1["runtime"]["effective"]["flows_dir"],
        config.flows_dir.to_string_lossy().as_ref()
    );
    assert_ne!(saved.1["runtime"]["revision"], revision);
    let persisted = fs::read_to_string(&path).unwrap();
    assert!(persisted.contains("keep = 12"));
    assert!(persisted.contains("remote_access_enabled = false"));
    let conflict = request_json(
        app.clone(),
        "PATCH",
        "/workspace/api/settings",
        Some(payload),
    )
    .await;
    assert_eq!(conflict.0, StatusCode::CONFLICT);
    let bad = request_json(app.clone(), "PATCH", "/workspace/api/settings", Some(json!({
        "expected_revision": saved.1["runtime"]["revision"], "section": "runtime", "value": {"project_roots": ["relative-secret-value"]}
    }))).await;
    assert_eq!(bad.0, StatusCode::BAD_REQUEST);
    assert!(!bad.1.to_string().contains("relative-secret-value"));
    assert_eq!(fs::read_to_string(&path).unwrap(), persisted);
    let wrong_type = request_json(app.clone(), "PATCH", "/workspace/api/settings", Some(json!({
        "expected_revision": saved.1["runtime"]["revision"], "section": "runtime", "value": {"project_roots": "secret-value"}
    }))).await;
    assert_eq!(wrong_type.0, StatusCode::BAD_REQUEST);
    assert!(!wrong_type.1.to_string().contains("secret-value"));
    fs::write(&path, "[runtime]\nproject_roots = 'secret-from-file'\n").unwrap();
    let bad_file = request_json(app, "GET", "/workspace/api/settings", None).await;
    assert_eq!(bad_file.0, StatusCode::OK);
    assert_eq!(
        bad_file.1["runtime"]["stored"]["project_roots"],
        "secret-from-file"
    );
    assert!(!bad_file.1["runtime"]["validation_errors"]
        .to_string()
        .contains("secret-from-file"));
    assert!(bad_file.1["runtime"]["effective"].is_null());
    assert!(!bad_file.1["runtime"]["repair_defaults"].is_null());
}

#[tokio::test]
async fn model_settings_http_scopes_have_independent_conflicts_and_browser_import_preserves_authority(
) {
    let temp = tempfile::tempdir().unwrap();
    let config = settings(temp.path());
    spark_storage::ProjectRegistry::new(&config.data_dir)
        .register_project("/projects/scoped")
        .unwrap();
    let app = build_app(config.clone());
    let workspace = request_json(app.clone(), "GET", "/workspace/api/settings", None)
        .await
        .1;
    let project = request_json(
        app.clone(),
        "GET",
        "/workspace/api/settings?project_path=/projects/scoped",
        None,
    )
    .await
    .1;
    let group = json!({"provider": "codex", "model": "workspace-model"});
    let payload = json!({"expected_revision": workspace["models"]["revision"], "section": "models", "value": group});
    let saved = request_json(
        app.clone(),
        "PATCH",
        "/workspace/api/settings",
        Some(payload.clone()),
    )
    .await;
    assert_eq!(saved.0, StatusCode::OK, "{}", saved.1);
    assert_eq!(
        request_json(
            app.clone(),
            "PATCH",
            "/workspace/api/settings",
            Some(payload)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let payload = json!({"expected_revision": project["models"]["revision"], "section": "project_models", "value": {"project_path": "/projects/scoped", "model_settings": {"provider": "claude-code"}}});
    let saved_project = request_json(
        app.clone(),
        "PATCH",
        "/workspace/api/settings",
        Some(payload.clone()),
    )
    .await;
    assert_eq!(saved_project.0, StatusCode::OK, "{}", saved_project.1);
    assert_eq!(saved_project.1["models"]["source"], "project");
    assert!(saved_project.1["models"]["effective"]["model"].is_null());
    assert_eq!(
        request_json(
            app.clone(),
            "PATCH",
            "/workspace/api/settings",
            Some(payload)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let unchanged = request_json(app.clone(), "PATCH", "/workspace/api/settings", Some(json!({"expected_revision": saved_project.1["models"]["revision"], "section": "project_models", "value": {"project_path": "/projects/scoped"}}))).await;
    assert_eq!(
        unchanged.1, saved_project.1,
        "omission must not clear the group"
    );
    let cleared = request_json(app.clone(), "PATCH", "/workspace/api/settings", Some(json!({"expected_revision": saved_project.1["models"]["revision"], "section": "project_models", "value": {"project_path": "/projects/scoped", "model_settings": null}}))).await;
    assert_eq!(cleared.1["models"]["source"], "workspace");
    assert_eq!(cleared.1["models"]["effective"]["model"], "workspace-model");
    let bytes = fs::read(config.config_dir.join("spark.toml")).unwrap();
    let imported = request_json(app.clone(), "PATCH", "/workspace/api/settings", Some(json!({"expected_revision": "absent", "section": "import_models", "value": {"provider": "anthropic", "model": "old-browser"}}))).await;
    assert_eq!(imported.0, StatusCode::OK);
    assert_eq!(
        imported.1["models"]["effective"]["model"],
        "workspace-model"
    );
    assert_eq!(
        fs::read(config.config_dir.join("spark.toml")).unwrap(),
        bytes
    );
    let invalid = request_json(app, "PATCH", "/workspace/api/settings", Some(json!({"expected_revision": saved.1["models"]["revision"], "section": "models", "value": {"provider": "openai", "llm_profile": "SECRET_DO_NOT_EXPOSE"}}))).await;
    assert_eq!(invalid.0, StatusCode::BAD_REQUEST);
    assert!(!invalid.1.to_string().contains("SECRET_DO_NOT_EXPOSE"));
    assert_eq!(
        fs::read(config.config_dir.join("spark.toml")).unwrap(),
        bytes
    );
}

#[tokio::test]
async fn conversation_resource_requires_revisions_for_every_model_selector() {
    let temp = tempfile::tempdir().unwrap();
    let config = settings(temp.path());
    let app = build_app(config.clone());
    let uri = "/workspace/api/conversations/revision-chat/settings";
    let read_uri = "/workspace/api/conversations/revision-chat?project_path=/projects/revisions";
    let initial = request_json(app.clone(), "GET", read_uri, None).await;
    assert_eq!(initial.0, StatusCode::OK);
    let initial_revision = initial.1["settings"]["models"]["revision"].clone();
    let selectors = [
        json!({"provider": "openai"}),
        json!({"model": "new-model"}),
        json!({"reasoning_effort": "high"}),
        json!({"llm_profile": "profile"}),
        json!({"model_settings": {"provider": "openai"}}),
        json!({"model_settings": null}),
    ];
    for mut payload in selectors.clone() {
        payload["project_path"] = json!("/projects/revisions");
        let response = request_json(app.clone(), "PUT", uri, Some(payload)).await;
        assert_eq!(response.0, StatusCode::BAD_REQUEST, "{:?}", response.1);
        assert!(response.1["detail"]
            .as_str()
            .unwrap()
            .contains("expected_revision"));
    }
    let saved = request_json(
        app.clone(),
        "PUT",
        uri,
        Some(json!({
            "project_path": "/projects/revisions", "expected_revision": initial_revision,
            "provider": "openai", "model": "gpt-5"
        })),
    )
    .await;
    assert_eq!(saved.0, StatusCode::OK, "{:?}", saved.1);
    let paths = spark_storage::ProjectRegistry::new(&config.data_dir)
        .project_paths("/projects/revisions")
        .unwrap();
    let metadata = paths
        .conversations_dir
        .join("revision-chat/conversation.json");
    let before = fs::read(&metadata).unwrap();
    for mut payload in selectors {
        payload["project_path"] = json!("/projects/revisions");
        payload["expected_revision"] = initial_revision.clone();
        let response = request_json(app.clone(), "PUT", uri, Some(payload)).await;
        assert_eq!(response.0, StatusCode::CONFLICT, "{:?}", response.1);
        assert_eq!(fs::read(&metadata).unwrap(), before);
    }
    let read = request_json(app, "GET", read_uri, None).await;
    assert_eq!(read.1["settings"], saved.1["settings"]);
}

#[tokio::test]
async fn scoped_conversation_settings_get_set_and_inherit_use_common_revisions() {
    let temp = tempfile::tempdir().unwrap();
    let app = build_app(settings(temp.path()));
    let uri =
        "/workspace/api/settings?project_path=/projects/scoped-chat&conversation_id=scoped-chat";
    let before = request_json(app.clone(), "GET", uri, None).await;
    assert_eq!(before.0, StatusCode::OK);
    assert_eq!(before.1["models"]["scope"], "conversation");
    let value = json!({"project_path":"/projects/scoped-chat", "conversation_id":"scoped-chat", "model_settings":{"provider":"claude-code"}});
    let payload = json!({"section":"conversation_models", "expected_revision":before.1["models"]["revision"], "value":value});
    let saved = request_json(
        app.clone(),
        "PATCH",
        "/workspace/api/settings",
        Some(payload.clone()),
    )
    .await;
    assert_eq!(saved.0, StatusCode::OK, "{:?}", saved.1);
    assert_eq!(saved.1["models"]["source"], "conversation");
    let stale = request_json(
        app.clone(),
        "PATCH",
        "/workspace/api/settings",
        Some(payload),
    )
    .await;
    assert_eq!(stale.0, StatusCode::CONFLICT);
    let unchanged = request_json(app.clone(), "PATCH", "/workspace/api/settings", Some(json!({"section":"conversation_models", "expected_revision":saved.1["models"]["revision"], "value":{"project_path":"/projects/scoped-chat", "conversation_id":"scoped-chat"}}))).await;
    assert_eq!(unchanged.1, saved.1);
    let inherited = request_json(app.clone(), "PATCH", "/workspace/api/settings", Some(json!({"section":"conversation_models", "expected_revision":saved.1["models"]["revision"], "value":{"project_path":"/projects/scoped-chat", "conversation_id":"scoped-chat", "model_settings":null}}))).await;
    assert_eq!(inherited.0, StatusCode::OK);
    assert_eq!(inherited.1["models"]["source"], "workspace");
    let read = request_json(app, "GET", uri, None).await;
    assert_eq!(read.1, inherited.1);
}

#[tokio::test]
async fn project_execution_settings_require_revisions_and_null_restores_inheritance() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).unwrap();
    fs::write(
        settings.config_dir.join("execution-profiles.toml"),
        "[profiles.dev]\nlabel = 'Development'\nmode = 'native'\n",
    )
    .unwrap();
    let app = build_app(settings.clone());
    let registered = request_json(
        app.clone(),
        "POST",
        "/workspace/api/projects/register",
        Some(json!({"project_path":"/projects/revision"})),
    )
    .await;
    assert_eq!(registered.0, StatusCode::OK);
    let uri = "/workspace/api/settings?project_path=%2Fprojects%2Frevision";
    let read = request_json(app.clone(), "GET", uri, None).await.1;
    let revision = read["execution"]["revision"].clone();
    let file = spark_storage::ProjectRegistry::new(&settings.data_dir)
        .project_paths("/projects/revision")
        .unwrap()
        .project_file;
    let original = fs::read(&file).unwrap();
    for value in [json!("dev"), Value::Null] {
        let rejected = request_json(
            app.clone(),
            "PATCH",
            "/workspace/api/projects/state",
            Some(json!({"project_path":"/projects/revision", "execution_profile_id":value})),
        )
        .await;
        assert_eq!(rejected.0, StatusCode::BAD_REQUEST, "{rejected:?}");
        assert_eq!(fs::read(&file).unwrap(), original);
    }
    let payload = json!({"project_path":"/projects/revision", "execution_profile_id":"dev", "expected_revision":revision});
    let saved = request_json(
        app.clone(),
        "PATCH",
        "/workspace/api/projects/state",
        Some(payload.clone()),
    )
    .await;
    assert_eq!(saved.0, StatusCode::OK, "{saved:?}");
    let committed = fs::read(&file).unwrap();
    let conflict = request_json(
        app.clone(),
        "PATCH",
        "/workspace/api/projects/state",
        Some(payload),
    )
    .await;
    assert_eq!(conflict.0, StatusCode::CONFLICT);
    assert_eq!(fs::read(&file).unwrap(), committed);
    let read = request_json(app.clone(), "GET", uri, None).await.1;
    assert_eq!(read["execution"]["stored"], "dev");
    let reopen = request_json(
        app.clone(),
        "POST",
        "/workspace/api/projects/register",
        Some(json!({"project_path":"/projects/revision", "execution_profile_id":"dev"})),
    )
    .await;
    assert_eq!(reopen.0, StatusCode::BAD_REQUEST);
    assert_eq!(fs::read(&file).unwrap(), committed);
    let unchanged = request_json(app.clone(), "PATCH", "/workspace/api/settings", Some(json!({"section":"project_execution", "expected_revision":read["execution"]["revision"], "value":{"project_path":"/projects/revision"}}))).await;
    assert_eq!(unchanged.0, StatusCode::OK);
    assert_eq!(
        unchanged.1["execution"]["revision"],
        read["execution"]["revision"]
    );
    assert_eq!(fs::read(&file).unwrap(), committed);
    let cleared = request_json(app.clone(), "PATCH", "/workspace/api/settings", Some(json!({"section":"project_execution", "expected_revision":read["execution"]["revision"], "value":{"project_path":"/projects/revision", "execution_profile_id":null}}))).await;
    assert_eq!(cleared.0, StatusCode::OK, "{cleared:?}");
    assert!(cleared.1["execution"]["stored"].is_null());
    assert_eq!(cleared.1["execution"]["source"], "workspace");
}

#[tokio::test]
async fn client_preferences_are_isolated_revision_checked_and_validated() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let app = build_app(settings.clone());
    let uri = "/workspace/api/settings?client_id=browser-one";
    let initial = request_json(app.clone(), "GET", uri, None).await;
    assert_eq!(initial.0, StatusCode::OK);
    assert_eq!(
        initial.1["preferences"]["effective"]["editor_sidebar_width"],
        288
    );
    let payload = json!({"section":"client_preferences", "expected_revision": initial.1["preferences"]["revision"],
        "value":{"client_id":"browser-one", "preferences":{"editor_mode":"raw", "editor_sidebar_width":400,
            "show_advanced_controls":true,"expand_child_flows":true,"graph_settings_open":true,
            "runs_scope":"all","triggers_scope":"active","home_sidebar_primary_split_ratio":0.6}}});
    let saved = request_json(
        app.clone(),
        "PATCH",
        "/workspace/api/settings",
        Some(payload.clone()),
    )
    .await;
    assert_eq!(saved.0, StatusCode::OK, "{saved:?}");
    for (field, expected) in [
        ("home_sidebar_primary_split_ratio", json!(0.6)),
        ("show_advanced_controls", json!(true)),
        ("expand_child_flows", json!(true)),
        ("graph_settings_open", json!(true)),
        ("runs_scope", json!("all")),
        ("triggers_scope", json!("active")),
    ] {
        assert_eq!(saved.1["preferences"]["stored"][field], expected);
        let restarted = request_json(build_app(settings.clone()), "GET", uri, None).await;
        assert_eq!(restarted.1["preferences"]["effective"][field], expected);
    }
    let file = settings.config_dir.join("clients/browser-one.toml");
    let original = fs::read(&file).unwrap();
    assert_eq!(
        request_json(
            app.clone(),
            "PATCH",
            "/workspace/api/settings",
            Some(payload)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    for preferences in [
        json!({"editor_sidebar_width":12}),
        json!({"home_sidebar_primary_split_ratio":-0.1}),
        json!({"home_sidebar_primary_split_ratio":1.1}),
        json!({"home_sidebar_primary_split_ratio":"0.5"}),
        json!({"editor_mode":"unsupported"}),
        json!({"selected_record":"session-only"}),
        json!({"runs_scope":"unknown"}),
        json!({"triggers_scope":"unknown"}),
        json!({"expand_child_flows":"true"}),
    ] {
        let bad = json!({"section":"client_preferences", "expected_revision":saved.1["preferences"]["revision"],
            "value":{"client_id":"browser-one", "preferences":preferences}});
        assert_eq!(
            request_json(app.clone(), "PATCH", "/workspace/api/settings", Some(bad))
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(fs::read(&file).unwrap(), original);
    let other = request_json(
        app.clone(),
        "GET",
        "/workspace/api/settings?client_id=browser-two",
        None,
    )
    .await;
    assert_eq!(
        other.1["preferences"]["effective"]["editor_mode"],
        "structured"
    );
    assert!(!settings
        .config_dir
        .join("clients/browser-two.toml")
        .exists());
    for bad_scope in [
        "client_id=..%2Fescape",
        "client_id=",
        "client_id=browser-one&project_path=%2Ftmp",
    ] {
        assert_eq!(
            request_json(
                app.clone(),
                "GET",
                &format!("/workspace/api/settings?{bad_scope}"),
                None
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    let restarted = build_app(settings);
    let read = request_json(restarted, "GET", uri, None).await;
    assert_eq!(
        read.1["preferences"]["stored"],
        saved.1["preferences"]["stored"]
    );
}

#[tokio::test]
async fn profile_resource_adapters_share_settings_revisions_and_validation() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let app = build_app(settings.clone());
    let profile = json!({"id":"team", "provider":"openai_compatible", "base_url":"http://localhost:9999/v1", "models":["model"], "api_key_env":"SPARK_ROUTE_PROFILE_MISSING"});
    let created = request_json(
        app.clone(),
        "PATCH",
        "/attractor/api/llm-profiles",
        Some(json!({"expected_revision":"absent", "value":[profile]})),
    )
    .await;
    assert_eq!(created.0, StatusCode::OK, "{}", created.1);
    let revision = &created.1["llm_profiles"]["revision"];
    let resource = request_json(app.clone(), "GET", "/attractor/api/llm-profiles", None).await;
    assert_eq!(&resource.1["revision"], revision);
    assert_eq!(resource.1["profiles"][0]["id"], "team");
    let stale = request_json(
        app.clone(),
        "PATCH",
        "/workspace/api/settings",
        Some(json!({"section":"llm_profiles", "expected_revision":"absent", "value":[]})),
    )
    .await;
    assert_eq!(stale.0, StatusCode::CONFLICT);
    let invalid = request_json(app.clone(), "PATCH", "/attractor/api/execution-placement-settings", Some(json!({"expected_revision":"absent", "value":{"profiles":[{"id":"bad", "label":"Bad", "mode":"local_container", "image":"worker:test", "metadata":{"container.mounts":["bad"]}}], "default_execution_profile_id":null}}))).await;
    assert_eq!(invalid.0, StatusCode::BAD_REQUEST, "{}", invalid.1);
    let removed = request_json(
        app.clone(),
        "PATCH",
        "/attractor/api/llm-profiles",
        Some(json!({"expected_revision":revision, "value":[]})),
    )
    .await;
    assert_eq!(removed.0, StatusCode::OK, "{}", removed.1);
    let restarted = request_json(build_app(settings), "GET", "/workspace/api/settings", None).await;
    assert_eq!(restarted.1["llm_profiles"]["stored"], json!([]));
}

#[tokio::test]
async fn referenced_synthesized_native_profile_cannot_be_removed_and_project_still_executes() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let app = build_app(settings.clone());
    let registered = request_json(
        app.clone(),
        "POST",
        "/workspace/api/projects/register",
        Some(json!({"project_path":project, "execution_profile_id":"native"})),
    )
    .await;
    assert_eq!(registered.0, StatusCode::OK, "{registered:?}");
    let path = settings.config_dir.join("execution-profiles.toml");
    assert!(!path.exists());
    let rejected = request_json(
        app.clone(),
        "PATCH",
        "/workspace/api/settings",
        Some(json!({
            "section":"execution_profiles", "expected_revision":"absent",
            "value":{"profiles":[{"id":"container", "label":"Container", "mode":"local_container",
                "enabled":true, "image":"worker:latest", "capabilities":["shell"]}],
                "default_execution_profile_id":"container"}
        })),
    )
    .await;
    assert_eq!(rejected.0, StatusCode::BAD_REQUEST, "{rejected:?}");
    assert!(
        rejected
            .1
            .to_string()
            .contains("project.toml.execution_profile_id"),
        "{rejected:?}"
    );
    assert!(rejected
        .1
        .to_string()
        .contains(registered.1["project_id"].as_str().unwrap()));
    assert!(!path.exists());
    let launched = request_json(app, "POST", "/attractor/pipelines", Some(json!({
        "run_id":"native-after-rejected-save", "working_directory":project, "wait":true,
        "flow_content":"schema_version: '1'\nid: native\nnodes:\n  start: {kind: start}\n  end: {kind: exit}\nedges:\n  - {from: start, to: end}\n"
    }))).await;
    assert_eq!(launched.0, StatusCode::OK, "{launched:?}");
    let bundle = attractor_runtime::RunStore::for_settings(&settings)
        .read_run_bundle("native-after-rejected-save")
        .unwrap()
        .unwrap();
    assert_eq!(bundle.record.unwrap().status, "completed");
    assert_eq!(
        bundle.checkpoint.unwrap().context["internal.execution_profile_snapshot"]["profile"]["id"],
        "native"
    );
    assert!(!path.exists());
}

#[tokio::test]
async fn file_edited_model_defaults_have_scoped_errors_and_workflow_rejects_before_persistence() {
    for scope in ["workspace", "project"] {
        for group in [
            "provider='nonexistent-provider'",
            "provider='openai'\nmodel='claude-sonnet-4-5'",
            "provider='codex'\nreasoning_effort='invalid'",
            "llm_profile='missing'",
            "provider='openai_compatible'",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let settings = settings(temp.path());
            let project = temp.path().join("project");
            fs::create_dir_all(&project).unwrap();
            let registry = spark_storage::ProjectRegistry::new(&settings.data_dir);
            registry
                .register_project(project.to_str().unwrap())
                .unwrap();
            let (path, section) = if scope == "workspace" {
                (settings.config_dir.join("spark.toml"), "models")
            } else {
                (
                    registry
                        .project_paths(project.to_str().unwrap())
                        .unwrap()
                        .project_file,
                    "model_settings",
                )
            };
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let original = fs::read_to_string(&path).unwrap_or_default();
            fs::write(&path, format!("{original}\n[{section}]\n{group}\n")).unwrap();
            let app = build_app(settings.clone());
            let uri = if scope == "workspace" {
                "/workspace/api/settings".to_owned()
            } else {
                format!("/workspace/api/settings?project_path={}", project.display())
            };
            let read = request_json(app.clone(), "GET", &uri, None).await;
            assert_eq!(read.0, StatusCode::OK, "{scope}: {group}: {read:?}");
            assert!(read.1["models"]["effective"].is_null());
            assert!(!read.1["models"]["validation_errors"]
                .as_array()
                .unwrap()
                .is_empty());
            let launched = request_json(app, "POST", "/attractor/pipelines", Some(json!({
                "run_id":"invalid-defaults", "working_directory":project, "wait":true,
                "flow_content":"schema_version: '1'\nid: invalid\nnodes:\n  start: {kind: start}\n  end: {kind: exit}\nedges:\n  - {from: start, to: end}\n"
            }))).await;
            assert_eq!(
                launched.1["status"], "validation_error",
                "{scope}: {group}: {launched:?}"
            );
            assert!(attractor_runtime::RunStore::for_settings(&settings)
                .read_run_bundle("invalid-defaults")
                .unwrap()
                .is_none());
        }
    }
}
