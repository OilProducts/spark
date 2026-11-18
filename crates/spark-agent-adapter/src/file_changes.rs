//! Per-file diffs a file-editing tool call reports, carried as
//! `tool_call.file_changes`.

use serde::Serialize;
use serde_json::Value;
use similar::{ChangeTag, TextDiff};
use spark_common::segments::truncate_utf8;

/// Diff text kept per tool call, across all of its entries.
const MAX_DIFF_BYTES: usize = 64 * 1024;
/// Texts larger than this together are not compared.
const MAX_COMPARE_BYTES: usize = 1024 * 1024;

#[derive(Serialize)]
pub(crate) struct FileChange {
    pub path: String,
    /// `add`, `delete`, `update` or `move`, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub move_from: Option<String>,
    pub diff: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

impl FileChange {
    pub(crate) fn new(path: &str, kind: Option<&'static str>, diff: String) -> Self {
        Self {
            path: path.to_string(),
            kind,
            move_from: None,
            diff,
            truncated: false,
        }
    }

    /// `old` against `new` line by line, each hunk opened by a bare `@@`
    /// since there are no reliable line numbers.
    pub(crate) fn compare(path: &str, old: &str, new: &str) -> Self {
        if old.len() + new.len() > MAX_COMPARE_BYTES {
            let mut change = Self::new(path, None, String::new());
            change.truncated = true;
            return change;
        }
        // Lines, not raw text, so a missing final newline changes nothing.
        let old = old.lines().collect::<Vec<_>>();
        let new = new.lines().collect::<Vec<_>>();
        let diff = TextDiff::from_slices(&old, &new);
        let mut text = String::new();
        for group in diff.grouped_ops(3) {
            text.push_str("@@\n");
            for change in group.iter().flat_map(|op| diff.iter_changes(op)) {
                text.push(match change.tag() {
                    ChangeTag::Delete => '-',
                    ChangeTag::Insert => '+',
                    ChangeTag::Equal => ' ',
                });
                text.push_str(change.value());
                text.push('\n');
            }
        }
        Self::new(path, None, text)
    }
}

/// Every line of `text` after `marker`, as for a file added or deleted whole.
pub(crate) fn prefixed(text: &str, marker: char) -> String {
    text.lines()
        .map(|line| format!("{marker}{line}\n"))
        .collect()
}

/// The `file_changes` payload, or `None` without any. Diff text past the
/// per-call cap is cut in entry order; every entry keeps its path.
pub(crate) fn payload(mut changes: Vec<FileChange>) -> Option<Value> {
    if changes.is_empty() {
        return None;
    }
    let mut budget = MAX_DIFF_BYTES;
    for change in &mut changes {
        let (diff, cut) = truncate_utf8(&change.diff, budget);
        budget -= diff.len();
        change.truncated |= cut;
        change.diff = diff;
    }
    serde_json::to_value(changes).ok()
}
