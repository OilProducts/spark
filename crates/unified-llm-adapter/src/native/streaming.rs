use std::collections::{BTreeMap, VecDeque};

use serde_json::{json, Value};

use crate::errors::{AdapterError, AdapterErrorKind};
use crate::events::{merge_stream_usage, StreamAccumulator, StreamEvent, StreamEventType};
use crate::provider_utils::{ProviderStreamPayloadError, ProviderStreamRecord, SseParser};
use crate::request::{FinishReason, RateLimitInfo, Response, ThinkingData, ToolCall};
use crate::usage::Usage;

use super::anthropic::AnthropicStreamTranslator;
use super::common::{
    configuration_error, estimate_reasoning_tokens, merge_tool_calls_for_stream,
    normalize_rate_limit_headers, stream_raw_payload,
};
use super::gemini::GeminiStreamTranslator;
use super::openai::OpenAiStreamTranslator;
use super::types::NativeStreamBody;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ActiveStreamBlock {
    Text(String),
    Reasoning,
    ToolCall(String),
}

#[derive(Debug, Clone)]
pub(super) struct NativeStreamState {
    provider: &'static str,
    pub(super) rate_limit: Option<RateLimitInfo>,
    events: Vec<Result<StreamEvent, AdapterError>>,
    pub(super) accumulator: StreamAccumulator,
    pub(super) raw_payloads: Vec<Value>,
    pub(super) started: bool,
    pub(super) active_texts: BTreeMap<String, String>,
    active_reasoning: bool,
    pub(super) active_tool_calls: BTreeMap<String, ToolCall>,
    active_tool_call_order: Vec<String>,
    next_text_id: usize,
    next_tool_call_id: usize,
    pub(super) last_response: Option<Response>,
    pub(super) finish_reason: Option<FinishReason>,
    usage: Option<Usage>,
}

