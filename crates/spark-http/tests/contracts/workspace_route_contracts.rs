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
    let app = build_app(settings.clone()).expect("app");
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
async fn claude_connection_routes_validate_codes_and_cancel() {
    let temp = tempfile::tempdir().unwrap();
    let app = build_app(settings(temp.path())).expect("app");
    for (body, status) in [
        (
            json!({"code": "a#b", "token": "unexpected"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (json!({}), StatusCode::UNPROCESSABLE_ENTITY),
        // No sign-in is waiting for a code.
        (json!({"code": "a#b"}), StatusCode::BAD_REQUEST),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/workspace/api/claude/login/code")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{body}");
    }
    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/workspace/api/claude/login")
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
}

/// Deleting a project removes its conversations from the store before its
/// directory goes; deleting it again is unknown.
#[tokio::test]
async fn project_deletion_route_removes_conversations_then_the_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    let project_dir = root.join("doomed");
    fs::create_dir_all(&project_dir).expect("project");
    let app = build_app(settings.clone()).expect("app");
    let registered = request_json(
        app.clone(),
        "POST",
        "/workspace/api/projects/register",
        Some(json!({"project_path": project_dir})),
    )
    .await;
    let project_path = registered.1["project_path"].as_str().unwrap().to_string();
    let saved = request_json(
        app.clone(),
        "PUT",
        "/workspace/api/conversations/conversation-doomed/settings",
        Some(json!({"project_path": project_path, "chat_mode": "plan", "expected_revision": "0"})),
    )
    .await;
    assert_eq!(saved.0, StatusCode::OK, "{}", saved.1);
    let project_root = spark_storage::ProjectRegistry::new(&settings.data_dir)
        .project_paths(&project_path)
        .unwrap()
        .root;
    assert!(project_root.exists());

    let remove = format!(
        "/workspace/api/projects?project_path={}",
        crate::encode_query(&project_path)
    );
    let deleted = request_json(app.clone(), "DELETE", &remove, None).await;
    assert_eq!(deleted.0, StatusCode::OK, "{}", deleted.1);
    assert_eq!(deleted.1["status"], "deleted");
    assert_eq!(deleted.1["project_path"], project_path.as_str());
    assert!(!project_root.exists());
    let listed = request_json(
        app.clone(),
        "GET",
        "/workspace/api/projects/conversations",
        None,
    )
    .await;
    assert!(
        !listed.1.to_string().contains("conversation-doomed"),
        "{}",
        listed.1
    );
    // Stale reads and state updates of the deleted project resurrect nothing.
    stale_project_reads(&app, &project_path).await;
    let favorite = request_json(
        app.clone(),
        "PATCH",
        "/workspace/api/projects/state",
        Some(json!({"project_path": project_path, "is_favorite": true})),
    )
    .await;
    assert_eq!(favorite.0, StatusCode::NOT_FOUND, "{}", favorite.1);
    assert!(!project_root.exists());
    assert_eq!(
        spark_storage::ProjectRegistry::new(&settings.data_dir)
            .read_project_record(&project_path)
            .unwrap(),
        None
    );
    let projects = request_json(app.clone(), "GET", "/workspace/api/projects", None).await;
    assert!(!projects.1.to_string().contains("doomed"), "{}", projects.1);
    let again = request_json(app, "DELETE", &remove, None).await;
    assert_eq!(again.0, StatusCode::NOT_FOUND, "{}", again.1);
}

/// Reads of a project and its conversations, as a stale browser tab sends them.
async fn stale_project_reads(app: &axum::Router, project_path: &str) {
    let project = crate::encode_query(project_path);
    for uri in [
        format!("/workspace/api/projects/conversations?project_path={project}"),
        format!("/workspace/api/conversations/conversation-doomed?project_path={project}"),
        format!("/workspace/api/conversations/conversation-unseen?project_path={project}"),
        format!("/workspace/api/roles?project_path={project}"),
        format!("/workspace/api/missions?project_path={project}"),
    ] {
        request_json(app.clone(), "GET", &uri, None).await;
    }
}

/// A store opened for one Spark home never serves settings for another;
/// the refusal comes before any startup work in either home.
#[tokio::test]
async fn app_on_a_store_refuses_settings_for_another_home() {
    let temp = tempfile::tempdir().expect("tempdir");
    let ours = settings(&temp.path().join("ours"));
    let theirs = settings(&temp.path().join("theirs"));
    let store = spark_storage::ConversationStore::open(&theirs.data_dir).expect("store");
    let backend = spark_workspace::WorkspaceConversationService::default_agent_turn_backend(&ours);
    let Err(error) =
        spark_http::build_app_with_conversation_store(ours.clone(), store.clone(), backend.clone())
    else {
        panic!("mismatched homes were accepted");
    };
    assert!(
        error.to_string().contains("belongs to Spark home"),
        "{error}"
    );
    assert!(!ours.data_dir.exists());
    let no_projects = spark_storage::ProjectRegistry::new(&theirs.data_dir)
        .list_project_records()
        .expect("projects");
    assert!(no_projects.is_empty());

    // Another spelling of the store's own home is the same home.
    let mut same = theirs.clone();
    same.data_dir = theirs.data_dir.join(".");
    assert!(spark_http::build_app_with_conversation_store(same, store, backend).is_ok());
}

/// While a deletion's cleanup is pending, stale reads neither register nor
/// recreate anything, and the finished cleanup leaves no project behind.
#[tokio::test]
async fn stale_reads_during_pending_project_cleanup_recreate_nothing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    let project_dir = root.join("pending");
    fs::create_dir_all(&project_dir).expect("project");
    let store = crate::shared_store(&settings.data_dir);
    let app = spark_http::build_app_with_conversation_store(
        settings.clone(),
        store.clone(),
        spark_workspace::WorkspaceConversationService::default_agent_turn_backend(&settings),
    )
    .expect("app on the home's store");
    let registered = request_json(
        app.clone(),
        "POST",
        "/workspace/api/projects/register",
        Some(json!({"project_path": project_dir})),
    )
    .await;
    let project_path = registered.1["project_path"].as_str().unwrap().to_string();
    let saved = request_json(
        app.clone(),
        "PUT",
        "/workspace/api/conversations/conversation-doomed/settings",
        Some(json!({"project_path": project_path, "chat_mode": "plan", "expected_revision": "0"})),
    )
    .await;
    assert_eq!(saved.0, StatusCode::OK, "{}", saved.1);
    let paths = spark_storage::ProjectRegistry::new(&settings.data_dir)
        .project_paths(&project_path)
        .unwrap();
    store
        .database()
        .fail_next_checkpoints([spark_storage::conversation::CheckpointFault::Busy]);
    let remove = format!(
        "/workspace/api/projects?project_path={}",
        crate::encode_query(&project_path)
    );
    let deferred = request_json(app.clone(), "DELETE", &remove, None).await;
    assert_eq!(
        deferred.0,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        deferred.1
    );

    stale_project_reads(&app, &project_path).await;
    assert!(!paths.conversations_dir.join("conversation-unseen").exists());
    let favorite = request_json(
        app.clone(),
        "PATCH",
        "/workspace/api/projects/state",
        Some(json!({"project_path": project_path, "is_favorite": true})),
    )
    .await;
    assert_eq!(favorite.0, StatusCode::CONFLICT, "{}", favorite.1);

    assert!(store.finish_project_deletion(&project_path).unwrap());
    assert!(!paths.root.exists());
    stale_project_reads(&app, &project_path).await;
    let favorite = request_json(
        app.clone(),
        "PATCH",
        "/workspace/api/projects/state",
        Some(json!({"project_path": project_path, "is_favorite": true})),
    )
    .await;
    assert_eq!(favorite.0, StatusCode::NOT_FOUND, "{}", favorite.1);
    assert!(!paths.root.exists());
    assert_eq!(
        spark_storage::ProjectRegistry::new(&settings.data_dir)
            .read_project_record(&project_path)
            .unwrap(),
        None
    );
    // Explicit registration still brings the project back.
    let again = request_json(
        app.clone(),
        "POST",
        "/workspace/api/projects/register",
        Some(json!({"project_path": project_dir})),
    )
    .await;
    assert_eq!(again.0, StatusCode::OK, "{}", again.1);
    assert!(paths.root.exists());
}

#[tokio::test]
async fn workspace_project_routes_persist_records_and_return_json_errors() {
    let temp = tempfile::tempdir().expect("tempdir");
    // Canonicalize: registered project paths come back canonical (macOS /var -> /private/var).
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("project");
    let app = build_app(settings.clone()).expect("app");

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
    // The registered project plus the default Home project.
    assert_eq!(list.1.as_array().expect("projects").len(), 2);

    // The registry is a database row: no project file is written.
    let project_root = settings
        .data_dir
        .join("projects")
        .join(register.1["project_id"].as_str().expect("project id"));
    assert!(!project_root.join("project.toml").exists());

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
    let app = build_app(settings).expect("app");

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
        build_app(settings(temp.path())).expect("app"),
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
    let app = build_app(settings(temp.path())).expect("app");

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
    let app = build_app(settings(temp.path())).expect("app");

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
async fn project_conversations_route_returns_summary_shape_for_existing_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_dir = temp.path().join("project-a");
    fs::create_dir_all(&project_dir).expect("project dir");
    let app = crate::store_app_default(settings.clone());

    let register = request_json(
        app.clone(),
        "POST",
        "/workspace/api/projects/register",
        Some(json!({"project_path": project_dir})),
    )
    .await;
    assert_eq!(register.0, StatusCode::OK);
    crate::seed_conversation(
        &settings.data_dir,
        &json!({
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
            crate::encode_query(register.1["project_path"].as_str().expect("project path"))
        ),
        None,
    )
    .await;

    assert_eq!(listed.0, StatusCode::OK);
    // A SummarySnapshot. A project with no started roles has no role threads.
    let entries = crate::summary_entries(&listed.1);
    assert_eq!(entries.len(), 1);
    let thread = entries
        .iter()
        .find(|entry| entry["conversation_id"] == "conversation-a")
        .expect("thread");
    assert_eq!(thread["handle"], "amber-anchor");
    assert_eq!(thread["title"], "Design thread");
    assert_eq!(thread["preview"], "Design thread preview");
    assert!(thread.get("turns").is_none());
    assert!(listed.1["watermark"].as_str().is_some());
}

#[tokio::test]
async fn registering_a_project_writes_no_definitions() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let app = build_app(settings.clone()).expect("app");
    let project = temp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let project_path = project.to_string_lossy().into_owned();

    let registered = request_json(
        app.clone(),
        "POST",
        "/workspace/api/projects/register",
        Some(json!({"project_path": project_path})),
    )
    .await;
    assert_eq!(registered.0, StatusCode::OK, "{}", registered.1);
    assert!(!project.join(".spark").exists());
}

