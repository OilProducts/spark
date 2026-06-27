use std::collections::{BTreeMap, BTreeSet};
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
use crate::native::{
    NativeCompleteRequest, NativeCompleteResponse, NativeCompleteTransport, NativeStreamResponse,
};
use crate::provider_utils::{ProviderStreamRecord, SseParser};
use crate::request::{
    ContentPart, FinishReason, FinishReasonKind, ImageData, Message, MessageRole, RateLimitInfo,
    Request, Response, ResponseFormat, ToolCall, Warning,
};
use crate::usage::Usage;

const DEFAULT_COMPATIBLE_BASE_URL: &str = "https://api.openai.com";
const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
const CHAT_COMPLETIONS_BODY_OPTION_KEYS: &[&str] = &[
    "frequency_penalty",
    "logit_bias",
    "logprobs",
    "max_completion_tokens",
    "max_tokens",
    "metadata",
    "n",
    "parallel_tool_calls",
    "presence_penalty",
    "response_format",
    "seed",
    "service_tier",
    "stop",
    "stream_options",
    "temperature",
    "tool_choice",
    "tools",
    "top_logprobs",
    "top_p",
    "user",
];
const RESPONSES_ONLY_OPTION_KEYS: &[&str] = &[
    "background",
    "conversation",
    "include",
    "instructions",
    "input",
    "max_output_tokens",
    "max_tool_calls",
    "previous_response_id",
    "prompt",
    "reasoning",
    "safety_identifier",
    "store",
    "text",
    "truncation",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAICompatibleRequestConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default)]
    pub default_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub require_api_key: bool,
}

impl Default for OpenAICompatibleRequestConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: None,
            default_headers: BTreeMap::new(),
            require_api_key: false,
        }
    }
}

impl OpenAICompatibleRequestConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: Some(api_key.into()),
            require_api_key: true,
            ..Self::default()
        }
    }
}

impl From<&OpenAICompatibleRequestConfig> for OpenAICompatibleRequestConfig {
    fn from(config: &OpenAICompatibleRequestConfig) -> Self {
        config.clone()
    }
}

impl From<&ProviderConfig> for OpenAICompatibleRequestConfig {
    fn from(config: &ProviderConfig) -> Self {
        let mut default_headers = BTreeMap::new();
        for key in ["HTTP-Referer", "X-Title"] {
            if let Some(value) = config.options.get(key) {
                default_headers.insert(key.to_string(), value.clone());
            }
        }
        let require_api_key = config
            .options
            .get("require_api_key")
            .map(|value| value != "false")
            .unwrap_or_else(|| config.provider == "openrouter");
        Self {
            api_key: config.api_key.clone(),
            base_url: config.base_url.clone(),
            default_headers,
            require_api_key,
        }
    }
}

#[derive(Clone)]
pub struct OpenAICompatibleAdapter {
    provider: String,
    config: OpenAICompatibleRequestConfig,
    transport: Arc<dyn NativeCompleteTransport>,
}

impl OpenAICompatibleAdapter {
    pub fn new(
        provider: impl AsRef<str>,
        config: impl Into<OpenAICompatibleRequestConfig>,
        transport: Arc<dyn NativeCompleteTransport>,
    ) -> Result<Self, AdapterError> {
        let provider = normalize_provider(provider.as_ref());
        validate_compatible_provider(&provider)?;
        let config = config.into();
        validate_config(&provider, &config)?;
        Ok(Self {
            provider,
            config,
            transport,
        })
    }

    pub fn without_transport(
        provider: impl AsRef<str>,
        config: impl Into<OpenAICompatibleRequestConfig>,
    ) -> Result<Self, AdapterError> {
        let provider = normalize_provider(provider.as_ref());
        validate_compatible_provider(&provider)?;
        let config = config.into();
        validate_config(&provider, &config)?;
        Ok(Self {
            transport: Arc::new(MissingCompatibleTransport {
                provider: provider.clone(),
            }),
            provider,
            config,
        })
    }

    pub fn openai_compatible(
        config: impl Into<OpenAICompatibleRequestConfig>,
        transport: Arc<dyn NativeCompleteTransport>,
    ) -> Self {
        Self::new("openai_compatible", config, transport)
            .expect("openai_compatible is a supported compatible provider adapter")
    }
}

impl ProviderAdapter for OpenAICompatibleAdapter {
    fn name(&self) -> &str {
        &self.provider
    }

    fn complete(&self, request: Request) -> Result<Response, AdapterError> {
        let prepared =
            prepare_chat_completions_request(&self.provider, &request, self.config.clone(), false)?;
        let native_response = self.transport.complete(prepared.request)?;
        if !(200..=299).contains(&native_response.status) {
            return Err(provider_status_error(&self.provider, native_response));
        }
        translate_chat_completions_response_with_warnings(
            &self.provider,
            native_response.body,
            &native_response.headers,
            prepared.warnings,
        )
    }

    fn stream(&self, request: Request) -> Result<StreamEvents, AdapterError> {
        let prepared =
            prepare_chat_completions_request(&self.provider, &request, self.config.clone(), true)?;
        let native_response = self.transport.stream(prepared.request)?;
        if !(200..=299).contains(&native_response.status) {
            return Err(provider_status_error(
                &self.provider,
                native_stream_error_response(native_response),
            ));
        }
        Ok(translate_chat_completions_stream_response_with_warnings(
            &self.provider,
            native_response.body,
            &native_response.headers,
            prepared.warnings,
        ))
    }

    fn supports_tool_choice(&self, mode: &str) -> bool {
        matches!(
            mode.trim().to_ascii_lowercase().as_str(),
            "auto" | "none" | "required" | "any" | "named" | "function" | "tool"
        )
    }
}

#[derive(Clone)]
pub struct OpenRouterAdapter {
    inner: OpenAICompatibleAdapter,
}

impl OpenRouterAdapter {
    pub fn new(
        config: impl Into<OpenAICompatibleRequestConfig>,
        transport: Arc<dyn NativeCompleteTransport>,
    ) -> Result<Self, AdapterError> {
        let mut config = config.into();
        let explicitly_allows_missing_key =
            !config.require_api_key && non_empty(config.base_url.as_deref()).is_some();
        if config.base_url.is_none() {
            config.base_url = Some(OPENROUTER_BASE_URL.to_string());
        }
        if !explicitly_allows_missing_key {
            config.require_api_key = true;
        }
        Ok(Self {
            inner: OpenAICompatibleAdapter::new("openrouter", config, transport)?,
        })
    }

