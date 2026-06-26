use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::errors::AdapterError;
use crate::request::{FinishReason, Response, ToolCall};
use crate::usage::Usage;

pub type StreamEvents = Box<dyn Iterator<Item = Result<StreamEvent, AdapterError>> + Send>;

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
}

impl StreamAccumulator {
    pub fn push(&mut self, event: StreamEvent) {
        match event.r#type {
            StreamEventType::TextDelta => {
                if let Some(text) = event.delta.as_deref() {
                    self.final_text.push_str(text);
                }
            }
            StreamEventType::ReasoningDelta => {
                if let Some(text) = event.reasoning_delta.as_deref() {
                    self.reasoning_text.push_str(text);
                }
            }
            StreamEventType::ToolCallEnd => {
                if let Some(tool_call) = event.tool_call.clone() {
                    self.tool_calls.push(tool_call);
                }
            }
            StreamEventType::Finish => {
                self.finish_reason = event.finish_reason.clone();
                if let Some(usage) = event.usage.clone() {
                    self.usage = Some(usage.normalized());
                }
            }
            StreamEventType::ProviderEvent => {
                if let Some(raw) = event.raw.clone() {
                    self.raw_provider_events.push(raw);
                }
            }
            _ => {}
        }
        self.events.push(event);
    }
}
