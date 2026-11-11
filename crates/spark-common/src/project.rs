use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use crate::error::{Result, SparkCommonError};
use crate::paths::normalize_path;

pub fn normalize_project_path(value: impl AsRef<str>) -> Result<Option<PathBuf>> {
    let trimmed = value.as_ref().trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    normalize_path(trimmed).map(Some)
}

pub fn build_project_id(project_path: impl AsRef<str>) -> Result<String> {
    let normalized_path =
        normalize_project_path(project_path)?.ok_or(SparkCommonError::EmptyProjectPath)?;
    let normalized_text = normalized_path.to_string_lossy();
    let name = normalized_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let slug = slugify(name);
    let mut hasher = Sha1::new();
    hasher.update(normalized_text.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    Ok(format!("{slug}-{}", &digest[..12]))
}

/// Resolves the path that identifies the repository containing `path`: the
/// git common directory, shared by every linked worktree of one repository.
/// Outside a git repository, falls back to the normalized path itself.
pub fn repository_scope_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    match run_git(
        path,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ) {
        Ok(text) if !text.is_empty() => {
            std::fs::canonicalize(&text).unwrap_or_else(|_| PathBuf::from(text))
        }
        _ => normalize_path(path.to_string_lossy().as_ref()).unwrap_or_else(|_| path.to_path_buf()),
    }
}

/// Runs `git -C <directory> <args>`: trimmed stdout on success, else the
/// error output.
pub fn run_git(directory: &Path, args: &[&str]) -> std::result::Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(args)
        .output()
        .map_err(|error| format!("Unable to run git: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!("git {} failed: {stderr}", args.join(" ")));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// The project's Spark configuration: specification, profiles, standards,
/// templates, decisions, flows, roles, playbooks and triggers.
pub fn definitions_dir(project: &Path) -> PathBuf {
    project.join(".spark").join("definitions")
}

/// `G`, the installation's private storage for one project:
/// `<home>/projects/<project-id>/`, holding `conversations/`,
/// `memory/<role>/`, `runs/<run-id>/` (unless runs are configured elsewhere)
/// and `work/`. Pure path arithmetic: nothing is created or registered.
pub fn project_storage_dir(home: &Path, project_id: &str) -> PathBuf {
    home.join("projects").join(project_id)
}

/// `G` for the project at `project_path`, by its path-derived identity.
pub fn project_storage_dir_for(home: &Path, project_path: &str) -> Result<PathBuf> {
    Ok(project_storage_dir(home, &build_project_id(project_path)?))
}

/// The shell variable naming a run's [`project_work_dir`], for tools and agents.
pub const PROJECT_WORK_DIR_ENV: &str = "SPARK_PROJECT_WORK_DIR";

/// `G/work`: durable working files flows and agents keep for a project,
/// bound to runs as `internal.project_work_dir` / `SPARK_PROJECT_WORK_DIR`.
pub fn project_work_dir(home: &Path, project_path: &str) -> Result<PathBuf> {
    Ok(project_storage_dir_for(home, project_path)?.join("work"))
}

/// Where pipeline run roots live.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunsRoot {
    /// The installation home `H`: each project's runs are `G/runs/<run-id>`.
    Home(PathBuf),
    /// An explicitly configured root: `<root>/<project-id>/<run-id>`.
    External(PathBuf),
}

impl RunsRoot {
    /// The directory whose children are project ids.
    pub fn projects_dir(&self) -> PathBuf {
        match self {
            Self::Home(home) => home.join("projects"),
            Self::External(root) => root.clone(),
        }
    }

    /// The directory whose children are one project's run roots.
    pub fn project_runs_dir(&self, project_id: &str) -> PathBuf {
        match self {
            Self::Home(home) => project_storage_dir(home, project_id).join("runs"),
            Self::External(root) => root.join(project_id),
        }
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    format!("{:x}", sha2::Sha256::digest(bytes))
}

pub fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut last_was_separator = false;
    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            slug.push(ch);
            last_was_separator = false;
        } else if !last_was_separator {
            slug.push('-');
            last_was_separator = true;
        }
    }
    let trimmed = slug.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "project".to_string()
    } else {
        trimmed
    }
}
