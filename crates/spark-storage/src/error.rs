use std::path::PathBuf;

/// Result type used by storage compatibility primitives.
pub type Result<T> = std::result::Result<T, StorageError>;

/// Errors raised by typed filesystem storage helpers.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("Settings changed in {path}; reload before saving.")]
    SettingsConflict { path: PathBuf },

    #[error("Invalid settings in {path}: {reason}")]
    SettingsValidation { path: PathBuf, reason: String },
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

    #[error("Conversation database error: {0}")]
    ConversationDatabase(#[from] rusqlite::Error),

    #[error(
        "Conversation database {path} has unsupported schema version {found} (expected {expected}); stop Spark and remove it to reset conversation history."
    )]
    UnsupportedConversationDatabase {
        path: PathBuf,
        found: i64,
        expected: i64,
    },

    #[error("Another Spark server is using {path}; stop it or choose a different Spark home.")]
    HomeLocked { path: PathBuf },

    /// Destructive filesystem cleanup did not run; every file is intact and
    /// the cleanup is retried later.
    #[error("Cleanup deferred: {reason}")]
    CleanupDeferred { reason: String },

    /// A WAL checkpoint failed or did not checkpoint every frame; it is
    /// retried later and never affects an acknowledged transaction.
    #[error("Checkpoint incomplete: {reason}")]
    CheckpointIncomplete { reason: String },
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
