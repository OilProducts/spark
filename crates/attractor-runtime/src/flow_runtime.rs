use std::collections::{BTreeMap, BTreeSet};

use attractor_core::{
    dot_value_to_json, AttractorContext, ContextMap, DotAttribute, DotEdge, DotGraph, DotNode,
    DotScopeDefaults, DotValue, DotValueType, FlowDefinition, FlowEdge, FlowNode, NodeConfig,
    NodeId, NodeKind, RoutingEdge,
};
use serde_json::{json, Value};

use crate::error::{Result, RuntimeStorageError};
use crate::handlers::{
    HANDLER_CODERGEN, HANDLER_CONDITIONAL, HANDLER_EXIT, HANDLER_FAN_IN, HANDLER_MANAGER_LOOP,
    HANDLER_PARALLEL, HANDLER_START, HANDLER_TOOL, HANDLER_WAIT_HUMAN,
};

const CORE_NODE_EXTENSION_KEYS: &[&str] = &[
    "allow_partial",
    "auto_status",
    "class",
    "codergen.contract_repair_attempts",
    "codergen.execution_mode",
    "codergen.response_contract",
    "codergen.runtime_mode",
    "error_policy",
    "fallback_retry_target",
    "fidelity",
    "goal_gate",
    "join_k",
    "join_policy",
    "join_quorum",
    "kind",
    "label",
    "llm_model",
    "llm_profile",
    "llm_provider",
    "manager.actions",
    "manager.max_cycles",
    "manager.poll_interval",
    "manager.steer_cooldown",
    "manager.stop_condition",
    "max_retries",
    "max_parallel",
    "options",
    "prompt",
    "reasoning_effort",
    "retry_policy",
    "retry_target",
    "shape",
    "spark.reads_context",
    "spark.writes_context",
    "stack.child_autostart",
    "stack.child_flow_ref",
    "thread_id",
    "timeout",
    "tool.artifacts.paths",
    "tool.artifacts.stderr",
    "tool.artifacts.stdout",
    "tool.command",
    "tool.hooks.post",
    "tool.hooks.pre",
    "type",
];

pub fn resolve_start_node(flow: &FlowDefinition) -> Result<String> {
    let starts = flow
        .nodes
        .iter()
        .filter(|(_, node)| node.kind == NodeKind::Start)
        .map(|(node_id, _)| node_id.clone())
        .collect::<Vec<_>>();
    match starts.as_slice() {
        [start] => Ok(start.clone()),
        [] => Err(RuntimeStorageError::InvalidRuntimeGraph {
            reason: "No start node found; expected kind=start".to_string(),
        }),
        _ => Err(RuntimeStorageError::InvalidRuntimeGraph {
            reason: format!("Ambiguous start nodes: {}", sorted_join(starts)),
        }),
    }
}

pub fn is_exit_node(flow: &FlowDefinition, node_id: &str) -> bool {
    flow.nodes
        .get(node_id)
        .is_some_and(|node| node.kind == NodeKind::Exit)
}

pub fn is_conditional_node(flow: &FlowDefinition, node_id: &str) -> bool {
    flow.nodes
        .get(node_id)
        .is_some_and(|node| node.kind == NodeKind::Conditional)
}

pub fn outgoing_routing_edges(flow: &FlowDefinition, node_id: &str) -> Result<Vec<RoutingEdge>> {
    flow.edges
        .iter()
        .filter(|edge| edge.from == node_id)
        .map(routing_edge_from_flow_edge)
        .collect()
}

pub fn routing_edge_from_flow_edge(edge: &FlowEdge) -> Result<RoutingEdge> {
    let mut routing_edge = RoutingEdge::new(
        NodeId::try_from(edge.from.as_str()).map_err(RuntimeStorageError::from)?,
        NodeId::try_from(edge.to.as_str()).map_err(RuntimeStorageError::from)?,
    );
    routing_edge.label = edge.label.clone();
    routing_edge.condition = edge.condition.clone();
    routing_edge.weight = edge.weight;
    routing_edge.attributes = edge.extensions.clone();
    if let Some(transition) = edge.transition.as_deref() {
        routing_edge.attributes.insert(
            "transition".to_string(),
            Value::String(transition.to_string()),
        );
    }
    Ok(routing_edge)
}

