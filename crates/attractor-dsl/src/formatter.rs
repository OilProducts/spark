use std::collections::{BTreeMap, BTreeSet};

use attractor_core::{DotAttribute, DotEdge, DotGraph, DotValue};

use crate::error::DotParseError;
use crate::parser::{normalize_graph, parse_dot};

pub fn canonicalize_dot(source: &str) -> Result<String, DotParseError> {
    parse_dot(source).map(|graph| format_dot(&graph))
}

pub fn canonicalize_readable_dot(source: &str) -> Result<String, DotParseError> {
    parse_dot(source).map(|graph| format_readable_dot(&graph))
}

pub fn format_dot(graph: &DotGraph) -> String {
    let mut lines = vec![format!("digraph {} {{", graph.graph_id)];

    if !graph.graph_attrs.is_empty() {
        lines.push(format!("  graph [{}];", format_attrs(&graph.graph_attrs)));
    }

    for (node_id, node) in &graph.nodes {
        if node.attrs.is_empty() {
            lines.push(format!("  {node_id};"));
        } else {
            lines.push(format!("  {node_id} [{}];", format_attrs(&node.attrs)));
        }
    }

    let mut edges = graph.edges.iter().collect::<Vec<_>>();
    edges.sort_by(|left, right| edge_sort_key(left).cmp(&edge_sort_key(right)));
    for edge in edges {
        lines.push(format_edge(edge));
    }

    lines.push("}".to_string());
    lines.join("\n") + "\n"
}

pub fn format_readable_dot(graph: &DotGraph) -> String {
    let mut lines = vec![format!("digraph {} {{", graph.graph_id)];

    if !graph.graph_attrs.is_empty() {
        lines.push(format!("  graph [{}];", format_attrs(&graph.graph_attrs)));
    }
    if !graph.defaults.node.is_empty() {
        lines.push(format!("  node [{}];", format_attrs(&graph.defaults.node)));
    }
    if !graph.defaults.edge.is_empty() {
        lines.push(format!("  edge [{}];", format_attrs(&graph.defaults.edge)));
    }

    let mut outgoing_edges: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (edge_index, edge) in graph.edges.iter().enumerate() {
        outgoing_edges
            .entry(edge.source.clone())
            .or_default()
            .push(edge_index);
    }

    let mut emitted_nodes = BTreeSet::new();
    let mut emitted_edge_ids = BTreeSet::new();

    if let Some(start_node_id) = resolve_readable_start_node(graph) {
        emit_readable_node(
            &start_node_id,
            graph,
            &outgoing_edges,
            &mut emitted_nodes,
            &mut emitted_edge_ids,
            &mut lines,
        );
    }

    for node_id in readable_node_order(graph) {
        emit_readable_node(
            &node_id,
            graph,
            &outgoing_edges,
            &mut emitted_nodes,
            &mut emitted_edge_ids,
            &mut lines,
        );
    }

    for (edge_index, edge) in graph.edges.iter().enumerate() {
        if !emitted_edge_ids.contains(&edge_index) {
            lines.push(format_edge(edge));
        }
    }

    lines.push("}".to_string());
    lines.join("\n") + "\n"
}

pub fn semantic_signature(graph: &DotGraph) -> String {
    let mut normalized = normalize_graph(graph);
    normalized.graph_id = "__semantic__".to_string();
    format_dot(&normalized)
}

pub fn semantic_equivalent(left: &DotGraph, right: &DotGraph) -> bool {
    semantic_signature(left) == semantic_signature(right)
}

fn emit_readable_node(
    node_id: &str,
    graph: &DotGraph,
    outgoing_edges: &BTreeMap<String, Vec<usize>>,
    emitted_nodes: &mut BTreeSet<String>,
    emitted_edge_ids: &mut BTreeSet<usize>,
    lines: &mut Vec<String>,
) {
    let Some(node) = graph.nodes.get(node_id) else {
        return;
    };
    if !emitted_nodes.insert(node_id.to_string()) {
        return;
    }

    lines.push(String::new());
    if node.attrs.is_empty() {
        lines.push(format!("  {node_id};"));
    } else {
        lines.push(format!("  {node_id} [{}];", format_attrs(&node.attrs)));
    }

    let edge_indexes = outgoing_edges.get(node_id).cloned().unwrap_or_default();
    for edge_index in &edge_indexes {
        let edge = &graph.edges[*edge_index];
        lines.push(format_edge(edge));
        emitted_edge_ids.insert(*edge_index);
    }

    for edge_index in edge_indexes {
        let target = graph.edges[edge_index].target.clone();
        if !emitted_nodes.contains(&target) {
            emit_readable_node(
                &target,
                graph,
                outgoing_edges,
                emitted_nodes,
                emitted_edge_ids,
                lines,
            );
        }
    }
}

