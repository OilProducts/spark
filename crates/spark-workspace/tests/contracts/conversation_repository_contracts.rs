use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
use spark_storage::ProjectRegistry;
use spark_workspace::{ConversationSettingsUpdate, WorkspaceError};

#[test]
fn conversation_service_reads_python_state_sidecars_and_truncates_tool_output_for_snapshots() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = "/projects/service-app";
    let project = ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths(project_path)
        .expect("project");
    let long_output = format!("{}{}", "x".repeat(8 * 1024), "tail");
    write_state(
        &project.conversations_dir,
        "conversation-a",
        json!({
            "schema_version": 5,
            "settings_schema_version": 1,
            "revision": 7,
            "conversation_id": "conversation-a",
            "conversation_handle": "amber-anchor",
            "project_path": project_path,
            "chat_mode": "chat",
            "provider": "codex",
            "model": null,
            "reasoning_effort": null,
            "title": "",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:07Z",
            "turns": [
                {
                    "id": "turn-user",
                    "role": "user",
                    "content": "Summarize this",
                    "timestamp": "2026-01-01T00:00:00Z",
                    "status": "complete",
                    "kind": "message"
                },
                {
                    "id": "turn-assistant",
                    "role": "assistant",
                    "content": "Done",
                    "timestamp": "2026-01-01T00:00:01Z",
                    "status": "complete",
                    "kind": "message"
                }
            ],
            "segments": [
                {
                    "id": "segment-tool",
                    "turn_id": "turn-assistant",
                    "order": 1,
                    "kind": "tool_call",
                    "role": "assistant",
                    "status": "complete",
                    "timestamp": "2026-01-01T00:00:02Z",
                    "updated_at": "2026-01-01T00:00:03Z",
                    "content": "",
                    "source": {},
                    "tool_call": {
                        "id": "tool-1",
                        "kind": "command_execution",
                        "status": "completed",
                        "title": "Run command",
                        "output": long_output,
                        "file_paths": []
                    }
                }
            ],
            "event_log": [
                {"message": "Review requested", "timestamp": "2026-01-01T00:00:04Z"}
            ],
            "flow_run_requests": [
                {"id": "request-a", "created_at": "2026-01-01T00:00:04Z", "updated_at": "2026-01-01T00:00:04Z", "flow_name": "flow.dot", "summary": "Run", "project_path": project_path, "conversation_id": "conversation-a", "source_turn_id": "turn-assistant"}
            ],
            "flow_launches": [
                {"id": "launch-a", "created_at": "2026-01-01T00:00:05Z", "updated_at": "2026-01-01T00:00:05Z", "flow_name": "flow.dot", "summary": "Run", "project_path": project_path, "conversation_id": "conversation-a", "source_turn_id": "turn-assistant", "status": "launched", "run_id": "run-a"},
                {"id": "launch-b", "created_at": "2026-01-01T00:00:06Z", "updated_at": "2026-01-01T00:00:06Z", "flow_name": "flow.dot", "summary": "Run", "project_path": project_path, "conversation_id": "conversation-a", "source_turn_id": "turn-assistant", "status": "launch_failed"}
            ]
        }),
    );

    // A conversation whose assistant turn is still streaming lists as running.
    write_state(
        &project.conversations_dir,
        "conversation-b",
        json!({
            "schema_version": 5,
            "settings_schema_version": 1,
            "revision": 2,
            "conversation_id": "conversation-b",
            "project_path": project_path,
            "title": "Working",
            "created_at": "2025-12-01T00:00:00Z",
            "updated_at": "2025-12-01T00:00:01Z",
            "turns": [
                {"id": "turn-b-user", "role": "user", "content": "Go", "timestamp": "2025-12-01T00:00:00Z", "status": "complete", "kind": "message"},
                {"id": "turn-b-assistant", "role": "assistant", "content": "", "timestamp": "2025-12-01T00:00:01Z", "status": "streaming", "kind": "message"}
            ],
            "segments": []
        }),
    );

    let service = crate::conversations(settings);
    let snapshot = service
        .get_snapshot("conversation-a", Some(project_path))
        .expect("snapshot");
    assert_eq!(snapshot["title"], "Summarize this");
    assert_eq!(snapshot["llm_profile"], Value::Null);
    assert_eq!(snapshot["event_log"][0]["message"], "Review requested");
    assert_eq!(snapshot["flow_run_requests"][0]["id"], "request-a");
    assert_eq!(snapshot["segments"][0]["tool_call"]["output_size"], 8196);
    assert_eq!(
        snapshot["segments"][0]["tool_call"]["output"]
            .as_str()
            .expect("preview")
            .len(),
        8 * 1024
    );
    assert_eq!(
        snapshot["segments"][0]["tool_call"]["output_truncated"],
        true
    );

    let tool_output = service
        .get_segment_tool_output("conversation-a", "segment-tool", Some(project_path))
        .expect("tool output");
    assert_eq!(tool_output["output"].as_str().expect("output").len(), 8196);
    assert_eq!(tool_output["output_size"], 8196);

    let mut summaries = service
        .list_project_conversations(project_path)
        .expect("summaries");
    // The Coordinator's placeholder row is pinned first.
    assert_eq!(
        summaries.remove(0).standing_role.unwrap().name,
        "coordinator"
    );
    assert_eq!(summaries[0].conversation_id, "conversation-a");
    assert_eq!(summaries[0].last_message_preview.as_deref(), Some("Done"));
    assert_eq!(summaries[0].launched_run_ids, vec!["run-a".to_string()]);
    assert!(!summaries[0].running);
    let running = summaries
        .iter()
        .find(|summary| summary.conversation_id == "conversation-b")
        .expect("streaming conversation listed");
    assert!(running.running);
}

