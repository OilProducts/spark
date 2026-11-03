use std::fs;
use std::io::Write;

use serde_json::json;
use spark_common::project::build_project_id;
use spark_storage::conversation::{CatchUp, ChangeScope, ConversationMutation, TranscriptTurn};
use spark_storage::{
    ConversationRepository, ConversationStore, DeletedProjectRecord, ProjectRecordUpdate,
    ProjectRegistry, StorageError,
};

#[test]
fn project_registry_registers_lists_and_preserves_toml_shape() {
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
    let project_file = home
        .join("workspace/projects")
        .join(&record.project_id)
        .join("project.toml");
    let text = fs::read_to_string(project_file).expect("project toml");
    let lines = text.lines().take(6).collect::<Vec<_>>();
    assert_eq!(lines[0], format!("project_id = \"{}\"", record.project_id));
    assert_eq!(
        lines[1],
        format!("project_path = \"{}\"", project_dir.display())
    );
    assert_eq!(lines[2], "display_name = \"Registered Project\"");
    assert!(lines[3].starts_with("created_at = \""));
    assert!(lines[4].starts_with("last_opened_at = \""));
    assert_eq!(lines[5], "is_favorite = false");
    assert!(home
        .join("workspace/projects")
        .join(&record.project_id)
        .join("flow-run-requests")
        .is_dir());

    assert_eq!(registry.list_project_records().expect("list"), vec![record]);
}

#[test]
fn project_registry_reads_python_created_record_with_missing_optionals() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    let project_path = temp.path().join("python-created");
    let project_id = build_project_id(project_path.to_str().expect("utf-8")).expect("project id");
    let root = home.join("workspace/projects").join(&project_id);
    fs::create_dir_all(&root).expect("root");
    fs::write(
        root.join("project.toml"),
        format!(
            "project_id = \"{project_id}\"\nproject_path = \"{}\"\ndisplay_name = \"Python Created\"\ncreated_at = \"2026-01-01T00:00:00Z\"\nlast_opened_at = \"2026-01-01T00:00:01Z\"\nis_favorite = false\n",
            project_path.display()
        ),
    )
    .expect("project toml");

    let record = ProjectRegistry::new(home)
        .read_project_record_by_id(&project_id)
        .expect("read")
        .expect("record");

    assert_eq!(record.display_name, "Python Created");
    assert_eq!(record.last_accessed_at, None);
    assert_eq!(record.active_conversation_id, None);
    assert_eq!(record.execution_profile_id, None);
}

#[test]
fn project_registry_updates_optional_state_and_deletes_project_handles() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    let project_path = "/projects/my-app";
    let registry = ProjectRegistry::new(&home);
    let record = registry.register_project(project_path).expect("register");
    assert_eq!(record.project_path, project_path);

    let updated = registry
        .update_project_record(
            project_path,
            ProjectRecordUpdate {
                last_accessed_at: Some(Some("2026-02-03T04:05:06Z".to_string())),
                is_favorite: Some(true),
                active_conversation_id: Some(Some("conversation-1".to_string())),
                execution_profile_id: Some(Some("native".to_string())),
                expected_revision: Some(
                    spark_storage::settings::read_settings_document(
                        &registry.project_paths(project_path).unwrap().project_file,
                    )
                    .unwrap()
                    .revision,
                ),
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

    // Validation alone never removes anything; the store removes the
    // directory after tombstoning the project's conversations.
    let deleted = registry.deletable_project(project_path).expect("deletable");
    assert_eq!(deleted.project_id, updated.project_id);
    let project_root = home.join("workspace/projects").join(&updated.project_id);
    assert!(project_root.exists());
    let store = ConversationStore::open(&home).expect("store");
    store.delete_project(&deleted, &|_| {}).expect("delete");
    assert!(!project_root.exists());
    assert_eq!(registry.read_project_record(project_path).unwrap(), None);
}

fn seed_historical_metadata(registry: &ProjectRegistry, path: &str) -> spark_storage::ProjectPaths {
    let paths = registry.ensure_project_paths(path).expect("initialize");
    let mut payload: toml::Value = fs::read_to_string(&paths.project_file)
        .unwrap()
        .parse()
        .unwrap();
    for (key, value) in [
        ("display_name", "Custom project name"),
        ("created_at", "2001-01-01T00:00:00Z"),
        ("last_opened_at", "2002-01-01T00:00:00Z"),
        ("last_accessed_at", "2003-01-01T00:00:00Z"),
        ("active_conversation_id", "conversation-1"),
        ("execution_profile_id", "native"),
    ] {
        payload
            .as_table_mut()
            .unwrap()
            .insert(key.into(), value.into());
    }
    payload["is_favorite"] = true.into();
    fs::write(&paths.project_file, toml::to_string(&payload).unwrap()).unwrap();
    // A fixed old mtime detects writes even on filesystems with coarse timestamps.
    fs::File::options()
        .write(true)
        .open(&paths.project_file)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000))
        .unwrap();
    paths
}

