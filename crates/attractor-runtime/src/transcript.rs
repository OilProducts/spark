//! Run transcript persistence on the shared conversation record model.
//!
//! Runs reuse `spark_storage::conversation::{Transcript, TranscriptSegment}`
//! as their durable render record. Workflow boundaries are segments of kind
//! `boundary` whose run-only metadata lives in [`BoundaryMeta`], outside the
//! shared segment core. Renderable LLM output must come from this record, not
//! from the operational journal.

use std::collections::BTreeMap;

use attractor_core::RawRuntimeEvent;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use spark_agent_adapter::codergen::{CodergenEvent, CodergenExecution};
use spark_common::events::{TurnStreamChannel, TurnStreamEvent, TurnStreamEventKind};
use spark_storage::conversation::{
    assistant_segment_id, context_compaction_segment_id, plan_segment_id, reasoning_segment_id,
    request_user_input_segment_id, segment_source, tool_call_id, tool_segment_id, BoundaryMeta,
    Transcript, TranscriptSegment, SEGMENT_KIND_ASSISTANT_MESSAGE, SEGMENT_KIND_BOUNDARY,
    SEGMENT_KIND_CONTEXT_COMPACTION, SEGMENT_KIND_PLAN, SEGMENT_KIND_REASONING,
    SEGMENT_KIND_REQUEST_USER_INPUT, SEGMENT_KIND_TOOL_CALL,
};
use spark_storage::{write_json_atomic, JsonWriteOptions};

use crate::error::Result;
use crate::paths::RunRootPaths;

pub fn read_run_transcript(paths: &RunRootPaths) -> Result<Transcript> {
    let path = paths.transcript_json();
    if !path.exists() {
        return Ok(Transcript::default());
    }
    let payload: Value = spark_storage::read_json(&path)?;
    if payload.get("segments").is_some() {
        let transcript: Transcript = serde_json::from_value(payload).map_err(|source| {
            spark_storage::StorageError::JsonRead {
                path: path.clone(),
                source,
            }
        })?;
        return Ok(transcript);
    }
    Ok(legacy_transcript_from_entries(&payload))
}

pub fn persist_transcript_runtime_event(
    paths: &RunRootPaths,
    event: &RawRuntimeEvent,
) -> Result<()> {
    let Some(sequence) = event.sequence else {
        return Ok(());
    };
    let mut transcript = read_run_transcript(paths)?;
    match event.event_type.as_str() {
        "PipelineStarted" | "PipelineCompleted" | "PipelineFailed" => {
            upsert_run_boundary(&mut transcript, event, sequence);
        }
        "StageStarted" | "StageCompleted" | "StageFailed" | "StageRetrying" => {
            upsert_stage_boundary(&mut transcript, event, sequence);
        }
        "human_gate" => {
            if event
                .payload
                .get("answer")
                .is_some_and(|answer| !answer.is_null())
            {
                upsert_input_answer(&mut transcript, event, sequence);
            } else {
                upsert_input(&mut transcript, event, sequence);
            }
        }
        "InterviewCompleted" => {
            upsert_input_answer(&mut transcript, event, sequence);
        }
        "InterviewInform" => {
            let id = format!("segment-notice-journal-{sequence}");
            let content = string_payload(event, "message")
                .or_else(|| string_payload(event, "prompt"))
                .or_else(|| string_payload(event, "question"))
                .unwrap_or_else(|| "Interview update".to_string());
            let mut segment = segment_shell(
                &id,
                "run",
                sequence as i64,
                SEGMENT_KIND_CONTEXT_COMPACTION,
                "system",
                "complete",
                &event.emitted_at,
            );
            segment.content = content;
            transcript.upsert_segment(segment);
        }
        _ => {}
    }
    write_transcript(paths, &mut transcript)
}

pub fn persist_codergen_transcript(
    paths: &RunRootPaths,
    run_id: &str,
    node_id: &str,
    execution: &CodergenExecution,
) -> Result<()> {
    let mut transcript = read_run_transcript(paths)?;
    for event in &execution.events {
        persist_codergen_event(&mut transcript, run_id, node_id, event);
    }
    upsert_final_assistant_text(&mut transcript, node_id, &execution.response_text);
    write_transcript(paths, &mut transcript)
}

