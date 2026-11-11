use std::collections::BTreeMap;
use std::fs;

use serde_json::{json, Value};
use spark_common::project::build_project_id;
use spark_storage::conversation::{
    Accepted, Admission, CatchUp, ChangeScope, ConversationCommit, ExecutionOutcome, MessageInput,
    Settlement, TOOL_OUTPUT_INLINE_LIMIT_BYTES,
};
use spark_storage::{
    ConversationRepository, ConversationStore, DeletedProjectRecord, ProjectRecordUpdate,
    ProjectRegistry, StorageError,
};

#[test]
fn project_registry_registers_lists_and_survives_restart_as_rows() {
    let temp = tempfile::tempdir().expect("tempdir");
    // Canonicalize: registered records hold canonical paths (macOS /var -> /private/var).
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let home = root.join("spark-home");
    let project_dir = root.join("Registered Project");
    fs::create_dir_all(&project_dir).expect("project dir");
    let registry = ProjectRegistry::new(&home);

    let record = registry
        .register_project(project_dir.to_str().expect("utf-8"))
        .expect("register");

    assert_eq!(record.display_name, "Registered Project");
    assert_eq!(record.project_path, project_dir.to_string_lossy());
    assert!(!record.is_favorite);
    assert!(record.folder_exists);
    assert!(!record.created_at.is_empty());
    let project_root = home.join("projects").join(&record.project_id);
    assert!(project_root.join("conversations").is_dir());
    // The registry is a database row: no project.toml, no lock, and no
    // obsolete artifact directories.
    let mut created: Vec<_> = fs::read_dir(&project_root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    created.sort();
    assert_eq!(created, vec!["conversations"]);
    assert!(home
        .join(spark_storage::conversation::STATE_DATABASE_FILE_NAME)
        .is_file());

    assert_eq!(
        ProjectRegistry::new(&home)
            .list_project_records()
            .expect("list"),
        vec![record.clone()]
    );

    // Private model and execution defaults are columns of the same row.
    let path = record.project_path.as_str();
    let first = registry.project_settings(path).unwrap().revision;
    registry
        .update_project_model_settings(
            path,
            &first,
            Some(&json!({"provider": "codex", "model": "m"})),
        )
        .unwrap();
    let second = registry.project_settings(path).unwrap().revision;
    assert_ne!(second, first);
    registry
        .update_project_record(
            path,
            ProjectRecordUpdate {
                expected_revision: Some(second.clone()),
                execution_profile_id: Some(Some("native-dev".into())),
                ..ProjectRecordUpdate::default()
            },
        )
        .unwrap();
    let restarted = ProjectRegistry::new(&home);
    let third = restarted.project_settings(path).unwrap().revision;
    assert!(third != first && third != second);
    assert_eq!(
        restarted.project_settings(path).unwrap(),
        spark_storage::ProjectSettings {
            revision: third,
            model_settings: Some(json!({"provider": "codex", "model": "m"})),
            execution_profile_id: Some("native-dev".into()),
        }
    );
    assert_eq!(
        restarted
            .read_project_record(path)
            .unwrap()
            .unwrap()
            .execution_profile_id
            .as_deref(),
        Some("native-dev")
    );
    assert!(!project_root.join("project.toml").exists());
    // A corrupt stored selection is reported, never read as absent.
    rusqlite::Connection::open(home.join(spark_storage::conversation::STATE_DATABASE_FILE_NAME))
        .unwrap()
        .execute_batch(
            "PRAGMA ignore_check_constraints = ON; UPDATE projects SET model_settings = 'not json'",
        )
        .unwrap();
    assert!(restarted.project_settings(path).is_err());
}

#[test]
fn project_registry_never_reads_project_toml() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    let project_path = "/projects/file-created";
    let project_id = build_project_id(project_path).expect("project id");
    let root = home.join("projects").join(&project_id);
    fs::create_dir_all(&root).expect("root");
    let stray = format!(
        "project_id = \"{project_id}\"\nproject_path = \"{project_path}\"\ndisplay_name = \"File\"\n\
         created_at = \"2026-01-01T00:00:00Z\"\nlast_opened_at = \"2026-01-01T00:00:01Z\"\n\
         execution_profile_id = \"native\"\n[model_settings]\nprovider = \"codex\"\n"
    );
    fs::write(root.join("project.toml"), &stray).expect("project toml");
    let registry = ProjectRegistry::new(&home);

    assert_eq!(
        registry.read_project_record_by_id(&project_id).unwrap(),
        None
    );
    assert!(registry.list_project_records().unwrap().is_empty());
    let record = registry.register_project(project_path).unwrap();
    assert_eq!(record.display_name, "file-created");
    assert_eq!(record.execution_profile_id, None);
    let settings = registry.project_settings(project_path).unwrap();
    assert_eq!(settings.model_settings, None);
    assert_eq!(
        fs::read_to_string(root.join("project.toml")).unwrap(),
        stray
    );
}

