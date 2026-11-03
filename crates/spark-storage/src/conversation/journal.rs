use serde_json::{json, Value};

use super::records::{ConversationMeta, TranscriptSegment, TranscriptTurn};

/// Entry type for snapshot-level commits (metadata/artifact changes); they
/// publish live as a full `conversation_snapshot` payload instead.
pub const CONVERSATION_SNAPSHOT_REF_TYPE: &str = "conversation_snapshot_ref";

/// One committed change as published to live clients. Its revision is the
/// conversation's legacy per-conversation counter, allocated only at the
/// commit boundary; nothing replays these entries, so a reconnecting client
/// receives a fresh snapshot instead.
#[derive(Debug, Clone, PartialEq)]
pub struct JournalEntry {
    pub revision: i64,
    pub committed_at: String,
    pub kind: JournalEntryKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum JournalEntryKind {
    TurnUpserted {
        turn: TranscriptTurn,
    },
    SegmentUpserted {
        segment: TranscriptSegment,
    },
    /// One obsolete projected segment removed by compatibility repair.
    SegmentTombstoned {
        turn_id: String,
        segment_id: String,
    },
    /// Metadata and/or artifact records changed in this commit. Journaled as a
    /// slim `conversation_snapshot_ref` line; published live as a full
    /// `conversation_snapshot` payload.
    SnapshotCommitted,
}

impl JournalEntry {
    /// The entry's upsert or tombstone payload.
    pub fn journal_line_payload(&self, meta: &ConversationMeta) -> Value {
        match &self.kind {
            JournalEntryKind::TurnUpserted { turn } => json!({
                "type": "turn_upsert",
                "revision": self.revision,
                "conversation_id": meta.conversation_id,
                "project_path": meta.project_path,
                "title": meta.title,
                "updated_at": self.committed_at,
                "turn": turn,
            }),
            JournalEntryKind::SegmentUpserted { segment } => json!({
                "type": "segment_upsert",
                "revision": self.revision,
                "conversation_id": meta.conversation_id,
                "project_path": meta.project_path,
                "title": meta.title,
                "updated_at": self.committed_at,
                "segment": segment,
            }),
            JournalEntryKind::SegmentTombstoned {
                turn_id,
                segment_id,
            } => json!({
                "type": "segment_tombstone",
                "revision": self.revision,
                "conversation_id": meta.conversation_id,
                "project_path": meta.project_path,
                "title": meta.title,
                "updated_at": self.committed_at,
                "turn_id": turn_id,
                "segment_id": segment_id,
            }),
            JournalEntryKind::SnapshotCommitted => json!({
                "type": CONVERSATION_SNAPSHOT_REF_TYPE,
                "revision": self.revision,
                "conversation_id": meta.conversation_id,
                "project_path": meta.project_path,
                "updated_at": self.committed_at,
            }),
        }
    }

    /// The payload published to connected clients for this entry. Identical to
    /// the journal line except for snapshot-level commits, which publish the
    /// full `conversation_snapshot` shape so live clients can apply them
    /// directly.
    pub fn live_event_payload(&self, meta: &ConversationMeta, snapshot: &Value) -> Value {
        match &self.kind {
            JournalEntryKind::SnapshotCommitted => {
                let mut state = snapshot.clone();
                if let Some(object) = state.as_object_mut() {
                    object.insert("revision".to_string(), json!(self.revision));
                    object.insert("updated_at".to_string(), json!(self.committed_at));
                }
                json!({
                    "type": "conversation_snapshot",
                    "revision": self.revision,
                    "state": state,
                })
            }
            _ => self.journal_line_payload(meta),
        }
    }
}