    pub fn without_transport(
        config: impl Into<OpenAICompatibleRequestConfig>,
    ) -> Result<Self, AdapterError> {
        let mut config = config.into();
        let explicitly_allows_missing_key =
            !config.require_api_key && non_empty(config.base_url.as_deref()).is_some();
        if config.base_url.is_none() {
            config.base_url = Some(OPENROUTER_BASE_URL.to_string());
        }
        if !explicitly_allows_missing_key {
            config.require_api_key = true;
        }
        Ok(Self {
            inner: OpenAICompatibleAdapter::without_transport("openrouter", config)?,
        })
    }
}

impl ProviderAdapter for OpenRouterAdapter {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn complete(&self, request: Request) -> Result<Response, AdapterError> {
        self.inner.complete(request)
    }

    fn stream(&self, request: Request) -> Result<StreamEvents, AdapterError> {
        self.inner.stream(request)
    }

    fn initialize(&self) -> Result<(), AdapterError> {
        self.inner.initialize()
    }

    fn close(&self) -> Result<(), AdapterError> {
        self.inner.close()
    }

    fn supports_tool_choice(&self, mode: &str) -> bool {
        self.inner.supports_tool_choice(mode)
    }
}

#[derive(Clone)]
pub struct LiteLLMAdapter {
    inner: OpenAICompatibleAdapter,
}

impl LiteLLMAdapter {
    pub fn new(
        config: impl Into<OpenAICompatibleRequestConfig>,
        transport: Arc<dyn NativeCompleteTransport>,
    ) -> Result<Self, AdapterError> {
        Ok(Self {
            inner: OpenAICompatibleAdapter::new("litellm", config, transport)?,
        })
    }

    pub fn without_transport(
        config: impl Into<OpenAICompatibleRequestConfig>,
    ) -> Result<Self, AdapterError> {
        Ok(Self {
            inner: OpenAICompatibleAdapter::without_transport("litellm", config)?,
        })
    }
}

impl ProviderAdapter for LiteLLMAdapter {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn complete(&self, request: Request) -> Result<Response, AdapterError> {
        self.inner.complete(request)
    }

    fn stream(&self, request: Request) -> Result<StreamEvents, AdapterError> {
        self.inner.stream(request)
    }

    fn initialize(&self) -> Result<(), AdapterError> {
        self.inner.initialize()
    }

    fn close(&self) -> Result<(), AdapterError> {
        self.inner.close()
    }

    fn supports_tool_choice(&self, mode: &str) -> bool {
        self.inner.supports_tool_choice(mode)
    }
}

#[derive(Debug, Clone)]
struct MissingCompatibleTransport {
    provider: String,
}

impl NativeCompleteTransport for MissingCompatibleTransport {
    fn complete(
        &self,
        _request: NativeCompleteRequest,
    ) -> Result<NativeCompleteResponse, AdapterError> {
        Err(AdapterError::provider(
            AdapterErrorKind::Configuration,
            format!(
                "Provider '{}' has a Rust Chat Completions compatibility adapter, but no HTTP transport is configured",
                self.provider
            ),
            Some(self.provider.clone()),
        ))
    }
}

pub fn build_openai_compatible_chat_request<C>(
    provider: &str,
    request: &Request,
    config: C,
) -> Result<NativeCompleteRequest, AdapterError>
where
    C: Into<OpenAICompatibleRequestConfig>,
{
    Ok(prepare_chat_completions_request(provider, request, config, false)?.request)
}

pub fn build_openai_compatible_chat_stream_request<C>(
    provider: &str,
    request: &Request,
    config: C,
) -> Result<NativeCompleteRequest, AdapterError>
where
    C: Into<OpenAICompatibleRequestConfig>,
{
    Ok(prepare_chat_completions_request(provider, request, config, true)?.request)
}

pub fn translate_chat_completions_response(
    provider: &str,
    payload: Value,
) -> Result<Response, AdapterError> {
    translate_chat_completions_response_with_warnings(
        provider,
        payload,
        &BTreeMap::new(),
        Vec::new(),
    )
}

pub fn translate_chat_completions_response_with_headers(
    provider: &str,
    payload: Value,
    headers: &BTreeMap<String, String>,
) -> Result<Response, AdapterError> {
    translate_chat_completions_response_with_warnings(provider, payload, headers, Vec::new())
}

pub fn translate_chat_completions_stream_response(
    provider: &str,
    body: Vec<Result<Value, AdapterError>>,
    headers: &BTreeMap<String, String>,
) -> StreamEvents {
    translate_chat_completions_stream_response_with_warnings(provider, body, headers, Vec::new())
}

#[derive(Debug, Clone)]
struct PreparedCompatibleRequest {
    request: NativeCompleteRequest,
    warnings: Vec<Warning>,
}

fn prepare_chat_completions_request<C>(
    provider: &str,
    request: &Request,
    config: C,
    stream: bool,
) -> Result<PreparedCompatibleRequest, AdapterError>
where
    C: Into<OpenAICompatibleRequestConfig>,
{
    let provider = normalize_provider(provider);
    validate_compatible_provider(&provider)?;
    request.validate_for_client().map_err(|message| {
        AdapterError::provider(
            AdapterErrorKind::InvalidRequest,
            message,
            Some(provider.clone()),
        )
    })?;
    let config = config.into();
    validate_config(&provider, &config)?;

    let active_options = provider_options_object(request, &provider)?;
    let mut warnings = Vec::new();
    let mut headers = config.default_headers.clone();
    apply_bearer_auth(&mut headers, config.api_key.as_deref());
    if let Some(options) = active_options {
        apply_provider_option_headers(&provider, &mut headers, options)?;
    }

    let mut body = chat_completions_body(&provider, request, active_options, &mut warnings)?;
    if stream {
        body.insert("stream".to_string(), json!(true));
    }

    Ok(PreparedCompatibleRequest {
        request: NativeCompleteRequest {
            provider,
            method: "POST".to_string(),
            url: chat_completions_url(config.base_url.as_deref()),
            headers,
            body: Value::Object(body),
        },
        warnings,
    })
}

