use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::errors::AdapterError;
use crate::request::{
    ContentPart, FinishReason, Message, MessageRole, Response, ThinkingData, ToolCall,
};
use crate::usage::Usage;

const DEFAULT_TEXT_ID: &str = "text";
const DEFAULT_TOOL_CALL_ID: &str = "tool_call";

pub(crate) fn merge_stream_usage(current: Option<Usage>, update: Usage) -> Option<Usage> {
    if !usage_has_observations(&update) {
        return current.map(Usage::normalized);
    }

    let update = update.normalized();
    let Some(current) = current else {
        return Some(update);
    };
    let current = current.normalized();
    let input_tokens = merge_token_count(current.input_tokens, update.input_tokens);
    let output_tokens = merge_token_count(current.output_tokens, update.output_tokens);
    let total_tokens = (input_tokens + output_tokens)
        .max(current.total_tokens)
        .max(update.total_tokens);

    Some(Usage {
        input_tokens,
        output_tokens,
        total_tokens,
        reasoning_tokens: merge_optional_token_count(
            current.reasoning_tokens,
            update.reasoning_tokens,
        ),
        cache_read_tokens: merge_optional_token_count(
            current.cache_read_tokens,
            update.cache_read_tokens,
        ),
        cache_write_tokens: merge_optional_token_count(
            current.cache_write_tokens,
            update.cache_write_tokens,
        ),
        raw: update.raw.or(current.raw),
    })
}

fn usage_has_observations(usage: &Usage) -> bool {
    usage.input_tokens != 0
        || usage.output_tokens != 0
        || usage.total_tokens != 0
        || usage.reasoning_tokens.is_some()
        || usage.cache_read_tokens.is_some()
        || usage.cache_write_tokens.is_some()
        || usage.raw.is_some()
}

fn merge_token_count(current: u64, update: u64) -> u64 {
    if update == 0 {
        current
    } else if current == 0 {
        update
    } else {
        current.max(update)
    }
}

fn merge_optional_token_count(current: Option<u64>, update: Option<u64>) -> Option<u64> {
    match (current, update) {
        (None, None) => None,
        (Some(current), None) => Some(current),
        (None, Some(update)) => Some(update),
        (Some(current), Some(update)) => Some(current.max(update)),
    }
}

pub trait StreamEventStream: Iterator<Item = Result<StreamEvent, AdapterError>> + Send {
    fn close(&mut self) -> Result<(), AdapterError> {
        Ok(())
    }
}

pub type StreamEvents = Box<dyn StreamEventStream>;

pub fn stream_events<I>(iter: I) -> StreamEvents
where
    I: Iterator<Item = Result<StreamEvent, AdapterError>> + Send + 'static,
{
    Box::new(IteratorStream { iter })
}

pub fn managed_stream<I, F>(iter: I, close: F) -> StreamEvents
where
    I: Iterator<Item = Result<StreamEvent, AdapterError>> + Send + 'static,
    F: FnMut() -> Result<(), AdapterError> + Send + 'static,
{
    Box::new(ManagedStream {
        iter,
        close: Some(close),
    })
}

struct IteratorStream<I> {
    iter: I,
}

impl<I> Iterator for IteratorStream<I>
where
    I: Iterator<Item = Result<StreamEvent, AdapterError>>,
{
    type Item = Result<StreamEvent, AdapterError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }
}

impl<I> StreamEventStream for IteratorStream<I> where
    I: Iterator<Item = Result<StreamEvent, AdapterError>> + Send
{
}

struct ManagedStream<I, F>
where
    F: FnMut() -> Result<(), AdapterError>,
{
    iter: I,
    close: Option<F>,
}

impl<I, F> Iterator for ManagedStream<I, F>
where
    I: Iterator<Item = Result<StreamEvent, AdapterError>>,
    F: FnMut() -> Result<(), AdapterError>,
{
    type Item = Result<StreamEvent, AdapterError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }
}

impl<I, F> StreamEventStream for ManagedStream<I, F>
where
    I: Iterator<Item = Result<StreamEvent, AdapterError>> + Send,
    F: FnMut() -> Result<(), AdapterError> + Send,
{
    fn close(&mut self) -> Result<(), AdapterError> {
        let Some(mut close) = self.close.take() else {
            return Ok(());
        };
        close()
    }
}

