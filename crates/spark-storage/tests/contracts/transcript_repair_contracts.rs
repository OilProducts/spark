use std::fs;

use serde_json::{json, Value};
use spark_common::events::TurnStreamEvent;
use spark_storage::conversation::{ConversationMutation, TranscriptSegment, TranscriptTurn};
use spark_storage::{ConversationRepository, ProjectRegistry};

fn provider_event(value: Value) -> TurnStreamEvent {
    serde_json::from_value(value).expect("provider event")
}

fn transcript_segment(value: Value) -> TranscriptSegment {
    serde_json::from_value(value).expect("transcript segment")
}

fn tool_call(id: &str, status: &str) -> Value {
    json!({
        "id": id,
        "kind": "command_execution",
        "status": status,
        "title": "Run command",
        "command": format!("run {id}"),
        "output": "",
    })
}

/// The legacy malformed shape (duplicate order-derived `turn_completed`
/// markers plus forever-`running` tool starts after the final answer) is
/// projected as-is on reopen: no segment is removed or rebuilt, ordinary
/// turn-boundary finalization closes the stale starts, and a second reopen
/// changes nothing.
#[test]
fn reopen_projects_legacy_malformed_terminal_turn_as_is() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    let project_path = "/projects/repair-app";
    let registry = ProjectRegistry::new(&home);
    let project = registry
        .ensure_project_paths(project_path)
        .expect("project paths");
    let repo = ConversationRepository::new(&home);
    let conversation_id = "conversation-repair";

    let turn: TranscriptTurn = serde_json::from_value(json!({
        "id": "turn-1",
        "role": "assistant",
        "kind": "message",
        "content": "Final answer.",
        "status": "complete",
        "timestamp": "2026-08-15T23:12:44Z",
    }))
    .expect("turn");

    // Legacy malformed projection: final answer at order 2, a duplicate pair
    // of order-derived turn_completed markers, and the two stale running tool
    // starts appended after the final answer.
    let malformed_segments = vec![
        transcript_segment(json!({
            "id": "segment-tool-app-1-exec-ok",
            "turn_id": "turn-1",
            "order": 1,
            "kind": "tool_call",
            "role": "system",
            "status": "complete",
            "timestamp": "2026-08-15T23:13:00Z",
            "tool_call": tool_call("exec-ok", "completed"),
        })),
        transcript_segment(json!({
            "id": "segment-assistant-app-1-msg-1",
            "turn_id": "turn-1",
            "order": 2,
            "kind": "assistant_message",
            "role": "assistant",
            "status": "complete",
            "timestamp": "2026-08-15T23:14:25Z",
            "content": "Final answer.",
        })),
        transcript_segment(json!({
            "id": "segment-agent-event-turn-1-turn_completed-3",
            "turn_id": "turn-1",
            "order": 3,
            "kind": "agent_event",
            "role": "system",
            "status": "complete",
            "timestamp": "2026-08-15T23:14:25Z",
            "content": "turn_completed",
        })),
        transcript_segment(json!({
            "id": "segment-tool-app-1-exec-a",
            "turn_id": "turn-1",
            "order": 4,
            "kind": "tool_call",
            "role": "system",
            "status": "running",
            "timestamp": "2026-08-15T23:14:25Z",
            "tool_call": tool_call("exec-a", "running"),
        })),
        transcript_segment(json!({
            "id": "segment-tool-app-1-exec-b",
            "turn_id": "turn-1",
            "order": 5,
            "kind": "tool_call",
            "role": "system",
            "status": "running",
            "timestamp": "2026-08-15T23:14:25Z",
            "tool_call": tool_call("exec-b", "running"),
        })),
        transcript_segment(json!({
            "id": "segment-agent-event-turn-1-turn_completed-6",
            "turn_id": "turn-1",
            "order": 6,
            "kind": "agent_event",
            "role": "system",
            "status": "complete",
            "timestamp": "2026-08-15T23:14:26Z",
            "content": "turn_completed",
        })),
    ];
    let mut mutations = vec![ConversationMutation::TurnUpserted { turn }];
    mutations.extend(
        malformed_segments
            .into_iter()
            .map(|segment| ConversationMutation::SegmentUpserted { segment }),
    );
    repo.commit_conversation(conversation_id, project_path, 0, mutations)
        .expect("seed malformed projection");

    // Raw provider activity: one completing tool, two starts without
    // completion, the final answer, and the turn boundary.
    let events = vec![
        provider_event(json!({
            "kind": "tool_call_started",
            "source": {"app_turn_id": "app-1", "item_id": "exec-ok", "raw_kind": "tool_item_started"},
            "tool_call": tool_call("exec-ok", "running"),
        })),
        provider_event(json!({
            "kind": "tool_call_completed",
            "source": {"app_turn_id": "app-1", "item_id": "exec-ok", "raw_kind": "tool_item_completed"},
            "tool_call": tool_call("exec-ok", "completed"),
        })),
        provider_event(json!({
            "kind": "tool_call_started",
            "source": {"app_turn_id": "app-1", "item_id": "exec-a", "raw_kind": "tool_item_started"},
            "tool_call": tool_call("exec-a", "running"),
        })),
        provider_event(json!({
            "kind": "tool_call_started",
            "source": {"app_turn_id": "app-1", "item_id": "exec-b", "raw_kind": "tool_item_started"},
            "tool_call": tool_call("exec-b", "running"),
        })),
        provider_event(json!({
            "kind": "content_completed",
            "channel": "assistant",
            "phase": "final_answer",
            "message": "Final answer.",
            "source": {"app_turn_id": "app-1", "item_id": "msg-1", "raw_kind": "agent_message"},
        })),
        provider_event(json!({
            "kind": "turn_completed",
            "status": "completed",
            "source": {"app_turn_id": "app-1", "raw_kind": "turn_completed"},
        })),
    ];
    for mut event in events {
        repo.append_provider_event(conversation_id, project_path, "turn-1", &mut event)
            .expect("append provider event");
    }

    let conversation_dir = project.conversations_dir.join(conversation_id);
    let events_before = fs::read(conversation_dir.join("events.jsonl")).expect("events before");

    let snapshot = repo
        .read_snapshot(conversation_id, Some(project_path))
        .expect("read")
        .expect("snapshot");

    let segments: Vec<&Value> = snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .filter(|segment| segment["turn_id"] == "turn-1")
        .collect();

    // History is not rewritten: both legacy markers survive, nothing is
    // tombstoned.
    for legacy in [
        "segment-agent-event-turn-1-turn_completed-3",
        "segment-agent-event-turn-1-turn_completed-6",
    ] {
        assert!(
            segments.iter().any(|segment| segment["id"] == legacy),
            "{legacy} kept: {segments:#?}"
        );
    }
    let transcript_raw =
        fs::read_to_string(conversation_dir.join("transcript.jsonl")).expect("transcript");
    assert!(!transcript_raw.contains("segment_tombstone"));

    // Ordinary turn-boundary finalization closes the unmatched starts.
    for exec in ["exec-a", "exec-b"] {
        let segment = segments
            .iter()
            .find(|segment| segment["id"] == format!("segment-tool-app-1-{exec}"))
            .unwrap_or_else(|| panic!("{exec} segment"));
        assert_eq!(segment["status"], "complete");
        assert_eq!(segment["tool_call"]["status"], "yielded");
    }

    // Raw provider activity is append-only: the pre-reopen bytes are an
    // unchanged prefix of the post-reopen log.
    let events_after = fs::read(conversation_dir.join("events.jsonl")).expect("events after");
    assert!(events_after.starts_with(&events_before));

    // Second reopen: fully converged, no further mutations of any kind.
    let transcript_before_reopen = transcript_raw;
    let revision_before_reopen = snapshot["revision"].as_i64().expect("revision");
    let reopened = repo
        .read_snapshot(conversation_id, Some(project_path))
        .expect("reopen")
        .expect("snapshot");
    assert_eq!(
        reopened["revision"].as_i64().expect("revision"),
        revision_before_reopen
    );
    let transcript_after_reopen =
        fs::read_to_string(conversation_dir.join("transcript.jsonl")).expect("transcript after");
    assert_eq!(transcript_after_reopen, transcript_before_reopen);
}

