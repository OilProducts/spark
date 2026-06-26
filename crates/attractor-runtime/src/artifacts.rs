use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use attractor_core::ArtifactInfo;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use spark_storage::{append_line, write_json_atomic, write_text_atomic, JsonWriteOptions};

use crate::error::{Result, RuntimeStorageError};
use crate::paths::{validate_relative_path, RunRootPaths};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodeArtifacts {
    pub prompt: Option<String>,
    pub response: Option<String>,
    pub status: Option<Value>,
    pub under_logs: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolHookFailureRecord {
    pub command: String,
    pub exit_code: i32,
    pub hook_phase: String,
    pub stderr: String,
    pub stdout: String,
}

pub fn ensure_run_layout(paths: &RunRootPaths) -> Result<()> {
    for dir in [
        paths.logs_dir(),
        paths.logs_dir().join("artifacts"),
        paths.artifacts_dir(),
        paths.result_dir(),
    ] {
        fs::create_dir_all(&dir)
            .map_err(|source| RuntimeStorageError::io("create directory", dir, source))?;
    }
    if !paths.run_log().exists() {
        write_text_atomic(paths.run_log(), "")?;
    }
    Ok(())
}

pub fn write_node_artifacts(
    paths: &RunRootPaths,
    node_id: &str,
    artifacts: &NodeArtifacts,
) -> Result<PathBuf> {
    let node_path = validate_relative_path(node_id)?;
    let root = if artifacts.under_logs {
        paths.logs_dir().join(node_path)
    } else {
        paths.root.join(node_path)
    };
    fs::create_dir_all(&root).map_err(|source| {
        RuntimeStorageError::io("create node artifact directory", &root, source)
    })?;
    if let Some(prompt) = artifacts.prompt.as_deref() {
        write_text_atomic(root.join("prompt.md"), prompt)?;
    }
    if let Some(response) = artifacts.response.as_deref() {
        write_text_atomic(root.join("response.md"), response)?;
    }
    if let Some(status) = artifacts.status.as_ref() {
        write_json_atomic(
            root.join("status.json"),
            status,
            JsonWriteOptions::default(),
        )?;
    }
    Ok(root)
}

pub fn write_tool_output_log(paths: &RunRootPaths, node_id: &str, output: &str) -> Result<PathBuf> {
    let node_path = validate_relative_path(node_id)?;
    let stage_dir = paths.logs_dir().join(node_path);
    fs::create_dir_all(&stage_dir).map_err(|source| {
        RuntimeStorageError::io("create tool log directory", &stage_dir, source)
    })?;
    let output_path = stage_dir.join("tool_output.txt");
    write_text_atomic(&output_path, output)?;
    Ok(output_path)
}

pub fn append_tool_hook_failure(
    paths: &RunRootPaths,
    node_id: &str,
    record: &ToolHookFailureRecord,
) -> Result<PathBuf> {
    let node_path = validate_relative_path(node_id)?;
    let stage_dir = paths.logs_dir().join(node_path);
    fs::create_dir_all(&stage_dir).map_err(|source| {
        RuntimeStorageError::io("create tool hook log directory", &stage_dir, source)
    })?;
    let output_path = stage_dir.join("tool_hook_failures.jsonl");
    let line = serde_json::to_string(record)
        .map_err(|source| RuntimeStorageError::json(&output_path, source))?;
    append_line(&output_path, format!("{line}\n"))?;
    Ok(output_path)
}

pub fn write_tool_text_artifact(
    paths: &RunRootPaths,
    node_id: &str,
    relative_path: &str,
    text: &str,
) -> Result<PathBuf> {
    let node_path = validate_relative_path(node_id)?;
    let artifact_path = validate_relative_path(relative_path)?;
    let destination = paths.artifacts_dir().join(node_path).join(artifact_path);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|source| {
            RuntimeStorageError::io("create tool artifact directory", parent, source)
        })?;
    }
    write_text_atomic(&destination, text)?;
    Ok(destination)
}

pub fn copy_tool_artifact_matches(
    paths: &RunRootPaths,
    node_id: &str,
    cwd: &Path,
    patterns: &[String],
) -> Result<Vec<PathBuf>> {
    let node_path = validate_relative_path(node_id)?;
    let mut copied = Vec::new();
    for pattern in patterns {
        let safe_pattern = validate_relative_path(pattern)?;
        let glob_pattern = cwd.join(&safe_pattern).to_string_lossy().to_string();
        let matches = glob::glob(&glob_pattern).map_err(|source| {
            RuntimeStorageError::InvalidRuntimeGraph {
                reason: format!("invalid artifact glob '{pattern}': {source}"),
            }
        })?;
        for entry in matches {
            let source = entry.map_err(|source| RuntimeStorageError::InvalidRuntimeGraph {
                reason: format!("artifact glob failed for '{pattern}': {source}"),
            })?;
            if !source.is_file() {
                continue;
            }
            let relative =
                source
                    .strip_prefix(cwd)
                    .map_err(|_| RuntimeStorageError::UnsafeArtifactPath {
                        path: source.to_string_lossy().to_string(),
                        reason: "captured path must stay within the tool workdir".to_string(),
                    })?;
            let relative = validate_relative_path(&relative.to_string_lossy())?;
            let destination = paths
                .artifacts_dir()
                .join(&node_path)
                .join("captured")
                .join(relative);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(|source| {
                    RuntimeStorageError::io("create captured artifact directory", parent, source)
                })?;
            }
            fs::copy(&source, &destination).map_err(|source_error| {
                RuntimeStorageError::io("copy tool artifact", &source, source_error)
            })?;
            copied.push(destination);
        }
    }
    Ok(copied)
}

