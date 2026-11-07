//! Test fixture: stores one conversation's rows exactly as a snapshot gives
//! them (ids, timestamps, statuses), through the database's low-level write.
//! Contract tests only: Spark itself changes conversation state through the
//! repository's typed operations. Loaded with `#[path]` by the test binaries
//! that seed fixtures.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};
use spark_storage::conversation::{
    record_from_snapshot, ArtifactCollection, ArtifactRow, ConversationRow, SegmentRow, TurnRow,
    ORDER_UNASSIGNED,
};
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
    store
        .database()
        .write(|tx| {
            meta.revision = tx.revision();
            // The seeded settings were written with the conversation.
            if snapshot.get("settings_revision").is_none() {
                meta.settings_revision = meta.revision;
            }
            tx.put_conversation(&ConversationRow {
                id: id.clone(),
                project_path: meta.project_path.clone(),
                handle: meta.conversation_handle.clone(),
                created_at: meta.created_at.clone(),
                updated_at: updated_at.clone(),
                title: meta.title.clone(),
                preview: preview.clone(),
                running,
                needs_attention,
                launched_run_ids: launched_run_ids.clone(),
                metadata: serde_json::to_value(&meta).expect("metadata"),
            })?;
            for (position, turn) in turns.iter().enumerate() {
                let unsettled = turn.role == "assistant"
                    && (matches!(turn.status.as_str(), "pending" | "streaming" | "queued")
                        || pending_question(&turn.id));
                tx.put_turn(&TurnRow {
                    conversation_id: id.clone(),
                    id: turn.id.clone(),
                    role: turn.role.clone(),
                    kind: turn.kind.clone().unwrap_or_else(|| "message".to_string()),
                    position: position as i64,
                    settled: !unsettled,
                    status: turn.status.clone(),
                    updated_at: updated_at.clone(),
                    payload: serde_json::to_value(turn).expect("turn"),
                })?;
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
                tx.put_segment(&SegmentRow {
                    conversation_id: id.clone(),
                    id: segment.id.clone(),
                    turn_id: segment.turn_id.clone(),
                    kind: segment.kind.clone(),
                    order: segment.order,
                    status: segment.status.clone(),
                    updated_at: updated_at.clone(),
                    payload: payload.clone(),
                })?;
                if let Some(output) = payload["tool_call"]["output_artifact"].as_str() {
                    let blob = root.join(output);
                    tx.put_blob_reference(
                        &id,
                        "tool_output",
                        &segment.id,
                        &home_relative(store.home_dir(), &blob),
                    )?;
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
                    tx.put_artifact(&ArtifactRow {
                        conversation_id: id.clone(),
                        collection: collection.to_string(),
                        id: artifact_id.clone(),
                        turn_id: owner(&artifact_id, &artifact),
                        status: artifact["status"].as_str().unwrap_or_default().to_string(),
                        updated_at: updated_at.clone(),
                        payload: artifact,
                    })?;
                }
            }
            Ok(())
        })
        .expect("seed conversation");
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
