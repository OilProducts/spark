use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::error::{Result, SparkCommonError};
use crate::paths::{
    detect_project_root, ensure_writable_directory, normalize_path, Environment, ProcessEnvironment,
};

pub const ENV_HOME_DIR: &str = "SPARK_HOME";
pub const ENV_FLOWS_DIR: &str = "SPARK_FLOWS_DIR";
pub const ENV_UI_DIR: &str = "SPARK_UI_DIR";
pub const ENV_PROJECT_ROOTS: &str = "SPARK_PROJECT_ROOTS";

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionSettings {
    pub server_host: Option<String>,
    pub server_port: Option<u16>,
    pub client_api_base_url: Option<String>,
}

impl ConnectionSettings {
    pub fn validate(&self) -> Result<()> {
        if self.server_host.as_ref().is_some_and(|host| {
            host.is_empty()
                || host != host.trim()
                || host.chars().any(char::is_whitespace)
                || (host.parse::<std::net::IpAddr>().is_err()
                    && !host
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'.'))
        }) {
            return Err(SparkCommonError::SettingsValidation(
                "server_host must be an IP address or hostname.".into(),
            ));
        }
        if let Some(value) = &self.client_api_base_url {
            let valid = url::Url::parse(value).ok().is_some_and(|url| {
                matches!(url.scheme(), "http" | "https")
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
            });
            if !valid {
                return Err(SparkCommonError::SettingsValidation("client_api_base_url must be an HTTP(S) URL without credentials, query or fragment.".into()));
            }
        }
        Ok(())
    }

    pub fn resolve(
        &self,
        env: &impl Environment,
        host: Option<&str>,
        port: Option<u16>,
    ) -> Result<Self> {
        self.validate()?;
        let environment_port = env.get_var("SPARK_PORT");
        let server_port = match port {
            Some(port) => port,
            None => match environment_port.filter(|value| !value.is_empty()) {
                Some(value) => value.parse().map_err(|_| {
                    SparkCommonError::SettingsValidation(
                        "SPARK_PORT must be an integer from 0 to 65535.".into(),
                    )
                })?,
                None => self.server_port.unwrap_or(8000),
            },
        };
        let resolved = Self {
            server_host: Some(
                host.map(str::to_owned)
                    .or_else(|| env.get_var("SPARK_HOST").filter(|value| !value.is_empty()))
                    .or_else(|| self.server_host.clone())
                    .unwrap_or_else(|| "127.0.0.1".into()),
            ),
            server_port: Some(server_port),
            client_api_base_url: Some(
                env.get_var("SPARK_API_BASE_URL")
                    .filter(|value| !value.trim().is_empty())
                    .or_else(|| self.client_api_base_url.clone())
                    .unwrap_or_else(|| "http://127.0.0.1:8000".into()),
            ),
        };
        resolved.validate()?;
        Ok(resolved)
    }
}

