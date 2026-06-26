use std::path::Path;

use serde_json::{json, Map, Value};
use spark_common::settings::SparkSettings;
use spark_storage::{
    load_trigger_state, read_trigger_definition, save_trigger_state, TriggerAction,
    TriggerDefinition, TriggerRepositories, TriggerRuntimeStateRepository, TriggerState,
};
use spark_triggers::{
    state, TriggerCreateRequest, TriggerError, TriggerService, TriggerUpdateRequest,
    WebhookHandleRequest,
};

#[test]
fn create_list_webhook_generates_credentials_and_hides_secret_hash() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let created = TriggerService::new(settings.clone())
        .create_trigger(webhook_create_request())
        .expect("create webhook");

    assert!(created.id.starts_with("trigger-"));
    assert_eq!(
        created.source["webhook_key"]
            .as_str()
            .expect("webhook key")
            .len(),
        16
    );
    assert_eq!(created.webhook_secret.as_deref().expect("secret").len(), 32);
    assert!(created.source.get("secret_hash").is_none());
    assert!(settings
        .workspace_dir
        .join("trigger-state")
        .join(format!("{}.json", created.id))
        .exists());

    let persisted = read_trigger_definition(&settings.config_dir, &created.id)
        .expect("read")
        .expect("definition");
    assert!(
        persisted.source["secret_hash"]
            .as_str()
            .expect("hash")
            .len()
            == 64
    );
    let listed = TriggerService::new(settings)
        .list_triggers()
        .expect("list triggers");
    assert_eq!(listed[0].id, created.id);
    assert!(listed[0].webhook_secret.is_none());
}

#[test]
fn schedule_create_computes_next_run_at() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let created = TriggerService::new(settings)
        .create_trigger(TriggerCreateRequest {
            name: "Schedule".to_string(),
            enabled: true,
            source_type: "schedule".to_string(),
            action: action(),
            source: Map::from_iter([
                ("kind".to_string(), json!("interval")),
                ("interval_seconds".to_string(), json!(300)),
            ]),
        })
        .expect("create schedule");

    assert!(created
        .state
        .next_run_at
        .as_deref()
        .unwrap_or("")
        .ends_with('Z'));
}

#[test]
fn service_crud_uses_production_repositories_for_definition_and_state_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let repositories = TriggerRepositories::from_settings(&settings);
    let service = TriggerService::with_repositories(repositories.clone());

    let created = service
        .create_trigger(TriggerCreateRequest {
            name: "Schedule".to_string(),
            enabled: true,
            source_type: "schedule".to_string(),
            action: action(),
            source: Map::from_iter([
                ("kind".to_string(), json!("interval")),
                ("interval_seconds".to_string(), json!(60)),
            ]),
        })
        .expect("create trigger");
    let definition_path = repositories
        .definitions
        .definition_path(&created.id)
        .expect("definition path");
    let state_path = repositories
        .runtime_state
        .state_path(&created.id)
        .expect("state path");
    assert!(definition_path.exists());
    assert!(state_path.exists());
    assert_eq!(
        repositories
            .definitions
            .get(&created.id)
            .expect("read definition")
            .expect("definition")
            .name,
        "Schedule"
    );

    service
        .update_trigger(
            &created.id,
            TriggerUpdateRequest {
                name: Some("Schedule disabled".to_string()),
                enabled: Some(false),
                ..TriggerUpdateRequest::default()
            },
        )
        .expect("update trigger");
    let updated = repositories
        .definitions
        .get(&created.id)
        .expect("read updated")
        .expect("definition");
    assert_eq!(updated.name, "Schedule disabled");
    assert!(!updated.enabled);
    assert!(repositories
        .runtime_state
        .load(&created.id)
        .expect("read state")
        .next_run_at
        .as_deref()
        .unwrap_or("")
        .ends_with('Z'));

    service.delete_trigger(&created.id).expect("delete trigger");
    assert!(repositories
        .definitions
        .get(&created.id)
        .expect("read deleted")
        .is_none());
    assert!(!state_path.exists());
}

