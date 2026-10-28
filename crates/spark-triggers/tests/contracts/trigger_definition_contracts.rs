use std::path::Path;

use serde_json::{json, Map, Value};
use spark_common::project::run_git;
use spark_common::settings::SparkSettings;
use spark_storage::{TriggerAction, TriggerDefinition, TriggerRepositories, TriggerState};
use spark_triggers::{
    state, TriggerCreateRequest, TriggerError, TriggerService, TriggerUpdateRequest,
    WebhookHandleRequest,
};

#[test]
fn create_list_webhook_generates_credentials_and_hides_secret_hash() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = project(&settings, "project");
    let created = TriggerService::new(settings.clone())
        .create_trigger(webhook_create_request(&project))
        .expect("create webhook");

    assert!(created.id.starts_with("trigger-"));
    assert_eq!(created.project_path, project);
    assert_eq!(
        created.source["webhook_key"]
            .as_str()
            .expect("webhook key")
            .len(),
        16
    );
    assert_eq!(created.webhook_secret.as_deref().expect("secret").len(), 32);
    assert!(created.source.get("secret_hash").is_none());
    assert!(created.state.secret_hash.is_none());

    // Credentials live in runtime state only, never in the org repo.
    let repositories = TriggerRepositories::from_settings(&settings);
    let persisted = repositories
        .definitions(&project)
        .unwrap()
        .get(&created.id)
        .expect("read")
        .expect("definition");
    assert!(persisted.source.is_empty());
    let org_file = std::fs::read_to_string(
        repositories
            .definitions(&project)
            .unwrap()
            .definition_path(&created.id)
            .unwrap(),
    )
    .unwrap();
    assert!(!org_file.contains("webhook_key") && !org_file.contains("secret_hash"));
    let state = repositories
        .runtime_state(&project)
        .unwrap()
        .load(&created.id)
        .expect("state");
    assert_eq!(
        state.webhook_key.as_deref(),
        created.source["webhook_key"].as_str()
    );
    assert_eq!(state.secret_hash.as_deref().expect("hash").len(), 64);

    let listed = TriggerService::new(settings)
        .list_triggers(None)
        .expect("list triggers");
    assert_eq!(listed[0].id, created.id);
    assert_eq!(
        listed[0].source["webhook_key"],
        created.source["webhook_key"]
    );
    assert!(listed[0].webhook_secret.is_none());
}

