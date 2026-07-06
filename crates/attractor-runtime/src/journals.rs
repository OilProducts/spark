use attractor_core::{JournalEntry, RawRuntimeEvent};
use serde_json::Value;

pub fn journal_entries_from_events(events: &[RawRuntimeEvent]) -> Vec<JournalEntry> {
    let mut entries = events
        .iter()
        .filter_map(journal_entry_from_event)
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| right.sequence.cmp(&left.sequence));
    entries
}

pub fn journal_entry_from_event(event: &RawRuntimeEvent) -> Option<JournalEntry> {
    let sequence = event.sequence?;
    let emitted_at = non_empty(&event.emitted_at)?;
    let raw_type = non_empty(&event.event_type)?;
    let payload = serde_json::to_value(event).ok()?;
    let node_id = journal_node_id(event, &raw_type);
    let stage_index = numeric_payload(event, "index");
    let source_scope = match string_payload(event, "source_scope").as_deref() {
        Some("child") => "child".to_string(),
        _ => "root".to_string(),
    };
    let source_parent_node_id = string_payload(event, "source_parent_node_id");
    let source_flow_name = string_payload(event, "source_flow_name");
    let question_id = string_payload(event, "question_id");
    Some(JournalEntry {
        id: format!("journal-{sequence}"),
        sequence,
        emitted_at,
        kind: journal_kind(&raw_type),
        raw_type: raw_type.clone(),
        severity: journal_severity(&raw_type, event),
        summary: journal_summary(
            &raw_type,
            event,
            node_id.as_deref(),
            &source_scope,
            source_parent_node_id.as_deref(),
            source_flow_name.as_deref(),
        ),
        node_id,
        stage_index,
        source_scope,
        source_parent_node_id,
        source_flow_name,
        question_id,
        payload,
    })
}

fn journal_node_id(event: &RawRuntimeEvent, raw_type: &str) -> Option<String> {
    if raw_type == "CheckpointSaved" {
        return string_payload(event, "current_node");
    }
    ["node_id", "node", "name", "stage", "current_node"]
        .into_iter()
        .find_map(|key| string_payload(event, key))
}

fn journal_kind(raw_type: &str) -> String {
    if raw_type == "log" {
        "log"
    } else if matches!(raw_type, "runtime" | "state" | "LLMContent") {
        raw_type
    } else if raw_type.starts_with("LLM") {
        "runtime"
    } else if raw_type == "run_meta" {
        "metadata"
    } else if matches!(
        raw_type,
        "PipelineStarted"
            | "PipelineCompleted"
            | "PipelineFailed"
            | "PipelineRetryStarted"
            | "PipelineRetryCompleted"
            | "PipelinePaused"
            | "CancelRequested"
            | "ChildRunStarted"
            | "ChildRunCompleted"
            | "lifecycle"
    ) {
        "lifecycle"
    } else if raw_type.starts_with("Stage") {
        "stage"
    } else if raw_type.starts_with("Parallel") {
        "parallel"
    } else if matches!(
        raw_type,
        "ChildInterventionRequested" | "HumanInterventionRequested"
    ) {
        "intervention"
    } else if raw_type.starts_with("Interview") || raw_type == "human_gate" {
        "interview"
    } else if raw_type == "CheckpointSaved" {
        "checkpoint"
    } else {
        "other"
    }
    .to_string()
}

fn journal_severity(raw_type: &str, event: &RawRuntimeEvent) -> String {
    for key in ["severity", "level"] {
        if let Some(value) = string_payload(event, key) {
            let normalized = value.to_lowercase();
            if matches!(normalized.as_str(), "info" | "warning" | "error") {
                return normalized;
            }
        }
    }
    if raw_type == "log" {
        let message = string_payload(event, "msg")
            .unwrap_or_default()
            .to_lowercase();
        if message.contains("error") || message.contains("fail") {
            return "error".to_string();
        }
        if message.contains("warn") {
            return "warning".to_string();
        }
        return "info".to_string();
    }
    if matches!(raw_type, "PipelineFailed" | "StageFailed") {
        return "error".to_string();
    }
    if matches!(
        raw_type,
        "StageRetrying" | "PipelinePaused" | "CancelRequested" | "PipelineRetryStarted"
    ) {
        return "warning".to_string();
    }
    if raw_type == "PipelineCompleted"
        && string_payload(event, "outcome").as_deref() == Some("failure")
    {
        return "warning".to_string();
    }
    if matches!(
        raw_type,
        "ChildInterventionRequested" | "HumanInterventionRequested"
    ) {
        let status = string_payload(event, "status")
            .or_else(|| string_payload(event, "intervention_status"));
        return if status.as_deref() == Some("delivered") {
            "info".to_string()
        } else {
            "warning".to_string()
        };
    }
    if raw_type == "runtime" {
        if let Some(status) = string_payload(event, "status") {
            if matches!(status.as_str(), "failed" | "validation_error") {
                return "error".to_string();
            }
            if matches!(
                status.as_str(),
                "cancel_requested" | "abort_requested" | "canceled" | "aborted"
            ) {
                return "warning".to_string();
            }
        }
    }
    "info".to_string()
}

