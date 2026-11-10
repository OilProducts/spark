use std::collections::{BTreeMap, BTreeSet};

use attractor_core::{
    parse_context_read_contract, parse_context_write_contract, DotAttribute, DotGraph, DotNode,
    DotValue,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const KNOWN_HANDLER_TYPES: &[&str] = &[
    "start",
    "exit",
    "codergen",
    "wait.human",
    "switch",
    "parallel",
    "parallel.fan_in",
    "tool",
    "stack.manager_loop",
];
const SUPPORTED_LAUNCH_INPUT_TYPES: &[&str] = &["string", "string[]", "boolean", "number", "json"];
const PROTECTED_EXECUTION_CONTEXT_PREFIX: &str = "_attractor.runtime.execution_";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

impl DiagnosticSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub rule_id: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
    #[serde(default)]
    pub line: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

impl Diagnostic {
    pub fn new(
        rule_id: impl Into<String>,
        severity: DiagnosticSeverity,
        message: impl Into<String>,
    ) -> Self {
        Self {
            rule_id: rule_id.into(),
            severity,
            message: message.into(),
            line: 0,
            node_id: None,
            edge: None,
            fix: None,
        }
    }

    fn with_line(mut self, line: usize) -> Self {
        self.line = line;
        self
    }

    fn with_node(mut self, node_id: impl Into<String>) -> Self {
        self.node_id = Some(node_id.into());
        self
    }

    fn with_edge(mut self, source: impl Into<String>, target: impl Into<String>) -> Self {
        self.edge = Some((source.into(), target.into()));
        self
    }

    fn with_fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }
}

pub trait LintRule {
    fn apply(&self, graph: &DotGraph) -> Vec<Diagnostic>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub errors: Vec<Diagnostic>,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let detail = self
            .errors
            .iter()
            .map(format_diagnostic_error)
            .collect::<Vec<_>>()
            .join("; ");
        write!(
            formatter,
            "validation failed with {} error(s): {detail}",
            self.errors.len()
        )
    }
}

impl std::error::Error for ValidationError {}