#[test]
fn project_registry_updates_optional_state_and_deletes_project_handles() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    let project_path = "/projects/my-app";
    let registry = ProjectRegistry::new(&home);
    let record = registry.register_project(project_path).expect("register");
    assert_eq!(record.project_path, project_path);
    assert_eq!(
        registry
            .project_settings("/projects/other")
            .unwrap()
            .revision,
        "absent"
    );

    let revision = registry.project_settings(project_path).unwrap().revision;
    let updated = registry
        .update_project_record(
            project_path,
            ProjectRecordUpdate {
                last_accessed_at: Some(Some("2026-02-03T04:05:06Z".to_string())),
                is_favorite: Some(true),
                active_conversation_id: Some(Some("conversation-1".to_string())),
                execution_profile_id: Some(Some("native".to_string())),
                expected_revision: Some(revision.clone()),
                ..ProjectRecordUpdate::default()
            },
        )
        .expect("update");
    assert!(updated.is_favorite);
    assert_eq!(
        updated.active_conversation_id.as_deref(),
        Some("conversation-1")
    );
    assert_eq!(updated.execution_profile_id.as_deref(), Some("native"));
    // The settings revision moved, so the stale one conflicts.
    assert!(matches!(
        registry.update_project_record(
            project_path,
            ProjectRecordUpdate {
                execution_profile_id: Some(None),
                expected_revision: Some(revision.clone()),
                ..ProjectRecordUpdate::default()
            },
        ),
        Err(StorageError::SettingsConflict { .. })
    ));
    assert!(matches!(
        registry.update_project_model_settings(project_path, &revision, Some(&json!({}))),
        Err(StorageError::SettingsConflict { .. })
    ));
    let models = json!({"provider": "codex", "model": "gpt-5", "thinking": null});
    let current = registry.project_settings(project_path).unwrap().revision;
    registry
        .update_project_model_settings(project_path, &current, Some(&models))
        .unwrap();
    let stored = ProjectRegistry::new(&home)
        .project_settings(project_path)
        .unwrap();
    assert_eq!(
        stored.model_settings,
        Some(json!({"provider": "codex", "model": "gpt-5"}))
    );
    assert_eq!(stored.execution_profile_id.as_deref(), Some("native"));
    assert_ne!(stored.revision, current);

    // Validation alone never removes anything; the store removes the
    // conversation files after tombstoning, then `G` once it is empty.
    let deleted = registry.deletable_project(project_path).expect("deletable");
    assert_eq!(deleted.project_id, updated.project_id);
    let project_root = home.join("projects").join(&updated.project_id);
    assert!(project_root.exists());
    let store = ConversationStore::open(&home).expect("store");
    store.delete_project(&deleted, &|_| {}).expect("delete");
    assert!(!project_root.exists());
    assert_eq!(registry.read_project_record(project_path).unwrap(), None);
    assert_eq!(
        registry.project_settings(project_path).unwrap().revision,
        "absent"
    );
    // A revision read before the removal never matches the next registration.
    registry.ensure_project_paths(project_path).unwrap();
    assert!(matches!(
        registry.update_project_model_settings(project_path, &stored.revision, Some(&models)),
        Err(StorageError::SettingsConflict { .. })
    ));
    assert!(matches!(
        registry.update_project_record(
            project_path,
            ProjectRecordUpdate {
                execution_profile_id: Some(Some("native".into())),
                expected_revision: Some(stored.revision.clone()),
                ..ProjectRecordUpdate::default()
            },
        ),
        Err(StorageError::SettingsConflict { .. })
    ));
}

/// A loaded record saves only over the payload it read: a change another
/// writer made meanwhile is a conflict, and the refused save writes nothing.
#[test]
fn a_loaded_record_saves_only_over_what_it_read() {
    use spark_storage::workspace_records::RecordRepository;
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let paths = ProjectRegistry::new(&home)
        .ensure_project_paths("/projects/records")
        .unwrap();
    let roles = RecordRepository::roles(&home, &paths.project_id);
    let mut record = roles.load("lead").unwrap();
    assert_eq!(record.stored(), None);
    record.save(&json!({"name": "lead", "n": 1})).unwrap();
    record.save(&json!({"name": "lead", "n": 2})).unwrap();
    roles
        .write("lead", &json!({"name": "lead", "n": 3}))
        .unwrap();
    assert!(matches!(
        record.save(&json!({"name": "lead", "n": 4})),
        Err(StorageError::RecordChanged { .. })
    ));
    assert_eq!(
        roles.read("lead").unwrap(),
        Some(json!({"name": "lead", "n": 3}))
    );
}

/// Sets every optional field of a project's row, as an older session would
/// have left it.
fn seed_historical_metadata(registry: &ProjectRegistry, path: &str) -> spark_storage::ProjectPaths {
    let paths = registry.ensure_project_paths(path).expect("initialize");
    let connection = rusqlite::Connection::open(
        registry
            .home_dir()
            .join(spark_storage::conversation::STATE_DATABASE_FILE_NAME),
    )
    .unwrap();
    connection
        .execute(
            "UPDATE projects SET display_name = 'Custom project name', \
             created_at = '2001-01-01T00:00:00Z', last_opened_at = '2002-01-01T00:00:00Z', \
             last_accessed_at = '2003-01-01T00:00:00Z', active_conversation_id = 'conversation-1', \
             execution_profile_id = 'native', is_favorite = 1 WHERE project_id = ?1",
            [&paths.project_id],
        )
        .unwrap();
    paths
}

#[test]
fn initialization_recreates_directories_and_reads_preserve_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let registry = ProjectRegistry::new(temp.path());
    let path = "/projects/idempotent";
    assert_eq!(registry.read_project_record(path).unwrap(), None);
    assert!(!registry.home_dir().join("projects").exists());
    let paths = seed_historical_metadata(&registry, path);
    let record = registry.read_project_record(path).unwrap().unwrap();
    assert_eq!(record.display_name, "Custom project name");
    assert!(paths.conversations_dir.is_dir());
    fs::remove_dir(&paths.conversations_dir).unwrap();
    assert_eq!(
        registry.read_project_record(path).unwrap(),
        Some(record.clone())
    );
    assert!(!paths.conversations_dir.exists());
    registry.ensure_project_paths(path).unwrap();
    assert!(paths.conversations_dir.is_dir());
    assert_eq!(registry.read_project_record(path).unwrap(), Some(record));
}

