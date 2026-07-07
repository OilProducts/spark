use serde_json::Value;
use time::OffsetDateTime;

use crate::error::{Result, StorageError};
use crate::ConversationRepository;

use super::journal::{JournalEntry, JournalEntryKind};
use super::mutations::{ConversationMetadataPatch, ConversationMutation};
use super::projection::{record_from_snapshot, snapshot_from_record};
use super::records::{ConversationRecord, TranscriptSegment};

/// The result of one committed conversation mutation batch.
#[derive(Debug, Clone, PartialEq)]
pub struct ConversationCommit {
    pub record: ConversationRecord,
    /// Stamped journal entries, in commit order. Publish these verbatim.
    pub journal_entries: Vec<JournalEntry>,
    /// The exact journal line / live wire payloads appended for each entry.
    pub journal_payloads: Vec<Value>,
    /// Legacy full-snapshot projection of the committed state.
    pub snapshot: Value,
    /// Final committed revision.
    pub revision: i64,
    /// True when `base_revision` was stale and mutations were applied onto a
    /// newer committed state.
    pub rebased: bool,
}

impl ConversationRepository {
    /// The single conversation commit boundary.
    ///
    /// Loads the latest committed state, applies identity-keyed mutations onto
    /// it (so a stale `base_revision` rebases instead of clobbering), allocates
    /// segment orders and all journal revisions, maintains conversation
    /// metadata (timestamps, title, handle), persists durable state, and
    /// appends the stamped journal entries.
    pub fn commit_conversation(
        &self,
        conversation_id: &str,
        project_path: &str,
        base_revision: i64,
        mutations: Vec<ConversationMutation>,
    ) -> Result<ConversationCommit> {
        if mutations.is_empty() {
            return Err(commit_rejected(conversation_id, "No mutations to commit."));
        }
        let latest_snapshot = self.read_snapshot(conversation_id, Some(project_path))?;
        if latest_snapshot.is_none() && base_revision != 0 {
            return Err(commit_rejected(
                conversation_id,
                "Unknown conversation for a non-zero base revision.",
            ));
        }
        let mut record = match latest_snapshot.as_ref() {
            Some(snapshot) => record_from_snapshot(snapshot)?,
            None => ConversationRecord::new(conversation_id, project_path),
        };
        let latest_revision = record.meta.revision;
        let rebased = latest_revision != base_revision;

        validate_segment_targets(conversation_id, &record, &mutations)?;

        let now = iso_now();
        let mut entry_kinds = Vec::new();
        let mut snapshot_level_change = false;
        for mutation in mutations {
            match mutation {
                ConversationMutation::MetadataUpdated { patch } => {
                    apply_metadata_patch(&mut record, patch);
                    snapshot_level_change = true;
                }
                ConversationMutation::TurnUpserted { turn } => {
                    record.transcript.upsert_turn(turn.clone());
                    entry_kinds.push(JournalEntryKind::TurnUpserted { turn });
                }
                ConversationMutation::SegmentUpserted { mut segment } => {
                    resolve_segment_order(&record, &mut segment);
                    record.transcript.upsert_segment(segment.clone());
                    entry_kinds.push(JournalEntryKind::SegmentUpserted { segment });
                }
                ConversationMutation::ArtifactUpserted {
                    collection,
                    artifact,
                } => {
                    upsert_artifact(record.artifacts.collection_mut(collection), artifact);
                    snapshot_level_change = true;
                }
                ConversationMutation::WorkflowEventAppended { event } => {
                    record.artifacts.event_log.push(event);
                    snapshot_level_change = true;
                }
            }
        }
        if snapshot_level_change {
            entry_kinds.push(JournalEntryKind::SnapshotCommitted);
        }

        maintain_metadata(&mut record, &now);
        self.ensure_record_handle(&mut record, &now)?;

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

        let snapshot = snapshot_from_record(&record);
        self.write_snapshot(&snapshot)?;
        let mut journal_payloads = Vec::with_capacity(journal_entries.len());
        for entry in &journal_entries {
            let payload = entry.legacy_event_payload(&record.meta, &snapshot);
            self.append_conversation_event(conversation_id, &record.meta.project_path, &payload)?;
            journal_payloads.push(payload);
        }

        Ok(ConversationCommit {
            record,
            journal_entries,
            journal_payloads,
            snapshot,
            revision: final_revision,
            rebased,
        })
    }

    fn ensure_record_handle(&self, record: &mut ConversationRecord, now: &str) -> Result<()> {
        let project_paths = self.project_paths(&record.meta.project_path)?;
        let created_at = if record.meta.created_at.is_empty() {
            now
        } else {
            record.meta.created_at.as_str()
        };
        let preferred = (!record.meta.conversation_handle.is_empty())
            .then_some(record.meta.conversation_handle.as_str());
        record.meta.conversation_handle = self.handle_repository().ensure_conversation_handle(
            &record.meta.conversation_id,
            &project_paths.project_id,
            &record.meta.project_path,
            created_at,
            preferred,
        )?;
        Ok(())
    }
}

fn validate_segment_targets(
    conversation_id: &str,
    record: &ConversationRecord,
    mutations: &[ConversationMutation],
) -> Result<()> {
    for mutation in mutations {
        let ConversationMutation::SegmentUpserted { segment } = mutation else {
            continue;
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
                .then(|| truncate_title(&turn.content))
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "New thread".to_string())
}

fn truncate_title(value: &str) -> String {
    const LIMIT: usize = 64;
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= LIMIT {
        return collapsed;
    }
    let mut truncated = collapsed
        .chars()
        .take(LIMIT.saturating_sub(1))
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
    if artifact_id.is_empty() {
        collection.push(artifact);
        return;
    }
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

fn iso_now() -> String {
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
