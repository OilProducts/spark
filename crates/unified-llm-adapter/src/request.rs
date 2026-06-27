use std::collections::BTreeMap;
use std::fmt;

use serde::de::{self, DeserializeOwned};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::usage::Usage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
    Developer,
}

pub type Role = MessageRole;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentKind {
    Text,
    Image,
    Audio,
    Document,
    ToolCall,
    ToolResult,
    Thinking,
    RedactedThinking,
    Custom(String),
}

impl ContentKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::Audio => "audio",
            Self::Document => "document",
            Self::ToolCall => "tool_call",
            Self::ToolResult => "tool_result",
            Self::Thinking => "thinking",
            Self::RedactedThinking => "redacted_thinking",
            Self::Custom(kind) => kind.as_str(),
        }
    }

    fn from_kind(kind: &str) -> Self {
        match kind {
            "text" => Self::Text,
            "image" => Self::Image,
            "audio" => Self::Audio,
            "document" => Self::Document,
            "tool_call" => Self::ToolCall,
            "tool_result" => Self::ToolResult,
            "thinking" => Self::Thinking,
            "redacted_thinking" => Self::RedactedThinking,
            other => Self::Custom(other.to_string()),
        }
    }
}

impl Serialize for ContentKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ContentKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let kind = String::deserialize(deserializer)?;
        Ok(Self::from_kind(&kind))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContentPart {
    Text { text: String },
    Image { image: ImageData },
    Audio { audio: AudioData },
    Document { document: DocumentData },
    ToolCall { tool_call: ToolCallData },
    ToolResult { tool_result: ToolResultData },
    Thinking { thinking: ThinkingData },
    RedactedThinking { thinking: ThinkingData },
    Custom { kind: String, raw: Value },
    Raw { kind: String, raw: Value },
    Provider { raw: Value },
}

