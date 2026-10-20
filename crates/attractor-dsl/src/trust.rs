//! Spark's hash-log trust gate over flow and role files. A run may use a live
//! file only when the approval log (`$SPARK_HOME/config/approvals.jsonl`) has
//! an entry for its exact bytes and the file is not `deprecated`. Agents
//! propose changes as candidates under `$SPARK_HOME/candidates`; a human
//! promotes them, or accepts a hand-edited live file.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use attractor_core::{FlowDefinition, TrustStatus};
use spark_common::approvals::{
    append_approvals, approval_entries, is_approved, read_approvals, sha256_hex, timestamp_now,
    ApprovalEntry, APPROVALS_FILE, EVENT_HUMAN_SAVE, EVENT_PROMOTED,
};
use spark_common::settings::SparkSettings;

use crate::flow_files::{
    assemble_flow_files, file_trust, parse_role, read_flow_dir_texts, resolve_flow_roles_with,
    split_frontmatter, FLOW_FILE,
};
use crate::flow_sources::{list_flow_names, FlowSourceError};

const FLOWS_PREFIX: &str = "flows/";
const ROLES_PREFIX: &str = "roles/";

/// Where live flow and role files sit, and the log that approves them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustStore {
    pub flows_dir: PathBuf,
    pub roles_dir: PathBuf,
    pub log_path: PathBuf,
}

/// A flow loaded through the gate: the resolved form runs execute, and the
/// `$SPARK_HOME`-relative path -> SHA-256 of every file it used.
#[derive(Debug, Clone, PartialEq)]
pub struct ApprovedFlow {
    pub flow: FlowDefinition,
    pub files: BTreeMap<String, String>,
}

impl TrustStore {
    pub fn for_settings(settings: &SparkSettings) -> Self {
        Self {
            flows_dir: settings.flows_dir.clone(),
            roles_dir: settings.roles_dir(),
            log_path: settings.config_dir.join(APPROVALS_FILE),
        }
    }

    /// The log path of a live file (`flows/...` or `roles/...`), if it is one.
    pub fn key(&self, path: &Path) -> Option<String> {
        let under = |root: &Path| -> Option<String> {
            let rest = match path.strip_prefix(root) {
                Ok(rest) => rest.to_path_buf(),
                Err(_) => {
                    let (path, root) = (path.canonicalize().ok()?, root.canonicalize().ok()?);
                    path.strip_prefix(root).ok()?.to_path_buf()
                }
            };
            attractor_core::normalize_flow_ref(&slash_path(&rest))
        };
        under(&self.flows_dir)
            .map(|rest| format!("{FLOWS_PREFIX}{rest}"))
            .or_else(|| under(&self.roles_dir).map(|rest| format!("{ROLES_PREFIX}{rest}")))
    }

    /// The live file a log path names.
    pub fn live_path(&self, key: &str) -> Option<PathBuf> {
        let (root, rest) = if let Some(rest) = key.strip_prefix(FLOWS_PREFIX) {
            (&self.flows_dir, rest)
        } else {
            (&self.roles_dir, key.strip_prefix(ROLES_PREFIX)?)
        };
        let rest = attractor_core::normalize_flow_ref(rest)?;
        Some(root.join(rest))
    }

    fn entries(&self) -> Result<Vec<ApprovalEntry>, FlowSourceError> {
        read_approvals(&self.log_path).map_err(|error| log_error(&self.log_path, error))
    }

    fn append(&self, entries: &[ApprovalEntry]) -> Result<(), FlowSourceError> {
        append_approvals(&self.log_path, entries).map_err(|error| log_error(&self.log_path, error))
    }

    /// Logs `event` for files a writer just wrote, by their written bytes.
    pub fn record_written(
        &self,
        event: &str,
        by: &str,
        paths: &[PathBuf],
    ) -> Result<Vec<ApprovalEntry>, FlowSourceError> {
        let mut files = Vec::new();
        for path in paths {
            let key = self.key(path).ok_or_else(|| outside_error(path))?;
            let bytes = fs::read(path).map_err(|error| io_error(path, error))?;
            files.push((key, sha256_hex(&bytes)));
        }
        let entries = approval_entries(event, by, files);
        self.append(&entries)?;
        Ok(entries)
    }
}

