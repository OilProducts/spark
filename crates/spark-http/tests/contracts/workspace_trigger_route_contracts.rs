use std::fs;
use std::path::Path;

use attractor_runtime::RunStore;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::{json, Map, Value};
use spark_common::settings::SparkSettings;
use spark_http::build_app;
use spark_storage::{
    TriggerAction, TriggerDefinition, TriggerRepositories, TriggerState, TriggerStateHistoryEntry,
};
use tower::ServiceExt;

#[tokio::test]
async fn trigger_crud_routes_persist_definition_and_state_contracts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    install_run_flow(&settings);
    let other = project(&settings, "other");
    let project = project(&settings, "project");
    let org = spark_common::project::definitions_dir(Path::new(&project));
    let app = build_app(settings.clone()).expect("app");

    let empty = request_json(app.clone(), "GET", "/workspace/api/triggers", None).await;
    assert_eq!(empty.0, StatusCode::OK);
    assert_eq!(empty.1, json!([]));

    let created = request_json(
        app.clone(),
        "POST",
        "/workspace/api/triggers",
        Some(json!({
            "project_path": project,
            "message": "Watch the CI webhook",
            "name": "Compat webhook",
            "source_type": "webhook",
            "action": {
                "flow_name": RUN_FLOW,
                "static_context": {"origin": "compat"}
            },
            "source": {}
        })),
    )
    .await;
    assert_eq!(created.0, StatusCode::OK);
    assert_eq!(created.1["enabled"], json!(true));
    assert_eq!(created.1["project_path"], json!(project));
    assert_eq!(created.1["source_type"], "webhook");
    assert!(created.1["source"]["webhook_key"].as_str().unwrap().len() == 16);
    assert!(created.1["webhook_secret"].as_str().unwrap().len() == 32);
    assert!(created.1["source"].get("secret_hash").is_none());
    assert!(created.1["action"].get("project_path").is_none());
    let trigger_id = created.1["id"].as_str().unwrap().to_string();

    let listed = request_json(app.clone(), "GET", "/workspace/api/triggers", None).await;
    assert_eq!(listed.0, StatusCode::OK);
    assert_eq!(listed.1.as_array().expect("trigger list").len(), 1);
    assert_eq!(listed.1[0]["id"], trigger_id);
    assert_eq!(listed.1[0]["project_path"], json!(project));
    assert_eq!(listed.1[0]["name"], "Compat webhook");
    assert!(listed.1[0].get("webhook_secret").is_none());
    assert!(listed.1[0]["source"].get("secret_hash").is_none());
    let filtered = request_json(
        app.clone(),
        "GET",
        &format!("/workspace/api/triggers?project_path={other}"),
        None,
    )
    .await;
    assert_eq!(filtered.1, json!([]));

    let definition_path = org.join("triggers").join(format!("{trigger_id}.toml"));
    let definition_text = fs::read_to_string(&definition_path).expect("definition toml");
    for absent in ["enabled", "project_path", "webhook_key", "secret_hash"] {
        assert!(!definition_text.contains(absent), "{absent}");
    }
    let states = TriggerRepositories::from_settings(&settings)
        .runtime_state(&project)
        .unwrap();
    assert!(states.get(&trigger_id).unwrap().is_some());

    let url = format!("/workspace/api/triggers/{trigger_id}?project_path={project}");
    let unscoped = request_json(
        app.clone(),
        "GET",
        &format!("/workspace/api/triggers/{trigger_id}"),
        None,
    )
    .await;
    assert_eq!(unscoped.0, StatusCode::BAD_REQUEST);
    let elsewhere = request_json(
        app.clone(),
        "GET",
        &format!("/workspace/api/triggers/{trigger_id}?project_path={other}"),
        None,
    )
    .await;
    assert_eq!(elsewhere.0, StatusCode::NOT_FOUND);
    let described = request_json(app.clone(), "GET", &url, None).await;
    assert_eq!(described.0, StatusCode::OK);
    assert!(described.1.get("webhook_secret").is_none());
    assert!(described.1["source"].get("secret_hash").is_none());

    // Switching enabled writes state only: no definition change.
    let disabled = request_json(
        app.clone(),
        "PATCH",
        &url,
        Some(json!({"expected_revision": described.1["revision"], "enabled": false})),
    )
    .await;
    assert_eq!(disabled.0, StatusCode::OK);
    assert_eq!(disabled.1["enabled"], json!(false));
    assert_eq!(disabled.1["revision"], described.1["revision"]);
    assert!(!states.load(&trigger_id).unwrap().enabled);
    assert_eq!(
        definition_text,
        fs::read_to_string(&definition_path).unwrap()
    );

    let updated = request_json(
        app.clone(),
        "PATCH",
        &url,
        Some(json!({"expected_revision": described.1["revision"], "name": "Compat webhook updated", "regenerate_webhook_secret": true})),
    )
    .await;
    assert_eq!(updated.0, StatusCode::OK);
    assert_eq!(updated.1["name"], "Compat webhook updated");
    assert_eq!(updated.1["enabled"], json!(false));
    assert_eq!(
        updated.1["source"]["webhook_key"],
        described.1["source"]["webhook_key"]
    );
    assert!(updated.1["webhook_secret"].as_str().unwrap().len() == 32);

    let deleted = request_json(
        app,
        "DELETE",
        &format!(
            "{url}&expected_revision={}&message=Retired",
            updated.1["revision"].as_str().unwrap()
        ),
        None,
    )
    .await;
    assert_eq!(deleted.0, StatusCode::OK);
    assert_eq!(
        deleted.1,
        json!({"status": "deleted", "id": trigger_id, "project_path": project})
    );
    assert!(!definition_path.exists());
    assert_eq!(states.get(&trigger_id).unwrap(), None);
    assert_eq!(fs::read_dir(org.join("triggers")).unwrap().count(), 0);
}

