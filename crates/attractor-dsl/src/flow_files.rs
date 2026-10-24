//! Per-node flow directories in the Open Knowledge Format (OKF) v0.2 layout.
//!
//! A flow is a directory holding `flow.md` (`type: Flow`, body = goal) and one
//! `<node_id>.md` per node (`type: <node kind>`, frontmatter = config and the
//! node's outgoing `routes`, body = prompt or command). Roles live in
//! `roles/<name>.md` (`type: Role`, body = persona) and are merged into
//! `agent_task` nodes only by [`resolve_flow_roles`].

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::RwLock;

use attractor_core::{
    ExecutionConfig, FlowDefaults, FlowDefinition, FlowDefinitionError, FlowDiagnostic, FlowEdge,
    FlowExecutionLock, FlowInput, FlowMetadata, FlowNode, ManagerLoopConfig, NodeConfig,
    NodeContracts, NodeKind, NodeRuntimeConfig, RetryConfig, UiConfig,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::flow_sources::{validate_flow_definition, FlowManifest, FlowRoots, FlowSourceError};

pub const FLOW_FILE: &str = "flow.md";
/// OKF-reserved names the loader ignores.
const RESERVED_FILES: [&str; 2] = ["index.md", "log.md"];

// ponytail: one process-wide lock keeps loads from seeing a half-applied save;
// per-directory locks if save throughput ever matters.
static FLOW_DIR_LOCK: RwLock<()> = RwLock::new(());

#[derive(Debug, Serialize, Deserialize)]
struct FlowFile {
    #[serde(rename = "type")]
    file_type: String,
    id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    execution_lock: Option<FlowExecutionLock>,
    #[serde(default = "default_schema_version")]
    schema_version: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    inputs: Vec<FlowInput>,
    #[serde(default, skip_serializing_if = "FlowDefaults::is_empty")]
    defaults: FlowDefaults,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    metadata: FlowMetadata,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    extensions: BTreeMap<String, Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct NodeFile {
    #[serde(rename = "type")]
    kind: NodeKind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    config: serde_json::Map<String, Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    context: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runtime: Option<NodeRuntimeConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    contracts: Option<NodeContracts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    manager: Option<ManagerLoopConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    retry: Option<RetryConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    execution: Option<ExecutionConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ui: Option<UiConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    extensions: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    routes: Vec<Route>,
}

/// A `FlowEdge` without `from`; fields at their default are omitted.
#[derive(Debug, Serialize, Deserialize)]
struct Route {
    to: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    condition: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    weight: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transition: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    extensions: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
struct RoleFile {
    #[serde(rename = "type")]
    file_type: String,
    #[serde(default)]
    execution: Option<ExecutionConfig>,
}

/// Loads a flow directory in its authored form (roles unresolved), validated
/// and normalized.
pub fn read_flow_dir(dir: &Path) -> Result<FlowDefinition, FlowSourceError> {
    assemble_flow_files(&read_flow_dir_texts(dir)?)
}

/// The `flow.md` and node files of a flow directory (file name -> text), read
/// as one consistent snapshot: never a mix of files before and after a save.
pub fn read_flow_dir_texts(dir: &Path) -> Result<BTreeMap<String, String>, FlowSourceError> {
    let _guard = FLOW_DIR_LOCK
        .read()
        .unwrap_or_else(|error| error.into_inner());
    if !dir.join(FLOW_FILE).is_file() {
        return Err(FlowSourceError::new(404, "Flow not found."));
    }
    let mut files = BTreeMap::new();
    let mut unknown = Vec::new();
    for entry in fs::read_dir(dir).map_err(|error| read_error(dir, error))? {
        let entry = entry.map_err(|error| read_error(dir, error))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if RESERVED_FILES.contains(&name.as_str()) {
            continue;
        }
        if !name.ends_with(".md") || entry.path().is_dir() {
            unknown.push(name);
            continue;
        }
        let text = fs::read_to_string(entry.path()).map_err(|error| read_error(dir, error))?;
        files.insert(name, text);
    }
    if !unknown.is_empty() {
        unknown.sort();
        return Err(FlowSourceError::new(
            422,
            format!(
                "Flow directory {} contains files that are not flow or node files: {}",
                dir.display(),
                unknown.join(", ")
            ),
        ));
    }
    Ok(files)
}

/// Assembles `flow.md` and node files (file name -> text) into a validated,
/// normalized flow.
pub fn assemble_flow_files(
    files: &BTreeMap<String, String>,
) -> Result<FlowDefinition, FlowSourceError> {
    let header = files
        .get(FLOW_FILE)
        .ok_or_else(|| FlowSourceError::new(404, "Flow not found."))?;
    let mut flow = parse_flow_file(header)?;
    for (name, text) in files {
        if name == FLOW_FILE || RESERVED_FILES.contains(&name.as_str()) {
            continue;
        }
        let node_id = name.strip_suffix(".md").unwrap_or(name).to_string();
        let (node, edges) = parse_node_file(&node_id, text)?;
        flow.edges.extend(edges);
        flow.nodes.insert(node_id, node);
    }
    let flow = flow.normalize();
    validate_flow_definition(&flow).map_err(FlowSourceError::from)?;
    Ok(flow)
}

/// Renders a flow as `flow.md` plus one file per node (file name -> text).
pub fn render_flow_files(
    flow: &FlowDefinition,
) -> Result<BTreeMap<String, String>, FlowSourceError> {
    let flow = flow.normalize();
    let mut files = BTreeMap::new();
    files.insert(FLOW_FILE.to_string(), render_flow_file(&flow)?);
    for (node_id, node) in &flow.nodes {
        let file_name = format!("{node_id}.md");
        if file_name.eq_ignore_ascii_case(FLOW_FILE)
            || RESERVED_FILES
                .iter()
                .any(|reserved| file_name.eq_ignore_ascii_case(reserved))
        {
            return Err(FlowSourceError::new(
                422,
                format!("Node id '{node_id}' is reserved in flow directories; rename the node."),
            ));
        }
        let edges = flow
            .edges
            .iter()
            .filter(|edge| edge.from == *node_id)
            .cloned()
            .collect::<Vec<_>>();
        files.insert(file_name, render_node_file(node, edges)?);
    }
    Ok(files)
}

/// Writes a flow into `dir`, rewriting only files whose parsed content
/// changed (untouched files stay byte-identical), each atomically, and
/// deleting the files of removed nodes. Returns the names of the files it
/// wrote or deleted.
pub fn write_flow_dir(dir: &Path, flow: &FlowDefinition) -> Result<Vec<String>, FlowSourceError> {
    let flow = flow.normalize();
    let _guard = FLOW_DIR_LOCK
        .write()
        .unwrap_or_else(|error| error.into_inner());
    let files = render_flow_files(&flow)?;
    let mut changed = files
        .iter()
        .filter(|(name, text)| {
            fs::read_to_string(dir.join(name))
                .ok()
                .and_then(|existing| rerender_file(name, &existing).ok())
                .is_none_or(|existing| existing != **text)
        })
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    fs::create_dir_all(dir).map_err(|error| write_error(dir, error))?;
    let temp_dir = dir.parent().unwrap_or(dir);
    let dir_name = dir
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    for name in &changed {
        let path = dir.join(name);
        // The temp file sits beside the flow directory, never inside it, so
        // readers see no extra entries.
        let temp = temp_dir.join(format!(".{dir_name}.{name}.{}.tmp", std::process::id()));
        fs::write(&temp, &files[name]).map_err(|error| write_error(&temp, error))?;
        fs::rename(&temp, &path).map_err(|error| write_error(&path, error))?;
    }
    for entry in fs::read_dir(dir).map_err(|error| read_error(dir, error))? {
        let entry = entry.map_err(|error| read_error(dir, error))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".md")
            && !files.contains_key(&name)
            && !RESERVED_FILES.contains(&name.as_str())
            && entry.path().is_file()
        {
            fs::remove_file(entry.path()).map_err(|error| write_error(&entry.path(), error))?;
            changed.push(name);
        }
    }
    Ok(changed)
}

/// A flow loaded for a run: the resolved form runs execute, and the manifest
/// of every file it used.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedFlow {
    pub name: String,
    pub flow: FlowDefinition,
    pub manifest: FlowManifest,
}

/// Loads the named flow through `roots` for a run: assembles its directory,
/// resolves its roles, and hashes every file it used (`flow.md`, node files,
/// role files).
pub fn load_flow(roots: &FlowRoots, flow_name: &str) -> Result<LoadedFlow, FlowSourceError> {
    let resolved = roots.resolve_flow(flow_name)?;
    let texts = read_flow_dir_texts(&resolved.path)?;
    let authored = assemble_flow_files(&texts)?;
    let mut files = texts
        .iter()
        .map(|(file, text)| {
            (
                format!("{}/flows/{}/{file}", resolved.layer, resolved.name),
                spark_common::project::sha256_hex(text.as_bytes()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let flow = resolve_flow_roles_with(&authored, |name| {
        roots.role_text(name).map(|(key, text)| {
            files.insert(key, spark_common::project::sha256_hex(text.as_bytes()));
            text
        })
    })?;
    Ok(LoadedFlow {
        name: resolved.name,
        flow,
        manifest: FlowManifest {
            files,
            org: roots.org_state(),
            spark_version: env!("CARGO_PKG_VERSION").to_string(),
        },
    })
}

/// Resolves roles into the executable form: each role's persona is
/// prepended to the node prompt (followed by a blank line) and its
/// `execution` is merged under the node's own, field by field.
pub fn resolve_flow_roles(
    flow: &FlowDefinition,
    roots: &FlowRoots,
) -> Result<FlowDefinition, FlowSourceError> {
    resolve_flow_roles_with(flow, |name| roots.role_text(name).map(|(_, text)| text))
}

/// [`resolve_flow_roles`] with role file texts supplied by `role_text`.
pub(crate) fn resolve_flow_roles_with(
    flow: &FlowDefinition,
    mut role_text: impl FnMut(&str) -> Result<String, String>,
) -> Result<FlowDefinition, FlowSourceError> {
    let mut resolved = flow.clone();
    let mut errors = Vec::new();
    for (node_id, node) in resolved.nodes.iter_mut() {
        let Some(role) = node.role.take() else {
            continue;
        };
        let (persona, execution) = match role_text(&role).and_then(|text| parse_role(&role, &text))
        {
            Ok(role) => role,
            Err(message) => {
                errors.push(FlowDiagnostic {
                    rule_id: "role".to_string(),
                    message: format!("node '{node_id}': {message}"),
                    node_id: Some(node_id.clone()),
                    edge: None,
                });
                continue;
            }
        };
        if let Some(NodeConfig::AgentTask { prompt }) = node.config.as_mut() {
            *prompt = format!("{persona}\n\n{prompt}");
        }
        node.execution = merge_execution(node.execution.take(), execution);
    }
    // Role settings merged into a node must pass the same checks as node settings.
    if errors.is_empty() {
        let authored = flow.diagnostics();
        errors.extend(
            resolved
                .diagnostics()
                .into_iter()
                .filter(|diagnostic| !authored.contains(diagnostic)),
        );
    }
    if !errors.is_empty() {
        return Err(FlowDefinitionError {
            count: errors.len(),
            detail: errors
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join("; "),
            diagnostics: errors,
        }
        .into());
    }
    Ok(resolved.normalize())
}

/// Parses a role file into its persona and execution defaults.
pub(crate) fn parse_role(
    name: &str,
    text: &str,
) -> Result<(String, Option<ExecutionConfig>), String> {
    let (frontmatter, body) =
        split_frontmatter(text).map_err(|error| format!("{name}.md: {error}"))?;
    let role: RoleFile = serde_yaml::from_str(frontmatter)
        .map_err(|error| format!("role file {name}.md: {error}"))?;
    if role.file_type != "Role" {
        return Err(format!("role file {name}.md must declare type: Role"));
    }
    Ok((body.trim().to_string(), role.execution))
}

fn merge_execution(
    node: Option<ExecutionConfig>,
    role: Option<ExecutionConfig>,
) -> Option<ExecutionConfig> {
    let (node, role) = match (node, role) {
        (node, None) => return node,
        (None, role) => return role,
        (Some(node), Some(role)) => (node, role),
    };
    Some(ExecutionConfig {
        llm_provider: node.llm_provider.or(role.llm_provider),
        llm_profile: node.llm_profile.or(role.llm_profile),
        llm_model: node.llm_model.or(role.llm_model),
        reasoning_effort: node.reasoning_effort.or(role.reasoning_effort),
        thinking: node.thinking.or(role.thinking),
        thinking_budget_tokens: node.thinking_budget_tokens.or(role.thinking_budget_tokens),
        reasoning_mode: node.reasoning_mode.or(role.reasoning_mode),
        reasoning_summary: node.reasoning_summary.or(role.reasoning_summary),
    })
}

fn rerender_file(name: &str, text: &str) -> Result<String, FlowSourceError> {
    if name == FLOW_FILE {
        return render_flow_file(&parse_flow_file(text)?);
    }
    let node_id = name.strip_suffix(".md").unwrap_or(name);
    let (node, edges) = parse_node_file(node_id, text)?;
    render_node_file(&node, edges)
}

fn parse_flow_file(text: &str) -> Result<FlowDefinition, FlowSourceError> {
    let (frontmatter, body) =
        split_frontmatter(text).map_err(|error| file_error(FLOW_FILE, error))?;
    let file: FlowFile =
        serde_yaml::from_str(frontmatter).map_err(|error| file_error(FLOW_FILE, error))?;
    if file.file_type != "Flow" {
        return Err(file_error(FLOW_FILE, "must declare type: Flow"));
    }
    Ok(FlowDefinition {
        schema_version: file.schema_version,
        id: file.id,
        title: file.title,
        description: file.description,
        goal: body.trim().to_string(),
        inputs: file.inputs,
        defaults: file.defaults,
        nodes: BTreeMap::new(),
        edges: Vec::new(),
        metadata: file.metadata,
        extensions: file.extensions,
        execution_lock: file.execution_lock,
    })
}

fn render_flow_file(flow: &FlowDefinition) -> Result<String, FlowSourceError> {
    let file = FlowFile {
        file_type: "Flow".to_string(),
        id: flow.id.clone(),
        title: flow.title.clone(),
        description: flow.description.clone(),
        execution_lock: flow.execution_lock.clone(),
        schema_version: flow.schema_version.clone(),
        inputs: flow.inputs.clone(),
        defaults: flow.defaults.clone(),
        metadata: flow.metadata.clone(),
        extensions: flow.extensions.clone(),
    };
    render_file(FLOW_FILE, &file, flow.goal.trim())
}

fn parse_node_file(
    node_id: &str,
    text: &str,
) -> Result<(FlowNode, Vec<FlowEdge>), FlowSourceError> {
    let file_name = format!("{node_id}.md");
    let (frontmatter, body) =
        split_frontmatter(text).map_err(|error| file_error(&file_name, error))?;
    let file: NodeFile =
        serde_yaml::from_str(frontmatter).map_err(|error| file_error(&file_name, error))?;
    let body = body.trim();
    let mut config = file.config;
    let kind_value = serde_json::to_value(&file.kind).unwrap_or(Value::Null);
    if config.contains_key("kind") {
        return Err(file_error(
            &file_name,
            "config must not set kind; the node's type sets it",
        ));
    }
    if let Some(key) = body_key(&file.kind) {
        if config.contains_key(key) {
            return Err(file_error(
                &file_name,
                format!("config.{key} belongs in the file body"),
            ));
        }
        let value = match file.kind {
            NodeKind::Tool => Value::String(
                parse_command_block(body).map_err(|error| file_error(&file_name, error))?,
            ),
            NodeKind::Exit if body.is_empty() => Value::Null,
            _ => Value::String(body.to_string()),
        };
        if !value.is_null() {
            config.insert(key.to_string(), value);
        }
    } else if !body.is_empty() {
        return Err(file_error(
            &file_name,
            format!("{kind_value} nodes take no body; remove the text after the frontmatter"),
        ));
    }
    config.insert("kind".to_string(), kind_value);
    let config: NodeConfig = serde_json::from_value(Value::Object(config))
        .map_err(|error| file_error(&file_name, format!("invalid config: {error}")))?;
    let edges = file
        .routes
        .into_iter()
        .map(|route| FlowEdge {
            from: node_id.to_string(),
            to: route.to,
            label: route.label,
            condition: route.condition,
            weight: route.weight,
            transition: route.transition,
            extensions: route.extensions,
        })
        .collect();
    let node = FlowNode {
        kind: file.kind,
        label: file.title,
        description: file.description,
        role: file.role,
        config: Some(config),
        context: file.context,
        runtime: file.runtime,
        contracts: file.contracts,
        manager: file.manager,
        retry: file.retry,
        execution: file.execution,
        ui: file.ui,
        extensions: file.extensions,
    };
    Ok((node, edges))
}

fn render_node_file(node: &FlowNode, mut edges: Vec<FlowEdge>) -> Result<String, FlowSourceError> {
    // Same order as FlowDefinition::normalize, so file order is canonical.
    edges.sort_by(|left, right| {
        left.weight
            .cmp(&right.weight)
            .then_with(|| left.to.cmp(&right.to))
            .then_with(|| left.label.cmp(&right.label))
    });
    let mut config = match serde_json::to_value(&node.config) {
        Ok(Value::Object(config)) => config,
        _ => serde_json::Map::new(),
    };
    config.remove("kind");
    let body = body_key(&node.kind)
        .and_then(|key| config.remove(key))
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default();
    let body = match node.kind {
        NodeKind::Tool => render_command_block(&body),
        _ => body.trim().to_string(),
    };
    let file = NodeFile {
        kind: node.kind.clone(),
        title: node.label.clone(),
        description: node.description.clone(),
        role: node.role.clone(),
        config,
        context: node.context.clone(),
        runtime: node.runtime.clone(),
        contracts: node.contracts.clone(),
        manager: node.manager.clone(),
        retry: node.retry.clone(),
        execution: node.execution.clone(),
        ui: node.ui.clone(),
        extensions: node.extensions.clone(),
        routes: edges
            .into_iter()
            .map(|edge| Route {
                to: edge.to,
                label: edge.label,
                condition: edge.condition,
                weight: edge.weight,
                transition: edge.transition,
                extensions: edge.extensions,
            })
            .collect(),
    };
    render_file("node", &file, &body)
}

/// The config field a node kind keeps in its markdown body.
fn body_key(kind: &NodeKind) -> Option<&'static str> {
    match kind {
        NodeKind::AgentTask | NodeKind::HumanGate => Some("prompt"),
        NodeKind::Exit => Some("result_summary_prompt"),
        NodeKind::Tool => Some("command"),
        _ => None,
    }
}

fn render_file(
    file_name: &str,
    frontmatter: &impl Serialize,
    body: &str,
) -> Result<String, FlowSourceError> {
    let yaml = serde_yaml::to_string(frontmatter).map_err(|error| file_error(file_name, error))?;
    let mut text = format!("---\n{yaml}---\n");
    if !body.is_empty() {
        text.push('\n');
        text.push_str(body);
        text.push('\n');
    }
    Ok(text)
}

/// Splits `---` YAML frontmatter from the markdown body.
pub(crate) fn split_frontmatter(text: &str) -> Result<(&str, &str), String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
        .ok_or("must start with a --- YAML frontmatter block")?;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return Ok((&rest[..offset], &rest[offset + line.len()..]));
        }
        offset += line.len();
    }
    Err("frontmatter is missing its closing ---".to_string())
}

/// A tool body is exactly one fenced code block; its contents are the command.
fn parse_command_block(body: &str) -> Result<String, String> {
    const MESSAGE: &str = "a tool body must be exactly one fenced code block holding the command";
    let lines = body
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .collect::<Vec<_>>();
    let opening = lines.first().copied().unwrap_or_default();
    let fence_char = opening.chars().next().filter(|ch| matches!(ch, '`' | '~'));
    let Some(fence_char) = fence_char else {
        return Err(MESSAGE.to_string());
    };
    let fence_len = opening.chars().take_while(|ch| *ch == fence_char).count();
    let is_fence = |line: &str| {
        let line = line.trim();
        line.len() >= fence_len && line.chars().all(|ch| ch == fence_char)
    };
    if fence_len < 3 || lines.len() < 2 || !is_fence(lines[lines.len() - 1]) {
        return Err(MESSAGE.to_string());
    }
    let inner = &lines[1..lines.len() - 1];
    if inner.iter().any(|line| is_fence(line)) {
        return Err(MESSAGE.to_string());
    }
    Ok(inner.iter().map(|line| format!("{line}\n")).collect())
}

fn render_command_block(command: &str) -> String {
    let longest_run = command
        .split(|ch| ch != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat((longest_run + 1).max(3));
    format!("{fence}sh\n{command}{fence}")
}

fn file_error(file_name: &str, error: impl std::fmt::Display) -> FlowSourceError {
    FlowSourceError::new(422, format!("{file_name}: {error}"))
}

fn read_error(path: &Path, error: std::io::Error) -> FlowSourceError {
    FlowSourceError::new(500, format!("Unable to read {}: {error}", path.display()))
}

fn write_error(path: &Path, error: std::io::Error) -> FlowSourceError {
    FlowSourceError::new(500, format!("Unable to write {}: {error}", path.display()))
}

fn default_schema_version() -> String {
    "1".to_string()
}

fn is_zero(value: &i64) -> bool {
    *value == 0
}
