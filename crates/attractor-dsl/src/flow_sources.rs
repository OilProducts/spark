use std::path::{Component, Path, PathBuf};

use attractor_core::{FlowDefinition, FlowDefinitionError};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{detail}")]
pub struct FlowSourceError {
    status_code: u16,
    detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NamedFlowSource {
    pub name: String,
    pub path: PathBuf,
    pub content: String,
    pub flow: FlowDefinition,
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
    if Path::new(leaf_name).extension().is_some() {
        let extension = Path::new(leaf_name)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if !matches!(extension, "yaml" | "yml") {
            return Err(FlowSourceError::new(
                400,
                "Flow name must end with .yaml or .yml.",
            ));
        }
    } else {
        leaf_name.push_str(".yaml");
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

pub fn parse_flow_definition(source: &str) -> Result<FlowDefinition, FlowSourceError> {
    let flow = FlowDefinition::from_yaml_str(source).map_err(flow_definition_error)?;
    flow.validate().map_err(flow_definition_error)?;
    Ok(flow.normalize())
}

pub fn read_named_flow_source(
    flows_dir: impl AsRef<Path>,
    flow_name: &str,
) -> Result<NamedFlowSource, FlowSourceError> {
    let flows_dir = flows_dir.as_ref();
    let path = resolve_flow_path(flows_dir, flow_name)?;
    if !path.exists() {
        return Err(FlowSourceError::new(404, "Flow not found."));
    }
    let content = std::fs::read_to_string(&path).map_err(|source| {
        FlowSourceError::new(
            500,
            format!("Unable to read flow file {}: {source}", path.display()),
        )
    })?;
    let flow = parse_flow_definition(&content)?;
    let name = flow_name_from_path(flows_dir, &path)?;
    Ok(NamedFlowSource {
        name,
        path,
        content,
        flow,
    })
}

pub fn canonicalize_flow_yaml(source: &str) -> Result<String, FlowSourceError> {
    let flow = parse_flow_definition(source)?;
    serde_yaml::to_string(&flow).map_err(|source| {
        FlowSourceError::new(
            500,
            format!("Unable to serialize canonical flow YAML: {source}"),
        )
    })
}

pub fn inject_flow_goal(flow_content: &str, goal: &str) -> Result<String, FlowSourceError> {
    let mut flow = parse_flow_definition(flow_content)?;
    flow.goal = goal.trim().to_string();
    serde_yaml::to_string(&flow.normalize()).map_err(|source| {
        FlowSourceError::new(
            500,
            format!("Unable to serialize flow with launch goal: {source}"),
        )
    })
}

fn path_safety_error() -> FlowSourceError {
    FlowSourceError::new(400, "Flow name must be a relative path inside flows_dir.")
}

fn flow_definition_error(error: FlowDefinitionError) -> FlowSourceError {
    FlowSourceError::new(422, error.to_string())
}
