use std::fs;
use std::path::Path;

use attractor_core::{FlowDefinition, NodeKind};
use attractor_dsl::FlowSourceError;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use spark_common::settings::SparkSettings;
use spark_storage::{
    load_flow_catalog, normalize_execution_lock_value, normalize_launch_policy,
    read_flow_launch_policy, set_flow_catalog_entry, FlowCatalogEntry, FlowExecutionLockConfig,
    ALLOWED_EXECUTION_LOCK_CONFLICT_POLICIES, ALLOWED_EXECUTION_LOCK_SCOPES,
    ALLOWED_LAUNCH_POLICIES, LAUNCH_POLICY_AGENT_REQUESTABLE, LAUNCH_POLICY_DISABLED,
};

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
    pub launch_policy: Option<String>,
    pub effective_launch_policy: String,
    pub execution_lock: Option<FlowExecutionLockConfig>,
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

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFlowLaunchPolicyUpdate {
    pub launch_policy: String,
    #[serde(default)]
    pub execution_lock: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceFlowLaunchPolicyResponse {
    pub name: String,
    pub launch_policy: Option<String>,
    pub effective_launch_policy: String,
    pub execution_lock: Option<FlowExecutionLockConfig>,
    pub allowed_launch_policies: Vec<String>,
    pub allowed_execution_lock_scopes: Vec<String>,
    pub allowed_execution_lock_conflict_policies: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlowSurface {
    Human,
    Agent,
}

#[derive(Debug, Clone)]
pub struct WorkspaceFlowService {
    settings: SparkSettings,
}

impl WorkspaceFlowService {
    pub fn new(settings: SparkSettings) -> Self {
        Self { settings }
    }

    pub fn list_flows(&self, surface: Option<&str>) -> WorkspaceResult<Vec<WorkspaceFlowSummary>> {
        let surface = validate_surface(surface)?;
        let catalog = load_flow_catalog(&self.settings.config_dir)?;
        let names = attractor_api::list_logical_flow_names(&self.settings.flows_dir)
            .map_err(|error| flow_source_error(error, None))?;
        let mut flows = Vec::new();
        for name in names {
            let path = attractor_api::resolve_logical_flow_path(&self.settings.flows_dir, &name)
                .map_err(|error| flow_source_error(error, None))?;
            let entry = catalog.get(&name).cloned().unwrap_or_default();
            let summary = build_flow_summary(&path, &name, entry);
            if surface == FlowSurface::Agent
                && summary.effective_launch_policy != LAUNCH_POLICY_AGENT_REQUESTABLE
            {
                continue;
            }
            flows.push(summary);
        }
        Ok(flows)
    }

    pub fn describe_flow(
        &self,
        flow_name: &str,
        surface: Option<&str>,
    ) -> WorkspaceResult<WorkspaceFlowDescription> {
        let surface = validate_surface(surface)?;
        let source = self.read_existing_flow(flow_name)?;
        let catalog = load_flow_catalog(&self.settings.config_dir)?;
        let entry = catalog.get(&source.name).cloned().unwrap_or_default();
        let summary = build_flow_summary_from_definition(&source.name, &source.flow, entry);
        filter_flow_surface_or_404(&summary, surface)?;
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
        surface: Option<&str>,
    ) -> WorkspaceResult<WorkspaceFlowRaw> {
        let surface = validate_surface(surface)?;
        let policy_state = read_flow_launch_policy(&self.settings.config_dir, flow_name)?;
        if surface == FlowSurface::Agent
            && policy_state.effective_launch_policy != LAUNCH_POLICY_AGENT_REQUESTABLE
        {
            return Err(WorkspaceError::NotFound(format!(
                "Unknown flow: {}",
                policy_state.name
            )));
        }
        let source = self.read_existing_flow(flow_name)?;
        Ok(WorkspaceFlowRaw {
            name: source.name,
            content: source.content,
        })
    }

    pub fn validate_flow(&self, flow_name: &str) -> WorkspaceResult<Value> {
        let path = attractor_api::resolve_logical_flow_path(&self.settings.flows_dir, flow_name)
            .map_err(|error| flow_source_error(error, Some(flow_name)))?;
        if !path.exists() {
            return Err(WorkspaceError::NotFound(format!(
                "Unknown flow: {flow_name}"
            )));
        }
        let content = fs::read_to_string(&path).map_err(|error| {
            WorkspaceError::Internal(format!(
                "Unable to read flow file {}: {error}",
                path.display()
            ))
        })?;
        let name = attractor_dsl::flow_name_from_path(&self.settings.flows_dir, &path)
            .map_err(|error| flow_source_error(error, Some(flow_name)))?;
        let preview =
            attractor_api::preview_named_flow_source(&self.settings.flows_dir, &name, &content);
        let path = path.canonicalize().unwrap_or(path);
        let mut payload = Map::new();
        payload.insert("name".to_string(), Value::String(name));
        payload.insert(
            "path".to_string(),
            Value::String(path.to_string_lossy().into_owned()),
        );
        if let Value::Object(preview_payload) = preview.body {
            payload.extend(preview_payload);
        }
        Ok(Value::Object(payload))
    }

    pub fn update_launch_policy(
        &self,
        flow_name: &str,
        request: WorkspaceFlowLaunchPolicyUpdate,
    ) -> WorkspaceResult<WorkspaceFlowLaunchPolicyResponse> {
        let normalized_launch_policy = normalize_launch_policy(&request.launch_policy)?;
        let execution_lock = request
            .execution_lock
            .as_ref()
            .map(normalize_execution_lock_value)
            .transpose()?;
        self.ensure_flow_exists(flow_name)?;
        let policy_state = set_flow_catalog_entry(
            &self.settings.config_dir,
            flow_name,
            &normalized_launch_policy,
            execution_lock,
        )?;
        Ok(WorkspaceFlowLaunchPolicyResponse {
            name: policy_state.name,
            launch_policy: policy_state.launch_policy,
            effective_launch_policy: policy_state.effective_launch_policy,
            execution_lock: policy_state.execution_lock,
            allowed_launch_policies: ALLOWED_LAUNCH_POLICIES
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            allowed_execution_lock_scopes: ALLOWED_EXECUTION_LOCK_SCOPES
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            allowed_execution_lock_conflict_policies: ALLOWED_EXECUTION_LOCK_CONFLICT_POLICIES
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
        })
    }

    fn read_existing_flow(
        &self,
        flow_name: &str,
    ) -> WorkspaceResult<attractor_api::NamedFlowSource> {
        match attractor_api::read_named_flow_source(&self.settings.flows_dir, flow_name) {
            Ok(source) => Ok(source),
            Err(error) if error.status_code() == 404 => Err(WorkspaceError::NotFound(format!(
                "Unknown flow: {flow_name}"
            ))),
            Err(error) => Err(flow_source_error(error, Some(flow_name))),
        }
    }

    pub fn ensure_flow_exists(&self, flow_name: &str) -> WorkspaceResult<()> {
        let path = attractor_api::resolve_logical_flow_path(&self.settings.flows_dir, flow_name)
            .map_err(|error| flow_source_error(error, Some(flow_name)))?;
        if path.exists() {
            Ok(())
        } else {
            Err(WorkspaceError::NotFound(format!(
                "Unknown flow: {flow_name}"
            )))
        }
    }
}

fn validate_surface(surface: Option<&str>) -> WorkspaceResult<FlowSurface> {
    let surface = surface.unwrap_or("human").trim().to_ascii_lowercase();
    match surface.as_str() {
        "human" => Ok(FlowSurface::Human),
        "agent" => Ok(FlowSurface::Agent),
        _ => Err(WorkspaceError::Validation(
            "Flow surface must be 'human' or 'agent'.".to_string(),
        )),
    }
}

fn filter_flow_surface_or_404(
    flow: &WorkspaceFlowSummary,
    surface: FlowSurface,
) -> WorkspaceResult<()> {
    if surface == FlowSurface::Agent
        && flow.effective_launch_policy != LAUNCH_POLICY_AGENT_REQUESTABLE
    {
        return Err(WorkspaceError::NotFound(format!(
            "Unknown flow: {}",
            flow.name
        )));
    }
    Ok(())
}

fn build_flow_summary(
    flow_path: &Path,
    flow_name: &str,
    entry: FlowCatalogEntry,
) -> WorkspaceFlowSummary {
    let fallback_title = flow_stem(flow_name);
    let Ok(content) = fs::read_to_string(flow_path) else {
        return fallback_summary(flow_name, fallback_title, entry);
    };
    match attractor_dsl::parse_flow_definition(&content) {
        Ok(flow) => build_flow_summary_from_definition(flow_name, &flow, entry),
        Err(_) => fallback_summary(flow_name, fallback_title, entry),
    }
}

fn build_flow_summary_from_definition(
    flow_name: &str,
    flow: &FlowDefinition,
    entry: FlowCatalogEntry,
) -> WorkspaceFlowSummary {
    let title = first_non_empty(&[flow.title.as_str()]).unwrap_or_else(|| flow_stem(flow_name));
    let description =
        first_non_empty(&[flow.description.as_str(), flow.goal.as_str()]).unwrap_or_default();
    WorkspaceFlowSummary {
        name: flow_name.to_string(),
        title,
        description,
        launch_policy: entry.launch_policy.clone(),
        effective_launch_policy: entry
            .launch_policy
            .unwrap_or_else(|| LAUNCH_POLICY_DISABLED.to_string()),
        execution_lock: entry.execution_lock,
        graph_label: flow.title.clone(),
        graph_goal: flow.goal.clone(),
    }
}

fn fallback_summary(
    flow_name: &str,
    title: String,
    entry: FlowCatalogEntry,
) -> WorkspaceFlowSummary {
    WorkspaceFlowSummary {
        name: flow_name.to_string(),
        title,
        description: String::new(),
        launch_policy: entry.launch_policy.clone(),
        effective_launch_policy: entry
            .launch_policy
            .unwrap_or_else(|| LAUNCH_POLICY_DISABLED.to_string()),
        execution_lock: entry.execution_lock,
        graph_label: String::new(),
        graph_goal: String::new(),
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
