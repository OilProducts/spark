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
    /// Registered ACP harnesses by ID, selected as `acp:<id>`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub acp_harnesses: BTreeMap<String, AcpHarness>,
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
            acp_harnesses: BTreeMap::new(),
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
        for (id, harness) in &self.acp_harnesses {
            validate_harness_id(id)
                .and_then(|()| harness.validate())
                .map_err(|error| {
                    crate::SparkCommonError::SettingsValidation(format!(
                        "ACP harness {id:?}: {error}"
                    ))
                })?;
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

/// The provider-selector namespace of registered ACP harnesses.
pub const ACP_SELECTOR_PREFIX: &str = "acp:";
/// Replaced by the exact instruction text.
pub const INSTRUCTIONS_PLACEHOLDER: &str = "{{instructions}}";
/// Replaced by the path of a private file holding the exact instruction text.
pub const INSTRUCTIONS_FILE_PLACEHOLDER: &str = "{{instructions_file}}";
/// Spark's own bindings for every agent process, which no registration overrides.
pub const RESERVED_HARNESS_ENV: [&str; 3] =
    ["SPARK_HOME", "SPARK_API_BASE_URL", "SPARK_PROJECT_WORK_DIR"];

/// Who runs an agent turn: a native harness or a registered ACP harness.
/// Direct API providers are not harnesses.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HarnessIdentity {
    Codex,
    ClaudeCode,
    Acp(String),
}

impl HarnessIdentity {
    /// The harness a provider selector names, normalizing native aliases;
    /// `Ok(None)` for a direct provider. A malformed `acp:` selector is an
    /// error, never a direct provider.
    pub fn parse(selector: &str) -> Result<Option<Self>, String> {
        let selector = selector.trim();
        if selector
            .get(..ACP_SELECTOR_PREFIX.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(ACP_SELECTOR_PREFIX))
        {
            let id = &selector[ACP_SELECTOR_PREFIX.len()..];
            validate_harness_id(id).map_err(|error| format!("Invalid ACP selector: {error}"))?;
            return Ok(Some(Self::Acp(id.to_string())));
        }
        Ok(match selector.to_ascii_lowercase().as_str() {
            "codex" => Some(Self::Codex),
            "claude-code" | "claude_code" => Some(Self::ClaudeCode),
            _ => None,
        })
    }

    /// The canonical provider selector.
    pub fn selector(&self) -> String {
        match self {
            Self::Codex => "codex".into(),
            Self::ClaudeCode => "claude-code".into(),
            Self::Acp(id) => format!("{ACP_SELECTOR_PREFIX}{id}"),
        }
    }

    /// Whether `selector` names an ACP harness, whose reasoning effort IDs
    /// are opaque and kept exactly, never lowercased.
    pub fn is_acp(selector: &str) -> bool {
        matches!(Self::parse(selector), Ok(Some(Self::Acp(_))))
    }

    /// The owner a persisted runtime session is filed under.
    pub fn session_owner(&self) -> String {
        match self {
            Self::Codex => "codex_app_server".into(),
            Self::ClaudeCode => "claude_code_cli".into(),
            Self::Acp(_) => self.selector(),
        }
    }
}

/// Lowercase ASCII letters, digits, `-` and `_`, starting alphanumeric.
pub fn validate_harness_id(id: &str) -> Result<(), String> {
    let mut chars = id.chars();
    let valid = id.len() <= 64
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if valid {
        Ok(())
    } else {
        Err("IDs are 1-64 lowercase ASCII letters, digits, '-' or '_', starting with a letter or digit.".into())
    }
}

/// One registered ACP harness. Its ID (the map key) owns its sessions; the
/// name is only a label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcpHarness {
    pub name: String,
    /// Run directly with `args`, never through a shell.
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Applied over the inherited environment, before Spark's reserved bindings.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub permission_policy: AcpPermissionPolicy,
    /// A system-level channel for pinned instructions, used instead of ACP
    /// `systemPrompt` for instruction-bearing work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_instructions: Option<InstructionChannel>,
}

/// How permission requests are answered: `allow` picks an offered
/// allow-once option, else allow-always; `deny` refuses.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcpPermissionPolicy {
    #[default]
    Allow,
    Deny,
}

/// Where a harness takes system instructions: environment values and extra
/// arguments, templated with the instruction placeholders only, plus the
/// session config selections that make the harness use them.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionChannel {
    #[serde(default)]
    pub env: BTreeMap<String, EnvTemplate>,
    #[serde(default)]
    pub args: Vec<String>,
    /// Required ACP session config selections, by config ID.
    #[serde(default)]
    pub config: BTreeMap<String, String>,
}

