use std::fmt;
use std::str::FromStr;

use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::error::{Result, SparkCommonError};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TurnStreamEventKind {
    ContentDelta,
    ContentCompleted,
    ToolCallStarted,
    ToolCallUpdated,
    ToolCallCompleted,
    ToolCallFailed,
    TokenUsageUpdated,
    RequestUserInputRequested,
    ContextCompactionStarted,
    ContextCompactionCompleted,
    TurnCompleted,
    Error,
    Other(String),
}

impl TurnStreamEventKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::ContentDelta => "content_delta",
            Self::ContentCompleted => "content_completed",
            Self::ToolCallStarted => "tool_call_started",
            Self::ToolCallUpdated => "tool_call_updated",
            Self::ToolCallCompleted => "tool_call_completed",
            Self::ToolCallFailed => "tool_call_failed",
            Self::TokenUsageUpdated => "token_usage_updated",
            Self::RequestUserInputRequested => "request_user_input_requested",
            Self::ContextCompactionStarted => "context_compaction_started",
            Self::ContextCompactionCompleted => "context_compaction_completed",
            Self::TurnCompleted => "turn_completed",
            Self::Error => "error",
            Self::Other(value) => value.as_str(),
        }
    }

    pub fn requires_channel(&self) -> bool {
        matches!(self, Self::ContentDelta | Self::ContentCompleted)
    }
}

impl FromStr for TurnStreamEventKind {
    type Err = std::convert::Infallible;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match value {
            "content_delta" => Self::ContentDelta,
            "content_completed" => Self::ContentCompleted,
            "tool_call_started" => Self::ToolCallStarted,
            "tool_call_updated" => Self::ToolCallUpdated,
            "tool_call_completed" => Self::ToolCallCompleted,
            "tool_call_failed" => Self::ToolCallFailed,
            "token_usage_updated" => Self::TokenUsageUpdated,
            "request_user_input_requested" => Self::RequestUserInputRequested,
            "context_compaction_started" => Self::ContextCompactionStarted,
            "context_compaction_completed" => Self::ContextCompactionCompleted,
            "turn_completed" => Self::TurnCompleted,
            "error" => Self::Error,
            other => Self::Other(other.to_string()),
        })
    }
}

impl Serialize for TurnStreamEventKind {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TurnStreamEventKind {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(Self::from_str(&value).expect("infallible turn stream kind parser"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TurnStreamChannel {
    Assistant,
    Reasoning,
    Plan,
    Other(String),
}

impl TurnStreamChannel {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Assistant => "assistant",
            Self::Reasoning => "reasoning",
            Self::Plan => "plan",
            Self::Other(value) => value.as_str(),
        }
    }
}

impl FromStr for TurnStreamChannel {
    type Err = std::convert::Infallible;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match value {
            "assistant" => Self::Assistant,
            "reasoning" => Self::Reasoning,
            "plan" => Self::Plan,
            other => Self::Other(other.to_string()),
        })
    }
}

