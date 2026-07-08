use attractor_core::RawRuntimeEvent;
use attractor_runtime::{journal_entries_from_events, project_run_segments};
use serde_json::{json, Value};

fn codex_adapter_event(sequence: u64, node_id: &str, stream_event: Value) -> RawRuntimeEvent {
    serde_json::from_value(json!({
        "sequence": sequence,
        "type": "CodergenAdapter",
        "run_id": "run-segments",
        "emitted_at": format!("2026-07-08T10:00:{:02}.000000000Z", sequence),
        "adapter_event_type": "codex_app_server_session_event",
        "node_id": node_id,
        "payload": {
            "backend": "codex_app_server",
            "node_id": node_id,
            "turn_stream_event": stream_event,
        },
    }))
    .expect("raw event")
}

fn rust_llm_adapter_event(sequence: u64, node_id: &str, stream_event: Value) -> RawRuntimeEvent {
    serde_json::from_value(json!({
        "sequence": sequence,
        "type": "CodergenAdapter",
        "run_id": "run-segments",
        "emitted_at": format!("2026-07-08T10:00:{:02}.000000000Z", sequence),
        "adapter_event_type": "rust_agent_session_event",
        "node_id": node_id,
        "payload": {
            "backend": "rust_unified_llm_adapter",
            "node_id": node_id,
            "turn_stream_event": stream_event,
        },
    }))
    .expect("raw event")
}

fn stage_retrying(sequence: u64, node_id: &str, attempt: u64) -> RawRuntimeEvent {
    serde_json::from_value(json!({
        "sequence": sequence,
        "type": "StageRetrying",
        "run_id": "run-segments",
        "emitted_at": format!("2026-07-08T10:00:{:02}.000000000Z", sequence),
        "node_id": node_id,
        "name": node_id,
        "attempt": attempt,
    }))
    .expect("raw event")
}

#[test]
fn codex_stream_projects_rich_segments_per_node() {
    let events = vec![
        codex_adapter_event(
            1,
            "implement",
            json!({
                "kind": "content_delta",
                "channel": "reasoning",
                "content_delta": "Thinking about it",
                "message": "Thinking about it",
                "source": {"backend": "codex_app_server", "app_turn_id": "turn-1", "item_id": "r-1"},
            }),
        ),
        codex_adapter_event(
            2,
            "implement",
            json!({
                "kind": "tool_call_started",
                "tool_call": {"id": "call-1", "name": "shell", "status": "started"},
                "source": {"backend": "codex_app_server", "app_turn_id": "turn-1", "item_id": "call-1"},
            }),
        ),
        codex_adapter_event(
            3,
            "implement",
            json!({
                "kind": "tool_call_completed",
                "tool_call": {"id": "call-1", "name": "shell", "status": "completed", "output": "ok"},
                "source": {"backend": "codex_app_server", "app_turn_id": "turn-1", "item_id": "call-1"},
            }),
        ),
        codex_adapter_event(
            4,
            "implement",
            json!({
                "kind": "content_completed",
                "channel": "assistant",
                "phase": "final_answer",
                "content_delta": "Done.",
                "message": "Done.",
                "source": {"backend": "codex_app_server", "app_turn_id": "turn-1", "item_id": "msg-1"},
            }),
        ),
        codex_adapter_event(
            5,
            "review",
            json!({
                "kind": "content_delta",
                "channel": "assistant",
                "content_delta": "Reviewing",
                "message": "Reviewing",
                "source": {"backend": "codex_app_server", "app_turn_id": "turn-2", "item_id": "msg-2"},
            }),
        ),
    ];
    let entries = journal_entries_from_events(&events);
    let projection = project_run_segments(&entries);

    assert_eq!(projection.newest_sequence, 5);
    assert_eq!(projection.segments.len(), 4);

    let kinds: Vec<&str> = projection
        .segments
        .iter()
        .map(|segment| segment["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            "reasoning",
            "tool_call",
            "assistant_message",
            "assistant_message"
        ],
    );

    let tool = &projection.segments[1];
    assert_eq!(tool["status"], "complete");
    assert_eq!(tool["tool_call"]["output"], "ok");
    assert_eq!(tool["node_id"], "implement");
    assert_eq!(tool["attempt"], 0);
    assert_eq!(tool["turn_id"], "root:implement:attempt-0");
    assert_eq!(tool["latest_sequence"], 3);
    // Timestamps come from event emission, not the wall clock.
    assert_eq!(tool["updated_at"], "2026-07-08T10:00:03.000000000Z");

    let final_answer = &projection.segments[2];
    assert_eq!(final_answer["content"], "Done.");
    assert_eq!(final_answer["status"], "complete");
    assert_eq!(final_answer["phase"], "final_answer");

    let review = &projection.segments[3];
    assert_eq!(review["node_id"], "review");
    assert_eq!(review["status"], "streaming");
    assert_eq!(review["content"], "Reviewing");
}

