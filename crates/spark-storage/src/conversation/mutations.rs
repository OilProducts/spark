use serde_json::Value;

use super::records::{ArtifactCollection, TranscriptSegment, TranscriptTurn};

/// One typed change to a conversation's durable state. Mutations are keyed by
/// identity (turn id, segment id, artifact id, metadata field set) so the
/// commit boundary can apply them onto the latest committed state regardless
/// of the caller's `base_revision`.
#[derive(Debug, Clone, PartialEq)]
pub enum ConversationMutation {
    MetadataUpdated {
        patch: ConversationMetadataPatch,
    },
    TurnUpserted {
        turn: TranscriptTurn,
    },
    SegmentUpserted {
        segment: TranscriptSegment,
    },
    /// Upsert one artifact record (matched by `id`) in a collection.
    ArtifactUpserted {
        collection: ArtifactCollection,
        artifact: Value,
    },
    /// Append one workflow event to the conversation event log.
    WorkflowEventAppended {
        event: Value,
    },
}

/// Field-level metadata patch. `None` leaves a field unchanged; the nested
/// `Option` distinguishes "set" from "clear" for nullable settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConversationMetadataPatch {
    pub chat_mode: Option<String>,
    pub provider: Option<String>,
    pub model: Option<Option<String>>,
    pub llm_profile: Option<Option<String>>,
    pub reasoning_effort: Option<Option<String>>,
    pub title: Option<String>,
}

impl ConversationMetadataPatch {
    pub fn is_empty(&self) -> bool {
        self.chat_mode.is_none()
            && self.provider.is_none()
            && self.model.is_none()
            && self.llm_profile.is_none()
            && self.reasoning_effort.is_none()
            && self.title.is_none()
    }
}
