use std::fs;
use std::path::Path;

use attractor_runtime::RunStore;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::{json, Map};
use spark_common::settings::SparkSettings;
use spark_http::build_app;
use spark_storage::{
    read_trigger_definition, TriggerAction, TriggerDefinition, TriggerState,
    TriggerStateHistoryEntry,
};
use tower::ServiceExt;

#[tokio::test]
async fn trigger_crud_routes_persist_definition_and_state_contracts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings, "ops/run.yaml");
    let project_dir = temp.path().join("project");
    fs::create_dir_all(&project_dir).expect("project");
    let app = build_app(settings.clone());

    let empty = request_json(app.clone(), "GET", "/workspace/api/triggers", None).await;
    assert_eq!(empty.0, StatusCode::OK);
    assert_eq!(empty.1, json!([]));

    let created = request_json(
        app.clone(),
        "POST",
        "/workspace/api/triggers",
        Some(json!({
            "name": "Compat webhook",
            "source_type": "webhook",
            "action": {
                "flow_name": "ops/run.yaml",
                "project_path": project_dir,
                "static_context": {"origin": "compat"}
            },
            "source": {}
        })),
    )
    .await;
    assert_eq!(created.0, StatusCode::OK);
    assert_eq!(created.1["enabled"], json!(true));
    assert_eq!(created.1["source_type"], "webhook");
    assert!(created.1["source"]["webhook_key"].as_str().unwrap().len() == 16);
    assert!(created.1["webhook_secret"].as_str().unwrap().len() == 32);
    assert!(created.1["source"].get("secret_hash").is_none());
    let trigger_id = created.1["id"].as_str().unwrap().to_string();

    let listed = request_json(app.clone(), "GET", "/workspace/api/triggers", None).await;
    assert_eq!(listed.0, StatusCode::OK);
    assert_eq!(listed.1.as_array().expect("trigger list").len(), 1);
    assert_eq!(listed.1[0]["id"], trigger_id);
    assert_eq!(listed.1[0]["name"], "Compat webhook");
    assert!(listed.1[0].get("webhook_secret").is_none());
    assert!(listed.1[0]["source"].get("secret_hash").is_none());

    let definition_path = settings
        .config_dir
        .join("triggers")
        .join(format!("{trigger_id}.toml"));
    assert!(definition_path.exists());
    let persisted = read_trigger_definition(&settings.config_dir, &trigger_id)
        .expect("read persisted trigger")
        .expect("persisted trigger");
    assert!(persisted.enabled);
    assert!(fs::read_to_string(&definition_path)
        .expect("definition toml")
        .contains("enabled = true"));
    assert!(settings
        .workspace_dir
        .join("trigger-state")
        .join(format!("{trigger_id}.json"))
        .exists());

    let described = request_json(
        app.clone(),
        "GET",
        &format!("/workspace/api/triggers/{trigger_id}"),
        None,
    )
    .await;
    assert_eq!(described.0, StatusCode::OK);
    assert!(described.1.get("webhook_secret").is_none());
    assert!(described.1["source"].get("secret_hash").is_none());

    let updated = request_json(
        app.clone(),
        "PATCH",
        &format!("/workspace/api/triggers/{trigger_id}"),
        Some(json!({"name": "Compat webhook updated", "regenerate_webhook_secret": true})),
    )
    .await;
    assert_eq!(updated.0, StatusCode::OK);
    assert_eq!(updated.1["name"], "Compat webhook updated");
    assert_eq!(
        updated.1["source"]["webhook_key"],
        described.1["source"]["webhook_key"]
    );
    assert!(updated.1["webhook_secret"].as_str().unwrap().len() == 32);

    let deleted = request_json(
        app,
        "DELETE",
        &format!("/workspace/api/triggers/{trigger_id}"),
        None,
    )
    .await;
    assert_eq!(deleted.0, StatusCode::OK);
    assert_eq!(deleted.1, json!({"status": "deleted", "id": trigger_id}));
    assert!(!settings
        .config_dir
        .join("triggers")
        .join(format!("{}.toml", deleted.1["id"].as_str().unwrap()))
        .exists());
    assert!(!settings
        .workspace_dir
        .join("trigger-state")
        .join(format!("{}.json", deleted.1["id"].as_str().unwrap()))
        .exists());
}

