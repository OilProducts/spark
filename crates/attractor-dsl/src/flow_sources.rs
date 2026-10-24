use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use attractor_core::{normalize_flow_ref, FlowDefinition, FlowDefinitionError, FlowDiagnostic};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::flow_files::{read_flow_dir, FLOW_FILE};

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

/// The ordered roots flows and roles resolve through: the project's org repo
/// (`<project>/.spark/agent-org`), then the bundled library. Each root holds
/// `flows/` and `roles/`; a project file replaces a bundled one of the same
/// name. Reads never create anything, and a missing org means bundled only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowRoots {
    pub org: Option<PathBuf>,
    pub bundled: PathBuf,
}

/// Where a resolved flow directory sits: `layer` is `org` or `bundled`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFlowDir {
    pub layer: &'static str,
    pub name: String,
    pub path: PathBuf,
}

/// A run's `artifacts/flow/files.json`: the SHA-256 of every file the flow
/// used (`org/...` or `bundled/...`), the org repo state, and Spark's version.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowManifest {
    pub files: BTreeMap<String, String>,
    pub org: Option<OrgState>,
    pub spark_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgState {
    pub commit: Option<String>,
    pub dirty: bool,
}

pub const ORG_LAYER: &str = "org";
pub const BUNDLED_LAYER: &str = "bundled";

impl FlowRoots {
    /// The roots for `project` (none: bundled only) over `bundled`.
    pub fn new(bundled: impl Into<PathBuf>, project: Option<&Path>) -> Self {
        Self {
            org: project.map(spark_common::project::org_dir),
            bundled: bundled.into(),
        }
    }

    fn layers(&self) -> impl Iterator<Item = (&'static str, &Path)> {
        self.org
            .as_deref()
            .map(|org| (ORG_LAYER, org))
            .into_iter()
            .chain(std::iter::once((BUNDLED_LAYER, self.bundled.as_path())))
    }

    /// `<org>/flows/<name>`, where saves write; None without a project.
    pub fn org_flow_path(&self, flow_name: &str) -> Result<Option<PathBuf>, FlowSourceError> {
        let name = normalize_flow_name(flow_name)?;
        Ok(self
            .org
            .as_deref()
            .map(|org| join_name(&org.join("flows"), &name)))
    }

    /// The first flow directory named `flow_name`, if any root has one.
    pub fn find_flow(&self, flow_name: &str) -> Result<Option<ResolvedFlowDir>, FlowSourceError> {
        let name = normalize_flow_name(flow_name)?;
        Ok(self.layers().find_map(|(layer, root)| {
            let path = join_name(&root.join("flows"), &name);
            path.join(FLOW_FILE).is_file().then(|| ResolvedFlowDir {
                layer,
                name: name.clone(),
                path,
            })
        }))
    }

    /// Like [`Self::find_flow`], but a missing flow is a 404.
    pub fn resolve_flow(&self, flow_name: &str) -> Result<ResolvedFlowDir, FlowSourceError> {
        self.find_flow(flow_name)?.ok_or_else(|| {
            FlowSourceError::new(404, format!("Flow not found: {}", flow_name.trim()))
        })
    }

    /// Names of every flow in any root, each once, sorted.
    pub fn list_flow_names(&self) -> Result<Vec<String>, FlowSourceError> {
        let mut names = BTreeSet::new();
        for (_, root) in self.layers() {
            names.extend(list_flow_names(&root.join("flows"))?);
        }
        Ok(names.into_iter().collect())
    }

    /// The named flow in its authored form (roles referenced, not merged).
    /// The named flow in its authored form (roles referenced, not merged).
    pub fn read_named_flow_source(
        &self,
        flow_name: &str,
    ) -> Result<NamedFlowSource, FlowSourceError> {
        let resolved = self.resolve_flow(flow_name)?;
        let flow = read_flow_dir(&resolved.path)?;
        Ok(NamedFlowSource {
            name: resolved.name,
            path: resolved.path,
            content: flow_yaml(&flow)?,
            flow,
        })
    }

    /// The first role file named `name`: its manifest key and text.
    pub fn role_text(&self, name: &str) -> Result<(String, String), String> {
        let file = format!("{name}.md");
        let unknown =
            || format!("unknown role '{name}' (no roles/{file} in the project or bundled library)");
        if name.is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
            return Err(unknown());
        }
        self.layers()
            .find_map(|(layer, root)| {
                fs::read_to_string(root.join("roles").join(&file))
                    .ok()
                    .map(|text| (format!("{layer}/roles/{file}"), text))
            })
            .ok_or_else(unknown)
    }

    /// The org repo's HEAD and whether it has uncommitted changes; None when
    /// there is no org repo.
    pub fn org_state(&self) -> Option<OrgState> {
        let org = self
            .org
            .as_deref()
            .filter(|org| org.join(".git").exists())?;
        let run_git = spark_common::project::run_git;
        Some(OrgState {
            commit: run_git(org, &["rev-parse", "HEAD"]).ok(),
            dirty: run_git(org, &["status", "--porcelain"]).is_ok_and(|text| !text.is_empty()),
        })
    }
}

/// The flow name a subflow's `flow_ref` names: relative to the directory
/// holding the parent flow (`parent_flow_name`). The name then resolves
/// through the roots, so a project copy of a bundled child wins. None for a
/// ref that is absolute or climbs out.
pub fn flow_ref_name(parent_flow_name: Option<&str>, flow_ref: &str) -> Option<String> {
    let flow_ref = normalize_flow_ref(flow_ref)?;
    let parent_dir = parent_flow_name
        .and_then(|name| normalize_flow_name(name).ok())
        .and_then(|name| name.rsplit_once('/').map(|(dir, _)| dir.to_string()));
    normalize_flow_name(&match parent_dir {
        Some(dir) => format!("{dir}/{flow_ref}"),
        None => flow_ref,
    })
    .ok()
}

fn join_name(root: &Path, name: &str) -> PathBuf {
    name.split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

fn flow_name_from_path(flows_dir: &Path, flow_path: &Path) -> Option<String> {
    let parts = flow_path
        .strip_prefix(flows_dir)
        .ok()?
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Names of every flow directory (one containing `flow.md`) under
/// `flows_dir`; a missing directory has none.
fn list_flow_names(flows_dir: &Path) -> Result<Vec<String>, FlowSourceError> {
    fn collect(dir: &Path, paths: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in fs::read_dir(dir)? {
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
    if !flows_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    collect(flows_dir, &mut paths).map_err(|source| {
        FlowSourceError::new(
            500,
            format!(
                "Unable to list flows directory {}: {source}",
                flows_dir.display()
            ),
        )
    })?;
    Ok(paths
        .iter()
        .filter_map(|path| flow_name_from_path(flows_dir, path))
        .collect())
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

fn path_safety_error() -> FlowSourceError {
    FlowSourceError::new(
        400,
        "Flow name must be a relative path inside the flows directory.",
    )
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
