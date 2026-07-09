use attractor_core::JournalEntry;
use attractor_runtime::project_run_transcript;
use serde_json::{json, Value};

fn journal_entry(
    sequence: u64,
    raw_type: &str,
    node_id: Option<&str>,
    payload: Value,
) -> JournalEntry {
    JournalEntry {
        id: format!("journal-{sequence}"),
        sequence,
        emitted_at: format!("2026-01-01T00:00:{sequence:02}Z"),
        kind: "other".to_string(),
        raw_type: raw_type.to_string(),
        severity: "info".to_string(),
        summary: String::new(),
        node_id: node_id.map(str::to_string),
        stage_index: None,
        source_scope: "root".to_string(),
        source_parent_node_id: None,
        source_flow_name: None,
        question_id: None,
        payload,
    }
}

/// Journal payloads mirror the serialized RawRuntimeEvent shape.
fn runtime_entry(
    sequence: u64,
    raw_type: &str,
    node_id: Option<&str>,
    payload: Value,
) -> JournalEntry {
    let mut event_payload = payload.as_object().cloned().unwrap_or_default();
    let payload_value = json!({
        "run_id": "run-a",
        "type": raw_type,
        "emitted_at": format!("2026-01-01T00:00:{sequence:02}Z"),
        "sequence": sequence,
    });
    let mut merged = payload_value.as_object().cloned().expect("object");
    merged.append(&mut event_payload);
    journal_entry(sequence, raw_type, node_id, Value::Object(merged))
}

fn adapter_entry(sequence: u64, node_id: &str, turn_stream_event: Value) -> JournalEntry {
    journal_entry(
        sequence,
        "CodergenAdapter",
        Some(node_id),
        json!({
            "adapter_event_type": "rust_agent_session_event",
            "node_id": node_id,
            "payload": {"turn_stream_event": turn_stream_event},
        }),
    )
}

/// Content deltas surface in the journal as flat LLMContent entries carrying
/// the same turn_stream_event passthrough.
fn llm_content_entry(sequence: u64, node_id: &str, turn_stream_event: Value) -> JournalEntry {
    journal_entry(
        sequence,
        "LLMContent",
        Some(node_id),
        json!({
            "node_id": node_id,
            "turn_stream_event": turn_stream_event,
        }),
    )
}

#[test]
fn codergen_events_coalesce_into_shared_segments() {
    let entries = vec![
        llm_content_entry(
            10,
            "build",
            json!({
                "kind": "content_delta",
                "channel": "assistant",
                "content_delta": "Hello ",
            }),
        ),
        llm_content_entry(
            11,
            "build",
            json!({
                "kind": "content_delta",
                "channel": "assistant",
                "content_delta": "world",
            }),
        ),
        llm_content_entry(
            12,
            "build",
            json!({
                "kind": "content_completed",
                "channel": "assistant",
                "content_delta": "Hello world",
            }),
        ),
        llm_content_entry(
            13,
            "build",
            json!({
                "kind": "content_delta",
                "channel": "reasoning",
                "content_delta": "thinking...",
            }),
        ),
        adapter_entry(
            14,
            "build",
            json!({
                "kind": "tool_call_started",
                "tool_call": {"id": "call-1", "title": "Run ls", "command": "ls"},
            }),
        ),
        adapter_entry(
            15,
            "build",
            json!({
                "kind": "tool_call_completed",
                "tool_call": {"id": "call-1", "title": "Run ls", "command": "ls", "output": "README.md"},
            }),
        ),
        adapter_entry(
            16,
            "build",
            json!({
                "kind": "request_user_input_requested",
                "request_user_input": {
                    "request_id": "request-1",
                    "questions": [{"id": "q1", "question": "Approve?", "options": []}],
                },
            }),
        ),
    ];

    let transcript = project_run_transcript(&entries);
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
}

#[test]
fn runtime_events_produce_boundary_segments_with_metadata() {
    let entries = vec![
        runtime_entry(1, "PipelineStarted", None, json!({"name": "compat-flow"})),
        runtime_entry(
            2,
            "StageStarted",
            Some("build"),
            json!({"node_id": "build", "stage_index": 0, "attempt": 1}),
        ),
        runtime_entry(
            3,
            "StageCompleted",
            Some("build"),
            json!({"node_id": "build", "stage_index": 0, "attempt": 1}),
        ),
    ];

    let transcript = project_run_transcript(&entries);
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
fn projection_is_deterministic_regardless_of_entry_order() {
    let mut entries = vec![
        runtime_entry(
            2,
            "StageStarted",
            Some("build"),
            json!({"node_id": "build", "stage_index": 0, "attempt": 1}),
        ),
        llm_content_entry(
            3,
            "build",
            json!({
                "kind": "content_completed",
                "channel": "assistant",
                "content_delta": "Done.",
            }),
        ),
        runtime_entry(1, "PipelineStarted", None, json!({"name": "compat-flow"})),
    ];
    let forward = project_run_transcript(&entries);
    entries.reverse();
    let reversed = project_run_transcript(&entries);
    assert_eq!(forward, reversed);
    assert_eq!(forward.segments.len(), 3);
}
