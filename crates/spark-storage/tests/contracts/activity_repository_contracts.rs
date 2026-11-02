use serde_json::json;
use spark_storage::conversation::{TranscriptSegment, TranscriptTurn};
use spark_storage::{ActivityRepository, TranscriptRecord};
use std::io::Write;

#[test]
fn oversized_jsonl_records_remain_valid_tail_authority() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repository = ActivityRepository::new(temp.path());
    let completion = repository
        .append_event(
            json!({"type": "content_completed", "text": "x".repeat(128 * 1024)}),
            "now",
        )
        .expect("large event");
    repository
        .append_transcript(&TranscriptRecord::SegmentUpsert {
            revision: 1,
            committed_at: "now".to_string(),
            source_event_sequence: completion.sequence,
            segment: serde_json::from_value(json!({
                "id": "large-answer", "turn_id": "turn", "kind": "assistant_message",
                "content": "y".repeat(128 * 1024)
            }))
            .expect("large segment"),
        })
        .expect("transcript after large event");

    let reopened = ActivityRepository::new(temp.path());
    let next = reopened
        .append_event(json!({"type": "content_completed"}), "later")
        .expect("event after large transcript");
    reopened
        .append_transcript(&TranscriptRecord::SegmentUpsert {
            revision: 2,
            committed_at: "later".to_string(),
            source_event_sequence: next.sequence,
            segment: serde_json::from_value(json!({
                "id": "next", "turn_id": "turn", "kind": "assistant_message"
            }))
            .expect("next segment"),
        })
        .expect("transcript after large transcript tail");
}

#[test]
fn event_and_transcript_share_one_externalized_tool_output() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repository = ActivityRepository::new(temp.path());
    let full_output = "tool output".repeat(2_000);
    let mut tool_call = json!({"id": "call-1", "output": full_output.clone()});
    repository
        .externalize_tool_output("call-1", &mut tool_call)
        .expect("externalize event output");
    let event = repository
        .append_event(
            json!({"type": "tool_call_completed", "tool_call": tool_call}),
            "now",
        )
        .expect("tool event");
    repository
        .append_transcript(&TranscriptRecord::SegmentUpsert {
            revision: 1,
            committed_at: "now".to_string(),
            source_event_sequence: event.sequence,
            segment: serde_json::from_value(json!({
                "id": "segment-tool", "turn_id": "turn", "kind": "tool_call",
                "tool_call": event.event["tool_call"]
            }))
            .expect("tool segment"),
        })
        .expect("tool transcript");

    let transcript = repository.read_transcript_records().expect("transcript");
    let TranscriptRecord::SegmentUpsert { segment, .. } = &transcript[0] else {
        panic!("segment record");
    };
    assert_eq!(segment.tool_call.as_ref(), Some(&event.event["tool_call"]));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("tool-output/call-1.txt")).expect("artifact"),
        full_output
    );
    assert!(!temp.path().join("tool-output/segment-tool.txt").exists());
}

#[test]
fn activity_repository_keeps_deltas_out_of_the_logical_transcript() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repository = ActivityRepository::new(temp.path());
    for index in 0..50_000 {
        repository
            .append_event(json!({"type": "content_delta", "index": index}), "now")
            .expect("append delta");
    }
    let completion = repository
        .append_event(json!({"type": "content_completed", "text": "done"}), "now")
        .expect("append completion");
    repository
        .append_transcript(&TranscriptRecord::SegmentUpsert {
            revision: 1,
            committed_at: "now".to_string(),
            source_event_sequence: completion.sequence,
            segment: serde_json::from_value::<TranscriptSegment>(json!({
                "id": "answer", "turn_id": "turn", "kind": "assistant_message", "content": "done"
            }))
            .expect("segment"),
        })
        .expect("append semantic record");

    assert_eq!(repository.read_events().expect("events").len(), 50_001);
    assert_eq!(
        repository.read_transcript_records().expect("records").len(),
        1
    );
    assert_eq!(
        repository
            .hydrate_transcript()
            .expect("transcript")
            .segments
            .len(),
        1
    );
    assert!(repository
        .uncommitted_event_suffix()
        .expect("suffix")
        .is_empty());
}

#[test]
fn hydration_is_last_write_wins_by_stable_id() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repository = ActivityRepository::new(temp.path());
    for (revision, content) in [(1, "working"), (2, "complete")] {
        let event = repository
            .append_event(json!({"type": "turn_completed"}), "now")
            .unwrap();
        repository
            .append_transcript(&TranscriptRecord::TurnUpsert {
                revision,
                committed_at: "now".to_string(),
                source_event_sequence: event.sequence,
                turn: TranscriptTurn {
                    id: "turn".to_string(),
                    content: content.to_string(),
                    ..Default::default()
                },
            })
            .unwrap();
    }
    let transcript = repository.hydrate_transcript().unwrap();
    assert_eq!(transcript.turns.len(), 1);
    assert_eq!(transcript.turns[0].content, "complete");
}

