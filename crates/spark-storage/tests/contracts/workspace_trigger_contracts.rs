use std::path::Path;

use serde_json::{json, Map, Value};
use spark_common::project::run_git;
use spark_storage::{
    StorageError, TriggerAction, TriggerDefinition, TriggerRepositories, TriggerState,
    TriggerStateHistoryEntry,
};

/// Repositories over a temp SPARK_HOME and a project whose org repo has a
/// git identity, since every definition write commits.
fn fixture(root: &Path) -> (TriggerRepositories, String) {
    let home = root.join("spark-home");
    let project = root.join("project");
    std::fs::create_dir_all(&project).expect("project");
    let org = spark_common::project::ensure_org_repo(&project).expect("org repo");
    for (key, value) in [
        ("user.name", "Trigger Test"),
        ("user.email", "trigger@example.com"),
    ] {
        run_git(&org, &["config", key, value]).expect("git config");
    }
    (
        TriggerRepositories::new(home.join("config"), home.clone()),
        project.to_string_lossy().into_owned(),
    )
}

fn org(project: &str) -> std::path::PathBuf {
    spark_common::project::org_dir(Path::new(project))
}

fn last_commit(project: &str) -> String {
    run_git(&org(project), &["log", "-1", "--format=%s"]).expect("git log")
}

#[test]
fn webhook_definitions_keep_credentials_out_of_the_org_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (repositories, project) = fixture(temp.path());
    let repository = repositories.definitions(&project).expect("repository");
    let definition = webhook_definition("trigger-storage");

    repository.put(&definition, None).expect("write definition");
    let path = repository.definition_path(&definition.id).expect("path");
    let text = std::fs::read_to_string(&path).expect("definition toml");
    assert!(text.contains("[action]"));
    assert!(text.contains(r#"static_context_json = "{\"origin\": \"compat\"}""#));
    for absent in ["webhook_key", "secret_hash", "enabled", "project_path"] {
        assert!(!text.contains(absent), "{absent} stays out of the org file");
    }

    let loaded = repository
        .get("trigger-storage")
        .expect("read definition")
        .expect("definition");
    assert!(!loaded.revision.is_empty());
    assert_eq!(loaded.project_path, repository.project_path());
    assert!(loaded.source.is_empty());
    assert_eq!(
        serde_json::to_value(TriggerDefinition {
            source: Map::new(),
            ..definition
        })
        .unwrap(),
        serde_json::to_value(loaded).unwrap()
    );
}

#[test]
fn schedule_poll_and_flow_event_sources_round_trip_source_shapes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (repositories, project) = fixture(temp.path());
    let repository = repositories.definitions(&project).expect("repository");
    for mut definition in [
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
        definition.revision = repository.put(&definition, None).expect("write definition");
        definition.project_path = repository.project_path().to_string();
        assert_eq!(
            repository
                .get(&definition.id)
                .expect("read")
                .expect("definition"),
            definition
        );
    }
}

#[test]
fn definitions_live_in_the_org_repo_and_every_write_commits_only_its_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (repositories, project) = fixture(temp.path());
    let repository = repositories.definitions(&project).expect("repository");
    let org = org(&project);
    assert_eq!(repository.root_dir(), org.join("triggers"));
    assert_eq!(
        repository.definition_path("trigger-a").expect("path"),
        org.join("triggers/trigger-a.toml")
    );
    assert!(repository.definition_path("../bad").is_err());
    // An unrelated change in the org repo stays out of trigger commits.
    std::fs::write(org.join("notes.md"), "draft").expect("notes");

    let mut first = webhook_definition("trigger-a");
    first.revision = repository.put(&first, None).expect("create");
    assert_eq!(last_commit(&project), "Create trigger Storage trigger");
    first.name = "Renamed".into();
    first.revision = repository
        .put(&first, Some("Rename for the nightly report"))
        .expect("edit");
    assert_eq!(last_commit(&project), "Rename for the nightly report");
    first.name = "Again".into();
    first.revision = repository.put(&first, Some("  ")).expect("edit");
    assert_eq!(last_commit(&project), "Edit trigger Again");

    // No lock or temp files in the org repo: only the definition, committed.
    let entries = std::fs::read_dir(org.join("triggers"))
        .expect("triggers dir")
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(entries, vec!["trigger-a.toml"]);
    assert_eq!(
        run_git(&org, &["status", "--porcelain"]).expect("status"),
        "?? notes.md"
    );

    repository
        .delete("trigger-a", &first.revision, None)
        .expect("delete");
    assert_eq!(last_commit(&project), "Delete trigger Again");
    assert!(repository.get("trigger-a").expect("missing").is_none());
    let mut second = webhook_definition("trigger-b");
    second.revision = repository.put(&second, None).expect("create");
    repository
        .delete("trigger-b", &second.revision, Some("No longer needed"))
        .expect("delete");
    assert_eq!(last_commit(&project), "No longer needed");
    assert_eq!(
        run_git(&org, &["status", "--porcelain"]).expect("status"),
        "?? notes.md"
    );
}