impl ContentPart {
    pub fn kind(&self) -> ContentKind {
        match self {
            Self::Text { .. } => ContentKind::Text,
            Self::Image { .. } => ContentKind::Image,
            Self::Audio { .. } => ContentKind::Audio,
            Self::Document { .. } => ContentKind::Document,
            Self::ToolCall { .. } => ContentKind::ToolCall,
            Self::ToolResult { .. } => ContentKind::ToolResult,
            Self::Thinking { .. } => ContentKind::Thinking,
            Self::RedactedThinking { .. } => ContentKind::RedactedThinking,
            Self::Custom { kind, .. } | Self::Raw { kind, .. } => ContentKind::Custom(kind.clone()),
            Self::Provider { .. } => ContentKind::Custom("provider".to_string()),
        }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn custom(kind: impl Into<String>, raw: Value) -> Self {
        Self::Custom {
            kind: kind.into(),
            raw,
        }
    }

    pub fn raw(kind: impl Into<String>, raw: Value) -> Self {
        Self::Raw {
            kind: kind.into(),
            raw,
        }
    }

    pub fn text_content(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text.as_str()),
            _ => None,
        }
    }

    pub fn validate_for_role(&self, role: MessageRole) -> Result<(), String> {
        self.validate_payload()?;
        if self.is_allowed_for_role(role) {
            return Ok(());
        }

        Err(format!(
            "{} content is not allowed for {} messages",
            self.kind().as_str(),
            role.as_str()
        ))
    }

    fn validate_payload(&self) -> Result<(), String> {
        match self {
            Self::Image { image } => image.validate(),
            Self::Audio { audio } => audio.validate(),
            Self::Document { document } => document.validate(),
            Self::ToolCall { tool_call } => tool_call.validate(),
            Self::ToolResult { tool_result } => tool_result.validate(),
            Self::Thinking { thinking } => {
                thinking.validate()?;
                if thinking.redacted {
                    return Err("thinking content requires redacted to be false".to_string());
                }
                Ok(())
            }
            Self::RedactedThinking { thinking } => {
                thinking.validate()?;
                if !thinking.redacted {
                    return Err(
                        "redacted_thinking content requires redacted to be true".to_string()
                    );
                }
                Ok(())
            }
            Self::Custom { kind, .. } | Self::Raw { kind, .. } => {
                validate_non_empty("content kind", kind)
            }
            Self::Text { .. } | Self::Provider { .. } => Ok(()),
        }
    }

    fn is_allowed_for_role(&self, role: MessageRole) -> bool {
        match self {
            Self::Text { .. } => matches!(
                role,
                MessageRole::System
                    | MessageRole::User
                    | MessageRole::Assistant
                    | MessageRole::Tool
                    | MessageRole::Developer
            ),
            Self::Image { .. } => matches!(role, MessageRole::User | MessageRole::Assistant),
            Self::Audio { .. } | Self::Document { .. } => matches!(role, MessageRole::User),
            Self::ToolCall { .. } => matches!(role, MessageRole::Assistant),
            Self::ToolResult { .. } => matches!(role, MessageRole::Tool),
            Self::Thinking { .. } | Self::RedactedThinking { .. } => {
                matches!(role, MessageRole::Assistant)
            }
            Self::Custom { .. } | Self::Raw { .. } | Self::Provider { .. } => true,
        }
    }

    fn to_json_value(&self) -> Result<Value, serde_json::Error> {
        let mut object = Map::new();
        match self {
            Self::Text { text } => {
                object.insert("kind".to_string(), Value::String("text".to_string()));
                object.insert("text".to_string(), Value::String(text.clone()));
            }
            Self::Image { image } => {
                object.insert("kind".to_string(), Value::String("image".to_string()));
                object.insert("image".to_string(), serde_json::to_value(image)?);
            }
            Self::Audio { audio } => {
                object.insert("kind".to_string(), Value::String("audio".to_string()));
                object.insert("audio".to_string(), serde_json::to_value(audio)?);
            }
            Self::Document { document } => {
                object.insert("kind".to_string(), Value::String("document".to_string()));
                object.insert("document".to_string(), serde_json::to_value(document)?);
            }
            Self::ToolCall { tool_call } => {
                object.insert("kind".to_string(), Value::String("tool_call".to_string()));
                object.insert("tool_call".to_string(), serde_json::to_value(tool_call)?);
            }
            Self::ToolResult { tool_result } => {
                object.insert("kind".to_string(), Value::String("tool_result".to_string()));
                object.insert(
                    "tool_result".to_string(),
                    serde_json::to_value(tool_result)?,
                );
            }
            Self::Thinking { thinking } => {
                object.insert("kind".to_string(), Value::String("thinking".to_string()));
                object.insert("thinking".to_string(), serde_json::to_value(thinking)?);
            }
            Self::RedactedThinking { thinking } => {
                object.insert(
                    "kind".to_string(),
                    Value::String("redacted_thinking".to_string()),
                );
                object.insert("thinking".to_string(), serde_json::to_value(thinking)?);
            }
            Self::Custom { kind, raw } | Self::Raw { kind, raw } => {
                object.insert("kind".to_string(), Value::String(kind.clone()));
                object.insert("raw".to_string(), raw.clone());
            }
            Self::Provider { raw } => {
                object.insert("kind".to_string(), Value::String("provider".to_string()));
                object.insert("raw".to_string(), raw.clone());
            }
        }
        Ok(Value::Object(object))
    }

    fn from_json_value(value: Value) -> Result<Self, String> {
        let Value::Object(mut object) = value else {
            return Err("content part must be a JSON object".to_string());
        };
        let Some(Value::String(kind)) = object.remove("kind") else {
            return Err("content part requires string kind".to_string());
        };

        match kind.as_str() {
            "text" => Ok(Self::Text {
                text: take_payload(&mut object, "text", &kind)?,
            }),
            "image" => Ok(Self::Image {
                image: take_payload(&mut object, "image", &kind)?,
            }),
            "audio" => Ok(Self::Audio {
                audio: take_payload(&mut object, "audio", &kind)?,
            }),
            "document" => Ok(Self::Document {
                document: take_payload(&mut object, "document", &kind)?,
            }),
            "tool_call" => Ok(Self::ToolCall {
                tool_call: take_payload(&mut object, "tool_call", &kind)?,
            }),
            "tool_result" => Ok(Self::ToolResult {
                tool_result: take_payload(&mut object, "tool_result", &kind)?,
            }),
            "thinking" => Ok(Self::Thinking {
                thinking: take_payload(&mut object, "thinking", &kind)?,
            }),
            "redacted_thinking" => Ok(Self::RedactedThinking {
                thinking: take_payload(&mut object, "thinking", &kind)?,
            }),
            "provider" => Ok(Self::Provider {
                raw: object.remove("raw").unwrap_or(Value::Object(object)),
            }),
            _ => Ok(Self::Custom {
                kind,
                raw: object.remove("raw").unwrap_or(Value::Object(object)),
            }),
        }
    }
}