pub fn list_artifacts(paths: &RunRootPaths) -> Result<Vec<ArtifactInfo>> {
    paths.ensure_exists()?;
    let canonical_run_root = fs::canonicalize(&paths.root)
        .map_err(|source| RuntimeStorageError::io("canonicalize run root", &paths.root, source))?;
    let mut files = BTreeMap::<String, PathBuf>::new();
    if !paths.logs_manifest_json().is_file() {
        add_file(
            &mut files,
            &paths.root,
            &canonical_run_root,
            paths.manifest_json(),
        )?;
    }
    add_file(
        &mut files,
        &paths.root,
        &canonical_run_root,
        paths.checkpoint_json(),
    )?;
    add_file(
        &mut files,
        &paths.root,
        &canonical_run_root,
        paths.run_log(),
    )?;
    add_file(
        &mut files,
        &paths.root,
        &canonical_run_root,
        paths.result_json(),
    )?;
    add_file(
        &mut files,
        &paths.root,
        &canonical_run_root,
        paths.result_markdown(),
    )?;
    add_files_recursive(
        &mut files,
        &paths.root,
        &canonical_run_root,
        &paths.logs_dir(),
    )?;

    for entry in fs::read_dir(&paths.root)
        .map_err(|source| RuntimeStorageError::io("read run root", &paths.root, source))?
    {
        let entry = entry
            .map_err(|source| RuntimeStorageError::io("read run root", &paths.root, source))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name();
        if matches!(name.to_str(), Some("artifacts" | "logs" | "result")) {
            continue;
        }
        for filename in ["prompt.md", "response.md", "status.json"] {
            add_file(
                &mut files,
                &paths.root,
                &canonical_run_root,
                path.join(filename),
            )?;
        }
    }

    add_files_recursive(
        &mut files,
        &paths.root,
        &canonical_run_root,
        &paths.artifacts_dir(),
    )?;

    let mut entries = Vec::new();
    for (relative_path, absolute_path) in files {
        if is_internal_file(&relative_path) {
            continue;
        }
        let metadata = fs::metadata(&absolute_path)
            .map_err(|source| RuntimeStorageError::io("stat artifact", &absolute_path, source))?;
        let media_type = artifact_media_type(&absolute_path);
        entries.push(ArtifactInfo {
            path: relative_path,
            size_bytes: metadata.len(),
            viewable: artifact_is_viewable(&media_type, &absolute_path),
            media_type,
        });
    }
    Ok(entries)
}

pub fn append_run_log(paths: &RunRootPaths, message: &str) -> Result<()> {
    let line = format!(
        "[{} UTC] {}\n",
        crate::events::utc_timestamp()
            .split_once('T')
            .map(|(date, time)| format!("{date} {}", time.trim_end_matches('Z')))
            .unwrap_or_else(|| crate::events::utc_timestamp()),
        message
    );
    spark_storage::append_line(paths.run_log(), line)?;
    Ok(())
}

fn add_files_recursive(
    files: &mut BTreeMap<String, PathBuf>,
    run_root: &Path,
    canonical_run_root: &Path,
    root: &Path,
) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    let canonical_root = fs::canonicalize(root).map_err(|source| {
        RuntimeStorageError::io("canonicalize artifact directory", root, source)
    })?;
    if !canonical_root.starts_with(canonical_run_root) {
        return Ok(());
    }
    for entry in fs::read_dir(root)
        .map_err(|source| RuntimeStorageError::io("read artifacts", root, source))?
    {
        let entry =
            entry.map_err(|source| RuntimeStorageError::io("read artifacts", root, source))?;
        let path = entry.path();
        let Ok(canonical_path) = fs::canonicalize(&path) else {
            continue;
        };
        if !canonical_path.starts_with(canonical_run_root) {
            continue;
        }
        if path.is_dir() {
            add_files_recursive(files, run_root, canonical_run_root, &path)?;
        } else {
            add_file(files, run_root, canonical_run_root, path)?;
        }
    }
    Ok(())
}

fn add_file(
    files: &mut BTreeMap<String, PathBuf>,
    run_root: &Path,
    canonical_run_root: &Path,
    path: PathBuf,
) -> Result<()> {
    if !path.is_file() {
        return Ok(());
    }
    let canonical_path = fs::canonicalize(&path)
        .map_err(|source| RuntimeStorageError::io("canonicalize artifact", &path, source))?;
    if !canonical_path.starts_with(canonical_run_root) {
        return Ok(());
    }
    let Ok(relative) = path.strip_prefix(run_root) else {
        return Ok(());
    };
    files.insert(relative.to_string_lossy().replace('\\', "/"), path);
    Ok(())
}

fn is_internal_file(relative_path: &str) -> bool {
    matches!(relative_path, "run.json" | "state.json" | "events.jsonl")
}

fn artifact_media_type(path: &Path) -> String {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "dot" => "text/vnd.graphviz",
        "md" => "text/markdown",
        "json" => "application/json",
        "txt" => "text/plain",
        "html" => "text/html",
        "svg" => "image/svg+xml",
        "xml" => "application/xml",
        "csv" => "text/csv",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn artifact_is_viewable(media_type: &str, path: &Path) -> bool {
    if media_type.starts_with("text/") {
        return true;
    }
    if matches!(
        media_type,
        "application/json" | "application/xml" | "image/svg+xml"
    ) {
        return true;
    }
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "json" | "txt" | "md" | "log" | "dot" | "yaml" | "yml" | "csv"
    )
}
