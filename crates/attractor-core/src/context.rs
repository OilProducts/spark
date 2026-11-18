use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AttractorCoreError, Result};

/// Allowed context key prefixes. A flow owns every `context.*` key; Spark
/// owns every other key.
pub const ALLOWED_CONTEXT_PREFIXES: &[&str] = &[
    "context.",
    "graph.",
    "internal.",
    "parallel.",
    "stack.",
    "human.gate.",
    "tool.",
    "_attractor.",
];

/// The flow's own namespace: the only keys a flow writes.
pub const FLOW_CONTEXT_PREFIX: &str = "context.";

/// Every Spark-owned key a flow may read. Every other key outside
/// `context.*` is private to Spark. The authoring guide's Context keys
/// section lists exactly these.
pub const READABLE_SPARK_KEYS: &[&str] = &[
    "internal.run_id",
    "internal.root_run_id",
    "internal.run_workdir",
    "internal.runs_dir",
    "internal.project_work_dir",
    "parallel.results",
    "human.gate.selected",
    "human.gate.label",
    "human.gate.note",
    "tool.output",
    "tool.exit_code",
    "stack.child.run_id",
    "stack.child.status",
    "stack.child.outcome",
    "stack.child.outcome_reason_code",
    "stack.child.outcome_reason_message",
    "stack.child.active_stage",
    "stack.child.completed_nodes",
    "stack.child.route_trace",
    "stack.child.failure_reason",
    "stack.child.failure_kind",
    "stack.child.retry_count",
    "stack.child.retry_counts",
    "stack.child.artifact_count",
    "stack.child.event_count",
    "stack.child.checkpoint_timestamp",
    "stack.child.latest_event_at",
    "stack.child.started_at",
    "stack.child.ended_at",
    "stack.child.intervention",
    "stack.child.intervention_status",
    "stack.child.intervention_delivery_mode",
    "stack.child.intervention_reason",
];

/// The readable Spark keys starting with `prefix`, such as `stack.child.`.
pub fn readable_spark_keys(prefix: &str) -> impl Iterator<Item = &'static str> + '_ {
    READABLE_SPARK_KEYS
        .iter()
        .copied()
        .filter(move |key| key.starts_with(prefix))
}

const ALLOWED_LAUNCH_CONTEXT_KEYS: &[&str] = &[
    "_attractor.runtime.launch_model",
    "_attractor.runtime.launch_provider",
    "_attractor.runtime.launch_profile",
    "_attractor.runtime.launch_reasoning_effort",
];

/// Live named-session handles (`session: <name>` on agent nodes). Runtime
/// owned: exempt from write contracts, and never written by agents.
pub const RUNTIME_SESSIONS_KEY: &str = "_attractor.runtime.sessions";
/// Runtime-only: the current node's route targets that used all their
/// `max_visits`, set on each node request so its prompt marks them unavailable.
pub const RUNTIME_UNAVAILABLE_ROUTES_KEY: &str = "_attractor.runtime.unavailable_routes";

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
        if !key.starts_with("context.") && !ALLOWED_LAUNCH_CONTEXT_KEYS.contains(&key.as_str()) {
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
