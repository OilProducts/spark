use std::collections::{BTreeMap, BTreeSet};

use attractor_core::{
    ContextMap, FlowDefinition, NodeConfig, Outcome, RoutingEdge, RUNTIME_UNAVAILABLE_ROUTES_KEY,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::Result;

/// The transition a completed invocation selected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition {
    pub from: String,
    /// The declared decision; empty for a fork entering its join.
    pub decision: String,
    pub target: String,
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

/// Engine default for how many times transitions may enter one node in a
/// run. CR-0100's own implement-change run legitimately needed eleven
/// implement entries; the default leaves ample headroom while ruling out
/// unbounded spins.
pub const DEFAULT_MAX_VISITS: u64 = 32;

pub const VISIT_LIMIT_REACHED: &str = "visit_limit_reached";
pub const NO_MATCHING_ROUTE: &str = "no_matching_route";

pub fn max_visits(flow: &FlowDefinition, node_id: &str) -> u64 {
    flow.nodes
        .get(node_id)
        .and_then(|node| node.runtime.as_ref())
        .and_then(|runtime| runtime.max_visits)
        .filter(|limit| *limit > 0)
        .unwrap_or(DEFAULT_MAX_VISITS)
}

/// Whether `target` used all its `max_visits`.
pub fn visits_exhausted(
    flow: &FlowDefinition,
    visits: &BTreeMap<String, u64>,
    target: &str,
) -> bool {
    visits.get(target).copied().unwrap_or(0) >= max_visits(flow, target)
}

/// The one dispatch rule every runner shares: look the completed
/// invocation's decision up in its node's route map (a fork enters its
/// join), then check that target's visit limit. An unknown decision or an
/// exhausted target is a routing failure; neither ever selects another
/// route.
pub fn select_transition(
    flow: &FlowDefinition,
    node_id: &str,
    outcome: &Outcome,
    visits: &BTreeMap<String, u64>,
) -> std::result::Result<Transition, String> {
    let node = flow
        .nodes
        .get(node_id)
        .ok_or_else(|| format!("{NO_MATCHING_ROUTE}: unknown node {node_id:?}"))?;
    let (decision, target) = match node.config.as_ref() {
        Some(NodeConfig::Parallel { join, .. }) => (String::new(), join.clone()),
        _ => match node.routes.get(&outcome.decision) {
            Some(route) => (outcome.decision.clone(), route.to.clone()),
            None => return Err(no_matching_route_reason(flow, node_id, &outcome.decision)),
        },
    };
    let used = visits.get(&target).copied().unwrap_or(0);
    let limit = max_visits(flow, &target);
    if used >= limit {
        return Err(format!(
            "{VISIT_LIMIT_REACHED}: node {node_id:?} decision {decision:?} selects {target:?}, which used {used} of its {limit} visits"
        ));
    }
    Ok(Transition {
        from: node_id.to_string(),
        decision,
        target,
    })
}

/// Why a returned decision selects nothing: the decision and the node's
/// full declared decision-to-target set.
pub fn no_matching_route_reason(flow: &FlowDefinition, node_id: &str, decision: &str) -> String {
    let routes = flow
        .nodes
        .get(node_id)
        .map(|node| {
            node.routes
                .iter()
                .map(|(decision, route)| format!("{decision:?} -> {:?}", route.to))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    format!(
        "{NO_MATCHING_ROUTE}: node {node_id:?} returned decision {decision:?}; declared decisions [{}]",
        routes.join(", ")
    )
}

/// Records the node's exhausted route targets under the runtime-only
/// `RUNTIME_UNAVAILABLE_ROUTES_KEY`: factual budget context for its prompt.
/// Exhausted decisions stay declared; choosing one fails the run.
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

pub fn stage_failure_reason(outcome: &Outcome) -> String {
    let reason = outcome.failure_reason.trim();
    if reason.is_empty() {
        "stage_failed".to_string()
    } else {
        reason.to_string()
    }
}
