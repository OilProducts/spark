use std::collections::BTreeMap;

use attractor_core::{RawRuntimeEvent, RunRecord};
use serde_json::{json, Value};
use spark_storage::{append_jsonl, read_jsonl, JsonLinesOptions};

use crate::error::Result;
use crate::paths::RunRootPaths;

pub fn append_event(paths: &RunRootPaths, mut event: RawRuntimeEvent) -> Result<RawRuntimeEvent> {
    for reserved in ["sequence", "type", "run_id", "emitted_at"] {
        event.payload.remove(reserved);
    }
    if event.run_id.trim().is_empty() {
        event.run_id = paths.run_id.clone();
    }
    if event.emitted_at.trim().is_empty() {
        event.emitted_at = utc_timestamp();
    }
    if !event.payload.contains_key("source_scope") {
        event.payload.insert(
            "source_scope".to_string(),
            Value::String("root".to_string()),
        );
    }
    if event.sequence.is_none() {
        event.sequence = Some(next_sequence(paths)?);
    }
    append_jsonl(paths.events_jsonl(), &event)?;
    Ok(event)
}

pub fn read_raw_events(paths: &RunRootPaths) -> Result<Vec<RawRuntimeEvent>> {
    if !paths.events_jsonl().exists() {
        return Ok(Vec::new());
    }
    Ok(read_jsonl(
        paths.events_jsonl(),
        JsonLinesOptions::allow_blank_lines(),
    )?)
}

pub fn next_sequence(paths: &RunRootPaths) -> Result<u64> {
    if let Some(sequence) = latest_sequence_from_tail(paths)? {
        return Ok(sequence + 1);
    }
    Ok(read_raw_events(paths)?
        .into_iter()
        .filter_map(|event| event.sequence)
        .max()
        .unwrap_or(0)
        + 1)
}

/// Window read from the end of events.jsonl large enough to cover the last
/// complete event line without rereading the whole log.
const SEQUENCE_TAIL_WINDOW_BYTES: u64 = 1 << 20;

/// Sequences are assigned in append order, so the last complete event line
/// holds the maximum. Reading just the file tail keeps appends O(1) instead
/// of reparsing the entire log, which dominated runtime on long runs. Returns
/// None when no parseable line with a sequence falls inside the tail window
/// (caller falls back to the full scan).
fn latest_sequence_from_tail(paths: &RunRootPaths) -> Result<Option<u64>> {
    Ok(latest_event_from_tail(paths)?.and_then(|event| event.sequence))
}

/// The last complete event within the tail window, without reading the rest
/// of the log. Because every appended event carries a dense sequence, its
/// sequence also equals the event count. Returns None for a missing or
/// wholly unparseable tail (caller decides whether a full scan is worth it).
pub fn latest_event_from_tail(paths: &RunRootPaths) -> Result<Option<RawRuntimeEvent>> {
    use std::io::{Read, Seek, SeekFrom};

    let path = paths.events_jsonl();
    if !path.exists() {
        return Ok(None);
    }
    let io_error = |operation: &'static str, source: std::io::Error| {
        crate::error::RuntimeStorageError::io(operation, &path, source)
    };
    let mut file = std::fs::File::open(&path).map_err(|source| io_error("open events", source))?;
    let length = file
        .metadata()
        .map_err(|source| io_error("stat events", source))?
        .len();
    let start = length.saturating_sub(SEQUENCE_TAIL_WINDOW_BYTES);
    file.seek(SeekFrom::Start(start))
        .map_err(|source| io_error("seek events", source))?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail)
        .map_err(|source| io_error("read events", source))?;
    let tail = String::from_utf8_lossy(&tail);
    for line in tail.lines().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Unparseable lines are either a partial trailing write or the
        // truncated first line of the tail window; keep scanning back.
        let Ok(event) = serde_json::from_str::<RawRuntimeEvent>(line) else {
            continue;
        };
        return Ok(Some(event));
    }
    Ok(None)
}

