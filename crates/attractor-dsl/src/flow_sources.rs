use std::path::{Component, Path, PathBuf};

use attractor_core::{DotAttribute, DotValue, DotValueType};
use thiserror::Error;

use crate::{
    build_transform_pipeline, format_dot, normalize_graph, parse_dot, DotGraph, DotParseError,
};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{detail}")]
pub struct FlowSourceError {
    status_code: u16,
    detail: String,
}

impl FlowSourceError {
    pub fn new(status_code: u16, detail: impl Into<String>) -> Self {
        Self {
            status_code,
            detail: detail.into(),
        }
    }

    pub fn status_code(&self) -> u16 {
        self.status_code
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

pub fn ensure_flows_dir(flows_dir: impl AsRef<Path>) -> Result<PathBuf, FlowSourceError> {
    let flows_dir = flows_dir.as_ref();
    std::fs::create_dir_all(flows_dir).map_err(|source| {
        FlowSourceError::new(
            500,
            format!(
                "Unable to create flows directory {}: {source}",
                flows_dir.display()
            ),
        )
    })?;
    Ok(flows_dir.to_path_buf())
}

pub fn normalize_flow_name(flow_name: &str) -> Result<String, FlowSourceError> {
    let raw_name = flow_name.trim().replace('\\', "/");
    if raw_name.is_empty() {
        return Err(FlowSourceError::new(400, "Flow name is required."));
    }
    if raw_name.ends_with('/') {
        return Err(FlowSourceError::new(
            400,
            "Flow name must reference a file.",
        ));
    }
    if raw_name.starts_with('/') {
        return Err(path_safety_error());
    }

    let mut parts = Vec::new();
    for part in raw_name.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(path_safety_error());
        }
        parts.push(part.to_string());
    }
    if parts.is_empty() {
        return Err(path_safety_error());
    }

    let Some(leaf_name) = parts.last_mut() else {
        return Err(FlowSourceError::new(
            400,
            "Flow name must reference a file.",
        ));
    };
    if leaf_name.is_empty() {
        return Err(FlowSourceError::new(
            400,
            "Flow name must reference a file.",
        ));
    }
    if !leaf_name.ends_with(".dot") {
        leaf_name.push_str(".dot");
    }

    Ok(parts.join("/"))
}

pub fn resolve_flow_path(
    flows_dir: impl AsRef<Path>,
    flow_name: &str,
) -> Result<PathBuf, FlowSourceError> {
    let normalized_name = normalize_flow_name(flow_name)?;
    let root = ensure_flows_dir(flows_dir)?;
    Ok(normalized_name
        .split('/')
        .fold(root, |path, part| path.join(part)))
}

pub fn flow_name_from_path(
    flows_dir: impl AsRef<Path>,
    flow_path: impl AsRef<Path>,
) -> Result<String, FlowSourceError> {
    let root = ensure_flows_dir(flows_dir)?;
    let relative = flow_path
        .as_ref()
        .strip_prefix(&root)
        .map_err(|_| path_safety_error())?;
    let parts = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return Err(FlowSourceError::new(
            400,
            "Flow name must reference a file.",
        ));
    }
    Ok(parts.join("/"))
}

pub fn load_flow_content(
    flows_dir: impl AsRef<Path>,
    flow_source: &str,
) -> Result<String, FlowSourceError> {
    let flow_path = resolve_flow_path(flows_dir, flow_source)?;
    if !flow_path.exists() {
        let leaf = flow_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| flow_source.to_string());
        return Err(FlowSourceError::new(404, format!("Flow not found: {leaf}")));
    }
    std::fs::read_to_string(&flow_path).map_err(|source| {
        FlowSourceError::new(
            500,
            format!("Unable to read flow file {}: {source}", flow_path.display()),
        )
    })
}

pub fn inject_pipeline_goal(flow_content: &str, goal: &str) -> Result<String, DotParseError> {
    let mut graph = parse_dot(flow_content)?;
    let line = graph
        .graph_attrs
        .get("goal")
        .map(|attr| attr.line)
        .unwrap_or_default();
    graph.graph_attrs.insert(
        "goal".to_string(),
        DotAttribute {
            key: "goal".to_string(),
            value: DotValue::String(goal.to_string()),
            value_type: DotValueType::String,
            line,
        },
    );
    Ok(format_dot(&graph))
}

pub fn semantic_signature_for_source(dot_content: &str) -> Result<String, DotParseError> {
    let graph = parse_dot(dot_content)?;
    Ok(semantic_signature_for_graph(&graph))
}

pub fn semantic_equivalent_sources(left: &str, right: &str) -> Result<bool, DotParseError> {
    Ok(semantic_signature_for_source(left)? == semantic_signature_for_source(right)?)
}

pub fn semantic_signature_for_graph(graph: &DotGraph) -> String {
    let transformed = build_transform_pipeline().apply(graph);
    let mut normalized = normalize_graph(&transformed);
    normalized.graph_id = "__semantic__".to_string();
    format_dot(&normalized)
}

fn path_safety_error() -> FlowSourceError {
    FlowSourceError::new(400, "Flow name must be a relative path inside flows_dir.")
}