pub fn persist_codergen_final_response_text(
    paths: &RunRootPaths,
    node_id: &str,
    response_text: &str,
) -> Result<()> {
    let mut transcript = read_run_transcript(paths)?;
    upsert_final_assistant_text(&mut transcript, node_id, response_text);
    write_transcript(paths, &mut transcript)
}

pub fn persist_codergen_transcript_event(
    paths: &RunRootPaths,
    run_id: &str,
    node_id: &str,
    event: &CodergenEvent,
) -> Result<()> {
    let mut transcript = read_run_transcript(paths)?;
    persist_codergen_event(&mut transcript, run_id, node_id, event);
    write_transcript(paths, &mut transcript)
}

fn node_turn_id(node_id: &str) -> String {
    format!("run-node-{node_id}")
}

fn segment_shell(
    id: &str,
    turn_id: &str,
    order: i64,
    kind: &str,
    role: &str,
    status: &str,
    timestamp: &str,
) -> TranscriptSegment {
    TranscriptSegment {
        id: id.to_string(),
        turn_id: turn_id.to_string(),
        order,
        kind: kind.to_string(),
        role: role.to_string(),
        status: status.to_string(),
        timestamp: timestamp.to_string(),
        updated_at: timestamp.to_string(),
        content: String::new(),
        completed_at: None,
        error: None,
        error_code: None,
        details: None,
        phase: None,
        artifact_id: None,
        tool_call: None,
        request_user_input: None,
        source: None,
        boundary: None,
        extra: Map::new(),
    }
}

fn next_transcript_order(transcript: &Transcript) -> i64 {
    transcript
        .segments
        .iter()
        .map(|segment| segment.order)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}

fn write_transcript(paths: &RunRootPaths, transcript: &mut Transcript) -> Result<()> {
    transcript.segments.sort_by(|left, right| {
        left.order
            .cmp(&right.order)
            .then_with(|| left.id.cmp(&right.id))
    });
    write_json_atomic(
        paths.transcript_json(),
        transcript,
        JsonWriteOptions::default(),
    )?;
    Ok(())
}

fn boundary_status(event_type: &str, event: &RawRuntimeEvent) -> String {
    match event_type {
        "PipelineCompleted" => {
            string_payload(event, "status").unwrap_or_else(|| "completed".to_string())
        }
        "PipelineFailed" | "StageFailed" => "failed".to_string(),
        "StageCompleted" => "completed".to_string(),
        "StageRetrying" => "retrying".to_string(),
        _ => "running".to_string(),
    }
}

fn upsert_run_boundary(transcript: &mut Transcript, event: &RawRuntimeEvent, sequence: u64) {
    let source_scope = string_payload(event, "source_scope").unwrap_or_else(|| "root".to_string());
    let source_parent_node_id = string_payload(event, "source_parent_node_id");
    let source_flow_name = string_payload(event, "source_flow_name");
    let key = format!(
        "boundary-{}-{}-{}-run-na-na",
        source_scope,
        source_parent_node_id.as_deref().unwrap_or("root"),
        source_flow_name.as_deref().unwrap_or("")
    );
    let status = boundary_status(event.event_type.as_str(), event);
    let summary = match event.event_type.as_str() {
        "PipelineCompleted" => {
            let outcome = string_payload(event, "outcome").unwrap_or_else(|| status.clone());
            format!("Run completed: {outcome}")
        }
        "PipelineFailed" => string_payload(event, "error")
            .map(|error| format!("Run failed: {error}"))
            .unwrap_or_else(|| "Run failed".to_string()),
        _ => string_payload(event, "name")
            .map(|name| format!("Run {name} started"))
            .unwrap_or_else(|| "Run started".to_string()),
    };
    let previous = transcript.find_segment(&key).cloned();
    let previous_meta = previous
        .as_ref()
        .and_then(|segment| segment.boundary.clone());
    let started_at = if event.event_type == "PipelineStarted" {
        Some(event.emitted_at.clone())
    } else {
        previous_meta
            .as_ref()
            .and_then(|meta| meta.started_at.clone())
    };
    let ended_at = if event.event_type == "PipelineStarted" {
        previous_meta
            .as_ref()
            .and_then(|meta| meta.ended_at.clone())
    } else {
        Some(event.emitted_at.clone())
    };
    let meta = BoundaryMeta {
        node_id: None,
        stage_index: None,
        attempt: None,
        source_scope,
        source_parent_node_id,
        source_flow_name,
        model: previous_meta
            .as_ref()
            .and_then(|meta| meta.model.clone())
            .or_else(|| string_payload(event, "model")),
        started_at,
        ended_at,
        summary: summary.clone(),
    };
    upsert_boundary_segment(
        transcript, &key, previous, meta, status, summary, event, sequence,
    );
}

