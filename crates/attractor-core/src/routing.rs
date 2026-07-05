use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::conditions::evaluate_condition;
use crate::context::AttractorContext;
use crate::graph::{NodeId, RoutingEdge};
use crate::outcomes::{Outcome, OutcomeStatus};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuggestedNextIds(pub Vec<NodeId>);

impl SuggestedNextIds {
    pub fn as_slice(&self) -> &[NodeId] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NextNodeSuggestion {
    pub node_id: NodeId,
    pub rank: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutingInput {
    pub outcome: Outcome,
    pub context: AttractorContext,
    #[serde(default)]
    pub candidate_edges: Vec<RoutingEdge>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutingDecision {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_edge: Option<RoutingEdge>,
    #[serde(default)]
    pub reason: String,
}

pub fn normalize_label(label: &str) -> String {
    let mut text = label.trim().to_lowercase();
    if text.starts_with('[') {
        if let Some(index) = text.find(']') {
            text = text[index + 1..].trim().to_string();
        }
    }
    let mut chars = text.chars();
    let first = chars.next();
    let second = chars.next();
    if matches!(second, Some(')')) && first.is_some_and(|ch| ch.is_ascii_alphanumeric()) {
        text = chars.as_str().trim().to_string();
    }
    let mut chars = text.chars();
    let first = chars.next();
    let second = chars.next();
    let third = chars.next();
    if first.is_some_and(|ch| ch.is_ascii_alphanumeric())
        && matches!(second, Some(' '))
        && matches!(third, Some('-'))
    {
        text = chars.as_str().trim().to_string();
    }
    text
}

pub fn best_edge_by_weight_then_lexical(edges: &[RoutingEdge]) -> Option<RoutingEdge> {
    edges
        .iter()
        .min_by(|left, right| {
            right
                .weight
                .cmp(&left.weight)
                .then_with(|| left.target.as_str().cmp(right.target.as_str()))
        })
        .cloned()
}

pub fn select_next_edge(edges: &[RoutingEdge], outcome: &Outcome) -> Option<RoutingEdge> {
    select_next_edge_with_context(edges, outcome, &AttractorContext::new())
}

pub fn select_next_edge_with_context(
    edges: &[RoutingEdge],
    outcome: &Outcome,
    context: &AttractorContext,
) -> Option<RoutingEdge> {
    let condition_results = edges
        .iter()
        .filter_map(|edge| {
            let condition = edge.condition_text();
            (!condition.is_empty()).then(|| {
                (
                    condition.to_string(),
                    evaluate_condition(condition, outcome, context),
                )
            })
        })
        .collect();
    select_next_edge_with_condition_results(edges, outcome, &condition_results)
}

pub fn select_next_edge_with_condition_results(
    edges: &[RoutingEdge],
    outcome: &Outcome,
    condition_results: &BTreeMap<String, bool>,
) -> Option<RoutingEdge> {
    let condition_matched: Vec<_> = edges
        .iter()
        .filter(|edge| {
            let condition = edge.condition_text();
            !condition.is_empty() && condition_results.get(condition).copied().unwrap_or(false)
        })
        .cloned()
        .collect();
    if let Some(edge) = best_edge_by_weight_then_lexical(&condition_matched) {
        return Some(edge);
    }

    let preferred = outcome.preferred_label.trim();
    if !preferred.is_empty() {
        let normalized_preferred = normalize_label(preferred);
        if let Some(edge) = edges.iter().find(|edge| {
            edge.condition_text().is_empty() && normalize_label(&edge.label) == normalized_preferred
        }) {
            return Some(edge.clone());
        }
    }

    if !outcome.suggested_next_ids.is_empty() {
        for suggested_id in &outcome.suggested_next_ids {
            if let Some(edge) = edges.iter().find(|edge| {
                edge.condition_text().is_empty() && edge.target.as_str() == suggested_id
            }) {
                return Some(edge.clone());
            }
        }
    }

    let unconditional: Vec<_> = edges
        .iter()
        .filter(|edge| edge.condition_text().is_empty())
        .cloned()
        .collect();
    best_edge_by_weight_then_lexical(&unconditional)
}

pub fn select_failure_route_edge_with_context(
    edges: &[RoutingEdge],
    outcome: &Outcome,
    context: &AttractorContext,
) -> Option<RoutingEdge> {
    if outcome.status != OutcomeStatus::Fail {
        return select_next_edge_with_context(edges, outcome, context);
    }

    let exact_fail_edges = edges
        .iter()
        .filter(|edge| {
            is_exact_outcome_fail_condition(edge.condition_text())
                && evaluate_condition(edge.condition_text(), outcome, context)
        })
        .cloned()
        .collect::<Vec<_>>();
    if let Some(edge) = best_edge_by_weight_then_lexical(&exact_fail_edges) {
        return Some(edge);
    }

    select_next_edge_with_context(edges, outcome, context)
}

pub fn is_exact_outcome_fail_condition(condition: &str) -> bool {
    condition
        .split_whitespace()
        .collect::<String>()
        .eq_ignore_ascii_case("outcome=fail")
}
