use std::collections::{BTreeMap, BTreeSet};

use attractor_core::{
    parse_context_read_contract, parse_context_write_contract, DotAttribute, DotEdge, DotGraph,
    DotNode, DotValue,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const VALID_FIDELITY: &[&str] = &[
    "full",
    "truncate",
    "compact",
    "summary:low",
    "summary:medium",
    "summary:high",
];
const KNOWN_HANDLER_TYPES: &[&str] = &[
    "start",
    "exit",
    "codergen",
    "wait.human",
    "conditional",
    "parallel",
    "parallel.fan_in",
    "tool",
    "stack.manager_loop",
];
const SUPPORTED_PARALLEL_JOIN_POLICIES: &[&str] =
    &["wait_all", "first_success", "k_of_n", "quorum"];
const SUPPORTED_LAUNCH_INPUT_TYPES: &[&str] = &["string", "string[]", "boolean", "number", "json"];
const ALLOWED_STYLESHEET_PROPERTIES: &[&str] = &[
    "llm_model",
    "llm_provider",
    "llm_profile",
    "reasoning_effort",
];
const ALLOWED_REASONING_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh"];
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

    if exit_nodes.len() != 1 {
        if exit_nodes.is_empty() {
            diagnostics.push(Diagnostic::new(
                "terminal_node",
                DiagnosticSeverity::Error,
                "pipeline must have exactly one exit node, found 0",
            ));
        } else {
            for node in &exit_nodes {
                diagnostics.push(
                    Diagnostic::new(
                        "terminal_node",
                        DiagnosticSeverity::Error,
                        format!(
                            "pipeline must have exactly one exit node, found {}",
                            exit_nodes.len()
                        ),
                    )
                    .with_line(node.line)
                    .with_node(&node.node_id),
                );
            }
        }
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

        diagnostics.extend(validate_edge_condition(edge.attrs.get("condition"), edge));
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

    diagnostics.extend(validate_retry_targets(graph));
    diagnostics.extend(validate_goal_gate_retry_targets(graph));
    diagnostics.extend(validate_fidelity_values(graph));
    diagnostics.extend(validate_known_types(graph));
    diagnostics.extend(validate_prompt_on_llm_nodes(graph));
    diagnostics.extend(validate_stylesheet(graph));
    diagnostics.extend(validate_tool_handler_attrs(graph));
    diagnostics.extend(validate_parallel_join_attrs(graph, &out_degree));
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

pub fn preview_payload_for_graph(graph: &DotGraph) -> Value {
    let diagnostics = validate_graph(graph);
    let errors = error_diagnostics(&diagnostics);
    json!({
        "status": if errors.is_empty() { "ok" } else { "validation_error" },
        "diagnostics": diagnostics_payload(&diagnostics),
        "errors": diagnostics_payload(&errors),
    })
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

fn validate_edge_condition(
    condition_attr: Option<&DotAttribute>,
    edge: &DotEdge,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let Some(condition_attr) = condition_attr else {
        return diagnostics;
    };
    let DotValue::String(condition) = &condition_attr.value else {
        return vec![Diagnostic::new(
            "condition_syntax",
            DiagnosticSeverity::Error,
            "edge condition must be a string",
        )
        .with_line(edge.line)
        .with_edge(&edge.source, &edge.target)];
    };
    let stripped = condition.trim();
    if stripped.is_empty() {
        return diagnostics;
    }

    for clause in split_condition_clauses(stripped) {
        let clause = clause.trim().to_string();
        if clause.is_empty() {
            diagnostics.push(
                Diagnostic::new(
                    "condition_syntax",
                    DiagnosticSeverity::Error,
                    "empty condition clause is not allowed",
                )
                .with_line(edge.line)
                .with_edge(&edge.source, &edge.target),
            );
            continue;
        }

        if let Some(operator) = find_unsupported_condition_operator(&clause) {
            diagnostics.push(
                Diagnostic::new(
                    "condition_syntax",
                    DiagnosticSeverity::Error,
                    format!("unsupported operator '{operator}' in condition clause '{clause}'"),
                )
                .with_line(edge.line)
                .with_edge(&edge.source, &edge.target),
            );
            continue;
        }

        let Some(key) = condition_clause_key(&clause) else {
            diagnostics.push(
                Diagnostic::new(
                    "condition_syntax",
                    DiagnosticSeverity::Error,
                    format!("invalid condition clause '{clause}'"),
                )
                .with_line(edge.line)
                .with_edge(&edge.source, &edge.target),
            );
            continue;
        };

        if key == "outcome" || key == "preferred_label" {
            continue;
        }
        if let Some(context_path) = key.strip_prefix("context.") {
            if is_context_condition_path(context_path) {
                continue;
            }
        }
        diagnostics.push(
            Diagnostic::new(
                "condition_syntax",
                DiagnosticSeverity::Error,
                format!("invalid condition variable '{key}'"),
            )
            .with_line(edge.line)
            .with_edge(&edge.source, &edge.target),
        );
    }

    diagnostics
}

fn split_condition_clauses(condition: &str) -> Vec<String> {
    let mut clauses = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut escaped = false;
    let mut chars = condition.chars().peekable();

    while let Some(ch) = chars.next() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            current.push(ch);
            escaped = true;
            continue;
        }
        if ch == '"' {
            in_quotes = !in_quotes;
            current.push(ch);
            continue;
        }
        if !in_quotes && ch == '&' && chars.peek() == Some(&'&') {
            chars.next();
            clauses.push(current);
            current = String::new();
            continue;
        }
        current.push(ch);
    }
    clauses.push(current);
    clauses
}

fn find_unsupported_condition_operator(clause: &str) -> Option<String> {
    let unquoted = strip_quoted_segments(clause);
    if unquoted.contains("||") {
        return Some("||".to_string());
    }

    for word in unquoted
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .filter(|word| !word.is_empty())
    {
        match word.to_ascii_lowercase().as_str() {
            "contains" => return Some("contains".to_string()),
            "matches" => return Some("matches".to_string()),
            "or" => return Some("OR".to_string()),
            "not" => return Some("NOT".to_string()),
            _ => {}
        }
    }

    for symbol in [">=", "<=", ">", "<"] {
        if unquoted.contains(symbol) {
            return Some(symbol.to_string());
        }
    }
    None
}

fn strip_quoted_segments(text: &str) -> String {
    let mut result = String::new();
    let mut in_quotes = false;
    let mut escaped = false;
    for ch in text.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            in_quotes = !in_quotes;
            continue;
        }
        if !in_quotes {
            result.push(ch);
        }
    }
    result
}