#[test]
fn schedule_create_computes_next_run_at() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = project(&settings, "project");
    let created = TriggerService::new(settings)
        .create_trigger(TriggerCreateRequest {
            project_path: project,
            message: None,
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
fn service_crud_commits_definitions_and_keeps_enabled_in_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = project(&settings, "project");
    let repositories = TriggerRepositories::from_settings(&settings);
    let service = TriggerService::with_repositories(repositories.clone());
    let definitions = repositories.definitions(&project).unwrap();
    let states = repositories.runtime_state(&project).unwrap();
    let org = spark_common::project::org_dir(Path::new(&project));
    let head = || run_git(&org, &["log", "-1", "--format=%H %s"]).unwrap();

    let created = service
        .create_trigger(TriggerCreateRequest {
            project_path: project.clone(),
            message: Some("Nightly digest for the team".into()),
            name: "Schedule".to_string(),
            enabled: false,
            source_type: "schedule".to_string(),
            action: action(),
            source: Map::from_iter([
                ("kind".to_string(), json!("interval")),
                ("interval_seconds".to_string(), json!(60)),
            ]),
        })
        .expect("create trigger");
    assert!(head().ends_with(" Nightly digest for the team"));
    let definition_path = definitions.definition_path(&created.id).unwrap();
    assert_eq!(
        definition_path,
        org.join(format!("triggers/{}.toml", created.id))
    );
    let state_path = states.state_path(&created.id).unwrap();
    assert!(state_path.exists());
    assert!(!created.enabled);
    assert!(!states.load(&created.id).unwrap().enabled);

    // Switching enabled writes state only: no definition change, no commit.
    let before = (head(), std::fs::read(&definition_path).unwrap());
    let enabled = service
        .update_trigger(
            &project,
            &created.id,
            TriggerUpdateRequest {
                expected_revision: created.revision.clone(),
                enabled: Some(true),
                ..TriggerUpdateRequest::default()
            },
        )
        .expect("enable trigger");
    assert!(enabled.enabled);
    assert_eq!(enabled.revision, created.revision);
    assert!(states.load(&created.id).unwrap().enabled);
    assert_eq!(before, (head(), std::fs::read(&definition_path).unwrap()));

    let renamed = service
        .update_trigger(
            &project,
            &created.id,
            TriggerUpdateRequest {
                expected_revision: enabled.revision,
                name: Some("Schedule renamed".to_string()),
                ..TriggerUpdateRequest::default()
            },
        )
        .expect("update trigger");
    assert!(head().ends_with(" Edit trigger Schedule renamed"));
    assert_eq!(
        definitions.get(&created.id).unwrap().unwrap().name,
        "Schedule renamed"
    );
    assert!(states
        .load(&created.id)
        .expect("read state")
        .next_run_at
        .as_deref()
        .unwrap_or("")
        .ends_with('Z'));

    service
        .delete_trigger(&project, &created.id, &renamed.revision, None)
        .expect("delete trigger");
    assert!(head().ends_with(" Delete trigger Schedule renamed"));
    assert!(definitions.get(&created.id).unwrap().is_none());
    assert!(!state_path.exists());
    assert_eq!(run_git(&org, &["status", "--porcelain"]).unwrap(), "");
}

#[test]
fn a_rejected_commit_creates_nothing_and_keeps_edits_and_deletes_undone() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = project(&settings, "project");
    let service = TriggerService::new(settings.clone());
    let created = service
        .create_trigger(webhook_create_request(&project))
        .expect("create");
    let org = spark_common::project::org_dir(Path::new(&project));
    let hook = org.join(".git/hooks/pre-commit");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    assert!(service
        .create_trigger(TriggerCreateRequest {
            enabled: false,
            name: "Never created".into(),
            ..webhook_create_request(&project)
        })
        .is_err());
    let listed = service.list_triggers(Some(&project)).unwrap();
    assert_eq!(listed.len(), 1, "the failed create is not discoverable");
    let states = TriggerRepositories::from_settings(&settings)
        .runtime_state(&project)
        .unwrap();
    assert_eq!(
        std::fs::read_dir(states.root_dir())
            .unwrap()
            .filter(|entry| entry
                .as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|ext| ext == "json"))
            .count(),
        1,
        "no state is left for the failed create"
    );

    let edit = TriggerUpdateRequest {
        expected_revision: created.revision.clone(),
        name: Some("Renamed".into()),
        enabled: Some(false),
        ..TriggerUpdateRequest::default()
    };
    assert!(service
        .update_trigger(&project, &created.id, edit.clone())
        .is_err());
    let current = service.get_trigger(&project, &created.id).unwrap().unwrap();
    assert_eq!((current.name.as_str(), current.enabled), ("Webhook", true));
    assert_eq!(current.revision, created.revision);
    assert!(service
        .delete_trigger(&project, &created.id, &created.revision, None)
        .is_err());
    assert!(service
        .get_trigger(&project, &created.id)
        .unwrap()
        .is_some());

    std::fs::remove_file(&hook).unwrap();
    let retried = service
        .update_trigger(&project, &created.id, edit)
        .expect("retry with the original revision");
    assert_eq!((retried.name.as_str(), retried.enabled), ("Renamed", false));
}

