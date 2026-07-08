use std::fs;

use serde_json::{json, Value};
use spark_storage::conversation::{
    record_from_snapshot, snapshot_from_record, ArtifactCollection, ConversationMetadataPatch,
    ConversationMutation, JournalEntryKind, TranscriptSegment, TranscriptTurn, TransientStreamBody,
    TransientStreamEvent, ORDER_UNASSIGNED, TOOL_OUTPUT_INLINE_LIMIT_BYTES,
};
use spark_storage::{ConversationRepository, ProjectRegistry, StorageError};

fn setup(project_path: &str) -> (tempfile::TempDir, ConversationRepository) {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    ProjectRegistry::new(&home)
        .ensure_project_paths(project_path)
        .expect("project paths");
    (temp, ConversationRepository::new(&home))
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

fn user_turn(id: &str, content: &str) -> TranscriptTurn {
    TranscriptTurn {
        id: id.to_string(),
        role: "user".to_string(),
        kind: Some("message".to_string()),
        content: content.to_string(),
        status: "complete".to_string(),
        timestamp: "2026-01-01T00:00:00Z".to_string(),
        ..TranscriptTurn::default()
    }
}

fn assistant_turn(id: &str, status: &str) -> TranscriptTurn {
    TranscriptTurn {
        id: id.to_string(),
        role: "assistant".to_string(),
        kind: Some("message".to_string()),
        status: status.to_string(),
        timestamp: "2026-01-01T00:00:00Z".to_string(),
        ..TranscriptTurn::default()
    }
}

fn segment(id: &str, turn_id: &str, order: i64) -> TranscriptSegment {
    TranscriptSegment {
        id: id.to_string(),
        turn_id: turn_id.to_string(),
        order,
        kind: "assistant_message".to_string(),
        role: "assistant".to_string(),
        status: "streaming".to_string(),
        timestamp: "2026-01-01T00:00:01Z".to_string(),
        updated_at: "2026-01-01T00:00:01Z".to_string(),
        content: "partial".to_string(),
        completed_at: None,
        error: None,
        error_code: None,
        details: None,
        phase: None,
        artifact_id: None,
        tool_call: None,
        request_user_input: None,
        source: Some(json!({})),
        boundary: None,
        extra: serde_json::Map::new(),
    }
}

#[test]
fn commit_conversation_creates_conversations_and_allocates_strictly_increasing_revisions() {
    let project_path = "/projects/commit-basics";
    let (_temp, repo) = setup(project_path);

    let commit = repo
        .commit_conversation(
            "conversation-a",
            project_path,
            0,
            vec![
                ConversationMutation::TurnUpserted {
                    turn: user_turn("turn-user", "Hello there commit boundary"),
                },
                ConversationMutation::TurnUpserted {
                    turn: assistant_turn("turn-assistant", "pending"),
                },
            ],
        )
        .expect("initial commit");

    assert_eq!(commit.revision, 2);
    assert!(!commit.rebased);
    assert_eq!(
        commit
            .journal_entries
            .iter()
            .map(|entry| entry.revision)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(commit.record.meta.title, "Hello there commit boundary");
    assert_eq!(commit.record.meta.conversation_handle.split('-').count(), 2);
    assert!(!commit.record.meta.created_at.is_empty());
    assert_eq!(commit.journal_payloads[0]["type"], "turn_upsert");
    assert_eq!(commit.journal_payloads[0]["turn"]["id"], "turn-user");
    assert_eq!(commit.journal_payloads[0]["revision"], 1);

    let second = repo
        .commit_conversation(
            "conversation-a",
            project_path,
            commit.revision,
            vec![ConversationMutation::SegmentUpserted {
                segment: segment("segment-a", "turn-assistant", ORDER_UNASSIGNED),
            }],
        )
        .expect("second commit");
    assert_eq!(second.revision, 3);
    assert_eq!(second.journal_entries[0].revision, 3);
    assert!(matches!(
        &second.journal_entries[0].kind,
        JournalEntryKind::SegmentUpserted { segment } if segment.order == 1
    ));

    let persisted = repo
        .read_snapshot("conversation-a", Some(project_path))
        .expect("read")
        .expect("snapshot");
    assert_eq!(persisted["revision"], 3);
    assert_eq!(persisted["turns"].as_array().expect("turns").len(), 2);
    assert_eq!(persisted["segments"][0]["id"], "segment-a");
    let replay = repo
        .read_conversation_events_after("conversation-a", project_path, 0)
        .expect("replay");
    assert_eq!(
        replay
            .iter()
            .map(|event| event["revision"].as_i64().expect("revision"))
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );

    // A fresh conversation persists only split record files — no legacy
    // state.json/events.jsonl and no project-level sidecar files.
    let root = conversation_dir(&_temp, project_path, "conversation-a");
    for expected in [
        "conversation.json",
        "transcript.json",
        "event-log.json",
        "journal.jsonl",
        "artifacts/flow-run-requests.json",
        "artifacts/flow-launches.json",
        "artifacts/run-recoveries.json",
        "artifacts/proposed-plans.json",
    ] {
        assert!(root.join(expected).exists(), "missing {expected}");
    }
    assert!(!root.join("state.json").exists());
    assert!(!root.join("events.jsonl").exists());
    let project = ProjectRegistry::new(_temp.path().join("spark-home"))
        .ensure_project_paths(project_path)
        .expect("project paths");
    assert!(!project
        .flow_run_requests_dir
        .join("conversation-a.json")
        .exists());
    let meta: Value = serde_json::from_str(
        &fs::read_to_string(root.join("conversation.json")).expect("conversation.json"),
    )
    .expect("meta json");
    assert_eq!(meta["revision"], 3);
    assert!(meta.get("turns").is_none());
}

#[test]
fn commit_conversation_rebases_stale_base_without_clobbering_concurrent_writes() {
    let project_path = "/projects/commit-rebase";
    let (_temp, repo) = setup(project_path);

    let started = repo
        .commit_conversation(
            "conversation-b",
            project_path,
            0,
            vec![
                ConversationMutation::TurnUpserted {
                    turn: user_turn("turn-user", "Launch a flow while streaming"),
                },
                ConversationMutation::TurnUpserted {
                    turn: assistant_turn("turn-assistant", "streaming"),
                },
            ],
        )
        .expect("start commit");

    // A streaming segment commits on top of the started state.
    let streamed = repo
        .commit_conversation(
            "conversation-b",
            project_path,
            started.revision,
            vec![ConversationMutation::SegmentUpserted {
                segment: segment("segment-stream", "turn-assistant", ORDER_UNASSIGNED),
            }],
        )
        .expect("stream commit");

    // A concurrent artifact review still holds the pre-stream revision. Today
    // this interleaving clobbers the streamed segment; the commit boundary
    // must rebase by mutation identity instead.
    let artifact = repo
        .commit_conversation(
            "conversation-b",
            project_path,
            started.revision,
            vec![ConversationMutation::ArtifactUpserted {
                collection: ArtifactCollection::FlowRunRequests,
                artifact: json!({"id": "request-a", "status": "pending_review"}),
            }],
        )
        .expect("artifact commit");

    assert!(artifact.rebased);
    assert_eq!(artifact.revision, streamed.revision + 1);
    let persisted = repo
        .read_snapshot("conversation-b", Some(project_path))
        .expect("read")
        .expect("snapshot");
    assert_eq!(persisted["segments"][0]["id"], "segment-stream");
    assert_eq!(persisted["flow_run_requests"][0]["id"], "request-a");
    assert_eq!(persisted["revision"], artifact.revision);

    // The artifact commit journals a snapshot entry stamped at its revision.
    assert_eq!(
        artifact.journal_payloads[0]["type"],
        "conversation_snapshot"
    );
    assert_eq!(
        artifact.journal_payloads[0]["state"]["revision"],
        artifact.revision
    );
    assert_eq!(
        artifact.journal_payloads[0]["state"]["segments"][0]["id"],
        "segment-stream"
    );
}

#[test]
fn commit_conversation_keeps_segment_orders_stable_for_late_updates() {
    let project_path = "/projects/commit-orders";
    let (_temp, repo) = setup(project_path);

    let commit = repo
        .commit_conversation(
            "conversation-c",
            project_path,
            0,
            vec![
                ConversationMutation::TurnUpserted {
                    turn: assistant_turn("turn-assistant", "streaming"),
                },
                ConversationMutation::SegmentUpserted {
                    segment: segment("segment-early", "turn-assistant", ORDER_UNASSIGNED),
                },
                ConversationMutation::SegmentUpserted {
                    segment: segment("segment-late", "turn-assistant", ORDER_UNASSIGNED),
                },
            ],
        )
        .expect("seed commit");

    // A late update targeting the older segment keeps its original order even
    // though newer segments now exist.
    let mut updated = segment("segment-early", "turn-assistant", ORDER_UNASSIGNED);
    updated.status = "complete".to_string();
    updated.content = "final".to_string();
    let late = repo
        .commit_conversation(
            "conversation-c",
            project_path,
            commit.revision,
            vec![ConversationMutation::SegmentUpserted { segment: updated }],
        )
        .expect("late update");

    let persisted = repo
        .read_snapshot("conversation-c", Some(project_path))
        .expect("read")
        .expect("snapshot");
    let segments = persisted["segments"].as_array().expect("segments");
    let early = segments
        .iter()
        .find(|segment| segment["id"] == "segment-early")
        .expect("early segment");
    assert_eq!(early["order"], 1);
    assert_eq!(early["status"], "complete");
    let late_segment = segments
        .iter()
        .find(|segment| segment["id"] == "segment-late")
        .expect("late segment");
    assert_eq!(late_segment["order"], 2);
    assert_eq!(late.revision, commit.revision + 1);
}

#[test]
fn commit_conversation_rejects_empty_batches_unknown_bases_and_orphan_segments() {
    let project_path = "/projects/commit-rejects";
    let (_temp, repo) = setup(project_path);

    let error = repo
        .commit_conversation("conversation-d", project_path, 0, Vec::new())
        .expect_err("empty batch");
    assert!(matches!(
        error,
        StorageError::ConversationCommitRejected { .. }
    ));

    let error = repo
        .commit_conversation(
            "conversation-d",
            project_path,
            7,
            vec![ConversationMutation::TurnUpserted {
                turn: user_turn("turn-user", "hello"),
            }],
        )
        .expect_err("unknown conversation with non-zero base");
    assert!(matches!(
        error,
        StorageError::ConversationCommitRejected { .. }
    ));

    let error = repo
        .commit_conversation(
            "conversation-d",
            project_path,
            0,
            vec![ConversationMutation::SegmentUpserted {
                segment: segment("segment-x", "turn-missing", ORDER_UNASSIGNED),
            }],
        )
        .expect_err("segment targeting unknown turn");
    assert!(matches!(
        error,
        StorageError::ConversationCommitRejected { .. }
    ));
    assert!(repo
        .read_snapshot("conversation-d", Some(project_path))
        .expect("read")
        .is_none());
}

#[test]
fn commit_conversation_applies_metadata_patches_and_workflow_events() {
    let project_path = "/projects/commit-metadata";
    let (_temp, repo) = setup(project_path);

    let commit = repo
        .commit_conversation(
            "conversation-e",
            project_path,
            0,
            vec![
                ConversationMutation::MetadataUpdated {
                    patch: ConversationMetadataPatch {
                        chat_mode: Some("plan".to_string()),
                        provider: Some("codex".to_string()),
                        model: Some(Some("gpt-5.3-codex".to_string())),
                        ..ConversationMetadataPatch::default()
                    },
                },
                ConversationMutation::WorkflowEventAppended {
                    event: json!({"message": "flow launched", "timestamp": "2026-01-01T00:00:02Z"}),
                },
            ],
        )
        .expect("metadata commit");
    assert_eq!(commit.record.meta.chat_mode, "plan");
    assert_eq!(commit.record.meta.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(commit.revision, 1);
    // Live payload carries full state for connected clients...
    assert_eq!(commit.journal_payloads[0]["type"], "conversation_snapshot");
    assert_eq!(commit.journal_payloads[0]["state"]["chat_mode"], "plan");
    // ...but the journal file line is a slim ref that never embeds state.
    let journal = fs::read_to_string(
        conversation_dir(&_temp, project_path, "conversation-e").join("journal.jsonl"),
    )
    .expect("journal");
    let line: Value = serde_json::from_str(journal.lines().next().expect("line")).expect("json");
    assert_eq!(line["type"], "conversation_snapshot_ref");
    assert_eq!(line["revision"], 1);
    assert!(line.get("state").is_none());

    let cleared = repo
        .commit_conversation(
            "conversation-e",
            project_path,
            commit.revision,
            vec![ConversationMutation::MetadataUpdated {
                patch: ConversationMetadataPatch {
                    model: Some(None),
                    ..ConversationMetadataPatch::default()
                },
            }],
        )
        .expect("clear model");
    assert_eq!(cleared.record.meta.model, None);

    let persisted = repo
        .read_snapshot("conversation-e", Some(project_path))
        .expect("read")
        .expect("snapshot");
    assert_eq!(persisted["chat_mode"], "plan");
    assert_eq!(persisted["model"], Value::Null);
    assert_eq!(persisted["event_log"][0]["message"], "flow launched");
}

#[test]
fn commit_conversation_externalizes_oversized_tool_outputs() {
    let project_path = "/projects/commit-tool-output";
    let (temp, repo) = setup(project_path);
    let big_output = format!(
        "line\u{2014}{}",
        "y".repeat(TOOL_OUTPUT_INLINE_LIMIT_BYTES * 2)
    );

    let mut tool_segment = segment("segment-tool-call-1", "turn-assistant", ORDER_UNASSIGNED);
    tool_segment.kind = "tool_call".to_string();
    tool_segment.tool_call = Some(json!({"id": "call-1", "output": big_output}));
    let mut unsafe_segment = segment("../escape", "turn-assistant", ORDER_UNASSIGNED);
    unsafe_segment.kind = "tool_call".to_string();
    unsafe_segment.tool_call = Some(json!({"id": "call-2", "output": big_output}));

    let commit = repo
        .commit_conversation(
            "conversation-h",
            project_path,
            0,
            vec![
                ConversationMutation::TurnUpserted {
                    turn: assistant_turn("turn-assistant", "streaming"),
                },
                ConversationMutation::SegmentUpserted {
                    segment: tool_segment,
                },
                ConversationMutation::SegmentUpserted {
                    segment: unsafe_segment,
                },
            ],
        )
        .expect("commit");

    // Persisted transcript and journal line both carry the bounded preview.
    let persisted = repo
        .read_snapshot("conversation-h", Some(project_path))
        .expect("read")
        .expect("snapshot");
    let stored = persisted["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .find(|segment| segment["id"] == "segment-tool-call-1")
        .expect("tool segment")["tool_call"]
        .clone();
    assert_eq!(stored["output_truncated"], true);
    assert_eq!(
        stored["output_size"].as_u64().expect("size") as usize,
        big_output.len()
    );
    assert!(stored["output"].as_str().expect("preview").len() <= TOOL_OUTPUT_INLINE_LIMIT_BYTES);
    let journal_segment = commit
        .journal_payloads
        .iter()
        .find(|payload| payload["segment"]["id"] == "segment-tool-call-1")
        .expect("journal segment");
    assert_eq!(
        journal_segment["segment"]["tool_call"]["output_truncated"],
        true
    );

    // The sidecar holds the full output and serves the read path.
    let sidecar = conversation_dir(&temp, project_path, "conversation-h")
        .join("tool-output")
        .join("segment-tool-call-1.txt");
    assert_eq!(fs::read_to_string(&sidecar).expect("sidecar"), big_output);
    assert_eq!(
        repo.read_segment_tool_output("conversation-h", Some(project_path), "segment-tool-call-1")
            .expect("read output"),
        Some(big_output.clone())
    );

    // Unsafe segment ids stay inline and never touch the filesystem.
    let unsafe_stored = persisted["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .find(|segment| segment["id"] == "../escape")
        .expect("unsafe segment")["tool_call"]
        .clone();
    assert!(unsafe_stored.get("output_truncated").is_none());
    assert_eq!(unsafe_stored["output"], json!(big_output));
    assert_eq!(
        repo.read_segment_tool_output("conversation-h", Some(project_path), "../escape")
            .expect("unsafe read"),
        None
    );

    // Re-upserting the segment with its preview (a client echo) must not
    // overwrite the sidecar with truncated content.
    let mut echoed = segment("segment-tool-call-1", "turn-assistant", ORDER_UNASSIGNED);
    echoed.kind = "tool_call".to_string();
    echoed.tool_call = Some(json!({
        "id": "call-1",
        "output": stored["output"],
        "output_size": stored["output_size"],
        "output_truncated": true,
        "status": "complete",
    }));
    repo.commit_conversation(
        "conversation-h",
        project_path,
        commit.revision,
        vec![ConversationMutation::SegmentUpserted { segment: echoed }],
    )
    .expect("echo commit");
    assert_eq!(fs::read_to_string(&sidecar).expect("sidecar"), big_output);
}

#[test]
fn commit_snapshot_projection_round_trips_read_snapshot_output() {
    let project_path = "/projects/commit-roundtrip";
    let (_temp, repo) = setup(project_path);

    let mut anchored = segment("segment-anchor", "turn-assistant", ORDER_UNASSIGNED);
    anchored.kind = "flow_run_request".to_string();
    anchored.artifact_id = Some("request-a".to_string());
    anchored
        .extra
        .insert("event_kind".to_string(), json!("session_start"));
    repo.commit_conversation(
        "conversation-f",
        project_path,
        0,
        vec![
            ConversationMutation::TurnUpserted {
                turn: user_turn("turn-user", "Round trip"),
            },
            ConversationMutation::TurnUpserted {
                turn: assistant_turn("turn-assistant", "complete"),
            },
            ConversationMutation::SegmentUpserted { segment: anchored },
            ConversationMutation::ArtifactUpserted {
                collection: ArtifactCollection::FlowRunRequests,
                artifact: json!({"id": "request-a", "status": "pending_review"}),
            },
        ],
    )
    .expect("commit");

    let snapshot = repo
        .read_snapshot("conversation-f", Some(project_path))
        .expect("read")
        .expect("snapshot");
    let record = record_from_snapshot(&snapshot).expect("record");
    assert_eq!(
        record.transcript.segments[0].extra["event_kind"],
        json!("session_start")
    );
    assert_eq!(
        record.transcript.segments[0].artifact_id.as_deref(),
        Some("request-a")
    );
    let reprojected = snapshot_from_record(&record);
    assert_eq!(reprojected, snapshot);
}

#[test]
fn transient_stream_events_are_never_appended_to_the_journal() {
    let project_path = "/projects/commit-transient";
    let (temp, repo) = setup(project_path);
    repo.commit_conversation(
        "conversation-g",
        project_path,
        0,
        vec![ConversationMutation::TurnUpserted {
            turn: assistant_turn("turn-assistant", "streaming"),
        }],
    )
    .expect("seed");

    // Transient events carry a stream sequence instead of a revision, so the
    // journal appender refuses them even if a caller tries.
    let transient = TransientStreamEvent {
        conversation_id: "conversation-g".to_string(),
        turn_id: "turn-assistant".to_string(),
        stream_sequence: 4,
        base_revision: 1,
        body: TransientStreamBody::SegmentDelta {
            segment: segment("segment-live", "turn-assistant", 1),
        },
    };
    let payload = serde_json::to_value(&transient).expect("serialize");
    assert!(payload.get("revision").is_none());
    repo.append_conversation_event("conversation-g", project_path, &payload)
        .expect("append is a no-op");

    let replay = repo
        .read_conversation_events_after("conversation-g", project_path, 0)
        .expect("replay");
    assert_eq!(replay.len(), 1);
    assert_eq!(replay[0]["type"], "turn_upsert");

    let journal_text = fs::read_to_string(
        conversation_dir(&temp, project_path, "conversation-g").join("journal.jsonl"),
    )
    .expect("journal file");
    assert_eq!(journal_text.lines().count(), 1);
}
