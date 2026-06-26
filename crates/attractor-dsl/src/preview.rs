use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use attractor_core::{
    DotAttribute, DotEdge, DotGraph, DotNode, DotScopeDefaults, DotSubgraphScope, DotValue,
};
use serde_json::{json, Number, Value};

use crate::{
    apply_graph_transforms_with_extra, diagnostics_payload, parse_dot, validate_graph, Diagnostic,
    DiagnosticSeverity, DotParseError, GraphTransform,
};

const UNSUPPORTED_PREVIEW_ATTRS: &[&str] = &["human.default_choice"];

const NODE_PREVIEW_KEYS: &[&str] = &[
    "shape",
    "prompt",
    "tool.command",
    "tool.hooks.pre",
    "tool.hooks.post",
    "tool.artifacts.paths",
    "tool.artifacts.stdout",
    "tool.artifacts.stderr",
    "join_policy",
    "join_k",
    "join_quorum",
    "error_policy",
    "max_parallel",
    "type",
    "max_retries",
    "goal_gate",
    "retry_target",
    "fallback_retry_target",
    "fidelity",
    "thread_id",
    "class",
    "timeout",
    "llm_model",
    "llm_provider",
    "llm_profile",
    "reasoning_effort",
    "auto_status",
    "allow_partial",
    "manager.poll_interval",
    "manager.max_cycles",
    "manager.stop_condition",
    "manager.actions",
    "manager.steer_cooldown",
    "stack.child_autostart",
];

const GRAPH_PREVIEW_KEYS: &[(&str, Option<&str>)] = &[
    ("goal", None),
    ("label", Some("")),
    ("model_stylesheet", None),
    ("default_max_retries", None),
    ("retry_target", None),
    ("fallback_retry_target", None),
    ("default_fidelity", None),
    ("stack.child_dotfile", None),
    ("stack.child_workdir", None),
    ("tool.hooks.pre", None),
    ("tool.hooks.post", None),
    ("ui_default_llm_model", None),
    ("ui_default_llm_provider", None),
    ("ui_default_llm_profile", None),
    ("ui_default_reasoning_effort", None),
];

