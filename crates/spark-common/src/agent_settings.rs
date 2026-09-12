use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionConfig {
    #[serde(default)]
    pub native: NativeAgentSettings,
    #[serde(default)]
    pub environment_inheritance: EnvironmentInheritancePolicy,
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
            native: NativeAgentSettings::default(),
            environment_inheritance: EnvironmentInheritancePolicy::default(),
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

impl SessionConfig {
    pub fn validate(&self) -> crate::Result<()> {
        self.native.validate()?;
        if self
            .tool_output_limits
            .keys()
            .chain(self.line_limits.keys())
            .any(|key| key.trim().is_empty() || key.chars().any(char::is_control))
        {
            return Err(crate::SparkCommonError::SettingsValidation(
                "Tool limit names must be nonempty and contain no control characters.".into(),
            ));
        }
        if self.default_command_timeout_ms == 0
            || self.max_command_timeout_ms < self.default_command_timeout_ms
            || (self.enable_loop_detection && self.loop_detection_window == 0)
        {
            return Err(crate::SparkCommonError::SettingsValidation(
                "Agent timeouts must be positive with default <= maximum; enabled loop detection requires a positive window.".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentInheritancePolicy {
    InheritAll,
    InheritNone,
    #[default]
    InheritCoreOnly,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeAgentSettings {
    pub codex_binary: Option<String>,
    pub codex_runtime_root: Option<String>,
    pub codex_seed_dir: Option<String>,
    pub claude_binary: Option<String>,
    pub claude_config_dir: Option<String>,
    pub claude_permission_mode: Option<String>,
    pub codex_jsonrpc_trace: Option<bool>,
    pub agent_trace: Option<bool>,
}

impl NativeAgentSettings {
    pub fn validate(&self) -> crate::Result<()> {
        if [
            &self.codex_binary,
            &self.codex_runtime_root,
            &self.codex_seed_dir,
            &self.claude_binary,
            &self.claude_config_dir,
        ]
        .into_iter()
        .flatten()
        .any(|value| value.trim().is_empty() || value.contains('\0'))
        {
            return Err(crate::SparkCommonError::SettingsValidation(
                "Agent binary and home paths must be nonempty or omitted.".into(),
            ));
        }
        if self.claude_permission_mode.as_deref().is_some_and(|mode| {
            ![
                "default",
                "acceptEdits",
                "bypassPermissions",
                "plan",
                "dontAsk",
                "auto",
            ]
            .contains(&mode)
        }) {
            return Err(crate::SparkCommonError::SettingsValidation(
                "Unsupported Claude permission mode.".into(),
            ));
        }
        Ok(())
    }

    pub fn resolve(&self, env: &impl crate::paths::Environment) -> crate::Result<Self> {
        self.validate()?;
        let text = |key: &str, stored: &Option<String>| {
            env.get_var(key)
                .filter(|value| !value.trim().is_empty())
                .or_else(|| stored.clone())
        };
        let flag = |key: &str, stored: Option<bool>| {
            env.get_var(key)
                .map(|value| crate::debug::is_truthy_env_value(&value))
                .or(stored)
                .or(Some(false))
        };
        let resolved = Self {
            codex_binary: text("SPARK_CODEX_APP_SERVER_BIN", &self.codex_binary),
            codex_runtime_root: text("ATTRACTOR_CODEX_RUNTIME_ROOT", &self.codex_runtime_root),
            codex_seed_dir: text("ATTRACTOR_CODEX_SEED_DIR", &self.codex_seed_dir),
            claude_binary: text("SPARK_CLAUDE_CODE_BIN", &self.claude_binary),
            claude_config_dir: text("SPARK_CLAUDE_CODE_CONFIG_DIR", &self.claude_config_dir),
            claude_permission_mode: text(
                "SPARK_CLAUDE_CODE_PERMISSION_MODE",
                &self.claude_permission_mode,
            )
            .or_else(|| Some("bypassPermissions".into())),
            codex_jsonrpc_trace: flag("SPARK_DEBUG_CODEX_JSONRPC", self.codex_jsonrpc_trace),
            agent_trace: flag("SPARK_DEBUG_AGENT_TRACE", self.agent_trace),
        };
        resolved.validate()?;
        Ok(resolved)
    }

    pub fn retain_startup_paths(&mut self, startup: &Self) {
        self.codex_runtime_root = startup.codex_runtime_root.clone();
        self.codex_seed_dir = startup.codex_seed_dir.clone();
        self.claude_config_dir = startup.claude_config_dir.clone();
    }
}