fn journal_summary(
    raw_type: &str,
    event: &RawRuntimeEvent,
    node_id: Option<&str>,
    source_scope: &str,
    source_parent_node_id: Option<&str>,
    source_flow_name: Option<&str>,
) -> String {
    let source_prefix = source_prefix(source_scope, source_parent_node_id, source_flow_name);
    match raw_type {
        "log" => string_payload(event, "msg").unwrap_or_else(|| "Log entry".to_string()),
        "lifecycle" => string_payload(event, "phase")
            .map(|phase| format!("{source_prefix}Lifecycle phase: {phase}"))
            .unwrap_or_else(|| format!("{source_prefix}Lifecycle event")),
        "runtime" => runtime_summary(event, &source_prefix),
        "state" => {
            let state_node = string_payload(event, "node")
                .or_else(|| node_id.map(str::to_string))
                .unwrap_or_else(|| "unknown".to_string());
            let status = string_payload(event, "status").unwrap_or_else(|| "updated".to_string());
            format!("{source_prefix}Node {state_node} status: {status}")
        }
        "run_meta" => string_payload(event, "flow_name")
            .map(|flow_name| format!("{source_prefix}Run metadata captured for {flow_name}"))
            .unwrap_or_else(|| format!("{source_prefix}Run metadata captured")),
        "PipelineStarted" => {
            format!(
                "{source_prefix}Pipeline started at {}",
                node_id.unwrap_or("start")
            )
        }
        "PipelineCompleted" => pipeline_completed_summary(event, node_id, &source_prefix),
        "PipelineFailed" => string_payload(event, "error")
            .map(|error| format!("{source_prefix}Pipeline failed: {error}"))
            .unwrap_or_else(|| format!("{source_prefix}Pipeline failed")),
        "PipelineRetryStarted" => {
            let retry_node = string_payload(event, "current_node")
                .or_else(|| node_id.map(str::to_string))
                .unwrap_or_else(|| "checkpoint".to_string());
            format!("{source_prefix}Retry started from {retry_node}")
        }
        "PipelineRetryCompleted" => string_payload(event, "status")
            .map(|status| format!("{source_prefix}Retry completed ({status})"))
            .unwrap_or_else(|| format!("{source_prefix}Retry completed")),
        "PipelinePaused" => {
            let current = string_payload(event, "current_node")
                .or_else(|| node_id.map(str::to_string))
                .unwrap_or_else(|| "checkpoint".to_string());
            format!("{source_prefix}Pipeline paused at {current}")
        }
        "CancelRequested" => format!("{source_prefix}Cancel requested"),
        "ChildRunStarted" => {
            let label = string_payload(event, "child_flow_name")
                .or_else(|| string_payload(event, "child_run_id"))
                .unwrap_or_else(|| "child run".to_string());
            format!("{source_prefix}Child run started: {label}")
        }
        "ChildRunCompleted" => {
            let label = string_payload(event, "child_flow_name")
                .or_else(|| string_payload(event, "child_run_id"))
                .unwrap_or_else(|| "child run".to_string());
            string_payload(event, "status")
                .map(|status| format!("{source_prefix}Child run completed: {label} ({status})"))
                .unwrap_or_else(|| format!("{source_prefix}Child run completed: {label}"))
        }
        "ChildInterventionRequested" => {
            let child_run_id =
                string_payload(event, "child_run_id").unwrap_or_else(|| "child run".to_string());
            let status = string_payload(event, "status").unwrap_or_else(|| "requested".to_string());
            let reason = string_payload(event, "reason");
            if status == "delivered" {
                format!("{source_prefix}Child intervention delivered for {child_run_id}")
            } else if let Some(reason) = reason {
                format!("{source_prefix}Child intervention {status} for {child_run_id}: {reason}")
            } else {
                format!("{source_prefix}Child intervention {status} for {child_run_id}")
            }
        }
        "HumanInterventionRequested" => {
            let target_run_id =
                string_payload(event, "target_run_id").unwrap_or_else(|| "run".to_string());
            let status = string_payload(event, "intervention_status")
                .or_else(|| string_payload(event, "status"))
                .unwrap_or_else(|| "requested".to_string());
            let reason = string_payload(event, "intervention_reason")
                .or_else(|| string_payload(event, "reason"));
            if let Some(reason) = reason {
                format!("{source_prefix}Human intervention {status} for {target_run_id}: {reason}")
            } else {
                format!("{source_prefix}Human intervention {status} for {target_run_id}")
            }
        }
        "StageStarted" => {
            format!(
                "{source_prefix}Stage {} started",
                node_id.unwrap_or("unknown")
            )
        }
        "StageCompleted" => string_payload(event, "outcome")
            .map(|outcome| {
                format!(
                    "{source_prefix}Stage {} completed ({outcome})",
                    node_id.unwrap_or("unknown")
                )
            })
            .unwrap_or_else(|| {
                format!(
                    "{source_prefix}Stage {} completed",
                    node_id.unwrap_or("unknown")
                )
            }),
        "StageFailed" => string_payload(event, "error")
            .map(|error| {
                format!(
                    "{source_prefix}Stage {} failed: {error}",
                    node_id.unwrap_or("unknown")
                )
            })
            .unwrap_or_else(|| {
                format!(
                    "{source_prefix}Stage {} failed",
                    node_id.unwrap_or("unknown")
                )
            }),
        "StageRetrying" => numeric_payload(event, "attempt")
            .map(|attempt| {
                format!(
                    "{source_prefix}Stage {} retrying (attempt {attempt})",
                    node_id.unwrap_or("unknown")
                )
            })
            .unwrap_or_else(|| {
                format!(
                    "{source_prefix}Stage {} retrying",
                    node_id.unwrap_or("unknown")
                )
            }),
        "LLMRequestStarted" => {
            format!(
                "{source_prefix}LLM request started for {}",
                node_id.unwrap_or("unknown")
            )
        }
        "LLMRequestCompleted" => {
            format!(
                "{source_prefix}LLM request completed for {}",
                node_id.unwrap_or("unknown")
            )
        }
        "LLMTokenUsage" => {
            format!(
                "{source_prefix}LLM token usage updated for {}",
                node_id.unwrap_or("unknown")
            )
        }
        "ParallelStarted" => numeric_payload(event, "branch_count")
            .map(|count| format!("{source_prefix}Parallel fan-out started ({count} branches)"))
            .unwrap_or_else(|| format!("{source_prefix}Parallel fan-out started")),
        "ParallelBranchStarted" => {
            let branch = string_payload(event, "branch")
                .or_else(|| node_id.map(str::to_string))
                .unwrap_or_else(|| "unknown".to_string());
            format!("{source_prefix}Parallel branch {branch} started")
        }
        "ParallelBranchCompleted" => {
            let branch = string_payload(event, "branch")
                .or_else(|| node_id.map(str::to_string))
                .unwrap_or_else(|| "unknown".to_string());
            let success = event.payload.get("success").and_then(Value::as_bool);
            match success {
                Some(true) => {
                    format!("{source_prefix}Parallel branch {branch} completed (success)")
                }
                Some(false) => {
                    format!("{source_prefix}Parallel branch {branch} completed (failed)")
                }
                None => format!("{source_prefix}Parallel branch {branch} completed"),
            }
        }
        "ParallelCompleted" => match (
            numeric_payload(event, "success_count"),
            numeric_payload(event, "failure_count"),
        ) {
            (Some(success), Some(failed)) => {
                format!("{source_prefix}Parallel fan-out completed ({success} success, {failed} failed)")
            }
            _ => format!("{source_prefix}Parallel fan-out completed"),
        },
        "InterviewStarted" => {
            format!(
                "{source_prefix}Interview started for {}",
                node_id.unwrap_or("human gate")
            )
        }
        "InterviewInform" => {
            let message = string_payload(event, "message")
                .or_else(|| string_payload(event, "prompt"))
                .or_else(|| string_payload(event, "question"));
            message
                .map(|message| {
                    format!(
                        "{source_prefix}Interview info for {}: {message}",
                        node_id.unwrap_or("human gate")
                    )
                })
                .unwrap_or_else(|| {
                    format!(
                        "{source_prefix}Interview info for {}",
                        node_id.unwrap_or("human gate")
                    )
                })
        }
        "InterviewCompleted" => interview_completed_summary(event, node_id, &source_prefix),
        "human_gate" => string_payload(event, "prompt")
            .map(|prompt| format!("{source_prefix}Human gate pending: {prompt}"))
            .unwrap_or_else(|| {
                format!(
                    "{source_prefix}Human gate pending for {}",
                    node_id.unwrap_or("unknown")
                )
            }),
        "CheckpointSaved" => {
            let current =
                string_payload(event, "current_node").unwrap_or_else(|| "current node".to_string());
            format!("{source_prefix}Checkpoint saved at {current}")
        }
        "LLMContent" => {
            let channel =
                string_payload(event, "channel").unwrap_or_else(|| "assistant".to_string());
            let status = string_payload(event, "status").unwrap_or_else(|| "streaming".to_string());
            format!(
                "{source_prefix}{} output {status} for {}",
                capitalize(&channel),
                node_id.unwrap_or("current node")
            )
        }
        _ => format!(
            "{source_prefix}{}",
            if raw_type.is_empty() {
                "event"
            } else {
                raw_type
            }
        ),
    }
}

