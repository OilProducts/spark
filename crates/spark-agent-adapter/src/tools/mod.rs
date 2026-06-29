pub mod builtins;

use std::collections::BTreeMap;
use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;
use std::thread;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Map, Value};
use unified_llm_adapter::tools::{validate_parameter_schema, validate_tool_name};
use unified_llm_adapter::{
    AdapterError, AdapterErrorKind, Message, Tool, ToolCall, ToolInvocation, ToolResult,
};

use crate::config::SessionConfig;
use crate::environment::ExecutionEnvironment;
use crate::events::EventKind;
use crate::history::TurnContent;
use crate::truncation::truncate_tool_output;

pub type ToolExecutor =
    Arc<dyn Fn(ToolExecution) -> Result<ToolExecutionOutput, AdapterError> + Send + Sync + 'static>;
pub type ToolTruncationHook = Arc<dyn Fn(ToolTruncation) -> Value + Send + Sync + 'static>;
pub type ToolEventHook = Arc<dyn Fn(ToolDispatchEvent) + Send + Sync + 'static>;
pub type ToolHostControlHook = Arc<dyn Fn(TurnContent) + Send + Sync + 'static>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl ToolDefinition {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: impl Into<Option<Value>>,
    ) -> Result<Self, String> {
        let definition = Self {
            name: name.into(),
            description: description.into(),
            parameters: parameters
                .into()
                .unwrap_or_else(|| json!({"type": "object"})),
        };
        definition.validate()?;
        Ok(definition)
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_tool_name(&self.name, "tool name")?;
        validate_parameter_schema(&self.parameters)?;
        jsonschema::meta::validate(&self.parameters)
            .map_err(|error| format!("tool parameters must be a valid JSON Schema: {error}"))?;
        Ok(())
    }

    pub fn to_llm_tool(&self) -> Tool {
        Tool::passive_with_schema(
            self.name.clone(),
            Some(self.description.clone()),
            Some(self.parameters.clone()),
        )
        .expect("validated tool definitions convert to unified tool definitions")
    }
}

impl TryFrom<Tool> for ToolDefinition {
    type Error = String;

    fn try_from(tool: Tool) -> Result<Self, Self::Error> {
        Self::new(
            tool.name,
            tool.description.unwrap_or_default(),
            Some(tool.parameters.unwrap_or_else(|| json!({"type": "object"}))),
        )
    }
}

impl From<ToolDefinition> for Tool {
    fn from(definition: ToolDefinition) -> Self {
        definition.to_llm_tool()
    }
}

#[derive(Clone)]
pub struct RegisteredTool {
    pub definition: ToolDefinition,
    pub executor: ToolExecutor,
}

impl RegisteredTool {
    pub fn new<R, F>(definition: ToolDefinition, executor: F) -> Self
    where
        R: Serialize + 'static,
        F: Fn(ToolExecution) -> Result<R, AdapterError> + Send + Sync + 'static,
    {
        let name = definition.name.clone();
        Self::new_with_executor(
            definition,
            Arc::new(move |invocation| {
                let output = executor(invocation)?;
                serde_json::to_value(output)
                    .map(ToolExecutionOutput::success)
                    .map_err(|error| {
                        AdapterError::new(
                            AdapterErrorKind::InvalidToolCall,
                            format!(
                                "tool handler {name:?} returned non-serializable content: {error}"
                            ),
                        )
                    })
            }),
        )
    }

    pub fn new_with_executor(definition: ToolDefinition, executor: ToolExecutor) -> Self {
        Self {
            definition,
            executor,
        }
    }

    pub fn not_executable(definition: ToolDefinition) -> Self {
        let name = definition.name.clone();
        Self::new_with_executor(
            definition,
            Arc::new(move |_| {
                Err(AdapterError::new(
                    AdapterErrorKind::InvalidToolCall,
                    format!("Tool {name:?} has no Rust executor"),
                ))
            }),
        )
    }

    pub fn to_llm_tool(&self) -> Tool {
        self.definition.to_llm_tool()
    }
}

