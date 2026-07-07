use std::collections::{BTreeMap, BTreeSet, VecDeque};

use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::conditions::split_condition_clauses;
use crate::context::validate_context_key;
use crate::graph::{
    DotAttribute, DotEdge, DotGraph, DotNode, DotScopeDefaults, DotValue, DotValueType,
};

pub type FlowMetadata = BTreeMap<String, Value>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FlowDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub goal: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<FlowInput>,
    #[serde(default, skip_serializing_if = "FlowDefaults::is_empty")]
    pub defaults: FlowDefaults,
    #[serde(default)]
    pub nodes: BTreeMap<String, FlowNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<FlowEdge>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: FlowMetadata,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FlowInput {
    pub key: String,
    #[serde(default)]
    pub label: String,
    #[serde(default = "default_input_type")]
    pub r#type: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FlowDefaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fidelity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, Value>,
}

impl FlowDefaults {
    fn is_empty(&self) -> bool {
        self.fidelity.is_none()
            && self.max_retries.is_none()
            && self.llm_provider.is_none()
            && self.llm_profile.is_none()
            && self.llm_model.is_none()
            && self.reasoning_effort.is_none()
            && self.extensions.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Start,
    Exit,
    AgentTask,
    HumanGate,
    Conditional,
    Parallel,
    FanIn,
    Tool,
    Subflow,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FlowNode {
    pub kind: NodeKind,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<NodeConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub context: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<NodeRuntimeConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contracts: Option<NodeContracts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manager: Option<ManagerLoopConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<ExecutionConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<UiConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeConfig {
    Start {},
    Exit {},
    AgentTask {
        #[serde(default)]
        prompt: String,
    },
    HumanGate {
        #[serde(default)]
        prompt: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        decisions: Vec<HumanDecision>,
    },
    Conditional {},
    Parallel {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        join_policy: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_parallel: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        join_k: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        join_quorum: Option<f64>,
    },
    FanIn {},
    Tool {
        #[serde(default)]
        command: String,
    },
    Subflow {
        flow_ref: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        input_map: BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NodeRuntimeConfig {
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_partial: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub auto_status: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub goal_gate: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fidelity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_retry_target: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NodeContracts {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reads_context: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writes_context: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repair_attempts: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_mode: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ManagerLoopConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_interval: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cycles: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_condition: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steer_cooldown: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_autostart: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_workdir: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HumanDecision {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RetryConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExecutionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UiConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FlowEdge {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub condition: String,
    #[serde(default)]
    pub weight: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowDiagnostic {
    pub rule_id: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("flow definition validation failed with {count} error(s): {detail}")]
pub struct FlowDefinitionError {
    pub count: usize,
    pub detail: String,
    pub diagnostics: Vec<FlowDiagnostic>,
}

impl FlowDefinition {
    pub fn from_yaml_str(source: &str) -> Result<Self, FlowDefinitionError> {
        serde_yaml::from_str::<Self>(source).map_err(|source| FlowDefinitionError {
            count: 1,
            detail: format!("invalid YAML flow definition: {source}"),
            diagnostics: vec![FlowDiagnostic {
                rule_id: "parse_error".to_string(),
                message: source.to_string(),
                node_id: None,
                edge: None,
            }],
        })
    }

    pub fn validate(&self) -> Result<(), FlowDefinitionError> {
        let diagnostics = self.diagnostics();
        if diagnostics.is_empty() {
            return Ok(());
        }
        Err(FlowDefinitionError {
            count: diagnostics.len(),
            detail: diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.clone())
                .collect::<Vec<_>>()
                .join("; "),
            diagnostics,
        })
    }

    pub fn normalize(&self) -> Self {
        let mut normalized = self.clone();
        normalized.schema_version = normalized.schema_version.trim().to_string();
        normalized.id = normalized.id.trim().to_string();
        normalized.title = normalized.title.trim().to_string();
        normalized.description = normalized.description.trim().to_string();
        normalized.goal = normalized.goal.trim().to_string();
        normalized.edges.sort_by(|left, right| {
            left.from
                .cmp(&right.from)
                .then_with(|| left.weight.cmp(&right.weight))
                .then_with(|| left.to.cmp(&right.to))
                .then_with(|| left.label.cmp(&right.label))
        });
        normalized
    }

    pub fn to_canonical_json_value(&self) -> Value {
        serde_json::to_value(self.normalize()).unwrap_or_else(|_| json!({}))
    }

    pub fn to_canonical_json_string(&self) -> String {
        serde_json::to_string_pretty(&self.to_canonical_json_value())
            .unwrap_or_else(|_| "{}".to_string())
    }

    pub fn to_runtime_dot_graph(&self) -> DotGraph {
        let mut graph_attrs = BTreeMap::new();
        insert_string_attr(&mut graph_attrs, "schema_version", &self.schema_version);
        insert_string_attr(&mut graph_attrs, "label", &self.title);
        insert_string_attr(&mut graph_attrs, "spark.title", &self.title);
        insert_string_attr(&mut graph_attrs, "spark.description", &self.description);
        insert_string_attr(&mut graph_attrs, "goal", &self.goal);
        if let Some(value) = self.defaults.fidelity.as_deref() {
            insert_string_attr(&mut graph_attrs, "default_fidelity", value);
        }
        if let Some(value) = self.defaults.max_retries {
            insert_i64_attr(&mut graph_attrs, "default_max_retries", value as i64);
        }
        if let Some(value) = self.defaults.llm_provider.as_deref() {
            insert_string_attr(&mut graph_attrs, "ui_default_llm_provider", value);
        }
        if let Some(value) = self.defaults.llm_profile.as_deref() {
            insert_string_attr(&mut graph_attrs, "ui_default_llm_profile", value);
        }
        if let Some(value) = self.defaults.llm_model.as_deref() {
            insert_string_attr(&mut graph_attrs, "ui_default_llm_model", value);
        }
        if let Some(value) = self.defaults.reasoning_effort.as_deref() {
            insert_string_attr(&mut graph_attrs, "ui_default_reasoning_effort", value);
        }
        if !self.inputs.is_empty() {
            if let Ok(value) = serde_json::to_string(&self.inputs) {
                insert_string_attr(&mut graph_attrs, "spark.launch_inputs", &value);
            }
        }
        merge_value_attrs(&mut graph_attrs, &self.metadata);
        merge_value_attrs(&mut graph_attrs, &self.extensions);

        let nodes = self
            .nodes
            .iter()
            .map(|(node_id, node)| (node_id.clone(), runtime_dot_node(node_id, node)))
            .collect();
        let edges = self
            .edges
            .iter()
            .map(|edge| runtime_dot_edge(edge))
            .collect();
        DotGraph {
            graph_id: self.id.clone(),
            graph_attrs,
            nodes,
            edges,
            defaults: DotScopeDefaults::default(),
            subgraphs: Vec::new(),
        }
    }

    pub fn diagnostics(&self) -> Vec<FlowDiagnostic> {
        let mut diagnostics = Vec::new();
        if self.id.trim().is_empty() {
            diagnostics.push(diagnostic("flow_id", "flow id must be non-empty"));
        }

        for node_id in self.nodes.keys() {
            if !valid_id(node_id) {
                diagnostics.push(node_diagnostic(
                    "node_id",
                    format!("invalid node id '{node_id}'"),
                    node_id,
                ));
            }
        }

        let start_nodes = self
            .nodes
            .iter()
            .filter(|(_, node)| node.kind == NodeKind::Start)
            .map(|(node_id, _)| node_id.clone())
            .collect::<Vec<_>>();
        if start_nodes.len() != 1 {
            diagnostics.push(diagnostic(
                "start_node",
                format!(
                    "flow must have exactly one start node, found {}",
                    start_nodes.len()
                ),
            ));
        }
        let exit_nodes = self
            .nodes
            .iter()
            .filter(|(_, node)| node.kind == NodeKind::Exit)
            .map(|(node_id, _)| node_id.clone())
            .collect::<BTreeSet<_>>();
        if exit_nodes.len() != 1 {
            diagnostics.push(diagnostic(
                "exit_node",
                format!(
                    "flow must have exactly one exit node, found {}",
                    exit_nodes.len()
                ),
            ));
        }

        for (node_id, node) in &self.nodes {
            for key in node.context.keys() {
                if let Err(error) = validate_context_key(key) {
                    diagnostics.push(node_diagnostic("context_key", format!("{error}"), node_id));
                }
            }
            if let Some(contracts) = node.contracts.as_ref() {
                for key in contracts
                    .reads_context
                    .iter()
                    .chain(contracts.writes_context.iter())
                {
                    if let Err(error) = validate_context_key(key) {
                        diagnostics.push(node_diagnostic(
                            "context_contract",
                            format!("{error}"),
                            node_id,
                        ));
                    }
                }
            }
            if let Some(NodeConfig::HumanGate { decisions, .. }) = node.config.as_ref() {
                let mut values = BTreeSet::new();
                for decision in decisions {
                    if decision.value.trim().is_empty() {
                        diagnostics.push(node_diagnostic(
                            "human_decision",
                            "human gate decision value must be non-empty",
                            node_id,
                        ));
                    }
                    if !values.insert(decision.value.trim().to_string()) {
                        diagnostics.push(node_diagnostic(
                            "human_decision",
                            format!("duplicate human gate decision '{}'", decision.value),
                            node_id,
                        ));
                    }
                }
            }
            if let Some(NodeConfig::Subflow {
                flow_ref,
                input_map,
            }) = node.config.as_ref()
            {
                if !valid_flow_ref(flow_ref) {
                    diagnostics.push(node_diagnostic(
                        "subflow_ref",
                        format!("invalid subflow reference '{flow_ref}'"),
                        node_id,
                    ));
                }
                for child_key in input_map.keys() {
                    if let Err(error) = validate_context_key(child_key) {
                        diagnostics.push(node_diagnostic(
                            "subflow_input_map",
                            format!("invalid child input key '{child_key}': {error}"),
                            node_id,
                        ));
                    }
                }
            }
        }

        for edge in &self.edges {
            if !self.nodes.contains_key(&edge.from) {
                diagnostics.push(edge_diagnostic(
                    "edge_source",
                    format!(
                        "edge source '{}' does not reference an existing node",
                        edge.from
                    ),
                    edge,
                ));
            }
            if !self.nodes.contains_key(&edge.to) {
                diagnostics.push(edge_diagnostic(
                    "edge_target",
                    format!(
                        "edge target '{}' does not reference an existing node",
                        edge.to
                    ),
                    edge,
                ));
            }
            if exit_nodes.contains(&edge.from) {
                diagnostics.push(edge_diagnostic(
                    "exit_outgoing",
                    format!("exit node '{}' must not have outgoing edges", edge.from),
                    edge,
                ));
            }
            if !condition_is_parseable(&edge.condition) {
                diagnostics.push(edge_diagnostic(
                    "condition",
                    format!("invalid edge condition '{}'", edge.condition),
                    edge,
                ));
            }
        }

        if diagnostics.iter().all(|diagnostic| {
            diagnostic.rule_id != "edge_source" && diagnostic.rule_id != "edge_target"
        }) {
            diagnostics.extend(self.reachability_diagnostics(start_nodes.first()));
        }

        diagnostics
    }

    fn reachability_diagnostics(&self, start_node: Option<&String>) -> Vec<FlowDiagnostic> {
        let Some(start_node) = start_node else {
            return Vec::new();
        };
        let mut outgoing = BTreeMap::<&str, Vec<&str>>::new();
        for edge in &self.edges {
            outgoing
                .entry(edge.from.as_str())
                .or_default()
                .push(edge.to.as_str());
        }
        let mut reachable = BTreeSet::new();
        let mut queue = VecDeque::from([start_node.as_str()]);
        while let Some(node_id) = queue.pop_front() {
            if !reachable.insert(node_id) {
                continue;
            }
            if let Some(targets) = outgoing.get(node_id) {
                for target in targets {
                    queue.push_back(target);
                }
            }
        }
        self.nodes
            .keys()
            .filter(|node_id| !reachable.contains(node_id.as_str()))
            .map(|node_id| {
                node_diagnostic(
                    "reachability",
                    format!("node '{node_id}' is unreachable from start node"),
                    node_id,
                )
            })
            .collect()
    }
}

impl Default for NodeKind {
    fn default() -> Self {
        Self::AgentTask
    }
}

impl Default for FlowDefinition {
    fn default() -> Self {
        Self {
            schema_version: default_schema_version(),
            id: String::new(),
            title: String::new(),
            description: String::new(),
            goal: String::new(),
            inputs: Vec::new(),
            defaults: FlowDefaults::default(),
            nodes: BTreeMap::new(),
            edges: Vec::new(),
            metadata: BTreeMap::new(),
            extensions: BTreeMap::new(),
        }
    }
}

pub fn flow_definition_schema_value() -> Value {
    serde_json::to_value(schema_for!(FlowDefinition)).unwrap_or_else(|_| json!({}))
}

fn runtime_dot_node(node_id: &str, node: &FlowNode) -> DotNode {
    let mut attrs = BTreeMap::new();
    insert_string_attr(&mut attrs, "label", &node.label);
    insert_string_attr(&mut attrs, "description", &node.description);
    insert_string_attr(&mut attrs, "shape", runtime_shape(&node.kind));
    insert_string_attr(&mut attrs, "type", runtime_handler_type(&node.kind));
    if let Some(prompt) = node_prompt(node) {
        insert_string_attr(&mut attrs, "prompt", &prompt);
    }
    if let Some(NodeConfig::Tool { command }) = node.config.as_ref() {
        insert_string_attr(&mut attrs, "tool.command", command);
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
            insert_string_attr(&mut attrs, "llm_provider", value);
        }
        if let Some(value) = execution.llm_profile.as_deref() {
            insert_string_attr(&mut attrs, "llm_profile", value);
        }
        if let Some(value) = execution.llm_model.as_deref() {
            insert_string_attr(&mut attrs, "llm_model", value);
        }
        if let Some(value) = execution.reasoning_effort.as_deref() {
            insert_string_attr(&mut attrs, "reasoning_effort", value);
        }
    }
    if let Some(NodeConfig::HumanGate { decisions, .. }) = node.config.as_ref() {
        if let Ok(value) = serde_json::to_string(decisions) {
            insert_string_attr(&mut attrs, "options", &value);
        }
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
            attrs.insert(
                "join_quorum".to_string(),
                DotAttribute {
                    key: "join_quorum".to_string(),
                    value: DotValue::Float(*value),
                    value_type: DotValueType::Float,
                    line: 0,
                },
            );
        }
    }
    merge_value_attrs(&mut attrs, &node.context);
    merge_value_attrs(&mut attrs, &node.extensions);
    let explicit_attr_keys = attrs.keys().cloned().collect();
    DotNode {
        node_id: node_id.to_string(),
        attrs,
        line: 0,
        declaration_order: 0,
        explicit_attr_keys,
    }
}

fn runtime_dot_edge(edge: &FlowEdge) -> DotEdge {
    let mut attrs = BTreeMap::new();
    insert_string_attr(&mut attrs, "label", &edge.label);
    insert_string_attr(&mut attrs, "condition", &edge.condition);
    insert_i64_attr(&mut attrs, "weight", edge.weight);
    if let Some(transition) = edge.transition.as_deref() {
        insert_string_attr(&mut attrs, "transition", transition);
    }
    merge_value_attrs(&mut attrs, &edge.extensions);
    DotEdge {
        source: edge.from.clone(),
        target: edge.to.clone(),
        attrs,
        line: 0,
    }
}

fn runtime_shape(kind: &NodeKind) -> &'static str {
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

fn runtime_handler_type(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Start => "start",
        NodeKind::Exit => "exit",
        NodeKind::AgentTask => "codergen",
        NodeKind::HumanGate => "wait.human",
        NodeKind::Conditional => "conditional",
        NodeKind::Parallel => "parallel",
        NodeKind::FanIn => "parallel.fan_in",
        NodeKind::Tool => "tool",
        NodeKind::Subflow => "stack.manager_loop",
    }
}

fn node_prompt(node: &FlowNode) -> Option<String> {
    match node.config.as_ref()? {
        NodeConfig::AgentTask { prompt } | NodeConfig::HumanGate { prompt, .. } => {
            Some(prompt.trim().to_string())
        }
        _ => None,
    }
    .filter(|value| !value.is_empty())
}

fn merge_value_attrs(attrs: &mut BTreeMap<String, DotAttribute>, values: &BTreeMap<String, Value>) {
    for (key, value) in values {
        attrs.insert(key.clone(), json_attr(key, value.clone()));
    }
}

fn insert_string_attr(attrs: &mut BTreeMap<String, DotAttribute>, key: &str, value: &str) {
    if value.trim().is_empty() {
        return;
    }
    attrs.insert(
        key.to_string(),
        DotAttribute {
            key: key.to_string(),
            value: DotValue::String(value.to_string()),
            value_type: DotValueType::String,
            line: 0,
        },
    );
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

fn json_attr(key: &str, value: Value) -> DotAttribute {
    let (value, value_type) = match value {
        Value::Null => (DotValue::Null, DotValueType::String),
        Value::Bool(value) => (DotValue::Boolean(value), DotValueType::Boolean),
        Value::Number(number) if number.is_i64() => (
            DotValue::Integer(number.as_i64().unwrap_or_default()),
            DotValueType::Integer,
        ),
        Value::Number(number) => (
            DotValue::Float(number.as_f64().unwrap_or_default()),
            DotValueType::Float,
        ),
        Value::String(value) => (DotValue::String(value), DotValueType::String),
        other => (
            DotValue::String(serde_json::to_string(&other).unwrap_or_default()),
            DotValueType::String,
        ),
    };
    DotAttribute {
        key: key.to_string(),
        value,
        value_type,
        line: 0,
    }
}

fn diagnostic(rule_id: &str, message: impl Into<String>) -> FlowDiagnostic {
    FlowDiagnostic {
        rule_id: rule_id.to_string(),
        message: message.into(),
        node_id: None,
        edge: None,
    }
}

fn node_diagnostic(
    rule_id: &str,
    message: impl Into<String>,
    node_id: impl Into<String>,
) -> FlowDiagnostic {
    FlowDiagnostic {
        rule_id: rule_id.to_string(),
        message: message.into(),
        node_id: Some(node_id.into()),
        edge: None,
    }
}

fn edge_diagnostic(rule_id: &str, message: impl Into<String>, edge: &FlowEdge) -> FlowDiagnostic {
    FlowDiagnostic {
        rule_id: rule_id.to_string(),
        message: message.into(),
        node_id: None,
        edge: Some((edge.from.clone(), edge.to.clone())),
    }
}

fn valid_id(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn valid_flow_ref(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && !value.starts_with('/')
        && !value.split('/').any(|part| part.is_empty() || part == "..")
        && (value.ends_with(".yaml") || value.ends_with(".yml"))
}

fn condition_is_parseable(value: &str) -> bool {
    split_condition_clauses(value)
        .into_iter()
        .all(|clause| condition_clause_is_parseable(clause.trim()))
}

fn condition_clause_is_parseable(value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    if let Some((left, right)) = value.split_once("!=").or_else(|| value.split_once('=')) {
        return valid_condition_key(left.trim()) && !right.trim().is_empty();
    }
    valid_condition_key(value)
}

fn valid_condition_key(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.')
}

fn default_schema_version() -> String {
    "1".to_string()
}

fn default_input_type() -> String {
    "string".to_string()
}

fn is_false(value: &bool) -> bool {
    !*value
}
