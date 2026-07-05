use std::sync::Arc;

use crate::client::ProviderAdapter;
use crate::errors::{AdapterError, AdapterErrorKind};
use crate::events::StreamEvents;
use crate::request::{Request, Response};

use super::common::{
    native_stream_chunk_error_response, normalize_provider, provider_status_error,
    validate_native_provider,
};
use super::dispatcher::{
    build_native_complete_request, build_native_stream_request,
    translate_native_complete_response_with_headers, translate_native_stream_response,
};
use super::types::{
    NativeCompleteRequest, NativeCompleteResponse, NativeCompleteTransport, NativeRequestConfig,
    NativeStreamChunkResponse,
};

#[derive(Clone)]
pub struct NativeProviderAdapter {
    provider: String,
    config: NativeRequestConfig,
    transport: Arc<dyn NativeCompleteTransport>,
}

impl NativeProviderAdapter {
    pub fn new(
        provider: impl AsRef<str>,
        config: impl Into<NativeRequestConfig>,
        transport: Arc<dyn NativeCompleteTransport>,
    ) -> Result<Self, AdapterError> {
        let provider = normalize_provider(provider.as_ref());
        validate_native_provider(&provider)?;
        let config = config.into();
        config.timeout.validate().map_err(|message| {
            AdapterError::provider(
                AdapterErrorKind::Configuration,
                message,
                Some(provider.clone()),
            )
        })?;
        Ok(Self {
            provider,
            config,
            transport,
        })
    }

    pub fn without_transport(
        provider: impl AsRef<str>,
        config: impl Into<NativeRequestConfig>,
    ) -> Result<Self, AdapterError> {
        let provider = normalize_provider(provider.as_ref());
        validate_native_provider(&provider)?;
        let config = config.into();
        config.timeout.validate().map_err(|message| {
            AdapterError::provider(
                AdapterErrorKind::Configuration,
                message,
                Some(provider.clone()),
            )
        })?;
        Ok(Self {
            transport: Arc::new(MissingNativeCompleteTransport {
                provider: provider.clone(),
            }),
            provider,
            config,
        })
    }

    pub fn openai(
        config: impl Into<NativeRequestConfig>,
        transport: Arc<dyn NativeCompleteTransport>,
    ) -> Self {
        Self::new("openai", config, transport)
            .expect("openai is a supported native provider adapter")
    }

    pub fn anthropic(
        config: impl Into<NativeRequestConfig>,
        transport: Arc<dyn NativeCompleteTransport>,
    ) -> Self {
        Self::new("anthropic", config, transport)
            .expect("anthropic is a supported native provider adapter")
    }

    pub fn gemini(
        config: impl Into<NativeRequestConfig>,
        transport: Arc<dyn NativeCompleteTransport>,
    ) -> Self {
        Self::new("gemini", config, transport)
            .expect("gemini is a supported native provider adapter")
    }
}

impl ProviderAdapter for NativeProviderAdapter {
    fn name(&self) -> &str {
        &self.provider
    }

    fn complete(&self, request: Request) -> Result<Response, AdapterError> {
        let native_request =
            build_native_complete_request(&self.provider, &request, self.config.clone())?;
        let native_response = self.transport.complete(native_request)?;
        if !(200..=299).contains(&native_response.status) {
            return Err(provider_status_error(&self.provider, native_response));
        }
        translate_native_complete_response_with_headers(
            &self.provider,
            native_response.body,
            &native_response.headers,
        )
    }

    fn stream(&self, request: Request) -> Result<StreamEvents, AdapterError> {
        let native_request =
            build_native_stream_request(&self.provider, &request, self.config.clone())?;
        let native_response = self.transport.stream_chunks(native_request)?;
        if !(200..=299).contains(&native_response.status) {
            return Err(provider_status_error(
                &self.provider,
                native_stream_chunk_error_response(native_response),
            ));
        }
        let NativeStreamChunkResponse { headers, body, .. } = native_response;
        Ok(translate_native_stream_response(
            &self.provider,
            body,
            &headers,
        ))
    }

    fn supports_tool_choice(&self, mode: &str) -> bool {
        matches!(
            mode.trim().to_ascii_lowercase().as_str(),
            "auto" | "none" | "required" | "named"
        )
    }
}

#[derive(Debug, Clone)]
struct MissingNativeCompleteTransport {
    provider: String,
}

impl NativeCompleteTransport for MissingNativeCompleteTransport {
    fn complete(
        &self,
        _request: NativeCompleteRequest,
    ) -> Result<NativeCompleteResponse, AdapterError> {
        Err(AdapterError::provider(
            AdapterErrorKind::Configuration,
            format!(
                "Provider '{}' has a Rust native adapter, but no HTTP transport is configured",
                self.provider
            ),
            Some(self.provider.clone()),
        ))
    }
}