/// Loads a live flow directory for a run: assembles it, resolves its roles,
/// and refuses it unless every file it uses (`flow.md`, node files, role
/// files) is approved and none is `deprecated`.
pub fn load_approved_flow(
    trust: &TrustStore,
    flow_dir: &Path,
) -> Result<ApprovedFlow, FlowSourceError> {
    let texts = read_flow_dir_texts(flow_dir)?;
    let authored = assemble_flow_files(&texts)?;
    let mut used = texts
        .iter()
        .map(|(name, text)| (flow_dir.join(name), text.clone()))
        .collect::<Vec<_>>();
    let mut roles = BTreeMap::new();
    for role in authored
        .nodes
        .values()
        .filter_map(|node| node.role.clone())
        .collect::<BTreeSet<_>>()
    {
        let path = trust.roles_dir.join(format!("{role}.md"));
        if let Ok(text) = fs::read_to_string(&path) {
            used.push((path, text.clone()));
            roles.insert(role, text);
        }
    }
    let flow = resolve_flow_roles_with(&authored, |name| {
        roles.get(name).cloned().ok_or_else(|| {
            format!(
                "unknown role '{name}' (no {})",
                trust.roles_dir.join(format!("{name}.md")).display()
            )
        })
    })?;

    let entries = trust.entries()?;
    let mut files = BTreeMap::new();
    let mut refusals = Vec::new();
    for (path, text) in used {
        let Some(key) = trust.key(&path) else {
            refusals.push(format!(
                "{}: outside the flows and roles directories, so it cannot be approved",
                path.display()
            ));
            continue;
        };
        let sha256 = sha256_hex(text.as_bytes());
        if !is_approved(&entries, &key, &sha256) {
            refusals.push(format!(
                "{key}: not approved (no approval-log entry matches its current content)"
            ));
        }
        match file_trust(&text) {
            Ok(fields) if fields.status == TrustStatus::Deprecated => {
                refusals.push(format!("{key}: status is deprecated"));
            }
            Ok(_) => {}
            Err(error) => refusals.push(format!("{key}: {error}")),
        }
        files.insert(key, sha256);
    }
    if !refusals.is_empty() {
        return Err(FlowSourceError::new(
            403,
            format!(
                "Launch refused for flow {}:\n- {}\nReview each unapproved file, then approve it with `spark flow accept <path>` (a hand-edited live file) or `spark flow promote <candidate>` (a proposal under $SPARK_HOME/candidates). Deprecated files do not run.",
                flow_dir.display(),
                refusals.join("\n- ")
            ),
        ));
    }
    Ok(ApprovedFlow { flow, files })
}

/// `spark flow accept`: logs `human_save` for the current bytes of a live
/// file, or of every file in a live flow directory. Files are not modified.
pub fn accept(
    trust: &TrustStore,
    path: &Path,
    by: &str,
) -> Result<Vec<ApprovalEntry>, FlowSourceError> {
    let path = live_target(trust, path)?;
    let paths = if path.is_dir() {
        read_flow_dir_texts(&path)?
            .into_keys()
            .map(|name| path.join(name))
            .collect()
    } else {
        vec![path]
    };
    trust.record_written(EVENT_HUMAN_SAVE, by, &paths)
}