#[tokio::test]
async fn triggers_are_created_in_registered_projects_only() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    install_run_flow(&settings);
    let unregistered = temp.path().join("unregistered");
    fs::create_dir_all(&unregistered).expect("project");
    let response = request_json(
        build_app(settings).expect("app"),
        "POST",
        "/workspace/api/triggers",
        Some(json!({
            "project_path": unregistered,
            "name": "Nowhere",
            "source_type": "webhook",
            "action": {"flow_name": RUN_FLOW},
            "source": {}
        })),
    )
    .await;
    assert_eq!(response.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        response.1,
        json!({"detail": "Triggers are created in a registered project."})
    );
}

#[tokio::test]
async fn webhook_route_authenticates_storage_backed_trigger_and_dispatches_run() {
    let temp = tempfile::tempdir().expect("tempdir");
    // Canonicalize: dispatched run records hold canonical paths (macOS /var -> /private/var).
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let settings = settings(&root);
    install_run_flow(&settings);
    let project_dir = project(&settings, "webhook-project");
    let app = build_app(settings.clone()).expect("app");

    let created = request_json(
        app.clone(),
        "POST",
        "/workspace/api/triggers",
        Some(json!({
            "project_path": project_dir,
            "name": "Compat webhook",
            "source_type": "webhook",
            "action": {
                "flow_name": RUN_FLOW,
                "static_context": {"origin": "route"}
            },
            "source": {}
        })),
    )
    .await;
    assert_eq!(created.0, StatusCode::OK);
    let trigger_id = created.1["id"].as_str().expect("trigger id").to_string();
    let webhook_key = created.1["source"]["webhook_key"]
        .as_str()
        .expect("webhook key")
        .to_string();
    let webhook_secret = created.1["webhook_secret"]
        .as_str()
        .expect("webhook secret")
        .to_string();

    let missing_headers = request_json(
        app.clone(),
        "POST",
        "/workspace/api/webhooks",
        Some(json!({"payload": "compat"})),
    )
    .await;
    assert_eq!(missing_headers.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        missing_headers.1,
        json!({"detail": "Webhook key and secret headers are required."})
    );

    let bad_secret = request_json_with_headers(
        app.clone(),
        "POST",
        "/workspace/api/webhooks",
        &[
            ("X-Spark-Webhook-Key", &webhook_key),
            ("X-Spark-Webhook-Secret", "not-the-secret"),
        ],
        Some(json!({"payload": "compat"})),
    )
    .await;
    assert_eq!(bad_secret.0, StatusCode::FORBIDDEN);
    assert_eq!(
        bad_secret.1,
        json!({"detail": "Webhook secret is invalid."})
    );

    let unknown_key = request_json_with_headers(
        app.clone(),
        "POST",
        "/workspace/api/webhooks",
        &[
            ("X-Spark-Webhook-Key", "missing-key"),
            ("X-Spark-Webhook-Secret", &webhook_secret),
        ],
        Some(json!({"payload": "compat"})),
    )
    .await;
    assert_eq!(unknown_key.0, StatusCode::NOT_FOUND);
    assert_eq!(unknown_key.1, json!({"detail": "Unknown webhook key."}));

    let malformed = request_text_with_headers(
        app.clone(),
        "POST",
        "/workspace/api/webhooks",
        &[
            ("X-Spark-Webhook-Key", &webhook_key),
            ("X-Spark-Webhook-Secret", &webhook_secret),
        ],
        Some("{not-json"),
    )
    .await;
    assert_eq!(malformed.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&malformed.1).expect("json"),
        json!({"detail": "Webhook payload must be valid JSON."})
    );

    let non_object = request_json_with_headers(
        app.clone(),
        "POST",
        "/workspace/api/webhooks",
        &[
            ("X-Spark-Webhook-Key", &webhook_key),
            ("X-Spark-Webhook-Secret", &webhook_secret),
        ],
        Some(json!(["not-object"])),
    )
    .await;
    assert_eq!(non_object.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        non_object.1,
        json!({"detail": "Webhook payload must be a JSON object."})
    );

    let accepted = request_json_with_headers(
        app,
        "POST",
        "/workspace/api/webhooks",
        &[
            ("X-Spark-Webhook-Key", &webhook_key),
            ("X-Spark-Webhook-Secret", &webhook_secret),
            ("X-Spark-Webhook-Request-Id", "request-1"),
        ],
        Some(json!({"payload": "compat"})),
    )
    .await;
    assert_eq!(accepted.0, StatusCode::OK);
    assert_eq!(accepted.1, json!({"ok": true, "trigger_id": trigger_id}));
    let state = load_state(&settings, &project_dir, &trigger_id);
    assert_eq!(state.last_result.as_deref(), Some("success"));
    let run_id = state.recent_history[0].run_id.as_deref().expect("run id");
    let run = RunStore::for_settings(&settings)
        .read_run_bundle(run_id)
        .expect("read webhook run")
        .expect("webhook run");
    let record = run.record.expect("run record");
    assert_eq!(record.flow_name, RUN_FLOW);
    assert_eq!(record.project_path, project_dir);
    let context = run.checkpoint.expect("checkpoint").context;
    assert_eq!(
        context["context.trigger_static"],
        json!({"origin": "route"})
    );
    assert_eq!(
        context["context.trigger_payload"],
        json!({"payload": "compat"})
    );
    assert_eq!(
        context["context.spark_trigger"],
        json!({
            "trigger_id": trigger_id,
            "trigger_name": "Compat webhook",
            "source_type": "webhook"
        })
    );
}

