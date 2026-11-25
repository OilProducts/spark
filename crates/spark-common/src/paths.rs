use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use crate::error::{Result, SparkCommonError};

/// Environment lookup abstraction used by path and settings resolution tests.
pub trait Environment {
    fn get_var(&self, key: &str) -> Option<String>;

    /// Like [`Environment::get_var`], keeping a value that is not UTF-8.
    fn get_var_os(&self, key: &str) -> Option<OsString> {
        self.get_var(key).map(OsString::from)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessEnvironment;

impl Environment for ProcessEnvironment {
    fn get_var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|value| !value.is_empty())
    }

    fn get_var_os(&self, key: &str) -> Option<OsString> {
        std::env::var_os(key).filter(|value| !value.is_empty())
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
    normalize_path_with_env(value, &ProcessEnvironment)
}

/// [`normalize_path`], expanding `~` with `env`'s home.
pub fn normalize_path_with_env(value: impl AsRef<Path>, env: &impl Environment) -> Result<PathBuf> {
    let expanded = expand_tilde_with_env(value.as_ref(), env);
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

/// Removes Windows extended-length (`\\?\C:\...` and `\\?\UNC\server\share\...`)
/// prefixes that `std::fs::canonicalize` produces on Windows, so paths compare
/// equal to user-facing paths and work in `PATH` and child shells.
///
/// The prefix stays wherever the conventional Win32 form could name another
/// target or be unusable: a non-UTF-8 path or one of another shape, including
/// a UNC server of `?` or `.` (a device path once stripped); a component that
/// ends in a space or `.` (including `.` and `..`) or holds a `/`; a component
/// whose stem (before the first `.` or `:`, trailing spaces trimmed) is a
/// reserved device name such as `NUL`, `COM1` or `CONIN$`; or a result of
/// 248 or more UTF-16 code units. 248 is the Win32 directory limit and keeps
/// every file under `MAX_PATH` (260) without the long-path opt-in.
pub fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    let (conventional, names) = if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        let mut parts = unc.split('\\');
        if matches!(parts.next().unwrap_or_default(), "" | "?" | ".")
            || parts.next().unwrap_or_default().is_empty()
        {
            return path;
        }
        (format!(r"\\{unc}"), unc)
    } else {
        match text.strip_prefix(r"\\?\") {
            Some(rest)
                if rest.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
                    && rest[1..].starts_with(r":\") =>
            {
                (rest.to_string(), &rest[3..])
            }
            _ => return path,
        }
    };
    let ambiguous = |name: &str| {
        name.ends_with([' ', '.']) || name.contains('/') || is_reserved_device_name(name)
    };
    if conventional.encode_utf16().count() >= 248 || names.split('\\').any(ambiguous) {
        return path;
    }
    PathBuf::from(conventional)
}

fn is_reserved_device_name(name: &str) -> bool {
    let stem = name
        .split(['.', ':'])
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || matches!(stem.as_str(), "CONIN$" | "CONOUT$")
        || ["COM", "LPT"].into_iter().any(|device| {
            stem.strip_prefix(device).is_some_and(|number| {
                matches!(
                    number,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
}

/// Like [`std::fs::canonicalize`], without Windows `\\?\` verbatim prefixes.
pub fn canonicalize_clean(path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path).map(strip_verbatim_prefix)
}

/// Resolves the user's home directory from `HOME`, falling back to Windows'
/// `USERPROFILE` and `HOMEDRIVE` + `HOMEPATH`. Empty values are skipped, and
/// a value that is not UTF-8 is kept as is.
pub fn user_home_dir_with_env(env: &impl Environment) -> Option<PathBuf> {
    let var = |key: &str| env.get_var_os(key).filter(|value| !value.is_empty());
    var("HOME")
        .or_else(|| var("USERPROFILE"))
        .or_else(|| {
            let mut home = var("HOMEDRIVE")?;
            home.push(var("HOMEPATH")?);
            Some(home)
        })
        .map(PathBuf::from)
}

pub fn user_home_dir() -> Option<PathBuf> {
    user_home_dir_with_env(&ProcessEnvironment)
}

/// What follows the `~` of a `~`, `~/...` or `~\...` path, byte for byte.
pub(crate) fn tilde_rest(path: &Path) -> Option<&Path> {
    // `~\` only separates on Windows; elsewhere it is split off as bytes.
    #[cfg(unix)]
    let backslash_rest = || {
        use std::os::unix::ffi::OsStrExt;
        let rest = path.as_os_str().as_bytes().strip_prefix(br"~\")?;
        Some(Path::new(std::ffi::OsStr::from_bytes(rest)))
    };
    #[cfg(not(unix))]
    let backslash_rest = || path.to_str()?.strip_prefix(r"~\").map(Path::new);
    path.strip_prefix("~").ok().or_else(backslash_rest)
}

/// Expands `~`, `~/...`, and `~\...` using `env`, keeping both the home and
/// the rest of the path byte for byte.
pub fn expand_tilde_with_env(path: &Path, env: &impl Environment) -> PathBuf {
    let Some(rest) = tilde_rest(path) else {
        return path.to_path_buf();
    };
    match user_home_dir_with_env(env) {
        Some(home) if rest.as_os_str().is_empty() => home,
        Some(home) => home.join(rest),
        None => path.to_path_buf(),
    }
}

/// The process environment's variables whose name and value are UTF-8;
/// [`std::env::vars`] panics on any other, such as a non-UTF-8 `HOME`.
pub fn utf8_env_vars() -> BTreeMap<String, String> {
    std::env::vars_os()
        .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
        .collect()
}

/// [`expand_tilde_with_env`] with the process environment.
pub fn expand_tilde(path: &Path) -> PathBuf {
    expand_tilde_with_env(path, &ProcessEnvironment)
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
            // `//server/share` and `\\server\share` name one share; one
            // spelling keeps their project IDs equal, as `C:/` and `C:\` are.
            // A server of `?` or `.` (`//?/C:/repo`) is a device path, which
            // backslashes would make verbatim, so it keeps its spelling.
            Component::Prefix(prefix) => match prefix.kind() {
                std::path::Prefix::UNC(server, share)
                    if !matches!(server.to_str(), Some("" | "?" | ".")) && !share.is_empty() =>
                {
                    let mut unc = OsString::from(r"\\");
                    unc.push(server);
                    unc.push(r"\");
                    unc.push(share);
                    normalized.push(unc);
                }
                _ => normalized.push(prefix.as_os_str()),
            },
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Normal(value) => normalized.push(value),
        }
    }
    normalized
}
