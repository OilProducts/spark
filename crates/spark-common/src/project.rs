use std::path::{Path, PathBuf};
use std::process::Command;

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

/// A project's agent organization: its own flows (`flows/`) and roles
/// (`roles/`), in a git repository whose history is the audit trail.
pub fn org_dir(project: &Path) -> PathBuf {
    project.join(".spark").join("agent-org")
}

/// Creates the project's org repository on its first write: the directory,
/// `git init`, and `<project>/.spark/.gitignore` holding `*` if missing.
pub fn ensure_org_repo(project: &Path) -> std::result::Result<PathBuf, String> {
    let org = org_dir(project);
    std::fs::create_dir_all(&org)
        .map_err(|error| format!("Unable to create {}: {error}", org.display()))?;
    if !org.join(".git").exists() {
        run_git(&org, &["init", "-q"])?;
    }
    let ignore = project.join(".spark").join(".gitignore");
    if !ignore.exists() {
        std::fs::write(&ignore, "*\n")
            .map_err(|error| format!("Unable to write {}: {error}", ignore.display()))?;
    }
    Ok(org)
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
