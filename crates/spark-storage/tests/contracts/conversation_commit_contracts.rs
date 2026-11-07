use std::collections::BTreeMap;
use std::fs;

use serde_json::{json, Value};
use spark_storage::conversation::{
    record_from_snapshot, snapshot_from_record, Accepted, Admission, Answer, ArtifactChange,
    ArtifactCollection, ArtifactOutcome, CommitNotice, ConversationCommit,
    ConversationMetadataPatch, EntityKind, EntityRow, ExecutionOutcome, MessageInput, Settlement,
    TransientStreamBody, TransientStreamEvent, TOOL_OUTPUT_INLINE_LIMIT_BYTES,
};
use spark_storage::{ConversationRepository, ConversationStore, ProjectRegistry, StorageError};

fn setup(project_path: &str) -> (tempfile::TempDir, ConversationRepository) {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    ProjectRegistry::new(&home)
        .ensure_project_paths(project_path)
        .expect("project paths");
    let store = ConversationStore::open(&home).expect("store");
    (temp, ConversationRepository::new(store))
}

fn conversation_dir(
    temp: &tempfile::TempDir,
    project_path: &str,
    conversation_id: &str,
) -> std::path::PathBuf {
    ProjectRegistry::new(temp.path().join("spark-home"))
        .ensure_project_paths(project_path)
        .expect("project paths")
        .conversations_dir
        .join(conversation_id)
}

/// The full projection of a conversation's committed state.
fn snapshot(repo: &ConversationRepository, conversation_id: &str) -> Value {
    repo.read_snapshot(conversation_id, None)
        .expect("read")
        .expect("snapshot")
}

/// The row a commit changed, by kind and id.
fn changed<'a>(commit: &'a ConversationCommit, kind: EntityKind, id: &str) -> &'a EntityRow {
    commit
        .changes
        .rows
        .iter()
        .find(|row| row.kind == kind && row.id == id)
        .unwrap_or_else(|| panic!("changed {kind:?} {id}"))
}

fn message(user_turn_id: &str, assistant_turn_id: &str, content: &str) -> MessageInput {
    MessageInput {
        admission: Admission::Start,
        user_turn_id: user_turn_id.to_string(),
        content: content.to_string(),
        attachments: None,
        client_request_sha256: None,
        expected_settings_revision: 0,
        settings: ConversationMetadataPatch::default(),
        held: false,
        assistant_turn_id: assistant_turn_id.to_string(),
        execution_settings: json!({}),
    }
}

/// Accept a message that starts `turn-assistant`.
fn start(
    repo: &ConversationRepository,
    conversation_id: &str,
    project_path: &str,
    content: &str,
) -> ConversationCommit {
    let (commit, accepted) = repo
        .accept_message(
            conversation_id,
            project_path,
            message("turn-user", "turn-assistant", content),
        )
        .expect("accept message");
    assert!(matches!(accepted, Accepted::Started { .. }), "{accepted:?}");
    commit.expect("acceptance commit")
}

/// Record one provider event of the running `turn-assistant`.
fn record(
    repo: &ConversationRepository,
    conversation_id: &str,
    project_path: &str,
    event: Value,
) -> Option<ConversationCommit> {
    repo.record_provider_event(
        conversation_id,
        project_path,
        "turn-assistant",
        &serde_json::from_value(event).expect("provider event"),
        &BTreeMap::new(),
    )
    .expect("record event")
    .commit
}

/// A completed tool call of `turn-assistant`.
fn tool_event(tool_call: Value) -> Value {
    json!({"kind": "tool_call_completed", "tool_call": tool_call})
}

/// Record a completed tool call carrying `output` (externalized when large).
fn record_tool(
    repo: &ConversationRepository,
    conversation_id: &str,
    project_path: &str,
    call_id: &str,
    output: &str,
) -> ConversationCommit {
    record(
        repo,
        conversation_id,
        project_path,
        tool_event(json!({"id": call_id, "output": output})),
    )
    .expect("tool commit")
}

/// The segment a tool call of `turn-assistant` writes.
fn tool_segment_id(call_id: &str) -> String {
    format!("segment-tool-turn-assistant-{call_id}")
}

/// The blob a commit's tool segment references, relative to its conversation.
fn output_artifact(commit: &ConversationCommit, call_id: &str) -> String {
    changed(commit, EntityKind::Segment, &tool_segment_id(call_id))
        .payload
        .as_ref()
        .and_then(|segment| segment["tool_call"]["output_artifact"].as_str())
        .expect("output artifact")
        .to_string()
}

/// Answer and settle `turn-assistant`.
fn settle(repo: &ConversationRepository, conversation_id: &str, project_path: &str) -> Settlement {
    record(
        repo,
        conversation_id,
        project_path,
        json!({"kind": "content_completed", "channel": "assistant",
            "phase": "final_answer", "message": "Done."}),
    );
    repo.settle_execution(
        conversation_id,
        project_path,
        "turn-assistant",
        ExecutionOutcome::Returned {
            final_text: Some("Done.".to_string()),
            token_usage: None,
            token_usage_breakdown: None,
            app_thread_id: None,
            app_turn_id: None,
            thread_resume_failure: None,
        },
        &BTreeMap::new(),
    )
    .expect("settle")
}

