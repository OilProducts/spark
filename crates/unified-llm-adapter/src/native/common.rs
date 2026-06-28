use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde_json::{json, Map, Value};

use crate::errors::{
    error_from_status_code, extract_error_details_from_raw, retry_after_from_headers, AdapterError,
    AdapterErrorKind,
};
use crate::request::{
    ContentPart, ImageData, Message, MessageRole, RateLimitInfo, Request, ThinkingData, ToolCall,
};
use crate::tools::{Tool, ToolChoice, ToolChoiceKind};

use super::types::{NativeCompleteResponse, NativeStreamChunkResponse, NativeStreamResponse};

pub(super) fn stream_raw_payload(payloads: &[Value]) -> Option<Value> {
    match payloads.len() {
        0 => None,
        1 => payloads.first().cloned(),
        _ => Some(Value::Array(payloads.to_vec())),
    }
}
pub(super) fn value_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

pub(super) fn value_identifier(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(|value| {
        value
            .as_str()
            .map(str::to_string)
            .or_else(|| value.as_u64().map(|value| value.to_string()))
            .or_else(|| value.as_i64().map(|value| value.to_string()))
    })
}
pub(super) fn merge_tool_calls_for_stream(
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

pub(super) fn merge_stream_tool_arguments(
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

pub(super) fn provider_payload_error(provider: &'static str, payload: &Value) -> AdapterError {
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
pub(super) fn estimate_reasoning_tokens(parts: &[ContentPart]) -> Option<u64> {
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

pub(super) fn source_model(model: &str) -> Option<String> {
    non_empty(Some(model)).map(str::to_string)
}

pub(super) fn normalize_rate_limit_headers(
    headers: &BTreeMap<String, String>,
) -> Option<RateLimitInfo> {
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

pub(super) fn header_u64(headers: &BTreeMap<String, String>, names: &[&str]) -> Option<u64> {
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

pub(super) fn header_string(headers: &BTreeMap<String, String>, names: &[&str]) -> Option<String> {
    header_value(headers, names).map(str::to_string)
}

pub(super) fn header_value<'a>(
    headers: &'a BTreeMap<String, String>,
    names: &[&str],
) -> Option<&'a str> {
    for name in names {
        for (key, value) in headers {
            if key.eq_ignore_ascii_case(name) {
                return non_empty(Some(value));
            }
        }
    }
    None
}

pub(super) fn validate_provider_thinking_source(
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

pub(super) fn token_at(value: &Value, path: &[&str]) -> Option<u64> {
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

pub(super) fn text_from_value(value: &Value) -> Option<String> {
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

pub(super) fn string_field(object: &Map<String, Value>, key: &str) -> Option<String> {
    object.get(key).and_then(Value::as_str).map(str::to_string)
}

pub(super) fn non_empty_owned(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}
pub(super) fn normalized_image_validation_error(
    provider: &'static str,
    message: String,
) -> AdapterError {
    AdapterError::provider(
        AdapterErrorKind::InvalidRequest,
        message,
        Some(provider.to_string()),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum NormalizedImageSource {
    Url { url: String, media_type: String },
    Data { data: Vec<u8>, media_type: String },
}

pub(super) fn normalize_image_source(
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

pub(super) fn is_local_media_path(value: &str) -> bool {
    value.starts_with('/') || value.starts_with("./") || value.starts_with('~')
}

pub(super) fn expand_user_path(value: &str) -> PathBuf {
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

pub(super) fn infer_image_media_type(path: &Path) -> String {
    path.extension()
        .and_then(|extension| extension.to_str())
        .and_then(image_media_type_from_extension)
        .unwrap_or("image/png")
        .to_string()
}

pub(super) fn infer_image_media_type_from_value(value: &str) -> String {
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

pub(super) fn image_media_type_from_extension(extension: &str) -> Option<&'static str> {
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
pub(super) fn tool_call_arguments_object(
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
pub(super) fn validate_tool_definition(
    tool: &Tool,
    provider: &'static str,
) -> Result<(), AdapterError> {
    tool.validate()
        .map_err(|message| invalid_request_error(provider, message))
}

pub(super) fn tool_choice_kind(
    value: &ToolChoice,
    provider: &'static str,
) -> Result<ToolChoiceKind, AdapterError> {
    value
        .kind()
        .map_err(|error| error.into_adapter_error(provider))
}
pub(super) fn tool_names(tools: &[Tool]) -> Result<BTreeSet<String>, AdapterError> {
    tools
        .iter()
        .map(|tool| {
            validate_tool_definition(tool, "native")?;
            Ok(tool.name.clone())
        })
        .collect()
}

pub(super) fn insert_generation_fields(
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
pub(super) enum ProviderGenerationShape {
    OpenAiResponses,
    AnthropicMessages,
}
pub(super) fn instruction_text(
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

pub(super) fn text_only_message(
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

pub(super) fn provider_options_object<'a>(
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
pub(super) fn trim_url_path_suffix(value: &str, suffixes: &[&str]) -> String {
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

pub(super) fn append_path_segment(
    value: &str,
    segment: &str,
    terminal_suffixes: &[&str],
) -> String {
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

pub(super) fn append_query_pair(url: &str, key: &str, value: &str) -> String {
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

pub(super) fn split_url_suffix(value: &str) -> (String, String) {
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

pub(super) fn percent_encode_path_segment(value: &str) -> String {
    percent_encode(value, false)
}

pub(super) fn percent_encode_query_value(value: &str) -> String {
    percent_encode(value, true)
}

pub(super) fn percent_encode(value: &str, space_plus: bool) -> String {
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
pub(super) fn role_wire_value(role: MessageRole) -> &'static str {
    match role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
        MessageRole::Developer => "developer",
    }
}

pub(super) fn encode_base64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

pub(super) fn json_compact(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

pub(super) fn stable_json(value: &Value) -> String {
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

pub(super) fn stable_digest(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

pub(super) fn deep_merge(base: Value, overlay: Value) -> Value {
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

pub(super) fn deep_insert(target: &mut Value, protected: Value) {
    if let (Some(target), Value::Object(protected)) = (target.as_object_mut(), protected) {
        for (key, value) in protected {
            target.insert(key, value);
        }
    }
}

pub(super) fn deep_insert_with_recursive_keys(
    target: &mut Value,
    protected: Value,
    recursive_keys: &[&str],
) {
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

pub(super) fn deep_merge_protected_object(target: &mut Value, protected: Value) {
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

pub(super) fn select_json_fields(
    options: &Map<String, Value>,
    allowed_keys: &[&str],
) -> Map<String, Value> {
    let mut selected = Map::new();
    for key in allowed_keys {
        if let Some(value) = options.get(*key) {
            selected.insert((*key).to_string(), value.clone());
        }
    }
    selected
}

pub(super) fn append_array_field(body: &mut Value, key: &str, items: Vec<Value>) {
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

pub(super) fn apply_bearer_auth(headers: &mut BTreeMap<String, String>, api_key: Option<&str>) {
    remove_header_case_insensitive(headers, "authorization");
    if let Some(api_key) = non_empty(api_key) {
        headers.insert("Authorization".to_string(), format!("Bearer {api_key}"));
    }
}

pub(super) fn remove_header_case_insensitive(headers: &mut BTreeMap<String, String>, header: &str) {
    let matching_keys = headers
        .keys()
        .filter(|key| key.eq_ignore_ascii_case(header))
        .cloned()
        .collect::<Vec<_>>();
    for key in matching_keys {
        headers.remove(&key);
    }
}

pub(super) fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

pub(super) fn non_empty_str(value: &str) -> Option<&str> {
    non_empty(Some(value))
}

pub(super) fn normalize_provider(provider: &str) -> String {
    provider.trim().to_ascii_lowercase()
}

pub(super) fn validate_native_provider(provider: &str) -> Result<(), AdapterError> {
    match provider {
        "openai" | "anthropic" | "gemini" => Ok(()),
        other => Err(configuration_error(
            other,
            format!("Unsupported native provider {other:?}"),
        )),
    }
}

pub(super) fn provider_status_error(
    provider: &str,
    response: NativeCompleteResponse,
) -> AdapterError {
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

pub(super) fn native_stream_error_response(
    response: NativeStreamResponse,
) -> NativeCompleteResponse {
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

pub(super) fn native_stream_chunk_error_response(
    response: NativeStreamChunkResponse,
) -> NativeCompleteResponse {
    native_stream_error_response(response.into_buffered())
}

pub(super) fn provider_error_details(body: &Value) -> (String, Option<String>) {
    let (message, error_code) = extract_error_details_from_raw(body);
    (message.unwrap_or_else(|| json_compact(body)), error_code)
}

pub(super) fn invalid_request_error(
    provider: &'static str,
    message: impl Into<String>,
) -> AdapterError {
    AdapterError::provider(
        AdapterErrorKind::InvalidRequest,
        message,
        Some(provider.to_string()),
    )
}

pub(super) fn invalid_response_error(
    provider: &'static str,
    message: impl Into<String>,
) -> AdapterError {
    AdapterError::provider(
        AdapterErrorKind::Provider,
        message,
        Some(provider.to_string()),
    )
}

pub(super) fn invalid_tool_call_error(
    provider: &'static str,
    message: impl Into<String>,
) -> AdapterError {
    AdapterError::provider(
        AdapterErrorKind::InvalidToolCall,
        message,
        Some(provider.to_string()),
    )
}

pub(super) fn unsupported_tool_choice(
    provider: &'static str,
    message: impl Into<String>,
) -> AdapterError {
    AdapterError::provider(
        AdapterErrorKind::UnsupportedToolChoice,
        message,
        Some(provider.to_string()),
    )
}

pub(super) fn configuration_error(provider: &str, message: impl Into<String>) -> AdapterError {
    AdapterError::provider(
        AdapterErrorKind::Configuration,
        message,
        Some(provider.to_string()),
    )
}