fn chat_completions_body(
    provider: &str,
    request: &Request,
    active_options: Option<&Map<String, Value>>,
    warnings: &mut Vec<Warning>,
) -> Result<Map<String, Value>, AdapterError> {
    let mut body = Map::new();
    body.insert("model".to_string(), json!(request.model));
    body.insert(
        "messages".to_string(),
        Value::Array(chat_messages(provider, &request.messages)?),
    );
    if !request.tools.is_empty() {
        let tools = chat_tools(provider, &request.tools, warnings)?;
        if !tools.is_empty() {
            body.insert("tools".to_string(), Value::Array(tools));
        }
    }
    if let Some(tool_choice) = request.tool_choice.as_ref() {
        body.insert(
            "tool_choice".to_string(),
            chat_tool_choice_value(tool_choice),
        );
    }
    if let Some(temperature) = request.temperature {
        body.insert("temperature".to_string(), json!(temperature));
    }
    if let Some(top_p) = request.top_p {
        body.insert("top_p".to_string(), json!(top_p));
    }
    if let Some(max_tokens) = request.max_tokens {
        body.insert("max_tokens".to_string(), json!(max_tokens));
    }
    if !request.stop_sequences.is_empty() {
        body.insert("stop".to_string(), json!(request.stop_sequences));
    }
    if !request.metadata.is_empty() {
        body.insert("metadata".to_string(), json!(request.metadata));
    }
    if let Some(response_format) = request.response_format.as_ref() {
        body.insert(
            "response_format".to_string(),
            chat_response_format(response_format),
        );
    }
    if request.reasoning_effort.is_some() {
        warnings.push(unsupported_warning(
            "unsupported_reasoning_effort",
            "OpenAI-compatible Chat Completions ignores reasoning_effort; use the native OpenAI Responses adapter for Responses reasoning controls",
        ));
    }

    if let Some(options) = active_options {
        warn_responses_only_options(options, warnings);
        let mut selected = select_json_fields(options, CHAT_COMPLETIONS_BODY_OPTION_KEYS);
        selected.remove("model");
        let provider_tools = selected.remove("tools");
        selected.remove("headers");
        let mut merged = deep_merge(Value::Object(body), Value::Object(selected));
        if let Some(Value::Array(tools)) = provider_tools {
            let tools = chat_tools(provider, &tools, warnings)?;
            append_array_field(&mut merged, "tools", tools);
        } else if provider_tools.is_some() {
            return Err(invalid_request_error(
                provider,
                format!("provider_options.{provider}.tools must be an array"),
            ));
        }
        return merged.as_object().cloned().ok_or_else(|| {
            invalid_request_error(provider, "Chat Completions request body must be an object")
        });
    }

    Ok(body)
}

fn chat_messages(provider: &str, messages: &[Message]) -> Result<Vec<Value>, AdapterError> {
    messages
        .iter()
        .map(|message| chat_message(provider, message))
        .collect()
}

fn chat_message(provider: &str, message: &Message) -> Result<Value, AdapterError> {
    let mut payload = Map::new();
    payload.insert("role".to_string(), json!(role_wire_value(message.role)));
    if let Some(name) = non_empty(message.name.as_deref()) {
        payload.insert("name".to_string(), json!(name));
    }

    match message.role {
        MessageRole::Assistant => {
            let mut text_parts = Vec::new();
            let mut tool_calls = Vec::new();
            for part in &message.content {
                match part {
                    ContentPart::Text { text } => text_parts.push(text.as_str()),
                    ContentPart::ToolCall { tool_call } => {
                        tool_calls.push(chat_tool_call_payload(provider, tool_call)?);
                    }
                    ContentPart::Thinking { .. } | ContentPart::RedactedThinking { .. } => {
                        return Err(invalid_request_error(
                            provider,
                            "OpenAI-compatible Chat Completions request history does not support reasoning content parts",
                        ));
                    }
                    _ => {
                        return Err(invalid_request_error(
                            provider,
                            format!(
                                "OpenAI-compatible Chat Completions assistant messages do not support {} content parts",
                                part.kind().as_str()
                            ),
                        ));
                    }
                }
            }
            if !text_parts.is_empty() {
                payload.insert("content".to_string(), json!(text_parts.join("")));
            }
            if !tool_calls.is_empty() {
                payload.insert("tool_calls".to_string(), Value::Array(tool_calls));
            }
            if !payload.contains_key("content") && !payload.contains_key("tool_calls") {
                payload.insert("content".to_string(), json!(""));
            }
        }
        MessageRole::Tool => {
            let (tool_call_id, content) = chat_tool_message_content(provider, message)?;
            payload.insert("tool_call_id".to_string(), json!(tool_call_id));
            payload.insert("content".to_string(), json!(content));
        }
        MessageRole::System | MessageRole::Developer => {
            let text = text_only_message(provider, message)?;
            payload.insert("content".to_string(), json!(text));
        }
        MessageRole::User => {
            payload.insert("content".to_string(), chat_user_content(provider, message)?);
        }
    }

    Ok(Value::Object(payload))
}

fn chat_user_content(provider: &str, message: &Message) -> Result<Value, AdapterError> {
    if message.content.len() == 1 {
        if let Some(text) = message.content[0].text_content() {
            return Ok(json!(text));
        }
    }

    let mut parts = Vec::new();
    for part in &message.content {
        match part {
            ContentPart::Text { text } => parts.push(json!({
                "type": "text",
                "text": text,
            })),
            ContentPart::Image { image } => parts.push(chat_image_part(provider, image)?),
            _ => {
                return Err(invalid_request_error(
                    provider,
                    format!(
                        "OpenAI-compatible Chat Completions user messages do not support {} content parts",
                        part.kind().as_str()
                    ),
                ));
            }
        }
    }
    Ok(Value::Array(parts))
}