#[test]
fn accepting_a_message_creates_the_conversation_and_returns_only_changed_rows() {
    let project_path = "/projects/commit-basics";
    let _temp = tempfile::tempdir().expect("tempdir");
    let home = _temp.path().join("spark-home");
    let repo = ConversationRepository::new(ConversationStore::open(&home).expect("store"));

    let commit = start(
        &repo,
        "conversation-a",
        project_path,
        "Hello there commit boundary",
    );

    // One home-global revision per transaction, stamped on every changed row.
    assert_eq!(commit.revision, 1);
    assert_eq!(commit.changes.from_revision, 0);
    assert_eq!(commit.changes.watermark, 1);
    assert_eq!(commit.changes.rows.len(), 3);
    assert!(commit.changes.rows.iter().all(|row| row.revision == 1));
    let conversation = changed(&commit, EntityKind::Conversation, "conversation-a");
    let summary = conversation.payload.as_ref().expect("summary");
    assert_eq!(summary["title"], "Hello there commit boundary");
    assert_eq!(summary["handle"].as_str().unwrap().split('-').count(), 2);
    assert!(!summary["created_at"].as_str().unwrap().is_empty());
    assert!(summary.get("turns").is_none());
    assert_eq!(
        changed(&commit, EntityKind::Turn, "turn-user")
            .payload
            .as_ref()
            .unwrap()["content"],
        "Hello there commit boundary"
    );

    let second = record(
        &repo,
        "conversation-a",
        project_path,
        json!({"kind": "tool_call_started", "tool_call": {"id": "call-a"}}),
    )
    .expect("second commit");
    assert_eq!(second.revision, 2);
    // Only the new segment, the turn it started streaming and the
    // conversation's summary changed.
    assert_eq!(second.changes.rows.len(), 3, "{:?}", second.changes.rows);
    assert_eq!(
        changed(&second, EntityKind::Segment, &tool_segment_id("call-a")).order,
        Some(1)
    );
    assert_eq!(
        changed(&second, EntityKind::Turn, "turn-assistant")
            .payload
            .as_ref()
            .unwrap()["status"],
        "streaming"
    );

    let persisted = repo
        .read_snapshot("conversation-a", Some(project_path))
        .expect("read")
        .expect("snapshot");
    assert_eq!(persisted["revision"], 2);
    assert_eq!(persisted["turns"].as_array().expect("turns").len(), 2);
    assert_eq!(persisted["segments"][0]["id"], tool_segment_id("call-a"));
    // Every entity is a database row; no split conversation files exist.
    let stored = repo
        .store()
        .database()
        .read(|read| read.conversation("conversation-a"))
        .expect("rows")
        .expect("conversation row");
    assert_eq!(stored.turns.len(), 2);
    assert_eq!(stored.segments.len(), 1);
    assert_eq!(stored.metadata["revision"], 2);
    assert!(stored.metadata.get("turns").is_none());
    let root = conversation_dir(&_temp, project_path, "conversation-a");
    for legacy in [
        "conversation.json",
        "transcript.jsonl",
        "event-log.json",
        "events.jsonl",
        "provider-events.jsonl",
        "artifacts",
    ] {
        assert!(!root.join(legacy).exists(), "unexpected {legacy}");
    }
    let project = ProjectRegistry::new(_temp.path().join("spark-home"))
        .ensure_project_paths(project_path)
        .expect("project paths");
    assert!(!project
        .flow_run_requests_dir
        .join("conversation-a.json")
        .exists());
}

#[test]
fn provider_events_keep_segment_orders_stable_for_late_updates() {
    let project_path = "/projects/commit-orders";
    let (_temp, repo) = setup(project_path);
    start(&repo, "conversation-c", project_path, "Run two tools");
    for call_id in ["call-early", "call-late"] {
        record(
            &repo,
            "conversation-c",
            project_path,
            json!({"kind": "tool_call_started", "tool_call": {"id": call_id}}),
        )
        .expect("started");
    }
    let started = snapshot(&repo, "conversation-c")["revision"]
        .as_i64()
        .unwrap();

    // A late completion of the older tool keeps its original order even
    // though a newer segment now exists.
    let late = record_tool(&repo, "conversation-c", project_path, "call-early", "final");

    let persisted = repo
        .read_snapshot("conversation-c", Some(project_path))
        .expect("read")
        .expect("snapshot");
    let segments = persisted["segments"].as_array().expect("segments");
    let find = |id: &str| {
        segments
            .iter()
            .find(|segment| segment["id"] == id)
            .unwrap_or_else(|| panic!("segment {id}"))
    };
    let early = find(&tool_segment_id("call-early"));
    assert_eq!(early["order"], 1);
    assert_eq!(early["status"], "complete");
    assert_eq!(early["tool_call"]["output"], "final");
    assert_eq!(find(&tool_segment_id("call-late"))["order"], 2);
    assert_eq!(late.revision, started + 1);
}

/// Commit notices name the segments a commit ends and the executions it
/// settles or removes, so a listener can retire their display text. A return
/// that only waits for an answer settles nothing.
#[test]
fn commit_notices_name_ended_segments_and_settled_executions() {
    let project_path = "/projects/commit-notices";
    let (_temp, repo) = setup(project_path);
    let notices = std::sync::Arc::new(std::sync::Mutex::new(Vec::<CommitNotice>::new()));
    let sink = notices.clone();
    repo.store()
        .set_commit_listener(std::sync::Arc::new(move |notice: &CommitNotice| {
            sink.lock().unwrap().push(notice.clone());
        }));
    let since = |count: usize| notices.lock().unwrap()[count..].to_vec();
    let event = |conversation_id: &str, value: Value, held: &BTreeMap<String, String>| {
        repo.record_provider_event(
            conversation_id,
            project_path,
            "turn-assistant",
            &serde_json::from_value(value).expect("provider event"),
            held,
        )
        .expect("record event")
    };
    let delta = json!({"kind": "content_delta", "channel": "assistant",
        "content_delta": "Let", "source": {"item_id": "item-1"}});
    let mine = |segment_id: &str| {
        (
            "conversation-n".to_string(),
            "turn-assistant".to_string(),
            segment_id.to_string(),
        )
    };
    start(&repo, "conversation-n", project_path, "Ask first");

    // Opening a segment ends nothing; its end names it.
    let count = notices.lock().unwrap().len();
    let (segment_id, committed) = event("conversation-n", delta.clone(), &BTreeMap::new())
        .opened
        .expect("opened");
    assert_eq!(committed, "");
    let held = BTreeMap::from([(segment_id.clone(), "Let me ask.".to_string())]);
    let ended = event(
        "conversation-n",
        json!({"kind": "content_completed", "channel": "assistant",
            "message": "Let me ask.", "source": {"item_id": "item-1"}}),
        &held,
    );
    assert_eq!(ended.ended, [segment_id.clone()]);
    let closed: Vec<_> = since(count)
        .into_iter()
        .flat_map(|notice| notice.closed_segments)
        .collect();
    assert_eq!(closed, [mine(&segment_id)]);

    // A question-only return keeps the execution open.
    event(
        "conversation-n",
        json!({"kind": "request_user_input_requested", "source": {"item_id": "input-1"},
            "request_user_input": {"itemId": "input-1", "questions": [{"id": "decision",
                "header": "Approve", "question": "Approve?", "isOther": false,
                "isSecret": false, "options": [{"label": "Approve", "description": "Go"}]}]}}),
        &BTreeMap::new(),
    );
    let count = notices.lock().unwrap().len();
    let waiting = repo
        .settle_execution(
            "conversation-n",
            project_path,
            "turn-assistant",
            ExecutionOutcome::Returned {
                final_text: None,
                token_usage: None,
                token_usage_breakdown: None,
                app_thread_id: None,
                app_turn_id: None,
                thread_resume_failure: None,
            },
            &BTreeMap::new(),
        )
        .expect("question-only return");
    assert!(!waiting.settled);
    assert!(since(count)
        .iter()
        .all(|notice| notice.settled_executions.is_empty()));

    // The answer's continuation settles it, once.
    let (_, answer) = repo
        .answer_question(
            "conversation-n",
            project_path,
            "decision",
            &BTreeMap::from([("decision".to_string(), "Approve".to_string())]),
        )
        .expect("answer");
    assert!(matches!(answer, Answer::Answered(_)), "{answer:?}");
    let count = notices.lock().unwrap().len();
    assert!(settle(&repo, "conversation-n", project_path).settled);
    let settled: Vec<_> = since(count)
        .into_iter()
        .flat_map(|notice| notice.settled_executions)
        .collect();
    assert_eq!(
        settled,
        [("conversation-n".to_string(), "turn-assistant".to_string())]
    );

    // Deleting a conversation removes its open execution.
    start(
        &repo,
        "conversation-gone",
        project_path,
        "Stream then vanish",
    );
    event("conversation-gone", delta, &BTreeMap::new());
    let count = notices.lock().unwrap().len();
    repo.delete_conversation("conversation-gone", project_path)
        .expect("delete");
    let removed = since(count);
    assert_eq!(removed.len(), 1);
    assert_eq!(
        removed[0].settled_executions,
        [(
            "conversation-gone".to_string(),
            "turn-assistant".to_string()
        )]
    );
}

