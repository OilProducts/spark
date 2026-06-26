use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::request::{FinishReason, ToolCall};
use crate::usage::Usage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamEvent {
    pub event_type: StreamEventType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<FinishReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

impl StreamEvent {
    pub fn text_delta(text: impl Into<String>) -> Self {
        Self {
            event_type: StreamEventType::TextDelta,
            text: Some(text.into()),
            reasoning: None,
            tool_call: None,
            finish_reason: None,
            usage: None,
            error: None,
            raw: None,
        }
    }

    pub fn finish(reason: FinishReason, usage: Option<Usage>) -> Self {
        Self {
            event_type: StreamEventType::Finish,
            text: None,
            reasoning: None,
            tool_call: None,
            finish_reason: Some(reason),
            usage,
            error: None,
            raw: None,
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
        match event.event_type {
            StreamEventType::TextDelta => {
                if let Some(text) = event.text.as_deref() {
                    self.final_text.push_str(text);
                }
            }
            StreamEventType::ReasoningDelta => {
                if let Some(text) = event.reasoning.as_deref().or(event.text.as_deref()) {
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
