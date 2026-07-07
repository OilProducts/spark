use std::fs;

use serde_json::{json, Value};
use spark_storage::conversation::{ConversationMutation, TranscriptTurn};
use spark_storage::{
    ConversationRepository, ProjectPaths, ProjectRegistry, CONVERSATION_STATE_SCHEMA_VERSION,
    UNSUPPORTED_CONVERSATION_STATE_SCHEMA,
};

fn setup(project_path: &str) -> (tempfile::TempDir, ConversationRepository, ProjectPaths) {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    let project = ProjectRegistry::new(&home)
        .ensure_project_paths(project_path)
        .expect("project paths");
    (temp, ConversationRepository::new(&home), project)
}

/// Write the real pre-split layout by hand: core keys in `state.json`,
/// artifact arrays in the project-level sidecar files.
fn write_legacy_files(project: &ProjectPaths, snapshot: &Value) {
    let object = snapshot.as_object().expect("snapshot object");
    let conversation_id = snapshot["conversation_id"]
        .as_str()
        .expect("conversation id");
    let project_path = snapshot["project_path"].as_str().expect("project path");
    let root = project.conversations_dir.join(conversation_id);
    fs::create_dir_all(&root).expect("conversation dir");
    let mut core = serde_json::Map::new();
    for (key, value) in object {
        if !matches!(
            key.as_str(),
            "event_log"
                | "flow_run_requests"
                | "flow_launches"
                | "run_recoveries"
                | "proposed_plans"
        ) {
            core.insert(key.clone(), value.clone());
        }
    }
    let array = |key: &str| object.get(key).cloned().unwrap_or_else(|| json!([]));
    fs::write(
        root.join("state.json"),
        serde_json::to_string_pretty(&Value::Object(core)).expect("state json"),
    )
    .expect("state.json");
    fs::write(
        project
            .flow_run_requests_dir
            .join(format!("{conversation_id}.json")),
        serde_json::to_string_pretty(&json!({
            "conversation_id": conversation_id,
            "project_id": project.project_id,
            "project_path": project_path,
            "event_log": array("event_log"),
            "flow_run_requests": array("flow_run_requests"),
        }))
        .expect("json"),
    )
    .expect("flow-run-requests sidecar");
    fs::write(
        project
            .flow_launches_dir
            .join(format!("{conversation_id}.json")),
        serde_json::to_string_pretty(&json!({
            "conversation_id": conversation_id,
            "project_id": project.project_id,
            "project_path": project_path,
            "flow_launches": array("flow_launches"),
            "run_recoveries": array("run_recoveries"),
        }))
        .expect("json"),
    )
    .expect("flow-launches sidecar");
    fs::write(
        project
            .proposed_plans_dir
            .join(format!("{conversation_id}.json")),
        serde_json::to_string_pretty(&json!({
            "conversation_id": conversation_id,
            "project_id": project.project_id,
            "project_path": project_path,
            "proposed_plans": array("proposed_plans"),
        }))
        .expect("json"),
    )
    .expect("proposed-plans sidecar");
}

