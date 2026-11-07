//! Typed conversation record model, the app-owned SQLite store, and the
//! single commit boundary.
//!
//! Every conversation entity is a row in the home's conversation database,
//! the only authority for conversation state; attachment and tool-output
//! bytes are immutable blobs it references. Services change a conversation
//! only through the typed operations on [`crate::ConversationRepository`]
//! (see [`operations`]); each reads, validates and writes current rows in
//! one transaction.

mod attachments;
mod commit;
mod database;
#[cfg(test)]
mod database_tests;
mod identity;
pub mod operations;
mod projection;
mod records;
mod runtime_session;
mod store;
mod tool_output;
mod transient;

pub(crate) use commit::{
    completed_replies, derived_title, parse_json, parse_meta, payload, payloads, Open,
};
pub(crate) use store::remove_dir_if_exists;
pub(crate) use store::{open_executions, EVENT_LOG_COLLECTION};
pub(crate) use tool_output::{
    externalize_segment_tool_output, externalize_tool_output, is_safe_segment_file_id,
    safe_tool_output_artifact,
};

pub use attachments::{turn_attachment_ids, ConversationAttachment, ATTACHMENTS_DIR_NAME};
pub use commit::ConversationCommit;
pub use database::{
    CatchUp, ChangeBatch, ChangeScope, CheckpointFault, CheckpointedView, ConversationDatabase,
    EntityKind, EntityRow, PageCursor, PageRead, ProjectDeletionMarker, ReadTransaction,
    ResetReason, StoredConversation, TurnPage, CATCH_UP_MAX_BYTES, CATCH_UP_MAX_ROWS,
    CONVERSATION_DATABASE_FILE_NAME, CONVERSATION_DATABASE_SCHEMA_VERSION,
};
pub use identity::{
    agent_event_segment_id, assistant_segment_id, context_compaction_segment_id,
    model_tool_segment_id, plan_segment_id, reasoning_segment_id, request_user_input_segment_id,
    segment_source, tool_call_id, tool_segment_id,
};
pub use operations::{
    Accepted, Admission, Answer, AnsweredQuestion, ArtifactChange, ArtifactOutcome,
    ConversationMetadataPatch, Delivery, EventOutcome, ExecutionOutcome, FlowRequestApproval,
    MessageInput, Settlement,
};
pub use projection::{record_from_snapshot, snapshot_from_record};
pub use records::{
    ArtifactCollection, BoundaryMeta, ConversationArtifacts, ConversationMeta, ConversationRecord,
    Transcript, TranscriptSegment, TranscriptTurn, ORDER_UNASSIGNED, SEGMENT_KIND_AGENT_EVENT,
    SEGMENT_KIND_ASSISTANT_MESSAGE, SEGMENT_KIND_BOUNDARY, SEGMENT_KIND_CONTEXT_COMPACTION,
    SEGMENT_KIND_MODEL_TOOL_CALL, SEGMENT_KIND_PLAN, SEGMENT_KIND_REASONING,
    SEGMENT_KIND_REQUEST_USER_INPUT, SEGMENT_KIND_TOOL_CALL,
};
pub use runtime_session::{RuntimeSession, RUNTIME_SESSION_SCHEMA_VERSION};
pub use store::{CommitListener, CommitNotice, ConversationStore, SERVER_LOCK_FILE_NAME};
pub use tool_output::{truncate_utf8, TOOL_OUTPUT_INLINE_LIMIT_BYTES};
pub use transient::{TransientStreamBody, TransientStreamEvent, TRANSIENT_STREAM_EVENT_TYPE};
