use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use serde::de;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::errors::{AdapterError, AdapterErrorKind};
use crate::request::{Message, ToolCall};
use crate::timeouts::{check_abort, AbortSignal};

pub type ToolExecuteHandler =
    Arc<dyn Fn(ToolInvocation) -> Result<Value, AdapterError> + Send + Sync + 'static>;

#[derive(Clone)]
pub struct ToolRepair(
    Arc<dyn Fn(ToolRepairInvocation) -> Result<Value, AdapterError> + Send + Sync + 'static>,
);

impl ToolRepair {
    pub fn new<F>(repair_handler: F) -> Self
    where
        F: Fn(ToolRepairInvocation) -> Result<Value, AdapterError> + Send + Sync + 'static,
    {
        Self(Arc::new(repair_handler))
    }

    pub fn repair(&self, invocation: ToolRepairInvocation) -> Result<Value, AdapterError> {
        invocation.check_abort()?;
        (self.0)(invocation)
    }
}

impl fmt::Debug for ToolRepair {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ToolRepair(..)")
    }
}

impl PartialEq for ToolRepair {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Clone, Serialize)]
pub struct Tool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
    #[serde(default, flatten)]
    pub provider_metadata: BTreeMap<String, Value>,
    #[serde(skip)]
    pub execute_handler: Option<ToolExecuteHandler>,
}

impl Tool {
    pub fn passive(name: impl Into<String>) -> Result<Self, String> {
        let tool = Self {
            name: name.into(),
            description: None,
            parameters: None,
            provider_metadata: BTreeMap::new(),
            execute_handler: None,
        };
        tool.validate()?;
        Ok(tool)
    }

    pub fn passive_with_schema(
        name: impl Into<String>,
        description: impl Into<Option<String>>,
        parameters: impl Into<Option<Value>>,
    ) -> Result<Self, String> {
        let tool = Self {
            name: name.into(),
            description: description.into(),
            parameters: parameters.into(),
            provider_metadata: BTreeMap::new(),
            execute_handler: None,
        };
        tool.validate()?;
        Ok(tool)
    }

    pub fn active<R, F>(name: impl Into<String>, execute_handler: F) -> Result<Self, String>
    where
        R: Serialize + 'static,
        F: Fn(ToolInvocation) -> Result<R, AdapterError> + Send + Sync + 'static,
    {
        Self::active_with_schema(name, None::<String>, None::<Value>, execute_handler)
    }

    pub fn active_with_schema<R, F>(
        name: impl Into<String>,
        description: impl Into<Option<String>>,
        parameters: impl Into<Option<Value>>,
        execute_handler: F,
    ) -> Result<Self, String>
    where
        R: Serialize + 'static,
        F: Fn(ToolInvocation) -> Result<R, AdapterError> + Send + Sync + 'static,
    {
        let handler = Arc::new(move |invocation: ToolInvocation| {
            let value = execute_handler(invocation)?;
            serde_json::to_value(value).map_err(|error| {
                AdapterError::new(
                    AdapterErrorKind::InvalidToolCall,
                    format!("tool handler returned non-serializable content: {error}"),
                )
            })
        });
        Self::active_with_value_handler(name, description, parameters, handler)
    }

    pub fn active_with_value_handler(
        name: impl Into<String>,
        description: impl Into<Option<String>>,
        parameters: impl Into<Option<Value>>,
        execute_handler: ToolExecuteHandler,
    ) -> Result<Self, String> {
        let tool = Self {
            name: name.into(),
            description: description.into(),
            parameters: parameters.into(),
            provider_metadata: BTreeMap::new(),
            execute_handler: Some(execute_handler),
        };
        tool.validate()?;
        Ok(tool)
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_tool_name(&self.name, "tool name")?;
        if let Some(parameters) = self.parameters.as_ref() {
            validate_parameter_schema(parameters)?;
        }
        Ok(())
    }

    pub fn is_passive(&self) -> bool {
        self.execute_handler.is_none()
    }

    pub fn is_active(&self) -> bool {
        self.execute_handler.is_some()
    }

    pub fn execute(
        &self,
        invocation: ToolInvocation,
    ) -> Result<crate::request::ToolResult, AdapterError> {
        invocation.check_abort()?;
        let Some(handler) = self.execute_handler.as_ref() else {
            return Err(AdapterError::new(
                AdapterErrorKind::InvalidToolCall,
                format!("Tool {:?} has no execute handler", self.name),
            ));
        };
        let tool_call_id = invocation.tool_call_id.clone();
        let content = handler(invocation)?;
        Ok(crate::request::ToolResult::success(tool_call_id, content))
    }
}

impl fmt::Debug for Tool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Tool")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("parameters", &self.parameters)
            .field("provider_metadata", &self.provider_metadata)
            .field("is_active", &self.is_active())
            .finish()
    }
}

