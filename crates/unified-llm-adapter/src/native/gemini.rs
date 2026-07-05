use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Map, Value};

use crate::errors::{AdapterError, AdapterErrorKind};
use crate::events::StreamEvent;
use crate::provider_utils::ProviderStreamRecord;
use crate::request::{
    ContentPart, FinishReason, FinishReasonKind, ImageData, Message, MessageRole, RateLimitInfo,
    Request, Response, ResponseFormat, ThinkingData, ToolCall, ToolResultData,
};
use crate::tools::{Tool, ToolChoice, ToolChoiceKind};
use crate::usage::Usage;

use super::common::*;
use super::streaming::{stream_record_payload, NativeStreamState};
use super::types::{NativeCompleteRequest, NativeRequestConfig};

const GEMINI_DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com";
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

    let timeout = config.timeout;
    Ok(NativeCompleteRequest {
        provider: "gemini".to_string(),
        method: "POST".to_string(),
        url,
        headers,
        timeout,
        abort_signal: request.abort_signal.clone(),
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

    let timeout = config.timeout;
    Ok(NativeCompleteRequest {
        provider: "gemini".to_string(),
        method: "POST".to_string(),
        url,
        headers,
        timeout,
        abort_signal: request.abort_signal.clone(),
        body: gemini_generate_content_body(request)?,
    })
}
pub(super) struct GeminiStreamTranslator {
    state: NativeStreamState,
    active_text: String,
    active_reasoning: String,
    emitted_tool_calls: BTreeSet<String>,
    pub(super) finished: bool,
}

impl GeminiStreamTranslator {
    pub(super) fn new(headers: &BTreeMap<String, String>) -> Self {
        Self {
            state: NativeStreamState::new("gemini", headers),
            active_text: String::new(),
            active_reasoning: String::new(),
            emitted_tool_calls: BTreeSet::new(),
            finished: false,
        }
    }

    pub(super) fn apply(
        &mut self,
        item: Result<ProviderStreamRecord, AdapterError>,
    ) -> Vec<Result<StreamEvent, AdapterError>> {
        if self.finished {
            return Vec::new();
        }
        let record = match item {
            Ok(record) => record,
            Err(error) => {
                self.finished = true;
                if self.state.started {
                    self.state.push_iterator_error(error);
                    return self.state.take_events();
                }
                return vec![Err(error)];
            }
        };
        if record.done {
            self.finished = true;
            let raw = stream_raw_payload(&self.state.raw_payloads);
            return self.state.finish(raw);
        }
        let payload = match stream_record_payload("gemini", record) {
            Ok(payload) => payload,
            Err(error) => {
                self.finished = true;
                if self.state.started {
                    self.state.push_iterator_error(error);
                    return self.state.take_events();
                }
                return vec![Err(error)];
            }
        };
        for payload in gemini_stream_payloads(payload) {
            self.state.raw_payloads.push(payload.clone());
            let raw = Some(payload.clone());

            if payload.get("error").is_some() {
                self.finished = true;
                self.state
                    .push_error(provider_payload_error("gemini", &payload), raw);
                return self.state.take_events();
            }
            if !is_gemini_stream_payload(&payload) {
                self.state.provider_event(payload);
                continue;
            }

            let response = match translate_gemini_generate_content_response(
                payload.clone(),
                self.state.rate_limit.clone(),
            ) {
                Ok(response) => response,
                Err(error) => {
                    self.finished = true;
                    self.state.push_error(error, raw);
                    return self.state.take_events();
                }
            };
            self.state.finish_reason = Some(response.finish_reason.clone());
            self.state.record_usage(response.usage.clone());
            self.state.last_response = Some(Response {
                raw: None,
                ..response.clone()
            });
            self.state.ensure_started(
                raw.clone(),
                Some(Response {
                    raw: None,
                    ..response.clone()
                }),
            );

            let text = response.message.text();
            if !text.is_empty() {
                let delta = if text == self.active_text || self.active_text.starts_with(&text) {
                    String::new()
                } else if let Some(suffix) = text.strip_prefix(&self.active_text) {
                    suffix.to_string()
                } else {
                    text.clone()
                };
                if !delta.is_empty() {
                    self.state
                        .text_delta(Some("text_0".to_string()), delta, raw.clone());
                }
                if text.starts_with(&self.active_text) {
                    self.active_text = text;
                } else {
                    self.active_text.push_str(&response.message.text());
                }
            }

            let reasoning = response.reasoning().unwrap_or_default();
            if !reasoning.is_empty() {
                let delta = if reasoning == self.active_reasoning
                    || self.active_reasoning.starts_with(&reasoning)
                {
                    String::new()
                } else if let Some(suffix) = reasoning.strip_prefix(&self.active_reasoning) {
                    suffix.to_string()
                } else {
                    reasoning.clone()
                };
                if !delta.is_empty() {
                    self.state.reasoning_delta_with_metadata(
                        delta,
                        gemini_reasoning_metadata(&response),
                        raw.clone(),
                    );
                }
                if reasoning.starts_with(&self.active_reasoning) {
                    self.active_reasoning = reasoning;
                } else {
                    self.active_reasoning
                        .push_str(&response.reasoning().unwrap_or_default());
                }
            }

            let tool_calls = response.tool_calls();
            if !tool_calls.is_empty() && self.state.active_texts.contains_key("text_0") {
                self.state
                    .text_end(Some("text_0".to_string()), None, raw.clone());
            }

            for tool_call in tool_calls {
                let signature = format!(
                    "{}:{}:{}",
                    tool_call.id,
                    tool_call.name,
                    json_compact(&tool_call.arguments)
                );
                if self.emitted_tool_calls.insert(signature) {
                    self.state.tool_call_start(tool_call.clone(), raw.clone());
                    self.state.tool_call_end(Some(tool_call), raw.clone());
                }
            }

            if response_has_provider_content(&response) {
                self.state.provider_event(payload);
            }
        }
        self.state.take_events()
    }

    pub(super) fn finish_eof(&mut self) -> Vec<Result<StreamEvent, AdapterError>> {
        if self.finished {
            return Vec::new();
        }
        self.finished = true;
        let raw = stream_raw_payload(&self.state.raw_payloads);
        self.state.finish(raw)
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
pub(super) fn translate_gemini_generate_content_response(
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
                    invalid_tool_call_error(
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
fn gemini_tool_declaration(tool: &Tool) -> Result<Value, AdapterError> {
    validate_tool_definition(tool, "gemini")?;
    let mut declaration = Map::new();
    declaration.insert("name".to_string(), json!(tool.name));
    if let Some(description) = tool.description.as_ref() {
        declaration.insert("description".to_string(), json!(description));
    }
    if let Some(parameters) = tool.parameters.as_ref() {
        declaration.insert("parametersJsonSchema".to_string(), parameters.clone());
    }
    Ok(Value::Object(declaration))
}
fn gemini_tool_config(
    value: Option<&ToolChoice>,
    tool_names: &BTreeSet<String>,
) -> Result<Option<Value>, AdapterError> {
    let choice = value
        .map(|choice| tool_choice_kind(choice, "gemini"))
        .transpose()?;
    let choice = match choice {
        Some(choice) => choice,
        None if tool_names.is_empty() => return Ok(None),
        None => ToolChoiceKind::Auto,
    };
    match choice {
        ToolChoiceKind::Auto => Ok(Some(json!({
            "functionCallingConfig": {"mode": "AUTO"},
        }))),
        ToolChoiceKind::None => Ok(Some(json!({
            "functionCallingConfig": {"mode": "NONE"},
        }))),
        ToolChoiceKind::Required => {
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
        ToolChoiceKind::Named(name) => {
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
    }
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