/// An environment value: text, or a JSON value whose strings are templated
/// before it is serialized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvTemplate {
    Text(String),
    Json(serde_json::Value),
}

impl AcpHarness {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() || self.name.chars().any(char::is_control) {
            return Err("The name must be nonempty, without control characters.".into());
        }
        if self.executable.trim().is_empty() {
            return Err("The executable must be nonempty.".into());
        }
        if self
            .args
            .iter()
            .chain(self.env.values())
            .chain([&self.executable])
            .any(|value| value.contains('\0'))
        {
            return Err(
                "The executable, arguments and environment values cannot contain NUL.".into(),
            );
        }
        validate_env_keys(self.env.keys())?;
        if let Some(channel) = &self.system_instructions {
            channel.validate()?;
        }
        Ok(())
    }

    /// An opaque token for everything that shapes a launch and its sessions:
    /// command, arguments, environment and what it inherits, policy and
    /// instruction channel, but not the label. It changes whenever any of
    /// them does.
    pub fn generation(&self, inheritance: EnvironmentInheritancePolicy) -> String {
        let Self {
            name: _,
            executable,
            args,
            env,
            permission_policy,
            system_instructions,
        } = self;
        let identity = serde_json::json!([
            executable,
            args,
            env,
            permission_policy,
            system_instructions,
            inheritance
        ]);
        crate::project::sha256_hex(identity.to_string().as_bytes())
    }

    /// This captured registration, with environment values that
    /// [`SessionConfig::redact_acp_env`] replaced taken from `live`, the
    /// current registration, when it is still the one captured.
    pub fn restore_env(
        self,
        live: Option<&AcpHarness>,
        inheritance: EnvironmentInheritancePolicy,
    ) -> Result<Self, String> {
        let channel = self
            .system_instructions
            .iter()
            .flat_map(|channel| channel.env.values().flat_map(EnvTemplate::strings));
        let Some(generation) = self
            .env
            .values()
            .map(String::as_str)
            .chain(channel)
            .find_map(|value| Some(value.split_once(REDACTED_ENV)?.1))
        else {
            return Ok(self);
        };
        live.filter(|live| live.generation(inheritance) == generation)
            .cloned()
            .ok_or_else(|| {
                "the registration this work started with is no longer in the settings, so its environment cannot be restored; start the work again."
                    .into()
            })
    }
}

/// Stands in for an ACP environment value, or the text of an
/// instruction-channel environment template, in the settings Spark shows: a
/// registration saved with it keeps the stored value.
pub const HIDDEN_ENV_VALUE: &str = "spark-hidden-value";

/// Stands in for an ACP environment value or instruction-channel environment
/// template in captured settings, followed by its registration's generation:
/// execution history keeps no values.
const REDACTED_ENV: &str = "spark-redacted:";

impl SessionConfig {
    /// Replaces ACP environment values and instruction-channel environment
    /// templates, which may hold secrets, so this can be persisted;
    /// [`AcpHarness::restore_env`] takes them back from the live registration
    /// with the same generation.
    pub fn redact_acp_env(&mut self) {
        for harness in self.acp_harnesses.values_mut() {
            let stand_in = format!(
                "{REDACTED_ENV}{}",
                harness.generation(self.environment_inheritance)
            );
            for value in harness.env.values_mut() {
                value.clone_from(&stand_in);
            }
            let templates = harness
                .system_instructions
                .iter_mut()
                .flat_map(|channel| channel.env.values_mut());
            for template in templates {
                // The placeholders it used stay, so the captured channel is valid.
                let used = [INSTRUCTIONS_PLACEHOLDER, INSTRUCTIONS_FILE_PLACEHOLDER]
                    .into_iter()
                    .filter(|placeholder| {
                        template
                            .strings()
                            .iter()
                            .any(|text| text.contains(placeholder))
                    })
                    .collect::<String>();
                *template = EnvTemplate::Text(used + &stand_in);
            }
        }
    }
}