fn upsert_stage_boundary(transcript: &mut Transcript, event: &RawRuntimeEvent, sequence: u64) {
    let node_id = string_payload(event, "node_id")
        .or_else(|| string_payload(event, "node"))
        .or_else(|| string_payload(event, "stage"));
    let stage_index =
        numeric_payload(event, "stage_index").or_else(|| numeric_payload(event, "index"));
    let attempt = numeric_payload(event, "attempt")
        .or_else(|| numeric_payload(event, "retry_attempt"))
        .or_else(|| numeric_payload(event, "stage_attempt"));
    let source_scope = string_payload(event, "source_scope").unwrap_or_else(|| "root".to_string());
    let source_parent_node_id = string_payload(event, "source_parent_node_id");
    let source_flow_name = string_payload(event, "source_flow_name");
    let key = format!(
        "boundary-{}-{}-{}-{}-{}-{}",
        source_scope,
        source_parent_node_id.as_deref().unwrap_or("root"),
        source_flow_name.as_deref().unwrap_or(""),
        node_id.as_deref().unwrap_or("run"),
        stage_index
            .map(|value| value.to_string())
            .unwrap_or_else(|| "na".to_string()),
        attempt
            .map(|value| value.to_string())
            .unwrap_or_else(|| "na".to_string())
    );
    let status = boundary_status(event.event_type.as_str(), event);
    let summary = match event.event_type.as_str() {
        "StageCompleted" => format!(
            "Stage {} completed",
            node_id.as_deref().unwrap_or("unknown")
        ),
        "StageFailed" => string_payload(event, "error")
            .map(|error| {
                format!(
                    "Stage {} failed: {error}",
                    node_id.as_deref().unwrap_or("unknown")
                )
            })
            .unwrap_or_else(|| format!("Stage {} failed", node_id.as_deref().unwrap_or("unknown"))),
        "StageRetrying" => format!("Stage {} retrying", node_id.as_deref().unwrap_or("unknown")),
        _ => format!("Stage {} started", node_id.as_deref().unwrap_or("unknown")),
    };
    let previous = transcript.find_segment(&key).cloned();
    let previous_meta = previous
        .as_ref()
        .and_then(|segment| segment.boundary.clone());
    let started_at = if event.event_type == "StageStarted" {
        Some(event.emitted_at.clone())
    } else {
        previous_meta
            .as_ref()
            .and_then(|meta| meta.started_at.clone())
    };
    let ended_at = if status == "running" {
        previous_meta
            .as_ref()
            .and_then(|meta| meta.ended_at.clone())
    } else {
        Some(event.emitted_at.clone())
    };
    let meta = BoundaryMeta {
        node_id,
        stage_index,
        attempt,
        source_scope,
        source_parent_node_id,
        source_flow_name,
        model: previous_meta
            .as_ref()
            .and_then(|meta| meta.model.clone())
            .or_else(|| string_payload(event, "model")),
        started_at,
        ended_at,
        summary: summary.clone(),
    };
    upsert_boundary_segment(
        transcript, &key, previous, meta, status, summary, event, sequence,
    );
}