pub fn validate_graph(graph: &DotGraph) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    let start_nodes = find_start_nodes(graph);
    let exit_nodes = find_exit_nodes(graph);

    if start_nodes.len() != 1 {
        if start_nodes.is_empty() {
            diagnostics.push(Diagnostic::new(
                "start_node",
                DiagnosticSeverity::Error,
                "pipeline must have exactly one start node, found 0",
            ));
        } else {
            for node in &start_nodes {
                diagnostics.push(
                    Diagnostic::new(
                        "start_node",
                        DiagnosticSeverity::Error,
                        format!(
                            "pipeline must have exactly one start node, found {}",
                            start_nodes.len()
                        ),
                    )
                    .with_line(node.line)
                    .with_node(&node.node_id),
                );
            }
        }
    }

    if exit_nodes.is_empty() {
        diagnostics.push(Diagnostic::new(
            "terminal_node",
            DiagnosticSeverity::Error,
            "pipeline must have at least one exit node",
        ));
    }

    let mut in_degree: BTreeMap<String, usize> = graph
        .nodes
        .keys()
        .map(|node_id| (node_id.clone(), 0))
        .collect();
    let mut out_degree: BTreeMap<String, usize> = graph
        .nodes
        .keys()
        .map(|node_id| (node_id.clone(), 0))
        .collect();

    for edge in &graph.edges {
        if !graph.nodes.contains_key(&edge.source) {
            diagnostics.push(
                Diagnostic::new(
                    "edge_source_exists",
                    DiagnosticSeverity::Error,
                    format!(
                        "edge source '{}' does not reference an existing node",
                        edge.source
                    ),
                )
                .with_line(edge.line)
                .with_edge(&edge.source, &edge.target),
            );
        }
        if !graph.nodes.contains_key(&edge.target) {
            diagnostics.push(
                Diagnostic::new(
                    "edge_target_exists",
                    DiagnosticSeverity::Error,
                    format!(
                        "edge target '{}' does not reference an existing node",
                        edge.target
                    ),
                )
                .with_line(edge.line)
                .with_edge(&edge.source, &edge.target)
                .with_fix(format!(
                    "define node '{}' or update the edge target",
                    edge.target
                )),
            );
        }

        if let Some(count) = out_degree.get_mut(&edge.source) {
            *count += 1;
        }
        if let Some(count) = in_degree.get_mut(&edge.target) {
            *count += 1;
        }
    }

    for start in &start_nodes {
        if in_degree.get(&start.node_id).copied().unwrap_or_default() != 0 {
            diagnostics.push(
                Diagnostic::new(
                    "start_no_incoming",
                    DiagnosticSeverity::Error,
                    format!("start node '{}' must have no incoming edges", start.node_id),
                )
                .with_line(start.line)
                .with_node(&start.node_id),
            );
        }
    }

    for exit_node in &exit_nodes {
        if out_degree
            .get(&exit_node.node_id)
            .copied()
            .unwrap_or_default()
            != 0
        {
            diagnostics.push(
                Diagnostic::new(
                    "exit_no_outgoing",
                    DiagnosticSeverity::Error,
                    format!(
                        "exit node '{}' must have no outgoing edges",
                        exit_node.node_id
                    ),
                )
                .with_line(exit_node.line)
                .with_node(&exit_node.node_id),
            );
        }
    }

    let exit_node_ids = exit_nodes
        .iter()
        .map(|node| node.node_id.as_str())
        .collect::<BTreeSet<_>>();
    for node in nodes_in_declaration_order(graph) {
        if exit_node_ids.contains(node.node_id.as_str()) {
            continue;
        }
        if out_degree.get(&node.node_id).copied().unwrap_or_default() == 0 {
            diagnostics.push(
                Diagnostic::new(
                    "node_has_outgoing_edge",
                    DiagnosticSeverity::Error,
                    format!(
                        "node '{}' must declare at least one outgoing edge",
                        node.node_id
                    ),
                )
                .with_line(node.line)
                .with_node(&node.node_id),
            );
        }
    }

    if start_nodes.len() == 1 {
        let reachable = reachable_nodes(graph, &start_nodes[0].node_id);
        for node in nodes_in_declaration_order(graph) {
            if !reachable.contains(&node.node_id) {
                diagnostics.push(
                    Diagnostic::new(
                        "reachability",
                        DiagnosticSeverity::Error,
                        format!("node '{}' is not reachable from start node", node.node_id),
                    )
                    .with_line(node.line)
                    .with_node(&node.node_id),
                );
            }
        }
    }

    diagnostics.extend(validate_known_types(graph));
    diagnostics.extend(validate_prompt_on_llm_nodes(graph));
    let resolved = crate::transforms::apply_graph_transforms(graph);
    // Validate every declaration even when overridden, then check resolved combinations.
    for (id, attrs, resolved_controls) in std::iter::once((None, &graph.graph_attrs, false))
        .chain(
            graph
                .nodes
                .iter()
                .map(|(id, node)| (Some(id), &node.attrs, false)),
        )
        .chain(
            resolved
                .nodes
                .iter()
                .map(|(id, node)| (Some(id), &node.attrs, true)),
        )
    {
        let text = |key| attrs.get(key).map(attr_text);
        let thinking = text("thinking");
        let budget = text("thinking_budget_tokens");
        let mode = text("reasoning_mode");
        let summary = text("reasoning_summary");
        let result = if budget.as_ref().is_some_and(|v| v.parse::<u64>().is_err()) {
            Err("thinking_budget_tokens must be an integer of at least 1024".into())
        } else {
            let validate = if resolved_controls {
                spark_common::settings::validate_reasoning_controls
            } else {
                spark_common::settings::validate_reasoning_control_fields
            };
            validate(
                thinking.as_deref(),
                budget.and_then(|v| v.parse().ok()),
                mode.as_deref(),
                summary.as_deref(),
            )
        };
        if let Err(message) = result {
            let mut diagnostic =
                Diagnostic::new("reasoning_controls", DiagnosticSeverity::Error, message);
            if let Some(id) = id {
                diagnostic = diagnostic.with_node(id);
            }
            diagnostics.push(diagnostic);
        }
    }
    diagnostics.extend(validate_tool_handler_attrs(graph));
    diagnostics.extend(validate_execution_context_write_authority(graph));

    diagnostics
}

