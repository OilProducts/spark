use std::{fs, process::Command};

use serde_json::json;
use spark_storage::{StorageError, TriggerDefinition, TriggerDefinitionRepository};

fn definition() -> TriggerDefinition {
    serde_json::from_value(json!({
        "id": "revision-test", "name": "Original", "enabled": true, "protected": false,
        "source_type": "schedule", "created_at": "2026-09-11T00:00:00Z", "updated_at": "2026-09-11T00:00:00Z",
        "action": {"flow_name": "test.yaml", "static_context": {}},
        "source": {"kind": "interval", "interval_seconds": 60}
    })).unwrap()
}

#[test]
fn trigger_updates_and_deletes_require_the_loaded_revision() {
    let root = tempfile::tempdir().unwrap();
    let repository = TriggerDefinitionRepository::new(root.path());
    let original = definition();
    let revision = repository.put(&original).unwrap();
    assert!(matches!(
        repository.put(&original),
        Err(StorageError::SettingsConflict { .. })
    ));
    let mut draft = repository.get(&original.id).unwrap().unwrap();
    assert_eq!(draft.revision, revision);
    draft.name = "Saved".into();
    let saved_revision = repository.put(&draft).unwrap();
    assert_ne!(revision, saved_revision);
    assert!(matches!(
        repository.put(&draft),
        Err(StorageError::SettingsConflict { .. })
    ));
    assert!(matches!(
        repository.delete(&original.id, &revision),
        Err(StorageError::SettingsConflict { .. })
    ));
    let path = repository.definition_path(&original.id).unwrap();
    let before = fs::read(&path).unwrap();
    draft.revision = saved_revision.clone();
    draft.source.insert("interval_seconds".into(), json!(-1));
    assert!(repository.put(&draft).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    // External edits, including fields unknown to this binary, invalidate a loaded draft.
    fs::write(
        &path,
        format!("# external edit\n{}", String::from_utf8(before).unwrap()),
    )
    .unwrap();
    assert!(matches!(
        repository.delete(&original.id, &saved_revision),
        Err(StorageError::SettingsConflict { .. })
    ));
    let current = repository.get(&original.id).unwrap().unwrap();
    repository.delete(&original.id, &current.revision).unwrap();
    assert!(repository.get(&original.id).unwrap().is_none());
}

#[test]
fn trigger_process_writer() {
    let Ok(root) = std::env::var("SPARK_TEST_TRIGGER_CONFIG") else {
        return;
    };
    let repository = TriggerDefinitionRepository::new(root);
    let mut draft = repository.get("revision-test").unwrap().unwrap();
    draft.revision = std::env::var("SPARK_TEST_TRIGGER_REVISION").unwrap();
    draft.name = std::process::id().to_string();
    match repository.put(&draft) {
        Ok(_) => {}
        Err(StorageError::SettingsConflict { .. }) => std::process::exit(23),
        Err(error) => panic!("{error}"),
    }
}

#[test]
fn separate_processes_cannot_overwrite_the_same_trigger_revision() {
    let root = tempfile::tempdir().unwrap();
    let repository = TriggerDefinitionRepository::new(root.path());
    let revision = repository.put(&definition()).unwrap();
    let spawn = || {
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "trigger_process_writer"])
            .env("SPARK_TEST_TRIGGER_CONFIG", root.path())
            .env("SPARK_TEST_TRIGGER_REVISION", &revision)
            .spawn()
            .unwrap()
    };
    let mut first = spawn();
    let mut second = spawn();
    let mut statuses = [
        first.wait().unwrap().code().unwrap(),
        second.wait().unwrap().code().unwrap(),
    ];
    statuses.sort();
    assert_eq!(statuses, [0, 23]);
    assert_ne!(
        repository.get("revision-test").unwrap().unwrap().revision,
        revision
    );
}
