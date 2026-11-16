use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::json;
use spark_storage::conversation::{
    Admission, ExecutionOutcome, MessageInput, RuntimeSession, RUNTIME_SESSION_SCHEMA_VERSION,
};
use spark_storage::{ConversationRepository, ConversationStore, ProjectRegistry};

fn setup(
    project_path: &str,
) -> (
    tempfile::TempDir,
    Arc<ConversationStore>,
    ConversationRepository,
) {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    ProjectRegistry::new(&home)
        .ensure_project_paths(project_path)
        .expect("project paths");
    let store = ConversationStore::open(&home).expect("store");
    (temp, store.clone(), ConversationRepository::new(store))
}

fn start(repo: &ConversationRepository, conversation_id: &str, project_path: &str, turn: &str) {
    start_on(repo, conversation_id, project_path, turn, json!({}));
}

fn start_on(
    repo: &ConversationRepository,
    conversation_id: &str,
    project_path: &str,
    turn: &str,
    execution_settings: serde_json::Value,
) {
    repo.accept_message(
        conversation_id,
        project_path,
        MessageInput {
            admission: Admission::Start,
            user_turn_id: format!("turn-user-{turn}"),
            content: "Hello".to_string(),
            attachments: None,
            client_request_sha256: None,
            expected_settings_revision: 0,
            settings: Default::default(),
            held: false,
            assistant_turn_id: format!("turn-assistant-{turn}"),
            execution_settings,
        },
    )
    .expect("accept message");
}

/// Record the final answer of `turn_id`.
fn answer(repo: &ConversationRepository, conversation_id: &str, project_path: &str, turn_id: &str) {
    let event = serde_json::from_value(json!({"kind": "content_completed",
        "channel": "assistant", "phase": "final_answer", "message": "Done."}))
    .expect("event");
    repo.record_provider_event(
        conversation_id,
        project_path,
        turn_id,
        &event,
        &BTreeMap::new(),
    )
    .expect("answer");
}

fn returned(
    app_thread_id: Option<&str>,
    thread_resume_failure: Option<(String, Option<String>, Option<serde_json::Value>)>,
) -> ExecutionOutcome {
    ExecutionOutcome::Returned {
        final_text: Some("Done.".to_string()),
        token_usage: None,
        token_usage_breakdown: None,
        app_thread_id: app_thread_id.map(str::to_string),
        app_turn_id: None,
        thread_resume_failure,
        continued_question: None,
    }
}

fn session(thread_id: &str) -> serde_json::Value {
    serde_json::to_value(RuntimeSession {
        schema_version: RUNTIME_SESSION_SCHEMA_VERSION,
        provider: "codex_app_server".to_string(),
        thread_id: Some(thread_id.to_string()),
        established_at: "2026-01-01T00:00:00Z".to_string(),
        last_turn_id: Some("turn-assistant".to_string()),
        resume_failed: false,
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        cwd: None,
        configuration_generation: None,
        instructions_sha256: None,
    })
    .expect("session payload")
}

#[test]
fn runtime_session_round_trips_and_missing_session_reads_as_none() {
    let project_path = "/projects/runtime-session";
    let (_temp, _store, repo) = setup(project_path);
    start(&repo, "conversation-a", project_path, "1");

    assert_eq!(
        repo.read_runtime_session("conversation-a", Some(project_path))
            .expect("read absent"),
        None
    );

    answer(&repo, "conversation-a", project_path, "turn-assistant-1");
    let settled = repo
        .settle_execution(
            "conversation-a",
            project_path,
            "turn-assistant-1",
            returned(Some("thread-alpha"), None),
            &BTreeMap::new(),
        )
        .expect("settle");
    assert!(settled.settled);
    let read = repo
        .read_runtime_session("conversation-a", Some(project_path))
        .expect("read")
        .expect("session");
    assert_eq!(read.schema_version, RUNTIME_SESSION_SCHEMA_VERSION);
    assert_eq!(read.provider, "codex_app_server");
    assert_eq!(read.thread_id.as_deref(), Some("thread-alpha"));
    assert_eq!(read.last_turn_id.as_deref(), Some("turn-assistant-1"));
    assert!(!read.resume_failed);
    assert!(!read.established_at.is_empty());

    // A lost provider thread keeps the session for debugging, never resumed.
    start(&repo, "conversation-a", project_path, "2");
    answer(&repo, "conversation-a", project_path, "turn-assistant-2");
    repo.settle_execution(
        "conversation-a",
        project_path,
        "turn-assistant-2",
        returned(
            None,
            Some((
                "The provider thread was lost.".to_string(),
                Some("thread_resume_failed".to_string()),
                None,
            )),
        ),
        &BTreeMap::new(),
    )
    .expect("settle after lost thread");
    let tombstoned = repo
        .read_runtime_session("conversation-a", Some(project_path))
        .expect("read tombstone")
        .expect("tombstoned session");
    assert_eq!(
        tombstoned,
        RuntimeSession {
            resume_failed: true,
            updated_at: tombstoned.updated_at.clone(),
            ..read
        }
    );
}

