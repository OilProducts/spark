use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use crate::error::{Result, SparkCommonError};

/// Environment lookup abstraction used by path and settings resolution tests.
pub trait Environment {
    fn get_var(&self, key: &str) -> Option<String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessEnvironment;

impl Environment for ProcessEnvironment {
    fn get_var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|value| !value.is_empty())
    }
}

impl Environment for BTreeMap<String, String> {
    fn get_var(&self, key: &str) -> Option<String> {
        self.get(key).cloned()
    }
}

impl Environment for HashMap<String, String> {
    fn get_var(&self, key: &str) -> Option<String> {
        self.get(key).cloned()
    }
}

impl Environment for [(String, String)] {
    fn get_var(&self, key: &str) -> Option<String> {
        self.iter()
            .find_map(|(candidate, value)| (candidate == key).then(|| value.clone()))
    }
}

impl<const N: usize> Environment for [(String, String); N] {
    fn get_var(&self, key: &str) -> Option<String> {
        self.as_slice().get_var(key)
    }
}

pub fn normalize_path(value: impl AsRef<Path>) -> Result<PathBuf> {
    let expanded = expand_tilde(value.as_ref());
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()
            .map_err(SparkCommonError::CurrentDirectory)?
            .join(expanded)
    };
    let mut symlink_count = 0;
    Ok(strip_verbatim_prefix(resolve_non_strict(
        &absolute,
        &mut symlink_count,
    )))
}

/// Removes Windows extended-length (`\\?\C:\...` and `\\?\UNC\server\share`)
/// prefixes that `std::fs::canonicalize` produces on Windows, so paths compare
/// equal to user-facing paths and work in `PATH` and child shells.
pub fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{unc}"));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        let bytes = rest.as_bytes();
        if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            return PathBuf::from(rest);
        }
    }
    path
}

/// Like [`std::fs::canonicalize`], without Windows `\\?\` verbatim prefixes.
pub fn canonicalize_clean(path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path).map(strip_verbatim_prefix)
}

/// Resolves the user's home directory from `HOME`, falling back to Windows'
/// `USERPROFILE` and `HOMEDRIVE` + `HOMEPATH`.
pub fn user_home_dir_with_env(env: &impl Environment) -> Option<PathBuf> {
    env.get_var("HOME")
        .or_else(|| env.get_var("USERPROFILE"))
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            let drive = env.get_var("HOMEDRIVE").filter(|v| !v.is_empty())?;
            let path = env.get_var("HOMEPATH").filter(|v| !v.is_empty())?;
            Some(PathBuf::from(format!("{drive}{path}")))
        })
}

pub fn user_home_dir() -> Option<PathBuf> {
    user_home_dir_with_env(&ProcessEnvironment)
}

