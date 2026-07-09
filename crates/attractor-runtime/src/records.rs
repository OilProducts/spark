use std::collections::BTreeSet;

use attractor_core::RunRecord;
use serde_json::{Map, Value};
use spark_storage::{read_json_optional, write_json_atomic, JsonWriteOptions};

use crate::error::{Result, RuntimeStorageError};
use crate::paths::RunRootPaths;

const RUN_RECORD_FIELDS: &[&str] = &[
    "run_id",
    "flow_name",
    "status",
    "outcome",
    "outcome_reason_code",
    "outcome_reason_message",
    "working_directory",
    "model",
    "provider",
    "llm_provider",
    "llm_profile",
    "reasoning_effort",
    "started_at",
    "ended_at",
    "project_path",
    "git_branch",
    "git_commit",
    "spec_id",
    "plan_id",
    "continued_from_run_id",
    "continued_from_node",
    "continued_from_flow_mode",
    "continued_from_flow_name",
    "parent_run_id",
    "parent_node_id",
    "root_run_id",
    "child_invocation_index",
    "execution_mode",
    "execution_profile_id",
    "execution_container_image",
    "execution_profile_capabilities",
    "execution_lock",
    "cleanup_error",
    "last_error",
    "token_usage",
    "token_usage_breakdown",
    "estimated_model_cost",
    "launch_context",
];

pub fn read_run_record(paths: &RunRootPaths) -> Result<Option<RunRecord>> {
    let Some(value) = read_json_optional::<Value>(paths.run_json())? else {
        return Ok(None);
    };
    let mut record: RunRecord = serde_json::from_value(value)
        .map_err(|source| RuntimeStorageError::json(paths.run_json(), source))?;
    normalize_record(&mut record);
    Ok(Some(record))
}

pub fn write_run_record(paths: &RunRootPaths, record: &RunRecord) -> Result<()> {
    let mut normalized = record.clone();
    normalize_record_for_write(&mut normalized);
    let mut output = serde_json::to_value(&normalized)
        .map_err(|source| RuntimeStorageError::json(paths.run_json(), source))?;
    let Some(output_object) = output.as_object_mut() else {
        return Err(RuntimeStorageError::UnsafeArtifactPath {
            path: paths.run_json().to_string_lossy().into_owned(),
            reason: "run record must serialize to a JSON object".to_string(),
        });
    };

    if let Some(existing) = read_json_optional::<Value>(paths.run_json())? {
        if let Some(existing_object) = existing.as_object() {
            merge_unknown_fields(output_object, existing_object);
        }
    }

    write_json_atomic(paths.run_json(), &output, JsonWriteOptions::default())?;
    Ok(())
}

pub fn normalize_run_status(status: &str) -> String {
    match status {
        "success" => "completed".to_string(),
        "fail" => "failed".to_string(),
        "aborted" => "canceled".to_string(),
        "abort_requested" => "cancel_requested".to_string(),
        "cancelled" => "canceled".to_string(),
        _ => status.to_string(),
    }
}

pub fn normalize_record(record: &mut RunRecord) {
    record.status = normalize_run_status(record.status.trim());
    if record.execution_mode.trim().is_empty() {
        record.execution_mode = "native".to_string();
    }
    if record.llm_provider.trim().is_empty() && !record.provider.trim().is_empty() {
        record.llm_provider = record.provider.trim().to_string();
    }
    if record.provider.trim().is_empty() && !record.llm_provider.trim().is_empty() {
        record.provider = record.llm_provider.trim().to_string();
    }
}

pub fn normalize_record_for_write(record: &mut RunRecord) {
    normalize_record(record);
    let mut provider = record
        .llm_provider
        .trim()
        .to_string()
        .if_empty_then(record.provider.trim().to_string())
        .if_empty_then("codex".to_string());
    if record
        .llm_profile
        .as_deref()
        .is_some_and(|profile| !profile.trim().is_empty() && profile.trim() == provider)
    {
        provider = "codex".to_string();
    }
    record.provider = provider.clone();
    record.llm_provider = provider;
}

pub fn mark_record_retry_started(record: &mut RunRecord) {
    record.status = "running".to_string();
    record.outcome = None;
    record.outcome_reason_code = None;
    record.outcome_reason_message = None;
    record.ended_at = None;
    record.last_error.clear();
}

/// A run blocked on a human gate: still alive, surfaced as needs-input.
pub fn mark_record_waiting(record: &mut RunRecord) {
    record.status = "waiting".to_string();
    record.ended_at = None;
    record.last_error.clear();
}

pub fn mark_record_running_after_wait(record: &mut RunRecord) {
    record.status = "running".to_string();
    record.ended_at = None;
}

pub fn mark_record_paused(record: &mut RunRecord) {
    record.status = "paused".to_string();
    record.ended_at = Some(crate::events::utc_timestamp());
    record.last_error.clear();
}

pub fn mark_record_cancel_requested(record: &mut RunRecord) {
    record.status = "cancel_requested".to_string();
    record.outcome = None;
    record.outcome_reason_code = None;
    record.outcome_reason_message = None;
    record.last_error = "cancel_requested_by_user".to_string();
}

pub fn mark_record_canceled(record: &mut RunRecord, last_error: impl Into<String>) {
    record.status = "canceled".to_string();
    record.outcome = None;
    record.outcome_reason_code = None;
    record.outcome_reason_message = None;
    record.ended_at = Some(crate::events::utc_timestamp());
    record.last_error = last_error.into();
}

fn merge_unknown_fields(output: &mut Map<String, Value>, existing: &Map<String, Value>) {
    let known = RUN_RECORD_FIELDS.iter().copied().collect::<BTreeSet<_>>();
    for (key, value) in existing {
        if !known.contains(key.as_str()) && !output.contains_key(key) {
            output.insert(key.clone(), value.clone());
        }
    }
}

trait EmptyStringFallback {
    fn if_empty_then(self, fallback: String) -> String;
}

impl EmptyStringFallback for String {
    fn if_empty_then(self, fallback: String) -> String {
        if self.trim().is_empty() {
            fallback
        } else {
            self
        }
    }
}
