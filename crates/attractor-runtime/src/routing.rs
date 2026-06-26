use std::str::FromStr;

use attractor_core::{
    attr_string, is_exact_outcome_fail_condition, node_shape, normalize_label,
    routing_edge_from_dot_edge, select_failure_route_edge_with_context, AttractorContext, DotGraph,
    Outcome, OutcomeStatus, RoutingEdge,
};
use serde::{Deserialize, Serialize};

use crate::context::{OUTCOME_KEY, PREFERRED_LABEL_KEY};
use crate::error::{Result, RuntimeStorageError};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NextNodeSelection {
    pub current_node: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_node: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_edge: Option<RoutingEdge>,
    pub reason: String,
}

pub fn resolve_start_node(graph: &DotGraph) -> Result<String> {
    let starts = graph
        .nodes
        .values()
        .filter(|node| node_shape(node) == "Mdiamond")
        .map(|node| node.node_id.clone())
        .collect::<Vec<_>>();
    match starts.as_slice() {
        [start] => Ok(start.clone()),
        [] => {
            for candidate in ["start", "Start"] {
                if graph.nodes.contains_key(candidate) {
                    return Ok(candidate.to_string());
                }
            }
            Err(RuntimeStorageError::InvalidRuntimeGraph {
                reason: "No start node found; expected shape=Mdiamond or node id start/Start"
                    .to_string(),
            })
        }
        _ => Err(RuntimeStorageError::InvalidRuntimeGraph {
            reason: format!("Ambiguous start nodes: {}", sorted_join(starts)),
        }),
    }
}

pub fn is_exit_node(graph: &DotGraph, node_id: &str) -> bool {
    let shape_exit_nodes = graph
        .nodes
        .values()
        .filter(|node| node_shape(node) == "Msquare")
        .map(|node| node.node_id.as_str())
        .collect::<Vec<_>>();
    if !shape_exit_nodes.is_empty() {
        return shape_exit_nodes.contains(&node_id);
    }
    matches!(node_id, "exit" | "end" | "Exit" | "End")
}

pub fn outgoing_routing_edges(graph: &DotGraph, node_id: &str) -> Result<Vec<RoutingEdge>> {
    graph
        .edges
        .iter()
        .filter(|edge| edge.source == node_id)
        .map(routing_edge_from_dot_edge)
        .collect::<attractor_core::Result<Vec<_>>>()
        .map_err(RuntimeStorageError::from)
}

pub fn routing_outcome_for_node(
    graph: &DotGraph,
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
    routing_outcome_for_node_with_prior(
        graph,
        node_id,
        outcome,
        prior_status,
        prior_preferred_label,
    )
}

pub fn routing_outcome_for_node_with_prior(
    graph: &DotGraph,
    node_id: &str,
    outcome: &Outcome,
    prior_status: Option<OutcomeStatus>,
    prior_preferred_label: &str,
) -> Outcome {
    if !is_conditional_node(graph, node_id) {
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
    graph: &DotGraph,
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
        graph,
        node_id,
        outcome,
        context,
        prior_status,
        prior_preferred_label,
    )
}

pub fn select_next_node_with_prior(
    graph: &DotGraph,
    node_id: &str,
    outcome: &Outcome,
    context: &AttractorContext,
    prior_status: Option<OutcomeStatus>,
    prior_preferred_label: &str,
) -> Result<NextNodeSelection> {
    let routing_edges = outgoing_routing_edges(graph, node_id)?;
    let routing_outcome = routing_outcome_for_node_with_prior(
        graph,
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

pub fn is_conditional_node(graph: &DotGraph, node_id: &str) -> bool {
    let Some(node) = graph.nodes.get(node_id) else {
        return false;
    };
    let explicit_type = attr_string(&node.attrs, "type");
    if !explicit_type.trim().is_empty() {
        return explicit_type.trim() == "conditional";
    }
    node_shape(node) == "diamond"
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

fn sorted_join(mut values: Vec<String>) -> String {
    values.sort();
    values.join(", ")
}
