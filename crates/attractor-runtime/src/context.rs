use std::collections::BTreeMap;

use attractor_core::{
    apply_launch_context, attr_i64, attr_text, dot_value_to_json, normalize_context_updates,
    resolve_context_write_contract, validate_context_updates_against_contract_with_exemptions,
    AttractorContext, CheckpointState, ContextMap, DotAttribute, DotGraph, FailureKind,
    LaunchContext, Outcome, OutcomeStatus, RoutingEdge,
};
use serde_json::{json, Value};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::error::Result;

pub const NODE_OUTCOMES_KEY: &str = "_attractor.node_outcomes";
pub const OUTCOME_KEY: &str = "outcome";
pub const PREFERRED_LABEL_KEY: &str = "preferred_label";
pub const CURRENT_NODE_KEY: &str = "current_node";
pub const WORKFLOW_OUTCOME_KEY: &str = "context.workflow_outcome";
pub const WORKFLOW_OUTCOME_REASON_CODE_KEY: &str = "context.workflow_outcome_reason_code";
pub const WORKFLOW_OUTCOME_REASON_MESSAGE_KEY: &str = "context.workflow_outcome_reason_message";
pub const DEFAULT_MAX_RETRIES_KEY: &str = "default_max_retries";
pub const RUNTIME_RETRY_NODE_ID_KEY: &str = "_attractor.runtime.retry.node_id";
pub const RUNTIME_RETRY_ATTEMPT_KEY: &str = "_attractor.runtime.retry.attempt";
pub const RUNTIME_RETRY_MAX_ATTEMPTS_KEY: &str = "_attractor.runtime.retry.max_attempts";
pub const RUNTIME_RETRY_FAILURE_REASON_KEY: &str = "_attractor.runtime.retry.failure_reason";
pub const RUNTIME_FIDELITY_KEY: &str = "_attractor.runtime.fidelity";
pub const RUNTIME_THREAD_ID_KEY: &str = "_attractor.runtime.thread_id";
pub const INTERNAL_PIPELINE_RETRY_RUN_ID_KEY: &str = "internal.pipeline_retry_run_id";
const CODERGEN_CONTEXT_UPDATE_EXEMPT_KEYS: &[&str] = &["last_response", "last_stage"];
const PARALLEL_CONTEXT_UPDATE_EXEMPT_KEYS: &[&str] = &["parallel.results"];
const TOOL_CONTEXT_UPDATE_EXEMPT_PREFIXES: &[&str] = &["context.tool."];
const HUMAN_CONTEXT_UPDATE_EXEMPT_PREFIXES: &[&str] = &["human.gate."];
const FAN_IN_CONTEXT_UPDATE_EXEMPT_PREFIXES: &[&str] = &["parallel.fan_in."];
const MANAGER_LOOP_CONTEXT_UPDATE_EXEMPT_PREFIXES: &[&str] = &["context.stack.child."];

pub fn graph_attr_context_seed(graph: &DotGraph) -> ContextMap {
    let mut seeded = ContextMap::new();
    for (key, attr) in &graph.graph_attrs {
        seeded.insert(format!("graph.{key}"), dot_value_to_json(&attr.value));
    }
    seeded
        .entry("graph.goal".to_string())
        .or_insert_with(|| Value::String(String::new()));
    seeded.insert(
        format!("graph.{DEFAULT_MAX_RETRIES_KEY}"),
        json!(attr_i64(&graph.graph_attrs, DEFAULT_MAX_RETRIES_KEY, 0)),
    );
    seeded
}

pub fn initialize_runtime_context(
    graph: &DotGraph,
    start_node: &str,
    launch_context: &LaunchContext,
) -> Result<AttractorContext> {
    let mut context = AttractorContext::from_map(graph_attr_context_seed(graph))?;
    seed_builtin_context(&mut context, start_node)?;
    apply_launch_context(&mut context, launch_context)?;
    Ok(context)
}

