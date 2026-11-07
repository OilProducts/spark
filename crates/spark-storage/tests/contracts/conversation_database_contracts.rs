use serde_json::json;
use spark_storage::conversation::{
    ArtifactRow, AttachmentRow, CatchUp, ChangeBatch, ChangeScope, ConversationDatabase,
    ConversationRow, EntityKind, PageRead, ResetReason, SegmentRow, TurnPage, TurnRow,
    CATCH_UP_MAX_BYTES, CATCH_UP_MAX_ROWS, CONVERSATION_DATABASE_FILE_NAME,
    CONVERSATION_DATABASE_SCHEMA_VERSION,
};
use spark_storage::StorageError;

fn open() -> (tempfile::TempDir, ConversationDatabase) {
    let temp = tempfile::tempdir().expect("tempdir");
    let database = ConversationDatabase::open(temp.path().join("spark-home")).expect("open");
    (temp, database)
}

fn conversation(id: &str, project_path: &str, updated_at: &str) -> ConversationRow {
    ConversationRow {
        id: id.to_string(),
        project_path: project_path.to_string(),
        handle: format!("handle-{id}"),
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: updated_at.to_string(),
        title: "New thread".to_string(),
        preview: String::new(),
        running: false,
        needs_attention: false,
        launched_run_ids: Vec::new(),
        metadata: json!({"chat_mode": "chat"}),
    }
}

fn turn(conversation_id: &str, id: &str, position: i64) -> TurnRow {
    TurnRow {
        conversation_id: conversation_id.to_string(),
        id: id.to_string(),
        role: "user".to_string(),
        kind: "message".to_string(),
        position,
        settled: true,
        status: "complete".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        payload: json!({"id": id, "role": "user"}),
    }
}

fn segment(conversation_id: &str, id: &str, turn_id: &str, order: i64) -> SegmentRow {
    SegmentRow {
        conversation_id: conversation_id.to_string(),
        id: id.to_string(),
        turn_id: turn_id.to_string(),
        kind: "assistant_message".to_string(),
        order,
        status: "complete".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        payload: json!({"id": id}),
    }
}

fn artifact(conversation_id: &str, id: &str, turn_id: Option<&str>) -> ArtifactRow {
    ArtifactRow {
        conversation_id: conversation_id.to_string(),
        collection: "proposed_plans".to_string(),
        id: id.to_string(),
        turn_id: turn_id.map(str::to_string),
        status: "pending".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        payload: json!({"id": id}),
    }
}

fn attachment(conversation_id: &str, id: &str, status: &str) -> AttachmentRow {
    AttachmentRow {
        conversation_id: conversation_id.to_string(),
        id: id.to_string(),
        blob_path: format!("attachments/{id}"),
        status: status.to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        payload: json!({"id": id}),
    }
}

fn watermark(database: &ConversationDatabase) -> i64 {
    database.read(|read| Ok(read.watermark())).expect("read")
}

fn batch(database: &ConversationDatabase, scope: ChangeScope<'_>, since: i64) -> ChangeBatch {
    match database
        .read(|read| read.changes(scope, database.generation(), since))
        .expect("changes")
    {
        CatchUp::Batch(batch) => batch,
        CatchUp::Reset(reason) => panic!("unexpected reset {reason:?}"),
    }
}

fn page(
    database: &ConversationDatabase,
    before: Option<&spark_storage::conversation::PageCursor>,
) -> TurnPage {
    match database
        .read(|read| read.page("c1", 2, before))
        .expect("page")
    {
        PageRead::Page(page) => page,
        other => panic!("unexpected page read {other:?}"),
    }
}

#[test]
fn open_configures_pragmas_and_keeps_generation_across_reopen() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    let database = ConversationDatabase::open(&home).expect("open");

    assert_eq!(database.path(), home.join(CONVERSATION_DATABASE_FILE_NAME));
    assert_eq!(database.pragma("journal_mode").unwrap(), "wal");
    assert_eq!(database.pragma("synchronous").unwrap(), "1", "NORMAL");
    assert_eq!(database.pragma("foreign_keys").unwrap(), "1");
    assert_eq!(database.pragma("checkpoint_fullfsync").unwrap(), "1");
    assert_eq!(database.pragma("wal_autocheckpoint").unwrap(), "1000");

    let generation = database.generation().to_string();
    assert_eq!(generation.len(), 36, "uuid generation: {generation}");
    assert_eq!(watermark(&database), 0);
    drop(database);

    let reopened = ConversationDatabase::open(&home).expect("reopen");
    assert_eq!(reopened.generation(), generation);
    let (_other_temp, other) = open();
    assert_ne!(other.generation(), generation);
}