fn chat_tool_message_content(
    provider: &str,
    message: &Message,
) -> Result<(String, String), AdapterError> {
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
                            provider,
                            "OpenAI-compatible tool messages must use a single tool_call_id",
                        ));
                    }
                }
                call_id = Some(tool_result.tool_call_id.clone());
                if result.replace(tool_result.content.clone()).is_some() {
                    return Err(invalid_request_error(
                        provider,
                        "OpenAI-compatible tool messages support only one tool_result payload",
                    ));
                }
            }
            _ => {
                return Err(invalid_request_error(
                    provider,
                    "OpenAI-compatible tool messages support only text or tool_result content",
                ));
            }
        }
    }
    let call_id = call_id.ok_or_else(|| {
        invalid_request_error(
            provider,
            "OpenAI-compatible tool messages require a tool_call_id",
        )
    })?;
    let content = match (result, text.is_empty()) {
        (Some(_), false) => {
            return Err(invalid_request_error(
                provider,
                "OpenAI-compatible tool messages cannot mix text and tool_result content",
            ));
        }
        (Some(value), true) => chat_tool_output_value(value),
        (None, false) => text.join(""),
        (None, true) => {
            return Err(invalid_request_error(
                provider,
                "OpenAI-compatible tool messages require text or tool_result content",
            ));
        }
    };
    Ok((call_id, content))
}

fn chat_tool_output_value(value: Value) -> String {
    match value {
        Value::String(text) => text,
        other => json_compact(&other),
    }
}

fn chat_image_part(provider: &str, image: &ImageData) -> Result<Value, AdapterError> {
    let url = image_url(provider, image)?;
    let mut image_url = Map::new();
    image_url.insert("url".to_string(), json!(url));
    if let Some(detail) = non_empty(image.detail.as_deref()) {
        image_url.insert("detail".to_string(), json!(detail));
    }
    Ok(json!({
        "type": "image_url",
        "image_url": Value::Object(image_url),
    }))
}

fn chat_tool_call_payload(provider: &str, tool_call: &ToolCall) -> Result<Value, AdapterError> {
    tool_call.validate().map_err(|message| {
        AdapterError::provider(
            AdapterErrorKind::InvalidRequest,
            message,
            Some(provider.to_string()),
        )
    })?;
    Ok(json!({
        "id": tool_call.id,
        "type": tool_call.r#type,
        "function": {
            "name": tool_call.name,
            "arguments": tool_call.raw_arguments.clone().unwrap_or_else(|| json_compact(&tool_call.arguments)),
        },
    }))
}

fn chat_tools(
    provider: &str,
    tools: &[Value],
    warnings: &mut Vec<Warning>,
) -> Result<Vec<Value>, AdapterError> {
    let mut payloads = Vec::new();
    for tool in tools {
        let Some(object) = tool.as_object() else {
            return Err(invalid_request_error(
                provider,
                "OpenAI-compatible tool definitions must be objects",
            ));
        };
        let tool_type = object
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("function");
        if tool_type != "function" {
            warnings.push(unsupported_warning(
                "unsupported_responses_tool",
                format!(
                    "OpenAI-compatible Chat Completions ignores unsupported Responses tool type {tool_type}",
                ),
            ));
            continue;
        }
        let function = object.get("function").and_then(Value::as_object);
        let source = function.unwrap_or(object);
        let name = source
            .get("name")
            .and_then(Value::as_str)
            .and_then(non_empty_str)
            .ok_or_else(|| {
                invalid_request_error(
                    provider,
                    "OpenAI-compatible function tool definitions require a string name",
                )
            })?;
        let mut function_payload = Map::new();
        function_payload.insert("name".to_string(), json!(name));
        if let Some(description) = source.get("description").and_then(Value::as_str) {
            function_payload.insert("description".to_string(), json!(description));
        }
        if let Some(parameters) = source
            .get("parameters")
            .or_else(|| source.get("parametersJsonSchema"))
        {
            function_payload.insert("parameters".to_string(), parameters.clone());
        }
        payloads.push(json!({
            "type": "function",
            "function": Value::Object(function_payload),
        }));
    }
    Ok(payloads)
}

fn chat_tool_choice_value(value: &Value) -> Value {
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
            "openai_compatible",
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
                invalid_request_error(
                    "openai_compatible",
                    "named tool_choice requires a tool name",
                )
            }),
        _ => Ok(ToolChoiceSpec::RawObject),
    }
}

fn chat_response_format(response_format: &ResponseFormat) -> Value {
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

fn translate_chat_completions_response_with_warnings(
    provider: &str,
    payload: Value,
    headers: &BTreeMap<String, String>,
    mut warnings: Vec<Warning>,
) -> Result<Response, AdapterError> {
    let provider = normalize_provider(provider);
    validate_compatible_provider(&provider)?;
    let raw = payload.clone();
    let object = payload.as_object().ok_or_else(|| {
        invalid_response_error(
            &provider,
            "OpenAI-compatible Chat Completions response must be a JSON object",
        )
    })?;
    let choice = object
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(Value::as_object)
        .ok_or_else(|| {
            invalid_response_error(
                &provider,
                "OpenAI-compatible Chat Completions response requires choices[0]",
            )
        })?;
    let message = choice
        .get("message")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            invalid_response_error(
                &provider,
                "OpenAI-compatible Chat Completions response requires choices[0].message",
            )
        })?;
    let model = string_field(object, "model").unwrap_or_default();
    let mut content = chat_response_message_content(&provider, message)?;
    let tool_calls = chat_response_tool_calls(&provider, message.get("tool_calls"))?;
    for tool_call in &tool_calls {
        content.push(ContentPart::ToolCall {
            tool_call: tool_call.clone(),
        });
    }
    let text = content
        .iter()
        .filter_map(ContentPart::text_content)
        .collect::<Vec<_>>()
        .join("");
    let usage = usage_from_chat_completions(object.get("usage"), &mut warnings).normalized();

    Ok(Response {
        id: string_field(object, "id").unwrap_or_default(),
        model,
        provider,
        message: Message {
            role: MessageRole::Assistant,
            content,
            ..Message::default()
        },
        finish_reason: chat_finish_reason(
            choice.get("finish_reason").and_then(Value::as_str),
            !tool_calls.is_empty(),
            false,
        ),
        usage,
        raw: Some(raw),
        warnings: deduplicate_warnings(warnings),
        rate_limit: normalize_rate_limit_headers(headers),
        text,
        tool_calls,
        ..Response::default()
    })
}

