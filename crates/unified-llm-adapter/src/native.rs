use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::client::ProviderAdapter;
use crate::env::ProviderConfig;
use crate::errors::{
    error_from_status_code, extract_error_details_from_raw, retry_after_from_headers, AdapterError,
    AdapterErrorKind,
};
use crate::events::{
    merge_stream_usage, stream_events, StreamAccumulator, StreamEvent, StreamEventType,
    StreamEvents,
};
use crate::provider_utils::{ProviderStreamPayloadError, ProviderStreamRecord, SseParser};
use crate::request::{
    ContentPart, FinishReason, FinishReasonKind, ImageData, Message, MessageRole, RateLimitInfo,
    Request, Response, ResponseFormat, ThinkingData, ToolCall, ToolResultData,
};
use crate::usage::Usage;

const OPENAI_DEFAULT_BASE_URL: &str = "https://api.openai.com";
const ANTHROPIC_DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const ANTHROPIC_PROMPT_CACHING_BETA: &str = "prompt-caching-2024-07-31";
const ANTHROPIC_MAX_CACHE_BREAKPOINTS: usize = 4;
const GEMINI_DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com";
const OPENAI_RESPONSES_OPTION_KEYS: &[&str] = &[
    "background",
    "conversation",
    "include",
    "instructions",
    "input",
    "max_output_tokens",
    "max_tool_calls",
    "metadata",
    "model",
    "parallel_tool_calls",
    "previous_response_id",
    "prompt",
    "reasoning",
    "response_format",
    "safety_identifier",
    "service_tier",
    "store",
    "stream",
    "stream_options",
    "temperature",
    "text",
    "tool_choice",
    "tools",
    "top_logprobs",
    "top_p",
    "truncation",
    "user",
];
const GEMINI_BODY_OPTION_KEYS: &[&str] = &[
    "cachedContent",
    "groundingConfig",
    "labels",
    "safetySettings",
    "systemInstruction",
    "toolConfig",
];
const GEMINI_GENERATION_OPTION_KEYS: &[&str] = &[
    "candidateCount",
    "frequencyPenalty",
    "logprobs",
    "maxOutputTokens",
    "mediaResolution",
    "presencePenalty",
    "responseLogprobs",
    "responseMimeType",
    "responseSchema",
    "seed",
    "stopSequences",
    "temperature",
    "thinkingConfig",
    "topK",
    "topP",
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
}

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
        Ok(Self {
            provider,
            config: config.into(),
            transport,
        })
    }

    pub fn without_transport(
        provider: impl AsRef<str>,
        config: impl Into<NativeRequestConfig>,
    ) -> Result<Self, AdapterError> {
        let provider = normalize_provider(provider.as_ref());
        validate_native_provider(&provider)?;
        Ok(Self {
            transport: Arc::new(MissingNativeCompleteTransport {
                provider: provider.clone(),
            }),
            provider,
            config: config.into(),
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
        let native_response = self.transport.stream(native_request)?;
        if !(200..=299).contains(&native_response.status) {
            return Err(provider_status_error(
                &self.provider,
                native_stream_error_response(native_response),
            ));
        }
        Ok(translate_native_stream_response(
            &self.provider,
            native_response.body,
            &native_response.headers,
        ))
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

pub fn build_native_complete_request<C>(
    provider: &str,
    request: &Request,
    config: C,
) -> Result<NativeCompleteRequest, AdapterError>
where
    C: Into<NativeRequestConfig>,
{
    match normalize_provider(provider).as_str() {
        "openai" => build_openai_responses_request(request, config),
        "anthropic" => build_anthropic_messages_request(request, config),
        "gemini" => build_gemini_generate_content_request(request, config),
        other => Err(configuration_error(
            other,
            format!("Unsupported native provider {other:?}"),
        )),
    }
}

pub fn build_native_stream_request<C>(
    provider: &str,
    request: &Request,
    config: C,
) -> Result<NativeCompleteRequest, AdapterError>
where
    C: Into<NativeRequestConfig>,
{
    match normalize_provider(provider).as_str() {
        "openai" => build_openai_responses_stream_request(request, config),
        "anthropic" => build_anthropic_messages_stream_request(request, config),
        "gemini" => build_gemini_stream_generate_content_request(request, config),
        other => Err(configuration_error(
            other,
            format!("Unsupported native provider {other:?}"),
        )),
    }
}

pub fn build_openai_responses_request<C>(
    request: &Request,
    config: C,
) -> Result<NativeCompleteRequest, AdapterError>
where
    C: Into<NativeRequestConfig>,
{
    request.validate_for_client().map_err(|message| {
        AdapterError::provider(
            AdapterErrorKind::InvalidRequest,
            message,
            Some("openai".to_string()),
        )
    })?;
    let config = config.into();
    let mut headers = config.default_headers.clone();
    apply_bearer_auth(&mut headers, config.api_key.as_deref());
    if let Some(organization) = non_empty(config.organization.as_deref()) {
        headers.insert("OpenAI-Organization".to_string(), organization.to_string());
    }
    if let Some(project) = non_empty(config.project.as_deref()) {
        headers.insert("OpenAI-Project".to_string(), project.to_string());
    }

    let body = openai_responses_body(request)?;
    Ok(NativeCompleteRequest {
        provider: "openai".to_string(),
        method: "POST".to_string(),
        url: openai_responses_url(config.base_url.as_deref()),
        headers,
        body,
    })
}

pub fn build_openai_responses_stream_request<C>(
    request: &Request,
    config: C,
) -> Result<NativeCompleteRequest, AdapterError>
where
    C: Into<NativeRequestConfig>,
{
    let mut native_request = build_openai_responses_request(request, config)?;
    deep_insert(&mut native_request.body, json!({"stream": true}));
    Ok(native_request)
}

pub fn build_anthropic_messages_request<C>(
    request: &Request,
    config: C,
) -> Result<NativeCompleteRequest, AdapterError>
where
    C: Into<NativeRequestConfig>,
{
    request.validate_for_client().map_err(|message| {
        AdapterError::provider(
            AdapterErrorKind::InvalidRequest,
            message,
            Some("anthropic".to_string()),
        )
    })?;
    let config = config.into();
    let active_options = provider_options_object(request, "anthropic")?;
    let mut headers = config.default_headers.clone();
    remove_header_case_insensitive(&mut headers, "authorization");
    if let Some(api_key) = non_empty(config.api_key.as_deref()) {
        headers.insert("x-api-key".to_string(), api_key.to_string());
    }
    headers.insert(
        "anthropic-version".to_string(),
        ANTHROPIC_VERSION.to_string(),
    );
    let body = anthropic_messages_body(request, active_options)?;
    if let Some(beta_headers) =
        anthropic_beta_headers(active_options, value_contains_key(&body, "cache_control"))?
    {
        headers.insert("anthropic-beta".to_string(), beta_headers.join(","));
    }

    Ok(NativeCompleteRequest {
        provider: "anthropic".to_string(),
        method: "POST".to_string(),
        url: anthropic_messages_url(config.base_url.as_deref()),
        headers,
        body,
    })
}

pub fn build_anthropic_messages_stream_request<C>(
    request: &Request,
    config: C,
) -> Result<NativeCompleteRequest, AdapterError>
where
    C: Into<NativeRequestConfig>,
{
    let mut native_request = build_anthropic_messages_request(request, config)?;
    deep_insert(&mut native_request.body, json!({"stream": true}));
    Ok(native_request)
}

pub fn build_gemini_generate_content_request<C>(
    request: &Request,
    config: C,
) -> Result<NativeCompleteRequest, AdapterError>
where
    C: Into<NativeRequestConfig>,
{
    request.validate_for_client().map_err(|message| {
        AdapterError::provider(
            AdapterErrorKind::InvalidRequest,
            message,
            Some("gemini".to_string()),
        )
    })?;
    let config = config.into();
    let mut headers = config.default_headers.clone();
    remove_header_case_insensitive(&mut headers, "authorization");
    let mut url = gemini_generate_content_url(config.base_url.as_deref(), &request.model);
    if let Some(api_key) = non_empty(config.api_key.as_deref()) {
        url = append_query_pair(&url, "key", api_key);
    }

    Ok(NativeCompleteRequest {
        provider: "gemini".to_string(),
        method: "POST".to_string(),
        url,
        headers,
        body: gemini_generate_content_body(request)?,
    })
}

pub fn build_gemini_stream_generate_content_request<C>(
    request: &Request,
    config: C,
) -> Result<NativeCompleteRequest, AdapterError>
where
    C: Into<NativeRequestConfig>,
{
    request.validate_for_client().map_err(|message| {
        AdapterError::provider(
            AdapterErrorKind::InvalidRequest,
            message,
            Some("gemini".to_string()),
        )
    })?;
    let config = config.into();
    let mut headers = config.default_headers.clone();
    remove_header_case_insensitive(&mut headers, "authorization");
    let mut url = gemini_stream_generate_content_url(config.base_url.as_deref(), &request.model);
    url = append_query_pair(&url, "alt", "sse");
    if let Some(api_key) = non_empty(config.api_key.as_deref()) {
        url = append_query_pair(&url, "key", api_key);
    }

    Ok(NativeCompleteRequest {
        provider: "gemini".to_string(),
        method: "POST".to_string(),
        url,
        headers,
        body: gemini_generate_content_body(request)?,
    })
}

pub fn translate_native_complete_response(
    provider: &str,
    payload: Value,
) -> Result<Response, AdapterError> {
    translate_native_complete_response_with_headers(provider, payload, &BTreeMap::new())
}

pub fn translate_native_complete_response_with_headers(
    provider: &str,
    payload: Value,
    headers: &BTreeMap<String, String>,
) -> Result<Response, AdapterError> {
    let rate_limit = normalize_rate_limit_headers(headers);
    match normalize_provider(provider).as_str() {
        "openai" => translate_openai_responses_response(payload, rate_limit),
        "anthropic" => translate_anthropic_messages_response(payload, rate_limit),
        "gemini" => translate_gemini_generate_content_response(payload, rate_limit),
        other => Err(configuration_error(
            other,
            format!("Unsupported native provider {other:?}"),
        )),
    }
}

pub fn translate_native_stream_response(
    provider: &str,
    body: Vec<Result<Value, AdapterError>>,
    headers: &BTreeMap<String, String>,
) -> StreamEvents {
    let provider = normalize_provider(provider);
    let body = native_stream_records(body);
    let mut results = match provider.as_str() {
        "openai" => translate_openai_stream(body, headers),
        "anthropic" => translate_anthropic_stream(body, headers),
        "gemini" => translate_gemini_stream(body, headers),
        other => vec![Err(configuration_error(
            other,
            format!("Unsupported native provider {other:?}"),
        ))],
    };

    if results.is_empty() {
        let response = Response {
            provider,
            ..Response::default()
        };
        results.push(Ok(StreamEvent {
            response: Some(response),
            ..StreamEvent::finish(FinishReason::Other, Some(Usage::default()))
        }));
    }

    stream_events(results.into_iter())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ActiveStreamBlock {
    Text(String),
    Reasoning,
    ToolCall(String),
}

#[derive(Debug, Clone)]
struct NativeStreamState {
    provider: &'static str,
    rate_limit: Option<RateLimitInfo>,
    events: Vec<Result<StreamEvent, AdapterError>>,
    accumulator: StreamAccumulator,
    raw_payloads: Vec<Value>,
    started: bool,
    active_texts: BTreeMap<String, String>,
    active_reasoning: bool,
    active_tool_calls: BTreeMap<String, ToolCall>,
    active_tool_call_order: Vec<String>,
    next_text_id: usize,
    next_tool_call_id: usize,
    last_response: Option<Response>,
    finish_reason: Option<FinishReason>,
    usage: Option<Usage>,
}

impl NativeStreamState {
    fn new(provider: &'static str, headers: &BTreeMap<String, String>) -> Self {
        Self {
            provider,
            rate_limit: normalize_rate_limit_headers(headers),
            events: Vec::new(),
            accumulator: StreamAccumulator::default(),
            raw_payloads: Vec::new(),
            started: false,
            active_texts: BTreeMap::new(),
            active_reasoning: false,
            active_tool_calls: BTreeMap::new(),
            active_tool_call_order: Vec::new(),
            next_text_id: 0,
            next_tool_call_id: 0,
            last_response: None,
            finish_reason: None,
            usage: None,
        }
    }

    fn push(&mut self, event: StreamEvent) {
        self.accumulator.push(event.clone());
        self.events.push(Ok(event));
    }

    fn push_error(&mut self, error: AdapterError, raw: Option<Value>) {
        let response = self.current_response(FinishReason::Error);
        self.push(StreamEvent {
            r#type: StreamEventType::Error,
            finish_reason: Some(FinishReason::Error),
            usage: Some(response.usage.clone()),
            response: Some(response),
            error: Some(error),
            raw,
            ..StreamEvent::new(StreamEventType::Error)
        });
    }

    fn push_iterator_error(&mut self, error: AdapterError) {
        self.events.push(Err(error));
    }

    fn record_usage(&mut self, usage: Usage) {
        self.usage = merge_stream_usage(self.usage.take(), usage);
    }

    fn ensure_started(&mut self, raw: Option<Value>, response: Option<Response>) {
        if let Some(response) = response {
            self.record_usage(response.usage.clone());
            self.last_response = Some(response);
        }
        if self.started {
            return;
        }
        self.started = true;
        let response = self.last_response.clone().unwrap_or_else(|| Response {
            provider: self.provider.to_string(),
            rate_limit: self.rate_limit.clone(),
            ..Response::default()
        });
        self.push(StreamEvent {
            r#type: StreamEventType::StreamStart,
            response: Some(Response {
                raw: None,
                ..response
            }),
            raw,
            ..StreamEvent::new(StreamEventType::StreamStart)
        });
    }

    fn text_start(&mut self, text_id: Option<String>, raw: Option<Value>) {
        self.ensure_started(raw.clone(), None);
        self.close_reasoning(raw.clone());
        let text_id = self.resolve_text_id(text_id);
        if self.active_texts.contains_key(&text_id) {
            return;
        }
        self.active_texts.insert(text_id.clone(), String::new());
        self.push(StreamEvent {
            r#type: StreamEventType::TextStart,
            text_id: Some(text_id),
            raw,
            ..StreamEvent::new(StreamEventType::TextStart)
        });
    }

    fn text_delta(&mut self, text_id: Option<String>, delta: String, raw: Option<Value>) {
        if delta.is_empty() {
            return;
        }
        self.ensure_started(raw.clone(), None);
        self.close_reasoning(raw.clone());
        let text_id = self.resolve_text_id(text_id);
        if !self.active_texts.contains_key(&text_id) {
            self.active_texts.insert(text_id.clone(), String::new());
            self.push(StreamEvent {
                r#type: StreamEventType::TextStart,
                text_id: Some(text_id.clone()),
                raw: raw.clone(),
                ..StreamEvent::new(StreamEventType::TextStart)
            });
        }
        if let Some(active_text) = self.active_texts.get_mut(&text_id) {
            active_text.push_str(&delta);
        }
        self.push(StreamEvent {
            delta: Some(delta),
            text_id: Some(text_id),
            raw,
            ..StreamEvent::text_delta("")
        });
    }

    fn text_end(
        &mut self,
        text_id: Option<String>,
        final_text: Option<String>,
        raw: Option<Value>,
    ) {
        self.ensure_started(raw.clone(), None);
        let text_id = self.resolve_text_id(text_id);
        if !self.active_texts.contains_key(&text_id) {
            self.active_texts.insert(text_id.clone(), String::new());
            self.push(StreamEvent {
                r#type: StreamEventType::TextStart,
                text_id: Some(text_id.clone()),
                raw: raw.clone(),
                ..StreamEvent::new(StreamEventType::TextStart)
            });
        }
        if let Some(final_text) = final_text {
            let active = self.active_texts.get(&text_id).cloned().unwrap_or_default();
            let missing = if final_text == active || active.starts_with(&final_text) {
                String::new()
            } else if let Some(suffix) = final_text.strip_prefix(&active) {
                suffix.to_string()
            } else {
                final_text
            };
            if !missing.is_empty() {
                self.text_delta(Some(text_id.clone()), missing, raw.clone());
            }
        }
        self.push(StreamEvent {
            r#type: StreamEventType::TextEnd,
            text_id: Some(text_id.clone()),
            raw,
            ..StreamEvent::new(StreamEventType::TextEnd)
        });
        self.active_texts.remove(&text_id);
    }

    fn reasoning_delta(&mut self, delta: String, raw: Option<Value>) {
        self.reasoning_delta_with_metadata(delta, None, raw);
    }

    fn reasoning_delta_with_metadata(
        &mut self,
        delta: String,
        thinking: Option<ThinkingData>,
        raw: Option<Value>,
    ) {
        if delta.is_empty() && thinking.is_none() {
            return;
        }
        self.ensure_started(raw.clone(), None);
        self.close_all_text(raw.clone());
        self.close_all_tool_calls(raw.clone());
        if !self.active_reasoning {
            self.reasoning_start_with_metadata(thinking.clone(), raw.clone());
        }
        if delta.is_empty() {
            self.push(StreamEvent {
                r#type: StreamEventType::ReasoningDelta,
                thinking,
                raw,
                ..StreamEvent::new(StreamEventType::ReasoningDelta)
            });
            return;
        }
        self.push(StreamEvent {
            reasoning_delta: Some(delta),
            thinking,
            raw,
            ..StreamEvent::new(StreamEventType::ReasoningDelta)
        });
    }

    fn reasoning_start_with_metadata(
        &mut self,
        thinking: Option<ThinkingData>,
        raw: Option<Value>,
    ) {
        self.ensure_started(raw.clone(), None);
        self.close_all_text(raw.clone());
        self.close_all_tool_calls(raw.clone());
        if self.active_reasoning {
            if thinking.is_some() {
                self.push(StreamEvent {
                    r#type: StreamEventType::ReasoningDelta,
                    thinking,
                    raw,
                    ..StreamEvent::new(StreamEventType::ReasoningDelta)
                });
            }
            return;
        }
        self.active_reasoning = true;
        self.push(StreamEvent {
            r#type: StreamEventType::ReasoningStart,
            thinking,
            raw,
            ..StreamEvent::new(StreamEventType::ReasoningStart)
        });
    }

    fn tool_call_start(&mut self, tool_call: ToolCall, raw: Option<Value>) {
        self.ensure_started(raw.clone(), None);
        self.close_reasoning(raw.clone());
        let key = self.resolve_tool_call_id(&tool_call, false);
        let mut tool_call = tool_call;
        if tool_call.id.is_empty() {
            tool_call.id = key.clone();
        }
        if self.active_tool_calls.contains_key(&key) {
            let merged = merge_tool_calls_for_stream(
                self.active_tool_calls.remove(&key),
                tool_call.clone(),
                false,
            );
            self.active_tool_calls.insert(key, merged);
            return;
        }
        self.active_tool_call_order.push(key.clone());
        self.active_tool_calls.insert(key, tool_call.clone());
        self.push(StreamEvent {
            r#type: StreamEventType::ToolCallStart,
            tool_call: Some(tool_call),
            raw,
            ..StreamEvent::new(StreamEventType::ToolCallStart)
        });
    }

    fn tool_call_delta(&mut self, tool_call: ToolCall, raw: Option<Value>) {
        self.ensure_started(raw.clone(), None);
        self.close_reasoning(raw.clone());
        let key = self.resolve_tool_call_id(&tool_call, false);
        if !self.active_tool_calls.contains_key(&key) {
            let mut started = tool_call.clone();
            if started.id.is_empty() {
                started.id = key.clone();
            }
            started.arguments = Value::String(String::new());
            started.raw_arguments = Some(String::new());
            self.active_tool_call_order.push(key.clone());
            self.active_tool_calls.insert(key.clone(), started.clone());
            self.push(StreamEvent {
                r#type: StreamEventType::ToolCallStart,
                tool_call: Some(started),
                raw: raw.clone(),
                ..StreamEvent::new(StreamEventType::ToolCallStart)
            });
        }
        let mut tool_call = tool_call;
        if tool_call.id.is_empty() {
            tool_call.id = key.clone();
        }
        let merged = merge_tool_calls_for_stream(
            self.active_tool_calls.remove(&key),
            tool_call.clone(),
            false,
        );
        self.active_tool_calls.insert(key, merged);
        self.push(StreamEvent {
            r#type: StreamEventType::ToolCallDelta,
            tool_call: Some(tool_call),
            raw,
            ..StreamEvent::new(StreamEventType::ToolCallDelta)
        });
    }

    fn tool_call_end(&mut self, tool_call: Option<ToolCall>, raw: Option<Value>) {
        self.ensure_started(raw.clone(), None);
        self.close_reasoning(raw.clone());
        let key = tool_call
            .as_ref()
            .map(|incoming| self.resolve_tool_call_id(incoming, true))
            .or_else(|| self.only_active_tool_call_id());
        let Some(key) = key else {
            return;
        };
        let final_tool_call = match (self.active_tool_calls.remove(&key), tool_call) {
            (Some(current), Some(mut incoming)) => {
                if incoming.id.is_empty() {
                    incoming.id = key.clone();
                }
                merge_tool_calls_for_stream(Some(current), incoming, true)
            }
            (Some(current), None) => current,
            (None, Some(mut incoming)) => {
                if incoming.id.is_empty() {
                    incoming.id = key.clone();
                }
                incoming
            }
            (None, None) => return,
        };
        self.active_tool_call_order
            .retain(|active_key| active_key != &key);
        self.push(StreamEvent {
            r#type: StreamEventType::ToolCallEnd,
            tool_call: Some(final_tool_call),
            raw,
            ..StreamEvent::new(StreamEventType::ToolCallEnd)
        });
    }

    fn provider_event(&mut self, raw: Value) {
        self.ensure_started(Some(raw.clone()), None);
        self.push(StreamEvent::provider_event(raw));
    }

    fn close_reasoning(&mut self, raw: Option<Value>) {
        if self.active_reasoning {
            self.active_reasoning = false;
            self.push(StreamEvent {
                r#type: StreamEventType::ReasoningEnd,
                raw,
                ..StreamEvent::new(StreamEventType::ReasoningEnd)
            });
        }
    }

    fn close_all_text(&mut self, raw: Option<Value>) {
        let text_ids = self.active_texts.keys().cloned().collect::<Vec<_>>();
        for text_id in text_ids {
            self.text_end(Some(text_id), None, raw.clone());
        }
    }

    fn close_all_tool_calls(&mut self, raw: Option<Value>) {
        let tool_call_ids = std::mem::take(&mut self.active_tool_call_order);
        for tool_call_id in tool_call_ids {
            self.tool_call_end_by_id(tool_call_id, raw.clone());
        }
        let remaining = self.active_tool_calls.keys().cloned().collect::<Vec<_>>();
        for tool_call_id in remaining {
            self.tool_call_end_by_id(tool_call_id, raw.clone());
        }
    }

    fn tool_call_end_by_id(&mut self, tool_call_id: String, raw: Option<Value>) {
        if let Some(tool_call) = self.active_tool_calls.remove(&tool_call_id) {
            self.push(StreamEvent {
                r#type: StreamEventType::ToolCallEnd,
                tool_call: Some(tool_call),
                raw,
                ..StreamEvent::new(StreamEventType::ToolCallEnd)
            });
        }
    }

    fn resolve_text_id(&mut self, text_id: Option<String>) -> String {
        if let Some(text_id) = text_id.filter(|text_id| !text_id.is_empty()) {
            return text_id;
        }
        if self.active_texts.len() == 1 {
            if let Some(text_id) = self.active_texts.keys().next() {
                return text_id.clone();
            }
        }
        let text_id = format!("text_{}", self.next_text_id);
        self.next_text_id += 1;
        text_id
    }

    fn resolve_tool_call_id(&mut self, tool_call: &ToolCall, final_fragment: bool) -> String {
        if !tool_call.id.is_empty() && self.active_tool_calls.contains_key(&tool_call.id) {
            return tool_call.id.clone();
        }
        if final_fragment {
            if let Some(tool_call_id) = self.only_active_tool_call_id() {
                return tool_call_id;
            }
        }
        if !tool_call.id.is_empty() {
            return tool_call.id.clone();
        }
        if let Some(tool_call_id) = self.only_active_tool_call_id() {
            return tool_call_id;
        }
        let tool_call_id = format!("tool_call_{}", self.next_tool_call_id);
        self.next_tool_call_id += 1;
        tool_call_id
    }

    fn only_active_tool_call_id(&self) -> Option<String> {
        (self.active_tool_calls.len() == 1)
            .then(|| self.active_tool_calls.keys().next().cloned())
            .flatten()
    }

    fn finish(mut self, raw: Option<Value>) -> Vec<Result<StreamEvent, AdapterError>> {
        self.ensure_started(raw.clone(), None);
        self.close_all_text(raw.clone());
        self.close_reasoning(raw.clone());
        self.close_all_tool_calls(raw.clone());
        let reason = self
            .finish_reason
            .clone()
            .or_else(|| {
                self.last_response
                    .as_ref()
                    .map(|response| response.finish_reason.clone())
            })
            .unwrap_or(FinishReason::Other);
        let response = self.current_response(reason);
        let usage = response.usage.clone();
        self.push(StreamEvent {
            finish_reason: Some(response.finish_reason.clone()),
            usage: Some(usage),
            response: Some(response),
            raw,
            ..StreamEvent::finish(FinishReason::Other, None)
        });
        self.events
    }

    fn current_response(&self, finish_reason: FinishReason) -> Response {
        let accumulated = self.accumulator.response.clone();
        let mut response = self
            .last_response
            .clone()
            .unwrap_or_else(|| accumulated.clone());
        response.provider = if response.provider.is_empty() {
            self.provider.to_string()
        } else {
            response.provider
        };
        response.finish_reason = finish_reason;
        let usage = self
            .last_response
            .as_ref()
            .map(|response| response.usage.clone())
            .and_then(|usage| merge_stream_usage(self.usage.clone(), usage))
            .or_else(|| self.usage.clone())
            .and_then(|usage| merge_stream_usage(Some(usage), accumulated.usage.clone()))
            .or_else(|| merge_stream_usage(None, accumulated.usage.clone()));
        response.usage = usage.unwrap_or_default().normalized();
        response.raw = stream_raw_payload(&self.raw_payloads);
        response.raw_provider_events = self.raw_payloads.clone();
        response.rate_limit = response.rate_limit.or_else(|| self.rate_limit.clone());
        if response.text().is_empty() && !accumulated.text().is_empty() {
            response.text = accumulated.text();
        }
        if response.message.content.is_empty() && !accumulated.message.content.is_empty() {
            response.message = accumulated.message.clone();
        }
        if response.tool_calls().is_empty() && !accumulated.tool_calls().is_empty() {
            response.tool_calls = accumulated.tool_calls();
        }
        if self.provider == "anthropic" && response.usage.reasoning_tokens.is_none() {
            if let Some(estimated_reasoning_tokens) =
                estimate_reasoning_tokens(&response.message.content)
            {
                response.usage.reasoning_tokens = Some(estimated_reasoning_tokens);
            }
        }
        response
    }
}

fn native_stream_records(
    body: Vec<Result<Value, AdapterError>>,
) -> Vec<Result<ProviderStreamRecord, AdapterError>> {
    let mut parser = SseParser::default();
    let mut json_buffer = String::new();
    let mut json_mode = false;
    let mut records = Vec::new();

    for item in body {
        match item {
            Ok(Value::String(chunk)) => {
                if json_mode || (!parser.has_pending_input() && looks_like_json_stream(&chunk)) {
                    records.extend(parser.finish().into_iter().map(Ok));
                    json_mode = true;
                    json_buffer.push_str(&chunk);
                    match parse_json_stream_records(&json_buffer) {
                        JsonStreamParse::Complete(parsed) => {
                            records.extend(parsed.into_iter().map(Ok));
                            json_buffer.clear();
                            json_mode = false;
                        }
                        JsonStreamParse::Incomplete => {}
                        JsonStreamParse::Malformed(error) => {
                            records.push(Ok(malformed_json_stream_record(
                                std::mem::take(&mut json_buffer),
                                error.message,
                            )));
                            json_mode = false;
                        }
                    }
                } else {
                    records.extend(parser.push_str(&chunk).into_iter().map(Ok));
                }
            }
            Ok(payload) => {
                flush_json_stream_buffer(&mut json_buffer, &mut json_mode, &mut records);
                records.extend(parser.finish().into_iter().map(Ok));
                records.push(Ok(ProviderStreamRecord::from_json(payload)));
            }
            Err(error) => {
                flush_json_stream_buffer(&mut json_buffer, &mut json_mode, &mut records);
                records.extend(parser.finish().into_iter().map(Ok));
                records.push(Err(error));
            }
        }
    }

    flush_json_stream_buffer(&mut json_buffer, &mut json_mode, &mut records);
    records.extend(parser.finish().into_iter().map(Ok));
    records
}

#[derive(Debug)]
enum JsonStreamParse {
    Complete(Vec<ProviderStreamRecord>),
    Incomplete,
    Malformed(ProviderStreamPayloadError),
}

fn looks_like_json_stream(chunk: &str) -> bool {
    matches!(
        chunk.trim_start().as_bytes().first(),
        Some(b'{') | Some(b'[')
    )
}

fn parse_json_stream_records(input: &str) -> JsonStreamParse {
    if input.trim().is_empty() {
        return JsonStreamParse::Complete(Vec::new());
    }

    let mut stream = serde_json::Deserializer::from_str(input).into_iter::<Value>();
    let mut records = Vec::new();
    while let Some(result) = stream.next() {
        match result {
            Ok(payload) => records.push(ProviderStreamRecord::from_json(payload)),
            Err(error) if error.is_eof() => return JsonStreamParse::Incomplete,
            Err(error) => {
                return JsonStreamParse::Malformed(ProviderStreamPayloadError {
                    message: error.to_string(),
                    raw: input.to_string(),
                });
            }
        }
    }

    if input[stream.byte_offset()..].trim().is_empty() {
        JsonStreamParse::Complete(records)
    } else {
        JsonStreamParse::Malformed(ProviderStreamPayloadError {
            message: "trailing data after JSON stream payload".to_string(),
            raw: input.to_string(),
        })
    }
}

fn flush_json_stream_buffer(
    json_buffer: &mut String,
    json_mode: &mut bool,
    records: &mut Vec<Result<ProviderStreamRecord, AdapterError>>,
) {
    if json_buffer.trim().is_empty() {
        json_buffer.clear();
        *json_mode = false;
        return;
    }

    match parse_json_stream_records(json_buffer) {
        JsonStreamParse::Complete(parsed) => records.extend(parsed.into_iter().map(Ok)),
        JsonStreamParse::Incomplete => records.push(Ok(malformed_json_stream_record(
            std::mem::take(json_buffer),
            "incomplete JSON stream payload".to_string(),
        ))),
        JsonStreamParse::Malformed(error) => records.push(Ok(malformed_json_stream_record(
            std::mem::take(json_buffer),
            error.message,
        ))),
    }
    json_buffer.clear();
    *json_mode = false;
}

fn malformed_json_stream_record(raw: String, message: String) -> ProviderStreamRecord {
    ProviderStreamRecord {
        event: None,
        sse_event: None,
        json_event: None,
        data: raw.clone(),
        retry: None,
        payload: None,
        payload_error: Some(ProviderStreamPayloadError { message, raw }),
        done: false,
    }
}

fn stream_record_payload(
    provider: &'static str,
    record: ProviderStreamRecord,
) -> Result<Value, AdapterError> {
    if let Some(payload) = record.payload.clone() {
        return Ok(payload);
    }

    let message = record
        .payload_error
        .as_ref()
        .map(|error| format!("Malformed provider stream payload: {}", error.message))
        .unwrap_or_else(|| "Provider stream event did not contain a JSON payload".to_string());
    let mut error = AdapterError::provider(
        AdapterErrorKind::Stream,
        message,
        Some(provider.to_string()),
    );
    error.raw = Some(serde_json::to_value(&record).unwrap_or_else(|_| {
        json!({
            "data": record.data,
            "event": record.event,
            "sse_event": record.sse_event,
            "done": record.done,
        })
    }));
    Err(error)
}

fn translate_openai_stream(
    body: Vec<Result<ProviderStreamRecord, AdapterError>>,
    headers: &BTreeMap<String, String>,
) -> Vec<Result<StreamEvent, AdapterError>> {
    let mut state = NativeStreamState::new("openai", headers);
    let mut tool_call_aliases = BTreeMap::new();

    for item in body {
        let record = match item {
            Ok(record) => record,
            Err(error) => {
                if state.started {
                    state.push_iterator_error(error);
                    return state.events;
                }
                return vec![Err(error)];
            }
        };
        if record.done {
            let raw = stream_raw_payload(&state.raw_payloads);
            return state.finish(raw);
        }
        let event_type = record.event.clone().unwrap_or_default();
        let payload = match stream_record_payload("openai", record) {
            Ok(payload) => payload,
            Err(error) => {
                if state.started {
                    state.push_iterator_error(error);
                    return state.events;
                }
                return vec![Err(error)];
            }
        };
        state.raw_payloads.push(payload.clone());
        let raw = Some(payload.clone());
        let event_type = if event_type.is_empty() {
            value_string(&payload, "type").unwrap_or_default()
        } else {
            event_type
        };

        if event_type == "response.created" || event_type == "response.in_progress" {
            let response = payload.get("response").cloned().and_then(|value| {
                translate_openai_responses_response(value, state.rate_limit.clone()).ok()
            });
            state.ensure_started(raw, response);
            continue;
        }

        if matches!(
            event_type.as_str(),
            "response.output_text.delta" | "response.text.delta" | "response.refusal.delta"
        ) {
            if let Some(delta) = value_string(&payload, "delta") {
                state.text_delta(openai_stream_item_id(&payload, "text"), delta, raw);
            }
            continue;
        }

        if matches!(
            event_type.as_str(),
            "response.output_text.done" | "response.text.done" | "response.refusal.done"
        ) {
            let final_text =
                value_string(&payload, "text").or_else(|| value_string(&payload, "delta"));
            state.text_end(openai_stream_item_id(&payload, "text"), final_text, raw);
            continue;
        }

        if event_type.contains("reasoning") && event_type.ends_with(".delta") {
            if let Some(delta) = value_string(&payload, "delta")
                .or_else(|| value_string(&payload, "text"))
                .or_else(|| value_string(&payload, "summary"))
            {
                state.reasoning_delta(delta, raw);
            }
            continue;
        }

        if event_type == "response.output_item.added" {
            if let Some(item) = payload.get("item") {
                if let Some(tool_call) = openai_stream_tool_call(item) {
                    remember_openai_stream_tool_call_aliases(
                        &mut tool_call_aliases,
                        &payload,
                        item,
                        &tool_call.id,
                    );
                    state.tool_call_start(tool_call, raw);
                } else {
                    state.provider_event(payload);
                }
            } else {
                state.provider_event(payload);
            }
            continue;
        }

        if event_type == "response.function_call_arguments.delta" {
            let id = openai_stream_tool_call_delta_id(&payload, &tool_call_aliases)
                .unwrap_or_else(|| "function_call".to_string());
            let name = value_string(&payload, "name").unwrap_or_default();
            let delta = value_string(&payload, "delta").unwrap_or_default();
            state.tool_call_delta(
                ToolCall {
                    id,
                    name,
                    arguments: Value::String(delta.clone()),
                    raw_arguments: Some(delta),
                    r#type: "function".to_string(),
                },
                raw,
            );
            continue;
        }

        if event_type == "response.output_item.done" {
            if let Some(item) = payload
                .get("item")
                .and_then(|item| openai_stream_tool_call(item).map(|tool_call| (item, tool_call)))
            {
                let (item, mut tool_call) = item;
                if let Some(id) =
                    openai_stream_tool_call_alias_id(&payload, Some(item), &tool_call_aliases)
                {
                    tool_call.id = id;
                }
                remember_openai_stream_tool_call_aliases(
                    &mut tool_call_aliases,
                    &payload,
                    item,
                    &tool_call.id,
                );
                state.tool_call_end(Some(tool_call), raw);
            } else if let Some((text_id, text)) =
                payload.get("item").and_then(openai_stream_text_item)
            {
                state.text_end(text_id, text, raw);
            } else {
                state.provider_event(payload);
            }
            continue;
        }

        if event_type == "response.completed" || event_type == "response.done" {
            if let Some(response_payload) = payload.get("response").cloned() {
                match translate_openai_responses_response(
                    response_payload,
                    state.rate_limit.clone(),
                ) {
                    Ok(response) => {
                        state.finish_reason = Some(response.finish_reason.clone());
                        state.record_usage(response.usage.clone());
                        state.last_response = Some(response);
                    }
                    Err(error) => {
                        state.push_error(error, raw);
                        return state.events;
                    }
                }
            }
            return state.finish(raw);
        }

        if event_type == "response.failed" || event_type == "error" {
            state.push_error(provider_payload_error("openai", &payload), raw);
            return state.events;
        }

        state.provider_event(payload);
    }

    let raw = stream_raw_payload(&state.raw_payloads);
    state.finish(raw)
}

fn translate_anthropic_stream(
    body: Vec<Result<ProviderStreamRecord, AdapterError>>,
    headers: &BTreeMap<String, String>,
) -> Vec<Result<StreamEvent, AdapterError>> {
    let mut state = NativeStreamState::new("anthropic", headers);
    let mut active_blocks: BTreeMap<String, ActiveStreamBlock> = BTreeMap::new();

    for item in body {
        let record = match item {
            Ok(record) => record,
            Err(error) => {
                if state.started {
                    state.push_iterator_error(error);
                    return state.events;
                }
                return vec![Err(error)];
            }
        };
        if record.done {
            let raw = stream_raw_payload(&state.raw_payloads);
            return state.finish(raw);
        }
        let event_type = record.event.clone().unwrap_or_default();
        let payload = match stream_record_payload("anthropic", record) {
            Ok(payload) => payload,
            Err(error) => {
                if state.started {
                    state.push_iterator_error(error);
                    return state.events;
                }
                return vec![Err(error)];
            }
        };
        state.raw_payloads.push(payload.clone());
        let raw = Some(payload.clone());
        let event_type = if event_type.is_empty() {
            value_string(&payload, "type").unwrap_or_default()
        } else {
            event_type
        };

        match event_type.as_str() {
            "message_start" => {
                let response = payload.get("message").cloned().and_then(|value| {
                    translate_anthropic_messages_response(value, state.rate_limit.clone()).ok()
                });
                state.ensure_started(raw, response);
            }
            "content_block_start" => {
                let Some(block) = payload.get("content_block") else {
                    state.provider_event(payload);
                    continue;
                };
                let block_index =
                    value_identifier(&payload, "index").unwrap_or_else(|| "0".to_string());
                let block_type = value_string(block, "type").unwrap_or_default();
                match block_type.as_str() {
                    "text" => {
                        let text_id = format!("text_{block_index}");
                        active_blocks.insert(
                            block_index.clone(),
                            ActiveStreamBlock::Text(text_id.clone()),
                        );
                        if let Some(text) = value_string(block, "text") {
                            state.text_delta(Some(text_id), text, raw);
                        } else {
                            state.text_start(Some(text_id), raw);
                        }
                    }
                    "thinking" | "redacted_thinking" => {
                        let redacted = block_type == "redacted_thinking";
                        let metadata = anthropic_stream_thinking_metadata(
                            block,
                            redacted,
                            state
                                .last_response
                                .as_ref()
                                .and_then(|response| source_model(&response.model)),
                        );
                        active_blocks.insert(block_index.clone(), ActiveStreamBlock::Reasoning);
                        if let Some(text) = value_string(block, "thinking")
                            .or_else(|| value_string(block, "text"))
                            .or_else(|| value_string(block, "data"))
                        {
                            state.reasoning_delta_with_metadata(text, Some(metadata), raw);
                        } else {
                            state.reasoning_start_with_metadata(Some(metadata), raw);
                        }
                    }
                    "tool_use" => {
                        let id = value_string(block, "id")
                            .unwrap_or_else(|| format!("toolu_{block_index}"));
                        active_blocks
                            .insert(block_index.clone(), ActiveStreamBlock::ToolCall(id.clone()));
                        let name = value_string(block, "name").unwrap_or_default();
                        let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                        let raw_arguments = if input.as_object().is_some_and(Map::is_empty) {
                            String::new()
                        } else {
                            json_compact(&input)
                        };
                        state.tool_call_start(
                            ToolCall {
                                id,
                                name,
                                raw_arguments: Some(raw_arguments),
                                arguments: input,
                                r#type: "function".to_string(),
                            },
                            raw,
                        );
                    }
                    _ => state.provider_event(payload),
                }
            }
            "content_block_delta" => {
                let Some(delta) = payload.get("delta") else {
                    state.provider_event(payload);
                    continue;
                };
                let block_index =
                    value_identifier(&payload, "index").unwrap_or_else(|| "0".to_string());
                match value_string(delta, "type").as_deref() {
                    Some("text_delta") => {
                        if let Some(text) = value_string(delta, "text") {
                            let text_id = active_blocks
                                .get(&block_index)
                                .and_then(|block| match block {
                                    ActiveStreamBlock::Text(text_id) => Some(text_id.clone()),
                                    _ => None,
                                })
                                .unwrap_or_else(|| format!("text_{block_index}"));
                            state.text_delta(Some(text_id), text, raw);
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(text) = value_string(delta, "thinking") {
                            state.reasoning_delta(text, raw);
                        }
                    }
                    Some("signature_delta") => {
                        if let Some(signature) = value_string(delta, "signature") {
                            state.reasoning_delta_with_metadata(
                                String::new(),
                                Some(ThinkingData {
                                    text: String::new(),
                                    signature: Some(signature),
                                    redacted: false,
                                    source_provider: Some("anthropic".to_string()),
                                    source_model: state
                                        .last_response
                                        .as_ref()
                                        .and_then(|response| source_model(&response.model)),
                                }),
                                raw,
                            );
                        }
                    }
                    Some("input_json_delta") => {
                        let partial = value_string(delta, "partial_json").unwrap_or_default();
                        let id = active_blocks
                            .get(&block_index)
                            .and_then(|block| match block {
                                ActiveStreamBlock::ToolCall(tool_call_id) => {
                                    Some(tool_call_id.clone())
                                }
                                _ => None,
                            })
                            .unwrap_or_else(|| format!("toolu_{block_index}"));
                        state.tool_call_delta(
                            ToolCall {
                                id,
                                name: String::new(),
                                arguments: Value::String(partial.clone()),
                                raw_arguments: Some(partial),
                                r#type: "function".to_string(),
                            },
                            raw,
                        );
                    }
                    _ => state.provider_event(payload),
                }
            }
            "content_block_stop" => {
                let block_index =
                    value_identifier(&payload, "index").unwrap_or_else(|| "0".to_string());
                match active_blocks.remove(&block_index) {
                    Some(ActiveStreamBlock::Text(text_id)) => {
                        state.text_end(Some(text_id), None, raw)
                    }
                    Some(ActiveStreamBlock::Reasoning) => state.close_reasoning(raw),
                    Some(ActiveStreamBlock::ToolCall(tool_call_id)) => {
                        state.tool_call_end_by_id(tool_call_id, raw)
                    }
                    None => {
                        state.close_all_text(raw.clone());
                        state.close_reasoning(raw.clone());
                        state.close_all_tool_calls(raw);
                    }
                }
            }
            "message_delta" => {
                if let Some(delta) = payload.get("delta") {
                    if let Some(stop_reason) = value_string(delta, "stop_reason") {
                        let has_tool_calls = !state.accumulator.tool_calls.is_empty()
                            || !state.active_tool_calls.is_empty();
                        state.finish_reason =
                            Some(anthropic_finish_reason(Some(stop_reason), has_tool_calls));
                    }
                }
                if let Some(usage) = payload.get("usage") {
                    state.record_usage(usage_from_anthropic(Some(usage)));
                }
            }
            "message_stop" => return state.finish(raw),
            "error" => {
                state.push_error(provider_payload_error("anthropic", &payload), raw);
                return state.events;
            }
            _ => state.provider_event(payload),
        }
    }

    let raw = stream_raw_payload(&state.raw_payloads);
    state.finish(raw)
}

fn translate_gemini_stream(
    body: Vec<Result<ProviderStreamRecord, AdapterError>>,
    headers: &BTreeMap<String, String>,
) -> Vec<Result<StreamEvent, AdapterError>> {
    let mut state = NativeStreamState::new("gemini", headers);
    let mut active_text = String::new();
    let mut active_reasoning = String::new();
    let mut emitted_tool_calls = BTreeSet::new();

    for item in body {
        let record = match item {
            Ok(record) => record,
            Err(error) => {
                if state.started {
                    state.push_iterator_error(error);
                    return state.events;
                }
                return vec![Err(error)];
            }
        };
        if record.done {
            let raw = stream_raw_payload(&state.raw_payloads);
            return state.finish(raw);
        }
        let payload = match stream_record_payload("gemini", record) {
            Ok(payload) => payload,
            Err(error) => {
                if state.started {
                    state.push_iterator_error(error);
                    return state.events;
                }
                return vec![Err(error)];
            }
        };
        for payload in gemini_stream_payloads(payload) {
            state.raw_payloads.push(payload.clone());
            let raw = Some(payload.clone());

            if payload.get("error").is_some() {
                state.push_error(provider_payload_error("gemini", &payload), raw);
                return state.events;
            }
            if !is_gemini_stream_payload(&payload) {
                state.provider_event(payload);
                continue;
            }

            let response = match translate_gemini_generate_content_response(
                payload.clone(),
                state.rate_limit.clone(),
            ) {
                Ok(response) => response,
                Err(error) => {
                    state.push_error(error, raw);
                    return state.events;
                }
            };
            state.finish_reason = Some(response.finish_reason.clone());
            state.record_usage(response.usage.clone());
            state.last_response = Some(Response {
                raw: None,
                ..response.clone()
            });
            state.ensure_started(
                raw.clone(),
                Some(Response {
                    raw: None,
                    ..response.clone()
                }),
            );

            let text = response.message.text();
            if !text.is_empty() {
                let delta = if text == active_text || active_text.starts_with(&text) {
                    String::new()
                } else if let Some(suffix) = text.strip_prefix(&active_text) {
                    suffix.to_string()
                } else {
                    text.clone()
                };
                if !delta.is_empty() {
                    state.text_delta(Some("text_0".to_string()), delta, raw.clone());
                }
                if text.starts_with(&active_text) {
                    active_text = text;
                } else {
                    active_text.push_str(&response.message.text());
                }
            }

            let reasoning = response.reasoning().unwrap_or_default();
            if !reasoning.is_empty() {
                let delta =
                    if reasoning == active_reasoning || active_reasoning.starts_with(&reasoning) {
                        String::new()
                    } else if let Some(suffix) = reasoning.strip_prefix(&active_reasoning) {
                        suffix.to_string()
                    } else {
                        reasoning.clone()
                    };
                if !delta.is_empty() {
                    state.reasoning_delta_with_metadata(
                        delta,
                        gemini_reasoning_metadata(&response),
                        raw.clone(),
                    );
                }
                if reasoning.starts_with(&active_reasoning) {
                    active_reasoning = reasoning;
                } else {
                    active_reasoning.push_str(&response.reasoning().unwrap_or_default());
                }
            }

            let tool_calls = response.tool_calls();
            if !tool_calls.is_empty() && state.active_texts.contains_key("text_0") {
                state.text_end(Some("text_0".to_string()), None, raw.clone());
            }

            for tool_call in tool_calls {
                let signature = format!(
                    "{}:{}:{}",
                    tool_call.id,
                    tool_call.name,
                    json_compact(&tool_call.arguments)
                );
                if emitted_tool_calls.insert(signature) {
                    state.tool_call_start(tool_call.clone(), raw.clone());
                    state.tool_call_end(Some(tool_call), raw.clone());
                }
            }

            if response_has_provider_content(&response) {
                state.provider_event(payload);
            }
        }
    }

    let raw = stream_raw_payload(&state.raw_payloads);
    state.finish(raw)
}

fn stream_raw_payload(payloads: &[Value]) -> Option<Value> {
    match payloads.len() {
        0 => None,
        1 => payloads.first().cloned(),
        _ => Some(Value::Array(payloads.to_vec())),
    }
}

fn gemini_stream_payloads(payload: Value) -> Vec<Value> {
    match payload {
        Value::Array(values) if values.iter().all(Value::is_object) => values,
        other => vec![other],
    }
}

fn is_gemini_stream_payload(payload: &Value) -> bool {
    payload.get("candidates").is_some()
        || payload.get("usageMetadata").is_some()
        || payload.get("responseId").is_some()
        || payload.get("modelVersion").is_some()
        || payload.get("model").is_some()
}

fn gemini_reasoning_metadata(response: &Response) -> Option<ThinkingData> {
    response.message.content.iter().find_map(|part| match part {
        ContentPart::Thinking { thinking } | ContentPart::RedactedThinking { thinking } => {
            let mut metadata = thinking.clone();
            metadata.text.clear();
            Some(metadata)
        }
        _ => None,
    })
}

fn response_has_provider_content(response: &Response) -> bool {
    response
        .message
        .content
        .iter()
        .any(|part| matches!(part, ContentPart::Provider { .. }))
}

fn value_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn value_identifier(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(|value| {
        value
            .as_str()
            .map(str::to_string)
            .or_else(|| value.as_u64().map(|value| value.to_string()))
            .or_else(|| value.as_i64().map(|value| value.to_string()))
    })
}

fn openai_stream_item_id(payload: &Value, prefix: &str) -> Option<String> {
    value_identifier(payload, "item_id")
        .or_else(|| value_identifier(payload, "id"))
        .or_else(|| {
            value_identifier(payload, "output_index").map(|index| format!("{prefix}_{index}"))
        })
}

fn openai_stream_text_item(value: &Value) -> Option<(Option<String>, Option<String>)> {
    let object = value.as_object()?;
    let item_type = string_field(object, "type").unwrap_or_default();
    if !matches!(
        item_type.as_str(),
        "output_text" | "message" | "text" | "refusal"
    ) {
        return None;
    }
    let text_id = string_field(object, "item_id")
        .or_else(|| string_field(object, "id"))
        .or_else(|| string_field(object, "output_index").map(|index| format!("text_{index}")));
    let text = string_field(object, "text")
        .or_else(|| string_field(object, "content"))
        .or_else(|| string_field(object, "refusal"));
    Some((text_id, text))
}

fn openai_stream_tool_call(value: &Value) -> Option<ToolCall> {
    let object = value.as_object()?;
    let item_type = string_field(object, "type").unwrap_or_default();
    if item_type != "function_call" {
        return None;
    }
    let name = string_field(object, "name").unwrap_or_default();
    let id = string_field(object, "call_id")
        .or_else(|| string_field(object, "id"))
        .or_else(|| string_field(object, "item_id"))
        .unwrap_or_else(|| name.clone());
    let raw_arguments = string_field(object, "arguments").unwrap_or_default();
    let arguments = if raw_arguments.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(&raw_arguments)
            .unwrap_or_else(|_| Value::String(raw_arguments.clone()))
    };
    Some(ToolCall {
        id,
        name,
        arguments,
        raw_arguments: Some(raw_arguments),
        r#type: "function".to_string(),
    })
}

fn remember_openai_stream_tool_call_aliases(
    aliases: &mut BTreeMap<String, String>,
    payload: &Value,
    item: &Value,
    tool_call_id: &str,
) {
    let tool_call_id = tool_call_id.trim();
    if tool_call_id.is_empty() {
        return;
    }
    for key in openai_stream_tool_call_alias_keys(payload, Some(item)) {
        aliases.insert(key, tool_call_id.to_string());
    }
}

fn openai_stream_tool_call_delta_id(
    payload: &Value,
    aliases: &BTreeMap<String, String>,
) -> Option<String> {
    value_identifier(payload, "call_id")
        .or_else(|| openai_stream_tool_call_alias_id(payload, None, aliases))
        .or_else(|| value_identifier(payload, "item_id"))
        .or_else(|| value_identifier(payload, "output_index"))
}

fn openai_stream_tool_call_alias_id(
    payload: &Value,
    item: Option<&Value>,
    aliases: &BTreeMap<String, String>,
) -> Option<String> {
    openai_stream_tool_call_alias_keys(payload, item)
        .into_iter()
        .find_map(|key| aliases.get(&key).cloned())
}

fn openai_stream_tool_call_alias_keys(payload: &Value, item: Option<&Value>) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(item) = item {
        push_openai_stream_item_alias_keys(&mut keys, value_identifier(item, "id"));
        push_openai_stream_item_alias_keys(&mut keys, value_identifier(item, "item_id"));
        push_openai_stream_output_index_alias_key(
            &mut keys,
            value_identifier(item, "output_index"),
        );
    }
    push_openai_stream_item_alias_keys(&mut keys, value_identifier(payload, "item_id"));
    push_openai_stream_output_index_alias_key(&mut keys, value_identifier(payload, "output_index"));
    keys
}

fn push_openai_stream_item_alias_keys(keys: &mut Vec<String>, value: Option<String>) {
    let Some(value) = value else {
        return;
    };
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    keys.push(format!("item_id:{value}"));
    keys.push(format!("id:{value}"));
}

fn push_openai_stream_output_index_alias_key(keys: &mut Vec<String>, value: Option<String>) {
    let Some(value) = value else {
        return;
    };
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    keys.push(format!("output_index:{value}"));
}

fn merge_tool_calls_for_stream(
    current: Option<ToolCall>,
    incoming: ToolCall,
    final_fragment: bool,
) -> ToolCall {
    let Some(current) = current else {
        return incoming;
    };
    let id = if incoming.id.is_empty() {
        current.id
    } else {
        incoming.id
    };
    let name = if incoming.name.is_empty() {
        current.name
    } else {
        incoming.name
    };
    let r#type = if incoming.r#type.is_empty() {
        current.r#type
    } else {
        incoming.r#type
    };
    let (arguments, raw_arguments) = merge_stream_tool_arguments(
        current.arguments,
        current.raw_arguments,
        incoming.arguments,
        incoming.raw_arguments,
        final_fragment,
    );
    ToolCall {
        id,
        name,
        arguments,
        raw_arguments,
        r#type,
    }
}

fn merge_stream_tool_arguments(
    current_arguments: Value,
    current_raw: Option<String>,
    incoming_arguments: Value,
    incoming_raw: Option<String>,
    final_fragment: bool,
) -> (Value, Option<String>) {
    match (current_arguments, incoming_arguments) {
        (Value::Object(mut current), Value::Object(incoming)) => {
            for (key, value) in incoming {
                current.insert(key, value);
            }
            let arguments = Value::Object(current);
            let raw = incoming_raw
                .or(current_raw)
                .or_else(|| Some(json_compact(&arguments)));
            (arguments, raw)
        }
        (Value::String(current), Value::String(incoming)) => {
            let merged = if final_fragment && incoming.starts_with(&current) {
                incoming
            } else {
                format!("{current}{incoming}")
            };
            (Value::String(merged.clone()), Some(merged))
        }
        (current, Value::String(incoming)) => {
            let current_raw = current_raw.unwrap_or_else(|| json_compact(&current));
            let merged = if final_fragment && incoming.starts_with(&current_raw) {
                incoming
            } else {
                format!("{current_raw}{incoming}")
            };
            (Value::String(merged.clone()), Some(merged))
        }
        (_, incoming) => {
            let raw = incoming_raw.or_else(|| Some(json_compact(&incoming)));
            (incoming, raw)
        }
    }
}

fn provider_payload_error(provider: &'static str, payload: &Value) -> AdapterError {
    let error = payload.get("error").unwrap_or(payload);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| payload.get("message").and_then(Value::as_str))
        .unwrap_or("provider stream error");
    let mut error = AdapterError::provider(
        AdapterErrorKind::Stream,
        message.to_string(),
        Some(provider.to_string()),
    );
    error.raw = Some(payload.clone());
    error
}

fn anthropic_stream_thinking_metadata(
    block: &Value,
    redacted: bool,
    source_model: Option<String>,
) -> ThinkingData {
    ThinkingData {
        text: String::new(),
        signature: if redacted {
            None
        } else {
            block
                .as_object()
                .and_then(|object| string_field(object, "signature"))
        },
        redacted,
        source_provider: Some("anthropic".to_string()),
        source_model,
    }
}

fn translate_openai_responses_response(
    payload: Value,
    rate_limit: Option<RateLimitInfo>,
) -> Result<Response, AdapterError> {
    let raw = payload.clone();
    let object = payload.as_object().ok_or_else(|| {
        invalid_response_error("openai", "OpenAI Responses response must be a JSON object")
    })?;
    let model = string_field(object, "model").unwrap_or_default();

    let mut content = Vec::new();
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    if let Some(output) = object.get("output").and_then(Value::as_array) {
        for item in output {
            append_openai_response_item(item, &model, &mut content, &mut text, &mut tool_calls)?;
        }
    }

    Ok(Response {
        id: string_field(object, "id").unwrap_or_default(),
        model,
        provider: "openai".to_string(),
        message: Message {
            role: MessageRole::Assistant,
            content,
            ..Message::default()
        },
        finish_reason: openai_finish_reason(object, !tool_calls.is_empty()),
        usage: usage_from_openai(object.get("usage")).normalized(),
        raw: Some(raw),
        rate_limit,
        text,
        tool_calls,
        ..Response::default()
    })
}

fn append_openai_response_item(
    item: &Value,
    model: &str,
    content: &mut Vec<ContentPart>,
    text: &mut String,
    tool_calls: &mut Vec<ToolCall>,
) -> Result<(), AdapterError> {
    let Some(object) = item.as_object() else {
        return Ok(());
    };
    match object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "output_text" | "text" => {
            if let Some(value) = string_field(object, "text") {
                text.push_str(&value);
                content.push(ContentPart::Text { text: value });
            }
        }
        "message" => {
            if let Some(parts) = object.get("content").and_then(Value::as_array) {
                for part in parts {
                    append_openai_response_item(part, model, content, text, tool_calls)?;
                }
            }
        }
        "reasoning" | "thinking" | "redacted_thinking" => {
            if let Some(value) = openai_response_item_text(object) {
                let redacted = object
                    .get("type")
                    .and_then(Value::as_str)
                    .map(|kind| kind == "redacted_thinking")
                    .unwrap_or(false);
                let thinking = ThinkingData {
                    text: value,
                    signature: string_field(object, "signature"),
                    redacted,
                    source_provider: Some("openai".to_string()),
                    source_model: source_model(model),
                };
                if redacted {
                    content.push(ContentPart::RedactedThinking { thinking });
                } else {
                    content.push(ContentPart::Thinking { thinking });
                }
            }
        }
        "function_call" => {
            let Some(name) = string_field(object, "name").and_then(|name| non_empty_owned(name))
            else {
                return Ok(());
            };
            let id = string_field(object, "id")
                .or_else(|| string_field(object, "call_id"))
                .unwrap_or_else(|| name.clone());
            let arguments = object
                .get("arguments")
                .map(openai_response_arguments)
                .transpose()?
                .unwrap_or_else(|| (json!({}), "{}".to_string()));
            let tool_call = ToolCall {
                id,
                name,
                arguments: arguments.0,
                raw_arguments: Some(arguments.1),
                r#type: "function".to_string(),
            };
            content.push(ContentPart::ToolCall {
                tool_call: tool_call.clone(),
            });
            tool_calls.push(tool_call);
        }
        _ => {}
    }
    Ok(())
}

