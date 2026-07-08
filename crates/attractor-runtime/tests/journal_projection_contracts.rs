use attractor_core::RawRuntimeEvent;
use attractor_runtime::journals::journal_entries_from_events;
use serde_json::json;

fn adapter_stream_event(
    sequence: u64,
    node_id: &str,
    stream_event: serde_json::Value,
) -> RawRuntimeEvent {
    serde_json::from_value(json!({
        "sequence": sequence,
        "type": "CodergenAdapter",
        "run_id": "run-journal",
        "emitted_at": "2026-07-08T19:28:39.547405000Z",
        "adapter_event_type": "codex_app_server_session_event",
        "node_id": node_id,
        "payload": {
            "backend": "codex_app_server",
            "category": "assistant_text",
            "kind": stream_event["kind"],
            "node_id": node_id,
            "provider": "codex",
            "turn_stream_event": stream_event,
        },
    }))
    .expect("raw event")
}

#[test]
fn adapter_stream_deltas_project_as_flat_llm_content_entries() {
    let events = vec![
        adapter_stream_event(
            10,
            "implement",
            json!({
                "kind": "content_delta",
                "channel": "assistant",
                "phase": "final_answer",
                "content_delta": "Hello",
                "message": "Hello",
                "source": {"backend": "codex_app_server", "app_turn_id": "turn-1", "item_id": "msg-1"},
            }),
        ),
        // Whitespace-only deltas must survive (stream fidelity).
        adapter_stream_event(
            11,
            "implement",
            json!({
                "kind": "content_delta",
                "channel": "assistant",
                "phase": "final_answer",
                "content_delta": " ",
                "message": " ",
                "source": {"backend": "codex_app_server", "app_turn_id": "turn-1", "item_id": "msg-1"},
            }),
        ),
        adapter_stream_event(
            12,
            "implement",
            json!({
                "kind": "content_completed",
                "channel": "assistant",
                "phase": "final_answer",
                "content_delta": "Hello world",
                "message": "Hello world",
                "source": {"backend": "codex_app_server", "app_turn_id": "turn-1", "item_id": "msg-1"},
                "tool_call": null,
            }),
        ),
        // Non-content stream events keep their adapter identity.
        adapter_stream_event(
            13,
            "implement",
            json!({
                "kind": "tool_call_started",
                "source": {"backend": "codex_app_server", "app_turn_id": "turn-1"},
                "tool_call": {"name": "shell"},
            }),
        ),
    ];

    let entries = journal_entries_from_events(&events);
    assert_eq!(entries.len(), 4);
    // Entries sort newest-first.
    let by_sequence = |sequence: u64| {
        entries
            .iter()
            .find(|entry| entry.sequence == sequence)
            .expect("entry")
    };

    let delta = by_sequence(10);
    assert_eq!(delta.raw_type, "LLMContent");
    assert_eq!(delta.kind, "LLMContent");
    assert_eq!(delta.node_id.as_deref(), Some("implement"));
    assert_eq!(delta.summary, "Assistant output streaming for implement");
    assert_eq!(delta.payload["channel"], "assistant");
    assert_eq!(delta.payload["status"], "streaming");
    assert_eq!(delta.payload["content_delta"], "Hello");
    assert_eq!(delta.payload["source"]["item_id"], "msg-1");
    assert_eq!(delta.payload["phase"], "final_answer");

    let whitespace = by_sequence(11);
    assert_eq!(whitespace.raw_type, "LLMContent");
    assert_eq!(whitespace.payload["content_delta"], " ");

    let completed = by_sequence(12);
    assert_eq!(completed.raw_type, "LLMContent");
    assert_eq!(completed.payload["status"], "complete");
    assert_eq!(completed.payload["content_delta"], "Hello world");
    assert_eq!(completed.summary, "Assistant output complete for implement");

    let tool_call = by_sequence(13);
    assert_eq!(tool_call.raw_type, "CodergenAdapter");
    assert_eq!(
        tool_call.summary,
        "Codergen adapter event for implement: codex_app_server_session_event",
    );
}