/// A failed turn's unmatched starts are evidence, not yields: reopen keeps
/// them at stream position and closes them as failed with the stable
/// missing-completion error code.
#[test]
fn reopen_marks_unmatched_starts_failed_for_failed_turns() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    let project_path = "/projects/repair-failed-app";
    ProjectRegistry::new(&home)
        .ensure_project_paths(project_path)
        .expect("project paths");
    let repo = ConversationRepository::new(&home);
    let conversation_id = "conversation-repair-failed";

    let turn: TranscriptTurn = serde_json::from_value(json!({
        "id": "turn-1",
        "role": "assistant",
        "kind": "message",
        "content": "",
        "status": "failed",
        "timestamp": "2026-08-15T23:12:44Z",
    }))
    .expect("turn");
    let stale = transcript_segment(json!({
        "id": "segment-tool-app-1-exec-a",
        "turn_id": "turn-1",
        "order": 1,
        "kind": "tool_call",
        "role": "system",
        "status": "running",
        "timestamp": "2026-08-15T23:14:25Z",
        "tool_call": tool_call("exec-a", "running"),
    }));
    repo.commit_conversation(
        conversation_id,
        project_path,
        0,
        vec![
            ConversationMutation::TurnUpserted { turn },
            ConversationMutation::SegmentUpserted { segment: stale },
        ],
    )
    .expect("seed");

    for value in [
        json!({
            "kind": "tool_call_started",
            "source": {"app_turn_id": "app-1", "item_id": "exec-a", "raw_kind": "tool_item_started"},
            "tool_call": tool_call("exec-a", "running"),
        }),
        json!({
            "kind": "turn_completed",
            "status": "failed",
            "source": {"app_turn_id": "app-1", "raw_kind": "turn_completed"},
        }),
    ] {
        let mut event = provider_event(value);
        repo.append_provider_event(conversation_id, project_path, "turn-1", &mut event)
            .expect("append provider event");
    }

    let snapshot = repo
        .read_snapshot(conversation_id, Some(project_path))
        .expect("read")
        .expect("snapshot");
    let segment = snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .find(|segment| segment["id"] == "segment-tool-app-1-exec-a")
        .expect("tool segment");
    assert_eq!(segment["status"], "failed");
    assert_eq!(segment["error_code"], "tool_completion_missing");
    assert_eq!(segment["tool_call"]["status"], "failed");
    assert_eq!(segment["order"], 1);
}
