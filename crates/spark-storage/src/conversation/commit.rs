use std::collections::HashMap;
use std::path::Path;

use serde_json::{json, Value};
use time::OffsetDateTime;

use crate::error::{Result, StorageError};
use crate::ConversationRepository;

use super::database::{ArtifactRow, AttachmentRow, ConversationRow, SegmentRow, TurnRow};
use super::journal::{JournalEntry, JournalEntryKind};
use super::mutations::{ConversationMetadataPatch, ConversationMutation};
use super::projection::snapshot_from_record;
use super::provider_trace::{read_trace, replace_trace, sync_trace};
use super::records::{ConversationRecord, TranscriptSegment};
use super::store::{LoadedConversation, EVENT_LOG_COLLECTION};
use super::tool_output::{externalize_conversation_segment_tool_output, safe_tool_output_artifact};

/// The result of one committed conversation mutation batch.
#[derive(Debug, Clone, PartialEq)]
pub struct ConversationCommit {
    pub record: ConversationRecord,
    /// Stamped journal entries, in commit order. Publish these verbatim.
    pub journal_entries: Vec<JournalEntry>,
    /// The live wire payloads for each entry.
    pub journal_payloads: Vec<Value>,
    /// Legacy full-snapshot projection of the committed state.
    pub snapshot: Value,
    /// Final committed (per-conversation, legacy wire) revision.
    pub revision: i64,
    /// True when `base_revision` was stale and mutations were applied onto a
    /// newer committed state.
    pub rebased: bool,
}

impl ConversationRepository {
    /// The single conversation commit boundary.
    ///
    /// Under the project gate and conversation lock, applies identity-keyed
    /// mutations onto the loaded committed state (operational mutations
    /// rebase; stale settings updates conflict), writes only the changed rows
    /// and summary in one SQLite transaction, and publishes the new state to
    /// memory only after that transaction commits.
    pub fn commit_conversation(
        &self,
        conversation_id: &str,
        project_path: &str,
        base_revision: i64,
        mutations: Vec<ConversationMutation>,
    ) -> Result<ConversationCommit> {
        self.commit_conversation_projecting(
            conversation_id,
            project_path,
            base_revision,
            mutations,
            None,
        )
    }

