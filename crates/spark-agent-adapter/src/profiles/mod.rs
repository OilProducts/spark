use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use unified_llm_adapter::Tool;

use crate::config::SessionConfig;
use crate::session::ExecutionEnvironment;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderProfile {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub system_prompt: String,
    #[serde(default, alias = "tool_registry")]
    pub tools: Vec<Tool>,
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
            model: String::new(),
            system_prompt: String::new(),
            tools: Vec::new(),
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
        self.tools.clone()
    }

    pub fn build_system_prompt(&self, _environment: &ExecutionEnvironment) -> String {
        self.system_prompt.clone()
    }

    pub fn provider_options(&self) -> BTreeMap<String, Value> {
        self.provider_options.clone()
    }

    pub fn request_provider_options(&self, _config: &SessionConfig) -> BTreeMap<String, Value> {
        if self.id.trim().is_empty() {
            return self.provider_options();
        }

        BTreeMap::from([(
            self.id.clone(),
            Value::Object(self.provider_options().into_iter().collect::<Map<_, _>>()),
        )])
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
}

fn normalize_capability_name(capability: &str) -> String {
    let normalized = capability.trim().to_ascii_lowercase().replace('-', "_");
    normalized
        .strip_prefix("supports_")
        .unwrap_or(normalized.as_str())
        .to_string()
}