fn openai_response_item_text(object: &Map<String, Value>) -> Option<String> {
    object
        .get("text")
        .and_then(text_from_value)
        .or_else(|| object.get("summary").and_then(text_from_value))
        .or_else(|| object.get("content").and_then(text_from_value))
        .or_else(|| object.get("output_text").and_then(text_from_value))
}

fn openai_response_arguments(value: &Value) -> Result<(Value, String), AdapterError> {
    match value {
        Value::String(text) => {
            let parsed = if text.trim().is_empty() {
                json!({})
            } else {
                serde_json::from_str(text).map_err(|error| {
                    invalid_response_error(
                        "openai",
                        format!("OpenAI function_call arguments must be valid JSON: {error}"),
                    )
                })?
            };
            Ok((parsed, text.clone()))
        }
        other => Ok((other.clone(), json_compact(other))),
    }
}

fn translate_anthropic_messages_response(
    payload: Value,
    rate_limit: Option<RateLimitInfo>,
) -> Result<Response, AdapterError> {
    let raw = payload.clone();
    let object = payload.as_object().ok_or_else(|| {
        invalid_response_error(
            "anthropic",
            "Anthropic Messages response must be a JSON object",
        )
    })?;
    let model = string_field(object, "model").unwrap_or_default();
    let content = object
        .get("content")
        .map(|content| anthropic_response_content(content, &model))
        .transpose()?
        .unwrap_or_default();
    let text = content
        .iter()
        .filter_map(ContentPart::text_content)
        .collect::<Vec<_>>()
        .join("");
    let has_tool_calls = content
        .iter()
        .any(|part| matches!(part, ContentPart::ToolCall { .. }));

    let mut usage = usage_from_anthropic(object.get("usage")).normalized();
    if usage.reasoning_tokens.is_none() {
        if let Some(estimated_reasoning_tokens) = estimate_reasoning_tokens(&content) {
            usage.reasoning_tokens = Some(estimated_reasoning_tokens);
        }
    }

    Ok(Response {
        id: string_field(object, "id").unwrap_or_default(),
        model,
        provider: "anthropic".to_string(),
        message: Message {
            role: MessageRole::Assistant,
            content,
            ..Message::default()
        },
        finish_reason: anthropic_finish_reason(string_field(object, "stop_reason"), has_tool_calls),
        usage,
        raw: Some(raw),
        rate_limit,
        text,
        ..Response::default()
    })
}

