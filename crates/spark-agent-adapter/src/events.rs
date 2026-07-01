use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use spark_common::events::{
    TurnStreamChannel, TurnStreamEvent, TurnStreamEventKind, TurnStreamSource,
};
use time::OffsetDateTime;
use unified_llm_adapter::{StreamEvent, StreamEventType, Usage};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EventKind {
    SessionStart,
    SessionEnd,
    UserInput,
    ProcessingEnd,
    AssistantTextStart,
    AssistantTextDelta,
    AssistantTextEnd,
    AssistantReasoningStart,
    AssistantReasoningDelta,
    AssistantReasoningEnd,
    ModelToolCallStart,
    ModelToolCallDelta,
    ModelToolCallEnd,
    ModelUsageUpdate,
    ToolCallStart,
    ToolCallOutputDelta,
    ToolCallEnd,
    SteeringInjected,
    TurnLimit,
    LoopDetection,
    Warning,
    Error,
    Other(String),
}

impl EventKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::SessionStart => "session_start",
            Self::SessionEnd => "session_end",
            Self::UserInput => "user_input",
            Self::ProcessingEnd => "processing_end",
            Self::AssistantTextStart => "assistant_text_start",
            Self::AssistantTextDelta => "assistant_text_delta",
            Self::AssistantTextEnd => "assistant_text_end",
            Self::AssistantReasoningStart => "assistant_reasoning_start",
            Self::AssistantReasoningDelta => "assistant_reasoning_delta",
            Self::AssistantReasoningEnd => "assistant_reasoning_end",
            Self::ModelToolCallStart => "model_tool_call_start",
            Self::ModelToolCallDelta => "model_tool_call_delta",
            Self::ModelToolCallEnd => "model_tool_call_end",
            Self::ModelUsageUpdate => "model_usage_update",
            Self::ToolCallStart => "tool_call_start",
            Self::ToolCallOutputDelta => "tool_call_output_delta",
            Self::ToolCallEnd => "tool_call_end",
            Self::SteeringInjected => "steering_injected",
            Self::TurnLimit => "turn_limit",
            Self::LoopDetection => "loop_detection",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Other(value) => value.as_str(),
        }
    }

    pub fn to_turn_stream_kind(
        &self,
        data: &BTreeMap<String, Value>,
    ) -> Option<TurnStreamEventKind> {
        Some(match self {
            Self::AssistantTextStart
            | Self::AssistantReasoningStart
            | Self::ModelToolCallStart
            | Self::ModelToolCallDelta
            | Self::ModelToolCallEnd => TurnStreamEventKind::Other(self.as_str().to_string()),
            Self::AssistantTextDelta | Self::AssistantReasoningDelta => {
                TurnStreamEventKind::ContentDelta
            }
            Self::AssistantTextEnd | Self::AssistantReasoningEnd => {
                TurnStreamEventKind::ContentCompleted
            }
            Self::ToolCallStart => TurnStreamEventKind::ToolCallStarted,
            Self::ToolCallOutputDelta => TurnStreamEventKind::ToolCallUpdated,
            Self::ToolCallEnd if data.get("error").is_some_and(|value| !value.is_null()) => {
                TurnStreamEventKind::ToolCallFailed
            }
            Self::ToolCallEnd => TurnStreamEventKind::ToolCallCompleted,
            Self::ModelUsageUpdate => TurnStreamEventKind::TokenUsageUpdated,
            Self::Other(value) if value == "request_user_input_requested" => {
                TurnStreamEventKind::RequestUserInputRequested
            }
            Self::SessionStart | Self::SessionEnd | Self::Warning => {
                TurnStreamEventKind::Other(self.as_str().to_string())
            }
            Self::ProcessingEnd => TurnStreamEventKind::TurnCompleted,
            Self::Error => TurnStreamEventKind::Error,
            _ => return None,
        })
    }
}