#[test]
fn registration_and_updates_preserve_unspecified_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let registry = ProjectRegistry::new(temp.path());
    let path = "/projects/explicit";
    seed_historical_metadata(&registry, path);
    let mut expected = registry.read_project_record(path).unwrap().unwrap();
    let updated = registry
        .update_project_record(
            path,
            ProjectRecordUpdate {
                last_accessed_at: Some(Some("2004-01-01T00:00:00Z".into())),
                ..ProjectRecordUpdate::default()
            },
        )
        .unwrap();
    expected.last_accessed_at = Some("2004-01-01T00:00:00Z".into());
    assert_eq!(updated, expected);
    let registered = registry.register_project(path).unwrap();
    assert!(registered.last_opened_at > expected.last_opened_at);
    expected.last_opened_at = registered.last_opened_at.clone();
    assert_eq!(registered, expected);
    let fresh = registry
        .update_project_record(
            "/projects/update-missing",
            ProjectRecordUpdate {
                display_name: Some("New custom name".into()),
                is_favorite: Some(true),
                ..ProjectRecordUpdate::default()
            },
        )
        .unwrap();
    assert_eq!(fresh.display_name, "New custom name");
    assert!(fresh.is_favorite);
    assert!(!fresh.created_at.is_empty());
    assert_eq!(fresh.created_at, fresh.last_opened_at);
    assert_eq!(
        registry
            .read_project_record("/projects/update-missing")
            .unwrap(),
        Some(fresh)
    );
}

#[test]
fn project_operations_reject_empty_paths_without_initializing() {
    let temp = tempfile::tempdir().unwrap();
    let registry = ProjectRegistry::new(temp.path());
    for path in ["", "   "] {
        assert!(registry.ensure_project_paths(path).is_err());
        assert!(registry.register_project(path).is_err());
        assert!(registry.read_project_record(path).is_err());
        assert!(registry
            .update_project_record(path, ProjectRecordUpdate::default())
            .is_err());
    }
    assert!(!registry.home_dir().join("projects").exists());
}

#[test]
fn provider_events_persist_without_replacing_project_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let registry = ProjectRegistry::new(temp.path());
    let path = "/projects/provider-events";
    seed_historical_metadata(&registry, path);
    let repo = ConversationRepository::new(ConversationStore::open(temp.path()).unwrap());
    start(&repo, "conversation-1", path, "turn-1", &[]).unwrap();
    let before = registry.read_project_record(path).unwrap();
    for message in ["one", "two", "three"] {
        record(
            &repo,
            "conversation-1",
            path,
            "turn-1",
            tool_event(message, message),
        )
        .unwrap();
        assert_eq!(registry.read_project_record(path).unwrap(), before);
    }
    let snapshot = repo
        .read_snapshot("conversation-1", Some(path))
        .unwrap()
        .unwrap();
    let outputs: Vec<_> = snapshot["segments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|segment| segment["tool_call"]["output"].clone())
        .collect();
    assert_eq!(outputs, vec![json!("one"), json!("two"), json!("three")]);
}

#[test]
fn conversation_reads_never_register_projects_and_writes_need_a_live_conversation() {
    let temp = tempfile::tempdir().unwrap();
    let registry = ProjectRegistry::new(temp.path());
    let repo = ConversationRepository::new(ConversationStore::open(temp.path()).unwrap());
    let path = "/projects/unregistered";
    assert_eq!(
        repo.read_snapshot("conversation", Some(path)).unwrap(),
        None
    );
    assert!(repo
        .list_conversation_ids_for_project(path)
        .unwrap()
        .is_empty());
    repo.delete_conversation("conversation", path).unwrap();
    assert!(record(
        &repo,
        "conversation",
        path,
        "turn",
        tool_event("call", "hello")
    )
    .is_err());
    assert!(settle(&repo, "conversation", path, "turn").is_err());
    assert_eq!(registry.read_project_record(path).unwrap(), None);
    // The first message of a new conversation registers its project.
    start(&repo, "conversation", path, "turn", &[]).unwrap();
    assert!(registry.read_project_record(path).unwrap().is_some());
    settle(&repo, "conversation", path, "turn").unwrap();
    let session = repo
        .read_runtime_session("conversation", Some(path))
        .unwrap()
        .expect("session");
    assert_eq!(session.thread_id.as_deref(), Some("thread"));
    assert_eq!(session.last_turn_id.as_deref(), Some("turn"));
}

/// Accept a message that starts the execution `assistant_turn_id`, with
/// `attachments`.
fn start(
    repo: &ConversationRepository,
    conversation_id: &str,
    path: &str,
    assistant_turn_id: &str,
    attachments: &[&str],
) -> spark_storage::Result<Option<ConversationCommit>> {
    let attachments: Vec<_> = attachments.iter().map(|id| json!({"id": id})).collect();
    let (commit, accepted) = repo.accept_message(
        conversation_id,
        path,
        MessageInput {
            admission: Admission::Start,
            user_turn_id: format!("{assistant_turn_id}-user"),
            content: "hello".into(),
            attachments: (!attachments.is_empty()).then(|| json!(attachments)),
            client_request_sha256: None,
            expected_settings_revision: 0,
            settings: Default::default(),
            held: false,
            assistant_turn_id: assistant_turn_id.into(),
            execution_settings: json!({}),
        },
    )?;
    assert!(matches!(accepted, Accepted::Started { .. }), "{accepted:?}");
    Ok(commit)
}

/// Record one provider event of the execution `turn_id`.
fn record(
    repo: &ConversationRepository,
    conversation_id: &str,
    path: &str,
    turn_id: &str,
    event: Value,
) -> spark_storage::Result<Option<ConversationCommit>> {
    let event = serde_json::from_value(event).unwrap();
    Ok(repo
        .record_provider_event(conversation_id, path, turn_id, &event, &BTreeMap::new())?
        .commit)
}