impl NativeStreamState {
    pub(super) fn new(provider: &'static str, headers: &BTreeMap<String, String>) -> Self {
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

    pub(super) fn push(&mut self, event: StreamEvent) {
        self.accumulator.push(event.clone());
        self.events.push(Ok(event));
    }

    pub(super) fn push_error(&mut self, error: AdapterError, raw: Option<Value>) {
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

    pub(super) fn push_iterator_error(&mut self, error: AdapterError) {
        self.events.push(Err(error));
    }

    pub(super) fn take_events(&mut self) -> Vec<Result<StreamEvent, AdapterError>> {
        std::mem::take(&mut self.events)
    }

    pub(super) fn record_usage(&mut self, usage: Usage) {
        self.usage = merge_stream_usage(self.usage.take(), usage);
    }

    pub(super) fn ensure_started(&mut self, raw: Option<Value>, response: Option<Response>) {
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

    pub(super) fn text_start(&mut self, text_id: Option<String>, raw: Option<Value>) {
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

    pub(super) fn text_delta(
        &mut self,
        text_id: Option<String>,
        delta: String,
        raw: Option<Value>,
    ) {
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

    pub(super) fn text_end(
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

    pub(super) fn reasoning_delta(&mut self, delta: String, raw: Option<Value>) {
        self.reasoning_delta_with_metadata(delta, None, raw);
    }

    pub(super) fn reasoning_delta_with_metadata(
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

    pub(super) fn reasoning_start_with_metadata(
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

    pub(super) fn tool_call_start(&mut self, tool_call: ToolCall, raw: Option<Value>) {
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

    pub(super) fn tool_call_delta(&mut self, tool_call: ToolCall, raw: Option<Value>) {
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

    pub(super) fn tool_call_end(&mut self, tool_call: Option<ToolCall>, raw: Option<Value>) {
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

    pub(super) fn provider_event(&mut self, raw: Value) {
        self.ensure_started(Some(raw.clone()), None);
        self.push(StreamEvent::provider_event(raw));
    }

    pub(super) fn close_reasoning(&mut self, raw: Option<Value>) {
        if self.active_reasoning {
            self.active_reasoning = false;
            self.push(StreamEvent {
                r#type: StreamEventType::ReasoningEnd,
                raw,
                ..StreamEvent::new(StreamEventType::ReasoningEnd)
            });
        }
    }

    pub(super) fn close_all_text(&mut self, raw: Option<Value>) {
        let text_ids = self.active_texts.keys().cloned().collect::<Vec<_>>();
        for text_id in text_ids {
            self.text_end(Some(text_id), None, raw.clone());
        }
    }

    pub(super) fn close_all_tool_calls(&mut self, raw: Option<Value>) {
        let tool_call_ids = std::mem::take(&mut self.active_tool_call_order);
        for tool_call_id in tool_call_ids {
            self.tool_call_end_by_id(tool_call_id, raw.clone());
        }
        let remaining = self.active_tool_calls.keys().cloned().collect::<Vec<_>>();
        for tool_call_id in remaining {
            self.tool_call_end_by_id(tool_call_id, raw.clone());
        }
    }

    pub(super) fn tool_call_end_by_id(&mut self, tool_call_id: String, raw: Option<Value>) {
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

    pub(super) fn finish(&mut self, raw: Option<Value>) -> Vec<Result<StreamEvent, AdapterError>> {
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
        self.take_events()
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

struct NativeRecordStream {
    body: Option<NativeStreamBody>,
    parser: SseParser,
    json_buffer: String,
    json_mode: bool,
    pending: VecDeque<Result<ProviderStreamRecord, AdapterError>>,
    finished: bool,
}

impl NativeRecordStream {
    fn new(body: NativeStreamBody) -> Self {
        Self {
            body: Some(body),
            parser: SseParser::default(),
            json_buffer: String::new(),
            json_mode: false,
            pending: VecDeque::new(),
            finished: false,
        }
    }

    fn close(&mut self) {
        self.finished = true;
        self.pending.clear();
        self.body = None;
    }

    fn process_item(&mut self, item: Result<Value, AdapterError>) {
        match item {
            Ok(Value::String(chunk)) => {
                if self.json_mode
                    || (!self.parser.has_pending_input() && looks_like_json_stream(&chunk))
                {
                    self.pending
                        .extend(self.parser.finish().into_iter().map(Ok));
                    self.json_mode = true;
                    self.json_buffer.push_str(&chunk);
                    match parse_json_stream_records(&self.json_buffer) {
                        JsonStreamParse::Complete(parsed) => {
                            self.pending.extend(parsed.into_iter().map(Ok));
                            self.json_buffer.clear();
                            self.json_mode = false;
                        }
                        JsonStreamParse::Incomplete => {}
                        JsonStreamParse::Malformed(error) => {
                            self.pending.push_back(Ok(malformed_json_stream_record(
                                std::mem::take(&mut self.json_buffer),
                                error.message,
                            )));
                            self.json_mode = false;
                        }
                    }
                } else {
                    self.pending
                        .extend(self.parser.push_str(&chunk).into_iter().map(Ok));
                }
            }
            Ok(payload) => {
                let mut records = Vec::new();
                flush_json_stream_buffer(&mut self.json_buffer, &mut self.json_mode, &mut records);
                self.pending.extend(records);
                self.pending
                    .extend(self.parser.finish().into_iter().map(Ok));
                self.pending
                    .push_back(Ok(ProviderStreamRecord::from_json(payload)));
            }
            Err(error) => {
                let mut records = Vec::new();
                flush_json_stream_buffer(&mut self.json_buffer, &mut self.json_mode, &mut records);
                self.pending.extend(records);
                self.pending
                    .extend(self.parser.finish().into_iter().map(Ok));
                self.pending.push_back(Err(error));
            }
        }
    }

    fn finish_input(&mut self) {
        let mut records = Vec::new();
        flush_json_stream_buffer(&mut self.json_buffer, &mut self.json_mode, &mut records);
        self.pending.extend(records);
        self.pending
            .extend(self.parser.finish().into_iter().map(Ok));
        self.finished = true;
        self.body = None;
    }
}

impl Iterator for NativeRecordStream {
    type Item = Result<ProviderStreamRecord, AdapterError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(item) = self.pending.pop_front() {
                return Some(item);
            }
            if self.finished {
                return None;
            }
            let Some(body) = self.body.as_mut() else {
                self.finish_input();
                continue;
            };
            match body.next() {
                Some(item) => self.process_item(item),
                None => self.finish_input(),
            }
        }
    }
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

pub(super) fn stream_record_payload(
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

enum NativeProviderStreamTranslator {
    OpenAi(OpenAiStreamTranslator),
    Anthropic(AnthropicStreamTranslator),
    Gemini(GeminiStreamTranslator),
}

impl NativeProviderStreamTranslator {
    pub(super) fn new(
        provider: &str,
        headers: &BTreeMap<String, String>,
    ) -> Result<Self, AdapterError> {
        match provider {
            "openai" => Ok(Self::OpenAi(OpenAiStreamTranslator::new(headers))),
            "anthropic" => Ok(Self::Anthropic(AnthropicStreamTranslator::new(headers))),
            "gemini" => Ok(Self::Gemini(GeminiStreamTranslator::new(headers))),
            other => Err(configuration_error(
                other,
                format!("Unsupported native provider {other:?}"),
            )),
        }
    }

    fn apply(
        &mut self,
        item: Result<ProviderStreamRecord, AdapterError>,
    ) -> Vec<Result<StreamEvent, AdapterError>> {
        match self {
            Self::OpenAi(translator) => translator.apply(item),
            Self::Anthropic(translator) => translator.apply(item),
            Self::Gemini(translator) => translator.apply(item),
        }
    }

    pub(super) fn finish(&mut self) -> Vec<Result<StreamEvent, AdapterError>> {
        match self {
            Self::OpenAi(translator) => translator.finish_eof(),
            Self::Anthropic(translator) => translator.finish_eof(),
            Self::Gemini(translator) => translator.finish_eof(),
        }
    }

    fn is_finished(&self) -> bool {
        match self {
            Self::OpenAi(translator) => translator.finished,
            Self::Anthropic(translator) => translator.finished,
            Self::Gemini(translator) => translator.finished,
        }
    }
}

struct NativeTranslatedStream {
    records: NativeRecordStream,
    translator: Result<NativeProviderStreamTranslator, AdapterError>,
    pending: VecDeque<Result<StreamEvent, AdapterError>>,
    closed: bool,
}

impl NativeTranslatedStream {
    fn new(
        provider: String,
        headers: BTreeMap<String, String>,
        records: NativeRecordStream,
    ) -> Self {
        Self {
            records,
            translator: NativeProviderStreamTranslator::new(&provider, &headers),
            pending: VecDeque::new(),
            closed: false,
        }
    }
}

impl Iterator for NativeTranslatedStream {
    type Item = Result<StreamEvent, AdapterError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Some(event);
            }
            if self.closed {
                return None;
            }
            let translator = match self.translator.as_mut() {
                Ok(translator) => translator,
                Err(error) => {
                    self.closed = true;
                    return Some(Err(error.clone()));
                }
            };
            if translator.is_finished() {
                self.closed = true;
                self.records.close();
                return None;
            }
            match self.records.next() {
                Some(record) => self.pending.extend(translator.apply(record)),
                None => {
                    self.pending.extend(translator.finish());
                    self.closed = true;
                    self.records.close();
                }
            }
        }
    }
}

impl crate::events::StreamEventStream for NativeTranslatedStream {
    fn close(&mut self) -> Result<(), AdapterError> {
        self.closed = true;
        self.pending.clear();
        self.records.close();
        Ok(())
    }
}

pub(super) fn native_translated_stream(
    provider: String,
    headers: BTreeMap<String, String>,
    body: NativeStreamBody,
) -> crate::events::StreamEvents {
    Box::new(NativeTranslatedStream::new(
        provider,
        headers,
        NativeRecordStream::new(body),
    ))
}