#[test]
fn append_discards_partial_crash_tails_without_corrupting_the_next_record() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repository = ActivityRepository::new(temp.path());
    let first = repository
        .append_event(json!({"type": "first"}), "now")
        .expect("first event");
    let mut events = std::fs::OpenOptions::new()
        .append(true)
        .open(repository.events_path())
        .expect("events");
    write!(events, "{{\"sequence\":2").expect("partial event");
    drop(events);

    let second = repository
        .append_event(json!({"type": "second"}), "now")
        .expect("second event");
    assert_eq!(second.sequence, first.sequence + 1);
    assert_eq!(repository.read_events().expect("strict events").len(), 2);

    repository
        .append_transcript(&TranscriptRecord::TurnUpsert {
            revision: 1,
            committed_at: "now".to_string(),
            source_event_sequence: first.sequence,
            turn: TranscriptTurn {
                id: "first".to_string(),
                ..Default::default()
            },
        })
        .expect("first transcript");
    let mut transcript = std::fs::OpenOptions::new()
        .append(true)
        .open(repository.transcript_path())
        .expect("transcript");
    write!(transcript, "{{\"type\":\"turn_upsert\"").expect("partial transcript");
    drop(transcript);

    repository
        .append_transcript(&TranscriptRecord::TurnUpsert {
            revision: 2,
            committed_at: "now".to_string(),
            source_event_sequence: second.sequence,
            turn: TranscriptTurn {
                id: "second".to_string(),
                ..Default::default()
            },
        })
        .expect("second transcript");
    assert_eq!(
        repository
            .read_transcript_records()
            .expect("strict transcript")
            .len(),
        2
    );
}

#[test]
fn chat_and_execution_repositories_persist_identical_transcript_records() {
    let temp = tempfile::tempdir().expect("tempdir");
    let chat = ActivityRepository::new(temp.path().join("chat"));
    let execution = ActivityRepository::new(temp.path().join("execution"));
    let record = TranscriptRecord::SegmentUpsert {
        revision: 1,
        committed_at: "2026-08-09T00:00:00Z".to_string(),
        source_event_sequence: 1,
        segment: serde_json::from_value(json!({
            "id": "answer", "turn_id": "turn", "kind": "assistant_message",
            "status": "complete", "content": "same logical unit"
        }))
        .expect("segment"),
    };
    for repository in [&chat, &execution] {
        repository
            .append_event(json!({"type": "content_completed"}), "2026-08-09T00:00:00Z")
            .expect("event");
        repository.append_transcript(&record).expect("transcript");
    }
    assert_eq!(
        std::fs::read(chat.transcript_path()).expect("chat transcript"),
        std::fs::read(execution.transcript_path()).expect("execution transcript")
    );
}

#[test]
fn missing_completion_is_recovered_exactly_once() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repository = ActivityRepository::new(temp.path());
    repository
        .append_event(
            json!({"type": "content_completed", "content": "recovered"}),
            "now",
        )
        .expect("completion event");

    for _restart in 0..2 {
        for event in repository
            .uncommitted_event_suffix()
            .expect("uncommitted suffix")
        {
            repository
                .append_transcript(&TranscriptRecord::SegmentUpsert {
                    revision: repository.read_transcript_records().expect("records").len() as u64
                        + 1,
                    committed_at: event.committed_at,
                    source_event_sequence: event.sequence,
                    segment: serde_json::from_value(json!({
                        "id": "answer", "turn_id": "turn", "kind": "assistant_message",
                        "status": "complete", "content": event.event["content"]
                    }))
                    .expect("segment"),
                })
                .expect("recover completion");
        }
    }
    assert_eq!(
        repository.read_transcript_records().expect("records").len(),
        1
    );
}

#[test]
fn transcript_cursor_reads_only_the_new_suffix_in_a_large_execution() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repository = ActivityRepository::new(temp.path());
    for revision in 1..=10_000 {
        let event = repository
            .append_event(json!({"type": "content_completed"}), "now")
            .expect("event");
        repository
            .append_transcript(&TranscriptRecord::SegmentUpsert {
                revision,
                committed_at: "now".to_string(),
                source_event_sequence: event.sequence,
                segment: serde_json::from_value(json!({
                    "id": format!("segment-{revision}"), "turn_id": "turn",
                    "kind": "assistant_message", "status": "complete"
                }))
                .expect("segment"),
            })
            .expect("transcript");
    }
    let (initial, cursor) = repository
        .read_transcript_records_from(0)
        .expect("initial publication");
    assert_eq!(initial.len(), 10_000);

    let event = repository
        .append_event(json!({"type": "content_completed"}), "now")
        .expect("event");
    repository
        .append_transcript(&TranscriptRecord::SegmentUpsert {
            revision: 10_001,
            committed_at: "now".to_string(),
            source_event_sequence: event.sequence,
            segment: serde_json::from_value(json!({
                "id": "latest", "turn_id": "turn", "kind": "assistant_message",
                "status": "complete"
            }))
            .expect("segment"),
        })
        .expect("transcript");

    let (suffix, next_cursor) = repository
        .read_transcript_records_from(cursor)
        .expect("incremental publication");
    assert_eq!(suffix.len(), 1);
    assert_eq!(suffix[0].revision(), 10_001);
    assert!(next_cursor > cursor);
}

