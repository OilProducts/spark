//! One-time migration from the legacy conversation layout (`state.json` +
//! project-level artifact sidecars + `events.jsonl`) to split record files.
//!
//! Migration runs on the first read of a legacy conversation. It projects the
//! merged legacy snapshot into typed records, writes the split files, seeds
//! `journal.jsonl` with one `conversation_snapshot` checkpoint at the
//! carried-over revision (so clients reconnecting with any pre-migration
//! cursor replay full state naturally), renames the legacy files aside, and
//! absorbs the project-level sidecar files. Unsupported schemas error and
//! leave every file untouched. This module is deletable once local data is
//! converted.

use std::fs;
use std::path::Path;

use crate::error::{Result, StorageError};
use crate::workspace_conversations::{merge_sidecars, validate_supported_state};
use crate::ProjectPaths;

use super::journal::{JournalEntry, JournalEntryKind};
use super::projection::{record_from_snapshot, snapshot_from_record};
use super::store::{write_record, ConversationRecordPaths, RecordWritePlan};

pub(crate) fn migrate_legacy_conversation(
    project_paths: &ProjectPaths,
    conversation_id: &str,
) -> Result<()> {
    let root = project_paths.conversations_dir.join(conversation_id);
    let paths = ConversationRecordPaths::new(&root);
    if paths.conversation_json().exists() {
        return Ok(());
    }
    let state_path = paths.legacy_state_json();
    let text = match fs::read_to_string(&state_path) {
        Ok(text) => text,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(StorageError::io(
                "read conversation state",
                &state_path,
                source,
            ))
        }
    };
    let mut payload = serde_json::from_str::<serde_json::Value>(&text).map_err(|source| {
        StorageError::JsonRead {
            path: state_path.clone(),
            source,
        }
    })?;
    validate_supported_state(&state_path, &payload)?;
    let project_path = payload
        .get("project_path")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| project_paths.project_path.clone());
    merge_sidecars(project_paths, conversation_id, &project_path, &mut payload);
    let record = record_from_snapshot(&payload)?;

    write_record(
        &paths,
        &record,
        &RecordWritePlan {
            everything: true,
            ..RecordWritePlan::default()
        },
    )?;
    let checkpoint = JournalEntry {
        revision: record.meta.revision,
        committed_at: record.meta.updated_at.clone(),
        kind: JournalEntryKind::SnapshotCommitted,
    }
    .legacy_event_payload(&record.meta, &snapshot_from_record(&record));
    crate::append_jsonl_record(paths.journal_jsonl(), &checkpoint)?;

    rename_aside(&state_path)?;
    let legacy_events = paths.legacy_events_jsonl();
    if legacy_events.exists() {
        rename_aside(&legacy_events)?;
    }
    for sidecar in [
        project_paths
            .flow_run_requests_dir
            .join(format!("{conversation_id}.json")),
        project_paths
            .flow_launches_dir
            .join(format!("{conversation_id}.json")),
        project_paths
            .proposed_plans_dir
            .join(format!("{conversation_id}.json")),
    ] {
        match fs::remove_file(&sidecar) {
            Ok(()) => {}
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(StorageError::io(
                    "remove migrated conversation sidecar",
                    sidecar,
                    source,
                ))
            }
        }
    }
    Ok(())
}

fn rename_aside(path: &Path) -> Result<()> {
    let mut migrated = path.as_os_str().to_os_string();
    migrated.push(".migrated");
    fs::rename(path, &migrated)
        .map_err(|source| StorageError::io("rename legacy conversation file", path, source))
}
