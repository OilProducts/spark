use std::collections::BTreeMap;

use attractor_core::{Outcome, OutcomeStatus, RawRuntimeEvent};
use attractor_runtime::paths::RunRootPaths;
use attractor_runtime::read_run_transcript;
use attractor_runtime::transcript::{
    persist_codergen_transcript, persist_transcript_runtime_event,
};
use serde_json::{json, Value};
use spark_agent_adapter::codergen::{CodergenEvent, CodergenExecution};

fn run_paths(temp: &tempfile::TempDir) -> RunRootPaths {
    let paths = RunRootPaths::new(temp.path().join("runs"), "/projects/project-a", "run-a")
        .expect("run paths");
    std::fs::create_dir_all(&paths.root).expect("run root");
    paths
}

fn execution(events: Vec<CodergenEvent>, response_text: &str) -> CodergenExecution {
    CodergenExecution {
        outcome: Outcome::new(OutcomeStatus::Success),
        prompt: String::new(),
        response_text: response_text.to_string(),
        events,
        repair_attempts: 0,
        contract_violations: Vec::new(),
        usage: None,
    }
}

fn runtime_event(event_type: &str, sequence: u64, payload: Value) -> RawRuntimeEvent {
    RawRuntimeEvent {
        run_id: "run-a".to_string(),
        event_type: event_type.to_string(),
        emitted_at: format!("2026-01-01T00:00:{sequence:02}Z"),
        sequence: Some(sequence),
        payload: payload
            .as_object()
            .expect("payload object")
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<BTreeMap<_, _>>(),
    }
}

fn codergen_turn_event(turn_stream_event: Value) -> CodergenEvent {
    CodergenEvent::new(
        "turn_stream_event",
        BTreeMap::from([
            ("turn_stream_event".to_string(), turn_stream_event),
            ("emitted_at".to_string(), json!("2026-01-01T00:00:10Z")),
        ]),
    )
}

#[test]
fn codergen_events_coalesce_into_shared_segments() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = run_paths(&temp);
    let execution = execution(
        vec![
            codergen_turn_event(json!({
                "kind": "content_delta",
                "channel": "assistant",
                "content_delta": "Hello ",
            })),
            codergen_turn_event(json!({
                "kind": "content_delta",
                "channel": "assistant",
                "content_delta": "world",
            })),
            codergen_turn_event(json!({
                "kind": "content_completed",
                "channel": "assistant",
                "content_delta": "Hello world",
            })),
            codergen_turn_event(json!({
                "kind": "content_delta",
                "channel": "reasoning",
                "content_delta": "thinking...",
            })),
            codergen_turn_event(json!({
                "kind": "tool_call_started",
                "tool_call": {"id": "call-1", "title": "Run ls", "command": "ls"},
            })),
            codergen_turn_event(json!({
                "kind": "tool_call_completed",
                "tool_call": {"id": "call-1", "title": "Run ls", "command": "ls", "output": "README.md"},
            })),
            codergen_turn_event(json!({
                "kind": "request_user_input_requested",
                "request_user_input": {
                    "request_id": "request-1",
                    "questions": [{"id": "q1", "question": "Approve?", "options": []}],
                },
            })),
        ],
        "Hello world",
    );
    persist_codergen_transcript(&paths, "run-a", "build", &execution).expect("persist");

    let transcript = read_run_transcript(&paths).expect("read");
    let assistant = transcript
        .segments
        .iter()
        .filter(|segment| segment.kind == "assistant_message")
        .collect::<Vec<_>>();
    assert_eq!(assistant.len(), 1, "deltas coalesce into one segment");
    assert_eq!(assistant[0].content, "Hello world");
    assert_eq!(assistant[0].status, "complete");
    assert_eq!(assistant[0].turn_id, "run-node-build");

    let reasoning = transcript
        .segments
        .iter()
        .find(|segment| segment.kind == "reasoning")
        .expect("reasoning segment");
    assert_eq!(reasoning.content, "thinking...");
    assert_eq!(reasoning.status, "streaming");

    let tool = transcript
        .segments
        .iter()
        .find(|segment| segment.kind == "tool_call")
        .expect("tool segment");
    assert_eq!(tool.status, "completed");
    assert_eq!(
        tool.tool_call.as_ref().expect("tool_call")["output"],
        "README.md"
    );

    let input = transcript
        .segments
        .iter()
        .find(|segment| segment.kind == "request_user_input")
        .expect("input segment");
    assert_eq!(
        input.request_user_input.as_ref().expect("request")["request_id"],
        "request-1"
    );
    assert_eq!(input.content, "Approve?");

    // The persisted file is the shared record shape.
    let raw: Value =
        serde_json::from_str(&std::fs::read_to_string(paths.transcript_json()).expect("file"))
            .expect("json");
    assert!(raw.get("segments").is_some());
    assert!(raw.get("entries").is_none());
}