#[test]
fn conversation_settings_update_creates_shell_state_handle_and_mode_change_once() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = "/projects/settings-app";
    let service = crate::conversations(settings.clone());

    let ack = service
        .update_conversation_settings(
            "conversation-settings",
            ConversationSettingsUpdate {
                project_path: project_path.to_string(),
                chat_mode: Some("plan".to_string()),
                provider: Some("openai".to_string()),
                expected_revision: Some("0".into()),
                model: Some("gpt-5".to_string()),
                reasoning_effort: Some("high".to_string()),
                ..ConversationSettingsUpdate::default()
            },
        )
        .expect("settings update");
    let snapshot = service
        .get_snapshot("conversation-settings", Some(project_path))
        .expect("snapshot");
    // The mode-change turn and the settings write commit together.
    assert_eq!(snapshot["revision"], ack.revision);
    assert!(ack.changes.rows.iter().any(|row| row.kind
        == spark_storage::conversation::EntityKind::Conversation
        && row.id == "conversation-settings"));
    assert_eq!(snapshot["chat_mode"], "plan");
    assert_eq!(snapshot["provider"], "openai");
    assert_eq!(snapshot["model"], "gpt-5");
    assert_eq!(snapshot["llm_profile"], Value::Null);
    assert_eq!(snapshot["reasoning_effort"], "high");
    assert_eq!(snapshot["turns"].as_array().expect("turns").len(), 1);
    assert_eq!(snapshot["turns"][0]["kind"], "mode_change");
    assert_eq!(
        snapshot["conversation_handle"]
            .as_str()
            .expect("handle")
            .split('-')
            .count(),
        2
    );

    let second_ack = service
        .update_conversation_settings(
            "conversation-settings",
            ConversationSettingsUpdate {
                project_path: project_path.to_string(),
                chat_mode: Some("plan".to_string()),
                expected_revision: Some(snapshot["revision"].to_string()),
                ..ConversationSettingsUpdate::default()
            },
        )
        .expect("same mode update");
    let second = service
        .get_snapshot("conversation-settings", Some(project_path))
        .expect("snapshot");
    // A same-mode update adds no mode turn.
    assert_eq!(second["revision"], second_ack.revision);
    assert_eq!(second["turns"].as_array().expect("turns").len(), 1);

    let project = ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths(project_path)
        .expect("project");
    // The conversation and its handle are database rows; no files appear.
    let stored = crate::shared_store(&settings.data_dir)
        .database()
        .read(|read| read.conversation("conversation-settings"))
        .expect("rows")
        .expect("conversation");
    assert_eq!(
        stored.handle,
        snapshot["conversation_handle"].as_str().unwrap()
    );
    assert_eq!(stored.metadata["chat_mode"], "plan");
    assert!(!project
        .conversations_dir
        .join("conversation-settings")
        .exists());
    assert!(!project
        .flow_run_requests_dir
        .join("conversation-settings.json")
        .exists());

    let invalid = service
        .update_conversation_settings(
            "conversation-settings",
            ConversationSettingsUpdate {
                project_path: project_path.to_string(),
                provider: Some("unknown".to_string()),
                ..ConversationSettingsUpdate::default()
            },
        )
        .expect_err("invalid provider");
    assert_eq!(
        invalid.to_string(),
        "Provider must be blank or one of: codex, claude-code, openai, anthropic, gemini, openrouter, litellm, openai_compatible."
    );
}