impl fmt::Display for EventKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for EventKind {
    type Err = Infallible;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "session_start" | "SESSION_START" => Self::SessionStart,
            "session_end" | "SESSION_END" => Self::SessionEnd,
            "user_input" | "USER_INPUT" => Self::UserInput,
            "processing_end" | "PROCESSING_END" => Self::ProcessingEnd,
            "assistant_text_start" | "ASSISTANT_TEXT_START" => Self::AssistantTextStart,
            "assistant_text_delta" | "ASSISTANT_TEXT_DELTA" => Self::AssistantTextDelta,
            "assistant_text_end" | "ASSISTANT_TEXT_END" => Self::AssistantTextEnd,
            "assistant_reasoning_start" | "ASSISTANT_REASONING_START" => {
                Self::AssistantReasoningStart
            }
            "assistant_reasoning_delta" | "ASSISTANT_REASONING_DELTA" => {
                Self::AssistantReasoningDelta
            }
            "assistant_reasoning_end" | "ASSISTANT_REASONING_END" => Self::AssistantReasoningEnd,
            "model_tool_call_start" | "MODEL_TOOL_CALL_START" => Self::ModelToolCallStart,
            "model_tool_call_delta" | "MODEL_TOOL_CALL_DELTA" => Self::ModelToolCallDelta,
            "model_tool_call_end" | "MODEL_TOOL_CALL_END" => Self::ModelToolCallEnd,
            "model_usage_update" | "MODEL_USAGE_UPDATE" => Self::ModelUsageUpdate,
            "tool_call_start" | "TOOL_CALL_START" => Self::ToolCallStart,
            "tool_call_output_delta" | "TOOL_CALL_OUTPUT_DELTA" => Self::ToolCallOutputDelta,
            "tool_call_end" | "TOOL_CALL_END" => Self::ToolCallEnd,
            "steering_injected" | "STEERING_INJECTED" => Self::SteeringInjected,
            "turn_limit" | "TURN_LIMIT" => Self::TurnLimit,
            "loop_detection" | "LOOP_DETECTION" => Self::LoopDetection,
            "warning" | "WARNING" => Self::Warning,
            "error" | "ERROR" => Self::Error,
            other => Self::Other(other.to_string()),
        })
    }
}

impl From<&str> for EventKind {
    fn from(value: &str) -> Self {
        Self::from_str(value).expect("event kind parsing is infallible")
    }
}

impl From<String> for EventKind {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}

impl Serialize for EventKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for EventKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(Self::from(value))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionEvent {
    pub kind: EventKind,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Uuid>,
    #[serde(default)]
    pub data: BTreeMap<String, Value>,
}

impl SessionEvent {
    pub fn new(
        kind: impl Into<EventKind>,
        session_id: impl Into<Option<Uuid>>,
        data: BTreeMap<String, Value>,
    ) -> Self {
        Self {
            kind: kind.into(),
            timestamp: OffsetDateTime::now_utc(),
            session_id: session_id.into(),
            data,
        }
    }

    pub fn without_session(kind: impl Into<EventKind>, data: BTreeMap<String, Value>) -> Self {
        Self::new(kind, None, data)
    }

    pub fn from_stream_event(
        event: &StreamEvent,
        session_id: impl Into<Option<Uuid>>,
        response_id: Option<&str>,
    ) -> Vec<Self> {
        Self::from_stream_event_with_accumulated(event, session_id, response_id, None, None)
    }

    pub(crate) fn from_stream_event_with_accumulated(
        event: &StreamEvent,
        session_id: impl Into<Option<Uuid>>,
        response_id: Option<&str>,
        accumulated_text: Option<&str>,
        accumulated_reasoning: Option<&str>,
    ) -> Vec<Self> {
        let session_id = session_id.into();
        stream_event_payloads(event, response_id, accumulated_text, accumulated_reasoning)
            .into_iter()
            .map(|(kind, data)| SessionEvent::new(kind, session_id, data))
            .collect()
    }