/// A steer the running turn accepted but ended without reading fails on the
/// provider's report, though no delivered event came and the execution never
/// held its row. Text held when it was accepted stays before it, and the
/// turn still settles.
#[test]
fn a_steer_the_turn_never_read_fails_on_the_providers_report() {
    let project_path = "/projects/commit-steer";
    let (_temp, repo) = setup(project_path);
    start(&repo, "conversation-s", project_path, "Start");
    let opened = repo
        .record_provider_event(
            "conversation-s",
            project_path,
            "turn-assistant",
            &serde_json::from_value(json!({"kind": "content_delta", "channel": "assistant",
                "content_delta": "Work", "source": {"item_id": "item-1"}}))
            .expect("event"),
            &BTreeMap::new(),
        )
        .expect("delta");
    let (open_segment, _) = opened.opened.expect("opened");
    let mut steer = message("turn-steer", "unused", "Also do this");
    steer.admission = Admission::Send;
    let (_, accepted) = repo
        .accept_message("conversation-s", project_path, steer)
        .expect("steer");
    assert!(
        matches!(accepted, Accepted::DeliveryPending { .. }),
        "{accepted:?}"
    );
    repo.record_message_delivery("conversation-s", project_path, "turn-steer", true)
        .expect("handed to the provider");

    let held = BTreeMap::from([(open_segment.clone(), "Working".to_string())]);
    record(
        &repo,
        "conversation-s",
        project_path,
        json!({"kind": "user_message_not_delivered", "source": {"item_id": "turn-steer"},
            "message": "Not sent: the turn ended before the agent read it."}),
    )
    .expect("report commit");
    repo.record_provider_event(
        "conversation-s",
        project_path,
        "turn-assistant",
        &serde_json::from_value(json!({"kind": "content_completed", "channel": "assistant",
            "message": "Working", "source": {"item_id": "item-1"}}))
        .expect("event"),
        &held,
    )
    .expect("completion");
    assert!(settle(&repo, "conversation-s", project_path).settled);

    let persisted = snapshot(&repo, "conversation-s");
    let segment = |id: &str| {
        persisted["segments"]
            .as_array()
            .unwrap()
            .iter()
            .find(|segment| segment["id"] == id)
            .unwrap_or_else(|| panic!("segment {id}: {persisted}"))
            .clone()
    };
    let steer = segment("segment-user-message-turn-steer");
    assert_eq!(steer["status"], "failed", "{steer}");
    assert_eq!(
        steer["error"],
        "Not sent: the turn ended before the agent read it."
    );
    assert_eq!(steer["content"], "Also do this");
    assert_eq!(segment(&open_segment)["content"], "Working");
    let assistant = persisted["turns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|turn| turn["id"] == "turn-assistant")
        .unwrap();
    assert_eq!(assistant["status"], "complete", "{assistant}");
}

/// Operations on a conversation that does not exist, or on a turn that owns
/// no execution, write nothing.
#[test]
fn operations_on_missing_conversations_or_turns_write_nothing() {
    let project_path = "/projects/commit-rejects";
    let (_temp, repo) = setup(project_path);
    let watermark = || {
        repo.store()
            .database()
            .read(|read| Ok(read.watermark()))
            .unwrap()
    };
    let event = || -> spark_common::events::TurnStreamEvent {
        serde_json::from_value(json!({"kind": "tool_call_started", "tool_call": {"id": "x"}}))
            .unwrap()
    };

    let error = repo
        .record_provider_event(
            "conversation-d",
            project_path,
            "turn-assistant",
            &event(),
            &BTreeMap::new(),
        )
        .expect_err("event for an unknown conversation");
    assert!(matches!(
        error,
        StorageError::ConversationCommitRejected { .. }
    ));
    let error = repo
        .change_artifacts(
            "conversation-d",
            project_path,
            Vec::new(),
            Some("flow launched".to_string()),
        )
        .expect_err("artifacts of an unknown conversation");
    assert!(matches!(
        error,
        StorageError::ConversationCommitRejected { .. }
    ));
    assert_eq!(watermark(), 0);
    assert!(repo
        .read_snapshot("conversation-d", Some(project_path))
        .expect("read")
        .is_none());

    // An event for a turn that owns no execution is ignored: no revision.
    repo.touch_conversation("conversation-d", project_path)
        .expect("create");
    let before = snapshot(&repo, "conversation-d");
    let mark = watermark();
    let outcome = repo
        .record_provider_event(
            "conversation-d",
            project_path,
            "turn-missing",
            &event(),
            &BTreeMap::new(),
        )
        .expect("ignored event");
    assert!(outcome.commit.is_none());
    assert_eq!(watermark(), mark);
    assert_eq!(snapshot(&repo, "conversation-d"), before);
}

#[test]
fn settings_updates_and_workflow_events_commit_as_rows() {
    let project_path = "/projects/commit-metadata";
    let (_temp, repo) = setup(project_path);

    let (commit, _) = repo
        .update_settings(
            "conversation-e",
            project_path,
            None,
            ConversationMetadataPatch {
                chat_mode: Some("plan".to_string()),
                provider: Some("codex".to_string()),
                model: Some(Some("gpt-5.3-codex".to_string())),
                ..ConversationMetadataPatch::default()
            },
        )
        .expect("settings commit");
    let commit = commit.expect("commit");
    assert_eq!(commit.revision, 1);
    // The metadata travels as the conversation row; the mode change as a turn.
    let metadata = &changed(&commit, EntityKind::Conversation, "conversation-e")
        .payload
        .as_ref()
        .unwrap()["metadata"];
    assert_eq!(metadata["chat_mode"], "plan");
    assert_eq!(metadata["model"], "gpt-5.3-codex");
    let settings_revision = metadata["settings_revision"].as_i64().unwrap();
    assert_eq!(settings_revision, commit.revision);
    assert!(commit
        .changes
        .rows
        .iter()
        .any(|row| row.kind == EntityKind::Turn
            && row.payload.as_ref().unwrap()["kind"] == "mode_change"));

    // The workflow event is its own row.
    let (event, outcome) = repo
        .change_artifacts(
            "conversation-e",
            project_path,
            Vec::new(),
            Some("flow launched".to_string()),
        )
        .expect("event commit");
    assert_eq!(outcome, ArtifactOutcome::Changed(Vec::new()));
    assert!(event
        .expect("event commit")
        .changes
        .rows
        .iter()
        .any(|row| row.kind == EntityKind::Artifact
            && row.collection.as_deref() == Some("event_log")));

    let (cleared, _) = repo
        .update_settings(
            "conversation-e",
            project_path,
            Some(settings_revision),
            ConversationMetadataPatch {
                model: Some(None),
                ..ConversationMetadataPatch::default()
            },
        )
        .expect("clear model");
    assert_eq!(
        changed(
            &cleared.expect("clear commit"),
            EntityKind::Conversation,
            "conversation-e"
        )
        .payload
        .as_ref()
        .unwrap()["metadata"]["model"],
        Value::Null
    );

    let persisted = repo
        .read_snapshot("conversation-e", Some(project_path))
        .expect("read")
        .expect("snapshot");
    assert_eq!(persisted["chat_mode"], "plan");
    assert_eq!(persisted["model"], Value::Null);
    assert_eq!(persisted["event_log"][0]["message"], "flow launched");
}

#[test]
fn tool_events_externalize_oversized_outputs() {
    let project_path = "/projects/commit-tool-output";
    let (temp, repo) = setup(project_path);
    let big_output = format!(
        "line\u{2014}{}",
        "y".repeat(TOOL_OUTPUT_INLINE_LIMIT_BYTES * 2)
    );
    let segment_id = tool_segment_id("call-1");
    let read_output = |segment_id: &str| {
        repo.read_segment_tool_output("conversation-h", Some(project_path), segment_id)
            .expect("read output")
    };

    start(&repo, "conversation-h", project_path, "Run tools");
    let commit = record_tool(&repo, "conversation-h", project_path, "call-1", &big_output);
    record_tool(
        &repo,
        "conversation-h",
        project_path,
        "../escape",
        &big_output,
    );

    // The persisted transcript and the commit both carry the bounded preview.
    let persisted = snapshot(&repo, "conversation-h");
    let stored_segment = |id: &str| {
        persisted["segments"]
            .as_array()
            .expect("segments")
            .iter()
            .find(|segment| segment["id"] == id)
            .unwrap_or_else(|| panic!("segment {id}"))["tool_call"]
            .clone()
    };
    let stored = stored_segment(&segment_id);
    assert_eq!(stored["output_truncated"], true);
    assert_eq!(
        stored["output_size"].as_u64().expect("size") as usize,
        big_output.len()
    );
    assert!(stored["output"].as_str().expect("preview").len() <= TOOL_OUTPUT_INLINE_LIMIT_BYTES);
    assert_eq!(
        changed(&commit, EntityKind::Segment, &segment_id)
            .payload
            .as_ref()
            .unwrap()["tool_call"]["output_truncated"],
        true
    );

    // An immutable blob holds the full output and serves the read path.
    let artifact = stored["output_artifact"].as_str().expect("artifact");
    assert!(
        artifact.starts_with(&format!("tool-output/{segment_id}.")),
        "{artifact}"
    );
    let sidecar = conversation_dir(&temp, project_path, "conversation-h").join(artifact);
    assert_eq!(fs::read_to_string(&sidecar).expect("sidecar"), big_output);
    assert_eq!(read_output(&segment_id), Some(big_output.clone()));

    // Unsafe segment ids stay inline and never touch the filesystem.
    let unsafe_id = tool_segment_id("../escape");
    let unsafe_stored = stored_segment(&unsafe_id);
    assert!(unsafe_stored.get("output_truncated").is_none());
    assert_eq!(unsafe_stored["output"], json!(big_output));
    assert_eq!(read_output(&unsafe_id), None);

    // An event echoing the preview must not overwrite the blob with
    // truncated content or drop its reference.
    record(
        &repo,
        "conversation-h",
        project_path,
        tool_event(json!({
            "id": "call-1",
            "output": stored["output"],
            "output_size": stored["output_size"],
            "output_truncated": true,
            "status": "complete",
        })),
    );
    assert_eq!(fs::read_to_string(&sidecar).expect("sidecar"), big_output);
    assert_eq!(read_output(&segment_id), Some(big_output.clone()));

    // A new output version gets a new blob; the old bytes stay until cleanup.
    let second_output = "z".repeat(TOOL_OUTPUT_INLINE_LIMIT_BYTES * 2);
    let replaced = record_tool(
        &repo,
        "conversation-h",
        project_path,
        "call-1",
        &second_output,
    );
    assert_ne!(output_artifact(&replaced, "call-1"), artifact);
    assert_eq!(
        fs::read_to_string(&sidecar).expect("old blob kept"),
        big_output
    );
    assert_eq!(read_output(&segment_id), Some(second_output));
}

#[test]
fn commit_snapshot_projection_round_trips_read_snapshot_output() {
    let project_path = "/projects/commit-roundtrip";
    let (_temp, repo) = setup(project_path);

    start(&repo, "conversation-f", project_path, "Round trip");
    record(
        &repo,
        "conversation-f",
        project_path,
        json!({"kind": "session_start"}),
    )
    .expect("agent event");
    settle(&repo, "conversation-f", project_path);
    let (_, outcome) = repo
        .change_artifacts(
            "conversation-f",
            project_path,
            vec![ArtifactChange::Create {
                collection: ArtifactCollection::FlowRunRequests,
                artifact: json!({"id": "request-a", "status": "pending_review"}),
                owner: None,
                anchor_kind: "flow_run_request".to_string(),
            }],
            None,
        )
        .expect("artifact commit");
    assert!(
        matches!(outcome, ArtifactOutcome::Changed(_)),
        "{outcome:?}"
    );

    let snapshot = repo
        .read_snapshot("conversation-f", Some(project_path))
        .expect("read")
        .expect("snapshot");
    let record = record_from_snapshot(&snapshot).expect("record");
    let segments = &record.transcript.segments;
    assert!(segments
        .iter()
        .any(|segment| segment.extra.get("event_kind") == Some(&json!("session_start"))));
    let anchor = segments
        .iter()
        .find(|segment| segment.artifact_id.as_deref() == Some("request-a"))
        .expect("anchor segment");
    assert_eq!(anchor.kind, "flow_run_request");
    assert_eq!(anchor.turn_id, "turn-assistant");
    let reprojected = snapshot_from_record(&record);
    assert_eq!(reprojected, snapshot);
}

#[test]
fn transient_stream_events_are_never_stored() {
    let project_path = "/projects/commit-transient";
    let (_temp, repo) = setup(project_path);
    start(&repo, "conversation-g", project_path, "Stream");

    // Transient events carry a stream sequence instead of a revision.
    let transient = TransientStreamEvent {
        conversation_id: "conversation-g".to_string(),
        turn_id: "turn-assistant".to_string(),
        stream_sequence: 4,
        base_revision: 1,
        body: TransientStreamBody::SegmentText {
            segment_id: "segment-live".to_string(),
            text: "partial".to_string(),
        },
    };
    let payload = serde_json::to_value(&transient).expect("serialize");
    assert!(payload.get("revision").is_none());

    // Only the committed turns are stored.
    let stored = repo
        .store()
        .database()
        .read(|read| read.conversation("conversation-g"))
        .expect("rows")
        .expect("conversation");
    assert_eq!(stored.turns.len(), 2);
    assert!(stored.segments.is_empty());
}

#[test]
fn concurrent_operations_serialize_revision_allocation_and_keep_all_turns() {
    let project_path = "/projects/concurrent-commit";
    let (_temp, repository) = setup(project_path);
    repository
        .append_system_turn("conversation-concurrent", project_path, "notice", "Initial")
        .unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..8)
            .map(|i| {
                let repository = repository.clone();
                let barrier = barrier.clone();
                scope.spawn(move || {
                    barrier.wait();
                    repository
                        .append_system_turn(
                            "conversation-concurrent",
                            project_path,
                            "notice",
                            &format!("Concurrent turn {i}"),
                        )
                        .unwrap()
                        .expect("commit")
                })
            })
            .collect();
        let revisions: std::collections::BTreeSet<_> = workers
            .into_iter()
            .map(|w| w.join().unwrap().revision)
            .collect();
        assert_eq!(revisions.len(), 8);
    });
    let snapshot = repository
        .read_snapshot("conversation-concurrent", Some(project_path))
        .unwrap()
        .unwrap();
    assert_eq!(snapshot["turns"].as_array().unwrap().len(), 9);
}

