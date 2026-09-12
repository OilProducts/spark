use std::fs;

use serde_json::json;
use spark_common::project::build_project_id;
use spark_storage::{ProjectRecordUpdate, ProjectRegistry};

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

    let handles_path = home.join("workspace/conversation-handles.json");
    fs::create_dir_all(handles_path.parent().expect("parent")).expect("parent");
    fs::write(
        &handles_path,
        serde_json::to_string_pretty(&json!({
            "schema_version": 1,
            "pattern": "adjective-noun",
            "handles": {
                "amber-anchor": {
                    "conversation_id": "conversation-1",
                    "project_id": updated.project_id,
                    "project_path": project_path,
                    "created_at": "2026-01-01T00:00:00Z"
                },
                "brisk-bank": "malformed",
                "clear-cloud": {
                    "conversation_id": "conversation-2",
                    "project_id": "other",
                    "project_path": "/projects/other",
                    "created_at": "2026-01-01T00:00:00Z"
                },
                "dawn-dust": {
                    "project_id": updated.project_id,
                    "project_path": project_path,
                    "created_at": "2026-01-01T00:00:00Z"
                }
            },
            "conversation_ids": {
                "conversation-1": "amber-anchor",
                "conversation-2": "clear-cloud"
            }
        }))
        .expect("json"),
    )
    .expect("handles");

    let deleted = registry
        .delete_project_record(project_path)
        .expect("delete");
    assert_eq!(deleted.project_id, updated.project_id);
    assert!(!home
        .join("workspace/projects")
        .join(&updated.project_id)
        .exists());

    let handles: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(handles_path).expect("handles")).expect("json");
    assert!(handles["handles"].get("amber-anchor").is_none());
    assert!(handles["handles"].get("dawn-dust").is_none());
    assert_eq!(handles["handles"]["brisk-bank"], json!("malformed"));
    assert_eq!(
        handles["conversation_ids"]["conversation-2"],
        json!("clear-cloud")
    );
    assert!(handles["conversation_ids"].get("conversation-1").is_none());
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
    let bytes = fs::read(&paths.project_file).unwrap();
    let before = fs::metadata(&paths.project_file).unwrap();
    let repo = spark_storage::ConversationRepository::new(temp.path());
    for message in ["one", "two", "three"] {
        let mut event = serde_json::from_value(json!({
            "kind": "content_delta", "channel": "assistant", "message": message
        }))
        .unwrap();
        repo.append_provider_event("conversation-1", path, "turn-1", &mut event)
            .unwrap();
        assert_metadata_untouched(&paths.project_file, &bytes, &before);
    }
    let events =
        spark_storage::ActivityRepository::new(paths.conversations_dir.join("conversation-1"))
            .read_events()
            .unwrap();
    assert_eq!(events.len(), 3);
    for (event, message) in events.iter().zip(["one", "two", "three"]) {
        assert_eq!(event.event["type"], "provider_event");
        assert_eq!(event.event["event"]["message"], message);
    }
}

#[test]
fn conversation_operations_initialize_unregistered_projects() {
    let temp = tempfile::tempdir().unwrap();
    let registry = ProjectRegistry::new(temp.path());
    let repo = spark_storage::ConversationRepository::new(temp.path());
    for operation in ["read", "list", "delete", "trace", "provider", "session"] {
        let path = format!("/projects/unregistered-{operation}");
        assert_eq!(registry.read_project_record(&path).unwrap(), None);
        match operation {
            "read" => assert_eq!(
                repo.read_snapshot("conversation", Some(&path)).unwrap(),
                None
            ),
            "list" => assert!(repo
                .list_conversation_ids_for_project(&path)
                .unwrap()
                .is_empty()),
            "delete" => repo.delete_conversation("conversation", &path).unwrap(),
            "trace" => repo
                .append_codex_jsonrpc_trace("conversation", &path, "outbound", "hello")
                .unwrap(),
            "provider" => {
                let mut event = serde_json::from_value(
                    json!({"kind": "content_delta", "channel": "assistant", "message": "hello"}),
                )
                .unwrap();
                repo.append_provider_event("conversation", &path, "turn", &mut event)
                    .unwrap();
            }
            "session" => {
                let session = spark_storage::conversation::RuntimeSession {
                    schema_version: spark_storage::conversation::RUNTIME_SESSION_SCHEMA_VERSION,
                    provider: "codex_app_server".into(),
                    thread_id: Some("thread".into()),
                    established_at: "2001-01-01T00:00:00Z".into(),
                    last_turn_id: None,
                    resume_failed: false,
                    updated_at: "2001-01-01T00:00:00Z".into(),
                };
                repo.write_runtime_session("conversation", &path, &session)
                    .unwrap();
                assert_eq!(
                    repo.read_runtime_session("conversation", Some(&path))
                        .unwrap(),
                    Some(session)
                );
            }
            _ => unreachable!(),
        }
        assert!(registry.read_project_record(&path).unwrap().is_some());
    }
}