#[test]
fn a_refresh_overlapping_a_switch_off_and_rotation_cannot_undo_them() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = project(&settings, "project");
    let service = TriggerService::new(settings.clone());
    let created = service
        .create_trigger(webhook_create_request(&project))
        .expect("create");
    let states = TriggerRepositories::from_settings(&settings)
        .runtime_state(&project)
        .unwrap();
    let old_hash = states.load(&created.id).unwrap().secret_hash;
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let writer = {
        let states = states.clone();
        let id = created.id.clone();
        std::thread::spawn(move || {
            states
                .update(&id, |state| {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    state.enabled = false;
                    state.secret_hash = Some("rotated".into());
                })
                .unwrap();
        })
    };
    started_rx.recv().unwrap();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let reader = {
        let service = service.clone();
        let (project, id) = (project.clone(), created.id.clone());
        std::thread::spawn(move || {
            done_tx
                .send(service.get_trigger(&project, &id).unwrap().unwrap())
                .unwrap();
        })
    };
    assert!(
        done_rx
            .recv_timeout(std::time::Duration::from_millis(200))
            .is_err(),
        "the refresh waits for the writer"
    );
    release_tx.send(()).unwrap();
    writer.join().unwrap();
    assert!(!done_rx.recv().unwrap().enabled);
    reader.join().unwrap();
    let stored = states.load(&created.id).unwrap();
    assert!(!stored.enabled);
    assert_eq!(stored.secret_hash.as_deref(), Some("rotated"));
    assert_ne!(stored.secret_hash, old_hash);
}

#[test]
fn the_same_id_in_two_projects_gives_two_independent_triggers() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let first = project(&settings, "first");
    let second = project(&settings, "second");
    let repositories = TriggerRepositories::from_settings(&settings);
    for project in [&first, &second] {
        repositories
            .definitions(project)
            .unwrap()
            .put(&protected_definition("trigger-shared"), None)
            .expect("write");
    }
    let service = TriggerService::new(settings.clone());
    let revision = service
        .get_trigger(&first, "trigger-shared")
        .unwrap()
        .unwrap()
        .revision;
    service
        .update_trigger(
            &first,
            "trigger-shared",
            TriggerUpdateRequest {
                expected_revision: revision,
                name: Some("First only".into()),
                enabled: Some(false),
                ..TriggerUpdateRequest::default()
            },
        )
        .expect("update first");

    let listed = service.list_triggers(None).expect("list");
    assert_eq!(listed.len(), 2);
    let first_trigger = service
        .get_trigger(&first, "trigger-shared")
        .unwrap()
        .unwrap();
    let second_trigger = service
        .get_trigger(&second, "trigger-shared")
        .unwrap()
        .unwrap();
    assert_eq!(
        (first_trigger.name.as_str(), first_trigger.enabled),
        ("First only", false)
    );
    assert_eq!(
        (second_trigger.name.as_str(), second_trigger.enabled),
        ("Protected", true)
    );
    let filtered = service.list_triggers(Some(&second)).expect("filtered");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].project_path, second);
}

#[test]
fn validation_errors_cover_source_action_and_static_context_contracts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = project(&settings, "project");
    let service = TriggerService::new(settings);
    assert_validation(
        service.create_trigger(TriggerCreateRequest {
            name: " ".to_string(),
            ..webhook_create_request(&project)
        }),
        "Trigger name is required.",
    );
    assert_validation(
        service.create_trigger(TriggerCreateRequest {
            source_type: "unknown".to_string(),
            ..webhook_create_request(&project)
        }),
        "Unsupported trigger source type: unknown",
    );
    assert_validation(
        service.create_trigger(TriggerCreateRequest {
            action: Map::new(),
            ..webhook_create_request(&project)
        }),
        "Trigger action requires a flow_name.",
    );
    assert_validation(
        service.create_trigger(TriggerCreateRequest {
            source_type: "poll".to_string(),
            source: Map::from_iter([("url".to_string(), json!("ftp://bad"))]),
            ..webhook_create_request(&project)
        }),
        "Poll triggers require an http(s) url.",
    );
    assert_validation(
        service.create_trigger(TriggerCreateRequest {
            action: Map::from_iter([
                ("flow_name".to_string(), json!("ops/run.dot")),
                ("static_context".to_string(), json!(["not-object"])),
            ]),
            ..webhook_create_request(&project)
        }),
        "Trigger action static_context must be a JSON object.",
    );
    assert_validation(
        service.create_trigger(TriggerCreateRequest {
            source_type: "flow_event".to_string(),
            source: Map::from_iter([("statuses".to_string(), json!(["running"]))]),
            ..webhook_create_request(&project)
        }),
        "Flow-event triggers require terminal statuses when statuses are provided.",
    );
}