fn assert_metadata_untouched(path: &std::path::Path, bytes: &[u8], before: &fs::Metadata) {
    assert_eq!(fs::read(path).unwrap(), bytes);
    let after = fs::metadata(path).unwrap();
    assert_eq!(after.modified().unwrap(), before.modified().unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!((after.dev(), after.ino()), (before.dev(), before.ino()));
    }
}

#[test]
fn initialization_recreates_directories_and_reads_preserve_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let registry = ProjectRegistry::new(temp.path());
    let path = "/projects/idempotent";
    assert_eq!(registry.read_project_record(path).unwrap(), None);
    assert!(!registry.workspace_root().exists());
    let paths = seed_historical_metadata(&registry, path);
    let bytes = fs::read(&paths.project_file).unwrap();
    let before = fs::metadata(&paths.project_file).unwrap();
    let record = registry.read_project_record(path).unwrap().unwrap();
    assert_metadata_untouched(&paths.project_file, &bytes, &before);
    for directory in [
        &paths.conversations_dir,
        &paths.flow_run_requests_dir,
        &paths.flow_launches_dir,
        &paths.proposed_plans_dir,
    ] {
        assert!(directory.is_dir());
        fs::remove_dir(directory).unwrap();
        assert_eq!(
            registry.read_project_record(path).unwrap(),
            Some(record.clone())
        );
        assert!(!directory.exists());
        registry.ensure_project_paths(path).unwrap();
        assert!(directory.is_dir());
        assert_metadata_untouched(&paths.project_file, &bytes, &before);
    }
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
fn initialization_repairs_records_and_preserves_legacy_defaults() {
    let temp = tempfile::tempdir().unwrap();
    let registry = ProjectRegistry::new(temp.path());
    let path = "/projects/repair";
    let paths = seed_historical_metadata(&registry, path);
    let expected = registry.read_project_record(path).unwrap().unwrap();
    let mut payload: toml::Value = fs::read_to_string(&paths.project_file)
        .unwrap()
        .parse()
        .unwrap();
    payload.as_table_mut().unwrap().remove("project_id");
    fs::write(&paths.project_file, toml::to_string(&payload).unwrap()).unwrap();
    registry.ensure_project_paths(path).unwrap();
    assert_eq!(registry.read_project_record(path).unwrap(), Some(expected));

    payload.as_table_mut().unwrap().remove("created_at");
    payload["last_opened_at"] = 42.into();
    payload["active_conversation_id"] = false.into();
    fs::write(&paths.project_file, toml::to_string(&payload).unwrap()).unwrap();
    registry.ensure_project_paths(path).unwrap();
    let repaired = registry.read_project_record(path).unwrap().unwrap();
    assert_eq!(repaired.display_name, "Custom project name");
    assert_eq!(repaired.execution_profile_id.as_deref(), Some("native"));
    assert!(repaired.is_favorite);
    assert_eq!(repaired.active_conversation_id, None);
    assert!(!repaired.created_at.is_empty());
    assert_eq!(repaired.created_at, repaired.last_opened_at);

    let valid = fs::read(&paths.project_file).unwrap();
    fs::write(&paths.project_file, "not valid toml [").unwrap();
    // Project metadata now owns authored configuration; invalid TOML cannot be
    // repaired by silently dropping settings and reverting to built-in defaults.
    assert!(registry.ensure_project_paths(path).is_err());
    assert_eq!(
        fs::read_to_string(&paths.project_file).unwrap(),
        "not valid toml ["
    );
    fs::write(&paths.project_file, valid).unwrap();

    let mut payload: toml::Value = fs::read_to_string(&paths.project_file)
        .unwrap()
        .parse()
        .unwrap();
    payload.as_table_mut().unwrap().remove("is_favorite");
    let bytes = toml::to_string(&payload).unwrap().into_bytes();
    fs::write(&paths.project_file, &bytes).unwrap();
    let before = fs::metadata(&paths.project_file).unwrap();
    registry.ensure_project_paths(path).unwrap();
    assert_metadata_untouched(&paths.project_file, &bytes, &before);
    assert!(
        !registry
            .read_project_record(path)
            .unwrap()
            .unwrap()
            .is_favorite
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
    assert!(!registry.workspace_root().exists());
}

#[test]
fn provider_events_persist_without_replacing_project_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let registry = ProjectRegistry::new(temp.path());
    let path = "/projects/provider-events";
    let paths = seed_historical_metadata(&registry, path);
    let repo = ConversationRepository::new(ConversationStore::open(temp.path()).unwrap());
    repo.commit_conversation(
        "conversation-1",
        path,
        0,
        vec![assistant("turn-1", "streaming")],
    )
    .unwrap();
    let bytes = fs::read(&paths.project_file).unwrap();
    let before = fs::metadata(&paths.project_file).unwrap();
    for message in ["one", "two", "three"] {
        let mut event = serde_json::from_value(json!({
            "kind": "content_delta", "channel": "assistant", "message": message
        }))
        .unwrap();
        repo.append_provider_event("conversation-1", path, "turn-1", &mut event)
            .unwrap();
        assert_metadata_untouched(&paths.project_file, &bytes, &before);
    }
    let events = repo
        .unapplied_provider_events("conversation-1", path, "turn-1")
        .unwrap();
    assert_eq!(
        events
            .iter()
            .map(|record| record.sequence)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    for (event, message) in events.iter().zip(["one", "two", "three"]) {
        assert_eq!(event.event["message"], message);
    }
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
    let mut event = serde_json::from_value(
        json!({"kind": "content_delta", "channel": "assistant", "message": "hello"}),
    )
    .unwrap();
    assert!(repo
        .append_provider_event("conversation", path, "turn", &mut event)
        .is_err());
    assert!(repo
        .write_runtime_session("conversation", path, &session())
        .is_err());
    assert_eq!(registry.read_project_record(path).unwrap(), None);
    // The first commit of a new conversation registers its project.
    repo.commit_conversation("conversation", path, 0, vec![assistant("turn", "complete")])
        .unwrap();
    assert!(registry.read_project_record(path).unwrap().is_some());
    repo.write_runtime_session("conversation", path, &session())
        .unwrap();
    assert_eq!(
        repo.read_runtime_session("conversation", Some(path))
            .unwrap(),
        Some(session())
    );
}

fn session() -> spark_storage::conversation::RuntimeSession {
    spark_storage::conversation::RuntimeSession {
        schema_version: spark_storage::conversation::RUNTIME_SESSION_SCHEMA_VERSION,
        provider: "codex_app_server".into(),
        thread_id: Some("thread".into()),
        established_at: "2001-01-01T00:00:00Z".into(),
        last_turn_id: None,
        resume_failed: false,
        updated_at: "2001-01-01T00:00:00Z".into(),
    }
}

fn assistant(id: &str, status: &str) -> ConversationMutation {
    ConversationMutation::TurnUpserted {
        turn: TranscriptTurn {
            id: id.into(),
            role: "assistant".into(),
            status: status.into(),
            ..TranscriptTurn::default()
        },
    }
}

fn user(id: &str, attachments: &[&str]) -> ConversationMutation {
    let mut turn = TranscriptTurn {
        id: id.into(),
        role: "user".into(),
        content: "hello".into(),
        status: "complete".into(),
        ..TranscriptTurn::default()
    };
    if !attachments.is_empty() {
        turn.extra.insert(
            "attachments".into(),
            json!(attachments
                .iter()
                .map(|id| json!({"id": id}))
                .collect::<Vec<_>>()),
        );
    }
    ConversationMutation::TurnUpserted { turn }
}

/// A registered project with two conversations, an attachment and a trace.
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
    repo.commit_conversation(
        "chat-a",
        path,
        0,
        vec![
            user("turn-user", &[&attachment.id]),
            assistant("turn-a", "streaming"),
        ],
    )
    .unwrap();
    repo.commit_conversation("chat-b", path, 0, vec![assistant("turn-b", "complete")])
        .unwrap();
    let mut event = serde_json::from_value(
        json!({"kind": "content_delta", "channel": "assistant", "message": "partial"}),
    )
    .unwrap();
    repo.append_provider_event("chat-a", path, "turn-a", &mut event)
        .unwrap();
    let deleted = registry.deletable_project(path).unwrap();
    (store, deleted)
}

