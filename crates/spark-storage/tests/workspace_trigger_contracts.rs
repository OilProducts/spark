use serde_json::{json, Map, Value};
use spark_storage::{
    delete_trigger_definition, delete_trigger_state, load_trigger_state, read_trigger_definition,
    save_trigger_state, trigger_definition_path, trigger_state_path, write_trigger_definition,
    StorageError, TriggerAction, TriggerDefinition, TriggerDefinitionRepository,
    TriggerRuntimeStateRepository, TriggerState, TriggerStateHistoryEntry,
};

#[test]
fn trigger_definition_toml_round_trips_webhook_without_losing_secret_hash() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_dir = temp.path().join("spark-home/config");
    let definition = webhook_definition("trigger-storage");

    let path = write_trigger_definition(&config_dir, &definition).expect("write definition");
    let text = std::fs::read_to_string(&path).expect("definition toml");
    assert!(text.contains("[action]"));
    assert!(text.contains(r#"static_context_json = "{\"origin\": \"compat\"}""#));
    assert!(text.contains("secret_hash"));

    let loaded = read_trigger_definition(&config_dir, "trigger-storage")
        .expect("read definition")
        .expect("definition");
    assert_eq!(loaded, definition);
}

#[test]
fn schedule_poll_and_flow_event_sources_round_trip_source_shapes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_dir = temp.path().join("spark-home/config");
    for definition in [
        definition_with_source(
            "trigger-schedule",
            "schedule",
            Map::from_iter([
                ("kind".to_string(), json!("weekly")),
                ("weekdays".to_string(), json!(["mon", "fri"])),
                ("hour".to_string(), json!(9)),
                ("minute".to_string(), json!(30)),
            ]),
        ),
        definition_with_source(
            "trigger-poll",
            "poll",
            Map::from_iter([
                ("url".to_string(), json!("https://example.test/items")),
                ("interval_seconds".to_string(), json!(60)),
                ("items_path".to_string(), json!("items")),
                ("item_id_path".to_string(), json!("id")),
                ("headers".to_string(), json!({"x-test": "1"})),
            ]),
        ),
        definition_with_source(
            "trigger-flow-event",
            "flow_event",
            Map::from_iter([
                ("flow_name".to_string(), json!("upstream.dot")),
                ("statuses".to_string(), json!(["completed", "failed"])),
            ]),
        ),
    ] {
        write_trigger_definition(&config_dir, &definition).expect("write definition");
        assert_eq!(
            read_trigger_definition(&config_dir, &definition.id)
                .expect("read")
                .expect("definition"),
            definition
        );
    }
}

#[test]
fn trigger_definition_and_route_state_delete_only_their_canonical_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data_dir = temp.path().join("spark-home");
    let config_dir = data_dir.join("config");
    let definition = webhook_definition("trigger-delete");
    write_trigger_definition(&config_dir, &definition).expect("write definition");
    save_trigger_state(&data_dir, &definition.id, &TriggerState::default()).expect("write state");
    let definition_path = trigger_definition_path(&config_dir, &definition.id).expect("def path");
    let state_path = trigger_state_path(&data_dir, &definition.id).expect("state path");
    assert!(definition_path.exists());
    assert!(state_path.exists());

    delete_trigger_definition(&config_dir, &definition.id).expect("delete definition");
    assert!(!definition_path.exists());
    assert!(state_path.exists());

    delete_trigger_state(&data_dir, &definition.id).expect("delete state");
    assert!(!state_path.exists());
    assert_eq!(
        load_trigger_state(&data_dir, &definition.id).expect("missing state"),
        TriggerState::default()
    );
}

#[test]
fn trigger_definition_repository_uses_canonical_paths_and_keeps_list_compatible() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_dir = temp.path().join("spark-home/config");
    let repository = TriggerDefinitionRepository::new(config_dir.clone());
    let first = webhook_definition("trigger-a");
    let second = definition_with_source(
        "trigger-b",
        "schedule",
        Map::from_iter([
            ("kind".to_string(), json!("once")),
            ("run_at".to_string(), json!("2026-06-23T10:00:00Z")),
        ]),
    );

    assert_eq!(
        repository.root_dir().expect("root"),
        config_dir.join("triggers")
    );
    assert_eq!(
        repository
            .definition_path("trigger-a")
            .expect("definition path"),
        config_dir.join("triggers/trigger-a.toml")
    );
    repository.put(&second).expect("write second");
    repository.put(&first).expect("write first");
    std::fs::write(
        repository
            .root_dir()
            .expect("root")
            .join("trigger-broken.toml"),
        "not = [",
    )
    .expect("write invalid toml");

    let listed = repository
        .list()
        .expect("list")
        .into_iter()
        .map(|definition| definition.id)
        .collect::<Vec<_>>();
    assert_eq!(listed, vec!["trigger-a", "trigger-b"]);
    assert_eq!(
        repository
            .get("trigger-a")
            .expect("get")
            .expect("definition"),
        first
    );

    repository.delete("trigger-a").expect("delete");
    assert!(repository.get("trigger-a").expect("missing").is_none());
    assert!(matches!(
        repository.definition_path("../bad"),
        Err(StorageError::InvalidRepositoryPath { .. })
    ));
}

