#![forbid(unsafe_code)]

//! Filesystem storage primitives for the Rust rewrite.
//!
//! This crate provides the reusable JSON, TOML, JSONL, atomic file, and unknown
//! field policy primitives, plus the schema-specific stores built on them.

/// Atomic filesystem operations.
pub mod atomic;

/// Shared append-only conversation and node-execution activity storage.
pub mod activity;

/// Typed codecs for persisted files.
pub mod codecs;

/// Typed conversation record model and commit boundary.
pub mod conversation;

/// Storage error types.
pub mod error;

/// Revision-checked settings documents and backed-up version transitions.
pub mod settings;

/// Unknown-field policy helpers.
pub mod unknown_fields;

/// Workspace project registry repository.
pub mod workspace_projects;

/// Workspace conversation repositories.
pub mod workspace_conversations;

/// Workspace trigger definition and route-state repositories.
pub mod workspace_triggers;

pub use activity::{
    ActivityEvent, ActivityRepository, TranscriptRecord, ACTIVITY_EVENTS_FILE_NAME,
    ACTIVITY_TRANSCRIPT_FILE_NAME,
};
pub use atomic::{append_jsonl_record, append_line, write_atomic, write_text_atomic};
pub use codecs::{
    append_jsonl, read_json, read_json_optional, read_jsonl, read_toml, read_toml_optional,
    write_json_atomic, write_toml_atomic, JsonLinesOptions, JsonLinesPolicy, JsonWriteOptions,
};
pub use conversation::ConversationStore;
pub use error::{Result, StorageError};
pub use unknown_fields::{
    validate_json_object_fields, validate_json_object_fields_for, validate_toml_table_fields,
    validate_toml_table_fields_for, KnownFields, UnknownFieldPolicy, UnknownFieldReport,
};
pub use workspace_conversations::{
    normalize_conversation_handle, ConversationHandleMatch, ConversationRepository,
    RawConversationLogLine, CONVERSATION_STATE_SCHEMA_VERSION,
    UNSUPPORTED_CONVERSATION_STATE_SCHEMA, UNSUPPORTED_CONVERSATION_STATE_SEGMENTS,
};
pub use workspace_projects::{
    DeletedProjectRecord, ProjectPaths, ProjectRecord, ProjectRecordUpdate, ProjectRegistry,
    ProjectSettings,
};
pub use workspace_triggers::{
    definition_fingerprint, normalize_trigger_action_payload, normalize_trigger_source_payload,
    TriggerAction, TriggerDefinition, TriggerDefinitionRepository, TriggerIssue,
    TriggerReconciliation, TriggerRepositories, TriggerRuntimeStateRepository, TriggerState,
    TriggerStateHistoryEntry, DEFINITION_CHANGED,
};
pub mod workspace_records;
