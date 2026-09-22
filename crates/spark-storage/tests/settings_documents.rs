use std::fs;
use std::path::Path;
use std::process::Command;

use spark_storage::settings::{read_settings_document, update_settings_section};
use spark_storage::StorageError;

fn section(value: i64) -> toml::Value {
    toml::Value::Table(toml::Table::from_iter([("value".into(), value.into())]))
}

#[test]
fn section_updates_preserve_other_values_and_reject_stale_revisions() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("spark.toml");
    fs::write(
        &path,
        "[models]\nmodel = 'existing'\n[extension]\ncustom = [1, 2]\n",
    )
    .unwrap();
    let original = read_settings_document(&path).unwrap();
    let saved = update_settings_section(
        &path,
        &original.revision,
        "runtime",
        Some(section(1)),
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(saved.values["models"], original.values["models"]);
    assert_eq!(saved.values["extension"], original.values["extension"]);
    assert_ne!(saved.revision, original.revision);
    assert!(matches!(
        update_settings_section(
            &path,
            &original.revision,
            "runtime",
            Some(section(2)),
            |_| Ok(())
        ),
        Err(StorageError::SettingsConflict { .. })
    ));
    assert_eq!(
        read_settings_document(&path).unwrap().revision,
        saved.revision
    );
    fs::write(&path, "[runtime]\nvalue = 3\n").unwrap();
    assert!(matches!(
        update_settings_section(&path, &saved.revision, "runtime", None, |_| Ok(())),
        Err(StorageError::SettingsConflict { .. })
    ));
}

#[test]
fn invalid_saves_leave_original_bytes_untouched() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("spark.toml");
    let bytes = "# preserve on failure\n[runtime]\nvalue = 1\n";
    fs::write(&path, bytes).unwrap();
    let document = read_settings_document(&path).unwrap();
    assert!(update_settings_section(
        &path,
        &document.revision,
        "runtime",
        Some(section(2)),
        |_| Err(StorageError::SettingsValidation {
            path: path.clone(),
            reason: "invalid runtime".into()
        })
    )
    .is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), bytes);
}

#[test]
fn malformed_documents_report_location_without_echoing_values() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("spark.toml");
    fs::write(&path, "token = SECRET_DO_NOT_EXPOSE\n").unwrap();
    let error = read_settings_document(&path).unwrap_err().to_string();
    assert!(error.contains("spark.toml"));
    assert!(error.contains("byte"));
    assert!(!error.contains("SECRET_DO_NOT_EXPOSE"));
    fs::write(&path, "runtime = 'SECRET_DO_NOT_EXPOSE'\n").unwrap();
    let document = read_settings_document(&path).unwrap();
    let error = document
        .section::<spark_common::settings::RuntimeSettings>(&path, "runtime")
        .unwrap_err()
        .to_string();
    assert!(!error.contains("SECRET_DO_NOT_EXPOSE"));
}

#[test]
fn settings_writer_child() {
    let Some(path) = std::env::var_os("SPARK_TEST_SETTINGS_DOCUMENT") else {
        return;
    };
    let revision = std::env::var("SPARK_TEST_SETTINGS_REVISION").unwrap();
    match update_settings_section(
        Path::new(&path),
        &revision,
        "runtime",
        Some(section(1)),
        |_| Ok(()),
    ) {
        Ok(_) => {}
        Err(StorageError::SettingsConflict { .. }) => std::process::exit(23),
        Err(error) => panic!("{error}"),
    }
}

#[test]
fn separate_processes_cannot_overwrite_the_same_revision() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("config/spark.toml");
    let original = read_settings_document(&path).unwrap();
    let spawn = || {
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "settings_writer_child"])
            .env("SPARK_TEST_SETTINGS_DOCUMENT", &path)
            .env("SPARK_TEST_SETTINGS_REVISION", &original.revision)
            .spawn()
            .unwrap()
    };
    let mut first = spawn();
    let mut second = spawn();
    let mut results = [
        first.wait().unwrap().code().unwrap(),
        second.wait().unwrap().code().unwrap(),
    ];
    results.sort();
    assert_eq!(results, [0, 23]);
}