pub fn flow_context_seed(flow: &FlowDefinition) -> ContextMap {
    let mut seeded = ContextMap::new();
    seeded.insert(
        "graph.schema_version".to_string(),
        json!(flow.schema_version),
    );
    seeded.insert("graph.id".to_string(), json!(flow.id));
    seeded.insert("graph.label".to_string(), json!(flow.title));
    seeded.insert("graph.spark.title".to_string(), json!(flow.title));
    seeded.insert(
        "graph.spark.description".to_string(),
        json!(flow.description),
    );
    seeded.insert("graph.goal".to_string(), json!(flow.goal));
    seeded.insert(
        "graph.default_max_retries".to_string(),
        json!(flow.defaults.max_retries.unwrap_or(0)),
    );
    if let Some(value) = flow.defaults.fidelity.as_deref() {
        seeded.insert("graph.default_fidelity".to_string(), json!(value));
    }
    for (key, value) in &flow.metadata {
        seeded.insert(format!("graph.{key}"), value.clone());
    }
    for (key, value) in &flow.extensions {
        seeded.insert(format!("graph.{key}"), value.clone());
    }
    seeded
}

pub fn node_prompt(node: &FlowNode) -> String {
    match node.config.as_ref() {
        Some(NodeConfig::AgentTask { prompt }) | Some(NodeConfig::HumanGate { prompt, .. }) => {
            prompt.trim().to_string()
        }
        _ => String::new(),
    }
}

pub fn handler_type_for_node(node: &FlowNode) -> &'static str {
    match node.kind {
        NodeKind::Start => HANDLER_START,
        NodeKind::Exit => HANDLER_EXIT,
        NodeKind::AgentTask => HANDLER_CODERGEN,
        NodeKind::HumanGate => HANDLER_WAIT_HUMAN,
        NodeKind::Conditional => HANDLER_CONDITIONAL,
        NodeKind::Parallel => HANDLER_PARALLEL,
        NodeKind::FanIn => HANDLER_FAN_IN,
        NodeKind::Tool => HANDLER_TOOL,
        NodeKind::Subflow => HANDLER_MANAGER_LOOP,
    }
}

