use std::collections::BTreeMap;

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

const OPENAI_DEFAULT_BASE_URL: &str = "https://api.openai.com";
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
    let timeout = config.timeout;
    Ok(NativeCompleteRequest {
        provider: "openai".to_string(),
        method: "POST".to_string(),
        url: openai_responses_url(config.base_url.as_deref()),
        headers,
        timeout,
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
pub(super) struct OpenAiStreamTranslator {
    state: NativeStreamState,
    tool_call_aliases: BTreeMap<String, String>,
    pub(super) finished: bool,
}

impl OpenAiStreamTranslator {
    pub(super) fn new(headers: &BTreeMap<String, String>) -> Self {
        Self {
            state: NativeStreamState::new("openai", headers),
            tool_call_aliases: BTreeMap::new(),
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
        let payload = match stream_record_payload("openai", record) {
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

        if event_type == "response.created" || event_type == "response.in_progress" {
            let response = payload.get("response").cloned().and_then(|value| {
                translate_openai_responses_response(value, self.state.rate_limit.clone()).ok()
            });
            self.state.ensure_started(raw, response);
            return self.state.take_events();
        }

        if matches!(
            event_type.as_str(),
            "response.output_text.delta" | "response.text.delta" | "response.refusal.delta"
        ) {
            if let Some(delta) = value_string(&payload, "delta") {
                self.state
                    .text_delta(openai_stream_item_id(&payload, "text"), delta, raw);
            }
            return self.state.take_events();
        }

        if matches!(
            event_type.as_str(),
            "response.output_text.done" | "response.text.done" | "response.refusal.done"
        ) {
            let final_text =
                value_string(&payload, "text").or_else(|| value_string(&payload, "delta"));
            self.state
                .text_end(openai_stream_item_id(&payload, "text"), final_text, raw);
            return self.state.take_events();
        }

        if event_type.contains("reasoning") && event_type.ends_with(".delta") {
            if let Some(delta) = value_string(&payload, "delta")
                .or_else(|| value_string(&payload, "text"))
                .or_else(|| value_string(&payload, "summary"))
            {
                self.state.reasoning_delta(delta, raw);
            }
            return self.state.take_events();
        }

        if event_type == "response.output_item.added" {
            if let Some(item) = payload.get("item") {
                if let Some(tool_call) = openai_stream_tool_call(item) {
                    remember_openai_stream_tool_call_aliases(
                        &mut self.tool_call_aliases,
                        &payload,
                        item,
                        &tool_call.id,
                    );
                    self.state.tool_call_start(tool_call, raw);
                } else {
                    self.state.provider_event(payload);
                }
            } else {
                self.state.provider_event(payload);
            }
            return self.state.take_events();
        }

        if event_type == "response.function_call_arguments.delta" {
            let id = openai_stream_tool_call_delta_id(&payload, &self.tool_call_aliases)
                .unwrap_or_else(|| "function_call".to_string());
            let name = value_string(&payload, "name").unwrap_or_default();
            let delta = value_string(&payload, "delta").unwrap_or_default();
            self.state.tool_call_delta(
                ToolCall {
                    id,
                    name,
                    arguments: Value::String(delta.clone()),
                    raw_arguments: Some(delta),
                    r#type: "function".to_string(),
                },
                raw,
            );
            return self.state.take_events();
        }

        if event_type == "response.output_item.done" {
            if let Some((item, mut tool_call)) = payload
                .get("item")
                .and_then(|item| openai_stream_tool_call(item).map(|tool_call| (item, tool_call)))
            {
                if let Some(id) =
                    openai_stream_tool_call_alias_id(&payload, Some(item), &self.tool_call_aliases)
                {
                    tool_call.id = id;
                }
                remember_openai_stream_tool_call_aliases(
                    &mut self.tool_call_aliases,
                    &payload,
                    item,
                    &tool_call.id,
                );
                self.state.tool_call_end(Some(tool_call), raw);
            } else if let Some((text_id, text)) =
                payload.get("item").and_then(openai_stream_text_item)
            {
                self.state.text_end(text_id, text, raw);
            } else {
                self.state.provider_event(payload);
            }
            return self.state.take_events();
        }

        if event_type == "response.completed" || event_type == "response.done" {
            if let Some(response_payload) = payload.get("response").cloned() {
                match translate_openai_responses_response(
                    response_payload,
                    self.state.rate_limit.clone(),
                ) {
                    Ok(response) => {
                        self.state.finish_reason = Some(response.finish_reason.clone());
                        self.state.record_usage(response.usage.clone());
                        self.state.last_response = Some(response);
                    }
                    Err(error) => {
                        self.finished = true;
                        self.state.push_error(error, raw);
                        return self.state.take_events();
                    }
                }
            }
            self.finished = true;
            return self.state.finish(raw);
        }

        if event_type == "response.failed" || event_type == "error" {
            self.finished = true;
            self.state
                .push_error(provider_payload_error("openai", &payload), raw);
            return self.state.take_events();
        }

        self.state.provider_event(payload);
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
pub(super) fn translate_openai_responses_response(
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
                    invalid_tool_call_error(
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
            openai_tool_choice_value(tool_choice)?,
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
fn openai_image_url(image: &NormalizedImageSource) -> String {
    match image {
        NormalizedImageSource::Url { url, .. } => url.clone(),
        NormalizedImageSource::Data { data, media_type } => {
            format!("data:{media_type};base64,{}", encode_base64(data))
        }
    }
}
fn openai_tool_definition(tool: &Tool) -> Result<Value, AdapterError> {
    validate_tool_definition(tool, "openai")?;
    let mut function = Map::new();
    function.insert("name".to_string(), json!(tool.name));
    if let Some(description) = tool.description.as_ref() {
        function.insert("description".to_string(), json!(description));
    }
    if let Some(parameters) = tool.parameters.as_ref() {
        function.insert("parameters".to_string(), parameters.clone());
    }
    Ok(json!({
        "type": "function",
        "function": Value::Object(function),
    }))
}
fn openai_tool_choice_value(value: &ToolChoice) -> Result<Value, AdapterError> {
    Ok(match tool_choice_kind(value, "openai")? {
        ToolChoiceKind::Named(name) => json!({
            "type": "function",
            "function": {"name": name},
        }),
        ToolChoiceKind::Required => json!("required"),
        ToolChoiceKind::Auto => json!("auto"),
        ToolChoiceKind::None => json!("none"),
    })
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
fn openai_responses_url(base_url: Option<&str>) -> String {
    append_path_segment(
        &normalize_openai_base_url(base_url),
        "responses",
        &["/responses"],
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