pub fn validate(graph: &DotGraph) -> Vec<Diagnostic> {
    validate_graph(graph)
}

pub fn validate_with_extra<'a>(
    graph: &DotGraph,
    extra_rules: impl IntoIterator<Item = &'a dyn LintRule>,
) -> Vec<Diagnostic> {
    let mut diagnostics = validate_graph(graph);
    for rule in extra_rules {
        diagnostics.extend(rule.apply(graph));
    }
    diagnostics
}

pub fn validate_or_raise(graph: &DotGraph) -> Result<Vec<Diagnostic>, ValidationError> {
    let diagnostics = validate_graph(graph);
    let errors = error_diagnostics(&diagnostics);
    if errors.is_empty() {
        Ok(diagnostics)
    } else {
        Err(ValidationError { errors })
    }
}

pub fn validate_or_raise_with_extra<'a>(
    graph: &DotGraph,
    extra_rules: impl IntoIterator<Item = &'a dyn LintRule>,
) -> Result<Vec<Diagnostic>, ValidationError> {
    let diagnostics = validate_with_extra(graph, extra_rules);
    let errors = error_diagnostics(&diagnostics);
    if errors.is_empty() {
        Ok(diagnostics)
    } else {
        Err(ValidationError { errors })
    }
}

pub fn diagnostic_payload(diagnostic: &Diagnostic) -> Value {
    let mut payload = serde_json::Map::from_iter([
        ("rule".to_string(), json!(diagnostic.rule_id)),
        ("rule_id".to_string(), json!(diagnostic.rule_id)),
        ("severity".to_string(), json!(diagnostic.severity.as_str())),
        ("message".to_string(), json!(diagnostic.message)),
        ("line".to_string(), json!(diagnostic.line)),
        (
            "node".to_string(),
            diagnostic
                .node_id
                .as_ref()
                .map(|node_id| json!(node_id))
                .unwrap_or(Value::Null),
        ),
        (
            "node_id".to_string(),
            diagnostic
                .node_id
                .as_ref()
                .map(|node_id| json!(node_id))
                .unwrap_or(Value::Null),
        ),
    ]);
    if let Some((source, target)) = &diagnostic.edge {
        payload.insert("edge".to_string(), json!([source, target]));
    }
    if let Some(fix) = &diagnostic.fix {
        payload.insert("fix".to_string(), json!(fix));
    }
    Value::Object(payload)
}

pub fn diagnostics_payload(diagnostics: &[Diagnostic]) -> Vec<Value> {
    diagnostics.iter().map(diagnostic_payload).collect()
}

pub fn validate_launch_contract_declarations(graph: &DotGraph) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    if let Some(attr) = graph.graph_attrs.get("spark.launch_inputs") {
        diagnostics.extend(validate_launch_inputs_attr(attr));
    }

    for node in nodes_in_declaration_order(graph) {
        if let Some(attr) = node.attrs.get("spark.reads_context") {
            diagnostics.extend(validate_context_attr(
                attr,
                node,
                "spark.reads_context",
                "reads_context_valid",
                true,
            ));
        }
        if let Some(attr) = node.attrs.get("spark.writes_context") {
            diagnostics.extend(validate_context_attr(
                attr,
                node,
                "spark.writes_context",
                "writes_context_valid",
                false,
            ));
        }
    }

    diagnostics
}

fn error_diagnostics(diagnostics: &[Diagnostic]) -> Vec<Diagnostic> {
    diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        .cloned()
        .collect()
}

fn format_diagnostic_error(diagnostic: &Diagnostic) -> String {
    if diagnostic.line > 0 {
        format!(
            "[{}] line {}: {}",
            diagnostic.rule_id, diagnostic.line, diagnostic.message
        )
    } else {
        format!("[{}] {}", diagnostic.rule_id, diagnostic.message)
    }
}