#[test]
fn unsupported_schema_version_fails_open_without_wiping_rows() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    let database = ConversationDatabase::open(&home).expect("open");
    database
        .write(|write| write.put_conversation(&conversation("c1", "/p", "t1")))
        .expect("write");
    drop(database);

    let raw = rusqlite::Connection::open(home.join(CONVERSATION_DATABASE_FILE_NAME)).unwrap();
    raw.execute("UPDATE store_meta SET schema_version = 99", [])
        .unwrap();

    let error = ConversationDatabase::open(&home)
        .err()
        .expect("future schema must fail");
    assert!(
        matches!(
            error,
            StorageError::UnsupportedConversationDatabase {
                found: 99,
                expected: CONVERSATION_DATABASE_SCHEMA_VERSION,
                ..
            }
        ),
        "{error}"
    );
    let message = error.to_string();
    assert!(
        message.contains("stop every Spark server") && message.contains("remove exactly"),
        "{message}"
    );
    let rows: i64 = raw
        .query_row("SELECT count(*) FROM conversations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 1);
}

#[test]
fn constraints_reject_orphans_duplicate_live_handles_and_invalid_values() {
    let (_temp, database) = open();
    database
        .write(|write| {
            write.put_conversation(&conversation("c1", "/p", "t1"))?;
            write.put_turn(&turn("c1", "t1", 0))
        })
        .expect("seed");
    let before = watermark(&database);

    let rejected = [
        database
            .write(|write| write.put_turn(&turn("missing", "t", 0)))
            .map(drop),
        database
            .write(|write| write.put_segment(&segment("c1", "s", "missing-turn", 0)))
            .map(drop),
        database
            .write(|write| write.put_artifact(&artifact("c1", "a", Some("missing-turn"))))
            .map(drop),
        database
            .write(|write| write.put_attachment(&attachment("c1", "x", "uploaded")))
            .map(drop),
        database
            .write(|write| {
                let mut duplicate = conversation("c2", "/p", "t1");
                duplicate.handle = "handle-c1".to_string();
                write.put_conversation(&duplicate)
            })
            .map(drop),
        database
            .write(|write| {
                let mut invalid = conversation("c3", "/p", "t1");
                invalid.metadata = json!(["not", "an", "object"]);
                write.put_conversation(&invalid)
            })
            .map(drop),
        database
            .write(|write| write.put_blob_reference("c1", "unknown", "x", "blobs/x"))
            .map(drop),
    ];
    for (index, result) in rejected.into_iter().enumerate() {
        assert!(result.is_err(), "write {index} should be rejected");
    }
    assert_eq!(
        watermark(&database),
        before,
        "rejected writes allocate nothing"
    );

    // An unanchored artifact is valid; a deleted conversation releases its handle.
    database
        .write(|write| write.put_artifact(&artifact("c1", "free", None)))
        .expect("unanchored artifact");
    database
        .write(|write| write.delete_conversation("c1"))
        .expect("delete");
    database
        .write(|write| {
            let mut reuse = conversation("c2", "/p", "t2");
            reuse.handle = "handle-c1".to_string();
            write.put_conversation(&reuse)
        })
        .expect("handle reusable after tombstone");
    let error = database
        .write(|write| write.put_turn(&turn("c1", "t2", 1)))
        .expect_err("children of a deleted conversation are rejected");
    assert!(matches!(
        error,
        StorageError::ConversationCommitRejected { .. }
    ));
}

#[test]
fn failed_transaction_rolls_back_rows_and_revision_allocation() {
    let (_temp, database) = open();
    let (first, ()) = database
        .write(|write| write.put_conversation(&conversation("c1", "/p", "t1")))
        .expect("seed");
    assert_eq!(first, 1);

    let error = database
        .write(|write| -> spark_storage::Result<()> {
            write.put_turn(&turn("c1", "t1", 0))?;
            write.put_conversation(&conversation("c1", "/p", "t2"))?;
            Err(StorageError::ConversationCommitRejected {
                conversation_id: "c1".to_string(),
                reason: "validation failed".to_string(),
            })
        })
        .expect_err("rolled back");
    assert!(matches!(
        error,
        StorageError::ConversationCommitRejected { .. }
    ));
    assert_eq!(watermark(&database), 1);
    let changes = batch(&database, ChangeScope::Conversation("c1"), 0);
    assert_eq!(changes.rows.len(), 1);
    assert_eq!(changes.rows[0].revision, 1);
    assert_eq!(
        changes.rows[0].payload.as_ref().unwrap()["updated_at"],
        "t1"
    );

    let (next, ()) = database
        .write(|write| write.put_turn(&turn("c1", "t1", 0)))
        .expect("next");
    assert_eq!(next, 2, "no revision survives rollback");
}