pub fn node_attrs_for_handler(node_id: &str, node: &FlowNode) -> BTreeMap<String, DotAttribute> {
    let mut attrs = BTreeMap::new();
    insert_string_attr(&mut attrs, "label", &node.label);
    insert_string_attr(&mut attrs, "description", &node.description);
    insert_string_attr(&mut attrs, "type", handler_type_for_node(node));
    insert_string_attr(&mut attrs, "shape", shape_for_node_kind(&node.kind));
    let prompt = node_prompt(node);
    if !prompt.is_empty() {
        insert_string_attr(&mut attrs, "prompt", &prompt);
    }
    if let Some(NodeConfig::HumanGate { decisions, .. }) = node.config.as_ref() {
        if let Ok(value) = serde_json::to_string(decisions) {
            insert_string_attr(&mut attrs, "options", &value);
        }
    }
    if let Some(NodeConfig::Tool {
        command,
        env_map,
        output_map,
    }) = node.config.as_ref()
    {
        insert_string_attr(&mut attrs, "tool.command", command);
        insert_tool_map_attr(&mut attrs, "tool.env_map", env_map);
        insert_tool_map_attr(&mut attrs, "tool.output_map", output_map);
    }
    if let Some(NodeConfig::Subflow { flow_ref, .. }) = node.config.as_ref() {
        insert_string_attr(&mut attrs, "stack.child_flow_ref", flow_ref);
    }
    if let Some(NodeConfig::Parallel {
        join_policy,
        max_parallel,
        join_k,
        join_quorum,
    }) = node.config.as_ref()
    {
        if let Some(value) = join_policy.as_deref() {
            insert_string_attr(&mut attrs, "join_policy", value);
        }
        if let Some(value) = max_parallel {
            insert_i64_attr(&mut attrs, "max_parallel", *value as i64);
        }
        if let Some(value) = join_k {
            insert_i64_attr(&mut attrs, "join_k", *value as i64);
        }
        if let Some(value) = join_quorum {
            insert_f64_attr(&mut attrs, "join_quorum", *value);
        }
    }
    if let Some(runtime) = node.runtime.as_ref() {
        insert_bool_attr(&mut attrs, "allow_partial", runtime.allow_partial);
        insert_bool_attr(&mut attrs, "auto_status", runtime.auto_status);
        insert_bool_attr(&mut attrs, "goal_gate", runtime.goal_gate);
        if let Some(value) = runtime.error_policy.as_deref() {
            insert_string_attr(&mut attrs, "error_policy", value);
        }
        if let Some(value) = runtime.fidelity.as_deref() {
            insert_string_attr(&mut attrs, "fidelity", value);
        }
        if let Some(value) = runtime.thread_id.as_deref() {
            insert_string_attr(&mut attrs, "thread_id", value);
        }
        if let Some(value) = runtime.class.as_deref() {
            insert_string_attr(&mut attrs, "class", value);
        }
        if let Some(value) = runtime.retry_target.as_deref() {
            insert_string_attr(&mut attrs, "retry_target", value);
        }
        if let Some(value) = runtime.fallback_retry_target.as_deref() {
            insert_string_attr(&mut attrs, "fallback_retry_target", value);
        }
    }
    if let Some(contracts) = node.contracts.as_ref() {
        if !contracts.reads_context.is_empty() {
            insert_json_string_attr(&mut attrs, "spark.reads_context", &contracts.reads_context);
        }
        if !contracts.writes_context.is_empty() {
            insert_json_string_attr(
                &mut attrs,
                "spark.writes_context",
                &contracts.writes_context,
            );
        }
        if let Some(value) = contracts.response.as_deref() {
            insert_string_attr(&mut attrs, "codergen.response_contract", value);
        }
        if let Some(value) = contracts.repair_attempts {
            insert_i64_attr(
                &mut attrs,
                "codergen.contract_repair_attempts",
                value as i64,
            );
        }
        if let Some(value) = contracts.runtime_mode.as_deref() {
            insert_string_attr(&mut attrs, "codergen.runtime_mode", value);
        }
    }
    if let Some(manager) = node.manager.as_ref() {
        if let Some(value) = manager.poll_interval.as_deref() {
            insert_string_attr(&mut attrs, "manager.poll_interval", value);
        }
        if let Some(value) = manager.max_cycles {
            insert_i64_attr(&mut attrs, "manager.max_cycles", value as i64);
        }
        if let Some(value) = manager.stop_condition.as_deref() {
            insert_string_attr(&mut attrs, "manager.stop_condition", value);
        }
        if !manager.actions.is_empty() {
            insert_string_attr(&mut attrs, "manager.actions", &manager.actions.join(","));
        }
        if let Some(value) = manager.steer_cooldown.as_deref() {
            insert_string_attr(&mut attrs, "manager.steer_cooldown", value);
        }
        if let Some(value) = manager.child_autostart {
            insert_bool_attr(&mut attrs, "stack.child_autostart", value);
        }
    }
    if let Some(retry) = node.retry.as_ref() {
        if let Some(policy) = retry.policy.as_deref() {
            insert_string_attr(&mut attrs, "retry_policy", policy);
        }
        if let Some(max_retries) = retry.max_retries {
            insert_i64_attr(&mut attrs, "max_retries", max_retries as i64);
        }
    }
    if let Some(execution) = node.execution.as_ref() {
        if let Some(value) = execution.llm_provider.as_deref() {
            insert_authored_string_attr(&mut attrs, "llm_provider", value);
        }
        if let Some(value) = execution.llm_profile.as_deref() {
            insert_authored_string_attr(&mut attrs, "llm_profile", value);
        }
        if let Some(value) = execution.llm_model.as_deref() {
            insert_authored_string_attr(&mut attrs, "llm_model", value);
        }
        if let Some(value) = execution.reasoning_effort.as_deref() {
            insert_authored_string_attr(&mut attrs, "reasoning_effort", value);
        }
    }
    for (key, value) in &node.context {
        if !is_core_node_attr_key(key) {
            attrs.insert(key.clone(), json_attr(key, value.clone()));
        }
    }
    for (key, value) in &node.extensions {
        if !is_core_node_attr_key(key) {
            attrs
                .entry(key.clone())
                .or_insert_with(|| json_attr(key, value.clone()));
        }
    }
    attrs
        .entry("node_id".to_string())
        .or_insert_with(|| string_attr("node_id", node_id));
    attrs
}