#[test]
fn validation_errors_cover_source_action_and_static_context_contracts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = TriggerService::new(settings(temp.path()));
    assert_validation(
        service.create_trigger(TriggerCreateRequest {
            name: " ".to_string(),
            ..webhook_create_request()
        }),
        "Trigger name is required.",
    );
    assert_validation(
        service.create_trigger(TriggerCreateRequest {
            source_type: "unknown".to_string(),
            ..webhook_create_request()
        }),
        "Unsupported trigger source type: unknown",
    );
    assert_validation(
        service.create_trigger(TriggerCreateRequest {
            action: Map::new(),
            ..webhook_create_request()
        }),
        "Trigger action requires a flow_name.",
    );
    assert_validation(
        service.create_trigger(TriggerCreateRequest {
            source_type: "poll".to_string(),
            source: Map::from_iter([("url".to_string(), json!("ftp://bad"))]),
            ..webhook_create_request()
        }),
        "Poll triggers require an http(s) url.",
    );
    assert_validation(
        service.create_trigger(TriggerCreateRequest {
            action: Map::from_iter([
                ("flow_name".to_string(), json!("ops/run.dot")),
                ("static_context".to_string(), json!(["not-object"])),
            ]),
            ..webhook_create_request()
        }),
        "Trigger action static_context must be a JSON object.",
    );
    assert_validation(
        service.create_trigger(TriggerCreateRequest {
            source_type: "flow_event".to_string(),
            source: Map::from_iter([("statuses".to_string(), json!(["running"]))]),
            ..webhook_create_request()
        }),
        "Flow-event triggers require terminal statuses when statuses are provided.",
    );
}

#[test]
fn update_regenerates_webhook_secret_and_preserves_immutable_fields() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = TriggerService::new(settings.clone());
    let created = service
        .create_trigger(webhook_create_request())
        .expect("create webhook");
    let original_key = created.source["webhook_key"]
        .as_str()
        .expect("key")
        .to_string();
    let original_created_at = created.created_at.clone();

    let updated = service
        .update_trigger(
            &created.id,
            TriggerUpdateRequest {
                name: Some("Webhook updated".to_string()),
                regenerate_webhook_secret: true,
                ..TriggerUpdateRequest::default()
            },
        )
        .expect("update");

    assert_eq!(updated.id, created.id);
    assert_eq!(updated.created_at, original_created_at);
    assert_eq!(updated.name, "Webhook updated");
    assert_eq!(updated.source["webhook_key"], original_key);
    assert_ne!(updated.webhook_secret, created.webhook_secret);
    let persisted = read_trigger_definition(&settings.config_dir, &created.id)
        .expect("read")
        .expect("definition");
    assert_eq!(persisted.source["webhook_key"], original_key);
    assert!(
        persisted.source["secret_hash"]
            .as_str()
            .expect("hash")
            .len()
            == 64
    );
}

#[test]
fn webhook_handler_authenticates_by_stored_key_and_secret() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = TriggerService::new(settings.clone());
    let created = service
        .create_trigger(webhook_create_request())
        .expect("create webhook");
    let webhook_key = created.source["webhook_key"]
        .as_str()
        .expect("webhook key")
        .to_string();
    let webhook_secret = created.webhook_secret.expect("webhook secret");

    assert!(matches!(
        service.handle_webhook(webhook_request(
            "missing-key",
            &webhook_secret,
            json!({"payload": "compat"})
        )),
        Err(TriggerError::UnknownWebhookKey)
    ));
    assert!(matches!(
        service.handle_webhook(webhook_request(
            &webhook_key,
            "not-the-secret",
            json!({"payload": "compat"})
        )),
        Err(TriggerError::InvalidWebhookSecret)
    ));

    let response = service
        .handle_webhook(webhook_request(
            &webhook_key,
            &webhook_secret,
            json!({"payload": "compat"}),
        ))
        .expect("accepted webhook");
    assert!(response.ok);
    assert_eq!(response.trigger_id, created.id);
}