impl Serialize for TurnStreamChannel {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TurnStreamChannel {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(Self::from_str(&value).expect("infallible turn stream channel parser"))
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnStreamSource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_index: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TurnStreamEvent {
    pub kind: TurnStreamEventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<TurnStreamChannel>,
    #[serde(default)]
    pub source: TurnStreamSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_delta: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_user_input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

impl TurnStreamEvent {
    pub fn new(kind: TurnStreamEventKind) -> Result<Self> {
        let event = Self {
            kind,
            channel: None,
            source: TurnStreamSource::default(),
            content_delta: None,
            message: None,
            tool_call: None,
            request_user_input: None,
            token_usage: None,
            error: None,
            phase: None,
            status: None,
        };
        event.validate()?;
        Ok(event)
    }

    pub fn content_delta(channel: TurnStreamChannel, content: impl Into<String>) -> Self {
        let content = content.into();
        Self {
            kind: TurnStreamEventKind::ContentDelta,
            channel: Some(channel),
            source: TurnStreamSource::default(),
            content_delta: Some(content.clone()),
            message: Some(content),
            tool_call: None,
            request_user_input: None,
            token_usage: None,
            error: None,
            phase: None,
            status: None,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.kind.requires_channel() && self.channel.is_none() {
            return Err(SparkCommonError::EventValidation(
                "content TurnStreamEvent values must set channel".to_string(),
            ));
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for TurnStreamEvent {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        enum Field {
            Kind,
            Channel,
            Source,
            ContentDelta,
            Message,
            ToolCall,
            RequestUserInput,
            TokenUsage,
            Error,
            Phase,
            Status,
        }

        impl<'de> Deserialize<'de> for Field {
            fn deserialize<D>(deserializer: D) -> std::result::Result<Field, D::Error>
            where
                D: Deserializer<'de>,
            {
                struct FieldVisitor;

                impl Visitor<'_> for FieldVisitor {
                    type Value = Field;

                    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                        formatter.write_str("turn stream event field")
                    }

                    fn visit_str<E>(self, value: &str) -> std::result::Result<Field, E>
                    where
                        E: de::Error,
                    {
                        match value {
                            "kind" => Ok(Field::Kind),
                            "channel" => Ok(Field::Channel),
                            "source" => Ok(Field::Source),
                            "content_delta" => Ok(Field::ContentDelta),
                            "message" => Ok(Field::Message),
                            "tool_call" => Ok(Field::ToolCall),
                            "request_user_input" => Ok(Field::RequestUserInput),
                            "token_usage" => Ok(Field::TokenUsage),
                            "error" => Ok(Field::Error),
                            "phase" => Ok(Field::Phase),
                            "status" => Ok(Field::Status),
                            _ => Err(de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }

                deserializer.deserialize_identifier(FieldVisitor)
            }
        }

        struct EventVisitor;

        impl<'de> Visitor<'de> for EventVisitor {
            type Value = TurnStreamEvent;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("turn stream event object")
            }

            fn visit_map<V>(self, mut map: V) -> std::result::Result<Self::Value, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut kind = None;
                let mut channel = None;
                let mut source = None;
                let mut content_delta = None;
                let mut message = None;
                let mut tool_call = None;
                let mut request_user_input = None;
                let mut token_usage = None;
                let mut error = None;
                let mut phase = None;
                let mut status = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Kind => kind = Some(map.next_value()?),
                        Field::Channel => channel = Some(map.next_value()?),
                        Field::Source => source = Some(map.next_value()?),
                        Field::ContentDelta => content_delta = Some(map.next_value()?),
                        Field::Message => message = Some(map.next_value()?),
                        Field::ToolCall => tool_call = Some(map.next_value()?),
                        Field::RequestUserInput => request_user_input = Some(map.next_value()?),
                        Field::TokenUsage => token_usage = Some(map.next_value()?),
                        Field::Error => error = Some(map.next_value()?),
                        Field::Phase => phase = Some(map.next_value()?),
                        Field::Status => status = Some(map.next_value()?),
                    }
                }

                let event = TurnStreamEvent {
                    kind: kind.ok_or_else(|| de::Error::missing_field("kind"))?,
                    channel: channel.unwrap_or(None),
                    source: source.unwrap_or_default(),
                    content_delta: content_delta.unwrap_or(None),
                    message: message.unwrap_or(None),
                    tool_call: tool_call.unwrap_or(None),
                    request_user_input: request_user_input.unwrap_or(None),
                    token_usage: token_usage.unwrap_or(None),
                    error: error.unwrap_or(None),
                    phase: phase.unwrap_or(None),
                    status: status.unwrap_or(None),
                };
                event.validate().map_err(de::Error::custom)?;
                Ok(event)
            }
        }

        const FIELDS: &[&str] = &[
            "kind",
            "channel",
            "source",
            "content_delta",
            "message",
            "tool_call",
            "request_user_input",
            "token_usage",
            "error",
            "phase",
            "status",
        ];
        deserializer.deserialize_struct("TurnStreamEvent", FIELDS, EventVisitor)
    }
}
