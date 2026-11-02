//! One-time setup of a project's `.spark/definitions` from the embedded
//! Spark assets.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};

use spark_common::project::definitions_dir;

use crate::errors::{WorkspaceError, WorkspaceResult};

const MARKER: &str = ".initialized";

/// What `initialize_project` did, with paths relative to `definitions_dir`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSetup {
    pub definitions_dir: PathBuf,
    /// True when the marker was already present, so nothing was installed.
    pub already_initialized: bool,
    pub created: Vec<String>,
    pub skipped: Vec<String>,
}

/// Installs every missing file of the default definitions into
/// `<project>/.spark/definitions`, then writes `.initialized`. Once the marker
/// exists this does nothing: the project owns its copies from then on, and
/// later Spark versions never add, restore or rewrite them. Existing files are
/// skipped, never overwritten. A failure leaves the files written so far and
/// no marker, so a retry fills in the rest.
pub fn initialize_project(project: &Path) -> WorkspaceResult<ProjectSetup> {
    if !project.is_dir() {
        return Err(WorkspaceError::Validation(format!(
            "Project directory not found: {}",
            project.display()
        )));
    }
    let definitions = definitions_dir(project);
    let marker = definitions.join(MARKER);
    if std::fs::symlink_metadata(&marker).is_ok_and(|metadata| metadata.is_file()) {
        return Ok(ProjectSetup {
            definitions_dir: definitions,
            already_initialized: true,
            created: Vec::new(),
            skipped: Vec::new(),
        });
    }
    let (seed_version, plan) = setup_plan().map_err(WorkspaceError::Internal)?;
    let mut created = Vec::new();
    let mut skipped = Vec::new();
    for (relative, bytes) in &plan {
        if write_missing_file(&definitions.join(relative), bytes)? {
            created.push(relative.clone());
        } else {
            skipped.push(relative.clone());
        }
    }
    std::fs::write(
        &marker,
        format!("spark {}\nseed {seed_version}\n", env!("CARGO_PKG_VERSION")),
    )
    .map_err(|error| write_error(&marker, error))?;
    Ok(ProjectSetup {
        definitions_dir: definitions,
        already_initialized: false,
        created,
        skipped,
    })
}

/// The seed version and every definitions-relative path the setup installs,
/// with its bytes: the seed's directories, every bundled flow, role and
/// mission playbook, and a README and changelog rendered from the seed's
/// templates.
fn setup_plan() -> Result<(String, BTreeMap<String, Vec<u8>>), String> {
    let seed = spark_assets::agent_org::files()
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    let seed_version = seed_front_matter(&seed, "spec/README.md")?
        .get("version")
        .and_then(serde_yaml::Value::as_str)
        .filter(|version| !version.trim().is_empty())
        .ok_or("Seed spec/README.md declares no version")?
        .to_string();

    // The seed's root files are its own README and changelog; the
    // definitions get rendered records instead.
    let mut plan = seed
        .iter()
        .filter(|(path, _)| path.contains('/'))
        .map(|(path, bytes)| (path.clone(), bytes.to_vec()))
        .collect::<BTreeMap<_, _>>();

    let flows = spark_assets::flows::starter_flow_assets()
        .map_err(|error| format!("Bundled flows are unavailable: {error:?}"))?;
    for asset in flows {
        plan.insert(format!("flows/{}", asset.name), asset.content.into_bytes());
    }
    for (file, text) in spark_assets::roles::assets() {
        plan.insert(format!("roles/{file}"), text.into_bytes());
    }
    for (file, text) in spark_assets::playbooks::assets() {
        plan.insert(format!("playbooks/{file}"), text.into_bytes());
    }

    let profiles = seed
        .keys()
        .filter_map(|path| path.strip_prefix("profiles/")?.split_once('/'))
        .map(|(name, _)| name)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(", ");
    for record in ["README.md", "CHANGELOG.md"] {
        let template = format!("templates/{record}");
        let text = seed
            .get(&template)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .ok_or_else(|| format!("Seed {template} is missing"))?
            .replace("{{spark_version}}", env!("CARGO_PKG_VERSION"))
            .replace("{{seed_version}}", &seed_version)
            .replace("{{profiles}}", &profiles);
        plan.insert(record.to_string(), text.into_bytes());
    }

    for path in plan.keys() {
        let safe = Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
        if !safe {
            return Err(format!("Unsafe definitions path: {path}"));
        }
    }
    Ok((seed_version, plan))
}

fn seed_front_matter(
    seed: &BTreeMap<String, &'static [u8]>,
    path: &str,
) -> Result<serde_yaml::Mapping, String> {
    let invalid = || format!("Seed {path} has no valid front matter");
    let text = seed
        .get(path)
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .ok_or_else(|| format!("Seed {path} is missing"))?;
    let (front_matter, _) = text
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .ok_or_else(invalid)?;
    serde_yaml::from_str(front_matter).map_err(|_| invalid())
}

/// Writes `bytes` to `path` only if nothing exists there: true when created,
/// false when skipped.
fn write_missing_file(path: &Path, bytes: &[u8]) -> WorkspaceResult<bool> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| write_error(parent, error))?;
    }
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
        Err(error) => return Err(write_error(path, error)),
    };
    file.write_all(bytes)
        .map_err(|error| write_error(path, error))?;
    Ok(true)
}

fn write_error(path: &Path, error: std::io::Error) -> WorkspaceError {
    WorkspaceError::Internal(format!("Unable to write {}: {error}", path.display()))
}