fn readable_node_order(graph: &DotGraph) -> Vec<String> {
    let mut nodes = graph.nodes.values().collect::<Vec<_>>();
    nodes.sort_by(|left, right| {
        node_line_order(left.line)
            .cmp(&node_line_order(right.line))
            .then_with(|| left.declaration_order.cmp(&right.declaration_order))
            .then_with(|| left.node_id.cmp(&right.node_id))
    });
    nodes
        .into_iter()
        .map(|node| node.node_id.clone())
        .collect::<Vec<_>>()
}

fn node_line_order(line: usize) -> usize {
    if line == 0 {
        usize::MAX
    } else {
        line
    }
}

fn resolve_readable_start_node(graph: &DotGraph) -> Option<String> {
    let mut shape_starts = graph
        .nodes
        .iter()
        .filter_map(|(node_id, node)| {
            let shape = node.attrs.get("shape")?;
            (shape.value.to_string() == "Mdiamond").then(|| node_id.clone())
        })
        .collect::<Vec<_>>();

    if shape_starts.len() == 1 {
        return shape_starts.pop();
    }

    for candidate in ["start", "Start"] {
        if graph.nodes.contains_key(candidate) {
            return Some(candidate.to_string());
        }
    }

    None
}

fn format_edge(edge: &DotEdge) -> String {
    if edge.attrs.is_empty() {
        format!("  {} -> {};", edge.source, edge.target)
    } else {
        format!(
            "  {} -> {} [{}];",
            edge.source,
            edge.target,
            format_attrs(&edge.attrs)
        )
    }
}

fn edge_sort_key(edge: &DotEdge) -> (String, String, String) {
    let attrs = if edge.attrs.is_empty() {
        String::new()
    } else {
        format_attrs(&edge.attrs)
    };
    (edge.source.clone(), edge.target.clone(), attrs)
}

fn format_attrs(attrs: &BTreeMap<String, DotAttribute>) -> String {
    attrs
        .iter()
        .map(|(key, attr)| format!("{key}={}", format_value(attr)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_value(attr: &DotAttribute) -> String {
    match &attr.value {
        DotValue::Null => quote_dot_string(""),
        DotValue::String(value) => quote_dot_string(value),
        DotValue::Integer(value) => value.to_string(),
        DotValue::Float(value) => format_float(*value),
        DotValue::Boolean(value) => {
            if *value {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        DotValue::Duration(value) => value.raw.clone(),
    }
}

fn format_float(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_string();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-inf".to_string()
        } else {
            "inf".to_string()
        };
    }

    let mut rendered = value.to_string();
    if value.fract() == 0.0 && !rendered.contains('.') && !has_exponent(&rendered) {
        rendered.push_str(".0");
    }

    if value == 0.0 {
        return rendered;
    }

    match decimal_adjusted_exponent(&rendered) {
        Some(exponent) if !has_exponent(&rendered) && !(-4..16).contains(&exponent) => {
            decimal_to_python_scientific(&rendered).unwrap_or(rendered)
        }
        _ => rendered,
    }
}

fn has_exponent(value: &str) -> bool {
    value.contains('e') || value.contains('E')
}

fn decimal_adjusted_exponent(value: &str) -> Option<i32> {
    let value = value.strip_prefix('-').unwrap_or(value);
    let (integer, fractional) = value.split_once('.').unwrap_or((value, ""));

    let significant_integer = integer.trim_start_matches('0');
    if !significant_integer.is_empty() {
        return Some(significant_integer.len() as i32 - 1);
    }

    let first_fractional_digit = fractional.chars().position(|ch| ch != '0')?;
    Some(-((first_fractional_digit as i32) + 1))
}

fn decimal_to_python_scientific(value: &str) -> Option<String> {
    let (negative, unsigned) = value
        .strip_prefix('-')
        .map(|rest| (true, rest))
        .unwrap_or((false, value));
    let (integer, fractional) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let decimal_point = integer.len() as i32;
    let digits = format!("{integer}{fractional}");
    let first_significant = digits.chars().position(|ch| ch != '0')?;
    let exponent = decimal_point - first_significant as i32 - 1;
    let mut significant = digits[first_significant..]
        .trim_end_matches('0')
        .to_string();
    if significant.is_empty() {
        significant.push('0');
    }

    let mut mantissa = String::new();
    if negative {
        mantissa.push('-');
    }
    let mut significant_chars = significant.chars();
    mantissa.push(significant_chars.next()?);
    let rest = significant_chars.collect::<String>();
    if !rest.is_empty() {
        mantissa.push('.');
        mantissa.push_str(&rest);
    }

    Some(format!("{mantissa}e{}", format_python_exponent(exponent)))
}

fn format_python_exponent(exponent: i32) -> String {
    let sign = if exponent < 0 { '-' } else { '+' };
    let magnitude = exponent.abs();
    if magnitude < 10 {
        format!("{sign}0{magnitude}")
    } else {
        format!("{sign}{magnitude}")
    }
}

fn quote_dot_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\t' => escaped.push_str("\\t"),
            other => escaped.push(other),
        }
    }
    escaped.push('"');
    escaped
}