fn runtime_summary(event: &RawRuntimeEvent, source_prefix: &str) -> String {
    let Some(status) = string_payload(event, "status") else {
        return format!("{source_prefix}Run status updated");
    };
    let mut summary = format!("{source_prefix}Run status: {status}");
    if let Some(outcome) = string_payload(event, "outcome") {
        summary.push_str(&format!(" ({outcome})"));
    }
    let reason = string_payload(event, "outcome_reason_message")
        .or_else(|| string_payload(event, "outcome_reason_code"));
    if let Some(reason) = reason {
        format!("{summary}: {reason}")
    } else {
        summary
    }
}

fn pipeline_completed_summary(
    event: &RawRuntimeEvent,
    node_id: Option<&str>,
    source_prefix: &str,
) -> String {
    let node = node_id.unwrap_or("exit");
    match string_payload(event, "outcome").as_deref() {
        Some("failure") => {
            if let Some(reason) = string_payload(event, "outcome_reason_message")
                .or_else(|| string_payload(event, "outcome_reason_code"))
            {
                format!("{source_prefix}Pipeline completed at {node} (failure: {reason})")
            } else {
                format!("{source_prefix}Pipeline completed at {node} (failure)")
            }
        }
        Some(outcome) => format!("{source_prefix}Pipeline completed at {node} ({outcome})"),
        None => format!("{source_prefix}Pipeline completed at {node}"),
    }
}

