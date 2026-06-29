use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Map, Value};

use crate::errors::{AdapterError, AdapterErrorKind};
use crate::events::StreamEvent;
use crate::provider_utils::ProviderStreamRecord;
use crate::request::{
    ContentPart, FinishReason, FinishReasonKind, ImageData, Message, MessageRole, RateLimitInfo,
    Request, Response, ResponseFormat, ThinkingData, ToolCall, ToolResultData,
};
use crate::structured::STRUCTURED_OUTPUT_TOOL_NAME;
use crate::tools::{Tool, ToolChoiceKind};
use crate::usage::Usage;

use super::common::*;
use super::streaming::{stream_record_payload, ActiveStreamBlock, NativeStreamState};
use super::types::{NativeCompleteRequest, NativeRequestConfig};

const ANTHROPIC_DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const ANTHROPIC_PROMPT_CACHING_BETA: &str = "prompt-caching-2024-07-31";
const ANTHROPIC_MAX_CACHE_BREAKPOINTS: usize = 4;
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

    let timeout = config.timeout;
    Ok(NativeCompleteRequest {
        provider: "anthropic".to_string(),
        method: "POST".to_string(),
        url: anthropic_messages_url(config.base_url.as_deref()),
        headers,
        timeout,
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
pub(super) struct AnthropicStreamTranslator {
    state: NativeStreamState,
    active_blocks: BTreeMap<String, ActiveStreamBlock>,
    pub(super) finished: bool,
}

impl AnthropicStreamTranslator {
    pub(super) fn new(headers: &BTreeMap<String, String>) -> Self {
        Self {
            state: NativeStreamState::new("anthropic", headers),
            active_blocks: BTreeMap::new(),
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
        let event_type = record.event.clone().unwrap_or_default();
        let payload = match stream_record_payload("anthropic", record) {
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
        self.state.raw_payloads.push(payload.clone());
        let raw = Some(payload.clone());
        let event_type = if event_type.is_empty() {
            value_string(&payload, "type").unwrap_or_default()
        } else {
            event_type
        };

        match event_type.as_str() {
            "message_start" => {
                let response = payload.get("message").cloned().and_then(|value| {
                    translate_anthropic_messages_response(value, self.state.rate_limit.clone()).ok()
                });
                self.state.ensure_started(raw, response);
            }
            "content_block_start" => {
                let Some(block) = payload.get("content_block") else {
                    self.state.provider_event(payload);
                    return self.state.take_events();
                };
                let block_index =
                    value_identifier(&payload, "index").unwrap_or_else(|| "0".to_string());
                let block_type = value_string(block, "type").unwrap_or_default();
                match block_type.as_str() {
                    "text" => {
                        let text_id = format!("text_{block_index}");
                        self.active_blocks.insert(
                            block_index.clone(),
                            ActiveStreamBlock::Text(text_id.clone()),
                        );
                        if let Some(text) = value_string(block, "text") {
                            self.state.text_delta(Some(text_id), text, raw);
                        } else {
                            self.state.text_start(Some(text_id), raw);
                        }
                    }
                    "thinking" | "redacted_thinking" => {
                        let redacted = block_type == "redacted_thinking";
                        let metadata = anthropic_stream_thinking_metadata(
                            block,
                            redacted,
                            self.state
                                .last_response
                                .as_ref()
                                .and_then(|response| source_model(&response.model)),
                        );
                        self.active_blocks
                            .insert(block_index.clone(), ActiveStreamBlock::Reasoning);
                        if let Some(text) = value_string(block, "thinking")
                            .or_else(|| value_string(block, "text"))
                            .or_else(|| value_string(block, "data"))
                        {
                            self.state
                                .reasoning_delta_with_metadata(text, Some(metadata), raw);
                        } else {
                            self.state
                                .reasoning_start_with_metadata(Some(metadata), raw);
                        }
                    }
                    "tool_use" => {
                        let id = value_string(block, "id")
                            .unwrap_or_else(|| format!("toolu_{block_index}"));
                        self.active_blocks
                            .insert(block_index.clone(), ActiveStreamBlock::ToolCall(id.clone()));
                        let name = value_string(block, "name").unwrap_or_default();
                        let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                        let raw_arguments = if input.as_object().is_some_and(Map::is_empty) {
                            String::new()
                        } else {
                            json_compact(&input)
                        };
                        self.state.tool_call_start(
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
                    _ => self.state.provider_event(payload),
                }
            }
            "content_block_delta" => {
                let Some(delta) = payload.get("delta") else {
                    self.state.provider_event(payload);
                    return self.state.take_events();
                };
                let block_index =
                    value_identifier(&payload, "index").unwrap_or_else(|| "0".to_string());
                match value_string(delta, "type").as_deref() {
                    Some("text_delta") => {
                        if let Some(text) = value_string(delta, "text") {
                            let text_id = self
                                .active_blocks
                                .get(&block_index)
                                .and_then(|block| match block {
                                    ActiveStreamBlock::Text(text_id) => Some(text_id.clone()),
                                    _ => None,
                                })
                                .unwrap_or_else(|| format!("text_{block_index}"));
                            self.state.text_delta(Some(text_id), text, raw);
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(text) = value_string(delta, "thinking") {
                            self.state.reasoning_delta(text, raw);
                        }
                    }
                    Some("signature_delta") => {
                        if let Some(signature) = value_string(delta, "signature") {
                            self.state.reasoning_delta_with_metadata(
                                String::new(),
                                Some(ThinkingData {
                                    text: String::new(),
                                    signature: Some(signature),
                                    redacted: false,
                                    source_provider: Some("anthropic".to_string()),
                                    source_model: self
                                        .state
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
                        let id = self
                            .active_blocks
                            .get(&block_index)
                            .and_then(|block| match block {
                                ActiveStreamBlock::ToolCall(tool_call_id) => {
                                    Some(tool_call_id.clone())
                                }
                                _ => None,
                            })
                            .unwrap_or_else(|| format!("toolu_{block_index}"));
                        self.state.tool_call_delta(
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
                    _ => self.state.provider_event(payload),
                }
            }
            "content_block_stop" => {
                let block_index =
                    value_identifier(&payload, "index").unwrap_or_else(|| "0".to_string());
                match self.active_blocks.remove(&block_index) {
                    Some(ActiveStreamBlock::Text(text_id)) => {
                        self.state.text_end(Some(text_id), None, raw)
                    }
                    Some(ActiveStreamBlock::Reasoning) => self.state.close_reasoning(raw),
                    Some(ActiveStreamBlock::ToolCall(tool_call_id)) => {
                        self.state.tool_call_end_by_id(tool_call_id, raw)
                    }
                    None => {
                        self.state.close_all_text(raw.clone());
                        self.state.close_reasoning(raw.clone());
                        self.state.close_all_tool_calls(raw);
                    }
                }
            }
            "message_delta" => {
                if let Some(delta) = payload.get("delta") {
                    if let Some(stop_reason) = value_string(delta, "stop_reason") {
                        let has_tool_calls = !self.state.accumulator.tool_calls.is_empty()
                            || !self.state.active_tool_calls.is_empty();
                        self.state.finish_reason =
                            Some(anthropic_finish_reason(Some(stop_reason), has_tool_calls));
                    }
                }
                if let Some(usage) = payload.get("usage") {
                    self.state.record_usage(usage_from_anthropic(Some(usage)));
                }
            }
            "message_stop" => {
                self.finished = true;
                return self.state.finish(raw);
            }
            "error" => {
                self.finished = true;
                self.state
                    .push_error(provider_payload_error("anthropic", &payload), raw);
                return self.state.take_events();
            }
            _ => self.state.provider_event(payload),
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
pub(super) fn translate_anthropic_messages_response(
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
fn anthropic_messages_body(
    request: &Request,
    active_options: Option<&Map<String, Value>>,
) -> Result<Value, AdapterError> {
    let (mut messages, mut system_blocks) = anthropic_message_payloads(&request.messages)?;
    if let Some(instruction) = anthropic_system_instruction(active_options) {
        append_anthropic_system_text(&mut system_blocks, instruction);
    }
    let structured_output_tool = anthropic_structured_output_tool(request)?;
    if structured_output_tool.is_none() {
        if let Some(instruction) = anthropic_structured_output_instruction(request)? {
            append_anthropic_system_text(&mut system_blocks, instruction);
        }
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
        if let Some(output_config) = options.get("output_config") {
            if output_config.is_object() {
                body.insert("output_config".to_string(), output_config.clone());
            } else {
                return Err(invalid_request_error(
                    "anthropic",
                    "Anthropic provider_options.output_config must be an object",
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
        .map(|choice| tool_choice_kind(choice, "anthropic"))
        .transpose()?;
    let mut tool_names = tool_names(&request.tools)?;
    if !request.tools.is_empty() && !matches!(tool_choice, Some(ToolChoiceKind::None)) {
        let tools = request
            .tools
            .iter()
            .map(anthropic_tool_definition)
            .collect::<Result<Vec<_>, _>>()?;
        body.insert("tools".to_string(), Value::Array(tools));
    }
    if let Some(tool) = structured_output_tool {
        body.entry("tools".to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
            tools.push(tool);
        }
        tool_names.insert(STRUCTURED_OUTPUT_TOOL_NAME.to_string());
        body.insert(
            "tool_choice".to_string(),
            json!({"type": "tool", "name": STRUCTURED_OUTPUT_TOOL_NAME}),
        );
    } else if let Some(choice) = anthropic_tool_choice_payload(tool_choice.as_ref(), &tool_names)? {
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
fn anthropic_tool_definition(tool: &Tool) -> Result<Value, AdapterError> {
    validate_tool_definition(tool, "anthropic")?;
    let mut payload = Map::new();
    payload.insert("name".to_string(), json!(tool.name));
    payload.insert(
        "description".to_string(),
        json!(tool.description.clone().unwrap_or_default()),
    );
    payload.insert(
        "input_schema".to_string(),
        tool.parameters.clone().unwrap_or_else(|| {
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
fn anthropic_tool_choice_payload(
    value: Option<&ToolChoiceKind>,
    tool_names: &BTreeSet<String>,
) -> Result<Option<Value>, AdapterError> {
    let Some(value) = value else {
        return Ok(None);
    };
    match value {
        ToolChoiceKind::None => Ok(None),
        ToolChoiceKind::Auto => {
            if tool_names.is_empty() {
                Ok(None)
            } else {
                Ok(Some(json!({"type": "auto"})))
            }
        }
        ToolChoiceKind::Required => {
            if tool_names.is_empty() {
                return Err(unsupported_tool_choice(
                    "anthropic",
                    "Anthropic required tool_choice requires at least one tool",
                ));
            }
            Ok(Some(json!({"type": "any"})))
        }
        ToolChoiceKind::Named(name) => {
            if !tool_names.contains(name) {
                return Err(unsupported_tool_choice(
                    "anthropic",
                    format!("Anthropic named tool_choice {name:?} requires a matching tool"),
                ));
            }
            Ok(Some(json!({"type": "tool", "name": name})))
        }
    }
}
fn anthropic_system_instruction(active_options: Option<&Map<String, Value>>) -> Option<String> {
    active_options
        .and_then(|options| options.get("system_instruction"))
        .and_then(Value::as_str)
        .and_then(non_empty_str)
        .map(str::to_string)
}

fn anthropic_structured_output_tool(request: &Request) -> Result<Option<Value>, AdapterError> {
    let Some(input_schema) = anthropic_structured_output_tool_schema(request) else {
        return Ok(None);
    };
    if request
        .tools
        .iter()
        .any(|tool| tool.name == STRUCTURED_OUTPUT_TOOL_NAME)
    {
        return Ok(None);
    }

    let tool_choice = request
        .tool_choice
        .as_ref()
        .map(|choice| tool_choice_kind(choice, "anthropic"))
        .transpose()?;
    if !matches!(tool_choice, None | Some(ToolChoiceKind::Auto)) {
        return Ok(None);
    }

    Ok(Some(json!({
        "name": STRUCTURED_OUTPUT_TOOL_NAME,
        "description": "Return the final structured response.",
        "input_schema": input_schema,
    })))
}

fn anthropic_structured_output_tool_schema(request: &Request) -> Option<Value> {
    match request.response_format.as_ref()? {
        ResponseFormat::JsonObject => Some(json!({"type": "object"})),
        ResponseFormat::JsonSchema { json_schema, .. }
            if schema_supports_anthropic_tool_input(json_schema) =>
        {
            Some(json_schema.clone())
        }
        ResponseFormat::JsonSchema { .. } | ResponseFormat::Text => None,
    }
}

fn schema_supports_anthropic_tool_input(schema: &Value) -> bool {
    let Some(object) = schema.as_object() else {
        return false;
    };
    match object.get("type") {
        None => true,
        Some(Value::String(schema_type)) => schema_type == "object",
        Some(Value::Array(schema_types)) => schema_types
            .iter()
            .filter_map(Value::as_str)
            .any(|schema_type| schema_type == "object"),
        Some(_) => false,
    }
}

fn anthropic_structured_output_instruction(
    request: &Request,
) -> Result<Option<String>, AdapterError> {
    let Some(response_format) = request.response_format.as_ref() else {
        return Ok(None);
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
    Ok(schema_instruction)
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

fn explicit_tool_cache_control(tool: &Tool) -> Result<Option<Value>, AdapterError> {
    let Some(value) = tool.provider_metadata.get("cache_control") else {
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
fn anthropic_messages_url(base_url: Option<&str>) -> String {
    append_path_segment(
        &normalize_anthropic_base_url(base_url),
        "messages",
        &["/messages"],
    )
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