impl PartialEq for Tool {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.description == other.description
            && self.parameters == other.parameters
            && self.provider_metadata == other.provider_metadata
            && self.execute_handler.is_some() == other.execute_handler.is_some()
    }
}

impl<'de> Deserialize<'de> for Tool {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let Value::Object(mut object) = value else {
            return Err(de::Error::custom("tool definitions must be objects"));
        };

        let function = object.remove("function").and_then(|value| match value {
            Value::Object(function) => Some(function),
            _ => None,
        });
        object.remove("type");

        let has_function_shape = function.is_some();
        let mut source = function.unwrap_or_else(|| object.clone());
        let name = source
            .remove("name")
            .and_then(|value| value.as_str().map(str::to_string))
            .ok_or_else(|| de::Error::custom("tool definitions require a string name"))?;
        let description = source
            .remove("description")
            .and_then(|value| value.as_str().map(str::to_string));
        let parameters = source
            .remove("parameters")
            .or_else(|| source.remove("parametersJsonSchema"))
            .or_else(|| source.remove("input_schema"));

        let mut provider_metadata = BTreeMap::new();
        if has_function_shape {
            for (key, value) in object {
                provider_metadata.insert(key, value);
            }
        } else {
            for (key, value) in source {
                provider_metadata.insert(key, value);
            }
        }

