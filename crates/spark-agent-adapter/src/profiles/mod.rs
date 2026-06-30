use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use unified_llm_adapter::{get_model_info, ModelInfo, Tool};

use crate::config::SessionConfig;
use crate::context;
use crate::environment::ExecutionEnvironment;
use crate::tools::ToolRegistry;

pub mod anthropic;
pub mod gemini;
pub mod openai;

pub use anthropic::{build_anthropic_tool_registry, create_anthropic_profile};
pub use gemini::{
    build_gemini_tool_registry, create_gemini_profile, create_gemini_profile_with_options,
    GeminiProfileOptions,
};
pub use openai::{
    build_openai_compatible_tool_registry, build_openai_tool_registry,
    create_openai_compatible_profile, create_openai_profile, create_provider_profile,
    normalize_provider_selector, NormalizedProviderSelector, ProviderFamily,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderProfile {
    #[serde(default)]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_provider: Option<String>,
    #[serde(default)]
    pub model: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subagent_model_overrides: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub system_prompt: String,
    #[serde(default)]
    pub tools: Vec<Tool>,
    #[serde(default, skip)]
    pub tool_registry: ToolRegistry,
    #[serde(default)]
    pub capabilities: BTreeMap<String, bool>,
    #[serde(default, alias = "provider_options_map")]
    pub provider_options: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_cutoff: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_cutoff_date: Option<String>,
    #[serde(default)]
    pub supports_reasoning: bool,
    #[serde(default)]
    pub supports_streaming: bool,
    #[serde(default)]
    pub supports_parallel_tool_calls: bool,
}

impl Default for ProviderProfile {
    fn default() -> Self {
        Self {
            id: String::new(),
            request_provider: None,
            model: String::new(),
            subagent_model_overrides: Vec::new(),
            system_prompt: String::new(),
            tools: Vec::new(),
            tool_registry: ToolRegistry::new(),
            capabilities: BTreeMap::new(),
            provider_options: BTreeMap::new(),
            context_window_size: None,
            display_name: None,
            knowledge_cutoff: None,
            knowledge_cutoff_date: None,
            supports_reasoning: false,
            supports_streaming: false,
            supports_parallel_tool_calls: false,
        }
    }
}

impl ProviderProfile {
    pub fn new(id: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            model: model.into(),
            ..Self::default()
        }
    }

    pub fn tool_definitions(&self) -> Vec<Tool> {
        self.tools()
    }

    pub fn build_system_prompt(&self, environment: &ExecutionEnvironment) -> String {
        context::build_system_prompt(self, environment)
    }

    pub fn build_system_prompt_with_user_overrides(
        &self,
        environment: &ExecutionEnvironment,
        user_overrides: Option<&str>,
    ) -> String {
        context::build_system_prompt_with_user_overrides(self, environment, user_overrides)
    }

    pub fn tools(&self) -> Vec<Tool> {
        if self.tool_registry.is_empty() {
            return self.tools.clone();
        }
        self.tool_registry.llm_definitions()
    }

    pub fn registry(&self) -> ToolRegistry {
        if self.tool_registry.is_empty() {
            return ToolRegistry::from_tools(self.tools.clone());
        }
        self.tool_registry.clone()
    }

    pub fn set_tool_registry(&mut self, tool_registry: ToolRegistry) {
        self.tools = tool_registry.llm_definitions();
        self.tool_registry = tool_registry;
    }

    pub fn register_tool(&mut self, tool: Tool) -> Option<crate::tools::RegisteredTool> {
        if self.tool_registry.is_empty() && !self.tools.is_empty() {
            self.tool_registry = ToolRegistry::from_tools(self.tools.clone());
        }
        let replaced = self.tool_registry.register(tool);
        self.tools = self.tool_registry.llm_definitions();
        replaced
    }

    pub fn provider_options(&self) -> BTreeMap<String, Value> {
        self.provider_options.clone()
    }

    pub fn provider_options_for_config(&self, config: &SessionConfig) -> BTreeMap<String, Value> {
        let mut options = self.provider_options();
        let Some(reasoning_effort) = config.reasoning_effort.as_ref() else {
            return options;
        };

        match self.id.as_str() {
            "openai" => {
                merge_option_object(
                    &mut options,
                    "reasoning",
                    [("effort", Value::String(reasoning_effort.clone()))],
                );
            }
            "anthropic" => {
                merge_option_object(
                    &mut options,
                    "output_config",
                    [("effort", Value::String(reasoning_effort.clone()))],
                );
            }
            "gemini" => {
                merge_gemini_thinking_level(&mut options, reasoning_effort);
            }
            _ => {}
        }
        options
    }

    pub fn request_provider_options(&self, config: &SessionConfig) -> BTreeMap<String, Value> {
        if self.id.trim().is_empty() {
            return self.provider_options_for_config(config);
        }

        BTreeMap::from([(
            self.id.clone(),
            Value::Object(
                self.provider_options_for_config(config)
                    .into_iter()
                    .collect::<Map<_, _>>(),
            ),
        )])
    }

    pub fn request_provider_id(&self) -> Option<String> {
        self.request_provider
            .as_deref()
            .and_then(non_empty)
            .or_else(|| non_empty(&self.id))
            .map(str::to_string)
    }

    pub fn allows_subagent_model_override(&self, requested_model: &str) -> bool {
        let Some(requested_model) = non_empty(requested_model) else {
            return false;
        };
        if non_empty(&self.model) == Some(requested_model) {
            return true;
        }
        if self.capability_enabled("subagent_model_override") {
            return true;
        }
        self.subagent_model_overrides
            .iter()
            .filter_map(|model| non_empty(model))
            .any(|model| model == "*" || model == requested_model)
    }

    pub fn allowed_subagent_model_overrides(&self) -> Vec<String> {
        self.subagent_model_overrides
            .iter()
            .filter_map(|model| non_empty(model).map(str::to_string))
            .collect()
    }

    pub fn supports(&self, capability: impl AsRef<str>) -> bool {
        let normalized = normalize_capability_name(capability.as_ref());
        match normalized.as_str() {
            "reasoning" => self.supports_reasoning || self.capability_enabled("reasoning"),
            "streaming" => self.supports_streaming || self.capability_enabled("streaming"),
            "parallel_tool_calls" => {
                self.supports_parallel_tool_calls || self.capability_enabled("parallel_tool_calls")
            }
            _ => self.capability_enabled(&normalized),
        }
    }

    pub fn capability_flags(&self) -> &BTreeMap<String, bool> {
        &self.capabilities
    }

    pub fn set_capability_flags(&mut self, value: BTreeMap<String, bool>) {
        self.capabilities = value;
    }

    fn capability_enabled(&self, capability: &str) -> bool {
        self.capabilities.get(capability).copied().unwrap_or(false)
    }

    pub(crate) fn apply_model_metadata(&mut self) {
        let Some(model_info) = get_model_info(&self.model) else {
            return;
        };
        apply_model_info(self, &model_info);
    }
}