/// A completed tool call `call_id` with `output`.
fn tool_event(call_id: &str, output: &str) -> Value {
    json!({"kind": "tool_call_completed", "tool_call": {"id": call_id, "output": output}})
}

/// Answer and settle the execution `turn_id` on the provider thread `thread`.
fn settle(
    repo: &ConversationRepository,
    conversation_id: &str,
    path: &str,
    turn_id: &str,
) -> spark_storage::Result<Settlement> {
    let answer = json!({"kind": "content_completed", "channel": "assistant",
        "phase": "final_answer", "message": "Done."});
    record(repo, conversation_id, path, turn_id, answer)?;
    repo.settle_execution(
        conversation_id,
        path,
        turn_id,
        ExecutionOutcome::Returned {
            final_text: Some("Done.".into()),
            token_usage: None,
            token_usage_breakdown: None,
            app_thread_id: Some("thread".into()),
            app_turn_id: None,
            thread_resume_failure: None,
            continued_question: None,
        },
        &BTreeMap::new(),
    )
}

/// A registered project with two conversations, an attachment and an
/// externalized tool output.
fn seeded_project(
    home: &std::path::Path,
    path: &str,
) -> (std::sync::Arc<ConversationStore>, DeletedProjectRecord) {
    let registry = ProjectRegistry::new(home);
    registry.register_project(path).unwrap();
    let store = ConversationStore::open(home).unwrap();
    let repo = ConversationRepository::new(store.clone());
    let attachment = repo
        .create_attachment("chat-a", path, "notes.txt", b"attached bytes")
        .unwrap();
    start(&repo, "chat-a", path, "turn-a", &[&attachment.id]).unwrap();
    start(&repo, "chat-b", path, "turn-b", &[]).unwrap();
    settle(&repo, "chat-b", path, "turn-b").unwrap();
    let output = "x".repeat(TOOL_OUTPUT_INLINE_LIMIT_BYTES * 2);
    record(&repo, "chat-a", path, "turn-a", tool_event("call", &output)).unwrap();
    let deleted = registry.deletable_project(path).unwrap();
    seed_private_rows(home, path, &deleted.project_id);
    (store, deleted)
}

/// A role, a mission, an armed trigger and a global milestone for the project.
fn seed_private_rows(home: &std::path::Path, path: &str, project_id: &str) {
    use spark_storage::workspace_records::{RecordRepository, WorkflowMilestones};
    RecordRepository::roles(home, project_id)
        .write("coordinator", &json!({"name": "coordinator"}))
        .unwrap();
    RecordRepository::missions(home, project_id)
        .write("mission-1", &json!({"id": "mission-1"}))
        .unwrap();
    spark_storage::TriggerRepositories::new(home.join("config"), home)
        .runtime_state(path)
        .unwrap()
        .update("trigger-a", |state| {
            state.enabled = true;
            state.webhook_key = Some("key".into());
            state.secret_hash = Some("hash".into());
        })
        .unwrap();
    WorkflowMilestones::new(home)
        .record::<StorageError>(&format!("run-{project_id}"), |_| {
            Ok((
                vec![(
                    format!("milestone-{project_id}"),
                    json!({"project_path": path}),
                )],
                json!({}),
            ))
        })
        .unwrap();
}

/// Removal deletes the project's private rows, keeps its global milestone,
/// and a late trigger outcome recreates nothing.
fn assert_private_rows_removed(home: &std::path::Path, path: &str, project_id: &str) {
    use spark_storage::workspace_records::{RecordRepository, WorkflowMilestones};
    assert_eq!(
        ProjectRegistry::new(home)
            .read_project_record(path)
            .unwrap(),
        None
    );
    assert!(RecordRepository::roles(home, project_id)
        .list()
        .unwrap()
        .is_empty());
    assert!(RecordRepository::missions(home, project_id)
        .list()
        .unwrap()
        .is_empty());
    let triggers = spark_storage::TriggerRepositories::new(home.join("config"), home);
    let states = triggers.runtime_state(path).unwrap();
    assert_eq!(states.get("trigger-a").unwrap(), None);
    assert_eq!(triggers.find_webhook_key("key").unwrap(), None);
    assert_eq!(
        states
            .update_existing("trigger-a", |state| state.last_result = Some("late".into()))
            .unwrap(),
        None
    );
    assert_eq!(states.get("trigger-a").unwrap(), None);
    // A late activation or record write fails and recreates nothing.
    assert!(matches!(
        states.update("trigger-a", |state| state.enabled = true),
        Err(StorageError::ProjectNotRegistered { .. })
    ));
    assert_eq!(states.get("trigger-a").unwrap(), None);
    assert!(matches!(
        RecordRepository::missions(home, project_id).write("late", &json!({"id": "late"})),
        Err(StorageError::ProjectNotRegistered { .. })
    ));
    assert!(RecordRepository::missions(home, project_id)
        .list()
        .unwrap()
        .is_empty());
    assert!(WorkflowMilestones::new(home)
        .tail(100)
        .unwrap()
        .iter()
        .any(|entry| entry["project_path"] == path));
}