fn chat_response_message_content(
    provider: &str,
    message: &Map<String, Value>,
) -> Result<Vec<ContentPart>, AdapterError> {
    let mut content = Vec::new();
    if let Some(value) = message.get("content") {
        match value {
            Value::String(text) => {
                if !text.is_empty() {
                    content.push(ContentPart::Text { text: text.clone() });
                }
            }
            Value::Array(parts) => {
                for part in parts {
                    if let Some(text) = chat_response_text_part(part) {
                        content.push(ContentPart::Text { text });
                    } else {
                        content.push(ContentPart::Provider { raw: part.clone() });
                    }
                }
            }
            Value::Null => {}
            other => {
                return Err(invalid_response_error(
                    provider,
                    format!("OpenAI-compatible message content must be string, array, or null; got {other:?}"),
                ));
            }
        }
    }
    Ok(content)
}

fn chat_response_text_part(value: &Value) -> Option<String> {
    value
        .as_object()
        .and_then(|object| {
            object
                .get("text")
                .and_then(Value::as_str)
                .or_else(|| object.get("content").and_then(Value::as_str))
        })
        .map(str::to_string)
}

fn chat_response_tool_calls(
    provider: &str,
    value: Option<&Value>,
) -> Result<Vec<ToolCall>, AdapterError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        return Err(invalid_response_error(
            provider,
            "OpenAI-compatible message tool_calls must be an array",
        ));
    };
    items
        .iter()
        .map(|item| chat_response_tool_call(provider, item))
        .collect()
}

fn chat_response_tool_call(provider: &str, value: &Value) -> Result<ToolCall, AdapterError> {
    let object = value.as_object().ok_or_else(|| {
        invalid_response_error(
            provider,
            "OpenAI-compatible message tool_calls entries must be objects",
        )
    })?;
    let function = object
        .get("function")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            invalid_response_error(
                provider,
                "OpenAI-compatible tool_calls entries require function objects",
            )
        })?;
    let name = string_field(function, "name").unwrap_or_default();
    let id = string_field(object, "id").unwrap_or_else(|| name.clone());
    let raw_arguments = function
        .get("arguments")
        .map(chat_arguments_raw)
        .unwrap_or_else(|| "{}".to_string());
    let arguments = parse_chat_arguments(&raw_arguments);
    Ok(ToolCall {
        id,
        name,
        arguments,
        raw_arguments: Some(raw_arguments),
        r#type: string_field(object, "type").unwrap_or_else(|| "function".to_string()),
    })
}

fn translate_chat_completions_stream_response_with_warnings(
    provider: &str,
    body: Vec<Result<Value, AdapterError>>,
    headers: &BTreeMap<String, String>,
    warnings: Vec<Warning>,
) -> StreamEvents {
    let provider = normalize_provider(provider);
    let body = compatible_stream_records(body);
    let state = ChatStreamState::new(provider, headers, warnings);
    let events = state.translate(body);
    stream_events(events.into_iter())
}

#[derive(Debug, Clone)]
struct ChatStreamState {
    provider: String,
    rate_limit: Option<RateLimitInfo>,
    warnings: Vec<Warning>,
    events: Vec<Result<StreamEvent, AdapterError>>,
    accumulator: StreamAccumulator,
    raw_payloads: Vec<Value>,
    started: bool,
    active_text: Option<String>,
    active_tool_calls: BTreeMap<String, ToolCall>,
    tool_call_order: Vec<String>,
    tool_call_aliases: BTreeMap<String, String>,
    next_tool_call_id: usize,
    id: String,
    model: String,
    finish_reason: Option<FinishReason>,
    usage: Option<Usage>,
}

impl ChatStreamState {
    fn new(provider: String, headers: &BTreeMap<String, String>, warnings: Vec<Warning>) -> Self {
        Self {
            provider,
            rate_limit: normalize_rate_limit_headers(headers),
            warnings,
            events: Vec::new(),
            accumulator: StreamAccumulator::default(),
            raw_payloads: Vec::new(),
            started: false,
            active_text: None,
            active_tool_calls: BTreeMap::new(),
            tool_call_order: Vec::new(),
            tool_call_aliases: BTreeMap::new(),
            next_tool_call_id: 0,
            id: String::new(),
            model: String::new(),
            finish_reason: None,
            usage: None,
        }
    }

    fn translate(
        mut self,
        records: Vec<Result<ProviderStreamRecord, AdapterError>>,
    ) -> Vec<Result<StreamEvent, AdapterError>> {
        for item in records {
            let record = match item {
                Ok(record) => record,
                Err(error) => {
                    if self.started {
                        self.events.push(Err(error));
                        return self.events;
                    }
                    return vec![Err(error)];
                }
            };
            if record.done {
                return self.finish();
            }
            let payload = match stream_record_payload(&self.provider, record) {
                Ok(payload) => payload,
                Err(error) => {
                    if self.started {
                        self.events.push(Err(error));
                        return self.events;
                    }
                    return vec![Err(error)];
                }
            };
            self.raw_payloads.push(payload.clone());
            if payload.get("error").is_some() {
                self.push_error(
                    provider_payload_error(&self.provider, &payload),
                    Some(payload),
                );
                return self.events;
            }
            if is_responses_stream_event(&payload) {
                let event_type = value_string(&payload, "type")
                    .or_else(|| value_string(&payload, "event"))
                    .unwrap_or_else(|| "response event".to_string());
                self.warnings.push(unsupported_warning(
                    "unsupported_responses_stream_event",
                    format!(
                        "OpenAI-compatible Chat Completions received unsupported Responses stream event {event_type}",
                    ),
                ));
                self.provider_event(payload);
                continue;
            }
            self.apply_chat_stream_payload(payload);
        }
        self.finish()
    }

