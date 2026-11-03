use std::collections::{BTreeMap, BTreeSet, VecDeque};

use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::context::validate_context_key;
use crate::graph::{
    DotAttribute, DotEdge, DotGraph, DotNode, DotScopeDefaults, DotValue, DotValueType,
};

pub type FlowMetadata = BTreeMap<String, Value>;

/// The only flow schema this runtime accepts.
pub const FLOW_SCHEMA_VERSION: &str = "2";
/// The response contract every agent_task declares.
pub const DECISION_ENVELOPE: &str = "decision_envelope";
/// The built-in decision of start and fan_in nodes.
pub const NEXT_DECISION: &str = "next";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlowDefinition {
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: FlowMetadata,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, Value>,
    /// Runs of flows sharing a lock identity queue and execute one at a time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_lock: Option<FlowExecutionLock>,
}

/// A flow's execution lock (`execution_lock` in `flow.md`). Scope `project`
/// resolves identity from the repository containing the working directory,
/// so linked git worktrees share one lock; `conflict_policy` is `queue`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FlowExecutionLock {
    pub scope: String,
    pub key: String,
    #[serde(default = "default_conflict_policy")]
    pub conflict_policy: String,
}

fn default_conflict_policy() -> String {
    "queue".to_string()
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
#[serde(deny_unknown_fields)]
pub struct FlowDefaults {
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "spark_common::settings::deserialize_thinking_budget")]
    pub thinking_budget_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_summary: Option<String>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, Value>,
}

impl FlowDefaults {
    pub fn is_empty(&self) -> bool {
        self.max_retries.is_none()
            && self.llm_provider.is_none()
            && self.llm_profile.is_none()
            && self.llm_model.is_none()
            && self.reasoning_effort.is_none()
            && self.thinking.is_none()
            && self.thinking_budget_tokens.is_none()
            && self.reasoning_mode.is_none()
            && self.reasoning_summary.is_none()
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
    Switch,
    Parallel,
    FanIn,
    Tool,
    Subflow,
}

impl NodeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Exit => "exit",
            Self::AgentTask => "agent_task",
            Self::HumanGate => "human_gate",
            Self::Switch => "switch",
            Self::Parallel => "parallel",
            Self::FanIn => "fan_in",
            Self::Tool => "tool",
            Self::Subflow => "subflow",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlowNode {
    pub kind: NodeKind,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub description: String,
    /// Role file (`roles/<name>.md`) whose persona and execution defaults the
    /// loader merges into this `agent_task` node. Only the authored form sets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
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
    /// The node's complete decision vocabulary: each decision ID and the
    /// one route it selects. Edges are derived from this map.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub routes: BTreeMap<String, FlowRoute>,
}

/// One declared decision: its target and what choosing it means. `label` is
/// display text only; it has no routing authority.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlowRoute {
    pub to: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// Presentation metadata, such as canvas layout hints.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, Value>,
}

/// An exit's declared terminal result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExitResult {
    pub name: String,
    pub outcome: ExitOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExitOutcome {
    Success,
    Failure,
}