#[test]
fn competing_settings_updates_conflict_without_changing_history() {
    let project_path = "/projects/settings-conflict";
    let (_temp, repo) = setup(project_path);
    start(&repo, "chat", project_path, "Preserve this transcript");
    settle(&repo, "chat", project_path);
    let patches = [
        ConversationMetadataPatch {
            chat_mode: Some("plan".into()),
            ..Default::default()
        },
        ConversationMetadataPatch {
            model: Some(Some("gpt-5.3-codex".into())),
            ..Default::default()
        },
    ];
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let writers: Vec<_> = patches
        .iter()
        .cloned()
        .map(|patch| {
            let repo = repo.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                repo.update_settings("chat", project_path, Some(0), patch)
            })
        })
        .collect();
    let results: Vec<_> = writers
        .into_iter()
        .map(|writer| writer.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(StorageError::ConversationCommitRejected { .. })))
            .count(),
        1
    );
    let winner_snapshot = snapshot(&repo, "chat");
    let mode_changes = usize::from(winner_snapshot["chat_mode"] == "plan");
    assert_eq!(
        winner_snapshot["turns"].as_array().unwrap().len(),
        2 + mode_changes
    );
    let rows = || {
        repo.store()
            .database()
            .read(|read| read.conversation("chat"))
            .unwrap()
    };
    let before = rows();
    for patch in patches {
        assert!(matches!(
            repo.update_settings("chat", project_path, Some(0), patch),
            Err(StorageError::ConversationCommitRejected { .. })
        ));
    }
    // A message whose execution settings came from the old settings writes
    // nothing either.
    let (commit, accepted) = repo
        .accept_message(
            "chat",
            project_path,
            message("turn-user-2", "turn-assistant-2", "Next"),
        )
        .unwrap();
    assert_eq!(accepted, Accepted::SettingsChanged);
    assert!(commit.is_none());
    assert_eq!(rows(), before);
    assert_eq!(snapshot(&repo, "chat"), winner_snapshot);

    // Operations without settings still commit on top of the winner.
    repo.append_system_turn("chat", project_path, "notice", "Still here")
        .unwrap()
        .expect("commit");
    let after = snapshot(&repo, "chat");
    assert_eq!(after["chat_mode"], winner_snapshot["chat_mode"]);
    assert_eq!(after["model"], winner_snapshot["model"]);
    let winner_turns = winner_snapshot["turns"].as_array().unwrap();
    assert_eq!(
        after["turns"].as_array().unwrap()[..winner_turns.len()],
        winner_turns[..]
    );
}

