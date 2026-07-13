use spark_storage::conversation::{
    ConversationMutation, RuntimeSession, TranscriptTurn, RUNTIME_SESSION_SCHEMA_VERSION,
};
use spark_storage::{ConversationRepository, ProjectRegistry};

fn setup(project_path: &str) -> (tempfile::TempDir, ConversationRepository) {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    ProjectRegistry::new(&home)
        .ensure_project_paths(project_path)
        .expect("project paths");
    (temp, ConversationRepository::new(&home))
}

fn session(thread_id: &str) -> RuntimeSession {
    RuntimeSession {
        schema_version: RUNTIME_SESSION_SCHEMA_VERSION,
        provider: "codex_app_server".to_string(),
        thread_id: Some(thread_id.to_string()),
        established_at: "2026-01-01T00:00:00Z".to_string(),
        last_turn_id: Some("turn-assistant".to_string()),
        resume_failed: false,
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

#[test]
fn runtime_session_round_trips_and_missing_file_reads_as_none() {
    let project_path = "/projects/runtime-session";
    let (_temp, repo) = setup(project_path);

    assert_eq!(
        repo.read_runtime_session("conversation-a", Some(project_path))
            .expect("read absent"),
        None
    );

    let written = session("thread-alpha");
    repo.write_runtime_session("conversation-a", project_path, &written)
        .expect("write");
    let read = repo
        .read_runtime_session("conversation-a", Some(project_path))
        .expect("read")
        .expect("session");
    assert_eq!(read, written);

    let tombstoned = RuntimeSession {
        resume_failed: true,
        updated_at: "2026-01-01T00:00:05Z".to_string(),
        ..written
    };
    repo.write_runtime_session("conversation-a", project_path, &tombstoned)
        .expect("tombstone");
    assert_eq!(
        repo.read_runtime_session("conversation-a", Some(project_path))
            .expect("read tombstone"),
        Some(tombstoned)
    );
}

#[test]
fn runtime_session_presence_never_changes_snapshot_or_journal_reads() {
    let project_path = "/projects/runtime-session-isolated";
    let (_temp, repo) = setup(project_path);

    repo.commit_conversation(
        "conversation-b",
        project_path,
        0,
        vec![ConversationMutation::TurnUpserted {
            turn: TranscriptTurn {
                id: "turn-user".to_string(),
                role: "user".to_string(),
                kind: Some("message".to_string()),
                content: "Continuity is a separate authority".to_string(),
                status: "complete".to_string(),
                timestamp: "2026-01-01T00:00:00Z".to_string(),
                ..TranscriptTurn::default()
            },
        }],
    )
    .expect("commit");

    let snapshot_before = repo
        .read_snapshot("conversation-b", Some(project_path))
        .expect("read")
        .expect("snapshot");
    let events_before = repo
        .read_conversation_events_after("conversation-b", project_path, 0)
        .expect("events");

    repo.write_runtime_session("conversation-b", project_path, &session("thread-beta"))
        .expect("write session");

    let snapshot_after = repo
        .read_snapshot("conversation-b", Some(project_path))
        .expect("read again")
        .expect("snapshot");
    let events_after = repo
        .read_conversation_events_after("conversation-b", project_path, 0)
        .expect("events again");
    assert_eq!(snapshot_before, snapshot_after);
    assert_eq!(events_before, events_after);
}
