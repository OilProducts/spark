use std::path::Path;

use serde_json::{json, Value};
use time::OffsetDateTime;

use crate::error::{Result, StorageError};
use crate::ConversationRepository;

use super::database::{
    ArtifactRow, AttachmentRow, ChangeBatch, ConversationRow, SegmentRow, TurnRow,
};
use super::loaded::{is_active_status, LoadedConversation, StoredArtifact};
use super::mutations::{ConversationMetadataPatch, ConversationMutation};
use super::provider_trace::{read_trace, replace_trace, sync_trace};
use super::records::{ConversationMeta, ConversationRecord, TranscriptSegment, TranscriptTurn};
use super::store::EVENT_LOG_COLLECTION;
use super::tool_output::{externalize_conversation_segment_tool_output, safe_tool_output_artifact};

/// The result of one committed conversation mutation batch: its revision and
/// the rows it changed, never the conversation's full state.
#[derive(Debug, Clone, PartialEq)]
pub struct ConversationCommit {
    /// The transaction's home-global revision, now the conversation's.
    pub revision: i64,
    /// Every row this commit changed, at `revision`.
    pub changes: ChangeBatch,
    /// True when `base_revision` was stale and mutations were applied onto a
    /// newer committed state.
    pub rebased: bool,
}

/// What applying a mutation replaced, to restore if its transaction fails.
enum Undo {
    Turn(String, Option<(i64, TranscriptTurn)>),
    Segment(String, Option<TranscriptSegment>),
    Artifact(String, String, Option<StoredArtifact>),
}

impl ConversationRepository {
    /// The single conversation commit boundary.
    ///
    /// Under the project gate and conversation lock, applies identity-keyed
    /// mutations onto the loaded committed state (operational mutations
    /// rebase; stale settings updates conflict), writes only the changed rows
    /// and summary in one SQLite transaction, and keeps the new state in
    /// memory only once that transaction commits; a failed transaction
    /// restores exactly the entities it replaced.
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
        let commit = store.with_conversation(conversation_id, |slot| {
            if slot
                .as_ref()
                .is_some_and(|existing| existing.meta.project_path != project_path)
            {
                return Err(commit_rejected(
                    conversation_id,
                    "Conversation is already bound to a different project path.",
                ));
            }
            let is_new = slot.is_none();
            if is_new {
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
                *slot = Some(LoadedConversation::new(conversation_id, &project_path));
            }
            let loaded = slot.as_mut().expect("loaded above");
            let rebased = loaded.revision != base_revision;
            // Settings conflict only with a committed change made after the
            // caller read; draft uploads and summary touches commit none.
            if loaded.meta.revision > base_revision
                && mutations.iter().any(|mutation| {
                    matches!(mutation,
                ConversationMutation::MetadataUpdated { patch } if patch.model_settings.is_some() || patch.chat_mode.is_some())
                })
            {
                if is_new {
                    *slot = None;
                }
                return Err(StorageError::SettingsConflict {
                    path: root.join("conversation.json"),
                });
            }
            let previous_meta = loaded.meta.clone();
            let previous_next_position = loaded.next_position;
            let mut undo = Vec::new();
            let applied = apply_mutations(
                &store,
                conversation_id,
                &root,
                loaded,
                mutations,
                &mut undo,
            )
            .and_then(|changed| {
                write_changes(
                    &store,
                    conversation_id,
                    &root,
                    loaded,
                    &changed,
                    projected_turn,
                )
                .map(|written| (changed, written))
            });
            let ((changed, (revision, rows, projected)), loaded) = match applied {
                Ok(applied) => (applied, loaded),
                Err(error) => {
                    // Readers never saw the staged state: restore it.
                    for entry in undo.into_iter().rev() {
                        match entry {
                            Undo::Turn(_, Some((position, turn))) => {
                                loaded.put_turn(turn, position);
                            }
                            Undo::Turn(id, None) => {
                                loaded.remove_turn(&id);
                            }
                            Undo::Segment(_, Some(segment)) => {
                                loaded.put_segment(segment);
                            }
                            Undo::Segment(id, None) => {
                                loaded.remove_segment(&id);
                            }
                            Undo::Artifact(collection, id, Some(artifact)) => {
                                loaded.put_artifact(&collection, &id, artifact);
                            }
                            Undo::Artifact(collection, id, None) => {
                                loaded.remove_artifact(&collection, &id);
                            }
                        }
                    }
                    loaded.meta = previous_meta;
                    loaded.next_position = previous_next_position;
                    if is_new {
                        *slot = None;
                    }
                    return Err(error);
                }
            };
            loaded.revision = revision;
            loaded.meta.revision = revision;
            for id in &changed.newly_attached {
                if let Some(attachment) = loaded.attachments.get_mut(id) {
                    attachment.attached = true;
                }
            }
            if let Some((turn_id, sequence, _)) = &projected {
                let applied = loaded.applied_trace.entry(turn_id.clone()).or_default();
                *applied = (*applied).max(*sequence);
            }
            if is_new {
                store.release_handle(conversation_id);
            }
            if projected.as_ref().is_some_and(|(_, _, settled)| *settled) {
                prune_settled_trace(&store, &root, loaded);
            }
            Ok(ConversationCommit {
                revision,
                changes: ChangeBatch {
                    from_revision: revision - 1,
                    watermark: revision,
                    rows,
                },
                rebased,
            })
        })?;
        store.notify_commit(
            commit.revision,
            vec![(conversation_id.to_string(), project_path)],
        );
        Ok(commit)
    }
}