fn condition_clause_key(clause: &str) -> Option<String> {
    let mut operator_index = None;
    let mut operator_len = 0;
    let mut in_quotes = false;
    let mut escaped = false;
    for (index, ch) in clause.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            in_quotes = !in_quotes;
            continue;
        }
        if !in_quotes && ch == '!' && clause[index..].starts_with("!=") {
            operator_index = Some(index);
            operator_len = 2;
            break;
        }
        if !in_quotes && ch == '=' {
            operator_index = Some(index);
            operator_len = 1;
            break;
        }
    }

    let key = if let Some(index) = operator_index {
        let right_start = index + operator_len;
        if clause[right_start..].trim().is_empty() {
            return None;
        }
        clause[..index].trim()
    } else {
        clause.trim()
    };

    is_condition_key(key).then(|| key.to_string())
}

fn is_condition_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.')
}

fn is_context_condition_path(path: &str) -> bool {
    !path.is_empty()
        && path.split('.').all(|segment| {
            let mut chars = segment.chars();
            let Some(first) = chars.next() else {
                return false;
            };
            (first.is_ascii_alphabetic() || first == '_')
                && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        })
}

fn validate_retry_targets(graph: &DotGraph) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for key in ["retry_target", "fallback_retry_target"] {
        let Some(attr) = graph.graph_attrs.get(key) else {
            continue;
        };
        let target = attr_text(attr).trim().to_string();
        if !target.is_empty() && !graph.nodes.contains_key(&target) {
            diagnostics.push(
                Diagnostic::new(
                    "retry_target_exists",
                    DiagnosticSeverity::Warning,
                    format!("graph attribute {key} references missing node '{target}'"),
                )
                .with_line(attr.line),
            );
        }
    }

    for node in nodes_in_declaration_order(graph) {
        for key in ["retry_target", "fallback_retry_target"] {
            let Some(attr) = node.attrs.get(key) else {
                continue;
            };
            let target = attr_text(attr).trim().to_string();
            if !target.is_empty() && !graph.nodes.contains_key(&target) {
                diagnostics.push(
                    Diagnostic::new(
                        "retry_target_exists",
                        DiagnosticSeverity::Warning,
                        format!(
                            "node '{}' {key} references missing node '{target}'",
                            node.node_id
                        ),
                    )
                    .with_line(attr.line)
                    .with_node(&node.node_id),
                );
            }
        }
    }
    diagnostics
}

