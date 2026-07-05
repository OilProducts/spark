use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SDKErrorKind {
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

pub type AdapterErrorKind = SDKErrorKind;

impl SDKErrorKind {
    pub fn default_retryable(self) -> bool {
        matches!(
            self,
            Self::Provider | Self::RateLimit | Self::Server | Self::Network | Self::Stream
        )
    }

    pub fn spec_error_name(self) -> &'static str {
        match self {
            Self::Provider => "ProviderError",
            Self::Authentication => "AuthenticationError",
            Self::AccessDenied => "AccessDeniedError",
            Self::NotFound => "NotFoundError",
            Self::InvalidRequest => "InvalidRequestError",
            Self::RateLimit => "RateLimitError",
            Self::Server => "ServerError",
            Self::ContentFilter => "ContentFilterError",
            Self::ContextLength => "ContextLengthError",
            Self::QuotaExceeded => "QuotaExceededError",
            Self::RequestTimeout => "RequestTimeoutError",
            Self::Abort => "AbortError",
            Self::Network => "NetworkError",
            Self::Stream => "StreamError",
            Self::InvalidToolCall => "InvalidToolCallError",
            Self::UnsupportedToolChoice => "UnsupportedToolChoiceError",
            Self::NoObjectGenerated => "NoObjectGeneratedError",
            Self::Configuration => "ConfigurationError",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Error)]
#[error("{message}")]
pub struct SDKError {
    pub kind: SDKErrorKind,
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

pub type ProviderError = SDKError;
pub type AdapterError = SDKError;

impl SDKError {
    pub fn new(kind: SDKErrorKind, message: impl Into<String>) -> Self {
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
        kind: SDKErrorKind,
        message: impl Into<String>,
        provider: impl Into<Option<String>>,
    ) -> Self {
        Self {
            provider: provider.into(),
            ..Self::new(kind, message)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Classification {
    kind: SDKErrorKind,
    overrideable_by_message: bool,
}

impl Classification {
    const fn new(kind: SDKErrorKind, overrideable_by_message: bool) -> Self {
        Self {
            kind,
            overrideable_by_message,
        }
    }
}

pub fn is_provider_kind(kind: SDKErrorKind) -> bool {
    matches!(
        kind,
        SDKErrorKind::Provider
            | SDKErrorKind::Authentication
            | SDKErrorKind::AccessDenied
            | SDKErrorKind::NotFound
            | SDKErrorKind::InvalidRequest
            | SDKErrorKind::RateLimit
            | SDKErrorKind::Server
            | SDKErrorKind::ContentFilter
            | SDKErrorKind::ContextLength
            | SDKErrorKind::QuotaExceeded
            | SDKErrorKind::RequestTimeout
    )
}

pub fn classify_provider_error_message(
    message: Option<&str>,
    error_code: Option<&str>,
) -> Option<SDKErrorKind> {
    let haystack = normalized_error_signal(message, error_code)?;
    if haystack.contains("not found") || haystack.contains("does not exist") {
        return Some(SDKErrorKind::NotFound);
    }
    if haystack.contains("invalid key")
        || haystack.contains("authentication")
        || haystack.contains("unauthorized")
        || haystack.contains("unauthenticated")
    {
        return Some(SDKErrorKind::Authentication);
    }
    if haystack.contains("permission")
        || haystack.contains("forbidden")
        || haystack.contains("access denied")
        || haystack.contains("denied")
    {
        return Some(SDKErrorKind::AccessDenied);
    }
    if haystack.contains("too many tokens")
        || haystack.contains("context length")
        || haystack.contains("input too large")
        || haystack.contains("maximum context")
    {
        return Some(SDKErrorKind::ContextLength);
    }
    if haystack.contains("content filter") || haystack.contains("safety") {
        return Some(SDKErrorKind::ContentFilter);
    }
    if haystack.contains("quota") || haystack.contains("insufficient quota") {
        return Some(SDKErrorKind::QuotaExceeded);
    }
    None
}

pub fn classify_http_status_code(status_code: Option<u16>) -> SDKErrorKind {
    http_status_classification(status_code).kind
}

pub fn classify_grpc_code(grpc_code: Option<&str>) -> SDKErrorKind {
    grpc_code_classification(grpc_code).kind
}

pub fn retry_after_from_headers(headers: &BTreeMap<String, String>) -> Option<f64> {
    header_value(headers, "retry-after").and_then(parse_retry_after_seconds)
}

pub fn error_from_status_code(
    status_code: Option<u16>,
    message: impl Into<String>,
    provider: Option<&str>,
    error_code: Option<&str>,
    retry_after: Option<f64>,
    raw: Option<Value>,
) -> SDKError {
    let (message, error_code) = resolve_error_details(message.into(), error_code, raw.as_ref());
    let error_code_ref = error_code.as_deref();

    if let Some(grpc_code) = error_code_ref {
        let grpc_classification = grpc_code_classification(Some(grpc_code));
        if grpc_classification.kind != SDKErrorKind::Provider {
            let mut error = error_from_grpc_code_parts(
                grpc_code,
                Some(message),
                provider,
                error_code_ref,
                retry_after,
                raw,
            );
            error.status_code = status_code;
            return error;
        }
    }

    let classification = http_status_classification(status_code);
    let kind = classify_provider_error_message(Some(&message), error_code_ref)
        .filter(|_| classification.overrideable_by_message)
        .unwrap_or(classification.kind);
    SDKError {
        kind,
        message,
        provider: provider.map(str::to_string),
        status_code,
        error_code,
        retryable: kind.default_retryable(),
        retry_after,
        raw,
    }
}

pub fn error_from_grpc_code(
    grpc_code: impl AsRef<str>,
    message: impl Into<String>,
    provider: Option<&str>,
    retry_after: Option<f64>,
    raw: Option<Value>,
) -> SDKError {
    error_from_grpc_code_parts(
        grpc_code.as_ref(),
        Some(message.into()),
        provider,
        None,
        retry_after,
        raw,
    )
}

fn error_from_grpc_code_parts(
    grpc_code: &str,
    message: Option<String>,
    provider: Option<&str>,
    error_code: Option<&str>,
    retry_after: Option<f64>,
    raw: Option<Value>,
) -> SDKError {
    let grpc_classification = grpc_code_classification(Some(grpc_code));
    let normalized_code = grpc_classification.normalized_code;
    let code_for_details = error_code.or(normalized_code.as_deref());
    let (message, resolved_error_code) =
        resolve_error_details(message.unwrap_or_default(), code_for_details, raw.as_ref());
    let kind = classify_provider_error_message(Some(&message), resolved_error_code.as_deref())
        .filter(|_| grpc_classification.overrideable_by_message)
        .unwrap_or(grpc_classification.kind);
    SDKError {
        kind,
        message,
        provider: provider.map(str::to_string),
        status_code: None,
        error_code: resolved_error_code.or(normalized_code),
        retryable: kind.default_retryable(),
        retry_after,
        raw,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GrpcClassification {
    kind: SDKErrorKind,
    overrideable_by_message: bool,
    normalized_code: Option<String>,
}

fn http_status_classification(status_code: Option<u16>) -> Classification {
    match status_code {
        Some(400 | 422) => Classification::new(SDKErrorKind::InvalidRequest, true),
        Some(401) => Classification::new(SDKErrorKind::Authentication, false),
        Some(403) => Classification::new(SDKErrorKind::AccessDenied, false),
        Some(404) => Classification::new(SDKErrorKind::NotFound, false),
        Some(408) => Classification::new(SDKErrorKind::RequestTimeout, false),
        Some(413) => Classification::new(SDKErrorKind::ContextLength, false),
        Some(429) => Classification::new(SDKErrorKind::RateLimit, true),
        Some(500..=599) => Classification::new(SDKErrorKind::Server, true),
        _ => Classification::new(SDKErrorKind::Provider, true),
    }
}

fn grpc_code_classification(grpc_code: Option<&str>) -> GrpcClassification {
    let normalized_code = grpc_code
        .and_then(non_empty)
        .map(|code| code.rsplit('.').next().unwrap_or(code).to_ascii_uppercase());
    let (kind, overrideable_by_message) = match normalized_code.as_deref() {
        Some("NOT_FOUND") => (SDKErrorKind::NotFound, false),
        Some("INVALID_ARGUMENT") => (SDKErrorKind::InvalidRequest, true),
        Some("UNAUTHENTICATED") => (SDKErrorKind::Authentication, false),
        Some("PERMISSION_DENIED") => (SDKErrorKind::AccessDenied, false),
        Some("RESOURCE_EXHAUSTED") => (SDKErrorKind::RateLimit, true),
        Some("UNAVAILABLE") => (SDKErrorKind::Server, true),
        Some("DEADLINE_EXCEEDED") => (SDKErrorKind::RequestTimeout, false),
        Some("INTERNAL") => (SDKErrorKind::Server, true),
        _ => (SDKErrorKind::Provider, true),
    };
    GrpcClassification {
        kind,
        overrideable_by_message,
        normalized_code,
    }
}

fn resolve_error_details(
    message: String,
    error_code: Option<&str>,
    raw: Option<&Value>,
) -> (String, Option<String>) {
    let message = non_empty(&message).map(str::to_string);
    let error_code = error_code.and_then(non_empty).map(str::to_string);
    let (raw_message, raw_error_code) = raw
        .map(extract_error_details_from_raw)
        .unwrap_or((None, None));
    let resolved_message = message
        .or(raw_message)
        .or_else(|| error_code.clone())
        .or_else(|| raw_error_code.clone())
        .unwrap_or_else(|| "provider error".to_string());
    let resolved_error_code = error_code.or(raw_error_code);
    (resolved_message, resolved_error_code)
}

pub fn extract_error_details_from_raw(raw: &Value) -> (Option<String>, Option<String>) {
    if let Value::String(message) = raw {
        return (non_empty(message).map(str::to_string), None);
    }

    if let Some(error) = raw.get("error") {
        match error {
            Value::Object(error) => {
                let message = first_text_field(error, &["message", "detail", "description"]);
                let code = first_text_field(error, &["status", "code", "type", "error_code"]);
                return (message, code);
            }
            Value::String(message) => return (non_empty(message).map(str::to_string), None),
            _ => {}
        }
    }

    let message = raw.as_object().and_then(|object| {
        first_text_field(
            object,
            &["message", "detail", "description", "error_description"],
        )
    });
    let code = raw
        .as_object()
        .and_then(|object| first_text_field(object, &["status", "error_code", "code", "type"]));
    (message, code)
}

fn first_text_field(object: &serde_json::Map<String, Value>, fields: &[&str]) -> Option<String> {
    fields
        .iter()
        .filter_map(|field| object.get(*field).and_then(value_to_string))
        .find_map(|text| non_empty(&text).map(str::to_string))
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn normalized_error_signal(message: Option<&str>, error_code: Option<&str>) -> Option<String> {
    let mut signal = String::new();
    for value in [message, error_code].into_iter().flatten() {
        if !signal.is_empty() {
            signal.push(' ');
        }
        signal.push_str(value);
    }
    let signal = signal
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    non_empty(&signal).map(str::to_string)
}

fn header_value<'a>(headers: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .and_then(|(_, value)| non_empty(value))
}

fn parse_retry_after_seconds(value: &str) -> Option<f64> {
    let retry_after = value.trim().parse::<f64>().ok()?;
    (retry_after.is_finite() && retry_after >= 0.0).then_some(retry_after)
}

fn non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}
