//! Typed conversation record model and the single commit boundary.
//!
//! Five authorities make up a conversation: stable metadata, transcript render
//! state (turns/segments with artifact anchors), artifact records, the
//! committed journal, and runtime/debug sidecars. Services construct
//! [`ConversationMutation`] batches and commit them through
//! [`crate::ConversationRepository::commit_conversation`]; they never allocate
//! revisions or hand-build journal payloads.

mod commit;
mod identity;
mod journal;
mod migrate;
mod mutations;
mod projection;
mod records;
mod runtime_session;
mod store;
mod transient;

pub(crate) use migrate::migrate_legacy_conversation;
pub(crate) use store::{read_record, ConversationRecordPaths};

pub use commit::ConversationCommit;
pub use identity::{
    agent_event_segment_id, assistant_segment_id, context_compaction_segment_id,
    model_tool_segment_id, plan_segment_id, reasoning_segment_id, request_user_input_segment_id,
    segment_source, tool_call_id, tool_segment_id,
};
pub use journal::{JournalEntry, JournalEntryKind};
pub use mutations::{ConversationMetadataPatch, ConversationMutation};
pub use projection::{record_from_snapshot, snapshot_from_record};
pub use records::{
    ArtifactCollection, BoundaryMeta, ConversationArtifacts, ConversationMeta, ConversationRecord,
    Transcript, TranscriptSegment, TranscriptTurn, ORDER_UNASSIGNED, SEGMENT_KIND_AGENT_EVENT,
    SEGMENT_KIND_ASSISTANT_MESSAGE, SEGMENT_KIND_BOUNDARY, SEGMENT_KIND_CONTEXT_COMPACTION,
    SEGMENT_KIND_MODEL_TOOL_CALL, SEGMENT_KIND_PLAN, SEGMENT_KIND_REASONING,
    SEGMENT_KIND_REQUEST_USER_INPUT, SEGMENT_KIND_TOOL_CALL,
};
pub use runtime_session::{
    RuntimeSession, RUNTIME_SESSION_FILE_NAME, RUNTIME_SESSION_SCHEMA_VERSION,
};
pub use transient::{TransientStreamBody, TransientStreamEvent, TRANSIENT_STREAM_EVENT_TYPE};