/// `spark flow promote`: copies a candidate file, or every file of a
/// candidate directory, over its live path with `verified` appended and
/// `status: stable`, logs `promoted`, and deletes the candidate. Refuses
/// candidates without `generated` or `status: draft`, and any result that
/// leaves a flow using the promoted files invalid.
pub fn promote(
    trust: &TrustStore,
    candidates_dir: &Path,
    path: &Path,
    by: &str,
) -> Result<Vec<ApprovalEntry>, FlowSourceError> {
    let path = if path.exists() {
        path.to_path_buf()
    } else {
        candidates_dir.join(path)
    };
    let mut candidates = Vec::new();
    collect_markdown(&path, &mut candidates).map_err(|error| io_error(&path, error))?;
    if candidates.is_empty() {
        return Err(FlowSourceError::new(
            404,
            format!("No candidate files at {}", path.display()),
        ));
    }
    let canonical_candidates = candidates_dir
        .canonicalize()
        .map_err(|error| io_error(candidates_dir, error))?;

    // key -> (candidate path, promoted text)
    let mut promoted = BTreeMap::new();
    let mut refusals = Vec::new();
    let verified_at = timestamp_now();
    for candidate in &candidates {
        let key = candidate
            .canonicalize()
            .ok()
            .and_then(|path| {
                path.strip_prefix(&canonical_candidates)
                    .ok()
                    .map(slash_path)
            })
            // A flow file sits in a flow directory; a role file directly in roles/.
            .filter(|key| match key.strip_prefix(FLOWS_PREFIX) {
                Some(rest) => rest.contains('/') && rest.ends_with(".md"),
                None => key
                    .strip_prefix(ROLES_PREFIX)
                    .is_some_and(|rest| !rest.contains('/') && rest.ends_with(".md")),
            })
            .filter(|key| trust.live_path(key).is_some());
        let Some(key) = key else {
            refusals.push(format!(
                "{}: not a flow file under {}flows/<flow>/ or a role file under {}roles/",
                candidate.display(),
                candidates_dir.display(),
                candidates_dir.display()
            ));
            continue;
        };
        let text = fs::read_to_string(candidate).map_err(|error| io_error(candidate, error))?;
        match file_trust(&text) {
            Err(error) => refusals.push(format!("{key}: {error}")),
            Ok(fields) => {
                if fields.generated.is_none() {
                    refusals.push(format!("{key}: has no generated field"));
                }
                if fields.status != TrustStatus::Draft {
                    refusals.push(format!("{key}: status is not draft"));
                }
            }
        }
        match promoted_text(&text, by, &verified_at) {
            Ok(text) => {
                promoted.insert(key, (candidate.clone(), text));
            }
            Err(error) => refusals.push(format!("{key}: {error}")),
        }
    }
    if refusals.is_empty() {
        refusals = promotion_problems(trust, &promoted);
    }
    if !refusals.is_empty() {
        return Err(FlowSourceError::new(
            422,
            format!("Promotion refused:\n- {}", refusals.join("\n- ")),
        ));
    }

    let mut files = Vec::new();
    for (key, (_, text)) in &promoted {
        let live = trust.live_path(key).expect("checked above");
        write_atomic(&live, text)?;
        files.push((key.clone(), sha256_hex(text.as_bytes())));
    }
    let entries = approval_entries(EVENT_PROMOTED, by, files);
    trust.append(&entries)?;
    for (candidate, _) in promoted.values() {
        fs::remove_file(candidate).map_err(|error| io_error(candidate, error))?;
        // Drop candidate directories the promotion emptied.
        let mut dir = candidate.parent();
        while let Some(current) = dir.filter(|dir| dir.starts_with(candidates_dir)) {
            if current == candidates_dir || fs::remove_dir(current).is_err() {
                break;
            }
            dir = current.parent();
        }
    }
    Ok(entries)
}

/// Whether any node file in `dir` names one of `roles`, read from each file's
/// frontmatter alone so a flow that fails to assemble is still found.
fn flow_uses_roles(dir: &Path, roles: &BTreeMap<String, String>) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        fs::read_to_string(entry.path()).is_ok_and(|text| {
            split_frontmatter(&text)
                .ok()
                .and_then(|(frontmatter, _)| {
                    serde_yaml::from_str::<serde_yaml::Value>(frontmatter).ok()
                })
                .and_then(|value| value.get("role")?.as_str().map(str::to_string))
                .is_some_and(|role| roles.contains_key(&role))
        })
    })
}

/// Every flow that would use the promoted files must still assemble and
/// validate, with its roles resolved.
fn promotion_problems(
    trust: &TrustStore,
    promoted: &BTreeMap<String, (PathBuf, String)>,
) -> Vec<String> {
    let mut problems = Vec::new();
    let mut roles = BTreeMap::new();
    for (key, (_, text)) in promoted {
        if let Some(file) = key.strip_prefix(ROLES_PREFIX) {
            let name = file.strip_suffix(".md").unwrap_or(file).to_string();
            if let Err(error) = parse_role(&name, text) {
                problems.push(format!("{key}: {error}"));
            }
            roles.insert(name, text.clone());
        }
    }
    let role_text = |name: &str| {
        roles.get(name).cloned().map(Ok).unwrap_or_else(|| {
            let path = trust.roles_dir.join(format!("{name}.md"));
            fs::read_to_string(&path)
                .map_err(|_| format!("unknown role '{name}' (no {})", path.display()))
        })
    };

    // Flows whose own files are promoted, assembled from live files overlaid
    // with the promoted ones.
    let mut overlays: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (key, (_, text)) in promoted {
        if let Some((flow, file)) = key
            .strip_prefix(FLOWS_PREFIX)
            .and_then(|rest| rest.rsplit_once('/'))
        {
            overlays
                .entry(flow.to_string())
                .or_default()
                .insert(file.to_string(), text.clone());
        }
    }
    for (flow, files) in &overlays {
        let dir = trust.flows_dir.join(flow);
        let mut texts = if dir.join(FLOW_FILE).is_file() {
            match read_flow_dir_texts(&dir) {
                Ok(texts) => texts,
                Err(error) => {
                    problems.push(format!("flow {flow}: {}", error.detail()));
                    continue;
                }
            }
        } else {
            BTreeMap::new()
        };
        texts.extend(files.clone());
        if let Err(error) = assemble_flow_files(&texts)
            .and_then(|authored| resolve_flow_roles_with(&authored, role_text))
        {
            problems.push(format!("flow {flow}: {}", error.detail()));
        }
    }

    // Other live flows that reference a promoted role.
    if !roles.is_empty() {
        for flow in list_flow_names(&trust.flows_dir).unwrap_or_default() {
            if overlays.contains_key(&flow) {
                continue;
            }
            let dir = trust.flows_dir.join(&flow);
            if !flow_uses_roles(&dir, &roles) {
                continue;
            }
            if let Err(error) = read_flow_dir_texts(&dir)
                .and_then(|texts| assemble_flow_files(&texts))
                .and_then(|authored| resolve_flow_roles_with(&authored, role_text))
            {
                problems.push(format!("flow {flow}: {}", error.detail()));
            }
        }
    }
    problems
}

