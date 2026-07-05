use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, RuntimeStorageError>;

#[derive(Debug, thiserror::Error)]
pub enum RuntimeStorageError {
    #[error("Invalid run id {run_id:?}: {reason}")]
    InvalidRunId { run_id: String, reason: String },

    #[error("Run root does not exist: {path}")]
    MissingRunRoot { path: PathBuf },

    #[error("Unsafe artifact path {path:?}: {reason}")]
    UnsafeArtifactPath { path: String, reason: String },

    #[error("Invalid runtime graph: {reason}")]
    InvalidRuntimeGraph { reason: String },

    #[error("Unable to {action} {path}: {source}")]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Invalid runtime JSON in {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("{0}")]
    Core(#[from] attractor_core::AttractorCoreError),

    #[error("{0}")]
    Common(#[from] spark_common::SparkCommonError),

    #[error("{0}")]
    Storage(#[from] spark_storage::StorageError),
}

impl RuntimeStorageError {
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

    pub(crate) fn json(path: impl Into<PathBuf>, source: serde_json::Error) -> Self {
        Self::Json {
            path: path.into(),
            source,
        }
    }
}