#[test]
fn update_regenerates_webhook_secret_and_preserves_immutable_fields() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = project(&settings, "project");
    let service = TriggerService::new(settings.clone());
    let created = service
        .create_trigger(webhook_create_request(&project))
        .expect("create webhook");
    let original_key = created.source["webhook_key"]
        .as_str()
        .expect("key")
        .to_string();
    let original_created_at = created.created_at.clone();

    let updated = service
        .update_trigger(
            &project,
            &created.id,
            TriggerUpdateRequest {
                expected_revision: created.revision.clone(),
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
    let state = TriggerRepositories::from_settings(&settings)
        .runtime_state(&project)
        .unwrap()
        .load(&created.id)
        .unwrap();
    assert_eq!(state.webhook_key.as_deref(), Some(original_key.as_str()));
    assert_eq!(state.secret_hash.as_deref().expect("hash").len(), 64);
}

#[test]
fn a_webhook_definition_without_state_credentials_has_no_key_until_regenerated() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = project(&settings, "project");
    let mut definition = protected_definition("trigger-copied");
    definition.protected = false;
    // As if copied from another project's org: the file has no credentials.
    TriggerRepositories::from_settings(&settings)
        .definitions(&project)
        .unwrap()
        .put(&definition, None)
        .unwrap();
    let service = TriggerService::new(settings);
    let copied = service
        .get_trigger(&project, "trigger-copied")
        .unwrap()
        .unwrap();
    assert!(copied.source.get("webhook_key").is_none());

    let regenerated = service
        .update_trigger(
            &project,
            "trigger-copied",
            TriggerUpdateRequest {
                expected_revision: copied.revision.clone(),
                regenerate_webhook_secret: true,
                ..TriggerUpdateRequest::default()
            },
        )
        .expect("regenerate");
    assert_eq!(regenerated.revision, copied.revision, "state only");
    let key = regenerated.source["webhook_key"].as_str().expect("key");
    let response = service
        .handle_webhook(webhook_request(
            key,
            regenerated.webhook_secret.as_deref().unwrap(),
            json!({}),
        ))
        .expect("webhook");
    assert_eq!(response.trigger_id, "trigger-copied");
}