/// A failed transaction leaves the database at the previous revision;
/// nothing staged becomes visible.
#[test]
fn failed_transactions_roll_back_rows() {
    let project_path = "/projects/rollback";
    let (temp, repo) = setup(project_path);
    start(&repo, "conversation-rollback", project_path, "Hello");
    settle(&repo, "conversation-rollback", project_path);
    let rows = || {
        repo.store()
            .database()
            .read(|read| read.conversation("conversation-rollback"))
            .unwrap()
    };
    let before = rows();
    let before_snapshot = snapshot(&repo, "conversation-rollback");
    let plan = ConversationMetadataPatch {
        chat_mode: Some("plan".to_string()),
        ..Default::default()
    };
    // The mode change is staged before the unknown attachment rejects the
    // message inside the transaction.
    let error = repo
        .accept_message(
            "conversation-rollback",
            project_path,
            MessageInput {
                attachments: Some(json!([{"id": "att-missing"}])),
                settings: plan.clone(),
                ..message("turn-second", "turn-answer", "Second")
            },
        )
        .expect_err("unknown attachment");
    assert!(
        matches!(error, StorageError::ConversationCommitRejected { .. }),
        "{error}"
    );
    assert_eq!(rows(), before);
    assert_eq!(snapshot(&repo, "conversation-rollback"), before_snapshot);

    // Retrying the valid part, even twice, commits the message once.
    let retry = || {
        repo.accept_message(
            "conversation-rollback",
            project_path,
            MessageInput {
                settings: plan.clone(),
                ..message("turn-second", "turn-answer", "Second")
            },
        )
        .unwrap()
    };
    let (first, accepted) = retry();
    assert!(matches!(accepted, Accepted::Started { .. }), "{accepted:?}");
    assert!(first.is_some());
    let (again, _) = retry();
    assert!(again.is_none());
    // What a reopened store reads matches the committed state.
    let committed = self::snapshot(&repo, "conversation-rollback");
    drop(repo);
    let repo = ConversationRepository::new(
        ConversationStore::open(temp.path().join("spark-home")).expect("reopen"),
    );
    assert_eq!(self::snapshot(&repo, "conversation-rollback"), committed);
    let turns: Vec<_> = committed["turns"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|turn| turn["kind"] == "message")
        .map(|turn| turn["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        turns,
        vec!["turn-user", "turn-assistant", "turn-second", "turn-answer"]
    );
    let mode_changes = committed["turns"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|turn| turn["kind"] == "mode_change")
        .count();
    assert_eq!(mode_changes, 1);
}

/// Externalized output is read from its blob; inline output has no blob;
/// a preview whose blob is missing is an error, never complete output.
#[test]
fn segment_tool_output_distinguishes_inline_present_and_missing_bytes() {
    let project_path = "/projects/tool-output-reads";
    let (temp, repo) = setup(project_path);
    let root = conversation_dir(&temp, project_path, "conversation-reads");
    fs::create_dir_all(root.join("tool-output")).unwrap();
    fs::write(root.join("tool-output/present.txt"), "full present output").unwrap();
    start(&repo, "conversation-reads", project_path, "Read outputs");
    for tool_call in [
        json!({"id": "inline", "output": "inline output"}),
        json!({"id": "present", "output": "PREVIEW", "output_truncated": true,
            "output_size": 19, "output_artifact": "tool-output/present.txt"}),
        json!({"id": "missing", "output": "ONLY PREVIEW", "output_truncated": true,
            "output_size": 100000, "output_artifact": "tool-output/missing.txt"}),
    ] {
        record(
            &repo,
            "conversation-reads",
            project_path,
            tool_event(tool_call),
        )
        .expect("tool commit");
    }
    let read = |call_id: &str| {
        repo.read_segment_tool_output(
            "conversation-reads",
            Some(project_path),
            &tool_segment_id(call_id),
        )
    };
    assert_eq!(read("inline").unwrap(), None);
    assert_eq!(
        read("present").unwrap().as_deref(),
        Some("full present output")
    );
    assert!(matches!(
        read("missing"),
        Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound
    ));
}

/// Recovery collects tool-output blobs nothing needs, only behind the
/// cleanup barrier: an output version a later event replaced, a tombstoned
/// segment's output, and a publication whose transaction failed. Live bytes
/// stay; a deferred barrier keeps everything until the next startup retries.
#[test]
fn recovery_collects_unreferenced_tool_outputs_behind_the_barrier() {
    let project_path = "/projects/tool-output-garbage";
    let (temp, repo) = setup(project_path);
    let conversation = "conversation-garbage";
    let root = conversation_dir(&temp, project_path, conversation);
    let blobs = || -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(root.join("tool-output"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    };

    start(&repo, conversation, project_path, "Run tools");
    record_tool(
        &repo,
        conversation,
        project_path,
        "replaced",
        &big_output("a"),
    );
    record_tool(
        &repo,
        conversation,
        project_path,
        "removed",
        &big_output("b"),
    );
    assert_eq!(blobs().len(), 2);
    // A new version replaces the first; the other segment is tombstoned.
    let second = record_tool(
        &repo,
        conversation,
        project_path,
        "replaced",
        &big_output("c"),
    );
    repo.store()
        .database()
        .write(|tx| tx.delete_segment(conversation, &tool_segment_id("removed")))
        .unwrap();
    // A publication whose transaction failed leaves its blob unreferenced.
    fs::write(
        root.join("tool-output/segment-abandoned.0.txt"),
        big_output("d"),
    )
    .unwrap();
    let live_blob = output_artifact(&second, "replaced")
        .trim_start_matches("tool-output/")
        .to_string();
    let before = blobs();
    assert_eq!(before.len(), 4, "{before:?}");

    // A busy barrier removes nothing.
    repo.store()
        .database()
        .fail_next_checkpoints([spark_storage::conversation::CheckpointFault::Busy]);
    assert!(matches!(
        repo.store().collect_tool_output_garbage(),
        Err(StorageError::CleanupDeferred { .. })
    ));
    assert_eq!(blobs(), before);

    // The next startup collects exactly the garbage.
    drop(repo);
    let repo = ConversationRepository::new(
        ConversationStore::open(temp.path().join("spark-home")).expect("reopen"),
    );
    assert_eq!(blobs(), vec![live_blob]);
    assert_eq!(
        repo.read_segment_tool_output(
            conversation,
            Some(project_path),
            &tool_segment_id("replaced")
        )
        .unwrap(),
        Some(big_output("c"))
    );
    assert_eq!(repo.store().collect_tool_output_garbage().unwrap(), 0);
}

/// An open store checkpoints written transactions in the background.
#[test]
fn open_store_checkpoints_writes_periodically() {
    let project_path = "/projects/periodic-checkpoint";
    let (_temp, repo) = setup(project_path);
    start(&repo, "conversation-periodic", project_path, "Hello");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while repo.store().database().checkpoint_pending() {
        assert!(
            std::time::Instant::now() < deadline,
            "no passive checkpoint"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

const SHARED_ID_CHILD_ENV: &str = "SPARK_SHARED_CONVERSATION_ID_CHILD";

/// Two registered projects can hold conversation directories of the same
/// id (an abandoned publication has no SQL owner). Collection and reopening
/// must finish, so the work runs in a child process bounded by a deadline.
#[test]
fn collection_finishes_when_projects_share_a_conversation_directory_name() {
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "conversation_commit_contracts::shared_conversation_id_collection_child",
            "--nocapture",
        ])
        .env(SHARED_ID_CHILD_ENV, "1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("child test");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() > deadline {
            child.kill().unwrap();
            panic!("collection did not finish: {:?}", child.wait_with_output());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "{output:?}"
    );
}

#[test]
fn shared_conversation_id_collection_child() {
    if std::env::var_os(SHARED_ID_CHILD_ENV).is_none() {
        return;
    }
    let (live_project, other_project) = ("/projects/shared-a", "/projects/shared-b");
    let (temp, repo) = setup(live_project);
    let id = "conversation-shared";
    start(&repo, id, live_project, "Run a tool");
    let commit = record_tool(&repo, id, live_project, "live", &big_output("a"));
    let live_root = conversation_dir(&temp, live_project, id);
    let live_blob = live_root.join(output_artifact(&commit, "live"));
    // Abandoned blobs under the same id: one beside the live conversation,
    // one in another registered project's directory of that id.
    let abandoned_here = live_root.join("tool-output/segment-gone.0.txt");
    fs::write(&abandoned_here, big_output("c")).unwrap();
    let other_root = conversation_dir(&temp, other_project, id);
    fs::create_dir_all(other_root.join("tool-output")).unwrap();
    let abandoned_there = other_root.join("tool-output/segment-gone.1.txt");
    fs::write(&abandoned_there, big_output("d")).unwrap();

    assert_eq!(repo.store().collect_tool_output_garbage().unwrap(), 2);
    assert!(!abandoned_here.exists() && !abandoned_there.exists());
    fs::write(&abandoned_there, big_output("d")).unwrap();
    drop(repo);
    let _repo = ConversationRepository::new(
        ConversationStore::open(temp.path().join("spark-home")).expect("reopen"),
    );
    assert!(!abandoned_there.exists());
    assert_eq!(fs::read_to_string(&live_blob).unwrap(), big_output("a"));
}

fn big_output(fill: &str) -> String {
    fill.repeat(TOOL_OUTPUT_INLINE_LIMIT_BYTES * 2)
}

/// Every blob a live row of `conversation` references exists.
fn assert_live_references_exist(
    store: &ConversationStore,
    conversation: &str,
    root: &std::path::Path,
) {
    let stored = store
        .database()
        .read(|read| read.conversation(conversation))
        .unwrap()
        .expect("live conversation");
    for segment in &stored.segments {
        if let Some(artifact) = segment
            .pointer("/tool_call/output_artifact")
            .and_then(Value::as_str)
        {
            assert!(root.join(artifact).is_file(), "missing {artifact}");
        }
    }
    for (id, blob_path, ..) in &stored.attachments {
        assert!(store.home_dir().join(blob_path).is_file(), "missing {id}");
    }
}

/// Seed checkpointed row A referencing blob A, replace it with B under
/// NORMAL, and no checkpoint barrier that is busy, incomplete, without a WAL
/// or failing unlinks A; once one succeeds A goes, and after reopening the
/// surviving row still references existing bytes.
#[test]
fn replaced_output_bytes_survive_every_failed_checkpoint_barrier() {
    use spark_storage::conversation::CheckpointFault;
    let project_path = "/projects/barrier-blobs";
    let (temp, repo) = setup(project_path);
    let conversation = "conversation-barrier";
    let root = conversation_dir(&temp, project_path, conversation);
    start(&repo, conversation, project_path, "Run a tool");
    let a = record_tool(&repo, conversation, project_path, "tool", &big_output("a"));
    let blob_a = root.join(output_artifact(&a, "tool"));
    repo.store()
        .database()
        .after_checkpoint(|_| Ok(()))
        .unwrap();
    let b = record_tool(&repo, conversation, project_path, "tool", &big_output("b"));
    let blob_b = root.join(output_artifact(&b, "tool"));
    assert_ne!(blob_a, blob_b);

    for fault in [
        CheckpointFault::Busy,
        CheckpointFault::Incomplete,
        CheckpointFault::NoWal,
        CheckpointFault::Error,
    ] {
        repo.store().database().fail_next_checkpoints([fault]);
        let error = repo.store().collect_tool_output_garbage().unwrap_err();
        assert!(
            matches!(error, StorageError::CleanupDeferred { .. }),
            "{fault:?}: {error}"
        );
        assert_eq!(fs::read_to_string(&blob_a).unwrap(), big_output("a"));
    }
    assert_eq!(repo.store().collect_tool_output_garbage().unwrap(), 1);
    assert!(!blob_a.exists());

    drop(repo);
    let repo = ConversationRepository::new(
        ConversationStore::open(temp.path().join("spark-home")).expect("reopen"),
    );
    assert_live_references_exist(repo.store(), conversation, &root);
    assert_eq!(
        repo.read_segment_tool_output(conversation, Some(project_path), &tool_segment_id("tool"))
            .unwrap(),
        Some(big_output("b"))
    );
}

/// Collection running beside events that keep replacing an output never
/// removes bytes a live row references.
#[test]
fn references_survive_collection_running_beside_commits() {
    let project_path = "/projects/concurrent-collection";
    let (temp, repo) = setup(project_path);
    let conversation = "conversation-concurrent";
    let root = conversation_dir(&temp, project_path, conversation);
    start(&repo, conversation, project_path, "Run a tool");
    let done = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            while !done.load(std::sync::atomic::Ordering::SeqCst) {
                repo.store().collect_tool_output_garbage().unwrap();
            }
        });
        for index in 0..40 {
            record_tool(
                &repo,
                conversation,
                project_path,
                "tool",
                &big_output(&format!("{index:02}")),
            );
            assert_live_references_exist(repo.store(), conversation, &root);
        }
        done.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    assert_eq!(
        repo.read_segment_tool_output(conversation, Some(project_path), &tool_segment_id("tool"))
            .unwrap(),
        Some(big_output("39"))
    );
    drop(repo);
    let store = ConversationStore::open(temp.path().join("spark-home")).unwrap();
    assert_live_references_exist(&store, conversation, &root);
    let remaining = fs::read_dir(root.join("tool-output")).unwrap().count();
    assert_eq!(remaining, 1, "old versions are collected");
}

/// A conversation with an attached upload and an old draft, in a closed store.
fn conversation_with_attachments(temp: &tempfile::TempDir, conversation: &str) -> Vec<String> {
    let project_path = "/projects/corrupt-row";
    let home = temp.path().join("spark-home");
    ProjectRegistry::new(&home)
        .ensure_project_paths(project_path)
        .unwrap();
    let repo = ConversationRepository::new(ConversationStore::open(&home).unwrap());
    let sent = repo
        .create_attachment(conversation, project_path, "sent.txt", b"sent")
        .unwrap();
    let draft = repo
        .create_attachment(conversation, project_path, "draft.txt", b"draft")
        .unwrap();
    let (_, accepted) = repo
        .accept_message(
            conversation,
            project_path,
            MessageInput {
                attachments: Some(json!([{"id": sent.id}])),
                ..message("turn-sent", "turn-assistant", "Here")
            },
        )
        .unwrap();
    assert!(matches!(accepted, Accepted::Started { .. }), "{accepted:?}");
    vec![sent.id, draft.id]
}

/// A row that no longer parses must stop the prune: treating it as absent
/// would leave its bytes looking like an orphan to delete.
#[test]
fn a_corrupt_attachment_row_fails_the_prune_and_deletes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let conversation = "conversation-corrupt-row";
    let project_path = "/projects/corrupt-row";
    let ids = conversation_with_attachments(&temp, conversation);
    let home = temp.path().join("spark-home");
    let attachments = conversation_dir(&temp, project_path, conversation).join("attachments");
    let raw = rusqlite::Connection::open(
        home.join(spark_storage::conversation::CONVERSATION_DATABASE_FILE_NAME),
    )
    .unwrap();
    assert_eq!(
        raw.execute(
            "UPDATE attachments SET payload = '{\"corrupt\":true}' WHERE attachment_id = ?1",
            [&ids[0]],
        )
        .unwrap(),
        1
    );
    drop(raw);

    let repo = ConversationRepository::new(ConversationStore::open(&home).unwrap());
    let error = repo
        .prune_attachments(
            conversation,
            project_path,
            time::OffsetDateTime::now_utc() + time::Duration::hours(1),
        )
        .expect_err("a corrupt row fails the prune");
    assert!(error.to_string().contains(&ids[0]), "{error}");
    for id in &ids {
        assert!(
            fs::read_dir(attachments.join(id)).unwrap().next().is_some(),
            "{id} lost its bytes"
        );
    }
}

