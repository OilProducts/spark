use std::fs;

use serde_json::json;
use spark_common::project::build_project_id;
use spark_storage::{ProjectRecordUpdate, ProjectRegistry};

#[test]
fn project_registry_registers_lists_and_preserves_toml_shape() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    let project_dir = temp.path().join("Registered Project");
    fs::create_dir_all(&project_dir).expect("project dir");
    let registry = ProjectRegistry::new(&home);

    let record = registry
        .read_project_record(project_dir.to_str().expect("utf-8"))
        .expect("register")
        .expect("record");

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
    let record = registry
        .read_project_record(project_path)
        .expect("register")
        .expect("record");
    assert_eq!(record.project_path, project_path);

    let updated = registry
        .update_project_record(
            project_path,
            ProjectRecordUpdate {
                last_accessed_at: Some(Some("2026-02-03T04:05:06Z".to_string())),
                is_favorite: Some(true),
                active_conversation_id: Some(Some("conversation-1".to_string())),
                execution_profile_id: Some(Some("native".to_string())),
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
