use std::collections::BTreeMap;
use std::path::PathBuf;

use attractor_core::{ContextMap, FailureKind, FlowDefinition, Outcome, OutcomeStatus};
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
    pub event_type: String,
    #[serde(default)]
    pub payload: BTreeMap<String, Value>,
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

pub fn outcome_to_payload(outcome: &Outcome) -> Value {
    let mut payload = json!({
        "status": outcome.status.as_str(),
        "preferred_label": outcome.preferred_label,
        "suggested_next_ids": outcome.suggested_next_ids,
        "context_updates": outcome.context_updates,
        "failure_reason": outcome.failure_reason,
        "notes": outcome.notes,
        "retryable": outcome.retryable,
        "raw_response_text": outcome.raw_response_text,
    });
    if let Some(failure_kind) = outcome.failure_kind {
        payload["failure_kind"] = json!(failure_kind.as_str());
    }
    payload
}

pub fn outcome_from_payload(payload: &Value) -> Outcome {
    let Some(object) = payload.as_object() else {
        return Outcome {
            status: OutcomeStatus::Fail,
            failure_reason: "worker returned invalid outcome".to_string(),
            retryable: Some(false),
            failure_kind: Some(FailureKind::Runtime),
            ..Outcome::new(OutcomeStatus::Fail)
        };
    };
    let status = object
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("fail")
        .parse::<OutcomeStatus>()
        .unwrap_or(OutcomeStatus::Fail);
    let context_updates = object
        .get("context_updates")
        .and_then(Value::as_object)
        .map(|items| {
            items
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default();
    Outcome {
        status,
        preferred_label: string_field(object, "preferred_label"),
        suggested_next_ids: object
            .get("suggested_next_ids")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        context_updates,
        failure_reason: string_field(object, "failure_reason"),
        notes: string_field(object, "notes"),
        retryable: object.get("retryable").and_then(Value::as_bool),
        failure_kind: object
            .get("failure_kind")
            .and_then(Value::as_str)
            .and_then(parse_failure_kind),
        raw_response_text: string_field(object, "raw_response_text"),
    }
}

fn string_field(object: &serde_json::Map<String, Value>, key: &str) -> String {
    object
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn parse_failure_kind(value: &str) -> Option<FailureKind> {
    match value {
        "business" => Some(FailureKind::Business),
        "contract" => Some(FailureKind::Contract),
        "runtime" => Some(FailureKind::Runtime),
        _ => None,
    }
}