pub fn flow_graph_for_handler_compat(flow: &FlowDefinition) -> DotGraph {
    let graph_attrs = graph_attrs_for_handler(flow);
    let nodes = flow
        .nodes
        .iter()
        .map(|(node_id, node)| {
            let attrs = node_attrs_for_handler(node_id, node);
            let explicit_attr_keys = attrs.keys().cloned().collect();
            (
                node_id.clone(),
                DotNode {
                    node_id: node_id.clone(),
                    attrs,
                    line: 0,
                    declaration_order: 0,
                    explicit_attr_keys,
                },
            )
        })
        .collect();
    let edges = flow
        .edges
        .iter()
        .map(|edge| {
            let mut attrs = BTreeMap::new();
            insert_string_attr(&mut attrs, "label", &edge.label);
            insert_string_attr(&mut attrs, "condition", &edge.condition);
            insert_i64_attr(&mut attrs, "weight", edge.weight);
            if let Some(transition) = edge.transition.as_deref() {
                insert_string_attr(&mut attrs, "transition", transition);
            }
            for (key, value) in &edge.extensions {
                attrs.insert(key.clone(), json_attr(key, value.clone()));
            }
            DotEdge {
                source: edge.from.clone(),
                target: edge.to.clone(),
                attrs,
                line: 0,
            }
        })
        .collect();
    DotGraph {
        graph_id: flow.id.clone(),
        graph_attrs,
        nodes,
        edges,
        defaults: DotScopeDefaults::default(),
        subgraphs: Vec::new(),
    }
}

pub fn graph_attrs_for_handler(flow: &FlowDefinition) -> BTreeMap<String, DotAttribute> {
    let mut attrs = BTreeMap::new();
    insert_string_attr(&mut attrs, "schema_version", &flow.schema_version);
    insert_string_attr(&mut attrs, "label", &flow.title);
    insert_string_attr(&mut attrs, "spark.title", &flow.title);
    insert_string_attr(&mut attrs, "spark.description", &flow.description);
    insert_string_attr(&mut attrs, "goal", &flow.goal);
    if let Some(value) = flow.defaults.fidelity.as_deref() {
        insert_string_attr(&mut attrs, "default_fidelity", value);
    }
    if let Some(value) = flow.defaults.max_retries {
        insert_i64_attr(&mut attrs, "default_max_retries", value as i64);
    }
    if let Some(value) = flow.defaults.llm_provider.as_deref() {
        insert_string_attr(&mut attrs, "ui_default_llm_provider", value);
    }
    if let Some(value) = flow.defaults.llm_profile.as_deref() {
        insert_string_attr(&mut attrs, "ui_default_llm_profile", value);
    }
    if let Some(value) = flow.defaults.llm_model.as_deref() {
        insert_string_attr(&mut attrs, "ui_default_llm_model", value);
    }
    if let Some(value) = flow.defaults.reasoning_effort.as_deref() {
        insert_string_attr(&mut attrs, "ui_default_reasoning_effort", value);
    }
    if !flow.inputs.is_empty() {
        if let Ok(value) = serde_json::to_string(&flow.inputs) {
            insert_string_attr(&mut attrs, "spark.launch_inputs", &value);
        }
    }
    for (key, value) in &flow.metadata {
        attrs.insert(key.clone(), json_attr(key, value.clone()));
    }
    for (key, value) in &flow.extensions {
        attrs.insert(key.clone(), json_attr(key, value.clone()));
    }
    attrs
}

pub fn node_explicit_attr_keys(attrs: &BTreeMap<String, DotAttribute>) -> BTreeSet<String> {
    attrs.keys().cloned().collect()
}

pub fn flow_attr_text(flow: &FlowDefinition, key: &str) -> Option<String> {
    flow.metadata
        .get(key)
        .or_else(|| flow.extensions.get(key))
        .and_then(value_to_non_empty_text)
}

pub fn node_attr_text(node: &FlowNode, key: &str) -> Option<String> {
    if is_core_node_attr_key(key) {
        return typed_node_attr_text(node, key);
    }
    node.context
        .get(key)
        .or_else(|| node.extensions.get(key))
        .and_then(value_to_non_empty_text)
}

pub fn node_attr_bool(node: &FlowNode, key: &str, default: bool) -> bool {
    if is_core_node_attr_key(key) {
        return typed_node_attr_bool(node, key).unwrap_or(default);
    }
    node.context
        .get(key)
        .or_else(|| node.extensions.get(key))
        .map(value_to_bool)
        .unwrap_or(default)
}