/// Expands `~`, `~/...`, and `~\...` using `env`.
pub fn expand_tilde_with_env(path: &Path, env: &impl Environment) -> PathBuf {
    let path_text = path.to_string_lossy();
    if path_text == "~" {
        return user_home_dir_with_env(env).unwrap_or_else(|| path.to_path_buf());
    }
    if let Some(rest) = path_text
        .strip_prefix("~/")
        .or_else(|| path_text.strip_prefix(r"~\"))
    {
        if let Some(home) = user_home_dir_with_env(env) {
            return home.join(rest);
        }
    }
    path.to_path_buf()
}

pub fn detect_project_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for ancestor in manifest_dir.ancestors() {
        if ancestor.join("Cargo.toml").is_file() && ancestor.join("crates").is_dir() {
            return ancestor.to_path_buf();
        }
        if ancestor.join(".git").exists() {
            return ancestor.to_path_buf();
        }
        if ancestor.join("Cargo.toml").is_file() && ancestor.join("crates").is_dir() {
            return ancestor.to_path_buf();
        }
    }
    manifest_dir
}

pub fn ensure_directory(path: impl AsRef<Path>, label: &str) -> Result<()> {
    let target = normalize_path(path)?;
    std::fs::create_dir_all(&target).map_err(|source| SparkCommonError::DirectoryCreate {
        label: label.to_string(),
        path: target,
        source,
    })
}

pub fn resolve_runtime_workspace_path(value: impl AsRef<str>) -> Result<String> {
    resolve_runtime_workspace_path_with_env(value, &ProcessEnvironment)
}

pub fn resolve_runtime_workspace_path_with_env(
    value: impl AsRef<str>,
    env: &impl Environment,
) -> Result<String> {
    let normalized = match crate::project::normalize_project_path(value.as_ref())? {
        Some(path) => path,
        None => return Ok(String::new()),
    };

    if normalized.exists() {
        return Ok(normalized.to_string_lossy().into_owned());
    }

    let host_root = env
        .get_var("ATTRACTOR_HOST_REPO_ROOT")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(normalize_path)
        .transpose()?;

    let mut runtime_roots = Vec::new();
    if let Some(runtime_root) = env
        .get_var("ATTRACTOR_RUNTIME_REPO_ROOT")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        runtime_roots.push(normalize_path(runtime_root)?);
    }
    runtime_roots.push(detect_project_root());

    for runtime_root in runtime_roots {
        if !runtime_root.exists() {
            continue;
        }

        if let Some(host_root) = host_root.as_deref() {
            if let Ok(relative) = normalized.strip_prefix(host_root) {
                let candidate = runtime_root.join(relative);
                if candidate.exists() {
                    return Ok(normalize_path(candidate)?.to_string_lossy().into_owned());
                }
            }

            if let Some(name) = host_root.file_name() {
                if let Some(candidate) =
                    remap_by_matching_segment(&normalized, &runtime_root, name)?
                {
                    return Ok(candidate.to_string_lossy().into_owned());
                }
            }
        }

        if let Some(name) = runtime_root.file_name() {
            if let Some(candidate) = remap_by_matching_segment(&normalized, &runtime_root, name)? {
                return Ok(candidate.to_string_lossy().into_owned());
            }
        }
    }

    Ok(normalized.to_string_lossy().into_owned())
}

fn remap_by_matching_segment(
    requested_path: &Path,
    runtime_root: &Path,
    segment_name: &std::ffi::OsStr,
) -> Result<Option<PathBuf>> {
    let parts: Vec<OsString> = requested_path
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_os_string()),
            _ => None,
        })
        .collect();

    for index in (0..parts.len()).rev() {
        if parts[index] != segment_name {
            continue;
        }
        let mut candidate = runtime_root.to_path_buf();
        for part in &parts[index + 1..] {
            candidate.push(part);
        }
        if candidate.exists() {
            return Ok(Some(normalize_path(candidate)?));
        }
    }

    Ok(None)
}

fn expand_tilde(path: &Path) -> PathBuf {
    expand_tilde_with_env(path, &ProcessEnvironment)
}

fn resolve_non_strict(path: &Path, symlink_count: &mut usize) -> PathBuf {
    let mut resolved = PathBuf::new();
    let mut components = path.components();

    while let Some(component) = components.next() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Prefix(prefix) => resolved.push(prefix.as_os_str()),
            Component::RootDir => resolved.push(component.as_os_str()),
            Component::Normal(value) => {
                let candidate = resolved.join(value);
                match std::fs::symlink_metadata(&candidate) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        *symlink_count += 1;
                        if *symlink_count > 40 {
                            resolved.push(value);
                            continue;
                        }

                        let Ok(target) = std::fs::read_link(&candidate) else {
                            resolved.push(value);
                            continue;
                        };
                        let mut replacement = if target.is_absolute() {
                            target
                        } else {
                            resolved.join(target)
                        };
                        for remaining in components {
                            replacement.push(remaining.as_os_str());
                        }
                        return resolve_non_strict(&replacement, symlink_count);
                    }
                    Ok(_) | Err(_) => resolved.push(value),
                }
            }
        }
    }

    normalize_components(&resolved)
}

fn normalize_components(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Normal(value) => normalized.push(value),
        }
    }
    normalized
}