/// Makes every commit in the project's org repo fail, as a rejecting
/// pre-commit hook does; returns the hook so a test can remove it.
fn reject_commits(project: &str) -> std::path::PathBuf {
    let hook = org(project).join(".git/hooks/pre-commit");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    hook
}

#[test]
fn a_rejected_commit_leaves_the_definition_as_it_was_and_a_retry_succeeds() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (repositories, project) = fixture(temp.path());
    let repository = repositories.definitions(&project).expect("repository");
    let mut saved = webhook_definition("trigger-a");
    saved.revision = repository.put(&saved, None).expect("create");
    let head = run_git(&org(&project), &["rev-parse", "HEAD"]).unwrap();
    let hook = reject_commits(&project);

    // A failed create leaves no definition behind.
    assert!(repository
        .put(&webhook_definition("trigger-b"), None)
        .is_err());
    assert!(repository.get("trigger-b").unwrap().is_none());

    // A failed edit leaves the old definition and revision.
    let mut edited = saved.clone();
    edited.name = "Edited".into();
    assert!(repository.put(&edited, None).is_err());
    let current = repository.get("trigger-a").unwrap().unwrap();
    assert_eq!(
        (current.name.as_str(), &current.revision),
        ("Storage trigger", &saved.revision)
    );

    // A failed delete keeps the definition.
    assert!(repository
        .delete("trigger-a", &saved.revision, None)
        .is_err());
    assert_eq!(
        repository.get("trigger-a").unwrap().unwrap().revision,
        saved.revision
    );
    assert_eq!(
        run_git(&org(&project), &["rev-parse", "HEAD"]).unwrap(),
        head
    );
    assert_eq!(
        run_git(&org(&project), &["status", "--porcelain"]).unwrap(),
        ""
    );

    // Once commits work again, retrying with the original revision succeeds.
    std::fs::remove_file(hook).unwrap();
    repository.put(&edited, None).expect("retry edit");
    assert_eq!(last_commit(&project), "Edit trigger Edited");
}