/// Parses events appended after `offset`, returning them with the offset of
/// the end of the last complete line consumed. Partial trailing writes stay
/// unconsumed so the next call picks them up once completed. A shrunken file
/// (offset beyond the current length) returns `None`, signalling the caller
/// to fall back to a full rebuild.
pub fn read_raw_events_after(
    paths: &RunRootPaths,
    offset: u64,
) -> Result<Option<(Vec<RawRuntimeEvent>, u64)>> {
    use std::io::{Read, Seek, SeekFrom};

    let path = paths.events_jsonl();
    if !path.exists() {
        return Ok(if offset == 0 {
            Some((Vec::new(), 0))
        } else {
            None
        });
    }
    let io_error = |operation: &'static str, source: std::io::Error| {
        crate::error::RuntimeStorageError::io(operation, &path, source)
    };
    let mut file = std::fs::File::open(&path).map_err(|source| io_error("open events", source))?;
    let length = file
        .metadata()
        .map_err(|source| io_error("stat events", source))?
        .len();
    if offset > length {
        return Ok(None);
    }
    if offset == length {
        return Ok(Some((Vec::new(), offset)));
    }
    file.seek(SeekFrom::Start(offset))
        .map_err(|source| io_error("seek events", source))?;
    let mut appended = Vec::new();
    file.read_to_end(&mut appended)
        .map_err(|source| io_error("read events", source))?;
    let mut events = Vec::new();
    let mut consumed = offset;
    let mut line_start = 0usize;
    while let Some(newline) = appended[line_start..]
        .iter()
        .position(|byte| *byte == b'\n')
    {
        let line_end = line_start + newline;
        let line = String::from_utf8_lossy(&appended[line_start..line_end]);
        let line = line.trim();
        if !line.is_empty() {
            let event: RawRuntimeEvent = serde_json::from_str(line)
                .map_err(|source| crate::error::RuntimeStorageError::json(&path, source))?;
            events.push(event);
        }
        line_start = line_end + 1;
        consumed = offset + line_start as u64;
    }
    Ok(Some((events, consumed)))
}

pub fn lifecycle_event(run_id: impl Into<String>, phase: impl Into<String>) -> RawRuntimeEvent {
    event_with_payload(run_id, "lifecycle", [("phase", json!(phase.into()))])
}

pub fn runtime_status_event(
    run_id: impl Into<String>,
    status: impl Into<String>,
    outcome: Option<String>,
    reason_code: Option<String>,
    reason_message: Option<String>,
    last_error: Option<String>,
) -> RawRuntimeEvent {
    event_with_payload(
        run_id,
        "runtime",
        [
            ("status", json!(status.into())),
            ("outcome", option_json(outcome)),
            ("outcome_reason_code", option_json(reason_code)),
            ("outcome_reason_message", option_json(reason_message)),
            ("last_error", option_json(last_error)),
        ],
    )
}

pub fn run_metadata_event(record: &RunRecord) -> RawRuntimeEvent {
    let mut event = RawRuntimeEvent::new("run_meta", record.run_id.clone());
    let value = serde_json::to_value(record).unwrap_or_else(|_| json!({}));
    if let Some(object) = value.as_object() {
        event.payload.extend(
            object
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<BTreeMap<_, _>>(),
        );
    }
    event
}

pub fn run_metadata_event_with_graph_paths(
    record: &RunRecord,
    paths: &RunRootPaths,
) -> RawRuntimeEvent {
    let mut event = run_metadata_event(record);
    let flow_dir = paths.artifacts_dir().join("flow");
    let flow_source_path = flow_dir.join("flow-source.yaml");
    let flow_definition_path = flow_dir.join("flow-definition.json");
    event.payload.insert(
        "flow_source_path".to_string(),
        path_value_if_exists(flow_source_path),
    );
    event.payload.insert(
        "flow_definition_path".to_string(),
        path_value_if_exists(flow_definition_path),
    );
    event
}

