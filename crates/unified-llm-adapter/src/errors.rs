use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterErrorKind {
    Provider,
    Authentication,
    AccessDenied,
    NotFound,
    InvalidRequest,
    RateLimit,
    Server,
    ContentFilter,
    ContextLength,
    QuotaExceeded,
    RequestTimeout,
    Abort,
    Network,
    Stream,
    InvalidToolCall,
    UnsupportedToolChoice,
    NoObjectGenerated,
    Configuration,
}

impl AdapterErrorKind {
    pub fn default_retryable(self) -> bool {
        matches!(
            self,
            Self::Provider | Self::RateLimit | Self::Server | Self::Network | Self::Stream
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Error)]
#[error("{message}")]
pub struct AdapterError {
    pub kind: AdapterErrorKind,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

impl AdapterError {
    pub fn new(kind: AdapterErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            provider: None,
            status_code: None,
            error_code: None,
            retryable: kind.default_retryable(),
            retry_after: None,
            raw: None,
        }
    }

    pub fn provider(
        kind: AdapterErrorKind,
        message: impl Into<String>,
        provider: impl Into<Option<String>>,
    ) -> Self {
        Self {
            provider: provider.into(),
            ..Self::new(kind, message)
        }
    }
}

pub fn is_provider_kind(kind: AdapterErrorKind) -> bool {
    matches!(
        kind,
        AdapterErrorKind::Provider
            | AdapterErrorKind::Authentication
            | AdapterErrorKind::AccessDenied
            | AdapterErrorKind::NotFound
            | AdapterErrorKind::InvalidRequest
            | AdapterErrorKind::RateLimit
            | AdapterErrorKind::Server
            | AdapterErrorKind::ContentFilter
            | AdapterErrorKind::ContextLength
            | AdapterErrorKind::QuotaExceeded
            | AdapterErrorKind::RequestTimeout
    )
}

pub fn classify_provider_error_message(
    message: Option<&str>,
    error_code: Option<&str>,
) -> Option<AdapterErrorKind> {
    let haystack = format!(
        "{} {}",
        message.unwrap_or_default().to_lowercase(),
        error_code.unwrap_or_default().to_lowercase()
    );
    if haystack.trim().is_empty() {
        return None;
    }
    if haystack.contains("not_found")
        || haystack.contains("not found")
        || haystack.contains("does not exist")
    {
        return Some(AdapterErrorKind::NotFound);
    }
    if haystack.contains("invalid key")
        || haystack.contains("authentication")
        || haystack.contains("unauthorized")
    {
        return Some(AdapterErrorKind::Authentication);
    }
    if haystack.contains("permission")
        || haystack.contains("forbidden")
        || haystack.contains("denied")
    {
        return Some(AdapterErrorKind::AccessDenied);
    }
    if haystack.contains("too many tokens")
        || haystack.contains("context length")
        || haystack.contains("input too large")
    {
        return Some(AdapterErrorKind::ContextLength);
    }
    if haystack.contains("content filter") || haystack.contains("safety") {
        return Some(AdapterErrorKind::ContentFilter);
    }
    if haystack.contains("quota") || haystack.contains("insufficient_quota") {
        return Some(AdapterErrorKind::QuotaExceeded);
    }
    None
}

pub fn error_from_status_code(
    status_code: Option<u16>,
    message: impl Into<String>,
    provider: Option<&str>,
    error_code: Option<&str>,
    retry_after: Option<f64>,
    raw: Option<Value>,
) -> AdapterError {
    let message = message.into();
    let classified = classify_provider_error_message(Some(&message), error_code);
    let kind = classified.unwrap_or_else(|| match status_code {
        Some(400 | 422) => AdapterErrorKind::InvalidRequest,
        Some(401) => AdapterErrorKind::Authentication,
        Some(403) => AdapterErrorKind::AccessDenied,
        Some(404) => AdapterErrorKind::NotFound,
        Some(408) => AdapterErrorKind::RequestTimeout,
        Some(413) => AdapterErrorKind::ContextLength,
        Some(429) => AdapterErrorKind::RateLimit,
        Some(500..=599) => AdapterErrorKind::Server,
        _ => AdapterErrorKind::Provider,
    });
    AdapterError {
        kind,
        message,
        provider: provider.map(str::to_string),
        status_code,
        error_code: error_code.map(str::to_string),
        retryable: kind.default_retryable(),
        retry_after,
        raw,
    }
}
