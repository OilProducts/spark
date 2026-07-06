use std::collections::BTreeMap;

use attractor_core::RawRuntimeEvent;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use spark_agent_adapter::codergen::{CodergenEvent, CodergenExecution};
use spark_common::events::{TurnStreamChannel, TurnStreamEvent, TurnStreamEventKind};
use spark_storage::{read_json, write_json_atomic, JsonWriteOptions};

use crate::error::Result;
use crate::paths::RunRootPaths;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunTranscript {
    pub entries: Vec<RunTranscriptEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RunTranscriptEntry {
    Boundary(RunTranscriptBoundaryEntry),
    Segment(RunTranscriptSegmentEntry),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunTranscriptBoundaryEntry {
    pub kind: String,
    pub id: String,
    pub sequence: u64,
    pub node_id: Option<String>,
    pub stage_index: Option<u64>,
    pub attempt: Option<u64>,
    pub status: String,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub model: Option<String>,
    pub source_scope: String,
    pub source_parent_node_id: Option<String>,
    pub source_flow_name: Option<String>,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunTranscriptSegmentEntry {
    pub id: String,
    pub turn_id: String,
    pub order: u64,
    pub kind: String,
    pub role: String,
    pub status: String,
    pub timestamp: String,
    pub updated_at: String,
    pub content: String,
    pub completed_at: Option<String>,
    pub error: Option<String>,
    pub artifact_id: Option<String>,
    pub phase: Option<String>,
    pub tool_call: Option<Value>,
    pub request_user_input: Option<Value>,
    pub source: Option<Value>,
}

pub fn read_run_transcript(paths: &RunRootPaths) -> Result<RunTranscript> {
    if !paths.transcript_json().exists() {
        return Ok(RunTranscript::default());
    }
    Ok(read_json(paths.transcript_json())?)
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
            upsert_boundary(&mut transcript, event, sequence);
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
            upsert_entry(
                &mut transcript,
                &id,
                RunTranscriptEntry::Segment(segment_entry(
                    id.clone(),
                    "run".to_string(),
                    sequence,
                    "context_compaction",
                    "system",
                    "complete",
                    event.emitted_at.clone(),
                    event.emitted_at.clone(),
                    content,
                )),
            );
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

fn upsert_run_boundary(transcript: &mut RunTranscript, event: &RawRuntimeEvent, sequence: u64) {
    let source_scope = string_payload(event, "source_scope").unwrap_or_else(|| "root".to_string());
    let source_parent_node_id = string_payload(event, "source_parent_node_id");
    let source_flow_name = string_payload(event, "source_flow_name");
    let key = format!(
        "boundary-{}-{}-{}-run-na-na",
        source_scope,
        source_parent_node_id.as_deref().unwrap_or("root"),
        source_flow_name.as_deref().unwrap_or("")
    );
    let status = match event.event_type.as_str() {
        "PipelineCompleted" => {
            string_payload(event, "status").unwrap_or_else(|| "completed".to_string())
        }
        "PipelineFailed" => "failed".to_string(),
        _ => "running".to_string(),
    };
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
    let previous = transcript.entries.iter().find_map(|entry| match entry {
        RunTranscriptEntry::Boundary(existing) if existing.id == key => Some(existing.clone()),
        _ => None,
    });
    let started_at = if event.event_type == "PipelineStarted" {
        Some(event.emitted_at.clone())
    } else {
        previous.as_ref().and_then(|entry| entry.started_at.clone())
    };
    let next = RunTranscriptBoundaryEntry {
        kind: "boundary".to_string(),
        id: key.clone(),
        sequence: previous
            .as_ref()
            .map(|entry| entry.sequence)
            .unwrap_or(sequence),
        node_id: None,
        stage_index: None,
        attempt: None,
        status,
        started_at,
        ended_at: if event.event_type == "PipelineStarted" {
            previous.as_ref().and_then(|entry| entry.ended_at.clone())
        } else {
            Some(event.emitted_at.clone())
        },
        model: previous
            .as_ref()
            .and_then(|entry| entry.model.clone())
            .or_else(|| string_payload(event, "model")),
        source_scope,
        source_parent_node_id,
        source_flow_name,
        summary,
    };
    upsert_entry(transcript, &key, RunTranscriptEntry::Boundary(next));
}

fn persist_codergen_event(
    transcript: &mut RunTranscript,
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
            upsert_tool_call(transcript, node_id, event, &turn_event);
        }
        TurnStreamEventKind::RequestUserInputRequested => {
            let sequence = next_transcript_sequence(transcript);
            let request = normalize_request_user_input_value(
                turn_event.request_user_input.as_ref().unwrap_or(&json!({})),
                &format!("request-{sequence}"),
            );
            let request_id = request
                .get("request_id")
                .and_then(Value::as_str)
                .unwrap_or("request");
            let id = format!("segment-request-user-input-{node_id}-{request_id}");
            let mut segment = segment_entry(
                id.clone(),
                format!("run-node-{node_id}"),
                sequence,
                "request_user_input",
                "system",
                request
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("pending"),
                String::new(),
                String::new(),
                request_user_input_prompt_summary(&request),
            );
            segment.request_user_input = Some(Value::Object(request));
            segment.source = Some(turn_event_source_value(&turn_event));
            upsert_entry(transcript, &id, RunTranscriptEntry::Segment(segment));
        }
        TurnStreamEventKind::ContextCompactionStarted
        | TurnStreamEventKind::ContextCompactionCompleted
        | TurnStreamEventKind::Error => {
            let sequence = next_transcript_sequence(transcript);
            let id = format!("segment-notice-{node_id}-{sequence}");
            let status = if matches!(turn_event.kind, TurnStreamEventKind::Error) {
                "failed"
            } else {
                "complete"
            };
            let mut segment = segment_entry(
                id.clone(),
                format!("run-node-{node_id}"),
                sequence,
                "context_compaction",
                "system",
                status,
                String::new(),
                String::new(),
                turn_event
                    .message
                    .clone()
                    .or_else(|| turn_event.error.clone())
                    .or_else(|| turn_event.status.clone())
                    .unwrap_or_else(|| turn_event.kind.as_str().to_string()),
            );
            segment.source = Some(turn_event_source_value(&turn_event));
            upsert_entry(transcript, &id, RunTranscriptEntry::Segment(segment));
        }
        _ => {}
    }
}

fn upsert_boundary(transcript: &mut RunTranscript, event: &RawRuntimeEvent, sequence: u64) {
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
    let status = match event.event_type.as_str() {
        "StageCompleted" => "completed",
        "StageFailed" => "failed",
        "StageRetrying" => "retrying",
        _ => "running",
    };
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
    let previous = transcript.entries.iter().find_map(|entry| match entry {
        RunTranscriptEntry::Boundary(existing) if existing.id == key => Some(existing.clone()),
        _ => None,
    });
    let started_at = if event.event_type == "StageStarted" {
        Some(event.emitted_at.clone())
    } else {
        previous.as_ref().and_then(|entry| entry.started_at.clone())
    };
    let next = RunTranscriptBoundaryEntry {
        kind: "boundary".to_string(),
        id: key.clone(),
        sequence: previous
            .as_ref()
            .map(|entry| entry.sequence)
            .unwrap_or(sequence),
        node_id,
        stage_index,
        attempt,
        status: status.to_string(),
        started_at,
        ended_at: if status == "running" {
            previous.as_ref().and_then(|entry| entry.ended_at.clone())
        } else {
            Some(event.emitted_at.clone())
        },
        model: previous
            .as_ref()
            .and_then(|entry| entry.model.clone())
            .or_else(|| string_payload(event, "model")),
        source_scope,
        source_parent_node_id,
        source_flow_name,
        summary,
    };
    upsert_entry(transcript, &key, RunTranscriptEntry::Boundary(next));
}

fn upsert_message(
    transcript: &mut RunTranscript,
    node_id: &str,
    event: &CodergenEvent,
    turn_event: &TurnStreamEvent,
) {
    let channel = match turn_event.channel.as_ref() {
        Some(TurnStreamChannel::Reasoning) => "reasoning",
        Some(TurnStreamChannel::Plan) => "plan",
        _ => "assistant",
    };
    let source_key = turn_event
        .source
        .item_id
        .as_deref()
        .or(turn_event.source.response_id.as_deref())
        .or(turn_event.source.summary_index.map(|_| "summary"))
        .unwrap_or("default");
    let id = format!("message-{node_id}-{channel}-{source_key}");
    let delta = turn_event
        .content_delta
        .as_deref()
        .or(turn_event.message.as_deref())
        .unwrap_or("");
    let previous_content = transcript.entries.iter().find_map(|entry| match entry {
        RunTranscriptEntry::Segment(message) if message.id == id => Some(message.content.clone()),
        _ => None,
    });
    let status = if turn_event.kind == TurnStreamEventKind::ContentCompleted {
        "complete"
    } else {
        "streaming"
    };
    let content = if status == "complete" {
        delta.to_string()
    } else {
        format!("{}{}", previous_content.unwrap_or_default(), delta)
    };
    upsert_entry(
        transcript,
        &id,
        RunTranscriptEntry::Segment({
            let mut segment = segment_entry(
                id.clone(),
                format!("run-node-{node_id}"),
                next_transcript_sequence(transcript),
                match channel {
                    "reasoning" => "reasoning",
                    "plan" => "plan",
                    _ => "assistant_message",
                },
                "assistant",
                status,
                string_map_payload(&event.payload, "emitted_at").unwrap_or_default(),
                string_map_payload(&event.payload, "emitted_at").unwrap_or_default(),
                content,
            );
            segment.source = Some(turn_event_source_value(turn_event));
            segment
        }),
    );
}

fn upsert_final_assistant_text(transcript: &mut RunTranscript, node_id: &str, response_text: &str) {
    if response_text.trim().is_empty() || has_complete_assistant_message(transcript, node_id) {
        return;
    }
    let id = format!("message-{node_id}-assistant-default");
    upsert_entry(
        transcript,
        &id,
        RunTranscriptEntry::Segment(segment_entry(
            id.clone(),
            format!("run-node-{node_id}"),
            next_transcript_sequence(transcript),
            "assistant_message",
            "assistant",
            "complete",
            String::new(),
            String::new(),
            response_text.to_string(),
        )),
    );
}

fn has_complete_assistant_message(transcript: &RunTranscript, node_id: &str) -> bool {
    transcript.entries.iter().any(|entry| match entry {
        RunTranscriptEntry::Segment(message) => {
            message.turn_id == format!("run-node-{node_id}")
                && message.kind == "assistant_message"
                && message.status == "complete"
                && !message.content.trim().is_empty()
        }
        _ => false,
    })
}

fn upsert_tool_call(
    transcript: &mut RunTranscript,
    node_id: &str,
    _event: &CodergenEvent,
    turn_event: &TurnStreamEvent,
) {
    let tool = turn_event.tool_call.clone().unwrap_or_else(|| json!({}));
    let tool_id = tool
        .get("id")
        .and_then(Value::as_str)
        .or(turn_event.source.item_id.as_deref())
        .unwrap_or("tool");
    let id = format!("tool-{node_id}-{tool_id}");
    let status = match turn_event.kind {
        TurnStreamEventKind::ToolCallCompleted => "completed",
        TurnStreamEventKind::ToolCallFailed => "failed",
        _ => "running",
    };
    let normalized = normalize_tool_call(tool, status);
    upsert_entry(
        transcript,
        &id,
        RunTranscriptEntry::Segment({
            let mut segment = segment_entry(
                id.clone(),
                format!("run-node-{node_id}"),
                next_transcript_sequence(transcript),
                "tool_call",
                "system",
                status,
                String::new(),
                String::new(),
                normalized
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("Tool call")
                    .to_string(),
            );
            segment.tool_call = Some(normalized);
            segment.source = Some(turn_event_source_value(turn_event));
            segment
        }),
    );
}

fn upsert_input(transcript: &mut RunTranscript, event: &RawRuntimeEvent, sequence: u64) {
    let question_id = string_payload(event, "question_id").unwrap_or_else(|| sequence.to_string());
    let prompt = string_payload(event, "prompt").unwrap_or_default();
    let request = human_gate_request_user_input(event, &question_id, &prompt);
    let id = format!("segment-request-user-input-{question_id}");
    upsert_entry(
        transcript,
        &id,
        RunTranscriptEntry::Segment({
            let mut segment = segment_entry(
                id.clone(),
                string_payload(event, "node_id")
                    .map(|node_id| format!("run-node-{node_id}"))
                    .unwrap_or_else(|| "run".to_string()),
                sequence,
                "request_user_input",
                "system",
                "pending",
                event.emitted_at.clone(),
                event.emitted_at.clone(),
                prompt,
            );
            segment.request_user_input = Some(Value::Object(request));
            segment.source = Some(json!({
                "node_id": string_payload(event, "node_id"),
                "source_scope": string_payload(event, "source_scope").unwrap_or_else(|| "root".to_string()),
                "source_parent_node_id": string_payload(event, "source_parent_node_id"),
                "source_flow_name": string_payload(event, "source_flow_name"),
            }));
            segment
        }),
    );
}

fn upsert_input_answer(transcript: &mut RunTranscript, event: &RawRuntimeEvent, sequence: u64) {
    let question_id = string_payload(event, "question_id").unwrap_or_else(|| sequence.to_string());
    let answer = event.payload.get("answers").cloned().unwrap_or_else(|| {
        let mut answers = Map::new();
        answers.insert(
            question_id.clone(),
            json!(string_payload(event, "answer").unwrap_or_default()),
        );
        Value::Object(answers)
    });
    let existing = transcript.entries.iter().find_map(|entry| match entry {
        RunTranscriptEntry::Segment(segment)
            if segment.kind == "request_user_input"
                && request_user_input_matches(
                    segment.request_user_input.as_ref(),
                    &question_id,
                ) =>
        {
            Some(segment.clone())
        }
        _ => None,
    });
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
    let id = existing
        .as_ref()
        .map(|segment| segment.id.clone())
        .unwrap_or_else(|| format!("segment-request-user-input-{question_id}"));
    let mut segment = existing.unwrap_or_else(|| {
        segment_entry(
            id.clone(),
            string_payload(event, "node_id")
                .or_else(|| string_payload(event, "stage"))
                .map(|node_id| format!("run-node-{node_id}"))
                .unwrap_or_else(|| "run".to_string()),
            sequence,
            "request_user_input",
            "system",
            "pending",
            event.emitted_at.clone(),
            event.emitted_at.clone(),
            prompt,
        )
    });
    segment.status = "answered".to_string();
    segment.updated_at = event.emitted_at.clone();
    segment.completed_at = Some(event.emitted_at.clone());
    segment.request_user_input = Some(Value::Object(request));
    upsert_entry(transcript, &id, RunTranscriptEntry::Segment(segment));
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

fn upsert_entry(transcript: &mut RunTranscript, id: &str, entry: RunTranscriptEntry) {
    transcript
        .entries
        .retain(|existing| entry_id(existing) != id);
    transcript.entries.push(entry);
}

fn write_transcript(paths: &RunRootPaths, transcript: &mut RunTranscript) -> Result<()> {
    transcript.entries.sort_by(|left, right| {
        entry_sequence(left)
            .cmp(&entry_sequence(right))
            .then_with(|| entry_id(left).cmp(entry_id(right)))
    });
    write_json_atomic(
        paths.transcript_json(),
        transcript,
        JsonWriteOptions::default(),
    )?;
    Ok(())
}

fn next_transcript_sequence(transcript: &RunTranscript) -> u64 {
    transcript
        .entries
        .iter()
        .map(entry_sequence)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}

fn entry_sequence(entry: &RunTranscriptEntry) -> u64 {
    match entry {
        RunTranscriptEntry::Boundary(entry) => entry.sequence,
        RunTranscriptEntry::Segment(entry) => entry.order,
    }
}

fn entry_id(entry: &RunTranscriptEntry) -> &str {
    match entry {
        RunTranscriptEntry::Boundary(entry) => &entry.id,
        RunTranscriptEntry::Segment(entry) => &entry.id,
    }
}

fn segment_entry(
    id: String,
    turn_id: String,
    order: u64,
    kind: &str,
    role: &str,
    status: &str,
    timestamp: String,
    updated_at: String,
    content: String,
) -> RunTranscriptSegmentEntry {
    RunTranscriptSegmentEntry {
        id,
        turn_id,
        order,
        kind: kind.to_string(),
        role: role.to_string(),
        status: status.to_string(),
        timestamp,
        updated_at,
        content,
        completed_at: None,
        error: None,
        artifact_id: None,
        phase: None,
        tool_call: None,
        request_user_input: None,
        source: None,
    }
}

fn turn_event_source_value(turn_event: &TurnStreamEvent) -> Value {
    json!({
        "app_turn_id": turn_event.source.app_turn_id,
        "item_id": turn_event.source.item_id,
        "summary_index": turn_event.source.summary_index,
        "raw_kind": turn_event.source.raw_kind,
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