    pub fn to_turn_stream_event(&self) -> Option<TurnStreamEvent> {
        let kind = self.kind.to_turn_stream_kind(&self.data)?;
        let channel = match self.kind {
            EventKind::AssistantTextStart
            | EventKind::AssistantTextDelta
            | EventKind::AssistantTextEnd => Some(TurnStreamChannel::Assistant),
            EventKind::AssistantReasoningStart
            | EventKind::AssistantReasoningDelta
            | EventKind::AssistantReasoningEnd => Some(TurnStreamChannel::Reasoning),
            _ => None,
        };
        let content_delta = match self.kind {
            EventKind::AssistantTextDelta | EventKind::AssistantReasoningDelta => {
                data_string(&self.data, &["delta"])
            }
            EventKind::AssistantTextEnd | EventKind::AssistantReasoningEnd => {
                data_string(&self.data, &["text", "delta"])
            }
            EventKind::ToolCallOutputDelta => data_string(&self.data, &["delta", "output_delta"]),
            EventKind::ModelToolCallDelta => data_string(&self.data, &["delta"]),
            _ => None,
        };
        let message = content_delta.clone().or_else(|| match self.kind {
            EventKind::Error | EventKind::Warning => session_error_message(&self.data),
            _ => data_string(&self.data, &["message"]),
        });

        Some(TurnStreamEvent {
            kind,
            channel,
            source: self.turn_stream_source(),
            content_delta,
            message,
            tool_call: self.turn_stream_tool_call_payload(),
            request_user_input: self.turn_stream_request_user_input_payload(),
            token_usage: self.turn_stream_token_usage_payload(),
            error: if self.kind == EventKind::Error {
                session_error_message(&self.data)
            } else {
                None
            },
            error_code: if self.kind == EventKind::Error {
                session_error_code(&self.data)
            } else {
                None
            },
            details: match self.kind {
                EventKind::Error => session_error_details(&self.data),
                EventKind::SessionStart
                | EventKind::SessionEnd
                | EventKind::ProcessingEnd
                | EventKind::Warning => Some(Value::Object(
                    self.data.clone().into_iter().collect::<Map<_, _>>(),
                )),
                _ => None,
            },
            phase: data_string(&self.data, &["phase"]),
            status: data_string(&self.data, &["status"]),
        })
    }

    fn turn_stream_source(&self) -> TurnStreamSource {
        TurnStreamSource {
            backend: Some("agent_session".to_string()),
            session_id: self.session_id.map(|id| id.to_string()),
            app_turn_id: data_string(&self.data, &["app_turn_id"]),
            item_id: data_string(&self.data, &["item_id"]).or_else(|| match self.kind {
                EventKind::ModelToolCallStart
                | EventKind::ModelToolCallDelta
                | EventKind::ModelToolCallEnd => model_tool_call_id(&self.data),
                _ => None,
            }),
            response_id: data_string(&self.data, &["response_id"]),
            summary_index: self.data.get("summary_index").and_then(Value::as_u64),
            raw_kind: Some(self.kind.as_str().to_string()),
        }
    }

    fn turn_stream_tool_call_payload(&self) -> Option<Value> {
        match self.kind {
            EventKind::ModelToolCallStart
            | EventKind::ModelToolCallDelta
            | EventKind::ModelToolCallEnd => {
                Some(model_proposed_tool_call_payload(&self.kind, &self.data))
            }
            EventKind::ToolCallStart | EventKind::ToolCallOutputDelta | EventKind::ToolCallEnd => {
                Some(actual_tool_call_payload(&self.kind, &self.data))
            }
            _ => None,
        }
    }

    fn turn_stream_token_usage_payload(&self) -> Option<Value> {
        if self.kind != EventKind::ModelUsageUpdate {
            return None;
        }
        workspace_token_usage_payload(&self.data)
    }

    fn turn_stream_request_user_input_payload(&self) -> Option<Value> {
        if self.kind.as_str() != "request_user_input_requested" {
            return None;
        }
        self.data
            .get("request_user_input")
            .cloned()
            .or_else(|| Some(Value::Object(self.data.clone().into_iter().collect())))
    }
}

fn data_string(data: &BTreeMap<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let value = data.get(*key)?;
        match value {
            Value::Null => None,
            Value::String(text) => Some(text.clone()),
            other => Some(other.to_string()),
        }
    })
}

fn session_error_message(data: &BTreeMap<String, Value>) -> Option<String> {
    data_string(data, &["message"])
        .or_else(|| object_string(data.get("error").and_then(Value::as_object), &["message"]))
        .or_else(|| data_string(data, &["error"]))
}

fn session_error_code(data: &BTreeMap<String, Value>) -> Option<String> {
    data_string(data, &["error_code", "code", "error_kind", "kind", "name"]).or_else(|| {
        object_string(
            data.get("error").and_then(Value::as_object),
            &["error_code", "code", "kind", "name"],
        )
    })
}

fn session_error_details(data: &BTreeMap<String, Value>) -> Option<Value> {
    data.get("details").cloned().or_else(|| {
        let mut details = Map::new();
        for key in [
            "provider",
            "status_code",
            "retryable",
            "retry_after",
            "error_kind",
            "raw",
            "event_raw",
        ] {
            if let Some(value) = data.get(key) {
                details.insert(key.to_string(), value.clone());
            }
        }
        (!details.is_empty()).then(|| Value::Object(details))
    })
}