#[test]
fn architecture_has_one_activity_writer_and_no_run_projection_sources() {
    let repository_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repository root");
    for relative in [
        "crates/attractor-runtime/src/transcript.rs",
        "crates/attractor-runtime/src/segments.rs",
        "crates/attractor-runtime/src/journal_cache.rs",
        "crates/spark-storage/src/conversation/migrate.rs",
    ] {
        assert!(
            !repository_root.join(relative).exists(),
            "{relative} returned"
        );
    }
    let storage = std::fs::read_to_string(
        repository_root.join("crates/spark-storage/src/conversation/store.rs"),
    )
    .expect("conversation store");
    assert!(storage.contains("ActivityRepository"));
    assert!(!storage.contains("transcript.json\""));
    assert!(!storage.contains("journal.jsonl"));
}

fn turn_record(revision: u64, source_event_sequence: u64) -> TranscriptRecord {
    TranscriptRecord::TurnUpsert {
        revision,
        committed_at: "now".to_string(),
        source_event_sequence,
        turn: TranscriptTurn {
            id: format!("turn-{revision}"),
            ..Default::default()
        },
    }
}

fn append_raw(path: &std::path::Path, bytes: &[u8]) {
    std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .expect("open activity file")
        .write_all(bytes)
        .expect("append raw bytes");
}

#[test]
fn power_loss_nul_tails_are_ignored_by_reads_and_repaired_by_append() {
    let temp = tempfile::tempdir().expect("tempdir");
    let writer = ActivityRepository::new(temp.path());
    for _ in 0..70 {
        writer
            .append_event(json!({"type": "turn_completed"}), "now")
            .expect("event");
    }
    for revision in 1..=7 {
        writer
            .append_transcript(&turn_record(revision, revision))
            .expect("transcript");
    }
    let valid_transcript_len = std::fs::metadata(writer.transcript_path()).unwrap().len();
    append_raw(&writer.events_path(), &[0; 1_273]);
    append_raw(&writer.transcript_path(), &[0; 669]);
    let damaged_events = std::fs::read(writer.events_path()).unwrap();
    let damaged_transcript = std::fs::read(writer.transcript_path()).unwrap();

    // A restarted process reads before any append.
    let repository = ActivityRepository::new(temp.path());
    assert_eq!(repository.read_events().expect("events").len(), 70);
    assert_eq!(
        repository.read_transcript_records().expect("records").len(),
        7
    );
    assert_eq!(
        repository
            .hydrate_transcript()
            .expect("hydrate")
            .turns
            .len(),
        7
    );
    let (records, cursor) = repository
        .read_transcript_records_from(0)
        .expect("from start");
    assert_eq!(records.len(), 7);
    assert_eq!(cursor, valid_transcript_len);
    let (records, same) = repository
        .read_transcript_records_from(cursor)
        .expect("tail only");
    assert!(records.is_empty());
    assert_eq!(same, cursor);
    assert_eq!(
        repository.uncommitted_event_suffix().expect("suffix").len(),
        63
    );
    assert_eq!(
        std::fs::read(repository.events_path()).unwrap(),
        damaged_events
    );
    assert_eq!(
        std::fs::read(repository.transcript_path()).unwrap(),
        damaged_transcript
    );

    let event = repository
        .append_event(json!({"type": "turn_completed"}), "now")
        .expect("event");
    assert_eq!(event.sequence, 71);
    repository
        .append_transcript(&turn_record(8, 71))
        .expect("transcript");
    assert!(
        std::fs::metadata(repository.transcript_path())
            .unwrap()
            .len()
            < damaged_transcript.len() as u64
    );
    assert_eq!(repository.read_events().expect("events").len(), 71);
    assert_eq!(
        repository.read_transcript_records().expect("records").len(),
        8
    );

    let (records, next) = repository
        .read_transcript_records_from(cursor)
        .expect("replacement");
    assert_eq!(records, vec![turn_record(8, 71)]);
    let (records, _) = repository
        .read_transcript_records_from(next)
        .expect("no duplicates");
    assert!(records.is_empty());
}