        let tool = Self {
            name,
            description,
            parameters,
            provider_metadata,
            execute_handler: None,
        };
        tool.validate().map_err(de::Error::custom)?;
        Ok(tool)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolChoice {
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

impl ToolChoice {
    pub fn new(
        mode: impl Into<String>,
        tool_name: impl Into<Option<String>>,
    ) -> Result<Self, String> {
        let choice = Self::from_parts(mode.into(), tool_name.into());
        choice.validate_supported()?;
        Ok(choice)
    }

    pub fn auto() -> Self {
        Self {
            mode: "auto".to_string(),
            tool_name: None,
        }
    }

    pub fn none() -> Self {
        Self {
            mode: "none".to_string(),
            tool_name: None,
        }
    }

    pub fn required() -> Self {
        Self {
            mode: "required".to_string(),
            tool_name: None,
        }
    }

    pub fn named(tool_name: impl Into<String>) -> Result<Self, String> {
        Self::new("named", Some(tool_name.into()))
    }

    pub fn for_tool(tool_name: impl Into<String>) -> Result<Self, String> {
        Self::named(tool_name)
    }

    pub fn is_auto(&self) -> bool {
        matches!(self.kind(), Ok(ToolChoiceKind::Auto))
    }

    pub fn is_none(&self) -> bool {
        matches!(self.kind(), Ok(ToolChoiceKind::None))
    }

    pub fn is_required(&self) -> bool {
        matches!(self.kind(), Ok(ToolChoiceKind::Required))
    }

    pub fn is_named(&self) -> bool {
        matches!(self.kind(), Ok(ToolChoiceKind::Named(_)))
    }

    pub fn tool(&self) -> Option<&str> {
        self.tool_name.as_deref()
    }

    pub(crate) fn kind(&self) -> Result<ToolChoiceKind, ToolChoiceParseError> {
        let mode = self.mode.trim().to_ascii_lowercase();
        match mode.as_str() {
            "auto" => {
                self.reject_tool_name_for_non_named()?;
                Ok(ToolChoiceKind::Auto)
            }
            "none" => {
                self.reject_tool_name_for_non_named()?;
                Ok(ToolChoiceKind::None)
            }
            "required" | "any" => {
                self.reject_tool_name_for_non_named()?;
                Ok(ToolChoiceKind::Required)
            }
            "named" | "function" | "tool" => {
                let Some(tool_name) = self.tool_name.as_deref() else {
                    return Err(ToolChoiceParseError::Invalid(
                        "named tool_choice requires tool_name".to_string(),
                    ));
                };
                validate_tool_name(tool_name, "tool_name")
                    .map_err(ToolChoiceParseError::Invalid)?;
                Ok(ToolChoiceKind::Named(tool_name.to_string()))
            }
            _ => Err(ToolChoiceParseError::Unsupported(format!(
                "tool_choice mode {mode:?} is not supported"
            ))),
        }
    }

    fn from_parts(mode: String, tool_name: Option<String>) -> Self {
        let mode = mode.trim().to_ascii_lowercase();
        let mode = match mode.as_str() {
            "any" => "required".to_string(),
            "function" | "tool" => "named".to_string(),
            _ => mode,
        };
        Self { mode, tool_name }
    }

    fn validate_supported(&self) -> Result<(), String> {
        self.kind().map(|_| ()).map_err(|error| match error {
            ToolChoiceParseError::Invalid(message) | ToolChoiceParseError::Unsupported(message) => {
                message
            }
        })
    }

    pub(crate) fn validate_request_shape(&self) -> Result<(), String> {
        match self.kind() {
            Ok(_) | Err(ToolChoiceParseError::Unsupported(_)) => Ok(()),
            Err(ToolChoiceParseError::Invalid(message)) => Err(message),
        }
    }

    fn reject_tool_name_for_non_named(&self) -> Result<(), ToolChoiceParseError> {
        if self.tool_name.is_some() {
            return Err(ToolChoiceParseError::Invalid(
                "tool_name is only valid for named tool_choice".to_string(),
            ));
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for ToolChoice {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        match value {
            Value::String(mode) => Ok(Self::from_parts(mode, None)),
            Value::Object(object) => tool_choice_from_object(object).map_err(de::Error::custom),
            _ => Err(de::Error::custom("tool_choice must be a string or object")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ToolChoiceKind {
    Auto,
    None,
    Required,
    Named(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ToolChoiceParseError {
    Invalid(String),
    Unsupported(String),
}

impl ToolChoiceParseError {
    pub(crate) fn into_adapter_error(self, provider: &str) -> AdapterError {
        match self {
            Self::Invalid(message) => AdapterError::provider(
                AdapterErrorKind::InvalidRequest,
                message,
                Some(provider.to_string()),
            ),
            Self::Unsupported(message) => AdapterError::provider(
                AdapterErrorKind::UnsupportedToolChoice,
                message,
                Some(provider.to_string()),
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolInvocation {
    pub tool_call: ToolCall,
    pub arguments: Value,
    pub messages: Vec<Message>,
    pub abort_signal: Option<AbortSignal>,
    pub tool_call_id: String,
}

impl ToolInvocation {
    pub fn new(
        tool_call: ToolCall,
        messages: Vec<Message>,
        abort_signal: Option<AbortSignal>,
    ) -> Self {
        Self {
            arguments: tool_call.arguments.clone(),
            tool_call_id: tool_call.id.clone(),
            tool_call,
            messages,
            abort_signal,
        }
    }

    pub fn check_abort(&self) -> Result<(), AdapterError> {
        check_abort(self.abort_signal.as_ref())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolRepairInvocation {
    pub tool_call: ToolCall,
    pub tool_definition: Tool,
    pub validation_error: String,
    pub messages: Vec<Message>,
    pub abort_signal: Option<AbortSignal>,
    pub tool_call_id: String,
}

impl ToolRepairInvocation {
    pub fn new(
        tool_call: ToolCall,
        tool_definition: Tool,
        validation_error: impl Into<String>,
        messages: Vec<Message>,
        abort_signal: Option<AbortSignal>,
    ) -> Self {
        Self {
            tool_call_id: tool_call.id.clone(),
            tool_call,
            tool_definition,
            validation_error: validation_error.into(),
            messages,
            abort_signal,
        }
    }

    pub fn check_abort(&self) -> Result<(), AdapterError> {
        check_abort(self.abort_signal.as_ref())
    }
}

pub fn validate_tool_name(value: &str, field_name: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{field_name} must not be empty"));
    }
    if value.len() > 64 {
        return Err(format!("{field_name} must be 64 characters or fewer"));
    }
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err(format!("{field_name} must not be empty"));
    };
    if !first.is_ascii_alphabetic() {
        return Err(format!(
            "{field_name} must start with an ASCII letter and contain only ASCII letters, numbers, and underscores"
        ));
    }
    if chars.any(|ch| !(ch.is_ascii_alphanumeric() || ch == '_')) {
        return Err(format!(
            "{field_name} must start with an ASCII letter and contain only ASCII letters, numbers, and underscores"
        ));
    }
    Ok(())
}

pub fn validate_parameter_schema(schema: &Value) -> Result<(), String> {
    let object = schema
        .as_object()
        .ok_or_else(|| "tool parameters must be a JSON Schema object".to_string())?;
    match object.get("type") {
        Some(Value::String(kind)) if kind == "object" => Ok(()),
        _ => Err("tool parameters root type must be object".to_string()),
    }
}

fn tool_choice_from_object(mut object: Map<String, Value>) -> Result<ToolChoice, String> {
    let function_name = object
        .get("function")
        .and_then(Value::as_object)
        .and_then(|function| function.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let mode = object
        .remove("mode")
        .or_else(|| object.remove("type"))
        .and_then(|value| value.as_str().map(str::to_string))
        .or_else(|| function_name.as_ref().map(|_| "named".to_string()))
        .ok_or_else(|| "tool_choice requires mode".to_string())?;
    let tool_name = object
        .remove("tool_name")
        .or_else(|| object.remove("tool"))
        .or_else(|| object.remove("name"))
        .and_then(|value| value.as_str().map(str::to_string))
        .or(function_name);

    Ok(ToolChoice::from_parts(mode, tool_name))
}