fn seed_legacy_conversation(
    project: &ProjectPaths,
    project_path: &str,
    conversation_id: &str,
) -> Value {
    let snapshot = json!({
        "schema_version": CONVERSATION_STATE_SCHEMA_VERSION,
        "revision": 7,
        "conversation_id": conversation_id,
        "conversation_handle": "amber-anchor",
        "project_path": project_path,
        "chat_mode": "chat",
        "provider": "codex",
        "model": null,
        "llm_profile": null,
        "reasoning_effort": null,
        "title": "Legacy thread",
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:07Z",
        "turns": [
            {"id": "turn-user", "role": "user", "content": "hello legacy", "timestamp": "2026-01-01T00:00:00Z", "status": "complete", "kind": "message"},
            {"id": "turn-assistant", "role": "assistant", "content": "done", "timestamp": "2026-01-01T00:00:01Z", "status": "complete", "kind": "message"}
        ],
        "segments": [
            {"id": "segment-a", "turn_id": "turn-assistant", "order": 1, "kind": "assistant_message", "role": "assistant", "status": "complete", "timestamp": "2026-01-01T00:00:01Z", "updated_at": "2026-01-01T00:00:02Z", "content": "done", "source": {}}
        ],
        "event_log": [{"message": "launched", "timestamp": "2026-01-01T00:00:03Z"}],
        "flow_run_requests": [{"id": "request-a", "status": "pending_review"}],
        "flow_launches": [{"id": "launch-a"}],
        "run_recoveries": [{"id": "recovery-a"}],
        "proposed_plans": [{"id": "plan-a"}]
    });
    write_legacy_files(project, &snapshot);
    // Legacy journal entries in the legacy file.
    let events_path = project
        .conversations_dir
        .join(conversation_id)
        .join("events.jsonl");
    for revision in [6, 7] {
        let line = serde_json::to_string(&json!({
            "type": "turn_upsert",
            "revision": revision,
            "conversation_id": conversation_id,
            "project_path": project_path,
            "turn": {"id": "turn-user"}
        }))
        .expect("line");
        fs::write(
            &events_path,
            match fs::read_to_string(&events_path) {
                Ok(existing) => format!("{existing}{line}\n"),
                Err(_) => format!("{line}\n"),
            },
        )
        .expect("seed legacy events");
    }
    snapshot
}

#[test]
fn legacy_conversation_migrates_once_on_first_read() {
    let project_path = "/projects/migration-basic";
    let (_temp, repo, project) = setup(project_path);
    seed_legacy_conversation(&project, project_path, "conversation-legacy");

    // Pre-migration merged view, straight from the legacy files.
    let root = project.conversations_dir.join("conversation-legacy");
    assert!(root.join("state.json").exists());

    let migrated = repo
        .read_snapshot("conversation-legacy", Some(project_path))
        .expect("read")
        .expect("snapshot");

    // Read output carries the full merged legacy content.
    assert_eq!(migrated["revision"], 7);
    assert_eq!(migrated["title"], "Legacy thread");
    assert_eq!(migrated["turns"].as_array().expect("turns").len(), 2);
    assert_eq!(migrated["segments"][0]["id"], "segment-a");
    assert_eq!(migrated["event_log"][0]["message"], "launched");
    assert_eq!(migrated["flow_run_requests"][0]["id"], "request-a");
    assert_eq!(migrated["flow_launches"][0]["id"], "launch-a");
    assert_eq!(migrated["run_recoveries"][0]["id"], "recovery-a");
    assert_eq!(migrated["proposed_plans"][0]["id"], "plan-a");

    // Split records exist; legacy files renamed aside; sidecars absorbed.
    for expected in [
        "conversation.json",
        "transcript.json",
        "event-log.json",
        "journal.jsonl",
        "artifacts/flow-run-requests.json",
        "artifacts/flow-launches.json",
        "artifacts/run-recoveries.json",
        "artifacts/proposed-plans.json",
        "state.json.migrated",
        "events.jsonl.migrated",
    ] {
        assert!(root.join(expected).exists(), "missing {expected}");
    }
    assert!(!root.join("state.json").exists());
    assert!(!root.join("events.jsonl").exists());
    for dir in [
        &project.flow_run_requests_dir,
        &project.flow_launches_dir,
        &project.proposed_plans_dir,
    ] {
        assert!(!dir.join("conversation-legacy.json").exists());
    }

    // The journal starts with one snapshot checkpoint at the carried revision.
    let journal = fs::read_to_string(root.join("journal.jsonl")).expect("journal");
    let lines: Vec<Value> = journal
        .lines()
        .map(|line| serde_json::from_str(line).expect("journal line"))
        .collect();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["type"], "conversation_snapshot");
    assert_eq!(lines[0]["revision"], 7);
    assert_eq!(lines[0]["state"]["segments"][0]["id"], "segment-a");
    assert_eq!(lines[0]["state"]["flow_run_requests"][0]["id"], "request-a");
}

