use std::collections::BTreeMap;
use std::error::Error as _;
use std::time::{Duration, Instant};

use reqwest::blocking::{Client as BlockingReqwestClient, Response as BlockingReqwestResponse};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};
use reqwest::{Client as AsyncReqwestClient, Method, Response as AsyncReqwestResponse};
use serde_json::{json, Value};
use tokio::runtime::{Builder as RuntimeBuilder, Runtime};

use crate::errors::{retry_after_from_headers, AdapterError, AdapterErrorKind};
use crate::native::{
    NativeCompleteRequest, NativeCompleteResponse, NativeCompleteTransport, NativeStreamBody,
    NativeStreamChunkResponse, NativeStreamResponse,
};
use crate::timeouts::AdapterTimeout;

#[derive(Debug, Clone, Default)]
pub struct NativeHttpTransport;

impl NativeHttpTransport {
    pub fn new() -> Self {
        Self
    }
}

impl NativeCompleteTransport for NativeHttpTransport {
    fn complete(
        &self,
        request: NativeCompleteRequest,
    ) -> Result<NativeCompleteResponse, AdapterError> {
        execute_complete_request(request)
    }

    fn stream(&self, request: NativeCompleteRequest) -> Result<NativeStreamResponse, AdapterError> {
        Ok(execute_stream_request(request)?.into_buffered())
    }

    fn stream_chunks(
        &self,
        request: NativeCompleteRequest,
    ) -> Result<NativeStreamChunkResponse, AdapterError> {
        execute_stream_request(request)
    }
}

fn execute_complete_request(
    request: NativeCompleteRequest,
) -> Result<NativeCompleteResponse, AdapterError> {
    request.timeout.validate().map_err(|message| {
        AdapterError::provider(
            AdapterErrorKind::Configuration,
            message,
            Some(request.provider.clone()),
        )
    })?;

    let method = Method::from_bytes(request.method.as_bytes()).map_err(|error| {
        AdapterError::provider(
            AdapterErrorKind::Configuration,
            format!("Invalid HTTP method {:?}: {error}", request.method),
            Some(request.provider.clone()),
        )
    })?;
    let client = build_client(request.timeout, &request.provider)?;
    let headers = header_map(&request)?;
    let body = request_body(&request)?;

    let response = client
        .request(method, &request.url)
        .headers(headers)
        .timeout(seconds_duration(request.timeout.request))
        .body(body)
        .send()
        .map_err(|error| transport_error(&request, error))?;

    complete_response(&request.provider, response)
}

fn execute_stream_request(
    request: NativeCompleteRequest,
) -> Result<NativeStreamChunkResponse, AdapterError> {
    request.timeout.validate().map_err(|message| {
        AdapterError::provider(
            AdapterErrorKind::Configuration,
            message,
            Some(request.provider.clone()),
        )
    })?;

    let method = Method::from_bytes(request.method.as_bytes()).map_err(|error| {
        AdapterError::provider(
            AdapterErrorKind::Configuration,
            format!("Invalid HTTP method {:?}: {error}", request.method),
            Some(request.provider.clone()),
        )
    })?;
    let client = build_stream_client(request.timeout, &request.provider)?;
    let runtime = build_stream_runtime(&request.provider)?;
    let headers = header_map(&request)?;
    let body = request_body(&request)?;

    let response = runtime
        .block_on(async {
            client
                .request(method, &request.url)
                .headers(headers)
                .timeout(seconds_duration(request.timeout.request))
                .body(body)
                .send()
                .await
        })
        .map_err(|error| transport_error(&request, error))?;

    let status = response.status().as_u16();
    if !(200..=299).contains(&status) {
        let response = runtime.block_on(async_complete_response(&request.provider, response))?;
        return Ok(NativeStreamChunkResponse::buffered(NativeStreamResponse {
            status: response.status,
            headers: response.headers,
            body: vec![Ok(response.body)],
        }));
    }

    let headers = response_headers(response.headers());
    let body: NativeStreamBody = Box::new(HttpStreamBody {
        runtime,
        provider: request.provider,
        method: request.method,
        url: request.url,
        timeout: request.timeout,
        status,
        headers: headers.clone(),
        response: Some(response),
    });
    Ok(NativeStreamChunkResponse::new(status, headers, body))
}

fn request_body(request: &NativeCompleteRequest) -> Result<Vec<u8>, AdapterError> {
    serde_json::to_vec(&request.body).map_err(|error| {
        AdapterError::provider(
            AdapterErrorKind::Configuration,
            format!("Unable to serialize provider request body: {error}"),
            Some(request.provider.clone()),
        )
    })
}

