use std::collections::BTreeMap;
use std::path::PathBuf;

use attractor_runtime::RunStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
use spark_storage::{
    append_jsonl_record, read_jsonl, write_json_atomic, JsonLinesOptions, JsonWriteOptions,
};
use time::OffsetDateTime;

use crate::live::{LiveCursor, LiveEnvelope, LiveResource};
use crate::{WorkspaceError, WorkspaceResult};

pub const WORKFLOW_LOG_TAIL_LIMIT: usize = 100;

const WORKFLOW_LOG_FILE: &str = "event-log.jsonl";
const WORKFLOW_LOG_STATE_FILE: &str = "event-log-state.json";
const FAILURE_REASON_MAX_CHARS: usize = 200;

/// One milestone in the global, project-agnostic workflow event log.
///
/// The log deliberately has a high noise floor: only lifecycle boundaries a
/// human would act on or recount are recorded — never per-node progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowLogEntry {
    /// Deterministic identity: `{run_id}:{kind}` (gates append the question id),
    /// so readers can deduplicate replays and crash-window double-appends.
    pub id: String,
    pub seq: u64,
    pub timestamp: String,
    pub kind: String,
    pub message: String,
    pub project_path: String,
    pub run_id: String,
    pub flow_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct WorkflowLogRunState {
    #[serde(default)]
    status: String,
    #[serde(default)]
    logged_gate_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct WorkflowLogState {
    #[serde(default)]
    next_seq: u64,
    #[serde(default)]
    runs: BTreeMap<String, WorkflowLogRunState>,
}

fn workflow_log_path(settings: &SparkSettings) -> PathBuf {
    settings.workspace_dir.join(WORKFLOW_LOG_FILE)
}

fn workflow_log_state_path(settings: &SparkSettings) -> PathBuf {
    settings.workspace_dir.join(WORKFLOW_LOG_STATE_FILE)
}

fn read_workflow_log_state(settings: &SparkSettings) -> WorkflowLogState {
    let path = workflow_log_state_path(settings);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn iso_now() -> String {
    let now = OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
    )
}

fn one_line(value: &str) -> String {
    let line = value.lines().next().unwrap_or("").trim();
    if line.chars().count() > FAILURE_REASON_MAX_CHARS {
        let truncated: String = line.chars().take(FAILURE_REASON_MAX_CHARS).collect();
        format!("{truncated}…")
    } else {
        line.to_string()
    }
}

fn terminal_milestone_kind(status: &str) -> Option<&'static str> {
    match status {
        "completed" => Some("run_completed"),
        "failed" | "validation_error" => Some("run_failed"),
        "canceled" | "cancelled" | "aborted" => Some("run_canceled"),
        _ => None,
    }
}