#[tokio::test]
async fn trigger_routes_return_json_errors_for_missing_protected_json_and_unknown_flow() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    install_run_flow(&settings);
    crate::install_flow(&settings, trigger_flow("trigger-other"));
    let project = project(&settings, "project");
    let definitions = TriggerRepositories::from_settings(&settings)
        .definitions(&project)
        .unwrap();
    definitions
        .put(&protected_definition("trigger-protected"), None)
        .expect("protected");
    let app = build_app(settings.clone()).expect("app");

    let missing = request_json(
        app.clone(),
        "GET",
        &format!("/workspace/api/triggers/missing?project_path={project}"),
        None,
    )
    .await;
    assert_eq!(missing.0, StatusCode::NOT_FOUND);
    assert_eq!(missing.1, json!({"detail": "Unknown trigger."}));

    let protected_delete = request_json(
        app.clone(),
        "DELETE",
        &format!(
            "/workspace/api/triggers/trigger-protected?project_path={project}&expected_revision=protected"
        ),
        None,
    )
    .await;
    assert_eq!(protected_delete.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        protected_delete.1,
        json!({"detail": "Protected triggers cannot be deleted."})
    );

    let static_context = request_json(
        app.clone(),
        "PATCH",
        &format!("/workspace/api/triggers/trigger-protected?project_path={project}"),
        Some(json!({"expected_revision": definitions.get("trigger-protected").unwrap().unwrap().revision, "action": {"static_context": {"changed": true}}})),
    )
    .await;
    assert_eq!(static_context.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        static_context.1,
        json!({"detail": "Protected triggers do not allow static context changes."})
    );

    let malformed = request_text(
        app.clone(),
        "POST",
        "/workspace/api/triggers",
        Some("{not-json"),
    )
    .await;
    assert_eq!(malformed.0, StatusCode::BAD_REQUEST);
    assert!(
        serde_json::from_str::<serde_json::Value>(&malformed.1).unwrap()["detail"]
            .as_str()
            .unwrap()
            .contains("Failed to parse")
    );

    let unknown_field = request_text(
        app.clone(),
        "POST",
        "/workspace/api/triggers",
        Some(
            &json!({
                "project_path": project,
                "name": "bad",
                "enabled": true,
                "source_type": "webhook",
                "action": {"flow_name": RUN_FLOW},
                "source": {},
                "extra": true
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(unknown_field.0, StatusCode::BAD_REQUEST);
    assert!(
        serde_json::from_str::<serde_json::Value>(&unknown_field.1).unwrap()["detail"]
            .as_str()
            .unwrap()
            .contains("unknown field")
    );

    let unknown_flow = request_json(
        app,
        "POST",
        "/workspace/api/triggers",
        Some(json!({
            "project_path": project,
            "name": "Unknown flow",
            "enabled": true,
            "source_type": "webhook",
            "action": {"flow_name": "missing"},
            "source": {}
        })),
    )
    .await;
    // Saved, but activation fails: inactive, with the reason reported.
    assert_eq!(unknown_flow.0, StatusCode::OK);
    assert_eq!(unknown_flow.1["enabled"], false);
    assert_eq!(
        unknown_flow.1["state"]["activation_error"],
        "Unknown flow: missing"
    );

    assert!(definitions
        .get("trigger-protected")
        .expect("read protected")
        .is_some());
}

#[tokio::test]
async fn protected_trigger_delete_rejection_preserves_runtime_state_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    install_run_flow(&settings);
    let project = project(&settings, "project");
    let repositories = TriggerRepositories::from_settings(&settings);
    repositories
        .definitions(&project)
        .unwrap()
        .put(&protected_definition("trigger-protected"), None)
        .expect("protected");
    let original_state = TriggerState {
        last_error: Some("previous failure".to_string()),
        last_fired_at: Some("2026-06-22T16:17:00Z".to_string()),
        last_result: Some("failed".to_string()),
        next_run_at: Some("2026-06-22T17:17:00Z".to_string()),
        recent_history: vec![TriggerStateHistoryEntry {
            timestamp: "2026-06-22T16:17:00Z".to_string(),
            status: "failed".to_string(),
            message: "keep this state".to_string(),
            run_id: Some("run-previous".to_string()),
        }],
        ..TriggerState::default()
    };
    let states = repositories.runtime_state(&project).unwrap();
    states
        .save("trigger-protected", &original_state)
        .expect("write trigger state");
    let definition_path = repositories
        .definitions(&project)
        .unwrap()
        .definition_path("trigger-protected")
        .unwrap();
    let delete_url = format!(
        "/workspace/api/triggers/trigger-protected?project_path={project}&expected_revision=protected"
    );
    let original_definition_text = fs::read_to_string(&definition_path).expect("definition text");
    let app = build_app(settings.clone()).expect("app");

    let protected_delete = request_json(app.clone(), "DELETE", &delete_url, None).await;
    assert_eq!(protected_delete.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        protected_delete.1,
        json!({"detail": "Protected triggers cannot be deleted."})
    );
    assert_eq!(
        fs::read_to_string(&definition_path).expect("definition unchanged"),
        original_definition_text
    );
    assert_eq!(
        states.get("trigger-protected").unwrap(),
        Some(original_state.clone())
    );

    states.delete("trigger-protected").expect("remove state");
    let protected_delete_without_state = request_json(app, "DELETE", &delete_url, None).await;
    assert_eq!(protected_delete_without_state.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        protected_delete_without_state.1,
        json!({"detail": "Protected triggers cannot be deleted."})
    );
    assert_eq!(states.get("trigger-protected").unwrap(), None);
    assert_eq!(
        fs::read_to_string(&definition_path).expect("definition unchanged"),
        original_definition_text
    );
}

#[tokio::test]
async fn trigger_routes_reject_malformed_persisted_definitions_and_list_skips_them() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = project(&settings, "project");
    write_persisted_trigger_toml(
        &project,
        "trigger-missing-flow",
        "webhook",
        "",
        r#"webhook_key = "key"
secret_hash = "hash""#,
    );
    write_persisted_trigger_toml(
        &project,
        "trigger-unknown-source",
        "unknown",
        RUN_FLOW,
        r#"kind = "once"
run_at = "2026-06-23T10:00:00Z""#,
    );
    write_persisted_trigger_toml(
        &project,
        "trigger-bad-statuses",
        "flow_event",
        RUN_FLOW,
        r#"statuses = ["running"]"#,
    );
    write_persisted_trigger_toml(
        &project,
        "trigger-invalid-schedule",
        "schedule",
        RUN_FLOW,
        r#"kind = "weekly"
weekdays = ["mon", "noday"]
hour = 9
minute = 0"#,
    );
    let app = build_app(settings).expect("app");

    let listed = request_json(app.clone(), "GET", "/workspace/api/triggers", None).await;
    assert_eq!(listed.0, StatusCode::OK);
    assert_eq!(listed.1, json!([]));

    let missing_flow = request_json(
        app.clone(),
        "GET",
        &format!("/workspace/api/triggers/trigger-missing-flow?project_path={project}"),
        None,
    )
    .await;
    assert_eq!(missing_flow.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        missing_flow.1,
        json!({"detail": "Trigger action requires a flow_name."})
    );

    let unknown_source = request_json(
        app.clone(),
        "PATCH",
        &format!("/workspace/api/triggers/trigger-unknown-source?project_path={project}"),
        Some(json!({"expected_revision": "invalid-document", "name": "Ignored"})),
    )
    .await;
    assert_eq!(unknown_source.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        unknown_source.1,
        json!({"detail": "Unsupported trigger source type: unknown"})
    );

    let bad_statuses = request_json(
        app.clone(),
        "DELETE",
        &format!(
            "/workspace/api/triggers/trigger-bad-statuses?project_path={project}&expected_revision=invalid-document"
        ),
        None,
    )
    .await;
    assert_eq!(bad_statuses.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        bad_statuses.1,
        json!({"detail": "Flow-event triggers require terminal statuses when statuses are provided."})
    );

    let invalid_schedule = request_json(
        app,
        "GET",
        &format!("/workspace/api/triggers/trigger-invalid-schedule?project_path={project}"),
        None,
    )
    .await;
    assert_eq!(invalid_schedule.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        invalid_schedule.1,
        json!({"detail": "Weekly schedule triggers require weekdays using mon..sun."})
    );
}

#[tokio::test]
async fn poll_trigger_headers_stay_private_on_create_and_update_contract() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    install_run_flow(&settings);
    let project = project(&settings, "project");
    let app = build_app(settings.clone()).expect("app");

    let created = request_json(
        app.clone(),
        "POST",
        "/workspace/api/triggers",
        Some(json!({
            "project_path": project,
            "name": "Compat poll",
            "enabled": true,
            "source_type": "poll",
            "action": {"flow_name": RUN_FLOW},
            "source": {
                "url": "https://example.test/items",
                "interval_seconds": 60,
                "items_path": "items",
                "item_id_path": "id",
                "headers": {
                    "Authorization": "Bearer create-token",
                    "x-bool": true,
                    "x-false": false,
                    "x-null": null,
                    "x-number": 5,
                    "x-text": "ok"
                }
            }
        })),
    )
    .await;

    assert_eq!(created.0, StatusCode::OK, "{}", created.1);
    assert_eq!(created.1["enabled"], true);
    let headers = json!({
        "Authorization": "Bearer create-token",
        "x-bool": "True",
        "x-false": "False",
        "x-null": "None",
        "x-number": "5",
        "x-text": "ok"
    });
    assert_eq!(created.1["source"]["headers"], headers);
    assert_eq!(created.1["state"].get("poll_headers"), None);
    let trigger_id = created.1["id"].as_str().expect("trigger id");
    let definitions = TriggerRepositories::from_settings(&settings)
        .definitions(&project)
        .unwrap();
    let persisted = || {
        let definition = definitions
            .get(trigger_id)
            .expect("read persisted trigger")
            .expect("persisted trigger");
        let text = fs::read_to_string(definitions.definition_path(trigger_id).unwrap())
            .expect("definition toml");
        (definition, text)
    };
    let (definition, text) = persisted();
    assert_eq!(definition.source.get("headers"), None);
    assert!(
        !text.contains("headers") && !text.contains("token"),
        "{text}"
    );
    let state = load_state(&settings, &project, trigger_id);
    assert_eq!(Value::Object(state.poll_headers), headers);

    let url = format!("/workspace/api/triggers/{trigger_id}?project_path={project}");
    let mut source = created.1["source"].clone();
    source["headers"] = json!({"Authorization": "Bearer update-token"});
    let updated = request_json(
        app.clone(),
        "PATCH",
        &url,
        Some(json!({
            "expected_revision": created.1["revision"],
            "enabled": true,
            "source": source
        })),
    )
    .await;
    assert_eq!(updated.0, StatusCode::OK, "{}", updated.1);
    assert_eq!(updated.1["enabled"], true);
    assert_eq!(
        updated.1["source"]["headers"],
        json!({"Authorization": "Bearer update-token"})
    );
    let (definition, text) = persisted();
    assert_eq!(definition.source.get("headers"), None);
    assert!(
        !text.contains("headers") && !text.contains("token"),
        "{text}"
    );
    assert_eq!(
        Value::Object(load_state(&settings, &project, trigger_id).poll_headers),
        json!({"Authorization": "Bearer update-token"})
    );
    let listed = request_json(app, "GET", "/workspace/api/triggers", None).await;
    assert_eq!(
        listed.1[0]["source"]["headers"],
        json!({"Authorization": "Bearer update-token"})
    );
}