pub fn checkpoint_from_context(
    current_node: &str,
    completed_nodes: impl IntoIterator<Item = impl Into<String>>,
    context: &AttractorContext,
    retry_counts: impl IntoIterator<Item = (impl Into<String>, u64)>,
) -> CheckpointState {
    CheckpointState {
        timestamp: utc_timestamp(),
        current_node: current_node.to_string(),
        completed_nodes: completed_nodes.into_iter().map(Into::into).collect(),
        context: context.snapshot(),
        retry_counts: retry_counts
            .into_iter()
            .map(|(key, value)| (key.into(), value))
            .collect(),
        logs: context.logs().to_vec(),
    }
}

pub fn apply_outcome_context_updates(
    node_id: &str,
    node_attrs: &BTreeMap<String, DotAttribute>,
    context: &mut AttractorContext,
    outcome: &Outcome,
) -> Result<Outcome> {
    let mut normalized_updates = normalize_context_updates(&outcome.context_updates);
    let contract = resolve_context_write_contract(node_attrs);
    let exemptions = context_update_contract_exemptions(node_attrs);
    if let Some(violation) = validate_context_updates_against_contract_with_exemptions(
        &normalized_updates,
        &contract,
        exemptions.exact_keys,
        exemptions.prefixes,
    ) {
        let failure = Outcome {
            status: OutcomeStatus::Fail,
            failure_reason: violation.format_reason(Some(node_id)),
            retryable: Some(false),
            failure_kind: Some(FailureKind::Contract),
            raw_response_text: outcome.raw_response_text.clone(),
            notes: outcome.notes.clone(),
            ..Outcome::new(OutcomeStatus::Fail)
        };
        apply_runtime_outcome_fields(node_id, context, &failure)?;
        return Ok(failure);
    }

    normalized_updates.remove(NODE_OUTCOMES_KEY);
    let normalized_outcome = Outcome {
        context_updates: normalized_updates,
        ..outcome.clone()
    };
    context.apply_updates(&normalized_outcome.context_updates)?;
    apply_runtime_outcome_fields(node_id, context, &normalized_outcome)?;
    Ok(normalized_outcome)
}

#[derive(Debug, Clone, Copy)]
struct ContextUpdateContractExemptions {
    exact_keys: &'static [&'static str],
    prefixes: &'static [&'static str],
}

fn context_update_contract_exemptions(
    node_attrs: &BTreeMap<String, DotAttribute>,
) -> ContextUpdateContractExemptions {
    match resolved_builtin_handler_type(node_attrs) {
        "codergen" => ContextUpdateContractExemptions {
            exact_keys: CODERGEN_CONTEXT_UPDATE_EXEMPT_KEYS,
            prefixes: &[],
        },
        "parallel" => ContextUpdateContractExemptions {
            exact_keys: PARALLEL_CONTEXT_UPDATE_EXEMPT_KEYS,
            prefixes: &[],
        },
        "parallel.fan_in" => ContextUpdateContractExemptions {
            exact_keys: &[],
            prefixes: FAN_IN_CONTEXT_UPDATE_EXEMPT_PREFIXES,
        },
        "tool" => ContextUpdateContractExemptions {
            exact_keys: &[],
            prefixes: TOOL_CONTEXT_UPDATE_EXEMPT_PREFIXES,
        },
        "wait.human" => ContextUpdateContractExemptions {
            exact_keys: &[],
            prefixes: HUMAN_CONTEXT_UPDATE_EXEMPT_PREFIXES,
        },
        "stack.manager_loop" => ContextUpdateContractExemptions {
            exact_keys: &[],
            prefixes: MANAGER_LOOP_CONTEXT_UPDATE_EXEMPT_PREFIXES,
        },
        _ => ContextUpdateContractExemptions {
            exact_keys: &[],
            prefixes: &[],
        },
    }
}