#[test]
fn webhook_handler_authenticates_by_stored_key_and_secret() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project = project(&settings, "project");
    let service = TriggerService::new(settings.clone());
    let created = service
        .create_trigger(webhook_create_request(&project))
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
    let project = project(&settings, "project");
    let repositories = TriggerRepositories::from_settings(&settings);
    let definitions = repositories.definitions(&project).unwrap();
    let states = repositories.runtime_state(&project).unwrap();
    let original_state = TriggerState {
        last_result: Some("existing".to_string()),
        ..TriggerState::default()
    };
    definitions
        .put(&protected_definition("trigger-protected"), None)
        .expect("write protected");
    let original_definition = definitions.get("trigger-protected").unwrap().unwrap();
    states
        .save("trigger-protected", &original_state)
        .expect("write protected state");
    let service = TriggerService::new(settings.clone());

    assert_validation(
        service.update_trigger(
            &project,
            "trigger-protected",
            TriggerUpdateRequest {
                expected_revision: service
                    .get_trigger(&project, "trigger-protected")
                    .unwrap()
                    .unwrap()
                    .revision,
                source: Some(Map::new()),
                ..TriggerUpdateRequest::default()
            },
        ),
        "Protected triggers do not allow source changes.",
    );
    assert_validation(
        service.update_trigger(
            &project,
            "trigger-protected",
            TriggerUpdateRequest {
                expected_revision: service
                    .get_trigger(&project, "trigger-protected")
                    .unwrap()
                    .unwrap()
                    .revision,
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
            &project,
            "trigger-protected",
            TriggerUpdateRequest {
                expected_revision: service
                    .get_trigger(&project, "trigger-protected")
                    .unwrap()
                    .unwrap()
                    .revision,
                regenerate_webhook_secret: true,
                ..TriggerUpdateRequest::default()
            },
        ),
        "Protected triggers do not support webhook secret regeneration.",
    );
    assert!(matches!(
        service.delete_trigger(
            &project,
            "trigger-protected",
            &service
                .get_trigger(&project, "trigger-protected")
                .unwrap()
                .unwrap()
                .revision,
            None,
        ),
        Err(TriggerError::ProtectedDelete)
    ));
    assert_eq!(
        definitions
            .get("trigger-protected")
            .expect("read protected")
            .expect("definition"),
        original_definition
    );
    assert_eq!(
        states
            .load("trigger-protected")
            .expect("read protected state"),
        original_state
    );

    let updated = service
        .update_trigger(
            &project,
            "trigger-protected",
            TriggerUpdateRequest {
                expected_revision: service
                    .get_trigger(&project, "trigger-protected")
                    .unwrap()
                    .unwrap()
                    .revision,
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
        "trigger-bad-statuses",
        "flow_event",
        "ops/run.dot",
        r#"statuses = ["running"]"#,
    );
    write_persisted_trigger_toml(
        &project,
        "trigger-invalid-poll",
        "poll",
        "ops/run.dot",
        r#"url = "ftp://example.test/items"
interval_seconds = 60
items_path = "items"
item_id_path = "id""#,
    );
    write_persisted_trigger_toml(
        &project,
        "trigger-unknown-source",
        "unknown",
        "ops/run.dot",
        r#"kind = "once"
run_at = "2026-06-23T10:00:00Z""#,
    );

    let service = TriggerService::new(settings);
    assert!(service.list_triggers(None).expect("list").is_empty());
    assert_validation(
        service.get_trigger(&project, "trigger-missing-flow"),
        "Trigger action requires a flow_name.",
    );
    assert_validation(
        service.update_trigger(
            &project,
            "trigger-unknown-source",
            TriggerUpdateRequest {
                expected_revision: "invalid-document".into(),
                name: Some("ignored".to_string()),
                ..TriggerUpdateRequest::default()
            },
        ),
        "Unsupported trigger source type: unknown",
    );
    assert_validation(
        service.delete_trigger(&project, "trigger-bad-statuses", "invalid-document", None),
        "Flow-event triggers require terminal statuses when statuses are provided.",
    );
    assert_validation(
        service.get_trigger(&project, "trigger-invalid-poll"),
        "Poll triggers require an http(s) url.",
    );
}

#[test]
fn state_helpers_record_outcomes_and_bound_recent_history() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repository = TriggerRepositories::from_settings(&settings(temp.path()))
        .runtime_state("/spark-contract-fixture/project")
        .unwrap();
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

fn webhook_create_request(project: &str) -> TriggerCreateRequest {
    TriggerCreateRequest {
        project_path: project.to_string(),
        message: None,
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
        ("static_context".to_string(), json!({"origin": "test"})),
    ])
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
            flow_name: "ops/run.dot".to_string(),
            static_context: Map::from_iter([("origin".to_string(), json!("test"))]),
            execution_profile_id: None,
        },
        source: Map::new(),
        created_at: "2026-06-22T16:16:08Z".to_string(),
        updated_at: "2026-06-22T16:16:08Z".to_string(),
    }
}

fn write_persisted_trigger_toml(
    project: &str,
    id: &str,
    source_type: &str,
    flow_name: &str,
    source_body: &str,
) {
    let dir = spark_common::project::org_dir(Path::new(project)).join("triggers");
    std::fs::create_dir_all(&dir).expect("trigger config dir");
    std::fs::write(
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

/// A registered project whose org repo has a git identity, since every
/// definition write commits.
fn project(settings: &SparkSettings, name: &str) -> String {
    let project = settings.data_dir.parent().unwrap().join(name);
    std::fs::create_dir_all(&project).expect("project dir");
    let org = spark_common::project::ensure_org_repo(&project).expect("org repo");
    for (key, value) in [
        ("user.name", "Trigger Test"),
        ("user.email", "trigger@example.com"),
    ] {
        run_git(&org, &["config", key, value]).expect("git config");
    }
    spark_storage::ProjectRegistry::new(settings.data_dir.clone())
        .ensure_project_paths(&project.to_string_lossy())
        .expect("register project")
        .project_path
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