/// The entities one batch changed, by identity.
#[derive(Default)]
struct Changed {
    turns: Vec<String>,
    segments: Vec<String>,
    tombstoned_segments: Vec<String>,
    artifacts: Vec<(String, String)>,
    /// (segment id, previous tool-output artifact), for reference changes.
    previous_outputs: Vec<(String, Option<String>)>,
    newly_attached: Vec<String>,
}

fn apply_mutations(
    store: &super::ConversationStore,
    conversation_id: &str,
    root: &Path,
    loaded: &mut LoadedConversation,
    mutations: Vec<ConversationMutation>,
    undo: &mut Vec<Undo>,
) -> Result<Changed> {
    validate_segment_targets(conversation_id, loaded, &mutations)?;
    validate_new_turn_attachments(store.home_dir(), conversation_id, loaded, &mutations)?;
    let now = iso_now();
    let mut changed = Changed::default();
    for mutation in mutations {
        match mutation {
            ConversationMutation::MetadataUpdated { patch } => {
                apply_metadata_patch(&mut loaded.meta, patch);
            }
            ConversationMutation::TurnUpserted { mut turn } => {
                let existing = loaded.turn(&turn.id);
                keep_acceptance(&mut turn.extra, existing.map(|existing| &existing.extra));
                if existing.is_none() {
                    changed
                        .newly_attached
                        .extend(super::attachments::turn_attachment_ids(
                            turn.extra.get("attachments"),
                        ));
                }
                // A queued turn moves to the end whenever it is written.
                let position = match existing {
                    Some(existing) if existing.status != "queued" => {
                        loaded.position(&turn.id).expect("indexed turn")
                    }
                    _ => loaded.next_position,
                };
                let id = turn.id.clone();
                let previous = loaded.put_turn(turn, position);
                undo.push(Undo::Turn(id.clone(), previous));
                changed.turns.push(id);
            }
            ConversationMutation::SegmentUpserted { mut segment }
            | ConversationMutation::RecoveredSegmentUpserted { mut segment, .. } => {
                let existing = loaded.segment(&segment.id);
                if segment.has_unassigned_order() {
                    segment.order = existing.map_or_else(
                        || loaded.next_segment_order(&segment.turn_id),
                        |existing| existing.order,
                    );
                }
                if existing.is_none() {
                    // A message accepted into a running turn owns its uploads.
                    changed
                        .newly_attached
                        .extend(super::attachments::turn_attachment_ids(
                            segment.extra.get("attachments"),
                        ));
                }
                keep_acceptance(&mut segment.extra, existing.map(|existing| &existing.extra));
                let previous_output = existing.and_then(tool_output_artifact);
                // An echoed preview keeps referencing the stored output.
                if let (Some(previous), Some(tool_call)) = (
                    previous_output.as_ref(),
                    segment.tool_call.as_mut().and_then(Value::as_object_mut),
                ) {
                    if tool_call.get("output_truncated") == Some(&json!(true))
                        && !tool_call.contains_key("output_artifact")
                    {
                        tool_call.insert("output_artifact".into(), json!(previous));
                    }
                }
                externalize_conversation_segment_tool_output(root, &mut segment)?;
                let id = segment.id.clone();
                if !changed
                    .previous_outputs
                    .iter()
                    .any(|(segment_id, _)| segment_id == &id)
                {
                    changed.previous_outputs.push((id.clone(), previous_output));
                }
                // The artifact this segment anchors may now have an owner.
                if let Some(artifact_id) = segment.artifact_id.clone() {
                    for collection in super::store::ALL_COLLECTIONS {
                        if loaded
                            .artifact(collection.key(), &artifact_id)
                            .is_some_and(|artifact| artifact.owner.is_none())
                        {
                            changed
                                .artifacts
                                .push((collection.key().to_string(), artifact_id.clone()));
                        }
                    }
                }
                let previous = loaded.put_segment(segment);
                undo.push(Undo::Segment(id.clone(), previous));
                changed
                    .tombstoned_segments
                    .retain(|segment_id| segment_id != &id);
                changed.segments.push(id);
            }
            ConversationMutation::SegmentTombstoned { segment_id, .. } => {
                let previous = loaded.remove_segment(&segment_id);
                undo.push(Undo::Segment(segment_id.clone(), previous));
                changed.segments.retain(|id| id != &segment_id);
                changed.tombstoned_segments.push(segment_id);
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
                let collection = collection.key().to_string();
                let (position, owner) = match loaded.artifact(&collection, &id) {
                    Some(existing) => (existing.position, existing.owner.clone()),
                    None => (loaded.next_artifact_position(&collection), None),
                };
                let previous = loaded.put_artifact(
                    &collection,
                    &id,
                    StoredArtifact {
                        position,
                        owner,
                        payload: artifact,
                    },
                );
                undo.push(Undo::Artifact(collection.clone(), id.clone(), previous));
                changed.artifacts.push((collection, id));
            }
            ConversationMutation::WorkflowEventAppended { event } => {
                let id = format!("event-{}", uuid::Uuid::new_v4().simple());
                let position = loaded.next_artifact_position(EVENT_LOG_COLLECTION);
                let previous = loaded.put_artifact(
                    EVENT_LOG_COLLECTION,
                    &id,
                    StoredArtifact {
                        position,
                        owner: None,
                        payload: event,
                    },
                );
                undo.push(Undo::Artifact(
                    EVENT_LOG_COLLECTION.into(),
                    id.clone(),
                    previous,
                ));
                changed
                    .artifacts
                    .push((EVENT_LOG_COLLECTION.to_string(), id));
            }
        }
    }
    // Owners resolve once every turn and anchor of the batch is in place.
    for (collection, id) in dedup(&changed.artifacts) {
        let Some(artifact) = loaded.artifact(collection, id) else {
            continue;
        };
        if artifact.owner.is_some() || collection == EVENT_LOG_COLLECTION {
            continue;
        }
        let owner = artifact
            .payload
            .get("source_turn_id")
            .and_then(Value::as_str)
            .or_else(|| loaded.anchor_turn(id))
            .filter(|turn_id| loaded.turn(turn_id).is_some())
            .map(str::to_string);
        if owner.is_some() {
            let mut artifact = artifact.clone();
            artifact.owner = owner;
            let previous = loaded.put_artifact(collection, id, artifact);
            undo.push(Undo::Artifact(collection.clone(), id.clone(), previous));
        }
    }
    maintain_metadata(loaded, &now);
    if loaded.meta.conversation_handle.is_empty() {
        let preferred = store.reserved_handle(conversation_id);
        loaded.meta.conversation_handle = crate::workspace_conversations::free_handle(
            store,
            conversation_id,
            preferred.as_deref(),
        )?;
    }
    Ok(changed)
}