/// Projects a run's state change into global workflow log milestones.
///
/// Reads the run record, diffs its status against the projection state, and
/// scans the just-published live envelopes for pending human gates. Appends
/// any new milestones to the log (entries before state, so a crash can only
/// produce a duplicate line — deduplicated on read — never a lost one) and
/// returns them for publishing. Idempotent: the same transition observed twice
/// diffs to nothing. Child runs are skipped entirely — the parent's lifecycle
/// covers them.
pub fn project_run_milestones(
    settings: &SparkSettings,
    run_id: &str,
    new_envelopes: &[LiveEnvelope],
) -> WorkspaceResult<Vec<WorkflowLogEntry>> {
    let store = RunStore::for_settings(settings);
    // Record-only read: the projection never uses events or checkpoint, and
    // a full bundle read reparses the entire event log.
    let Some(paths) = store
        .find_run_root(run_id)
        .map_err(|error| WorkspaceError::Internal(error.to_string()))?
    else {
        return Ok(Vec::new());
    };
    let Some(record) = store
        .read_run_record(&paths)
        .map_err(|error| WorkspaceError::Internal(error.to_string()))?
    else {
        return Ok(Vec::new());
    };
    if record.parent_run_id.is_some() {
        return Ok(Vec::new());
    }

    let mut state = read_workflow_log_state(settings);
    let run_state = state.runs.entry(run_id.to_string()).or_default();
    let flow_name = if record.flow_name.is_empty() {
        run_id.to_string()
    } else {
        record.flow_name.clone()
    };
    let project_path = if record.project_path.is_empty() {
        record.working_directory.clone()
    } else {
        record.project_path.clone()
    };
    let timestamp = iso_now();
    let mut pending: Vec<(String, String, String, Option<String>)> = Vec::new();

    if run_state.status.is_empty() {
        let (kind, message) = if let Some(source_run) = record
            .continued_from_run_id
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            (
                "run_continued",
                format!(
                    "Continued {flow_name} from {}",
                    &source_run[..source_run.len().min(8)]
                ),
            )
        } else {
            ("run_started", format!("Started {flow_name}"))
        };
        pending.push((kind.to_string(), format!("{run_id}:{kind}"), message, None));
    }

    if run_state.status != record.status {
        if let Some(kind) = terminal_milestone_kind(&record.status) {
            let message = match kind {
                "run_completed" => format!("Completed {flow_name}"),
                "run_canceled" => format!("Canceled {flow_name}"),
                _ => {
                    let reason = record
                        .outcome_reason_message
                        .as_deref()
                        .filter(|value| !value.trim().is_empty())
                        .map(one_line)
                        .or_else(|| {
                            (!record.last_error.trim().is_empty())
                                .then(|| one_line(&record.last_error))
                        });
                    match reason {
                        Some(reason) => format!("{flow_name} failed: {reason}"),
                        None => format!("{flow_name} failed"),
                    }
                }
            };
            pending.push((kind.to_string(), format!("{run_id}:{kind}"), message, None));
        }
        run_state.status = record.status.clone();
    }

    for envelope in new_envelopes {
        if envelope.event_type != "run.question_pending" {
            continue;
        }
        if envelope.payload.get("source_scope").and_then(Value::as_str) == Some("child") {
            continue;
        }
        let Some(question_id) = envelope
            .payload
            .get("question_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if run_state.logged_gate_ids.iter().any(|id| id == question_id) {
            continue;
        }
        run_state.logged_gate_ids.push(question_id.to_string());
        let node_id = envelope
            .payload
            .get("node_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let location = node_id
            .as_deref()
            .map(|node| format!(" at {node}"))
            .unwrap_or_default();
        pending.push((
            "run_waiting_on_input".to_string(),
            format!("{run_id}:waiting:{question_id}"),
            format!("{flow_name} is waiting for input{location}"),
            node_id,
        ));
    }

    if pending.is_empty() {
        return Ok(Vec::new());
    }

    let log_path = workflow_log_path(settings);
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            WorkspaceError::Internal(format!("create workflow log directory: {error}"))
        })?;
    }

    let mut entries = Vec::with_capacity(pending.len());
    for (kind, id, message, node_id) in pending {
        let entry = WorkflowLogEntry {
            id,
            seq: state.next_seq,
            timestamp: timestamp.clone(),
            kind,
            message,
            project_path: project_path.clone(),
            run_id: run_id.to_string(),
            flow_name: flow_name.clone(),
            node_id,
        };
        state.next_seq += 1;
        append_jsonl_record(&log_path, &entry)
            .map_err(|error| WorkspaceError::Internal(error.to_string()))?;
        entries.push(entry);
    }

    write_json_atomic(
        workflow_log_state_path(settings),
        &state,
        JsonWriteOptions::default(),
    )
    .map_err(|error| WorkspaceError::Internal(error.to_string()))?;

    Ok(entries)
}

/// Reads the newest `limit` entries, oldest first, deduplicated by entry id
/// (crash-window double-appends keep the first occurrence).
pub fn read_workflow_log_tail(
    settings: &SparkSettings,
    limit: usize,
) -> WorkspaceResult<Vec<WorkflowLogEntry>> {
    let path = workflow_log_path(settings);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let entries: Vec<WorkflowLogEntry> =
        read_jsonl(&path, JsonLinesOptions::allow_blank_lines())
            .map_err(|error| WorkspaceError::Internal(error.to_string()))?;
    let mut seen = std::collections::BTreeSet::new();
    let mut deduped: Vec<WorkflowLogEntry> = Vec::with_capacity(entries.len());
    for entry in entries {
        if seen.insert(entry.id.clone()) {
            deduped.push(entry);
        }
    }
    let start = deduped.len().saturating_sub(limit);
    Ok(deduped.split_off(start))
}

pub fn workflow_log_envelope(entry: &WorkflowLogEntry) -> LiveEnvelope {
    LiveEnvelope {
        event_type: "workflow_log.entry".to_string(),
        project_path: Some(entry.project_path.clone()),
        resource: LiveResource {
            kind: "workflow_log".to_string(),
            id: Some(entry.id.clone()),
        },
        cursor: Some(LiveCursor {
            kind: "workflow_log_seq".to_string(),
            value: entry.seq as i64,
        }),
        payload: serde_json::to_value(entry).unwrap_or_else(|_| json!({})),
        reason: None,
    }
}

pub fn workflow_log_tail_envelopes(
    settings: &SparkSettings,
    limit: usize,
) -> WorkspaceResult<Vec<LiveEnvelope>> {
    Ok(read_workflow_log_tail(settings, limit)?
        .iter()
        .map(workflow_log_envelope)
        .collect())
}