fn is_core_node_attr_key(key: &str) -> bool {
    CORE_NODE_EXTENSION_KEYS.contains(&key)
}

fn typed_node_attr_text(node: &FlowNode, key: &str) -> Option<String> {
    let typed = match key {
        "error_policy" => node.runtime.as_ref()?.error_policy.clone(),
        "fidelity" => node.runtime.as_ref()?.fidelity.clone(),
        "thread_id" => node.runtime.as_ref()?.thread_id.clone(),
        "class" => node.runtime.as_ref()?.class.clone(),
        "retry_target" => node.runtime.as_ref()?.retry_target.clone(),
        "fallback_retry_target" => node.runtime.as_ref()?.fallback_retry_target.clone(),
        "retry_policy" => node.retry.as_ref()?.policy.clone(),
        "max_retries" => node
            .retry
            .as_ref()?
            .max_retries
            .map(|value| value.to_string()),
        "join_policy" => match node.config.as_ref()? {
            NodeConfig::Parallel { join_policy, .. } => join_policy.clone(),
            _ => None,
        },
        "max_parallel" => match node.config.as_ref()? {
            NodeConfig::Parallel { max_parallel, .. } => {
                max_parallel.map(|value| value.to_string())
            }
            _ => None,
        },
        "join_k" => match node.config.as_ref()? {
            NodeConfig::Parallel { join_k, .. } => join_k.map(|value| value.to_string()),
            _ => None,
        },
        "join_quorum" => match node.config.as_ref()? {
            NodeConfig::Parallel { join_quorum, .. } => join_quorum.map(|value| value.to_string()),
            _ => None,
        },
        "stack.child_flow_ref" => match node.config.as_ref()? {
            NodeConfig::Subflow { flow_ref, .. } => Some(flow_ref.clone()),
            _ => None,
        },
        "manager.poll_interval" => node.manager.as_ref()?.poll_interval.clone(),
        "manager.max_cycles" => node
            .manager
            .as_ref()?
            .max_cycles
            .map(|value| value.to_string()),
        "manager.stop_condition" => node.manager.as_ref()?.stop_condition.clone(),
        "manager.actions" => {
            let actions = &node.manager.as_ref()?.actions;
            (!actions.is_empty()).then(|| actions.join(","))
        }
        "manager.steer_cooldown" => node.manager.as_ref()?.steer_cooldown.clone(),
        "llm_provider" => node.execution.as_ref()?.llm_provider.clone(),
        "llm_profile" => node.execution.as_ref()?.llm_profile.clone(),
        "llm_model" => node.execution.as_ref()?.llm_model.clone(),
        "reasoning_effort" => node.execution.as_ref()?.reasoning_effort.clone(),
        "tool.command" => match node.config.as_ref()? {
            NodeConfig::Tool { command, .. } => Some(command.clone()),
            _ => None,
        },
        "prompt" => match node.config.as_ref()? {
            NodeConfig::AgentTask { prompt } | NodeConfig::HumanGate { prompt, .. } => {
                Some(prompt.clone())
            }
            _ => None,
        },
        _ => None,
    };
    typed.and_then(|value| {
        let trimmed = value.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    })
}

fn typed_node_attr_bool(node: &FlowNode, key: &str) -> Option<bool> {
    match key {
        "allow_partial" => node.runtime.as_ref().map(|runtime| runtime.allow_partial),
        "auto_status" => node.runtime.as_ref().map(|runtime| runtime.auto_status),
        "goal_gate" => node.runtime.as_ref().map(|runtime| runtime.goal_gate),
        "stack.child_autostart" => node
            .manager
            .as_ref()
            .and_then(|manager| manager.child_autostart),
        _ => None,
    }
}

pub fn apply_runtime_context_fields(
    context: &mut AttractorContext,
    flow: &FlowDefinition,
) -> Result<()> {
    for (key, value) in flow_context_seed(flow) {
        context.set(&key, value)?;
    }
    Ok(())
}

fn shape_for_node_kind(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Start => "Mdiamond",
        NodeKind::Exit => "Msquare",
        NodeKind::AgentTask => "box",
        NodeKind::HumanGate => "hexagon",
        NodeKind::Conditional => "diamond",
        NodeKind::Parallel => "component",
        NodeKind::FanIn => "tripleoctagon",
        NodeKind::Tool => "parallelogram",
        NodeKind::Subflow => "house",
    }
}