#[test]
fn persisted_trigger_definitions_are_validated_and_malformed_files_are_skipped_by_list() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_dir = temp.path().join("spark-home/config");
    let repository = TriggerDefinitionRepository::new(config_dir.clone());
    repository
        .put(&webhook_definition("trigger-valid"))
        .expect("write valid");

    for (trigger_id, toml, expected) in [
        (
            "trigger-missing-flow",
            persisted_trigger_toml(
                "trigger-missing-flow",
                "webhook",
                "",
                r#"webhook_key = "key"
secret_hash = "hash""#,
            ),
            "Trigger action requires a flow_name.",
        ),
        (
            "trigger-unknown-source",
            persisted_trigger_toml(
                "trigger-unknown-source",
                "unknown",
                "ops/run.dot",
                r#"kind = "once"
run_at = "2026-06-23T10:00:00Z""#,
            ),
            "Unsupported trigger source type: unknown",
        ),
        (
            "trigger-missing-secret",
            persisted_trigger_toml(
                "trigger-missing-secret",
                "webhook",
                "ops/run.dot",
                r#"webhook_key = "key""#,
            ),
            "Webhook triggers require secret_hash.",
        ),
        (
            "trigger-invalid-schedule",
            persisted_trigger_toml(
                "trigger-invalid-schedule",
                "schedule",
                "ops/run.dot",
                r#"kind = "weekly"
weekdays = ["mon", "noday"]
hour = 9
minute = 0"#,
            ),
            "Weekly schedule triggers require weekdays using mon..sun.",
        ),
    ] {
        let path = repository
            .definition_path(trigger_id)
            .expect("definition path");
        std::fs::write(path, toml).expect("write malformed definition");
        assert_invalid_reason(repository.get(trigger_id), expected);
    }

    let listed = repository
        .list()
        .expect("list")
        .into_iter()
        .map(|definition| definition.id)
        .collect::<Vec<_>>();
    assert_eq!(listed, vec!["trigger-valid"]);
}

#[test]
fn trigger_runtime_state_repository_defaults_saves_updates_and_validates_ids() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data_dir = temp.path().join("spark-home");
    let repository = TriggerRuntimeStateRepository::new(data_dir.clone());

    assert_eq!(
        repository.root_dir().expect("root"),
        data_dir.join("workspace/trigger-state")
    );
    assert_eq!(
        repository.state_path("trigger-state").expect("state path"),
        data_dir.join("workspace/trigger-state/trigger-state.json")
    );
    assert_eq!(
        repository.load("trigger-state").expect("missing state"),
        TriggerState::default()
    );

    let saved = TriggerState {
        last_result: Some("queued".to_string()),
        ..TriggerState::default()
    };
    repository
        .save("trigger-state", &saved)
        .expect("save state");
    let updated = repository
        .update("trigger-state", |state| {
            state.last_error = Some("transient".to_string());
            state.recent_history.push(TriggerStateHistoryEntry {
                timestamp: "2026-06-23T10:00:00Z".to_string(),
                status: "failed".to_string(),
                message: "transient".to_string(),
                run_id: None,
            });
        })
        .expect("update state");
    assert_eq!(updated.last_result.as_deref(), Some("queued"));
    assert_eq!(updated.last_error.as_deref(), Some("transient"));
    assert_eq!(
        repository.load("trigger-state").expect("load updated"),
        updated
    );

    let partial_path = repository
        .state_path("trigger-partial")
        .expect("partial path");
    std::fs::write(&partial_path, r#"{"last_result":"completed"}"#).expect("write partial");
    let partial = repository.load("trigger-partial").expect("load partial");
    assert_eq!(partial.last_result.as_deref(), Some("completed"));
    assert!(partial.recent_history.is_empty());

    assert!(matches!(
        repository.state_path("bad/name"),
        Err(StorageError::InvalidRepositoryPath { .. })
    ));
    repository.delete("trigger-state").expect("delete state");
    assert_eq!(
        repository.load("trigger-state").expect("deleted state"),
        TriggerState::default()
    );
}

fn webhook_definition(id: &str) -> TriggerDefinition {
    definition_with_source(
        id,
        "webhook",
        Map::from_iter([
            ("webhook_key".to_string(), json!("webhook-key")),
            ("secret_hash".to_string(), json!("secret-hash")),
        ]),
    )
}

fn definition_with_source(
    id: &str,
    source_type: &str,
    source: Map<String, Value>,
) -> TriggerDefinition {
    TriggerDefinition {
        id: id.to_string(),
        name: "Storage trigger".to_string(),
        enabled: true,
        protected: false,
        source_type: source_type.to_string(),
        action: TriggerAction {
            flow_name: "ops/run.dot".to_string(),
            project_path: Some("/tmp/project".to_string()),
            static_context: Map::from_iter([("origin".to_string(), json!("compat"))]),
        },
        source,
        created_at: "2026-06-22T16:16:08Z".to_string(),
        updated_at: "2026-06-22T16:16:08Z".to_string(),
    }
}

fn persisted_trigger_toml(
    id: &str,
    source_type: &str,
    flow_name: &str,
    source_body: &str,
) -> String {
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
    )
}

fn assert_invalid_reason(result: spark_storage::Result<Option<TriggerDefinition>>, expected: &str) {
    match result {
        Err(StorageError::InvalidRepositoryPath { reason, .. }) => assert_eq!(reason, expected),
        Ok(_) => panic!("expected invalid definition {expected:?}, got success"),
        Err(error) => panic!("expected invalid definition {expected:?}, got {error}"),
    }
}