#[allow(clippy::too_many_arguments)]
fn upsert_boundary_segment(
    transcript: &mut Transcript,
    key: &str,
    previous: Option<TranscriptSegment>,
    meta: BoundaryMeta,
    status: String,
    summary: String,
    event: &RawRuntimeEvent,
    sequence: u64,
) {
    let order = previous
        .as_ref()
        .map(|segment| segment.order)
        .unwrap_or(sequence as i64);
    let timestamp = previous
        .as_ref()
        .map(|segment| segment.timestamp.clone())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| event.emitted_at.clone());
    let turn_id = meta
        .node_id
        .as_deref()
        .map(node_turn_id)
        .unwrap_or_else(|| "run".to_string());
    let mut segment = segment_shell(
        key,
        &turn_id,
        order,
        SEGMENT_KIND_BOUNDARY,
        "system",
        &status,
        &timestamp,
    );
    segment.updated_at = event.emitted_at.clone();
    segment.content = summary;
    segment.boundary = Some(meta);
    transcript.upsert_segment(segment);
}

fn persist_codergen_event(
    transcript: &mut Transcript,
    _run_id: &str,
    node_id: &str,
    event: &CodergenEvent,
) {
    let Some(turn_event) = event
        .payload
        .get("turn_stream_event")
        .and_then(|value| serde_json::from_value::<TurnStreamEvent>(value.clone()).ok())
    else {
        return;
    };
    match turn_event.kind {
        TurnStreamEventKind::ContentDelta | TurnStreamEventKind::ContentCompleted => {
            upsert_message(transcript, node_id, event, &turn_event);
        }
        TurnStreamEventKind::ToolCallStarted
        | TurnStreamEventKind::ToolCallUpdated
        | TurnStreamEventKind::ToolCallCompleted
        | TurnStreamEventKind::ToolCallFailed => {
            upsert_tool_call(transcript, node_id, &turn_event);
        }
        TurnStreamEventKind::RequestUserInputRequested => {
            let order = next_transcript_order(transcript);
            let request = normalize_request_user_input_value(
                turn_event.request_user_input.as_ref().unwrap_or(&json!({})),
                &format!("request-{order}"),
            );
            let scope_key = node_turn_id(node_id);
            let id = request_user_input_segment_id(&scope_key, &turn_event, &request);
            let mut segment = segment_shell(
                &id,
                &scope_key,
                order,
                SEGMENT_KIND_REQUEST_USER_INPUT,
                "system",
                request
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("pending"),
                "",
            );
            segment.content = request_user_input_prompt_summary(&request);
            segment.request_user_input = Some(Value::Object(request));
            segment.source = Some(segment_source(&turn_event, None));
            transcript.upsert_segment(segment);
        }
        TurnStreamEventKind::ContextCompactionStarted
        | TurnStreamEventKind::ContextCompactionCompleted => {
            let scope_key = node_turn_id(node_id);
            let id = context_compaction_segment_id(&scope_key, &turn_event);
            let complete = turn_event.kind == TurnStreamEventKind::ContextCompactionCompleted;
            let order = transcript
                .find_segment(&id)
                .map(|segment| segment.order)
                .unwrap_or_else(|| next_transcript_order(transcript));
            let mut segment = segment_shell(
                &id,
                &scope_key,
                order,
                SEGMENT_KIND_CONTEXT_COMPACTION,
                "system",
                if complete { "complete" } else { "running" },
                "",
            );
            segment.content = turn_event
                .message
                .clone()
                .or_else(|| turn_event.status.clone())
                .unwrap_or_else(|| turn_event.kind.as_str().to_string());
            segment.source = Some(segment_source(&turn_event, None));
            transcript.upsert_segment(segment);
        }
        TurnStreamEventKind::Error => {
            let order = next_transcript_order(transcript);
            let id = format!("segment-notice-{node_id}-{order}");
            let mut segment = segment_shell(
                &id,
                &node_turn_id(node_id),
                order,
                SEGMENT_KIND_CONTEXT_COMPACTION,
                "system",
                "failed",
                "",
            );
            segment.content = turn_event
                .message
                .clone()
                .or_else(|| turn_event.error.clone())
                .or_else(|| turn_event.status.clone())
                .unwrap_or_else(|| turn_event.kind.as_str().to_string());
            segment.source = Some(segment_source(&turn_event, None));
            transcript.upsert_segment(segment);
        }
        _ => {}
    }
}

