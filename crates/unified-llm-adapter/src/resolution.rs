use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{list_models, ModelInfo};
use crate::errors::{AdapterError, AdapterErrorKind};

pub const RUNTIME_LAUNCH_MODEL_KEY: &str = "_attractor.runtime.launch_model";
pub const RUNTIME_LAUNCH_PROVIDER_KEY: &str = "_attractor.runtime.launch_provider";
pub const RUNTIME_LAUNCH_PROFILE_KEY: &str = "_attractor.runtime.launch_profile";
pub const RUNTIME_LAUNCH_REASONING_EFFORT_KEY: &str = "_attractor.runtime.launch_reasoning_effort";
pub const DISPLAY_MODEL_PLACEHOLDER: &str = "codex default (config/profile)";

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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveLlmProfile {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
}

impl ActiveLlmProfile {
    pub fn new(provider: impl Into<String>, default_model: impl Into<Option<String>>) -> Self {
        Self {
            provider: provider.into(),
            default_model: default_model.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCapabilities {
    #[serde(default)]
    pub tools: bool,
    #[serde(default)]
    pub vision: bool,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub structured_output: bool,
}

impl ModelCapabilities {
    pub fn tools() -> Self {
        Self {
            tools: true,
            ..Self::default()
        }
    }

    pub fn vision() -> Self {
        Self {
            vision: true,
            ..Self::default()
        }
    }

    pub fn reasoning() -> Self {
        Self {
            reasoning: true,
            ..Self::default()
        }
    }

    pub fn structured_output() -> Self {
        Self {
            structured_output: true,
            ..Self::default()
        }
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            tools: self.tools || other.tools,
            vision: self.vision || other.vision,
            reasoning: self.reasoning || other.reasoning,
            structured_output: self.structured_output || other.structured_output,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HighLevelLlmResolutionInputs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_profile: Option<ActiveLlmProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_default_provider: Option<String>,
    #[serde(default)]
    pub required_capabilities: ModelCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedLlmModel {
    pub provider: String,
    pub model: String,
}

pub fn resolve_high_level_provider_and_model(
    inputs: &HighLevelLlmResolutionInputs,
) -> Result<ResolvedLlmModel, AdapterError> {
    let explicit_provider = first_text([inputs.provider.as_deref()]);
    let active_profile_provider = inputs
        .active_profile
        .as_ref()
        .and_then(|profile| first_text([Some(profile.provider.as_str())]));
    let provider = first_text([
        explicit_provider.as_deref(),
        active_profile_provider.as_deref(),
        inputs.client_default_provider.as_deref(),
    ])
    .map(|provider| provider.to_ascii_lowercase())
    .ok_or_else(|| {
        configuration_error("No provider configured; set request.provider, active profile, or Client.default_provider")
    })?;

    let profile_default_model =
        active_profile_default_model_for_provider(inputs.active_profile.as_ref(), &provider);
    let model = first_model_text([inputs.model.as_deref(), profile_default_model.as_deref()])
        .or_else(|| {
            latest_native_model(&provider, inputs.required_capabilities).map(|model| model.id)
        })
        .ok_or_else(|| omitted_model_error(&provider, inputs.required_capabilities))?;

    Ok(ResolvedLlmModel { provider, model })
}

pub fn resolve_effective_llm_model(
    inputs: &LlmResolutionInputs,
    context: &BTreeMap<String, Value>,
) -> Option<String> {
    let launch_model = context_text(context, RUNTIME_LAUNCH_MODEL_KEY);
    first_model_text([
        inputs.node_model.as_deref(),
        launch_model.as_deref(),
        inputs.fallback_model.as_deref(),
    ])
}

pub fn is_display_model_placeholder(value: &str) -> bool {
    value.trim().eq_ignore_ascii_case(DISPLAY_MODEL_PLACEHOLDER)
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

fn first_model_text<'a>(values: impl IntoIterator<Item = Option<&'a str>>) -> Option<String> {
    values
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|value| !value.is_empty() && !is_display_model_placeholder(value))
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

fn active_profile_default_model_for_provider(
    active_profile: Option<&ActiveLlmProfile>,
    provider: &str,
) -> Option<String> {
    let profile = active_profile?;
    let profile_provider = first_text([Some(profile.provider.as_str())])?;
    if profile_provider.to_ascii_lowercase() != provider {
        return None;
    }
    first_text([profile.default_model.as_deref()])
}

fn latest_native_model(provider: &str, capabilities: ModelCapabilities) -> Option<ModelInfo> {
    if !is_native_default_provider(provider) {
        return None;
    }
    list_models(Some(provider))
        .into_iter()
        .find(|model| model_satisfies(model, capabilities))
}

fn model_satisfies(model: &ModelInfo, capabilities: ModelCapabilities) -> bool {
    if capabilities.tools && !model.supports_tools {
        return false;
    }
    if capabilities.vision && !model.supports_vision {
        return false;
    }
    if capabilities.reasoning && !model.supports_reasoning {
        return false;
    }
    if capabilities.structured_output && !model.supports_tools {
        return false;
    }
    true
}

fn omitted_model_error(provider: &str, capabilities: ModelCapabilities) -> AdapterError {
    let detail = if capabilities.tools
        || capabilities.vision
        || capabilities.reasoning
        || capabilities.structured_output
    {
        " with the requested capabilities"
    } else {
        ""
    };
    configuration_error(format!(
        "No model configured for provider {provider:?}{detail}; set request.model or an active profile default_model"
    ))
}

fn is_native_default_provider(provider: &str) -> bool {
    matches!(
        provider.trim().to_ascii_lowercase().as_str(),
        "openai" | "anthropic" | "gemini"
    )
}

fn configuration_error(message: impl Into<String>) -> AdapterError {
    AdapterError::new(AdapterErrorKind::Configuration, message)
}
