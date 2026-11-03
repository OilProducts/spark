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
    let (_temp, repo) = setup(project_path);
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
    let _ = first;
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
    repo.store().database().defer_next_checkpoints(1);
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