#[test]
fn project_deletion_tombstones_everything_then_removes_only_conversation_files() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let path = "/projects/doomed";
    let (store, deleted) = seeded_project(&home, path);
    let repo = ConversationRepository::new(store.clone());
    let handle = repo.conversation_handle("chat-a", None).unwrap();
    let project_root = home.join("projects").join(&deleted.project_id);
    assert!(project_root
        .join("conversations/chat-a/tool-output")
        .exists());
    let retained = Retained::seed(&home, &deleted.project_id, temp.path());

    let cancelled = std::sync::Mutex::new(Vec::new());
    store
        .delete_project(&deleted, &|id| {
            cancelled.lock().unwrap().push(id.to_string())
        })
        .unwrap();
    assert_eq!(*cancelled.lock().unwrap(), vec!["chat-a", "chat-b"]);
    assert!(!project_root.join("conversations").exists());
    retained.assert_intact();
    // Retained trees keep `G` but never the cleanup marker.
    assert_eq!(store.pending_project_deletion(path).unwrap(), None);
    assert_private_rows_removed(&home, path, &deleted.project_id);
    for id in ["chat-a", "chat-b"] {
        assert_eq!(
            store
                .database()
                .read(|read| read.conversation_location(id))
                .unwrap(),
            Some((path.to_string(), true))
        );
        assert_eq!(repo.read_snapshot(id, None).unwrap(), None);
    }
    // Old cursors converge through tombstones.
    let CatchUp::Batch(batch) = store
        .database()
        .read(|read| {
            read.changes(
                ChangeScope::Conversation("chat-a"),
                store.database().generation(),
                0,
            )
        })
        .unwrap()
    else {
        panic!("catch-up");
    };
    assert!(batch.rows.iter().all(|row| row.deleted));
    assert!(!batch.rows.is_empty());

    // Late callbacks for the deleted work fail and recreate nothing.
    assert!(record(&repo, "chat-a", path, "turn-a", tool_event("late", "late")).is_err());
    assert!(start(&repo, "chat-a", path, "turn-late", &[]).is_err());
    assert!(settle(&repo, "chat-a", path, "turn-a").is_err());
    assert!(!project_root.join("conversations").exists());

    // Re-registration starts fresh over the retained memory, work and runs;
    // the deleted handle is free again but a deleted conversation id never
    // comes back.
    ProjectRegistry::new(&home).register_project(path).unwrap();
    retained.assert_intact();
    assert_eq!(
        spark_storage::TriggerRepositories::new(home.join("config"), &home)
            .runtime_state(path)
            .unwrap()
            .get("trigger-a")
            .unwrap(),
        None,
        "re-registration never re-arms a trigger"
    );
    assert!(start(&repo, "chat-a", path, "turn-new", &[]).is_err());
    assert_eq!(
        repo.conversation_handle("chat-new", Some(&handle)).unwrap(),
        handle
    );
    start(&repo, "chat-new", path, "turn-new", &[]).unwrap();
    assert_eq!(
        repo.read_snapshot("chat-new", None).unwrap().unwrap()["conversation_handle"],
        handle
    );
}

/// Every file under `root`, with its bytes.
fn tree(root: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push((path.clone(), fs::read(&path).unwrap()));
            }
        }
    }
    files.sort();
    files
}

/// What project removal keeps: learned memory with a symlink out, a Git
/// worktree under `work` with an uncommitted edit, default and external run
/// roots, and an unknown sibling in `G`.
struct Retained {
    roots: Vec<std::path::PathBuf>,
    snapshot: Vec<Vec<(std::path::PathBuf, Vec<u8>)>>,
    worktree: std::path::PathBuf,
    outside: std::path::PathBuf,
}

impl Retained {
    fn seed(home: &std::path::Path, project_id: &str, scratch: &std::path::Path) -> Self {
        let storage = home.join("projects").join(project_id);
        let outside = scratch.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("target.md"), "outside bytes").unwrap();
        let memory = storage.join("memory/coordinator");
        fs::create_dir_all(&memory).unwrap();
        fs::write(memory.join("index.md"), "learned").unwrap();
        std::os::unix::fs::symlink(outside.join("target.md"), memory.join("link.md")).unwrap();
        // Conversation cleanup removes a link, never what it points at.
        let conversations = storage.join("conversations/chat-a");
        std::os::unix::fs::symlink(&outside, conversations.join("escape")).unwrap();

        let source = scratch.join("source-repo");
        fs::create_dir_all(&source).unwrap();
        let git = |dir: &std::path::Path, args: &[&str]| {
            let output = std::process::Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-C"])
                .arg(dir)
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            String::from_utf8(output.stdout).unwrap()
        };
        git(&source, &["init", "-q"]);
        fs::write(source.join("tracked.txt"), "committed").unwrap();
        git(&source, &["add", "."]);
        git(&source, &["commit", "-q", "-m", "seed"]);
        let worktree = storage.join("work/checkouts/run-kept");
        fs::create_dir_all(worktree.parent().unwrap()).unwrap();
        git(
            &source,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "kept",
                worktree.to_str().unwrap(),
            ],
        );
        fs::write(worktree.join("tracked.txt"), "uncommitted edit").unwrap();
        fs::write(storage.join("work/ledger.md"), "ledger").unwrap();

        let default_run = storage.join("runs/run-default");
        fs::create_dir_all(&default_run).unwrap();
        fs::write(default_run.join("run.json"), "{}").unwrap();
        let external_run = scratch
            .join("external-runs")
            .join(project_id)
            .join("run-ext");
        fs::create_dir_all(&external_run).unwrap();
        fs::write(external_run.join("run.json"), "{}").unwrap();
        fs::create_dir_all(storage.join("unknown")).unwrap();
        fs::write(storage.join("unknown/keep.bin"), "sibling").unwrap();

        let roots = vec![
            storage.join("memory"),
            storage.join("work"),
            storage.join("runs"),
            storage.join("unknown"),
            scratch.join("external-runs"),
        ];
        let snapshot = roots.iter().map(|root| tree(root)).collect();
        Self {
            roots,
            snapshot,
            worktree,
            outside,
        }
    }

    fn assert_intact(&self) {
        let now: Vec<_> = self.roots.iter().map(|root| tree(root)).collect();
        assert_eq!(now, self.snapshot);
        assert_eq!(
            fs::read_to_string(self.outside.join("target.md")).unwrap(),
            "outside bytes"
        );
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&self.worktree)
            .args(["status", "--porcelain"])
            .output()
            .unwrap();
        assert!(status.status.success(), "{status:?}");
        assert_eq!(
            String::from_utf8_lossy(&status.stdout).trim(),
            "M tracked.txt"
        );
    }
}

