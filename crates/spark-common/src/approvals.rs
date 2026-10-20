//! The approval log, `$SPARK_HOME/config/approvals.jsonl`: an append-only
//! record of file contents a human (or bundled seeding) vouched for. A live
//! flow or role file may run only when its exact bytes have an entry for its
//! path.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const APPROVALS_FILE: &str = "approvals.jsonl";

/// Bundled seeding wrote this exact content.
pub const EVENT_INSTALLED: &str = "installed";
/// An editor save, or `spark flow accept`.
pub const EVENT_HUMAN_SAVE: &str = "human_save";
/// A candidate was promoted over the live file.
pub const EVENT_PROMOTED: &str = "promoted";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalEntry {
    pub at: String,
    pub event: String,
    /// Relative to `$SPARK_HOME`, for example `flows/x/y.md` or `roles/r.md`.
    pub path: String,
    pub sha256: String,
    pub by: String,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Now, as ISO 8601 with an explicit offset.
pub fn timestamp_now() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}+00:00",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

/// The actor bundled seeding records, `<producer>/<version>` (OKF §7).
pub fn installer_actor() -> String {
    format!("spark/{}", env!("CARGO_PKG_VERSION"))
}

/// Every entry in the log; a missing log is empty. Malformed lines are
/// skipped, so a torn trailing write never blocks reads.
pub fn read_approvals(log: &Path) -> std::io::Result<Vec<ApprovalEntry>> {
    let text = match std::fs::read_to_string(log) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect())
}

/// Appends one line per entry.
pub fn append_approvals(log: &Path, entries: &[ApprovalEntry]) -> std::io::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    if let Some(parent) = log.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut text = String::new();
    for entry in entries {
        text.push_str(&serde_json::to_string(entry).map_err(std::io::Error::other)?);
        text.push('\n');
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)?
        .write_all(text.as_bytes())
}

/// Builds entries stamped now for `(path, sha256)` pairs.
pub fn approval_entries(
    event: &str,
    by: &str,
    files: impl IntoIterator<Item = (String, String)>,
) -> Vec<ApprovalEntry> {
    let at = timestamp_now();
    files
        .into_iter()
        .map(|(path, sha256)| ApprovalEntry {
            at: at.clone(),
            event: event.to_string(),
            path,
            sha256,
            by: by.to_string(),
        })
        .collect()
}

/// Whether `bytes` at `path` have an entry of any event.
pub fn is_approved(entries: &[ApprovalEntry], path: &str, sha256: &str) -> bool {
    entries
        .iter()
        .any(|entry| entry.path == path && entry.sha256 == sha256)
}

/// The latest `installed` hash per path under `prefix` (for example `flows/`),
/// keyed by the path with the prefix removed.
pub fn installed_hashes(entries: &[ApprovalEntry], prefix: &str) -> BTreeMap<String, String> {
    entries
        .iter()
        .filter(|entry| entry.event == EVENT_INSTALLED)
        .filter_map(|entry| {
            let name = entry.path.strip_prefix(prefix)?;
            Some((name.to_string(), entry.sha256.clone()))
        })
        .collect()
}
