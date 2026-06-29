use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::env::ProviderConfig;
use crate::errors::{AdapterError, AdapterErrorKind};
use crate::timeouts::{AbortSignal, AdapterTimeout};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NativeRequestConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default)]
    pub default_headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default)]
    pub timeout: AdapterTimeout,
}

impl NativeRequestConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: Some(api_key.into()),
            ..Self::default()
        }
    }
}

impl From<&NativeRequestConfig> for NativeRequestConfig {
    fn from(config: &NativeRequestConfig) -> Self {
        config.clone()
    }
}

impl From<&ProviderConfig> for NativeRequestConfig {
    fn from(config: &ProviderConfig) -> Self {
        Self {
            api_key: config.api_key.clone(),
            base_url: config.base_url.clone(),
            default_headers: BTreeMap::new(),
            organization: config.options.get("organization").cloned(),
            project: config.options.get("project").cloned(),
            timeout: AdapterTimeout::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeCompleteRequest {
    pub provider: String,
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub timeout: AdapterTimeout,
    #[serde(default, skip)]
    pub abort_signal: Option<AbortSignal>,
    pub body: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeCompleteResponse {
    pub status: u16,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    pub body: Value,
}

impl NativeCompleteResponse {
    pub fn ok(body: Value) -> Self {
        Self {
            status: 200,
            headers: BTreeMap::new(),
            body,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NativeStreamResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<Result<Value, AdapterError>>,
}

impl NativeStreamResponse {
    pub fn ok(body: impl IntoIterator<Item = Value>) -> Self {
        Self {
            status: 200,
            headers: BTreeMap::new(),
            body: body.into_iter().map(Ok).collect(),
        }
    }

    pub fn sse(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            headers: BTreeMap::new(),
            body: vec![Ok(Value::String(body.into()))],
        }
    }
}

pub type NativeStreamBody = Box<dyn Iterator<Item = Result<Value, AdapterError>> + Send>;

pub struct NativeStreamChunkResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: NativeStreamBody,
}

impl NativeStreamChunkResponse {
    pub fn new(status: u16, headers: BTreeMap<String, String>, body: NativeStreamBody) -> Self {
        Self {
            status,
            headers,
            body,
        }
    }

    pub fn buffered(response: NativeStreamResponse) -> Self {
        Self {
            status: response.status,
            headers: response.headers,
            body: Box::new(response.body.into_iter()),
        }
    }

    pub fn into_buffered(self) -> NativeStreamResponse {
        NativeStreamResponse {
            status: self.status,
            headers: self.headers,
            body: self.body.collect(),
        }
    }
}

pub trait NativeCompleteTransport: Send + Sync {
    fn complete(
        &self,
        request: NativeCompleteRequest,
    ) -> Result<NativeCompleteResponse, AdapterError>;

    fn stream(&self, request: NativeCompleteRequest) -> Result<NativeStreamResponse, AdapterError> {
        Err(AdapterError::provider(
            AdapterErrorKind::Stream,
            format!(
                "Provider '{}' has a Rust native adapter, but no streaming transport is configured",
                request.provider
            ),
            Some(request.provider),
        ))
    }

    fn stream_chunks(
        &self,
        request: NativeCompleteRequest,
    ) -> Result<NativeStreamChunkResponse, AdapterError> {
        self.stream(request)
            .map(NativeStreamChunkResponse::buffered)
    }
}