pub fn human_gate_pending_event(
    run_id: impl Into<String>,
    question_id: impl Into<String>,
    node_id: impl Into<String>,
    flow_name: impl Into<String>,
    prompt: impl Into<String>,
    details: Option<String>,
    options: Vec<Value>,
) -> RawRuntimeEvent {
    let run_id = run_id.into();
    event_with_payload(
        run_id.clone(),
        "human_gate",
        [
            ("question_id", json!(question_id.into())),
            ("run_id", json!(run_id)),
            ("node_id", json!(node_id.into())),
            ("flow_name", json!(flow_name.into())),
            ("prompt", json!(prompt.into())),
            ("details", details.map(Value::String).unwrap_or(Value::Null)),
            ("options", Value::Array(options)),
            ("answer", Value::Null),
        ],
    )
}

pub fn human_gate_answered_event(
    run_id: impl Into<String>,
    question_id: impl Into<String>,
    node_id: Option<String>,
    flow_name: Option<String>,
    prompt: Option<String>,
    answer: impl Into<String>,
    note: Option<String>,
) -> RawRuntimeEvent {
    let run_id = run_id.into();
    event_with_payload(
        run_id,
        "InterviewCompleted",
        [
            ("question_id", json!(question_id.into())),
            ("node_id", option_json(node_id.clone())),
            ("stage", option_json(node_id)),
            ("flow_name", option_json(flow_name)),
            ("question", option_json(prompt)),
            ("answer", json!(answer.into())),
            ("note", option_json(note)),
            ("outcome_provenance", json!("accepted")),
        ],
    )
}

pub fn child_run_started_event(
    parent_run_id: impl Into<String>,
    child_run_id: impl Into<String>,
    parent_node_id: impl Into<String>,
    root_run_id: impl Into<String>,
    child_flow_name: impl Into<String>,
) -> RawRuntimeEvent {
    let parent_run_id = parent_run_id.into();
    event_with_payload(
        parent_run_id.clone(),
        "ChildRunStarted",
        [
            ("child_run_id", json!(child_run_id.into())),
            ("parent_run_id", json!(parent_run_id)),
            ("parent_node_id", json!(parent_node_id.into())),
            ("root_run_id", json!(root_run_id.into())),
            ("child_flow_name", json!(child_flow_name.into())),
        ],
    )
}

#[allow(clippy::too_many_arguments)]
pub fn child_run_completed_event(
    parent_run_id: impl Into<String>,
    child_run_id: impl Into<String>,
    parent_node_id: impl Into<String>,
    root_run_id: impl Into<String>,
    child_flow_name: impl Into<String>,
    status: impl Into<String>,
    outcome: Option<String>,
    reason_code: Option<String>,
    reason_message: Option<String>,
    failure_reason: Option<String>,
) -> RawRuntimeEvent {
    let parent_run_id = parent_run_id.into();
    event_with_payload(
        parent_run_id.clone(),
        "ChildRunCompleted",
        [
            ("child_run_id", json!(child_run_id.into())),
            ("parent_run_id", json!(parent_run_id)),
            ("parent_node_id", json!(parent_node_id.into())),
            ("root_run_id", json!(root_run_id.into())),
            ("child_flow_name", json!(child_flow_name.into())),
            ("status", json!(status.into())),
            ("outcome", option_json(outcome)),
            ("outcome_reason_code", option_json(reason_code)),
            ("outcome_reason_message", option_json(reason_message)),
            ("failure_reason", option_json(failure_reason)),
        ],
    )
}

