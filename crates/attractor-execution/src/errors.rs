use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionProfileFieldError {
    pub field: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
}

impl ExecutionProfileFieldError {
    pub fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
            profile_id: None,
        }
    }

    pub fn for_profile(
        profile_id: impl Into<String>,
        field: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
            profile_id: Some(profile_id.into()),
        }
    }
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct ExecutionProfileConfigError {
    pub message: String,
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
    pub field_errors: Vec<ExecutionProfileFieldError>,
}

impl ExecutionProfileConfigError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
            field_errors: Vec::new(),
        }
    }

    pub fn with_source(
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            message: message.into(),
            source: Some(Box::new(source)),
            field_errors: Vec::new(),
        }
    }

    pub fn with_field_errors(field_errors: Vec<ExecutionProfileFieldError>) -> Self {
        let detail = field_errors
            .iter()
            .map(|error| format!("{}: {}", error.field, error.message))
            .collect::<Vec<_>>()
            .join("; ");
        Self {
            message: format!("invalid execution profile config: {detail}"),
            source: None,
            field_errors,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{message}")]
pub struct ExecutionProfileSelectionError {
    pub message: String,
}

impl ExecutionProfileSelectionError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[error("{message}")]
pub struct ExecutionLaunchError {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
}

impl ExecutionLaunchError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: None,
            retryable: None,
            status_code: None,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[error("{message}")]
pub struct ExecutionProtocolError {
    pub message: String,
}

impl ExecutionProtocolError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}
