//! A read-only view of a project's definitions: its flows and roles by
//! layer (bundled, project, or a project copy shadowing bundled) under
//! `.spark/definitions`, and the started roles' memory. Nothing here writes;
//! the flow editor and agents change the files.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use attractor_dsl::FlowRoots;
use serde::Serialize;
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;

use crate::inbox::{invalid, InboxEvent};
use crate::roles::{memory_dir, RoleRecord, WorkspaceRoleService};
use crate::{WorkspaceError, WorkspaceResult};
use spark_storage::{ConversationRepository, ConversationStore};
use std::sync::Arc;

const RECENT_WAKES: usize = 10;

#[derive(Debug, Clone, Serialize)]
pub struct RoleUsage {
    pub flow: String,
    pub node: String,
}

/// A started role's record and its newest inbox events.
#[derive(Debug, Clone, Serialize)]
pub struct LiveRole {
    #[serde(flatten)]
    pub role: RoleRecord,
    /// The newest inbox events, newest first.
    pub recent_events: Vec<InboxEvent>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DefinitionItem {
    /// `flow` or `role`.
    pub kind: &'static str,
    pub name: String,
    /// `bundled`, `project`, or `project_copy` (a project file shadowing a bundled one).
    pub layer: &'static str,
    /// Relative to the layer's root: `flows/<name>` or `roles/<name>.md`.
    pub path: String,
    /// A role's details in the list; absent for flows.
    #[serde(flatten)]
    pub role: Option<RoleDetails>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoleDetails {
    /// The role file's frontmatter `title` and `description`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The flow nodes that name this role.
    pub usage: Vec<RoleUsage>,
    /// Whether the role file declares `standing: true`; false when it does not parse.
    pub standing: bool,
    /// The started role, or null.
    pub live: Option<LiveRole>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DefinitionFile {
    pub name: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoleFileDetail {
    #[serde(flatten)]
    pub item: DefinitionItem,
    pub files: Vec<DefinitionFile>,
    /// For a project copy: `git diff` from today's bundled version to the copy.
    pub bundled_diff: Option<String>,
}

pub struct WorkspaceDefinitionService {
    settings: SparkSettings,
    store: Arc<ConversationStore>,
}

impl WorkspaceDefinitionService {
    pub fn new(settings: SparkSettings, store: Arc<ConversationStore>) -> Self {
        Self { settings, store }
    }

    fn roots(&self, project: &str) -> WorkspaceResult<FlowRoots> {
        if project.trim().is_empty() {
            return Err(invalid("Project path is required"));
        }
        Ok(FlowRoots::new(
            self.settings.bundled_dir(),
            Some(Path::new(project.trim())),
        ))
    }

    /// The project's flows and roles, flows first, each sorted by name.
    pub fn list(&self, project: &str) -> WorkspaceResult<Value> {
        let roots = self.roots(project)?;
        let org = roots.org.clone().unwrap();
        let mut items = Vec::new();
        let mut usage: BTreeMap<String, Vec<RoleUsage>> = BTreeMap::new();
        for name in roots.list_flow_names().map_err(flow_error)? {
            let flow = item(&roots, "flow", name)?;
            let root = if flow.layer == "bundled" {
                &roots.bundled
            } else {
                &org
            };
            // A flow whose files lead outside its root, or do not parse, uses no roles.
            if let Ok(definition) = item_files(root, &flow.path, "flow")
                .and_then(|files| attractor_dsl::assemble_flow_files(&files).map_err(flow_error))
            {
                for (node, definition) in &definition.nodes {
                    if let Some(role) = &definition.role {
                        usage.entry(role.clone()).or_default().push(RoleUsage {
                            flow: flow.name.clone(),
                            node: node.clone(),
                        });
                    }
                }
            }
            items.push(flow);
        }
        let mut live = self.live_roles(project)?;
        for name in role_names(&roots) {
            let mut role = item(&roots, "role", name)?;
            // A role file that does not parse has no title or description
            // and cannot be started.
            let file = roots
                .role_text(&role.name)
                .and_then(|(_, text)| attractor_dsl::parse_role(&role.name, &text))
                .ok();
            role.role = Some(RoleDetails {
                title: file.as_ref().and_then(|file| file.title.clone()),
                description: file.as_ref().and_then(|file| file.description.clone()),
                usage: usage.remove(&role.name).unwrap_or_default(),
                standing: file.is_some_and(|file| file.standing),
                live: live.remove(&role.name),
            });
            items.push(role);
        }
        Ok(json!({ "items": items }))
    }

    fn live_roles(&self, project: &str) -> WorkspaceResult<BTreeMap<String, LiveRole>> {
        let repository = ConversationRepository::new(self.store.clone());
        WorkspaceRoleService::new(self.settings.clone(), self.store.clone())
            .list(project)?
            .into_iter()
            .map(|role| {
                let events = repository.recent_inbox_events(&role.conversation_id, RECENT_WAKES)?;
                Ok((
                    role.name.clone(),
                    LiveRole {
                        role,
                        recent_events: events,
                    },
                ))
            })
            .collect()
    }

    /// A role's file as its winning layer has it; for a project copy, also
    /// its diff from bundled.
    pub fn role_file(&self, project: &str, name: &str) -> WorkspaceResult<RoleFileDetail> {
        let roots = self.roots(project)?;
        let org = roots.org.clone().unwrap();
        if !valid_role_name(name) {
            return Err(invalid(format!("Invalid role name: {name}")));
        }
        let item = item(&roots, "role", name.to_string())?;
        let root = if item.layer == "bundled" {
            &roots.bundled
        } else {
            &org
        };
        let files = item_files(root, &item.path, "role")?
            .into_iter()
            .map(|(name, content)| DefinitionFile { name, content })
            .collect();
        let bundled_diff = if item.layer == "project_copy" {
            // Read (and so contain) the bundled file before diffing it.
            item_files(&roots.bundled, &item.path, "role")?;
            Some(diff_no_index(
                &roots.bundled.join(&item.path),
                &org.join(&item.path),
            )?)
        } else {
            None
        };
        Ok(RoleFileDetail {
            item,
            files,
            bundled_diff,
        })
    }

    /// A role's memory files (relative paths, sorted), or one file's text
    /// when `file` is given.
    pub fn memory(&self, project: &str, role: &str, file: Option<&str>) -> WorkspaceResult<Value> {
        self.roots(project)?;
        if !valid_role_name(role) {
            return Err(invalid(format!("Invalid role name: {role}")));
        }
        // A symlinked memory directory must not lead out of the project's
        // private storage.
        let dir = memory_dir(&self.settings, project.trim(), role)?;
        let root =
            spark_common::project::project_storage_dir_for(&self.settings.data_dir, project.trim())
                .map_err(|error| invalid(error.to_string()))?;
        let dir = match (dir.canonicalize(), root.canonicalize()) {
            (Ok(dir), Ok(root)) if dir.starts_with(&root) && dir.is_dir() => dir,
            _ if file.is_none() => return Ok(json!({ "role": role, "files": [] })),
            _ => {
                return Err(WorkspaceError::NotFound(format!(
                    "No memory for role {role}"
                )))
            }
        };
        let Some(file) = file else {
            let mut files = Vec::new();
            list_files(&dir, &dir, &mut files);
            files.sort();
            return Ok(json!({ "role": role, "files": files }));
        };
        let path = dir.join(relative_path(file)?);
        // A symlink must not lead out of the memory directory either.
        if !inside(&dir, &path) || !path.is_file() {
            return Err(WorkspaceError::NotFound(format!(
                "No memory file {file} for role {role}"
            )));
        }
        let content = fs::read_to_string(&path)
            .map_err(|error| WorkspaceError::Internal(format!("Unable to read {file}: {error}")))?;
        Ok(json!({ "role": role, "file": file, "content": content }))
    }
}

fn flow_error(error: attractor_dsl::FlowSourceError) -> WorkspaceError {
    crate::flows::flow_source_error(error, None)
}

fn valid_role_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\']) && !name.starts_with('.')
}

/// A request path as relative components only: no `..`, root or prefix.
fn relative_path(path: &str) -> WorkspaceResult<PathBuf> {
    let path = Path::new(path.trim());
    if path.as_os_str().is_empty()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(invalid(format!(
            "Path must stay inside the definitions: {}",
            path.display()
        )));
    }
    Ok(path.to_path_buf())
}

fn item_rel_path(kind: &str, name: &str) -> String {
    match kind {
        "flow" => format!("flows/{name}"),
        _ => format!("roles/{name}.md"),
    }
}

fn exists(root: &Path, kind: &str, rel: &str) -> bool {
    match kind {
        "flow" => root.join(rel).join(attractor_dsl::FLOW_FILE).is_file(),
        _ => root.join(rel).is_file(),
    }
}

fn item(roots: &FlowRoots, kind: &'static str, name: String) -> WorkspaceResult<DefinitionItem> {
    let path = item_rel_path(kind, &name);
    let in_org = roots
        .org
        .as_deref()
        .is_some_and(|org| exists(org, kind, &path));
    let layer = match (in_org, exists(&roots.bundled, kind, &path)) {
        (true, true) => "project_copy",
        (true, false) => "project",
        (false, true) => "bundled",
        (false, false) => {
            return Err(WorkspaceError::NotFound(format!("No {kind} named {name}")));
        }
    };
    Ok(DefinitionItem {
        kind,
        name,
        layer,
        path,
        role: None,
    })
}

fn role_names(roots: &FlowRoots) -> BTreeSet<String> {
    roots
        .org
        .iter()
        .chain(std::iter::once(&roots.bundled))
        .filter_map(|root| fs::read_dir(root.join("roles")).ok())
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let stem = name.strip_suffix(".md")?.to_string();
            (entry.path().is_file() && valid_role_name(&stem)).then_some(stem)
        })
        .collect()
}