#[test]
fn project_deletion_tombstones_everything_then_removes_the_directory() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let path = "/projects/doomed";
    let (store, deleted) = seeded_project(&home, path);
    let repo = ConversationRepository::new(store.clone());
    let handle = repo.conversation_handle("chat-a", None).unwrap();
    let project_root = home.join("workspace/projects").join(&deleted.project_id);
    assert!(project_root
        .join("conversations/chat-a/provider-trace.jsonl")
        .exists());

    let cancelled = std::sync::Mutex::new(Vec::new());
    store
        .delete_project(&deleted, &|id| {
            cancelled.lock().unwrap().push(id.to_string())
        })
        .unwrap();
    assert_eq!(*cancelled.lock().unwrap(), vec!["chat-a", "chat-b"]);
    assert!(!project_root.exists());
    assert_eq!(store.pending_project_deletion(path).unwrap(), None);
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
    let mut event = serde_json::from_value(
        json!({"kind": "content_delta", "channel": "assistant", "message": "late"}),
    )
    .unwrap();
    assert!(repo
        .append_provider_event("chat-a", path, "turn-a", &mut event)
        .is_err());
    assert!(repo
        .commit_conversation("chat-a", path, 0, vec![assistant("turn-a", "complete")])
        .is_err());
    assert!(repo
        .write_runtime_session("chat-a", path, &session())
        .is_err());
    assert!(!project_root.exists());

    // Re-registration starts fresh; the deleted handle is free again but a
    // deleted conversation id never comes back.
    ProjectRegistry::new(&home).register_project(path).unwrap();
    assert!(repo
        .commit_conversation("chat-a", path, 0, vec![assistant("turn-new", "complete")])
        .is_err());
    assert_eq!(
        repo.conversation_handle("chat-new", Some(&handle)).unwrap(),
        handle
    );
    let fresh = repo
        .commit_conversation("chat-new", path, 0, vec![assistant("turn-new", "complete")])
        .unwrap();
    assert_eq!(fresh.record.meta.conversation_handle, handle);
}