fn anthropic_response_content(
    value: &Value,
    model: &str,
) -> Result<Vec<ContentPart>, AdapterError> {
    let Some(blocks) = value.as_array() else {
        return Ok(Vec::new());
    };
    let mut content = Vec::new();
    for block in blocks {
        let Some(object) = block.as_object() else {
            continue;
        };
        match object
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "text" => {
                if let Some(text) = string_field(object, "text") {
                    content.push(ContentPart::Text { text });
                }
            }
            "thinking" => {
                if let Some(text) = string_field(object, "thinking") {
                    content.push(ContentPart::Thinking {
                        thinking: ThinkingData {
                            text,
                            signature: string_field(object, "signature"),
                            redacted: false,
                            source_provider: Some("anthropic".to_string()),
                            source_model: source_model(model),
                        },
                    });
                }
            }
            "redacted_thinking" => {
                if let Some(text) = string_field(object, "data") {
                    content.push(ContentPart::RedactedThinking {
                        thinking: ThinkingData {
                            text,
                            signature: None,
                            redacted: true,
                            source_provider: Some("anthropic".to_string()),
                            source_model: source_model(model),
                        },
                    });
                }
            }
            "tool_use" => {
                let Some(id) = string_field(object, "id") else {
                    continue;
                };
                let Some(name) = string_field(object, "name") else {
                    continue;
                };
                content.push(ContentPart::ToolCall {
                    tool_call: ToolCall {
                        id,
                        name,
                        arguments: object.get("input").cloned().unwrap_or_else(|| json!({})),
                        raw_arguments: object.get("input").map(json_compact),
                        r#type: "function".to_string(),
                    },
                });
            }
            _ => content.push(ContentPart::Provider { raw: block.clone() }),
        }
    }
    Ok(content)
}

