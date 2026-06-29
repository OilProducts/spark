use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use unified_llm_adapter::{
    ContentPart, FinishReason, Message, MessageRole, ThinkingData, ToolCallData, ToolResultData,
    Usage, Warning,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TurnContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl Default for TurnContent {
    fn default() -> Self {
        Self::Text(String::new())
    }
}

impl TurnContent {
    pub fn text(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Parts(parts) => parts
                .iter()
                .filter_map(ContentPart::text_content)
                .collect::<Vec<_>>()
                .join(""),
        }
    }

    pub fn content_parts(&self) -> Vec<ContentPart> {
        match self {
            Self::Text(text) => vec![ContentPart::text(text.clone())],
            Self::Parts(parts) => parts.clone(),
        }
    }
}

impl From<String> for TurnContent {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for TurnContent {
    fn from(value: &str) -> Self {
        Self::Text(value.to_string())
    }
}

impl From<Vec<ContentPart>> for TurnContent {
    fn from(value: Vec<ContentPart>) -> Self {
        Self::Parts(value)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserTurn {
    pub content: TurnContent,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

impl UserTurn {
    pub fn new(content: impl Into<TurnContent>) -> Self {
        Self {
            content: content.into(),
            timestamp: OffsetDateTime::now_utc(),
        }
    }

    pub fn text(&self) -> String {
        self.content.text()
    }

    pub fn content_parts(&self) -> Vec<ContentPart> {
        self.content.content_parts()
    }

    pub fn to_message(&self) -> Message {
        message_from_content(MessageRole::User, &self.content)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemTurn {
    pub content: TurnContent,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

impl SystemTurn {
    pub fn new(content: impl Into<TurnContent>) -> Self {
        Self {
            content: content.into(),
            timestamp: OffsetDateTime::now_utc(),
        }
    }

    pub fn text(&self) -> String {
        self.content.text()
    }

    pub fn content_parts(&self) -> Vec<ContentPart> {
        self.content.content_parts()
    }

    pub fn to_message(&self) -> Message {
        message_from_content(MessageRole::System, &self.content)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SteeringTurn {
    pub content: TurnContent,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

impl SteeringTurn {
    pub fn new(content: impl Into<TurnContent>) -> Self {
        Self {
            content: content.into(),
            timestamp: OffsetDateTime::now_utc(),
        }
    }

    pub fn text(&self) -> String {
        self.content.text()
    }

    pub fn content_parts(&self) -> Vec<ContentPart> {
        self.content.content_parts()
    }

    pub fn to_message(&self) -> Message {
        message_from_content(MessageRole::User, &self.content)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantTurn {
    pub content: TurnContent,
    #[serde(default)]
    pub tool_calls: Vec<ToolCallData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<FinishReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
    #[serde(default)]
    pub warnings: Vec<Warning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

impl AssistantTurn {
    pub fn new(content: impl Into<TurnContent>) -> Self {
        Self {
            content: content.into(),
            tool_calls: Vec::new(),
            reasoning: None,
            usage: None,
            response_id: None,
            finish_reason: None,
            raw: None,
            warnings: Vec::new(),
            error: None,
            timestamp: OffsetDateTime::now_utc(),
        }
    }

    pub fn text(&self) -> String {
        self.content.text()
    }

    pub fn content_parts(&self) -> Vec<ContentPart> {
        let mut parts = self.content.content_parts();
        if let Some(reasoning) = self.reasoning.as_ref() {
            if !parts.iter().any(|part| {
                matches!(
                    part,
                    ContentPart::Thinking { .. } | ContentPart::RedactedThinking { .. }
                )
            }) {
                parts.push(ContentPart::Thinking {
                    thinking: ThinkingData {
                        text: reasoning.clone(),
                        signature: None,
                        redacted: false,
                        source_provider: None,
                        source_model: None,
                    },
                });
            }
        }
        for tool_call in &self.tool_calls {
            let already_present = parts.iter().any(|part| {
                matches!(
                    part,
                    ContentPart::ToolCall { tool_call: existing } if existing.id == tool_call.id
                )
            });
            if !already_present {
                parts.push(ContentPart::ToolCall {
                    tool_call: tool_call.clone(),
                });
            }
        }
        parts
    }

    pub fn to_message(&self) -> Message {
        Message {
            role: MessageRole::Assistant,
            content: self.content_parts(),
            name: None,
            tool_call_id: None,
            provider_metadata: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultsTurn {
    #[serde(default)]
    pub result_list: Vec<ToolResultData>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

impl ToolResultsTurn {
    pub fn new<T>(results: impl IntoIterator<Item = T>) -> Self
    where
        T: Into<ToolResultData>,
    {
        Self {
            result_list: results.into_iter().map(Into::into).collect(),
            timestamp: OffsetDateTime::now_utc(),
        }
    }

    pub fn results(&self) -> &[ToolResultData] {
        &self.result_list
    }

    pub fn set_results<T>(&mut self, results: impl IntoIterator<Item = T>)
    where
        T: Into<ToolResultData>,
    {
        self.result_list = results.into_iter().map(Into::into).collect();
    }

    pub fn to_messages(&self) -> Vec<Message> {
        self.result_list
            .iter()
            .map(|result| Message {
                role: MessageRole::Tool,
                content: vec![ContentPart::ToolResult {
                    tool_result: result.clone(),
                }],
                name: None,
                tool_call_id: Some(result.tool_call_id.clone()),
                provider_metadata: BTreeMap::new(),
            })
            .collect()
    }
}

impl Default for ToolResultsTurn {
    fn default() -> Self {
        Self::new(Vec::<ToolResultData>::new())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum HistoryTurn {
    User(UserTurn),
    Assistant(AssistantTurn),
    ToolResults(ToolResultsTurn),
    System(SystemTurn),
    Steering(SteeringTurn),
}

impl HistoryTurn {
    pub fn to_messages(&self) -> Vec<Message> {
        match self {
            Self::User(turn) => vec![turn.to_message()],
            Self::Assistant(turn) => vec![turn.to_message()],
            Self::ToolResults(turn) => turn.to_messages(),
            Self::System(turn) => vec![turn.to_message()],
            Self::Steering(turn) => vec![turn.to_message()],
        }
    }
}

pub fn history_to_messages(history: &[HistoryTurn]) -> Vec<Message> {
    history.iter().flat_map(HistoryTurn::to_messages).collect()
}

fn message_from_content(role: MessageRole, content: &TurnContent) -> Message {
    Message {
        role,
        content: content.content_parts(),
        name: None,
        tool_call_id: None,
        provider_metadata: BTreeMap::new(),
    }
}