/// The row recheck behind the cleanup barrier: an old directory with no
/// attachment row stays while anything still references bytes under it.
#[test]
fn a_directory_with_a_live_reference_survives_the_prune() {
    let project_path = "/projects/live-reference";
    let conversation = "conversation-live-reference";
    let (temp, repo) = setup(project_path);
    start(&repo, conversation, project_path, "Hello");
    let attachments = conversation_dir(&temp, project_path, conversation).join("attachments");
    let referenced = attachments.join("att-referenced");
    let unreferenced = attachments.join("att-unreferenced");
    for dir in [&referenced, &unreferenced] {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("content"), "bytes").unwrap();
    }
    let blob = referenced
        .join("content")
        .strip_prefix(repo.store().home_dir())
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/");
    repo.store()
        .database()
        .write(|tx| tx.put_blob_reference(conversation, "attachment", "att-other", &blob))
        .unwrap();

    let removed = repo
        .prune_attachments(
            conversation,
            project_path,
            time::OffsetDateTime::now_utc() + time::Duration::hours(1),
        )
        .unwrap();
    assert!(referenced.join("content").exists(), "{removed:?}");
    assert!(!unreferenced.exists(), "{removed:?}");
}

/// A missing or unknown settings version fails loading, naming the conversation.
#[test]
fn a_conversation_with_an_unsupported_settings_schema_version_does_not_load() {
    let project_path = "/projects/settings-version";
    for (version, shown) in [("1 + 1", "2"), ("NULL", "0")] {
        let (temp, repo) = setup(project_path);
        let conversation = "conversation-settings-version";
        start(&repo, conversation, project_path, "Hello");
        drop(repo);
        let home = temp.path().join("spark-home");
        let raw = rusqlite::Connection::open(
            home.join(spark_storage::conversation::CONVERSATION_DATABASE_FILE_NAME),
        )
        .unwrap();
        let updated = if version == "NULL" {
            raw.execute(
                "UPDATE conversations SET metadata = json_remove(metadata, '$.settings_schema_version')",
                [],
            )
        } else {
            raw.execute(
                "UPDATE conversations SET metadata = json_set(metadata, '$.settings_schema_version', 2)",
                [],
            )
        };
        assert_eq!(updated.unwrap(), 1);
        drop(raw);
        let repo = ConversationRepository::new(ConversationStore::open(&home).unwrap());
        let names_version = |error: StorageError| {
            let message = error.to_string();
            assert!(
                message.contains(conversation)
                    && message.contains(&format!("settings_schema_version {shown}")),
                "{message}"
            );
        };
        names_version(
            repo.append_system_turn(conversation, project_path, "notice", "Hello")
                .expect_err("an operation on an unsupported version fails"),
        );
        names_version(
            repo.read_snapshot(conversation, None)
                .expect_err("reading an unsupported version fails"),
        );
    }
}