/// The tombstone commit is durable under NORMAL, but no checkpoint barrier
/// that is busy, incomplete, without a WAL, or failing removes a byte of the
/// project; restart finishes the cleanup, and another project's live rows
/// still reference existing bytes.
#[test]
fn deferred_project_cleanup_keeps_every_file_until_restart_finishes_it() {
    use spark_storage::conversation::CheckpointFault;
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let path = "/projects/deferred";
    let (store, deleted) = seeded_project(&home, path);
    let survivor = "/projects/survivor";
    seeded_project_in(&store, &home, survivor);
    let repo = ConversationRepository::new(store.clone());
    let kept = repo
        .create_attachment("chat-a-finished", survivor, "kept.txt", b"kept bytes")
        .unwrap();
    start(&repo, "chat-a-finished", survivor, "turn-kept", &[&kept.id]).unwrap();
    let project_root = home.join("projects").join(&deleted.project_id);
    let retained = Retained::seed(&home, &deleted.project_id, temp.path());
    // Everything so far is checkpointed; the deletion commits under NORMAL.
    store.database().after_checkpoint(|_| Ok(())).unwrap();
    let before = tree(&project_root);
    assert!(before
        .iter()
        .any(|(file, _)| file.parent().unwrap().ends_with("tool-output")));

    let faults = [
        CheckpointFault::Busy,
        CheckpointFault::Incomplete,
        CheckpointFault::NoWal,
        CheckpointFault::Error,
    ];
    store.database().fail_next_checkpoints(faults);
    let error = store.delete_project(&deleted, &|_| {}).unwrap_err();
    assert!(
        matches!(error, StorageError::CleanupDeferred { .. }),
        "{error}"
    );
    for fault in &faults[1..] {
        let error = store.finish_project_deletion(path).unwrap_err();
        assert!(
            matches!(error, StorageError::CleanupDeferred { .. }),
            "{fault:?}: {error}"
        );
        assert_eq!(tree(&project_root), before, "{fault:?}");
    }
    assert_eq!(tree(&project_root), before);
    assert_eq!(
        store.pending_project_deletion(path).unwrap(),
        Some(deleted.clone())
    );
    // The rows went with the tombstones, before file cleanup.
    assert_private_rows_removed(&home, path, &deleted.project_id);
    // Nothing new is admitted while cleanup is pending.
    assert!(start(&repo, "chat-c", path, "turn-c", &[]).is_err());
    drop(repo);
    drop(store);

    let store = ConversationStore::open(&home).unwrap();
    assert!(!project_root.join("conversations").exists());
    retained.assert_intact();
    assert_eq!(store.pending_project_deletion(path).unwrap(), None);
    let stored = store
        .database()
        .read(|read| read.conversation("chat-a-finished"))
        .unwrap()
        .expect("survivor");
    assert_eq!(stored.attachments.len(), 1);
    for (_, blob_path, _, _) in &stored.attachments {
        assert_eq!(fs::read(home.join(blob_path)).unwrap(), b"kept bytes");
    }
}

#[cfg(unix)]
#[test]
fn failed_directory_removal_keeps_the_marker_and_is_retried() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let path = "/projects/stuck";
    let (store, deleted) = seeded_project(&home, path);
    let retained = Retained::seed(&home, &deleted.project_id, temp.path());
    let conversations = home
        .join("projects")
        .join(&deleted.project_id)
        .join("conversations");
    fs::set_permissions(&conversations, fs::Permissions::from_mode(0o555)).unwrap();
    let error = store.delete_project(&deleted, &|_| {}).unwrap_err();
    assert!(matches!(error, StorageError::Io { .. }), "{error}");
    assert_eq!(
        store.pending_project_deletion(path).unwrap(),
        Some(deleted.clone())
    );
    retained.assert_intact();
    assert_private_rows_removed(&home, path, &deleted.project_id);
    // Registration of the same path cannot proceed until cleanup finishes.
    assert!(store.finish_project_deletion(path).is_err());
    fs::set_permissions(&conversations, fs::Permissions::from_mode(0o755)).unwrap();
    // Registration finishes the pending cleanup first, then reuses the
    // retained trees without importing a deleted record.
    store
        .register_project(path, || ProjectRegistry::new(&home).register_project(path))
        .unwrap();
    assert!(!conversations.join("chat-a").exists());
    retained.assert_intact();
    assert_eq!(store.pending_project_deletion(path).unwrap(), None);
    let records =
        spark_storage::workspace_records::RecordRepository::roles(&home, &deleted.project_id);
    assert!(records.list().unwrap().is_empty());
    assert_eq!(
        spark_storage::TriggerRepositories::new(home.join("config"), &home)
            .runtime_state(path)
            .unwrap()
            .get("trigger-a")
            .unwrap(),
        None
    );
}

