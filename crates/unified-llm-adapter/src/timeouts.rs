use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::errors::{AdapterError, AdapterErrorKind};

pub const DEFAULT_CONNECT_TIMEOUT_SECONDS: f64 = 10.0;
pub const DEFAULT_REQUEST_TIMEOUT_SECONDS: f64 = 120.0;
pub const DEFAULT_STREAM_READ_TIMEOUT_SECONDS: f64 = 30.0;

#[derive(Clone, Default)]
pub struct AbortSignal {
    state: Arc<AbortState>,
}

#[derive(Default)]
struct AbortState {
    aborted: AtomicBool,
    reason: Mutex<Option<String>>,
}

impl AbortSignal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn aborted(&self) -> bool {
        self.state.aborted.load(Ordering::SeqCst)
    }

    pub fn reason(&self) -> Option<String> {
        self.state.reason.lock().expect("abort reason lock").clone()
    }

    pub fn abort<R>(&self, reason: R)
    where
        R: IntoAbortReason,
    {
        if self.state.aborted.swap(true, Ordering::SeqCst) {
            return;
        }
        *self.state.reason.lock().expect("abort reason lock") = reason.into_abort_reason();
    }

    pub fn throw_if_aborted(&self) -> Result<(), AdapterError> {
        if self.aborted() {
            return Err(abort_error("operation", self.reason()));
        }
        Ok(())
    }
}

impl fmt::Debug for AbortSignal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AbortSignal")
            .field("aborted", &self.aborted())
            .field("reason", &self.reason())
            .finish()
    }
}

impl PartialEq for AbortSignal {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AbortController {
    pub signal: AbortSignal,
}

impl AbortController {
    pub fn new() -> Self {
        Self {
            signal: AbortSignal::new(),
        }
    }

    pub fn signal(&self) -> AbortSignal {
        self.signal.clone()
    }

    pub fn abort<R>(&self, reason: R)
    where
        R: IntoAbortReason,
    {
        self.signal.abort(reason);
    }
}

pub trait IntoAbortReason {
    fn into_abort_reason(self) -> Option<String>;
}

impl IntoAbortReason for String {
    fn into_abort_reason(self) -> Option<String> {
        Some(self)
    }
}

impl IntoAbortReason for &str {
    fn into_abort_reason(self) -> Option<String> {
        Some(self.to_string())
    }
}

impl IntoAbortReason for Option<String> {
    fn into_abort_reason(self) -> Option<String> {
        self
    }
}

impl IntoAbortReason for Option<&str> {
    fn into_abort_reason(self) -> Option<String> {
        self.map(str::to_string)
    }
}

impl IntoAbortReason for () {
    fn into_abort_reason(self) -> Option<String> {
        None
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct TimeoutConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_step: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_read: Option<f64>,
}

impl TimeoutConfig {
    pub fn new(total: Option<f64>, per_step: Option<f64>, stream_read: Option<f64>) -> Self {
        Self {
            total,
            per_step,
            stream_read,
        }
    }

    pub fn total(seconds: f64) -> Self {
        Self {
            total: Some(seconds),
            ..Self::default()
        }
    }

    pub fn per_step(seconds: f64) -> Self {
        Self {
            per_step: Some(seconds),
            ..Self::default()
        }
    }

    pub fn with_stream_read(mut self, stream_read: Option<f64>) -> Self {
        self.stream_read = stream_read;
        self
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_timeout_value("total", self.total)?;
        validate_timeout_value("per_step", self.per_step)?;
        validate_timeout_value("stream_read", self.stream_read)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AdapterTimeout {
    pub connect: f64,
    pub request: f64,
    pub stream_read: f64,
}

impl Default for AdapterTimeout {
    fn default() -> Self {
        Self {
            connect: DEFAULT_CONNECT_TIMEOUT_SECONDS,
            request: DEFAULT_REQUEST_TIMEOUT_SECONDS,
            stream_read: DEFAULT_STREAM_READ_TIMEOUT_SECONDS,
        }
    }
}

impl AdapterTimeout {
    pub fn new(connect: f64, request: f64, stream_read: f64) -> Self {
        Self {
            connect,
            request,
            stream_read,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_required_timeout_value("connect", self.connect)?;
        validate_required_timeout_value("request", self.request)?;
        validate_required_timeout_value("stream_read", self.stream_read)
    }
}

pub fn check_abort(signal: Option<&AbortSignal>) -> Result<(), AdapterError> {
    if let Some(signal) = signal {
        signal.throw_if_aborted()?;
    }
    Ok(())
}

pub fn abort_error(scope: impl Into<String>, reason: Option<String>) -> AdapterError {
    let scope = scope.into();
    let message = reason
        .as_ref()
        .filter(|reason| !reason.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| format!("{scope} aborted"));
    let mut error = AdapterError::new(AdapterErrorKind::Abort, message);
    error.raw = Some(json!({
        "scope": scope,
        "reason": reason,
    }));
    error
}

pub fn timeout_error(scope: impl Into<String>, timeout: Option<f64>) -> AdapterError {
    let scope = scope.into();
    let message = match timeout {
        Some(timeout) => format!("{scope} timed out after {timeout} seconds"),
        None => format!("{scope} timed out"),
    };
    let mut error = AdapterError::new(AdapterErrorKind::RequestTimeout, message);
    error.raw = Some(json!({
        "scope": scope,
        "timeout": timeout,
    }));
    error
}

fn validate_timeout_value(field_name: &str, value: Option<f64>) -> Result<(), String> {
    if let Some(value) = value {
        validate_required_timeout_value(field_name, value)?;
    }
    Ok(())
}

fn validate_required_timeout_value(field_name: &str, value: f64) -> Result<(), String> {
    if !value.is_finite() {
        return Err(format!("{field_name} timeout must be finite"));
    }
    if value < 0.0 {
        return Err(format!("{field_name} timeout must be non-negative"));
    }
    Ok(())
}