    /// `commit_conversation` for a batch that projects every provider trace
    /// event appended for `projected_turn` so far: the turn's applied trace
    /// cursor advances in the same transaction. Once that turn is settled,
    /// its trace records are pruned behind the cleanup barrier.
    pub fn commit_conversation_projecting(
        &self,
        conversation_id: &str,
        project_path: &str,
        base_revision: i64,
        mutations: Vec<ConversationMutation>,
        projected_turn: Option<&str>,
    ) -> Result<ConversationCommit> {
        if mutations.is_empty() {
            return Err(commit_rejected(conversation_id, "No mutations to commit."));
        }
        let store = self.store().clone();
        let project_paths = store.registry().project_paths(project_path)?;
        let project_path = project_paths.project_path.clone();
        let gate = store.project_gate(&project_path);
        let _gate = gate.read().unwrap_or_else(|poison| poison.into_inner());
        store.admit_project(&project_path)?;
        let root = project_paths.conversations_dir.join(conversation_id);
        store.with_conversation(conversation_id, |slot| {
            let mut loaded = match slot.as_ref() {
                Some(existing) if existing.record.meta.project_path != project_path => {
                    return Err(commit_rejected(
                        conversation_id,
                        "Conversation is already bound to a different project path.",
                    ));
                }
                Some(existing) => existing.clone(),
                None => {
                    // A deleted conversation's id is never reused.
                    if store
                        .database()
                        .read(|read| read.conversation_location(conversation_id))?
                        .is_some()
                    {
                        return Err(commit_rejected(
                            conversation_id,
                            "conversation does not exist or was deleted",
                        ));
                    }
                    if base_revision != 0 {
                        return Err(commit_rejected(
                            conversation_id,
                            "Unknown conversation for a non-zero base revision.",
                        ));
                    }
                    store.registry().ensure_project_paths(&project_path)?;
                    LoadedConversation::new(conversation_id, &project_path)
                }
            };
            let is_new = slot.is_none();
            let latest_revision = loaded.record.meta.revision;
            let rebased = latest_revision != base_revision;
            if rebased
                && mutations.iter().any(|mutation| {
                    matches!(mutation,
                ConversationMutation::MetadataUpdated { patch } if patch.model_settings.is_some() || patch.chat_mode.is_some())
                })
            {
                return Err(StorageError::SettingsConflict {
                    path: root.join("conversation.json"),
                });
            }
            validate_segment_targets(conversation_id, &loaded.record, &mutations)?;
            validate_new_turn_attachments(store.home_dir(), conversation_id, &loaded, &mutations)?;

            let previous_outputs: HashMap<String, String> = loaded
                .record
                .transcript
                .segments
                .iter()
                .filter_map(|segment| Some((segment.id.clone(), tool_output_artifact(segment)?)))
                .collect();
            let now = iso_now();
            let mut entry_kinds = Vec::new();
            let mut snapshot_level_change = false;
            let mut changed_turns = Vec::new();
            let mut new_turns = Vec::new();
            let mut new_segment_attachments = Vec::new();
            let mut changed_segments = Vec::new();
            let mut tombstoned_segments = Vec::new();
            let mut changed_artifacts = Vec::new();
            let mut appended_events = Vec::new();
            for mutation in mutations {
                let record = &mut loaded.record;
                match mutation {
                    ConversationMutation::MetadataUpdated { patch } => {
                        apply_metadata_patch(record, patch);
                        snapshot_level_change = true;
                    }
                    ConversationMutation::TurnUpserted { mut turn } => {
                        let existing = record.transcript.find_turn(&turn.id);
                        keep_acceptance(
                            &mut turn.extra,
                            existing.map(|existing| &existing.extra),
                        );
                        // A queued turn moves to the end whenever it is written.
                        let moves = existing.is_none_or(|existing| existing.status == "queued");
                        if existing.is_none() {
                            new_turns.push(turn.clone());
                        }
                        if moves {
                            loaded.positions.insert(turn.id.clone(), loaded.next_position);
                            loaded.next_position += 1;
                        }
                        record.transcript.upsert_turn(turn.clone());
                        changed_turns.push(turn.id.clone());
                        entry_kinds.push(JournalEntryKind::TurnUpserted { turn });
                    }
                    ConversationMutation::SegmentUpserted { mut segment }
                    | ConversationMutation::RecoveredSegmentUpserted { mut segment, .. } => {
                        resolve_segment_order(record, &mut segment);
                        let existing = record.transcript.find_segment(&segment.id);
                        if existing.is_none() {
                            // A message accepted into a running turn owns its uploads.
                            new_segment_attachments.extend(
                                super::attachments::turn_attachment_ids(
                                    segment.extra.get("attachments"),
                                ),
                            );
                        }
                        keep_acceptance(&mut segment.extra, existing.map(|existing| &existing.extra));
                        // An echoed preview keeps referencing the stored output.
                        if let (Some(previous), Some(tool_call)) = (
                            previous_outputs.get(&segment.id),
                            segment.tool_call.as_mut().and_then(Value::as_object_mut),
                        ) {
                            if tool_call.get("output_truncated") == Some(&json!(true))
                                && !tool_call.contains_key("output_artifact")
                            {
                                tool_call.insert("output_artifact".into(), json!(previous));
                            }
                        }
                        externalize_conversation_segment_tool_output(&root, &mut segment)?;
                        record.transcript.upsert_segment(segment.clone());
                        changed_segments.push(segment.id.clone());
                        entry_kinds.push(JournalEntryKind::SegmentUpserted { segment });
                    }
                    ConversationMutation::SegmentTombstoned {
                        turn_id,
                        segment_id,
                    } => {
                        record.transcript.remove_segment(&segment_id);
                        tombstoned_segments.push(segment_id.clone());
                        entry_kinds.push(JournalEntryKind::SegmentTombstoned {
                            turn_id,
                            segment_id,
                        });
                    }
                    ConversationMutation::ArtifactUpserted {
                        collection,
                        mut artifact,
                    } => {
                        if !artifact.is_object() {
                            return Err(commit_rejected(
                                conversation_id,
                                "Artifact records must be JSON objects.",
                            ));
                        }
                        // Every artifact row needs an identity.
                        let id = match artifact.get("id").and_then(Value::as_str) {
                            Some(id) if !id.is_empty() => id.to_string(),
                            _ => {
                                let id = format!("artifact-{}", uuid::Uuid::new_v4().simple());
                                artifact["id"] = json!(id);
                                id
                            }
                        };
                        upsert_artifact(record.artifacts.collection_mut(collection), artifact);
                        changed_artifacts.push((collection, id));
                        snapshot_level_change = true;
                    }
                    ConversationMutation::WorkflowEventAppended { event } => {
                        record.artifacts.event_log.push(event.clone());
                        appended_events.push(event);
                        snapshot_level_change = true;
                    }
                }
            }
            if snapshot_level_change {
                entry_kinds.push(JournalEntryKind::SnapshotCommitted);
            }

            let record = &mut loaded.record;
            maintain_metadata(record, &now);
            if is_new || record.meta.conversation_handle.is_empty() {
                let preferred = store.reserved_handle(conversation_id).or_else(|| {
                    (!record.meta.conversation_handle.is_empty())
                        .then(|| record.meta.conversation_handle.clone())
                });
                record.meta.conversation_handle = crate::workspace_conversations::free_handle(
                    &store,
                    conversation_id,
                    preferred.as_deref(),
                )?;
            }
            let final_revision = latest_revision + entry_kinds.len() as i64;
            record.meta.revision = final_revision;
            let journal_entries: Vec<JournalEntry> = entry_kinds
                .into_iter()
                .enumerate()
                .map(|(index, kind)| JournalEntry {
                    revision: latest_revision + 1 + index as i64,
                    committed_at: now.clone(),
                    kind,
                })
                .collect();
            let snapshot = snapshot_from_record(record);
            let journal_payloads = journal_entries
                .iter()
                .map(|entry| match entry.kind {
                    JournalEntryKind::SnapshotCommitted => {
                        entry.live_event_payload(&record.meta, &snapshot)
                    }
                    _ => entry.journal_line_payload(&record.meta),
                })
                .collect();

            let newly_attached: Vec<String> = new_turns
                .iter()
                .flat_map(|turn| super::attachments::turn_attachment_ids(turn.extra.get("attachments")))
                .chain(new_segment_attachments)
                .collect();
            let projected = projected_turn.and_then(|turn_id| {
                let sequence = *loaded.trace_last_by_turn.get(turn_id)?;
                let settled = loaded
                    .record
                    .transcript
                    .find_turn(turn_id)
                    .is_some_and(|turn| !is_active_status(&turn.status));
                Some((turn_id.to_string(), sequence, settled))
            });
            let allocated = loaded.next_trace_sequence.saturating_sub(1);
            if projected.is_some() {
                // The applied cursor may only cover durable trace records.
                sync_trace(&root)?;
            }
            let conversation_row = summary_row(conversation_id, &loaded.record)?;
            let loaded_ref = &loaded;
            store.database().write(|tx| {
                tx.put_conversation(&conversation_row)?;
                for turn_id in dedup(&changed_turns) {
                    let Some(turn) = loaded_ref.record.transcript.find_turn(turn_id) else {
                        continue;
                    };
                    tx.put_turn(&TurnRow {
                        conversation_id: conversation_id.to_string(),
                        id: turn_id.clone(),
                        position: loaded_ref.positions[turn_id],
                        status: turn.status.clone(),
                        updated_at: now.clone(),
                        payload: to_object(turn)?,
                    })?;
                }
                for segment_id in dedup(&tombstoned_segments) {
                    tx.delete_segment(conversation_id, segment_id)?;
                }
                for segment_id in dedup(&changed_segments) {
                    let Some(segment) = loaded_ref.record.transcript.find_segment(segment_id)
                    else {
                        continue;
                    };
                    tx.put_segment(&SegmentRow {
                        conversation_id: conversation_id.to_string(),
                        id: segment.id.clone(),
                        turn_id: segment.turn_id.clone(),
                        order: segment.order,
                        status: segment.status.clone(),
                        updated_at: now.clone(),
                        payload: to_object(segment)?,
                    })?;
                    let current = tool_output_artifact(segment);
                    let previous = previous_outputs.get(segment_id);
                    if let Some(previous) = previous.filter(|previous| Some(*previous) != current.as_ref()) {
                        tx.delete_blob_reference(
                            conversation_id,
                            "tool_output",
                            segment_id,
                            &store.home_relative(&root.join(previous)),
                        )?;
                    }
                    if let Some(current) = current {
                        tx.put_blob_reference(
                            conversation_id,
                            "tool_output",
                            segment_id,
                            &store.home_relative(&root.join(current)),
                        )?;
                    }
                }
                for (collection, id) in dedup(&changed_artifacts) {
                    let Some(artifact) = loaded_ref
                        .record
                        .artifacts
                        .collection(*collection)
                        .iter()
                        .find(|artifact| artifact.get("id").and_then(Value::as_str) == Some(id))
                    else {
                        continue;
                    };
                    tx.put_artifact(&ArtifactRow {
                        conversation_id: conversation_id.to_string(),
                        collection: collection.key().to_string(),
                        id: id.clone(),
                        turn_id: artifact
                            .get("source_turn_id")
                            .and_then(Value::as_str)
                            .filter(|turn_id| loaded_ref.positions.contains_key(*turn_id))
                            .map(str::to_string),
                        status: artifact
                            .get("status")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        updated_at: now.clone(),
                        payload: artifact.clone(),
                    })?;
                }
                for event in &appended_events {
                    tx.put_artifact(&ArtifactRow {
                        conversation_id: conversation_id.to_string(),
                        collection: EVENT_LOG_COLLECTION.to_string(),
                        id: format!("event-{}", uuid::Uuid::new_v4().simple()),
                        turn_id: None,
                        status: String::new(),
                        updated_at: now.clone(),
                        payload: event.clone(),
                    })?;
                }
                for id in &newly_attached {
                    let attachment = &loaded_ref.attachments[id];
                    tx.put_attachment(&AttachmentRow {
                        conversation_id: conversation_id.to_string(),
                        id: id.clone(),
                        blob_path: attachment.blob_path.clone(),
                        status: "attached".to_string(),
                        updated_at: now.clone(),
                        payload: to_object(&attachment.record)?,
                    })?;
                }
                if let Some((turn_id, sequence, settled)) = &projected {
                    tx.set_applied_trace(conversation_id, turn_id, *sequence)?;
                    if *settled {
                        // Pruning may drop any allocated record; stay above all of them.
                        tx.raise_trace_high_water(conversation_id, allocated)?;
                    }
                }
                Ok(())
            })?;

            for id in &newly_attached {
                if let Some(attachment) = loaded.attachments.get_mut(id) {
                    attachment.attached = true;
                }
            }
            if let Some((turn_id, sequence, _)) = &projected {
                let applied = loaded.applied_trace.entry(turn_id.clone()).or_default();
                *applied = (*applied).max(*sequence);
            }
            let commit = ConversationCommit {
                record: loaded.record.clone(),
                journal_entries,
                journal_payloads,
                snapshot,
                revision: final_revision,
                rebased,
            };
            let prune = projected.as_ref().is_some_and(|(_, _, settled)| *settled);
            if is_new {
                store.release_handle(conversation_id);
            }
            *slot = Some(loaded);
            if prune {
                if let Some(loaded) = slot.as_ref() {
                    prune_settled_trace(&store, &root, loaded);
                }
            }
            Ok(commit)
        })
    }
}