fn nodes_in_declaration_order(graph: &DotGraph) -> Vec<&DotNode> {
    let mut nodes = graph.nodes.values().collect::<Vec<_>>();
    nodes.sort_by(|left, right| {
        left.declaration_order
            .cmp(&right.declaration_order)
            .then_with(|| left.node_id.cmp(&right.node_id))
    });
    nodes
}

fn find_start_nodes(graph: &DotGraph) -> Vec<&DotNode> {
    nodes_in_declaration_order(graph)
        .into_iter()
        .filter(|node| {
            attr_str(&node.attrs, "shape") == "Mdiamond"
                || matches!(node.node_id.as_str(), "start" | "Start")
        })
        .collect()
}

fn find_exit_nodes(graph: &DotGraph) -> Vec<&DotNode> {
    let mut shape_nodes = Vec::new();
    let mut fallback_nodes = Vec::new();
    for node in nodes_in_declaration_order(graph) {
        let shape = attr_str(&node.attrs, "shape");
        if shape == "Msquare" {
            shape_nodes.push(node);
        } else if matches!(node.node_id.as_str(), "exit" | "end" | "Exit" | "End") {
            fallback_nodes.push(node);
        }
    }
    if shape_nodes.is_empty() {
        fallback_nodes
    } else {
        shape_nodes
    }
}

fn reachable_nodes(graph: &DotGraph, start_id: &str) -> BTreeSet<String> {
    let mut adjacency: BTreeMap<String, Vec<String>> = graph
        .nodes
        .keys()
        .map(|node_id| (node_id.clone(), Vec::new()))
        .collect();
    for edge in &graph.edges {
        if let Some(targets) = adjacency.get_mut(&edge.source) {
            targets.push(edge.target.clone());
        }
    }

    let mut visited = BTreeSet::new();
    let mut stack = vec![start_id.to_string()];
    while let Some(current) = stack.pop() {
        if !visited.insert(current.clone()) {
            continue;
        }
        if let Some(targets) = adjacency.get(&current) {
            for target in targets {
                if !visited.contains(target) {
                    stack.push(target.clone());
                }
            }
        }
    }
    visited
}

fn validate_known_types(graph: &DotGraph) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for node in nodes_in_declaration_order(graph) {
        let Some(attr) = node.attrs.get("type") else {
            continue;
        };
        let handler_type = attr_text(attr).trim().to_string();
        if !handler_type.is_empty() && !KNOWN_HANDLER_TYPES.contains(&handler_type.as_str()) {
            diagnostics.push(
                Diagnostic::new(
                    "type_known",
                    DiagnosticSeverity::Warning,
                    format!(
                        "node '{}' type '{}' is not a recognized handler type",
                        node.node_id, handler_type
                    ),
                )
                .with_line(attr.line)
                .with_node(&node.node_id),
            );
        }
    }
    diagnostics
}

fn validate_prompt_on_llm_nodes(graph: &DotGraph) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for node in nodes_in_declaration_order(graph) {
        if !resolves_to_codergen(node) {
            continue;
        }
        if has_authored_non_empty_attr(node, "prompt") || has_authored_non_empty_attr(node, "label")
        {
            continue;
        }
        diagnostics.push(
            Diagnostic::new(
                "prompt_on_llm_nodes",
                DiagnosticSeverity::Warning,
                format!(
                    "node '{}' resolves to codergen and should define a non-empty prompt or label",
                    node.node_id
                ),
            )
            .with_line(node.line)
            .with_node(&node.node_id),
        );
    }
    diagnostics
}

