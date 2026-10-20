use std::path::{Component, Path, PathBuf};

use attractor_core::{FlowDefinition, FlowDefinitionError, FlowDiagnostic};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::flow_files::{read_flow_dir, resolve_flow_roles, FLOW_FILE};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{detail}")]
pub struct FlowSourceError {
    status_code: u16,
    detail: String,
    diagnostics: Vec<FlowDiagnostic>,
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
            diagnostics: Vec::new(),
        }
    }

    pub fn status_code(&self) -> u16 {
        self.status_code
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }

    pub fn diagnostics(&self) -> &[FlowDiagnostic] {
        &self.diagnostics
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
            "Flow name must not end with '/'.",
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

    let leaf_name = parts.last().map(String::as_str).unwrap_or_default();
    if leaf_name.ends_with(".yaml") || leaf_name.ends_with(".yml") || leaf_name.ends_with(".md") {
        return Err(FlowSourceError::new(
            400,
            "Flow names name a flow directory; drop the file extension.",
        ));
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

/// Reads a named flow directory in its authored form, as one YAML document.
pub fn load_flow_content(
    flows_dir: impl AsRef<Path>,
    flow_name: &str,
) -> Result<String, FlowSourceError> {
    flow_yaml(&read_named_flow(flows_dir, flow_name)?)
}

/// Reads a named flow directory in the resolved form runs execute: roles
/// merged in, as one YAML document.
pub fn load_resolved_flow_content(
    flows_dir: impl AsRef<Path>,
    roles_dir: &Path,
    flow_name: &str,
) -> Result<String, FlowSourceError> {
    flow_yaml(&resolve_flow_roles(
        &read_named_flow(flows_dir, flow_name)?,
        roles_dir,
    )?)
}

fn read_named_flow(
    flows_dir: impl AsRef<Path>,
    flow_name: &str,
) -> Result<FlowDefinition, FlowSourceError> {
    let flow_path = resolve_flow_path(flows_dir, flow_name)?;
    read_flow_dir(&flow_path).map_err(|error| {
        if error.status_code() == 404 {
            FlowSourceError::new(404, format!("Flow not found: {}", flow_name.trim()))
        } else {
            error
        }
    })
}

/// Resolves a subflow's `flow_ref` against `base_dir`: the directory that
/// contains the parent flow's directory. Preview and runtime share this.
pub fn resolve_flow_ref(base_dir: &Path, flow_ref: &str) -> PathBuf {
    let flow_ref = flow_ref.trim().replace('\\', "/");
    let flow_ref = Path::new(&flow_ref);
    if flow_ref.is_absolute() {
        flow_ref.to_path_buf()
    } else {
        base_dir.join(flow_ref)
    }
}

/// Names of every flow directory (one containing `flow.md`) under `flows_dir`.
pub fn list_flow_names(flows_dir: impl AsRef<Path>) -> Result<Vec<String>, FlowSourceError> {
    fn collect(dir: &Path, paths: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            let hidden = path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with('.'));
            if hidden || !path.is_dir() {
                continue;
            }
            if path.join(FLOW_FILE).is_file() {
                paths.push(path);
            } else {
                collect(&path, paths)?;
            }
        }
        Ok(())
    }
    let flows_dir = ensure_flows_dir(flows_dir)?;
    let mut paths = Vec::new();
    collect(&flows_dir, &mut paths).map_err(|source| list_error(&flows_dir, source))?;
    let mut names = paths
        .into_iter()
        .map(|path| flow_name_from_path(&flows_dir, path))
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    Ok(names)
}

fn list_error(flows_dir: &Path, source: std::io::Error) -> FlowSourceError {
    FlowSourceError::new(
        500,
        format!(
            "Unable to list flows directory {}: {source}",
            flows_dir.display()
        ),
    )
}

pub fn parse_flow_definition(source: &str) -> Result<FlowDefinition, FlowSourceError> {
    let flow = FlowDefinition::from_yaml_str(source).map_err(flow_definition_error)?;
    validate_flow_definition(&flow).map_err(flow_definition_error)?;
    Ok(flow.normalize())
}

pub fn validate_flow_definition(flow: &FlowDefinition) -> Result<(), FlowDefinitionError> {
    flow.validate()?;
    let diagnostics: Vec<_> = crate::validate_graph(&flow.to_runtime_dot_graph())
        .into_iter()
        .filter(|d| {
            matches!(
                d.rule_id.as_str(),
                "reasoning_controls" | "stylesheet_syntax"
            )
        })
        .map(|d| FlowDiagnostic {
            rule_id: d.rule_id,
            message: d.message,
            node_id: d.node_id,
            edge: None,
        })
        .collect();
    if !diagnostics.is_empty() {
        return Err(FlowDefinitionError {
            count: diagnostics.len(),
            detail: diagnostics
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join("; "),
            diagnostics,
        });
    }
    Ok(())
}

pub fn read_named_flow_source(
    flows_dir: impl AsRef<Path>,
    flow_name: &str,
) -> Result<NamedFlowSource, FlowSourceError> {
    let flows_dir = flows_dir.as_ref();
    let path = resolve_flow_path(flows_dir, flow_name)?;
    let flow = read_flow_dir(&path)?;
    let name = flow_name_from_path(flows_dir, &path)?;
    Ok(NamedFlowSource {
        name,
        path,
        content: flow_yaml(&flow)?,
        flow,
    })
}

pub fn flow_yaml(flow: &FlowDefinition) -> Result<String, FlowSourceError> {
    serde_yaml::to_string(flow).map_err(|source| {
        FlowSourceError::new(500, format!("Unable to serialize flow YAML: {source}"))
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
    error.into()
}

impl From<FlowDefinitionError> for FlowSourceError {
    fn from(error: FlowDefinitionError) -> Self {
        Self {
            status_code: 422,
            detail: error.to_string(),
            diagnostics: error.diagnostics,
        }
    }
}