#[tokio::test]
async fn webhook_route_authenticates_storage_backed_trigger_and_dispatches_run() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings, "ops/run.yaml");
    let project_dir = temp.path().join("webhook-project");
    fs::create_dir_all(&project_dir).expect("project");
    let app = build_app(settings.clone());

    let created = request_json(
        app.clone(),
        "POST",
        "/workspace/api/triggers",
        Some(json!({
            "name": "Compat webhook",
            "source_type": "webhook",
            "action": {
                "flow_name": "ops/run.yaml",
                "project_path": project_dir,
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
    let state = spark_storage::load_trigger_state(&settings.data_dir, &trigger_id)
        .expect("load trigger state");
    assert_eq!(state.last_result.as_deref(), Some("success"));
    let run_id = state.recent_history[0].run_id.as_deref().expect("run id");
    let run = RunStore::for_settings(&settings)
        .read_run_bundle(run_id)
        .expect("read webhook run")
        .expect("webhook run");
    let record = run.record.expect("run record");
    assert_eq!(record.flow_name, "ops/run.yaml");
    assert_eq!(record.project_path, project_dir.to_string_lossy());
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
    write_flow(&settings, "ops/run.yaml");
    write_flow(&settings, "ops/other.yaml");
    spark_storage::write_trigger_definition(
        &settings.config_dir,
        &protected_definition("trigger-protected"),
    )
    .expect("protected");
    let app = build_app(settings.clone());

    let missing = request_json(app.clone(), "GET", "/workspace/api/triggers/missing", None).await;
    assert_eq!(missing.0, StatusCode::NOT_FOUND);
    assert_eq!(missing.1, json!({"detail": "Unknown trigger."}));

    let protected_delete = request_json(
        app.clone(),
        "DELETE",
        "/workspace/api/triggers/trigger-protected",
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
        "/workspace/api/triggers/trigger-protected",
        Some(json!({"action": {"static_context": {"changed": true}}})),
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
                "name": "bad",
                "enabled": true,
                "source_type": "webhook",
                "action": {"flow_name": "ops/run.yaml"},
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
            "name": "Unknown flow",
            "enabled": true,
            "source_type": "webhook",
            "action": {"flow_name": "missing.yaml"},
            "source": {}
        })),
    )
    .await;
    assert_eq!(unknown_flow.0, StatusCode::NOT_FOUND);
    assert_eq!(
        unknown_flow.1,
        json!({"detail": "Unknown flow: missing.yaml"})
    );

    assert!(
        read_trigger_definition(&settings.config_dir, "trigger-protected")
            .expect("read protected")
            .is_some()
    );
}

#[tokio::test]
async fn protected_trigger_delete_rejection_preserves_runtime_state_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings, "ops/run.yaml");
    spark_storage::write_trigger_definition(
        &settings.config_dir,
        &protected_definition("trigger-protected"),
    )
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
    };
    spark_storage::save_trigger_state(&settings.data_dir, "trigger-protected", &original_state)
        .expect("write trigger state");
    let definition_path = settings
        .config_dir
        .join("triggers")
        .join("trigger-protected.toml");
    let state_path = settings
        .workspace_dir
        .join("trigger-state")
        .join("trigger-protected.json");
    let original_definition_text = fs::read_to_string(&definition_path).expect("definition text");
    let original_state_value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&state_path).expect("state text"))
            .expect("state json");
    let app = build_app(settings.clone());

    let protected_delete = request_json(
        app.clone(),
        "DELETE",
        "/workspace/api/triggers/trigger-protected",
        None,
    )
    .await;
    assert_eq!(protected_delete.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        protected_delete.1,
        json!({"detail": "Protected triggers cannot be deleted."})
    );
    assert_eq!(
        fs::read_to_string(&definition_path).expect("definition unchanged"),
        original_definition_text
    );
    let state_after_delete: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&state_path).expect("state unchanged"))
            .expect("state json");
    assert_eq!(state_after_delete, original_state_value);

    fs::remove_file(&state_path).expect("remove state file");
    let protected_delete_without_state = request_json(
        app,
        "DELETE",
        "/workspace/api/triggers/trigger-protected",
        None,
    )
    .await;
    assert_eq!(protected_delete_without_state.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        protected_delete_without_state.1,
        json!({"detail": "Protected triggers cannot be deleted."})
    );
    assert!(!state_path.exists());
    assert_eq!(
        fs::read_to_string(&definition_path).expect("definition unchanged"),
        original_definition_text
    );
}