/// Client identifiers are document names, never paths or credentials.
pub fn validate_client_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 80
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err(SparkCommonError::SettingsValidation(
            "client_id must contain 1–80 ASCII letters, digits, underscores or hyphens.".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditorMode {
    #[default]
    Structured,
    Raw,
}

/// Reusable presentation choices only; drafts and selected records stay in session state.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientPreferences {
    pub flow_edge_ports: Option<
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, LayoutEdgePorts>>,
    >,
    pub run_presentation: Option<RunPresentation>,
    pub editor_mode: Option<EditorMode>,
    pub editor_sidebar_width: Option<u16>,
    pub home_sidebar_primary_split_ratio: Option<f64>,
    pub show_advanced_controls: Option<bool>,
    pub expand_child_flows: Option<bool>,
    pub graph_settings_open: Option<bool>,
    pub runs_scope: Option<PresentationScope>,
    pub triggers_scope: Option<PresentationScope>,
    pub flow_node_positions: Option<
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, LayoutPosition>>,
    >,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayoutSide {
    Top,
    Right,
    Bottom,
    Left,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutEdgePorts {
    pub source_side: LayoutSide,
    pub target_side: LayoutSide,
    pub source_slot: u32,
    pub target_slot: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunPresentation {
    pub sort: Option<String>,
    pub activity_mode: Option<String>,
    pub inspector_tab: Option<String>,
    pub timeline_category: Option<String>,
    pub timeline_severity: Option<String>,
    pub graph_height: Option<u16>,
}

impl RunPresentation {
    pub fn validate(&self) -> Result<()> {
        for (value, allowed) in [
            (&self.sort, &["newest", "oldest"][..]),
            (&self.activity_mode, &["all", "transcript", "events"][..]),
            (
                &self.inspector_tab,
                &["activity", "result", "details", "context", "artifacts"][..],
            ),
            (
                &self.timeline_category,
                &[
                    "all",
                    "lifecycle",
                    "stage",
                    "parallel",
                    "interview",
                    "checkpoint",
                    "log",
                    "runtime",
                    "state",
                    "metadata",
                    "other",
                ][..],
            ),
            (
                &self.timeline_severity,
                &["all", "info", "warning", "error"][..],
            ),
        ] {
            if value
                .as_ref()
                .is_some_and(|value| !allowed.contains(&value.as_str()))
            {
                return Err(SparkCommonError::SettingsValidation(
                    "Invalid run presentation choice.".into(),
                ));
            }
        }
        if self
            .graph_height
            .is_some_and(|height| !(280..=960).contains(&height))
        {
            return Err(SparkCommonError::SettingsValidation(
                "Run graph height must be between 280 and 960 pixels.".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutPosition {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationScope {
    Active,
    All,
}

impl ClientPreferences {
    pub fn validate(&self) -> Result<()> {
        if let Some(run) = &self.run_presentation {
            run.validate()?;
        }
        if self.flow_node_positions.as_ref().is_some_and(|layouts| {
            layouts
                .values()
                .flat_map(|nodes| nodes.values())
                .any(|position| !position.x.is_finite() || !position.y.is_finite())
        }) {
            return Err(SparkCommonError::SettingsValidation(
                "Layout coordinates must be finite.".into(),
            ));
        }
        if self
            .home_sidebar_primary_split_ratio
            .is_some_and(|ratio| !ratio.is_finite() || !(0.0..=1.0).contains(&ratio))
        {
            return Err(SparkCommonError::SettingsValidation(
                "home_sidebar_primary_split_ratio must be between 0 and 1.".into(),
            ));
        }
        if self
            .editor_sidebar_width
            .is_some_and(|width| !(256..=560).contains(&width))
        {
            return Err(SparkCommonError::SettingsValidation(
                "editor_sidebar_width must be between 256 and 560 pixels.".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopSettings {
    #[serde(default)]
    pub remote_access_enabled: bool,
}

/// Authored runtime choices; bootstrap home and derived directories are not settings.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSettings {
    pub runs_dir: Option<PathBuf>,
    pub flows_dir: Option<PathBuf>,
    pub ui_dir: Option<PathBuf>,
    #[serde(default)]
    pub project_roots: Vec<PathBuf>,
}

impl RuntimeSettings {
    pub fn validate(&self) -> Result<()> {
        if [&self.runs_dir, &self.flows_dir, &self.ui_dir]
            .into_iter()
            .flatten()
            .chain(self.project_roots.iter())
            .any(|path| path.as_os_str().is_empty())
        {
            return Err(SparkCommonError::SettingsValidation(
                "Runtime paths must be nonempty or omitted.".into(),
            ));
        }
        if self
            .project_roots
            .iter()
            .any(|path| !expand_for_absolute_check(path).is_absolute())
        {
            return Err(SparkCommonError::SettingsValidation(
                "Runtime project_roots must contain absolute paths.".into(),
            ));
        }
        Ok(())
    }
}

/// A scope overrides the whole selection, including provider-default omissions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSettings {
    pub provider: Option<String>,
    pub llm_profile: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self {
            provider: Some("codex".into()),
            llm_profile: None,
            model: None,
            reasoning_effort: None,
        }
    }
}

/// Use with `serde(default, deserialize_with)` on nullable patch fields.
/// Serde's ordinary nested Option treats an explicit null as an omission.
pub fn deserialize_nullable_patch<'de, D, T>(
    deserializer: D,
) -> std::result::Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

impl ModelSettings {
    pub fn validate(&self) -> Result<()> {
        if self.provider.is_some() == self.llm_profile.is_some() {
            return Err(SparkCommonError::SettingsValidation(
                "Select exactly one provider or LLM profile.".into(),
            ));
        }
        for value in [
            &self.provider,
            &self.llm_profile,
            &self.model,
            &self.reasoning_effort,
        ]
        .into_iter()
        .flatten()
        {
            if value.trim().is_empty() {
                return Err(SparkCommonError::SettingsValidation(
                    "Model selection fields must be nonempty or omitted.".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSettingsSource {
    Workspace,
    Project,
    Conversation,
}

pub fn resolve_model_settings<'a>(
    workspace: &'a ModelSettings,
    project: Option<&'a ModelSettings>,
    conversation: Option<&'a ModelSettings>,
) -> Result<(&'a ModelSettings, ModelSettingsSource)> {
    workspace.validate()?;
    if let Some(project) = project {
        project.validate()?;
    }
    if let Some(conversation) = conversation {
        conversation.validate()?;
    }
    Ok(if let Some(conversation) = conversation {
        (conversation, ModelSettingsSource::Conversation)
    } else if let Some(project) = project {
        (project, ModelSettingsSource::Project)
    } else {
        (workspace, ModelSettingsSource::Workspace)
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SparkSettings {
    pub connections: ConnectionSettings,
    pub providers: crate::provider_settings::ProviderConnections,
    pub agents: crate::agent_settings::SessionConfig,
    pub project_root: PathBuf,
    pub data_dir: PathBuf,
    pub config_dir: PathBuf,
    pub runtime_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub workspace_dir: PathBuf,
    pub projects_dir: PathBuf,
    pub attractor_dir: PathBuf,
    pub runs_dir: PathBuf,
    pub flows_dir: PathBuf,
    pub ui_dir: Option<PathBuf>,
    pub project_roots: Vec<PathBuf>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SettingsOverrides {
    pub data_dir: Option<PathBuf>,
    pub runs_dir: Option<PathBuf>,
    pub flows_dir: Option<PathBuf>,
    pub ui_dir: Option<PathBuf>,
}

pub fn resolve_settings(overrides: &SettingsOverrides) -> Result<SparkSettings> {
    resolve_settings_with_env(overrides, &ProcessEnvironment)
}

pub fn resolve_settings_with_env(
    overrides: &SettingsOverrides,
    env: &impl Environment,
) -> Result<SparkSettings> {
    resolve_settings_with_persisted(overrides, env, &RuntimeSettings::default())
}

pub fn resolve_settings_with_persisted(
    overrides: &SettingsOverrides,
    env: &impl Environment,
    persisted: &RuntimeSettings,
) -> Result<SparkSettings> {
    persisted.validate()?;
    let project_root = detect_project_root();
    let default_data_dir = PathBuf::from("~").join(".spark");

    let data_dir = coalesce_path(
        overrides.data_dir.as_ref(),
        env.get_var(ENV_HOME_DIR).as_deref(),
        &default_data_dir,
    )?;
    let config_dir = data_dir.join("config");
    let runtime_dir = data_dir.join("runtime");
    let logs_dir = data_dir.join("logs");
    let workspace_dir = data_dir.join("workspace");
    let projects_dir = workspace_dir.join("projects");
    let attractor_dir = data_dir.join("attractor");
    let runs_dir = coalesce_path(
        overrides.runs_dir.as_ref(),
        None,
        persisted
            .runs_dir
            .as_ref()
            .unwrap_or(&attractor_dir.join("runs")),
    )?;
    let flows_dir = coalesce_path(
        overrides.flows_dir.as_ref(),
        env.get_var(ENV_FLOWS_DIR).as_deref(),
        persisted
            .flows_dir
            .as_ref()
            .unwrap_or(&data_dir.join("flows")),
    )?;
    let ui_dir = coalesce_optional_path(
        overrides.ui_dir.as_ref(),
        env.get_var(ENV_UI_DIR).as_deref(),
        persisted.ui_dir.as_ref(),
    )?;
    let project_roots = match env.get_var(ENV_PROJECT_ROOTS) {
        Some(value) => parse_project_roots(Some(&value))?,
        None => persisted
            .project_roots
            .iter()
            .map(normalize_path)
            .collect::<Result<Vec<_>>>()?,
    };

    Ok(SparkSettings {
        connections: ConnectionSettings::default(),
        providers: Default::default(),
        agents: Default::default(),
        project_root,
        data_dir,
        config_dir,
        runtime_dir,
        logs_dir,
        workspace_dir,
        projects_dir,
        attractor_dir,
        runs_dir,
        flows_dir,
        ui_dir,
        project_roots,
    })
}

pub fn validate_settings(settings: &SparkSettings) -> Result<()> {
    ensure_writable_directory(&settings.config_dir, "config")?;
    ensure_writable_directory(&settings.runtime_dir, "runtime")?;
    ensure_writable_directory(&settings.logs_dir, "logs")?;
    ensure_writable_directory(&settings.workspace_dir, "workspace")?;
    ensure_writable_directory(&settings.projects_dir, "projects")?;
    ensure_writable_directory(&settings.attractor_dir, "attractor")?;
    ensure_writable_directory(&settings.runs_dir, "runs")?;
    ensure_writable_directory(&settings.flows_dir, "flows")?;
    if let Some(ui_dir) = settings.ui_dir.as_deref() {
        if !ui_dir.join("index.html").exists() {
            return Err(SparkCommonError::InvalidUiDirectory(ui_dir.to_path_buf()));
        }
    }
    Ok(())
}

pub fn parse_project_roots(value: Option<&str>) -> Result<Vec<PathBuf>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_empty() {
        return Ok(Vec::new());
    }

    let roots = std::env::split_paths(value)
        .filter_map(|entry| {
            let text = entry.to_string_lossy().trim().to_string();
            if text.is_empty() {
                None
            } else {
                Some(PathBuf::from(text))
            }
        })
        .filter(|entry| expand_for_absolute_check(entry).is_absolute())
        .map(normalize_path)
        .collect::<Result<Vec<_>>>()?;
    Ok(roots)
}

fn coalesce_path(
    cli_value: Option<&PathBuf>,
    env_value: Option<&str>,
    default_value: &PathBuf,
) -> Result<PathBuf> {
    if let Some(value) = cli_value {
        return normalize_path(value);
    }
    if let Some(value) = env_value.filter(|value| !value.is_empty()) {
        return normalize_path(value);
    }
    normalize_path(default_value)
}

fn coalesce_optional_path(
    cli_value: Option<&PathBuf>,
    env_value: Option<&str>,
    default_value: Option<&PathBuf>,
) -> Result<Option<PathBuf>> {
    if let Some(value) = cli_value {
        return normalize_path(value).map(Some);
    }
    if let Some(value) = env_value.filter(|value| !value.is_empty()) {
        return normalize_path(value).map(Some);
    }
    default_value.map(normalize_path).transpose()
}

fn expand_for_absolute_check(path: &std::path::Path) -> PathBuf {
    let text = path.to_string_lossy();
    if text == "~" {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| path.to_path_buf());
    }
    if let Some(rest) = text.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            return home.join(rest);
        }
    }
    path.to_path_buf()
}

/// Non-secret resolved settings captured for an execution; profiles accompany this in their domain snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionConfiguration {
    #[serde(default)]
    pub runtime: RuntimeSettings,
    #[serde(default)]
    pub config_dir: PathBuf,
    pub providers: crate::provider_settings::ProviderConnections,
    pub agents: crate::agent_settings::SessionConfig,
}

impl ExecutionConfiguration {
    pub fn retain_startup_settings(&mut self, settings: &SparkSettings) {
        self.config_dir = settings.config_dir.clone();
        self.runtime = RuntimeSettings {
            runs_dir: Some(settings.runs_dir.clone()),
            flows_dir: Some(settings.flows_dir.clone()),
            ui_dir: settings.ui_dir.clone(),
            project_roots: settings.project_roots.clone(),
        };
        self.agents
            .native
            .retain_startup_paths(&settings.agents.native);
    }
}