fn translate_gemini_generate_content_response(
    payload: Value,
    rate_limit: Option<RateLimitInfo>,
) -> Result<Response, AdapterError> {
    let raw = payload.clone();
    let object = payload.as_object().ok_or_else(|| {
        invalid_response_error(
            "gemini",
            "Gemini generateContent response must be a JSON object",
        )
    })?;
    let candidate = object
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|candidates| candidates.first())
        .and_then(Value::as_object);
    let model = string_field(object, "modelVersion")
        .or_else(|| string_field(object, "model"))
        .or_else(|| candidate.and_then(|candidate| string_field(candidate, "modelVersion")))
        .unwrap_or_default();
    let content = candidate
        .and_then(|candidate| candidate.get("content"))
        .map(|content| gemini_response_content(content, &model, 0))
        .transpose()?
        .unwrap_or_default();
    let text = content
        .iter()
        .filter_map(ContentPart::text_content)
        .collect::<Vec<_>>()
        .join("");
    let has_tool_calls = content
        .iter()
        .any(|part| matches!(part, ContentPart::ToolCall { .. }));
    let finish_reason = candidate
        .and_then(|candidate| string_field(candidate, "finishReason"))
        .map(|raw| gemini_finish_reason(raw.as_str(), has_tool_calls))
        .unwrap_or(FinishReason::Other);

    Ok(Response {
        id: string_field(object, "responseId").unwrap_or_default(),
        model,
        provider: "gemini".to_string(),
        message: Message {
            role: MessageRole::Assistant,
            content,
            ..Message::default()
        },
        finish_reason,
        usage: usage_from_gemini(object.get("usageMetadata")).normalized(),
        raw: Some(raw),
        rate_limit,
        text,
        ..Response::default()
    })
}

fn gemini_response_content(
    value: &Value,
    model: &str,
    candidate_index: usize,
) -> Result<Vec<ContentPart>, AdapterError> {
    let Some(parts) = value.get("parts").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut content = Vec::new();
    for (part_index, part) in parts.iter().enumerate() {
        let Some(object) = part.as_object() else {
            continue;
        };
        if let Some(text) = string_field(object, "text") {
            if object
                .get("thought")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                content.push(ContentPart::Thinking {
                    thinking: ThinkingData {
                        text,
                        signature: string_field(object, "thoughtSignature"),
                        redacted: false,
                        source_provider: Some("gemini".to_string()),
                        source_model: source_model(model),
                    },
                });
            } else {
                content.push(ContentPart::Text { text });
            }
            continue;
        }
        if let Some(function_call) = object.get("functionCall").and_then(Value::as_object) {
            let Some(name) = string_field(function_call, "name") else {
                continue;
            };
            let (args, raw_arguments) = gemini_function_call_arguments(function_call)?;
            let id = string_field(function_call, "id")
                .or_else(|| string_field(function_call, "functionCallId"))
                .unwrap_or_else(|| {
                    gemini_synthetic_tool_call_id(&name, candidate_index, part_index, &args)
                });
            content.push(ContentPart::ToolCall {
                tool_call: ToolCall {
                    id,
                    name,
                    raw_arguments: Some(raw_arguments),
                    arguments: args,
                    r#type: "function".to_string(),
                },
            });
            continue;
        }
        content.push(ContentPart::Provider { raw: part.clone() });
    }
    Ok(content)
}

fn usage_from_openai(value: Option<&Value>) -> Usage {
    let source = value.unwrap_or(&Value::Null);
    Usage {
        input_tokens: token_at(source, &["input_tokens"])
            .or_else(|| token_at(source, &["prompt_tokens"]))
            .unwrap_or(0),
        output_tokens: token_at(source, &["output_tokens"])
            .or_else(|| token_at(source, &["completion_tokens"]))
            .unwrap_or(0),
        total_tokens: token_at(source, &["total_tokens"]).unwrap_or(0),
        reasoning_tokens: token_at(source, &["output_tokens_details", "reasoning_tokens"])
            .or_else(|| token_at(source, &["output_tokens_details", "reasoningTokens"])),
        cache_read_tokens: token_at(source, &["input_tokens_details", "cached_tokens"])
            .or_else(|| token_at(source, &["input_tokens_details", "cachedTokens"])),
        cache_write_tokens: None,
        raw: value.cloned(),
    }
}

fn usage_from_anthropic(value: Option<&Value>) -> Usage {
    let source = value.unwrap_or(&Value::Null);
    Usage {
        input_tokens: token_at(source, &["input_tokens"]).unwrap_or(0),
        output_tokens: token_at(source, &["output_tokens"]).unwrap_or(0),
        total_tokens: token_at(source, &["total_tokens"]).unwrap_or(0),
        reasoning_tokens: token_at(source, &["reasoning_tokens"]),
        cache_read_tokens: token_at(source, &["cache_read_input_tokens"]),
        cache_write_tokens: token_at(source, &["cache_creation_input_tokens"])
            .or_else(|| token_at(source, &["cache_write_tokens"]))
            .or_else(|| token_at(source, &["cacheWriteTokens"])),
        raw: value.cloned(),
    }
}

fn usage_from_gemini(value: Option<&Value>) -> Usage {
    let source = value.unwrap_or(&Value::Null);
    Usage {
        input_tokens: token_at(source, &["promptTokenCount"]).unwrap_or(0),
        output_tokens: token_at(source, &["candidatesTokenCount"]).unwrap_or(0),
        total_tokens: token_at(source, &["totalTokenCount"]).unwrap_or(0),
        reasoning_tokens: token_at(source, &["thoughtsTokenCount"]),
        cache_read_tokens: token_at(source, &["cachedContentTokenCount"]),
        cache_write_tokens: None,
        raw: value.cloned(),
    }
}

fn gemini_function_call_arguments(
    function_call: &Map<String, Value>,
) -> Result<(Value, String), AdapterError> {
    let value = function_call
        .get("args")
        .or_else(|| function_call.get("arguments"))
        .unwrap_or(&Value::Null);
    match value {
        Value::Null => Ok((json!({}), "{}".to_string())),
        Value::String(text) => {
            let parsed = if text.trim().is_empty() {
                json!({})
            } else {
                serde_json::from_str(text).map_err(|error| {
                    invalid_response_error(
                        "gemini",
                        format!("Gemini functionCall arguments must be valid JSON: {error}"),
                    )
                })?
            };
            Ok((parsed, text.clone()))
        }
        other => Ok((other.clone(), stable_json(other))),
    }
}

fn estimate_reasoning_tokens(parts: &[ContentPart]) -> Option<u64> {
    let mut estimated_total = 0_u64;
    let mut saw_reasoning = false;

    for part in parts {
        let thinking = match part {
            ContentPart::Thinking { thinking } | ContentPart::RedactedThinking { thinking } => {
                thinking
            }
            _ => continue,
        };
        saw_reasoning = true;

        let character_count = thinking.text.chars().count() as u64;
        if character_count == 0 {
            continue;
        }
        let estimated = character_count.saturating_add(3) / 4;
        estimated_total = estimated_total.saturating_add(estimated.max(1));
    }

    saw_reasoning.then_some(estimated_total)
}

fn source_model(model: &str) -> Option<String> {
    non_empty(Some(model)).map(str::to_string)
}