const CRASH_CHILD_ENV: &str = "SPARK_CONVERSATION_CRASH_CHILD_HOME";

/// A process killed (aborted) in the middle of a SQL transaction, after
/// publishing its blob. This is a process-crash check, not a power-loss
/// check: the OS still writes everything the process handed it.
#[test]
fn a_killed_process_keeps_committed_state_and_abandons_the_rest() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("spark-home");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "conversation_commit_contracts::process_kill_child",
            "--nocapture",
        ])
        .env(CRASH_CHILD_ENV, &home)
        .output()
        .expect("child test");
    assert!(!output.status.success(), "the child aborts: {output:?}");
    let project_path = "/projects/killed";
    let conversation = "conversation-killed";
    let root = conversation_dir(&temp, project_path, conversation);
    let abandoned_blob = root.join("tool-output/segment-abandoned.0.txt");
    assert!(
        abandoned_blob.is_file(),
        "the child reached its crash point"
    );

    let repo = ConversationRepository::new(ConversationStore::open(&home).unwrap());
    // Startup collects the unreferenced blob behind the barrier.
    assert!(!abandoned_blob.exists());
    let stored = repo
        .store()
        .database()
        .read(|read| read.conversation(conversation))
        .unwrap()
        .unwrap();
    let turns: Vec<_> = stored.turns.iter().map(|(id, ..)| id.as_str()).collect();
    assert_eq!(turns, vec!["turn-user", "turn-assistant"]);
    // The event committed before the crash is kept.
    assert_eq!(stored.turns[1].2["status"], "streaming");
    assert!(stored.segments.is_empty());
    // The aborted transaction's revision was never allocated.
    let watermark = repo
        .store()
        .database()
        .read(|read| Ok(read.watermark()))
        .unwrap();
    assert_eq!(watermark, 2);
    // The next operation takes the next revision.
    let next = record(
        &repo,
        conversation,
        project_path,
        json!({"kind": "tool_call_started", "tool_call": {"id": "call-after"}}),
    )
    .expect("next commit");
    assert_eq!(next.revision, 3);
}