fn build_client(
    timeout: AdapterTimeout,
    provider: &str,
) -> Result<BlockingReqwestClient, AdapterError> {
    BlockingReqwestClient::builder()
        .connect_timeout(seconds_duration(timeout.connect))
        .build()
        .map_err(|error| {
            AdapterError::provider(
                AdapterErrorKind::Configuration,
                format!("Unable to build HTTP transport: {error}"),
                Some(provider.to_string()),
            )
        })
}

fn build_stream_client(
    timeout: AdapterTimeout,
    provider: &str,
) -> Result<AsyncReqwestClient, AdapterError> {
    AsyncReqwestClient::builder()
        .connect_timeout(seconds_duration(timeout.connect))
        .read_timeout(seconds_duration(timeout.stream_read))
        .build()
        .map_err(|error| {
            AdapterError::provider(
                AdapterErrorKind::Configuration,
                format!("Unable to build HTTP streaming transport: {error}"),
                Some(provider.to_string()),
            )
        })
}

fn build_stream_runtime(provider: &str) -> Result<Runtime, AdapterError> {
    RuntimeBuilder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            AdapterError::provider(
                AdapterErrorKind::Configuration,
                format!("Unable to build HTTP streaming runtime: {error}"),
                Some(provider.to_string()),
            )
        })
}

fn header_map(request: &NativeCompleteRequest) -> Result<HeaderMap, AdapterError> {
    let mut headers = HeaderMap::new();
    for (name, value) in &request.headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
            AdapterError::provider(
                AdapterErrorKind::Configuration,
                format!("Invalid HTTP header name {name:?}: {error}"),
                Some(request.provider.clone()),
            )
        })?;
        let header_value = HeaderValue::from_str(value).map_err(|error| {
            AdapterError::provider(
                AdapterErrorKind::Configuration,
                format!("Invalid HTTP header value for {name:?}: {error}"),
                Some(request.provider.clone()),
            )
        })?;
        headers.insert(header_name, header_value);
    }
    if !headers.contains_key(CONTENT_TYPE) {
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    }
    Ok(headers)
}

fn complete_response(
    provider: &str,
    response: BlockingReqwestResponse,
) -> Result<NativeCompleteResponse, AdapterError> {
    let status = response.status().as_u16();
    let headers = response_headers(response.headers());
    let body_bytes = response.bytes().map_err(|error| {
        let mut adapter_error = AdapterError::provider(
            AdapterErrorKind::Network,
            format!("Unable to read provider response body: {error}"),
            Some(provider.to_string()),
        );
        adapter_error.status_code = Some(status);
        adapter_error.retry_after = retry_after_from_headers(&headers);
        adapter_error.raw = Some(json!({
            "status": status,
            "headers": headers.clone(),
        }));
        adapter_error
    })?;
    match serde_json::from_slice::<Value>(&body_bytes) {
        Ok(body) => Ok(NativeCompleteResponse {
            status,
            headers,
            body,
        }),
        Err(_) if !(200..=299).contains(&status) => Ok(NativeCompleteResponse {
            status,
            headers,
            body: Value::String(String::from_utf8_lossy(&body_bytes).into_owned()),
        }),
        Err(error) => {
            let body = String::from_utf8_lossy(&body_bytes).into_owned();
            let mut adapter_error = AdapterError::provider(
                AdapterErrorKind::Provider,
                format!("Provider '{provider}' returned malformed JSON: {error}"),
                Some(provider.to_string()),
            );
            adapter_error.status_code = Some(status);
            adapter_error.retry_after = retry_after_from_headers(&headers);
            adapter_error.raw = Some(json!({
                "status": status,
                "headers": headers,
                "body": body,
            }));
            Err(adapter_error)
        }
    }
}

async fn async_complete_response(
    provider: &str,
    response: AsyncReqwestResponse,
) -> Result<NativeCompleteResponse, AdapterError> {
    let status = response.status().as_u16();
    let headers = response_headers(response.headers());
    let body_bytes = response.bytes().await.map_err(|error| {
        let mut adapter_error = AdapterError::provider(
            AdapterErrorKind::Network,
            format!("Unable to read provider response body: {error}"),
            Some(provider.to_string()),
        );
        adapter_error.status_code = Some(status);
        adapter_error.retry_after = retry_after_from_headers(&headers);
        adapter_error.raw = Some(json!({
            "status": status,
            "headers": headers.clone(),
        }));
        adapter_error
    })?;
    match serde_json::from_slice::<Value>(&body_bytes) {
        Ok(body) => Ok(NativeCompleteResponse {
            status,
            headers,
            body,
        }),
        Err(_) if !(200..=299).contains(&status) => Ok(NativeCompleteResponse {
            status,
            headers,
            body: Value::String(String::from_utf8_lossy(&body_bytes).into_owned()),
        }),
        Err(error) => {
            let body = String::from_utf8_lossy(&body_bytes).into_owned();
            let mut adapter_error = AdapterError::provider(
                AdapterErrorKind::Provider,
                format!("Provider '{provider}' returned malformed JSON: {error}"),
                Some(provider.to_string()),
            );
            adapter_error.status_code = Some(status);
            adapter_error.retry_after = retry_after_from_headers(&headers);
            adapter_error.raw = Some(json!({
                "status": status,
                "headers": headers,
                "body": body,
            }));
            Err(adapter_error)
        }
    }
}