fn actual_tool_call_payload(kind: &EventKind, data: &BTreeMap<String, Value>) -> Value {
    let nested = data.get("tool_call").and_then(Value::as_object);
    let id = object_string(nested, &["id", "tool_call_id", "call_id"])
        .or_else(|| data_string(data, &["id", "tool_call_id", "call_id", "item_id"]))
        .or_else(|| object_string(nested, &["name", "tool_name"]))
        .or_else(|| data_string(data, &["tool_name", "name", "command"]))
        .unwrap_or_else(|| "tool".to_string());
    let name = object_string(nested, &["name", "tool_name"])
        .or_else(|| data_string(data, &["tool_name", "name"]));
    let command = object_string(nested, &["command"]).or_else(|| data_string(data, &["command"]));
    let output = object_string(nested, &["output", "result", "delta", "output_delta"])
        .or_else(|| data_string(data, &["output", "result", "delta", "output_delta"]));
    let error = object_string(nested, &["error"]).or_else(|| data_string(data, &["error"]));
    let status = data_string(data, &["status"])
        .or_else(|| object_string(nested, &["status"]))
        .unwrap_or_else(|| match kind {
            EventKind::ToolCallStart | EventKind::ToolCallOutputDelta => "running".to_string(),
            EventKind::ToolCallEnd if error.is_some() => "failed".to_string(),
            EventKind::ToolCallEnd => "completed".to_string(),
            _ => "running".to_string(),
        });
    let title = data_string(data, &["title"])
        .or_else(|| object_string(nested, &["title"]))
        .or_else(|| name.clone())
        .or_else(|| command.clone())
        .unwrap_or_else(|| "Tool call".to_string());
    let tool_kind = data_string(data, &["kind", "tool_kind"])
        .or_else(|| object_string(nested, &["kind", "tool_kind"]))
        .unwrap_or_else(|| "tool_call".to_string());

    let mut payload = Map::new();
    payload.insert("id".to_string(), Value::String(id));
    payload.insert("kind".to_string(), Value::String(tool_kind));
    payload.insert("status".to_string(), Value::String(status));
    payload.insert("title".to_string(), Value::String(title));
    payload.insert(
        "output".to_string(),
        output.map(Value::String).unwrap_or(Value::Null),
    );
    payload.insert(
        "error".to_string(),
        error.map(Value::String).unwrap_or(Value::Null),
    );
    if let Some(command) = command {
        payload.insert("command".to_string(), Value::String(command));
    }
    if let Some(paths) = data
        .get("file_paths")
        .or_else(|| nested.and_then(|object| object.get("file_paths")))
        .cloned()
    {
        payload.insert("file_paths".to_string(), paths);
    }
    payload.extend(
        data.iter()
            .filter(|(key, _)| {
                !matches!(
                    key.as_str(),
                    "tool_call"
                        | "id"
                        | "tool_call_id"
                        | "call_id"
                        | "item_id"
                        | "tool_name"
                        | "name"
                        | "kind"
                        | "tool_kind"
                        | "status"
                        | "title"
                        | "command"
                        | "output"
                        | "result"
                        | "delta"
                        | "output_delta"
                        | "error"
                        | "file_paths"
                )
            })
            .map(|(key, value)| (key.clone(), value.clone())),
    );

    Value::Object(payload)
}

fn model_proposed_tool_call_payload(kind: &EventKind, data: &BTreeMap<String, Value>) -> Value {
    let mut payload = data
        .get("tool_call")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let id = model_tool_call_id(data).unwrap_or_else(|| "model_tool_call".to_string());
    let name = payload
        .get("name")
        .and_then(Value::as_str)
        .and_then(non_empty_string)
        .or_else(|| data_string(data, &["name", "tool_name"]))
        .unwrap_or_else(|| "Model tool call".to_string());
    let status = match kind {
        EventKind::ModelToolCallStart => "proposed",
        EventKind::ModelToolCallDelta => "streaming",
        EventKind::ModelToolCallEnd => "completed",
        _ => "proposed",
    };

    payload.insert("id".to_string(), Value::String(id));
    payload.insert(
        "kind".to_string(),
        Value::String("model_tool_call".to_string()),
    );
    payload.insert("status".to_string(), Value::String(status.to_string()));
    payload
        .entry("name".to_string())
        .or_insert_with(|| Value::String(name.clone()));
    payload
        .entry("title".to_string())
        .or_insert_with(|| Value::String(name));
    if let Some(delta) = data.get("delta").cloned() {
        payload.insert("delta".to_string(), delta);
    }
    if let Some(response_id) = data.get("response_id").cloned() {
        payload.insert("response_id".to_string(), response_id);
    }
    payload.extend(
        data.iter()
            .filter(|(key, _)| !matches!(key.as_str(), "tool_call" | "delta" | "response_id"))
            .map(|(key, value)| (key.clone(), value.clone())),
    );

    Value::Object(payload)
}