fn validate_tool_handler_attrs(graph: &DotGraph) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (key, message) in legacy_tool_attr_messages() {
        if let Some(attr) = graph.graph_attrs.get(key) {
            diagnostics.push(
                Diagnostic::new("tool_attr_namespaced", DiagnosticSeverity::Error, message)
                    .with_line(attr.line),
            );
        }
    }

    for node in nodes_in_declaration_order(graph) {
        for (key, message) in legacy_tool_attr_messages() {
            if let Some(attr) = node.attrs.get(key) {
                diagnostics.push(
                    Diagnostic::new("tool_attr_namespaced", DiagnosticSeverity::Error, message)
                        .with_line(attr.line)
                        .with_node(&node.node_id),
                );
            }
        }
        if resolves_to_handler_type(node) != "tool" {
            continue;
        }
        if has_non_empty_attr(node, "tool.command") {
            continue;
        }
        diagnostics.push(
            Diagnostic::new(
                "tool_command_required",
                DiagnosticSeverity::Error,
                format!(
                    "node '{}' resolves to tool and must define a non-empty tool.command",
                    node.node_id
                ),
            )
            .with_line(node.line)
            .with_node(&node.node_id),
        );
    }
    diagnostics
}

fn validate_execution_context_write_authority(graph: &DotGraph) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for node in nodes_in_declaration_order(graph) {
        let Some(attr) = node.attrs.get("spark.writes_context") else {
            continue;
        };
        let DotValue::String(raw) = &attr.value else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<Value>(raw) else {
            continue;
        };
        let Some(items) = parsed.as_array() else {
            continue;
        };
        let protected_keys = items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|key| key.starts_with(PROTECTED_EXECUTION_CONTEXT_PREFIX))
            .map(ToOwned::to_owned)
            .collect::<BTreeSet<_>>();
        if protected_keys.is_empty() {
            continue;
        }
        diagnostics.push(
            Diagnostic::new(
                "execution_placement_context_non_authoritative",
                DiagnosticSeverity::Error,
                format!(
                    "node '{}' cannot declare execution placement runtime keys in spark.writes_context: {}",
                    node.node_id,
                    protected_keys.into_iter().collect::<Vec<_>>().join(", ")
                ),
            )
            .with_line(attr.line)
            .with_node(&node.node_id),
        );
    }
    diagnostics
}

fn validate_launch_inputs_attr(attr: &DotAttribute) -> Vec<Diagnostic> {
    let DotValue::String(raw) = &attr.value else {
        return vec![Diagnostic::new(
            "launch_inputs_valid",
            DiagnosticSeverity::Error,
            "spark.launch_inputs must be a JSON array string",
        )
        .with_line(attr.line)];
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Vec::new();
    }
    let parsed = match serde_json::from_str::<Value>(raw) {
        Ok(value) => value,
        Err(_) => {
            return vec![Diagnostic::new(
                "launch_inputs_valid",
                DiagnosticSeverity::Error,
                "Launch inputs must be valid JSON.",
            )
            .with_line(attr.line)];
        }
    };
    let Some(items) = parsed.as_array() else {
        return vec![Diagnostic::new(
            "launch_inputs_valid",
            DiagnosticSeverity::Error,
            "Launch inputs must be a JSON array.",
        )
        .with_line(attr.line)];
    };

    let mut diagnostics = Vec::new();
    let mut seen_keys = BTreeSet::new();
    for item in items {
        let Some(record) = item.as_object() else {
            diagnostics.push(
                Diagnostic::new(
                    "launch_inputs_valid",
                    DiagnosticSeverity::Error,
                    "Each launch input must be an object.",
                )
                .with_line(attr.line),
            );
            continue;
        };
        let key = record
            .get("key")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if key.is_empty() {
            diagnostics.push(
                Diagnostic::new(
                    "launch_inputs_valid",
                    DiagnosticSeverity::Error,
                    "Context keys are required.",
                )
                .with_line(attr.line),
            );
        } else if !key.starts_with("context.") {
            diagnostics.push(
                Diagnostic::new(
                    "launch_inputs_valid",
                    DiagnosticSeverity::Error,
                    format!("Context keys must use the context.* namespace: {key}"),
                )
                .with_line(attr.line),
            );
        } else if !seen_keys.insert(key.to_string()) {
            diagnostics.push(
                Diagnostic::new(
                    "launch_inputs_valid",
                    DiagnosticSeverity::Error,
                    format!("Launch input keys must be unique: {key}"),
                )
                .with_line(attr.line),
            );
        }
        let input_type = record.get("type").and_then(Value::as_str).unwrap_or("");
        if !SUPPORTED_LAUNCH_INPUT_TYPES.contains(&input_type) {
            diagnostics.push(
                Diagnostic::new(
                    "launch_inputs_valid",
                    DiagnosticSeverity::Error,
                    format!(
                        "Unsupported launch input type: {}",
                        record.get("type").unwrap_or(&Value::Null)
                    ),
                )
                .with_line(attr.line),
            );
        }
        let label = record
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if label.is_empty() && !key.is_empty() {
            diagnostics.push(
                Diagnostic::new(
                    "launch_inputs_valid",
                    DiagnosticSeverity::Error,
                    format!("Launch input labels are required for {key}"),
                )
                .with_line(attr.line),
            );
        }
    }
    diagnostics
}