impl InstructionChannel {
    fn validate(&self) -> Result<(), String> {
        validate_env_keys(self.env.keys())?;
        let mut referenced = false;
        for template in self.templates() {
            render_instruction_template(template, "", "")?;
            referenced |= template.contains(INSTRUCTIONS_PLACEHOLDER)
                || template.contains(INSTRUCTIONS_FILE_PLACEHOLDER);
        }
        if !referenced {
            return Err(format!(
                "system_instructions must use {INSTRUCTIONS_PLACEHOLDER} or {INSTRUCTIONS_FILE_PLACEHOLDER}."
            ));
        }
        if self
            .config
            .iter()
            .any(|(id, value)| id.trim().is_empty() || value.trim().is_empty())
        {
            return Err("system_instructions config selections must be nonempty.".into());
        }
        Ok(())
    }

    /// Whether rendering needs an instruction file.
    pub fn uses_file(&self) -> bool {
        self.templates()
            .iter()
            .any(|template| template.contains(INSTRUCTIONS_FILE_PLACEHOLDER))
    }

    /// The environment values and extra arguments carrying `instructions`,
    /// with `file` as the instruction file's path.
    #[allow(clippy::type_complexity)]
    pub fn render(
        &self,
        instructions: &str,
        file: &str,
    ) -> Result<(Vec<(String, String)>, Vec<String>), String> {
        let env = self
            .env
            .iter()
            .map(|(key, template)| {
                let value = match template {
                    EnvTemplate::Text(text) => {
                        render_instruction_template(text, instructions, file)?
                    }
                    EnvTemplate::Json(value) => {
                        render_json(value, instructions, file).map(|value| value.to_string())?
                    }
                };
                Ok((key.clone(), value))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let args = self
            .args
            .iter()
            .map(|arg| render_instruction_template(arg, instructions, file))
            .collect::<Result<Vec<_>, _>>()?;
        if env
            .iter()
            .map(|(_, value)| value)
            .chain(&args)
            .any(|value| value.contains('\0'))
        {
            return Err("Instructions containing NUL cannot be passed through the environment or arguments.".into());
        }
        Ok((env, args))
    }

    fn templates(&self) -> Vec<&str> {
        self.args
            .iter()
            .map(String::as_str)
            .chain(self.env.values().flat_map(EnvTemplate::strings))
            .collect()
    }
}

impl EnvTemplate {
    /// Its strings, each a template.
    fn strings(&self) -> Vec<&str> {
        fn walk<'a>(value: &'a serde_json::Value, out: &mut Vec<&'a str>) {
            match value {
                serde_json::Value::String(text) => out.push(text),
                serde_json::Value::Array(items) => items.iter().for_each(|item| walk(item, out)),
                serde_json::Value::Object(map) => map.values().for_each(|item| walk(item, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        match self {
            Self::Text(text) => out.push(text.as_str()),
            Self::Json(value) => walk(value, &mut out),
        }
        out
    }
}

fn render_json(
    value: &serde_json::Value,
    instructions: &str,
    file: &str,
) -> Result<serde_json::Value, String> {
    use serde_json::Value;
    Ok(match value {
        Value::String(text) => {
            Value::String(render_instruction_template(text, instructions, file)?)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| render_json(item, instructions, file))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| Ok((key.clone(), render_json(item, instructions, file)?)))
                .collect::<Result<_, String>>()?,
        ),
        other => other.clone(),
    })
}

/// Substitutes the two instruction placeholders in one pass, so text they
/// insert is never substituted again. Any other `{{` is an unknown placeholder.
pub fn render_instruction_template(
    template: &str,
    instructions: &str,
    file: &str,
) -> Result<String, String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(at) = rest.find("{{") {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        rest = if let Some(after) = tail.strip_prefix(INSTRUCTIONS_PLACEHOLDER) {
            out.push_str(instructions);
            after
        } else if let Some(after) = tail.strip_prefix(INSTRUCTIONS_FILE_PLACEHOLDER) {
            out.push_str(file);
            after
        } else {
            return Err(format!(
                "Unknown placeholder; only {INSTRUCTIONS_PLACEHOLDER} and {INSTRUCTIONS_FILE_PLACEHOLDER} are substituted."
            ));
        };
    }
    out.push_str(rest);
    Ok(out)
}

fn validate_env_keys<'a>(mut keys: impl Iterator<Item = &'a String>) -> Result<(), String> {
    keys.try_for_each(|key| {
        if key.is_empty() || key.contains(['=', '\0']) {
            Err("Environment names must be nonempty, without '=' or NUL.".into())
        } else if RESERVED_HARNESS_ENV.contains(&key.as_str()) {
            Err(format!("{key} is bound by Spark and cannot be overridden."))
        } else {
            Ok(())
        }
    })
}