#[allow(clippy::too_many_arguments)]
pub fn child_intervention_requested_event(
    parent_run_id: impl Into<String>,
    child_run_id: impl Into<String>,
    parent_node_id: impl Into<String>,
    root_run_id: impl Into<String>,
    target_node_id: Option<String>,
    status: impl Into<String>,
    delivery_mode: impl Into<String>,
    reason: impl Into<String>,
    child_failure_reason: impl Into<String>,
    message: Option<String>,
) -> RawRuntimeEvent {
    let parent_run_id = parent_run_id.into();
    event_with_payload(
        parent_run_id.clone(),
        "ChildInterventionRequested",
        [
            ("child_run_id", json!(child_run_id.into())),
            ("parent_run_id", json!(parent_run_id)),
            ("parent_node_id", json!(parent_node_id.into())),
            ("root_run_id", json!(root_run_id.into())),
            ("target_node_id", option_json(target_node_id)),
            ("status", json!(status.into())),
            ("delivery_mode", json!(delivery_mode.into())),
            ("reason", json!(reason.into())),
            ("child_failure_reason", json!(child_failure_reason.into())),
            ("message", option_json(message)),
        ],
    )
}

#[allow(clippy::too_many_arguments)]
pub fn human_intervention_requested_event(
    parent_run_id: impl Into<String>,
    target_run_id: impl Into<String>,
    target_node_id: Option<String>,
    message: impl Into<String>,
    intervention_status: impl Into<String>,
    intervention_delivery_mode: impl Into<String>,
    intervention_reason: impl Into<String>,
    result: Value,
) -> RawRuntimeEvent {
    let parent_run_id = parent_run_id.into();
    event_with_payload(
        parent_run_id.clone(),
        "HumanInterventionRequested",
        [
            ("parent_run_id", json!(parent_run_id)),
            ("target_run_id", json!(target_run_id.into())),
            ("target_node_id", option_json(target_node_id)),
            ("message", json!(message.into())),
            ("intervention_status", json!(intervention_status.into())),
            (
                "intervention_delivery_mode",
                json!(intervention_delivery_mode.into()),
            ),
            ("intervention_reason", json!(intervention_reason.into())),
            ("result", result),
        ],
    )
}

pub fn log_event(run_id: impl Into<String>, message: impl Into<String>) -> RawRuntimeEvent {
    event_with_payload(run_id, "log", [("msg", json!(message.into()))])
}

pub fn cleanup_error_event(
    run_id: impl Into<String>,
    message: impl Into<String>,
) -> RawRuntimeEvent {
    event_with_payload(
        run_id,
        "cleanup_error",
        [
            ("type", json!("cleanup_error")),
            ("message", json!(message.into())),
        ],
    )
}

pub fn pipeline_started_event(
    run_id: impl Into<String>,
    graph_id: impl Into<String>,
    current_node: impl Into<String>,
    resumed: bool,
) -> RawRuntimeEvent {
    let graph_id = graph_id.into();
    event_with_payload(
        run_id,
        "PipelineStarted",
        [
            ("id", json!(graph_id.clone())),
            ("name", json!(graph_id)),
            ("current_node", json!(current_node.into())),
            ("resumed", json!(resumed)),
        ],
    )
}

pub fn pipeline_completed_event(
    run_id: impl Into<String>,
    current_node: impl Into<String>,
    status: impl Into<String>,
    outcome: Option<String>,
    artifact_count: usize,
) -> RawRuntimeEvent {
    event_with_payload(
        run_id,
        "PipelineCompleted",
        [
            ("current_node", json!(current_node.into())),
            ("status", json!(status.into())),
            ("outcome", option_json(outcome)),
            ("artifact_count", json!(artifact_count)),
        ],
    )
}

pub fn pipeline_completed_event_with_reasons(
    run_id: impl Into<String>,
    current_node: impl Into<String>,
    status: impl Into<String>,
    outcome: Option<String>,
    outcome_reason_code: Option<String>,
    outcome_reason_message: Option<String>,
    artifact_count: usize,
) -> RawRuntimeEvent {
    event_with_payload(
        run_id,
        "PipelineCompleted",
        [
            ("current_node", json!(current_node.into())),
            ("status", json!(status.into())),
            ("outcome", option_json(outcome)),
            ("outcome_reason_code", option_json(outcome_reason_code)),
            (
                "outcome_reason_message",
                option_json(outcome_reason_message),
            ),
            ("artifact_count", json!(artifact_count)),
        ],
    )
}