impl<I, F> Drop for ManagedStream<I, F>
where
    F: FnMut() -> Result<(), AdapterError>,
{
    fn drop(&mut self) {
        if let Some(mut close) = self.close.take() {
            let _ = close();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEventType {
    StreamStart,
    TextStart,
    TextDelta,
    TextEnd,
    ReasoningStart,
    ReasoningDelta,
    ReasoningEnd,
    ToolCallStart,
    ToolCallDelta,
    ToolCallEnd,
    Finish,
    Error,
    ProviderEvent,
    Custom(String),
}

impl StreamEventType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::StreamStart => "stream_start",
            Self::TextStart => "text_start",
            Self::TextDelta => "text_delta",
            Self::TextEnd => "text_end",
            Self::ReasoningStart => "reasoning_start",
            Self::ReasoningDelta => "reasoning_delta",
            Self::ReasoningEnd => "reasoning_end",
            Self::ToolCallStart => "tool_call_start",
            Self::ToolCallDelta => "tool_call_delta",
            Self::ToolCallEnd => "tool_call_end",
            Self::Finish => "finish",
            Self::Error => "error",
            Self::ProviderEvent => "provider_event",
            Self::Custom(event_type) => event_type.as_str(),
        }
    }

    fn from_type(event_type: &str) -> Self {
        match event_type {
            "stream_start" => Self::StreamStart,
            "text_start" => Self::TextStart,
            "text_delta" => Self::TextDelta,
            "text_end" => Self::TextEnd,
            "reasoning_start" => Self::ReasoningStart,
            "reasoning_delta" => Self::ReasoningDelta,
            "reasoning_end" => Self::ReasoningEnd,
            "tool_call_start" => Self::ToolCallStart,
            "tool_call_delta" => Self::ToolCallDelta,
            "tool_call_end" => Self::ToolCallEnd,
            "finish" => Self::Finish,
            "error" => Self::Error,
            "provider_event" => Self::ProviderEvent,
            other => Self::Custom(other.to_string()),
        }
    }
}

