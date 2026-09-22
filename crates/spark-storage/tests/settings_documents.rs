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
fn unsupported_core_versions_are_rejected_naming_the_file_and_version() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("spark.toml");
    for found in [2, 0] {
        let bytes = format!("schema_version = {found}\n[runtime]\nproject_roots = []\n");
        fs::write(&path, &bytes).unwrap();
        for error in [
            read_settings_document(&path).unwrap_err(),
            spark_storage::settings::load_core_settings(&path).unwrap_err(),
            update_settings_section(&path, "absent", "runtime", Some(section(2)), |_| Ok(()))
                .unwrap_err(),
        ] {
            let message = error.to_string();
            assert!(message.contains(&path.display().to_string()), "{message}");
            assert!(
                message.contains(&format!("schema_version {found}; expected 1")),
                "{message}"
            );
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), bytes);
    }
}

#[test]
fn fresh_core_bootstrap_creates_the_current_document() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("config/spark.toml");
    let created = spark_storage::settings::load_core_settings(&path).unwrap();
    assert_eq!(created.values["schema_version"].as_integer(), Some(1));
    assert_eq!(
        spark_storage::settings::load_core_settings(&path)
            .unwrap()
            .revision,
        created.revision
    );
}

#[test]
fn legacy_defaults_file_is_rejected_naming_the_file() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("spark.toml");
    let legacy = root.path().join("ui-defaults.json");
    fs::write(&legacy, "{\"llm_provider\":\"codex\"}").unwrap();
    let error = spark_storage::settings::load_core_settings(&core)
        .unwrap_err()
        .to_string();
    assert!(error.contains(&legacy.display().to_string()), "{error}");
    assert!(!core.exists());
}

#[test]
fn unsupported_conversation_settings_version_is_rejected_naming_the_file() {
    let root = tempfile::tempdir().unwrap();
    let project = spark_storage::ProjectRegistry::new(root.path())
        .ensure_project_paths("/projects/old")
        .unwrap();
    let dir = project.conversations_dir.join("chat");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("conversation.json");
    let mut meta = spark_storage::conversation::ConversationMeta::new("chat", "/projects/old");
    meta.settings_schema_version = 0;
    let bytes = serde_json::to_vec(&meta).unwrap();
    fs::write(&path, &bytes).unwrap();
    let error = spark_storage::ConversationRepository::new(root.path())
        .read_snapshot("chat", Some("/projects/old"))
        .unwrap_err()
        .to_string();
    assert!(error.contains(&path.display().to_string()), "{error}");
    assert!(error.contains("settings_schema_version 0"), "{error}");
    assert_eq!(fs::read(&path).unwrap(), bytes);
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
fn core_bootstrap_validates_all_known_sections() {
    use spark_storage::settings::load_core_settings;
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("spark.toml");
    for invalid in [
        "schema_version = 1\n[models]\nprovider = 'codex'\nllm_profile = 'conflict'\n",
        "schema_version = 1\n[runtime]\nflows_dir = 123\n",
        "schema_version = 1\n[desktop]\nremote_access_enabled = 'SECRET_DO_NOT_EXPOSE'\n",
    ] {
        fs::write(&core, invalid).unwrap();
        let error = load_core_settings(&core).unwrap_err().to_string();
        assert!(!error.contains("SECRET_DO_NOT_EXPOSE"));
        assert_eq!(fs::read_to_string(&core).unwrap(), invalid);
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
