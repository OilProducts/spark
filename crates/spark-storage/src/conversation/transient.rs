use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::records::{TranscriptSegment, TranscriptTurn};

/// Wire payload discriminant for transient stream deltas.
pub const TRANSIENT_STREAM_EVENT_TYPE: &str = "stream_delta";

/// A live stream update for connected clients. Transient events carry a
/// per-turn stream sequence instead of a durable revision, are never appended
/// to the committed journal (the journal writer only accepts
/// [`super::journal::JournalEntry`]), and may be dropped on reconnect because
/// the committed transcript restores durable state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransientStreamEvent {
    pub conversation_id: String,
    pub turn_id: String,
    /// Monotonic within one streaming turn; not comparable across turns.
    pub stream_sequence: u64,
    /// The committed revision this delta renders on top of.
    pub base_revision: i64,
    #[serde(flatten)]
    pub body: TransientStreamBody,
}

impl TransientStreamEvent {
    /// The live wire payload for this delta. Carries `type: "stream_delta"`
    /// and no `revision`, so the journal appender refuses it and clients can
    /// discriminate committed events from transients on payload type alone.
    pub fn wire_payload(&self) -> Value {
        let mut payload = serde_json::to_value(self).unwrap_or_else(|_| json!({}));
        if let Some(object) = payload.as_object_mut() {
            object.insert("type".to_string(), json!(TRANSIENT_STREAM_EVENT_TYPE));
        }
        payload
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "delta_kind", rename_all = "snake_case")]
pub enum TransientStreamBody {
    /// Coalesced turn render state (streaming assistant content, status).
    TurnDelta { turn: TranscriptTurn },
    /// Coalesced segment render state.
    SegmentDelta { segment: TranscriptSegment },
    /// Token usage progress for the active turn.
    TokenUsage { token_usage: Value },
}