#[tokio::test]
async fn enabled_creation_naming_a_missing_profile_saves_an_inactive_definition() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    install_run_flow(&settings);
    let project = project(&settings, "project");
    let app = build_app(settings.clone()).expect("app");

    let created = request_json(
        app.clone(),
        "POST",
        "/workspace/api/triggers",
        Some(json!({
            "project_path": project, "name": "Profiled", "enabled": true,
            "source_type": "webhook", "source": {},
            "action": {"flow_name": RUN_FLOW, "execution_profile_id": "missing"}
        })),
    )
    .await;

    assert_eq!(created.0, StatusCode::OK, "{}", created.1);
    assert_eq!(created.1["enabled"], false);
    let error = created.1["state"]["activation_error"].as_str().unwrap();
    assert!(error.contains("missing"), "{error}");
    let id = created.1["id"].as_str().unwrap();
    let definition = TriggerRepositories::from_settings(&settings)
        .definitions(&project)
        .unwrap()
        .get(id)
        .unwrap()
        .expect("definition saved");
    assert_eq!(
        definition.action.execution_profile_id.as_deref(),
        Some("missing")
    );
    let listed = request_json(app, "GET", "/workspace/api/triggers", None).await;
    assert_eq!(listed.1[0]["id"], id);
    assert_eq!(listed.1[0]["enabled"], false);
    assert_eq!(listed.1[0]["state"]["activation_error"], error);
}

