use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use spark_common::settings::SparkSettings;
use toml::value::Table;

use crate::errors::{
    ExecutionProfileConfigError, ExecutionProfileFieldError, ExecutionProfileSelectionError,
};
use crate::modes::{normalize_execution_mode, ExecutionMode};

pub const EXECUTION_PROFILES_FILENAME: &str = "execution-profiles.toml";
pub const IMPLEMENTATION_NATIVE_PROFILE_ID: &str = "native";

pub trait ExecutionProfileSettings {
    fn config_dir(&self) -> &Path;
}

impl ExecutionProfileSettings for SparkSettings {
    fn config_dir(&self) -> &Path {
        &self.config_dir
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionProfileConfigRoot {
    pub config_dir: PathBuf,
}

impl ExecutionProfileConfigRoot {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }
}

impl ExecutionProfileSettings for ExecutionProfileConfigRoot {
    fn config_dir(&self) -> &Path {
        &self.config_dir
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionProfile {
    pub id: String,
    pub label: String,
    pub mode: ExecutionMode,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

impl ExecutionProfile {
    pub fn implementation_native() -> Self {
        Self {
            id: IMPLEMENTATION_NATIVE_PROFILE_ID.to_string(),
            label: "Native".to_string(),
            mode: ExecutionMode::Native,
            enabled: true,
            image: None,
            capabilities: Vec::new(),
            metadata: BTreeMap::new(),
        }
    }

    pub fn is_native(&self) -> bool {
        self.mode == ExecutionMode::Native
    }

    pub fn is_container(&self) -> bool {
        self.mode == ExecutionMode::LocalContainer
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExecutionProfileGraph {
    #[serde(default)]
    pub profiles: BTreeMap<String, ExecutionProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_execution_profile_id: Option<String>,
    #[serde(default)]
    pub synthesized_native_default: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionProfileSelection {
    pub profile: ExecutionProfile,
    pub selected_profile_id: String,
    pub selection_source: String,
}

pub fn load_execution_profile_config(
    settings: &impl ExecutionProfileSettings,
    explicit_profile_id: Option<&str>,
    project_default_profile_id: Option<&str>,
    spark_default_profile_id: Option<&str>,
) -> Result<ExecutionProfileGraph, ExecutionProfileConfigError> {
    let selected_profile_id = first_profile_id([
        explicit_profile_id,
        project_default_profile_id,
        spark_default_profile_id,
    ]);
    let config_path = settings.config_dir().join(EXECUTION_PROFILES_FILENAME);
    if !config_path.exists() {
        if let Some(selected_profile_id) = selected_profile_id {
            return Err(ExecutionProfileConfigError::new(format!(
                "execution profile '{selected_profile_id}' was selected, but {EXECUTION_PROFILES_FILENAME} does not exist"
            )));
        }
        return Ok(ExecutionProfileGraph {
            profiles: BTreeMap::from([(
                IMPLEMENTATION_NATIVE_PROFILE_ID.to_string(),
                ExecutionProfile::implementation_native(),
            )]),
            default_execution_profile_id: None,
            synthesized_native_default: true,
        });
    }

    let raw_text = fs::read_to_string(&config_path).map_err(|source| {
        ExecutionProfileConfigError::with_source(
            format!("cannot read {EXECUTION_PROFILES_FILENAME}: {source}"),
            source,
        )
    })?;
    let raw = raw_text.parse::<toml::Value>().map_err(|source| {
        ExecutionProfileConfigError::with_source(
            format!("invalid {EXECUTION_PROFILES_FILENAME}: {source}"),
            source,
        )
    })?;
    let table = raw.as_table().cloned().unwrap_or_default();
    let graph = normalize_graph(&table)?;
    let selected_profile_id = first_profile_id([
        explicit_profile_id,
        project_default_profile_id,
        spark_default_profile_id,
        graph.default_execution_profile_id.as_deref(),
    ]);
    if let Some(selected_profile_id) = selected_profile_id {
        validate_selected_profile(&graph, &selected_profile_id)
            .map_err(|error| ExecutionProfileConfigError::new(error.message))?;
    }
    Ok(graph)
}

pub fn resolve_execution_profile_by_id(
    settings: &impl ExecutionProfileSettings,
    explicit_profile_id: Option<&str>,
    project_default_profile_id: Option<&str>,
    spark_default_profile_id: Option<&str>,
) -> Result<ExecutionProfileSelection, ExecutionProfileSelectionError> {
    let graph = load_execution_profile_config(
        settings,
        explicit_profile_id,
        project_default_profile_id,
        spark_default_profile_id,
    )
    .map_err(|error| ExecutionProfileSelectionError::new(error.message))?;
    let (selected_profile_id, selection_source) = selected_profile_id(
        explicit_profile_id,
        project_default_profile_id,
        spark_default_profile_id,
        graph.default_execution_profile_id.as_deref(),
    );
    let Some(selected_profile_id) = selected_profile_id else {
        let profile = ExecutionProfile::implementation_native();
        return Ok(ExecutionProfileSelection {
            selected_profile_id: profile.id.clone(),
            profile,
            selection_source,
        });
    };

    let profile = graph
        .profiles
        .get(&selected_profile_id)
        .cloned()
        .ok_or_else(|| {
            ExecutionProfileSelectionError::new(format!(
                "selected execution profile '{selected_profile_id}' does not exist"
            ))
        })?;
    if !profile.enabled {
        return Err(ExecutionProfileSelectionError::new(format!(
            "selected execution profile '{selected_profile_id}' is disabled"
        )));
    }
    Ok(ExecutionProfileSelection {
        selected_profile_id,
        profile,
        selection_source,
    })
}

fn normalize_graph(raw: &Table) -> Result<ExecutionProfileGraph, ExecutionProfileConfigError> {
    let mut field_errors = Vec::new();
    let defaults = table_field(raw, "defaults", &mut field_errors);
    let profiles = table_field(raw, "profiles", &mut field_errors);
    let default_execution_profile_id =
        load_default_execution_profile_id(defaults.as_ref(), &mut field_errors);
    let profiles = load_profiles(profiles.as_ref(), &mut field_errors);
    if !field_errors.is_empty() {
        return Err(ExecutionProfileConfigError::with_field_errors(field_errors));
    }
    Ok(ExecutionProfileGraph {
        profiles,
        default_execution_profile_id,
        synthesized_native_default: false,
    })
}

fn table_field(
    raw: &Table,
    key: &str,
    field_errors: &mut Vec<ExecutionProfileFieldError>,
) -> Option<Table> {
    match raw.get(key) {
        None => Some(Table::new()),
        Some(value) => value.as_table().cloned().or_else(|| {
            field_errors.push(ExecutionProfileFieldError::new(
                key,
                format!("{key} must be a table"),
            ));
            Some(Table::new())
        }),
    }
}

fn load_default_execution_profile_id(
    raw: Option<&Table>,
    field_errors: &mut Vec<ExecutionProfileFieldError>,
) -> Option<String> {
    let Some(raw) = raw else {
        return None;
    };
    let Some(value) = raw.get("execution_profile_id") else {
        return None;
    };
    if let Some(value) = optional_string(value) {
        return Some(value);
    }
    field_errors.push(ExecutionProfileFieldError::new(
        "defaults.execution_profile_id",
        "execution_profile_id must be a non-empty string",
    ));
    None
}

fn load_profiles(
    raw: Option<&Table>,
    field_errors: &mut Vec<ExecutionProfileFieldError>,
) -> BTreeMap<String, ExecutionProfile> {
    let mut profiles = BTreeMap::new();
    let Some(raw) = raw else {
        return profiles;
    };
    for (profile_id, raw_profile) in raw {
        let normalized_id = normalize_text(profile_id);
        if normalized_id.is_empty() {
            field_errors.push(ExecutionProfileFieldError::new(
                "profiles.<id>",
                "profile id must be non-empty",
            ));
            continue;
        }
        let Some(raw_profile) = raw_profile.as_table() else {
            field_errors.push(ExecutionProfileFieldError::for_profile(
                normalized_id.clone(),
                format!("profiles.{normalized_id}"),
                "profile must be a table",
            ));
            continue;
        };
        let start_error_count = field_errors.len();
        let mode = profile_mode(raw_profile, &normalized_id, field_errors);
        let enabled = optional_bool(raw_profile, &normalized_id, field_errors);
        let label = required_profile_text(raw_profile, "label", &normalized_id, field_errors);
        let capabilities = optional_capabilities(raw_profile, &normalized_id, field_errors);
        let image = optional_profile_text(raw_profile, "image", &normalized_id, field_errors);

        if mode == Some(ExecutionMode::LocalContainer) && image.is_none() {
            profile_error(
                field_errors,
                &normalized_id,
                "image",
                "image is required for local_container profiles",
            );
        }

        if field_errors.len() != start_error_count {
            continue;
        }
        profiles.insert(
            normalized_id.clone(),
            ExecutionProfile {
                id: normalized_id,
                label: label.unwrap_or_default(),
                mode: mode.unwrap_or(ExecutionMode::Native),
                enabled: enabled.unwrap_or(true),
                image,
                capabilities: capabilities.unwrap_or_default(),
                metadata: optional_metadata(raw_profile.get("metadata")),
            },
        );
    }
    profiles
}

fn profile_mode(
    raw_profile: &Table,
    profile_id: &str,
    field_errors: &mut Vec<ExecutionProfileFieldError>,
) -> Option<ExecutionMode> {
    let Some(raw_mode) = raw_profile.get("mode") else {
        profile_error(field_errors, profile_id, "mode", "mode is required");
        return None;
    };
    let text = raw_mode
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| raw_mode.to_string());
    match normalize_execution_mode(text) {
        Ok(mode) => Some(mode),
        Err(message) => {
            profile_error(field_errors, profile_id, "mode", message);
            None
        }
    }
}

fn optional_bool(
    raw_profile: &Table,
    profile_id: &str,
    field_errors: &mut Vec<ExecutionProfileFieldError>,
) -> Option<bool> {
    let Some(value) = raw_profile.get("enabled") else {
        return Some(true);
    };
    if let Some(value) = value.as_bool() {
        return Some(value);
    }
    profile_error(
        field_errors,
        profile_id,
        "enabled",
        "enabled must be a boolean",
    );
    None
}

fn required_profile_text(
    raw_profile: &Table,
    key: &str,
    profile_id: &str,
    field_errors: &mut Vec<ExecutionProfileFieldError>,
) -> Option<String> {
    let Some(value) = raw_profile.get(key) else {
        profile_error(field_errors, profile_id, key, format!("{key} is required"));
        return None;
    };
    if let Some(value) = optional_string(value) {
        return Some(value);
    }
    profile_error(
        field_errors,
        profile_id,
        key,
        format!("{key} must be a non-empty string"),
    );
    None
}

fn optional_profile_text(
    raw_profile: &Table,
    key: &str,
    profile_id: &str,
    field_errors: &mut Vec<ExecutionProfileFieldError>,
) -> Option<String> {
    let Some(value) = raw_profile.get(key) else {
        return None;
    };
    let Some(text) = value.as_str() else {
        profile_error(
            field_errors,
            profile_id,
            key,
            format!("{key} must be a string"),
        );
        return None;
    };
    normalize_optional_text(text)
}

fn optional_capabilities(
    raw_profile: &Table,
    profile_id: &str,
    field_errors: &mut Vec<ExecutionProfileFieldError>,
) -> Option<Vec<String>> {
    let Some(value) = raw_profile.get("capabilities") else {
        return Some(Vec::new());
    };
    let Some(items) = value.as_array() else {
        profile_error(
            field_errors,
            profile_id,
            "capabilities",
            "capabilities must be an array of non-empty strings",
        );
        return None;
    };
    let mut capabilities = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let Some(value) = optional_string(item) else {
            field_errors.push(ExecutionProfileFieldError::for_profile(
                profile_id.to_string(),
                format!("profiles.{profile_id}.capabilities[{index}]"),
                "capability must be a non-empty string",
            ));
            return None;
        };
        capabilities.push(value);
    }
    Some(capabilities)
}

fn optional_metadata(value: Option<&toml::Value>) -> BTreeMap<String, Value> {
    value
        .and_then(toml::Value::as_table)
        .map(|table| {
            table
                .iter()
                .map(|(key, value)| (key.clone(), toml_value_to_json(value)))
                .collect()
        })
        .unwrap_or_default()
}

fn toml_value_to_json(value: &toml::Value) -> Value {
    match value {
        toml::Value::String(value) => Value::String(value.clone()),
        toml::Value::Integer(value) => Value::Number((*value).into()),
        toml::Value::Float(value) => serde_json::Number::from_f64(*value)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        toml::Value::Boolean(value) => Value::Bool(*value),
        toml::Value::Datetime(value) => Value::String(value.to_string()),
        toml::Value::Array(items) => Value::Array(items.iter().map(toml_value_to_json).collect()),
        toml::Value::Table(items) => Value::Object(
            items
                .iter()
                .map(|(key, value)| (key.clone(), toml_value_to_json(value)))
                .collect(),
        ),
    }
}

fn validate_selected_profile(
    graph: &ExecutionProfileGraph,
    profile_id: &str,
) -> Result<(), ExecutionProfileSelectionError> {
    let profile = graph.profiles.get(profile_id).ok_or_else(|| {
        ExecutionProfileSelectionError::new(format!(
            "selected execution profile '{profile_id}' does not exist"
        ))
    })?;
    if !profile.enabled {
        return Err(ExecutionProfileSelectionError::new(format!(
            "selected execution profile '{profile_id}' is disabled"
        )));
    }
    Ok(())
}

fn selected_profile_id(
    explicit_profile_id: Option<&str>,
    project_default_profile_id: Option<&str>,
    spark_default_profile_id: Option<&str>,
    graph_default_profile_id: Option<&str>,
) -> (Option<String>, String) {
    for (source, value) in [
        ("explicit", explicit_profile_id),
        ("project_default", project_default_profile_id),
        ("spark_default", spark_default_profile_id),
        ("spark_default", graph_default_profile_id),
    ] {
        if let Some(value) = normalize_optional_text(value.unwrap_or_default()) {
            return (Some(value), source.to_string());
        }
    }
    (None, "implementation_default".to_string())
}

fn first_profile_id<'a>(values: impl IntoIterator<Item = Option<&'a str>>) -> Option<String> {
    values
        .into_iter()
        .find_map(|value| normalize_optional_text(value.unwrap_or_default()))
}

fn profile_error(
    field_errors: &mut Vec<ExecutionProfileFieldError>,
    profile_id: &str,
    field_name: &str,
    message: impl Into<String>,
) {
    field_errors.push(ExecutionProfileFieldError::for_profile(
        profile_id.to_string(),
        format!("profiles.{profile_id}.{field_name}"),
        message,
    ));
}

fn optional_string(value: &toml::Value) -> Option<String> {
    value.as_str().and_then(normalize_optional_text)
}

fn normalize_optional_text(value: impl AsRef<str>) -> Option<String> {
    let value = value.as_ref().trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn normalize_text(value: impl AsRef<str>) -> String {
    value.as_ref().trim().to_string()
}

fn default_enabled() -> bool {
    true
}
