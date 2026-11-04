use std::fs;

use serde_json::{json, Value};
use spark_storage::conversation::{
    record_from_snapshot, snapshot_from_record, ArtifactCollection, ConversationMetadataPatch,
    ConversationMutation, EntityKind, EntityRow, TranscriptSegment, TranscriptTurn,
    TransientStreamBody, TransientStreamEvent, ORDER_UNASSIGNED, TOOL_OUTPUT_INLINE_LIMIT_BYTES,
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
fn changed<'a>(
    commit: &'a spark_storage::conversation::ConversationCommit,
    kind: EntityKind,
    id: &str,
) -> &'a EntityRow {
    commit
        .changes
        .rows
        .iter()
        .find(|row| row.kind == kind && row.id == id)
        .unwrap_or_else(|| panic!("changed {kind:?} {id}"))
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
fn commit_conversation_creates_conversations_and_returns_only_changed_rows() {
    let project_path = "/projects/commit-basics";
    let _temp = tempfile::tempdir().expect("tempdir");
    let home = _temp.path().join("spark-home");
    let repo = ConversationRepository::new(ConversationStore::open(&home).expect("store"));

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

    // One home-global revision per transaction, stamped on every changed row.
    assert_eq!(commit.revision, 1);
    assert!(!commit.rebased);
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
    assert_eq!(second.revision, 2);
    // Only the segment and its conversation's summary changed.
    assert_eq!(second.changes.rows.len(), 2);
    assert_eq!(
        changed(&second, EntityKind::Segment, "segment-a").order,
        Some(1)
    );

    let persisted = repo
        .read_snapshot("conversation-a", Some(project_path))
        .expect("read")
        .expect("snapshot");
    assert_eq!(persisted["revision"], 2);
    assert_eq!(persisted["turns"].as_array().expect("turns").len(), 2);
    assert_eq!(persisted["segments"][0]["id"], "segment-a");
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

    // The artifact commit's result is the artifact row and the summary it
    // changed, never a snapshot of the conversation.
    assert_eq!(artifact.changes.rows.len(), 2);
    let row = changed(&artifact, EntityKind::Artifact, "request-a");
    assert_eq!(row.collection.as_deref(), Some("flow_run_requests"));
    assert_eq!(row.revision, artifact.revision);
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
    assert_eq!(commit.revision, 1);
    // The metadata travels as the conversation row; the workflow event as its own row.
    let metadata = &changed(&commit, EntityKind::Conversation, "conversation-e")
        .payload
        .as_ref()
        .unwrap()["metadata"];
    assert_eq!(metadata["chat_mode"], "plan");
    assert_eq!(metadata["model"], "gpt-5.3-codex");
    assert!(commit
        .changes
        .rows
        .iter()
        .any(|row| row.kind == EntityKind::Artifact
            && row.collection.as_deref() == Some("event_log")));

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
    assert_eq!(
        changed(&cleared, EntityKind::Conversation, "conversation-e")
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
    let changed_segment = changed(&commit, EntityKind::Segment, "segment-tool-call-1");
    assert_eq!(
        changed_segment.payload.as_ref().unwrap()["tool_call"]["output_truncated"],
        true
    );

    // An immutable blob holds the full output and serves the read path.
    let artifact = stored["output_artifact"].as_str().expect("artifact");
    assert!(
        artifact.starts_with("tool-output/segment-tool-call-1."),
        "{artifact}"
    );
    let sidecar = conversation_dir(&temp, project_path, "conversation-h").join(artifact);
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
    // overwrite the blob with truncated content or drop its reference.
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
    assert_eq!(
        repo.read_segment_tool_output("conversation-h", Some(project_path), "segment-tool-call-1")
            .expect("read output after echo"),
        Some(big_output.clone())
    );

    // A new output version gets a new blob; the old bytes stay until cleanup.
    let mut replaced = segment("segment-tool-call-1", "turn-assistant", ORDER_UNASSIGNED);
    replaced.kind = "tool_call".to_string();
    let second_output = "z".repeat(TOOL_OUTPUT_INLINE_LIMIT_BYTES * 2);
    replaced.tool_call = Some(json!({"id": "call-1", "output": second_output}));
    let replaced = repo
        .commit_conversation(
            "conversation-h",
            project_path,
            0,
            vec![ConversationMutation::SegmentUpserted { segment: replaced }],
        )
        .expect("replace output");
    let new_artifact = changed(&replaced, EntityKind::Segment, "segment-tool-call-1")
        .payload
        .as_ref()
        .and_then(|segment| segment["tool_call"]["output_artifact"].as_str())
        .expect("new artifact")
        .to_string();
    assert_ne!(new_artifact, artifact);
    assert_eq!(
        fs::read_to_string(&sidecar).expect("old blob kept"),
        big_output
    );
    assert_eq!(
        repo.read_segment_tool_output("conversation-h", Some(project_path), "segment-tool-call-1")
            .expect("read replaced"),
        Some(second_output)
    );
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

    // Only the committed turn is stored.
    let stored = repo
        .store()
        .database()
        .read(|read| read.conversation("conversation-g"))
        .expect("rows")
        .expect("conversation");
    assert_eq!(stored.turns.len(), 1);
    assert!(stored.segments.is_empty());
    let _ = temp;
}

#[test]
fn concurrent_commits_serialize_revision_allocation_and_keep_all_turns() {
    let (temp, repository) = setup("/projects/concurrent-commit");
    repository
        .commit_conversation(
            "conversation-concurrent",
            "/projects/concurrent-commit",
            0,
            vec![ConversationMutation::TurnUpserted {
                turn: user_turn("initial", "Initial"),
            }],
        )
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
                        .commit_conversation(
                            "conversation-concurrent",
                            "/projects/concurrent-commit",
                            1,
                            vec![ConversationMutation::TurnUpserted {
                                turn: user_turn(&format!("turn-{i}"), "Concurrent turn"),
                            }],
                        )
                        .unwrap()
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
        .read_snapshot(
            "conversation-concurrent",
            Some("/projects/concurrent-commit"),
        )
        .unwrap()
        .unwrap();
    assert_eq!(snapshot["turns"].as_array().unwrap().len(), 9);
    let _ = temp;
}

#[test]
fn competing_mode_updates_conflict_without_changing_history_and_events_still_rebase() {
    let project_path = "/projects/mode-conflict";
    let (temp, repo) = setup(project_path);
    let initial = repo
        .commit_conversation(
            "chat",
            project_path,
            0,
            vec![ConversationMutation::TurnUpserted {
                turn: user_turn("user", "Preserve this transcript"),
            }],
        )
        .unwrap();
    let mode_update = |mode: &str| {
        vec![
            ConversationMutation::TurnUpserted {
                turn: TranscriptTurn {
                    id: format!("mode-{mode}"),
                    kind: Some("mode_change".into()),
                    content: mode.into(),
                    ..user_turn("unused", "")
                },
            },
            ConversationMutation::MetadataUpdated {
                patch: ConversationMetadataPatch {
                    chat_mode: Some(mode.into()),
                    ..Default::default()
                },
            },
        ]
    };
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let writers: Vec<_> = ["plan", "chat"]
        .into_iter()
        .map(|mode| {
            let repo = repo.clone();
            let barrier = barrier.clone();
            let mutations = mode_update(mode);
            let revision = initial.revision;
            std::thread::spawn(move || {
                barrier.wait();
                repo.commit_conversation("chat", project_path, revision, mutations)
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
            .filter(|result| matches!(result, Err(StorageError::SettingsConflict { .. })))
            .count(),
        1
    );
    let winner = results.into_iter().find_map(Result::ok).unwrap();
    let winner_snapshot = snapshot(&repo, "chat");
    assert_eq!(winner_snapshot["turns"].as_array().unwrap().len(), 2);
    let rows = || {
        repo.store()
            .database()
            .read(|read| read.conversation("chat"))
            .unwrap()
    };
    let before = rows();
    assert!(matches!(
        repo.commit_conversation("chat", project_path, initial.revision, mode_update("plan")),
        Err(StorageError::SettingsConflict { .. })
    ));
    assert_eq!(rows(), before);
    let _ = &temp;
    assert_eq!(
        repo.read_snapshot("chat", Some(project_path))
            .unwrap()
            .unwrap(),
        winner_snapshot
    );
    let _ = winner;
    let operational = repo
        .commit_conversation(
            "chat",
            project_path,
            initial.revision,
            vec![ConversationMutation::TurnUpserted {
                turn: assistant_turn("assistant", "complete"),
            }],
        )
        .unwrap();
    assert!(operational.rebased);
    let after = snapshot(&repo, "chat");
    assert_eq!(after["chat_mode"], winner_snapshot["chat_mode"]);
    assert_eq!(
        after["turns"].as_array().unwrap()[..2],
        winner_snapshot["turns"].as_array().unwrap()[..]
    );
}

#[test]
fn read_snapshot_derives_plan_segment_artifact_id_from_plan_record() {
    let project_path = "/projects/plan-link";
    let (temp, repo) = setup(project_path);
    let mut plan_segment = segment("segment-plan-1", "turn-1", 1);
    plan_segment.kind = "plan".to_string();
    plan_segment.status = "complete".to_string();
    repo.commit_conversation(
        "conversation-plan-link",
        project_path,
        0,
        vec![
            ConversationMutation::TurnUpserted {
                turn: assistant_turn("turn-1", "complete"),
            },
            ConversationMutation::SegmentUpserted {
                segment: plan_segment,
            },
            ConversationMutation::ArtifactUpserted {
                collection: ArtifactCollection::ProposedPlans,
                artifact: json!({
                    "id": "proposed-plan-1",
                    "status": "pending_review",
                    "source_segment_id": "segment-plan-1",
                }),
            },
        ],
    )
    .expect("commit");

    let snapshot = repo
        .read_snapshot("conversation-plan-link", Some(project_path))
        .expect("read")
        .expect("snapshot");
    let segment = snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .find(|segment| segment["id"] == "segment-plan-1")
        .expect("plan segment");
    assert_eq!(segment["artifact_id"], "proposed-plan-1");

    // Derived at read time only; the stored segment is not rewritten.
    let stored = repo
        .store()
        .database()
        .read(|read| read.conversation("conversation-plan-link"))
        .expect("rows")
        .expect("conversation");
    assert!(stored.segments[0].get("artifact_id").is_none());
    let _ = temp;
}

/// A failed transaction leaves both the database and the loaded state at the
/// previous revision; nothing staged becomes visible.
#[test]
fn failed_transactions_roll_back_rows_and_memory() {
    let project_path = "/projects/rollback";
    let (temp, repo) = setup(project_path);
    let first = repo
        .commit_conversation(
            "conversation-rollback",
            project_path,
            0,
            vec![ConversationMutation::TurnUpserted {
                turn: user_turn("turn-user", "Hello"),
            }],
        )
        .expect("seed");
    let rows = || {
        repo.store()
            .database()
            .read(|read| read.conversation("conversation-rollback"))
            .unwrap()
    };
    let before = rows();
    let before_snapshot = snapshot(&repo, "conversation-rollback");
    // A non-object event violates the row constraint inside the transaction.
    let error = repo
        .commit_conversation(
            "conversation-rollback",
            project_path,
            first.revision,
            vec![
                ConversationMutation::TurnUpserted {
                    turn: user_turn("turn-second", "Second"),
                },
                ConversationMutation::WorkflowEventAppended {
                    event: json!("not an object"),
                },
            ],
        )
        .expect_err("constraint violation");
    assert!(
        matches!(error, StorageError::ConversationDatabase(_)),
        "{error}"
    );
    assert_eq!(rows(), before);
    let snapshot = repo
        .read_snapshot("conversation-rollback", Some(project_path))
        .unwrap()
        .unwrap();
    assert_eq!(snapshot, before_snapshot);

    // Retrying the valid part, even twice, commits the turn once.
    let second = vec![ConversationMutation::TurnUpserted {
        turn: user_turn("turn-second", "Second"),
    }];
    let retried = repo
        .commit_conversation(
            "conversation-rollback",
            project_path,
            first.revision,
            second.clone(),
        )
        .unwrap();
    repo.commit_conversation(
        "conversation-rollback",
        project_path,
        retried.revision,
        second,
    )
    .unwrap();
    // The cache that survived the rollback matches what SQL reloads.
    let cached = self::snapshot(&repo, "conversation-rollback");
    drop(repo);
    let repo = ConversationRepository::new(
        ConversationStore::open(temp.path().join("spark-home")).expect("reopen"),
    );
    assert_eq!(self::snapshot(&repo, "conversation-rollback"), cached);
    let turns: Vec<_> = cached["turns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|turn| turn["id"].as_str().unwrap())
        .collect();
    assert_eq!(turns, vec!["turn-user", "turn-second"]);
}

/// Externalized output is read from its blob; inline output has no blob;
/// a preview whose blob is missing is an error, never complete output.
#[test]
fn segment_tool_output_distinguishes_inline_present_and_missing_bytes() {
    let project_path = "/projects/tool-output-reads";
    let (temp, repo) = setup(project_path);
    let tool = |id: &str, tool_call: Value| {
        let mut segment = segment(id, "turn-assistant", ORDER_UNASSIGNED);
        segment.kind = "tool_call".to_string();
        segment.tool_call = Some(tool_call);
        ConversationMutation::SegmentUpserted { segment }
    };
    let root = conversation_dir(&temp, project_path, "conversation-reads");
    fs::create_dir_all(root.join("tool-output")).unwrap();
    fs::write(root.join("tool-output/present.txt"), "full present output").unwrap();
    repo.commit_conversation(
        "conversation-reads",
        project_path,
        0,
        vec![
            ConversationMutation::TurnUpserted {
                turn: assistant_turn("turn-assistant", "complete"),
            },
            tool(
                "segment-inline",
                json!({"id": "a", "output": "inline output"}),
            ),
            tool(
                "segment-present",
                json!({"id": "b", "output": "PREVIEW", "output_truncated": true,
                    "output_size": 19, "output_artifact": "tool-output/present.txt"}),
            ),
            tool(
                "segment-missing",
                json!({"id": "c", "output": "ONLY PREVIEW", "output_truncated": true,
                    "output_size": 100000, "output_artifact": "tool-output/missing.txt"}),
            ),
        ],
    )
    .unwrap();
    let read = |segment: &str| {
        repo.read_segment_tool_output("conversation-reads", Some(project_path), segment)
    };
    assert_eq!(read("segment-inline").unwrap(), None);
    assert_eq!(
        read("segment-present").unwrap().as_deref(),
        Some("full present output")
    );
    assert!(matches!(
        read("segment-missing"),
        Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound
    ));
}

/// Recovery collects tool-output blobs nothing needs, only behind the
/// cleanup barrier: an output version a commit replaced, a tombstoned
/// segment's output, and a publication whose transaction failed. Live bytes
/// and bytes a retained provider trace still needs for replay stay; a
/// deferred barrier keeps everything until the next startup retries.
#[test]
fn recovery_collects_unreferenced_tool_outputs_behind_the_barrier() {
    let project_path = "/projects/tool-output-garbage";
    let (temp, repo) = setup(project_path);
    let conversation = "conversation-garbage";
    let root = conversation_dir(&temp, project_path, conversation);
    let tool = |id: &str, output: &str| {
        let mut segment = segment(id, "turn-assistant", ORDER_UNASSIGNED);
        segment.kind = "tool_call".to_string();
        segment.tool_call = Some(json!({"id": id, "output": output}));
        ConversationMutation::SegmentUpserted { segment }
    };
    let blobs = || -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(root.join("tool-output"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    };
    let big = |fill: &str| fill.repeat(TOOL_OUTPUT_INLINE_LIMIT_BYTES * 2);

    let first = repo
        .commit_conversation(
            conversation,
            project_path,
            0,
            vec![
                ConversationMutation::TurnUpserted {
                    turn: assistant_turn("turn-assistant", "streaming"),
                },
                tool("segment-replaced", &big("a")),
                tool("segment-removed", &big("b")),
            ],
        )
        .unwrap();
    let replaced = blobs();
    assert_eq!(replaced.len(), 2);
    // A new version replaces the first; the other segment is tombstoned.
    let second = repo
        .commit_conversation(
            conversation,
            project_path,
            first.revision,
            vec![
                tool("segment-replaced", &big("c")),
                ConversationMutation::SegmentTombstoned {
                    turn_id: "turn-assistant".to_string(),
                    segment_id: "segment-removed".to_string(),
                },
            ],
        )
        .unwrap();
    // A publication whose transaction fails leaves its blob unreferenced.
    repo.commit_conversation(
        conversation,
        project_path,
        second.revision,
        vec![
            tool("segment-abandoned", &big("d")),
            ConversationMutation::WorkflowEventAppended {
                event: json!("not an object"),
            },
        ],
    )
    .expect_err("constraint violation");
    // A provider event recorded but not yet projected needs its blob for replay.
    let mut event = serde_json::from_value(json!({
        "kind": "tool_call_completed",
        "tool_call": {"id": "call-replay", "output": big("e")},
    }))
    .unwrap();
    let traced = repo
        .append_provider_event(conversation, project_path, "turn-assistant", &mut event)
        .unwrap();
    let replay_blob = traced.event["tool_call"]["output_artifact"]
        .as_str()
        .unwrap()
        .trim_start_matches("tool-output/")
        .to_string();
    let live_blob = changed(&second, EntityKind::Segment, "segment-replaced")
        .payload
        .as_ref()
        .and_then(|segment| segment["tool_call"]["output_artifact"].as_str())
        .unwrap()
        .trim_start_matches("tool-output/")
        .to_string();
    let before = blobs();
    assert_eq!(before.len(), 5, "{before:?}");

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
    let mut kept = vec![live_blob, replay_blob];
    kept.sort();
    assert_eq!(blobs(), kept);
    assert_eq!(
        repo.read_segment_tool_output(conversation, Some(project_path), "segment-replaced")
            .unwrap(),
        Some(big("c"))
    );
    assert_eq!(
        repo.unapplied_provider_events(conversation, project_path, "turn-assistant")
            .unwrap()
            .len(),
        1
    );
    assert_eq!(repo.store().collect_tool_output_garbage().unwrap(), 0);
}

/// An open store checkpoints written transactions in the background.
#[test]
fn open_store_checkpoints_writes_periodically() {
    let project_path = "/projects/periodic-checkpoint";
    let (_temp, repo) = setup(project_path);
    repo.commit_conversation(
        "conversation-periodic",
        project_path,
        0,
        vec![ConversationMutation::TurnUpserted {
            turn: user_turn("turn-user", "Hello"),
        }],
    )
    .unwrap();
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
    let big = |fill: &str| fill.repeat(TOOL_OUTPUT_INLINE_LIMIT_BYTES * 2);
    let mut tool = segment("segment-tool", "turn-assistant", ORDER_UNASSIGNED);
    tool.kind = "tool_call".to_string();
    tool.tool_call = Some(json!({"id": "call-live", "output": big("a")}));
    let commit = repo
        .commit_conversation(
            id,
            live_project,
            0,
            vec![
                ConversationMutation::TurnUpserted {
                    turn: assistant_turn("turn-assistant", "streaming"),
                },
                ConversationMutation::SegmentUpserted { segment: tool },
            ],
        )
        .unwrap();
    let mut event = serde_json::from_value(json!({
        "kind": "tool_call_completed",
        "tool_call": {"id": "call-replay", "output": big("b")},
    }))
    .unwrap();
    let traced = repo
        .append_provider_event(id, live_project, "turn-assistant", &mut event)
        .unwrap();
    let live_root = conversation_dir(&temp, live_project, id);
    let live_blob = live_root.join(
        changed(&commit, EntityKind::Segment, "segment-tool")
            .payload
            .as_ref()
            .unwrap()["tool_call"]["output_artifact"]
            .as_str()
            .unwrap(),
    );
    let replay_blob = live_root.join(
        traced.event["tool_call"]["output_artifact"]
            .as_str()
            .unwrap(),
    );
    // Abandoned blobs under the same id: one beside the live conversation,
    // one in another registered project's directory of that id.
    let abandoned_here = live_root.join("tool-output/segment-gone.0.txt");
    fs::write(&abandoned_here, big("c")).unwrap();
    let other_root = conversation_dir(&temp, other_project, id);
    fs::create_dir_all(other_root.join("tool-output")).unwrap();
    let abandoned_there = other_root.join("tool-output/segment-gone.1.txt");
    fs::write(&abandoned_there, big("d")).unwrap();

    assert_eq!(repo.store().collect_tool_output_garbage().unwrap(), 2);
    assert!(!abandoned_here.exists() && !abandoned_there.exists());
    fs::write(&abandoned_there, big("d")).unwrap();
    drop(repo);
    let repo = ConversationRepository::new(
        ConversationStore::open(temp.path().join("spark-home")).expect("reopen"),
    );
    assert!(!abandoned_there.exists());
    assert_eq!(fs::read_to_string(&live_blob).unwrap(), big("a"));
    assert_eq!(fs::read_to_string(&replay_blob).unwrap(), big("b"));
    assert_eq!(
        repo.unapplied_provider_events(id, live_project, "turn-assistant")
            .unwrap()
            .len(),
        1
    );
}

/// A tool-call segment carrying `output` (externalized when large).
fn tool_segment(id: &str, output: &str) -> ConversationMutation {
    let mut segment = segment(id, "turn-assistant", ORDER_UNASSIGNED);
    segment.kind = "tool_call".to_string();
    segment.tool_call = Some(json!({"id": id, "output": output}));
    ConversationMutation::SegmentUpserted { segment }
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
    let output = |commit: &spark_storage::conversation::ConversationCommit| {
        root.join(
            changed(commit, EntityKind::Segment, "segment-tool")
                .payload
                .as_ref()
                .unwrap()["tool_call"]["output_artifact"]
                .as_str()
                .unwrap(),
        )
    };
    let a = repo
        .commit_conversation(
            conversation,
            project_path,
            0,
            vec![
                ConversationMutation::TurnUpserted {
                    turn: assistant_turn("turn-assistant", "streaming"),
                },
                tool_segment("segment-tool", &big_output("a")),
            ],
        )
        .unwrap();
    let blob_a = output(&a);
    repo.store()
        .database()
        .after_checkpoint(|_| Ok(()))
        .unwrap();
    let b = repo
        .commit_conversation(
            conversation,
            project_path,
            a.revision,
            vec![tool_segment("segment-tool", &big_output("b"))],
        )
        .unwrap();
    let blob_b = output(&b);
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
        repo.read_segment_tool_output(conversation, Some(project_path), "segment-tool")
            .unwrap(),
        Some(big_output("b"))
    );
}

/// Trace pruning: a projection whose transaction fails moves no cursor; a
/// failed barrier prunes nothing; restart replays nothing settled; pruning
/// keeps an older active turn's unapplied records; per-turn cursors and
/// runtime sessions of different providers stay independent; and the
/// allocator stays above every pruned or truncated sequence across restarts.
#[test]
fn trace_pruning_waits_for_the_barrier_and_keeps_an_older_active_turn() {
    use spark_storage::conversation::{CheckpointFault, RuntimeSession};
    let project_path = "/projects/trace-barrier";
    let (temp, repo) = setup(project_path);
    let home = temp.path().join("spark-home");
    let conversation = "conversation-trace";
    let trace = conversation_dir(&temp, project_path, conversation)
        .join(spark_storage::conversation::PROVIDER_TRACE_FILE_NAME);
    let turn = |id: &str, status: &str| ConversationMutation::TurnUpserted {
        turn: assistant_turn(id, status),
    };
    let append = |repo: &ConversationRepository, turn_id: &str| {
        let mut event = serde_json::from_value(
            json!({"kind": "content_delta", "channel": "assistant", "message": "x"}),
        )
        .unwrap();
        repo.append_provider_event(conversation, project_path, turn_id, &mut event)
            .unwrap()
            .sequence
    };
    let project = |repo: &ConversationRepository, turn_id: &str, mutations| {
        repo.commit_conversation_projecting(conversation, project_path, 0, mutations, Some(turn_id))
    };
    let unapplied = |repo: &ConversationRepository, turn_id: &str| -> Vec<u64> {
        repo.unapplied_provider_events(conversation, project_path, turn_id)
            .unwrap()
            .iter()
            .map(|record| record.sequence)
            .collect()
    };
    let applied = |repo: &ConversationRepository| -> Vec<(String, u64)> {
        repo.store()
            .database()
            .read(|read| read.conversation(conversation))
            .unwrap()
            .unwrap()
            .turns
            .into_iter()
            .map(|(id, _, applied, _)| (id, applied))
            .collect()
    };
    let traced = || -> Vec<u64> {
        fs::read_to_string(&trace)
            .unwrap()
            .lines()
            .map(|line| {
                serde_json::from_str::<Value>(line).unwrap()["sequence"]
                    .as_u64()
                    .unwrap()
            })
            .collect()
    };

    repo.commit_conversation(
        conversation,
        project_path,
        0,
        vec![turn("turn-old", "streaming"), turn("turn-new", "streaming")],
    )
    .unwrap();
    assert_eq!(append(&repo, "turn-old"), 1);
    project(&repo, "turn-old", vec![turn("turn-old", "streaming")]).unwrap();
    assert_eq!(append(&repo, "turn-old"), 2);
    assert_eq!(append(&repo, "turn-new"), 3);
    assert_eq!(append(&repo, "turn-new"), 4);
    let session = |provider: &str| RuntimeSession {
        schema_version: spark_storage::conversation::RUNTIME_SESSION_SCHEMA_VERSION,
        provider: provider.to_string(),
        thread_id: Some(format!("{provider}-thread")),
        established_at: "2026-01-01T00:00:00Z".to_string(),
        last_turn_id: None,
        resume_failed: false,
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    for provider in ["codex_app_server", "claude_code"] {
        repo.write_runtime_session(conversation, project_path, &session(provider))
            .unwrap();
    }

    // A projection whose transaction fails leaves both cursors where they were.
    project(
        &repo,
        "turn-new",
        vec![
            turn("turn-new", "complete"),
            ConversationMutation::WorkflowEventAppended {
                event: json!("not an object"),
            },
        ],
    )
    .expect_err("constraint violation");
    assert_eq!(unapplied(&repo, "turn-new"), vec![3, 4]);
    let expected = vec![("turn-old".to_string(), 1), ("turn-new".to_string(), 0)];
    assert_eq!(applied(&repo), expected);

    // Settling turn-new commits, but no failed barrier truncates the trace.
    for fault in [
        CheckpointFault::Busy,
        CheckpointFault::Incomplete,
        CheckpointFault::NoWal,
        CheckpointFault::Error,
    ] {
        repo.store().database().fail_next_checkpoints([fault]);
        project(&repo, "turn-new", vec![turn("turn-new", "complete")]).unwrap();
        assert_eq!(traced(), vec![1, 2, 3, 4], "{fault:?}");
    }
    let expected = vec![("turn-old".to_string(), 1), ("turn-new".to_string(), 4)];
    assert_eq!(applied(&repo), expected);

    // Restart: the settled turn's leftover records are ignored, not replayed.
    let revision = snapshot(&repo, conversation)["revision"].clone();
    drop(repo);
    let repo = ConversationRepository::new(ConversationStore::open(&home).unwrap());
    assert_eq!(snapshot(&repo, conversation)["revision"], revision);
    assert_eq!(unapplied(&repo, "turn-new"), Vec::<u64>::new());
    assert_eq!(unapplied(&repo, "turn-old"), vec![2]);
    assert_eq!(
        repo.read_runtime_session(conversation, Some(project_path))
            .unwrap(),
        Some(session("claude_code"))
    );

    // A barrier that succeeds prunes everything except turn-old's record
    // above its cursor.
    project(&repo, "turn-new", vec![turn("turn-new", "complete")]).unwrap();
    assert_eq!(traced(), vec![2]);
    assert_eq!(applied(&repo), expected);
    drop(repo);
    let repo = ConversationRepository::new(ConversationStore::open(&home).unwrap());
    assert_eq!(unapplied(&repo, "turn-old"), vec![2]);
    // The allocator stays above the pruned records 3 and 4.
    assert_eq!(append(&repo, "turn-old"), 5);
    project(&repo, "turn-old", vec![turn("turn-old", "complete")]).unwrap();
    assert_eq!(fs::read(&trace).unwrap(), Vec::<u8>::new(), "settled trace");
    drop(repo);
    // And above the truncated record 5 after another restart.
    let repo = ConversationRepository::new(ConversationStore::open(&home).unwrap());
    assert_eq!(append(&repo, "turn-old"), 6);
}

/// Collection running beside commits that keep replacing an output never
/// removes bytes a live row references.
#[test]
fn references_survive_collection_running_beside_commits() {
    let project_path = "/projects/concurrent-collection";
    let (temp, repo) = setup(project_path);
    let conversation = "conversation-concurrent";
    let root = conversation_dir(&temp, project_path, conversation);
    repo.commit_conversation(
        conversation,
        project_path,
        0,
        vec![ConversationMutation::TurnUpserted {
            turn: assistant_turn("turn-assistant", "streaming"),
        }],
    )
    .unwrap();
    let done = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            while !done.load(std::sync::atomic::Ordering::SeqCst) {
                repo.store().collect_tool_output_garbage().unwrap();
            }
        });
        for index in 0..40 {
            repo.commit_conversation(
                conversation,
                project_path,
                0,
                vec![tool_segment(
                    "segment-tool",
                    &big_output(&format!("{index:02}")),
                )],
            )
            .unwrap();
            assert_live_references_exist(repo.store(), conversation, &root);
        }
        done.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    assert_eq!(
        repo.read_segment_tool_output(conversation, Some(project_path), "segment-tool")
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
    let mut turn = user_turn("turn-sent", "Here");
    turn.extra
        .insert("attachments".into(), json!([{"id": sent.id}]));
    repo.commit_conversation(
        conversation,
        project_path,
        0,
        vec![ConversationMutation::TurnUpserted { turn }],
    )
    .unwrap();
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
    repo.commit_conversation(
        conversation,
        project_path,
        0,
        vec![ConversationMutation::TurnUpserted {
            turn: user_turn("turn-user", "Hello"),
        }],
    )
    .unwrap();
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
        repo.commit_conversation(
            conversation,
            project_path,
            0,
            vec![ConversationMutation::TurnUpserted {
                turn: user_turn("turn-user", "Hello"),
            }],
        )
        .unwrap();
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
        let error = repo
            .read_snapshot(conversation, None)
            .expect_err("an unsupported version fails");
        let message = error.to_string();
        assert!(
            message.contains(conversation)
                && message.contains(&format!("settings_schema_version {shown}")),
            "{message}"
        );
    }
}

/// The applied cursor may only cover durable trace records: a failed sync
/// publishes nothing, and the retry after it does.
#[cfg(unix)]
#[test]
fn a_failed_trace_sync_publishes_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let project_path = "/projects/trace-sync";
    let conversation = "conversation-trace-sync";
    let (temp, repo) = setup(project_path);
    let notices = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = notices.clone();
    repo.store()
        .set_commit_listener(std::sync::Arc::new(move |_| {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }));
    let turn = || ConversationMutation::TurnUpserted {
        turn: assistant_turn("turn-assistant", "streaming"),
    };
    repo.commit_conversation(conversation, project_path, 0, vec![turn()])
        .unwrap();
    let mut event = serde_json::from_value(
        json!({"kind": "content_delta", "channel": "assistant", "message": "x"}),
    )
    .unwrap();
    repo.append_provider_event(conversation, project_path, "turn-assistant", &mut event)
        .unwrap();
    let trace = conversation_dir(&temp, project_path, conversation)
        .join(spark_storage::conversation::PROVIDER_TRACE_FILE_NAME);
    let watermark = |repo: &ConversationRepository| {
        repo.store()
            .database()
            .read(|read| Ok(read.watermark()))
            .unwrap()
    };
    let (before_watermark, before_notices) = (
        watermark(&repo),
        notices.load(std::sync::atomic::Ordering::SeqCst),
    );

    fs::set_permissions(&trace, fs::Permissions::from_mode(0o000)).unwrap();
    let failed = repo.commit_conversation_projecting(
        conversation,
        project_path,
        0,
        vec![ConversationMutation::TurnUpserted {
            turn: assistant_turn("turn-assistant", "complete"),
        }],
        Some("turn-assistant"),
    );
    fs::set_permissions(&trace, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(failed.is_err(), "{failed:?}");
    assert_eq!(watermark(&repo), before_watermark);
    assert_eq!(
        notices.load(std::sync::atomic::Ordering::SeqCst),
        before_notices
    );
    assert_eq!(
        snapshot(&repo, conversation)["turns"][0]["status"],
        "streaming"
    );
    assert_eq!(
        repo.unapplied_provider_events(conversation, project_path, "turn-assistant")
            .unwrap()
            .len(),
        1
    );

    repo.commit_conversation_projecting(
        conversation,
        project_path,
        0,
        vec![ConversationMutation::TurnUpserted {
            turn: assistant_turn("turn-assistant", "complete"),
        }],
        Some("turn-assistant"),
    )
    .unwrap();
    assert_eq!(
        snapshot(&repo, conversation)["turns"][0]["status"],
        "complete"
    );
    assert!(repo
        .unapplied_provider_events(conversation, project_path, "turn-assistant")
        .unwrap()
        .is_empty());
}

const CRASH_CHILD_ENV: &str = "SPARK_CONVERSATION_CRASH_CHILD_HOME";

/// A process killed (aborted) in the middle of a SQL transaction, after
/// publishing its blob and after appending a trace record it never
/// projected. This is a process-crash check, not a power-loss check: the OS
/// still writes everything the process handed it.
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
    assert!(stored.segments.is_empty());
    // The aborted transaction's revision was never allocated.
    let watermark = repo
        .store()
        .database()
        .read(|read| Ok(read.watermark()))
        .unwrap();
    assert_eq!(watermark, 2);
    assert_eq!(stored.turns[1].2, 1, "applied cursor");
    let unapplied: Vec<_> = repo
        .unapplied_provider_events(conversation, project_path, "turn-assistant")
        .unwrap()
        .iter()
        .map(|record| record.sequence)
        .collect();
    assert_eq!(unapplied, vec![2]);
    let mut event = serde_json::from_value(
        json!({"kind": "content_delta", "channel": "assistant", "message": "after"}),
    )
    .unwrap();
    let next = repo
        .append_provider_event(conversation, project_path, "turn-assistant", &mut event)
        .unwrap();
    assert_eq!(next.sequence, 3);
    let commit = repo
        .commit_conversation(
            conversation,
            project_path,
            watermark,
            vec![ConversationMutation::TurnUpserted {
                turn: assistant_turn("turn-assistant", "complete"),
            }],
        )
        .unwrap();
    assert_eq!(commit.revision, 3);
}

#[test]
fn process_kill_child() {
    let Some(home) = std::env::var_os(CRASH_CHILD_ENV) else {
        return;
    };
    let project_path = "/projects/killed";
    let conversation = "conversation-killed";
    let repo = ConversationRepository::new(ConversationStore::open(&home).unwrap());
    repo.commit_conversation(
        conversation,
        project_path,
        0,
        vec![
            ConversationMutation::TurnUpserted {
                turn: user_turn("turn-user", "Hello"),
            },
            ConversationMutation::TurnUpserted {
                turn: assistant_turn("turn-assistant", "streaming"),
            },
        ],
    )
    .unwrap();
    let append = || {
        let mut event = serde_json::from_value(
            json!({"kind": "content_delta", "channel": "assistant", "message": "x"}),
        )
        .unwrap();
        repo.append_provider_event(conversation, project_path, "turn-assistant", &mut event)
            .unwrap()
    };
    append();
    repo.commit_conversation_projecting(
        conversation,
        project_path,
        1,
        vec![ConversationMutation::TurnUpserted {
            turn: assistant_turn("turn-assistant", "streaming"),
        }],
        Some("turn-assistant"),
    )
    .unwrap();
    // Appended, never projected.
    append();
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
        let mut abandoned = assistant_turn("turn-abandoned", "streaming");
        abandoned.content = "never committed".to_string();
        tx.put_turn(&spark_storage::conversation::TurnRow {
            conversation_id: conversation.to_string(),
            id: "turn-abandoned".to_string(),
            position: 2,
            status: "streaming".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            payload: serde_json::to_value(&abandoned).unwrap(),
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
