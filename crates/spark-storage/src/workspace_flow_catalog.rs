use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use toml::Value as TomlValue;

use crate::error::{Result, StorageError};
use crate::write_text_atomic;

pub const FLOW_CATALOG_FILE_NAME: &str = "flow-catalog.toml";
pub const LAUNCH_POLICY_AGENT_REQUESTABLE: &str = "agent_requestable";
pub const LAUNCH_POLICY_TRIGGER_ONLY: &str = "trigger_only";
pub const LAUNCH_POLICY_DISABLED: &str = "disabled";
pub const EXECUTION_LOCK_SCOPE_PROJECT: &str = "project";
pub const EXECUTION_LOCK_CONFLICT_POLICY_QUEUE: &str = "queue";

pub const ALLOWED_LAUNCH_POLICIES: &[&str] = &[
    LAUNCH_POLICY_AGENT_REQUESTABLE,
    LAUNCH_POLICY_DISABLED,
    LAUNCH_POLICY_TRIGGER_ONLY,
];
pub const ALLOWED_EXECUTION_LOCK_SCOPES: &[&str] = &[EXECUTION_LOCK_SCOPE_PROJECT];
pub const ALLOWED_EXECUTION_LOCK_CONFLICT_POLICIES: &[&str] =
    &[EXECUTION_LOCK_CONFLICT_POLICY_QUEUE];

