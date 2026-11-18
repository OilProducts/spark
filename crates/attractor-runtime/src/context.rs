use std::collections::BTreeMap;

use attractor_core::{
    apply_launch_context, attr_i64, dot_value_to_json, readable_spark_keys,
    resolve_context_write_contract, validate_context_updates_against_contract_with_exemptions,
    AttractorContext, CheckpointState, ContextMap, DotGraph, FailureKind, FlowDefinition, FlowNode,
    LaunchContext, NodeKind, Outcome, FLOW_CONTEXT_PREFIX,
};
use serde_json::{json, Value};

use crate::error::Result;
use crate::events::utc_timestamp;

/// Each executed node's last invocation status and decision, by node ID.
pub const NODE_OUTCOMES_KEY: &str = "_attractor.node_outcomes";
pub const CURRENT_NODE_KEY: &str = "current_node";
pub const DEFAULT_MAX_RETRIES_KEY: &str = "default_max_retries";
pub const RUNTIME_RETRY_NODE_ID_KEY: &str = "_attractor.runtime.retry.node_id";
pub const RUNTIME_RETRY_ATTEMPT_KEY: &str = "_attractor.runtime.retry.attempt";
pub const RUNTIME_RETRY_MAX_ATTEMPTS_KEY: &str = "_attractor.runtime.retry.max_attempts";
pub const RUNTIME_RETRY_FAILURE_REASON_KEY: &str = "_attractor.runtime.retry.failure_reason";
pub const RUNTIME_LAST_NODE_ID_KEY: &str = "_attractor.runtime.last_node.id";
pub const RUNTIME_LAST_NODE_UPDATES_KEY: &str = "_attractor.runtime.last_node.context_updates";
pub use attractor_core::RUNTIME_SESSIONS_KEY;
pub const INTERNAL_PIPELINE_RETRY_RUN_ID_KEY: &str = "internal.pipeline_retry_run_id";
/// The owning project's `G/work`, bound at launch and inherited by child runs;
/// tools and agent processes see it as [`PROJECT_WORK_DIR_ENV`].
pub const PROJECT_WORK_DIR_KEY: &str = "internal.project_work_dir";
pub use spark_common::project::PROJECT_WORK_DIR_ENV;
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

/// The entry `key` of a sessions section: `named` (handles by session name)
/// or `subflows` (the sessions of each subflow node's child runs).
pub(crate) fn session_entry(context: &ContextMap, section: &str, key: &str) -> Option<Value> {
    context
        .get(RUNTIME_SESSIONS_KEY)?
        .get(section)?
        .get(key)
        .cloned()
}

/// `context`'s sessions with `section.key` set to `value`.
pub(crate) fn sessions_with_entry(
    context: &ContextMap,
    section: &str,
    key: &str,
    value: Value,
) -> Value {
    let mut sessions = context
        .get(RUNTIME_SESSIONS_KEY)
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    sessions[section][key] = value;
    sessions
}

pub fn initialize_runtime_context(
    flow: &FlowDefinition,
    start_node: &str,
    launch_context: &LaunchContext,
) -> Result<AttractorContext> {
    let mut context = AttractorContext::from_map(crate::flow_runtime::flow_context_seed(flow))?;
    seed_builtin_context(&mut context, start_node)?;
    apply_launch_context(&mut context, launch_context)?;
    Ok(context)
}

pub fn checkpoint_from_context(
    current_node: &str,
    completed_nodes: impl IntoIterator<Item = impl Into<String>>,
    context: &AttractorContext,
    retry_counts: impl IntoIterator<Item = (impl Into<String>, u64)>,
    visit_counts: &BTreeMap<String, u64>,
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
        visit_counts: visit_counts.clone(),
        logs: context.logs().to_vec(),
    }
}

/// Applies a node's context updates when its write contract admits them;
/// the Spark keys its kind emits need no declaration.
pub fn apply_outcome_context_updates(
    node_id: &str,
    node: &FlowNode,
    context: &mut AttractorContext,
    outcome: &Outcome,
) -> Result<Outcome> {
    if let Some(violation) = context_contract_violation(node_id, node, outcome) {
        let failure = contract_failure(node_id, outcome, &violation);
        apply_runtime_outcome_fields(node_id, context, &failure)?;
        return Ok(failure);
    }
    let mut updates = outcome.context_updates.clone();
    updates.remove(NODE_OUTCOMES_KEY);
    let applied = Outcome {
        context_updates: updates,
        ..outcome.clone()
    };
    context.apply_updates(&applied.context_updates)?;
    apply_runtime_outcome_fields(node_id, context, &applied)?;
    Ok(applied)
}