fn merge_option_object<const N: usize>(
    options: &mut BTreeMap<String, Value>,
    key: &str,
    values: [(&str, Value); N],
) {
    let mut object = match options.remove(key) {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    for (field, value) in values {
        object.insert(field.to_string(), value);
    }
    options.insert(key.to_string(), Value::Object(object));
}

fn merge_gemini_thinking_level(options: &mut BTreeMap<String, Value>, reasoning_effort: &str) {
    let mut object = match options.remove("thinkingConfig") {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    if !object.contains_key("thinkingBudget") {
        object.insert(
            "thinkingLevel".to_string(),
            Value::String(reasoning_effort.to_string()),
        );
    }
    options.insert("thinkingConfig".to_string(), Value::Object(object));
}

fn non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn normalize_capability_name(capability: &str) -> String {
    let normalized = capability.trim().to_ascii_lowercase().replace('-', "_");
    normalized
        .strip_prefix("supports_")
        .unwrap_or(normalized.as_str())
        .to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InstructionProviderFamily {
    OpenAI,
    Anthropic,
    Gemini,
}

pub(crate) fn instruction_provider_family(
    profile: &ProviderProfile,
) -> Option<InstructionProviderFamily> {
    for selector in [
        non_empty(&profile.id),
        profile.request_provider.as_deref().and_then(non_empty),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(family) = instruction_family_from_provider_selector(selector) {
            return Some(family);
        }
    }

    if let Some(model_info) = get_model_info(&profile.model) {
        return instruction_family_from_text(&model_info.provider);
    }

    let haystack = [
        profile.id.as_str(),
        profile.model.as_str(),
        profile.display_name.as_deref().unwrap_or_default(),
    ]
    .join(" ")
    .to_ascii_lowercase();
    if haystack.contains("anthropic") || haystack.contains("claude") {
        return Some(InstructionProviderFamily::Anthropic);
    }
    if haystack.contains("gemini") {
        return Some(InstructionProviderFamily::Gemini);
    }
    let model = profile.model.to_ascii_lowercase();
    if haystack.contains("openai")
        || model.starts_with("gpt-")
        || model.starts_with("o1")
        || model.starts_with("o3")
    {
        return Some(InstructionProviderFamily::OpenAI);
    }
    None
}

fn instruction_family_from_provider_selector(value: &str) -> Option<InstructionProviderFamily> {
    match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "openai" | "openrouter" | "litellm" | "codex" | "openai_compatible" | "compatible" => {
            Some(InstructionProviderFamily::OpenAI)
        }
        "anthropic" | "claude" | "claude_code" => Some(InstructionProviderFamily::Anthropic),
        "gemini" | "google" | "google_gemini" => Some(InstructionProviderFamily::Gemini),
        _ => None,
    }
}

fn instruction_family_from_text(value: &str) -> Option<InstructionProviderFamily> {
    match value.trim().to_ascii_lowercase().as_str() {
        "openai" => Some(InstructionProviderFamily::OpenAI),
        "anthropic" => Some(InstructionProviderFamily::Anthropic),
        "gemini" | "google" => Some(InstructionProviderFamily::Gemini),
        _ => None,
    }
}

pub(crate) fn apply_model_info(profile: &mut ProviderProfile, model_info: &ModelInfo) {
    if profile.display_name.is_none() {
        profile.display_name = Some(model_info.display_name.clone());
    }
    if profile.context_window_size.is_none() {
        profile.context_window_size = model_info
            .context_window
            .and_then(|value| u64::try_from(value).ok());
    }
    profile
        .capabilities
        .entry("reasoning".to_string())
        .or_insert(model_info.supports_reasoning);
    profile
        .capabilities
        .entry("vision".to_string())
        .or_insert(model_info.supports_vision);
    profile.supports_reasoning =
        profile.supports_reasoning || profile.capability_enabled("reasoning");
}