impl ConversationRepository {
    /// Test fixtures: store a complete record's rows exactly as given
    /// (revision, timestamps, handle), replacing any loaded state.
    #[doc(hidden)]
    pub fn seed_record(&self, record: &ConversationRecord) -> Result<()> {
        let store = self.store().clone();
        let mut record = record.clone();
        let conversation_id = record.meta.conversation_id.clone();
        record.meta.project_path = store
            .registry()
            .ensure_project_paths(&record.meta.project_path)?
            .project_path;
        if record.meta.conversation_handle.is_empty() {
            record.meta.conversation_handle =
                crate::workspace_conversations::free_handle(&store, &conversation_id, None)?;
        }
        for collection in super::store::ALL_COLLECTIONS {
            for artifact in record.artifacts.collection_mut(collection) {
                if artifact.get("id").and_then(Value::as_str).is_none() {
                    artifact["id"] = json!(format!("artifact-{}", uuid::Uuid::new_v4().simple()));
                }
            }
        }
        let mut loaded = LoadedConversation::new(&conversation_id, &record.meta.project_path);
        for (position, turn) in record.transcript.turns.iter().enumerate() {
            loaded.positions.insert(turn.id.clone(), position as i64);
        }
        loaded.next_position = record.transcript.turns.len() as i64;
        loaded.record = record;
        let row = summary_row(&conversation_id, &loaded.record)?;
        store.with_conversation(&conversation_id, |slot| {
            let record = &loaded.record;
            store.database().write(|tx| {
                tx.put_conversation(&row)?;
                for turn in &record.transcript.turns {
                    tx.put_turn(&TurnRow {
                        conversation_id: conversation_id.clone(),
                        id: turn.id.clone(),
                        position: loaded.positions[&turn.id],
                        status: turn.status.clone(),
                        updated_at: record.meta.updated_at.clone(),
                        payload: to_object(turn)?,
                    })?;
                }
                for segment in &record.transcript.segments {
                    tx.put_segment(&SegmentRow {
                        conversation_id: conversation_id.clone(),
                        id: segment.id.clone(),
                        turn_id: segment.turn_id.clone(),
                        order: segment.order,
                        status: segment.status.clone(),
                        updated_at: record.meta.updated_at.clone(),
                        payload: to_object(segment)?,
                    })?;
                }
                for collection in super::store::ALL_COLLECTIONS {
                    for artifact in record.artifacts.collection(collection) {
                        tx.put_artifact(&ArtifactRow {
                            conversation_id: conversation_id.clone(),
                            collection: collection.key().to_string(),
                            id: artifact["id"].as_str().unwrap_or_default().to_string(),
                            turn_id: None,
                            status: artifact["status"].as_str().unwrap_or_default().to_string(),
                            updated_at: record.meta.updated_at.clone(),
                            payload: artifact.clone(),
                        })?;
                    }
                }
                for event in &record.artifacts.event_log {
                    tx.put_artifact(&ArtifactRow {
                        conversation_id: conversation_id.clone(),
                        collection: EVENT_LOG_COLLECTION.to_string(),
                        id: format!("event-{}", uuid::Uuid::new_v4().simple()),
                        turn_id: None,
                        status: String::new(),
                        updated_at: record.meta.updated_at.clone(),
                        payload: event.clone(),
                    })?;
                }
                Ok(())
            })?;
            *slot = Some(loaded);
            Ok(())
        })
    }
}

