use std::str::FromStr;

use attractor_core::{
    is_exact_outcome_fail_condition, normalize_label, select_failure_route_edge_with_context,
    AttractorContext, FlowDefinition, Outcome, OutcomeStatus, RoutingEdge,
};
use serde::{Deserialize, Serialize};

use crate::context::{OUTCOME_KEY, PREFERRED_LABEL_KEY};
use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NextNodeSelection {
    pub current_node: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_node: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_edge: Option<RoutingEdge>,
    pub reason: String,
}

pub fn resolve_start_node(flow: &FlowDefinition) -> Result<String> {
    crate::flow_runtime::resolve_start_node(flow)
}

pub fn is_exit_node(flow: &FlowDefinition, node_id: &str) -> bool {
    crate::flow_runtime::is_exit_node(flow, node_id)
}

pub fn outgoing_routing_edges(flow: &FlowDefinition, node_id: &str) -> Result<Vec<RoutingEdge>> {
    crate::flow_runtime::outgoing_routing_edges(flow, node_id)
}

pub fn routing_outcome_for_node(
    flow: &FlowDefinition,
    node_id: &str,
    outcome: &Outcome,
    context: &AttractorContext,
) -> Outcome {
    let prior_status = context
        .get(OUTCOME_KEY)
        .and_then(|value| value.as_str())
        .and_then(|value| OutcomeStatus::from_str(value.trim()).ok());
    let prior_preferred_label = context
        .get(PREFERRED_LABEL_KEY)
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    routing_outcome_for_node_with_prior(flow, node_id, outcome, prior_status, prior_preferred_label)
}

pub fn routing_outcome_for_node_with_prior(
    flow: &FlowDefinition,
    node_id: &str,
    outcome: &Outcome,
    prior_status: Option<OutcomeStatus>,
    prior_preferred_label: &str,
) -> Outcome {
    if !is_conditional_node(flow, node_id) {
        return outcome.clone();
    }
    let Some(prior_status) = prior_status else {
        return outcome.clone();
    };
    Outcome {
        status: prior_status,
        preferred_label: prior_preferred_label.to_string(),
        suggested_next_ids: outcome.suggested_next_ids.clone(),
        context_updates: outcome.context_updates.clone(),
        failure_reason: outcome.failure_reason.clone(),
        notes: outcome.notes.clone(),
        retryable: outcome.retryable,
        failure_kind: outcome.failure_kind,
        raw_response_text: outcome.raw_response_text.clone(),
    }
}

pub fn select_next_node(
    flow: &FlowDefinition,
    node_id: &str,
    outcome: &Outcome,
    context: &AttractorContext,
) -> Result<NextNodeSelection> {
    let prior_status = context
        .get(OUTCOME_KEY)
        .and_then(|value| value.as_str())
        .and_then(|value| OutcomeStatus::from_str(value.trim()).ok());
    let prior_preferred_label = context
        .get(PREFERRED_LABEL_KEY)
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    select_next_node_with_prior(
        flow,
        node_id,
        outcome,
        context,
        prior_status,
        prior_preferred_label,
    )
}

pub fn select_next_node_with_prior(
    flow: &FlowDefinition,
    node_id: &str,
    outcome: &Outcome,
    context: &AttractorContext,
    prior_status: Option<OutcomeStatus>,
    prior_preferred_label: &str,
) -> Result<NextNodeSelection> {
    let routing_edges = outgoing_routing_edges(flow, node_id)?;
    let routing_outcome = routing_outcome_for_node_with_prior(
        flow,
        node_id,
        outcome,
        prior_status,
        prior_preferred_label,
    );
    let selected_edge =
        select_failure_route_edge_with_context(&routing_edges, &routing_outcome, context);
    let reason = selected_edge
        .as_ref()
        .map(|edge| selection_reason(edge, &routing_outcome))
        .unwrap_or_else(|| "no_route".to_string());
    Ok(NextNodeSelection {
        current_node: node_id.to_string(),
        selected_node: selected_edge
            .as_ref()
            .map(|edge| edge.target.as_str().to_string()),
        selected_edge,
        reason,
    })
}

pub fn is_conditional_node(flow: &FlowDefinition, node_id: &str) -> bool {
    crate::flow_runtime::is_conditional_node(flow, node_id)
}

fn selection_reason(edge: &RoutingEdge, outcome: &Outcome) -> String {
    if !edge.condition_text().is_empty() {
        if outcome.status == OutcomeStatus::Fail
            && is_exact_outcome_fail_condition(edge.condition_text())
        {
            return "failure_condition".to_string();
        }
        return "condition".to_string();
    }
    if !outcome.preferred_label.trim().is_empty()
        && normalize_label(&edge.label) == normalize_label(&outcome.preferred_label)
    {
        return "preferred_label".to_string();
    }
    if outcome
        .suggested_next_ids
        .iter()
        .any(|node_id| node_id == edge.target.as_str())
    {
        return "suggested_next_id".to_string();
    }
    "unconditional".to_string()
}