fn resolved_builtin_handler_type(node_attrs: &BTreeMap<String, DotAttribute>) -> &'static str {
    if let Some(handler_type) = attr_text(node_attrs, "type") {
        if known_builtin_handler_type(&handler_type) {
            return match handler_type.as_str() {
                "start" => "start",
                "exit" => "exit",
                "codergen" => "codergen",
                "wait.human" => "wait.human",
                "conditional" => "conditional",
                "parallel" => "parallel",
                "parallel.fan_in" => "parallel.fan_in",
                "tool" => "tool",
                "stack.manager_loop" => "stack.manager_loop",
                _ => "codergen",
            };
        }
    }
    if let Some(shape) = attr_text(node_attrs, "shape") {
        if let Some(handler_type) = handler_type_for_shape(&shape) {
            return handler_type;
        }
    }
    "codergen"
}

pub fn known_builtin_handler_type(handler_type: &str) -> bool {
    matches!(
        handler_type,
        "start"
            | "exit"
            | "codergen"
            | "wait.human"
            | "conditional"
            | "parallel"
            | "parallel.fan_in"
            | "tool"
            | "stack.manager_loop"
    )
}

pub fn handler_type_for_shape(shape: &str) -> Option<&'static str> {
    match shape {
        "Mdiamond" => Some("start"),
        "Msquare" => Some("exit"),
        "box" => Some("codergen"),
        "hexagon" => Some("wait.human"),
        "diamond" => Some("conditional"),
        "component" => Some("parallel"),
        "tripleoctagon" => Some("parallel.fan_in"),
        "parallelogram" => Some("tool"),
        "house" => Some("stack.manager_loop"),
        _ => None,
    }
}

pub fn seed_builtin_context(context: &mut AttractorContext, current_node: &str) -> Result<()> {
    if !matches!(context.get(NODE_OUTCOMES_KEY), Some(Value::Object(_))) {
        context.set(NODE_OUTCOMES_KEY, json!({}))?;
    }
    if context.get(OUTCOME_KEY).is_none() {
        context.set(OUTCOME_KEY, json!(""))?;
    }
    if context.get(PREFERRED_LABEL_KEY).is_none() {
        context.set(PREFERRED_LABEL_KEY, json!(""))?;
    }
    for key in [
        WORKFLOW_OUTCOME_KEY,
        WORKFLOW_OUTCOME_REASON_CODE_KEY,
        WORKFLOW_OUTCOME_REASON_MESSAGE_KEY,
    ] {
        if context.get(key).is_none() {
            context.set(key, json!(""))?;
        }
    }
    context.set(CURRENT_NODE_KEY, json!(current_node))?;
    Ok(())
}

pub fn reset_workflow_outcome_context(context: &mut AttractorContext) -> Result<()> {
    context.set(WORKFLOW_OUTCOME_KEY, json!(""))?;
    context.set(WORKFLOW_OUTCOME_REASON_CODE_KEY, json!(""))?;
    context.set(WORKFLOW_OUTCOME_REASON_MESSAGE_KEY, json!(""))?;
    Ok(())
}

pub fn set_runtime_retry_context(
    context: &mut AttractorContext,
    node_id: &str,
    attempt: u64,
    max_attempts: u64,
    failure_reason: &str,
) -> Result<()> {
    context.set(RUNTIME_RETRY_NODE_ID_KEY, json!(node_id))?;
    context.set(RUNTIME_RETRY_ATTEMPT_KEY, json!(attempt))?;
    context.set(RUNTIME_RETRY_MAX_ATTEMPTS_KEY, json!(max_attempts))?;
    context.set(RUNTIME_RETRY_FAILURE_REASON_KEY, json!(failure_reason))?;
    Ok(())
}

pub fn clear_runtime_retry_context(context: &mut AttractorContext) -> Result<()> {
    context.set(RUNTIME_RETRY_NODE_ID_KEY, json!(""))?;
    context.set(RUNTIME_RETRY_ATTEMPT_KEY, json!(0))?;
    context.set(RUNTIME_RETRY_MAX_ATTEMPTS_KEY, json!(0))?;
    context.set(RUNTIME_RETRY_FAILURE_REASON_KEY, json!(""))?;
    Ok(())
}