fn upsert_message(
    transcript: &mut Transcript,
    node_id: &str,
    event: &CodergenEvent,
    turn_event: &TurnStreamEvent,
) {
    let scope_key = node_turn_id(node_id);
    let (id, kind) = match turn_event.channel.as_ref() {
        Some(TurnStreamChannel::Reasoning) => (
            reasoning_segment_id(&scope_key, turn_event),
            SEGMENT_KIND_REASONING,
        ),
        Some(TurnStreamChannel::Plan) => {
            (plan_segment_id(&scope_key, turn_event), SEGMENT_KIND_PLAN)
        }
        _ => (
            assistant_segment_id(&scope_key, turn_event),
            SEGMENT_KIND_ASSISTANT_MESSAGE,
        ),
    };
    let delta = turn_event
        .content_delta
        .as_deref()
        .or(turn_event.message.as_deref())
        .unwrap_or("");
    let previous = transcript.find_segment(&id).cloned();
    let complete = turn_event.kind == TurnStreamEventKind::ContentCompleted;
    let content = if complete {
        delta.to_string()
    } else {
        format!(
            "{}{}",
            previous
                .as_ref()
                .map(|segment| segment.content.clone())
                .unwrap_or_default(),
            delta
        )
    };
    let emitted_at = string_map_payload(&event.payload, "emitted_at").unwrap_or_default();
    let order = previous
        .as_ref()
        .map(|segment| segment.order)
        .unwrap_or_else(|| next_transcript_order(transcript));
    let timestamp = previous
        .as_ref()
        .map(|segment| segment.timestamp.clone())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| emitted_at.clone());
    let mut segment = segment_shell(
        &id,
        &scope_key,
        order,
        kind,
        "assistant",
        if complete { "complete" } else { "streaming" },
        &timestamp,
    );
    segment.updated_at = emitted_at;
    segment.content = content;
    segment.source = Some(segment_source(turn_event, None));
    transcript.upsert_segment(segment);
}

fn upsert_final_assistant_text(transcript: &mut Transcript, node_id: &str, response_text: &str) {
    if response_text.trim().is_empty() || has_complete_assistant_message(transcript, node_id) {
        return;
    }
    let scope_key = node_turn_id(node_id);
    let id = format!("segment-assistant-{scope_key}");
    let order = transcript
        .find_segment(&id)
        .map(|segment| segment.order)
        .unwrap_or_else(|| next_transcript_order(transcript));
    let mut segment = segment_shell(
        &id,
        &scope_key,
        order,
        SEGMENT_KIND_ASSISTANT_MESSAGE,
        "assistant",
        "complete",
        "",
    );
    segment.content = response_text.to_string();
    transcript.upsert_segment(segment);
}

fn has_complete_assistant_message(transcript: &Transcript, node_id: &str) -> bool {
    let turn_id = node_turn_id(node_id);
    transcript.segments.iter().any(|segment| {
        segment.turn_id == turn_id
            && segment.kind == SEGMENT_KIND_ASSISTANT_MESSAGE
            && segment.status == "complete"
            && !segment.content.trim().is_empty()
    })
}

fn upsert_tool_call(transcript: &mut Transcript, node_id: &str, turn_event: &TurnStreamEvent) {
    let tool = turn_event.tool_call.clone().unwrap_or_else(|| json!({}));
    let scope_key = node_turn_id(node_id);
    let id = tool_segment_id(&scope_key, turn_event, &tool);
    let status = match turn_event.kind {
        TurnStreamEventKind::ToolCallCompleted => "completed",
        TurnStreamEventKind::ToolCallFailed => "failed",
        _ => "running",
    };
    let normalized = normalize_tool_call(tool, status);
    let order = transcript
        .find_segment(&id)
        .map(|segment| segment.order)
        .unwrap_or_else(|| next_transcript_order(transcript));
    let mut segment = segment_shell(
        &id,
        &scope_key,
        order,
        SEGMENT_KIND_TOOL_CALL,
        "system",
        status,
        "",
    );
    segment.content = normalized
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Tool call")
        .to_string();
    segment.source = Some(segment_source(
        turn_event,
        tool_call_id(&normalized).as_deref(),
    ));
    segment.tool_call = Some(normalized);
    transcript.upsert_segment(segment);
}

