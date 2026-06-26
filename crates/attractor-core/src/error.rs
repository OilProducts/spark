/// Result type used by Attractor core contracts.
pub type Result<T> = std::result::Result<T, AttractorCoreError>;

/// Errors raised while validating shared Attractor contract data.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AttractorCoreError {
    #[error("Unsupported context key namespace for '{key}'")]
    InvalidContextNamespace { key: String },

    #[error("Invalid context key '{key}': {reason}")]
    InvalidContextKey { key: String, reason: String },

    #[error("Invalid launch context key '{key}': {reason}")]
    InvalidLaunchContextKey { key: String, reason: String },

    #[error("Invalid launch context value at '{path}': {reason}")]
    InvalidLaunchContextValue { path: String, reason: String },

    #[error("Malformed {contract} contract: {reason}")]
    MalformedContextContract {
        contract: &'static str,
        reason: String,
    },

    #[error("Invalid {kind} identifier '{value}': {reason}")]
    InvalidIdentifier {
        kind: &'static str,
        value: String,
        reason: String,
    },
}