#[tokio::test]
async fn rejected_registrations_record_nothing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let app = build_app(settings.clone()).expect("app");
    let file = temp.path().join("file");
    fs::write(&file, "").unwrap();
    let unprofiled = temp.path().join("unprofiled");
    fs::create_dir_all(&unprofiled).unwrap();

    for body in [
        json!({"project_path": temp.path().join("missing").to_string_lossy()}),
        json!({"project_path": file.to_string_lossy()}),
        json!({"project_path": unprofiled.to_string_lossy(), "execution_profile_id": "missing"}),
    ] {
        let rejected = request_json(
            app.clone(),
            "POST",
            "/workspace/api/projects/register",
            Some(body.clone()),
        )
        .await;
        assert_eq!(
            rejected.0,
            StatusCode::BAD_REQUEST,
            "{body}: {}",
            rejected.1
        );
    }
    assert!(!temp.path().join("missing").exists());
    assert!(!unprofiled.join(".spark").exists());
    let projects = request_json(app, "GET", "/workspace/api/projects", None).await;
    let listed = projects.1.as_array().expect("projects");
    assert!(
        listed
            .iter()
            .all(|project| project["display_name"] == "Home"),
        "{}",
        projects.1
    );
}

#[tokio::test]
async fn startup_registers_home_without_writing_definitions_or_config() {
    let temp = tempfile::tempdir().expect("tempdir");
    crate::isolate_home();
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
    // Spark's default SPARK_HOME is ~/.spark, where Home's definitions would go.
    let spark_home = home.join(".spark");
    let mut settings = settings(temp.path());
    settings.data_dir = spark_home.clone();
    settings.config_dir = spark_home.join("config");
    fs::create_dir_all(&settings.config_dir).unwrap();
    let config_files = [
        (
            "spark.toml",
            "[server]\n[execution_profiles.dev]\nlabel = 'Development'\nmode = 'native'\n",
        ),
        ("settings.toml", "# global settings\n"),
        ("client-settings.json", "{}\n"),
    ];
    for (name, text) in config_files {
        fs::write(settings.config_dir.join(name), text).unwrap();
    }

    let app = build_app(settings.clone()).expect("app");
    assert!(!spark_home.join("definitions").exists());
    let projects = request_json(app, "GET", "/workspace/api/projects", None).await;
    assert_eq!(
        projects.1[0]["project_path"],
        home.to_string_lossy().as_ref(),
        "{}",
        projects.1
    );
    let mut names = fs::read_dir(&settings.config_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    let mut expected = config_files.map(|(name, _)| name.to_string()).to_vec();
    expected.sort();
    assert_eq!(names, expected);
    for (name, text) in config_files {
        assert_eq!(
            fs::read_to_string(settings.config_dir.join(name)).unwrap(),
            text
        );
    }
}

#[tokio::test]
async fn home_project_is_default_and_lists_span_projects() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    let app = build_app(settings.clone()).expect("app");
    let home = spark_common::project::normalize_project_path(&std::env::var("HOME").unwrap())
        .unwrap()
        .unwrap()
        .to_string_lossy()
        .into_owned();

    let projects = request_json(app.clone(), "GET", "/workspace/api/projects", None).await;
    assert_eq!(projects.0, StatusCode::OK);
    let listed = projects.1.as_array().expect("projects");
    assert_eq!(listed.len(), 1, "{}", projects.1);
    assert_eq!(listed[0]["project_path"], home.as_str());
    assert_eq!(listed[0]["display_name"], "Home");
    assert_eq!(listed[0]["is_default"], true);
    assert_eq!(listed[0]["folder_exists"], true);
    let remove_home = format!(
        "/workspace/api/projects?project_path={}",
        crate::encode_query(&home)
    );
    let refused = request_json(app.clone(), "DELETE", &remove_home, None).await;
    assert_eq!(refused.0, StatusCode::BAD_REQUEST, "{}", refused.1);
    let projects = request_json(app.clone(), "GET", "/workspace/api/projects", None).await;
    assert_eq!(projects.1.as_array().unwrap().len(), 1);

    let mut paths = vec![];
    for name in ["project-a", "project-b"] {
        let dir = root.join(name);
        fs::create_dir_all(&dir).expect("project dir");
        let registered = request_json(
            app.clone(),
            "POST",
            "/workspace/api/projects/register",
            Some(json!({"project_path": dir})),
        )
        .await;
        assert_eq!(registered.1["is_default"], false);
        let project_path = registered.1["project_path"].as_str().unwrap().to_string();
        let conversation = format!("/workspace/api/conversations/conversation-{name}");
        // A conversation without rows is not found; its revision is 0.
        let snapshot = request_json(
            app.clone(),
            "GET",
            &format!(
                "{conversation}?project_path={}",
                crate::encode_query(&project_path)
            ),
            None,
        )
        .await;
        assert_eq!(snapshot.0, StatusCode::NOT_FOUND);
        let saved = request_json(
            app.clone(),
            "PUT",
            &format!("{conversation}/settings"),
            Some(json!({"project_path": project_path, "chat_mode": "plan", "expected_revision": "0"})),
        )
        .await;
        assert_eq!(saved.0, StatusCode::OK, "{}", saved.1);
        let mission = request_json(
            app.clone(),
            "POST",
            &format!(
                "/workspace/api/missions?project_path={}",
                crate::encode_query(&project_path)
            ),
            Some(json!({"fields": {"title": name}})),
        )
        .await;
        assert_eq!(mission.0, StatusCode::OK, "{}", mission.1);
        assert!(mission.1.get("source_conversation_id").is_none());
        paths.push(project_path);
    }
    // Thread lists are SummarySnapshots; missions are a plain list.
    let project_paths = |items: &Value| {
        let entries = if items.is_array() {
            items.as_array().unwrap().clone()
        } else {
            crate::summary_entries(items)
        };
        let mut paths: Vec<String> = entries
            .iter()
            .filter(|item| item.get("standing_role").is_none())
            .map(|item| item["project_path"].as_str().unwrap().to_string())
            .collect();
        paths.sort();
        paths
    };

    let all = request_json(
        app.clone(),
        "GET",
        "/workspace/api/projects/conversations",
        None,
    )
    .await;
    assert_eq!(all.0, StatusCode::OK, "{}", all.1);
    assert_eq!(project_paths(&all.1), paths);
    let scoped = format!(
        "/workspace/api/projects/conversations?project_path={}",
        crate::encode_query(&paths[0])
    );
    let chats = request_json(app.clone(), "GET", &scoped, None).await;
    assert_eq!(project_paths(&chats.1), [paths[0].clone()]);

    let all = request_json(app.clone(), "GET", "/workspace/api/missions", None).await;
    assert_eq!(all.0, StatusCode::OK, "{}", all.1);
    assert_eq!(project_paths(&all.1["missions"]), paths);
    let scoped = format!(
        "/workspace/api/missions?project_path={}",
        crate::encode_query(&paths[1])
    );
    let one = request_json(app.clone(), "GET", &scoped, None).await;
    assert_eq!(project_paths(&one.1["missions"]), [paths[1].clone()]);

    assert_eq!(chats.1["project_path"], paths[0].as_str());
    let handle = crate::summary_entries(&chats.1)
        .iter()
        .find(|entry| entry.get("standing_role").is_none())
        .expect("chat")["handle"]
        .as_str()
        .unwrap()
        .to_string();
    let from_chat = request_json(
        app.clone(),
        "POST",
        &format!("/workspace/api/missions?conversation_handle={handle}"),
        Some(json!({"fields": {"title": "From chat"}})),
    )
    .await;
    assert_eq!(from_chat.0, StatusCode::OK, "{}", from_chat.1);
    assert_eq!(from_chat.1["project_path"], paths[0].as_str());
    assert_eq!(
        from_chat.1["source_conversation_id"],
        "conversation-project-a"
    );
    let conflicting = request_json(
        app.clone(),
        "POST",
        &format!(
            "/workspace/api/missions?conversation_handle={handle}&project_path={}",
            crate::encode_query(&paths[1])
        ),
        Some(json!({"fields": {"title": "Wrong project"}})),
    )
    .await;
    assert_eq!(conflicting.0, StatusCode::BAD_REQUEST, "{}", conflicting.1);

    fs::remove_dir_all(root.join("project-b")).expect("remove project folder");
    let projects = request_json(app, "GET", "/workspace/api/projects", None).await;
    let folder_exists = |path: &str| {
        projects
            .1
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["project_path"] == path)
            .unwrap()["folder_exists"]
            .clone()
    };
    assert_eq!(folder_exists(&paths[0]), true);
    assert_eq!(folder_exists(&paths[1]), false);
}