pub fn set_runtime_fidelity_context(
    graph: &DotGraph,
    node_id: &str,
    incoming_edge: Option<&RoutingEdge>,
    context: &mut AttractorContext,
    force_fidelity: Option<&str>,
) -> Result<String> {
    let fidelity = force_fidelity
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| resolve_runtime_fidelity(graph, node_id, incoming_edge));
    let thread_id = resolve_runtime_thread_id(graph, node_id, incoming_edge, &fidelity);
    context.set(RUNTIME_FIDELITY_KEY, json!(fidelity.clone()))?;
    context.set(RUNTIME_THREAD_ID_KEY, json!(thread_id))?;
    Ok(fidelity)
}

pub fn resolve_runtime_fidelity(
    graph: &DotGraph,
    node_id: &str,
    incoming_edge: Option<&RoutingEdge>,
) -> String {
    if let Some(edge_fidelity) = incoming_edge.and_then(|edge| edge_attr_text(edge, "fidelity")) {
        return edge_fidelity;
    }
    if let Some(node) = graph.nodes.get(node_id) {
        if let Some(node_fidelity) = attr_text(&node.attrs, "fidelity") {
            return node_fidelity;
        }
    }
    if let Some(graph_fidelity) = attr_text(&graph.graph_attrs, "default_fidelity") {
        return graph_fidelity;
    }
    "compact".to_string()
}

pub fn resolve_runtime_thread_id(
    graph: &DotGraph,
    node_id: &str,
    incoming_edge: Option<&RoutingEdge>,
    fidelity: &str,
) -> String {
    if fidelity != "full" {
        return String::new();
    }
    if let Some(node) = graph.nodes.get(node_id) {
        if let Some(thread_id) = attr_text(&node.attrs, "thread_id") {
            return thread_id;
        }
    }
    if let Some(edge_thread_id) = incoming_edge.and_then(|edge| edge_attr_text(edge, "thread_id")) {
        return edge_thread_id;
    }
    if let Some(graph_thread_id) = attr_text(&graph.graph_attrs, "thread_id") {
        return graph_thread_id;
    }
    if let Some(node) = graph.nodes.get(node_id) {
        if let Some(class_name) = attr_text(&node.attrs, "class")
            .and_then(|value| value.split(',').next().map(str::trim).map(str::to_string))
            .filter(|value| !value.is_empty())
        {
            return class_name;
        }
    }
    incoming_edge
        .map(|edge| edge.source.as_str().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| node_id.to_string())
}

pub fn checkpoint_requests_full_fidelity_degrade(checkpoint: &CheckpointState) -> bool {
    checkpoint
        .context
        .get(RUNTIME_FIDELITY_KEY)
        .and_then(Value::as_str)
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("full"))
}

fn apply_runtime_outcome_fields(
    node_id: &str,
    context: &mut AttractorContext,
    outcome: &Outcome,
) -> Result<()> {
    context.set(OUTCOME_KEY, json!(outcome.status.as_str()))?;
    context.set(PREFERRED_LABEL_KEY, json!(outcome.preferred_label.clone()))?;
    remember_node_outcome(context, node_id, outcome.status.as_str())?;
    Ok(())
}

fn remember_node_outcome(
    context: &mut AttractorContext,
    node_id: &str,
    status: &str,
) -> Result<()> {
    let mut outcomes = context
        .get(NODE_OUTCOMES_KEY)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    outcomes.insert(node_id.to_string(), Value::String(status.to_string()));
    context.set(NODE_OUTCOMES_KEY, Value::Object(outcomes))?;
    Ok(())
}

fn utc_timestamp() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_default()
}

fn edge_attr_text(edge: &RoutingEdge, key: &str) -> Option<String> {
    edge.attributes
        .get(key)
        .and_then(value_to_trimmed_string)
        .filter(|value| !value.is_empty())
}

fn value_to_trimmed_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(value) => Some(value.trim().to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}