#[test]
fn migration_preserves_revision_continuity_and_checkpoint_replay() {
    let project_path = "/projects/migration-continuity";
    let (_temp, repo, project) = setup(project_path);
    seed_legacy_conversation(&project, project_path, "conversation-cont");

    // First read migrates (carried revision 7); the next commit continues
    // from it without regressing.
    repo.read_snapshot("conversation-cont", Some(project_path))
        .expect("read")
        .expect("snapshot");
    let commit = repo
        .commit_conversation(
            "conversation-cont",
            project_path,
            7,
            vec![ConversationMutation::TurnUpserted {
                turn: TranscriptTurn {
                    id: "turn-next".to_string(),
                    role: "user".to_string(),
                    kind: Some("message".to_string()),
                    content: "after migration".to_string(),
                    status: "complete".to_string(),
                    timestamp: "2026-01-02T00:00:00Z".to_string(),
                    ..TranscriptTurn::default()
                },
            }],
        )
        .expect("post-migration commit");
    assert_eq!(commit.revision, 8);

    // Replay from a pre-migration cursor returns the checkpoint snapshot and
    // the new committed entry, in revision order.
    let replay = repo
        .read_conversation_events_after("conversation-cont", project_path, 3)
        .expect("replay");
    assert_eq!(
        replay
            .iter()
            .map(|event| {
                (
                    event["type"].as_str().expect("type").to_string(),
                    event["revision"].as_i64().expect("revision"),
                )
            })
            .collect::<Vec<_>>(),
        vec![
            ("conversation_snapshot".to_string(), 7),
            ("turn_upsert".to_string(), 8)
        ]
    );
}

#[test]
fn unsupported_legacy_schema_errors_and_leaves_files_untouched() {
    let project_path = "/projects/migration-unsupported";
    let (_temp, repo, project) = setup(project_path);
    let root = project.conversations_dir.join("conversation-old");
    fs::create_dir_all(&root).expect("conversation dir");
    let state = json!({
        "schema_version": 4,
        "revision": 1,
        "segments": []
    });
    fs::write(
        root.join("state.json"),
        serde_json::to_string_pretty(&state).expect("json"),
    )
    .expect("state");

    let error = repo
        .read_snapshot("conversation-old", Some(project_path))
        .expect_err("unsupported schema");
    assert_eq!(error.to_string(), UNSUPPORTED_CONVERSATION_STATE_SCHEMA);

    // Nothing was migrated, renamed, or created.
    assert!(root.join("state.json").exists());
    assert!(!root.join("state.json.migrated").exists());
    assert!(!root.join("conversation.json").exists());
    assert!(!root.join("journal.jsonl").exists());
}

#[test]
fn second_read_after_migration_does_not_re_migrate() {
    let project_path = "/projects/migration-idempotent";
    let (_temp, repo, project) = setup(project_path);
    seed_legacy_conversation(&project, project_path, "conversation-idem");
    let root = project.conversations_dir.join("conversation-idem");

    let first = repo
        .read_snapshot("conversation-idem", Some(project_path))
        .expect("first read")
        .expect("snapshot");
    let migrated_state =
        fs::read_to_string(root.join("state.json.migrated")).expect("migrated state");
    let journal_after_first = fs::read_to_string(root.join("journal.jsonl")).expect("journal");

    let second = repo
        .read_snapshot("conversation-idem", Some(project_path))
        .expect("second read")
        .expect("snapshot");
    assert_eq!(first, second);
    assert_eq!(
        fs::read_to_string(root.join("state.json.migrated")).expect("migrated state"),
        migrated_state
    );
    // No duplicate checkpoint appended.
    assert_eq!(
        fs::read_to_string(root.join("journal.jsonl")).expect("journal"),
        journal_after_first
    );
}