#[tokio::test]
async fn a_chat_without_a_project_belongs_to_home_and_so_do_its_missions_and_runs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    let flow_name = crate::install_flow(
        &settings,
        json!({
            "schema_version": "2",
            "id": "review",
            "title": "Review",
            "nodes": {
                "start": {
                    "kind": "start",
                    "config": {"kind": "start"},
                    "routes": {
                        "next": {"to": "done", "description": "Enter the first workflow step."}
                    }
                },
                "done": {
                    "kind": "exit",
                    "config": {
                        "kind": "exit",
                        "result": {"name": "completed", "outcome": "success"}
                    }
                }
            }
        }),
    );
    let project = root.join("bound-project");
    fs::create_dir_all(&project).unwrap();
    let project = project.to_string_lossy().into_owned();
    let agent = std::sync::Arc::new(GatedAgent::default());
    agent.release();
    let app = spark_http::build_app_with_agent_turn_backend(settings, agent).expect("app");
    let home = spark_common::project::normalize_project_path(&std::env::var("HOME").unwrap())
        .unwrap()
        .unwrap()
        .to_string_lossy()
        .into_owned();

    // New chats with no project, through either write that creates a chat.
    let settings_chat = request_json(
        app.clone(),
        "PUT",
        "/workspace/api/conversations/home-settings-chat/settings",
        Some(json!({"chat_mode": "plan", "expected_revision": "0"})),
    )
    .await;
    assert_eq!(settings_chat.0, StatusCode::OK, "{}", settings_chat.1);
    assert_eq!(
        crate::conversation_payload(&settings_chat.1["changes"])["project_path"],
        home.as_str()
    );
    let turn_chat = request_json(
        app.clone(),
        "POST",
        "/workspace/api/conversations/home-turn-chat/turns",
        Some(json!({"message": "Hello"})),
    )
    .await;
    assert_eq!(turn_chat.0, StatusCode::OK, "{}", turn_chat.1);
    let turn_chat_row = crate::conversation_payload(&turn_chat.1["changes"]);
    assert_eq!(turn_chat_row["project_path"], home.as_str());

    // An explicit project still binds, and later writes keep that binding.
    let bound = request_json(
        app.clone(),
        "PUT",
        "/workspace/api/conversations/bound-chat/settings",
        Some(json!({"project_path": project})),
    )
    .await;
    assert_eq!(
        crate::conversation_payload(&bound.1["changes"])["project_path"],
        project.as_str(),
        "{}",
        bound.1
    );
    let rebound = request_json(
        app.clone(),
        "PUT",
        "/workspace/api/conversations/bound-chat/settings",
        Some(json!({
            "chat_mode": "plan",
            "expected_revision": crate::conversation_payload(&bound.1["changes"])["metadata"]["settings_revision"].to_string(),
        })),
    )
    .await;
    assert_eq!(rebound.0, StatusCode::OK, "{}", rebound.1);
    assert_eq!(
        crate::conversation_payload(&rebound.1["changes"])["project_path"],
        project.as_str()
    );

    let handle = turn_chat_row["handle"].as_str().unwrap();
    let mission = request_json(
        app.clone(),
        "POST",
        &format!("/workspace/api/missions?conversation_handle={handle}"),
        Some(json!({"fields": {"title": "From a Home chat"}})),
    )
    .await;
    assert_eq!(mission.0, StatusCode::OK, "{}", mission.1);
    assert_eq!(mission.1["project_path"], home.as_str());
    let run = request_json(
        app.clone(),
        "POST",
        &format!("/workspace/api/conversations/by-handle/{handle}/flow-run-requests"),
        Some(json!({"flow_name": flow_name, "summary": "Run from Home."})),
    )
    .await;
    assert_eq!(run.0, StatusCode::OK, "{}", run.1);
    let runs = request_json(
        app,
        "GET",
        &format!(
            "/attractor/runs?project_path={}",
            crate::encode_query(&home)
        ),
        None,
    )
    .await;
    assert!(
        runs.1["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|listed| listed["run_id"] == run.1["run_id"]),
        "{}",
        runs.1
    );
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

fn settings(root: &Path) -> SparkSettings {
    crate::isolate_home();
    SparkSettings {
        startup_sources: Default::default(),
        connections: Default::default(),
        providers: Default::default(),
        agents: Default::default(),
        project_root: root.join("source"),
        data_dir: root.join("spark-home"),
        config_dir: root.join("spark-home/config"),
        runtime_dir: root.join("spark-home/runtime"),
        runs_dir: None,
        ui_dir: None,
        project_roots: Vec::new(),
    }
}

/// A mission agent scripted over the real HTTP routes: its first turn launches
/// two flows, and once both runs have reported back it closes the mission. It
/// reads its handle, mission, and project from the pinned frame, as an agent
/// would.
struct ScriptedMissionAgent {
    app: std::sync::OnceLock<axum::Router>,
    runtime: tokio::runtime::Handle,
    prompts: std::sync::Mutex<Vec<String>>,
}
impl ScriptedMissionAgent {
    fn call(&self, method: &str, uri: &str, body: Value) -> (StatusCode, Value, String) {
        let app = self.app.get().expect("app").clone();
        self.runtime
            .block_on(request_json(app, method, uri, Some(body)))
    }
}
impl spark_agent_adapter::AgentTurnBackend for ScriptedMissionAgent {
    fn run_turn(
        &self,
        request: spark_agent_adapter::AgentTurnRequest,
    ) -> Result<spark_agent_adapter::AgentTurnOutput, spark_agent_adapter::AgentError> {
        let frame = request.metadata[spark_agent_adapter::AGENT_INSTRUCTIONS_METADATA_KEY]
            .as_str()
            .expect("pinned frame")
            .to_string();
        let field = |label: &str| {
            frame
                .lines()
                .find_map(|line| line.strip_prefix(label))
                .expect(label)
                .to_string()
        };
        let (handle, mission, project) = (
            field("Conversation handle: "),
            field("Mission ID: "),
            field("Project path: "),
        );
        let mut prompts = self.prompts.lock().unwrap();
        prompts.push(request.prompt.clone());
        let reported = prompts
            .iter()
            .map(|prompt| prompt.matches("Run run-").count())
            .sum::<usize>();
        drop(prompts);
        let reply = if request.prompt == "Begin work on this mission." {
            for (flow, summary) in [("tests/ok", "Build it"), ("tests/ok", "Test it")] {
                let launched = self.call(
                    "POST",
                    &format!("/workspace/api/conversations/by-handle/{handle}/flow-run-requests"),
                    json!({"flow_name": flow, "summary": summary}),
                );
                assert_eq!(launched.0, StatusCode::OK, "{}", launched.1);
            }
            "Launched both runs."
        } else if reported == 2 {
            let closed = self.call(
                "POST",
                &format!(
                    "/workspace/api/missions/{mission}/close?project_path={}",
                    crate::encode_query(&project)
                ),
                json!({"status": "done", "reason": "Both runs completed", "actor": "assistant"}),
            );
            assert_eq!(closed.0, StatusCode::OK, "{}", closed.1);
            "Closed the mission."
        } else {
            "Waiting for the other run."
        };
        Ok(final_answer(reply))
    }
}

#[derive(Default)]
struct GatedAgent {
    released: std::sync::Mutex<bool>,
    opened: std::sync::Condvar,
}
impl GatedAgent {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.opened.notify_all();
    }
}
impl spark_agent_adapter::AgentTurnBackend for GatedAgent {
    fn run_turn(
        &self,
        _request: spark_agent_adapter::AgentTurnRequest,
    ) -> Result<spark_agent_adapter::AgentTurnOutput, spark_agent_adapter::AgentError> {
        let (released, _) = self
            .opened
            .wait_timeout_while(
                self.released.lock().unwrap(),
                std::time::Duration::from_secs(60),
                |released| !*released,
            )
            .unwrap();
        assert!(*released, "mission agent gate was never released");
        Ok(final_answer("Understood."))
    }
}

