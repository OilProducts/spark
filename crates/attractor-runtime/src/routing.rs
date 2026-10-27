use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use attractor_core::{
    is_exact_outcome_fail_condition, normalize_label, select_failure_route_edge_with_context,
    AttractorContext, ContextMap, FlowDefinition, Outcome, OutcomeStatus, RoutingEdge,
    RUNTIME_UNAVAILABLE_ROUTES_KEY,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

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

/// Engine default for how many times routing may enter one node in a run.
/// CR-0100's own implement-change run legitimately needed eleven implement
/// entries; the default leaves ample headroom while ruling out unbounded spins.
pub const DEFAULT_MAX_VISITS: u64 = 32;

pub const VISIT_LIMIT_REACHED: &str = "visit_limit_reached";

pub fn max_visits(flow: &FlowDefinition, node_id: &str) -> u64 {
    flow.nodes
        .get(node_id)
        .and_then(|node| node.runtime.as_ref())
        .and_then(|runtime| runtime.max_visits)
        .filter(|limit| *limit > 0)
        .unwrap_or(DEFAULT_MAX_VISITS)
}

/// The visit-limit check every routing path shares: a target that used all
/// its `max_visits` is not eligible.
pub fn visits_exhausted(
    flow: &FlowDefinition,
    visits: &BTreeMap<String, u64>,
    target: &str,
) -> bool {
    visits.get(target).copied().unwrap_or(0) >= max_visits(flow, target)
}

pub fn visit_limit_reason(flow: &FlowDefinition, target: &str, from_node: &str) -> String {
    format!(
        "{VISIT_LIMIT_REACHED}: {target} used all {} visits; no other route from {from_node}",
        max_visits(flow, target)
    )
}

/// The first candidate with visits left, or the `visit_limit_reached` reason
/// when every candidate is exhausted. Used for retry-target fallbacks.
pub fn first_target_with_visits_left(
    flow: &FlowDefinition,
    from_node: &str,
    visits: &BTreeMap<String, u64>,
    candidates: impl IntoIterator<Item = String>,
) -> std::result::Result<Option<String>, String> {
    let mut exhausted = None;
    for target in candidates {
        if !visits_exhausted(flow, visits, &target) {
            return Ok(Some(target));
        }
        exhausted.get_or_insert(target);
    }
    match exhausted {
        Some(target) => Err(visit_limit_reason(flow, &target, from_node)),
        None => Ok(None),
    }
}

/// Puts the node's exhausted route targets under the runtime-only
/// `RUNTIME_UNAVAILABLE_ROUTES_KEY`, so its prompt marks those routes.
pub fn mark_unavailable_routes(
    context: &mut ContextMap,
    flow: &FlowDefinition,
    edges: &[RoutingEdge],
    visits: &BTreeMap<String, u64>,
) {
    let unavailable = edges
        .iter()
        .map(|edge| edge.target.as_str())
        .filter(|target| visits_exhausted(flow, visits, target))
        .collect::<BTreeSet<_>>();
    if unavailable.is_empty() {
        context.remove(RUNTIME_UNAVAILABLE_ROUTES_KEY);
    } else {
        context.insert(
            RUNTIME_UNAVAILABLE_ROUTES_KEY.to_string(),
            json!(unavailable),
        );
    }
}

/// Selects the next edge among targets with visits left. When none is
/// eligible but an exhausted one would have been taken, `selected_node` is
/// empty, `selected_edge` is that edge and `reason` is `visit_limit_reached`.
pub fn select_next_node_with_prior(
    flow: &FlowDefinition,
    node_id: &str,
    outcome: &Outcome,
    context: &AttractorContext,
    prior_status: Option<OutcomeStatus>,
    prior_preferred_label: &str,
    visits: &BTreeMap<String, u64>,
) -> Result<NextNodeSelection> {
    let routing_edges = outgoing_routing_edges(flow, node_id)?;
    let routing_outcome = routing_outcome_for_node_with_prior(
        flow,
        node_id,
        outcome,
        prior_status,
        prior_preferred_label,
    );
    let (eligible, exhausted): (Vec<_>, Vec<_>) = routing_edges
        .into_iter()
        .partition(|edge| !visits_exhausted(flow, visits, edge.target.as_str()));
    if let Some(edge) = select_failure_route_edge_with_context(&eligible, &routing_outcome, context)
    {
        return Ok(NextNodeSelection {
            current_node: node_id.to_string(),
            selected_node: Some(edge.target.as_str().to_string()),
            reason: selection_reason(&edge, &routing_outcome),
            selected_edge: Some(edge),
        });
    }
    let blocked = select_failure_route_edge_with_context(&exhausted, &routing_outcome, context);
    Ok(NextNodeSelection {
        current_node: node_id.to_string(),
        selected_node: None,
        reason: blocked
            .as_ref()
            .map(|edge| visit_limit_reason(flow, edge.target.as_str(), node_id))
            .unwrap_or_else(|| "no_route".to_string()),
        selected_edge: blocked,
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