fn normalize_rate_limit_headers(headers: &BTreeMap<String, String>) -> Option<RateLimitInfo> {
    let rate_limit = RateLimitInfo {
        requests_remaining: header_u64(
            headers,
            &[
                "x-ratelimit-remaining-requests",
                "anthropic-ratelimit-requests-remaining",
            ],
        ),
        requests_limit: header_u64(
            headers,
            &[
                "x-ratelimit-limit-requests",
                "anthropic-ratelimit-requests-limit",
            ],
        ),
        tokens_remaining: header_u64(
            headers,
            &[
                "x-ratelimit-remaining-tokens",
                "anthropic-ratelimit-tokens-remaining",
            ],
        ),
        tokens_limit: header_u64(
            headers,
            &[
                "x-ratelimit-limit-tokens",
                "anthropic-ratelimit-tokens-limit",
            ],
        ),
        reset_at: header_string(
            headers,
            &[
                "x-ratelimit-reset",
                "x-ratelimit-reset-requests",
                "x-ratelimit-reset-tokens",
                "ratelimit-reset",
                "anthropic-ratelimit-requests-reset",
                "anthropic-ratelimit-tokens-reset",
            ],
        ),
    };

    if rate_limit.requests_remaining.is_none()
        && rate_limit.requests_limit.is_none()
        && rate_limit.tokens_remaining.is_none()
        && rate_limit.tokens_limit.is_none()
        && rate_limit.reset_at.is_none()
    {
        None
    } else {
        Some(rate_limit)
    }
}

fn header_u64(headers: &BTreeMap<String, String>, names: &[&str]) -> Option<u64> {
    let text = header_value(headers, names)?;
    text.parse::<u64>().ok().or_else(|| {
        text.parse::<f64>().ok().and_then(|value| {
            if value.is_finite() && value >= 0.0 && value.fract() == 0.0 {
                Some(value as u64)
            } else {
                None
            }
        })
    })
}

fn header_string(headers: &BTreeMap<String, String>, names: &[&str]) -> Option<String> {
    header_value(headers, names).map(str::to_string)
}

fn header_value<'a>(headers: &'a BTreeMap<String, String>, names: &[&str]) -> Option<&'a str> {
    for name in names {
        for (key, value) in headers {
            if key.eq_ignore_ascii_case(name) {
                return non_empty(Some(value));
            }
        }
    }
    None
}

fn validate_provider_thinking_source(
    provider: &'static str,
    thinking: &ThinkingData,
    content_kind: &'static str,
) -> Result<(), AdapterError> {
    if let Some(source_provider) = thinking.source_provider.as_deref().and_then(non_empty_str) {
        let normalized_source = normalize_provider(source_provider);
        if normalized_source != provider {
            return Err(invalid_request_error(
                provider,
                format!(
                    "{provider} request history cannot include {content_kind} content from {source_provider}; provider thinking content is only valid for same-provider continuation"
                ),
            ));
        }
        return Ok(());
    }

    let has_signature = thinking
        .signature
        .as_deref()
        .and_then(non_empty_str)
        .is_some();
    if has_signature || thinking.redacted {
        return Err(invalid_request_error(
            provider,
            format!(
                "{provider} {content_kind} content with a signature or redacted payload requires source_provider provenance for same-provider continuation"
            ),
        ));
    }

    Ok(())
}

fn token_at(value: &Value, path: &[&str]) -> Option<u64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    match current {
        Value::Number(number) => number
            .as_u64()
            .or_else(|| number.as_i64().and_then(|value| u64::try_from(value).ok())),
        Value::String(text) => text.parse::<u64>().ok(),
        _ => None,
    }
}

fn text_from_value(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(values) => {
            let text = values
                .iter()
                .filter_map(text_from_value)
                .collect::<Vec<_>>()
                .join("");
            (!text.is_empty()).then_some(text)
        }
        Value::Object(object) => object
            .get("text")
            .and_then(text_from_value)
            .or_else(|| object.get("content").and_then(text_from_value)),
        _ => None,
    }
}

fn string_field(object: &Map<String, Value>, key: &str) -> Option<String> {
    object.get(key).and_then(Value::as_str).map(str::to_string)
}

fn non_empty_owned(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

fn openai_finish_reason(object: &Map<String, Value>, has_tool_calls: bool) -> FinishReason {
    let candidate = string_field(object, "finish_reason")
        .or_else(|| string_field(object, "finishReason"))
        .or_else(|| string_field(object, "status"));
    let incomplete_reason = object
        .get("incomplete_details")
        .or_else(|| object.get("incompleteDetails"))
        .and_then(Value::as_object)
        .and_then(|details| string_field(details, "reason"));

    let (raw, mut reason) = match (candidate, incomplete_reason) {
        (Some(status), Some(incomplete_reason))
            if status.trim().eq_ignore_ascii_case("incomplete") =>
        {
            let reason = openai_finish_reason_kind(&incomplete_reason);
            (status, reason)
        }
        (Some(raw), _) => {
            let reason = openai_finish_reason_kind(&raw);
            (raw, reason)
        }
        (None, Some(raw)) => {
            let reason = openai_finish_reason_kind(&raw);
            (raw, reason)
        }
        (None, None) => {
            return if has_tool_calls {
                FinishReason::ToolCalls
            } else {
                FinishReason::Other
            };
        }
    };

    if has_tool_calls
        && !matches!(
            reason,
            FinishReasonKind::Length | FinishReasonKind::ContentFilter | FinishReasonKind::Error
        )
    {
        reason = FinishReasonKind::ToolCalls;
    }
    FinishReason::from_provider(reason, raw)
}

fn openai_finish_reason_kind(raw: &str) -> FinishReasonKind {
    match raw.trim().to_ascii_lowercase().as_str() {
        "completed" | "complete" | "done" | "success" | "succeeded" => FinishReasonKind::Stop,
        "incomplete" | "length" | "max_tokens" | "max_output_tokens" => FinishReasonKind::Length,
        "tool_calls" | "tool_use" => FinishReasonKind::ToolCalls,
        "content_filter" | "safety" | "refusal" => FinishReasonKind::ContentFilter,
        "error" | "failed" | "cancelled" | "canceled" => FinishReasonKind::Error,
        _ => FinishReasonKind::Other,
    }
}

fn anthropic_finish_reason(stop_reason: Option<String>, has_tool_calls: bool) -> FinishReason {
    let Some(raw) = stop_reason else {
        return if has_tool_calls {
            FinishReason::ToolCalls
        } else {
            FinishReason::Other
        };
    };
    let reason = match raw.trim().to_ascii_lowercase().as_str() {
        "end_turn" | "stop_sequence" => FinishReasonKind::Stop,
        "max_tokens" => FinishReasonKind::Length,
        "tool_use" => FinishReasonKind::ToolCalls,
        "content_filter" | "safety" | "refusal" => FinishReasonKind::ContentFilter,
        "error" | "failed" | "cancelled" | "canceled" => FinishReasonKind::Error,
        _ if has_tool_calls => FinishReasonKind::ToolCalls,
        _ => FinishReasonKind::Other,
    };
    FinishReason::from_provider(reason, raw)
}

fn gemini_finish_reason(raw: &str, has_tool_calls: bool) -> FinishReason {
    let reason = match raw.trim().to_ascii_uppercase().as_str() {
        "STOP" => {
            if has_tool_calls {
                FinishReasonKind::ToolCalls
            } else {
                FinishReasonKind::Stop
            }
        }
        "MAX_TOKENS" => FinishReasonKind::Length,
        "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" => {
            FinishReasonKind::ContentFilter
        }
        "MALFORMED_FUNCTION_CALL" => FinishReasonKind::Error,
        _ if has_tool_calls => FinishReasonKind::ToolCalls,
        _ => FinishReasonKind::Other,
    };
    FinishReason::from_provider(reason, raw.to_string())
}

fn openai_responses_body(request: &Request) -> Result<Value, AdapterError> {
    let mut body = Map::new();
    body.insert("model".to_string(), json!(request.model));

    if let Some(instructions) = instruction_text(&request.messages, "openai")? {
        body.insert("instructions".to_string(), json!(instructions));
    }

    let mut input = Vec::new();
    for message in &request.messages {
        if matches!(message.role, MessageRole::System | MessageRole::Developer) {
            continue;
        }
        input.extend(openai_message_input_items(message)?);
    }
    if !input.is_empty() {
        body.insert("input".to_string(), Value::Array(input));
    }

    if !request.tools.is_empty() {
        let tools = request
            .tools
            .iter()
            .map(openai_tool_definition)
            .collect::<Result<Vec<_>, _>>()?;
        body.insert("tools".to_string(), Value::Array(tools));
    }
    if let Some(tool_choice) = request.tool_choice.as_ref() {
        body.insert(
            "tool_choice".to_string(),
            openai_tool_choice_value(tool_choice),
        );
    }
    insert_generation_fields(&mut body, request, ProviderGenerationShape::OpenAiResponses);
    if !request.metadata.is_empty() {
        body.insert("metadata".to_string(), json!(request.metadata));
    }
    if let Some(reasoning_effort) = non_empty(request.reasoning_effort.as_deref()) {
        body.insert(
            "reasoning".to_string(),
            json!({
                "effort": reasoning_effort,
            }),
        );
    }
    if let Some(response_format) = request.response_format.as_ref() {
        body.insert(
            "response_format".to_string(),
            openai_response_format(response_format),
        );
    }

    let portable_body = Value::Object(body.clone());
    if let Some(options) = provider_options_object(request, "openai")? {
        let mut options = select_json_fields(options, OPENAI_RESPONSES_OPTION_KEYS);
        let native_tools = options.remove("tools");
        let mut merged = deep_merge(Value::Object(body), Value::Object(options));
        deep_insert_with_recursive_keys(&mut merged, portable_body, &["reasoning"]);
        if let Some(Value::Array(native_tools)) = native_tools {
            append_array_field(&mut merged, "tools", native_tools);
        }
        return Ok(merged);
    }

    Ok(portable_body)
}

fn openai_message_input_items(message: &Message) -> Result<Vec<Value>, AdapterError> {
    if message.role == MessageRole::Tool {
        return Ok(vec![openai_tool_output_from_message(message)?]);
    }

    let mut items = Vec::new();
    let mut content = Vec::new();
    let flush_content = |items: &mut Vec<Value>, content: &mut Vec<Value>| {
        if content.is_empty() {
            return;
        }
        let mut item = Map::new();
        item.insert("type".to_string(), json!("message"));
        item.insert("role".to_string(), json!(role_wire_value(message.role)));
        item.insert("content".to_string(), Value::Array(std::mem::take(content)));
        if let Some(name) = non_empty(message.name.as_deref()) {
            item.insert("name".to_string(), json!(name));
        }
        items.push(Value::Object(item));
    };

    for part in &message.content {
        match part {
            ContentPart::Text { text } => {
                let content_type = if message.role == MessageRole::Assistant {
                    "output_text"
                } else {
                    "input_text"
                };
                content.push(json!({
                    "type": content_type,
                    "text": text,
                }));
            }
            ContentPart::Image { image } => {
                content.push(openai_image_part(image)?);
            }
            ContentPart::ToolCall { tool_call } => {
                flush_content(&mut items, &mut content);
                items.push(openai_function_call_item(tool_call)?);
            }
            ContentPart::ToolResult { tool_result } => {
                flush_content(&mut items, &mut content);
                items.push(openai_tool_output_from_result(tool_result)?);
            }
            ContentPart::Thinking { .. } | ContentPart::RedactedThinking { .. } => {
                return Err(invalid_request_error(
                    "openai",
                    "OpenAI Responses request history does not support reasoning content parts",
                ));
            }
            _ => {
                return Err(invalid_request_error(
                    "openai",
                    format!(
                        "OpenAI Responses request history does not support {} content parts",
                        part.kind().as_str()
                    ),
                ));
            }
        }
    }
    flush_content(&mut items, &mut content);
    Ok(items)
}

fn openai_tool_output_from_message(message: &Message) -> Result<Value, AdapterError> {
    let mut call_id = message.tool_call_id.clone();
    let mut result = None;
    let mut text = Vec::new();
    for part in &message.content {
        match part {
            ContentPart::Text { text: value } => text.push(value.as_str()),
            ContentPart::ToolResult { tool_result } => {
                if let Some(existing) = call_id.as_deref() {
                    if existing != tool_result.tool_call_id {
                        return Err(invalid_request_error(
                            "openai",
                            "OpenAI tool messages must use a single tool_call_id",
                        ));
                    }
                }
                call_id = Some(tool_result.tool_call_id.clone());
                if result.replace(tool_result.content.clone()).is_some() {
                    return Err(invalid_request_error(
                        "openai",
                        "OpenAI tool messages support only one tool_result payload",
                    ));
                }
            }
            _ => {
                return Err(invalid_request_error(
                    "openai",
                    "OpenAI tool messages support only text or tool_result content",
                ));
            }
        }
    }

    let call_id = call_id.ok_or_else(|| {
        invalid_request_error("openai", "OpenAI tool messages require a tool_call_id")
    })?;
    let output = match (result, text.is_empty()) {
        (Some(_), false) => {
            return Err(invalid_request_error(
                "openai",
                "OpenAI tool messages cannot mix text and tool_result content",
            ));
        }
        (Some(value), true) => openai_tool_output_value(value),
        (None, false) => Value::String(text.join("")),
        (None, true) => {
            return Err(invalid_request_error(
                "openai",
                "OpenAI tool messages require text or tool_result content",
            ));
        }
    };

    Ok(json!({
        "type": "function_call_output",
        "call_id": call_id,
        "output": output,
    }))
}

fn openai_tool_output_from_result(result: &ToolResultData) -> Result<Value, AdapterError> {
    Ok(json!({
        "type": "function_call_output",
        "call_id": result.tool_call_id,
        "output": openai_tool_output_value(result.content.clone()),
    }))
}

fn openai_tool_output_value(value: Value) -> Value {
    match value {
        Value::Array(_) | Value::String(_) => value,
        other => Value::String(json_compact(&other)),
    }
}

fn openai_function_call_item(tool_call: &ToolCall) -> Result<Value, AdapterError> {
    tool_call.validate().map_err(|message| {
        AdapterError::provider(
            AdapterErrorKind::InvalidRequest,
            message,
            Some("openai".to_string()),
        )
    })?;
    Ok(json!({
        "type": "function_call",
        "id": tool_call.id,
        "name": tool_call.name,
        "arguments": tool_call.raw_arguments.clone().unwrap_or_else(|| json_compact(&tool_call.arguments)),
    }))
}

fn openai_image_part(image: &ImageData) -> Result<Value, AdapterError> {
    let source = normalize_image_source(image, "openai")?;
    let mut part = Map::new();
    part.insert("type".to_string(), json!("input_image"));
    part.insert("image_url".to_string(), json!(openai_image_url(&source)));
    if let Some(detail) = non_empty(image.detail.as_deref()) {
        part.insert("detail".to_string(), json!(detail));
    }
    Ok(Value::Object(part))
}

fn anthropic_messages_body(
    request: &Request,
    active_options: Option<&Map<String, Value>>,
) -> Result<Value, AdapterError> {
    let (mut messages, mut system_blocks) = anthropic_message_payloads(&request.messages)?;
    if let Some(instruction) = anthropic_structured_output_instruction(request, active_options)? {
        append_anthropic_system_text(&mut system_blocks, instruction);
    }

    let mut body = Map::new();
    body.insert("model".to_string(), json!(request.model));
    body.insert("messages".to_string(), Value::Array(messages.clone()));
    body.insert(
        "max_tokens".to_string(),
        json!(request.max_tokens.unwrap_or(4096)),
    );
    insert_generation_fields(
        &mut body,
        request,
        ProviderGenerationShape::AnthropicMessages,
    );
    if !request.metadata.is_empty() {
        body.insert("metadata".to_string(), json!(request.metadata));
    }
    if let Some(options) = active_options {
        if let Some(thinking) = options.get("thinking") {
            if thinking.is_object() {
                body.insert("thinking".to_string(), thinking.clone());
            } else {
                return Err(invalid_request_error(
                    "anthropic",
                    "Anthropic provider_options.thinking must be an object",
                ));
            }
        }
    }
    if let Some(system) = anthropic_system_payload(system_blocks) {
        body.insert("system".to_string(), system);
    }

    let tool_choice = request
        .tool_choice
        .as_ref()
        .map(parse_tool_choice)
        .transpose()?;
    let tool_names = tool_names(&request.tools)?;
    if !request.tools.is_empty() && !matches!(tool_choice, Some(ToolChoiceSpec::None)) {
        let tools = request
            .tools
            .iter()
            .map(anthropic_tool_definition)
            .collect::<Result<Vec<_>, _>>()?;
        body.insert("tools".to_string(), Value::Array(tools));
    }
    if let Some(choice) = anthropic_tool_choice_payload(tool_choice.as_ref(), &tool_names)? {
        body.insert("tool_choice".to_string(), choice);
    }

    messages = body
        .remove("messages")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    body.insert("messages".to_string(), Value::Array(messages));

    let cache_control = anthropic_cache_control(active_options)?;
    apply_anthropic_cache_control(&mut body, cache_control.as_ref());

    Ok(Value::Object(body))
}

fn anthropic_message_payloads(
    messages: &[Message],
) -> Result<(Vec<Value>, Vec<Value>), AdapterError> {
    let mut payloads: Vec<Value> = Vec::new();
    let mut system_blocks = Vec::new();

    for message in messages {
        if matches!(message.role, MessageRole::System | MessageRole::Developer) {
            append_anthropic_system_message_blocks(&mut system_blocks, message)?;
            continue;
        }

        let payload = anthropic_message_payload(message)?;
        append_anthropic_message(&mut payloads, payload);
    }

    Ok((payloads, system_blocks))
}

fn anthropic_message_payload(message: &Message) -> Result<Value, AdapterError> {
    let role = match message.role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "user",
        other => {
            return Err(invalid_request_error(
                "anthropic",
                format!("Anthropic messages request does not support role {other:?} here"),
            ));
        }
    };

    let mut content = Vec::new();
    for part in &message.content {
        match part {
            ContentPart::Text { text } => content.push(json!({
                "type": "text",
                "text": text,
            })),
            ContentPart::Custom { kind, raw } | ContentPart::Raw { kind, raw } => {
                content.push(anthropic_raw_content_block(raw, Some(kind))?);
            }
            ContentPart::Provider { raw } => {
                content.push(anthropic_raw_content_block(raw, None)?);
            }
            ContentPart::Image { image } => content.push(anthropic_image_block(image)?),
            ContentPart::Thinking { thinking } => {
                validate_provider_thinking_source("anthropic", thinking, "thinking")?;
                let mut block = Map::new();
                block.insert("type".to_string(), json!("thinking"));
                block.insert("thinking".to_string(), json!(thinking.text));
                if let Some(signature) = non_empty(thinking.signature.as_deref()) {
                    block.insert("signature".to_string(), json!(signature));
                }
                content.push(Value::Object(block));
            }
            ContentPart::RedactedThinking { thinking } => {
                validate_provider_thinking_source("anthropic", thinking, "redacted_thinking")?;
                content.push(json!({
                    "type": "redacted_thinking",
                    "data": thinking.text,
                }));
            }
            ContentPart::ToolCall { tool_call } => {
                if message.role != MessageRole::Assistant {
                    return Err(invalid_request_error(
                        "anthropic",
                        "Anthropic tool_use blocks are only valid in assistant messages",
                    ));
                }
                content.push(json!({
                    "type": "tool_use",
                    "id": tool_call.id,
                    "name": tool_call.name,
                    "input": tool_call_arguments_object(tool_call, "anthropic")?,
                }));
            }
            ContentPart::ToolResult { tool_result } => {
                if message.role != MessageRole::Tool {
                    return Err(invalid_request_error(
                        "anthropic",
                        "Anthropic tool_result blocks are only valid in tool messages",
                    ));
                }
                let mut block = Map::new();
                block.insert("type".to_string(), json!("tool_result"));
                block.insert("tool_use_id".to_string(), json!(tool_result.tool_call_id));
                block.insert(
                    "content".to_string(),
                    anthropic_tool_result_content(tool_result)?,
                );
                if tool_result.is_error {
                    block.insert("is_error".to_string(), json!(true));
                }
                content.push(Value::Object(block));
            }
            _ => {
                return Err(invalid_request_error(
                    "anthropic",
                    format!(
                        "Anthropic messages request does not support {} content parts",
                        part.kind().as_str()
                    ),
                ));
            }
        }
    }

    if role == "user" {
        prioritize_anthropic_tool_results(&mut content);
    }
    if let Some(cache_control) = anthropic_message_cache_control(message)? {
        apply_cache_control_to_last_block(&mut content, &cache_control);
    }

    let mut payload = Map::new();
    payload.insert("role".to_string(), json!(role));
    payload.insert("content".to_string(), Value::Array(content));
    if let Some(name) = non_empty(message.name.as_deref()) {
        payload.insert("name".to_string(), json!(name));
    }
    Ok(Value::Object(payload))
}