fn value_to_non_empty_text(value: &Value) -> Option<String> {
    let text = match value {
        Value::Null => String::new(),
        Value::String(value) => value.trim().to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    };
    (!text.is_empty()).then_some(text)
}

fn value_to_bool(value: &Value) -> bool {
    match value {
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_i64().unwrap_or_default() != 0,
        Value::String(value) => value.trim().eq_ignore_ascii_case("true"),
        Value::Null | Value::Array(_) | Value::Object(_) => false,
    }
}

fn insert_string_attr(attrs: &mut BTreeMap<String, DotAttribute>, key: &str, value: &str) {
    if !value.trim().is_empty() {
        attrs.insert(key.to_string(), string_attr(key, value));
    }
}

fn insert_tool_map_attr(
    attrs: &mut BTreeMap<String, DotAttribute>,
    key: &str,
    map: &BTreeMap<String, String>,
) {
    if map.is_empty() {
        return;
    }
    if let Ok(value) = serde_json::to_string(map) {
        insert_string_attr(attrs, key, &value);
    }
}

fn insert_authored_string_attr(attrs: &mut BTreeMap<String, DotAttribute>, key: &str, value: &str) {
    if !value.trim().is_empty() {
        attrs.insert(key.to_string(), string_attr_with_line(key, value, 1));
    }
}

fn insert_i64_attr(attrs: &mut BTreeMap<String, DotAttribute>, key: &str, value: i64) {
    attrs.insert(
        key.to_string(),
        DotAttribute {
            key: key.to_string(),
            value: DotValue::Integer(value),
            value_type: DotValueType::Integer,
            line: 0,
        },
    );
}

fn insert_f64_attr(attrs: &mut BTreeMap<String, DotAttribute>, key: &str, value: f64) {
    attrs.insert(
        key.to_string(),
        DotAttribute {
            key: key.to_string(),
            value: DotValue::Float(value),
            value_type: DotValueType::Float,
            line: 0,
        },
    );
}

fn insert_bool_attr(attrs: &mut BTreeMap<String, DotAttribute>, key: &str, value: bool) {
    attrs.insert(
        key.to_string(),
        DotAttribute {
            key: key.to_string(),
            value: DotValue::Boolean(value),
            value_type: DotValueType::Boolean,
            line: 0,
        },
    );
}

fn insert_json_string_attr<T: serde::Serialize>(
    attrs: &mut BTreeMap<String, DotAttribute>,
    key: &str,
    value: &T,
) {
    if let Ok(value) = serde_json::to_string(value) {
        insert_string_attr(attrs, key, &value);
    }
}

fn string_attr(key: &str, value: &str) -> DotAttribute {
    string_attr_with_line(key, value, 0)
}

fn string_attr_with_line(key: &str, value: &str, line: usize) -> DotAttribute {
    DotAttribute {
        key: key.to_string(),
        value: DotValue::String(value.to_string()),
        value_type: DotValueType::String,
        line,
    }
}

fn json_attr(key: &str, value: Value) -> DotAttribute {
    match value {
        Value::Null => DotAttribute {
            key: key.to_string(),
            value: DotValue::Null,
            value_type: DotValueType::String,
            line: 0,
        },
        Value::Bool(value) => DotAttribute {
            key: key.to_string(),
            value: DotValue::Boolean(value),
            value_type: DotValueType::Boolean,
            line: 0,
        },
        Value::Number(value) => {
            if let Some(integer) = value.as_i64() {
                DotAttribute {
                    key: key.to_string(),
                    value: DotValue::Integer(integer),
                    value_type: DotValueType::Integer,
                    line: 0,
                }
            } else {
                DotAttribute {
                    key: key.to_string(),
                    value: DotValue::Float(value.as_f64().unwrap_or_default()),
                    value_type: DotValueType::Float,
                    line: 0,
                }
            }
        }
        Value::String(value) => string_attr(key, &value),
        Value::Array(_) | Value::Object(_) => string_attr(key, &value.to_string()),
    }
}

fn sorted_join(mut values: Vec<String>) -> String {
    values.sort();
    values.join(", ")
}

pub fn handler_attr_json(attrs: &BTreeMap<String, DotAttribute>, key: &str) -> Option<Value> {
    attrs.get(key).map(|attr| dot_value_to_json(&attr.value))
}
