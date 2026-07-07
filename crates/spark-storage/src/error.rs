use std::path::PathBuf;

/// Result type used by storage compatibility primitives.
pub type Result<T> = std::result::Result<T, StorageError>;

/// Errors raised by typed filesystem storage helpers.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("Unable to {action} {path}: {source}")]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Unable to parse JSON file {path}: {source}")]
    JsonRead {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("Unable to serialize JSON for {path}: {source}")]
    JsonWrite {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("Unable to parse TOML file {path}: {source}")]
    TomlRead {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("Unable to serialize TOML for {path}: {source}")]
    TomlWrite {
        path: PathBuf,
        #[source]
        source: toml::ser::Error,
    },

    #[error("Expected {format} top-level {expected} in {path}")]
    InvalidDocumentShape {
        path: PathBuf,
        format: &'static str,
        expected: &'static str,
    },

    #[error("Unknown top-level fields in {path}: {fields:?}")]
    UnknownFields { path: PathBuf, fields: Vec<String> },

    #[error("Unable to parse JSONL record at {path}:{line}: {source}")]
    JsonlLine {
        path: PathBuf,
        line: usize,
        #[source]
        source: serde_json::Error,
    },

    #[error("Unable to atomically replace {path} from temporary file {temp_path}: {source}")]
    AtomicPersist {
        path: PathBuf,
        temp_path: PathBuf,
        #[source]
        source: tempfile::PathPersistError,
    },

    #[error("Invalid repository path {path}: {reason}")]
    InvalidRepositoryPath { path: PathBuf, reason: String },

    #[error("{reason}")]
    InvalidConversationState { path: PathBuf, reason: String },

    #[error("Conversation commit rejected for {conversation_id}: {reason}")]
    ConversationCommitRejected {
        conversation_id: String,
        reason: String,
    },
}

impl StorageError {
    pub(crate) fn io(
        action: &'static str,
        path: impl Into<PathBuf>,
        source: std::io::Error,
    ) -> Self {
        Self::Io {
            action,
            path: path.into(),
            source,
        }
    }
}