fn response_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    let mut normalized = BTreeMap::new();
    for (name, value) in headers {
        let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
        normalized
            .entry(name.as_str().to_string())
            .and_modify(|existing: &mut String| {
                existing.push(',');
                existing.push_str(&value);
            })
            .or_insert(value);
    }
    normalized
}

fn transport_error(request: &NativeCompleteRequest, error: reqwest::Error) -> AdapterError {
    let is_timeout = error.is_timeout();
    let kind = if is_timeout {
        AdapterErrorKind::RequestTimeout
    } else {
        AdapterErrorKind::Network
    };
    let scope = if is_timeout && error.is_connect() {
        "connect"
    } else if is_timeout {
        "request"
    } else {
        "network"
    };
    let timeout = match scope {
        "connect" => Some(request.timeout.connect),
        "request" => Some(request.timeout.request),
        _ => None,
    };
    let mut adapter_error = AdapterError::provider(
        kind,
        format!(
            "Provider '{}' HTTP {scope} failed: {error}",
            request.provider
        ),
        Some(request.provider.clone()),
    );
    adapter_error.raw = Some(json!({
        "scope": scope,
        "method": request.method.clone(),
        "url": request.url.clone(),
        "timeout": timeout,
        "error": error.to_string(),
    }));
    adapter_error
}

struct HttpStreamBody {
    runtime: Runtime,
    provider: String,
    method: String,
    url: String,
    timeout: AdapterTimeout,
    status: u16,
    headers: BTreeMap<String, String>,
    response: Option<AsyncReqwestResponse>,
}

impl Iterator for HttpStreamBody {
    type Item = Result<Value, AdapterError>;

    fn next(&mut self) -> Option<Self::Item> {
        let response = self.response.as_mut()?;
        let read_started_at = Instant::now();
        match self.runtime.block_on(async { response.chunk().await }) {
            Ok(Some(chunk)) => Some(Ok(Value::String(
                String::from_utf8_lossy(&chunk).into_owned(),
            ))),
            Ok(None) => {
                self.response = None;
                None
            }
            Err(error) => {
                self.response = None;
                Some(Err(stream_read_error(
                    self,
                    error,
                    read_started_at.elapsed(),
                )))
            }
        }
    }
}

fn stream_read_error(
    stream: &HttpStreamBody,
    error: reqwest::Error,
    elapsed: Duration,
) -> AdapterError {
    let error_message = error.to_string();
    let source_message = error.source().map(ToString::to_string);
    let exceeded_stream_read_timeout = elapsed >= seconds_duration(stream.timeout.stream_read);
    let is_timeout = exceeded_stream_read_timeout || error.is_timeout() || {
        let message = format!(
            "{} {}",
            error_message,
            source_message.as_deref().unwrap_or_default()
        )
        .to_ascii_lowercase();
        message.contains("timed out") || message.contains("timeout") || message.contains("deadline")
    };
    let kind = if is_timeout {
        AdapterErrorKind::RequestTimeout
    } else {
        AdapterErrorKind::Network
    };
    let mut adapter_error = AdapterError::provider(
        kind,
        format!(
            "Provider '{}' HTTP stream_read failed: {error}",
            stream.provider
        ),
        Some(stream.provider.clone()),
    );
    adapter_error.status_code = Some(stream.status);
    adapter_error.retry_after = retry_after_from_headers(&stream.headers);
    adapter_error.raw = Some(json!({
        "scope": "stream_read",
        "method": stream.method.clone(),
        "url": stream.url.clone(),
        "timeout": stream.timeout.stream_read,
        "status": stream.status,
        "headers": stream.headers.clone(),
        "error": error_message,
        "source": source_message,
    }));
    adapter_error
}

fn seconds_duration(seconds: f64) -> Duration {
    Duration::from_secs_f64(seconds)
}
