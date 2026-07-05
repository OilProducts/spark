use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AttractorCoreError, Result};

macro_rules! string_id {
    ($name:ident, $kind:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self> {
                let value = value.into();
                let normalized = value.trim();
                if normalized.is_empty() {
                    return Err(AttractorCoreError::InvalidIdentifier {
                        kind: $kind,
                        value,
                        reason: "identifier must be non-empty".to_string(),
                    });
                }
                Ok(Self(normalized.to_string()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = AttractorCoreError;

            fn try_from(value: String) -> Result<Self> {
                Self::new(value)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = AttractorCoreError;

            fn try_from(value: &str) -> Result<Self> {
                Self::new(value)
            }
        }

        impl FromStr for $name {
            type Err = AttractorCoreError;

            fn from_str(value: &str) -> Result<Self> {
                Self::new(value)
            }
        }
    };
}

string_id!(RunId, "run");
string_id!(StageId, "stage");
string_id!(CheckpointId, "checkpoint");

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct JournalSequence(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeNodeState {
    Pending,
    Running,
    Waiting,
    Completed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Pending,
    Running,
    Waiting,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunExecutionLock {
    pub scope: String,
    pub key: String,
    pub conflict_policy: String,
    pub identity: String,
    pub state: String,
    #[serde(default)]
    pub queue_position: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub flow_name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub outcome: Option<String>,
    #[serde(default)]
    pub outcome_reason_code: Option<String>,
    #[serde(default)]
    pub outcome_reason_message: Option<String>,
    #[serde(default)]
    pub working_directory: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub llm_provider: String,
    #[serde(default)]
    pub llm_profile: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub ended_at: Option<String>,
    #[serde(default)]
    pub project_path: String,
    #[serde(default)]
    pub git_branch: Option<String>,
    #[serde(default)]
    pub git_commit: Option<String>,
    #[serde(default)]
    pub spec_id: Option<String>,
    #[serde(default)]
    pub plan_id: Option<String>,
    #[serde(default)]
    pub continued_from_run_id: Option<String>,
    #[serde(default)]
    pub continued_from_node: Option<String>,
    #[serde(default)]
    pub continued_from_flow_mode: Option<String>,
    #[serde(default)]
    pub continued_from_flow_name: Option<String>,
    #[serde(default)]
    pub parent_run_id: Option<String>,
    #[serde(default)]
    pub parent_node_id: Option<String>,
    #[serde(default)]
    pub root_run_id: Option<String>,
    #[serde(default)]
    pub child_invocation_index: Option<u64>,
    #[serde(default = "default_execution_mode")]
    pub execution_mode: String,
    #[serde(default)]
    pub execution_profile_id: Option<String>,
    #[serde(default)]
    pub execution_container_image: Option<String>,
    #[serde(default)]
    pub execution_profile_capabilities: Option<Value>,
    #[serde(default)]
    pub execution_lock: Option<RunExecutionLock>,
    #[serde(default)]
    pub cleanup_error: Option<String>,
    #[serde(default)]
    pub last_error: String,
    #[serde(default)]
    pub token_usage: Option<u64>,
    #[serde(default)]
    pub token_usage_breakdown: Option<Value>,
    #[serde(default)]
    pub estimated_model_cost: Option<Value>,
}

impl RunRecord {
    pub fn new(run_id: impl Into<String>, project_path: impl Into<String>) -> Self {
        let run_id = run_id.into();
        let project_path = project_path.into();
        Self {
            run_id: run_id.clone(),
            status: "running".to_string(),
            provider: "codex".to_string(),
            llm_provider: "codex".to_string(),
            project_path: project_path.clone(),
            working_directory: project_path,
            root_run_id: Some(run_id),
            execution_mode: default_execution_mode(),
            ..Self::default()
        }
    }
}

fn default_execution_mode() -> String {
    "native".to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckpointState {
    #[serde(default)]
    pub timestamp: String,
    pub current_node: String,
    #[serde(default)]
    pub completed_nodes: Vec<String>,
    #[serde(default)]
    pub context: std::collections::BTreeMap<String, Value>,
    #[serde(default)]
    pub retry_counts: std::collections::BTreeMap<String, u64>,
    #[serde(default)]
    pub logs: Vec<String>,
}

impl CheckpointState {
    pub fn new(current_node: impl Into<String>) -> Self {
        Self {
            timestamp: String::new(),
            current_node: current_node.into(),
            completed_nodes: Vec::new(),
            context: std::collections::BTreeMap::new(),
            retry_counts: std::collections::BTreeMap::new(),
            logs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RawRuntimeEvent {
    #[serde(default)]
    pub sequence: Option<u64>,
    #[serde(rename = "type", default)]
    pub event_type: String,
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub emitted_at: String,
    #[serde(flatten, default)]
    pub payload: std::collections::BTreeMap<String, Value>,
}

impl RawRuntimeEvent {
    pub fn new(event_type: impl Into<String>, run_id: impl Into<String>) -> Self {
        Self {
            sequence: None,
            event_type: event_type.into(),
            run_id: run_id.into(),
            emitted_at: String::new(),
            payload: std::collections::BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub id: String,
    pub sequence: u64,
    pub emitted_at: String,
    pub kind: String,
    pub raw_type: String,
    pub severity: String,
    pub summary: String,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub stage_index: Option<u64>,
    #[serde(default)]
    pub source_scope: String,
    #[serde(default)]
    pub source_parent_node_id: Option<String>,
    #[serde(default)]
    pub source_flow_name: Option<String>,
    #[serde(default)]
    pub question_id: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunManifest {
    #[serde(default)]
    pub goal: String,
    #[serde(default)]
    pub graph_id: String,
    #[serde(default)]
    pub start_node: String,
    #[serde(default)]
    pub started_at: String,
    #[serde(flatten, default)]
    pub extra: std::collections::BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunResult {
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub source_node_id: Option<String>,
    #[serde(default)]
    pub source_artifact_path: Option<String>,
    #[serde(default)]
    pub display_mode: Option<String>,
    #[serde(default)]
    pub body_markdown: String,
    #[serde(default)]
    pub summary_enabled: bool,
    #[serde(default)]
    pub summary_prompt: Option<String>,
    #[serde(default)]
    pub summary_error: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

impl RunResult {
    pub fn pending(run_id: impl Into<String>, status: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            status: status.into(),
            state: "pending".to_string(),
            ..Self::default()
        }
    }

    pub fn unavailable(run_id: impl Into<String>, status: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            status: status.into(),
            state: "unavailable".to_string(),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactInfo {
    pub path: String,
    pub size_bytes: u64,
    pub media_type: String,
    pub viewable: bool,
}