/// One transaction writing the changed rows, their references and the
/// summary. Returns its revision, the changed rows, and the projected trace
/// cursor: (turn id, sequence, settled).
fn write_changes(
    store: &super::ConversationStore,
    conversation_id: &str,
    root: &Path,
    loaded: &LoadedConversation,
    changed: &Changed,
    projected_turn: Option<&str>,
) -> Result<(i64, Vec<super::EntityRow>, Option<(String, u64, bool)>)> {
    let now = loaded.meta.updated_at.clone();
    let projected = projected_turn.and_then(|turn_id| {
        let sequence = *loaded.trace_last_by_turn.get(turn_id)?;
        let settled = loaded
            .turn(turn_id)
            .is_some_and(|turn| !is_active_status(&turn.status));
        Some((turn_id.to_string(), sequence, settled))
    });
    let allocated = loaded.next_trace_sequence.saturating_sub(1);
    if projected.is_some() {
        // The applied cursor may only cover durable trace records.
        sync_trace(root)?;
    }
    let conversation_row = summary_row(conversation_id, loaded)?;
    let (revision, rows) = store.database().write(|tx| {
        let mut conversation_row = conversation_row.clone();
        conversation_row.metadata["revision"] = json!(tx.revision());
        tx.put_conversation(&conversation_row)?;
        for turn_id in dedup(&changed.turns) {
            let Some(turn) = loaded.turn(turn_id) else {
                continue;
            };
            tx.put_turn(&TurnRow {
                conversation_id: conversation_id.to_string(),
                id: turn_id.clone(),
                position: loaded.position(turn_id).expect("indexed turn"),
                status: turn.status.clone(),
                updated_at: now.clone(),
                payload: to_object(turn)?,
            })?;
        }
        for segment_id in dedup(&changed.tombstoned_segments) {
            tx.delete_segment(conversation_id, segment_id)?;
        }
        for (segment_id, previous) in &changed.previous_outputs {
            let Some(segment) = loaded.segment(segment_id) else {
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
            if let Some(previous) = previous
                .as_ref()
                .filter(|previous| Some(*previous) != current.as_ref())
            {
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
        for (collection, id) in dedup(&changed.artifacts) {
            let Some(artifact) = loaded.artifact(collection, id) else {
                continue;
            };
            tx.put_artifact(&ArtifactRow {
                conversation_id: conversation_id.to_string(),
                collection: collection.clone(),
                id: id.clone(),
                turn_id: artifact.owner.clone(),
                status: artifact
                    .payload
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                updated_at: now.clone(),
                payload: artifact.payload.clone(),
            })?;
        }
        for id in &changed.newly_attached {
            let Some(attachment) = loaded.attachments.get(id) else {
                continue;
            };
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
        tx.changed_rows(conversation_id)
    })?;
    Ok((revision, rows, projected))
}

impl ConversationRepository {
    /// Test fixtures: store a complete record's rows exactly as given
    /// (timestamps, handle), replacing any loaded state. A missing handle or
    /// title is derived as a commit would.
    #[doc(hidden)]
    pub fn seed_record(&self, record: &ConversationRecord) -> Result<()> {
        let store = self.store().clone();
        let conversation_id = record.meta.conversation_id.clone();
        let project_path = store
            .registry()
            .ensure_project_paths(&record.meta.project_path)?
            .project_path;
        let mut loaded = LoadedConversation::new(&conversation_id, &project_path);
        loaded.meta = record.meta.clone();
        loaded.meta.project_path = project_path.clone();
        for (position, turn) in record.transcript.turns.iter().enumerate() {
            loaded.put_turn(turn.clone(), position as i64);
        }
        for segment in &record.transcript.segments {
            loaded.put_segment(segment.clone());
        }
        let collections = super::store::ALL_COLLECTIONS
            .iter()
            .map(|collection| (collection.key(), record.artifacts.collection(*collection)))
            .chain([(EVENT_LOG_COLLECTION, &record.artifacts.event_log)]);
        for (collection, artifacts) in collections {
            for artifact in artifacts {
                let mut artifact = artifact.clone();
                let id = match artifact.get("id").and_then(Value::as_str) {
                    Some(id) if collection != EVENT_LOG_COLLECTION => id.to_string(),
                    _ => {
                        let id = format!("artifact-{}", uuid::Uuid::new_v4().simple());
                        if collection != EVENT_LOG_COLLECTION {
                            artifact["id"] = json!(id);
                        }
                        id
                    }
                };
                let owner = artifact
                    .get("source_turn_id")
                    .and_then(Value::as_str)
                    .or_else(|| loaded.anchor_turn(&id))
                    .filter(|turn_id| loaded.turn(turn_id).is_some())
                    .map(str::to_string);
                let position = loaded.next_artifact_position(collection);
                loaded.put_artifact(
                    collection,
                    &id,
                    StoredArtifact {
                        position,
                        owner,
                        payload: artifact,
                    },
                );
            }
        }
        if loaded.meta.conversation_handle.is_empty() {
            loaded.meta.conversation_handle =
                crate::workspace_conversations::free_handle(&store, &conversation_id, None)?;
        }
        // Every committed row carries a title; derive a missing one the same way.
        if loaded.meta.title.trim().is_empty() {
            loaded.meta.title = loaded.derived_title();
        }
        let row = summary_row(&conversation_id, &loaded)?;
        let root = store.conversation_root(&project_path, &conversation_id)?;
        let updated_at = loaded.meta.updated_at.clone();
        let full = loaded.record();
        store.with_conversation(&conversation_id, |slot| {
            let (revision, ()) = store.database().write(|tx| {
                let mut row = row.clone();
                row.metadata["revision"] = json!(tx.revision());
                tx.put_conversation(&row)?;
                for turn in &full.transcript.turns {
                    tx.put_turn(&TurnRow {
                        conversation_id: conversation_id.clone(),
                        id: turn.id.clone(),
                        position: loaded.position(&turn.id).expect("indexed turn"),
                        status: turn.status.clone(),
                        updated_at: updated_at.clone(),
                        payload: to_object(turn)?,
                    })?;
                }
                for segment in &full.transcript.segments {
                    tx.put_segment(&SegmentRow {
                        conversation_id: conversation_id.clone(),
                        id: segment.id.clone(),
                        turn_id: segment.turn_id.clone(),
                        order: segment.order,
                        status: segment.status.clone(),
                        updated_at: updated_at.clone(),
                        payload: to_object(segment)?,
                    })?;
                    if let Some(output) = tool_output_artifact(segment) {
                        tx.put_blob_reference(
                            &conversation_id,
                            "tool_output",
                            &segment.id,
                            &store.home_relative(&root.join(output)),
                        )?;
                    }
                }
                let collections = super::store::ALL_COLLECTIONS
                    .iter()
                    .map(|collection| collection.key())
                    .chain([EVENT_LOG_COLLECTION]);
                for collection in collections {
                    for (id, artifact) in loaded.artifacts_in(collection) {
                        tx.put_artifact(&ArtifactRow {
                            conversation_id: conversation_id.clone(),
                            collection: collection.to_string(),
                            id: id.to_string(),
                            turn_id: artifact.owner.clone(),
                            status: artifact.payload["status"]
                                .as_str()
                                .unwrap_or_default()
                                .to_string(),
                            updated_at: updated_at.clone(),
                            payload: artifact.payload.clone(),
                        })?;
                    }
                }
                Ok(())
            })?;
            loaded.revision = revision;
            loaded.meta.revision = revision;
            *slot = Some(loaded);
            store.notify_commit(revision, vec![(conversation_id.clone(), project_path)]);
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
                    .turn(&record.turn_id)
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

/// Metadata and summary columns, from the loaded indexes.
fn summary_row(conversation_id: &str, loaded: &LoadedConversation) -> Result<ConversationRow> {
    let summary = loaded.summary();
    let meta = &loaded.meta;
    Ok(ConversationRow {
        id: conversation_id.to_string(),
        project_path: meta.project_path.clone(),
        handle: meta.conversation_handle.clone(),
        created_at: meta.created_at.clone(),
        updated_at: meta.updated_at.clone(),
        title: meta.title.clone(),
        preview: summary.preview,
        running: summary.running,
        needs_attention: summary.needs_attention,
        launched_run_ids: summary.launched_run_ids,
        metadata: to_object(meta)?,
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
            ConversationMutation::TurnUpserted { turn } if loaded.turn(&turn.id).is_none() => {
                turn.extra.get("attachments")
            }
            ConversationMutation::SegmentUpserted { segment }
            | ConversationMutation::RecoveredSegmentUpserted { segment, .. }
                if loaded.segment(&segment.id).is_none() =>
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
    loaded: &LoadedConversation,
    mutations: &[ConversationMutation],
) -> Result<()> {
    for mutation in mutations {
        let segment = match mutation {
            ConversationMutation::SegmentUpserted { segment }
            | ConversationMutation::RecoveredSegmentUpserted { segment, .. } => segment,
            _ => continue,
        };
        let turn_known = loaded.turn(&segment.turn_id).is_some()
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

fn apply_metadata_patch(meta: &mut ConversationMeta, patch: ConversationMetadataPatch) {
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

fn maintain_metadata(loaded: &mut LoadedConversation, now: &str) {
    if loaded.meta.created_at.trim().is_empty() {
        loaded.meta.created_at = now.to_string();
    }
    loaded.meta.updated_at = now.to_string();
    let title = loaded.meta.title.trim();
    if title.is_empty() || title == "New thread" {
        loaded.meta.title = loaded.derived_title();
    }
}

pub(crate) fn truncate_text(value: &str, limit: usize) -> String {
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