fn model_tool_call_id(data: &BTreeMap<String, Value>) -> Option<String> {
    let nested = data.get("tool_call").and_then(Value::as_object);
    object_string(nested, &["id", "tool_call_id", "call_id"])
        .or_else(|| data_string(data, &["id", "tool_call_id", "call_id"]))
}

fn workspace_token_usage_payload(data: &BTreeMap<String, Value>) -> Option<Value> {
    if let Some(token_usage) = data.get("token_usage") {
        return Some(token_usage.clone());
    }
    if has_workspace_usage_shape(data.get("usage")) {
        return data.get("usage").cloned();
    }

    let usage = data.get("usage").unwrap_or(&Value::Null);
    let input_tokens = value_u64(usage, &["input_tokens", "inputTokens"])
        .or_else(|| data_u64(data, &["input_tokens", "inputTokens"]))
        .unwrap_or(0);
    let output_tokens = value_u64(usage, &["output_tokens", "outputTokens"])
        .or_else(|| data_u64(data, &["output_tokens", "outputTokens"]))
        .unwrap_or(0);
    let total_tokens = value_u64(usage, &["total_tokens", "totalTokens"])
        .or_else(|| data_u64(data, &["total_tokens", "totalTokens"]))
        .unwrap_or(input_tokens + output_tokens);
    let cached_input_tokens = value_u64(usage, &["cache_read_tokens", "cachedInputTokens"])
        .or_else(|| data_u64(data, &["cache_read_tokens", "cachedInputTokens"]))
        .unwrap_or(0)
        .min(input_tokens);
    let reasoning_output_tokens = value_u64(usage, &["reasoning_tokens", "reasoningOutputTokens"])
        .or_else(|| data_u64(data, &["reasoning_tokens", "reasoningOutputTokens"]));

    if input_tokens == 0
        && output_tokens == 0
        && total_tokens == 0
        && cached_input_tokens == 0
        && reasoning_output_tokens.is_none()
    {
        return None;
    }

    let mut total = Map::new();
    total.insert("inputTokens".to_string(), Value::from(input_tokens));
    total.insert(
        "cachedInputTokens".to_string(),
        Value::from(cached_input_tokens),
    );
    total.insert("outputTokens".to_string(), Value::from(output_tokens));
    if let Some(reasoning_output_tokens) = reasoning_output_tokens {
        total.insert(
            "reasoningOutputTokens".to_string(),
            Value::from(reasoning_output_tokens),
        );
    }
    total.insert("totalTokens".to_string(), Value::from(total_tokens));

    let mut payload = Map::new();
    payload.insert("total".to_string(), Value::Object(total));
    Some(Value::Object(payload))
}

pub(crate) fn workspace_token_usage_payload_from_usage(usage: &Usage) -> Option<Value> {
    let usage = serde_json::to_value(usage.clone().normalized()).ok()?;
    workspace_token_usage_payload(&BTreeMap::from([("usage".to_string(), usage)]))
}

fn has_workspace_usage_shape(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_object)
        .is_some_and(|object| object.contains_key("total") || object.contains_key("last"))
}

fn object_string(object: Option<&Map<String, Value>>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let value = object?.get(*key)?;
        match value {
            Value::Null => None,
            Value::String(text) => Some(text.clone()),
            other => Some(other.to_string()),
        }
    })
}

fn non_empty_string(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn data_u64(data: &BTreeMap<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| value_to_u64(data.get(*key)?))
}

fn value_u64(value: &Value, keys: &[&str]) -> Option<u64> {
    let object = value.as_object()?;
    keys.iter().find_map(|key| value_to_u64(object.get(*key)?))
}

