use std::path::Path;

use attractor_core::{FlowDefinition, FlowExecutionLock, NodeKind};
use attractor_dsl::{FlowRoots, FlowSourceError};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use spark_common::settings::SparkSettings;

use crate::errors::{WorkspaceError, WorkspaceResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceFlowFeatures {
    pub has_human_gate: bool,
    pub has_manager_loop: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceFlowSummary {
    pub name: String,
    pub title: String,
    pub description: String,
    pub execution_lock: Option<FlowExecutionLock>,
    pub graph_label: String,
    pub graph_goal: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceFlowDescription {
    #[serde(flatten)]
    pub summary: WorkspaceFlowSummary,
    pub node_count: usize,
    pub edge_count: usize,
    pub features: WorkspaceFlowFeatures,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceFlowRaw {
    pub name: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct WorkspaceFlowService {
    settings: SparkSettings,
}

impl WorkspaceFlowService {
    pub fn new(settings: SparkSettings) -> Self {
        Self { settings }
    }

    fn roots(&self, project_path: Option<&str>) -> FlowRoots {
        attractor_api::flow_roots(&self.settings, project_path)
    }

    pub fn list_flows(
        &self,
        project_path: Option<&str>,
    ) -> WorkspaceResult<Vec<WorkspaceFlowSummary>> {
        let roots = self.roots(project_path);
        let names = roots
            .list_flow_names()
            .map_err(|error| flow_source_error(error, None))?;
        let mut flows = Vec::new();
        for name in names {
            let resolved = roots
                .resolve_flow(&name)
                .map_err(|error| flow_source_error(error, None))?;
            flows.push(build_flow_summary(&resolved.path, &name));
        }
        Ok(flows)
    }

    pub fn describe_flow(
        &self,
        flow_name: &str,
        project_path: Option<&str>,
    ) -> WorkspaceResult<WorkspaceFlowDescription> {
        let source = self.read_existing_flow(flow_name, project_path)?;
        let summary = build_flow_summary_from_definition(&source.name, &source.flow);
        Ok(WorkspaceFlowDescription {
            node_count: source.flow.nodes.len(),
            edge_count: source.flow.edges.len(),
            features: WorkspaceFlowFeatures {
                has_human_gate: source
                    .flow
                    .nodes
                    .values()
                    .any(|node| node.kind == NodeKind::HumanGate),
                has_manager_loop: source
                    .flow
                    .nodes
                    .values()
                    .any(|node| node.kind == NodeKind::Subflow),
            },
            summary,
        })
    }

    pub fn raw_flow(
        &self,
        flow_name: &str,
        project_path: Option<&str>,
    ) -> WorkspaceResult<WorkspaceFlowRaw> {
        let source = self.read_existing_flow(flow_name, project_path)?;
        Ok(WorkspaceFlowRaw {
            name: source.name,
            content: source.content,
        })
    }

    pub fn validate_flow(
        &self,
        flow_name: &str,
        project_path: Option<&str>,
    ) -> WorkspaceResult<Value> {
        let roots = self.roots(project_path);
        let resolved = match roots.resolve_flow(flow_name) {
            Ok(resolved) => resolved,
            Err(error) => return Err(flow_source_error(error, Some(flow_name))),
        };
        // Referenced roles must resolve; the preview keeps the authored form.
        let preview = match attractor_dsl::read_flow_dir(&resolved.path).and_then(|flow| {
            attractor_dsl::resolve_flow_roles(&flow, &roots)?;
            attractor_dsl::flow_yaml(&flow)
        }) {
            Ok(content) => {
                attractor_api::preview_named_flow_source(&roots, &resolved.name, &content)
            }
            Err(error) => attractor_api::flow_source_error_preview(&error),
        };
        let path = resolved.path.canonicalize().unwrap_or(resolved.path);
        let mut payload = Map::new();
        payload.insert("name".to_string(), Value::String(resolved.name));
        payload.insert(
            "path".to_string(),
            Value::String(path.to_string_lossy().into_owned()),
        );
        if let Value::Object(preview_payload) = preview.body {
            payload.extend(preview_payload);
        }
        Ok(Value::Object(payload))
    }

    pub fn ensure_flow_exists(
        &self,
        flow_name: &str,
        project_path: Option<&str>,
    ) -> WorkspaceResult<()> {
        self.roots(project_path)
            .resolve_flow(flow_name)
            .map(|_| ())
            .map_err(|error| flow_source_error(error, Some(flow_name)))
    }

    fn read_existing_flow(
        &self,
        flow_name: &str,
        project_path: Option<&str>,
    ) -> WorkspaceResult<attractor_dsl::NamedFlowSource> {
        self.roots(project_path)
            .read_named_flow_source(flow_name)
            .map_err(|error| flow_source_error(error, Some(flow_name)))
    }
}

fn build_flow_summary(flow_path: &Path, flow_name: &str) -> WorkspaceFlowSummary {
    match attractor_dsl::read_flow_dir(flow_path) {
        Ok(flow) => build_flow_summary_from_definition(flow_name, &flow),
        Err(_) => WorkspaceFlowSummary {
            name: flow_name.to_string(),
            title: flow_stem(flow_name),
            description: String::new(),
            execution_lock: None,
            graph_label: String::new(),
            graph_goal: String::new(),
        },
    }
}

fn build_flow_summary_from_definition(
    flow_name: &str,
    flow: &FlowDefinition,
) -> WorkspaceFlowSummary {
    let title = first_non_empty(&[flow.title.as_str()]).unwrap_or_else(|| flow_stem(flow_name));
    let description =
        first_non_empty(&[flow.description.as_str(), flow.goal.as_str()]).unwrap_or_default();
    WorkspaceFlowSummary {
        name: flow_name.to_string(),
        title,
        description,
        execution_lock: flow.execution_lock.clone(),
        graph_label: flow.title.clone(),
        graph_goal: flow.goal.clone(),
    }
}

fn first_non_empty(values: &[&str]) -> Option<String> {
    values
        .iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .map(str::to_string)
}

fn flow_stem(flow_name: &str) -> String {
    Path::new(flow_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| flow_name.to_string())
}

fn flow_source_error(error: FlowSourceError, missing_name: Option<&str>) -> WorkspaceError {
    match error.status_code() {
        400 => WorkspaceError::Validation(error.detail().to_string()),
        404 => WorkspaceError::NotFound(format!(
            "Unknown flow: {}",
            missing_name.unwrap_or_else(|| error.detail())
        )),
        403 => WorkspaceError::Forbidden(error.detail().to_string()),
        503 => WorkspaceError::ServiceUnavailable(error.detail().to_string()),
        _ => WorkspaceError::Internal(error.detail().to_string()),
    }
}