pub fn pipeline_failed_event(
    run_id: impl Into<String>,
    current_node: impl Into<String>,
    error: impl Into<String>,
    artifact_count: usize,
) -> RawRuntimeEvent {
    event_with_payload(
        run_id,
        "PipelineFailed",
        [
            ("current_node", json!(current_node.into())),
            ("status", json!("failed")),
            ("error", json!(error.into())),
            ("artifact_count", json!(artifact_count)),
        ],
    )
}

pub fn pipeline_retry_started_event(
    run_id: impl Into<String>,
    current_node: impl Into<String>,
    completed_nodes: Vec<String>,
) -> RawRuntimeEvent {
    event_with_payload(
        run_id,
        "PipelineRetryStarted",
        [
            ("current_node", json!(current_node.into())),
            ("completed_nodes", json!(completed_nodes)),
        ],
    )
}

pub fn pipeline_retry_completed_event(
    run_id: impl Into<String>,
    status: impl Into<String>,
    last_error: Option<String>,
) -> RawRuntimeEvent {
    event_with_payload(
        run_id,
        "PipelineRetryCompleted",
        [
            ("status", json!(status.into())),
            ("last_error", option_json(last_error)),
        ],
    )
}

pub fn pipeline_paused_event(
    run_id: impl Into<String>,
    current_node: impl Into<String>,
) -> RawRuntimeEvent {
    event_with_payload(
        run_id,
        "PipelinePaused",
        [
            ("current_node", json!(current_node.into())),
            ("status", json!("paused")),
        ],
    )
}

pub fn cancel_requested_event(run_id: impl Into<String>) -> RawRuntimeEvent {
    event_with_payload(
        run_id,
        "CancelRequested",
        [
            ("status", json!("cancel_requested")),
            ("last_error", json!("cancel_requested_by_user")),
        ],
    )
}

pub fn stage_started_event(
    run_id: impl Into<String>,
    index: u64,
    node_id: impl Into<String>,
) -> RawRuntimeEvent {
    let node_id = node_id.into();
    event_with_payload(
        run_id,
        "StageStarted",
        [
            ("index", json!(index)),
            ("name", json!(node_id.clone())),
            ("node_id", json!(node_id)),
        ],
    )
}

pub fn stage_completed_event(
    run_id: impl Into<String>,
    index: u64,
    node_id: impl Into<String>,
    outcome: impl Into<String>,
) -> RawRuntimeEvent {
    stage_completed_event_with_notes(run_id, index, node_id, outcome, None)
}

/// Notes carry the outcome's human-readable result (tool stdout, completion
/// summaries) into the journal, truncated so chatty tools cannot bloat it.
pub fn stage_completed_event_with_notes(
    run_id: impl Into<String>,
    index: u64,
    node_id: impl Into<String>,
    outcome: impl Into<String>,
    notes: Option<&str>,
) -> RawRuntimeEvent {
    const STAGE_NOTES_LIMIT: usize = 200;
    let node_id = node_id.into();
    let mut event = event_with_payload(
        run_id,
        "StageCompleted",
        [
            ("index", json!(index)),
            ("name", json!(node_id.clone())),
            ("node_id", json!(node_id)),
            ("outcome", json!(outcome.into())),
        ],
    );
    if let Some(notes) = notes.map(str::trim).filter(|notes| !notes.is_empty()) {
        let truncated: String = notes.chars().take(STAGE_NOTES_LIMIT).collect();
        event.payload.insert("notes".to_string(), json!(truncated));
    }
    event
}

