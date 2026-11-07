use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Wire payload discriminant for transient stream deltas.
pub const TRANSIENT_STREAM_EVENT_TYPE: &str = "stream_delta";

/// A live stream update for connected clients. Transient events carry a
/// per-turn stream sequence instead of a durable revision and are never
/// stored; the live hub keeps the latest text frame of each open segment for
/// replay after a client's committed baseline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransientStreamEvent {
    pub conversation_id: String,
    pub turn_id: String,
    /// Monotonic within one execution; clients compare it per segment.
    pub stream_sequence: u64,
    /// The committed revision this delta renders on top of.
    pub base_revision: i64,
    #[serde(flatten)]
    pub body: TransientStreamBody,
}

impl TransientStreamEvent {
    /// The live wire payload for this delta. Carries `type: "stream_delta"`
    /// and no `revision`, so clients can
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
    /// The full text an open segment has grown to since its committed row:
    /// its content, or a tool call's output. Clients overlay it on that row
    /// until the row closes.
    SegmentText { segment_id: String, text: String },
    /// The turn is thinking, or has stopped, with the harness's token estimate.
    Thinking {
        active: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        estimated_tokens: Option<u64>,
    },
}