impl Serialize for ContentPart {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.to_json_value()
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ContentPart {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        Self::from_json_value(value).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ImageData {
    pub fn url(url: impl Into<String>) -> Self {
        Self {
            url: Some(url.into()),
            data: None,
            media_type: None,
            detail: None,
        }
    }

    pub fn data(data: impl Into<Vec<u8>>, media_type: Option<String>) -> Self {
        Self {
            url: None,
            data: Some(data.into()),
            media_type: Some(media_type.unwrap_or_else(|| "image/png".to_string())),
            detail: None,
        }
    }

    pub fn effective_media_type(&self) -> Option<&str> {
        self.media_type
            .as_deref()
            .or_else(|| self.data.as_ref().map(|_| "image/png"))
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_exactly_one_source(self.url.as_deref(), self.data.as_ref(), "image")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

impl AudioData {
    pub fn validate(&self) -> Result<(), String> {
        validate_exactly_one_source(self.url.as_deref(), self.data.as_ref(), "audio")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
}

impl DocumentData {
    pub fn validate(&self) -> Result<(), String> {
        validate_exactly_one_source(self.url.as_deref(), self.data.as_ref(), "document")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    #[serde(default)]
    pub content: Vec<ContentPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub provider_metadata: BTreeMap<String, Value>,
}

impl Default for Message {
    fn default() -> Self {
        Self {
            role: MessageRole::Assistant,
            content: Vec::new(),
            name: None,
            tool_call_id: None,
            provider_metadata: BTreeMap::new(),
        }
    }
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self::from_text(MessageRole::User, text)
    }

    pub fn system(text: impl Into<String>) -> Self {
        Self::from_text(MessageRole::System, text)
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self::from_text(MessageRole::Assistant, text)
    }

    pub fn developer(text: impl Into<String>) -> Self {
        Self::from_text(MessageRole::Developer, text)
    }

    pub fn tool_result(
        tool_call_id: impl Into<String>,
        content: impl Into<Value>,
        is_error: bool,
    ) -> Self {
        let tool_call_id = tool_call_id.into();
        Self {
            role: MessageRole::Tool,
            content: vec![ContentPart::ToolResult {
                tool_result: ToolResultData {
                    tool_call_id: tool_call_id.clone(),
                    content: content.into(),
                    is_error,
                    image_data: None,
                    image_media_type: None,
                },
            }],
            name: None,
            tool_call_id: Some(tool_call_id),
            provider_metadata: BTreeMap::new(),
        }
    }

    pub fn from_text(role: MessageRole, text: impl Into<String>) -> Self {
        Self {
            role,
            content: vec![ContentPart::Text { text: text.into() }],
            name: None,
            tool_call_id: None,
            provider_metadata: BTreeMap::new(),
        }
    }

    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(ContentPart::text_content)
            .collect::<Vec<_>>()
            .join("")
    }

    pub fn validate(&self) -> Result<(), String> {
        for part in &self.content {
            part.validate_for_role(self.role)?;
        }
        Ok(())
    }
}

impl MessageRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
            Self::Developer => "developer",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
    #[serde(default)]
    pub raw_arguments: Option<String>,
    #[serde(default = "default_function_type")]
    pub r#type: String,
}

impl ToolCall {
    pub fn validate(&self) -> Result<(), String> {
        validate_non_empty("tool call id", &self.id)?;
        validate_non_empty("tool call name", &self.name)?;
        validate_non_empty("tool call type", &self.r#type)
    }
}

pub type ToolCallData = ToolCall;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub content: Value,
    #[serde(default)]
    pub is_error: bool,
}

impl ToolResult {
    pub fn success(tool_call_id: impl Into<String>, content: impl Into<Value>) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            content: content.into(),
            is_error: false,
        }
    }

    pub fn error(tool_call_id: impl Into<String>, content: impl Into<Value>) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            content: content.into(),
            is_error: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultData {
    pub tool_call_id: String,
    pub content: Value,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_data: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_media_type: Option<String>,
}

impl ToolResultData {
    pub fn validate(&self) -> Result<(), String> {
        validate_non_empty("tool result tool_call_id", &self.tool_call_id)
    }
}

impl From<ToolResult> for ToolResultData {
    fn from(result: ToolResult) -> Self {
        Self {
            tool_call_id: result.tool_call_id,
            content: result.content,
            is_error: result.is_error,
            image_data: None,
            image_media_type: None,
        }
    }
}

impl From<ToolResultData> for ToolResult {
    fn from(result: ToolResultData) -> Self {
        Self {
            tool_call_id: result.tool_call_id,
            content: result.content,
            is_error: result.is_error,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThinkingData {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default)]
    pub redacted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_model: Option<String>,
}

impl ThinkingData {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(provider) = self.source_provider.as_deref() {
            validate_non_empty("thinking source_provider", provider)?;
        }
        if let Some(model) = self.source_model.as_deref() {
            validate_non_empty("thinking source_model", model)?;
        }
        if self.redacted {
            return Ok(());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    Text,
    #[serde(rename = "json", alias = "json_object")]
    JsonObject,
    JsonSchema {
        json_schema: Value,
        #[serde(default)]
        strict: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReasonKind {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Error,
    Other,
}

impl FinishReasonKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Length => "length",
            Self::ToolCalls => "tool_calls",
            Self::ContentFilter => "content_filter",
            Self::Error => "error",
            Self::Other => "other",
        }
    }

    fn from_reason(reason: &str) -> Self {
        match reason {
            "stop" => Self::Stop,
            "length" => Self::Length,
            "tool_calls" => Self::ToolCalls,
            "content_filter" => Self::ContentFilter,
            "error" => Self::Error,
            _ => Self::Other,
        }
    }
}

impl Serialize for FinishReasonKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for FinishReasonKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let reason = String::deserialize(deserializer)?;
        Ok(Self::from_reason(&reason))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FinishReason {
    pub reason: FinishReasonKind,
    #[serde(default)]
    pub raw: Option<String>,
}

impl Default for FinishReason {
    fn default() -> Self {
        Self::Other
    }
}

#[allow(non_upper_case_globals)]
impl FinishReason {
    pub const Stop: Self = Self {
        reason: FinishReasonKind::Stop,
        raw: None,
    };
    pub const Length: Self = Self {
        reason: FinishReasonKind::Length,
        raw: None,
    };
    pub const ToolCalls: Self = Self {
        reason: FinishReasonKind::ToolCalls,
        raw: None,
    };
    pub const ContentFilter: Self = Self {
        reason: FinishReasonKind::ContentFilter,
        raw: None,
    };
    pub const Error: Self = Self {
        reason: FinishReasonKind::Error,
        raw: None,
    };
    pub const Other: Self = Self {
        reason: FinishReasonKind::Other,
        raw: None,
    };

    pub fn new(reason: FinishReasonKind, raw: impl Into<Option<String>>) -> Self {
        Self {
            reason,
            raw: raw.into(),
        }
    }

    pub fn from_provider(reason: FinishReasonKind, raw: impl Into<String>) -> Self {
        Self {
            reason,
            raw: Some(raw.into()),
        }
    }
}

impl<'de> Deserialize<'de> for FinishReason {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct FinishReasonFields {
            reason: FinishReasonKind,
            #[serde(default)]
            raw: Option<String>,
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum FinishReasonInput {
            Reason(String),
            Fields(FinishReasonFields),
        }

        match FinishReasonInput::deserialize(deserializer)? {
            FinishReasonInput::Reason(reason) => Ok(Self {
                reason: FinishReasonKind::from_reason(&reason),
                raw: None,
            }),
            FinishReasonInput::Fields(fields) => Ok(Self {
                reason: fields.reason,
                raw: fields.raw,
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub model: String,
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default)]
    pub tools: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stop_sequences: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
    #[serde(default)]
    pub provider_options: BTreeMap<String, Value>,
}

impl Default for Request {
    fn default() -> Self {
        Self {
            model: String::new(),
            messages: Vec::new(),
            provider: None,
            tools: Vec::new(),
            tool_choice: None,
            response_format: None,
            temperature: None,
            top_p: None,
            max_tokens: None,
            stop_sequences: Vec::new(),
            reasoning_effort: None,
            metadata: BTreeMap::new(),
            provider_options: BTreeMap::new(),
        }
    }
}

impl Request {
    pub fn new(model: impl Into<String>, messages: Vec<Message>) -> Self {
        Self {
            model: model.into(),
            messages,
            ..Self::default()
        }
    }

    pub fn validate_for_client(&self) -> Result<(), String> {
        validate_non_empty("request model", &self.model)?;
        for message in &self.messages {
            message.validate()?;
        }
        if let Some(temperature) = self.temperature {
            validate_finite("temperature", temperature)?;
        }
        if let Some(top_p) = self.top_p {
            validate_finite("top_p", top_p)?;
        }
        Ok(())
    }
}

pub type LlmRequest = Request;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Response {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub message: Message,
    #[serde(default)]
    pub finish_reason: FinishReason,
    #[serde(default)]
    pub usage: Usage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
    #[serde(default)]
    pub warnings: Vec<Warning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit: Option<RateLimitInfo>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub raw_provider_events: Vec<Value>,
}

impl Response {
    pub fn from_text(text: impl Into<String>) -> Self {
        Self {
            message: Message::assistant(text),
            ..Self::default()
        }
    }

    pub fn text(&self) -> String {
        let message_text = self.message.text();
        if message_text.is_empty() {
            self.text.clone()
        } else {
            message_text
        }
    }

    pub fn tool_calls(&self) -> Vec<ToolCall> {
        let extracted = self
            .message
            .content
            .iter()
            .filter_map(|part| match part {
                ContentPart::ToolCall { tool_call } => Some(tool_call.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        if extracted.is_empty() {
            self.tool_calls.clone()
        } else {
            extracted
        }
    }

    pub fn reasoning(&self) -> Option<String> {
        let mut saw_reasoning = false;
        let reasoning = self
            .message
            .content
            .iter()
            .filter_map(|part| match part {
                ContentPart::Thinking { thinking } | ContentPart::RedactedThinking { thinking } => {
                    saw_reasoning = true;
                    Some(thinking.text.as_str())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        saw_reasoning.then_some(reasoning)
    }
}

pub type LlmResponse = Response;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warning {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimitInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requests_remaining: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requests_limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_remaining: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<String>,
}

fn default_function_type() -> String {
    "function".to_string()
}

fn take_payload<T: DeserializeOwned>(
    object: &mut Map<String, Value>,
    field: &str,
    kind: &str,
) -> Result<T, String> {
    let value = object
        .remove(field)
        .ok_or_else(|| format!("{kind} content requires {field}"))?;
    serde_json::from_value(value)
        .map_err(|error| format!("invalid {kind} {field} payload: {error}"))
}

fn validate_non_empty(field: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    Ok(())
}

fn validate_exactly_one_source<T>(
    url: Option<&str>,
    data: Option<&T>,
    kind: &str,
) -> Result<(), String> {
    if url.is_some() == data.is_some() {
        return Err(format!(
            "exactly one of url or data must be provided for {kind}"
        ));
    }
    Ok(())
}

fn validate_finite(field: &str, value: f64) -> Result<(), String> {
    if !value.is_finite() {
        return Err(format!("{field} must be finite"));
    }
    Ok(())
}

impl fmt::Display for FinishReasonKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