#[test]
fn deletion_during_provider_output_stops_the_writer_without_recreating_files() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let path = "/projects/streaming";
    let (store, deleted) = seeded_project(&home, path);
    let repo = ConversationRepository::new(store.clone());
    let writer = std::thread::spawn(move || {
        for index in 0.. {
            let event = tool_event(&format!("call-{index}"), &format!("{index}"));
            if let Err(error) = record(&repo, "chat-a", path, "turn-a", event) {
                return (index, error);
            }
        }
        unreachable!()
    });
    std::thread::sleep(std::time::Duration::from_millis(20));
    store.delete_project(&deleted, &|_| {}).unwrap();
    let (_, error) = writer.join().unwrap();
    assert!(
        matches!(error, StorageError::ConversationCommitRejected { .. }),
        "{error}"
    );
    assert!(!home.join("projects").join(&deleted.project_id).exists());
}

#[test]
fn turns_cannot_reference_attachments_whose_bytes_are_missing() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let path = "/projects/missing-bytes";
    let repo = ConversationRepository::new(ConversationStore::open(&home).unwrap());
    let attachment = repo
        .create_attachment("chat", path, "notes.txt", b"bytes")
        .unwrap();
    let (_, blob) = repo
        .read_attachment("chat", path, &attachment.id)
        .unwrap()
        .expect("readable");
    fs::remove_file(&blob).unwrap();
    assert_eq!(
        repo.read_attachment("chat", path, &attachment.id).unwrap(),
        None
    );
    let error = start(&repo, "chat", path, "turn-assistant", &[&attachment.id]).unwrap_err();
    assert!(error.to_string().contains("not available"), "{error}");
}

#[test]
fn a_second_store_on_the_same_home_is_refused_until_the_first_closes() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let other = temp.path().join("other");
    let store = ConversationStore::open(&home).unwrap();
    assert!(matches!(
        ConversationStore::open(&home),
        Err(StorageError::HomeLocked { .. })
    ));
    // Independent homes do not contend.
    let independent = ConversationStore::open(&other).unwrap();
    drop(independent);
    drop(store);
    ConversationStore::open(&home).expect("lock released on drop");
    assert!(home
        .join(spark_storage::conversation::SERVER_LOCK_FILE_NAME)
        .exists());
}

#[test]
fn startup_leaves_historical_files_in_place_and_imports_none_of_them() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let registry = ProjectRegistry::new(&home);
    let paths = registry.ensure_project_paths("/projects/legacy").unwrap();
    let historical = [
        home.join("workspace/conversation-handles.json"),
        home.join("workspace/projects/legacy/conversations/old-chat/conversation.json"),
        paths.conversations_dir.join("old-chat/conversation.json"),
    ];
    for file in &historical {
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, r#"{"id":"old-chat"}"#).unwrap();
    }
    // Fresh state, then a restart over the same state.
    for _ in 0..2 {
        let store = ConversationStore::open(&home).unwrap();
        for file in &historical {
            assert_eq!(fs::read_to_string(file).unwrap(), r#"{"id":"old-chat"}"#);
        }
        let repo = ConversationRepository::new(store.clone());
        assert!(repo.conversation_summaries(None).unwrap().is_empty());
        assert!(registry
            .read_project_record("/projects/legacy")
            .unwrap()
            .is_some());
    }
}

#[test]
fn uploads_after_or_during_project_deletion_recreate_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let registry = ProjectRegistry::new(&home);

    // Pending cleanup: the upload is refused before any file is written.
    let pending = "/projects/pending-upload";
    let (store, deleted) = seeded_project(&home, pending);
    let pending_root = home.join("projects").join(&deleted.project_id);
    store
        .database()
        .fail_next_checkpoints([spark_storage::conversation::CheckpointFault::Busy]);
    assert!(store.delete_project(&deleted, &|_| {}).is_err());
    let repo = ConversationRepository::new(store.clone());
    for conversation in ["chat-a", "chat-new"] {
        assert!(repo
            .create_attachment(conversation, pending, "late.txt", b"late")
            .is_err());
    }
    assert!(!pending_root.join("conversations/chat-new").exists());
    let draft_dirs = fs::read_dir(pending_root.join("conversations/chat-a/attachments"))
        .unwrap()
        .count();
    assert_eq!(draft_dirs, 1, "only the seeded attachment remains");

    // Finished deletion: a late upload to the deleted conversation neither
    // re-registers the project nor recreates its directory.
    let finished = "/projects/finished-upload";
    let (_, deleted) = seeded_project_in(&store, &home, finished);
    store.delete_project(&deleted, &|_| {}).unwrap();
    let error = repo
        .create_attachment("chat-a-finished", finished, "late.txt", b"late")
        .unwrap_err();
    assert!(
        matches!(error, StorageError::ConversationCommitRejected { .. }),
        "{error}"
    );
    assert!(!home.join("projects").join(&deleted.project_id).exists());
    assert_eq!(registry.read_project_record(finished).unwrap(), None);
}

/// A registered project with one conversation, in an already-open store.
fn seeded_project_in(
    store: &std::sync::Arc<ConversationStore>,
    home: &std::path::Path,
    path: &str,
) -> (std::sync::Arc<ConversationStore>, DeletedProjectRecord) {
    let registry = ProjectRegistry::new(home);
    registry.register_project(path).unwrap();
    let repo = ConversationRepository::new(store.clone());
    start(&repo, "chat-a-finished", path, "turn-a", &[]).unwrap();
    settle(&repo, "chat-a-finished", path, "turn-a").unwrap();
    (store.clone(), registry.deletable_project(path).unwrap())
}