    fn apply_chat_stream_payload(&mut self, payload: Value) {
        if let Some(id) = value_string(&payload, "id") {
            self.id = id;
        }
        if let Some(model) = value_string(&payload, "model") {
            self.model = model;
        }
        let raw = Some(payload.clone());
        if let Some(usage) = payload.get("usage") {
            let mut warnings = Vec::new();
            self.record_usage(usage_from_chat_completions(Some(usage), &mut warnings));
            self.warnings.extend(warnings);
        }
        let Some(choices) = payload.get("choices").and_then(Value::as_array) else {
            self.provider_event(payload);
            return;
        };
        if choices.is_empty() {
            self.ensure_started(raw);
            return;
        }
        for choice in choices {
            let Some(choice) = choice.as_object() else {
                continue;
            };
            let choice_index =
                value_identifier_map(choice, "index").unwrap_or_else(|| "0".to_string());
            if let Some(delta) = choice.get("delta").and_then(Value::as_object) {
                if delta.get("role").is_some() {
                    self.ensure_started(raw.clone());
                }
                if let Some(content) = delta.get("content").and_then(Value::as_str) {
                    self.text_delta(content.to_string(), raw.clone());
                }
                if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
                    for tool_call in tool_calls {
                        self.tool_call_delta(&choice_index, tool_call, raw.clone());
                    }
                }
            }
            if let Some(finish_reason) = choice.get("finish_reason").and_then(Value::as_str) {
                let has_tool_calls =
                    !self.active_tool_calls.is_empty() || !self.accumulator.tool_calls.is_empty();
                self.finish_reason = Some(chat_finish_reason(
                    Some(finish_reason),
                    has_tool_calls,
                    false,
                ));
            }
        }
    }

    fn push(&mut self, event: StreamEvent) {
        self.accumulator.push(event.clone());
        self.events.push(Ok(event));
    }

    fn ensure_started(&mut self, raw: Option<Value>) {
        if self.started {
            return;
        }
        self.started = true;
        self.push(StreamEvent {
            r#type: StreamEventType::StreamStart,
            response: Some(Response {
                id: self.id.clone(),
                model: self.model.clone(),
                provider: self.provider.clone(),
                warnings: deduplicate_warnings(self.warnings.clone()),
                rate_limit: self.rate_limit.clone(),
                ..Response::default()
            }),
            raw,
            ..StreamEvent::new(StreamEventType::StreamStart)
        });
    }

    fn text_delta(&mut self, delta: String, raw: Option<Value>) {
        if delta.is_empty() {
            return;
        }
        self.ensure_started(raw.clone());
        if self.active_text.is_none() {
            self.active_text = Some(String::new());
            self.push(StreamEvent {
                r#type: StreamEventType::TextStart,
                text_id: Some("text_0".to_string()),
                raw: raw.clone(),
                ..StreamEvent::new(StreamEventType::TextStart)
            });
        }
        if let Some(active_text) = self.active_text.as_mut() {
            active_text.push_str(&delta);
        }
        self.push(StreamEvent {
            delta: Some(delta),
            text_id: Some("text_0".to_string()),
            raw,
            ..StreamEvent::text_delta("")
        });
    }

    fn text_end(&mut self, raw: Option<Value>) {
        let Some(text) = self.active_text.take() else {
            return;
        };
        self.push(StreamEvent {
            r#type: StreamEventType::TextEnd,
            delta: Some(text),
            text_id: Some("text_0".to_string()),
            raw,
            ..StreamEvent::new(StreamEventType::TextEnd)
        });
    }

    fn tool_call_delta(&mut self, choice_index: &str, value: &Value, raw: Option<Value>) {
        let Some(object) = value.as_object() else {
            return;
        };
        let tool_index = value_identifier_map(object, "index").unwrap_or_else(|| "0".to_string());
        let alias = format!("{choice_index}:{tool_index}");
        let id = string_field(object, "id")
            .or_else(|| self.tool_call_aliases.get(&alias).cloned())
            .unwrap_or_else(|| {
                let id = format!("tool_call_{}", self.next_tool_call_id);
                self.next_tool_call_id += 1;
                id
            });
        self.tool_call_aliases.insert(alias, id.clone());
        let function = object.get("function").and_then(Value::as_object);
        let name = function
            .and_then(|function| string_field(function, "name"))
            .unwrap_or_default();
        let arguments_delta = function
            .and_then(|function| function.get("arguments"))
            .map(chat_arguments_raw)
            .unwrap_or_default();
        self.ensure_started(raw.clone());
        if !self.active_tool_calls.contains_key(&id) {
            let started = ToolCall {
                id: id.clone(),
                name: name.clone(),
                arguments: Value::String(String::new()),
                raw_arguments: Some(String::new()),
                r#type: string_field(object, "type").unwrap_or_else(|| "function".to_string()),
            };
            self.active_tool_calls.insert(id.clone(), started.clone());
            self.tool_call_order.push(id.clone());
            self.push(StreamEvent {
                r#type: StreamEventType::ToolCallStart,
                tool_call: Some(started),
                raw: raw.clone(),
                ..StreamEvent::new(StreamEventType::ToolCallStart)
            });
        }
        if let Some(current) = self.active_tool_calls.get_mut(&id) {
            if !name.is_empty() {
                current.name = name.clone();
            }
            let current_raw = current.raw_arguments.clone().unwrap_or_default();
            let merged_raw = format!("{current_raw}{arguments_delta}");
            current.raw_arguments = Some(merged_raw.clone());
            current.arguments = Value::String(merged_raw);
            if let Some(tool_type) = string_field(object, "type") {
                current.r#type = tool_type;
            }
        }
        self.push(StreamEvent {
            r#type: StreamEventType::ToolCallDelta,
            tool_call: Some(ToolCall {
                id,
                name,
                arguments: Value::String(arguments_delta.clone()),
                raw_arguments: Some(arguments_delta),
                r#type: string_field(object, "type").unwrap_or_else(|| "function".to_string()),
            }),
            raw,
            ..StreamEvent::new(StreamEventType::ToolCallDelta)
        });
    }

    fn close_tool_calls(&mut self, raw: Option<Value>) {
        let ids = std::mem::take(&mut self.tool_call_order);
        for id in ids {
            if let Some(tool_call) = self.active_tool_calls.remove(&id) {
                self.push(StreamEvent {
                    r#type: StreamEventType::ToolCallEnd,
                    tool_call: Some(finalize_stream_tool_call(tool_call)),
                    raw: raw.clone(),
                    ..StreamEvent::new(StreamEventType::ToolCallEnd)
                });
            }
        }
        let remaining = self.active_tool_calls.keys().cloned().collect::<Vec<_>>();
        for id in remaining {
            if let Some(tool_call) = self.active_tool_calls.remove(&id) {
                self.push(StreamEvent {
                    r#type: StreamEventType::ToolCallEnd,
                    tool_call: Some(finalize_stream_tool_call(tool_call)),
                    raw: raw.clone(),
                    ..StreamEvent::new(StreamEventType::ToolCallEnd)
                });
            }
        }
    }

    fn record_usage(&mut self, usage: Usage) {
        self.usage = merge_stream_usage(self.usage.take(), usage);
    }

    fn provider_event(&mut self, payload: Value) {
        self.ensure_started(Some(payload.clone()));
        self.push(StreamEvent::provider_event(payload));
    }

    fn push_error(&mut self, error: AdapterError, raw: Option<Value>) {
        self.ensure_started(raw.clone());
        self.text_end(raw.clone());
        self.close_tool_calls(raw.clone());
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

    fn finish(mut self) -> Vec<Result<StreamEvent, AdapterError>> {
        let raw = self.raw_payloads.last().cloned();
        self.ensure_started(raw.clone());
        self.text_end(raw.clone());
        self.close_tool_calls(raw.clone());
        let has_tool_calls = !self.accumulator.tool_calls.is_empty();
        let finish_reason = self.finish_reason.clone().unwrap_or_else(|| {
            if has_tool_calls {
                FinishReason::ToolCalls
            } else {
                FinishReason::Stop
            }
        });
        let response = self.current_response(finish_reason);
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

    fn current_response(&mut self, finish_reason: FinishReason) -> Response {
        let mut accumulated = self.accumulator.clone();
        let response = accumulated.finalize();
        let usage = self
            .usage
            .clone()
            .and_then(|usage| merge_stream_usage(Some(usage), response.usage.clone()))
            .or_else(|| merge_stream_usage(None, response.usage.clone()))
            .unwrap_or_default()
            .normalized();
        Response {
            id: if self.id.is_empty() {
                response.id
            } else {
                self.id.clone()
            },
            model: if self.model.is_empty() {
                response.model
            } else {
                self.model.clone()
            },
            provider: self.provider.clone(),
            message: response.message,
            finish_reason,
            usage,
            raw: self.raw_payloads.last().cloned(),
            warnings: deduplicate_warnings(self.warnings.clone()),
            rate_limit: self.rate_limit.clone(),
            text: response.text,
            tool_calls: response.tool_calls,
            raw_provider_events: self.raw_payloads.clone(),
        }
    }
}

fn compatible_stream_records(
    body: Vec<Result<Value, AdapterError>>,
) -> Vec<Result<ProviderStreamRecord, AdapterError>> {
    let mut parser = SseParser::default();
    let mut records = Vec::new();
    for item in body {
        match item {
            Ok(Value::String(chunk)) => records.extend(parser.push_str(&chunk).into_iter().map(Ok)),
            Ok(payload) => {
                records.extend(parser.finish().into_iter().map(Ok));
                records.push(Ok(ProviderStreamRecord::from_json(payload)));
            }
            Err(error) => {
                records.extend(parser.finish().into_iter().map(Ok));
                records.push(Err(error));
            }
        }
    }
    records.extend(parser.finish().into_iter().map(Ok));
    records
}

fn stream_record_payload(
    provider: &str,
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

fn is_responses_stream_event(payload: &Value) -> bool {
    payload
        .get("type")
        .or_else(|| payload.get("event"))
        .and_then(Value::as_str)
        .map(|event| event.starts_with("response."))
        .unwrap_or(false)
}

fn finalize_stream_tool_call(mut tool_call: ToolCall) -> ToolCall {
    let raw_arguments = tool_call.raw_arguments.clone().unwrap_or_default();
    tool_call.arguments = parse_chat_arguments(&raw_arguments);
    tool_call.raw_arguments = Some(raw_arguments);
    tool_call
}

fn usage_from_chat_completions(value: Option<&Value>, warnings: &mut Vec<Warning>) -> Usage {
    let source = value.unwrap_or(&Value::Null);
    if token_at(source, &["completion_tokens_details", "reasoning_tokens"])
        .or_else(|| token_at(source, &["output_tokens_details", "reasoning_tokens"]))
        .is_some()
    {
        warnings.push(unsupported_warning(
            "unsupported_reasoning_token_visibility",
            "OpenAI-compatible Chat Completions does not expose reasoning token visibility through the unified compatible adapter",
        ));
    }
    Usage {
        input_tokens: token_at(source, &["prompt_tokens"])
            .or_else(|| token_at(source, &["input_tokens"]))
            .unwrap_or(0),
        output_tokens: token_at(source, &["completion_tokens"])
            .or_else(|| token_at(source, &["output_tokens"]))
            .unwrap_or(0),
        total_tokens: token_at(source, &["total_tokens"]).unwrap_or(0),
        reasoning_tokens: None,
        cache_read_tokens: token_at(source, &["prompt_tokens_details", "cached_tokens"])
            .or_else(|| token_at(source, &["input_tokens_details", "cached_tokens"])),
        cache_write_tokens: None,
        raw: value.cloned(),
    }
}

fn chat_finish_reason(raw: Option<&str>, has_tool_calls: bool, is_error: bool) -> FinishReason {
    if is_error {
        return FinishReason::Error;
    }
    let Some(raw) = raw else {
        return if has_tool_calls {
            FinishReason::ToolCalls
        } else {
            FinishReason::Other
        };
    };
    let reason = match raw.trim().to_ascii_lowercase().as_str() {
        "stop" => {
            if has_tool_calls {
                FinishReasonKind::ToolCalls
            } else {
                FinishReasonKind::Stop
            }
        }
        "length" | "max_tokens" | "max_completion_tokens" => FinishReasonKind::Length,
        "tool_calls" | "function_call" => FinishReasonKind::ToolCalls,
        "content_filter" | "safety" | "refusal" => FinishReasonKind::ContentFilter,
        "error" | "failed" | "cancelled" | "canceled" => FinishReasonKind::Error,
        _ if has_tool_calls => FinishReasonKind::ToolCalls,
        _ => FinishReasonKind::Other,
    };
    FinishReason::from_provider(reason, raw.to_string())
}

fn apply_provider_option_headers(
    provider: &str,
    headers: &mut BTreeMap<String, String>,
    options: &Map<String, Value>,
) -> Result<(), AdapterError> {
    let Some(value) = options.get("headers") else {
        return Ok(());
    };
    let Some(object) = value.as_object() else {
        return Err(invalid_request_error(
            provider,
            format!("provider_options.{provider}.headers must be an object"),
        ));
    };
    for (name, value) in object {
        let header_value = match value {
            Value::String(text) => text.clone(),
            Value::Number(number) => number.to_string(),
            Value::Bool(value) => value.to_string(),
            _ => {
                return Err(invalid_request_error(
                    provider,
                    format!("provider_options.{provider}.headers values must be scalar"),
                ));
            }
        };
        remove_header_case_insensitive(headers, name);
        headers.insert(name.clone(), header_value);
    }
    Ok(())
}

fn warn_responses_only_options(options: &Map<String, Value>, warnings: &mut Vec<Warning>) {
    for key in RESPONSES_ONLY_OPTION_KEYS {
        if options.contains_key(*key) {
            warnings.push(unsupported_warning(
                "unsupported_responses_option",
                format!(
                    "OpenAI-compatible Chat Completions ignores Responses-only provider option {key}",
                ),
            ));
        }
    }
}

fn unsupported_warning(code: impl Into<String>, message: impl Into<String>) -> Warning {
    Warning {
        message: message.into(),
        code: Some(code.into()),
    }
}

fn deduplicate_warnings(warnings: Vec<Warning>) -> Vec<Warning> {
    let mut seen = BTreeSet::new();
    warnings
        .into_iter()
        .filter(|warning| {
            seen.insert((
                warning.code.clone().unwrap_or_default(),
                warning.message.clone(),
            ))
        })
        .collect()
}

fn image_url(provider: &str, image: &ImageData) -> Result<String, AdapterError> {
    match (image.url.as_deref(), image.data.as_ref()) {
        (Some(url), None) => Ok(url.to_string()),
        (None, Some(data)) => {
            let media_type = image
                .effective_media_type()
                .unwrap_or("image/png")
                .trim()
                .to_string();
            Ok(format!(
                "data:{media_type};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(data)
            ))
        }
        _ => Err(invalid_request_error(
            provider,
            "exactly one of url or data must be provided for image",
        )),
    }
}

fn text_only_message(provider: &str, message: &Message) -> Result<String, AdapterError> {
    let mut fragments = Vec::new();
    for part in &message.content {
        match part {
            ContentPart::Text { text } => fragments.push(text.as_str()),
            _ => {
                return Err(invalid_request_error(
                    provider,
                    format!(
                        "OpenAI-compatible Chat Completions {} messages must be text only",
                        role_wire_value(message.role)
                    ),
                ));
            }
        }
    }
    Ok(fragments.join(""))
}

fn parse_chat_arguments(raw_arguments: &str) -> Value {
    if raw_arguments.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(raw_arguments)
            .unwrap_or_else(|_| Value::String(raw_arguments.to_string()))
    }
}

fn chat_arguments_raw(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => json_compact(other),
    }
}