#[test]
fn one_revision_stamps_every_changed_row_and_tombstones_flow_through_catch_up() {
    let (_temp, database) = open();
    let (created, ()) = database
        .write(|write| {
            write.put_conversation(&conversation("c1", "/p", "t1"))?;
            write.put_turn(&turn("c1", "t1", 0))?;
            write.put_segment(&segment("c1", "s1", "t1", 0))?;
            write.put_artifact(&artifact("c1", "a1", Some("t1")))?;
            write.put_attachment(&attachment("c1", "x1", "attached"))?;
            write.put_blob_reference("c1", "attachment", "x1", "attachments/x1")?;
            write.put_runtime_session("c1", "codex", &json!({"thread_id": "th"}))
        })
        .expect("create");
    let rows = batch(&database, ChangeScope::Conversation("c1"), 0).rows;
    assert_eq!(
        rows.len(),
        5,
        "internal session/reference rows are not exposed"
    );
    assert!(rows
        .iter()
        .all(|row| row.revision == created && !row.deleted));
    let conversation_row = rows
        .iter()
        .find(|row| row.kind == EntityKind::Conversation)
        .unwrap();
    let payload = conversation_row.payload.as_ref().unwrap();
    assert_eq!(payload["running"], false);
    assert_eq!(payload["metadata"]["chat_mode"], "chat");
    assert!(payload.get("turns").is_none());

    let (removed, deleted) = database
        .write(|write| write.delete_segment("c1", "s1"))
        .expect("delete segment");
    assert!(deleted);
    let changes = batch(&database, ChangeScope::Conversation("c1"), created);
    assert_eq!(changes.watermark, removed);
    let mut kinds: Vec<_> = changes
        .rows
        .iter()
        .map(|row| (row.kind, row.deleted, row.payload.is_none()))
        .collect();
    kinds.sort_by_key(|(kind, ..)| *kind as u8);
    assert_eq!(
        kinds,
        vec![
            (EntityKind::Conversation, false, false),
            (EntityKind::Segment, true, true)
        ]
    );
    let tombstone = changes
        .rows
        .iter()
        .find(|row| row.kind == EntityKind::Segment)
        .unwrap();
    assert_eq!(
        tombstone.turn_id.as_deref(),
        Some("t1"),
        "tombstones keep their owner"
    );
    assert!(page(&database, None).rows.iter().all(|row| row.id != "s1"));

    let (gone, _) = database
        .write(|write| write.delete_conversation("c1"))
        .expect("delete conversation");
    assert!(database
        .read(|read| read.summaries(Some("/p")))
        .unwrap()
        .is_empty());
    assert_eq!(
        database.read(|read| read.page("c1", 50, None)).unwrap(),
        PageRead::Missing
    );
    // An arbitrarily old cursor still converges to the tombstones.
    let rows = batch(&database, ChangeScope::Conversation("c1"), 0).rows;
    assert_eq!(rows.len(), 5);
    assert!(rows
        .iter()
        .all(|row| row.deleted && row.payload.is_none() && row.revision >= removed));
    assert!(rows
        .iter()
        .filter(|row| row.kind != EntityKind::Segment)
        .all(|row| row.revision == gone));
    let summary = batch(
        &database,
        ChangeScope::Summaries { project_path: None },
        created,
    )
    .rows;
    assert_eq!(summary.len(), 1);
    assert!(summary[0].deleted);
}