pub const DEFAULT_AGENT_REQUESTABLE_FLOWS: &[&str] = &[
    "software-development/implement-change-request.yaml",
    "software-development/spec-implementation/implement-spec.yaml",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowExecutionLockConfig {
    pub scope: String,
    pub key: String,
    pub conflict_policy: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowCatalogEntry {
    #[serde(default)]
    pub launch_policy: Option<String>,
    #[serde(default)]
    pub execution_lock: Option<FlowExecutionLockConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowLaunchPolicyState {
    pub name: String,
    pub launch_policy: Option<String>,
    pub effective_launch_policy: String,
    pub execution_lock: Option<FlowExecutionLockConfig>,
}

pub fn flow_catalog_path(config_dir: impl AsRef<Path>) -> Result<PathBuf> {
    let config_dir = config_dir.as_ref();
    fs::create_dir_all(config_dir)
        .map_err(|source| StorageError::io("create config directory", config_dir, source))?;
    Ok(config_dir.join(FLOW_CATALOG_FILE_NAME))
}

pub fn load_flow_catalog(
    config_dir: impl AsRef<Path>,
) -> Result<BTreeMap<String, FlowCatalogEntry>> {
    let path = flow_catalog_path(config_dir)?;
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let text = fs::read_to_string(&path)
        .map_err(|source| StorageError::io("read flow catalog", &path, source))?;
    if text.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    let payload = text
        .parse::<TomlValue>()
        .map_err(|source| StorageError::TomlRead {
            path: path.clone(),
            source,
        })?;
    let Some(flows_section) = payload.get("flows") else {
        return Ok(BTreeMap::new());
    };
    let flows = flows_section.as_table().ok_or_else(|| {
        invalid_catalog(
            &path,
            format!(
                "Flow catalog file is missing a valid [flows] section: {}",
                path.display()
            ),
        )
    })?;

    let mut catalog = BTreeMap::new();
    for (raw_flow_name, raw_entry) in flows {
        // Catalogs written before the YAML cutover may persist entries for
        // ".dot" flows; those names no longer resolve. Skip them rather than
        // failing startup on old state — the next catalog write drops them.
        if raw_flow_name.ends_with(".dot") {
            eprintln!(
                "warning: ignoring legacy flow catalog entry {raw_flow_name:?} in {}",
                path.display()
            );
            continue;
        }
        let entry = raw_entry.as_table().ok_or_else(|| {
            invalid_catalog(
                &path,
                format!(
                    "Flow catalog entry for {raw_flow_name:?} must be a table: {}",
                    path.display()
                ),
            )
        })?;
        let launch_policy = match entry.get("launch_policy") {
            Some(value) => {
                let policy = value.as_str().ok_or_else(|| {
                    invalid_catalog(
                        &path,
                        format!(
                            "Flow catalog entry for {raw_flow_name:?} must define launch_policy as a string: {}",
                            path.display()
                        ),
                    )
                })?;
                Some(normalize_launch_policy(policy)?)
            }
            None => None,
        };
        let execution_lock = parse_execution_lock(entry, raw_flow_name, &path)?;
        let flow_name = normalize_flow_name(raw_flow_name)?;
        catalog.insert(
            flow_name,
            FlowCatalogEntry {
                launch_policy,
                execution_lock,
            },
        );
    }
    Ok(catalog)
}

pub fn write_flow_catalog(
    config_dir: impl AsRef<Path>,
    catalog: &BTreeMap<String, FlowCatalogEntry>,
) -> Result<PathBuf> {
    let path = flow_catalog_path(config_dir)?;
    let mut normalized_catalog = BTreeMap::new();
    for (flow_name, entry) in catalog {
        let flow_name = normalize_flow_name(flow_name)?;
        let launch_policy = entry
            .launch_policy
            .as_deref()
            .map(normalize_launch_policy)
            .transpose()?;
        let execution_lock = entry
            .execution_lock
            .as_ref()
            .map(normalize_execution_lock_config)
            .transpose()?;
        normalized_catalog.insert(
            flow_name,
            FlowCatalogEntry {
                launch_policy,
                execution_lock,
            },
        );
    }

    let mut lines = Vec::new();
    for (flow_name, entry) in normalized_catalog {
        if entry.launch_policy.is_none() && entry.execution_lock.is_none() {
            continue;
        }
        lines.push(format!("[flows.{}]", toml_string(&flow_name)));
        if let Some(launch_policy) = entry.launch_policy {
            lines.push(format!("launch_policy = {}", toml_string(&launch_policy)));
        }
        if let Some(execution_lock) = entry.execution_lock {
            lines.push(String::new());
            lines.push(format!(
                "[flows.{}.execution_lock]",
                toml_string(&flow_name)
            ));
            lines.push(format!("scope = {}", toml_string(&execution_lock.scope)));
            lines.push(format!("key = {}", toml_string(&execution_lock.key)));
            lines.push(format!(
                "conflict_policy = {}",
                toml_string(&execution_lock.conflict_policy)
            ));
        }
        lines.push(String::new());
    }
    write_text_atomic(&path, lines.join("\n"))?;
    Ok(path)
}

pub fn read_flow_launch_policy(
    config_dir: impl AsRef<Path>,
    flow_name: &str,
) -> Result<FlowLaunchPolicyState> {
    let normalized_flow_name = normalize_flow_name(flow_name)?;
    let catalog = load_flow_catalog(config_dir)?;
    let entry = catalog
        .get(&normalized_flow_name)
        .cloned()
        .unwrap_or_default();
    let effective_launch_policy = entry
        .launch_policy
        .clone()
        .unwrap_or_else(|| LAUNCH_POLICY_DISABLED.to_string());
    Ok(FlowLaunchPolicyState {
        name: normalized_flow_name,
        launch_policy: entry.launch_policy,
        effective_launch_policy,
        execution_lock: entry.execution_lock,
    })
}

pub fn set_flow_launch_policy(
    config_dir: impl AsRef<Path>,
    flow_name: &str,
    launch_policy: &str,
) -> Result<FlowLaunchPolicyState> {
    let normalized_flow_name = normalize_flow_name(flow_name)?;
    let normalized_launch_policy = normalize_launch_policy(launch_policy)?;
    let mut catalog = load_flow_catalog(config_dir.as_ref())?;
    let execution_lock = catalog
        .get(&normalized_flow_name)
        .and_then(|entry| entry.execution_lock.clone());
    catalog.insert(
        normalized_flow_name.clone(),
        FlowCatalogEntry {
            launch_policy: Some(normalized_launch_policy.clone()),
            execution_lock: execution_lock.clone(),
        },
    );
    write_flow_catalog(config_dir, &catalog)?;
    Ok(FlowLaunchPolicyState {
        name: normalized_flow_name,
        launch_policy: Some(normalized_launch_policy.clone()),
        effective_launch_policy: normalized_launch_policy,
        execution_lock,
    })
}

pub fn set_flow_catalog_entry(
    config_dir: impl AsRef<Path>,
    flow_name: &str,
    launch_policy: &str,
    execution_lock: Option<FlowExecutionLockConfig>,
) -> Result<FlowLaunchPolicyState> {
    let normalized_flow_name = normalize_flow_name(flow_name)?;
    let normalized_launch_policy = normalize_launch_policy(launch_policy)?;
    let normalized_execution_lock = execution_lock
        .as_ref()
        .map(normalize_execution_lock_config)
        .transpose()?;
    let mut catalog = load_flow_catalog(config_dir.as_ref())?;
    catalog.insert(
        normalized_flow_name.clone(),
        FlowCatalogEntry {
            launch_policy: Some(normalized_launch_policy.clone()),
            execution_lock: normalized_execution_lock.clone(),
        },
    );
    write_flow_catalog(config_dir, &catalog)?;
    Ok(FlowLaunchPolicyState {
        name: normalized_flow_name,
        launch_policy: Some(normalized_launch_policy.clone()),
        effective_launch_policy: normalized_launch_policy,
        execution_lock: normalized_execution_lock,
    })
}

pub fn seed_default_flow_catalog(config_dir: impl AsRef<Path>) -> Result<Vec<String>> {
    let mut catalog = load_flow_catalog(config_dir.as_ref())?;
    let mut missing = Vec::new();
    for flow_name in DEFAULT_AGENT_REQUESTABLE_FLOWS {
        let normalized_flow_name = normalize_flow_name(flow_name)?;
        if catalog.contains_key(&normalized_flow_name) {
            continue;
        }
        catalog.insert(
            normalized_flow_name.clone(),
            FlowCatalogEntry {
                launch_policy: Some(LAUNCH_POLICY_AGENT_REQUESTABLE.to_string()),
                execution_lock: None,
            },
        );
        missing.push(normalized_flow_name);
    }
    if !missing.is_empty() {
        write_flow_catalog(config_dir, &catalog)?;
    }
    Ok(missing)
}

pub fn normalize_flow_name(flow_name: &str) -> Result<String> {
    attractor_dsl::normalize_flow_name(flow_name).map_err(|error| {
        StorageError::InvalidRepositoryPath {
            path: PathBuf::from(flow_name),
            reason: error.detail().to_string(),
        }
    })
}

pub fn normalize_launch_policy(launch_policy: &str) -> Result<String> {
    let normalized = launch_policy.trim().to_ascii_lowercase();
    if ALLOWED_LAUNCH_POLICIES.contains(&normalized.as_str()) {
        return Ok(normalized);
    }
    Err(invalid_value(format!(
        "Launch policy must be one of: {}",
        ALLOWED_LAUNCH_POLICIES.join(", ")
    )))
}

pub fn normalize_execution_lock_config(
    execution_lock: &FlowExecutionLockConfig,
) -> Result<FlowExecutionLockConfig> {
    let key = execution_lock.key.trim();
    if key.is_empty() {
        return Err(invalid_value("Execution lock key is required."));
    }
    Ok(FlowExecutionLockConfig {
        scope: normalize_execution_lock_scope(&execution_lock.scope)?,
        key: key.to_string(),
        conflict_policy: normalize_execution_lock_conflict_policy(&execution_lock.conflict_policy)?,
    })
}

pub fn normalize_execution_lock_value(value: &JsonValue) -> Result<FlowExecutionLockConfig> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid_value("Execution lock must be an object."))?;
    let raw_scope = json_scalar_to_string(object.get("scope"));
    let raw_key = json_scalar_to_string(object.get("key"));
    let raw_conflict_policy = json_scalar_to_string(object.get("conflict_policy"));
    normalize_execution_lock_config(&FlowExecutionLockConfig {
        scope: raw_scope,
        key: raw_key,
        conflict_policy: raw_conflict_policy,
    })
}

pub fn normalize_execution_lock_scope(scope: &str) -> Result<String> {
    let normalized = scope.trim().to_ascii_lowercase();
    if ALLOWED_EXECUTION_LOCK_SCOPES.contains(&normalized.as_str()) {
        return Ok(normalized);
    }
    Err(invalid_value(format!(
        "Execution lock scope must be one of: {}",
        ALLOWED_EXECUTION_LOCK_SCOPES.join(", ")
    )))
}

pub fn normalize_execution_lock_conflict_policy(conflict_policy: &str) -> Result<String> {
    let normalized = conflict_policy.trim().to_ascii_lowercase();
    if ALLOWED_EXECUTION_LOCK_CONFLICT_POLICIES.contains(&normalized.as_str()) {
        return Ok(normalized);
    }
    Err(invalid_value(format!(
        "Execution lock conflict policy must be one of: {}",
        ALLOWED_EXECUTION_LOCK_CONFLICT_POLICIES.join(", ")
    )))
}

fn parse_execution_lock(
    entry: &toml::map::Map<String, TomlValue>,
    raw_flow_name: &str,
    path: &Path,
) -> Result<Option<FlowExecutionLockConfig>> {
    let Some(raw_execution_lock) = entry.get("execution_lock") else {
        return Ok(None);
    };
    let table = raw_execution_lock.as_table().ok_or_else(|| {
        invalid_catalog(
            path,
            format!(
                "Flow catalog entry for {raw_flow_name:?} must define execution_lock as a table: {}",
                path.display()
            ),
        )
    })?;
    let raw_scope = toml_scalar_to_string(table.get("scope"));
    let raw_key = toml_scalar_to_string(table.get("key"));
    let raw_conflict_policy = toml_scalar_to_string(table.get("conflict_policy"));
    normalize_execution_lock_config(&FlowExecutionLockConfig {
        scope: raw_scope,
        key: raw_key,
        conflict_policy: raw_conflict_policy,
    })
    .map(Some)
    .map_err(|error| {
        invalid_catalog(
            path,
            format!(
                "Invalid execution_lock for flow catalog entry {raw_flow_name:?}: {}",
                catalog_error_reason(error)
            ),
        )
    })
}

fn toml_scalar_to_string(value: Option<&TomlValue>) -> String {
    match value {
        Some(TomlValue::String(value)) => value.clone(),
        Some(TomlValue::Integer(value)) => value.to_string(),
        Some(TomlValue::Float(value)) => value.to_string(),
        Some(TomlValue::Boolean(value)) => value.to_string(),
        Some(TomlValue::Datetime(value)) => value.to_string(),
        Some(_) | None => String::new(),
    }
}

fn json_scalar_to_string(value: Option<&JsonValue>) -> String {
    match value {
        Some(JsonValue::String(value)) => value.clone(),
        Some(JsonValue::Number(value)) => value.to_string(),
        Some(JsonValue::Bool(value)) => value.to_string(),
        Some(JsonValue::Null) | Some(JsonValue::Array(_)) | Some(JsonValue::Object(_)) | None => {
            String::new()
        }
    }
}

fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn invalid_value(reason: impl Into<String>) -> StorageError {
    StorageError::InvalidRepositoryPath {
        path: PathBuf::from(FLOW_CATALOG_FILE_NAME),
        reason: reason.into(),
    }
}

fn invalid_catalog(path: &Path, reason: impl Into<String>) -> StorageError {
    StorageError::InvalidRepositoryPath {
        path: path.to_path_buf(),
        reason: reason.into(),
    }
}

fn catalog_error_reason(error: StorageError) -> String {
    match error {
        StorageError::InvalidRepositoryPath { reason, .. } => reason,
        other => other.to_string(),
    }
}