fn interview_completed_summary(
    event: &RawRuntimeEvent,
    node_id: Option<&str>,
    source_prefix: &str,
) -> String {
    let node = node_id.unwrap_or("human gate");
    let answer = string_payload(event, "answer");
    let provenance = string_payload(event, "outcome_provenance")
        .or_else(|| string_payload(event, "provenance"))
        .or_else(|| {
            if event.event_type == "InterviewCompleted" {
                answer.as_deref().and_then(|answer| {
                    if answer.eq_ignore_ascii_case("skipped") {
                        Some("skipped".to_string())
                    } else if !answer.is_empty() {
                        Some("accepted".to_string())
                    } else {
                        None
                    }
                })
            } else {
                None
            }
        });
    match provenance.as_deref() {
        Some("skipped") => format!("{source_prefix}Interview completed for {node} (skipped)"),
        Some("accepted") => answer
            .map(|answer| {
                format!("{source_prefix}Interview completed for {node} (accepted answer: {answer})")
            })
            .unwrap_or_else(|| {
                format!("{source_prefix}Interview completed for {node} (accepted answer)")
            }),
        _ => answer
            .map(|answer| format!("{source_prefix}Interview completed for {node} ({answer})"))
            .unwrap_or_else(|| format!("{source_prefix}Interview completed for {node}")),
    }
}

fn source_prefix(
    source_scope: &str,
    source_parent_node_id: Option<&str>,
    source_flow_name: Option<&str>,
) -> String {
    if source_scope != "child" {
        return String::new();
    }
    let parent = source_parent_node_id.unwrap_or("parent node");
    if let Some(flow) = source_flow_name {
        format!("Child flow {flow} via {parent}: ")
    } else {
        format!("Child flow via {parent}: ")
    }
}

fn string_payload(event: &RawRuntimeEvent, key: &str) -> Option<String> {
    event.payload.get(key).and_then(value_to_non_empty_string)
}

fn numeric_payload(event: &RawRuntimeEvent, key: &str) -> Option<u64> {
    event.payload.get(key).and_then(Value::as_u64)
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn value_to_non_empty_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(value) => non_empty(value),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