const EDGE_PREVIEW_KEYS: &[&str] = &[
    "label",
    "condition",
    "weight",
    "fidelity",
    "thread_id",
    "loop_restart",
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreviewOptions {
    pub expand_children: bool,
    pub flow_source_dir: Option<PathBuf>,
    pub run_workdir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DotPreview {
    pub graph: Option<DotGraph>,
    pub payload: Value,
}

pub fn preview_dot_source(dot_source: &str) -> DotPreview {
    preview_dot_source_with_extra(dot_source, std::iter::empty())
}

pub fn preview_dot_source_with_extra(
    dot_source: &str,
    extra_transforms: impl IntoIterator<Item = Box<dyn GraphTransform>>,
) -> DotPreview {
    match parse_dot(dot_source) {
        Ok(graph) => {
            let graph = apply_graph_transforms_with_extra(&graph, extra_transforms);
            let diagnostics = validate_graph(&graph);
            let errors = error_diagnostics(&diagnostics);
            DotPreview {
                graph: Some(graph),
                payload: json!({
                    "status": if errors.is_empty() { "ok" } else { "validation_error" },
                    "diagnostics": diagnostics_payload(&diagnostics),
                    "errors": diagnostics_payload(&errors),
                }),
            }
        }
        Err(error) => DotPreview {
            graph: None,
            payload: parse_error_payload(&error),
        },
    }
}

pub fn preview_response_payload(dot_source: &str) -> Value {
    preview_response_payload_with_options(dot_source, PreviewOptions::default())
}

pub fn preview_response_payload_with_options(dot_source: &str, options: PreviewOptions) -> Value {
    preview_response_payload_with_extra(dot_source, options, std::iter::empty())
}

pub fn preview_response_payload_with_extra(
    dot_source: &str,
    options: PreviewOptions,
    extra_transforms: impl IntoIterator<Item = Box<dyn GraphTransform>>,
) -> Value {
    let preview = preview_dot_source_with_extra(dot_source, extra_transforms);
    let Some(graph) = preview.graph.as_ref() else {
        return preview.payload;
    };

    let child_previews = if options.expand_children {
        build_child_preview_payload(
            graph,
            options.flow_source_dir.as_deref(),
            options.run_workdir.as_deref(),
        )
    } else {
        BTreeMap::new()
    };
    let graph_payload = graph_payload_with_child_previews(
        graph,
        if child_previews.is_empty() {
            None
        } else {
            Some(child_previews)
        },
    );

    let mut payload = match preview.payload {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    payload.insert("graph".to_string(), graph_payload);
    Value::Object(payload)
}

pub fn parse_error_payload(error: &DotParseError) -> Value {
    let parse_diag = json!({
        "rule": "parse_error",
        "rule_id": "parse_error",
        "severity": "error",
        "message": error.to_string(),
        "line": error.line(),
        "node": null,
        "node_id": null,
    });
    json!({
        "status": "parse_error",
        "error": error.to_string(),
        "diagnostics": [parse_diag.clone()],
        "errors": [parse_diag],
    })
}

pub fn graph_payload(graph: &DotGraph) -> Value {
    graph_payload_with_child_previews(graph, None)
}

pub fn graph_payload_with_child_previews(
    graph: &DotGraph,
    child_previews: Option<BTreeMap<String, Value>>,
) -> Value {
    let nodes = nodes_in_declaration_order(graph)
        .into_iter()
        .map(node_payload)
        .collect::<Vec<_>>();
    let edges = graph.edges.iter().map(edge_payload).collect::<Vec<_>>();
    let subgraphs = graph
        .subgraphs
        .iter()
        .map(subgraph_payload)
        .collect::<Vec<_>>();

    let mut payload = serde_json::Map::from_iter([
        ("nodes".to_string(), Value::Array(nodes)),
        ("graph_attrs".to_string(), graph_attrs_payload(graph)),
        ("edges".to_string(), Value::Array(edges)),
        ("defaults".to_string(), defaults_payload(&graph.defaults)),
        ("subgraphs".to_string(), Value::Array(subgraphs)),
    ]);
    if let Some(child_previews) = child_previews {
        payload.insert(
            "child_previews".to_string(),
            Value::Object(child_previews.into_iter().collect()),
        );
    }
    Value::Object(payload)
}

fn node_payload(node: &DotNode) -> Value {
    let mut payload = serde_json::Map::from_iter([
        ("id".to_string(), json!(node.node_id)),
        (
            "label".to_string(),
            attr_value_or_default(&node.attrs, "label", Value::String(node.node_id.clone())),
        ),
    ]);
    for key in NODE_PREVIEW_KEYS {
        payload.insert((*key).to_string(), attr_value_or_null(&node.attrs, key));
    }
    merge_extension_attrs(payload, &node.attrs)
}

fn graph_attrs_payload(graph: &DotGraph) -> Value {
    let mut payload = serde_json::Map::new();
    for (key, default) in GRAPH_PREVIEW_KEYS {
        let value = match default {
            Some(default) => attr_value_or_default(
                &graph.graph_attrs,
                key,
                Value::String((*default).to_string()),
            ),
            None => attr_value_or_null(&graph.graph_attrs, key),
        };
        payload.insert((*key).to_string(), value);
    }
    merge_extension_attrs(payload, &graph.graph_attrs)
}

fn edge_payload(edge: &DotEdge) -> Value {
    let mut payload = serde_json::Map::from_iter([
        ("from".to_string(), json!(edge.source)),
        ("to".to_string(), json!(edge.target)),
    ]);
    for key in EDGE_PREVIEW_KEYS {
        payload.insert((*key).to_string(), attr_value_or_null(&edge.attrs, key));
    }
    merge_extension_attrs(payload, &edge.attrs)
}

fn defaults_payload(defaults: &DotScopeDefaults) -> Value {
    json!({
        "node": all_attrs_payload(&defaults.node),
        "edge": all_attrs_payload(&defaults.edge),
    })
}

fn subgraph_payload(scope: &DotSubgraphScope) -> Value {
    json!({
        "id": scope.id,
        "attrs": all_attrs_payload(&scope.attrs),
        "node_ids": scope.node_ids,
        "defaults": defaults_payload(&scope.defaults),
        "subgraphs": scope.subgraphs.iter().map(subgraph_payload).collect::<Vec<_>>(),
    })
}

fn merge_extension_attrs(
    mut payload: serde_json::Map<String, Value>,
    attrs: &BTreeMap<String, DotAttribute>,
) -> Value {
    for (key, value) in all_attrs_payload_map(attrs) {
        payload.entry(key).or_insert(value);
    }
    Value::Object(payload)
}

fn all_attrs_payload(attrs: &BTreeMap<String, DotAttribute>) -> Value {
    Value::Object(all_attrs_payload_map(attrs))
}

fn all_attrs_payload_map(attrs: &BTreeMap<String, DotAttribute>) -> serde_json::Map<String, Value> {
    attrs
        .iter()
        .filter(|(key, _)| !UNSUPPORTED_PREVIEW_ATTRS.contains(&key.as_str()))
        .map(|(key, attr)| (key.clone(), dot_value_to_json(&attr.value)))
        .collect()
}

fn attr_value_or_null(attrs: &BTreeMap<String, DotAttribute>, key: &str) -> Value {
    attrs
        .get(key)
        .map(|attr| dot_value_to_json(&attr.value))
        .unwrap_or(Value::Null)
}

fn attr_value_or_default(
    attrs: &BTreeMap<String, DotAttribute>,
    key: &str,
    default: Value,
) -> Value {
    attrs
        .get(key)
        .map(|attr| dot_value_to_json(&attr.value))
        .unwrap_or(default)
}

fn dot_value_to_json(value: &DotValue) -> Value {
    match value {
        DotValue::Null => Value::Null,
        DotValue::String(value) => Value::String(value.clone()),
        DotValue::Integer(value) => Value::Number(Number::from(*value)),
        DotValue::Float(value) => Number::from_f64(*value)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(value.to_string())),
        DotValue::Boolean(value) => Value::Bool(*value),
        DotValue::Duration(value) => Value::String(value.raw.clone()),
    }
}

fn error_diagnostics(diagnostics: &[Diagnostic]) -> Vec<Diagnostic> {
    diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        .cloned()
        .collect()
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

fn build_child_preview_payload(
    graph: &DotGraph,
    flow_source_dir: Option<&Path>,
    run_workdir: Option<&Path>,
) -> BTreeMap<String, Value> {
    let child_dotfile = preview_attr_string(graph.graph_attrs.get("stack.child_dotfile"));
    if child_dotfile.is_empty() {
        return BTreeMap::new();
    }

    let authored_child_workdir =
        preview_authored_attr_string(graph.graph_attrs.get("stack.child_workdir"));
    let child_workdir_path =
        resolve_preview_child_workdir(&authored_child_workdir, flow_source_dir, run_workdir);
    let child_dot_path = resolve_preview_child_dot_path(
        &child_dotfile,
        &child_workdir_path,
        flow_source_dir,
        run_workdir,
        !authored_child_workdir.is_empty(),
    );
    if !child_dot_path.exists() {
        return BTreeMap::new();
    }

    let Ok(child_source) = std::fs::read_to_string(&child_dot_path) else {
        return BTreeMap::new();
    };
    let child_preview = preview_dot_source(&child_source);
    if child_preview
        .payload
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| status != "ok")
    {
        return BTreeMap::new();
    }
    let Some(child_graph) = child_preview.graph.as_ref() else {
        return BTreeMap::new();
    };
    let child_graph_payload = graph_payload(child_graph);
    let child_flow_label = child_graph_payload
        .get("graph_attrs")
        .and_then(|value| value.get("label"))
        .and_then(Value::as_str)
        .filter(|label| !label.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            child_dot_path
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
                .unwrap_or_default()
        });
    let child_flow_name = child_dot_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let child_flow_path = child_dot_path.to_string_lossy().to_string();

    nodes_in_declaration_order(graph)
        .into_iter()
        .filter(|node| is_manager_loop_node(node))
        .map(|node| {
            (
                node.node_id.clone(),
                json!({
                    "flow_name": child_flow_name,
                    "flow_path": child_flow_path,
                    "flow_label": child_flow_label,
                    "parent_node_id": node.node_id,
                    "read_only": true,
                    "provenance": "derived_child_preview",
                    "graph": child_graph_payload,
                }),
            )
        })
        .collect()
}

fn preview_attr_string(attr: Option<&DotAttribute>) -> String {
    attr.map(|attr| dot_value_to_text(&attr.value).trim().to_string())
        .unwrap_or_default()
}

fn preview_authored_attr_string(attr: Option<&DotAttribute>) -> String {
    match attr {
        Some(attr) if attr.line != 0 => preview_attr_string(Some(attr)),
        _ => String::new(),
    }
}

fn dot_value_to_text(value: &DotValue) -> String {
    match value {
        DotValue::Null => String::new(),
        DotValue::String(value) => value.clone(),
        DotValue::Integer(value) => value.to_string(),
        DotValue::Float(value) => value.to_string(),
        DotValue::Boolean(value) => value.to_string(),
        DotValue::Duration(value) => value.raw.clone(),
    }
}

fn resolve_preview_child_workdir(
    authored_child_workdir: &str,
    preview_base_dir: Option<&Path>,
    run_workdir: Option<&Path>,
) -> PathBuf {
    let fallback_cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let base_dir = run_workdir.or(preview_base_dir).unwrap_or(&fallback_cwd);
    if !authored_child_workdir.is_empty() {
        return resolve_preview_path_from_base(authored_child_workdir, base_dir);
    }
    run_workdir
        .or(preview_base_dir)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| normalize_path_lexically(&fallback_cwd))
}