fn validate_goal_gate_retry_targets(graph: &DotGraph) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for node in nodes_in_declaration_order(graph) {
        let Some(goal_gate_attr) = node.attrs.get("goal_gate") else {
            continue;
        };
        if !attr_is_true(Some(goal_gate_attr)) {
            continue;
        }
        let has_retry_target = ["retry_target", "fallback_retry_target"].iter().any(|key| {
            node.attrs
                .get(*key)
                .map(|attr| !attr_text(attr).trim().is_empty())
                .unwrap_or(false)
        });
        if !has_retry_target {
            diagnostics.push(
                Diagnostic::new(
                    "goal_gate_has_retry",
                    DiagnosticSeverity::Warning,
                    format!(
                        "node '{}' has goal_gate=true but does not define retry_target or fallback_retry_target",
                        node.node_id
                    ),
                )
                .with_line(goal_gate_attr.line)
                .with_node(&node.node_id),
            );
        }
    }
    diagnostics
}

fn validate_fidelity_values(graph: &DotGraph) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if let Some(attr) = graph.graph_attrs.get("default_fidelity") {
        check_fidelity_attr(&mut diagnostics, "graph", attr, attr.line, None, None);
    }
    for node in nodes_in_declaration_order(graph) {
        if let Some(attr) = node.attrs.get("fidelity") {
            check_fidelity_attr(
                &mut diagnostics,
                &format!("node '{}'", node.node_id),
                attr,
                attr.line,
                Some(&node.node_id),
                None,
            );
        }
    }
    for edge in &graph.edges {
        if let Some(attr) = edge.attrs.get("fidelity") {
            check_fidelity_attr(
                &mut diagnostics,
                &format!("edge {}->{}", edge.source, edge.target),
                attr,
                attr.line,
                None,
                Some((&edge.source, &edge.target)),
            );
        }
    }
    diagnostics
}

