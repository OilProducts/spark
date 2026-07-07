use serde_json::{json, Value};

use super::records::{ConversationMeta, TranscriptSegment, TranscriptTurn};

/// One committed, replayable journal entry. Revisions are allocated only at
/// the repository commit boundary and are strictly increasing per
/// conversation.
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
    /// Metadata and/or artifact records changed in this commit. The legacy
    /// journal representation is a full `conversation_snapshot` line.
    SnapshotCommitted,
}

impl JournalEntry {
    /// Project this entry into the historical `events.jsonl` line shape, which
    /// is also the live wire payload shape published to connected clients.
    pub fn legacy_event_payload(&self, meta: &ConversationMeta, snapshot: &Value) -> Value {
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
        }
    }
}