fn upsert_input(transcript: &mut Transcript, event: &RawRuntimeEvent, sequence: u64) {
    let question_id = string_payload(event, "question_id").unwrap_or_else(|| sequence.to_string());
    let prompt = string_payload(event, "prompt").unwrap_or_default();
    let request = human_gate_request_user_input(event, &question_id, &prompt);
    let id = format!("segment-request-user-input-{question_id}");
    let turn_id = string_payload(event, "node_id")
        .map(|node_id| node_turn_id(&node_id))
        .unwrap_or_else(|| "run".to_string());
    let mut segment = segment_shell(
        &id,
        &turn_id,
        sequence as i64,
        SEGMENT_KIND_REQUEST_USER_INPUT,
        "system",
        "pending",
        &event.emitted_at,
    );
    segment.content = prompt;
    segment.request_user_input = Some(Value::Object(request));
    segment.source = Some(json!({
        "node_id": string_payload(event, "node_id"),
        "source_scope": string_payload(event, "source_scope").unwrap_or_else(|| "root".to_string()),
        "source_parent_node_id": string_payload(event, "source_parent_node_id"),
        "source_flow_name": string_payload(event, "source_flow_name"),
    }));
    transcript.upsert_segment(segment);
}

fn upsert_input_answer(transcript: &mut Transcript, event: &RawRuntimeEvent, sequence: u64) {
    let question_id = string_payload(event, "question_id").unwrap_or_else(|| sequence.to_string());
    let answer = event.payload.get("answers").cloned().unwrap_or_else(|| {
        let mut answers = Map::new();
        answers.insert(
            question_id.clone(),
            json!(string_payload(event, "answer").unwrap_or_default()),
        );
        Value::Object(answers)
    });
    let existing = transcript
        .segments
        .iter()
        .find(|segment| {
            segment.kind == SEGMENT_KIND_REQUEST_USER_INPUT
                && request_user_input_matches(segment.request_user_input.as_ref(), &question_id)
        })
        .cloned();
    let prompt = existing
        .as_ref()
        .map(|segment| segment.content.clone())
        .filter(|content| !content.trim().is_empty())
        .or_else(|| string_payload(event, "prompt"))
        .or_else(|| string_payload(event, "question"))
        .unwrap_or_default();
    let mut request = existing
        .as_ref()
        .and_then(|segment| segment.request_user_input.as_ref())
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_else(|| human_gate_request_user_input(event, &question_id, &prompt));
    request.insert("status".to_string(), json!("answered"));
    request.insert(
        "answers".to_string(),
        normalize_answer_value(answer, &question_id),
    );
    request.insert("submitted_at".to_string(), json!(event.emitted_at.clone()));
    let mut segment = existing.unwrap_or_else(|| {
        let turn_id = string_payload(event, "node_id")
            .or_else(|| string_payload(event, "stage"))
            .map(|node_id| node_turn_id(&node_id))
            .unwrap_or_else(|| "run".to_string());
        let id = format!("segment-request-user-input-{question_id}");
        let mut shell = segment_shell(
            &id,
            &turn_id,
            sequence as i64,
            SEGMENT_KIND_REQUEST_USER_INPUT,
            "system",
            "pending",
            &event.emitted_at,
        );
        shell.content = prompt;
        shell
    });
    segment.status = "answered".to_string();
    segment.updated_at = event.emitted_at.clone();
    segment.completed_at = Some(event.emitted_at.clone());
    segment.request_user_input = Some(Value::Object(request));
    transcript.upsert_segment(segment);
}