#[test]
fn process_kill_child() {
    let Some(home) = std::env::var_os(CRASH_CHILD_ENV) else {
        return;
    };
    let project_path = "/projects/killed";
    let conversation = "conversation-killed";
    let repo = ConversationRepository::new(ConversationStore::open(&home).unwrap());
    start(&repo, conversation, project_path, "Hello");
    record(
        &repo,
        conversation,
        project_path,
        json!({"kind": "token_usage_updated", "token_usage": {"total_tokens": 1}}),
    )
    .expect("event commit");
    // A publication: durable blob first, then a transaction that never commits.
    let root = ProjectRegistry::new(&home)
        .project_paths(project_path)
        .unwrap()
        .conversations_dir
        .join(conversation);
    let blob = root.join("tool-output/segment-abandoned.0.txt");
    fs::create_dir_all(blob.parent().unwrap()).unwrap();
    fs::write(&blob, big_output("z")).unwrap();
    let store = repo.store();
    let _ = store.database().write(|tx| -> spark_storage::Result<()> {
        tx.put_turn(&spark_storage::conversation::TurnRow {
            conversation_id: conversation.to_string(),
            id: "turn-abandoned".to_string(),
            role: "assistant".to_string(),
            kind: "message".to_string(),
            position: 2,
            settled: false,
            status: "streaming".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            payload: json!({
                "id": "turn-abandoned",
                "role": "assistant",
                "kind": "message",
                "content": "never committed",
                "status": "streaming",
                "timestamp": "2026-01-01T00:00:00Z",
            }),
        })?;
        tx.put_blob_reference(
            conversation,
            "tool_output",
            "segment-abandoned",
            &blob
                .strip_prefix(store.home_dir())
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/"),
        )?;
        std::process::abort()
    });
}