#[test]
fn desktop_migration_can_retry_source_backup_failures_without_partial_core_updates() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("spark.toml");
    let legacy = root.path().join("spark-desktop.json");
    let backup = root.path().join("spark-desktop.json.v0.bak");
    let original = b"{\"remote_access_enabled\":true}\n";
    fs::write(&legacy, original).unwrap();
    fs::write(&backup, b"different").unwrap();
    assert!(spark_storage::settings::migrate_desktop_settings(&core, &legacy).is_err());
    assert!(!core.exists());
    fs::write(&backup, original).unwrap();
    let migrated = spark_storage::settings::migrate_desktop_settings(&core, &legacy).unwrap();
    assert_eq!(
        migrated.values["desktop"]["remote_access_enabled"].as_bool(),
        Some(true)
    );
    assert_eq!(
        spark_storage::settings::migrate_desktop_settings(&core, &legacy)
            .unwrap()
            .revision,
        migrated.revision
    );
}

#[test]
fn every_legacy_conversation_migrates_with_backups_without_rewriting_history() {
    let root = tempfile::tempdir().unwrap();
    let registry = spark_storage::ProjectRegistry::new(root.path());
    let project = registry
        .ensure_project_paths("/projects/migration")
        .unwrap();
    let mut originals = Vec::new();
    for (id, mode) in [("unopened", "plan"), ("active", "chat")] {
        let dir = project.conversations_dir.join(id);
        fs::create_dir_all(&dir).unwrap();
        let mut meta =
            spark_storage::conversation::ConversationMeta::new(id, "/projects/migration");
        meta.chat_mode = mode.into();
        meta.provider = "anthropic".into();
        meta.model = Some("legacy-model".into());
        let mut value = serde_json::to_value(meta).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("settings_schema_version");
        value.as_object_mut().unwrap().remove("model_settings");
        value["future_metadata"] = serde_json::json!({"preserve": true});
        let bytes = serde_json::to_vec_pretty(&value).unwrap();
        fs::write(dir.join("conversation.json"), &bytes).unwrap();
        fs::write(
            dir.join("transcript.jsonl"),
            "historical model selectors stay here\n",
        )
        .unwrap();
        fs::write(dir.join("events.jsonl"), "historical events stay here\n").unwrap();
        originals.push((dir, bytes, mode));
    }
    spark_storage::settings::migrate_workspace_conversation_settings(root.path()).unwrap();
    for (dir, bytes, mode) in &originals {
        assert_eq!(
            fs::read(dir.join("conversation.json.settings-v0.bak")).unwrap(),
            *bytes
        );
        let migrated: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join("conversation.json")).unwrap()).unwrap();
        assert_eq!(migrated["settings_schema_version"], 1);
        assert!(migrated["model_settings"].is_null());
        assert_eq!(migrated["chat_mode"], *mode);
        assert_eq!(migrated["future_metadata"]["preserve"], true);
        assert_eq!(
            fs::read_to_string(dir.join("transcript.jsonl")).unwrap(),
            "historical model selectors stay here\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join("events.jsonl")).unwrap(),
            "historical events stay here\n"
        );
    }
    let before: Vec<_> = originals
        .iter()
        .map(|(dir, _, _)| fs::read(dir.join("conversation.json")).unwrap())
        .collect();
    spark_storage::settings::migrate_workspace_conversation_settings(root.path()).unwrap();
    for ((dir, _, _), before) in originals.iter().zip(before) {
        assert_eq!(fs::read(dir.join("conversation.json")).unwrap(), before);
    }
}

