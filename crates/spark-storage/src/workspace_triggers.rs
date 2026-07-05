use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use spark_common::project::normalize_project_path;
use spark_common::settings::SparkSettings;
use toml::Value as TomlValue;

use crate::error::{Result, StorageError};
use crate::{write_json_atomic, write_text_atomic, JsonWriteOptions};

const SOURCE_SCHEDULE: &str = "schedule";
const SOURCE_POLL: &str = "poll";
const SOURCE_WEBHOOK: &str = "webhook";
const SOURCE_FLOW_EVENT: &str = "flow_event";
const WEEKDAY_ORDER: &[&str] = &["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
const TERMINAL_PIPELINE_STATUSES: &[&str] = &[
    "completed",
    "failed",
    "validation_error",
    "canceled",
    "cancelled",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerAction {
    pub flow_name: String,
    pub project_path: Option<String>,
    pub static_context: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerDefinition {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub protected: bool,
    pub source_type: String,
    pub action: TriggerAction,
    pub source: Map<String, Value>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TriggerState {
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub last_fired_at: Option<String>,
    #[serde(default)]
    pub last_result: Option<String>,
    #[serde(default)]
    pub next_run_at: Option<String>,
    #[serde(default)]
    pub recent_history: Vec<TriggerStateHistoryEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerStateHistoryEntry {
    pub timestamp: String,
    pub status: String,
    pub message: String,
    #[serde(default)]
    pub run_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TriggerDefinitionRepository {
    config_dir: PathBuf,
}

impl TriggerDefinitionRepository {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }

    pub fn root_dir(&self) -> Result<PathBuf> {
        trigger_config_dir(&self.config_dir)
    }

    pub fn definition_path(&self, trigger_id: &str) -> Result<PathBuf> {
        let id = validate_trigger_id(trigger_id)?;
        Ok(self.root_dir()?.join(format!("{id}.toml")))
    }

    pub fn list(&self) -> Result<Vec<TriggerDefinition>> {
        let root = self.root_dir()?;
        let mut paths = fs::read_dir(&root)
            .map_err(|source| StorageError::io("list trigger definitions", &root, source))?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("toml"))
            .collect::<Vec<_>>();
        paths.sort();
        let mut definitions = Vec::new();
        for path in paths {
            let Some(trigger_id) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            match self.get(trigger_id) {
                Ok(Some(definition)) => definitions.push(definition),
                Ok(None) | Err(_) => {}
            }
        }
        Ok(definitions)
    }

    pub fn get(&self, trigger_id: &str) -> Result<Option<TriggerDefinition>> {
        let path = self.definition_path(trigger_id)?;
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(StorageError::io("read trigger definition", &path, source)),
        };
        let payload = text
            .parse::<TomlValue>()
            .map_err(|source| StorageError::TomlRead {
                path: path.clone(),
                source,
            })?;
        let table = payload
            .as_table()
            .ok_or_else(|| invalid_trigger(&path, "Trigger definition must be a TOML table."))?;
        parse_trigger_definition(trigger_id, table, &path).map(Some)
    }

    pub fn put(&self, definition: &TriggerDefinition) -> Result<PathBuf> {
        let path = self.definition_path(&definition.id)?;
        write_text_atomic(&path, trigger_definition_toml(definition))?;
        Ok(path)
    }

    pub fn delete(&self, trigger_id: &str) -> Result<()> {
        let path = self.definition_path(trigger_id)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(StorageError::io("delete trigger definition", &path, source)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TriggerRuntimeStateRepository {
    data_dir: PathBuf,
}

impl TriggerRuntimeStateRepository {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    pub fn root_dir(&self) -> Result<PathBuf> {
        trigger_state_dir(&self.data_dir)
    }

    pub fn state_path(&self, trigger_id: &str) -> Result<PathBuf> {
        let id = validate_trigger_id(trigger_id)?;
        Ok(self.root_dir()?.join(format!("{id}.json")))
    }

    pub fn load(&self, trigger_id: &str) -> Result<TriggerState> {
        let path = self.state_path(trigger_id)?;
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(TriggerState::default())
            }
            Err(source) => return Err(StorageError::io("read trigger state", &path, source)),
        };
        serde_json::from_str(&text).or_else(|_| Ok(TriggerState::default()))
    }

    pub fn save(&self, trigger_id: &str, state: &TriggerState) -> Result<PathBuf> {
        let path = self.state_path(trigger_id)?;
        write_json_atomic(&path, state, JsonWriteOptions::default())?;
        Ok(path)
    }

    pub fn update<F>(&self, trigger_id: &str, update: F) -> Result<TriggerState>
    where
        F: FnOnce(&mut TriggerState),
    {
        let mut state = self.load(trigger_id)?;
        update(&mut state);
        self.save(trigger_id, &state)?;
        Ok(state)
    }

    pub fn delete(&self, trigger_id: &str) -> Result<()> {
        let path = self.state_path(trigger_id)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(StorageError::io("delete trigger state", &path, source)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TriggerRepositories {
    pub definitions: TriggerDefinitionRepository,
    pub runtime_state: TriggerRuntimeStateRepository,
}

impl TriggerRepositories {
    pub fn new(config_dir: impl Into<PathBuf>, data_dir: impl Into<PathBuf>) -> Self {
        Self {
            definitions: TriggerDefinitionRepository::new(config_dir),
            runtime_state: TriggerRuntimeStateRepository::new(data_dir),
        }
    }

    pub fn from_settings(settings: &SparkSettings) -> Self {
        Self::new(settings.config_dir.clone(), settings.data_dir.clone())
    }
}

pub fn trigger_config_dir(config_dir: impl AsRef<Path>) -> Result<PathBuf> {
    let path = config_dir.as_ref().join("triggers");
    fs::create_dir_all(&path)
        .map_err(|source| StorageError::io("create trigger config directory", &path, source))?;
    Ok(path)
}

pub fn trigger_definition_path(config_dir: impl AsRef<Path>, trigger_id: &str) -> Result<PathBuf> {
    TriggerDefinitionRepository::new(config_dir.as_ref().to_path_buf()).definition_path(trigger_id)
}

pub fn trigger_state_dir(data_dir: impl AsRef<Path>) -> Result<PathBuf> {
    let path = data_dir.as_ref().join("workspace").join("trigger-state");
    fs::create_dir_all(&path)
        .map_err(|source| StorageError::io("create trigger state directory", &path, source))?;
    Ok(path)
}

pub fn trigger_state_path(data_dir: impl AsRef<Path>, trigger_id: &str) -> Result<PathBuf> {
    TriggerRuntimeStateRepository::new(data_dir.as_ref().to_path_buf()).state_path(trigger_id)
}

pub fn list_trigger_definitions(config_dir: impl AsRef<Path>) -> Result<Vec<TriggerDefinition>> {
    TriggerDefinitionRepository::new(config_dir.as_ref().to_path_buf()).list()
}

pub fn read_trigger_definition(
    config_dir: impl AsRef<Path>,
    trigger_id: &str,
) -> Result<Option<TriggerDefinition>> {
    TriggerDefinitionRepository::new(config_dir.as_ref().to_path_buf()).get(trigger_id)
}

pub fn write_trigger_definition(
    config_dir: impl AsRef<Path>,
    definition: &TriggerDefinition,
) -> Result<PathBuf> {
    TriggerDefinitionRepository::new(config_dir.as_ref().to_path_buf()).put(definition)
}

pub fn delete_trigger_definition(config_dir: impl AsRef<Path>, trigger_id: &str) -> Result<()> {
    TriggerDefinitionRepository::new(config_dir.as_ref().to_path_buf()).delete(trigger_id)
}

pub fn load_trigger_state(data_dir: impl AsRef<Path>, trigger_id: &str) -> Result<TriggerState> {
    TriggerRuntimeStateRepository::new(data_dir.as_ref().to_path_buf()).load(trigger_id)
}

pub fn save_trigger_state(
    data_dir: impl AsRef<Path>,
    trigger_id: &str,
    state: &TriggerState,
) -> Result<PathBuf> {
    TriggerRuntimeStateRepository::new(data_dir.as_ref().to_path_buf()).save(trigger_id, state)
}

pub fn delete_trigger_state(data_dir: impl AsRef<Path>, trigger_id: &str) -> Result<()> {
    TriggerRuntimeStateRepository::new(data_dir.as_ref().to_path_buf()).delete(trigger_id)
}

pub fn normalize_trigger_action_payload(payload: &Map<String, Value>) -> Result<TriggerAction> {
    normalize_trigger_action_payload_at(payload, Path::new("trigger definition"))
}

pub fn normalize_trigger_source_payload(
    source_type: &str,
    payload: &Map<String, Value>,
    preserve_secret_hash: Option<&Value>,
) -> Result<Map<String, Value>> {
    normalize_trigger_source_payload_at(
        source_type,
        payload,
        preserve_secret_hash,
        Path::new("trigger definition"),
    )
}

fn parse_trigger_definition(
    trigger_id: &str,
    payload: &toml::map::Map<String, TomlValue>,
    path: &Path,
) -> Result<TriggerDefinition> {
    let action_payload = payload
        .get("action")
        .and_then(|value| value.as_table())
        .ok_or_else(|| {
            invalid_trigger(
                path,
                "Trigger definition must include [action] and [source] sections.",
            )
        })?;
    let source_payload = payload
        .get("source")
        .and_then(|value| value.as_table())
        .ok_or_else(|| {
            invalid_trigger(
                path,
                "Trigger definition must include [action] and [source] sections.",
            )
        })?;
    let action = normalize_trigger_action_payload_at(&parse_action_payload(action_payload), path)?;
    let source_type = toml_scalar_to_string(payload.get("source_type"))
        .trim()
        .to_string();
    let source = normalize_trigger_source_payload_at(
        &source_type,
        &parse_source(source_payload, path)?,
        None,
        path,
    )?;
    Ok(TriggerDefinition {
        id: trigger_id.to_string(),
        name: toml_scalar_to_string(payload.get("name"))
            .trim()
            .to_string(),
        enabled: payload
            .get("enabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(true),
        protected: payload
            .get("protected")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        source_type,
        action,
        source,
        created_at: toml_scalar_to_string(payload.get("created_at")),
        updated_at: toml_scalar_to_string(payload.get("updated_at")),
    })
}

fn parse_action_payload(payload: &toml::map::Map<String, TomlValue>) -> Map<String, Value> {
    payload
        .iter()
        .map(|(key, value)| (key.clone(), toml_value_to_json(value)))
        .collect()
}

fn parse_source(
    payload: &toml::map::Map<String, TomlValue>,
    path: &Path,
) -> Result<Map<String, Value>> {
    let mut source = Map::new();
    for (key, value) in payload {
        if let Some(json_key) = key.strip_suffix("_json") {
            if let Some(text) = value.as_str() {
                let parsed = serde_json::from_str::<Value>(text).map_err(|_| {
                    invalid_trigger(path, format!("Invalid JSON value for source.{key}"))
                })?;
                source.insert(json_key.to_string(), parsed);
                continue;
            }
        }
        source.insert(key.clone(), toml_value_to_json(value));
    }
    Ok(source)
}

fn normalize_trigger_action_payload_at(
    payload: &Map<String, Value>,
    path: &Path,
) -> Result<TriggerAction> {
    let flow_name = payload
        .get("flow_name")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if flow_name.is_empty() {
        return Err(invalid_trigger(
            path,
            "Trigger action requires a flow_name.",
        ));
    }
    let project_path = payload
        .get("project_path")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(|value| {
            normalize_project_path(value)
                .map_err(|error| invalid_trigger(path, error.to_string()))?
                .map(|path| path.to_string_lossy().into_owned())
                .ok_or_else(|| invalid_trigger(path, "Project path is required."))
        })
        .transpose()?;
    let static_context = match payload.get("static_context") {
        Some(Value::Object(object)) => object.clone(),
        Some(Value::Null) | None => match payload
            .get("static_context_json")
            .and_then(|value| value.as_str())
        {
            Some(text) => serde_json::from_str::<Value>(text)
                .ok()
                .and_then(|value| value.as_object().cloned())
                .ok_or_else(|| {
                    invalid_trigger(
                        path,
                        "Trigger action static_context_json must be valid JSON.",
                    )
                })?,
            None => Map::new(),
        },
        Some(_) => {
            return Err(invalid_trigger(
                path,
                "Trigger action static_context must be a JSON object.",
            ))
        }
    };
    Ok(TriggerAction {
        flow_name,
        project_path,
        static_context,
    })
}

fn normalize_trigger_source_payload_at(
    source_type: &str,
    payload: &Map<String, Value>,
    preserve_secret_hash: Option<&Value>,
    path: &Path,
) -> Result<Map<String, Value>> {
    match source_type.trim() {
        SOURCE_SCHEDULE => normalize_schedule_source(payload, path),
        SOURCE_POLL => normalize_poll_source(payload, path),
        SOURCE_WEBHOOK => normalize_webhook_source(payload, preserve_secret_hash, path),
        SOURCE_FLOW_EVENT => normalize_flow_event_source(payload, path),
        other => Err(invalid_trigger(
            path,
            format!("Unsupported trigger source type: {other}"),
        )),
    }
}

fn normalize_schedule_source(
    payload: &Map<String, Value>,
    path: &Path,
) -> Result<Map<String, Value>> {
    let kind = payload
        .get("kind")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if !["once", "interval", "weekly"].contains(&kind.as_str()) {
        return Err(invalid_trigger(
            path,
            "Schedule triggers require kind=once|interval|weekly.",
        ));
    }
    let mut source = Map::new();
    source.insert("kind".to_string(), Value::String(kind.clone()));
    match kind.as_str() {
        "once" => {
            let run_at = payload
                .get("run_at")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .trim();
            if run_at.is_empty() {
                return Err(invalid_trigger(
                    path,
                    "One-shot schedule triggers require run_at.",
                ));
            }
            source.insert("run_at".to_string(), Value::String(run_at.to_string()));
        }
        "interval" => {
            source.insert(
                "interval_seconds".to_string(),
                Value::from(coerce_positive_int(
                    payload.get("interval_seconds"),
                    "interval_seconds",
                    path,
                )?),
            );
        }
        "weekly" => {
            let raw_weekdays = payload
                .get("weekdays")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            let mut weekdays = raw_weekdays
                .iter()
                .map(|value| {
                    json_value_to_python_string(value)
                        .trim()
                        .to_ascii_lowercase()
                })
                .collect::<Vec<_>>();
            if weekdays.is_empty()
                || weekdays
                    .iter()
                    .any(|weekday| !WEEKDAY_ORDER.contains(&weekday.as_str()))
            {
                return Err(invalid_trigger(
                    path,
                    "Weekly schedule triggers require weekdays using mon..sun.",
                ));
            }
            weekdays.sort_by_key(|weekday| {
                WEEKDAY_ORDER
                    .iter()
                    .position(|candidate| *candidate == weekday)
                    .unwrap_or(usize::MAX)
            });
            weekdays.dedup();
            source.insert(
                "weekdays".to_string(),
                Value::Array(weekdays.into_iter().map(Value::String).collect()),
            );
            source.insert(
                "hour".to_string(),
                Value::from(coerce_int_range(payload.get("hour"), "hour", 0, 23, path)?),
            );
            source.insert(
                "minute".to_string(),
                Value::from(coerce_int_range(
                    payload.get("minute"),
                    "minute",
                    0,
                    59,
                    path,
                )?),
            );
        }
        _ => {}
    }
    Ok(source)
}

fn normalize_poll_source(payload: &Map<String, Value>, path: &Path) -> Result<Map<String, Value>> {
    let url = payload
        .get("url")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(invalid_trigger(
            path,
            "Poll triggers require an http(s) url.",
        ));
    }
    let interval_seconds =
        coerce_positive_int(payload.get("interval_seconds"), "interval_seconds", path)?;
    let items_path = payload
        .get("items_path")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let item_id_path = payload
        .get("item_id_path")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if items_path.is_empty() || item_id_path.is_empty() {
        return Err(invalid_trigger(
            path,
            "Poll triggers require items_path and item_id_path.",
        ));
    }
    let headers = payload
        .get("headers")
        .and_then(|value| value.as_object())
        .map(|object| {
            object
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        Value::String(json_value_to_python_string(value)),
                    )
                })
                .collect()
        })
        .unwrap_or_else(Map::new);
    Ok(Map::from_iter([
        ("url".to_string(), Value::String(url)),
        (
            "interval_seconds".to_string(),
            Value::from(interval_seconds),
        ),
        ("items_path".to_string(), Value::String(items_path)),
        ("item_id_path".to_string(), Value::String(item_id_path)),
        ("headers".to_string(), Value::Object(headers)),
    ]))
}

fn normalize_webhook_source(
    payload: &Map<String, Value>,
    preserve_secret_hash: Option<&Value>,
    path: &Path,
) -> Result<Map<String, Value>> {
    let webhook_key = payload
        .get("webhook_key")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let secret_hash = payload
        .get("secret_hash")
        .or(preserve_secret_hash)
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if webhook_key.is_empty() {
        return Err(invalid_trigger(
            path,
            "Webhook triggers require webhook_key.",
        ));
    }
    if secret_hash.is_empty() {
        return Err(invalid_trigger(
            path,
            "Webhook triggers require secret_hash.",
        ));
    }
    Ok(Map::from_iter([
        ("webhook_key".to_string(), Value::String(webhook_key)),
        ("secret_hash".to_string(), Value::String(secret_hash)),
    ]))
}

fn normalize_flow_event_source(
    payload: &Map<String, Value>,
    path: &Path,
) -> Result<Map<String, Value>> {
    let flow_name = payload
        .get("flow_name")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| Value::String(value.to_string()))
        .unwrap_or(Value::Null);
    let statuses = payload
        .get("statuses")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|value| {
            json_value_to_python_string(&value)
                .trim()
                .to_ascii_lowercase()
        })
        .collect::<Vec<_>>();
    let normalized_statuses = statuses
        .iter()
        .filter(|status| TERMINAL_PIPELINE_STATUSES.contains(&status.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !statuses.is_empty() && normalized_statuses.is_empty() {
        return Err(invalid_trigger(
            path,
            "Flow-event triggers require terminal statuses when statuses are provided.",
        ));
    }
    Ok(Map::from_iter([
        ("flow_name".to_string(), flow_name),
        (
            "statuses".to_string(),
            Value::Array(normalized_statuses.into_iter().map(Value::String).collect()),
        ),
    ]))
}

fn coerce_positive_int(value: Option<&Value>, field_name: &str, path: &Path) -> Result<i64> {
    let number = coerce_int(value, field_name, path)?;
    if number <= 0 {
        return Err(invalid_trigger(
            path,
            format!("{field_name} must be greater than zero."),
        ));
    }
    Ok(number)
}

fn coerce_int_range(
    value: Option<&Value>,
    field_name: &str,
    minimum: i64,
    maximum: i64,
    path: &Path,
) -> Result<i64> {
    let number = coerce_int(value, field_name, path)?;
    if number < minimum || number > maximum {
        return Err(invalid_trigger(
            path,
            format!("{field_name} must be between {minimum} and {maximum}."),
        ));
    }
    Ok(number)
}

fn coerce_int(value: Option<&Value>, field_name: &str, path: &Path) -> Result<i64> {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .ok_or_else(|| invalid_trigger(path, format!("{field_name} must be an integer."))),
        _ => Err(invalid_trigger(
            path,
            format!("{field_name} must be an integer."),
        )),
    }
}

fn trigger_definition_toml(definition: &TriggerDefinition) -> String {
    let mut lines = vec![
        format!("id = {}", toml_string(&definition.id)),
        format!("name = {}", toml_string(&definition.name)),
        format!("enabled = {}", toml_bool(definition.enabled)),
        format!("protected = {}", toml_bool(definition.protected)),
        format!("source_type = {}", toml_string(&definition.source_type)),
        format!("created_at = {}", toml_string(&definition.created_at)),
        format!("updated_at = {}", toml_string(&definition.updated_at)),
        String::new(),
        "[action]".to_string(),
        format!("flow_name = {}", toml_string(&definition.action.flow_name)),
    ];
    if let Some(project_path) = definition.action.project_path.as_deref() {
        lines.push(format!("project_path = {}", toml_string(project_path)));
    }
    if !definition.action.static_context.is_empty() {
        lines.push(format!(
            "static_context_json = {}",
            toml_string(&json_string_python_style(&Value::Object(
                definition.action.static_context.clone()
            )))
        ));
    }
    lines.push(String::new());
    lines.push("[source]".to_string());
    let mut source_keys = definition.source.keys().cloned().collect::<Vec<_>>();
    source_keys.sort();
    for key in source_keys {
        let Some(value) = definition.source.get(&key) else {
            continue;
        };
        if value.is_null() {
            continue;
        }
        lines.extend(toml_source_line(&key, value));
    }
    lines.push(String::new());
    lines.join("\n")
}

fn toml_source_line(key: &str, value: &Value) -> Vec<String> {
    match value {
        Value::Bool(value) => vec![format!("{key} = {}", toml_bool(*value))],
        Value::Number(value) => vec![format!("{key} = {value}")],
        Value::Array(values) => vec![format!(
            "{key} = [{}]",
            values
                .iter()
                .map(|value| toml_string(&json_value_to_python_string(value)))
                .collect::<Vec<_>>()
                .join(", ")
        )],
        Value::Object(_) => vec![format!(
            "{key}_json = {}",
            toml_string(&json_string_python_style(value))
        )],
        Value::String(value) => vec![format!("{key} = {}", toml_string(value))],
        Value::Null => Vec::new(),
    }
}

fn toml_value_to_json(value: &TomlValue) -> Value {
    match value {
        TomlValue::String(value) => Value::String(value.clone()),
        TomlValue::Integer(value) => Value::from(*value),
        TomlValue::Float(value) => Value::from(*value),
        TomlValue::Boolean(value) => Value::from(*value),
        TomlValue::Datetime(value) => Value::String(value.to_string()),
        TomlValue::Array(values) => Value::Array(values.iter().map(toml_value_to_json).collect()),
        TomlValue::Table(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), toml_value_to_json(value)))
                .collect(),
        ),
    }
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

fn json_value_to_python_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::Null => "None".to_string(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

fn json_string_python_style(value: &Value) -> String {
    match value {
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort();
            let parts = keys
                .into_iter()
                .map(|key| {
                    format!(
                        "{}: {}",
                        serde_json::to_string(key).expect("json key"),
                        json_string_python_style(&object[key])
                    )
                })
                .collect::<Vec<_>>();
            format!("{{{}}}", parts.join(", "))
        }
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(json_string_python_style)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::String(value) => serde_json::to_string(value).expect("json string"),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null => "null".to_string(),
    }
}

fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn toml_bool(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

fn validate_trigger_id(trigger_id: &str) -> Result<&str> {
    let id = trigger_id.trim();
    if id.is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id == "."
        || id == ".."
        || id.contains("..")
    {
        return Err(StorageError::InvalidRepositoryPath {
            path: PathBuf::from(trigger_id),
            reason: "Trigger id must be a file name.".to_string(),
        });
    }
    Ok(id)
}

fn invalid_trigger(path: &Path, reason: impl Into<String>) -> StorageError {
    StorageError::InvalidRepositoryPath {
        path: path.to_path_buf(),
        reason: reason.into(),
    }
}
