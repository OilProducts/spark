//! Test fixture: stores one conversation's rows exactly as a snapshot gives
//! them (ids, timestamps, statuses), writing the database file directly as
//! an outside writer. Spark itself changes conversation state only through
//! the repository's typed operations; it has no row writer to call. Loaded
//! with `#[path]` by the test binaries that seed fixtures.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use rusqlite::{params, Transaction, TransactionBehavior};
use serde_json::{json, Value};
use spark_storage::conversation::{record_from_snapshot, ArtifactCollection, ORDER_UNASSIGNED};
use spark_storage::{ConversationRepository, ConversationStore, ProjectRegistry};

const COLLECTIONS: [ArtifactCollection; 4] = [
    ArtifactCollection::FlowRunRequests,
    ArtifactCollection::FlowLaunches,
    ArtifactCollection::RunRecoveries,
    ArtifactCollection::ProposedPlans,
];

pub fn seed_conversation_snapshot(store: &Arc<ConversationStore>, snapshot: &Value) {
    let record = record_from_snapshot(snapshot).expect("conversation snapshot");
    let mut meta = record.meta;
    if snapshot.get("settings_schema_version").is_none() {
        meta.settings_schema_version = 1;
    }
    let id = meta.conversation_id.clone();
    let paths = ProjectRegistry::new(store.home_dir().to_path_buf())
        .ensure_project_paths(&meta.project_path)
        .expect("project paths");
    meta.project_path = paths.project_path;
    let root = paths.conversations_dir.join(&id);
    let repository = ConversationRepository::new(store.clone());
    if meta.conversation_handle.is_empty() {
        meta.conversation_handle = repository
            .conversation_handle(&id, None)
            .expect("conversation handle");
    }
    let turns = &record.transcript.turns;
    let message = |turn: &&spark_storage::conversation::TranscriptTurn| {
        turn.kind.as_deref().unwrap_or("message") == "message"
    };
    if meta.title.trim().is_empty() {
        meta.title = turns
            .iter()
            .filter(message)
            .find(|turn| turn.role == "user")
            .map(|turn| truncate_text(&turn.content, 64))
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| "New thread".to_string());
    }
    let preview = turns
        .iter()
        .rev()
        .filter(message)
        .map(|turn| truncate_text(&turn.content, 120))
        .find(|text| !text.is_empty())
        .unwrap_or_default();
    let running = turns.iter().any(|turn| {
        turn.role == "assistant" && matches!(turn.status.as_str(), "pending" | "streaming")
    });
    let artifacts = &record.artifacts;
    let needs_attention = artifacts
        .flow_run_requests
        .iter()
        .any(|artifact| artifact["status"].as_str().unwrap_or("pending") == "pending")
        || artifacts.proposed_plans.iter().any(|artifact| {
            artifact["status"].as_str().unwrap_or("pending_review") == "pending_review"
        });
    let launched_run_ids: Vec<String> = artifacts
        .flow_launches
        .iter()
        .filter_map(|launch| launch["run_id"].as_str())
        .map(str::trim)
        .filter(|run_id| !run_id.is_empty())
        .map(str::to_string)
        .collect();
    let segments = &record.transcript.segments;
    let pending_question = |turn_id: &str| {
        segments.iter().any(|segment| {
            segment.turn_id == turn_id
                && segment.kind == "request_user_input"
                && segment.status == "pending"
        })
    };
    let updated_at = meta.updated_at.clone();
    write_rows(store, |tx, revision| {
        meta.revision = revision;
        // The seeded settings were written with the conversation.
        if snapshot.get("settings_revision").is_none() {
            meta.settings_revision = meta.revision;
        }
        tx.execute(
            "INSERT INTO conversations (id, project_path, handle, created_at, updated_at, \
             revision, title, preview, running, needs_attention, launched_run_ids, metadata) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                id,
                meta.project_path,
                meta.conversation_handle,
                meta.created_at,
                updated_at,
                revision,
                meta.title,
                preview,
                running,
                needs_attention,
                json!(launched_run_ids).to_string(),
                serde_json::to_value(&meta).expect("metadata").to_string(),
            ],
        )?;
        for (position, turn) in turns.iter().enumerate() {
            let unsettled = turn.role == "assistant"
                && (matches!(turn.status.as_str(), "pending" | "streaming" | "queued")
                    || pending_question(&turn.id));
            tx.execute(
                "INSERT INTO turns (conversation_id, turn_id, role, kind, position, settled, \
                 status, updated_at, revision, payload) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    id,
                    turn.id,
                    turn.role,
                    turn.kind.as_deref().unwrap_or("message"),
                    position as i64,
                    !unsettled,
                    turn.status,
                    updated_at,
                    revision,
                    serde_json::to_string(turn).expect("turn"),
                ],
            )?;
        }
        let mut next_order: HashMap<String, i64> = HashMap::new();
        for segment in segments {
            let mut segment = segment.clone();
            let next = next_order.entry(segment.turn_id.clone()).or_insert(1);
            if segment.order == ORDER_UNASSIGNED {
                segment.order = *next;
            }
            *next = (*next).max(segment.order + 1);
            let payload = serde_json::to_value(&segment).expect("segment");
            tx.execute(
                "INSERT INTO segments (conversation_id, segment_id, turn_id, kind, sort_order, \
                 status, updated_at, revision, payload) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    id,
                    segment.id,
                    segment.turn_id,
                    segment.kind,
                    segment.order,
                    segment.status,
                    updated_at,
                    revision,
                    payload.to_string(),
                ],
            )?;
            if let Some(output) = payload["tool_call"]["output_artifact"].as_str() {
                let blob = home_relative(store.home_dir(), &root.join(output));
                put_blob_reference(tx, revision, &id, "tool_output", &segment.id, &blob)?;
            }
        }
        let owner = |artifact_id: &str, artifact: &Value| {
            artifact["source_turn_id"]
                .as_str()
                .map(str::to_string)
                .or_else(|| {
                    segments
                        .iter()
                        .find(|segment| segment.artifact_id.as_deref() == Some(artifact_id))
                        .map(|segment| segment.turn_id.clone())
                })
                .filter(|turn_id| turns.iter().any(|turn| &turn.id == turn_id))
        };
        let collections = COLLECTIONS
            .iter()
            .map(|collection| (collection.key(), artifacts.collection(*collection)))
            .chain([("event_log", &artifacts.event_log)]);
        for (collection, entries) in collections {
            for (index, artifact) in entries.iter().enumerate() {
                let mut artifact = artifact.clone();
                let artifact_id = match artifact["id"].as_str() {
                    Some(artifact_id) if collection != "event_log" => artifact_id.to_string(),
                    _ => {
                        let artifact_id = format!("artifact-{collection}-{index}");
                        if collection != "event_log" {
                            artifact["id"] = json!(artifact_id);
                        }
                        artifact_id
                    }
                };
                tx.execute(
                    "INSERT INTO artifacts (conversation_id, collection, artifact_id, turn_id, \
                     position, status, updated_at, revision, payload) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        id,
                        collection,
                        artifact_id,
                        owner(&artifact_id, &artifact),
                        index as i64 + 1,
                        artifact["status"].as_str().unwrap_or_default(),
                        updated_at,
                        revision,
                        artifact.to_string(),
                    ],
                )?;
            }
        }
        Ok(())
    });
}