#[test]
fn protected_triggers_reject_forbidden_edits_and_delete_but_allow_flow_name() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let original_definition = protected_definition("trigger-protected");
    let original_state = TriggerState {
        last_result: Some("existing".to_string()),
        ..TriggerState::default()
    };
    spark_storage::write_trigger_definition(&settings.config_dir, &original_definition)
        .expect("write protected");
    save_trigger_state(&settings.data_dir, "trigger-protected", &original_state)
        .expect("write protected state");
    let service = TriggerService::new(settings.clone());

    assert_validation(
        service.update_trigger(
            "trigger-protected",
            TriggerUpdateRequest {
                source: Some(Map::new()),
                ..TriggerUpdateRequest::default()
            },
        ),
        "Protected triggers do not allow source changes.",
    );
    assert_validation(
        service.update_trigger(
            "trigger-protected",
            TriggerUpdateRequest {
                action: Some(Map::from_iter([(
                    "static_context".to_string(),
                    json!({"changed": true}),
                )])),
                ..TriggerUpdateRequest::default()
            },
        ),
        "Protected triggers do not allow static context changes.",
    );
    assert_validation(
        service.update_trigger(
            "trigger-protected",
            TriggerUpdateRequest {
                action: Some(Map::from_iter([(
                    "project_path".to_string(),
                    json!("/tmp/other"),
                )])),
                ..TriggerUpdateRequest::default()
            },
        ),
        "Protected triggers do not allow project target changes.",
    );
    assert_validation(
        service.update_trigger(
            "trigger-protected",
            TriggerUpdateRequest {
                regenerate_webhook_secret: true,
                ..TriggerUpdateRequest::default()
            },
        ),
        "Protected triggers do not support webhook secret regeneration.",
    );
    assert!(matches!(
        service.delete_trigger("trigger-protected"),
        Err(TriggerError::ProtectedDelete)
    ));
    assert_eq!(
        read_trigger_definition(&settings.config_dir, "trigger-protected")
            .expect("read protected")
            .expect("definition"),
        original_definition
    );
    assert_eq!(
        load_trigger_state(&settings.data_dir, "trigger-protected").expect("read protected state"),
        original_state
    );

    let updated = service
        .update_trigger(
            "trigger-protected",
            TriggerUpdateRequest {
                action: Some(Map::from_iter([(
                    "flow_name".to_string(),
                    json!("ops/other.dot"),
                )])),
                ..TriggerUpdateRequest::default()
            },
        )
        .expect("allowed flow update");
    assert_eq!(updated.action.flow_name, "ops/other.dot");
}

#[test]
fn service_rejects_malformed_persisted_definitions_and_list_skips_them() {
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
        "trigger-missing-secret",
        "webhook",
        "ops/run.dot",
        r#"webhook_key = "key""#,
    );
    write_persisted_trigger_toml(
        &settings,
        "trigger-invalid-poll",
        "poll",
        "ops/run.dot",
        r#"url = "ftp://example.test/items"
interval_seconds = 60
items_path = "items"
item_id_path = "id""#,
    );
    write_persisted_trigger_toml(
        &settings,
        "trigger-unknown-source",
        "unknown",
        "ops/run.dot",
        r#"kind = "once"
run_at = "2026-06-23T10:00:00Z""#,
    );

    let service = TriggerService::new(settings);
    assert!(service.list_triggers().expect("list").is_empty());
    assert_validation(
        service.get_trigger("trigger-missing-flow"),
        "Trigger action requires a flow_name.",
    );
    assert_validation(
        service.update_trigger(
            "trigger-unknown-source",
            TriggerUpdateRequest {
                name: Some("ignored".to_string()),
                ..TriggerUpdateRequest::default()
            },
        ),
        "Unsupported trigger source type: unknown",
    );
    assert_validation(
        service.delete_trigger("trigger-missing-secret"),
        "Webhook triggers require secret_hash.",
    );
    assert_validation(
        service.get_trigger("trigger-invalid-poll"),
        "Poll triggers require an http(s) url.",
    );
}