impl fmt::Debug for RegisteredTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegisteredTool")
            .field("definition", &self.definition)
            .finish_non_exhaustive()
    }
}

impl PartialEq for RegisteredTool {
    fn eq(&self, other: &Self) -> bool {
        self.definition == other.definition
    }
}

impl From<ToolDefinition> for RegisteredTool {
    fn from(definition: ToolDefinition) -> Self {
        Self::not_executable(definition)
    }
}

impl From<Tool> for RegisteredTool {
    fn from(tool: Tool) -> Self {
        let definition = ToolDefinition::try_from(tool.clone())
            .expect("unified tool must convert to a definition");
        if tool.is_passive() {
            return Self::not_executable(definition);
        }

        Self::new_with_executor(
            definition,
            Arc::new(move |invocation| {
                let mut tool_call = invocation.tool_call;
                tool_call.arguments = invocation.arguments;
                let result =
                    tool.execute(ToolInvocation::new(tool_call, invocation.messages, None))?;
                Ok(ToolExecutionOutput {
                    event_output: result.content.clone(),
                    content: result.content,
                    is_error: result.is_error,
                    image_data: result.image_data,
                    image_media_type: result.image_media_type,
                })
            }),
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolExecution {
    pub tool_call: ToolCall,
    pub tool_call_id: String,
    pub tool_name: String,
    pub arguments: Value,
    pub messages: Vec<Message>,
    pub execution_environment: ExecutionEnvironment,
    pub config: SessionConfig,
    pub capabilities: BTreeMap<String, bool>,
    pub host_controls: ToolHostControls,
}

#[derive(Clone, Default)]
pub struct ToolHostControls {
    steering_hook: Option<ToolHostControlHook>,
    follow_up_hook: Option<ToolHostControlHook>,
}

impl ToolHostControls {
    pub fn new(
        steering_hook: impl Into<Option<ToolHostControlHook>>,
        follow_up_hook: impl Into<Option<ToolHostControlHook>>,
    ) -> Self {
        Self {
            steering_hook: steering_hook.into(),
            follow_up_hook: follow_up_hook.into(),
        }
    }

    pub fn steer(&self, content: impl Into<TurnContent>) -> bool {
        let Some(hook) = self.steering_hook.as_ref() else {
            return false;
        };
        hook(content.into());
        true
    }

    pub fn follow_up(&self, content: impl Into<TurnContent>) -> bool {
        let Some(hook) = self.follow_up_hook.as_ref() else {
            return false;
        };
        hook(content.into());
        true
    }

    pub fn supports_steering(&self) -> bool {
        self.steering_hook.is_some()
    }

    pub fn supports_follow_up(&self) -> bool {
        self.follow_up_hook.is_some()
    }
}

impl fmt::Debug for ToolHostControls {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ToolHostControls")
            .field("supports_steering", &self.supports_steering())
            .field("supports_follow_up", &self.supports_follow_up())
            .finish()
    }
}

impl PartialEq for ToolHostControls {
    fn eq(&self, other: &Self) -> bool {
        match (&self.steering_hook, &other.steering_hook) {
            (Some(left), Some(right)) if !Arc::ptr_eq(left, right) => return false,
            (None, None) | (Some(_), Some(_)) => {}
            _ => return false,
        }
        match (&self.follow_up_hook, &other.follow_up_hook) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            (None, None) => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolExecutionOutput {
    pub content: Value,
    pub is_error: bool,
    pub event_output: Value,
    pub image_data: Option<Vec<u8>>,
    pub image_media_type: Option<String>,
}

impl ToolExecutionOutput {
    pub fn success(content: impl Into<Value>) -> Self {
        let content = content.into();
        Self {
            event_output: content.clone(),
            content,
            is_error: false,
            image_data: None,
            image_media_type: None,
        }
    }

    pub fn error(content: impl Into<Value>) -> Self {
        let content = content.into();
        Self {
            event_output: content.clone(),
            content,
            is_error: true,
            image_data: None,
            image_media_type: None,
        }
    }

    pub fn with_event_output(mut self, event_output: impl Into<Value>) -> Self {
        self.event_output = event_output.into();
        self
    }

    pub fn with_image(
        mut self,
        image_data: impl Into<Vec<u8>>,
        image_media_type: impl Into<Option<String>>,
    ) -> Self {
        self.image_data = Some(image_data.into());
        self.image_media_type = image_media_type.into();
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolTruncation {
    pub tool_call_id: String,
    pub tool_name: String,
    pub full_content: Value,
    pub default_model_content: Value,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolDispatchEvent {
    pub kind: EventKind,
    pub data: BTreeMap<String, Value>,
}

impl ToolDispatchEvent {
    pub fn tool_call_start(tool_call: &ToolCall) -> Self {
        let mut data = BTreeMap::from([
            (
                "tool_call_id".to_string(),
                Value::String(tool_call.id.clone()),
            ),
            (
                "tool_name".to_string(),
                Value::String(tool_call.name.clone()),
            ),
            ("arguments".to_string(), tool_call.arguments.clone()),
        ]);
        if let Some(raw_arguments) = tool_call.raw_arguments.as_ref() {
            data.insert(
                "raw_arguments".to_string(),
                Value::String(raw_arguments.clone()),
            );
        }
        Self {
            kind: EventKind::ToolCallStart,
            data,
        }
    }

    pub fn tool_call_end(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        output: ToolExecutionOutput,
        model_content: Value,
    ) -> Self {
        let tool_call_id = tool_call_id.into();
        let tool_name = tool_name.into();
        let mut data = BTreeMap::from([
            ("tool_call_id".to_string(), Value::String(tool_call_id)),
            ("tool_name".to_string(), Value::String(tool_name)),
            ("model_content".to_string(), model_content),
        ]);
        if output.is_error {
            data.insert("error".to_string(), output.event_output);
        } else {
            data.insert("output".to_string(), output.event_output);
        }
        Self {
            kind: EventKind::ToolCallEnd,
            data,
        }
    }
}

#[derive(Clone)]
pub struct ToolDispatchContext {
    pub execution_environment: ExecutionEnvironment,
    pub messages: Vec<Message>,
    pub config: SessionConfig,
    pub capabilities: BTreeMap<String, bool>,
    pub supports_parallel_tool_calls: bool,
    pub host_controls: ToolHostControls,
    pub truncation_hook: Option<ToolTruncationHook>,
    pub event_hook: Option<ToolEventHook>,
}

impl Default for ToolDispatchContext {
    fn default() -> Self {
        Self {
            execution_environment: ExecutionEnvironment::default(),
            messages: Vec::new(),
            config: SessionConfig::default(),
            capabilities: BTreeMap::new(),
            supports_parallel_tool_calls: false,
            host_controls: ToolHostControls::default(),
            truncation_hook: None,
            event_hook: None,
        }
    }
}

impl fmt::Debug for ToolDispatchContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ToolDispatchContext")
            .field("execution_environment", &self.execution_environment)
            .field("messages", &self.messages)
            .field("config", &self.config)
            .field("capabilities", &self.capabilities)
            .field(
                "supports_parallel_tool_calls",
                &self.supports_parallel_tool_calls,
            )
            .field("host_controls", &self.host_controls)
            .field("has_truncation_hook", &self.truncation_hook.is_some())
            .field("has_event_hook", &self.event_hook.is_some())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ToolRegistry {
    tools: Vec<RegisteredTool>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_tools(tools: impl IntoIterator<Item = Tool>) -> Self {
        let mut registry = Self::new();
        for tool in tools {
            registry.register(tool);
        }
        registry
    }

    pub fn register(&mut self, tool: impl Into<RegisteredTool>) -> Option<RegisteredTool> {
        let tool = tool.into();
        if let Some(index) = self
            .tools
            .iter()
            .position(|candidate| candidate.definition.name == tool.definition.name)
        {
            return Some(std::mem::replace(&mut self.tools[index], tool));
        }

        self.tools.push(tool);
        None
    }

    pub fn unregister(&mut self, name: &str) -> Option<RegisteredTool> {
        let index = self
            .tools
            .iter()
            .position(|tool| tool.definition.name == name)?;
        Some(self.tools.remove(index))
    }

    pub fn get(&self, name: &str) -> Option<&RegisteredTool> {
        self.tools.iter().find(|tool| tool.definition.name == name)
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .iter()
            .map(|tool| tool.definition.clone())
            .collect()
    }

    pub fn llm_definitions(&self) -> Vec<Tool> {
        self.tools.iter().map(RegisteredTool::to_llm_tool).collect()
    }

    pub fn names(&self) -> Vec<String> {
        self.tools
            .iter()
            .map(|tool| tool.definition.name.clone())
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn dispatch(&self, tool_call: ToolCall, context: ToolDispatchContext) -> ToolResult {
        dispatch_one(self, tool_call, context)
    }

    pub fn dispatch_many(
        &self,
        tool_calls: impl IntoIterator<Item = ToolCall>,
        context: ToolDispatchContext,
    ) -> Vec<ToolResult> {
        let tool_calls = tool_calls.into_iter().collect::<Vec<_>>();
        if !context.supports_parallel_tool_calls || tool_calls.len() < 2 {
            return tool_calls
                .into_iter()
                .map(|tool_call| self.dispatch(tool_call, context.clone()))
                .collect();
        }

        thread::scope(|scope| {
            let handles = tool_calls
                .into_iter()
                .map(|tool_call| {
                    let tool_call_id = tool_call.id.clone();
                    let context = context.clone();
                    (
                        tool_call_id,
                        scope.spawn(move || self.dispatch(tool_call, context)),
                    )
                })
                .collect::<Vec<_>>();

            handles
                .into_iter()
                .map(|(tool_call_id, handle)| match handle.join() {
                    Ok(result) => result,
                    Err(_) => ToolResult::error(
                        tool_call_id,
                        Value::String("tool dispatch worker panicked".to_string()),
                    ),
                })
                .collect()
        })
    }
}

impl From<Vec<Tool>> for ToolRegistry {
    fn from(tools: Vec<Tool>) -> Self {
        Self::from_tools(tools)
    }
}

impl Serialize for ToolRegistry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.definitions().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ToolRegistry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let definitions = Vec::<ToolDefinition>::deserialize(deserializer)?;
        Ok(Self {
            tools: definitions
                .into_iter()
                .map(RegisteredTool::not_executable)
                .collect(),
        })
    }
}

fn dispatch_one(
    registry: &ToolRegistry,
    tool_call: ToolCall,
    context: ToolDispatchContext,
) -> ToolResult {
    let tool_call_id = tool_call.id.clone();
    let tool_name = tool_call.name.clone();
    if let Some(event_hook) = context.event_hook.as_ref() {
        event_hook(ToolDispatchEvent::tool_call_start(&tool_call));
    }

    let Some(registered) = registry.get(&tool_name) else {
        return finalize_tool_result(
            &tool_call_id,
            &tool_name,
            ToolExecutionOutput::error(Value::String(format!("Unknown tool: {tool_name}"))),
            &context,
        );
    };

    let arguments = match parse_tool_arguments(&tool_call) {
        Ok(arguments) => arguments,
        Err(message) => {
            return finalize_tool_result(
                &tool_call_id,
                &tool_name,
                ToolExecutionOutput::error(Value::String(message)),
                &context,
            );
        }
    };
    if let Err(message) = validate_tool_arguments(&registered.definition, &arguments) {
        return finalize_tool_result(
            &tool_call_id,
            &tool_name,
            ToolExecutionOutput::error(Value::String(message)),
            &context,
        );
    }

    let invocation = ToolExecution {
        tool_call,
        tool_call_id: tool_call_id.clone(),
        tool_name: tool_name.clone(),
        arguments,
        messages: context.messages.clone(),
        execution_environment: context.execution_environment.clone(),
        config: context.config.clone(),
        capabilities: context.capabilities.clone(),
        host_controls: context.host_controls.clone(),
    };
    let executor = registered.executor.clone();
    let execution = catch_unwind(AssertUnwindSafe(|| (executor)(invocation)));
    let output = match execution {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => ToolExecutionOutput::error(Value::String(format!(
            "Tool error ({tool_name}): {}",
            error.message
        ))),
        Err(_) => ToolExecutionOutput::error(Value::String(format!(
            "Tool error ({tool_name}): executor panicked"
        ))),
    };

    finalize_tool_result(&tool_call_id, &tool_name, output, &context)
}

fn parse_tool_arguments(tool_call: &ToolCall) -> Result<Value, String> {
    let arguments = if let Some(raw_arguments) = tool_call.raw_arguments.as_deref() {
        parse_raw_arguments(raw_arguments, &tool_call.name)?
    } else if let Value::String(raw_arguments) = &tool_call.arguments {
        parse_raw_arguments(raw_arguments, &tool_call.name)?
    } else if tool_call.arguments.is_null() {
        Value::Object(Map::new())
    } else {
        tool_call.arguments.clone()
    };

    if !arguments.is_object() {
        return Err(format!(
            "Invalid arguments for tool: {}: expected a JSON object",
            tool_call.name
        ));
    }
    Ok(arguments)
}

fn parse_raw_arguments(raw_arguments: &str, tool_name: &str) -> Result<Value, String> {
    let raw_arguments = raw_arguments.trim();
    if raw_arguments.is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    serde_json::from_str(raw_arguments)
        .map_err(|error| format!("Invalid arguments for tool: {tool_name}: {error}"))
}

fn validate_tool_arguments(definition: &ToolDefinition, arguments: &Value) -> Result<(), String> {
    let validator = jsonschema::validator_for(&definition.parameters).map_err(|error| {
        format!(
            "Invalid arguments for tool: {}: schema is invalid: {error}",
            definition.name
        )
    })?;
    validator
        .validate(arguments)
        .map_err(|error| format!("Invalid arguments for tool: {}: {error}", definition.name))
}

fn finalize_tool_result(
    tool_call_id: &str,
    tool_name: &str,
    output: ToolExecutionOutput,
    context: &ToolDispatchContext,
) -> ToolResult {
    let default_model_content =
        default_truncated_content(&output.content, tool_name, &context.config);
    let truncation = ToolTruncation {
        tool_call_id: tool_call_id.to_string(),
        tool_name: tool_name.to_string(),
        full_content: output.content.clone(),
        default_model_content,
        is_error: output.is_error,
    };
    let model_content = context
        .truncation_hook
        .as_ref()
        .map(|hook| hook(truncation.clone()))
        .unwrap_or(truncation.default_model_content);

    if let Some(event_hook) = context.event_hook.as_ref() {
        event_hook(ToolDispatchEvent::tool_call_end(
            tool_call_id,
            tool_name,
            output.clone(),
            model_content.clone(),
        ));
    }

    let result = if output.is_error {
        ToolResult::error(tool_call_id.to_string(), model_content)
    } else {
        ToolResult::success(tool_call_id.to_string(), model_content)
    };
    match output.image_data {
        Some(image_data) => result.with_image(image_data, output.image_media_type),
        None => result,
    }
}

fn default_truncated_content(content: &Value, tool_name: &str, config: &SessionConfig) -> Value {
    match content {
        Value::String(output) => Value::String(truncate_tool_output(output, tool_name, config)),
        _ => {
            let model_text = structured_model_text(content);
            let truncated = truncate_tool_output(&model_text, tool_name, config);
            if truncated == model_text {
                content.clone()
            } else {
                Value::String(truncated)
            }
        }
    }
}

fn structured_model_text(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(values) => {
            if values.is_empty() {
                "[]".to_string()
            } else {
                values
                    .iter()
                    .map(structured_model_text)
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
        Value::Object(object) => {
            if object.is_empty() {
                return "{}".to_string();
            }
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            entries
                .into_iter()
                .map(|(key, value)| {
                    let rendered = structured_model_text(value);
                    if rendered.contains('\n') {
                        format!("{key}:\n{rendered}")
                    } else {
                        format!("{key}: {rendered}")
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
    }
}
