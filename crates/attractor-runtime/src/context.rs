use std::collections::BTreeMap;

use attractor_core::{
    apply_launch_context, attr_i64, attr_text, dot_value_to_json, normalize_context_updates,
    resolve_context_write_contract, validate_context_updates_against_contract_with_exemptions,
    AttractorContext, CheckpointState, ContextMap, DotAttribute, DotGraph, FailureKind,
    FlowDefinition, FlowNode, LaunchContext, NodeKind, Outcome,
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
/// tools see it as [`PROJECT_WORK_DIR_ENV`].
pub const PROJECT_WORK_DIR_KEY: &str = "internal.project_work_dir";
pub const PROJECT_WORK_DIR_ENV: &str = "SPARK_PROJECT_WORK_DIR";
const CODERGEN_CONTEXT_UPDATE_EXEMPT_KEYS: &[&str] =
    &["last_response", "last_stage", RUNTIME_SESSIONS_KEY];
const MANAGER_LOOP_CONTEXT_UPDATE_EXEMPT_KEYS: &[&str] = &[RUNTIME_SESSIONS_KEY];
const PARALLEL_CONTEXT_UPDATE_EXEMPT_KEYS: &[&str] = &["parallel.results", RUNTIME_SESSIONS_KEY];
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
        let failure = contract_failure(node_id, outcome, &violation);
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

pub fn apply_outcome_context_updates_for_node(
    node_id: &str,
    node: &FlowNode,
    context: &mut AttractorContext,
    outcome: &Outcome,
) -> Result<Outcome> {
    let node_attrs = crate::flow_runtime::node_attrs_for_handler(node_id, node);
    let mut normalized_updates = normalize_context_updates(&outcome.context_updates);
    let contract = resolve_context_write_contract(&node_attrs);
    let exemptions = context_update_contract_exemptions_for_node(node);
    if let Some(violation) = validate_context_updates_against_contract_with_exemptions(
        &normalized_updates,
        &contract,
        exemptions.exact_keys,
        exemptions.prefixes,
    ) {
        let failure = contract_failure(node_id, outcome, &violation);
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

pub(crate) fn outcome_satisfies_context_contract_for_node(
    node_id: &str,
    node: &FlowNode,
    outcome: &Outcome,
) -> bool {
    let node_attrs = crate::flow_runtime::node_attrs_for_handler(node_id, node);
    let updates = normalize_context_updates(&outcome.context_updates);
    let contract = resolve_context_write_contract(&node_attrs);
    let exemptions = context_update_contract_exemptions_for_node(node);
    validate_context_updates_against_contract_with_exemptions(
        &updates,
        &contract,
        exemptions.exact_keys,
        exemptions.prefixes,
    )
    .is_none()
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
            exact_keys: MANAGER_LOOP_CONTEXT_UPDATE_EXEMPT_KEYS,
            prefixes: MANAGER_LOOP_CONTEXT_UPDATE_EXEMPT_PREFIXES,
        },
        _ => ContextUpdateContractExemptions {
            exact_keys: &[],
            prefixes: &[],
        },
    }
}

fn context_update_contract_exemptions_for_node(node: &FlowNode) -> ContextUpdateContractExemptions {
    match node.kind {
        NodeKind::AgentTask => ContextUpdateContractExemptions {
            exact_keys: CODERGEN_CONTEXT_UPDATE_EXEMPT_KEYS,
            prefixes: &[],
        },
        NodeKind::Parallel => ContextUpdateContractExemptions {
            exact_keys: PARALLEL_CONTEXT_UPDATE_EXEMPT_KEYS,
            prefixes: &[],
        },
        NodeKind::FanIn => ContextUpdateContractExemptions {
            exact_keys: &[],
            prefixes: FAN_IN_CONTEXT_UPDATE_EXEMPT_PREFIXES,
        },
        NodeKind::Tool => ContextUpdateContractExemptions {
            exact_keys: &[],
            prefixes: TOOL_CONTEXT_UPDATE_EXEMPT_PREFIXES,
        },
        NodeKind::HumanGate => ContextUpdateContractExemptions {
            exact_keys: &[],
            prefixes: HUMAN_CONTEXT_UPDATE_EXEMPT_PREFIXES,
        },
        NodeKind::Subflow => ContextUpdateContractExemptions {
            exact_keys: MANAGER_LOOP_CONTEXT_UPDATE_EXEMPT_KEYS,
            prefixes: MANAGER_LOOP_CONTEXT_UPDATE_EXEMPT_PREFIXES,
        },
        NodeKind::Start | NodeKind::Exit | NodeKind::Switch => ContextUpdateContractExemptions {
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
                "switch" => "switch",
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
            | "switch"
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
        "diamond" => Some("switch"),
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
        .filter(|(key, _)| {
            key.starts_with("context.")
                && !key.starts_with("context.tool.")
                && !key.starts_with("context.stack.")
        })
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
