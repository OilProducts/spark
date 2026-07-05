use std::path::PathBuf;

/// Result type used by shared Spark compatibility primitives.
pub type Result<T> = std::result::Result<T, SparkCommonError>;

/// Errors raised by the compatibility-oriented common crate.
#[derive(Debug, thiserror::Error)]
pub enum SparkCommonError {
    #[error("Project path is required.")]
    EmptyProjectPath,

    #[error("Unable to create {label} directory: {path}")]
    DirectoryCreate {
        label: String,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{label} directory is not writable: {path}")]
    DirectoryNotWritable {
        label: String,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("UI directory does not contain index.html: {0}")]
    InvalidUiDirectory(PathBuf),

    #[error("{0}")]
    SourceCheckoutGuard(String),

    #[error("Unable to resolve current directory: {0}")]
    CurrentDirectory(#[source] std::io::Error),

    #[error("Unable to join process line reader thread.")]
    ProcessReaderJoin,

    #[error("Invalid turn stream event: {0}")]
    EventValidation(String),
}