/// The candidate's text with `verified: { by, at }` appended and
/// `status: stable`; the body is kept as is.
fn promoted_text(text: &str, by: &str, at: &str) -> Result<String, String> {
    let (frontmatter, body) = split_frontmatter(text)?;
    let mut fields: serde_yaml::Mapping =
        serde_yaml::from_str(frontmatter).map_err(|error| error.to_string())?;
    let mut verified = match fields.remove("verified") {
        None | Some(serde_yaml::Value::Null) => Vec::new(),
        Some(serde_yaml::Value::Sequence(list)) => list,
        Some(one) => vec![one],
    };
    let mut entry = serde_yaml::Mapping::new();
    entry.insert("by".into(), by.into());
    entry.insert("at".into(), at.into());
    verified.push(entry.into());
    fields.insert("verified".into(), serde_yaml::Value::Sequence(verified));
    fields.insert("status".into(), "stable".into());
    let yaml = serde_yaml::to_string(&fields).map_err(|error| error.to_string())?;
    Ok(format!("---\n{yaml}---\n{body}"))
}

/// A path given on the command line: as is when it exists, else relative to
/// `$SPARK_HOME` (`flows/...` or `roles/...`). It must be a live file.
fn live_target(trust: &TrustStore, path: &Path) -> Result<PathBuf, FlowSourceError> {
    let path = if path.exists() {
        path.to_path_buf()
    } else {
        trust
            .live_path(&slash_path(path))
            .filter(|path| path.exists())
            .ok_or_else(|| FlowSourceError::new(404, format!("No such file: {}", path.display())))?
    };
    trust.key(&path).ok_or_else(|| outside_error(&path))?;
    Ok(path)
}

fn collect_markdown(path: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if path.is_file() {
        files.push(path.to_path_buf());
        return Ok(());
    }
    let mut entries = fs::read_dir(path)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort();
    for entry in entries {
        if entry.is_dir() {
            collect_markdown(&entry, files)?;
        } else if entry.extension().is_some_and(|ext| ext == "md") {
            files.push(entry);
        }
    }
    Ok(())
}

fn write_atomic(path: &Path, text: &str) -> Result<(), FlowSourceError> {
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
    // Beside the target's directory, so flow loaders never see the temp file.
    let temp = parent.parent().unwrap_or(parent).join(format!(
        ".{}.{}.promote.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    fs::write(&temp, text).map_err(|error| io_error(&temp, error))?;
    fs::rename(&temp, path).map_err(|error| io_error(path, error))
}

fn slash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn outside_error(path: &Path) -> FlowSourceError {
    FlowSourceError::new(
        400,
        format!(
            "{} is not a live flow or role file under $SPARK_HOME/flows or $SPARK_HOME/roles.",
            path.display()
        ),
    )
}

fn io_error(path: &Path, error: std::io::Error) -> FlowSourceError {
    FlowSourceError::new(500, format!("{}: {error}", path.display()))
}

fn log_error(path: &Path, error: std::io::Error) -> FlowSourceError {
    FlowSourceError::new(
        500,
        format!("Unable to use approval log {}: {error}", path.display()),
    )
}
