use serde_json::json;
use spark_storage::conversation::{
    ArtifactRow, AttachmentRow, CatchUp, ChangeBatch, ChangeScope, ConversationDatabase,
    ConversationRow, EntityKind, PageRead, ResetReason, SegmentRow, TurnPage, TurnRow,
    CATCH_UP_MAX_BYTES, CATCH_UP_MAX_ROWS, CONVERSATION_DATABASE_FILE_NAME,
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
        position,
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
                expected: 1,
                ..
            }
        ),
        "{error}"
    );
    assert!(error.to_string().contains("remove it to reset"));
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