pub fn stage_failed_event(
    run_id: impl Into<String>,
    index: u64,
    node_id: impl Into<String>,
    error: impl Into<String>,
    will_retry: bool,
    attempt: Option<u64>,
) -> RawRuntimeEvent {
    let node_id = node_id.into();
    let mut event = event_with_payload(
        run_id,
        "StageFailed",
        [
            ("index", json!(index)),
            ("name", json!(node_id.clone())),
            ("node_id", json!(node_id)),
            ("error", json!(error.into())),
            ("will_retry", json!(will_retry)),
        ],
    );
    if let Some(attempt) = attempt {
        event.payload.insert("attempt".to_string(), json!(attempt));
    }
    event
}

pub fn stage_retrying_event(
    run_id: impl Into<String>,
    index: u64,
    node_id: impl Into<String>,
    attempt: u64,
    delay_ms: f64,
) -> RawRuntimeEvent {
    let node_id = node_id.into();
    event_with_payload(
        run_id,
        "StageRetrying",
        [
            ("index", json!(index)),
            ("name", json!(node_id.clone())),
            ("node_id", json!(node_id)),
            ("attempt", json!(attempt)),
            ("delay", json!(delay_ms)),
        ],
    )
}

pub fn state_event(
    run_id: impl Into<String>,
    node_id: impl Into<String>,
    status: impl Into<String>,
) -> RawRuntimeEvent {
    event_with_payload(
        run_id,
        "state",
        [
            ("node", json!(node_id.into())),
            ("status", json!(status.into())),
        ],
    )
}

pub fn llm_request_started_event(
    run_id: impl Into<String>,
    node_id: impl Into<String>,
    payload: Value,
) -> RawRuntimeEvent {
    event_with_payload(
        run_id,
        "LLMRequestStarted",
        [("node_id", json!(node_id.into())), ("payload", payload)],
    )
}

pub fn codergen_adapter_event(
    run_id: impl Into<String>,
    node_id: impl Into<String>,
    adapter_event_type: impl Into<String>,
    payload: Value,
) -> RawRuntimeEvent {
    event_with_payload(
        run_id,
        "CodergenAdapter",
        [
            ("node_id", json!(node_id.into())),
            ("adapter_event_type", json!(adapter_event_type.into())),
            ("payload", payload),
        ],
    )
}

pub fn llm_request_completed_event(
    run_id: impl Into<String>,
    node_id: impl Into<String>,
    payload: Value,
) -> RawRuntimeEvent {
    event_with_payload(
        run_id,
        "LLMRequestCompleted",
        [("node_id", json!(node_id.into())), ("payload", payload)],
    )
}

pub fn llm_token_usage_event(
    run_id: impl Into<String>,
    node_id: impl Into<String>,
    token_usage: Value,
) -> RawRuntimeEvent {
    event_with_payload(
        run_id,
        "LLMTokenUsage",
        [
            ("node_id", json!(node_id.into())),
            ("token_usage", token_usage),
        ],
    )
}

pub fn checkpoint_saved_event(
    run_id: impl Into<String>,
    current_node: impl Into<String>,
    completed_nodes: Vec<String>,
) -> RawRuntimeEvent {
    event_with_payload(
        run_id,
        "CheckpointSaved",
        [
            ("current_node", json!(current_node.into())),
            ("completed_nodes", json!(completed_nodes)),
            ("persisted", json!(true)),
        ],
    )
}

pub fn interview_started_event(
    run_id: impl Into<String>,
    node_id: impl Into<String>,
    question: impl Into<String>,
) -> RawRuntimeEvent {
    let node_id = node_id.into();
    event_with_payload(
        run_id,
        "InterviewStarted",
        [
            ("stage", json!(node_id.clone())),
            ("node_id", json!(node_id)),
            ("question", json!(question.into())),
        ],
    )
}