#[test]
fn conversation_service_defaults_llm_profile_and_allocates_summary_handles() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = "/projects/defaults-app";
    let project = ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths(project_path)
        .expect("project");
    let service = crate::conversations(settings.clone());

    let shell = service
        .get_snapshot("conversation-shell", Some(project_path))
        .expect("shell");
    assert_eq!(shell["revision"], 0);
    assert_eq!(shell["llm_profile"], Value::Null);

    write_state(
        &project.conversations_dir,
        "conversation-legacy-defaults",
        json!({
            "schema_version": 5,
            "settings_schema_version": 1,
            "revision": 3,
            "conversation_id": "conversation-legacy-defaults",
            "project_path": project_path,
            "chat_mode": "chat",
            "provider": "codex",
            "model": null,
            "reasoning_effort": null,
            "title": "Legacy defaults",
            "created_at": "2026-01-02T00:00:00Z",
            "updated_at": "2026-01-02T00:00:03Z",
            "turns": [],
            "segments": []
        }),
    );

    let legacy = service
        .get_snapshot("conversation-legacy-defaults", Some(project_path))
        .expect("legacy snapshot");
    assert_eq!(legacy["llm_profile"], Value::Null);

    let summaries = service
        .list_project_conversations(project_path)
        .expect("summaries");
    let summary = summaries
        .iter()
        .find(|summary| summary.conversation_id == "conversation-legacy-defaults")
        .expect("summary");
    assert_eq!(summary.conversation_handle.split('-').count(), 2);

    let stored = crate::shared_store(&settings.data_dir)
        .database()
        .read(|read| read.conversation("conversation-legacy-defaults"))
        .expect("rows")
        .expect("conversation");
    assert_eq!(stored.handle, summary.conversation_handle);
}

#[test]
fn conversation_service_rejects_project_mismatch_and_deletes_conversation_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_a = ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths("/projects/a")
        .expect("project a");
    write_state(
        &project_a.conversations_dir,
        "conversation-a",
        json!({
            "schema_version": 5,
            "settings_schema_version": 1,
            "revision": 1,
            "conversation_id": "conversation-a",
            "conversation_handle": "calm-river",
            "project_path": "/projects/a",
            "chat_mode": "chat",
            "provider": "codex",
            "model": null,
            "reasoning_effort": null,
            "title": "A",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:01Z",
            "turns": [],
            "segments": []
        }),
    );
    let service = crate::conversations(settings.clone());

    let mismatch = service
        .get_snapshot("conversation-a", Some("/projects/b"))
        .expect_err("mismatch");
    assert!(matches!(mismatch, WorkspaceError::Validation(_)));
    assert_eq!(
        mismatch.to_string(),
        "Conversation is already bound to a different project path."
    );

    service
        .update_conversation_settings(
            "conversation-a",
            ConversationSettingsUpdate {
                project_path: "/projects/a".to_string(),
                ..ConversationSettingsUpdate::default()
            },
        )
        .expect("ensure handle");
    let deleted = service
        .delete_conversation("conversation-a", "/projects/a")
        .expect("delete");
    assert_eq!(deleted.status, "deleted");
    assert!(!project_a.conversations_dir.join("conversation-a").exists());
    let missing = service
        .delete_conversation("conversation-a", "/projects/a")
        .expect_err("missing");
    assert!(matches!(missing, WorkspaceError::NotFound(_)));
}

fn write_state(conversations_dir: &Path, conversation_id: &str, payload: Value) {
    assert_eq!(payload["conversation_id"], conversation_id);
    let data_dir = conversations_dir
        .ancestors()
        .nth(4)
        .expect("data directory");
    crate::write_conversation_snapshot(data_dir, &payload);
}

pub(super) fn settings(root: &Path) -> SparkSettings {
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
        project_roots: Vec::<PathBuf>::new(),
    }
}