/// Drop trace records no unsettled turn still needs, once the settling
/// commit is checkpointed: without the barrier, power loss could restore an
/// older active turn whose trace was already removed. Failure leaves
/// ignorable records behind.
fn prune_settled_trace(store: &super::ConversationStore, root: &Path, loaded: &LoadedConversation) {
    let result = store.database().after_checkpoint(|_| {
        let retained: Vec<_> = read_trace(root)?
            .into_iter()
            .filter(|record| {
                loaded
                    .record
                    .transcript
                    .find_turn(&record.turn_id)
                    .is_some_and(|turn| is_active_status(&turn.status))
                    && record.sequence
                        > loaded
                            .applied_trace
                            .get(&record.turn_id)
                            .copied()
                            .unwrap_or(0)
            })
            .collect();
        replace_trace(root, &retained)
    });
    if let Err(error) = result {
        eprintln!(
            "Deferred provider trace pruning for {}: {error}",
            root.display()
        );
    }
}

pub(crate) fn is_active_status(status: &str) -> bool {
    matches!(status, "pending" | "streaming")
}

/// Metadata and summary columns, derived from the committed record.
fn summary_row(conversation_id: &str, record: &ConversationRecord) -> Result<ConversationRow> {
    let preview = record
        .transcript
        .turns
        .iter()
        .rev()
        .filter(|turn| turn.turn_kind() == "message")
        .map(|turn| truncate_text(&turn.content, 120))
        .find(|preview| !preview.is_empty())
        .unwrap_or_default();
    let running = record
        .transcript
        .turns
        .iter()
        .any(|turn| turn.role == "assistant" && is_active_status(&turn.status));
    let pending = |artifacts: &[Value], status: &str| {
        artifacts
            .iter()
            .any(|artifact| artifact.get("status").and_then(Value::as_str) == Some(status))
    };
    let needs_attention = pending(&record.artifacts.flow_run_requests, "pending")
        || pending(&record.artifacts.proposed_plans, "pending_review");
    let launched_run_ids = record
        .artifacts
        .flow_launches
        .iter()
        .filter_map(|launch| launch.get("run_id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|run_id| !run_id.is_empty())
        .map(str::to_string)
        .collect();
    Ok(ConversationRow {
        id: conversation_id.to_string(),
        project_path: record.meta.project_path.clone(),
        handle: record.meta.conversation_handle.clone(),
        created_at: record.meta.created_at.clone(),
        updated_at: record.meta.updated_at.clone(),
        title: record.meta.title.clone(),
        preview,
        running,
        needs_attention,
        launched_run_ids,
        metadata: to_object(&record.meta)?,
    })
}

fn to_object(value: &impl serde::Serialize) -> Result<Value> {
    serde_json::to_value(value).map_err(|source| StorageError::JsonWrite {
        path: std::path::PathBuf::from(super::CONVERSATION_DATABASE_FILE_NAME),
        source,
    })
}

fn dedup<T: PartialEq>(items: &[T]) -> Vec<&T> {
    let mut unique: Vec<&T> = Vec::with_capacity(items.len());
    for item in items {
        if !unique.contains(&item) {
            unique.push(item);
        }
    }
    unique
}

/// What an accepted message recorded: the fingerprint of the request its
/// client turn id was accepted for, and the uploads it owns.
const ACCEPTANCE_KEYS: [&str; 2] = ["client_request_sha256", "attachments"];

/// An accepted message's record never changes, even when a later upsert
/// (such as a provider's delivery report) rebuilds the entity without it.
fn keep_acceptance(
    extra: &mut serde_json::Map<String, Value>,
    existing: Option<&serde_json::Map<String, Value>>,
) {
    for key in ACCEPTANCE_KEYS {
        if let Some(accepted) = existing.and_then(|existing| existing.get(key)) {
            extra.insert(key.to_string(), accepted.clone());
        }
    }
}

fn tool_output_artifact(segment: &TranscriptSegment) -> Option<String> {
    segment
        .tool_call
        .as_ref()?
        .get("output_artifact")?
        .as_str()
        .and_then(safe_tool_output_artifact)
        .map(str::to_string)
}

/// A new turn, or a new message segment accepted into a running turn, may
/// only start referencing attachments that are stored and readable at
/// commit time.
fn validate_new_turn_attachments(
    home_dir: &Path,
    conversation_id: &str,
    loaded: &LoadedConversation,
    mutations: &[ConversationMutation],
) -> Result<()> {
    for mutation in mutations {
        let references = match mutation {
            ConversationMutation::TurnUpserted { turn }
                if loaded.record.transcript.find_turn(&turn.id).is_none() =>
            {
                turn.extra.get("attachments")
            }
            ConversationMutation::SegmentUpserted { segment }
            | ConversationMutation::RecoveredSegmentUpserted { segment, .. }
                if loaded.record.transcript.find_segment(&segment.id).is_none() =>
            {
                segment.extra.get("attachments")
            }
            _ => continue,
        };
        for id in super::attachments::turn_attachment_ids(references) {
            let readable = loaded
                .attachments
                .get(&id)
                .is_some_and(|attachment| attachment.readable(home_dir));
            if !readable {
                return Err(commit_rejected(
                    conversation_id,
                    &format!("Attachment {id} is not available in this conversation. Remove it and attach the file again."),
                ));
            }
        }
    }
    Ok(())
}

fn validate_segment_targets(
    conversation_id: &str,
    record: &ConversationRecord,
    mutations: &[ConversationMutation],
) -> Result<()> {
    for mutation in mutations {
        let segment = match mutation {
            ConversationMutation::SegmentUpserted { segment }
            | ConversationMutation::RecoveredSegmentUpserted { segment, .. } => segment,
            _ => continue,
        };
        let turn_known = record.transcript.find_turn(&segment.turn_id).is_some()
            || mutations.iter().any(|candidate| {
                matches!(candidate, ConversationMutation::TurnUpserted { turn } if turn.id == segment.turn_id)
            });
        if !turn_known {
            return Err(commit_rejected(
                conversation_id,
                &format!(
                    "Segment {} targets unknown turn {}.",
                    segment.id, segment.turn_id
                ),
            ));
        }
    }
    Ok(())
}

fn resolve_segment_order(record: &ConversationRecord, segment: &mut TranscriptSegment) {
    if !segment.has_unassigned_order() {
        return;
    }
    if let Some(existing) = record.transcript.find_segment(&segment.id) {
        segment.order = existing.order;
        return;
    }
    segment.order = record.transcript.next_segment_order(&segment.turn_id);
}

fn apply_metadata_patch(record: &mut ConversationRecord, patch: ConversationMetadataPatch) {
    let meta = &mut record.meta;
    if let Some(group) = patch.model_settings {
        meta.model_settings = group;
    }
    if let Some(chat_mode) = patch.chat_mode {
        meta.chat_mode = chat_mode;
    }
    if let Some(provider) = patch.provider {
        meta.provider = provider;
    }
    if let Some(model) = patch.model {
        meta.model = model;
    }
    if let Some(llm_profile) = patch.llm_profile {
        meta.llm_profile = llm_profile;
    }
    if let Some(reasoning_effort) = patch.reasoning_effort {
        meta.reasoning_effort = reasoning_effort;
    }
    if let Some(title) = patch.title {
        meta.title = title;
    }
}

fn maintain_metadata(record: &mut ConversationRecord, now: &str) {
    if record.meta.created_at.trim().is_empty() {
        record.meta.created_at = now.to_string();
    }
    record.meta.updated_at = now.to_string();
    let title = record.meta.title.trim();
    if title.is_empty() || title == "New thread" {
        record.meta.title = derive_title(record);
    }
}

fn derive_title(record: &ConversationRecord) -> String {
    record
        .transcript
        .turns
        .iter()
        .find_map(|turn| {
            (turn.turn_kind() == "message" && turn.role == "user")
                .then(|| truncate_text(&turn.content, 64))
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "New thread".to_string())
}

fn truncate_text(value: &str, limit: usize) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= limit {
        return collapsed;
    }
    let mut truncated = collapsed
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>();
    truncated = truncated.trim_end().to_string();
    truncated.push('\u{2026}');
    truncated
}

fn upsert_artifact(collection: &mut Vec<Value>, artifact: Value) {
    let artifact_id = artifact
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if let Some(existing) = collection
        .iter_mut()
        .find(|existing| existing.get("id").and_then(Value::as_str) == Some(artifact_id.as_str()))
    {
        *existing = artifact;
    } else {
        collection.push(artifact);
    }
}

fn commit_rejected(conversation_id: &str, reason: &str) -> StorageError {
    StorageError::ConversationCommitRejected {
        conversation_id: conversation_id.to_string(),
        reason: reason.to_string(),
    }
}

pub(crate) fn iso_now() -> String {
    let now = OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}
