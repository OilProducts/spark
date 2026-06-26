#![forbid(unsafe_code)]

//! Filesystem storage primitives for the Rust rewrite.
//!
//! This crate reserves the reusable JSON, TOML, JSONL, atomic file, unknown
//! field policy, and repository trait boundaries. Schema-specific repositories
//! remain owned by later milestones.

/// Atomic filesystem operations.
pub mod atomic;

/// Typed codecs for persisted files.
pub mod codecs;

/// Storage error types.
pub mod error;

/// Schema-neutral repository traits and adapters.
pub mod repository;

/// Unknown-field policy helpers.
pub mod unknown_fields;

/// Workspace project registry repository.
pub mod workspace_projects;

/// Workspace conversation repositories.
pub mod workspace_conversations;

/// Workspace source-tree flow catalog repository.
pub mod workspace_flow_catalog;

/// Workspace trigger definition and route-state repositories.
pub mod workspace_triggers;

pub use atomic::{append_jsonl_record, append_line, write_atomic, write_text_atomic};
pub use codecs::{
    append_jsonl, read_json, read_json_optional, read_jsonl, read_toml, read_toml_optional,
    write_json_atomic, write_toml_atomic, JsonLinesOptions, JsonLinesPolicy, JsonWriteOptions,
};
pub use error::{Result, StorageError};
pub use repository::{
    AppendLogRepository, DocumentRepository, JsonRepository, JsonlRepository, StorageFormat,
    TomlRepository,
};
pub use unknown_fields::{
    validate_json_object_fields, validate_json_object_fields_for, validate_toml_table_fields,
    validate_toml_table_fields_for, KnownFields, UnknownFieldPolicy, UnknownFieldReport,
};
pub use workspace_conversations::{
    normalize_conversation_handle, ConversationHandleMatch, ConversationHandleRecord,
    ConversationHandleRepository, ConversationRepository, RawConversationLogLine,
    CONVERSATION_HANDLE_PATTERN, CONVERSATION_HANDLE_SCHEMA_VERSION,
    CONVERSATION_STATE_SCHEMA_VERSION, UNSUPPORTED_CONVERSATION_STATE_SCHEMA,
    UNSUPPORTED_CONVERSATION_STATE_SEGMENTS,
};
pub use workspace_flow_catalog::{
    flow_catalog_path, load_flow_catalog, normalize_execution_lock_config,
    normalize_execution_lock_conflict_policy, normalize_execution_lock_scope,
    normalize_execution_lock_value, normalize_flow_name, normalize_launch_policy,
    read_flow_launch_policy, seed_default_flow_catalog, set_flow_catalog_entry,
    set_flow_launch_policy, write_flow_catalog, FlowCatalogEntry, FlowExecutionLockConfig,
    FlowLaunchPolicyState, ALLOWED_EXECUTION_LOCK_CONFLICT_POLICIES, ALLOWED_EXECUTION_LOCK_SCOPES,
    ALLOWED_LAUNCH_POLICIES, DEFAULT_AGENT_REQUESTABLE_FLOWS, EXECUTION_LOCK_CONFLICT_POLICY_QUEUE,
    EXECUTION_LOCK_SCOPE_PROJECT, FLOW_CATALOG_FILE_NAME, LAUNCH_POLICY_AGENT_REQUESTABLE,
    LAUNCH_POLICY_DISABLED, LAUNCH_POLICY_TRIGGER_ONLY,
};
pub use workspace_projects::{
    DeletedProjectRecord, ProjectPaths, ProjectRecord, ProjectRecordUpdate, ProjectRegistry,
};
pub use workspace_triggers::{
    delete_trigger_definition, delete_trigger_state, list_trigger_definitions, load_trigger_state,
    normalize_trigger_action_payload, normalize_trigger_source_payload, read_trigger_definition,
    save_trigger_state, trigger_config_dir, trigger_definition_path, trigger_state_dir,
    trigger_state_path, write_trigger_definition, TriggerAction, TriggerDefinition,
    TriggerDefinitionRepository, TriggerRepositories, TriggerRuntimeStateRepository, TriggerState,
    TriggerStateHistoryEntry,
};