impl ExitOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NodeConfig {
    Start {},
    Exit {
        result: ExitResult,
        /// When true, reaching this exit produces the run result by pointing
        /// a summarizer agent at the run's recorded transcripts and
        /// artifacts. Failed runs use the first opted-in exit by node ID.
        #[serde(default, skip_serializing_if = "is_false")]
        result_summary: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result_summary_prompt: Option<String>,
    },
    AgentTask {
        #[serde(default)]
        prompt: String,
    },
    HumanGate {
        #[serde(default)]
        prompt: String,
    },
    /// Selects a declared decision by comparing one context value with JSON
    /// scalar case keys (`"0"` is number 0, `'"0"'` is string "0").
    Switch {
        on: String,
        #[serde(default)]
        cases: BTreeMap<String, String>,
        otherwise: String,
    },
    Parallel {
        branches: Vec<String>,
        join: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_parallel: Option<u64>,
        /// Keep a branch that ended with a tolerable execution error and
        /// still enter the join; fatal conditions always fail the owner.
        #[serde(default, skip_serializing_if = "is_false")]
        tolerate_branch_execution_errors: bool,
    },
    FanIn {},
    Tool {
        #[serde(default)]
        command: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env_map: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        output_map: BTreeMap<String, String>,
        /// Exit-code adapter: each listed code (a decimal integer key)
        /// selects a declared decision.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        exit_codes: BTreeMap<String, String>,
        /// JSON adapter: the named top-level field of the tool's single JSON
        /// stdout object is the decision.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decision_field: Option<String>,
    },
    Subflow {
        flow_ref: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        input_map: BTreeMap<String, String>,
        /// Child context keys copied into the parent (`child_key: parent_key`)
        /// when the child run ends; an absent child key clears the parent key.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        output_map: BTreeMap<String, String>,
        /// Child terminal result name -> local decision.
        #[serde(default)]
        results: BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeRuntimeConfig {
    /// Behavior when this node was started but not checkpointed before a restart.
    #[serde(default, skip_serializing_if = "RecoveryPolicy::is_default")]
    pub recovery_policy: RecoveryPolicy,
    /// Named session (`session: <name>`) this agent_task node continues.
    /// Nodes sharing a name continue one provider session; absent means a
    /// fresh session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Context key (`workdir_from: context.workspace.path`) naming the
    /// directory this agent_task node's agent runs in; same rules as
    /// `manager.child_workdir_from`. Absent means the run's working directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    /// Maximum number of times a transition may enter this node within one
    /// run (or one branch invocation). A decision selecting an exhausted
    /// target fails with `visit_limit_reached`. Absent means the engine
    /// default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_visits: Option<u64>,
    /// Maximum node execution time, in seconds or as a duration such as
    /// `90s`, `15m` or `2h`. Agent and tool nodes past it fail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<String>,
}

/// The longest node `timeout`, so every node deadline is representable.
pub const MAX_NODE_TIMEOUT_SECONDS: f64 = 365.0 * 86400.0;

impl NodeRuntimeConfig {
    /// `timeout` in seconds, from `30`, `250ms`, `90s`, `15m`, `2h` or `1d`.
    pub fn timeout_seconds(&self) -> Result<Option<f64>, String> {
        let Some(raw) = self.timeout.as_deref() else {
            return Ok(None);
        };
        let (number, scale) = [
            ("ms", 0.001),
            ("s", 1.0),
            ("m", 60.0),
            ("h", 3600.0),
            ("d", 86400.0),
        ]
        .into_iter()
        .find_map(|(unit, scale)| raw.trim().strip_suffix(unit).map(|number| (number, scale)))
        .unwrap_or((raw.trim(), 1.0));
        match number.trim().parse::<f64>().map(|number| number * scale) {
            Ok(seconds) if seconds > 0.0 && seconds <= MAX_NODE_TIMEOUT_SECONDS => Ok(Some(seconds)),
            _ => Err(format!(
                "runtime.timeout '{raw}' must be a positive duration up to 365d, such as 30, 90s, 15m or 2h"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryPolicy {
    #[default]
    Rerun,
    Pause,
}

impl RecoveryPolicy {
    fn is_default(&self) -> bool {
        matches!(self, Self::Rerun)
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_workdir_from: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "spark_common::settings::deserialize_thinking_budget")]
    pub thinking_budget_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_summary: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UiConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
}

/// A derived, read-only edge for graph views: a declared route, or a fork's
/// structural branch or join connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowEdge {
    pub from: String,
    pub to: String,
    pub kind: FlowEdgeKind,
    /// The route's decision ID; None for a structural edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowEdgeKind {
    Route,
    Branch,
    Join,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowDiagnostic {
    pub rule_id: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("flow definition validation failed with {count} error(s): {detail}")]
pub struct FlowDefinitionError {
    pub count: usize,
    pub detail: String,
    pub diagnostics: Vec<FlowDiagnostic>,
}

impl FlowDefinitionError {
    pub fn from_diagnostics(diagnostics: Vec<FlowDiagnostic>) -> Self {
        Self {
            count: diagnostics.len(),
            detail: diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.clone())
                .collect::<Vec<_>>()
                .join("; "),
            diagnostics,
        }
    }
}

impl FlowDefinition {
    pub fn from_json_str(source: &str) -> Result<Self, FlowDefinitionError> {
        serde_json::from_str::<Self>(source).map_err(|source| {
            FlowDefinitionError::from_diagnostics(vec![FlowDiagnostic {
                rule_id: "parse_error".to_string(),
                message: format!("invalid JSON flow definition: {source}"),
                node_id: None,
                edge: None,
                decision: None,
            }])
        })
    }

    pub fn validate(&self) -> Result<(), FlowDefinitionError> {
        let diagnostics = self.diagnostics();
        if diagnostics.is_empty() {
            return Ok(());
        }
        Err(FlowDefinitionError::from_diagnostics(diagnostics))
    }

    pub fn normalize(&self) -> Self {
        let mut normalized = self.clone();
        normalized.schema_version = normalized.schema_version.trim().to_string();
        normalized.id = normalized.id.trim().to_string();
        normalized.title = normalized.title.trim().to_string();
        normalized.description = normalized.description.trim().to_string();
        normalized.goal = normalized.goal.trim().to_string();
        // Canonical forms, so a flow survives the directory format unchanged:
        // every node has its kind's config, prompts are trimmed (the runtime
        // trims them anyway) and commands end with a newline.
        for node in normalized.nodes.values_mut() {
            if node.config.is_none() {
                node.config = default_node_config(&node.kind);
            }
            match node.config.as_mut() {
                Some(NodeConfig::AgentTask { prompt } | NodeConfig::HumanGate { prompt }) => {
                    *prompt = prompt.trim().to_string();
                }
                Some(NodeConfig::Exit {
                    result_summary_prompt,
                    ..
                }) => {
                    *result_summary_prompt = result_summary_prompt
                        .as_deref()
                        .map(str::trim)
                        .filter(|prompt| !prompt.is_empty())
                        .map(str::to_string);
                }
                Some(NodeConfig::Tool { command, .. })
                    if !command.is_empty() && !command.ends_with('\n') =>
                {
                    command.push('\n');
                }
                _ => {}
            }
            for route in node.routes.values_mut() {
                route.to = route.to.trim().to_string();
                route.description = route.description.trim().to_string();
                route.label = route.label.trim().to_string();
            }
        }
        normalized
    }

    pub fn to_canonical_json_value(&self) -> Value {
        serde_json::to_value(self.normalize()).unwrap_or_else(|_| json!({}))
    }

    pub fn to_canonical_json_string(&self) -> String {
        serde_json::to_string_pretty(&self.to_canonical_json_value())
            .unwrap_or_else(|_| "{}".to_string())
    }

    /// Every derived edge: each node's routes in decision order, then each
    /// fork's branch and join connections in declaration order.
    pub fn edges(&self) -> Vec<FlowEdge> {
        let mut edges = Vec::new();
        for (node_id, node) in &self.nodes {
            for (decision, route) in &node.routes {
                edges.push(FlowEdge {
                    from: node_id.clone(),
                    to: route.to.clone(),
                    kind: FlowEdgeKind::Route,
                    decision: Some(decision.clone()),
                    description: route.description.clone(),
                    label: route.label.clone(),
                });
            }
            if let Some(NodeConfig::Parallel { branches, join, .. }) = node.config.as_ref() {
                for branch in branches {
                    edges.push(FlowEdge {
                        from: node_id.clone(),
                        to: branch.clone(),
                        kind: FlowEdgeKind::Branch,
                        decision: None,
                        description: String::new(),
                        label: String::new(),
                    });
                }
                edges.push(FlowEdge {
                    from: node_id.clone(),
                    to: join.clone(),
                    kind: FlowEdgeKind::Join,
                    decision: None,
                    description: String::new(),
                    label: String::new(),
                });
            }
        }
        edges
    }

    /// The route `decision` selects at `node_id`, by exact lookup.
    pub fn route(&self, node_id: &str, decision: &str) -> Option<&FlowRoute> {
        self.nodes.get(node_id)?.routes.get(decision)
    }

    /// The declared result of an exit node.
    pub fn exit_result(&self, node_id: &str) -> Option<&ExitResult> {
        match self.nodes.get(node_id)?.config.as_ref()? {
            NodeConfig::Exit { result, .. } => Some(result),
            _ => None,
        }
    }

    /// Each declared terminal result name and its outcome.
    pub fn terminal_results(&self) -> BTreeMap<String, ExitOutcome> {
        self.nodes
            .keys()
            .filter_map(|node_id| self.exit_result(node_id))
            .map(|result| (result.name.clone(), result.outcome))
            .collect()
    }

    pub fn to_runtime_dot_graph(&self) -> DotGraph {
        let mut graph_attrs = BTreeMap::new();
        insert_string_attr(&mut graph_attrs, "schema_version", &self.schema_version);
        insert_string_attr(&mut graph_attrs, "label", &self.title);
        insert_string_attr(&mut graph_attrs, "spark.title", &self.title);
        insert_string_attr(&mut graph_attrs, "spark.description", &self.description);
        insert_string_attr(&mut graph_attrs, "goal", &self.goal);
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
        if let Some(value) = self.defaults.thinking.as_deref() {
            insert_string_attr(&mut graph_attrs, "thinking", value);
        }
        if let Some(value) = self.defaults.thinking_budget_tokens {
            insert_string_attr(
                &mut graph_attrs,
                "thinking_budget_tokens",
                &value.to_string(),
            );
        }
        if let Some(value) = self.defaults.reasoning_mode.as_deref() {
            insert_string_attr(&mut graph_attrs, "reasoning_mode", value);
        }
        if let Some(value) = self.defaults.reasoning_summary.as_deref() {
            insert_string_attr(&mut graph_attrs, "reasoning_summary", value);
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
        let edges = self.edges().iter().map(dot_edge).collect();
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
        if self.schema_version.trim() != FLOW_SCHEMA_VERSION {
            diagnostics.push(diagnostic(
                "schema_version",
                format!(
                    "flow schema_version must be '{FLOW_SCHEMA_VERSION}', found '{}'; copy the current bundled flow or convert this flow's routes to declared decisions",
                    self.schema_version.trim()
                ),
            ));
        }
        for (node_id, thinking, budget, mode, summary) in std::iter::once((
            None,
            self.defaults.thinking.as_deref(),
            self.defaults.thinking_budget_tokens,
            self.defaults.reasoning_mode.as_deref(),
            self.defaults.reasoning_summary.as_deref(),
        ))
        .chain(self.nodes.iter().filter_map(|(id, node)| {
            node.execution.as_ref().map(|v| {
                (
                    Some(id),
                    v.thinking.as_deref(),
                    v.thinking_budget_tokens,
                    v.reasoning_mode.as_deref(),
                    v.reasoning_summary.as_deref(),
                )
            })
        })) {
            if let Err(message) = spark_common::settings::validate_reasoning_control_fields(
                thinking, budget, mode, summary,
            ) {
                diagnostics.push(FlowDiagnostic {
                    rule_id: "reasoning_controls".into(),
                    message,
                    node_id: node_id.cloned(),
                    edge: None,
                    decision: None,
                });
            }
        }
        if self.id.trim().is_empty() {
            diagnostics.push(diagnostic("flow_id", "flow id must be non-empty"));
        }
        if let Some(lock) = &self.execution_lock {
            if !lock.scope.trim().eq_ignore_ascii_case("project") {
                diagnostics.push(diagnostic(
                    "execution_lock",
                    "execution_lock.scope must be project",
                ));
            }
            if lock.key.trim().is_empty() {
                diagnostics.push(diagnostic(
                    "execution_lock",
                    "execution_lock.key must be non-empty",
                ));
            }
            let policy = lock.conflict_policy.trim();
            if !policy.is_empty() && !policy.eq_ignore_ascii_case("queue") {
                diagnostics.push(diagnostic(
                    "execution_lock",
                    "execution_lock.conflict_policy must be queue",
                ));
            }
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
        if !self.nodes.values().any(|node| node.kind == NodeKind::Exit) {
            diagnostics.push(diagnostic(
                "exit_node",
                "flow must have at least one exit node",
            ));
        }

        for (node_id, node) in &self.nodes {
            diagnostics.extend(self.node_setting_diagnostics(node_id, node));
            diagnostics.extend(self.node_route_diagnostics(node_id, node));
        }
        diagnostics.extend(self.exit_result_diagnostics());
        diagnostics.extend(self.session_diagnostics());
        for (key, _) in self
            .extensions
            .iter()
            .filter(|(key, _)| is_removed_field(key) || is_legacy_routing_field(key))
        {
            diagnostics.push(diagnostic(
                "removed_field",
                format!("flow sets {key}, which is no longer supported"),
            ));
        }

        // Graph-shape checks need every target to exist.
        if diagnostics.iter().all(|diagnostic| {
            !matches!(
                diagnostic.rule_id.as_str(),
                "route_target" | "fork_structure" | "node_config"
            )
        }) {
            diagnostics.extend(self.reachability_diagnostics(start_nodes.first()));
            diagnostics.extend(self.fork_diagnostics());
            diagnostics.extend(self.switch_writer_diagnostics());
        }

        diagnostics
    }

    fn node_setting_diagnostics(&self, node_id: &str, node: &FlowNode) -> Vec<FlowDiagnostic> {
        let mut diagnostics = Vec::new();
        if let Some(Err(message)) = node
            .runtime
            .as_ref()
            .map(NodeRuntimeConfig::timeout_seconds)
        {
            diagnostics.push(node_diagnostic("node_timeout", message, node_id));
        }
        if node.runtime.as_ref().and_then(|runtime| runtime.max_visits) == Some(0) {
            diagnostics.push(node_diagnostic(
                "max_visits",
                format!("node '{node_id}' sets runtime.max_visits 0; budgets must be positive"),
                node_id,
            ));
        }
        if let Some(role) = node.role.as_deref() {
            if node.kind != NodeKind::AgentTask {
                diagnostics.push(node_diagnostic(
                    "role",
                    format!("role '{role}' is only allowed on agent_task nodes"),
                    node_id,
                ));
            } else if !valid_id(role) {
                diagnostics.push(node_diagnostic(
                    "role",
                    format!("invalid role name '{role}'"),
                    node_id,
                ));
            }
        }
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
        if node.kind == NodeKind::AgentTask {
            let response = node
                .contracts
                .as_ref()
                .and_then(|contracts| contracts.response.as_deref())
                .map(str::trim);
            if response != Some(DECISION_ENVELOPE) {
                diagnostics.push(node_diagnostic(
                    "response_contract",
                    format!(
                        "agent_task '{node_id}' must declare contracts.response: {DECISION_ENVELOPE}"
                    ),
                    node_id,
                ));
            }
        } else if let Some(response) = node
            .contracts
            .as_ref()
            .and_then(|contracts| contracts.response.as_deref())
        {
            diagnostics.push(node_diagnostic(
                "response_contract",
                format!("node '{node_id}' sets contracts.response '{response}', which only agent_task nodes take"),
                node_id,
            ));
        }
        if let Some(session) = node.runtime.as_ref().and_then(|r| r.session.as_deref()) {
            if node.kind != NodeKind::AgentTask {
                diagnostics.push(node_diagnostic(
                    "session",
                    format!("session '{session}' is only allowed on agent_task nodes"),
                    node_id,
                ));
            } else if !valid_id(session) {
                diagnostics.push(node_diagnostic(
                    "session",
                    format!("invalid session name '{session}'"),
                    node_id,
                ));
            }
        }
        if let Some(key) = node
            .runtime
            .as_ref()
            .and_then(|r| r.workdir_from.as_deref())
        {
            if node.kind != NodeKind::AgentTask {
                diagnostics.push(node_diagnostic(
                    "workdir_from",
                    format!("workdir_from '{key}' is only allowed on agent_task nodes"),
                    node_id,
                ));
            } else if let Err(error) = validate_context_key(key) {
                diagnostics.push(node_diagnostic("workdir_from", format!("{error}"), node_id));
            }
        }
        for (key, _) in node
            .extensions
            .iter()
            .filter(|(key, _)| is_removed_field(key) || is_legacy_routing_field(key))
        {
            diagnostics.push(node_diagnostic(
                "removed_field",
                format!("node '{node_id}' sets {key}, which is no longer supported"),
                node_id,
            ));
        }
        if let Some(NodeConfig::Subflow {
            flow_ref,
            input_map,
            output_map,
            ..
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
            let writes = node
                .contracts
                .as_ref()
                .map(|contracts| contracts.writes_context.as_slice())
                .unwrap_or_default();
            for parent_key in output_map.values() {
                if !writes.contains(parent_key) {
                    diagnostics.push(node_diagnostic(
                        "subflow_output_map",
                        format!(
                            "output_map parent key '{parent_key}' must be listed in the node's writes_context"
                        ),
                        node_id,
                    ));
                }
            }
        }
        diagnostics
    }

    /// Route-map and per-kind decision-domain rules for one node.
    fn node_route_diagnostics(&self, node_id: &str, node: &FlowNode) -> Vec<FlowDiagnostic> {
        let mut diagnostics = Vec::new();
        let file = format!("{node_id}.md");
        for (decision, route) in &node.routes {
            if !valid_decision_id(decision) {
                diagnostics.push(decision_diagnostic(
                    "decision_id",
                    format!("{file}: decision '{decision}' must match [a-z][a-z0-9_]*"),
                    node_id,
                    decision,
                ));
            }
            if route.description.trim().is_empty() {
                diagnostics.push(decision_diagnostic(
                    "route_description",
                    format!("{file}: decision '{decision}' needs a nonempty description"),
                    node_id,
                    decision,
                ));
            }
            if !self.nodes.contains_key(route.to.trim()) {
                diagnostics.push(FlowDiagnostic {
                    rule_id: "route_target".to_string(),
                    message: format!(
                        "{file}: decision '{decision}' targets '{}', which is not a node",
                        route.to
                    ),
                    node_id: Some(node_id.to_string()),
                    edge: Some((node_id.to_string(), route.to.clone())),
                    decision: Some(decision.clone()),
                });
            }
            for key in route
                .extensions
                .keys()
                .filter(|key| is_legacy_routing_field(key))
            {
                diagnostics.push(decision_diagnostic(
                    "removed_field",
                    format!(
                        "{file}: decision '{decision}' sets {key}, which is no longer supported"
                    ),
                    node_id,
                    decision,
                ));
            }
        }
        let decisions = node.routes.keys().cloned().collect::<BTreeSet<_>>();
        let mut config_error = |message: String| {
            diagnostics.push(node_diagnostic(
                "node_config",
                format!("{file}: {message}"),
                node_id,
            ));
        };
        match node.config.as_ref() {
            None if default_node_config(&node.kind).is_none() => {
                config_error(format!("{} nodes require config", node.kind.as_str()))
            }
            None => {}
            Some(config) if config_kind(config) != node.kind => config_error(format!(
                "config kind {} does not match node kind {}",
                config_kind(config).as_str(),
                node.kind.as_str()
            )),
            Some(_) => {}
        }
        let mut domain_messages = Vec::<String>::new();
        let mut structure_messages = Vec::<String>::new();
        let mut domain = |message: String| domain_messages.push(message);
        match (&node.kind, node.config.as_ref()) {
            (NodeKind::Start, _) | (NodeKind::FanIn, _) => {
                if decisions != BTreeSet::from([NEXT_DECISION.to_string()]) {
                    domain(format!(
                        "{} nodes declare exactly the decision '{NEXT_DECISION}'",
                        node.kind.as_str()
                    ));
                }
            }
            (NodeKind::Exit, config) => {
                if !decisions.is_empty() {
                    domain("exit nodes declare no routes".to_string());
                }
                if let Some(NodeConfig::Exit { result, .. }) = config {
                    if !valid_decision_id(&result.name) {
                        domain(format!(
                            "exit result name '{}' must match [a-z][a-z0-9_]*",
                            result.name
                        ));
                    }
                }
            }
            (NodeKind::Parallel, config) => {
                if !decisions.is_empty() {
                    domain("parallel nodes declare branches and a join, not routes".to_string());
                }
                if let Some(NodeConfig::Parallel {
                    branches,
                    join,
                    max_parallel,
                    ..
                }) = config
                {
                    if *max_parallel == Some(0) {
                        domain("max_parallel must be positive".to_string());
                    }
                    if branches.is_empty() {
                        domain("parallel nodes need at least one branch".to_string());
                    }
                    let mut seen = BTreeSet::new();
                    for branch in branches {
                        if !seen.insert(branch) {
                            domain(format!("branch '{branch}' is listed twice"));
                        }
                    }
                    for target in branches.iter().chain(std::iter::once(join)) {
                        if !self.nodes.contains_key(target) {
                            structure_messages.push(format!("fork target '{target}' is not a node"));
                        }
                    }
                    if self
                        .nodes
                        .get(join)
                        .is_some_and(|join| join.kind != NodeKind::FanIn)
                    {
                        structure_messages.push(format!("join '{join}' must be a fan_in node"));
                    }
                }
            }
            (NodeKind::AgentTask | NodeKind::HumanGate, _) => {
                if decisions.is_empty() {
                    domain(format!("{} nodes declare at least one decision", node.kind.as_str()));
                }
            }
            (
                NodeKind::Tool,
                Some(NodeConfig::Tool {
                    exit_codes,
                    decision_field,
                    ..
                }),
            ) => match (exit_codes.is_empty(), decision_field.as_deref()) {
                (false, None) => {
                    let used = exit_codes.values().cloned().collect::<BTreeSet<_>>();
                    for (code, decision) in exit_codes {
                        if code.trim().parse::<i32>().is_err() {
                            domain(format!("exit code key '{code}' must be an integer"));
                        }
                        if !decisions.contains(decision) {
                            domain(format!(
                                "exit code {code} selects undeclared decision '{decision}'"
                            ));
                        }
                    }
                    for decision in decisions.difference(&used) {
                        domain(format!("decision '{decision}' is not selected by any exit code"));
                    }
                }
                (true, Some(field)) => {
                    if field.trim().is_empty() {
                        domain("decision_field must be nonempty".to_string());
                    }
                    if decisions.is_empty() {
                        domain("tool nodes declare at least one decision".to_string());
                    }
                }
                _ => domain(
                    "tool nodes declare exactly one adapter: config.exit_codes or config.decision_field"
                        .to_string(),
                ),
            },
            (
                NodeKind::Switch,
                Some(NodeConfig::Switch {
                    on,
                    cases,
                    otherwise,
                }),
            ) => {
                let reads = node
                    .contracts
                    .as_ref()
                    .map(|contracts| contracts.reads_context.as_slice())
                    .unwrap_or_default();
                if !reads.contains(on) {
                    domain(format!("switch key '{on}' must be listed in reads_context"));
                }
                if node
                    .contracts
                    .as_ref()
                    .is_some_and(|contracts| !contracts.writes_context.is_empty())
                {
                    domain("switch nodes write no context".to_string());
                }
                let mut seen = Vec::<SwitchScalar>::new();
                let mut used = BTreeSet::new();
                for (key, decision) in cases {
                    match decode_case_key(key) {
                        Ok(value) => {
                            if seen.iter().any(|other| other.same(&value)) {
                                domain(format!("case {key} duplicates another case value"));
                            }
                            seen.push(value);
                        }
                        Err(message) => domain(format!("case {key}: {message}")),
                    }
                    used.insert(decision.clone());
                }
                used.insert(otherwise.clone());
                for decision in &used {
                    if !decisions.contains(decision) {
                        domain(format!("switch selects undeclared decision '{decision}'"));
                    }
                }
                for decision in decisions.difference(&used) {
                    domain(format!("decision '{decision}' is not selected by any case or otherwise"));
                }
            }
            (NodeKind::Subflow, Some(NodeConfig::Subflow { results, .. })) => {
                if results.is_empty() {
                    domain("subflow nodes map every child result in config.results".to_string());
                }
                let used = results.values().cloned().collect::<BTreeSet<_>>();
                for decision in &used {
                    if !decisions.contains(decision) {
                        domain(format!("child result maps to undeclared decision '{decision}'"));
                    }
                }
                for decision in decisions.difference(&used) {
                    domain(format!("decision '{decision}' is not mapped from any child result"));
                }
            }
            _ => {}
        }
        for (rule_id, messages) in [
            ("decision_domain", domain_messages),
            ("fork_structure", structure_messages),
        ] {
            diagnostics.extend(
                messages
                    .into_iter()
                    .map(|message| node_diagnostic(rule_id, format!("{file}: {message}"), node_id)),
            );
        }
        diagnostics
    }

    fn exit_result_diagnostics(&self) -> Vec<FlowDiagnostic> {
        let mut outcomes = BTreeMap::<&str, (ExitOutcome, &str)>::new();
        let mut diagnostics = Vec::new();
        for (node_id, node) in &self.nodes {
            let Some(NodeConfig::Exit { result, .. }) = node.config.as_ref() else {
                continue;
            };
            match outcomes.get(result.name.as_str()) {
                Some((outcome, other)) if *outcome != result.outcome => {
                    diagnostics.push(node_diagnostic(
                        "exit_result",
                        format!(
                            "exits '{other}' and '{node_id}' share result '{}' with different outcomes",
                            result.name
                        ),
                        node_id,
                    ));
                }
                _ => {
                    outcomes.insert(&result.name, (result.outcome, node_id));
                }
            }
        }
        diagnostics
    }

    /// Successors for graph analysis: routes plus fork branch and join edges.
    fn successors(&self) -> BTreeMap<&str, Vec<&str>> {
        let mut outgoing = BTreeMap::<&str, Vec<&str>>::new();
        for (node_id, node) in &self.nodes {
            let targets = outgoing.entry(node_id.as_str()).or_default();
            targets.extend(node.routes.values().map(|route| route.to.as_str()));
            if let Some(NodeConfig::Parallel { branches, join, .. }) = node.config.as_ref() {
                targets.extend(branches.iter().map(String::as_str));
                targets.push(join.as_str());
            }
        }
        outgoing
    }

    fn reachability_diagnostics(&self, start_node: Option<&String>) -> Vec<FlowDiagnostic> {
        let Some(start_node) = start_node else {
            return Vec::new();
        };
        let outgoing = self.successors();
        let reachable = reach(&outgoing, [start_node.as_str()]);
        let mut diagnostics = self
            .nodes
            .keys()
            .filter(|node_id| !reachable.contains(node_id.as_str()))
            .map(|node_id| {
                node_diagnostic(
                    "reachability",
                    format!("node '{node_id}' is unreachable from start node"),
                    node_id,
                )
            })
            .collect::<Vec<_>>();
        let mut incoming = BTreeMap::<&str, Vec<&str>>::new();
        for (from, targets) in &outgoing {
            for to in targets {
                incoming.entry(to).or_default().push(from);
            }
        }
        let exits = self
            .nodes
            .iter()
            .filter(|(_, node)| node.kind == NodeKind::Exit)
            .map(|(node_id, _)| node_id.as_str());
        let reaches_exit = reach(&incoming, exits);
        for node_id in self.nodes.keys() {
            if outgoing.get(node_id.as_str()).is_none_or(Vec::is_empty)
                && self.nodes[node_id].kind != NodeKind::Exit
            {
                diagnostics.push(node_diagnostic(
                    "nonterminal_leaf",
                    format!("node '{node_id}' is not an exit and declares no routes"),
                    node_id,
                ));
            } else if !reaches_exit.contains(node_id.as_str()) {
                diagnostics.push(node_diagnostic(
                    "exit_path",
                    format!("node '{node_id}' has no path to an exit"),
                    node_id,
                ));
            }
        }
        diagnostics
    }

    /// Structured forks: disjoint branch interiors entered only from their
    /// fork, ending at the paired join or a failure exit, with no nested
    /// forks and one fork per join.
    fn fork_diagnostics(&self) -> Vec<FlowDiagnostic> {
        let mut diagnostics = Vec::new();
        let mut joins = BTreeMap::<&str, &str>::new();
        let mut owner = BTreeMap::<&str, &str>::new();
        for (fork_id, node) in &self.nodes {
            let Some(NodeConfig::Parallel { branches, join, .. }) = node.config.as_ref() else {
                continue;
            };
            let mut fork_error = |message: String| {
                diagnostics.push(node_diagnostic(
                    "fork_structure",
                    format!("fork '{fork_id}': {message}"),
                    fork_id,
                ));
            };
            if let Some(other) = joins.insert(join, fork_id) {
                fork_error(format!("join '{join}' is also paired with fork '{other}'"));
            }
            for branch in branches {
                let interior = self.branch_interior(branch, join);
                for node_id in &interior {
                    if *node_id == fork_id.as_str() {
                        fork_error(format!("branch '{branch}' loops back to its fork"));
                    }
                    if self.nodes[*node_id].kind == NodeKind::Parallel {
                        fork_error(format!(
                            "branch '{branch}' contains nested fork '{node_id}'"
                        ));
                    }
                    if let Some(other) = owner.insert(node_id, branch) {
                        if other != branch {
                            fork_error(format!(
                                "node '{node_id}' belongs to both branch '{other}' and branch '{branch}'"
                            ));
                        }
                    }
                    for route in self.nodes[*node_id].routes.values() {
                        if let Some(result) = self.exit_result(&route.to) {
                            if result.outcome == ExitOutcome::Success {
                                fork_error(format!(
                                    "branch '{branch}' reaches success exit '{}' from '{node_id}'",
                                    route.to
                                ));
                            }
                        }
                    }
                }
                // Interiors are entered only from their fork.
                for (source_id, source) in &self.nodes {
                    if interior.contains(source_id.as_str()) || source_id == fork_id {
                        continue;
                    }
                    for route in source.routes.values() {
                        if interior.contains(route.to.as_str()) {
                            fork_error(format!(
                                "'{source_id}' routes into branch '{branch}' at '{}', bypassing the fork",
                                route.to
                            ));
                        }
                    }
                }
                let mut incoming = BTreeMap::<&str, Vec<&str>>::new();
                for node_id in &interior {
                    for route in self.nodes[*node_id].routes.values() {
                        incoming.entry(route.to.as_str()).or_default().push(node_id);
                    }
                }
                let ends = std::iter::once(join.as_str()).chain(
                    self.nodes
                        .keys()
                        .map(String::as_str)
                        .filter(|id| self.exit_result(id).is_some()),
                );
                let finishes = reach(&incoming, ends);
                for node_id in &interior {
                    if !finishes.contains(node_id) {
                        fork_error(format!(
                            "branch node '{node_id}' has no path to join '{join}' or a failure exit"
                        ));
                    }
                }
            }
            // The join is entered only from its branches.
            for (source_id, source) in &self.nodes {
                if source.routes.values().any(|route| route.to == *join)
                    && !branches.iter().any(|branch| {
                        self.branch_interior(branch, join)
                            .contains(source_id.as_str())
                    })
                {
                    fork_error(format!(
                        "'{source_id}' routes to join '{join}' outside the fork's branches"
                    ));
                }
            }
        }
        for (join_id, node) in &self.nodes {
            if node.kind == NodeKind::FanIn && !joins.contains_key(join_id.as_str()) {
                diagnostics.push(node_diagnostic(
                    "fork_structure",
                    format!("fan_in '{join_id}' is not the join of any fork"),
                    join_id,
                ));
            }
        }
        diagnostics
    }

    /// Nodes a branch can execute: reachable from its entry by routes,
    /// stopping at the paired join and at exits.
    pub fn branch_interior(&self, entry: &str, join: &str) -> BTreeSet<&str> {
        let mut interior = BTreeSet::new();
        let mut queue = VecDeque::from([entry]);
        while let Some(node_id) = queue.pop_front() {
            let Some((node_id, node)) = self.nodes.get_key_value(node_id) else {
                continue;
            };
            if node_id == join || node.kind == NodeKind::Exit || !interior.insert(node_id.as_str())
            {
                continue;
            }
            queue.extend(node.routes.values().map(|route| route.to.as_str()));
        }
        interior
    }

    /// A switch's key needs a writer on some node that can run before it.
    fn switch_writer_diagnostics(&self) -> Vec<FlowDiagnostic> {
        let outgoing = self.successors();
        let mut incoming = BTreeMap::<&str, Vec<&str>>::new();
        for (from, targets) in &outgoing {
            for to in targets {
                incoming.entry(to).or_default().push(from);
            }
        }
        let mut diagnostics = Vec::new();
        for (node_id, node) in &self.nodes {
            let Some(NodeConfig::Switch { on, .. }) = node.config.as_ref() else {
                continue;
            };
            let predecessors = incoming.get(node_id.as_str()).cloned().unwrap_or_default();
            let ancestors = reach(&incoming, predecessors);
            let written = ancestors
                .iter()
                .filter(|ancestor| **ancestor != node_id.as_str())
                .any(|ancestor| self.node_writes(ancestor).contains(on.as_str()));
            if !written {
                diagnostics.push(node_diagnostic(
                    "switch_writer",
                    format!(
                        "{node_id}.md: no node before switch '{node_id}' writes '{on}' (writes_context or output_map)"
                    ),
                    node_id,
                ));
            }
        }
        diagnostics
    }

    /// Context keys a node declares it writes.
    fn node_writes(&self, node_id: &str) -> BTreeSet<&str> {
        let Some(node) = self.nodes.get(node_id) else {
            return BTreeSet::new();
        };
        let mut keys = node
            .contracts
            .iter()
            .flat_map(|contracts| contracts.writes_context.iter().map(String::as_str))
            .collect::<BTreeSet<_>>();
        match node.config.as_ref() {
            Some(NodeConfig::Tool { output_map, .. }) => {
                keys.extend(output_map.keys().map(String::as_str))
            }
            Some(NodeConfig::Subflow { output_map, .. }) => {
                keys.extend(output_map.values().map(String::as_str))
            }
            _ => {}
        }
        keys
    }

    /// Node ids of each named session, by session name.
    pub fn sessions(&self) -> BTreeMap<String, Vec<String>> {
        let mut sessions = BTreeMap::<String, Vec<String>>::new();
        for (node_id, node) in &self.nodes {
            if let Some(session) = node.runtime.as_ref().and_then(|r| r.session.as_deref()) {
                sessions
                    .entry(session.to_string())
                    .or_default()
                    .push(node_id.clone());
            }
        }
        sessions
    }

    /// Nodes sharing a session continue one provider session, so they may
    /// not name different providers or models.
    fn session_diagnostics(&self) -> Vec<FlowDiagnostic> {
        let mut diagnostics = Vec::new();
        for (session, node_ids) in self.sessions() {
            let executions = node_ids
                .iter()
                .filter_map(|node_id| self.nodes[node_id].execution.as_ref())
                .collect::<Vec<_>>();
            for (field, values) in [
                (
                    "llm_provider",
                    executions
                        .iter()
                        .filter_map(|e| e.llm_provider.as_deref())
                        .collect::<BTreeSet<_>>(),
                ),
                (
                    "llm_model",
                    executions
                        .iter()
                        .filter_map(|e| e.llm_model.as_deref())
                        .collect::<BTreeSet<_>>(),
                ),
            ] {
                if values.len() > 1 {
                    diagnostics.push(diagnostic(
                        "session_conflict",
                        format!(
                            "nodes in session '{session}' ({}) set conflicting {field} values: {}",
                            node_ids.join(", "),
                            values.into_iter().collect::<Vec<_>>().join(", ")
                        ),
                    ));
                }
            }
        }
        diagnostics
    }
}

fn reach<'a>(
    edges: &BTreeMap<&'a str, Vec<&'a str>>,
    roots: impl IntoIterator<Item = &'a str>,
) -> BTreeSet<&'a str> {
    let mut seen = BTreeSet::new();
    let mut queue = roots.into_iter().collect::<VecDeque<_>>();
    while let Some(node_id) = queue.pop_front() {
        if !seen.insert(node_id) {
            continue;
        }
        if let Some(targets) = edges.get(node_id) {
            queue.extend(targets.iter().copied());
        }
    }
    seen
}

fn config_kind(config: &NodeConfig) -> NodeKind {
    match config {
        NodeConfig::Start {} => NodeKind::Start,
        NodeConfig::Exit { .. } => NodeKind::Exit,
        NodeConfig::AgentTask { .. } => NodeKind::AgentTask,
        NodeConfig::HumanGate { .. } => NodeKind::HumanGate,
        NodeConfig::Switch { .. } => NodeKind::Switch,
        NodeConfig::Parallel { .. } => NodeKind::Parallel,
        NodeConfig::FanIn {} => NodeKind::FanIn,
        NodeConfig::Tool { .. } => NodeKind::Tool,
        NodeConfig::Subflow { .. } => NodeKind::Subflow,
    }
}

/// A decoded switch case key: a JSON boolean, finite number or string.
#[derive(Debug, Clone, PartialEq)]
pub enum SwitchScalar {
    Bool(bool),
    Number(f64),
    String(String),
}

impl SwitchScalar {
    /// Exact scalar equality: no coercion between types; numbers compare by
    /// value, so 0 and 0.0 are the same.
    pub fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Bool(left), Self::Bool(right)) => left == right,
            (Self::Number(left), Self::Number(right)) => left == right,
            (Self::String(left), Self::String(right)) => left == right,
            _ => false,
        }
    }

    /// The scalar a context value holds, or why it is unsupported.
    pub fn from_value(value: &Value) -> Result<Self, String> {
        match value {
            Value::Bool(value) => Ok(Self::Bool(*value)),
            Value::Number(number) => number
                .as_f64()
                .filter(|value| value.is_finite())
                .map(Self::Number)
                .ok_or_else(|| format!("number {number} is not finite")),
            Value::String(value) => Ok(Self::String(value.clone())),
            Value::Null => Err("null is not a supported switch value".to_string()),
            Value::Array(_) => Err("arrays are not supported switch values".to_string()),
            Value::Object(_) => Err("objects are not supported switch values".to_string()),
        }
    }
}

/// Decodes a switch case key, written as a JSON scalar.
pub fn decode_case_key(key: &str) -> Result<SwitchScalar, String> {
    let value = serde_json::from_str::<Value>(key)
        .map_err(|_| "case keys are JSON scalars, such as 0, true or '\"ready\"'".to_string())?;
    SwitchScalar::from_value(&value)
}

/// The decision a switch selects for `value`: the matching case, otherwise
/// `otherwise`. An unsupported value is an error, never `otherwise`.
pub fn switch_decision(
    cases: &BTreeMap<String, String>,
    otherwise: &str,
    value: &Value,
) -> Result<String, String> {
    let actual = SwitchScalar::from_value(value)?;
    for (key, decision) in cases {
        if decode_case_key(key)?.same(&actual) {
            return Ok(decision.clone());
        }
    }
    Ok(otherwise.to_string())
}

impl Default for NodeKind {
    fn default() -> Self {
        Self::AgentTask
    }
}

impl Default for FlowDefinition {
    fn default() -> Self {
        Self {
            schema_version: FLOW_SCHEMA_VERSION.to_string(),
            id: String::new(),
            title: String::new(),
            description: String::new(),
            goal: String::new(),
            inputs: Vec::new(),
            defaults: FlowDefaults::default(),
            nodes: BTreeMap::new(),
            metadata: BTreeMap::new(),
            extensions: BTreeMap::new(),
            execution_lock: None,
        }
    }
}

/// The config a node of `kind` has when it sets none. Kinds whose config
/// carries required settings have no default.
fn default_node_config(kind: &NodeKind) -> Option<NodeConfig> {
    Some(match kind {
        NodeKind::Start => NodeConfig::Start {},
        NodeKind::AgentTask => NodeConfig::AgentTask {
            prompt: String::new(),
        },
        NodeKind::HumanGate => NodeConfig::HumanGate {
            prompt: String::new(),
        },
        NodeKind::FanIn => NodeConfig::FanIn {},
        NodeKind::Exit
        | NodeKind::Switch
        | NodeKind::Parallel
        | NodeKind::Tool
        | NodeKind::Subflow => return None,
    })
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
    if let Some(NodeConfig::Tool {
        command,
        env_map,
        output_map,
        ..
    }) = node.config.as_ref()
    {
        insert_string_attr(&mut attrs, "tool.command", command);
        insert_tool_map_attr(&mut attrs, "tool.env_map", env_map);
        insert_tool_map_attr(&mut attrs, "tool.output_map", output_map);
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
        if let Some(value) = execution.thinking.as_deref() {
            insert_string_attr(&mut attrs, "thinking", value);
        }
        if let Some(value) = execution.thinking_budget_tokens {
            insert_string_attr(&mut attrs, "thinking_budget_tokens", &value.to_string());
        }
        if let Some(value) = execution.reasoning_mode.as_deref() {
            insert_string_attr(&mut attrs, "reasoning_mode", value);
        }
        if let Some(value) = execution.reasoning_summary.as_deref() {
            insert_string_attr(&mut attrs, "reasoning_summary", value);
        }
    }
    if let Some(NodeConfig::Subflow { flow_ref, .. }) = node.config.as_ref() {
        insert_string_attr(&mut attrs, "stack.child_flow_ref", flow_ref);
    }
    if let Some(NodeConfig::Parallel { max_parallel, .. }) = node.config.as_ref() {
        if let Some(value) = max_parallel {
            insert_i64_attr(&mut attrs, "max_parallel", *value as i64);
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

/// A derived edge as a DOT edge: `decision` and `description` attributes
/// for a route, `edge_kind` for every edge, and a display label.
pub fn dot_edge(edge: &FlowEdge) -> DotEdge {
    let mut attrs = BTreeMap::new();
    let label = if edge.label.is_empty() {
        edge.decision.clone().unwrap_or_default()
    } else {
        edge.label.clone()
    };
    insert_string_attr(&mut attrs, "label", &label);
    if let Some(decision) = edge.decision.as_deref() {
        insert_string_attr(&mut attrs, "decision", decision);
    }
    insert_string_attr(&mut attrs, "description", &edge.description);
    insert_string_attr(
        &mut attrs,
        "edge_kind",
        match edge.kind {
            FlowEdgeKind::Route => "route",
            FlowEdgeKind::Branch => "branch",
            FlowEdgeKind::Join => "join",
        },
    );
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
        NodeKind::Switch => "diamond",
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
        NodeKind::Switch => "switch",
        NodeKind::Parallel => "parallel",
        NodeKind::FanIn => "parallel.fan_in",
        NodeKind::Tool => "tool",
        NodeKind::Subflow => "stack.manager_loop",
    }
}

fn node_prompt(node: &FlowNode) -> Option<String> {
    match node.config.as_ref()? {
        NodeConfig::AgentTask { prompt } | NodeConfig::HumanGate { prompt } => {
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
        decision: None,
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
        decision: None,
    }
}

fn decision_diagnostic(
    rule_id: &str,
    message: impl Into<String>,
    node_id: &str,
    decision: &str,
) -> FlowDiagnostic {
    FlowDiagnostic {
        decision: Some(decision.to_string()),
        ..node_diagnostic(rule_id, message, node_id)
    }
}

/// Settings removed in favor of named sessions; a flow still setting one
/// fails validation instead of silently ignoring it.
fn is_removed_field(key: &str) -> bool {
    matches!(key, "fidelity" | "default_fidelity" | "thread_id")
}

/// Routing settings of the previous schema. They are rejected wherever they
/// appear, including extension bags, never dropped on save.
pub fn is_legacy_routing_field(key: &str) -> bool {
    matches!(
        key,
        "condition"
            | "weight"
            | "preferred_label"
            | "suggested_next_ids"
            | "retry_target"
            | "fallback_retry_target"
            | "goal_gate"
            | "allow_partial"
            | "auto_status"
            | "error_policy"
            | "join_policy"
            | "join_k"
            | "join_quorum"
            | "edges"
            | "outcome"
    )
}

fn valid_id(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

/// Decision IDs and terminal result names: `[a-z][a-z0-9_]*`.
pub fn valid_decision_id(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
}

/// A flow_ref with backslash separators turned into `/`, or None when it is not a
/// relative path that stays below its base directory. Validation, preview
/// and runtime all go through this.
pub fn normalize_flow_ref(value: &str) -> Option<String> {
    let value = value.trim().replace('\\', "/");
    let safe = !value.is_empty()
        && !value.starts_with('/')
        && !value.split('/').any(|part| part.is_empty() || part == "..");
    safe.then_some(value)
}

fn valid_flow_ref(value: &str) -> bool {
    normalize_flow_ref(value).is_some_and(|value| {
        // A flow_ref names a flow directory; YAML flow files are retired.
        !(value.ends_with(".yaml") || value.ends_with(".yml"))
    })
}

fn default_input_type() -> String {
    "string".to_string()
}

fn is_false(value: &bool) -> bool {
    !*value
}