async fn request_json(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    request_json_with_headers(app, method, uri, &[], body).await
}

async fn request_json_with_headers(
    app: axum::Router,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let body_text = body.map(|value| value.to_string());
    let response = request_text_with_headers(app, method, uri, headers, body_text.as_deref()).await;
    (
        response.0,
        serde_json::from_str(&response.1).expect("json response"),
    )
}

async fn request_text(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<&str>,
) -> (StatusCode, String) {
    request_text_with_headers(app, method, uri, &[], body).await
}

async fn request_text_with_headers(
    app: axum::Router,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> (StatusCode, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let request = builder
        .body(Body::from(body.unwrap_or_default().to_string()))
        .expect("request");
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&body).into_owned())
}

fn protected_definition(id: &str) -> TriggerDefinition {
    TriggerDefinition {
        revision: String::new(),
        project_path: String::new(),
        id: id.to_string(),
        name: "Protected".to_string(),
        protected: true,
        source_type: "webhook".to_string(),
        action: TriggerAction {
            mission_id: None,
            role: None,
            mode: "static".to_string(),
            flow_name: RUN_FLOW.to_string(),
            static_context: Map::from_iter([("origin".to_string(), json!("compat"))]),
            execution_profile_id: None,
        },
        source: Map::new(),
        created_at: "2026-06-22T16:16:08Z".to_string(),
        updated_at: "2026-06-22T16:16:08Z".to_string(),
    }
}

const RUN_FLOW: &str = "tests/trigger-route";

fn install_run_flow(settings: &SparkSettings) {
    assert_eq!(
        crate::install_flow(settings, trigger_flow("trigger_route")),
        RUN_FLOW
    );
}

fn trigger_flow(id: &str) -> Value {
    json!({
        "schema_version": "2",
        "id": id,
        "title": "Trigger Route",
        "nodes": {
            "start": {"kind": "start", "config": {"kind": "start"}, "routes": {"next": {"to": "done", "description": "Enter the first workflow step."}}},
            "done": {"kind": "exit", "config": {"kind": "exit", "result": {"name": "completed", "outcome": "success"}}}
        }
    })
}

fn write_persisted_trigger_toml(
    project: &str,
    id: &str,
    source_type: &str,
    flow_name: &str,
    source_body: &str,
) {
    let dir = spark_common::project::definitions_dir(Path::new(project)).join("triggers");
    fs::create_dir_all(&dir).expect("trigger config dir");
    fs::write(
        dir.join(format!("{id}.toml")),
        format!(
            r#"id = "{id}"
name = "Malformed"
protected = false
source_type = "{source_type}"
created_at = "2026-06-22T16:16:08Z"
updated_at = "2026-06-22T16:16:08Z"

[action]
flow_name = "{flow_name}"

[source]
{source_body}
"#
        ),
    )
    .expect("write malformed trigger definition");
}