#[tokio::test]
async fn trigger_routes_reject_malformed_persisted_definitions_and_list_skips_them() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_persisted_trigger_toml(
        &settings,
        "trigger-missing-flow",
        "webhook",
        "",
        r#"webhook_key = "key"
secret_hash = "hash""#,
    );
    write_persisted_trigger_toml(
        &settings,
        "trigger-unknown-source",
        "unknown",
        "ops/run.yaml",
        r#"kind = "once"
run_at = "2026-06-23T10:00:00Z""#,
    );
    write_persisted_trigger_toml(
        &settings,
        "trigger-missing-secret",
        "webhook",
        "ops/run.yaml",
        r#"webhook_key = "key""#,
    );
    write_persisted_trigger_toml(
        &settings,
        "trigger-invalid-schedule",
        "schedule",
        "ops/run.yaml",
        r#"kind = "weekly"
weekdays = ["mon", "noday"]
hour = 9
minute = 0"#,
    );
    let app = build_app(settings);

    let listed = request_json(app.clone(), "GET", "/workspace/api/triggers", None).await;
    assert_eq!(listed.0, StatusCode::OK);
    assert_eq!(listed.1, json!([]));

    let missing_flow = request_json(
        app.clone(),
        "GET",
        "/workspace/api/triggers/trigger-missing-flow",
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
        "/workspace/api/triggers/trigger-unknown-source",
        Some(json!({"name": "Ignored"})),
    )
    .await;
    assert_eq!(unknown_source.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        unknown_source.1,
        json!({"detail": "Unsupported trigger source type: unknown"})
    );

    let missing_secret = request_json(
        app.clone(),
        "DELETE",
        "/workspace/api/triggers/trigger-missing-secret",
        None,
    )
    .await;
    assert_eq!(missing_secret.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        missing_secret.1,
        json!({"detail": "Webhook triggers require secret_hash."})
    );

    let invalid_schedule = request_json(
        app,
        "GET",
        "/workspace/api/triggers/trigger-invalid-schedule",
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
async fn poll_trigger_headers_use_python_stringification_contract() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    write_flow(&settings, "ops/run.yaml");
    let app = build_app(settings.clone());

    let created = request_json(
        app,
        "POST",
        "/workspace/api/triggers",
        Some(json!({
            "name": "Compat poll",
            "source_type": "poll",
            "action": {"flow_name": "ops/run.yaml"},
            "source": {
                "url": "https://example.test/items",
                "interval_seconds": 60,
                "items_path": "items",
                "item_id_path": "id",
                "headers": {
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

    assert_eq!(created.0, StatusCode::OK);
    assert_eq!(
        created.1["source"]["headers"],
        json!({
            "x-bool": "True",
            "x-false": "False",
            "x-null": "None",
            "x-number": "5",
            "x-text": "ok"
        })
    );
    let trigger_id = created.1["id"].as_str().expect("trigger id");
    let persisted = read_trigger_definition(&settings.config_dir, trigger_id)
        .expect("read persisted trigger")
        .expect("persisted trigger");
    assert_eq!(
        persisted.source["headers"],
        json!({
            "x-bool": "True",
            "x-false": "False",
            "x-null": "None",
            "x-number": "5",
            "x-text": "ok"
        })
    );
    let definition_text = fs::read_to_string(
        settings
            .config_dir
            .join("triggers")
            .join(format!("{trigger_id}.toml")),
    )
    .expect("definition toml");
    assert!(definition_text.contains("headers_json = "));
    assert!(definition_text.contains(r#"\"x-bool\": \"True\""#));
    assert!(definition_text.contains(r#"\"x-null\": \"None\""#));
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
        id: id.to_string(),
        name: "Protected".to_string(),
        enabled: true,
        protected: true,
        source_type: "webhook".to_string(),
        action: TriggerAction {
            flow_name: "ops/run.yaml".to_string(),
            project_path: Some("/tmp/project".to_string()),
            static_context: Map::from_iter([("origin".to_string(), json!("compat"))]),
        },
        source: Map::from_iter([
            ("webhook_key".to_string(), json!("protected-key")),
            ("secret_hash".to_string(), json!("protected-secret-hash")),
        ]),
        created_at: "2026-06-22T16:16:08Z".to_string(),
        updated_at: "2026-06-22T16:16:08Z".to_string(),
    }
}

fn write_flow(settings: &SparkSettings, name: &str) {
    let path = settings.flows_dir.join(name);
    fs::create_dir_all(path.parent().expect("flow parent")).expect("flow parent");
    fs::write(
        path,
        "schema_version: '1'\nid: trigger-route\ntitle: Trigger Route\nnodes:\n  start:\n    kind: start\n  done:\n    kind: exit\nedges:\n  - from: start\n    to: done\n",
    )
    .expect("flow");
}

fn write_persisted_trigger_toml(
    settings: &SparkSettings,
    id: &str,
    source_type: &str,
    flow_name: &str,
    source_body: &str,
) {
    let dir = settings.config_dir.join("triggers");
    fs::create_dir_all(&dir).expect("trigger config dir");
    fs::write(
        dir.join(format!("{id}.toml")),
        format!(
            r#"id = "{id}"
name = "Malformed"
enabled = true
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

#[tokio::test]
async fn webhook_dispatch_returns_while_the_run_still_executes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("slow-webhook-project");
    fs::create_dir_all(&project_path).expect("project");
    let project_path_text = project_path.to_string_lossy().to_string();
    let slow_flow_path = settings.flows_dir.join("ops/slow-webhook.yaml");
    fs::create_dir_all(slow_flow_path.parent().expect("flow parent")).expect("flow parent");
    fs::write(
        slow_flow_path,
        concat!(
            "schema_version: '1'\n",
            "id: slow_hook\n",
            "title: Slow Hook\n",
            "nodes:\n",
            "  start:\n",
            "    kind: start\n",
            "  work:\n",
            "    kind: tool\n",
            "    config:\n",
            "      kind: tool\n",
            "      command: sleep 1\n",
            "  done:\n",
            "    kind: exit\n",
            "edges:\n",
            "  - from: start\n",
            "    to: work\n",
            "  - from: work\n",
            "    to: done\n",
        ),
    )
    .expect("write slow flow");

    let app = spark_http::build_app(settings.clone());

    let created = request_json(
        app.clone(),
        "POST",
        "/workspace/api/triggers",
        Some(json!({
            "name": "Slow webhook",
            "source_type": "webhook",
            "action": {
                "flow_name": "ops/slow-webhook.yaml",
                "project_path": project_path_text,
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

    // The dispatch must return at launch, not at run completion: well under
    // the ~1s the single slow tool node takes.
    let dispatch_started = std::time::Instant::now();
    let accepted = request_json_with_headers(
        app,
        "POST",
        "/workspace/api/webhooks",
        &[
            ("X-Spark-Webhook-Key", webhook_key.as_str()),
            ("X-Spark-Webhook-Secret", webhook_secret.as_str()),
            ("X-Spark-Webhook-Request-Id", "slow-dispatch"),
        ],
        Some(json!({"payload": "slow"})),
    )
    .await;
    let dispatch_elapsed = dispatch_started.elapsed();
    assert_eq!(accepted.0, StatusCode::OK);
    assert!(
        dispatch_elapsed < std::time::Duration::from_millis(450),
        "webhook dispatch blocked on run execution: {dispatch_elapsed:?}",
    );

    let trigger_id = accepted.1["trigger_id"].as_str().expect("trigger id");
    let state =
        spark_storage::load_trigger_state(&settings.data_dir, trigger_id).expect("trigger state");
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

    // The run is still executing at dispatch time and completes on its own.
    let store = RunStore::for_settings(&settings);
    let running_now = store
        .read_run_bundle(&run_id)
        .expect("bundle")
        .and_then(|bundle| bundle.record)
        .map(|record| record.status);
    assert_eq!(running_now.as_deref(), Some("running"));

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
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
