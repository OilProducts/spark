use spark_storage::conversation::{
    ConversationMutation, RuntimeSession, TranscriptTurn, RUNTIME_SESSION_SCHEMA_VERSION,
};
use spark_storage::{ConversationRepository, ConversationStore, ProjectRegistry};

fn setup(project_path: &str) -> (tempfile::TempDir, ConversationRepository) {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    ProjectRegistry::new(&home)
        .ensure_project_paths(project_path)
        .expect("project paths");
    let store = ConversationStore::open(&home).expect("store");
    (temp, ConversationRepository::new(store))
}

fn user_turn(content: &str) -> ConversationMutation {
    ConversationMutation::TurnUpserted {
        turn: TranscriptTurn {
            id: "turn-user".to_string(),
            role: "user".to_string(),
            kind: Some("message".to_string()),
            content: content.to_string(),
            status: "complete".to_string(),
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            ..TranscriptTurn::default()
        },
    }
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
fn runtime_session_round_trips_and_missing_session_reads_as_none() {
    let project_path = "/projects/runtime-session";
    let (_temp, repo) = setup(project_path);
    // Sessions belong to live conversations only.
    assert!(repo
        .write_runtime_session("conversation-a", project_path, &session("thread-early"))
        .is_err());
    repo.commit_conversation("conversation-a", project_path, 0, vec![user_turn("Hello")])
        .expect("commit");

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
fn runtime_session_presence_never_changes_snapshot_reads() {
    let project_path = "/projects/runtime-session-isolated";
    let (_temp, repo) = setup(project_path);

    repo.commit_conversation(
        "conversation-b",
        project_path,
        0,
        vec![user_turn("Continuity is a separate authority")],
    )
    .expect("commit");

    let snapshot_before = repo
        .read_snapshot("conversation-b", Some(project_path))
        .expect("read")
        .expect("snapshot");

    repo.write_runtime_session("conversation-b", project_path, &session("thread-beta"))
        .expect("write session");

    let snapshot_after = repo
        .read_snapshot("conversation-b", Some(project_path))
        .expect("read again")
        .expect("snapshot");
    assert_eq!(snapshot_before, snapshot_after);
}