#[test]
fn newer_core_versions_cannot_be_read_or_overwritten_by_any_settings_writer() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("spark.toml");
    let bytes = b"schema_version = 2\n[runtime]\nproject_roots = []\n";
    fs::write(&path, bytes).unwrap();
    assert!(read_settings_document(&path).is_err());
    assert!(
        update_settings_section(&path, "absent", "runtime", Some(section(2)), |_| Ok(())).is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn conversation_migration_retries_backups_and_rejects_future_versions() {
    let root = tempfile::tempdir().unwrap();
    let registry = spark_storage::ProjectRegistry::new(root.path());
    let project = registry.ensure_project_paths("/projects/retry").unwrap();
    let dir = project.conversations_dir.join("chat");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("conversation.json");
    let original =
        b"{\"settings_schema_version\":0,\"chat_mode\":\"plan\",\"provider\":\"openai\"}\n";
    fs::write(&path, original).unwrap();
    let backup = dir.join("conversation.json.settings-v0.bak");
    fs::write(&backup, "wrong backup").unwrap();
    assert!(spark_storage::settings::migrate_workspace_conversation_settings(root.path()).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    fs::write(&backup, original).unwrap();
    spark_storage::settings::migrate_workspace_conversation_settings(root.path()).unwrap();
    fs::write(&path, "{\"settings_schema_version\":2}").unwrap();
    assert!(spark_storage::settings::migrate_workspace_conversation_settings(root.path()).is_err());
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "{\"settings_schema_version\":2}"
    );
}

#[test]
fn project_operational_writer_child() {
    let Some(home) = std::env::var_os("SPARK_TEST_PROJECT_LOCK_HOME") else {
        return;
    };
    fs::write(Path::new(&home).join("child-ready"), b"ready").unwrap();
    spark_storage::ProjectRegistry::new(home)
        .update_project_record(
            "/projects/locked",
            spark_storage::ProjectRecordUpdate {
                is_favorite: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
}

#[test]
fn project_operational_updates_read_under_the_settings_document_lock() {
    use fs2::FileExt;
    use std::time::{Duration, Instant};
    let root = tempfile::tempdir().unwrap();
    let registry = spark_storage::ProjectRegistry::new(root.path());
    registry.register_project("/projects/locked").unwrap();
    let path = registry
        .project_paths("/projects/locked")
        .unwrap()
        .project_file;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path.with_extension("toml.lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "project_operational_writer_child", "--nocapture"])
        .env("SPARK_TEST_PROJECT_LOCK_HOME", root.path())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !root.path().join("child-ready").exists() {
        assert!(Instant::now() < deadline, "child did not start");
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        child.try_wait().unwrap().is_none(),
        "writer bypassed document lock"
    );
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str("execution_profile_id = 'new-profile'\n[model_settings]\nprovider = 'openai'\n[extension]\nkeep = true\n");
    spark_storage::write_text_atomic(&path, text).unwrap();
    drop(lock);
    assert!(child.wait().unwrap().success());
    let record = registry
        .read_project_record("/projects/locked")
        .unwrap()
        .unwrap();
    assert!(record.is_favorite);
    assert_eq!(record.execution_profile_id.as_deref(), Some("new-profile"));
    let document = read_settings_document(&path).unwrap();
    assert_eq!(
        document.values["model_settings"]["provider"].as_str(),
        Some("openai")
    );
    assert_eq!(document.values["extension"]["keep"].as_bool(), Some(true));
}

#[test]
fn core_and_desktop_bootstrap_order_preserves_authority_and_backups() {
    use spark_storage::settings::{migrate_core_settings, migrate_desktop_settings};
    for desktop_first in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let core = root.path().join("spark.toml");
        let legacy = root.path().join("spark-desktop.json");
        let original = b"# original\n[runtime]\nflows_dir = '/chosen'\n[extension]\nkeep = true\n";
        let source = b"{\"remote_access_enabled\":true}\n";
        fs::write(&core, original).unwrap();
        fs::write(&legacy, source).unwrap();
        if !desktop_first {
            migrate_core_settings(&core).unwrap();
        }
        let imported = migrate_desktop_settings(&core, &legacy).unwrap();
        assert_eq!(
            imported.values["desktop"]["remote_access_enabled"].as_bool(),
            Some(true)
        );
        assert_eq!(
            imported.values["runtime"]["flows_dir"].as_str(),
            Some("/chosen")
        );
        assert_eq!(imported.values["extension"]["keep"].as_bool(), Some(true));
        assert_eq!(
            fs::read(root.path().join("spark.toml.v0.bak")).unwrap(),
            original
        );
        assert_eq!(
            fs::read(root.path().join("spark-desktop.json.v0.bak")).unwrap(),
            source
        );
        assert_eq!(
            migrate_core_settings(&core).unwrap().revision,
            imported.revision
        );
        assert_eq!(
            migrate_desktop_settings(&core, &legacy).unwrap().revision,
            imported.revision
        );
    }
}

#[test]
fn core_bootstrap_validates_all_known_sections_before_versioning() {
    use spark_storage::settings::migrate_core_settings;
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("spark.toml");
    for invalid in [
        "[models]\nprovider = 'codex'\nllm_profile = 'conflict'\n",
        "[runtime]\nflows_dir = 123\n",
        "[desktop]\nremote_access_enabled = 'SECRET_DO_NOT_EXPOSE'\n",
    ] {
        fs::write(&core, invalid).unwrap();
        let error = migrate_core_settings(&core).unwrap_err().to_string();
        assert!(!error.contains("SECRET_DO_NOT_EXPOSE"));
        assert_eq!(fs::read_to_string(&core).unwrap(), invalid);
        assert!(!root.path().join("spark.toml.v0.bak").exists());
    }
}

#[test]
fn desktop_save_rejects_invalid_unrelated_core_sections_without_writing() {
    use spark_common::settings::DesktopSettings;
    use spark_storage::settings::update_desktop_settings;
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("spark.toml");
    let original = "schema_version = 1\n[models]\nprovider = 'codex'\nllm_profile = 'conflict'\n";
    fs::write(&core, original).unwrap();
    let document = read_settings_document(&core).unwrap();
    assert!(
        update_desktop_settings(&core, &document.revision, &DesktopSettings::default()).is_err()
    );
    assert_eq!(fs::read_to_string(&core).unwrap(), original);
}

#[test]
fn desktop_import_marker_prevents_reimport_after_section_removal() {
    use spark_storage::settings::{migrate_core_settings, migrate_desktop_settings};
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("spark.toml");
    let legacy = root.path().join("spark-desktop.json");
    fs::write(&legacy, "{\"remote_access_enabled\":true}").unwrap();
    migrate_core_settings(&core).unwrap();
    let imported = migrate_desktop_settings(&core, &legacy).unwrap();
    let cleared =
        update_settings_section(&core, &imported.revision, "desktop", None, |_| Ok(())).unwrap();
    let again = migrate_desktop_settings(&core, &legacy).unwrap();
    assert_eq!(again.revision, cleared.revision);
    assert!(!again.values.contains_key("desktop"));
}

#[test]
fn client_identity_is_persisted_and_invalid_existing_identity_is_not_replaced() {
    use spark_storage::settings::load_or_create_client_identity;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("app-config/client-id");
    let id = load_or_create_client_identity(&path).unwrap();
    assert!(id.starts_with("desktop-"));
    assert_eq!(id, load_or_create_client_identity(&path).unwrap());
    assert_ne!(
        id,
        load_or_create_client_identity(&root.path().join("other/client-id")).unwrap()
    );
    fs::write(&path, "../../SECRET_DO_NOT_EXPOSE").unwrap();
    let error = load_or_create_client_identity(&path)
        .unwrap_err()
        .to_string();
    assert!(!error.contains("SECRET_DO_NOT_EXPOSE"));
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "../../SECRET_DO_NOT_EXPOSE"
    );
}

#[test]
fn defaults_file_import_is_backed_up_once_and_authoritative_models_win() {
    use spark_storage::settings::migrate_core_settings;
    for authored in [false, true] {
        let home = tempfile::tempdir().unwrap();
        let core = home.path().join("spark.toml");
        let legacy = home.path().join("ui-defaults.json");
        let original = if authored {
            "schema_version = 1\n[models]\nprovider = 'anthropic'\nmodel = 'authored'\n"
        } else {
            "schema_version = 1\n[extension]\nkeep = true\n"
        };
        fs::write(&core, original).unwrap();
        let source = r#"{"llm_provider":"codex","llm_profile":"","llm_model":"old","reasoning_effort":"high"}"#;
        fs::write(&legacy, source).unwrap();
        let migrated = migrate_core_settings(&core).unwrap();
        assert_eq!(
            migrated.values["models"]["model"].as_str(),
            Some(if authored { "authored" } else { "old" })
        );
        assert_eq!(
            migrated.values["defaults_migration_version"].as_integer(),
            Some(1)
        );
        assert_eq!(
            fs::read_to_string(home.path().join("ui-defaults.json.v0.bak")).unwrap(),
            source
        );
        assert_eq!(
            fs::read_to_string(home.path().join("spark.toml.defaults-import-v1.bak")).unwrap(),
            original
        );
        fs::write(&legacy, "invalid later source must never be reimported").unwrap();
        assert_eq!(
            migrate_core_settings(&core).unwrap().revision,
            migrated.revision
        );
        assert!(!legacy.exists());
        assert!(fs::read_dir(home.path()).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("ui-defaults.json.cleanup-")));
        if !authored {
            assert_eq!(migrated.values["extension"]["keep"].as_bool(), Some(true));
        }
    }
}

#[test]
fn invalid_defaults_import_retains_source_and_redacts_values() {
    let home = tempfile::tempdir().unwrap();
    let core = home.path().join("spark.toml");
    let legacy = home.path().join("ui-defaults.json");
    let source = r#"{"llm_model":{"SECRET_DO_NOT_EXPOSE":true}}"#;
    fs::write(&legacy, source).unwrap();
    let error = spark_storage::settings::migrate_core_settings(&core)
        .unwrap_err()
        .to_string();
    assert!(!error.contains("SECRET_DO_NOT_EXPOSE"));
    assert!(!core.exists());
    assert_eq!(fs::read_to_string(legacy).unwrap(), source);
}

#[test]
fn conversation_migration_includes_projects_without_readable_registration() {
    let home = tempfile::tempdir().unwrap();
    for (id, metadata) in [("missing", None), ("invalid", Some("invalid TOML ["))] {
        let project = home.path().join("workspace/projects").join(id);
        let chat = project.join("conversations/old");
        fs::create_dir_all(&chat).unwrap();
        if let Some(metadata) = metadata {
            fs::write(project.join("project.toml"), metadata).unwrap();
        }
        let source = br#"{"id":"old","provider":"codex","model":"old-model","chat_mode":"plan","historical":{"model":"historical-model"}}"#;
        fs::write(chat.join("conversation.json"), source).unwrap();
        fs::write(chat.join("turns.jsonl"), "historical turn\n").unwrap();
        spark_storage::settings::migrate_workspace_conversation_settings(home.path()).unwrap();
        let migrated: serde_json::Value =
            serde_json::from_slice(&fs::read(chat.join("conversation.json")).unwrap()).unwrap();
        assert_eq!(migrated["settings_schema_version"], 1);
        assert!(migrated["model_settings"].is_null());
        assert_eq!(migrated["historical"]["model"], "historical-model");
        assert_eq!(migrated["chat_mode"], "plan");
        assert_eq!(
            fs::read(chat.join("conversation.json.settings-v0.bak")).unwrap(),
            source
        );
        assert_eq!(
            fs::read_to_string(chat.join("turns.jsonl")).unwrap(),
            "historical turn\n"
        );
        let before = fs::read(chat.join("conversation.json")).unwrap();
        spark_storage::settings::migrate_workspace_conversation_settings(home.path()).unwrap();
        assert_eq!(fs::read(chat.join("conversation.json")).unwrap(), before);
    }
}

#[test]
fn legacy_profile_defaults_cannot_import_a_deleted_reference() {
    let home = tempfile::tempdir().unwrap();
    let core = home.path().join("spark.toml");
    let source = r#"{"llm_profile":"team"}"#;
    fs::write(home.path().join("ui-defaults.json"), source).unwrap();
    let error = spark_storage::settings::migrate_core_settings(&core).unwrap_err();
    assert!(error.to_string().contains("Unknown profile reference"));
    assert!(!core.exists());
    assert_eq!(
        fs::read_to_string(home.path().join("ui-defaults.json")).unwrap(),
        source
    );
    fs::write(home.path().join("llm-profiles.toml"), "[profiles.' team ']\nprovider = 'openai_compatible'\nbase_url = 'http://localhost:9999/v1'\nmodels = ['model']\ndefault_model = 'model'\n").unwrap();
    let document = spark_storage::settings::migrate_core_settings(&core).unwrap();
    assert_eq!(
        document.values["models"]["llm_profile"].as_str(),
        Some("team")
    );
    assert_eq!(
        fs::read_to_string(home.path().join("ui-defaults.json.v0.bak")).unwrap(),
        source
    );
}

#[test]
fn completed_defaults_migration_retries_cleanup_without_reimporting() {
    let home = tempfile::tempdir().unwrap();
    let core = home.path().join("spark.toml");
    let source = home.path().join("ui-defaults.json");
    let bytes = r#"{"llm_provider":"codex","llm_model":"old"}"#;
    fs::write(&source, bytes).unwrap();
    spark_storage::settings::migrate_core_settings(&core).unwrap();
    assert!(!source.exists());
    assert_eq!(
        fs::read_to_string(home.path().join("ui-defaults.json.v0.bak")).unwrap(),
        bytes
    );
    let authored =
        "schema_version=1\ndefaults_migration_version=1\n[models]\nprovider='codex'\nmodel='new'\n";
    fs::write(&core, authored).unwrap();
    fs::write(&source, bytes).unwrap();
    spark_storage::settings::migrate_core_settings(&core).unwrap();
    assert!(!source.exists());
    assert_eq!(fs::read_to_string(&core).unwrap(), authored);
}

#[test]
fn new_client_target_uses_environment_then_current_storage_then_default() {
    let home = tempfile::tempdir().unwrap();
    let env = std::collections::BTreeMap::new();
    let resolve = spark_storage::settings::resolve_client_api_base_url;
    assert_eq!(
        resolve(home.path(), &env).unwrap().0,
        "http://127.0.0.1:8000"
    );
    fs::write(
        home.path().join("spark.toml"),
        "[connections]\nclient_api_base_url='http://localhost:4987'\n",
    )
    .unwrap();
    assert_eq!(
        resolve(home.path(), &env).unwrap().0,
        "http://localhost:4987"
    );
    let env = std::collections::BTreeMap::from([(
        "SPARK_API_BASE_URL".into(),
        " http://localhost:4988 ".into(),
    )]);
    assert_eq!(
        resolve(home.path(), &env).unwrap().0,
        "http://localhost:4988"
    );
}
