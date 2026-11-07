use serde_json::{json, Value};
use spark_workspace::ConversationTurnRequest;

fn turn(id: &str, role: &str, content: &str, second: u32) -> Value {
    json!({
        "id": id,
        "role": role,
        "content": content,
        "timestamp": format!("2026-01-01T00:00:{second:02}Z"),
        "status": "complete",
        "kind": "message"
    })
}

fn segment(id: &str, turn_id: &str, order: i64, kind: &str, status: &str, content: &str) -> Value {
    json!({
        "id": id,
        "turn_id": turn_id,
        "order": order,
        "kind": kind,
        "role": if kind == "user_message" { "user" } else { "assistant" },
        "status": status,
        "timestamp": "2026-01-01T00:00:01Z",
        "updated_at": "2026-01-01T00:00:01Z",
        "content": content,
        "source": {}
    })
}

/// Segments of several turns, interleaved and out of order, each reach only
/// their own turn's history, in order.
#[test]
fn history_reads_each_turns_own_segments() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = super::conversation_repository_contracts::settings(temp.path());
    let project = "/projects/history-index";
    crate::write_conversation_snapshot(
        &settings.data_dir,
        &json!({
            "schema_version": 5,
            "settings_schema_version": 1,
            "revision": 0,
            "conversation_id": "history-index",
            "conversation_handle": "",
            "project_path": project,
            "chat_mode": "chat",
            "provider": "codex",
            "model": null,
            "llm_profile": null,
            "reasoning_effort": null,
            "title": "History index",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:09Z",
            "turns": [
                turn("u1", "user", "First", 1),
                turn("a1", "assistant", "a1 final", 2),
                turn("u2", "user", "Second", 3),
                turn("a2", "assistant", "a2 answer", 4),
                turn("u3", "user", "Third", 5),
                turn("a3", "assistant", "a3 final", 6),
            ],
            "segments": [
                segment("a3-plan", "a3", 4, "plan", "complete", "a3 plan"),
                segment("a1-after", "a1", 3, "assistant_message", "complete", "a1 after"),
                segment("a2-said", "a2", 1, "assistant_message", "complete", "a2 said"),
                segment("a3-msg", "a3", 2, "user_message", "complete", "a3 message"),
                segment("a1-msg", "a1", 2, "user_message", "complete", "a1 message"),
                segment("a3-draft", "a3", 3, "assistant_message", "running", "a3 draft"),
                segment("a1-before", "a1", 1, "assistant_message", "complete", "a1 before"),
                segment("a3-before", "a3", 1, "assistant_message", "complete", "a3 before"),
                segment("a1-plan", "a1", 4, "plan", "complete", "a1 plan"),
            ],
            "event_log": [],
            "flow_run_requests": [],
            "flow_launches": [],
            "run_recoveries": [],
            "proposed_plans": []
        }),
    );
    let service = crate::conversations(settings);

    let (prepared, _) = service
        .start_turn(
            "history-index",
            ConversationTurnRequest {
                project_path: project.to_string(),
                message: "Next".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");
    let request = service
        .prepare_agent_turn_request(&prepared)
        .expect("agent turn request");
    let history = serde_json::to_value(&request.history)
        .expect("history")
        .as_array()
        .expect("history")
        .iter()
        .map(|turn| (turn["role"].clone(), turn["content"].clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        history,
        [
            ("user", "First"),
            ("assistant", "a1 before"),
            ("user", "a1 message"),
            ("assistant", "a1 after\n\na1 plan"),
            ("user", "Second"),
            ("assistant", "a2 answer"),
            ("user", "Third"),
            ("assistant", "a3 before"),
            ("user", "a3 message"),
            ("assistant", "a3 plan"),
        ]
        .map(|(role, content)| (json!(role), json!(content)))
    );
}