fn load_state(settings: &SparkSettings, project: &str, id: &str) -> TriggerState {
    TriggerRepositories::from_settings(settings)
        .runtime_state(project)
        .unwrap()
        .load(id)
        .expect("trigger state")
}

/// A registered project.
fn project(settings: &SparkSettings, name: &str) -> String {
    let project = settings.data_dir.parent().unwrap().join(name);
    fs::create_dir_all(&project).expect("project dir");
    spark_storage::ProjectRegistry::new(settings.data_dir.clone())
        .ensure_project_paths(&project.to_string_lossy())
        .expect("register project")
        .project_path
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

#[tokio::test]
async fn webhook_dispatch_returns_while_the_run_still_executes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path_text = project(&settings, "slow-webhook-project");
    // The flow's only work node blocks until the test creates the release
    // file, so the run provably cannot complete before the test allows it —
    // no wall-clock budget involved. The gate is bounded so a failing test
    // that never releases it cannot leave the run spinning.
    let release_path = temp.path().join("release-webhook-run");
    let release_path_text = release_path.to_string_lossy().to_string();
    let slow_flow = crate::install_flow(
        &settings,
        json!({
            "schema_version": "2",
            "id": "slow_hook",
            "title": "Slow Hook",
            "nodes": {
                "start": {"kind": "start", "config": {"kind": "start"}, "routes": {"next": {"to": "work", "description": "Enter the first workflow step."}}},
                "work": {
                    "kind": "tool",
                    "config": {
                        "kind": "tool",
                        "command": format!(
                            "for i in $(seq 1 600); do [ -f \"{release_path_text}\" ] && exit 0; sleep 0.1; done; exit 1"
                        ),
                        "exit_codes": {"0": "done"}
                    },
                    "routes": {"done": {"to": "done", "description": "The release file appeared."}}
                },
                "done": {"kind": "exit", "config": {"kind": "exit", "result": {"name": "completed", "outcome": "success"}}}
            }
        }),
    );

    let app = spark_http::build_app(settings.clone()).expect("app");

    let created = request_json(
        app.clone(),
        "POST",
        "/workspace/api/triggers",
        Some(json!({
            "project_path": project_path_text,
            "name": "Slow webhook",
            "source_type": "webhook",
            "action": {
                "flow_name": slow_flow,
                "static_context": {"origin": "slow"}
            },
            "source": {}
        })),
    )
    .await;
    assert_eq!(created.0, StatusCode::OK);
    let webhook_key = created.1["source"]["webhook_key"]
        .as_str()
        .expect("key")
        .to_string();
    let webhook_secret = created.1["webhook_secret"]
        .as_str()
        .expect("secret")
        .to_string();

    // The dispatch must return at launch, not at run completion. The run
    // cannot finish until this test creates the release file, so a
    // synchronous dispatch would deadlock here rather than merely be slow —
    // any finite timeout catches it, independent of machine load.
    let accepted = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        request_json_with_headers(
            app,
            "POST",
            "/workspace/api/webhooks",
            &[
                ("X-Spark-Webhook-Key", webhook_key.as_str()),
                ("X-Spark-Webhook-Secret", webhook_secret.as_str()),
                ("X-Spark-Webhook-Request-Id", "slow-dispatch"),
            ],
            Some(json!({"payload": "slow"})),
        ),
    )
    .await
    .expect("webhook dispatch blocked on run execution: the run cannot complete until the test releases it");
    assert_eq!(accepted.0, StatusCode::OK);

    let trigger_id = accepted.1["trigger_id"].as_str().expect("trigger id");
    let state = load_state(&settings, &project_path_text, trigger_id);
    assert_eq!(
        state.last_result.as_deref(),
        Some("success"),
        "activation success now means launched",
    );
    let run_id = state.recent_history[0]
        .run_id
        .as_deref()
        .expect("run id")
        .to_string();

    // With the gate still closed the run cannot have completed; waiting for
    // it to report running is a one-sided wait that only fails if the
    // detached run never actually executes.
    let store = RunStore::for_settings(&settings);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let status = store
            .read_run_bundle(&run_id)
            .expect("bundle")
            .and_then(|bundle| bundle.record)
            .map(|record| record.status);
        if status.as_deref() == Some("running") {
            break;
        }
        assert!(
            status.as_deref() != Some("completed") && status.as_deref() != Some("failed"),
            "run must stay in flight until the test releases it (last: {status:?})",
        );
        assert!(
            std::time::Instant::now() < deadline,
            "detached webhook run never started executing (last: {status:?})",
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    // Release the gate; only now may the run finish.
    fs::write(&release_path, b"go").expect("release run");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let status = store
            .read_run_bundle(&run_id)
            .expect("bundle")
            .and_then(|bundle| bundle.record)
            .map(|record| record.status);
        if status.as_deref() == Some("completed") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "detached webhook run never completed (last: {status:?})",
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn trigger_revisions_reject_missing_and_stale_http_writes() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    install_run_flow(&settings);
    let project = project(&settings, "project");
    let app = build_app(settings.clone()).expect("app");
    let created = request_json(
        app.clone(),
        "POST",
        "/workspace/api/triggers",
        Some(json!({
            "project_path": project,
            "name": "Original", "source_type": "schedule",
            "action": {"flow_name": RUN_FLOW},
            "source": {"kind": "interval", "interval_seconds": 60}
        })),
    )
    .await;
    assert_eq!(created.0, StatusCode::OK);
    let url = format!(
        "/workspace/api/triggers/{}?project_path={project}",
        created.1["id"].as_str().unwrap()
    );
    let revision = created.1["revision"].as_str().unwrap();
    let missing = request_json(app.clone(), "PATCH", &url, Some(json!({"name": "Missing"}))).await;
    assert_eq!(missing.0, StatusCode::BAD_REQUEST);
    let missing_delete = request_json(app.clone(), "DELETE", &url, None).await;
    assert_eq!(missing_delete.0, StatusCode::BAD_REQUEST);
    let saved = request_json(
        app.clone(),
        "PATCH",
        &url,
        Some(json!({"expected_revision": revision, "name": "Saved"})),
    )
    .await;
    assert_eq!(saved.0, StatusCode::OK);
    assert_ne!(saved.1["revision"], revision);
    let stale = request_json(
        app.clone(),
        "PATCH",
        &url,
        Some(json!({"expected_revision": revision, "name": "Stale"})),
    )
    .await;
    assert_eq!(stale.0, StatusCode::CONFLICT);
    let stale_delete = request_json(
        app.clone(),
        "DELETE",
        &format!("{url}&expected_revision={revision}"),
        None,
    )
    .await;
    assert_eq!(stale_delete.0, StatusCode::CONFLICT);
    let loaded = request_json(app, "GET", &url, None).await;
    assert_eq!(loaded.1["name"], "Saved");
    assert_eq!(loaded.1["revision"], saved.1["revision"]);
}

