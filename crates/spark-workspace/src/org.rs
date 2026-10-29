//! A read-only view of a project's agent organization: its flows and roles
//! by layer (bundled, project, or a project copy shadowing bundled), their
//! history in the org repo, and the standing roles' memory. Nothing here
//! writes; changes go through the Coordinator, the CoS and the flow editor.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use attractor_dsl::FlowRoots;
use serde::Serialize;
use serde_json::{json, Value};
use spark_common::project::run_git;
use spark_common::settings::SparkSettings;

use crate::inbox::{invalid, read_events, InboxEvent};
use crate::roles::{memory_dir, RoleRecord, WorkspaceRoleService};
use crate::{WorkspaceError, WorkspaceResult};
use spark_storage::{workspace_records::RecordRepository, ProjectRegistry};

const RECENT_WAKES: usize = 10;

#[derive(Debug, Clone, Serialize)]
pub struct RoleUsage {
    pub flow: String,
    pub node: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StandingRole {
    #[serde(flatten)]
    pub role: RoleRecord,
    /// The newest inbox events, newest first.
    pub recent_events: Vec<InboxEvent>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrgItem {
    /// `flow` or `role`.
    pub kind: &'static str,
    pub name: String,
    /// `bundled`, `project`, or `project_copy` (a project file shadowing a bundled one).
    pub layer: &'static str,
    /// Relative to the layer's root: `flows/<name>` or `roles/<name>.md`.
    pub path: String,
    /// Some file differs from the org repo's HEAD or is untracked.
    pub uncommitted: bool,
    /// For a committed project copy: whether bundled differs from the copy's first commit.
    pub bundled_changed: Option<bool>,
    /// A role file's frontmatter `title` and `description`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Vec<RoleUsage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub standing: Option<StandingRole>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrgFile {
    pub name: String,
    pub content: String,
    /// `untracked` or `modified` against the org repo's HEAD.
    pub status: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrgItemDetail {
    #[serde(flatten)]
    pub item: OrgItem,
    pub files: Vec<OrgFile>,
    /// For a project copy: `git diff` from today's bundled version to the copy.
    pub bundled_diff: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrgCommit {
    pub commit: String,
    pub author: String,
    pub date: String,
    pub message: String,
    pub diff: String,
}

pub struct WorkspaceOrgService {
    settings: SparkSettings,
}

impl WorkspaceOrgService {
    pub fn new(settings: SparkSettings) -> Self {
        Self { settings }
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
        let changed = uncommitted_paths(&org);
        let mut items = Vec::new();
        let mut usage: BTreeMap<String, Vec<RoleUsage>> = BTreeMap::new();
        for name in roots.list_flow_names().map_err(flow_error)? {
            let flow = item(&roots, &changed, "flow", name)?;
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
        let standing = self.standing_roles(project)?;
        for name in role_names(&roots) {
            let mut role = item(&roots, &changed, "role", name)?;
            // A role file that does not parse has no title or description.
            if let Ok(file) = roots
                .role_text(&role.name)
                .and_then(|(_, text)| attractor_dsl::parse_role(&role.name, &text))
            {
                role.title = file.title;
                role.description = file.description;
            }
            role.usage = Some(usage.remove(&role.name).unwrap_or_default());
            role.standing = standing.get(&role.name).cloned();
            items.push(role);
        }
        Ok(json!({
            "org_path": org,
            "bundled_path": roots.bundled,
            "org_exists": org.join(".git").exists(),
            "items": items,
        }))
    }

    fn standing_roles(&self, project: &str) -> WorkspaceResult<BTreeMap<String, StandingRole>> {
        let scope = ProjectRegistry::new(self.settings.data_dir.clone()).project_paths(project)?;
        let repo = RecordRepository::roles(&scope.root);
        WorkspaceRoleService::new(self.settings.clone())
            .list(project)?
            .into_iter()
            .map(|role| {
                let mut events = read_events(&repo, &role.name)?;
                events.reverse();
                events.truncate(RECENT_WAKES);
                Ok((
                    role.name.clone(),
                    StandingRole {
                        role,
                        recent_events: events,
                    },
                ))
            })
            .collect()
    }

    /// One item's files as its winning layer has them; for a project copy,
    /// also its diff from bundled and whether bundled moved since the copy.
    pub fn item(&self, project: &str, kind: &str, name: &str) -> WorkspaceResult<OrgItemDetail> {
        let roots = self.roots(project)?;
        let org = roots.org.clone().unwrap();
        let (kind, name) = match kind {
            "flow" => (
                "flow",
                attractor_dsl::normalize_flow_name(name).map_err(flow_error)?,
            ),
            "role" if valid_role_name(name) => ("role", name.to_string()),
            "role" => return Err(invalid(format!("Invalid role name: {name}"))),
            _ => return Err(invalid("kind must be flow or role")),
        };
        let changed = uncommitted_paths(&org);
        let item = item(&roots, &changed, kind, name)?;
        let root = if item.layer == "bundled" {
            &roots.bundled
        } else {
            &org
        };
        let files = item_files(root, &item.path, kind)?
            .into_iter()
            .map(|(file, content)| {
                let path = file_path(&item.path, kind, &file);
                OrgFile {
                    status: (item.layer != "bundled")
                        .then(|| changed.get(&path).copied())
                        .flatten(),
                    name: file,
                    content,
                }
            })
            .collect();
        let bundled_diff = if item.layer == "project_copy" {
            // Read (and so contain) the bundled files before diffing them.
            item_files(&roots.bundled, &item.path, kind)?;
            Some(diff_no_index(
                &roots.bundled.join(&item.path),
                &org.join(&item.path),
            )?)
        } else {
            None
        };
        Ok(OrgItemDetail {
            item,
            files,
            bundled_diff,
        })
    }

    /// Commits touching `path` in the org repo, newest first, with diffs.
    pub fn history(&self, project: &str, path: &str) -> WorkspaceResult<Vec<OrgCommit>> {
        let org = self.roots(project)?.org.unwrap();
        let path = relative_path(path)?;
        if !org.join(".git").exists() {
            return Ok(Vec::new());
        }
        let path = path.to_string_lossy();
        // A repo without commits has no history.
        let Ok(log) = run_git(
            &org,
            &[
                "--literal-pathspecs",
                "log",
                "-p",
                "--no-color",
                "--no-ext-diff",
                "--format=%x1e%H%x1f%an%x1f%aI%x1f%B%x1f",
                "--",
                &path,
            ],
        ) else {
            return Ok(Vec::new());
        };
        Ok(log
            .split('\x1e')
            .filter_map(|entry| {
                let mut fields = entry.splitn(5, '\x1f');
                Some(OrgCommit {
                    commit: fields.next()?.trim().to_string(),
                    author: fields.next()?.to_string(),
                    date: fields.next()?.to_string(),
                    message: fields.next()?.trim().to_string(),
                    diff: fields.next()?.trim().to_string(),
                })
            })
            .collect())
    }

    /// A role's memory files (relative paths, sorted), or one file's text
    /// when `file` is given.
    pub fn memory(&self, project: &str, role: &str, file: Option<&str>) -> WorkspaceResult<Value> {
        self.roots(project)?;
        if !valid_role_name(role) {
            return Err(invalid(format!("Invalid role name: {role}")));
        }
        // A symlinked memory directory must not lead out of the org repo.
        let dir = memory_dir(project.trim(), role);
        let org = spark_common::project::org_dir(Path::new(project.trim()));
        let dir = match (dir.canonicalize(), org.canonicalize()) {
            (Ok(dir), Ok(org)) if dir.starts_with(&org) && dir.is_dir() => dir,
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
            "Path must stay inside the org repo: {}",
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

fn file_path(item_path: &str, kind: &str, file: &str) -> String {
    match kind {
        "flow" => format!("{item_path}/{file}"),
        _ => item_path.to_string(),
    }
}

fn exists(root: &Path, kind: &str, rel: &str) -> bool {
    match kind {
        "flow" => root.join(rel).join(attractor_dsl::FLOW_FILE).is_file(),
        _ => root.join(rel).is_file(),
    }
}

fn item(
    roots: &FlowRoots,
    changed: &BTreeMap<String, &'static str>,
    kind: &'static str,
    name: String,
) -> WorkspaceResult<OrgItem> {
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
    let uncommitted = in_org
        && changed.keys().any(|file| match kind {
            "flow" => file.starts_with(&format!("{path}/")),
            _ => *file == path,
        });
    // Read (and so contain) the bundled files before comparing them; files
    // that lead outside their root, or git failing, leave it unknown.
    let bundled_changed = (layer == "project_copy")
        .then(|| {
            let bundled = item_files(&roots.bundled, &path, kind).ok()?;
            let copied = first_commit_texts(roots.org.as_deref()?, &path, kind)?.ok()?;
            Some(copied != bundled)
        })
        .flatten();
    Ok(OrgItem {
        kind,
        name,
        layer,
        path,
        uncommitted,
        bundled_changed,
        title: None,
        description: None,
        usage: None,
        standing: None,
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

/// The item's files as its first org commit recorded them; None before any commit.
fn first_commit_texts(
    org: &Path,
    path: &str,
    kind: &str,
) -> Option<WorkspaceResult<BTreeMap<String, String>>> {
    // Paths are literal, and listed unquoted (NUL-separated).
    let log = run_git(
        org,
        &[
            "--literal-pathspecs",
            "log",
            "--reverse",
            "--format=%H",
            "--",
            path,
        ],
    )
    .ok()?;
    let commit = log.lines().next()?;
    let listed = run_git(
        org,
        &[
            "--literal-pathspecs",
            "ls-tree",
            "-r",
            "-z",
            "--name-only",
            commit,
            "--",
            path,
        ],
    )
    .ok()?;
    let mut files = BTreeMap::new();
    for file in listed.split('\0').filter(|file| !file.is_empty()) {
        let name = Path::new(file)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let own = kind != "flow"
            || (name.ends_with(".md") && !attractor_dsl::RESERVED_FILES.contains(&name.as_str()));
        if !own {
            continue;
        }
        match git_blob(org, &format!("{commit}:{file}")) {
            Ok(text) => files.insert(name, text),
            Err(error) => return Some(Err(error)),
        };
    }
    Some(Ok(files))
}

/// Org-relative paths that differ from HEAD (staged or not) or are
/// untracked; without a HEAD every tracked file counts as changed.
fn uncommitted_paths(org: &Path) -> BTreeMap<String, &'static str> {
    let mut paths = BTreeMap::new();
    if !org.join(".git").exists() {
        return paths;
    }
    let tracked = run_git(org, &["diff", "--name-only", "-z", "HEAD"])
        .or_else(|_| run_git(org, &["ls-files", "-z", "--cached"]))
        .unwrap_or_default();
    let untracked =
        run_git(org, &["ls-files", "-z", "--others", "--exclude-standard"]).unwrap_or_default();
    for (list, status) in [(tracked, "modified"), (untracked, "untracked")] {
        for path in list.split('\0').filter(|path| !path.is_empty()) {
            paths.insert(path.to_string(), status);
        }
    }
    paths
}

/// A blob's exact text; `run_git` would trim it.
fn git_blob(org: &Path, spec: &str) -> WorkspaceResult<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(org)
        .args(["show", spec])
        .output()
        .map_err(|error| WorkspaceError::Internal(format!("Unable to run git: {error}")))?;
    if !output.status.success() {
        return Err(WorkspaceError::Internal(format!(
            "git show {spec} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
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
