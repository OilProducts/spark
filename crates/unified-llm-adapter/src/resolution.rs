use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const RUNTIME_LAUNCH_MODEL_KEY: &str = "_attractor.runtime.launch_model";
pub const RUNTIME_LAUNCH_PROVIDER_KEY: &str = "_attractor.runtime.launch_provider";
pub const RUNTIME_LAUNCH_PROFILE_KEY: &str = "_attractor.runtime.launch_profile";
pub const RUNTIME_LAUNCH_REASONING_EFFORT_KEY: &str = "_attractor.runtime.launch_reasoning_effort";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmResolutionInputs {
    #[serde(default)]
    pub node_model: Option<String>,
    #[serde(default)]
    pub node_provider: Option<String>,
    #[serde(default)]
    pub node_profile: Option<String>,
    #[serde(default)]
    pub node_reasoning_effort: Option<String>,
    #[serde(default)]
    pub node_reasoning_is_default_placeholder: bool,
    #[serde(default)]
    pub fallback_model: Option<String>,
    #[serde(default)]
    pub fallback_provider: Option<String>,
    #[serde(default)]
    pub fallback_profile: Option<String>,
    #[serde(default)]
    pub fallback_reasoning_effort: Option<String>,
}

pub fn resolve_effective_llm_model(
    inputs: &LlmResolutionInputs,
    context: &BTreeMap<String, Value>,
) -> Option<String> {
    let launch_model = context_text(context, RUNTIME_LAUNCH_MODEL_KEY);
    first_text([
        inputs.node_model.as_deref(),
        launch_model.as_deref(),
        inputs.fallback_model.as_deref(),
    ])
}

pub fn resolve_effective_llm_provider(
    inputs: &LlmResolutionInputs,
    context: &BTreeMap<String, Value>,
) -> String {
    let launch_provider = context_text(context, RUNTIME_LAUNCH_PROVIDER_KEY);
    first_text([
        inputs.node_provider.as_deref(),
        launch_provider.as_deref(),
        inputs.fallback_provider.as_deref(),
    ])
    .map(|value| value.to_lowercase())
    .unwrap_or_else(|| "codex".to_string())
}

pub fn resolve_effective_llm_profile(
    inputs: &LlmResolutionInputs,
    context: &BTreeMap<String, Value>,
) -> Option<String> {
    let launch_profile = context_text(context, RUNTIME_LAUNCH_PROFILE_KEY);
    first_text([
        inputs.node_profile.as_deref(),
        launch_profile.as_deref(),
        inputs.fallback_profile.as_deref(),
    ])
}

pub fn resolve_effective_reasoning_effort(
    inputs: &LlmResolutionInputs,
    context: &BTreeMap<String, Value>,
) -> Option<String> {
    let launch_reasoning = context_text(context, RUNTIME_LAUNCH_REASONING_EFFORT_KEY);
    let authored_node_reasoning = (!inputs.node_reasoning_is_default_placeholder)
        .then(|| inputs.node_reasoning_effort.as_deref())
        .flatten();
    first_text([
        authored_node_reasoning,
        launch_reasoning.as_deref(),
        inputs.fallback_reasoning_effort.as_deref(),
    ])
    .map(|value| value.to_lowercase())
}

fn first_text<'a>(values: impl IntoIterator<Item = Option<&'a str>>) -> Option<String> {
    values
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(str::to_string)
}

fn context_text(context: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    let value = context.get(key)?;
    Some(match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(number) => number.to_string(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).ok()?,
    })
}