fn value_to_u64(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_i64()
            .and_then(|value| u64::try_from(value).ok())
            .or_else(|| value.as_str()?.parse::<u64>().ok())
    })
}

fn stream_event_payloads(
    event: &StreamEvent,
    response_id: Option<&str>,
    accumulated_text: Option<&str>,
    accumulated_reasoning: Option<&str>,
) -> Vec<(EventKind, BTreeMap<String, Value>)> {
    match &event.r#type {
        StreamEventType::StreamStart
        | StreamEventType::ProviderEvent
        | StreamEventType::Custom(_) => Vec::new(),
        StreamEventType::TextStart => {
            let mut events = vec![(
                EventKind::AssistantTextStart,
                response_event_payload(response_id),
            )];
            if let Some(delta) = event.delta.as_deref() {
                events.push((
                    EventKind::AssistantTextDelta,
                    delta_payload(response_id, delta),
                ));
            }
            events
        }
        StreamEventType::TextDelta => event
            .delta
            .as_deref()
            .map(|delta| {
                vec![(
                    EventKind::AssistantTextDelta,
                    delta_payload(response_id, delta),
                )]
            })
            .unwrap_or_default(),
        StreamEventType::TextEnd => vec![(
            EventKind::AssistantTextEnd,
            text_end_payload(
                response_id,
                accumulated_text
                    .or(event.delta.as_deref())
                    .unwrap_or_default(),
                accumulated_reasoning,
            ),
        )],
        StreamEventType::ReasoningStart => {
            let mut events = vec![(
                EventKind::AssistantReasoningStart,
                response_event_payload(response_id),
            )];
            if let Some(delta) = reasoning_event_text(event) {
                events.push((
                    EventKind::AssistantReasoningDelta,
                    delta_payload(response_id, delta),
                ));
            }
            events
        }
        StreamEventType::ReasoningDelta => reasoning_event_text(event)
            .map(|delta| {
                vec![(
                    EventKind::AssistantReasoningDelta,
                    delta_payload(response_id, delta),
                )]
            })
            .unwrap_or_default(),
        StreamEventType::ReasoningEnd => vec![(
            EventKind::AssistantReasoningEnd,
            reasoning_end_payload(
                response_id,
                accumulated_reasoning
                    .or_else(|| reasoning_event_text(event))
                    .unwrap_or_default(),
            ),
        )],
        StreamEventType::ToolCallStart => vec![(
            EventKind::ModelToolCallStart,
            model_tool_call_payload(event, response_id),
        )],
        StreamEventType::ToolCallDelta => vec![(
            EventKind::ModelToolCallDelta,
            model_tool_call_payload(event, response_id),
        )],
        StreamEventType::ToolCallEnd => vec![(
            EventKind::ModelToolCallEnd,
            model_tool_call_payload(event, response_id),
        )],
        StreamEventType::Finish => stream_usage_payload(event)
            .map(|payload| vec![(EventKind::ModelUsageUpdate, payload)])
            .unwrap_or_default(),
        StreamEventType::Error => {
            let mut events = vec![(EventKind::Error, stream_error_payload(event, response_id))];
            if let Some(payload) = stream_usage_payload(event) {
                events.push((EventKind::ModelUsageUpdate, payload));
            }
            events
        }
    }
}

fn response_event_payload(response_id: Option<&str>) -> BTreeMap<String, Value> {
    response_id
        .filter(|value| !value.is_empty())
        .map(|value| {
            BTreeMap::from([("response_id".to_string(), Value::String(value.to_string()))])
        })
        .unwrap_or_default()
}

fn delta_payload(response_id: Option<&str>, delta: &str) -> BTreeMap<String, Value> {
    let mut payload = response_event_payload(response_id);
    payload.insert("delta".to_string(), Value::String(delta.to_string()));
    payload
}

fn text_end_payload(
    response_id: Option<&str>,
    text: &str,
    reasoning: Option<&str>,
) -> BTreeMap<String, Value> {
    let mut payload = response_event_payload(response_id);
    payload.insert("text".to_string(), Value::String(text.to_string()));
    payload.insert(
        "reasoning".to_string(),
        reasoning
            .map(|value| Value::String(value.to_string()))
            .unwrap_or(Value::Null),
    );
    payload
}