fn validate_context_attr(
    attr: &DotAttribute,
    node: &DotNode,
    attr_name: &str,
    rule_id: &str,
    read_contract: bool,
) -> Vec<Diagnostic> {
    let DotValue::String(raw) = &attr.value else {
        return vec![Diagnostic::new(
            rule_id,
            DiagnosticSeverity::Error,
            format!("{attr_name} must be a JSON array string"),
        )
        .with_line(attr.line)
        .with_node(&node.node_id)];
    };
    let parse_error = if read_contract {
        parse_context_read_contract(Some(raw)).parse_error
    } else {
        parse_context_write_contract(Some(raw)).parse_error
    };
    if parse_error.is_empty() {
        Vec::new()
    } else {
        vec![Diagnostic::new(
            rule_id,
            DiagnosticSeverity::Error,
            format!(
                "node '{}' {attr_name} parse error: {parse_error}",
                node.node_id
            ),
        )
        .with_line(attr.line)
        .with_node(&node.node_id)]
    }
}

fn legacy_tool_attr_messages() -> [(&'static str, &'static str); 3] {
    [
        (
            "tool_command",
            "legacy tool attr 'tool_command' is not supported; use 'tool.command'",
        ),
        (
            "tool_hooks.pre",
            "legacy tool attr 'tool_hooks.pre' is not supported; use 'tool.hooks.pre'",
        ),
        (
            "tool_hooks.post",
            "legacy tool attr 'tool_hooks.post' is not supported; use 'tool.hooks.post'",
        ),
    ]
}

fn attr_str(attrs: &BTreeMap<String, DotAttribute>, key: &str) -> String {
    attrs.get(key).map(attr_text).unwrap_or_default()
}

fn attr_text(attr: &DotAttribute) -> String {
    attr.value.to_string()
}

fn has_authored_non_empty_attr(node: &DotNode, key: &str) -> bool {
    node.explicit_attr_keys.contains(key) && has_non_empty_attr(node, key)
}

fn has_non_empty_attr(node: &DotNode, key: &str) -> bool {
    node.attrs
        .get(key)
        .map(|attr| !attr_text(attr).trim().is_empty())
        .unwrap_or(false)
}

fn resolves_to_codergen(node: &DotNode) -> bool {
    if let Some(attr) = node.attrs.get("type") {
        let explicit = attr_text(attr).trim().to_string();
        if !explicit.is_empty() && KNOWN_HANDLER_TYPES.contains(&explicit.as_str()) {
            return explicit == "codergen";
        }
    }
    if let Some(attr) = node.attrs.get("shape") {
        if let Some(mapped) = shape_handler_type(attr_text(attr).trim()) {
            return mapped == "codergen";
        }
    }
    true
}

fn resolves_to_handler_type(node: &DotNode) -> &'static str {
    if let Some(attr) = node.attrs.get("type") {
        let explicit = attr_text(attr).trim().to_string();
        if let Some(handler_type) = KNOWN_HANDLER_TYPES
            .iter()
            .copied()
            .find(|handler_type| *handler_type == explicit)
        {
            return handler_type;
        }
    }
    if let Some(attr) = node.attrs.get("shape") {
        if let Some(mapped) = shape_handler_type(attr_text(attr).trim()) {
            return mapped;
        }
    }
    "codergen"
}

fn shape_handler_type(shape: &str) -> Option<&'static str> {
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