fn chat_completions_url(base_url: Option<&str>) -> String {
    append_path_segment(
        &normalize_compatible_base_url(base_url),
        "chat/completions",
        &["/chat/completions"],
    )
}

fn normalize_compatible_base_url(base_url: Option<&str>) -> String {
    let base = non_empty(base_url).unwrap_or(DEFAULT_COMPATIBLE_BASE_URL);
    let normalized = trim_url_path_suffix(base, &["/chat/completions", "/responses"]);
    if normalized.ends_with("/v1") {
        normalized
    } else {
        append_path_segment(&normalized, "v1", &[])
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

fn normalize_rate_limit_headers(headers: &BTreeMap<String, String>) -> Option<RateLimitInfo> {
    let rate_limit = RateLimitInfo {
        requests_remaining: header_u64(headers, &["x-ratelimit-remaining-requests"]),
        requests_limit: header_u64(headers, &["x-ratelimit-limit-requests"]),
        tokens_remaining: header_u64(headers, &["x-ratelimit-remaining-tokens"]),
        tokens_limit: header_u64(headers, &["x-ratelimit-limit-tokens"]),
        reset_at: header_string(
            headers,
            &[
                "x-ratelimit-reset",
                "x-ratelimit-reset-requests",
                "x-ratelimit-reset-tokens",
                "ratelimit-reset",
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

fn provider_options_object<'a>(
    request: &'a Request,
    provider: &str,
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

fn provider_payload_error(provider: &str, payload: &Value) -> AdapterError {
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

fn value_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn value_identifier_map(object: &Map<String, Value>, key: &str) -> Option<String> {
    object.get(key).and_then(|value| {
        value
            .as_str()
            .map(str::to_string)
            .or_else(|| value.as_u64().map(|value| value.to_string()))
            .or_else(|| value.as_i64().map(|value| value.to_string()))
    })
}

fn string_field(object: &Map<String, Value>, key: &str) -> Option<String> {
    object.get(key).and_then(Value::as_str).map(str::to_string)
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

fn json_compact(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

fn invalid_request_error(provider: &str, message: impl Into<String>) -> AdapterError {
    AdapterError::provider(
        AdapterErrorKind::InvalidRequest,
        message,
        Some(provider.to_string()),
    )
}

fn invalid_response_error(provider: &str, message: impl Into<String>) -> AdapterError {
    AdapterError::provider(
        AdapterErrorKind::Provider,
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

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn non_empty_str(value: &str) -> Option<&str> {
    non_empty(Some(value))
}

fn normalize_provider(provider: &str) -> String {
    provider.trim().to_ascii_lowercase()
}

fn validate_compatible_provider(provider: &str) -> Result<(), AdapterError> {
    match provider {
        "openai_compatible" | "openrouter" | "litellm" => Ok(()),
        other => Err(configuration_error(
            other,
            format!("Unsupported OpenAI-compatible provider {other:?}"),
        )),
    }
}

fn validate_config(
    provider: &str,
    config: &OpenAICompatibleRequestConfig,
) -> Result<(), AdapterError> {
    if provider == "litellm" && non_empty(config.base_url.as_deref()).is_none() {
        return Err(configuration_error(
            provider,
            "LiteLLM requires LITELLM_BASE_URL or an explicit base_url",
        ));
    }
    if config.require_api_key && non_empty(config.api_key.as_deref()).is_none() {
        let message = if provider == "openrouter" {
            "OpenRouter requires OPENROUTER_API_KEY unless require_api_key is explicitly disabled"
        } else {
            "OpenAI-compatible provider requires an API key"
        };
        return Err(configuration_error(provider, message));
    }
    Ok(())
}
