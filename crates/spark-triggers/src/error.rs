use thiserror::Error;

use spark_storage::StorageError;

pub type TriggerResult<T> = std::result::Result<T, TriggerError>;

#[derive(Debug, Error)]
pub enum TriggerError {
    #[error("{0}")]
    Validation(String),

    #[error("Unknown trigger.")]
    UnknownTrigger,

    #[error("Unknown webhook key.")]
    UnknownWebhookKey,

    #[error("Webhook secret is invalid.")]
    InvalidWebhookSecret,

    #[error("Protected triggers cannot be deleted.")]
    ProtectedDelete,

    #[error(transparent)]
    Storage(StorageError),
}

impl From<StorageError> for TriggerError {
    fn from(value: StorageError) -> Self {
        match value {
            StorageError::InvalidRepositoryPath { reason, .. } => Self::Validation(reason),
            other => Self::Storage(other),
        }
    }
}
