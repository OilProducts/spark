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
    let fallback =
        || normalize_path(path.to_string_lossy().as_ref()).unwrap_or_else(|_| path.to_path_buf());
    let Ok(output) = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
    else {
        return fallback();
    };
    if !output.status.success() {
        return fallback();
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        return fallback();
    }
    std::fs::canonicalize(&text).unwrap_or_else(|_| PathBuf::from(text))
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