#[test]
fn valid_unterminated_records_survive_reads_and_append() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repository = ActivityRepository::new(temp.path());
    repository
        .append_event(json!({"type": "turn_completed"}), "now")
        .expect("event");
    repository
        .append_transcript(&turn_record(1, 1))
        .expect("transcript");
    append_raw(
        &repository.events_path(),
        &serde_json::to_vec(&json!({"sequence": 2, "committed_at": "now", "event": {}})).unwrap(),
    );
    append_raw(
        &repository.transcript_path(),
        &serde_json::to_vec(&turn_record(2, 2)).unwrap(),
    );

    let repository = ActivityRepository::new(temp.path());
    assert_eq!(repository.read_events().expect("events").len(), 2);
    let (records, cursor) = repository.read_transcript_records_from(0).expect("records");
    assert_eq!(records.len(), 2);
    assert_eq!(
        cursor,
        std::fs::metadata(repository.transcript_path())
            .unwrap()
            .len()
    );

    let event = repository
        .append_event(json!({"type": "turn_completed"}), "now")
        .expect("event");
    assert_eq!(event.sequence, 3);
    repository
        .append_transcript(&turn_record(3, 3))
        .expect("transcript");
    assert_eq!(repository.read_events().expect("events").len(), 3);
    assert_eq!(
        repository.read_transcript_records().expect("records").len(),
        3
    );
    let (records, next) = repository
        .read_transcript_records_from(cursor)
        .expect("suffix");
    assert_eq!(records, vec![turn_record(3, 3)]);
    assert!(repository
        .read_transcript_records_from(next)
        .unwrap()
        .0
        .is_empty());
}

#[test]
fn terminated_or_earlier_corruption_remains_a_read_error() {
    for corruption in [&b"{\"type\":\"turn_upsert\"\n"[..], &[0; 16][..]] {
        let temp = tempfile::tempdir().expect("tempdir");
        let repository = ActivityRepository::new(temp.path());
        repository
            .append_event(json!({"type": "turn_completed"}), "now")
            .expect("event");
        repository
            .append_transcript(&turn_record(1, 1))
            .expect("transcript");
        append_raw(&repository.events_path(), corruption);
        append_raw(&repository.transcript_path(), corruption);
        let mut later = serde_json::to_vec(&turn_record(2, 1)).unwrap();
        later.push(b'\n');
        append_raw(&repository.transcript_path(), &later);
        append_raw(
            &repository.events_path(),
            b"{\"sequence\":2,\"committed_at\":\"now\",\"event\":{}}\n",
        );

        assert!(repository.read_events().is_err());
        assert!(repository.read_transcript_records().is_err());
        assert!(repository.hydrate_transcript().is_err());
        assert!(repository.read_transcript_records_from(0).is_err());
    }
}

#[test]
fn recovery_suffix_rejects_terminated_corruption() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repository = ActivityRepository::new(temp.path());
    repository
        .append_event(json!({"type": "turn_completed"}), "now")
        .expect("event");
    append_raw(&repository.events_path(), b"BROKEN\n");
    append_raw(
        &repository.events_path(),
        b"{\"sequence\":3,\"committed_at\":\"now\",\"event\":{}}\n",
    );
    assert!(repository.read_events().is_err());
    assert!(repository.uncommitted_event_suffix().is_err());
}

/// A reader racing an append sees the record either whole or not at all:
/// a half-written record is not consumed, and the reused cursor delivers it
/// once when complete. A stale cursor past the end yields nothing.
#[test]
fn transcript_cursor_never_consumes_an_in_flight_append() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repository = ActivityRepository::new(temp.path());
    repository
        .append_event(json!({"type": "turn_completed"}), "now")
        .expect("event");
    repository
        .append_transcript(&turn_record(1, 1))
        .expect("transcript");
    let (_, cursor) = repository.read_transcript_records_from(0).expect("read");

    let mut record = serde_json::to_vec(&turn_record(2, 1)).unwrap();
    record.push(b'\n');
    let (first, rest) = record.split_at(record.len() / 2);
    append_raw(&repository.transcript_path(), first);
    let (records, mid) = repository
        .read_transcript_records_from(cursor)
        .expect("during append");
    assert!(records.is_empty());
    assert_eq!(mid, cursor);

    append_raw(&repository.transcript_path(), rest);
    let (records, end) = repository
        .read_transcript_records_from(mid)
        .expect("after append");
    assert_eq!(records, vec![turn_record(2, 1)]);
    let len = std::fs::metadata(repository.transcript_path())
        .unwrap()
        .len();
    assert_eq!(end, len);
    assert!(repository
        .read_transcript_records_from(end)
        .unwrap()
        .0
        .is_empty());
    let (records, past) = repository
        .read_transcript_records_from(len + 1_000)
        .expect("stale cursor");
    assert!(records.is_empty());
    assert_eq!(past, len);
}