#[test]
fn state_helpers_record_outcomes_and_bound_recent_history() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repository = TriggerRuntimeStateRepository::new(temp.path().join("spark-home"));
    let mut trigger_state = repository
        .load("trigger-state-history")
        .expect("load default state");

    for index in 0..25 {
        state::record_success(
            &mut trigger_state,
            format!("2026-06-23T10:{index:02}:00Z"),
            format!("run {index}"),
            Some(format!("run-{index}")),
        );
    }
    assert_eq!(
        trigger_state.recent_history.len(),
        state::MAX_RECENT_HISTORY_ENTRIES
    );
    assert_eq!(
        trigger_state.recent_history[0].timestamp,
        "2026-06-23T10:24:00Z"
    );
    assert_eq!(
        trigger_state
            .recent_history
            .first()
            .expect("newest history")
            .run_id
            .as_deref(),
        Some("run-24")
    );
    assert_eq!(
        trigger_state.recent_history[19].timestamp,
        "2026-06-23T10:05:00Z"
    );
    assert_eq!(trigger_state.last_error, None);
    assert_eq!(trigger_state.last_result.as_deref(), Some("success"));

    state::record_failure(&mut trigger_state, "2026-06-23T11:00:00Z", "webhook failed");
    repository
        .save("trigger-state-history", &trigger_state)
        .expect("save history state");
    let persisted = repository
        .load("trigger-state-history")
        .expect("load persisted history state");
    assert_eq!(persisted.recent_history.len(), 20);
    assert_eq!(trigger_state.last_error.as_deref(), Some("webhook failed"));
    assert_eq!(trigger_state.last_result.as_deref(), Some("failed"));
    assert_eq!(
        persisted
            .recent_history
            .first()
            .expect("newest history")
            .status,
        "failed"
    );
    assert_eq!(
        persisted
            .recent_history
            .first()
            .expect("newest history")
            .run_id
            .as_deref(),
        None
    );
    assert_eq!(
        persisted.recent_history[1].run_id.as_deref(),
        Some("run-24")
    );
    assert_eq!(
        persisted.recent_history[19].timestamp,
        "2026-06-23T10:06:00Z"
    );

    let state_path = repository
        .state_path("trigger-state-history")
        .expect("state path");
    let state_json = std::fs::read_to_string(state_path).expect("read history state json");
    let payload: Value = serde_json::from_str(&state_json).expect("parse history state json");
    assert_eq!(payload["recent_history"][0]["run_id"], Value::Null);
    assert_eq!(payload["recent_history"][1]["run_id"], json!("run-24"));
}

fn webhook_request(key: &str, secret: &str, payload: Value) -> WebhookHandleRequest {
    WebhookHandleRequest {
        webhook_key: key.to_string(),
        webhook_secret: secret.to_string(),
        request_id: Some("request-1".to_string()),
        payload: payload.as_object().cloned().unwrap_or_default(),
    }
}

fn assert_validation<T>(result: Result<T, TriggerError>, expected: &str) {
    match result {
        Err(TriggerError::Validation(message)) => assert_eq!(message, expected),
        Ok(_) => panic!("expected validation error {expected:?}, got success"),
        Err(error) => panic!("expected validation error {expected:?}, got {error}"),
    }
}

fn webhook_create_request() -> TriggerCreateRequest {
    TriggerCreateRequest {
        name: "Webhook".to_string(),
        enabled: true,
        source_type: "webhook".to_string(),
        action: action(),
        source: Map::new(),
    }
}

fn action() -> Map<String, Value> {
    Map::from_iter([
        ("flow_name".to_string(), json!("ops/run.dot")),
        ("project_path".to_string(), json!("/tmp/project")),
        ("static_context".to_string(), json!({"origin": "test"})),
    ])
}

fn protected_definition(id: &str) -> TriggerDefinition {
    TriggerDefinition {
        id: id.to_string(),
        name: "Protected".to_string(),
        enabled: true,
        protected: true,
        source_type: "webhook".to_string(),
        action: TriggerAction {
            flow_name: "ops/run.dot".to_string(),
            project_path: Some("/tmp/project".to_string()),
            static_context: Map::from_iter([("origin".to_string(), json!("test"))]),
        },
        source: Map::from_iter([
            ("webhook_key".to_string(), json!("protected-key")),
            ("secret_hash".to_string(), json!("protected-secret-hash")),
        ]),
        created_at: "2026-06-22T16:16:08Z".to_string(),
        updated_at: "2026-06-22T16:16:08Z".to_string(),
    }
}

fn write_persisted_trigger_toml(
    settings: &SparkSettings,
    id: &str,
    source_type: &str,
    flow_name: &str,
    source_body: &str,
) {
    let dir = settings.config_dir.join("triggers");
    std::fs::create_dir_all(&dir).expect("trigger config dir");
    std::fs::write(
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