#[test]
fn runtime_events_produce_boundary_segments_with_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = run_paths(&temp);
    persist_transcript_runtime_event(
        &paths,
        &runtime_event("PipelineStarted", 1, json!({"name": "compat-flow"})),
    )
    .expect("pipeline started");
    persist_transcript_runtime_event(
        &paths,
        &runtime_event(
            "StageStarted",
            2,
            json!({"node_id": "build", "stage_index": 0, "attempt": 1}),
        ),
    )
    .expect("stage started");
    persist_transcript_runtime_event(
        &paths,
        &runtime_event(
            "StageCompleted",
            3,
            json!({"node_id": "build", "stage_index": 0, "attempt": 1}),
        ),
    )
    .expect("stage completed");

    let transcript = read_run_transcript(&paths).expect("read");
    let boundaries = transcript
        .segments
        .iter()
        .filter(|segment| segment.kind == "boundary")
        .collect::<Vec<_>>();
    assert_eq!(boundaries.len(), 2);
    let run_boundary = boundaries
        .iter()
        .find(|segment| segment.boundary.as_ref().expect("meta").node_id.is_none())
        .expect("run boundary");
    assert_eq!(run_boundary.status, "running");
    assert_eq!(run_boundary.content, "Run compat-flow started");
    let stage_boundary = boundaries
        .iter()
        .find(|segment| {
            segment.boundary.as_ref().expect("meta").node_id.as_deref() == Some("build")
        })
        .expect("stage boundary");
    assert_eq!(stage_boundary.status, "completed");
    assert_eq!(stage_boundary.order, 2, "boundary keeps its original order");
    let meta = stage_boundary.boundary.as_ref().expect("meta");
    assert_eq!(meta.stage_index, Some(0));
    assert_eq!(meta.attempt, Some(1));
    assert_eq!(meta.started_at.as_deref(), Some("2026-01-01T00:00:02Z"));
    assert_eq!(meta.ended_at.as_deref(), Some("2026-01-01T00:00:03Z"));
    assert_eq!(stage_boundary.turn_id, "run-node-build");
}

#[test]
fn legacy_entries_transcript_files_read_compatibly() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = run_paths(&temp);
    std::fs::write(
        paths.transcript_json(),
        serde_json::to_string_pretty(&json!({
            "entries": [
                {
                    "kind": "boundary",
                    "id": "boundary-root-root--build-0-1",
                    "sequence": 4,
                    "nodeId": "build",
                    "stageIndex": 0,
                    "attempt": 1,
                    "status": "completed",
                    "startedAt": "2026-01-01T00:00:01Z",
                    "endedAt": "2026-01-01T00:00:04Z",
                    "model": "compat-model",
                    "sourceScope": "root",
                    "sourceParentNodeId": null,
                    "sourceFlowName": null,
                    "summary": "Stage build completed"
                },
                {
                    "id": "message-build-assistant-default",
                    "turn_id": "run-node-build",
                    "order": 5,
                    "kind": "assistant_message",
                    "role": "assistant",
                    "status": "complete",
                    "timestamp": "2026-01-01T00:00:05Z",
                    "updated_at": "2026-01-01T00:00:05Z",
                    "content": "Done.",
                    "completed_at": null,
                    "error": null,
                    "artifact_id": null,
                    "phase": null,
                    "tool_call": null,
                    "request_user_input": null,
                    "source": null
                }
            ]
        }))
        .expect("json"),
    )
    .expect("write legacy");

    let transcript = read_run_transcript(&paths).expect("read");
    assert_eq!(transcript.segments.len(), 2);
    let boundary = &transcript.segments[0];
    assert_eq!(boundary.kind, "boundary");
    assert_eq!(boundary.order, 4);
    assert_eq!(boundary.content, "Stage build completed");
    let meta = boundary.boundary.as_ref().expect("meta");
    assert_eq!(meta.node_id.as_deref(), Some("build"));
    assert_eq!(meta.model.as_deref(), Some("compat-model"));
    let message = &transcript.segments[1];
    assert_eq!(message.kind, "assistant_message");
    assert_eq!(message.content, "Done.");
    assert_eq!(message.order, 5);
}
