use std::path::PathBuf;

use attractor_core::{ContextMap, FailureKind, FlowDefinition, Outcome, RawRuntimeEvent};
use attractor_runtime::{
    ChildInterventionRequest, ChildInterventionResult, ChildRunRequest, ChildRunResult,
    HumanAnswer, HumanQuestion,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkerNodeRequest {
    pub run_id: String,
    pub flow: FlowDefinition,
    pub node_id: String,
    pub stage_index: u64,
    pub attempt: u64,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub context: ContextMap,
    #[serde(default)]
    pub context_logs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logs_root: Option<PathBuf>,
    #[serde(default)]
    pub working_dir: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_root: Option<RunRootMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRootMetadata {
    pub runs_root: spark_common::project::RunsRoot,
    pub project_id: String,
    pub root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WorkerFrame {
    #[serde(rename = "result")]
    Result(ResultFrame),
    #[serde(rename = "event")]
    Event(EventFrame),
    #[serde(rename = "human_gate_request")]
    HumanGateRequest(HumanGateRequestFrame),
    #[serde(rename = "human_gate_answer")]
    HumanGateAnswer(HumanGateAnswerFrame),
    #[serde(rename = "child_run_request")]
    ChildRunRequest(ChildRunRequestFrame),
    #[serde(rename = "child_run_result")]
    ChildRunResult(ChildRunResultFrame),
    #[serde(rename = "child_status_request")]
    ChildStatusRequest(ChildStatusRequestFrame),
    #[serde(rename = "child_status_result")]
    ChildStatusResult(ChildStatusResultFrame),
    #[serde(rename = "child_intervention_request")]
    ChildInterventionRequest(ChildInterventionRequestFrame),
    #[serde(rename = "child_intervention_result")]
    ChildInterventionResult(ChildInterventionResultFrame),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResultFrame {
    pub outcome: Value,
    #[serde(default)]
    pub context: ContextMap,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventFrame {
    pub event: RawRuntimeEvent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HumanGateRequestFrame {
    pub question: HumanQuestion,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HumanGateAnswerFrame {
    pub answer: HumanAnswer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChildRunRequestFrame {
    #[serde(flatten)]
    pub request: ChildRunRequest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChildRunResultFrame {
    pub result: ChildRunResult,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChildStatusRequestFrame {
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChildStatusResultFrame {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ChildRunResult>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChildInterventionRequestFrame {
    #[serde(flatten)]
    pub request: ChildInterventionRequest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChildInterventionResultFrame {
    pub result: ChildInterventionResult,
}

/// A worker's node result. The frame is the core `Outcome` itself, so the
/// host reads exactly what the worker's node produced.
pub fn outcome_to_payload(outcome: &Outcome) -> Value {
    serde_json::to_value(outcome).unwrap_or_else(|_| json!({}))
}

pub fn outcome_from_payload(payload: &Value) -> Outcome {
    serde_json::from_value(payload.clone()).unwrap_or_else(|error| {
        Outcome::error(
            FailureKind::Runtime,
            format!("worker returned invalid outcome: {error}"),
            Some(false),
        )
    })
}