fn request_user_input_matches(request: Option<&Value>, question_id: &str) -> bool {
    let Some(request) = request.and_then(Value::as_object) else {
        return false;
    };
    request
        .get("request_id")
        .and_then(Value::as_str)
        .is_some_and(|request_id| request_id == question_id)
        || request
            .get("questions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|question| question.get("id").and_then(Value::as_str) == Some(question_id))
}

fn normalize_answer_value(answer: Value, question_id: &str) -> Value {
    match answer {
        Value::Object(answers) => Value::Object(answers),
        Value::String(answer) => json!({ question_id: answer }),
        other if !other.is_null() => json!({ question_id: other }),
        _ => json!({}),
    }
}

fn normalize_tool_call(tool: Value, status: &str) -> Value {
    let title = tool
        .get("title")
        .and_then(Value::as_str)
        .or_else(|| tool.get("command").and_then(Value::as_str))
        .or_else(|| tool.get("name").and_then(Value::as_str))
        .unwrap_or("Tool call");
    json!({
        "id": tool.get("id").and_then(Value::as_str).unwrap_or("tool"),
        "kind": tool.get("kind").and_then(Value::as_str).unwrap_or("dynamic_tool"),
        "status": status,
        "title": title,
        "command": tool.get("command").cloned().unwrap_or(Value::Null),
        "output": tool.get("output").or_else(|| tool.get("content")).cloned().unwrap_or(Value::Null),
        "output_size": tool.get("outputSize").or_else(|| tool.get("output_size")).cloned().unwrap_or(Value::Null),
        "output_truncated": tool.get("outputTruncated").or_else(|| tool.get("output_truncated")).and_then(Value::as_bool).unwrap_or(false),
        "file_paths": tool.get("filePaths").or_else(|| tool.get("file_paths")).cloned().unwrap_or_else(|| json!([])),
    })
}

fn human_gate_request_user_input(
    event: &RawRuntimeEvent,
    question_id: &str,
    prompt: &str,
) -> Map<String, Value> {
    let options = event
        .payload
        .get("options")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|option| {
            let option = option.as_object()?;
            let label = option
                .get("label")
                .or_else(|| option.get("value"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if label.is_empty() {
                return None;
            }
            let mut output = Map::new();
            output.insert("label".to_string(), json!(label));
            if let Some(description) = option.get("description").and_then(Value::as_str) {
                output.insert("description".to_string(), json!(description));
            }
            Some(Value::Object(output))
        })
        .collect::<Vec<_>>();
    let mut request = Map::new();
    request.insert("request_id".to_string(), json!(question_id));
    request.insert("status".to_string(), json!("pending"));
    request.insert(
        "questions".to_string(),
        json!([{
            "id": question_id,
            "header": string_payload(event, "node_id").unwrap_or_else(|| "Human Gate".to_string()),
            "question": prompt,
            "question_type": if options.is_empty() { "FREEFORM" } else { "MULTIPLE_CHOICE" },
            "options": options,
            "allow_other": options.is_empty(),
            "is_secret": false,
        }]),
    );
    request.insert("answers".to_string(), json!({}));
    request
}

fn normalize_request_user_input_value(
    value: &Value,
    fallback_request_id: &str,
) -> Map<String, Value> {
    let object = value.as_object();
    let request_id = object
        .and_then(|object| {
            object
                .get("request_id")
                .or_else(|| object.get("itemId"))
                .and_then(Value::as_str)
        })
        .unwrap_or(fallback_request_id);
    let questions = object
        .and_then(|object| object.get("questions"))
        .and_then(Value::as_array)
        .map(|questions| {
            questions
                .iter()
                .enumerate()
                .filter_map(|(index, question)| {
                    let question = question.as_object()?;
                    let prompt = question
                        .get("question")
                        .and_then(Value::as_str)
                        .unwrap_or("User input requested.");
                    let options = question
                        .get("options")
                        .and_then(Value::as_array)
                        .map(|options| {
                            options
                                .iter()
                                .filter_map(|option| {
                                    let option = option.as_object()?;
                                    let label = option.get("label").and_then(Value::as_str)?;
                                    let mut output = Map::new();
                                    output.insert("label".to_string(), json!(label));
                                    if let Some(description) = option.get("description").and_then(Value::as_str) {
                                        output.insert("description".to_string(), json!(description));
                                    }
                                    Some(Value::Object(output))
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    Some(json!({
                        "id": question
                            .get("id")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .unwrap_or_else(|| format!("question-{}", index + 1)),
                        "header": question
                            .get("header")
                            .and_then(Value::as_str)
                            .unwrap_or("Human Gate"),
                        "question": prompt,
                        "question_type": question
                            .get("question_type")
                            .and_then(Value::as_str)
                            .unwrap_or(if options.is_empty() { "FREEFORM" } else { "MULTIPLE_CHOICE" }),
                        "options": options,
                        "allow_other": question.get("allow_other").and_then(Value::as_bool).unwrap_or(options.is_empty()),
                        "is_secret": question.get("is_secret").and_then(Value::as_bool).unwrap_or(false),
                    }))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            vec![json!({
                "id": request_id,
                "header": "Human Gate",
                "question": "User input requested.",
                "question_type": "FREEFORM",
                "options": [],
                "allow_other": true,
                "is_secret": false,
            })]
        });
    let mut request = Map::new();
    request.insert("request_id".to_string(), json!(request_id));
    request.insert(
        "status".to_string(),
        json!(object
            .and_then(|object| object.get("status"))
            .and_then(Value::as_str)
            .filter(|status| matches!(*status, "answered" | "expired"))
            .unwrap_or("pending")),
    );
    request.insert("questions".to_string(), Value::Array(questions));
    request.insert(
        "answers".to_string(),
        object
            .and_then(|object| object.get("answers"))
            .cloned()
            .unwrap_or_else(|| json!({})),
    );
    request
}

fn request_user_input_prompt_summary(request: &Map<String, Value>) -> String {
    let prompts = request
        .get("questions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|question| {
            question
                .get("question")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .collect::<Vec<_>>();
    match prompts.len() {
        0 => "User input requested.".to_string(),
        1 => prompts[0].to_string(),
        count => format!("{count} questions need user input."),
    }
}

fn string_payload(event: &RawRuntimeEvent, key: &str) -> Option<String> {
    string_map_payload(&event.payload, key)
}

fn string_map_payload(payload: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn numeric_payload(event: &RawRuntimeEvent, key: &str) -> Option<u64> {
    event.payload.get(key).and_then(Value::as_u64)
}

/// Legacy `{"entries": [...]}` run transcript files, read-compatibly converted
/// to the shared record shape.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyBoundaryEntry {
    id: String,
    sequence: u64,
    node_id: Option<String>,
    stage_index: Option<u64>,
    attempt: Option<u64>,
    status: String,
    started_at: Option<String>,
    ended_at: Option<String>,
    model: Option<String>,
    source_scope: String,
    source_parent_node_id: Option<String>,
    source_flow_name: Option<String>,
    summary: String,
}

fn legacy_transcript_from_entries(payload: &Value) -> Transcript {
    let mut transcript = Transcript::default();
    let Some(entries) = payload.get("entries").and_then(Value::as_array) else {
        return transcript;
    };
    for entry in entries {
        if entry.get("kind").and_then(Value::as_str) == Some(SEGMENT_KIND_BOUNDARY) {
            let Ok(legacy) = serde_json::from_value::<LegacyBoundaryEntry>(entry.clone()) else {
                continue;
            };
            let turn_id = legacy
                .node_id
                .as_deref()
                .map(node_turn_id)
                .unwrap_or_else(|| "run".to_string());
            let timestamp = legacy
                .started_at
                .clone()
                .or_else(|| legacy.ended_at.clone())
                .unwrap_or_default();
            let mut segment = segment_shell(
                &legacy.id,
                &turn_id,
                legacy.sequence as i64,
                SEGMENT_KIND_BOUNDARY,
                "system",
                &legacy.status,
                &timestamp,
            );
            segment.updated_at = legacy.ended_at.clone().unwrap_or(timestamp);
            segment.content = legacy.summary.clone();
            segment.boundary = Some(BoundaryMeta {
                node_id: legacy.node_id,
                stage_index: legacy.stage_index,
                attempt: legacy.attempt,
                source_scope: legacy.source_scope,
                source_parent_node_id: legacy.source_parent_node_id,
                source_flow_name: legacy.source_flow_name,
                model: legacy.model,
                started_at: legacy.started_at,
                ended_at: legacy.ended_at,
                summary: legacy.summary,
            });
            transcript.upsert_segment(segment);
            continue;
        }
        if let Ok(segment) = serde_json::from_value::<TranscriptSegment>(entry.clone()) {
            transcript.upsert_segment(segment);
        }
    }
    transcript.segments.sort_by(|left, right| {
        left.order
            .cmp(&right.order)
            .then_with(|| left.id.cmp(&right.id))
    });
    transcript
}