impl Serialize for StreamEventType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for StreamEventType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let event_type = String::deserialize(deserializer)?;
        Ok(Self::from_type(&event_type))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamEvent {
    #[serde(rename = "type")]
    pub r#type: StreamEventType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_delta: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<FinishReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<Response>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<AdapterError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

impl StreamEvent {
    pub fn new(event_type: StreamEventType) -> Self {
        Self {
            r#type: event_type,
            delta: None,
            text_id: None,
            reasoning_delta: None,
            thinking: None,
            tool_call: None,
            finish_reason: None,
            usage: None,
            response: None,
            error: None,
            raw: None,
        }
    }

    pub fn text_delta(text: impl Into<String>) -> Self {
        Self {
            delta: Some(text.into()),
            ..Self::new(StreamEventType::TextDelta)
        }
    }

    pub fn finish(reason: FinishReason, usage: Option<Usage>) -> Self {
        Self {
            finish_reason: Some(reason),
            usage,
            ..Self::new(StreamEventType::Finish)
        }
    }

    pub fn reasoning_delta(text: impl Into<String>) -> Self {
        Self {
            reasoning_delta: Some(text.into()),
            ..Self::new(StreamEventType::ReasoningDelta)
        }
    }

    pub fn provider_event(raw: impl Into<Value>) -> Self {
        Self {
            raw: Some(raw.into()),
            ..Self::new(StreamEventType::ProviderEvent)
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StreamAccumulator {
    #[serde(default)]
    pub events: Vec<StreamEvent>,
    #[serde(default)]
    pub final_text: String,
    #[serde(default)]
    pub reasoning_text: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<FinishReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(default)]
    pub raw_provider_events: Vec<Value>,
    #[serde(default)]
    pub response: Response,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_event: Option<StreamEvent>,
    #[serde(default, skip)]
    active_text: BTreeMap<String, String>,
    #[serde(default, skip)]
    active_reasoning: Option<String>,
    #[serde(default, skip)]
    active_reasoning_metadata: Option<ThinkingData>,
    #[serde(default, skip)]
    reasoning_parts: Vec<ThinkingData>,
    #[serde(default, skip)]
    active_tool_calls: BTreeMap<String, ToolCall>,
    #[serde(default, skip)]
    active_tool_call_order: Vec<String>,
}

impl StreamAccumulator {
    pub fn from_events(events: impl IntoIterator<Item = StreamEvent>) -> Self {
        let mut accumulator = Self::default();
        for event in events {
            accumulator.push(event);
        }
        accumulator
    }

    pub fn push(&mut self, event: StreamEvent) {
        if let Some(raw) = event.raw.clone() {
            self.raw_provider_events.push(raw);
        }

        match event.r#type {
            StreamEventType::StreamStart => self.overlay_response(event.response.clone()),
            StreamEventType::TextStart => {
                self.start_text(event.text_id.as_deref(), event.delta.as_deref())
            }
            StreamEventType::TextDelta => {
                self.append_text(event.text_id.as_deref(), event.delta.as_deref())
            }
            StreamEventType::TextEnd => {
                self.end_text(event.text_id.as_deref(), event.delta.as_deref())
            }
            StreamEventType::ReasoningStart => {
                self.start_reasoning(event.thinking.as_ref(), reasoning_event_text(&event))
            }
            StreamEventType::ReasoningDelta => {
                self.append_reasoning(event.thinking.as_ref(), reasoning_event_text(&event))
            }
            StreamEventType::ReasoningEnd => {
                self.end_reasoning(event.thinking.as_ref(), reasoning_event_text(&event))
            }
            StreamEventType::ToolCallStart => self.start_tool_call(event.tool_call.clone()),
            StreamEventType::ToolCallDelta => self.merge_tool_call(event.tool_call.clone(), false),
            StreamEventType::ToolCallEnd => self.end_tool_call(event.tool_call.clone()),
            StreamEventType::Finish | StreamEventType::Error => self.finish(event.clone()),
            StreamEventType::ProviderEvent | StreamEventType::Custom(_) => {}
        }

        self.events.push(event);
        self.rebuild_response();
    }

    pub fn append(&mut self, event: StreamEvent) {
        self.push(event);
    }

    pub fn extend(&mut self, events: impl IntoIterator<Item = StreamEvent>) {
        for event in events {
            self.push(event);
        }
    }

    pub fn finalize(&mut self) -> Response {
        self.flush_active_blocks();
        self.rebuild_response();
        self.response.clone()
    }

    fn finish(&mut self, event: StreamEvent) {
        let event_type = event.r#type.clone();
        let response_finish_reason = event
            .response
            .as_ref()
            .map(|response| response.finish_reason.clone());
        let response_usage = event
            .response
            .as_ref()
            .map(|response| response.usage.clone());
        self.flush_active_blocks();
        self.overlay_response(event.response.clone());
        self.finish_reason = event.finish_reason.clone().or_else(|| {
            if event_type == StreamEventType::Error {
                Some(FinishReason::Error)
            } else {
                response_finish_reason
            }
        });
        self.merge_usage(response_usage);
        self.merge_usage(event.usage.clone());
        self.finish_event = Some(event);
    }

    fn overlay_response(&mut self, response: Option<Response>) {
        let Some(response) = response else {
            return;
        };
        if !response.id.is_empty() {
            self.response.id = response.id;
        }
        if !response.model.is_empty() {
            self.response.model = response.model;
        }
        if !response.provider.is_empty() {
            self.response.provider = response.provider;
        }
        if !response.message.content.is_empty() {
            self.response.message = response.message;
        }
        self.response.finish_reason = response.finish_reason;
        self.merge_usage(Some(response.usage));
        if response.raw.is_some() {
            self.response.raw = response.raw;
        }
        if !response.warnings.is_empty() {
            self.response.warnings = response.warnings;
        }
        if response.rate_limit.is_some() {
            self.response.rate_limit = response.rate_limit;
        }
        if !response.text.is_empty() {
            self.response.text = response.text;
        }
        if !response.tool_calls.is_empty() {
            self.response.tool_calls = response.tool_calls;
        }
        if !response.raw_provider_events.is_empty() {
            self.response.raw_provider_events = response.raw_provider_events;
        }
    }

    fn merge_usage(&mut self, usage: Option<Usage>) {
        let Some(usage) = usage else {
            return;
        };
        self.usage = merge_stream_usage(self.usage.take(), usage);
        if let Some(usage) = self.usage.clone() {
            self.response.usage = usage;
        }
    }

    fn start_text(&mut self, text_id: Option<&str>, text: Option<&str>) {
        let key = stable_text_key(text_id);
        self.active_text.insert(key.clone(), String::new());
        self.append_text_to_key(key, text);
    }

    fn append_text(&mut self, text_id: Option<&str>, text: Option<&str>) {
        let Some(text) = text else {
            return;
        };
        let key = self.active_text_key(text_id);
        self.append_text_to_key(key, Some(text));
    }

    fn end_text(&mut self, text_id: Option<&str>, text: Option<&str>) {
        let key = self.active_text_key(text_id);
        self.active_text.entry(key.clone()).or_default();
        if let Some(text) = text {
            let active = self.active_text.get(&key).cloned().unwrap_or_default();
            if text == active {
                self.active_text.insert(key.clone(), text.to_string());
            } else if let Some(suffix) = text.strip_prefix(&active) {
                self.final_text.push_str(suffix);
                self.active_text.insert(key.clone(), text.to_string());
            } else {
                self.append_text_to_key(key.clone(), Some(text));
            }
        }
        self.active_text.remove(&key);
    }

    fn append_text_to_key(&mut self, key: String, text: Option<&str>) {
        let Some(text) = text else {
            return;
        };
        self.active_text.entry(key).or_default().push_str(text);
        self.final_text.push_str(text);
    }

    fn active_text_key(&self, text_id: Option<&str>) -> String {
        if let Some(text_id) = non_empty(text_id) {
            return text_id.to_string();
        }
        if self.active_text.len() == 1 {
            if let Some(key) = self.active_text.keys().next() {
                return key.clone();
            }
        }
        DEFAULT_TEXT_ID.to_string()
    }

    fn start_reasoning(&mut self, thinking: Option<&ThinkingData>, text: Option<&str>) {
        self.flush_active_blocks();
        self.active_reasoning = Some(String::new());
        self.active_reasoning_metadata = Some(reasoning_metadata(thinking));
        self.append_reasoning(thinking, text);
    }

    fn append_reasoning(&mut self, thinking: Option<&ThinkingData>, text: Option<&str>) {
        self.update_active_reasoning_metadata(thinking);
        let Some(text) = text else {
            return;
        };
        if self.active_reasoning.is_none() {
            self.flush_active_blocks();
            self.active_reasoning = Some(String::new());
            self.active_reasoning_metadata = Some(reasoning_metadata(thinking));
        }
        if let Some(active_reasoning) = self.active_reasoning.as_mut() {
            active_reasoning.push_str(text);
        }
        self.reasoning_text.push_str(text);
    }

    fn end_reasoning(&mut self, thinking: Option<&ThinkingData>, text: Option<&str>) {
        self.update_active_reasoning_metadata(thinking);
        if self.active_reasoning.is_none() {
            self.active_reasoning = Some(String::new());
            self.active_reasoning_metadata = Some(reasoning_metadata(thinking));
        }
        if let Some(text) = text {
            let active = self.active_reasoning.clone().unwrap_or_default();
            if text == active {
                self.active_reasoning = Some(text.to_string());
            } else if let Some(suffix) = text.strip_prefix(&active) {
                self.reasoning_text.push_str(suffix);
                self.active_reasoning = Some(text.to_string());
            } else {
                self.append_reasoning(thinking, Some(text));
            }
        }
        self.finalize_active_reasoning();
    }

    fn update_active_reasoning_metadata(&mut self, thinking: Option<&ThinkingData>) {
        let Some(incoming) = thinking else {
            return;
        };
        let metadata = self
            .active_reasoning_metadata
            .get_or_insert_with(default_thinking_metadata);
        if incoming.signature.is_some() {
            metadata.signature = incoming.signature.clone();
        }
        if incoming.redacted {
            metadata.redacted = true;
        }
        if incoming.source_provider.is_some() {
            metadata.source_provider = incoming.source_provider.clone();
        }
        if incoming.source_model.is_some() {
            metadata.source_model = incoming.source_model.clone();
        }
    }

    fn finalize_active_reasoning(&mut self) {
        let Some(text) = self.active_reasoning.take() else {
            self.active_reasoning_metadata = None;
            return;
        };
        let mut thinking = self
            .active_reasoning_metadata
            .take()
            .unwrap_or_else(default_thinking_metadata);
        thinking.text = text;
        if !thinking.text.is_empty() {
            self.reasoning_parts.push(thinking);
        }
    }

    fn start_tool_call(&mut self, tool_call: Option<ToolCall>) {
        let Some(mut tool_call) = tool_call else {
            return;
        };
        let key = self.tool_call_key(&tool_call, false);
        if tool_call.id.is_empty() {
            tool_call.id = key.clone();
        }
        if !self.active_tool_calls.contains_key(&key) {
            self.active_tool_call_order.push(key.clone());
        }
        let merged = match self.active_tool_calls.remove(&key) {
            Some(current) => merge_tool_calls(current, tool_call, false),
            None => tool_call,
        };
        self.active_tool_calls.insert(key, merged);
    }

    fn merge_tool_call(&mut self, tool_call: Option<ToolCall>, final_fragment: bool) {
        let Some(mut incoming) = tool_call else {
            return;
        };
        let key = self.tool_call_key(&incoming, final_fragment);
        if incoming.id.is_empty() {
            incoming.id = key.clone();
        }
        if !self.active_tool_calls.contains_key(&key) {
            self.active_tool_call_order.push(key.clone());
        }
        let merged = match self.active_tool_calls.remove(&key) {
            Some(current) => merge_tool_calls(current, incoming, final_fragment),
            None => incoming,
        };
        self.active_tool_calls.insert(key, merged);
    }

    fn end_tool_call(&mut self, tool_call: Option<ToolCall>) {
        let key = tool_call
            .as_ref()
            .map(|incoming| self.tool_call_key(incoming, true))
            .or_else(|| self.only_active_tool_call_key());
        let Some(key) = key else {
            return;
        };
        let existing = self.active_tool_calls.remove(&key);
        let final_tool_call = match (existing, tool_call) {
            (Some(current), Some(incoming)) => merge_tool_calls(current, incoming, true),
            (Some(current), None) => current,
            (None, Some(mut incoming)) => {
                if incoming.id.is_empty() {
                    incoming.id = key.clone();
                }
                incoming
            }
            (None, None) => return,
        };
        self.remove_active_tool_call_order(&key);
        self.tool_calls.push(final_tool_call);
    }

    fn tool_call_key(&self, tool_call: &ToolCall, final_fragment: bool) -> String {
        if !tool_call.id.is_empty() && self.active_tool_calls.contains_key(&tool_call.id) {
            return tool_call.id.clone();
        }
        if final_fragment {
            if let Some(key) = self.only_active_tool_call_key() {
                return key;
            }
        }
        if !tool_call.id.is_empty() {
            return tool_call.id.clone();
        }
        self.only_active_tool_call_key().unwrap_or_else(|| {
            format!(
                "{DEFAULT_TOOL_CALL_ID}_{}",
                self.active_tool_call_order.len()
            )
        })
    }

    fn only_active_tool_call_key(&self) -> Option<String> {
        (self.active_tool_calls.len() == 1)
            .then(|| self.active_tool_calls.keys().next().cloned())
            .flatten()
    }

    fn remove_active_tool_call_order(&mut self, key: &str) {
        self.active_tool_call_order
            .retain(|active_key| active_key != key);
    }

    fn flush_active_blocks(&mut self) {
        self.active_text.clear();
        self.finalize_active_reasoning();
        let active_order = std::mem::take(&mut self.active_tool_call_order);
        for key in active_order {
            if let Some(tool_call) = self.active_tool_calls.remove(&key) {
                self.tool_calls.push(tool_call);
            }
        }
        for (_, tool_call) in std::mem::take(&mut self.active_tool_calls) {
            self.tool_calls.push(tool_call);
        }
    }

    fn rebuild_response(&mut self) {
        if let Some(reason) = self.finish_reason.clone() {
            self.response.finish_reason = reason;
        }
        if let Some(usage) = self.usage.clone() {
            self.response.usage = usage.normalized();
        }
        if !self.final_text.is_empty() {
            self.response.text = self.final_text.clone();
        }
        if !self.tool_calls.is_empty() {
            self.response.tool_calls = self.tool_calls.clone();
        }
        self.response.raw_provider_events = self.raw_provider_events.clone();
        if self.response.raw.is_none() {
            self.response.raw = raw_payload_from_events(&self.raw_provider_events);
        }

        let mut content = Vec::new();
        if !self.final_text.is_empty() {
            content.push(ContentPart::Text {
                text: self.final_text.clone(),
            });
        }
        for thinking in self.current_reasoning_parts() {
            if thinking.redacted {
                content.push(ContentPart::RedactedThinking { thinking });
            } else {
                content.push(ContentPart::Thinking { thinking });
            }
        }
        for tool_call in &self.tool_calls {
            content.push(ContentPart::ToolCall {
                tool_call: tool_call.clone(),
            });
        }
        if !content.is_empty() {
            self.response.message = Message {
                role: MessageRole::Assistant,
                content,
                ..self.response.message.clone()
            };
        }

        if let Some(finish_event) = self.finish_event.as_mut() {
            finish_event.response = Some(self.response.clone());
            finish_event.finish_reason = Some(self.response.finish_reason.clone());
            finish_event.usage = Some(self.response.usage.clone());
        }
        if let Some(finish_event) = self.finish_event.clone() {
            if let Some(stored_event) = self
                .events
                .iter_mut()
                .rev()
                .find(|event| event.r#type == finish_event.r#type)
            {
                *stored_event = finish_event;
            }
        }
    }

    fn current_reasoning_parts(&self) -> Vec<ThinkingData> {
        let mut parts = self.reasoning_parts.clone();
        if let Some(active) = self.active_reasoning.as_ref() {
            if !active.is_empty() {
                let mut thinking = self
                    .active_reasoning_metadata
                    .clone()
                    .unwrap_or_else(default_thinking_metadata);
                thinking.text = active.clone();
                parts.push(thinking);
            }
        }
        if parts.is_empty() && !self.reasoning_text.is_empty() {
            parts.push(ThinkingData {
                text: self.reasoning_text.clone(),
                ..default_thinking_metadata()
            });
        }
        parts
    }
}

fn raw_payload_from_events(events: &[Value]) -> Option<Value> {
    match events.len() {
        0 => None,
        1 => events.first().cloned(),
        _ => Some(Value::Array(events.to_vec())),
    }
}

fn stable_text_key(text_id: Option<&str>) -> String {
    non_empty(text_id).unwrap_or(DEFAULT_TEXT_ID).to_string()
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.and_then(|value| (!value.is_empty()).then_some(value))
}

fn reasoning_event_text(event: &StreamEvent) -> Option<&str> {
    event.reasoning_delta.as_deref().or_else(|| {
        event
            .thinking
            .as_ref()
            .and_then(|thinking| non_empty(Some(&thinking.text)))
    })
}

fn reasoning_metadata(thinking: Option<&ThinkingData>) -> ThinkingData {
    let mut metadata = thinking.cloned().unwrap_or_else(default_thinking_metadata);
    metadata.text.clear();
    metadata
}

fn default_thinking_metadata() -> ThinkingData {
    ThinkingData {
        text: String::new(),
        signature: None,
        redacted: false,
        source_provider: None,
        source_model: None,
    }
}

fn merge_tool_calls(current: ToolCall, incoming: ToolCall, final_fragment: bool) -> ToolCall {
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
    let (arguments, raw_arguments) = merge_tool_arguments(
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

fn merge_tool_arguments(
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

fn json_compact(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}