#[test]
fn runtime_session_presence_never_changes_snapshot_reads() {
    let project_path = "/projects/runtime-session-isolated";
    let (_temp, store, repo) = setup(project_path);

    start(&repo, "conversation-b", project_path, "1");

    let snapshot_before = repo
        .read_snapshot("conversation-b", Some(project_path))
        .expect("read")
        .expect("snapshot");

    crate::conversation_seed::write_rows(&store, |tx, revision| {
        tx.execute(
            "INSERT INTO runtime_sessions (conversation_id, provider, revision, payload) \
             VALUES ('conversation-b', 'codex_app_server', ?1, ?2)",
            rusqlite::params![revision, session("thread-beta").to_string()],
        )
    });
    assert!(repo
        .read_runtime_session("conversation-b", Some(project_path))
        .expect("read session")
        .is_some());

    let snapshot_after = repo
        .read_snapshot("conversation-b", Some(project_path))
        .expect("read again")
        .expect("snapshot");
    assert_eq!(snapshot_before, snapshot_after);
}

/// Run one ACP turn on `provider` that sets up `session_id` with `generation`
/// and settles with `reset`, returning the session the conversation keeps.
fn acp_turn(
    repo: &ConversationRepository,
    project_path: &str,
    turn: &str,
    provider: &str,
    (session_id, generation): (&str, &str),
    reset: Option<(String, Option<String>, Option<serde_json::Value>)>,
) -> RuntimeSession {
    let conversation_id = "conversation-acp";
    let turn_id = format!("turn-assistant-{turn}");
    start_on(
        repo,
        conversation_id,
        project_path,
        turn,
        json!({"provider": provider}),
    );
    let established = serde_json::from_value(json!({
        "kind": "session_established",
        "source": {"backend": "acp", "app_thread_id": session_id},
        "status": "new",
        "details": {
            "harness": provider.trim_start_matches("acp:"),
            "session_id": session_id,
            "cwd": "/work/project",
            "configuration_generation": generation,
            "instructions_sha256": "instructions-hash",
        },
    }))
    .expect("event");
    repo.record_provider_event(
        conversation_id,
        project_path,
        &turn_id,
        &established,
        &BTreeMap::new(),
    )
    .expect("established");
    answer(repo, conversation_id, project_path, &turn_id);
    repo.settle_execution(
        conversation_id,
        project_path,
        &turn_id,
        returned(Some(session_id), reset),
        &BTreeMap::new(),
    )
    .expect("settle");
    repo.read_runtime_session(conversation_id, Some(project_path))
        .expect("read")
        .expect("session")
}

#[test]
fn acp_sessions_are_filed_under_their_harness_with_the_scope_they_were_set_up_in() {
    let project_path = "/projects/runtime-session-acp";
    let (_temp, _store, repo) = setup(project_path);

    let alpha = acp_turn(
        &repo,
        project_path,
        "1",
        "acp:alpha",
        ("shared-id", "gen-a"),
        None,
    );
    assert_eq!(
        (alpha.provider.as_str(), alpha.thread_id.as_deref()),
        ("acp:alpha", Some("shared-id"))
    );
    assert_eq!(alpha.cwd.as_deref(), Some("/work/project"));
    assert_eq!(alpha.configuration_generation.as_deref(), Some("gen-a"));
    assert_eq!(
        alpha.instructions_sha256.as_deref(),
        Some("instructions-hash")
    );

    // Another harness reporting the same opaque id owns a different session.
    let beta = acp_turn(
        &repo,
        project_path,
        "2",
        "acp:beta",
        ("shared-id", "gen-b"),
        None,
    );
    assert_eq!(
        (
            beta.provider.as_str(),
            beta.configuration_generation.as_deref()
        ),
        ("acp:beta", Some("gen-b"))
    );

    // A turn that could not continue its session settles on the fresh one it
    // set up, with that session's scope, and stays resumable.
    let reset = acp_turn(
        &repo,
        project_path,
        "3",
        "acp:beta",
        ("fresh-id", "gen-c"),
        Some((
            "ACP harness beta could not continue session shared-id".to_string(),
            Some("thread_resume_failure".to_string()),
            None,
        )),
    );
    assert_eq!(reset.thread_id.as_deref(), Some("fresh-id"));
    assert_eq!(reset.provider, "acp:beta");
    assert!(!reset.resume_failed);
    assert_eq!(reset.configuration_generation.as_deref(), Some("gen-c"));
    assert_eq!(reset.cwd.as_deref(), Some("/work/project"));
}