/// Whether `path`, symlinks resolved, exists inside `root`.
fn inside(root: &Path, path: &Path) -> bool {
    match (path.canonicalize(), root.canonicalize()) {
        (Ok(path), Ok(root)) => path.starts_with(root),
        _ => false,
    }
}

/// A flow's own files (file name -> text), or a role's single file, at
/// `rel` under `root`; no file may resolve outside `root`.
fn item_files(root: &Path, rel: &str, kind: &str) -> WorkspaceResult<BTreeMap<String, String>> {
    let path = root.join(rel);
    let escaped = || invalid(format!("{rel} leads outside {}", root.display()));
    if !inside(root, &path) {
        return Err(escaped());
    }
    if kind == "flow" {
        // Check every file before reading any.
        for entry in fs::read_dir(&path).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !attractor_dsl::RESERVED_FILES.contains(&name.as_str())
                && !inside(root, &entry.path())
            {
                return Err(escaped());
            }
        }
        return attractor_dsl::read_flow_dir_texts(&path).map_err(flow_error);
    }
    let text = fs::read_to_string(&path)
        .map_err(|error| WorkspaceError::Internal(format!("Unable to read role: {error}")))?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    Ok(BTreeMap::from([(name.to_string(), text)]))
}

/// `git diff --no-index` from `from` to `to`; exit 1 just means they differ.
fn diff_no_index(from: &Path, to: &Path) -> WorkspaceResult<String> {
    let output = Command::new("git")
        .args(["diff", "--no-index", "--no-color", "--no-ext-diff", "--"])
        .arg(from)
        .arg(to)
        .output()
        .map_err(|error| WorkspaceError::Internal(format!("Unable to run git: {error}")))?;
    match output.status.code() {
        Some(0 | 1) => Ok(String::from_utf8_lossy(&output.stdout).into_owned()),
        _ => Err(WorkspaceError::Internal(format!(
            "git diff failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

fn list_files(root: &Path, dir: &Path, files: &mut Vec<String>) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        // ponytail: symlinked directories are not followed.
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => list_files(root, &path, files),
            Ok(kind) if kind.is_file() => {
                if let Ok(rel) = path.strip_prefix(root) {
                    files.push(rel.to_string_lossy().to_string());
                }
            }
            _ => {}
        }
    }
}