async fn fire_webhook(app: axum::Router, key: &str, secret: &str) -> (StatusCode, Value) {
    request_json_with_headers(
        app,
        "POST",
        "/workspace/api/webhooks",
        &[
            ("X-Spark-Webhook-Key", key),
            ("X-Spark-Webhook-Secret", secret),
        ],
        Some(json!({})),
    )
    .await
}

/// An API edit of a trigger's source or action disarms its saved state at
/// once: reverting the edit, before any background reconciliation, does not
/// re-arm it, and webhooks are refused until it is activated again. A
/// descriptive edit keeps it armed.
#[tokio::test]
async fn an_operational_edit_disarms_until_activated_even_once_reverted() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(&temp.path().canonicalize().unwrap());
    install_run_flow(&settings);
    let project = project(&settings, "project");
    let app = build_app(settings.clone()).expect("app");
    let action =
        |origin: &str| json!({"flow_name": RUN_FLOW, "static_context": {"origin": origin}});
    let created = request_json(
        app.clone(),
        "POST",
        "/workspace/api/triggers",
        Some(json!({
            "project_path": project, "name": "Hook", "enabled": true,
            "source_type": "webhook", "action": action("first"), "source": {}
        })),
    )
    .await;
    assert_eq!(created.0, StatusCode::OK, "{}", created.1);
    assert_eq!(created.1["enabled"], true);
    let id = created.1["id"].as_str().unwrap().to_string();
    let key = created.1["source"]["webhook_key"]
        .as_str()
        .unwrap()
        .to_string();
    let secret = created.1["webhook_secret"].as_str().unwrap().to_string();
    let url = format!("/workspace/api/triggers/{id}?project_path={project}");
    let patch = |body: Value| request_json(app.clone(), "PATCH", &url, Some(body));

    let renamed =
        patch(json!({"expected_revision": created.1["revision"], "name": "Renamed"})).await;
    assert_eq!(renamed.0, StatusCode::OK, "{}", renamed.1);
    assert_eq!(renamed.1["enabled"], true);

    let edited =
        patch(json!({"expected_revision": renamed.1["revision"], "action": action("second")}))
            .await;
    assert_eq!(edited.0, StatusCode::OK, "{}", edited.1);
    assert_eq!(edited.1["enabled"], false);
    assert_eq!(
        edited.1["state"]["activation_error"],
        spark_storage::DEFINITION_CHANGED
    );

    let reverted =
        patch(json!({"expected_revision": edited.1["revision"], "action": action("first")})).await;
    assert_eq!(reverted.0, StatusCode::OK, "{}", reverted.1);
    assert_eq!(reverted.1["enabled"], false);
    assert!(!load_state(&settings, &project, &id).enabled);
    let refused = fire_webhook(app.clone(), &key, &secret).await;
    assert_eq!(refused.0, StatusCode::BAD_REQUEST);
    assert_eq!(refused.1["detail"], "Webhook trigger is disabled.");
    assert!(load_state(&settings, &project, &id).last_fired_at.is_none());

    let activated =
        patch(json!({"expected_revision": reverted.1["revision"], "enabled": true})).await;
    assert_eq!(activated.0, StatusCode::OK, "{}", activated.1);
    assert_eq!(activated.1["enabled"], true);
    assert_eq!(fire_webhook(app, &key, &secret).await.0, StatusCode::OK);
}