/// A context write outside the node's contract: a nonretryable contract
/// error that keeps the response as evidence and applies no update.
fn contract_failure(
    node_id: &str,
    outcome: &Outcome,
    violation: &attractor_core::ContextWriteContractViolation,
) -> Outcome {
    Outcome {
        raw_response_text: outcome.raw_response_text.clone(),
        notes: outcome.notes.clone(),
        ..Outcome::error(
            FailureKind::Contract,
            violation.format_reason(Some(node_id)),
            Some(false),
        )
    }
}

pub(crate) fn outcome_satisfies_context_contract_for_node(
    node_id: &str,
    node: &FlowNode,
    outcome: &Outcome,
) -> bool {
    context_contract_violation(node_id, node, outcome).is_none()
}

fn context_contract_violation(
    node_id: &str,
    node: &FlowNode,
    outcome: &Outcome,
) -> Option<attractor_core::ContextWriteContractViolation> {
    let node_attrs = crate::flow_runtime::node_attrs_for_handler(node_id, node);
    let contract = resolve_context_write_contract(&node_attrs);
    validate_context_updates_against_contract_with_exemptions(
        &outcome.context_updates,
        &contract,
        &spark_keys_emitted_by(&node.kind),
    )
}

/// The Spark keys a node kind writes itself, outside any write contract.
fn spark_keys_emitted_by(kind: &NodeKind) -> Vec<&'static str> {
    let (prefix, sessions) = match kind {
        NodeKind::AgentTask => ("", true),
        NodeKind::Parallel => ("parallel.results", true),
        NodeKind::Tool => ("tool.", false),
        NodeKind::HumanGate => ("human.gate.", false),
        NodeKind::Subflow => ("stack.child.", true),
        NodeKind::Start | NodeKind::Exit | NodeKind::Switch | NodeKind::FanIn => ("", false),
    };
    let mut keys = Vec::new();
    if !prefix.is_empty() {
        keys.extend(readable_spark_keys(prefix));
    }
    if sessions {
        keys.push(RUNTIME_SESSIONS_KEY);
    }
    keys
}

pub fn seed_builtin_context(context: &mut AttractorContext, current_node: &str) -> Result<()> {
    if !matches!(context.get(NODE_OUTCOMES_KEY), Some(Value::Object(_))) {
        context.set(NODE_OUTCOMES_KEY, json!({}))?;
    }
    context.set(CURRENT_NODE_KEY, json!(current_node))?;
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

fn apply_runtime_outcome_fields(
    node_id: &str,
    context: &mut AttractorContext,
    outcome: &Outcome,
) -> Result<()> {
    remember_node_outcome(context, node_id, outcome)?;
    remember_last_node_updates(context, node_id, outcome)?;
    Ok(())
}

/// Records which node ran last and the user-space context it produced, so
/// downstream surfaces (human gates, inspectors) can present a node's output
/// generically without flows declaring anything.
fn remember_last_node_updates(
    context: &mut AttractorContext,
    node_id: &str,
    outcome: &Outcome,
) -> Result<()> {
    let updates: serde_json::Map<String, Value> = outcome
        .context_updates
        .iter()
        .filter(|(key, _)| key.starts_with(FLOW_CONTEXT_PREFIX))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    context.set(RUNTIME_LAST_NODE_ID_KEY, json!(node_id))?;
    context.set(RUNTIME_LAST_NODE_UPDATES_KEY, Value::Object(updates))?;
    Ok(())
}

fn remember_node_outcome(
    context: &mut AttractorContext,
    node_id: &str,
    outcome: &Outcome,
) -> Result<()> {
    let mut outcomes = context
        .get(NODE_OUTCOMES_KEY)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    outcomes.insert(
        node_id.to_string(),
        json!({"status": outcome.status.as_str(), "decision": outcome.decision}),
    );
    context.set(NODE_OUTCOMES_KEY, Value::Object(outcomes))?;
    Ok(())
}
