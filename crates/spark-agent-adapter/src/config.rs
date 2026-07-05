use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionConfig {
    #[serde(default)]
    pub max_turns: u32,
    #[serde(default)]
    pub max_tool_rounds_per_input: u32,
    #[serde(default = "default_command_timeout_ms")]
    pub default_command_timeout_ms: u64,
    #[serde(default = "default_max_command_timeout_ms")]
    pub max_command_timeout_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub tool_output_limits: BTreeMap<String, u64>,
    #[serde(default)]
    pub line_limits: BTreeMap<String, u64>,
    #[serde(default = "default_enable_loop_detection")]
    pub enable_loop_detection: bool,
    #[serde(default = "default_loop_detection_window")]
    pub loop_detection_window: u32,
    #[serde(default = "default_max_subagent_depth")]
    pub max_subagent_depth: u32,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            max_turns: 0,
            max_tool_rounds_per_input: 0,
            default_command_timeout_ms: default_command_timeout_ms(),
            max_command_timeout_ms: default_max_command_timeout_ms(),
            reasoning_effort: None,
            tool_output_limits: BTreeMap::new(),
            line_limits: BTreeMap::new(),
            enable_loop_detection: default_enable_loop_detection(),
            loop_detection_window: default_loop_detection_window(),
            max_subagent_depth: default_max_subagent_depth(),
        }
    }
}

impl SessionConfig {
    pub fn tool_output_char_limits(&self) -> &BTreeMap<String, u64> {
        &self.tool_output_limits
    }

    pub fn set_tool_output_char_limits(&mut self, value: BTreeMap<String, u64>) {
        self.tool_output_limits = value;
    }

    pub fn tool_line_limits(&self) -> &BTreeMap<String, u64> {
        &self.line_limits
    }

    pub fn set_tool_line_limits(&mut self, value: BTreeMap<String, u64>) {
        self.line_limits = value;
    }
}

fn default_command_timeout_ms() -> u64 {
    10_000
}

fn default_max_command_timeout_ms() -> u64 {
    600_000
}

fn default_enable_loop_detection() -> bool {
    true
}

fn default_loop_detection_window() -> u32 {
    10
}

fn default_max_subagent_depth() -> u32 {
    1
}