#[test]
fn catch_up_filters_scope_and_requests_resync_instead_of_partial_batches() {
    let (_temp, database) = open();
    database
        .write(|write| {
            write.put_conversation(&conversation("c1", "/a", "t1"))?;
            write.put_conversation(&conversation("c2", "/b", "t1"))
        })
        .unwrap();
    let base = watermark(&database);
    database
        .write(|write| write.put_conversation(&conversation("c2", "/b", "t2")))
        .unwrap();

    let filtered = batch(
        &database,
        ChangeScope::Summaries {
            project_path: Some("/a"),
        },
        base,
    );
    assert!(
        filtered.rows.is_empty(),
        "other projects' revisions are filtered"
    );
    assert_eq!(filtered.watermark, base + 1, "empty batches still advance");
    assert_eq!(
        batch(
            &database,
            ChangeScope::Summaries { project_path: None },
            base
        )
        .rows
        .len(),
        1
    );

    let reset = |generation: &str, since| {
        database
            .read(|read| read.changes(ChangeScope::Conversation("c1"), generation, since))
            .unwrap()
    };
    assert_eq!(
        reset("stale", 0),
        CatchUp::Reset(ResetReason::GenerationChanged)
    );
    assert_eq!(
        reset(database.generation(), base + 2),
        CatchUp::Reset(ResetReason::CursorAhead)
    );

    let mark = watermark(&database);
    database
        .write(|write| {
            for position in 0..CATCH_UP_MAX_ROWS as i64 {
                write.put_turn(&turn("c1", &format!("t{position}"), position))?;
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(
        reset(database.generation(), mark),
        CatchUp::Reset(ResetReason::CatchupLimit),
        "{} rows exceed the cap",
        CATCH_UP_MAX_ROWS + 1
    );

    let mark = watermark(&database);
    database
        .write(|write| {
            let mut large = turn("c1", "large", 5000);
            large.payload = json!({"text": "x".repeat(CATCH_UP_MAX_BYTES)});
            write.put_turn(&large)
        })
        .unwrap();
    assert_eq!(
        reset(database.generation(), mark),
        CatchUp::Reset(ResetReason::CatchupLimit)
    );
}

#[test]
fn pages_use_keyset_order_and_reset_only_when_existing_turns_move() {
    let (_temp, database) = open();
    database
        .write(|write| {
            write.put_conversation(&conversation("c1", "/p", "t1"))?;
            for position in 0..5 {
                let id = format!("t{position}");
                write.put_turn(&turn("c1", &id, position))?;
                write.put_segment(&segment("c1", &format!("s{position}b"), &id, 1))?;
                write.put_segment(&segment("c1", &format!("s{position}a"), &id, 0))?;
            }
            write.put_artifact(&artifact("c1", "a4", Some("t4")))?;
            write.put_artifact(&artifact("c1", "a0", Some("t0")))
        })
        .unwrap();

    let newest = page(&database, None);
    assert_eq!(newest.turn_ids, ["t3", "t4"]);
    let ids: Vec<_> = newest.rows.iter().map(|row| row.id.as_str()).collect();
    // a0 awaits review, so the window pins it although t0 is outside the page.
    assert_eq!(ids, ["t3", "t4", "s3a", "s3b", "s4a", "s4b", "a4", "a0"]);
    assert_eq!(
        newest.pinned_artifact_ids,
        [("proposed_plans".to_string(), "a0".to_string())]
    );
    assert!(newest.pinned_turn_ids.is_empty());
    let cursor = newest.older_cursor.clone().expect("older cursor");
    assert_eq!(cursor.opening_revision, newest.watermark);

    // Appending at the newest end keeps older cursors valid.
    database
        .write(|write| write.put_turn(&turn("c1", "t5", 5)))
        .unwrap();
    let older = page(&database, Some(&cursor));
    assert_eq!(older.turn_ids, ["t1", "t2"]);
    let oldest_cursor = older.older_cursor.clone().unwrap();
    assert_eq!(oldest_cursor.opening_revision, cursor.opening_revision);
    let oldest = page(&database, Some(&oldest_cursor));
    assert_eq!(oldest.turn_ids, ["t0"]);
    assert!(oldest.rows.iter().any(|row| row.id == "a0"));
    assert_eq!(oldest.older_cursor, None);

    // Repositioning an existing turn invalidates cursors opened before it.
    database
        .write(|write| write.put_turn(&turn("c1", "t1", 6)))
        .unwrap();
    assert_eq!(
        database
            .read(|read| read.page("c1", 2, Some(&cursor)))
            .unwrap(),
        PageRead::Reset(ResetReason::OrderChanged)
    );
    let reopened = page(&database, None);
    assert_eq!(reopened.turn_ids, ["t5", "t1"]);
    assert!(matches!(
        database
            .read(|read| read.page("c1", 2, reopened.older_cursor.as_ref()))
            .unwrap(),
        PageRead::Page(_)
    ));
}

#[test]
fn windows_pin_in_flight_turns_with_their_segments_outside_the_page() {
    let (_temp, database) = open();
    database
        .write(|write| {
            write.put_conversation(&conversation("c1", "/p", "t1"))?;
            for position in 0..4 {
                let id = format!("t{position}");
                let mut row = turn("c1", &id, position);
                if position == 0 {
                    row.status = "queued".to_string();
                }
                if position == 1 {
                    row.status = "streaming".to_string();
                }
                write.put_turn(&row)?;
                write.put_segment(&segment("c1", &format!("s{position}"), &id, 0))?;
            }
            write.put_artifact(&ArtifactRow {
                status: "complete".to_string(),
                ..artifact("c1", "done", Some("t0"))
            })
        })
        .unwrap();
    let newest = page(&database, None);
    assert_eq!(newest.turn_ids, ["t2", "t3"]);
    assert_eq!(newest.pinned_turn_ids, ["t0", "t1"]);
    let ids: Vec<_> = newest.rows.iter().map(|row| row.id.as_str()).collect();
    // Settled artifacts of turns outside the page stay out of the window.
    assert_eq!(ids, ["t2", "t3", "s2", "s3", "t0", "s0", "t1", "s1"]);
    // An older page still returns a pinned turn in its place, and pins nothing twice.
    let older = page(&database, newest.older_cursor.as_ref());
    assert_eq!(older.turn_ids, ["t0", "t1"]);
    assert!(older.pinned_turn_ids.is_empty());
}

#[test]
fn reads_see_one_snapshot_while_another_connection_commits() {
    let (_temp, database) = open();
    database
        .write(|write| write.put_conversation(&conversation("c1", "/p", "t1")))
        .unwrap();

    database
        .read(|read| {
            let before = read.summaries(Some("/p"))?;
            let writer = rusqlite::Connection::open(database.path()).unwrap();
            writer
                .execute_batch(
                    "UPDATE store_meta SET next_revision = next_revision + 1;
                     UPDATE conversations SET title = 'Renamed', revision = 2 WHERE id = 'c1';",
                )
                .unwrap();
            assert_eq!(read.watermark(), 1);
            assert_eq!(read.summaries(Some("/p"))?, before);
            match read.changes(ChangeScope::Conversation("c1"), database.generation(), 1)? {
                CatchUp::Batch(batch) => assert!(batch.rows.is_empty()),
                other => panic!("{other:?}"),
            }
            Ok(())
        })
        .unwrap();

    let after = batch(&database, ChangeScope::Conversation("c1"), 1);
    assert_eq!(after.watermark, 2);
    assert_eq!(after.rows[0].payload.as_ref().unwrap()["title"], "Renamed");
}

#[test]
fn listing_returns_live_summaries_in_update_order() {
    let (_temp, database) = open();
    database
        .write(|write| {
            write.put_conversation(&conversation("c1", "/a", "2026-01-01T00:00:01Z"))?;
            write.put_conversation(&conversation("c2", "/a", "2026-01-01T00:00:03Z"))?;
            let mut flagged = conversation("c3", "/b", "2026-01-01T00:00:02Z");
            flagged.needs_attention = true;
            write.put_conversation(&flagged)
        })
        .unwrap();
    let ids = |rows: Vec<spark_storage::conversation::EntityRow>| {
        rows.into_iter().map(|row| row.id).collect::<Vec<_>>()
    };
    assert_eq!(
        ids(database.read(|r| r.summaries(Some("/a"))).unwrap()),
        ["c2", "c1"]
    );
    assert_eq!(
        ids(database.read(|r| r.summaries(None)).unwrap()),
        ["c2", "c3", "c1"]
    );
    assert_eq!(
        ids(database.read(|r| r.attention_summaries()).unwrap()),
        ["c3"]
    );
}

#[test]
fn read_query_plans_use_page_change_and_summary_indexes() {
    let (_temp, database) = open();
    let expected: &[(&str, &[&str])] = &[
        ("summaries_for_project", &["conversations_project_listing"]),
        ("attention_summaries", &["conversations_attention_listing"]),
        ("summary_changes", &["conversations_revision"]),
        (
            "conversation_changes",
            &[
                "turns_revision",
                "segments_revision",
                "artifacts_revision",
                "attachments_revision",
            ],
        ),
        ("order_changed", &["turns_order_changed"]),
        ("page_turns", &["turns_page"]),
        ("page_segments", &["segments_page"]),
        ("page_artifacts", &["artifacts_owner"]),
        ("pinned_turns", &["turns_status"]),
        ("pinned_artifacts", &["artifacts_pending"]),
    ];
    let plans = database.read_query_plans().expect("plans");
    assert_eq!(plans.len(), expected.len());
    for ((name, details), (expected_name, indexes)) in plans.iter().zip(expected) {
        assert_eq!(name, expected_name);
        for detail in details {
            assert!(
                (detail == "SCAN CONSTANT ROW" || !detail.starts_with("SCAN"))
                    && !detail.contains("TEMP B-TREE"),
                "{name} plan must not scan or sort: {details:?}"
            );
        }
        for index in *indexes {
            assert!(
                details
                    .iter()
                    .any(|detail| detail.contains(&format!("INDEX {index} ("))),
                "{name} plan should use {index}: {details:?}"
            );
        }
    }
}

#[test]
fn catch_up_byte_cap_counts_whole_rows_including_identities_and_tombstones() {
    let (_temp, database) = open();
    let long_turns = |conversation_id: &str, count: usize| {
        (0..count)
            .map(|index| TurnRow {
                payload: json!({}),
                ..turn(
                    conversation_id,
                    &format!("{index:04}{}", "t".repeat(4000)),
                    index as i64,
                )
            })
            .collect::<Vec<_>>()
    };
    database
        .write(|write| {
            write.put_conversation(&conversation("fits", "/p", "t1"))?;
            write.put_conversation(&conversation("big", "/p", "t1"))?;
            long_turns("fits", 300)
                .iter()
                .chain(&long_turns("big", 600))
                .try_for_each(|row| write.put_turn(row))
        })
        .unwrap();
    let catch_up = |conversation_id| {
        database
            .read(|read| {
                read.changes(
                    ChangeScope::Conversation(conversation_id),
                    database.generation(),
                    0,
                )
            })
            .unwrap()
    };

    // Under the cap the whole range comes back.
    match catch_up("fits") {
        CatchUp::Batch(batch) => assert_eq!(batch.rows.len(), 301),
        other => panic!("{other:?}"),
    }
    // Payloads total a few KiB, but the identities push the rows past 2 MiB.
    assert!(600 * "{}".len() < CATCH_UP_MAX_BYTES && 600 * 4000 > CATCH_UP_MAX_BYTES);
    assert_eq!(catch_up("big"), CatchUp::Reset(ResetReason::CatchupLimit));

    // Tombstones carry no payload and are still counted.
    database
        .write(|write| {
            long_turns("big", 600)
                .iter()
                .try_for_each(|row| write.delete_turn("big", &row.id).map(drop))
        })
        .unwrap();
    assert_eq!(catch_up("big"), CatchUp::Reset(ResetReason::CatchupLimit));
}

/// The periodic passive checkpoint runs only after writes, reports an
/// incomplete result, and stays pending so the next attempt retries it.
#[test]
fn passive_checkpoint_reports_incomplete_results_and_retries() {
    let (_temp, database) = open();
    assert!(!database.checkpoint_if_written().unwrap());
    database
        .write(|write| write.put_conversation(&conversation("c1", "/p", "t1")))
        .unwrap();
    assert!(database.checkpoint_pending());

    // A reader pinned to an older snapshot blocks checkpointing newer frames.
    let reader = rusqlite::Connection::open(database.path()).unwrap();
    reader.execute_batch("BEGIN").unwrap();
    reader
        .query_row("SELECT count(*) FROM conversations", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
    database
        .write(|write| write.put_conversation(&conversation("c2", "/p", "t2")))
        .unwrap();
    assert!(matches!(
        database.checkpoint_if_written(),
        Err(StorageError::CheckpointIncomplete { .. })
    ));
    assert!(database.checkpoint_pending());

    reader.execute_batch("COMMIT").unwrap();
    assert!(database.checkpoint_if_written().unwrap());
    assert!(!database.checkpoint_pending());
    assert!(!database.checkpoint_if_written().unwrap());

    // The shutdown checkpoint empties the log once writers stop.
    database.checkpoint_on_shutdown().unwrap();
    let wal = database.path().with_extension("sqlite3-wal");
    assert_eq!(std::fs::metadata(wal).unwrap().len(), 0);
}

const MEASURE_PHASE_ENV: &str = "SPARK_MEASURE_PHASE";
const MEASURE_ID: &str = "conversation-measure";
const MEASURE_PROJECT: &str = "/projects/measure";

/// Manual growth/latency measurement; no thresholds. Run with
/// `cargo test --release --all-features -p spark-storage --test contracts -- --ignored --exact
/// conversation_database_contracts::measure_conversation_growth_and_acceptance --nocapture`.
/// `SPARK_MEASURE_TURNS` (default `100,10000,100000`), `SPARK_MEASURE_SYNC`
/// (default `NORMAL,FULL`) and `SPARK_MEASURE_REPS` (default 200) select the
/// workload. Each configuration seeds a fresh home in one process and
/// measures it in another, so memory readings exclude seeding.
#[test]
#[ignore = "manual measurement"]
fn measure_conversation_growth_and_acceptance() {
    let env = |name: &str| std::env::var(name).ok();
    if let Some(phase) = env(MEASURE_PHASE_ENV) {
        let home = std::path::PathBuf::from(env("SPARK_MEASURE_HOME").expect("home"));
        let turns: usize = env("SPARK_MEASURE_TURNS").expect("turns").parse().unwrap();
        if phase == "seed" {
            seed_measured_conversation(&home, turns);
        } else {
            let reps = env("SPARK_MEASURE_REPS").map_or(200, |reps| reps.parse().unwrap());
            let sync = env("SPARK_MEASURE_SYNC").expect("sync");
            println!("MEASURE {}", measure_storage(&home, turns, &sync, reps));
        }
        return;
    }
    let list = |name: &str, default: &str| -> Vec<String> {
        env(name)
            .unwrap_or_else(|| default.to_string())
            .split(',')
            .map(str::to_string)
            .collect()
    };
    for turns in list("SPARK_MEASURE_TURNS", "100,10000,100000") {
        for sync in list("SPARK_MEASURE_SYNC", "NORMAL,FULL") {
            let temp = tempfile::tempdir().expect("tempdir");
            for phase in ["seed", "measure"] {
                let output = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--ignored",
                        "--exact",
                        "conversation_database_contracts::measure_conversation_growth_and_acceptance",
                        "--nocapture",
                    ])
                    .env(MEASURE_PHASE_ENV, phase)
                    .env("SPARK_MEASURE_HOME", temp.path().join("spark-home"))
                    .env("SPARK_MEASURE_TURNS", &turns)
                    .env("SPARK_MEASURE_SYNC", &sync)
                    .output()
                    .expect("measurement child");
                let stdout = String::from_utf8_lossy(&output.stdout);
                assert!(output.status.success(), "{stdout}\n{output:?}");
                for line in stdout.lines().filter(|line| line.starts_with("MEASURE ")) {
                    println!("{line}");
                }
            }
        }
    }
}

/// Resident set size of this process in KiB, from `ps`.
fn rss_kib() -> u64 {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

/// p50/p95/p99 of `samples`, in microseconds.
fn percentiles(samples: &mut [std::time::Duration]) -> serde_json::Value {
    samples.sort();
    let at = |q: f64| {
        let index = ((q * samples.len() as f64).ceil() as usize).clamp(1, samples.len()) - 1;
        samples[index].as_micros() as u64
    };
    json!({"p50_us": at(0.50), "p95_us": at(0.95), "p99_us": at(0.99), "n": samples.len()})
}

fn payload_bytes(rows: &[spark_storage::conversation::EntityRow]) -> usize {
    rows.iter()
        .map(|row| {
            row.payload
                .as_ref()
                .map_or(0, |payload| payload.to_string().len())
        })
        .sum()
}

const MEASURE_TEXT: &str = "lorem ipsum dolor sit amet lorem ipsum dolor sit amet lorem ipsum \
    dolor sit amet lorem ipsum dolor sit amet lorem ipsum dolor sit amet lorem ipsum dolor sit \
    amet lorem ipsum dolor sit amet lorem ipsum dolor sit amet lorem ipsum dolor sit amet";

fn measure_turn(
    id: String,
    role: &str,
    status: &str,
) -> spark_storage::conversation::TranscriptTurn {
    spark_storage::conversation::TranscriptTurn {
        id,
        role: role.to_string(),
        kind: Some("message".to_string()),
        content: MEASURE_TEXT.to_string(),
        status: status.to_string(),
        timestamp: "2026-01-01T00:00:00Z".to_string(),
        ..Default::default()
    }
}

fn measure_segment(turn_id: &str) -> spark_storage::conversation::TranscriptSegment {
    serde_json::from_value(json!({
        "id": format!("segment-{turn_id}"), "turn_id": turn_id, "order": 0,
        "kind": "assistant_message", "role": "assistant", "status": "complete",
        "timestamp": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
        "content": MEASURE_TEXT,
    }))
    .unwrap()
}

/// The conversation's `turns` of history (alternating user and assistant
/// turns, each answer with one segment) are SQL rows written in a single
/// transaction rather than repeated operations.
fn seed_measured_conversation(home: &std::path::Path, turns: usize) {
    use spark_storage::{ConversationRepository, ConversationStore};
    let store = ConversationStore::open(home).unwrap();
    ConversationRepository::new(store.clone())
        .touch_conversation(MEASURE_ID, MEASURE_PROJECT)
        .unwrap();
    store
        .database()
        .write(|tx| {
            for index in 0..turns {
                let turn_id = format!("turn-h{index}");
                let role = if index % 2 == 1 { "assistant" } else { "user" };
                tx.put_turn(&TurnRow {
                    conversation_id: MEASURE_ID.into(),
                    id: turn_id.clone(),
                    role: role.into(),
                    kind: "message".into(),
                    position: index as i64,
                    settled: true,
                    status: "complete".into(),
                    updated_at: "2026-01-01T00:00:00Z".into(),
                    payload: serde_json::to_value(measure_turn(turn_id.clone(), role, "complete"))
                        .unwrap(),
                })?;
                if role == "assistant" {
                    tx.put_segment(&SegmentRow {
                        conversation_id: MEASURE_ID.into(),
                        id: format!("segment-{turn_id}"),
                        turn_id: turn_id.clone(),
                        kind: "assistant_message".into(),
                        order: 0,
                        status: "complete".into(),
                        updated_at: "2026-01-01T00:00:00Z".into(),
                        payload: serde_json::to_value(measure_segment(&turn_id)).unwrap(),
                    })?;
                }
            }
            Ok(())
        })
        .unwrap();
}

/// Measure one seeded home: cold open, cold first acceptance, warm
/// acceptance, the SQL transaction alone, page reads and checkpoints.
fn measure_storage(
    home: &std::path::Path,
    turns: usize,
    sync: &str,
    reps: usize,
) -> serde_json::Value {
    use spark_storage::conversation::{Accepted, Admission, ExecutionOutcome, MessageInput};
    use spark_storage::{ConversationRepository, ConversationStore};
    use std::time::Instant;
    let (id, project) = (MEASURE_ID, MEASURE_PROJECT);
    let database_bytes = std::fs::metadata(home.join(CONVERSATION_DATABASE_FILE_NAME))
        .unwrap()
        .len();

    // Cold open: the store, then the bounded newest page a browser opens.
    let rss_before_open = rss_kib();
    let started = Instant::now();
    let store = ConversationStore::open(home).unwrap();
    let store_open_us = started.elapsed().as_micros() as u64;
    store.database().set_synchronous(sync).unwrap();
    assert_eq!(
        store.database().pragma("synchronous").unwrap(),
        if sync == "FULL" { "2" } else { "1" }
    );
    let repo = ConversationRepository::new(store.clone());
    let read_page = || {
        let started = Instant::now();
        let PageRead::Page(page) = store
            .database()
            .read(|read| read.page(id, 50, None))
            .unwrap()
        else {
            panic!("page");
        };
        (started.elapsed(), page)
    };
    let (cold_page, page) = read_page();
    let page_rows = page.rows.len() + 1;
    let page_payload_bytes = payload_bytes(&page.rows);
    let page_json_bytes = serde_json::to_vec(&page.rows).unwrap().len();
    drop(page);
    let rss_after_open = rss_kib();

    // Acceptance: a user turn and its pending answer. The answer settles in
    // a second, unmeasured operation, as a finished provider turn would.
    let mut position = 0;
    let mut accept = || {
        position += 1;
        let assistant = format!("turn-a{position}");
        let started = Instant::now();
        let (commit, accepted) = repo
            .accept_message(
                id,
                project,
                MessageInput {
                    admission: Admission::Start,
                    user_turn_id: format!("turn-u{position}"),
                    content: MEASURE_TEXT.to_string(),
                    attachments: None,
                    client_request_sha256: None,
                    expected_settings_revision: 0,
                    settings: Default::default(),
                    held: false,
                    assistant_turn_id: assistant.clone(),
                    execution_settings: json!({}),
                },
            )
            .unwrap();
        let elapsed = started.elapsed();
        assert!(matches!(accepted, Accepted::Started { .. }), "{accepted:?}");
        (elapsed, commit.expect("acceptance commit"), assistant)
    };
    let settle = |assistant: &str| {
        repo.settle_execution(
            id,
            project,
            assistant,
            ExecutionOutcome::Returned {
                final_text: Some(MEASURE_TEXT.to_string()),
                token_usage: None,
                token_usage_breakdown: None,
                app_thread_id: None,
                app_turn_id: None,
                thread_resume_failure: None,
            },
            &Default::default(),
        )
        .unwrap();
    };
    let (cold_accept, _, assistant) = accept();
    settle(&assistant);
    let rss_after_first_acceptance = rss_kib();

    let mut warm = Vec::with_capacity(reps);
    let (mut accepted_rows, mut accepted_payload, mut accepted_json) = (0, 0, 0);
    for _ in 0..reps {
        let (elapsed, commit, assistant) = accept();
        settle(&assistant);
        warm.push(elapsed);
        accepted_rows = accepted_rows.max(commit.changes.rows.len());
        accepted_payload = accepted_payload.max(payload_bytes(&commit.changes.rows));
        accepted_json = accepted_json.max(serde_json::to_vec(&commit.changes).unwrap().len());
    }

    // WAL frames one acceptance writes: after a complete checkpoint the next
    // commit restarts the WAL, so the frame count is that commit's pages.
    let page_size: u64 = store
        .database()
        .pragma("page_size")
        .unwrap()
        .parse()
        .unwrap();
    let mut frames = Vec::new();
    for _ in 0..20 {
        store.database().passive_checkpoint_frames().unwrap();
        let (_, _, assistant) = accept();
        frames.push(store.database().passive_checkpoint_frames().unwrap().1);
        settle(&assistant);
    }
    frames.sort();
    let acceptance_wal_frames = frames[frames.len() / 2] as u64;

    // The SQL transaction alone, with the rows an acceptance writes.
    let mut transactions = Vec::with_capacity(reps);
    for index in 0..reps {
        let started = Instant::now();
        store
            .database()
            .write(|tx| {
                for (offset, role) in [(0, "user"), (1, "assistant")] {
                    let turn_id = format!("turn-tx{index}-{role}");
                    tx.put_turn(&TurnRow {
                        conversation_id: id.into(),
                        id: turn_id.clone(),
                        role: role.into(),
                        kind: "message".into(),
                        position: (turns + 1_000_000 + 2 * index + offset) as i64,
                        settled: true,
                        status: "complete".into(),
                        updated_at: "2026-01-01T00:00:00Z".into(),
                        payload: serde_json::to_value(measure_turn(turn_id, role, "complete"))
                            .unwrap(),
                    })?;
                }
                tx.changed_rows(id)
            })
            .unwrap();
        transactions.push(started.elapsed());
    }

    // Checkpoints of exactly one acceptance's frames: the cleanup FULL
    // barrier, then the periodic passive checkpoint.
    let wal = home.join(format!("{CONVERSATION_DATABASE_FILE_NAME}-wal"));
    let wal_bytes = std::fs::metadata(&wal).map_or(0, |meta| meta.len());
    store.database().passive_checkpoint_frames().unwrap();
    let (_, _, assistant) = accept();
    let started = Instant::now();
    store.database().after_checkpoint(|_| Ok(())).unwrap();
    let full_barrier_us = started.elapsed().as_micros() as u64;
    settle(&assistant);
    store.database().passive_checkpoint_frames().unwrap();
    let (_, _, assistant) = accept();
    let started = Instant::now();
    store.database().checkpoint_if_written().unwrap();
    let passive_us = started.elapsed().as_micros() as u64;
    settle(&assistant);
    let (warm_page, _) = read_page();
    let rss_end = rss_kib();

    json!({
        "turns": turns,
        "synchronous": sync,
        "repetitions": reps,
        "database_bytes": database_bytes,
        "cold_store_open_us": store_open_us,
        "cold_page_us": cold_page.as_micros() as u64,
        "warm_page_us": warm_page.as_micros() as u64,
        "page_rows": page_rows,
        "page_payload_bytes": page_payload_bytes,
        "page_json_bytes": page_json_bytes,
        "cold_first_acceptance_us": cold_accept.as_micros() as u64,
        "warm_acceptance": percentiles(&mut warm),
        "acceptance_rows_written": accepted_rows,
        "acceptance_payload_bytes_written": accepted_payload,
        "acceptance_response_json_bytes": accepted_json,
        "acceptance_wal_frames": acceptance_wal_frames,
        "acceptance_wal_bytes": acceptance_wal_frames * (page_size + 24),
        "sql_transaction": percentiles(&mut transactions),
        "wal_file_bytes": wal_bytes,
        "full_barrier_after_one_acceptance_us": full_barrier_us,
        "passive_checkpoint_after_one_acceptance_us": passive_us,
        "rss_kib": {"before_open": rss_before_open, "after_open_and_page": rss_after_open,
            "after_first_acceptance": rss_after_first_acceptance, "end": rss_end},
    })
}