/// Registration and deletion of one project serialize on its lifecycle
/// gate: a deletion that starts while a registration runs waits for it, then
/// removes what it registered.
#[test]
fn registration_and_deletion_of_one_project_never_interleave() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let path = "/projects/contended";
    let registry = ProjectRegistry::new(&home);
    registry.register_project(path).unwrap();
    let store = ConversationStore::open(&home).unwrap();
    let deleted = registry.deletable_project(path).unwrap();
    let (inside_tx, inside_rx) = std::sync::mpsc::channel();
    let (finish_tx, finish_rx) = std::sync::mpsc::channel::<()>();
    let registering = {
        let (store, home) = (store.clone(), home.clone());
        std::thread::spawn(move || {
            store.register_project(path, || {
                inside_tx.send(()).unwrap();
                finish_rx.recv().unwrap();
                ProjectRegistry::new(&home).register_project(path)
            })
        })
    };
    inside_rx.recv().unwrap();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let deleting = {
        let store = store.clone();
        std::thread::spawn(move || {
            let result = store.delete_project(&deleted, &|_| {});
            done_tx.send(()).unwrap();
            result
        })
    };
    assert!(done_rx
        .recv_timeout(std::time::Duration::from_millis(300))
        .is_err());
    finish_tx.send(()).unwrap();
    registering.join().unwrap().unwrap();
    deleting.join().unwrap().unwrap();
    assert_eq!(registry.read_project_record(path).unwrap(), None);
    assert!(!registry.project_paths(path).unwrap().root.exists());
}

/// A download that arrives while cleanup holds the conversation waits for
/// it and reports the attachment gone, never a half-removed file.
#[test]
fn attachment_downloads_serialize_with_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let path = "/projects/download";
    let store = ConversationStore::open(&home).unwrap();
    let repo = ConversationRepository::new(store.clone());
    let draft = repo
        .create_attachment("chat", path, "draft.txt", b"draft bytes")
        .unwrap();
    let (_, bytes) = repo
        .read_attachment_bytes("chat", path, &draft.id)
        .unwrap()
        .expect("readable");
    assert_eq!(bytes, b"draft bytes");

    // Hold the database so cleanup stops inside the conversation lock.
    let (held_tx, held_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let holder = {
        let store = store.clone();
        std::thread::spawn(move || {
            store
                .database()
                .read(|_| {
                    held_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(())
                })
                .unwrap()
        })
    };
    held_rx.recv().unwrap();
    let pruning = {
        let repo = repo.clone();
        std::thread::spawn(move || {
            repo.prune_attachments(
                "chat",
                path,
                time::OffsetDateTime::now_utc() + time::Duration::hours(25),
            )
        })
    };
    std::thread::sleep(std::time::Duration::from_millis(100));
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let downloading = {
        let (repo, id) = (repo.clone(), draft.id.clone());
        std::thread::spawn(move || {
            let result = repo.read_attachment_bytes("chat", path, &id);
            done_tx.send(()).unwrap();
            result
        })
    };
    assert!(done_rx
        .recv_timeout(std::time::Duration::from_millis(300))
        .is_err());
    release_tx.send(()).unwrap();
    holder.join().unwrap();
    assert_eq!(pruning.join().unwrap().unwrap(), vec![draft.id.clone()]);
    assert_eq!(downloading.join().unwrap().unwrap(), None);
}

/// Admission is fenced before running work is cancelled: a send or a
/// provider callback arriving while cancellation runs is refused.
#[test]
fn work_arriving_during_deletion_cancellation_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let path = "/projects/cancelling";
    let (store, deleted) = seeded_project(&home, path);
    let repo = ConversationRepository::new(store.clone());
    let attempts = std::sync::Mutex::new(Vec::new());
    store
        .delete_project(&deleted, &|conversation_id| {
            attempts.lock().unwrap().push((
                conversation_id.to_string(),
                start(&repo, "chat-new", path, "turn-n", &[]).is_err(),
                record(
                    &repo,
                    conversation_id,
                    path,
                    "turn-a",
                    tool_event("late", "late"),
                )
                .is_err(),
            ));
        })
        .unwrap();
    assert_eq!(
        *attempts.lock().unwrap(),
        vec![
            ("chat-a".to_string(), true, true),
            ("chat-b".to_string(), true, true)
        ]
    );
    assert_eq!(
        store
            .database()
            .read(|read| read.conversation_location("chat-new"))
            .unwrap(),
        None
    );
    assert!(!home.join("projects").join(&deleted.project_id).exists());
}

/// A project-state update and a deletion serialize on the lifecycle gate:
/// the deletion waits for the update, then removes the updated project.
#[test]
fn project_state_updates_and_deletion_never_interleave() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let path = "/projects/updated-while-deleted";
    let registry = ProjectRegistry::new(&home);
    registry.register_project(path).unwrap();
    let store = ConversationStore::open(&home).unwrap();
    let deleted = registry.deletable_project(path).unwrap();
    let (inside_tx, inside_rx) = std::sync::mpsc::channel();
    let (finish_tx, finish_rx) = std::sync::mpsc::channel::<()>();
    let updating = {
        let (store, home) = (store.clone(), home.clone());
        std::thread::spawn(move || {
            store.update_project(path, || {
                inside_tx.send(()).unwrap();
                finish_rx.recv().unwrap();
                ProjectRegistry::new(&home).update_project_record(
                    path,
                    ProjectRecordUpdate {
                        is_favorite: Some(true),
                        ..ProjectRecordUpdate::default()
                    },
                )
            })
        })
    };
    inside_rx.recv().unwrap();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let deleting = {
        let store = store.clone();
        std::thread::spawn(move || {
            let result = store.delete_project(&deleted, &|_| {});
            done_tx.send(()).unwrap();
            result
        })
    };
    assert!(done_rx
        .recv_timeout(std::time::Duration::from_millis(300))
        .is_err());
    finish_tx.send(()).unwrap();
    assert!(updating.join().unwrap().unwrap().is_favorite);
    deleting.join().unwrap().unwrap();
    assert_eq!(registry.read_project_record(path).unwrap(), None);
    assert!(!registry.project_paths(path).unwrap().root.exists());
}