pub fn interview_completed_event(
    run_id: impl Into<String>,
    node_id: impl Into<String>,
    question: impl Into<String>,
    answer: impl Into<String>,
    outcome_provenance: impl Into<String>,
) -> RawRuntimeEvent {
    let node_id = node_id.into();
    event_with_payload(
        run_id,
        "InterviewCompleted",
        [
            ("stage", json!(node_id.clone())),
            ("node_id", json!(node_id)),
            ("question", json!(question.into())),
            ("answer", json!(answer.into())),
            ("outcome_provenance", json!(outcome_provenance.into())),
        ],
    )
}

pub fn parallel_started_event(
    run_id: impl Into<String>,
    node_id: impl Into<String>,
    branch_count: usize,
) -> RawRuntimeEvent {
    let node_id = node_id.into();
    event_with_payload(
        run_id,
        "ParallelStarted",
        [
            ("node_id", json!(node_id)),
            ("branch_count", json!(branch_count)),
        ],
    )
}

pub fn parallel_branch_started_event(
    run_id: impl Into<String>,
    node_id: impl Into<String>,
    branch: impl Into<String>,
    index: usize,
) -> RawRuntimeEvent {
    let node_id = node_id.into();
    event_with_payload(
        run_id,
        "ParallelBranchStarted",
        [
            ("node_id", json!(node_id)),
            ("branch", json!(branch.into())),
            ("index", json!(index)),
        ],
    )
}

pub fn parallel_branch_completed_event(
    run_id: impl Into<String>,
    node_id: impl Into<String>,
    branch: impl Into<String>,
    index: usize,
    success: bool,
) -> RawRuntimeEvent {
    let node_id = node_id.into();
    event_with_payload(
        run_id,
        "ParallelBranchCompleted",
        [
            ("node_id", json!(node_id)),
            ("branch", json!(branch.into())),
            ("index", json!(index)),
            ("success", json!(success)),
        ],
    )
}

pub fn parallel_completed_event(
    run_id: impl Into<String>,
    node_id: impl Into<String>,
    success_count: usize,
    failure_count: usize,
) -> RawRuntimeEvent {
    let node_id = node_id.into();
    event_with_payload(
        run_id,
        "ParallelCompleted",
        [
            ("node_id", json!(node_id)),
            ("success_count", json!(success_count)),
            ("failure_count", json!(failure_count)),
        ],
    )
}

fn event_with_payload<const N: usize>(
    run_id: impl Into<String>,
    event_type: impl Into<String>,
    entries: [(&'static str, Value); N],
) -> RawRuntimeEvent {
    let mut event = RawRuntimeEvent::new(event_type, run_id);
    event.payload = entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect();
    event
}

fn option_json(value: Option<String>) -> Value {
    value.map(Value::String).unwrap_or(Value::Null)
}

fn path_value_if_exists(path: std::path::PathBuf) -> Value {
    if path.exists() {
        json!(path.to_string_lossy().to_string())
    } else {
        Value::Null
    }
}

pub(crate) fn utc_timestamp() -> String {
    format_utc_timestamp(time::OffsetDateTime::now_utc())
}

fn format_utc_timestamp(value: time::OffsetDateTime) -> String {
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:09}Z",
        value.year(),
        u8::from(value.month()),
        value.day(),
        value.hour(),
        value.minute(),
        value.second(),
        value.nanosecond(),
    )
}

#[cfg(test)]
mod tests {
    use time::{Date, Month, PrimitiveDateTime, Time};

    use super::format_utc_timestamp;

    #[test]
    fn format_utc_timestamp_uses_fixed_nanosecond_width() {
        let date = Date::from_calendar_date(2026, Month::June, 28).unwrap();
        let time = Time::from_hms_nano(1, 2, 3, 40_500_000).unwrap();
        let timestamp = PrimitiveDateTime::new(date, time).assume_utc();

        assert_eq!(
            format_utc_timestamp(timestamp),
            "2026-06-28T01:02:03.040500000Z"
        );
    }
}
