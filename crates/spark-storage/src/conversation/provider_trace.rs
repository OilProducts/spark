//! Per-conversation provider trace: normalized provider events appended
//! before they are projected, so a crash mid-turn can replay them.
//!
//! The trace is recovery input only. Listing, opening and catch-up never read
//! it. Each turn's `applied_trace_sequence` row column is the authoritative
//! projection cursor; the conversation's `trace_sequence_high_water` keeps the
//! allocator above every sequence ever pruned from this file.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Result, StorageError};

pub const PROVIDER_TRACE_FILE_NAME: &str = "provider-trace.jsonl";

/// One durable provider event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderTraceRecord {
    pub sequence: u64,
    pub turn_id: String,
    /// When the event was recorded, so replay derives the same timestamps.
    pub recorded_at: String,
    pub event: Value,
}

pub(crate) fn trace_path(conversation_root: &Path) -> PathBuf {
    conversation_root.join(PROVIDER_TRACE_FILE_NAME)
}

/// Every complete record. A partial final line (an interrupted append) is
/// discarded; any other unreadable line is an explicit recovery error.
pub(crate) fn read_trace(conversation_root: &Path) -> Result<Vec<ProviderTraceRecord>> {
    let path = trace_path(conversation_root);
    let text = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(StorageError::io("read provider trace", &path, source)),
    };
    let complete = text.ends_with(b"\n");
    let lines: Vec<&[u8]> = text
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect();
    let mut records = Vec::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        match serde_json::from_slice::<ProviderTraceRecord>(line) {
            Ok(record) => records.push(record),
            Err(_) if index + 1 == lines.len() && !complete => {}
            Err(source) => {
                return Err(StorageError::JsonlLine {
                    path,
                    line: index + 1,
                    source,
                })
            }
        }
    }
    Ok(records)
}

/// Append one record. A process crash keeps written records. Power loss can
/// drop them, which leaves an applied cursor past the trace's end; replay only
/// reads above the cursor, so its already projected rows need nothing from them.
pub(crate) fn append_trace(conversation_root: &Path, record: &ProviderTraceRecord) -> Result<()> {
    let path = trace_path(conversation_root);
    let mut line = serde_json::to_vec(record).map_err(|source| StorageError::JsonWrite {
        path: path.clone(),
        source,
    })?;
    line.push(b'\n');
    fs::create_dir_all(conversation_root)
        .map_err(|source| StorageError::io("create conversation", conversation_root, source))?;
    // An interrupted append leaves an unterminated tail: a complete record
    // gets its newline, anything else is dropped, so it never becomes an
    // interior corrupt line once this record follows it.
    crate::activity::repair_partial_tail::<ProviderTraceRecord>(&path)?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| file.write_all(&line))
        .map_err(|source| StorageError::io("append provider trace", &path, source))
}

/// Replace the trace with `retained` records (empty truncates it). Callers
/// hold the conversation lock and have passed the cleanup checkpoint barrier.
pub(crate) fn replace_trace(
    conversation_root: &Path,
    retained: &[ProviderTraceRecord],
) -> Result<()> {
    let path = trace_path(conversation_root);
    if retained.is_empty() {
        return match OpenOptions::new().write(true).truncate(true).open(&path) {
            Ok(_) => Ok(()),
            Err(source) if source.kind() == ErrorKind::NotFound => Ok(()),
            Err(source) => Err(StorageError::io("truncate provider trace", &path, source)),
        };
    }
    let mut bytes = Vec::new();
    for record in retained {
        bytes.extend(
            serde_json::to_vec(record).map_err(|source| StorageError::JsonWrite {
                path: path.clone(),
                source,
            })?,
        );
        bytes.push(b'\n');
    }
    crate::write_atomic(&path, bytes)
}