#[test]
fn deferred_project_cleanup_keeps_every_file_until_restart_finishes_it() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let path = "/projects/deferred";
    let (store, deleted) = seeded_project(&home, path);
    let project_root = home.join("workspace/projects").join(&deleted.project_id);
    // The checkpoint barrier reports busy: the commit is durable in SQL, but
    // no file may be removed yet.
    store.database().defer_next_checkpoints(1);
    let error = store.delete_project(&deleted, &|_| {}).unwrap_err();
    assert!(
        matches!(error, StorageError::CleanupDeferred { .. }),
        "{error}"
    );
    assert!(project_root
        .join("conversations/chat-a/attachments")
        .exists());
    assert_eq!(
        store.pending_project_deletion(path).unwrap(),
        Some(deleted.clone())
    );
    // Nothing new is admitted while cleanup is pending.
    let repo = ConversationRepository::new(store.clone());
    assert!(repo
        .commit_conversation("chat-c", path, 0, vec![assistant("turn-c", "complete")])
        .is_err());
    drop(repo);
    drop(store);

    let store = ConversationStore::open(&home).unwrap();
    assert!(!project_root.exists());
    assert_eq!(store.pending_project_deletion(path).unwrap(), None);
}

#[cfg(unix)]
#[test]
fn failed_directory_removal_keeps_the_marker_and_is_retried() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let path = "/projects/stuck";
    let (store, deleted) = seeded_project(&home, path);
    let conversations = home
        .join("workspace/projects")
        .join(&deleted.project_id)
        .join("conversations");
    fs::set_permissions(&conversations, fs::Permissions::from_mode(0o555)).unwrap();
    let error = store.delete_project(&deleted, &|_| {}).unwrap_err();
    assert!(matches!(error, StorageError::Io { .. }), "{error}");
    assert_eq!(
        store.pending_project_deletion(path).unwrap(),
        Some(deleted.clone())
    );
    // Registration of the same path cannot proceed until cleanup finishes.
    assert!(store.finish_project_deletion(path).is_err());
    fs::set_permissions(&conversations, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(store.finish_project_deletion(path).unwrap());
    assert!(!conversations.exists());
    assert_eq!(store.pending_project_deletion(path).unwrap(), None);
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
            let mut event = serde_json::from_value(
                json!({"kind": "content_delta", "channel": "assistant", "message": format!("{index}")}),
            )
            .unwrap();
            if let Err(error) = repo.append_provider_event("chat-a", path, "turn-a", &mut event) {
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
    assert!(!home
        .join("workspace/projects")
        .join(&deleted.project_id)
        .exists());
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
    let error = repo
        .commit_conversation("chat", path, 0, vec![user("turn-user", &[&attachment.id])])
        .unwrap_err();
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
fn first_open_wipes_only_legacy_conversations_and_the_handle_index() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let registry = ProjectRegistry::new(&home);
    let paths = registry.ensure_project_paths("/projects/legacy").unwrap();
    let legacy = paths.conversations_dir.join("old-chat");
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("conversation.json"), "{}").unwrap();
    fs::create_dir_all(&paths.flow_launches_dir).unwrap();
    let handles = home.join("workspace/conversation-handles.json");
    fs::write(&handles, "{}").unwrap();
    let store = ConversationStore::open(&home).unwrap();
    assert!(!paths.conversations_dir.exists());
    assert!(!handles.exists());
    assert!(paths.project_file.exists());
    assert!(paths.flow_launches_dir.exists());
    drop(store);
    // Later opens never wipe again.
    fs::create_dir_all(&legacy).unwrap();
    ConversationStore::open(&home).unwrap();
    assert!(legacy.exists());
}

