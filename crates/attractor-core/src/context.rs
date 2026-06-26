use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AttractorCoreError, Result};

/// Allowed context key prefixes preserved from the Python runtime.
pub const ALLOWED_CONTEXT_PREFIXES: &[&str] = &[
    "context.",
    "graph.",
    "internal.",
    "parallel.",
    "stack.",
    "human.gate.",
    "work.",
    "_attractor.",
];

/// JSON-compatible context value.
pub type ContextValue = Value;

/// Deterministic serializable context key/value map.
pub type ContextMap = BTreeMap<String, ContextValue>;

/// Runtime context values plus log entries.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AttractorContext {
    #[serde(default)]
    values: ContextMap,
    #[serde(default)]
    logs: Vec<String>,
}

impl AttractorContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_map(values: ContextMap) -> Result<Self> {
        for key in values.keys() {
            validate_context_key(key)?;
        }
        Ok(Self {
            values,
            logs: Vec::new(),
        })
    }

    pub fn values(&self) -> &ContextMap {
        &self.values
    }

    pub fn logs(&self) -> &[String] {
        &self.logs
    }

    pub fn set(&mut self, key: impl Into<String>, value: ContextValue) -> Result<()> {
        let key = key.into();
        validate_context_key(&key)?;
        if value.is_null() {
            self.values.remove(&key);
        } else {
            self.values.insert(key, value);
        }
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<&ContextValue> {
        self.values.get(key)
    }

    pub fn get_string(&self, key: &str) -> String {
        self.get_string_or(key, "")
    }

    pub fn get_string_or(&self, key: &str, default: &str) -> String {
        match self.get(key) {
            Some(Value::Null) | None => default.to_string(),
            Some(value) => context_value_to_string(value),
        }
    }

    pub fn append_log(&mut self, entry: impl Into<String>) {
        self.logs.push(entry.into());
    }

    pub fn snapshot(&self) -> ContextMap {
        self.values.clone()
    }

    pub fn apply_updates(&mut self, updates: &ContextMap) -> Result<()> {
        for key in updates.keys() {
            validate_context_key(key)?;
        }
        for (key, value) in updates {
            if value.is_null() {
                self.values.remove(key);
            } else {
                self.values.insert(key.clone(), value.clone());
            }
        }
        Ok(())
    }

    pub fn merge_updates(&mut self, updates: &ContextMap) -> Result<()> {
        self.apply_updates(updates)
    }

    pub fn clone_isolated(&self) -> Self {
        self.clone()
    }

    /// Resolve a context path by checking flat keys before nested object paths.
    pub fn get_context_path(&self, path: &str) -> String {
        let path = path.trim();
        if path.is_empty() {
            return String::new();
        }

        for candidate in flat_path_candidates(path) {
            if let Some(value) = self.values.get(&candidate) {
                return context_value_to_string(value);
            }
        }

        for candidate in nested_path_candidates(path) {
            if let Some(value) = lookup_nested(&self.values, &candidate) {
                return context_value_to_string(value);
            }
        }

        String::new()
    }
}

/// Host-provided launch context normalized for runtime initialization.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LaunchContext {
    #[serde(default)]
    values: ContextMap,
}

impl LaunchContext {
    pub fn new(values: ContextMap) -> Result<Self> {
        let values = validate_launch_context(&values)?;
        Ok(Self { values })
    }

    pub fn empty() -> Self {
        Self::default()
    }

    pub fn values(&self) -> &ContextMap {
        &self.values
    }

    pub fn into_values(self) -> ContextMap {
        self.values
    }
}

pub fn validate_context_key(key: &str) -> Result<()> {
    if !key.contains('.') {
        return Ok(());
    }
    if ALLOWED_CONTEXT_PREFIXES
        .iter()
        .any(|prefix| key.starts_with(prefix))
    {
        return Ok(());
    }
    Err(AttractorCoreError::InvalidContextNamespace {
        key: key.to_string(),
    })
}

pub fn validate_launch_context(values: &ContextMap) -> Result<ContextMap> {
    let mut normalized = ContextMap::new();
    for (key, value) in values {
        if !key.starts_with("context.") {
            return Err(AttractorCoreError::InvalidLaunchContextKey {
                key: key.clone(),
                reason: "launch_context key must use the context.* namespace".to_string(),
            });
        }
        validate_json_compatible_value(value, key)?;
        normalized.insert(key.clone(), value.clone());
    }
    Ok(normalized)
}

pub fn apply_launch_context(context: &mut AttractorContext, launch: &LaunchContext) -> Result<()> {
    context.apply_updates(launch.values())
}

fn flat_path_candidates(path: &str) -> Vec<String> {
    if let Some(stripped) = path.strip_prefix("context.") {
        vec![path.to_string(), stripped.to_string()]
    } else {
        vec![path.to_string(), format!("context.{path}")]
    }
}

fn nested_path_candidates(path: &str) -> Vec<String> {
    if let Some(stripped) = path.strip_prefix("context.") {
        vec![path.to_string(), stripped.to_string()]
    } else {
        vec![path.to_string(), format!("context.{path}")]
    }
}

fn lookup_nested<'a>(values: &'a ContextMap, path: &str) -> Option<&'a ContextValue> {
    let mut parts = path.split('.');
    let first = parts.next()?;
    let mut current = values.get(first)?;
    for part in parts {
        current = current.as_object()?.get(part)?;
    }
    Some(current)
}

fn validate_json_compatible_value(value: &ContextValue, path: &str) -> Result<()> {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => Ok(()),
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                validate_json_compatible_value(item, &format!("{path}[{index}]"))?;
            }
            Ok(())
        }
        Value::Object(items) => {
            for (key, item) in items {
                validate_json_compatible_value(item, &format!("{path}.{key}"))?;
            }
            Ok(())
        }
    }
}

pub(crate) fn context_value_to_string(value: &ContextValue) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(true) => "true".to_string(),
        Value::Bool(false) => "false".to_string(),
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Array(_) | Value::Object(_) => {
            serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
        }
    }
}