/// Trigger creation racing project removal never leaves an activation or
/// credentials behind: each create either precedes the removal, which
/// deletes its row, or fails. Re-registration finds the definitions left in
/// the project inactive.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn trigger_creation_racing_project_removal_leaves_no_activation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(&temp.path().canonicalize().unwrap());
    install_run_flow(&settings);
    let project = project(&settings, "project");
    let app = build_app(settings.clone()).expect("app");
    let mut creates = (0..16)
        .map(|index| {
            let (app, project) = (app.clone(), project.clone());
            tokio::spawn(async move {
                request_json(
                    app,
                    "POST",
                    "/workspace/api/triggers",
                    Some(json!({
                        "project_path": project, "name": format!("Hook {index}"),
                        "enabled": true, "source_type": "webhook",
                        "action": {"flow_name": RUN_FLOW}, "source": {}
                    })),
                )
                .await
            })
        })
        .collect::<Vec<_>>();
    // The removal starts once one create has finished, while the rest run.
    let first = creates.remove(0).await.unwrap();
    assert_eq!(first.0, StatusCode::OK, "{}", first.1);
    let removal = format!(
        "/workspace/api/projects?project_path={}",
        crate::encode_query(&project)
    );
    let removed = request_json(app.clone(), "DELETE", &removal, None).await;
    assert_eq!(removed.0, StatusCode::OK, "{}", removed.1);
    let mut keys = vec![first.1["source"]["webhook_key"]
        .as_str()
        .unwrap()
        .to_string()];
    for create in creates {
        let (status, body) = create.await.unwrap();
        if status == StatusCode::OK {
            keys.push(body["source"]["webhook_key"].as_str().unwrap().to_string());
        } else {
            assert!(status.is_client_error(), "{status} {body}");
        }
    }
    let repositories = TriggerRepositories::from_settings(&settings);
    let states = repositories.runtime_state(&project).unwrap();
    let ids = repositories
        .definitions(&project)
        .unwrap()
        .definition_names()
        .unwrap();
    for id in &ids {
        assert_eq!(states.get(id).unwrap(), None, "{id}");
    }
    for key in &keys {
        assert_eq!(repositories.find_webhook_key(key).unwrap(), None);
    }

    let registered = request_json(
        app.clone(),
        "POST",
        "/workspace/api/projects/register",
        Some(json!({"project_path": project})),
    )
    .await;
    assert_eq!(registered.0, StatusCode::OK, "{}", registered.1);
    let listed = request_json(
        app,
        "GET",
        &format!("/workspace/api/triggers?project_path={project}"),
        None,
    )
    .await;
    assert_eq!(listed.1.as_array().unwrap().len(), ids.len());
    for trigger in listed.1.as_array().unwrap() {
        assert_eq!(trigger["enabled"], false, "{trigger}");
        assert!(trigger["source"].get("webhook_key").is_none(), "{trigger}");
    }
}

/// A definition broken after startup, and an invalid one added after it,
/// are reported (by the issues route and by each background tick), cannot
/// fire, and keep their activation until their removal is confirmed.
#[tokio::test]
async fn definitions_broken_after_startup_are_reported_and_cannot_fire() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(&temp.path().canonicalize().unwrap());
    install_run_flow(&settings);
    let project = project(&settings, "project");
    let app = crate::store_app_default(settings.clone());
    let created = request_json(
        app.clone(),
        "POST",
        "/workspace/api/triggers",
        Some(json!({
            "project_path": project, "name": "Hook", "enabled": true,
            "source_type": "webhook", "action": {"flow_name": RUN_FLOW}, "source": {}
        })),
    )
    .await;
    assert_eq!(created.1["enabled"], true, "{}", created.1);
    let id = created.1["id"].as_str().unwrap().to_string();
    let key = created.1["source"]["webhook_key"]
        .as_str()
        .unwrap()
        .to_string();
    let secret = created.1["webhook_secret"].as_str().unwrap().to_string();
    let issues = || request_json(app.clone(), "GET", "/workspace/api/trigger-issues", None);
    assert_eq!(issues().await, (StatusCode::OK, json!([])));

    let dir = spark_common::project::definitions_dir(Path::new(&project)).join("triggers");
    fs::write(dir.join(format!("{id}.toml")), "not = [toml").unwrap();
    write_persisted_trigger_toml(&project, "trigger-added", "unknown", RUN_FLOW, "");
    let reported = issues().await;
    assert_eq!(reported.0, StatusCode::OK);
    let reported_ids = reported
        .1
        .as_array()
        .unwrap()
        .iter()
        .map(|issue| {
            assert_eq!(issue["project_path"], project.as_str());
            assert!(!issue["error"].as_str().unwrap().is_empty());
            issue["trigger_id"].as_str().unwrap().to_string()
        })
        .collect::<Vec<_>>();
    let mut expected = vec![id.clone(), "trigger-added".to_string()];
    expected.sort();
    assert_eq!(reported_ids, expected);
    let triggers = spark_workspace::WorkspaceTriggerService::new(
        settings.clone(),
        crate::shared_store(&settings.data_dir),
    );
    let tick = triggers.process_due_trigger_sources().await.unwrap();
    assert_eq!(tick.1.len(), 2, "{:?}", tick.1);
    assert_eq!(
        request_json(app.clone(), "GET", "/workspace/api/triggers", None)
            .await
            .1,
        json!([])
    );
    assert_ne!(
        fire_webhook(app.clone(), &key, &secret).await.0,
        StatusCode::OK
    );
    let kept = load_state(&settings, &project, &id);
    assert!(kept.enabled && kept.last_fired_at.is_none());

    // Confirmed removal deletes the activation and clears the report.
    fs::remove_file(dir.join(format!("{id}.toml"))).unwrap();
    fs::remove_file(dir.join("trigger-added.toml")).unwrap();
    assert!(triggers
        .process_due_trigger_sources()
        .await
        .unwrap()
        .1
        .is_empty());
    assert_eq!(
        TriggerRepositories::from_settings(&settings)
            .runtime_state(&project)
            .unwrap()
            .get(&id)
            .unwrap(),
        None
    );
    assert_eq!(issues().await, (StatusCode::OK, json!([])));
}
