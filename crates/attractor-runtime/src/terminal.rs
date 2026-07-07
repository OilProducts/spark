use attractor_core::{AttractorContext, FlowDefinition};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::context::{
    NODE_OUTCOMES_KEY, WORKFLOW_OUTCOME_KEY, WORKFLOW_OUTCOME_REASON_CODE_KEY,
    WORKFLOW_OUTCOME_REASON_MESSAGE_KEY,
};

pub const GOAL_GATE_NO_RETRY_TARGET_REASON: &str = "Goal gate unsatisfied and no retry target";
pub const GOAL_GATE_UNSATISFIED_OUTCOME_CODE: &str = "goal_gate_unsatisfied";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalWorkflowOutcome {
    pub outcome: String,
    pub outcome_reason_code: Option<String>,
    pub outcome_reason_message: Option<String>,
}

pub fn resolve_terminal_workflow_outcome(
    context: &AttractorContext,
    default_outcome: &str,
    default_reason_code: Option<&str>,
    default_reason_message: Option<&str>,
) -> Option<TerminalWorkflowOutcome> {
    let raw_outcome = context_optional_text(context, WORKFLOW_OUTCOME_KEY);
    let outcome = match raw_outcome {
        Some(value) => {
            let normalized = value.to_ascii_lowercase();
            if !matches!(normalized.as_str(), "success" | "failure") {
                return None;
            }
            normalized
        }
        None => default_outcome.to_string(),
    };

    Some(TerminalWorkflowOutcome {
        outcome,
        outcome_reason_code: context_optional_text(context, WORKFLOW_OUTCOME_REASON_CODE_KEY)
            .or_else(|| default_reason_code.map(str::to_string)),
        outcome_reason_message: context_optional_text(context, WORKFLOW_OUTCOME_REASON_MESSAGE_KEY)
            .or_else(|| default_reason_message.map(str::to_string)),
    })
}

pub fn invalid_workflow_outcome_reason(context: &AttractorContext) -> String {
    let value = context_optional_text(context, WORKFLOW_OUTCOME_KEY)
        .unwrap_or_else(|| "<empty>".to_string());
    format!("invalid context.workflow_outcome: {value}")
}

pub fn check_goal_gates(
    flow: &FlowDefinition,
    context: &AttractorContext,
    completed_nodes: &[String],
) -> GoalGateCheck {
    let Some(statuses) = context.get(NODE_OUTCOMES_KEY).and_then(Value::as_object) else {
        return GoalGateCheck::satisfied();
    };

    for node_id in completed_nodes {
        let Some(node) = flow.nodes.get(node_id) else {
            continue;
        };
        if !crate::flow_runtime::node_attr_bool(node, "goal_gate", false) {
            continue;
        }
        let status = statuses
            .get(node_id)
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !matches!(status, "success" | "partial_success") {
            return GoalGateCheck {
                satisfied: false,
                failed_node_id: Some(node_id.clone()),
            };
        }
    }

    GoalGateCheck::satisfied()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalGateCheck {
    pub satisfied: bool,
    pub failed_node_id: Option<String>,
}

impl GoalGateCheck {
    pub fn satisfied() -> Self {
        Self {
            satisfied: true,
            failed_node_id: None,
        }
    }
}

pub fn resolve_failure_retry_target(flow: &FlowDefinition, node_id: &str) -> Option<String> {
    let node = flow.nodes.get(node_id)?;
    for key in ["retry_target", "fallback_retry_target"] {
        if let Some(target) = crate::flow_runtime::node_attr_text(node, key) {
            if flow.nodes.contains_key(&target) {
                return Some(target);
            }
        }
    }
    None
}

pub fn resolve_goal_gate_retry_target(
    flow: &FlowDefinition,
    failed_gate_node: &str,
) -> Option<String> {
    resolve_failure_retry_target(flow, failed_gate_node)
        .or_else(|| resolve_graph_retry_target(flow))
}

fn resolve_graph_retry_target(flow: &FlowDefinition) -> Option<String> {
    for key in ["retry_target", "fallback_retry_target"] {
        if let Some(target) = crate::flow_runtime::flow_attr_text(flow, key) {
            if flow.nodes.contains_key(&target) {
                return Some(target);
            }
        }
    }
    None
}

pub fn is_goal_gate_node(flow: &FlowDefinition, node_id: &str) -> bool {
    flow.nodes
        .get(node_id)
        .is_some_and(|node| crate::flow_runtime::node_attr_bool(node, "goal_gate", false))
}

fn context_optional_text(context: &AttractorContext, key: &str) -> Option<String> {
    let value = context.get(key)?;
    match value {
        Value::Null => None,
        Value::String(value) => non_empty(value),
        Value::Number(value) => non_empty(&value.to_string()),
        Value::Bool(value) => non_empty(&value.to_string()),
        Value::Array(_) | Value::Object(_) => non_empty(&value.to_string()),
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}