async fn wait_for_mission(app: &axum::Router, uri: &str, status: &str) -> Value {
    for _ in 0..600 {
        let mission = request_json(app.clone(), "GET", uri, None).await.1;
        if mission["status"] == status {
            return mission;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("mission never reached {status}");
}

#[tokio::test(flavor = "multi_thread")]
async fn role_routes_list_start_and_stop_roles_that_take_messages_by_handle() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let roles = settings.bundled_dir().join("roles");
    fs::create_dir_all(&roles).unwrap();
    for (name, text) in [
        (
            "coordinator",
            include_str!("../../../spark-assets/assets/roles/coordinator.md"),
        ),
        (
            "chief-of-staff",
            include_str!("../../../spark-assets/assets/roles/chief-of-staff.md"),
        ),
    ] {
        fs::write(roles.join(format!("{name}.md")), text).unwrap();
    }
    let project = temp.path().join("roles-project");
    fs::create_dir_all(&project).unwrap();
    let agent = std::sync::Arc::new(GatedAgent::default());
    agent.release();
    let app = spark_http::build_app_with_agent_turn_backend(settings, agent).expect("app");
    let project = project.to_string_lossy().to_string();
    let at = |suffix: &str| {
        format!(
            "/workspace/api/roles{suffix}?project_path={}",
            crate::encode_query(&project)
        )
    };
    // No role exists until it is started; listing starts none.
    let listed = request_json(app.clone(), "GET", &at(""), None).await;
    assert_eq!(listed.0, StatusCode::OK, "{}", listed.1);
    assert_eq!(listed.1["roles"], json!([]));
    let definitions = format!(
        "/workspace/api/definitions?project_path={}",
        crate::encode_query(&project)
    );
    let live = |list: &Value, name: &str| {
        list["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["kind"] == "role" && item["name"] == name)
            .unwrap_or_else(|| panic!("{name} in {list}"))["live"]
            .clone()
    };
    let browsed = request_json(app.clone(), "GET", &definitions, None).await;
    assert_eq!(browsed.0, StatusCode::OK, "{}", browsed.1);
    assert_eq!(live(&browsed.1, "coordinator"), Value::Null);
    assert_eq!(live(&browsed.1, "chief-of-staff"), Value::Null);
    let unknown = request_json(app.clone(), "POST", &at("/coordinator/pause"), None).await;
    assert_eq!(unknown.0, StatusCode::NOT_FOUND);
    let missing = request_json(app.clone(), "POST", &at("/no-such-role/start"), None).await;
    assert_eq!(missing.0, StatusCode::BAD_REQUEST, "{}", missing.1);

    let started = request_json(app.clone(), "POST", &at("/chief-of-staff/start"), None).await;
    assert_eq!(started.0, StatusCode::OK, "{}", started.1);
    assert_eq!(started.1["title"], "Chief of Staff");
    let coordinator = request_json(app.clone(), "POST", &at("/coordinator/start"), None).await;
    assert_eq!(coordinator.0, StatusCode::OK, "{}", coordinator.1);
    let listed = request_json(app.clone(), "GET", &at(""), None).await;
    let names: Vec<_> = listed.1["roles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|role| role["name"].clone())
        .collect();
    assert_eq!(names, [json!("chief-of-staff"), json!("coordinator")]);
    // The Roles tab's list shows the started role and its conversation.
    let browsed = request_json(app.clone(), "GET", &definitions, None).await;
    let chief = live(&browsed.1, "chief-of-staff");
    assert_eq!(chief["conversation_id"], started.1["conversation_id"]);
    assert!(chief["conversation_handle"].as_str().is_some(), "{chief}");
    assert!(chief["recent_events"].is_array(), "{chief}");
    let threads = request_json(
        app.clone(),
        "GET",
        &format!(
            "/workspace/api/projects/conversations?project_path={}",
            crate::encode_query(&project)
        ),
        None,
    )
    .await;
    assert!(crate::summary_entries(&threads.1)
        .iter()
        .any(
            |entry| entry["conversation_id"] == started.1["conversation_id"]
                && entry["standing_role"]["name"] == "chief-of-staff"
        ));
    let conversation = crate::conversation_view(
        app.clone(),
        started.1["conversation_id"].as_str().unwrap(),
        &project,
    )
    .await;
    let handle = conversation["conversation_handle"].as_str().unwrap();
    assert!(!handle.is_empty());
    // Roles take messages through their conversation, not a role control.
    let removed = request_json(
        app.clone(),
        "POST",
        &at("/chief-of-staff/send"),
        Some(json!({"conversation_handle": handle, "message": "Fixed it"})),
    )
    .await;
    assert_eq!(removed.0, StatusCode::NOT_FOUND);
    let messages = format!("/workspace/api/conversations/by-handle/{handle}/messages");
    let message = json!({"conversation_handle": handle, "message": "Fixed it"});
    for _ in 0..2 {
        let sent = request_json(app.clone(), "POST", &messages, Some(message.clone())).await;
        assert_eq!(sent.0, StatusCode::ACCEPTED, "{}", sent.1);
    }
    let listed = request_json(app.clone(), "GET", &at(""), None).await;
    assert_eq!(listed.1["roles"][0]["event_seq"], 2, "{}", listed.1);
    for bad in [
        json!({"message": "No sender"}),
        json!({"conversation_handle": handle, "message": "  "}),
        json!({"conversation_handle": handle, "message": 7}),
        json!({"conversation_handle": handle, "message": "Hi", "id": "retry-1"}),
    ] {
        let rejected = request_json(app.clone(), "POST", &messages, Some(bad.clone())).await;
        assert_eq!(rejected.0, StatusCode::BAD_REQUEST, "{bad}: {}", rejected.1);
    }
    let unknown = request_json(
        app.clone(),
        "POST",
        "/workspace/api/conversations/by-handle/no-such-handle/messages",
        Some(message),
    )
    .await;
    assert_eq!(unknown.0, StatusCode::NOT_FOUND);
    let stopped = request_json(app.clone(), "POST", &at("/chief-of-staff/stop"), None).await;
    assert_eq!(stopped.0, StatusCode::OK, "{}", stopped.1);
    assert_eq!(stopped.1["state"], "stopped");
}

/// Answers every turn at once and records each prompt it was given.
#[derive(Default)]
struct RecordingAgent {
    prompts: std::sync::Mutex<Vec<String>>,
}
impl spark_agent_adapter::AgentTurnBackend for RecordingAgent {
    fn run_turn(
        &self,
        request: spark_agent_adapter::AgentTurnRequest,
    ) -> Result<spark_agent_adapter::AgentTurnOutput, spark_agent_adapter::AgentError> {
        self.prompts.lock().unwrap().push(request.prompt);
        Ok(final_answer("Noted."))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_role_holds_user_messages_until_it_is_started_again() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let roles = settings.bundled_dir().join("roles");
    fs::create_dir_all(&roles).unwrap();
    fs::write(
        roles.join("coordinator.md"),
        include_str!("../../../spark-assets/assets/roles/coordinator.md"),
    )
    .unwrap();
    let project = temp.path().join("held-project");
    fs::create_dir_all(&project).unwrap();
    let project = project.to_string_lossy().to_string();
    let agent = std::sync::Arc::new(RecordingAgent::default());
    let app = spark_http::build_app_with_agent_turn_backend(settings, agent.clone()).expect("app");
    let control = |action: &str| {
        format!(
            "/workspace/api/roles/coordinator/{action}?project_path={}",
            crate::encode_query(&project)
        )
    };
    let prompts = || agent.prompts.lock().unwrap().clone();
    let settled = |conversation: Value| {
        conversation["turns"]
            .as_array()
            .unwrap()
            .iter()
            .all(|turn| turn["status"] == "complete")
    };
    let wait_settled = |id: String| {
        let (app, project) = (app.clone(), project.clone());
        async move {
            for _ in 0..600 {
                let conversation = crate::conversation_view(app.clone(), &id, &project).await;
                if settled(conversation.clone()) {
                    return conversation;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            panic!("conversation {id} never settled");
        }
    };
    let send = |id: &str, message: &str| {
        let (app, uri) = (
            app.clone(),
            format!("/workspace/api/conversations/{id}/turns"),
        );
        let body = json!({"project_path": project, "message": message});
        async move { request_json(app, "POST", &uri, Some(body)).await }
    };

    let started = request_json(app.clone(), "POST", &control("start"), None).await;
    assert_eq!(started.0, StatusCode::OK, "{}", started.1);
    let role_chat = started.1["conversation_id"].as_str().unwrap().to_string();
    wait_settled(role_chat.clone()).await;
    assert_eq!(prompts().len(), 1);
    let stopped = request_json(app.clone(), "POST", &control("stop"), None).await;
    assert_eq!(stopped.1["state"], "stopped");

    // Held: recorded as queued, no assistant turn, nothing reaches the backend.
    for message in ["First held", "Second held"] {
        let (status, acceptance, _) = send(&role_chat, message).await;
        assert_eq!(status, StatusCode::OK, "{acceptance}");
        assert_eq!(acceptance["status"], "queued");
        let snapshot = crate::conversation_view(app.clone(), &role_chat, &project).await;
        let turns = snapshot["turns"].as_array().unwrap();
        let last = turns.last().unwrap();
        assert_eq!(
            (last["content"].as_str(), last["status"].as_str()),
            (Some(message), Some("queued"))
        );
        assert!(turns.iter().all(|turn| turn["status"] != "pending"));
    }
    // An ordinary chat still runs while the role is stopped.
    send("ordinary-chat", "Hello").await;
    let ordinary = wait_settled("ordinary-chat".into()).await;
    assert_eq!(ordinary["turns"].as_array().unwrap().len(), 2);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(prompts()[1..], ["Hello".to_string()]);

    // Started again, the held messages run in order, each exactly once.
    let resumed = request_json(app.clone(), "POST", &control("start"), None).await;
    assert_eq!(resumed.0, StatusCode::OK, "{}", resumed.1);
    let conversation = wait_settled(role_chat.clone()).await;
    assert_eq!(conversation["turns"].as_array().unwrap().len(), 6);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(
        prompts()[2..],
        ["First held".to_string(), "Second held".to_string()]
    );
    // And a message to the running role starts a turn again.
    send(&role_chat, "After").await;
    wait_settled(role_chat).await;
    assert_eq!(prompts().last().unwrap(), "After");
    assert_eq!(prompts().len(), 5);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mission_agent_launches_runs_hears_their_events_and_closes_the_mission() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project = temp.path().join("missions-project");
    fs::create_dir_all(&project).unwrap();
    // The scripted agent launches `tests/ok`.
    assert_eq!(
        crate::install_flow(
            &settings,
            json!({
                "schema_version": "2",
                "id": "ok",
                "nodes": {
                    "start": {
                        "kind": "start",
                        "config": {"kind": "start"},
                        "routes": {
                            "next": {
                                "to": "done",
                                "description": "Enter the first workflow step."
                            }
                        }
                    },
                    "done": {
                        "kind": "exit",
                        "config": {
                            "kind": "exit",
                            "result": {"name": "completed", "outcome": "success"}
                        }
                    }
                }
            })
        ),
        "tests/ok"
    );
    let agent = std::sync::Arc::new(ScriptedMissionAgent {
        app: std::sync::OnceLock::new(),
        runtime: tokio::runtime::Handle::current(),
        prompts: Default::default(),
    });
    let app = spark_http::build_app_with_agent_turn_backend(settings, agent.clone()).expect("app");
    agent.app.set(app.clone()).ok().unwrap();
    let project = project.display().to_string();
    let created = request_json(
        app.clone(),
        "POST",
        &format!(
            "/workspace/api/missions?project_path={}",
            crate::encode_query(&project)
        ),
        Some(json!({"fields": {"title": "Ship", "description": "Build and test the change."}})),
    )
    .await;
    let id = created.1["id"].as_str().unwrap().to_string();
    let at = format!(
        "/workspace/api/missions/{id}?project_path={}",
        crate::encode_query(&project)
    );
    let started = request_json(
        app.clone(),
        "POST",
        &format!(
            "/workspace/api/missions/{id}/start?project_path={}",
            crate::encode_query(&project)
        ),
        None,
    )
    .await;
    assert_eq!(started.0, StatusCode::OK, "{}", started.1);
    let closed = wait_for_mission(&app, &at, "closed").await;
    assert_eq!(closed["closed"]["actor"], "assistant");
    assert_eq!(closed["closed"]["reason"], "Both runs completed");
    let runs = closed["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    assert!(runs.iter().all(|run| run["status"] == "completed"));

    // The agent closed the mission inside its turn; let that turn finish.
    let mut transcript = Value::Null;
    for _ in 0..600 {
        transcript = crate::conversation_view(app.clone(), &id, &project).await;
        if !transcript["turns"]
            .as_array()
            .unwrap()
            .iter()
            .any(|turn| turn["status"] == "pending" || turn["status"] == "streaming")
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let launches: Vec<_> = transcript["flow_launches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|launch| launch["run_id"].clone())
        .collect();
    assert_eq!(
        launches,
        runs.iter()
            .map(|run| run["run_id"].clone())
            .collect::<Vec<_>>()
    );
    let turns = transcript["turns"].as_array().unwrap();
    let user_text: String = turns
        .iter()
        .filter(|turn| turn["role"] == "user")
        .map(|turn| turn["content"].as_str().unwrap())
        .collect();
    for run in runs {
        let run_id = run["run_id"].as_str().unwrap();
        assert_eq!(
            user_text.matches(&format!("Run {run_id} ")).count(),
            1,
            "{user_text}"
        );
    }
    assert!(turns
        .iter()
        .filter(|turn| turn["role"] == "assistant")
        .all(|turn| turn["status"] == "complete"));
    assert!(turns.iter().any(|turn| turn["kind"] == "mission_notice"
        && turn["content"] == "Closed as done: Both runs completed"));
    // Mission conversations stay out of the Threads list.
    let threads = request_json(
        app,
        "GET",
        &format!(
            "/workspace/api/projects/conversations?project_path={}",
            crate::encode_query(&project)
        ),
        None,
    )
    .await;
    assert_eq!(threads.0, StatusCode::OK);
    assert!(!threads.1.to_string().contains(&id), "{}", threads.1);
}

#[tokio::test]
async fn mission_routes_enforce_records_revisions_messages_and_controls() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project = temp.path().join("missions-project");
    fs::create_dir_all(&project).unwrap();
    let agent = std::sync::Arc::new(GatedAgent::default());
    let app = spark_http::build_app_with_agent_turn_backend(settings, agent.clone()).expect("app");
    let list_uri = format!(
        "/workspace/api/missions?project_path={}",
        crate::encode_query(project.to_string_lossy())
    );
    let refused = request_json(
        app.clone(),
        "POST",
        &list_uri,
        Some(json!({"fields":{"title":"Hooked","hooks":[]}})),
    )
    .await;
    assert_eq!(refused.0, StatusCode::BAD_REQUEST, "{}", refused.1);
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
    assert_eq!(created.1["status"], "draft");
    assert_eq!(created.1["activity"][0]["actor"], "assistant");
    let id = created.1["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("mission-"));
    let at = |suffix: &str| {
        format!(
            "/workspace/api/missions/{id}{suffix}?project_path={}",
            crate::encode_query(project.to_string_lossy())
        )
    };
    let updated = request_json(
        app.clone(),
        "PATCH",
        &at(""),
        Some(json!({"revision":1,"fields":{"budget":{"total_runs":3}}})),
    )
    .await;
    assert_eq!(updated.0, StatusCode::OK);
    assert_eq!(updated.1["fields"]["budget"]["total_runs"], 3);
    let stale = request_json(
        app.clone(),
        "PATCH",
        &at(""),
        Some(json!({"revision":1,"fields":{"title":"Stale"}})),
    )
    .await;
    assert_eq!(stale.0, StatusCode::CONFLICT);
    for key in ["stage", "state", "runs", "cursor", "hooks", "reaction_flow"] {
        let rejected = request_json(
            app.clone(),
            "PATCH",
            &at(""),
            Some(json!({"revision":2,"fields":{key:null}})),
        )
        .await;
        assert_eq!(rejected.0, StatusCode::BAD_REQUEST, "{key}: {}", rejected.1);
    }
    let listed = request_json(app.clone(), "GET", &list_uri, None).await;
    assert_eq!(listed.1, json!({"missions":[updated.1]}));
    let fetched = request_json(app.clone(), "GET", &at(""), None).await;
    assert_eq!(fetched.1, updated.1);

    let message = json!({"id":"note-1","kind":"human.message","payload":{"message":"hello"}});
    let sent = request_json(app.clone(), "POST", &at("/events"), Some(message.clone())).await;
    assert_eq!(sent.0, StatusCode::OK, "{}", sent.1);
    assert_eq!(sent.1["event_seq"], 1);
    let again = request_json(app.clone(), "POST", &at("/events"), Some(message)).await;
    assert_eq!(again.1["event_seq"], 1);
    for bad in [
        json!({"payload":{}}),
        json!({"kind":"run.completed","payload":{}}),
    ] {
        let bad = request_json(app.clone(), "POST", &at("/events"), Some(bad)).await;
        assert_eq!(bad.0, StatusCode::BAD_REQUEST);
    }
    // The transcript replaced the inbox read route.
    let read = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(at("/events"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(read.status(), StatusCode::METHOD_NOT_ALLOWED);

    let started = request_json(app.clone(), "POST", &at("/start"), None).await;
    assert_eq!(started.0, StatusCode::OK, "{}", started.1);
    assert_eq!(started.1["conversation_id"], json!(id));
    assert_eq!(started.1["status"], "running");
    agent.release();
    let idle = wait_for_mission(&app, &at(""), "needs_you").await;
    assert_eq!(idle["status"], "needs_you");
    assert_eq!(
        request_json(app.clone(), "POST", &at("/start"), None)
            .await
            .0,
        StatusCode::CONFLICT
    );
    for removed in ["/pause", "/resume", "/explode"] {
        assert_eq!(
            request_json(app.clone(), "POST", &at(removed), None)
                .await
                .0,
            StatusCode::NOT_FOUND,
            "{removed}"
        );
    }
    let closed = request_json(
        app.clone(),
        "POST",
        &at("/close"),
        Some(json!({"status":"done","reason":"Shipped"})),
    )
    .await;
    assert_eq!(closed.1["closed"]["status"], "done");
    assert_eq!(closed.1["closed"]["actor"], "human");
    assert_eq!(closed.1["status"], "closed");
    let canceled = request_json(app.clone(), "POST", &at("/cancel"), None).await;
    assert_eq!(canceled.1["closed"]["status"], "done");
    assert_eq!(
        request_json(
            app.clone(),
            "GET",
            "/workspace/api/tasks?project_path=/tmp",
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let other = temp.path().join("other");
    fs::create_dir(&other).unwrap();
    let other_uri = format!(
        "/workspace/api/missions/{id}?project_path={}",
        crate::encode_query(other.to_string_lossy())
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
    let app = build_app(config.clone()).expect("app");
    let first = request_json(app.clone(), "GET", "/workspace/api/settings", None).await;
    let revision = first.1["runtime"]["revision"].clone();
    let payload = json!({"expected_revision": revision, "section": "runtime", "value": {"runs_dir": "/new/runs", "project_roots": []}});
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
    assert_eq!(saved.1["runtime"]["stored"]["runs_dir"], "/new/runs");
    assert_eq!(saved.1["runtime"]["effective"]["runs_dir"], json!(null));
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
    let app = build_app(config.clone()).expect("app");
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
    let invalid = request_json(app, "PATCH", "/workspace/api/settings", Some(json!({"expected_revision": saved.1["models"]["revision"], "section": "models", "value": {"provider": "openai", "llm_profile": "SECRET_DO_NOT_EXPOSE"}}))).await;
    assert_eq!(invalid.0, StatusCode::BAD_REQUEST);
    assert!(!invalid.1.to_string().contains("SECRET_DO_NOT_EXPOSE"));
    assert_eq!(
        fs::read(config.config_dir.join("spark.toml")).unwrap(),
        bytes
    );
}

#[tokio::test]
async fn utility_model_settings_http_round_trip_and_clear() {
    let temp = tempfile::tempdir().unwrap();
    let app = build_app(settings(temp.path())).expect("app");
    let read = || request_json(app.clone(), "GET", "/workspace/api/settings", None);
    let initial = read().await.1;
    assert!(initial["utility_models"]["stored"].is_null());
    let patch = |revision: &Value, value: Value| {
        request_json(
            app.clone(),
            "PATCH",
            "/workspace/api/settings",
            Some(
                json!({"expected_revision": revision, "section": "utility_models", "value": value}),
            ),
        )
    };
    let invalid = patch(
        &initial["utility_models"]["revision"],
        json!({"provider": "codex", "thinking": "sometimes"}),
    )
    .await;
    assert_eq!(invalid.0, StatusCode::BAD_REQUEST, "{}", invalid.1);
    let saved = patch(
        &initial["utility_models"]["revision"],
        json!({"provider": "claude-code", "model": "haiku"}),
    )
    .await;
    assert_eq!(saved.0, StatusCode::OK, "{}", saved.1);
    assert_eq!(saved.1["utility_models"]["stored"]["model"], "haiku");
    assert_eq!(
        read().await.1["utility_models"]["effective"]["provider"],
        "claude-code"
    );
    assert_eq!(read().await.1["models"], saved.1["models"]);
    let cleared = patch(&saved.1["utility_models"]["revision"], Value::Null).await;
    assert_eq!(cleared.0, StatusCode::OK, "{}", cleared.1);
    assert!(read().await.1["utility_models"]["stored"].is_null());
}

#[tokio::test]
async fn conversation_resource_requires_revisions_for_every_model_selector() {
    let temp = tempfile::tempdir().unwrap();
    let config = settings(temp.path());
    let app = build_app(config.clone()).expect("app");
    let uri = "/workspace/api/conversations/revision-chat/settings";
    let read_uri =
        "/workspace/api/settings?conversation_id=revision-chat&project_path=/projects/revisions";
    let initial = request_json(app.clone(), "GET", read_uri, None).await;
    assert_eq!(initial.0, StatusCode::OK);
    let initial_revision = initial.1["models"]["revision"].clone();
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
    // Rejected updates leave the stored conversation unchanged.
    let stored = || request_json(app.clone(), "GET", read_uri, None);
    let before = stored().await.1;
    for mut payload in selectors {
        payload["project_path"] = json!("/projects/revisions");
        payload["expected_revision"] = initial_revision.clone();
        let response = request_json(app.clone(), "PUT", uri, Some(payload)).await;
        assert_eq!(response.0, StatusCode::CONFLICT, "{:?}", response.1);
        assert_eq!(stored().await.1, before);
    }
    let read = request_json(app, "GET", read_uri, None).await;
    assert_eq!(
        read.1,
        crate::conversation_payload(&saved.1["changes"])["settings"]
    );
}

#[tokio::test]
async fn scoped_conversation_settings_get_set_and_inherit_use_common_revisions() {
    let temp = tempfile::tempdir().unwrap();
    let app = build_app(settings(temp.path())).expect("app");
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
        settings.config_dir.join("spark.toml"),
        "[execution_profiles.dev]\nlabel = 'Development'\nmode = 'native'\n",
    )
    .unwrap();
    let project = temp.path().join("revision");
    fs::create_dir_all(&project).unwrap();
    let project = project.to_string_lossy().into_owned();
    let app = build_app(settings.clone()).expect("app");
    let registered = request_json(
        app.clone(),
        "POST",
        "/workspace/api/projects/register",
        Some(json!({"project_path":project})),
    )
    .await;
    assert_eq!(registered.0, StatusCode::OK);
    let uri = &format!(
        "/workspace/api/settings?project_path={}",
        project.replace('/', "%2F")
    );
    let read = request_json(app.clone(), "GET", uri, None).await.1;
    let revision = read["execution"]["revision"].clone();
    let registry = spark_storage::ProjectRegistry::new(&settings.data_dir);
    let original = registry.project_settings(&project).unwrap();
    for value in [json!("dev"), Value::Null] {
        let rejected = request_json(
            app.clone(),
            "PATCH",
            "/workspace/api/projects/state",
            Some(json!({"project_path":project, "execution_profile_id":value})),
        )
        .await;
        assert_eq!(rejected.0, StatusCode::BAD_REQUEST, "{rejected:?}");
        assert_eq!(registry.project_settings(&project).unwrap(), original);
    }
    let payload =
        json!({"project_path":project, "execution_profile_id":"dev", "expected_revision":revision});
    let saved = request_json(
        app.clone(),
        "PATCH",
        "/workspace/api/projects/state",
        Some(payload.clone()),
    )
    .await;
    assert_eq!(saved.0, StatusCode::OK, "{saved:?}");
    let committed = registry.project_settings(&project).unwrap();
    let conflict = request_json(
        app.clone(),
        "PATCH",
        "/workspace/api/projects/state",
        Some(payload),
    )
    .await;
    assert_eq!(conflict.0, StatusCode::CONFLICT);
    assert_eq!(registry.project_settings(&project).unwrap(), committed);
    let read = request_json(app.clone(), "GET", uri, None).await.1;
    assert_eq!(read["execution"]["stored"], "dev");
    let reopen = request_json(
        app.clone(),
        "POST",
        "/workspace/api/projects/register",
        Some(json!({"project_path":project, "execution_profile_id":"dev"})),
    )
    .await;
    assert_eq!(reopen.0, StatusCode::BAD_REQUEST);
    assert_eq!(registry.project_settings(&project).unwrap(), committed);
    let unchanged = request_json(app.clone(), "PATCH", "/workspace/api/settings", Some(json!({"section":"project_execution", "expected_revision":read["execution"]["revision"], "value":{"project_path":project}}))).await;
    assert_eq!(unchanged.0, StatusCode::OK);
    assert_eq!(
        unchanged.1["execution"]["revision"],
        read["execution"]["revision"]
    );
    assert_eq!(registry.project_settings(&project).unwrap(), committed);
    let cleared = request_json(app.clone(), "PATCH", "/workspace/api/settings", Some(json!({"section":"project_execution", "expected_revision":read["execution"]["revision"], "value":{"project_path":project, "execution_profile_id":null}}))).await;
    assert_eq!(cleared.0, StatusCode::OK, "{cleared:?}");
    assert!(cleared.1["execution"]["stored"].is_null());
    assert_eq!(cleared.1["execution"]["source"], "workspace");
}

/// A settings draft read before a project's removal conflicts after its
/// re-registration, for model and execution defaults alike, instead of
/// overwriting the new registration's defaults.
#[tokio::test]
async fn project_settings_drafts_from_a_removed_registration_conflict() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    fs::create_dir_all(&settings.config_dir).unwrap();
    fs::write(
        settings.config_dir.join("spark.toml"),
        "[execution_profiles.dev]\nlabel = 'Development'\nmode = 'native'\n",
    )
    .unwrap();
    let project_dir = temp.path().join("again");
    fs::create_dir_all(&project_dir).unwrap();
    let app = build_app(settings.clone()).expect("app");
    let register = || {
        request_json(
            app.clone(),
            "POST",
            "/workspace/api/projects/register",
            Some(json!({"project_path": project_dir})),
        )
    };
    let registered = register().await;
    assert_eq!(registered.0, StatusCode::OK, "{}", registered.1);
    let project = registered.1["project_path"].as_str().unwrap().to_string();
    let uri = format!(
        "/workspace/api/settings?project_path={}",
        crate::encode_query(&project)
    );
    let stale = request_json(app.clone(), "GET", &uri, None).await.1;
    let removal = format!(
        "/workspace/api/projects?project_path={}",
        crate::encode_query(&project)
    );
    let removed = request_json(app.clone(), "DELETE", &removal, None).await;
    assert_eq!(removed.0, StatusCode::OK, "{}", removed.1);
    assert_eq!(register().await.0, StatusCode::OK);
    let registry = spark_storage::ProjectRegistry::new(&settings.data_dir);
    let fresh = registry.project_settings(&project).unwrap();

    let models = request_json(
        app.clone(),
        "PATCH",
        "/workspace/api/settings",
        Some(json!({
            "section": "project_models", "expected_revision": stale["models"]["revision"],
            "value": {"project_path": project, "model_settings": {"provider": "claude-code"}}
        })),
    )
    .await;
    assert_eq!(models.0, StatusCode::CONFLICT, "{models:?}");
    let execution = request_json(
        app.clone(),
        "PATCH",
        "/workspace/api/settings",
        Some(json!({
            "section": "project_execution", "expected_revision": stale["execution"]["revision"],
            "value": {"project_path": project, "execution_profile_id": "dev"}
        })),
    )
    .await;
    assert_eq!(execution.0, StatusCode::CONFLICT, "{execution:?}");
    let state = request_json(
        app.clone(),
        "PATCH",
        "/workspace/api/projects/state",
        Some(json!({
            "project_path": project, "execution_profile_id": "dev",
            "expected_revision": stale["execution"]["revision"]
        })),
    )
    .await;
    assert_eq!(state.0, StatusCode::CONFLICT, "{state:?}");
    assert_eq!(registry.project_settings(&project).unwrap(), fresh);

    // The new registration's own revision is accepted.
    let current = request_json(app.clone(), "GET", &uri, None).await.1;
    let saved = request_json(
        app,
        "PATCH",
        "/workspace/api/settings",
        Some(json!({
            "section": "project_execution", "expected_revision": current["execution"]["revision"],
            "value": {"project_path": project, "execution_profile_id": "dev"}
        })),
    )
    .await;
    assert_eq!(saved.0, StatusCode::OK, "{saved:?}");
    assert_eq!(saved.1["execution"]["stored"], "dev");
}

#[tokio::test]
async fn client_preferences_are_isolated_revision_checked_and_validated() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let mut app = build_app(settings.clone()).expect("app");
    let uri = "/workspace/api/settings?client_id=browser-one";
    let initial = request_json(app.clone(), "GET", uri, None).await;
    assert_eq!(initial.0, StatusCode::OK);
    assert_eq!(
        initial.1["preferences"]["effective"]["editor_sidebar_width"],
        288
    );
    assert_eq!(initial.1["preferences"]["effective"]["show_thinking"], true);
    let payload = json!({"section":"client_preferences", "expected_revision": initial.1["preferences"]["revision"],
        "value":{"client_id":"browser-one", "preferences":{"editor_mode":"files", "editor_sidebar_width":400,
            "show_advanced_controls":true,"expand_child_flows":true,"graph_settings_open":true,"show_thinking":false,
            "runs_scope":"all","triggers_scope":"active","home_sidebar_primary_split_ratio":0.6,"appearance":"dark"}}});
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
        ("show_thinking", json!(false)),
        ("runs_scope", json!("all")),
        ("triggers_scope", json!("active")),
        ("appearance", json!("dark")),
        ("editor_mode", json!("files")),
    ] {
        assert_eq!(saved.1["preferences"]["stored"][field], expected);
        // One app per home: the old one releases the home before a restart.
        drop(app);
        app = build_app(settings.clone()).expect("app");
        let restarted = request_json(app.clone(), "GET", uri, None).await;
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
        json!({"selected_record":"session-only"}),
        json!({"runs_scope":"unknown"}),
        json!({"triggers_scope":"unknown"}),
        json!({"appearance":"sepia"}),
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
    // An unknown mode, such as the old `raw`, falls back to structured.
    let legacy = json!({"section":"client_preferences", "expected_revision":saved.1["preferences"]["revision"],
        "value":{"client_id":"browser-one", "preferences":{"editor_mode":"raw"}}});
    let legacy = request_json(
        app.clone(),
        "PATCH",
        "/workspace/api/settings",
        Some(legacy),
    )
    .await;
    assert_eq!(legacy.0, StatusCode::OK, "{legacy:?}");
    assert_eq!(
        legacy.1["preferences"]["effective"]["editor_mode"],
        "structured"
    );
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
    drop(app);
    let restarted = build_app(settings).expect("app");
    let read = request_json(restarted, "GET", uri, None).await;
    assert_eq!(
        read.1["preferences"]["stored"],
        legacy.1["preferences"]["stored"]
    );
}

#[tokio::test]
async fn profile_resource_adapters_share_settings_revisions_and_validation() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let app = build_app(settings.clone()).expect("app");
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
    drop(app);
    let restarted = request_json(
        build_app(settings).expect("app"),
        "GET",
        "/workspace/api/settings",
        None,
    )
    .await;
    assert_eq!(restarted.1["llm_profiles"]["stored"], json!([]));
}

#[tokio::test]
async fn referenced_synthesized_native_profile_cannot_be_removed_and_project_still_executes() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let app = build_app(settings.clone()).expect("app");
    let registered = request_json(
        app.clone(),
        "POST",
        "/workspace/api/projects/register",
        Some(json!({"project_path":project, "execution_profile_id":"native"})),
    )
    .await;
    assert_eq!(registered.0, StatusCode::OK, "{registered:?}");
    let path = settings.config_dir.join("spark.toml");
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
        rejected.1.to_string().contains(&format!(
            "project defaults of {}.execution_profile_id",
            registered.1["project_path"].as_str().unwrap()
        )),
        "{rejected:?}"
    );
    assert!(!path.exists());
    let launched = request_json(app, "POST", "/attractor/pipelines", Some(json!({
        "run_id":"native-after-rejected-save", "working_directory":project, "wait":true,
        "flow_name": crate::install_flow(&settings, json!({"schema_version": "2", "id": "native", "nodes": {"start": {"kind": "start", "config": {"kind": "start"}, "routes": {"next": {"to": "end", "description": "Enter the first workflow step."}}}, "end": {"kind": "exit", "config": {"kind": "exit", "result": {"name": "completed", "outcome": "success"}}}}}))
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
            "provider='codex'\nthinking='invalid'",
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
            if scope == "workspace" {
                let path = settings.config_dir.join("spark.toml");
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(&path, format!("[models]\n{group}\n")).unwrap();
            } else {
                // Stored project defaults that no longer validate.
                let models = group
                    .lines()
                    .map(|line| {
                        let (key, value) = line.split_once('=').unwrap();
                        (key.to_string(), json!(value.trim_matches('\'')))
                    })
                    .collect::<serde_json::Map<_, _>>();
                let revision = registry
                    .project_settings(project.to_str().unwrap())
                    .unwrap()
                    .revision;
                registry
                    .update_project_model_settings(
                        project.to_str().unwrap(),
                        &revision,
                        Some(&json!(models)),
                    )
                    .unwrap();
            }
            let app = build_app(settings.clone()).expect("app");
            let uri = if scope == "workspace" {
                "/workspace/api/settings".to_owned()
            } else {
                format!(
                    "/workspace/api/settings?project_path={}",
                    crate::encode_query(project.to_string_lossy())
                )
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
                "flow_name": crate::install_flow(&settings, json!({"schema_version": "2", "id": "invalid", "nodes": {"start": {"kind": "start", "config": {"kind": "start"}, "routes": {"next": {"to": "end", "description": "Enter the first workflow step."}}}, "end": {"kind": "exit", "config": {"kind": "exit", "result": {"name": "completed", "outcome": "success"}}}}}))
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

/// A final answer as a codex turn reports it.
fn final_answer(text: &str) -> spark_agent_adapter::AgentTurnOutput {
    static ITEMS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let item = ITEMS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut event = spark_common::events::TurnStreamEvent::content_delta(
        spark_common::events::TurnStreamChannel::Assistant,
        text,
    );
    event.kind = spark_common::events::TurnStreamEventKind::ContentCompleted;
    event.phase = Some("final_answer".into());
    event.source.app_turn_id = Some(format!("app-turn-{item}"));
    event.source.item_id = Some(format!("item-{item}"));
    spark_agent_adapter::AgentTurnOutput {
        events: vec![event],
        final_assistant_text: Some(text.into()),
        ..Default::default()
    }
}

/// The next live event, past the keepalives a slow test outlasts.
async fn next_live_event<B, E>(events: &mut B) -> String
where
    B: futures_util::Stream<Item = Result<axum::body::Bytes, E>> + Unpin,
    E: std::fmt::Debug,
{
    use futures_util::StreamExt;
    loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(120), events.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let event = String::from_utf8(event.to_vec()).unwrap();
        if event != ": keepalive\n\n" {
            return event;
        }
    }
}

/// Nothing but keepalives arrives for a moment.
async fn assert_no_live_event<B, E>(events: &mut B)
where
    B: futures_util::Stream<Item = Result<axum::body::Bytes, E>> + Unpin,
    E: std::fmt::Debug,
{
    use futures_util::StreamExt;
    while let Ok(event) =
        tokio::time::timeout(std::time::Duration::from_millis(50), events.next()).await
    {
        let event = event.unwrap().unwrap();
        assert_eq!(event.as_ref(), b": keepalive\n\n", "unexpected live event");
    }
}

#[tokio::test]
async fn chat_model_route_exposes_efforts_with_or_without_a_project() {
    use futures_util::StreamExt;
    use std::io::{BufRead, Write};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    };
    use std::time::Duration;

    const CHILD: &str = "SPARK_REACHABLE_MODELS_TEST";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "workspace_route_contracts::chat_model_route_exposes_efforts_with_or_without_a_project", "--nocapture"])
            .env_clear().env(CHILD, "1").env("SPARK_FIXTURE_KEY", "fixture-secret").output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let mut settings = settings(temp.path());
    let binaries = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    std::env::set_var(
        "SPARK_CODEX_APP_SERVER_BIN",
        binaries.join("spark-agent-fake-codex-app-server"),
    );
    std::env::set_var(
        "SPARK_CLAUDE_CODE_BIN",
        binaries.join("spark-agent-fake-claude-code"),
    );
    let codex_log = temp.path().join("codex.log");
    let claude_log = temp.path().join("claude.log");
    std::env::set_var("SPARK_FAKE_CODEX_APP_SERVER_LOG", &codex_log);
    std::env::set_var("SPARK_FAKE_CLAUDE_CODE_LOG", &claude_log);
    let runtime = temp.path().join("codex-runtime");
    settings.agents.native.codex_runtime_root = Some(runtime.to_string_lossy().into());
    let app = build_app(settings).expect("app");
    let live = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/workspace/api/live/events?include_settings=true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(live.status(), StatusCode::OK);
    let mut events = live.into_body().into_data_stream();
    assert_eq!(events.next().await.unwrap().unwrap(), ": keepalive\n\n");
    let uri = "/workspace/api/projects/chat-models";
    let initial = request_json(app.clone(), "GET", uri, None).await;
    assert_eq!(initial.0, StatusCode::OK, "{}", initial.1);
    for provider in ["anthropic", "openai", "gemini", "openrouter", "litellm"] {
        assert!(initial.1["providers"].get(provider).is_none());
        assert!(!initial.1["models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|model| model["provider"] == provider));
    }
    let count_native = || {
        let codex = fs::read_to_string(&codex_log)
            .unwrap()
            .lines()
            .filter(|line| line.contains("model/list"))
            .count();
        let claude = fs::read_to_string(&claude_log)
            .unwrap()
            .lines()
            .filter(|line| *line == "-- invocation --")
            .count();
        (codex, claude)
    };
    assert_eq!(count_native(), (1, 1));
    for _ in 0..3 {
        let status =
            request_json(app.clone(), "GET", "/workspace/api/codex/connection", None).await;
        assert_eq!(status.0, StatusCode::OK);
        assert_eq!(status.1["status"], "disconnected");
        assert_eq!(
            initial.1,
            request_json(app.clone(), "GET", uri, None).await.1
        );
        assert_eq!(count_native(), (1, 1));
        assert_no_live_event(&mut events).await;
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let counts = Arc::new(Mutex::new(
        std::collections::BTreeMap::<String, usize>::new(),
    ));
    let stop = Arc::new(AtomicBool::new(false));
    let server_counts = counts.clone();
    let server_stop = stop.clone();
    let server = std::thread::spawn(move || {
        while !server_stop.load(Ordering::Relaxed) {
            let (mut stream, _) = match listener.accept() {
                Ok(stream) => stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("{error}"),
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(120)))
                .unwrap();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                headers.push_str(&line);
            }
            let path = headers.split_whitespace().nth(1).unwrap();
            let provider = path.split('/').nth(1).unwrap();
            let count = {
                let mut counts = server_counts.lock().unwrap();
                let count = counts.entry(provider.into()).or_default();
                *count += 1;
                *count
            };
            let auth = match provider {
                "anthropic" => "x-api-key: fixture-secret",
                "gemini" => "x-goog-api-key: fixture-secret",
                _ => "authorization: bearer fixture-secret",
            };
            if provider != "litellm" {
                assert!(headers.to_lowercase().contains(auth), "{headers}");
            }
            assert!(path.contains(if provider == "gemini" {
                "/v1beta/models"
            } else {
                "/v1/models"
            }));
            let (status, body) = match provider {
                "anthropic" => (
                    200,
                    json!({"data":[{"id":"claude-opus-4-6", "display_name":"Live Opus", "capabilities":{"effort":{"low":{"supported":true}}}}], "has_more":false}),
                ),
                "openai" => (
                    200,
                    json!({"data":[{"id":"gpt-6-astra"}, {"id":"text-embedding-3-small"}]}),
                ),
                "gemini" => (
                    200,
                    json!({"models":[{"name":"models/gemini-3.1-pro-preview", "thinking":false, "supportedGenerationMethods":["generateContent"]}, {"name":"models/embed", "supportedGenerationMethods":["embedContent"]}]}),
                ),
                "openrouter" if count == 1 => (401, json!({"error":"fixture unauthorized"})),
                "openrouter" => (
                    200,
                    json!({"data":[{"id":"live-router", "reasoning":{"supported_efforts":["high"], "default_effort":"high"}}]}),
                ),
                "litellm" => (200, json!({"data":[{"id":"proxy-model"}]})),
                _ => panic!("unexpected provider {provider}"),
            };
            let body = body.to_string();
            write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    let providers: serde_json::Map<String, Value> = ["anthropic", "openai", "gemini", "openrouter", "litellm"].into_iter().map(|provider| {
        (provider.into(), json!({"base_url":format!("http://{address}/{provider}"), "api_key_env": if provider == "litellm" { "MISSING_OPTIONAL_KEY" } else { "SPARK_FIXTURE_KEY" }}))
    }).collect();
    let view = request_json(app.clone(), "GET", "/workspace/api/settings", None)
        .await
        .1;
    let saved = request_json(app.clone(), "PATCH", "/workspace/api/settings", Some(json!({"section":"providers", "expected_revision":view["providers"]["revision"], "value": providers}))).await;
    assert_eq!(saved.0, StatusCode::OK, "{}", saved.1);
    let event = next_live_event(&mut events).await;
    assert!(event.contains("settings.changed"), "{event}");
    let first = request_json(app.clone(), "GET", uri, None).await.1;
    assert_eq!(first["providers"]["openrouter"]["status"], "unavailable");
    assert!(first["providers"]["openrouter"]["error"]
        .as_str()
        .unwrap()
        .contains("401"));
    assert!(!first.to_string().contains("fixture-secret"));
    let models = first["models"].as_array().unwrap();
    assert!(!models.iter().any(|model| model["provider"] == "openrouter"
        || model["id"] == "text-embedding-3-small"
        || model["id"] == "embed"));
    assert_eq!(
        models
            .iter()
            .find(|model| model["provider"] == "gemini")
            .unwrap()["supported_reasoning_efforts"],
        json!([])
    );
    assert_eq!(
        models
            .iter()
            .find(|model| model["provider"] == "openai")
            .unwrap()["supported_reasoning_efforts"],
        json!(["low", "medium", "high", "xhigh", "max"])
    );
    assert_eq!(
        first,
        request_json(
            app.clone(),
            "GET",
            "/workspace/api/projects/chat-models?project_path=%2Fproject",
            None
        )
        .await
        .1
    );
    assert!(counts.lock().unwrap().values().all(|count| *count == 1));
    assert_eq!(count_native(), (1, 1));
    for (index, section) in ["providers", "llm_profiles", "agents"]
        .into_iter()
        .enumerate()
    {
        let view = request_json(app.clone(), "GET", "/workspace/api/settings", None)
            .await
            .1;
        let saved = request_json(app.clone(), "PATCH", "/workspace/api/settings", Some(json!({"section":section, "expected_revision":view[section]["revision"], "value":view[section]["stored"]}))).await;
        assert_eq!(saved.0, StatusCode::OK, "{}", saved.1);
        let event = next_live_event(&mut events).await;
        assert!(event.contains("settings.changed"), "{event}");
        let refreshed = request_json(app.clone(), "GET", uri, None).await.1;
        assert_eq!(refreshed["providers"]["openrouter"]["status"], "available");
        assert_eq!(
            refreshed,
            request_json(app.clone(), "GET", uri, None).await.1
        );
        let expected = if section == "agents" { 3 } else { index + 2 };
        assert!(
            counts
                .lock()
                .unwrap()
                .values()
                .all(|count| *count == expected),
            "{:?}",
            counts
        );
        assert_eq!(
            count_native(),
            if section == "agents" { (2, 2) } else { (1, 1) }
        );
    }
    // Completion refreshes without depending on the dialog's status polling.
    let login = request_json(
        app.clone(),
        "POST",
        "/workspace/api/codex/login",
        Some(json!({"method":"browser"})),
    )
    .await;
    assert_eq!(login.0, StatusCode::OK, "{}", login.1);
    for _ in 0..100 {
        let status =
            request_json(app.clone(), "GET", "/workspace/api/codex/connection", None).await;
        if status.1["status"] == "connected" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    for _ in 0..2 {
        request_json(app.clone(), "GET", uri, None).await;
    }
    assert_eq!(count_native(), (3, 2));
    let event = next_live_event(&mut events).await;
    assert!(event.contains("settings.changed"), "{event}");
    assert!(event.contains("\"section\":\"codex\""), "{event}");
    request_json(app.clone(), "GET", "/workspace/api/codex/connection", None).await;
    request_json(app.clone(), "GET", uri, None).await;
    assert_eq!(count_native(), (3, 2));
    assert_no_live_event(&mut events).await;
    // An externally completed sign-out is observed by the existing connection check.
    fs::remove_file(runtime.join(".codex/fake-login-complete")).unwrap();
    request_json(app.clone(), "GET", "/workspace/api/codex/connection", None).await;
    for _ in 0..2 {
        request_json(app.clone(), "GET", uri, None).await;
    }
    assert_eq!(count_native(), (4, 2));
    let event = next_live_event(&mut events).await;
    assert!(event.contains("settings.changed"), "{event}");
    assert!(event.contains("\"section\":\"codex\""), "{event}");
    request_json(app.clone(), "GET", "/workspace/api/codex/connection", None).await;
    request_json(app.clone(), "GET", uri, None).await;
    assert_eq!(count_native(), (4, 2));
    assert_no_live_event(&mut events).await;
    assert!(counts.lock().unwrap().values().all(|count| *count == 3));
    stop.store(true, Ordering::Relaxed);
    server.join().unwrap();
    assert_eq!(
        request_json(
            app,
            "GET",
            "/workspace/api/projects/chat-models?project_path=",
            None
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn reasoning_controls_round_trip_and_validate_at_settings_scopes() {
    let temp = tempfile::tempdir().unwrap();
    let config = settings(temp.path());
    spark_storage::ProjectRegistry::new(&config.data_dir)
        .register_project("/projects/reasoning")
        .unwrap();
    let app = build_app(config).expect("app");
    let group = json!({"provider":"openai", "model":"future-model", "thinking":"budget", "thinking_budget_tokens":2048, "reasoning_mode":"pro", "reasoning_summary":"concise"});
    for (scope, query) in [
        ("models", ""),
        ("project_models", "?project_path=/projects/reasoning"),
        (
            "conversation_models",
            "?project_path=/projects/reasoning&conversation_id=reasoning-chat",
        ),
    ] {
        let uri = format!("/workspace/api/settings{query}");
        let initial = request_json(app.clone(), "GET", &uri, None).await;
        let value = if scope == "models" {
            group.clone()
        } else if scope == "project_models" {
            json!({"project_path":"/projects/reasoning", "model_settings":group})
        } else {
            json!({"project_path":"/projects/reasoning", "conversation_id":"reasoning-chat", "model_settings":group})
        };
        let saved = request_json(app.clone(), "PATCH", "/workspace/api/settings", Some(json!({"section":scope,"expected_revision":initial.1["models"]["revision"],"value":value}))).await;
        assert_eq!(saved.0, StatusCode::OK, "{scope}: {}", saved.1);
        let read = request_json(app.clone(), "GET", &uri, None).await;
        for field in [
            "thinking",
            "thinking_budget_tokens",
            "reasoning_mode",
            "reasoning_summary",
        ] {
            assert_eq!(
                read.1["models"]["effective"][field], group[field],
                "{scope}: {field}"
            );
        }
        for field in [
            "thinking",
            "thinking_budget_tokens",
            "reasoning_mode",
            "reasoning_summary",
        ] {
            let mut bad = value.clone();
            let target = if scope == "models" {
                &mut bad
            } else {
                &mut bad["model_settings"]
            };
            target[field] = if field == "thinking_budget_tokens" {
                json!(1)
            } else {
                json!("invalid")
            };
            let rejected = request_json(app.clone(), "PATCH", "/workspace/api/settings", Some(json!({"section":scope,"expected_revision":read.1["models"]["revision"],"value":bad}))).await;
            assert_eq!(
                rejected.0,
                StatusCode::BAD_REQUEST,
                "{scope}: {}",
                rejected.1
            );
            assert!(
                rejected.1.to_string().contains(field),
                "{scope}: {}",
                rejected.1
            );
        }
    }
}