fn check_fidelity_attr(
    diagnostics: &mut Vec<Diagnostic>,
    owner: &str,
    attr: &DotAttribute,
    line: usize,
    node_id: Option<&str>,
    edge: Option<(&str, &str)>,
) {
    let value = attr_text(attr);
    if value.is_empty() || VALID_FIDELITY.contains(&value.as_str()) {
        return;
    }
    let mut diagnostic = Diagnostic::new(
        "fidelity_valid",
        DiagnosticSeverity::Warning,
        format!("{owner} fidelity '{value}' is not a recognized mode"),
    )
    .with_line(line);
    if let Some(node_id) = node_id {
        diagnostic = diagnostic.with_node(node_id);
    }
    if let Some((source, target)) = edge {
        diagnostic = diagnostic.with_edge(source, target);
    }
    diagnostics.push(diagnostic);
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

fn validate_stylesheet(graph: &DotGraph) -> Vec<Diagnostic> {
    let Some(attr) = graph.graph_attrs.get("model_stylesheet") else {
        return Vec::new();
    };
    let DotValue::String(stylesheet) = &attr.value else {
        return vec![Diagnostic::new(
            "stylesheet_syntax",
            DiagnosticSeverity::Error,
            "model_stylesheet must be a string",
        )
        .with_line(attr.line)];
    };
    lint_stylesheet_syntax(stylesheet, attr.line)
}

fn lint_stylesheet_syntax(stylesheet: &str, line: usize) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let text = stylesheet.trim();
    if text.is_empty() {
        diagnostics.push(
            Diagnostic::new(
                "stylesheet_syntax",
                DiagnosticSeverity::Error,
                "model_stylesheet must include at least one rule",
            )
            .with_line(line),
        );
        return diagnostics;
    }

    let mut index = 0;
    while index < text.len() {
        index = skip_ascii_whitespace(text, index);
        if index >= text.len() {
            break;
        }
        let Some(brace) = find_unquoted(text, '{', index) else {
            diagnostics.push(
                Diagnostic::new(
                    "stylesheet_syntax",
                    DiagnosticSeverity::Error,
                    "stylesheet selector must be followed by '{'",
                )
                .with_line(line),
            );
            break;
        };

        let selector = text[index..brace].trim();
        if selector.is_empty() {
            diagnostics.push(
                Diagnostic::new(
                    "stylesheet_syntax",
                    DiagnosticSeverity::Error,
                    "stylesheet selector cannot be empty",
                )
                .with_line(line),
            );
        } else if !stylesheet_selector_valid(selector) {
            diagnostics.push(
                Diagnostic::new(
                    "stylesheet_syntax",
                    DiagnosticSeverity::Error,
                    format!(
                        "invalid stylesheet selector '{selector}', must be '*', 'shape', '.class', or '#node_id'"
                    ),
                )
                .with_line(line),
            );
        }

        let Some(close) = find_unquoted(text, '}', brace + 1) else {
            diagnostics.push(
                Diagnostic::new(
                    "stylesheet_syntax",
                    DiagnosticSeverity::Error,
                    "stylesheet block is missing closing '}'",
                )
                .with_line(line),
            );
            break;
        };

        let body = text[brace + 1..close].trim();
        if body.is_empty() {
            diagnostics.push(
                Diagnostic::new(
                    "stylesheet_syntax",
                    DiagnosticSeverity::Error,
                    "stylesheet rule block cannot be empty",
                )
                .with_line(line),
            );
        } else {
            let raw_statements = split_unquoted(body, ';');
            if raw_statements
                .iter()
                .enumerate()
                .any(|(statement_index, statement)| {
                    statement.trim().is_empty() && statement_index < raw_statements.len() - 1
                })
            {
                diagnostics.push(
                    Diagnostic::new(
                        "stylesheet_syntax",
                        DiagnosticSeverity::Error,
                        "stylesheet contains an empty declaration between ';' separators",
                    )
                    .with_line(line),
                );
            }

            let statements = raw_statements
                .iter()
                .map(|statement| statement.trim())
                .filter(|statement| !statement.is_empty())
                .collect::<Vec<_>>();
            if statements.is_empty() {
                diagnostics.push(
                    Diagnostic::new(
                        "stylesheet_syntax",
                        DiagnosticSeverity::Error,
                        "stylesheet rule block must include at least one declaration",
                    )
                    .with_line(line),
                );
            }

            for statement in statements {
                let colon_count = count_unquoted(statement, ':');
                if colon_count == 0 {
                    diagnostics.push(
                        Diagnostic::new(
                            "stylesheet_syntax",
                            DiagnosticSeverity::Error,
                            format!("stylesheet statement '{statement}' must contain ':'"),
                        )
                        .with_line(line),
                    );
                    continue;
                }
                if colon_count > 1 {
                    diagnostics.push(
                        Diagnostic::new(
                            "stylesheet_syntax",
                            DiagnosticSeverity::Error,
                            "stylesheet declarations must be separated by ';'",
                        )
                        .with_line(line),
                    );
                    continue;
                }

                let Some(colon) = find_unquoted(statement, ':', 0) else {
                    continue;
                };
                let key = statement[..colon].trim();
                if !ALLOWED_STYLESHEET_PROPERTIES.contains(&key) {
                    diagnostics.push(
                        Diagnostic::new(
                            "stylesheet_syntax",
                            DiagnosticSeverity::Error,
                            format!(
                                "unsupported stylesheet property '{key}', expected one of llm_model, llm_provider, llm_profile, reasoning_effort"
                            ),
                        )
                        .with_line(line),
                    );
                    continue;
                }

                let value = parse_stylesheet_value(statement[colon + 1..].trim());
                let Some(value) = value else {
                    diagnostics.push(
                        Diagnostic::new(
                            "stylesheet_syntax",
                            DiagnosticSeverity::Error,
                            format!(
                                "stylesheet property '{key}' must have a valid non-empty value"
                            ),
                        )
                        .with_line(line),
                    );
                    continue;
                };
                if key == "reasoning_effort" && !ALLOWED_REASONING_EFFORTS.contains(&value.as_str())
                {
                    diagnostics.push(
                        Diagnostic::new(
                            "stylesheet_syntax",
                            DiagnosticSeverity::Error,
                            "reasoning_effort must be one of: low, medium, high, xhigh",
                        )
                        .with_line(line),
                    );
                }
            }
        }

        index = close + 1;
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

fn validate_parallel_join_attrs(
    graph: &DotGraph,
    out_degree: &BTreeMap<String, usize>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for node in nodes_in_declaration_order(graph) {
        if resolves_to_handler_type(node) != "parallel" {
            continue;
        }
        let join_policy_attr = node.attrs.get("join_policy");
        let join_policy = join_policy_attr
            .map(attr_text)
            .unwrap_or_else(|| "wait_all".to_string())
            .trim()
            .to_string();
        if !SUPPORTED_PARALLEL_JOIN_POLICIES.contains(&join_policy.as_str()) {
            diagnostics.push(
                Diagnostic::new(
                    "parallel_join_policy",
                    DiagnosticSeverity::Error,
                    format!(
                        "node '{}' join_policy must be one of: wait_all, first_success, k_of_n, quorum",
                        node.node_id
                    ),
                )
                .with_line(join_policy_attr.map(|attr| attr.line).unwrap_or(node.line))
                .with_node(&node.node_id),
            );
            continue;
        }

        let join_k_attr = node.attrs.get("join_k");
        let join_quorum_attr = node.attrs.get("join_quorum");
        if join_policy != "k_of_n" && join_k_attr.is_some() {
            diagnostics.push(
                Diagnostic::new(
                    "parallel_join_threshold",
                    DiagnosticSeverity::Error,
                    format!(
                        "node '{}' join_k is only valid when join_policy is k_of_n",
                        node.node_id
                    ),
                )
                .with_line(join_k_attr.map(|attr| attr.line).unwrap_or(node.line))
                .with_node(&node.node_id),
            );
        }
        if join_policy != "quorum" && join_quorum_attr.is_some() {
            diagnostics.push(
                Diagnostic::new(
                    "parallel_join_threshold",
                    DiagnosticSeverity::Error,
                    format!(
                        "node '{}' join_quorum is only valid when join_policy is quorum",
                        node.node_id
                    ),
                )
                .with_line(join_quorum_attr.map(|attr| attr.line).unwrap_or(node.line))
                .with_node(&node.node_id),
            );
        }
        if join_policy == "k_of_n" {
            let branch_count = out_degree.get(&node.node_id).copied().unwrap_or_default();
            let join_k = join_k_attr.and_then(strict_positive_int_attr);
            if let Some(join_k) = join_k {
                if join_k > branch_count as i64 {
                    diagnostics.push(
                        Diagnostic::new(
                            "parallel_join_threshold",
                            DiagnosticSeverity::Error,
                            format!(
                                "node '{}' join_k must be <= outgoing branch count ({branch_count})",
                                node.node_id
                            ),
                        )
                        .with_line(join_k_attr.map(|attr| attr.line).unwrap_or(node.line))
                        .with_node(&node.node_id),
                    );
                }
            } else {
                diagnostics.push(
                    Diagnostic::new(
                        "parallel_join_threshold",
                        DiagnosticSeverity::Error,
                        format!(
                            "node '{}' join_k is required and must be an integer >= 1",
                            node.node_id
                        ),
                    )
                    .with_line(
                        join_k_attr
                            .or(join_policy_attr)
                            .map(|attr| attr.line)
                            .unwrap_or(node.line),
                    )
                    .with_node(&node.node_id),
                );
            }
        }
        if join_policy == "quorum" {
            if let Some(join_quorum_attr) = join_quorum_attr {
                let join_quorum = strict_float_attr(join_quorum_attr);
                if join_quorum
                    .map(|value| !value.is_finite() || value <= 0.0 || value > 1.0)
                    .unwrap_or(true)
                {
                    diagnostics.push(
                        Diagnostic::new(
                            "parallel_join_threshold",
                            DiagnosticSeverity::Error,
                            format!(
                                "node '{}' join_quorum must be finite and > 0 and <= 1",
                                node.node_id
                            ),
                        )
                        .with_line(join_quorum_attr.line)
                        .with_node(&node.node_id),
                    );
                }
            }
        }
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

fn attr_is_true(attr: Option<&DotAttribute>) -> bool {
    let Some(attr) = attr else {
        return false;
    };
    match &attr.value {
        DotValue::Boolean(value) => *value,
        DotValue::String(value) => value.trim().eq_ignore_ascii_case("true"),
        _ => false,
    }
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

fn strict_positive_int_attr(attr: &DotAttribute) -> Option<i64> {
    let parsed =
        match &attr.value {
            DotValue::Integer(value) => Some(*value),
            DotValue::String(value) => {
                let stripped = value.trim();
                if stripped.is_empty() {
                    None
                } else if stripped.chars().enumerate().all(|(index, ch)| {
                    ch.is_ascii_digit() || (index == 0 && matches!(ch, '+' | '-'))
                }) {
                    stripped.parse::<i64>().ok()
                } else {
                    None
                }
            }
            _ => None,
        }?;
    (parsed >= 1).then_some(parsed)
}

fn strict_float_attr(attr: &DotAttribute) -> Option<f64> {
    match &attr.value {
        DotValue::Integer(value) => Some(*value as f64),
        DotValue::Float(value) => Some(*value),
        DotValue::String(value) => value.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn stylesheet_selector_valid(selector: &str) -> bool {
    if selector == "*" {
        return true;
    }
    if let Some(class_name) = selector.strip_prefix('.') {
        return !class_name.is_empty()
            && class_name
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-');
    }
    if let Some(node_id) = selector.strip_prefix('#') {
        return is_node_id(node_id);
    }
    is_shape_selector(selector)
}

fn parse_stylesheet_value(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if value.starts_with('"') || value.ends_with('"') {
        if !(value.starts_with('"') && value.ends_with('"')) || value.len() <= 2 {
            return None;
        }
        let inner = &value[1..value.len() - 1];
        if !valid_quoted_stylesheet_inner(inner) {
            return None;
        }
        return Some(inner.to_string());
    }
    (!value.contains('"')).then(|| value.to_string())
}

fn valid_quoted_stylesheet_inner(value: &str) -> bool {
    let mut escaped = false;
    let mut has_char = false;
    for ch in value.chars() {
        if escaped {
            escaped = false;
            has_char = true;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            return false;
        }
        has_char = true;
    }
    has_char && !escaped
}

fn skip_ascii_whitespace(text: &str, start: usize) -> usize {
    let mut index = start;
    while index < text.len() {
        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        if !ch.is_ascii_whitespace() {
            break;
        }
        index += ch.len_utf8();
    }
    index
}

fn find_unquoted(text: &str, token: char, start: usize) -> Option<usize> {
    let mut in_quotes = false;
    let mut escaped = false;
    for (offset, ch) in text[start..].char_indices() {
        if ch == '\\' && in_quotes && !escaped {
            escaped = true;
            continue;
        }
        if ch == '"' && !escaped {
            in_quotes = !in_quotes;
        } else if ch == token && !in_quotes {
            return Some(start + offset);
        }
        escaped = false;
    }
    None
}

fn count_unquoted(text: &str, token: char) -> usize {
    let mut count = 0;
    let mut in_quotes = false;
    let mut escaped = false;
    for ch in text.chars() {
        if ch == '\\' && in_quotes && !escaped {
            escaped = true;
            continue;
        }
        if ch == '"' && !escaped {
            in_quotes = !in_quotes;
        } else if ch == token && !in_quotes {
            count += 1;
        }
        escaped = false;
    }
    count
}

fn split_unquoted(text: &str, token: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let mut escaped = false;
    for (index, ch) in text.char_indices() {
        if ch == '\\' && in_quotes && !escaped {
            escaped = true;
            continue;
        }
        if ch == '"' && !escaped {
            in_quotes = !in_quotes;
        } else if ch == token && !in_quotes {
            parts.push(text[start..index].to_string());
            start = index + ch.len_utf8();
        }
        escaped = false;
    }
    parts.push(text[start..].to_string());
    parts
}

fn is_shape_selector(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphabetic() && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn is_node_id(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}