fn reasoning_end_payload(response_id: Option<&str>, text: &str) -> BTreeMap<String, Value> {
    let mut payload = response_event_payload(response_id);
    payload.insert("text".to_string(), Value::String(text.to_string()));
    payload
}

fn model_tool_call_payload(
    event: &StreamEvent,
    response_id: Option<&str>,
) -> BTreeMap<String, Value> {
    let mut payload = response_event_payload(response_id);
    if let Some(tool_call) = event.tool_call.as_ref() {
        payload.insert(
            "tool_call".to_string(),
            serde_json::to_value(tool_call).expect("tool call is serializable"),
        );
    }
    if let Some(delta) = event.delta.as_deref() {
        payload.insert("delta".to_string(), Value::String(delta.to_string()));
    }
    payload
}

fn stream_usage_payload(event: &StreamEvent) -> Option<BTreeMap<String, Value>> {
    let usage = event
        .usage
        .clone()
        .filter(usage_has_observations)
        .or_else(|| {
            event
                .response
                .as_ref()
                .map(|response| response.usage.clone())
                .filter(usage_has_observations)
        })?
        .normalized();

    Some(BTreeMap::from([(
        "usage".to_string(),
        serde_json::to_value(usage).expect("usage is serializable"),
    )]))
}

fn stream_error_payload(event: &StreamEvent, response_id: Option<&str>) -> BTreeMap<String, Value> {
    let mut payload = response_event_payload(response_id);
    match event.error.as_ref() {
        Some(error) => {
            let code = error
                .error_code
                .clone()
                .unwrap_or_else(|| error.kind.spec_error_name().to_string());
            let mut details = Map::new();
            details.insert(
                "kind".to_string(),
                serde_json::to_value(error.kind).expect("error kind is serializable"),
            );
            details.insert("retryable".to_string(), Value::Bool(error.retryable));
            if let Some(provider) = error.provider.as_ref() {
                details.insert("provider".to_string(), Value::String(provider.clone()));
            }
            if let Some(status_code) = error.status_code {
                details.insert("status_code".to_string(), Value::from(status_code));
            }
            if let Some(error_code) = error.error_code.as_ref() {
                details.insert("error_code".to_string(), Value::String(error_code.clone()));
            }
            if let Some(retry_after) = error.retry_after {
                details.insert("retry_after".to_string(), Value::from(retry_after));
            }
            if let Some(raw) = error.raw.as_ref() {
                details.insert("raw".to_string(), raw.clone());
            }

            payload.insert("message".to_string(), Value::String(error.message.clone()));
            payload.insert("code".to_string(), Value::String(code));
            payload.insert("error".to_string(), Value::String(error.message.clone()));
            payload.insert(
                "error_kind".to_string(),
                serde_json::to_value(error.kind).expect("error kind is serializable"),
            );
            payload.insert("retryable".to_string(), Value::Bool(error.retryable));
            payload.insert("details".to_string(), Value::Object(details));
            if let Some(provider) = error.provider.as_ref() {
                payload.insert("provider".to_string(), Value::String(provider.clone()));
            }
            if let Some(status_code) = error.status_code {
                payload.insert("status_code".to_string(), Value::from(status_code));
            }
            if let Some(error_code) = error.error_code.as_ref() {
                payload.insert("error_code".to_string(), Value::String(error_code.clone()));
            }
            if let Some(retry_after) = error.retry_after {
                payload.insert("retry_after".to_string(), Value::from(retry_after));
            }
            if let Some(raw) = error.raw.as_ref() {
                payload.insert("raw".to_string(), raw.clone());
            }
        }
        None => {
            payload.insert(
                "message".to_string(),
                Value::String("model stream error".to_string()),
            );
            payload.insert("code".to_string(), Value::String("StreamError".to_string()));
            payload.insert(
                "error".to_string(),
                Value::String("model stream error".to_string()),
            );
        }
    }
    if let Some(raw) = event.raw.as_ref() {
        if payload.contains_key("raw") {
            payload.insert("event_raw".to_string(), raw.clone());
        } else {
            payload.insert("raw".to_string(), raw.clone());
        }
    }
    payload
}

fn reasoning_event_text(event: &StreamEvent) -> Option<&str> {
    event
        .reasoning_delta
        .as_deref()
        .or(event.delta.as_deref())
        .or_else(|| {
            event
                .thinking
                .as_ref()
                .map(|thinking| thinking.text.as_str())
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