fn append_anthropic_system_message_blocks(
    blocks: &mut Vec<Value>,
    message: &Message,
) -> Result<(), AdapterError> {
    let mut text_fragments = Vec::new();
    for part in &message.content {
        match part {
            ContentPart::Text { text } => text_fragments.push(text.as_str()),
            ContentPart::Custom { kind, raw } | ContentPart::Raw { kind, raw } => {
                flush_anthropic_system_text(blocks, &mut text_fragments);
                blocks.push(anthropic_raw_content_block(raw, Some(kind))?);
            }
            ContentPart::Provider { raw } => {
                flush_anthropic_system_text(blocks, &mut text_fragments);
                blocks.push(anthropic_raw_content_block(raw, None)?);
            }
            _ => {
                return Err(invalid_request_error(
                    "anthropic",
                    "Anthropic system and developer messages must be text or raw provider blocks",
                ));
            }
        }
    }
    flush_anthropic_system_text(blocks, &mut text_fragments);
    Ok(())
}

fn flush_anthropic_system_text(blocks: &mut Vec<Value>, fragments: &mut Vec<&str>) {
    if fragments.is_empty() {
        return;
    }
    append_anthropic_system_text(blocks, fragments.join("\n\n"));
    fragments.clear();
}

fn append_anthropic_system_text(blocks: &mut Vec<Value>, text: String) {
    if text.is_empty() {
        return;
    }
    if let Some(last_block) = blocks.last_mut().and_then(Value::as_object_mut) {
        let is_mergeable_text = last_block
            .get("type")
            .and_then(Value::as_str)
            .map(|kind| kind == "text")
            .unwrap_or(false)
            && !last_block.contains_key("cache_control");
        if is_mergeable_text {
            let last_text = last_block
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let merged = if last_text.is_empty() {
                text
            } else {
                format!("{last_text}\n\n{text}")
            };
            last_block.insert("text".to_string(), Value::String(merged));
            return;
        }
    }
    blocks.push(json!({
        "type": "text",
        "text": text,
    }));
}

fn anthropic_system_payload(blocks: Vec<Value>) -> Option<Value> {
    if blocks.is_empty() {
        return None;
    }
    if blocks.iter().all(is_plain_anthropic_text_block) {
        let text = blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        return (!text.is_empty()).then_some(Value::String(text));
    }
    Some(Value::Array(blocks))
}

fn is_plain_anthropic_text_block(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == 2
        && object
            .get("type")
            .and_then(Value::as_str)
            .map(|kind| kind == "text")
            .unwrap_or(false)
        && object.get("text").and_then(Value::as_str).is_some()
}

fn anthropic_raw_content_block(
    raw: &Value,
    fallback_type: Option<&str>,
) -> Result<Value, AdapterError> {
    let Some(raw_object) = raw.as_object() else {
        return Err(invalid_request_error(
            "anthropic",
            "Anthropic raw provider content blocks must be objects",
        ));
    };
    let mut object = raw_object.clone();
    if !object.contains_key("type") {
        let Some(kind) = fallback_type.and_then(non_empty_str) else {
            return Err(invalid_request_error(
                "anthropic",
                "Anthropic raw provider content blocks require a type",
            ));
        };
        object.insert("type".to_string(), json!(kind));
    }
    Ok(Value::Object(object))
}

fn append_anthropic_message(messages: &mut Vec<Value>, payload: Value) {
    let Some(payload_object) = payload.as_object() else {
        messages.push(payload);
        return;
    };
    let Some(role) = payload_object.get("role").and_then(Value::as_str) else {
        messages.push(payload);
        return;
    };

    if let Some(last) = messages.last_mut() {
        let same_role = last
            .get("role")
            .and_then(Value::as_str)
            .map(|last_role| last_role == role)
            .unwrap_or(false);
        if same_role {
            let next_content = payload_object
                .get("content")
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new()));
            merge_anthropic_content(last, next_content, role);
            return;
        }
    }
    messages.push(payload);
}

