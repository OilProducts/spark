use thiserror::Error;

pub type WorkspaceResult<T> = std::result::Result<T, WorkspaceError>;

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("{0}")]
    Validation(String),

    #[error("{0}")]
    NotFound(String),

    #[error("{0}")]
    Conflict(String),

    #[error("{0}")]
    Forbidden(String),

    #[error("{0}")]
    ServiceUnavailable(String),

    #[error("{0}")]
    Internal(String),
}

impl WorkspaceError {
    pub fn status_code(&self) -> u16 {
        match self {
            Self::Validation(_) => 400,
            Self::NotFound(_) => 404,
            Self::Conflict(_) => 409,
            Self::Forbidden(_) => 403,
            Self::ServiceUnavailable(_) => 503,
            Self::Internal(_) => 500,
        }
    }

    pub fn detail(&self) -> String {
        self.to_string()
    }
}

impl From<spark_storage::StorageError> for WorkspaceError {
    fn from(value: spark_storage::StorageError) -> Self {
        match value {
            spark_storage::StorageError::InvalidRepositoryPath { reason, .. } => {
                Self::Validation(reason)
            }
            spark_storage::StorageError::InvalidConversationState { reason, .. } => {
                Self::Validation(reason)
            }
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<spark_triggers::TriggerError> for WorkspaceError {
    fn from(value: spark_triggers::TriggerError) -> Self {
        match value {
            spark_triggers::TriggerError::UnknownTrigger => {
                Self::NotFound("Unknown trigger.".to_string())
            }
            spark_triggers::TriggerError::UnknownWebhookKey => {
                Self::NotFound("Unknown webhook key.".to_string())
            }
            spark_triggers::TriggerError::InvalidWebhookSecret => {
                Self::Forbidden("Webhook secret is invalid.".to_string())
            }
            spark_triggers::TriggerError::ProtectedDelete => {
                Self::Validation("Protected triggers cannot be deleted.".to_string())
            }
            spark_triggers::TriggerError::Validation(message) => Self::Validation(message),
            spark_triggers::TriggerError::Storage(error) => Self::from(error),
        }
    }
}
