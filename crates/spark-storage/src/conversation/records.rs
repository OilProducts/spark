use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Segment `order` sentinel meaning "allocate at the commit boundary".
pub const ORDER_UNASSIGNED: i64 = -1;

pub const SEGMENT_KIND_ASSISTANT_MESSAGE: &str = "assistant_message";
pub const SEGMENT_KIND_REASONING: &str = "reasoning";
pub const SEGMENT_KIND_PLAN: &str = "plan";
pub const SEGMENT_KIND_TOOL_CALL: &str = "tool_call";
pub const SEGMENT_KIND_MODEL_TOOL_CALL: &str = "model_tool_call";
pub const SEGMENT_KIND_REQUEST_USER_INPUT: &str = "request_user_input";
pub const SEGMENT_KIND_CONTEXT_COMPACTION: &str = "context_compaction";
pub const SEGMENT_KIND_AGENT_EVENT: &str = "agent_event";
pub const SEGMENT_KIND_BOUNDARY: &str = "boundary";

/// Stable conversation metadata. Mirrors the core key whitelist persisted in
/// `state.json`; transcript arrays and artifact records are separate
/// authorities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversationMeta {
    pub schema_version: i64,
    pub revision: i64,
    pub conversation_id: String,
    #[serde(default)]
    pub conversation_handle: String,
    pub project_path: String,
    #[serde(default = "default_chat_mode")]
    pub chat_mode: String,
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub llm_profile: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default = "default_title")]
    pub title: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

fn default_chat_mode() -> String {
    "chat".to_string()
}

fn default_provider() -> String {
    "codex".to_string()
}

fn default_title() -> String {
    "New thread".to_string()
}

impl ConversationMeta {
    pub fn new(conversation_id: &str, project_path: &str) -> Self {
        Self {
            schema_version: crate::CONVERSATION_STATE_SCHEMA_VERSION,
            revision: 0,
            conversation_id: conversation_id.to_string(),
            conversation_handle: String::new(),
            project_path: project_path.to_string(),
            chat_mode: default_chat_mode(),
            provider: default_provider(),
            model: None,
            llm_profile: None,
            reasoning_effort: None,
            title: default_title(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }
}

/// One conversation or run transcript turn. Unknown persisted fields are
/// preserved through `extra` so records never lose data on rewrite.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TranscriptTurn {
    pub id: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub timestamp: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_turn_id: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl TranscriptTurn {
    pub fn turn_kind(&self) -> &str {
        self.kind.as_deref().unwrap_or("message")
    }
}

/// One coalesced transcript segment. The core contract is shared between
/// project chat and run/LLM-node transcripts; scope-specific metadata stays
/// outside these fields (`extra` today, typed boundary metadata for runs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub id: String,
    pub turn_id: String,
    #[serde(default = "unassigned_order")]
    pub order: i64,
    pub kind: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub timestamp: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_user_input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn unassigned_order() -> i64 {
    ORDER_UNASSIGNED
}

impl TranscriptSegment {
    pub fn has_unassigned_order(&self) -> bool {
        self.order < 0
    }
}

/// The canonical durable render state: ordered turns and segments.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    #[serde(default)]
    pub turns: Vec<TranscriptTurn>,
    #[serde(default)]
    pub segments: Vec<TranscriptSegment>,
}

impl Transcript {
    pub fn find_turn(&self, turn_id: &str) -> Option<&TranscriptTurn> {
        self.turns.iter().find(|turn| turn.id == turn_id)
    }

    pub fn find_turn_mut(&mut self, turn_id: &str) -> Option<&mut TranscriptTurn> {
        self.turns.iter_mut().find(|turn| turn.id == turn_id)
    }

    pub fn find_segment(&self, segment_id: &str) -> Option<&TranscriptSegment> {
        self.segments
            .iter()
            .find(|segment| segment.id == segment_id)
    }

    pub fn upsert_turn(&mut self, turn: TranscriptTurn) {
        if let Some(existing) = self.find_turn_mut(&turn.id) {
            *existing = turn;
        } else {
            self.turns.push(turn);
        }
    }

    pub fn upsert_segment(&mut self, segment: TranscriptSegment) {
        if let Some(existing) = self
            .segments
            .iter_mut()
            .find(|existing| existing.id == segment.id)
        {
            *existing = segment;
        } else {
            self.segments.push(segment);
        }
    }

    /// Next `order` within a turn, matching the historical allocation rule
    /// (max existing order + 1, starting at 1).
    pub fn next_segment_order(&self, turn_id: &str) -> i64 {
        self.segments
            .iter()
            .filter(|segment| segment.turn_id == turn_id)
            .map(|segment| segment.order)
            .max()
            .unwrap_or(0)
            + 1
    }
}

/// Durable artifact records anchored from the transcript by id and kind.
/// Individual artifacts stay schemaless (`Value`) until their own records are
/// typed; the collections are the authority boundary.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConversationArtifacts {
    #[serde(default)]
    pub event_log: Vec<Value>,
    #[serde(default)]
    pub flow_run_requests: Vec<Value>,
    #[serde(default)]
    pub flow_launches: Vec<Value>,
    #[serde(default)]
    pub run_recoveries: Vec<Value>,
    #[serde(default)]
    pub proposed_plans: Vec<Value>,
}

impl ConversationArtifacts {
    pub fn collection(&self, collection: ArtifactCollection) -> &Vec<Value> {
        match collection {
            ArtifactCollection::FlowRunRequests => &self.flow_run_requests,
            ArtifactCollection::FlowLaunches => &self.flow_launches,
            ArtifactCollection::RunRecoveries => &self.run_recoveries,
            ArtifactCollection::ProposedPlans => &self.proposed_plans,
        }
    }

    pub fn collection_mut(&mut self, collection: ArtifactCollection) -> &mut Vec<Value> {
        match collection {
            ArtifactCollection::FlowRunRequests => &mut self.flow_run_requests,
            ArtifactCollection::FlowLaunches => &mut self.flow_launches,
            ArtifactCollection::RunRecoveries => &mut self.run_recoveries,
            ArtifactCollection::ProposedPlans => &mut self.proposed_plans,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactCollection {
    FlowRunRequests,
    FlowLaunches,
    RunRecoveries,
    ProposedPlans,
}

impl ArtifactCollection {
    pub fn key(&self) -> &'static str {
        match self {
            Self::FlowRunRequests => "flow_run_requests",
            Self::FlowLaunches => "flow_launches",
            Self::RunRecoveries => "run_recoveries",
            Self::ProposedPlans => "proposed_plans",
        }
    }
}

/// A complete typed view of one conversation's durable state.
#[derive(Debug, Clone, PartialEq)]
pub struct ConversationRecord {
    pub meta: ConversationMeta,
    pub transcript: Transcript,
    pub artifacts: ConversationArtifacts,
}

impl ConversationRecord {
    pub fn new(conversation_id: &str, project_path: &str) -> Self {
        Self {
            meta: ConversationMeta::new(conversation_id, project_path),
            transcript: Transcript::default(),
            artifacts: ConversationArtifacts::default(),
        }
    }
}