fn merge_anthropic_content(existing_message: &mut Value, next_content: Value, role: &str) {
    let Some(existing_object) = existing_message.as_object_mut() else {
        return;
    };
    let existing_content = existing_object
        .entry("content".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !existing_content.is_array() {
        *existing_content = json!([{"type": "text", "text": existing_content.clone()}]);
    }
    let Some(existing_blocks) = existing_content.as_array_mut() else {
        return;
    };
    match next_content {
        Value::Array(mut blocks) => existing_blocks.append(&mut blocks),
        Value::String(text) => existing_blocks.push(json!({"type": "text", "text": text})),
        other => existing_blocks.push(other),
    }
    if role == "user" {
        prioritize_anthropic_tool_results(existing_blocks);
    }
}

fn prioritize_anthropic_tool_results(blocks: &mut Vec<Value>) {
    let mut tool_results = Vec::new();
    let mut other = Vec::new();
    for block in std::mem::take(blocks) {
        if block
            .get("type")
            .and_then(Value::as_str)
            .map(|kind| kind == "tool_result")
            .unwrap_or(false)
        {
            tool_results.push(block);
        } else {
            other.push(block);
        }
    }
    tool_results.append(&mut other);
    *blocks = tool_results;
}

fn anthropic_tool_result_content(result: &ToolResultData) -> Result<Value, AdapterError> {
    if result.image_data.is_some() {
        let mut blocks = Vec::new();
        let text = match &result.content {
            Value::String(text) => text.clone(),
            Value::Null => String::new(),
            value => json_compact(value),
        };
        if !text.is_empty() {
            blocks.push(json!({
                "type": "text",
                "text": text,
            }));
        }
        blocks.push(anthropic_image_block(&ImageData {
            url: None,
            data: result.image_data.clone(),
            media_type: Some(
                result
                    .image_media_type
                    .clone()
                    .unwrap_or_else(|| "image/png".to_string()),
            ),
            detail: None,
        })?);
        return Ok(Value::Array(blocks));
    }

    Ok(match &result.content {
        Value::String(text) => Value::String(text.clone()),
        Value::Null => Value::String(String::new()),
        other => Value::String(json_compact(other)),
    })
}

fn anthropic_image_block(image: &ImageData) -> Result<Value, AdapterError> {
    let source = match normalize_image_source(image, "anthropic")? {
        NormalizedImageSource::Url { url, .. } => {
            json!({
                "type": "url",
                "url": url,
            })
        }
        NormalizedImageSource::Data { data, media_type } => {
            json!({
                "type": "base64",
                "media_type": media_type,
                "data": encode_base64(&data),
            })
        }
    };
    Ok(json!({
        "type": "image",
        "source": source,
    }))
}

fn gemini_image_part(image: &ImageData) -> Result<Value, AdapterError> {
    match normalize_image_source(image, "gemini")? {
        NormalizedImageSource::Url { url, media_type } => Ok(json!({
            "fileData": {
                "fileUri": url,
                "mimeType": media_type,
            },
        })),
        NormalizedImageSource::Data { data, media_type } => Ok(json!({
            "inlineData": {
                "data": encode_base64(&data),
                "mimeType": media_type,
            },
        })),
    }
}

fn normalized_image_validation_error(provider: &'static str, message: String) -> AdapterError {
    AdapterError::provider(
        AdapterErrorKind::InvalidRequest,
        message,
        Some(provider.to_string()),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NormalizedImageSource {
    Url { url: String, media_type: String },
    Data { data: Vec<u8>, media_type: String },
}

fn normalize_image_source(
    image: &ImageData,
    provider: &'static str,
) -> Result<NormalizedImageSource, AdapterError> {
    image
        .validate()
        .map_err(|message| normalized_image_validation_error(provider, message))?;

    if let Some(url) = image.url.as_deref() {
        if is_local_media_path(url) {
            let path = expand_user_path(url);
            let data = std::fs::read(&path).map_err(|error| {
                normalized_image_validation_error(
                    provider,
                    format!(
                        "unable to read local image input from {}: {error}",
                        path.display()
                    ),
                )
            })?;
            let media_type = image
                .media_type
                .clone()
                .unwrap_or_else(|| infer_image_media_type(&path));
            return Ok(NormalizedImageSource::Data { data, media_type });
        }
        let media_type = image
            .media_type
            .clone()
            .unwrap_or_else(|| infer_image_media_type_from_value(url));
        return Ok(NormalizedImageSource::Url {
            url: url.to_string(),
            media_type,
        });
    }

    let data = image.data.clone().ok_or_else(|| {
        normalized_image_validation_error(
            provider,
            "exactly one of url or data must be provided for image".to_string(),
        )
    })?;
    let media_type = image
        .media_type
        .clone()
        .unwrap_or_else(|| "image/png".to_string());
    Ok(NormalizedImageSource::Data { data, media_type })
}

fn is_local_media_path(value: &str) -> bool {
    value.starts_with('/') || value.starts_with("./") || value.starts_with('~')
}

fn expand_user_path(value: &str) -> PathBuf {
    if value == "~" {
        return env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(value));
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(rest))
            .unwrap_or_else(|| PathBuf::from(value));
    }
    PathBuf::from(value)
}

fn infer_image_media_type(path: &Path) -> String {
    path.extension()
        .and_then(|extension| extension.to_str())
        .and_then(image_media_type_from_extension)
        .unwrap_or("image/png")
        .to_string()
}

fn infer_image_media_type_from_value(value: &str) -> String {
    let without_query = value
        .split_once('?')
        .map(|(value, _)| value)
        .unwrap_or(value);
    let without_fragment = without_query
        .split_once('#')
        .map(|(value, _)| value)
        .unwrap_or(without_query);
    infer_image_media_type(Path::new(without_fragment))
}

fn image_media_type_from_extension(extension: &str) -> Option<&'static str> {
    match extension.to_ascii_lowercase().as_str() {
        "avif" => Some("image/avif"),
        "bmp" => Some("image/bmp"),
        "gif" => Some("image/gif"),
        "heic" => Some("image/heic"),
        "heif" => Some("image/heif"),
        "ico" => Some("image/x-icon"),
        "jpeg" | "jpg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "svg" => Some("image/svg+xml"),
        "tif" | "tiff" => Some("image/tiff"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

fn openai_image_url(image: &NormalizedImageSource) -> String {
    match image {
        NormalizedImageSource::Url { url, .. } => url.clone(),
        NormalizedImageSource::Data { data, media_type } => {
            format!("data:{media_type};base64,{}", encode_base64(data))
        }
    }
}

fn gemini_generate_content_body(request: &Request) -> Result<Value, AdapterError> {
    let mut body = Map::new();
    if let Some(system) = instruction_text(&request.messages, "gemini")? {
        body.insert(
            "systemInstruction".to_string(),
            json!({
                "parts": [{"text": system}],
            }),
        );
    }

    if !request.tools.is_empty() {
        let declarations = request
            .tools
            .iter()
            .map(gemini_tool_declaration)
            .collect::<Result<Vec<_>, _>>()?;
        body.insert(
            "tools".to_string(),
            json!([{
                "functionDeclarations": declarations,
            }]),
        );
    }

    let tool_names = tool_names(&request.tools)?;
    if let Some(tool_config) = gemini_tool_config(request.tool_choice.as_ref(), &tool_names)? {
        body.insert("toolConfig".to_string(), tool_config);
    }

    let mut tool_call_names = BTreeMap::new();
    let mut contents = Vec::new();
    for message in &request.messages {
        if matches!(message.role, MessageRole::System | MessageRole::Developer) {
            continue;
        }
        contents.push(gemini_message_payload(message, &mut tool_call_names)?);
    }
    body.insert("contents".to_string(), Value::Array(contents));

    let portable_generation_config = gemini_generation_config_from_request(request);
    let mut generation_config = portable_generation_config.clone();
    let portable_body = Value::Object(body.clone());
    let mut native_tools = None;
    if let Some(options) = provider_options_object(request, "gemini")? {
        let (native_options, native_generation_config, options_tools) =
            split_gemini_provider_options(options)?;
        native_tools = options_tools;
        if let Some(native_generation_config) = native_generation_config {
            deep_insert(&mut generation_config, native_generation_config);
        }
        if !native_options.is_empty() {
            body = deep_merge(Value::Object(body), Value::Object(native_options))
                .as_object()
                .cloned()
                .unwrap_or_default();
        }
    }
    if generation_config
        .as_object()
        .map(|object| !object.is_empty())
        .unwrap_or(false)
    {
        deep_insert(&mut generation_config, portable_generation_config);
        body.insert("generationConfig".to_string(), generation_config);
    }

    let mut merged = Value::Object(body);
    deep_insert(&mut merged, portable_body);
    if let Some(native_tools) = native_tools {
        append_array_field(&mut merged, "tools", native_tools);
    }
    Ok(merged)
}

fn gemini_message_payload(
    message: &Message,
    tool_call_names: &mut BTreeMap<String, String>,
) -> Result<Value, AdapterError> {
    let role = match message.role {
        MessageRole::User | MessageRole::Tool => "user",
        MessageRole::Assistant => "model",
        other => {
            return Err(invalid_request_error(
                "gemini",
                format!("Gemini generateContent request does not support role {other:?} here"),
            ));
        }
    };

    let mut parts = Vec::new();
    for part in &message.content {
        match part {
            ContentPart::Text { text } => parts.push(json!({ "text": text })),
            ContentPart::Image { image } => parts.push(gemini_image_part(image)?),
            ContentPart::Thinking { thinking } => {
                validate_provider_thinking_source("gemini", thinking, "thinking")?;
                let mut payload = Map::new();
                payload.insert("text".to_string(), json!(thinking.text));
                payload.insert("thought".to_string(), json!(true));
                if let Some(signature) = non_empty(thinking.signature.as_deref()) {
                    payload.insert("thoughtSignature".to_string(), json!(signature));
                }
                parts.push(Value::Object(payload));
            }
            ContentPart::RedactedThinking { .. } => {
                return Err(invalid_request_error(
                    "gemini",
                    "Gemini generateContent request history does not support redacted_thinking content parts",
                ));
            }
            ContentPart::ToolCall { tool_call } => {
                if message.role != MessageRole::Assistant {
                    return Err(invalid_request_error(
                        "gemini",
                        "Gemini functionCall parts are only valid in assistant messages",
                    ));
                }
                remember_gemini_tool_call_name(tool_call_names, tool_call)?;
                parts.push(json!({
                    "functionCall": {
                        "name": tool_call.name,
                        "args": tool_call_arguments_object(tool_call, "gemini")?,
                    },
                }));
            }
            ContentPart::ToolResult { tool_result } => {
                if message.role != MessageRole::Tool {
                    return Err(invalid_request_error(
                        "gemini",
                        "Gemini functionResponse parts are only valid in tool messages",
                    ));
                }
                let name = message
                    .name
                    .clone()
                    .or_else(|| tool_call_names.get(&tool_result.tool_call_id).cloned())
                    .or_else(|| gemini_synthetic_tool_call_name(&tool_result.tool_call_id))
                    .ok_or_else(|| {
                        invalid_request_error(
                            "gemini",
                            format!(
                                "Gemini tool_result content requires a known function name for tool_call_id {:?}",
                                tool_result.tool_call_id
                            ),
                        )
                    })?;
                parts.push(json!({
                    "functionResponse": {
                        "id": tool_result.tool_call_id,
                        "name": name,
                        "response": gemini_tool_result_response(tool_result)?,
                    },
                }));
            }
            _ => {
                return Err(invalid_request_error(
                    "gemini",
                    format!(
                        "Gemini generateContent request does not support {} content parts",
                        part.kind().as_str()
                    ),
                ));
            }
        }
    }

    Ok(json!({
        "role": role,
        "parts": parts,
    }))
}

fn remember_gemini_tool_call_name(
    tool_call_names: &mut BTreeMap<String, String>,
    tool_call: &ToolCall,
) -> Result<(), AdapterError> {
    match tool_call_names.get(&tool_call.id) {
        Some(existing) if existing != &tool_call.name => Err(invalid_request_error(
            "gemini",
            format!(
                "Gemini tool_call id {:?} is associated with both {:?} and {:?}",
                tool_call.id, existing, tool_call.name
            ),
        )),
        Some(_) => Ok(()),
        None => {
            tool_call_names.insert(tool_call.id.clone(), tool_call.name.clone());
            Ok(())
        }
    }
}

fn gemini_tool_result_response(result: &ToolResultData) -> Result<Value, AdapterError> {
    Ok(match &result.content {
        Value::Object(object) => Value::Object(object.clone()),
        other => json!({ "result": other }),
    })
}

fn tool_call_arguments_object(
    tool_call: &ToolCall,
    provider: &'static str,
) -> Result<Value, AdapterError> {
    match &tool_call.arguments {
        Value::Object(object) => Ok(Value::Object(object.clone())),
        Value::String(text) => {
            let parsed: Value = serde_json::from_str(text).map_err(|_| {
                invalid_request_error(
                    provider,
                    format!("{provider} tool_call arguments must be a JSON object"),
                )
            })?;
            if parsed.is_object() {
                Ok(parsed)
            } else {
                Err(invalid_request_error(
                    provider,
                    format!("{provider} tool_call arguments must be a JSON object"),
                ))
            }
        }
        Value::Null => Ok(json!({})),
        _ => Err(invalid_request_error(
            provider,
            format!("{provider} tool_call arguments must be a JSON object"),
        )),
    }
}

fn openai_tool_definition(tool: &Value) -> Result<Value, AdapterError> {
    let definition = parse_tool_definition(tool, "openai")?;
    let mut function = Map::new();
    function.insert("name".to_string(), json!(definition.name));
    if let Some(description) = definition.description {
        function.insert("description".to_string(), json!(description));
    }
    if let Some(parameters) = definition.parameters {
        function.insert("parameters".to_string(), parameters);
    }
    Ok(json!({
        "type": "function",
        "function": Value::Object(function),
    }))
}

fn anthropic_tool_definition(tool: &Value) -> Result<Value, AdapterError> {
    let definition = parse_tool_definition(tool, "anthropic")?;
    let mut payload = Map::new();
    payload.insert("name".to_string(), json!(definition.name));
    payload.insert(
        "description".to_string(),
        json!(definition.description.unwrap_or_default()),
    );
    payload.insert(
        "input_schema".to_string(),
        definition.parameters.unwrap_or_else(|| {
            json!({
                "type": "object",
                "properties": {},
            })
        }),
    );
    if let Some(cache_control) = explicit_tool_cache_control(tool)? {
        payload.insert("cache_control".to_string(), cache_control);
    }
    Ok(Value::Object(payload))
}

fn gemini_tool_declaration(tool: &Value) -> Result<Value, AdapterError> {
    let definition = parse_tool_definition(tool, "gemini")?;
    let mut declaration = Map::new();
    declaration.insert("name".to_string(), json!(definition.name));
    if let Some(description) = definition.description {
        declaration.insert("description".to_string(), json!(description));
    }
    if let Some(parameters) = definition.parameters {
        declaration.insert("parametersJsonSchema".to_string(), parameters);
    }
    Ok(Value::Object(declaration))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToolDefinition {
    name: String,
    description: Option<String>,
    parameters: Option<Value>,
}

fn parse_tool_definition(
    tool: &Value,
    provider: &'static str,
) -> Result<ToolDefinition, AdapterError> {
    let object = tool.as_object().ok_or_else(|| {
        invalid_request_error(
            provider,
            format!("{provider} tool definitions must be objects"),
        )
    })?;
    let function = object.get("function").and_then(Value::as_object);
    let source = function.unwrap_or(object);
    let name = source
        .get("name")
        .and_then(Value::as_str)
        .and_then(non_empty_str)
        .ok_or_else(|| {
            invalid_request_error(
                provider,
                format!("{provider} tool definitions require a string name"),
            )
        })?
        .to_string();
    let description = source
        .get("description")
        .and_then(Value::as_str)
        .map(str::to_string);
    let parameters = source
        .get("parameters")
        .or_else(|| source.get("parametersJsonSchema"))
        .cloned();
    Ok(ToolDefinition {
        name,
        description,
        parameters,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ToolChoiceSpec {
    Auto,
    None,
    Required,
    Named(String),
    RawObject,
}

fn parse_tool_choice(value: &Value) -> Result<ToolChoiceSpec, AdapterError> {
    match value {
        Value::String(mode) => tool_choice_from_mode(mode, None),
        Value::Object(object) => {
            if let Some(function_name) = object
                .get("function")
                .and_then(Value::as_object)
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                .and_then(non_empty_str)
            {
                return Ok(ToolChoiceSpec::Named(function_name.to_string()));
            }
            let mode = object
                .get("mode")
                .or_else(|| object.get("type"))
                .and_then(Value::as_str);
            let tool_name = object
                .get("tool_name")
                .or_else(|| object.get("tool"))
                .or_else(|| object.get("name"))
                .and_then(Value::as_str)
                .and_then(non_empty_str);
            if let Some(mode) = mode {
                return tool_choice_from_mode(mode, tool_name);
            }
            Ok(ToolChoiceSpec::RawObject)
        }
        _ => Err(invalid_request_error(
            "native",
            "tool_choice must be a string or object",
        )),
    }
}

fn tool_choice_from_mode(
    mode: &str,
    tool_name: Option<&str>,
) -> Result<ToolChoiceSpec, AdapterError> {
    match mode.trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(ToolChoiceSpec::Auto),
        "none" => Ok(ToolChoiceSpec::None),
        "required" | "any" => Ok(ToolChoiceSpec::Required),
        "named" | "function" | "tool" => tool_name
            .map(|name| ToolChoiceSpec::Named(name.to_string()))
            .ok_or_else(|| {
                invalid_request_error("native", "named tool_choice requires a tool name")
            }),
        _ => Ok(ToolChoiceSpec::RawObject),
    }
}

fn openai_tool_choice_value(value: &Value) -> Value {
    match parse_tool_choice(value) {
        Ok(ToolChoiceSpec::Named(name)) => json!({
            "type": "function",
            "function": {"name": name},
        }),
        Ok(ToolChoiceSpec::Required) => json!("required"),
        Ok(ToolChoiceSpec::Auto) => json!("auto"),
        Ok(ToolChoiceSpec::None) => json!("none"),
        Ok(ToolChoiceSpec::RawObject) | Err(_) => value.clone(),
    }
}

fn anthropic_tool_choice_payload(
    value: Option<&ToolChoiceSpec>,
    tool_names: &BTreeSet<String>,
) -> Result<Option<Value>, AdapterError> {
    let Some(value) = value else {
        return Ok(None);
    };
    match value {
        ToolChoiceSpec::None => Ok(None),
        ToolChoiceSpec::Auto => {
            if tool_names.is_empty() {
                Ok(None)
            } else {
                Ok(Some(json!({"type": "auto"})))
            }
        }
        ToolChoiceSpec::Required => {
            if tool_names.is_empty() {
                return Err(unsupported_tool_choice(
                    "anthropic",
                    "Anthropic required tool_choice requires at least one tool",
                ));
            }
            Ok(Some(json!({"type": "any"})))
        }
        ToolChoiceSpec::Named(name) => {
            if !tool_names.contains(name) {
                return Err(unsupported_tool_choice(
                    "anthropic",
                    format!("Anthropic named tool_choice {name:?} requires a matching tool"),
                ));
            }
            Ok(Some(json!({"type": "tool", "name": name})))
        }
        ToolChoiceSpec::RawObject => Ok(None),
    }
}

fn gemini_tool_config(
    value: Option<&Value>,
    tool_names: &BTreeSet<String>,
) -> Result<Option<Value>, AdapterError> {
    let choice = value.map(parse_tool_choice).transpose()?;
    let choice = match choice {
        Some(choice) => choice,
        None if tool_names.is_empty() => return Ok(None),
        None => ToolChoiceSpec::Auto,
    };
    match choice {
        ToolChoiceSpec::Auto => Ok(Some(json!({
            "functionCallingConfig": {"mode": "AUTO"},
        }))),
        ToolChoiceSpec::None => Ok(Some(json!({
            "functionCallingConfig": {"mode": "NONE"},
        }))),
        ToolChoiceSpec::Required => {
            if tool_names.is_empty() {
                return Err(unsupported_tool_choice(
                    "gemini",
                    "Gemini required tool_choice requires at least one tool",
                ));
            }
            Ok(Some(json!({
                "functionCallingConfig": {"mode": "ANY"},
            })))
        }
        ToolChoiceSpec::Named(name) => {
            if !tool_names.contains(&name) {
                return Err(unsupported_tool_choice(
                    "gemini",
                    format!("Gemini named tool_choice {name:?} requires a matching tool"),
                ));
            }
            Ok(Some(json!({
                "functionCallingConfig": {
                    "mode": "ANY",
                    "allowedFunctionNames": [name],
                },
            })))
        }
        ToolChoiceSpec::RawObject => Ok(None),
    }
}

fn tool_names(tools: &[Value]) -> Result<BTreeSet<String>, AdapterError> {
    tools
        .iter()
        .map(|tool| parse_tool_definition(tool, "native").map(|definition| definition.name))
        .collect()
}

fn insert_generation_fields(
    body: &mut Map<String, Value>,
    request: &Request,
    shape: ProviderGenerationShape,
) {
    match shape {
        ProviderGenerationShape::OpenAiResponses => {
            if let Some(temperature) = request.temperature {
                body.insert("temperature".to_string(), json!(temperature));
            }
            if let Some(top_p) = request.top_p {
                body.insert("top_p".to_string(), json!(top_p));
            }
            if let Some(max_tokens) = request.max_tokens {
                body.insert("max_output_tokens".to_string(), json!(max_tokens));
            }
            if !request.stop_sequences.is_empty() {
                body.insert("stop".to_string(), json!(request.stop_sequences));
            }
        }
        ProviderGenerationShape::AnthropicMessages => {
            if let Some(temperature) = request.temperature {
                body.insert("temperature".to_string(), json!(temperature));
            }
            if let Some(top_p) = request.top_p {
                body.insert("top_p".to_string(), json!(top_p));
            }
            if !request.stop_sequences.is_empty() {
                body.insert("stop_sequences".to_string(), json!(request.stop_sequences));
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderGenerationShape {
    OpenAiResponses,
    AnthropicMessages,
}

fn gemini_generation_config_from_request(request: &Request) -> Value {
    let mut generation_config = Map::new();
    if let Some(temperature) = request.temperature {
        generation_config.insert("temperature".to_string(), json!(temperature));
    }
    if let Some(top_p) = request.top_p {
        generation_config.insert("topP".to_string(), json!(top_p));
    }
    if let Some(max_tokens) = request.max_tokens {
        generation_config.insert("maxOutputTokens".to_string(), json!(max_tokens));
    }
    if !request.stop_sequences.is_empty() {
        generation_config.insert("stopSequences".to_string(), json!(request.stop_sequences));
    }
    if let Some(response_format) = request.response_format.as_ref() {
        match response_format {
            ResponseFormat::Text => {
                generation_config.insert("responseMimeType".to_string(), json!("text/plain"));
            }
            ResponseFormat::JsonObject => {
                generation_config.insert("responseMimeType".to_string(), json!("application/json"));
            }
            ResponseFormat::JsonSchema { json_schema, .. } => {
                generation_config.insert("responseMimeType".to_string(), json!("application/json"));
                generation_config.insert("responseSchema".to_string(), json_schema.clone());
            }
        }
    }
    Value::Object(generation_config)
}

fn split_gemini_provider_options(
    options: &Map<String, Value>,
) -> Result<(Map<String, Value>, Option<Value>, Option<Vec<Value>>), AdapterError> {
    let native_options = select_json_fields(options, GEMINI_BODY_OPTION_KEYS);
    let native_tools = match options.get("tools") {
        Some(Value::Array(tools)) => Some(tools.clone()),
        Some(_) => {
            return Err(invalid_request_error(
                "gemini",
                "Gemini provider_options.tools must be an array",
            ));
        }
        None => None,
    };
    let mut generation_config = options
        .get("generationConfig")
        .cloned()
        .map(|value| {
            if value.is_object() {
                validate_gemini_thinking_config(&value)?;
                Ok(value)
            } else {
                Err(invalid_request_error(
                    "gemini",
                    "Gemini provider_options.generationConfig must be an object",
                ))
            }
        })
        .transpose()?;
    for key in GEMINI_GENERATION_OPTION_KEYS {
        if let Some(value) = options.get(*key) {
            if *key == "thinkingConfig" && !value.is_object() {
                return Err(invalid_request_error(
                    "gemini",
                    "Gemini provider_options.thinkingConfig must be an object",
                ));
            }
            let config = generation_config.get_or_insert_with(|| Value::Object(Map::new()));
            if let Some(object) = config.as_object_mut() {
                object.insert((*key).to_string(), value.clone());
            }
        }
    }
    Ok((native_options, generation_config, native_tools))
}

fn validate_gemini_thinking_config(value: &Value) -> Result<(), AdapterError> {
    let Some(thinking_config) = value.get("thinkingConfig") else {
        return Ok(());
    };
    if thinking_config.is_object() {
        Ok(())
    } else {
        Err(invalid_request_error(
            "gemini",
            "Gemini provider_options.generationConfig.thinkingConfig must be an object",
        ))
    }
}

fn openai_response_format(response_format: &ResponseFormat) -> Value {
    match response_format {
        ResponseFormat::Text => json!({"type": "text"}),
        ResponseFormat::JsonObject => json!({"type": "json_object"}),
        ResponseFormat::JsonSchema {
            json_schema,
            strict,
        } => json!({
            "type": "json_schema",
            "json_schema": json_schema,
            "strict": strict,
        }),
    }
}

fn anthropic_structured_output_instruction(
    request: &Request,
    active_options: Option<&Map<String, Value>>,
) -> Result<Option<String>, AdapterError> {
    let system_instruction = active_options
        .and_then(|options| options.get("system_instruction"))
        .and_then(Value::as_str)
        .and_then(non_empty_str)
        .map(str::to_string);
    let Some(response_format) = request.response_format.as_ref() else {
        return Ok(system_instruction);
    };
    let schema_instruction = match response_format {
        ResponseFormat::JsonSchema { json_schema, .. } => {
            let schema = serde_json::to_string(json_schema).map_err(|error| {
                invalid_request_error(
                    "anthropic",
                    format!("Anthropic json_schema response_format must serialize: {error}"),
                )
            })?;
            Some(format!(
                "Return only valid JSON that matches the provided schema.\n\nJSON Schema:\n```json\n{schema}\n```"
            ))
        }
        ResponseFormat::JsonObject => Some("Return only valid JSON.".to_string()),
        ResponseFormat::Text => None,
    };
    Ok(match (system_instruction, schema_instruction) {
        (Some(system), Some(schema)) => Some(format!("{system}\n\n{schema}")),
        (Some(system), None) => Some(system),
        (None, Some(schema)) => Some(schema),
        (None, None) => None,
    })
}

fn instruction_text(
    messages: &[Message],
    provider: &'static str,
) -> Result<Option<String>, AdapterError> {
    let mut fragments = Vec::new();
    for message in messages {
        if !matches!(message.role, MessageRole::System | MessageRole::Developer) {
            continue;
        }
        if let Some(text) = text_only_message(message, provider)? {
            fragments.push(text);
        }
    }
    Ok((!fragments.is_empty()).then(|| fragments.join("\n\n")))
}

fn text_only_message(
    message: &Message,
    provider: &'static str,
) -> Result<Option<String>, AdapterError> {
    let mut fragments = Vec::new();
    for part in &message.content {
        match part {
            ContentPart::Text { text } => fragments.push(text.as_str()),
            _ => {
                return Err(invalid_request_error(
                    provider,
                    format!("{provider} system and developer messages must be text only"),
                ));
            }
        }
    }
    Ok((!fragments.is_empty()).then(|| fragments.join("\n\n")))
}

fn provider_options_object<'a>(
    request: &'a Request,
    provider: &'static str,
) -> Result<Option<&'a Map<String, Value>>, AdapterError> {
    let Some(value) = request.provider_options.get(provider) else {
        return Ok(None);
    };
    value.as_object().map(Some).ok_or_else(|| {
        invalid_request_error(
            provider,
            format!("provider_options.{provider} must be a JSON object"),
        )
    })
}

fn anthropic_cache_control(
    options: Option<&Map<String, Value>>,
) -> Result<Option<Value>, AdapterError> {
    if let Some(value) = options.and_then(|options| options.get("cache_control")) {
        return Ok(Some(anthropic_cache_control_object(
            value,
            "Anthropic cache_control must be an object",
        )?));
    }
    if anthropic_auto_cache_enabled(options)? {
        Ok(Some(json!({"type": "ephemeral"})))
    } else {
        Ok(None)
    }
}

fn anthropic_auto_cache_enabled(
    options: Option<&Map<String, Value>>,
) -> Result<bool, AdapterError> {
    let Some(value) = options.and_then(|options| options.get("auto_cache")) else {
        return Ok(true);
    };
    value
        .as_bool()
        .ok_or_else(|| invalid_request_error("anthropic", "Anthropic auto_cache must be a boolean"))
}

fn anthropic_cache_control_object(
    value: &Value,
    message: &'static str,
) -> Result<Value, AdapterError> {
    if value.is_object() {
        Ok(value.clone())
    } else {
        Err(invalid_request_error("anthropic", message))
    }
}

fn explicit_tool_cache_control(tool: &Value) -> Result<Option<Value>, AdapterError> {
    let Some(value) = tool.get("cache_control") else {
        return Ok(None);
    };
    anthropic_cache_control_object(value, "Anthropic tool cache_control must be an object")
        .map(Some)
}

fn anthropic_message_cache_control(message: &Message) -> Result<Option<Value>, AdapterError> {
    let Some(Value::Object(options)) = message.provider_metadata.get("anthropic") else {
        return Ok(None);
    };
    let Some(value) = options.get("cache_control") else {
        return Ok(None);
    };
    anthropic_cache_control_object(value, "Anthropic message cache_control must be an object")
        .map(Some)
}

fn apply_anthropic_cache_control(body: &mut Map<String, Value>, cache_control: Option<&Value>) {
    let Some(cache_control) = cache_control else {
        return;
    };
    let mut count = cache_control_count(&Value::Object(body.clone()));
    if count >= ANTHROPIC_MAX_CACHE_BREAKPOINTS {
        return;
    }

    if let Some(system) = body.get_mut("system") {
        if apply_cache_control_to_system(system, cache_control) {
            count += 1;
        }
    }
    if count >= ANTHROPIC_MAX_CACHE_BREAKPOINTS {
        return;
    }

    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        if let Some(last_tool) = tools.last_mut() {
            if insert_cache_control_if_absent(last_tool, cache_control) {
                count += 1;
            }
        }
    }
    if count >= ANTHROPIC_MAX_CACHE_BREAKPOINTS {
        return;
    }

    if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
        if let Some(target_index) = anthropic_conversation_prefix_cache_target_index(messages) {
            apply_cache_control_to_message(&mut messages[target_index], cache_control);
        }
    }
}

fn apply_cache_control_to_system(system: &mut Value, cache_control: &Value) -> bool {
    match system {
        Value::String(text) if !text.is_empty() => {
            *system = json!([{
                "type": "text",
                "text": text.clone(),
                "cache_control": cache_control.clone(),
            }]);
            true
        }
        Value::Array(blocks) if !blocks.is_empty() => {
            apply_cache_control_to_last_block(blocks, cache_control)
        }
        _ => false,
    }
}

fn anthropic_conversation_prefix_cache_target_index(messages: &[Value]) -> Option<usize> {
    if messages.is_empty() {
        return None;
    }
    let last_index = messages.len() - 1;
    let last_is_user = messages[last_index]
        .get("role")
        .and_then(Value::as_str)
        .map(|role| role == "user")
        .unwrap_or(false);
    if last_is_user {
        if last_index > 0 {
            Some(last_index - 1)
        } else {
            None
        }
    } else {
        Some(last_index)
    }
}

fn apply_cache_control_to_message(message: &mut Value, cache_control: &Value) -> bool {
    let Some(content) = message
        .as_object_mut()
        .and_then(|object| object.get_mut("content"))
    else {
        return false;
    };
    match content {
        Value::String(text) if !text.is_empty() => {
            *content = json!([{
                "type": "text",
                "text": text.clone(),
                "cache_control": cache_control.clone(),
            }]);
            true
        }
        Value::Array(blocks) if !blocks.is_empty() => {
            apply_cache_control_to_last_block(blocks, cache_control)
        }
        _ => false,
    }
}

fn apply_cache_control_to_last_block(blocks: &mut [Value], cache_control: &Value) -> bool {
    blocks
        .last_mut()
        .map(|block| insert_cache_control_if_absent(block, cache_control))
        .unwrap_or(false)
}

fn insert_cache_control_if_absent(value: &mut Value, cache_control: &Value) -> bool {
    let Some(object) = value.as_object_mut() else {
        return false;
    };
    if object.contains_key("cache_control") {
        return false;
    }
    object.insert("cache_control".to_string(), cache_control.clone());
    true
}

fn value_contains_key(value: &Value, key: &str) -> bool {
    match value {
        Value::Object(object) => {
            object.contains_key(key) || object.values().any(|value| value_contains_key(value, key))
        }
        Value::Array(values) => values.iter().any(|value| value_contains_key(value, key)),
        _ => false,
    }
}

fn cache_control_count(value: &Value) -> usize {
    match value {
        Value::Object(object) => {
            usize::from(object.contains_key("cache_control"))
                + object.values().map(cache_control_count).sum::<usize>()
        }
        Value::Array(values) => values.iter().map(cache_control_count).sum(),
        _ => 0,
    }
}

fn anthropic_beta_headers(
    options: Option<&Map<String, Value>>,
    prompt_caching_required: bool,
) -> Result<Option<Vec<String>>, AdapterError> {
    let Some(value) = options.and_then(|options| options.get("beta_headers")) else {
        return Ok(prompt_caching_required.then(|| vec![ANTHROPIC_PROMPT_CACHING_BETA.to_string()]));
    };
    let values = match value {
        Value::String(text) => text
            .split(',')
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>(),
        Value::Array(items) => {
            let mut values = Vec::new();
            for item in items {
                let Some(text) = item.as_str().and_then(non_empty_str) else {
                    return Err(invalid_request_error(
                        "anthropic",
                        "Anthropic beta_headers array must contain strings",
                    ));
                };
                values.push(text.to_string());
            }
            values
        }
        _ => {
            return Err(invalid_request_error(
                "anthropic",
                "Anthropic beta_headers must be a string or array of strings",
            ));
        }
    };
    let mut seen = BTreeSet::new();
    let deduplicated = values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect::<Vec<_>>();
    let mut deduplicated = deduplicated;
    if prompt_caching_required
        && !deduplicated
            .iter()
            .any(|value| value == ANTHROPIC_PROMPT_CACHING_BETA)
    {
        deduplicated.push(ANTHROPIC_PROMPT_CACHING_BETA.to_string());
    }
    Ok((!deduplicated.is_empty()).then_some(deduplicated))
}

fn openai_responses_url(base_url: Option<&str>) -> String {
    append_path_segment(
        &normalize_openai_base_url(base_url),
        "responses",
        &["/responses"],
    )
}

fn anthropic_messages_url(base_url: Option<&str>) -> String {
    append_path_segment(
        &normalize_anthropic_base_url(base_url),
        "messages",
        &["/messages"],
    )
}

fn gemini_generate_content_url(base_url: Option<&str>, model: &str) -> String {
    let normalized = normalize_gemini_base_url(base_url);
    let model = model.trim().strip_prefix("models/").unwrap_or(model.trim());
    append_path_segment(
        &normalized,
        &format!(
            "models/{}:generateContent",
            percent_encode_path_segment(model)
        ),
        &[],
    )
}

fn gemini_stream_generate_content_url(base_url: Option<&str>, model: &str) -> String {
    let normalized = normalize_gemini_base_url(base_url);
    let model = model.trim().strip_prefix("models/").unwrap_or(model.trim());
    append_path_segment(
        &normalized,
        &format!(
            "models/{}:streamGenerateContent",
            percent_encode_path_segment(model)
        ),
        &[],
    )
}

fn normalize_openai_base_url(base_url: Option<&str>) -> String {
    let base = non_empty(base_url).unwrap_or(OPENAI_DEFAULT_BASE_URL);
    let normalized = trim_url_path_suffix(base, &[]);
    if normalized.ends_with("/v1") || normalized.ends_with("/responses") {
        normalized
    } else {
        append_path_segment(&normalized, "v1", &[])
    }
}

fn normalize_anthropic_base_url(base_url: Option<&str>) -> String {
    let base = non_empty(base_url).unwrap_or(ANTHROPIC_DEFAULT_BASE_URL);
    let normalized = trim_url_path_suffix(base, &["/messages"]);
    if normalized.ends_with("/v1") {
        normalized
    } else {
        append_path_segment(&normalized, "v1", &[])
    }
}

fn normalize_gemini_base_url(base_url: Option<&str>) -> String {
    let base = non_empty(base_url).unwrap_or(GEMINI_DEFAULT_BASE_URL);
    let mut normalized =
        trim_url_path_suffix(base, &["/generateContent", "/streamGenerateContent"]);
    if let Some(index) = normalized.find("/models/") {
        normalized.truncate(index);
        normalized = normalized.trim_end_matches('/').to_string();
    }
    if normalized.ends_with("/v1beta") {
        normalized
    } else if normalized.ends_with("/v1") {
        format!("{}v1beta", normalized.trim_end_matches("v1"))
    } else {
        append_path_segment(&normalized, "v1beta", &[])
    }
}

fn trim_url_path_suffix(value: &str, suffixes: &[&str]) -> String {
    let (mut base, suffix) = split_url_suffix(value.trim());
    base = base.trim_end_matches('/').to_string();
    for suffix_to_strip in suffixes {
        if base.ends_with(suffix_to_strip) {
            base.truncate(base.len() - suffix_to_strip.len());
            base = base.trim_end_matches('/').to_string();
            break;
        }
    }
    format!("{base}{suffix}")
}

fn append_path_segment(value: &str, segment: &str, terminal_suffixes: &[&str]) -> String {
    let (base, suffix) = split_url_suffix(value);
    let base = base.trim_end_matches('/');
    if terminal_suffixes
        .iter()
        .any(|terminal_suffix| base.ends_with(terminal_suffix))
    {
        return format!("{base}{suffix}");
    }
    format!("{base}/{segment}{suffix}")
}

fn append_query_pair(url: &str, key: &str, value: &str) -> String {
    let fragment_index = url.find('#');
    let (without_fragment, fragment) = match fragment_index {
        Some(index) => (&url[..index], &url[index..]),
        None => (url, ""),
    };
    let separator = if without_fragment.contains('?') {
        "&"
    } else {
        "?"
    };
    format!(
        "{without_fragment}{separator}{key}={}{fragment}",
        percent_encode_query_value(value)
    )
}

fn split_url_suffix(value: &str) -> (String, String) {
    let query = value.find('?');
    let fragment = value.find('#');
    let split_at = match (query, fragment) {
        (Some(query), Some(fragment)) => query.min(fragment),
        (Some(query), None) => query,
        (None, Some(fragment)) => fragment,
        (None, None) => return (value.to_string(), String::new()),
    };
    (value[..split_at].to_string(), value[split_at..].to_string())
}

fn percent_encode_path_segment(value: &str) -> String {
    percent_encode(value, false)
}

fn percent_encode_query_value(value: &str) -> String {
    percent_encode(value, true)
}

fn percent_encode(value: &str, space_plus: bool) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        let keep = matches!(
            byte,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~'
        );
        if keep {
            encoded.push(byte as char);
        } else if space_plus && byte == b' ' {
            encoded.push('+');
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn gemini_synthetic_tool_call_id(
    name: &str,
    candidate_index: usize,
    part_index: usize,
    arguments: &Value,
) -> String {
    format!(
        "gemini_call_{}_c{}_p{}_{}",
        percent_encode_path_segment(name),
        candidate_index,
        part_index,
        stable_digest(&stable_json(arguments))
    )
}

fn gemini_synthetic_tool_call_name(tool_call_id: &str) -> Option<String> {
    let suffix = tool_call_id.strip_prefix("gemini_call_")?;
    let (without_digest, digest) = suffix.rsplit_once('_')?;
    if digest.is_empty() || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let (without_part, part) = without_digest.rsplit_once('_')?;
    let part_index = part.strip_prefix('p')?;
    if part_index.is_empty() || !part_index.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let (encoded_name, candidate) = without_part.rsplit_once('_')?;
    let candidate_index = candidate.strip_prefix('c')?;
    if candidate_index.is_empty() || !candidate_index.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    percent_decode(encoded_name).filter(|name| !name.is_empty())
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return None;
            }
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn role_wire_value(role: MessageRole) -> &'static str {
    match role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
        MessageRole::Developer => "developer",
    }
}

fn encode_base64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn json_compact(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

fn stable_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string()),
        Value::Array(values) => {
            let items = values.iter().map(stable_json).collect::<Vec<_>>().join(",");
            format!("[{items}]")
        }
        Value::Object(object) => {
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            let items = entries
                .into_iter()
                .map(|(key, value)| {
                    let key = serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string());
                    format!("{key}:{}", stable_json(value))
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{items}}}")
        }
    }
}

fn stable_digest(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn deep_merge(base: Value, overlay: Value) -> Value {
    match (base, overlay) {
        (Value::Object(mut base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                let merged = if let Some(existing) = base.remove(&key) {
                    deep_merge(existing, value)
                } else {
                    value
                };
                base.insert(key, merged);
            }
            Value::Object(base)
        }
        (_, overlay) => overlay,
    }
}

fn deep_insert(target: &mut Value, protected: Value) {
    if let (Some(target), Value::Object(protected)) = (target.as_object_mut(), protected) {
        for (key, value) in protected {
            target.insert(key, value);
        }
    }
}

fn deep_insert_with_recursive_keys(target: &mut Value, protected: Value, recursive_keys: &[&str]) {
    let (Some(target), Value::Object(protected)) = (target.as_object_mut(), protected) else {
        return;
    };
    for (key, value) in protected {
        if recursive_keys.contains(&key.as_str()) {
            if let Some(existing) = target.get_mut(&key) {
                if existing.is_object() && value.is_object() {
                    deep_merge_protected_object(existing, value);
                    continue;
                }
            }
        }
        target.insert(key, value);
    }
}

fn deep_merge_protected_object(target: &mut Value, protected: Value) {
    match (target, protected) {
        (Value::Object(target), Value::Object(protected)) => {
            for (key, value) in protected {
                if let Some(existing) = target.get_mut(&key) {
                    deep_merge_protected_object(existing, value);
                } else {
                    target.insert(key, value);
                }
            }
        }
        (target, protected) => {
            *target = protected;
        }
    }
}

fn select_json_fields(options: &Map<String, Value>, allowed_keys: &[&str]) -> Map<String, Value> {
    let mut selected = Map::new();
    for key in allowed_keys {
        if let Some(value) = options.get(*key) {
            selected.insert((*key).to_string(), value.clone());
        }
    }
    selected
}

fn append_array_field(body: &mut Value, key: &str, items: Vec<Value>) {
    if items.is_empty() {
        return;
    }
    let Some(object) = body.as_object_mut() else {
        return;
    };
    match object.get_mut(key) {
        Some(Value::Array(existing)) => existing.extend(items),
        Some(_) | None => {
            object.insert(key.to_string(), Value::Array(items));
        }
    }
}

fn apply_bearer_auth(headers: &mut BTreeMap<String, String>, api_key: Option<&str>) {
    remove_header_case_insensitive(headers, "authorization");
    if let Some(api_key) = non_empty(api_key) {
        headers.insert("Authorization".to_string(), format!("Bearer {api_key}"));
    }
}

fn remove_header_case_insensitive(headers: &mut BTreeMap<String, String>, header: &str) {
    let matching_keys = headers
        .keys()
        .filter(|key| key.eq_ignore_ascii_case(header))
        .cloned()
        .collect::<Vec<_>>();
    for key in matching_keys {
        headers.remove(&key);
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn non_empty_str(value: &str) -> Option<&str> {
    non_empty(Some(value))
}

fn normalize_provider(provider: &str) -> String {
    provider.trim().to_ascii_lowercase()
}

fn validate_native_provider(provider: &str) -> Result<(), AdapterError> {
    match provider {
        "openai" | "anthropic" | "gemini" => Ok(()),
        other => Err(configuration_error(
            other,
            format!("Unsupported native provider {other:?}"),
        )),
    }
}

fn provider_status_error(provider: &str, response: NativeCompleteResponse) -> AdapterError {
    let (message, error_code) = provider_error_details(&response.body);
    error_from_status_code(
        Some(response.status),
        message,
        Some(provider),
        error_code.as_deref(),
        retry_after_from_headers(&response.headers),
        Some(response.body),
    )
}

fn native_stream_error_response(response: NativeStreamResponse) -> NativeCompleteResponse {
    let body = response
        .body
        .into_iter()
        .find_map(Result::ok)
        .unwrap_or_else(|| json!({}));
    NativeCompleteResponse {
        status: response.status,
        headers: response.headers,
        body,
    }
}

fn provider_error_details(body: &Value) -> (String, Option<String>) {
    let (message, error_code) = extract_error_details_from_raw(body);
    (message.unwrap_or_else(|| json_compact(body)), error_code)
}

fn invalid_request_error(provider: &'static str, message: impl Into<String>) -> AdapterError {
    AdapterError::provider(
        AdapterErrorKind::InvalidRequest,
        message,
        Some(provider.to_string()),
    )
}

fn invalid_response_error(provider: &'static str, message: impl Into<String>) -> AdapterError {
    AdapterError::provider(
        AdapterErrorKind::Provider,
        message,
        Some(provider.to_string()),
    )
}

fn unsupported_tool_choice(provider: &'static str, message: impl Into<String>) -> AdapterError {
    AdapterError::provider(
        AdapterErrorKind::UnsupportedToolChoice,
        message,
        Some(provider.to_string()),
    )
}

fn configuration_error(provider: &str, message: impl Into<String>) -> AdapterError {
    AdapterError::provider(
        AdapterErrorKind::Configuration,
        message,
        Some(provider.to_string()),
    )
}