fn resolve_preview_child_dot_path(
    child_dotfile: &str,
    child_workdir_path: &Path,
    flow_source_dir: Option<&Path>,
    run_workdir: Option<&Path>,
    child_workdir_is_authored: bool,
) -> PathBuf {
    let child_dot_path = Path::new(child_dotfile);
    if child_dot_path.is_absolute() {
        return normalize_path_lexically(child_dot_path);
    }
    let base_dir = if child_workdir_is_authored {
        child_workdir_path
    } else {
        flow_source_dir
            .or(run_workdir)
            .unwrap_or(child_workdir_path)
    };
    resolve_preview_path_from_base(child_dotfile, base_dir)
}

fn resolve_preview_path_from_base(raw_path: impl AsRef<Path>, base_dir: &Path) -> PathBuf {
    let candidate = raw_path.as_ref();
    if candidate.is_absolute() {
        normalize_path_lexically(candidate)
    } else {
        normalize_path_lexically(&base_dir.join(candidate))
    }
}

fn normalize_path_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    normalized
}

fn is_manager_loop_node(node: &DotNode) -> bool {
    let node_type = node
        .attrs
        .get("type")
        .map(|attr| dot_value_to_text(&attr.value).trim().to_string())
        .unwrap_or_default();
    let node_shape = node
        .attrs
        .get("shape")
        .map(|attr| dot_value_to_text(&attr.value).trim().to_string())
        .unwrap_or_default();
    node_type == "stack.manager_loop" || node_shape == "house"
}