#[test]
fn uploads_after_or_during_project_deletion_recreate_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let registry = ProjectRegistry::new(&home);

    // Pending cleanup: the upload is refused before any file is written.
    let pending = "/projects/pending-upload";
    let (store, deleted) = seeded_project(&home, pending);
    let pending_root = home.join("workspace/projects").join(&deleted.project_id);
    store.database().defer_next_checkpoints(1);
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
    assert!(!home
        .join("workspace/projects")
        .join(&deleted.project_id)
        .exists());
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
    ConversationRepository::new(store.clone())
        .commit_conversation(
            "chat-a-finished",
            path,
            0,
            vec![assistant("turn-a", "complete")],
        )
        .unwrap();
    (store.clone(), registry.deletable_project(path).unwrap())
}

#[test]
fn an_interrupted_trace_append_is_repaired_before_the_next_one() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let path = "/projects/trace-tail";
    let repo = ConversationRepository::new(ConversationStore::open(&home).unwrap());
    repo.commit_conversation("chat", path, 0, vec![assistant("turn-a", "streaming")])
        .unwrap();
    let mut first = serde_json::from_value(
        json!({"kind": "content_delta", "channel": "assistant", "message": "kept"}),
    )
    .unwrap();
    repo.append_provider_event("chat", path, "turn-a", &mut first)
        .unwrap();
    let trace = ProjectRegistry::new(&home)
        .project_paths(path)
        .unwrap()
        .conversations_dir
        .join("chat/provider-trace.jsonl");
    // A crash mid-append leaves an unterminated, unparsable tail.
    fs::OpenOptions::new()
        .append(true)
        .open(&trace)
        .unwrap()
        .write_all(br#"{"sequence":2,"turn_id":"#)
        .unwrap();
    drop(repo);

    let reopen = || ConversationRepository::new(ConversationStore::open(&home).unwrap());
    let repo = reopen();
    let sequences = |repo: &ConversationRepository| {
        repo.unapplied_provider_events("chat", path, "turn-a")
            .unwrap()
            .iter()
            .map(|record| record.sequence)
            .collect::<Vec<_>>()
    };
    assert_eq!(sequences(&repo), vec![1]);
    let mut next = serde_json::from_value(
        json!({"kind": "content_delta", "channel": "assistant", "message": "after"}),
    )
    .unwrap();
    let appended = repo
        .append_provider_event("chat", path, "turn-a", &mut next)
        .unwrap();
    assert_eq!(appended.sequence, 2);
    assert_eq!(sequences(&repo), vec![1, 2]);
    drop(repo);
    assert_eq!(sequences(&reopen()), vec![1, 2]);

    // Corruption before the final record stays an explicit recovery error.
    let mut bytes = b"{\"broken\n".to_vec();
    bytes.extend(fs::read(&trace).unwrap());
    fs::write(&trace, bytes).unwrap();
    assert!(matches!(
        reopen().read_snapshot("chat", Some(path)),
        Err(StorageError::JsonlLine { line: 1, .. })
    ));
}