#[test]
fn runtime_state_updates_serialize_under_a_lock_in_the_data_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (repositories, project) = fixture(temp.path());
    let states = repositories.runtime_state(&project).expect("states");
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let holder = {
        let states = states.clone();
        std::thread::spawn(move || {
            states
                .update("trigger-a", |state| {
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
    let refresher = {
        let states = states.clone();
        std::thread::spawn(move || {
            let state = states
                .update("trigger-a", |state| state.next_run_at = Some("next".into()))
                .unwrap();
            done_tx.send(state).unwrap();
        })
    };
    // The second writer waits for the first instead of saving a stale copy.
    assert!(done_rx
        .recv_timeout(std::time::Duration::from_millis(200))
        .is_err());
    release_tx.send(()).unwrap();
    holder.join().unwrap();
    let refreshed = done_rx.recv().unwrap();
    refresher.join().unwrap();
    assert!(!refreshed.enabled);
    assert_eq!(refreshed.secret_hash.as_deref(), Some("rotated"));
    let stored = states.load("trigger-a").unwrap();
    assert_eq!(
        (
            stored.enabled,
            stored.secret_hash.as_deref(),
            stored.next_run_at.as_deref()
        ),
        (false, Some("rotated"), Some("next"))
    );
    // The lock lives beside the state, never in the org repo.
    assert!(states.root_dir().join("trigger-a.json.lock").exists());
    assert!(!org(&project).join("triggers").exists());
}

#[test]
fn reads_create_nothing_and_the_first_write_creates_the_org_repo() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    let project = temp.path().join("fresh");
    std::fs::create_dir_all(&project).expect("project");
    let repositories = TriggerRepositories::new(home.join("config"), home.clone());
    let repository = repositories
        .definitions(&project.to_string_lossy())
        .expect("repository");
    assert!(repository.list().expect("list").is_empty());
    assert!(repository.get("trigger-a").expect("get").is_none());
    assert!(!project.join(".spark").exists(), "reads create nothing");

    // The first write creates the org repo, then commits into it.
    let org = spark_common::project::org_dir(&project);
    let result = repository.put(&webhook_definition("trigger-a"), None);
    assert!(org.join(".git").is_dir());
    if result.is_ok() {
        assert_eq!(
            run_git(&org, &["log", "-1", "--format=%s"]).expect("log"),
            "Create trigger Storage trigger"
        );
    }
}

#[test]
fn hand_edited_and_committed_definitions_are_read_as_they_are() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (repositories, project) = fixture(temp.path());
    let repository = repositories.definitions(&project).expect("repository");
    let revision = repository
        .put(&webhook_definition("trigger-a"), None)
        .expect("create");
    let path = repository.definition_path("trigger-a").expect("path");
    let text = std::fs::read_to_string(&path).expect("text");
    std::fs::write(&path, text.replace("Storage trigger", "Edited by hand")).expect("edit");
    run_git(&org(&project), &["commit", "-qam", "Edit by hand"]).expect("commit");
    let loaded = repository
        .get("trigger-a")
        .expect("get")
        .expect("definition");
    assert_eq!(loaded.name, "Edited by hand");
    assert_ne!(loaded.revision, revision);
}

#[test]
fn the_same_id_in_two_projects_is_two_independent_triggers() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (_, first) = fixture(&temp.path().join("one"));
    let (_, second) = fixture(&temp.path().join("two"));
    let repositories = repositories_for(temp.path());
    for project in [&first, &second] {
        spark_storage::ProjectRegistry::new(temp.path().join("spark-home"))
            .ensure_project_paths(project)
            .expect("register");
    }
    let mut one = webhook_definition("trigger-a");
    one.name = "One".into();
    let mut two = webhook_definition("trigger-a");
    two.name = "Two".into();
    repositories
        .definitions(&first)
        .unwrap()
        .put(&one, None)
        .expect("first");
    repositories
        .definitions(&second)
        .unwrap()
        .put(&two, None)
        .expect("second");
    repositories
        .runtime_state(&first)
        .unwrap()
        .update("trigger-a", |state| state.enabled = false)
        .expect("state");

    let mut all = repositories
        .list_all()
        .expect("list all")
        .into_iter()
        .map(|definition| (definition.project_path, definition.name))
        .collect::<Vec<_>>();
    all.sort();
    let mut expected = vec![
        (first.clone(), "One".to_string()),
        (second.clone(), "Two".to_string()),
    ];
    expected.sort();
    assert_eq!(all, expected);
    assert!(
        !repositories
            .runtime_state(&first)
            .unwrap()
            .load("trigger-a")
            .unwrap()
            .enabled
    );
    assert!(
        repositories
            .runtime_state(&second)
            .unwrap()
            .load("trigger-a")
            .unwrap()
            .enabled
    );
}

fn repositories_for(root: &Path) -> TriggerRepositories {
    let home = root.join("spark-home");
    TriggerRepositories::new(home.join("config"), home)
}

#[test]
fn persisted_trigger_definitions_are_validated_and_malformed_files_are_skipped_by_list() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (repositories, project) = fixture(temp.path());
    let repository = repositories.definitions(&project).expect("repository");
    repository
        .put(&webhook_definition("trigger-valid"), None)
        .expect("write valid");

    let mut draft = webhook_definition("trigger-new-workspace-draft");
    draft.action.mode = "workspace_draft".to_string();
    match repository.put(&draft, None) {
        Err(StorageError::InvalidRepositoryPath { reason, .. }) => {
            assert_eq!(
                reason,
                "Trigger action mode must be static, mission, or role."
            )
        }
        other => panic!("expected workspace_draft creation to be rejected, got {other:?}"),
    }
    assert!(repository
        .get("trigger-new-workspace-draft")
        .expect("get")
        .is_none());

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
            "trigger-workspace-draft",
            persisted_trigger_toml(
                "trigger-workspace-draft",
                "flow_event",
                "ops/run.dot",
                r#"flow_name = "ops/run.dot"
statuses = ["completed"]"#,
            )
            .replace("[action]\n", "[action]\nmode = \"workspace_draft\"\n"),
            "Trigger action mode must be static, mission, or role.",
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
    let (repositories, project) = fixture(temp.path());
    let repository = repositories.runtime_state(&project).expect("repository");
    let project_root = spark_storage::ProjectRegistry::new(temp.path().join("spark-home"))
        .project_paths(&project)
        .expect("project paths")
        .root;

    // Beside the project's role records, never in the org repo.
    assert_eq!(repository.root_dir(), project_root.join("triggers"));
    assert_eq!(
        repository.state_path("trigger-state").expect("state path"),
        project_root.join("triggers/trigger-state.json")
    );
    let missing = repository.load("trigger-state").expect("missing state");
    assert_eq!(missing, TriggerState::default());
    assert!(missing.enabled, "a trigger is enabled until switched off");

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
    assert!(partial.enabled);
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
            // Ignored: webhook credentials live in runtime state.
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
        revision: String::new(),
        project_path: String::new(),
        id: id.to_string(),
        name: "Storage trigger".to_string(),
        protected: false,
        source_type: source_type.to_string(),
        action: TriggerAction {
            mission_id: None,
            role: None,
            mode: "static".to_string(),
            flow_name: "ops/run.dot".to_string(),
            static_context: Map::from_iter([("origin".to_string(), json!("compat"))]),
            execution_profile_id: None,
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
