use std::path::PathBuf;

use crate::error::{Result, SparkCommonError};
use crate::paths::{
    detect_project_root, ensure_writable_directory, normalize_path, Environment, ProcessEnvironment,
};

pub const ENV_HOME_DIR: &str = "SPARK_HOME";
pub const ENV_FLOWS_DIR: &str = "SPARK_FLOWS_DIR";
pub const ENV_UI_DIR: &str = "SPARK_UI_DIR";
pub const ENV_PROJECT_ROOTS: &str = "SPARK_PROJECT_ROOTS";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SparkSettings {
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
        &attractor_dir.join("runs"),
    )?;
    let flows_dir = coalesce_path(
        overrides.flows_dir.as_ref(),
        env.get_var(ENV_FLOWS_DIR).as_deref(),
        &data_dir.join("flows"),
    )?;
    let ui_dir = coalesce_optional_path(
        overrides.ui_dir.as_ref(),
        env.get_var(ENV_UI_DIR).as_deref(),
        None,
    )?;
    let project_roots = parse_project_roots(env.get_var(ENV_PROJECT_ROOTS).as_deref())?;

    Ok(SparkSettings {
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