/// Run `apply` in one transaction on the conversation database file, with
/// a newly allocated revision. It writes behind the store's back, so no
/// commit notice is published.
pub fn write_rows<T>(
    store: &ConversationStore,
    apply: impl FnOnce(&Transaction<'_>, i64) -> rusqlite::Result<T>,
) -> T {
    let mut connection =
        rusqlite::Connection::open(store.database().path()).expect("open conversation database");
    connection
        .busy_timeout(std::time::Duration::from_secs(30))
        .expect("busy timeout");
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .expect("foreign keys");
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .expect("write transaction");
    let revision = tx
        .query_row(
            "UPDATE store_meta SET next_revision = next_revision + 1 \
             WHERE singleton = 1 RETURNING next_revision - 1",
            [],
            |row| row.get(0),
        )
        .expect("revision");
    let value = apply(&tx, revision).expect("write rows");
    tx.commit().expect("commit rows");
    value
}

/// A live reference from `owner_id` to bytes under the Spark home.
pub fn put_blob_reference(
    tx: &Transaction<'_>,
    revision: i64,
    conversation_id: &str,
    owner_kind: &str,
    owner_id: &str,
    blob_path: &str,
) -> rusqlite::Result<usize> {
    tx.execute(
        "INSERT INTO blob_references (conversation_id, owner_kind, owner_id, blob_path, revision) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![conversation_id, owner_kind, owner_id, blob_path, revision],
    )
}

fn home_relative(home: &Path, path: &Path) -> String {
    path.strip_prefix(home)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn truncate_text(value: &str, limit: usize) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= limit {
        return collapsed;
    }
    let mut truncated: String = collapsed.chars().take(limit.saturating_sub(1)).collect();
    truncated = truncated.trim_end().to_string();
    truncated.push('\u{2026}');
    truncated
}