#[test]
fn sourceless_provider_streams_fall_back_to_turn_scoped_segments() {
    // Local/openai_compatible providers may emit no app ids at all; deltas
    // must still coalesce into one assistant segment per node attempt.
    let events = vec![
        rust_llm_adapter_event(
            1,
            "implement",
            json!({
                "kind": "content_delta",
                "channel": "assistant",
                "content_delta": "Hello",
                "message": "Hello",
                "source": {"backend": "rust_unified_llm_adapter"},
            }),
        ),
        rust_llm_adapter_event(
            2,
            "implement",
            json!({
                "kind": "content_delta",
                "channel": "assistant",
                "content_delta": " world",
                "message": " world",
                "source": {"backend": "rust_unified_llm_adapter"},
            }),
        ),
        rust_llm_adapter_event(
            3,
            "implement",
            json!({
                "kind": "tool_call_started",
                "tool_call": {"id": "call-9", "name": "read_file", "status": "started"},
                "source": {"backend": "rust_unified_llm_adapter"},
            }),
        ),
    ];
    let entries = journal_entries_from_events(&events);
    let projection = project_run_segments(&entries);

    assert_eq!(projection.segments.len(), 2);
    let assistant = &projection.segments[0];
    assert_eq!(assistant["kind"], "assistant_message");
    assert_eq!(assistant["content"], "Hello world");
    assert_eq!(
        assistant["id"],
        "segment-assistant-root:implement:attempt-0",
    );
    let tool = &projection.segments[1];
    assert_eq!(tool["kind"], "tool_call");
    assert_eq!(tool["status"], "running");
}

#[test]
fn retries_split_into_separate_attempt_transcripts() {
    let events = vec![
        rust_llm_adapter_event(
            1,
            "implement",
            json!({
                "kind": "content_delta",
                "channel": "assistant",
                "content_delta": "First try",
                "message": "First try",
                "source": {"backend": "rust_unified_llm_adapter"},
            }),
        ),
        stage_retrying(2, "implement", 1),
        rust_llm_adapter_event(
            3,
            "implement",
            json!({
                "kind": "content_delta",
                "channel": "assistant",
                "content_delta": "Second try",
                "message": "Second try",
                "source": {"backend": "rust_unified_llm_adapter"},
            }),
        ),
    ];
    let entries = journal_entries_from_events(&events);
    let projection = project_run_segments(&entries);

    assert_eq!(projection.segments.len(), 2);
    assert_eq!(projection.segments[0]["attempt"], 0);
    assert_eq!(projection.segments[0]["content"], "First try");
    assert_eq!(projection.segments[1]["attempt"], 1);
    assert_eq!(projection.segments[1]["content"], "Second try");
    assert_eq!(
        projection.segments[1]["turn_id"],
        "root:implement:attempt-1",
    );
}

#[test]
fn replaying_the_same_entries_is_deterministic() {
    let events = vec![
        codex_adapter_event(
            1,
            "implement",
            json!({
                "kind": "content_delta",
                "channel": "assistant",
                "content_delta": "a",
                "message": "a",
                "source": {"backend": "codex_app_server", "app_turn_id": "t", "item_id": "m"},
            }),
        ),
        codex_adapter_event(
            2,
            "implement",
            json!({
                "kind": "content_completed",
                "channel": "assistant",
                "content_delta": "ab",
                "message": "ab",
                "source": {"backend": "codex_app_server", "app_turn_id": "t", "item_id": "m"},
            }),
        ),
    ];
    let entries = journal_entries_from_events(&events);
    let first = project_run_segments(&entries);
    let second = project_run_segments(&entries);
    assert_eq!(first.segments, second.segments);
    assert_eq!(first.newest_sequence, second.newest_sequence);
    assert_eq!(first.segments[0]["content"], "ab");
}

#[test]
fn runs_without_stream_events_project_no_segments() {
    // Text-only codergen emits only a terminal request-completed event.
    let events = vec![serde_json::from_value::<RawRuntimeEvent>(json!({
        "sequence": 1,
        "type": "CodergenAdapter",
        "run_id": "run-segments",
        "emitted_at": "2026-07-08T10:00:01.000000000Z",
        "adapter_event_type": "rust_llm_adapter_request_completed",
        "node_id": "implement",
        "payload": {"backend": "rust_unified_llm_adapter", "node_id": "implement"},
    }))
    .expect("raw event")];
    let entries = journal_entries_from_events(&events);
    let projection = project_run_segments(&entries);
    assert!(projection.segments.is_empty());
    assert_eq!(projection.newest_sequence, 1);
}

#[test]
fn child_run_entries_carry_child_scope_metadata() {
    let mut entries = journal_entries_from_events(&[codex_adapter_event(
        1,
        "child_step",
        json!({
            "kind": "content_delta",
            "channel": "assistant",
            "content_delta": "child output",
            "message": "child output",
            "source": {"backend": "codex_app_server"},
        }),
    )]);
    // Simulate what combined_run_journal_entries stamps onto child entries.
    for entry in &mut entries {
        entry.source_scope = "child".to_string();
        entry.source_parent_node_id = Some("manager".to_string());
        entry.source_flow_name = Some("child-flow.dot".to_string());
        if let Some(payload) = entry.payload.as_object_mut() {
            payload.insert("source_run_id".to_string(), json!("run-child-1"));
        }
    }
    let projection = project_run_segments(&entries);
    assert_eq!(projection.segments.len(), 1);
    let segment = &projection.segments[0];
    assert_eq!(segment["source_scope"], "child");
    assert_eq!(segment["source_parent_node_id"], "manager");
    assert_eq!(segment["source_flow_name"], "child-flow.dot");
    assert_eq!(segment["source_run_id"], "run-child-1");
    assert_eq!(segment["turn_id"], "run-child-1:child_step:attempt-0");
}
