use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use spark_agent_adapter::{AgentRawLogLine, AgentThreadResumeFailure, AgentTurnOutput};
use spark_common::events::{
    TurnStreamChannel, TurnStreamEvent, TurnStreamEventKind, TurnStreamSource,
};
use spark_common::settings::SparkSettings;
use spark_workspace::{
    ConversationRequestUserInputAnswerRequest, ConversationTurnRequest,
    WorkspaceConversationService, WorkspaceError,
};

#[test]
fn start_turn_persists_one_user_one_assistant_turn_settings_and_events() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = WorkspaceConversationService::new(settings.clone());

    let (prepared, snapshot) = service
        .start_turn(
            "conversation-turn",
            ConversationTurnRequest {
                project_path: "/projects/turns".to_string(),
                message: "  Build this feature  ".to_string(),
                provider: Some("openai".to_string()),
                model: Some("gpt-5".to_string()),
                llm_profile: Some("frontier".to_string()),
                reasoning_effort: Some("high".to_string()),
                chat_mode: Some("plan".to_string()),
            },
        )
        .expect("start turn");

    assert_eq!(prepared.chat_mode, "plan");
    assert_eq!(prepared.provider, "openai");
    assert_eq!(prepared.model.as_deref(), Some("gpt-5"));
    assert_eq!(snapshot["chat_mode"], "plan");
    assert_eq!(snapshot["provider"], "openai");
    assert_eq!(snapshot["llm_profile"], "frontier");
    assert_eq!(snapshot["reasoning_effort"], "high");
    assert_eq!(snapshot["title"], "Build this feature");
    let turns = snapshot["turns"].as_array().expect("turns");
    assert_eq!(turns.len(), 3);
    assert_eq!(turns[0]["kind"], "mode_change");
    assert_eq!(turns[1]["role"], "user");
    assert_eq!(turns[1]["content"], "Build this feature");
    assert_eq!(turns[2]["role"], "assistant");
    assert_eq!(turns[2]["status"], "pending");
    assert_eq!(turns[2]["parent_turn_id"], turns[1]["id"]);
    assert_eq!(snapshot["revision"], 3);

    let events = service
        .read_events_after("conversation-turn", "/projects/turns", 0)
        .expect("events");
    assert_eq!(
        events
            .iter()
            .map(|event| event["type"].as_str().expect("type"))
            .collect::<Vec<_>>(),
        vec!["turn_upsert", "turn_upsert", "turn_upsert"]
    );
    assert_eq!(
        events
            .iter()
            .map(|event| event["revision"].as_i64().expect("revision"))
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );

    let conflict = service
        .start_turn(
            "conversation-turn",
            ConversationTurnRequest {
                project_path: "/projects/turns".to_string(),
                message: "again".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect_err("active assistant conflict");
    assert!(matches!(conflict, WorkspaceError::Conflict(_)));
}

#[test]
fn normalized_agent_events_update_segments_raw_logs_usage_and_resume_failures() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = WorkspaceConversationService::new(settings.clone());
    let (prepared, _snapshot) = service
        .start_turn(
            "conversation-events",
            ConversationTurnRequest {
                project_path: "/projects/events".to_string(),
                message: "Explain the change".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");

    let output = AgentTurnOutput {
        raw_log_lines: vec![AgentRawLogLine {
            direction: "incoming".to_string(),
            line: "{\"event\":\"delta\"}".to_string(),
        }],
        events: vec![
            content_delta("assistant", "Hel", "app-turn", "final"),
            content_completed("assistant", "Hello", "app-turn", "final"),
            content_delta("reasoning", "Because", "app-turn", "reasoning"),
            content_completed("reasoning", "Because.", "app-turn", "reasoning"),
            content_delta("plan", "1. Do it", "app-turn", "plan"),
            content_completed("plan", "1. Do it", "app-turn", "plan"),
            tool_event("tool_call_started", "tool-1", "running", "partial"),
            tool_event("tool_call_completed", "tool-1", "completed", "full output"),
            token_usage(json!({"total": {"inputTokens": 10, "outputTokens": 4}})),
        ],
        final_assistant_text: Some("Hello".to_string()),
        token_usage: Some(json!({"total": {"inputTokens": 10, "outputTokens": 4}})),
        thread_resume_failure: None,
    };
    let snapshot = service
        .ingest_agent_turn_output(
            "conversation-events",
            "/projects/events",
            &prepared.assistant_turn_id,
            "chat",
            output,
        )
        .expect("ingest output");

    let assistant_turn = snapshot["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == prepared.assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "complete");
    assert_eq!(assistant_turn["content"], "Hello");
    assert_eq!(
        assistant_turn["token_usage"]["total"]["inputTokens"],
        json!(10)
    );
    let segments = snapshot["segments"].as_array().expect("segments");
    assert_eq!(
        segments
            .iter()
            .filter(|segment| segment["kind"] == "assistant_message")
            .count(),
        1
    );
    let reasoning = segment_by_kind(segments, "reasoning");
    assert_eq!(reasoning["content"], "Because.");
    assert_eq!(reasoning["source"]["app_turn_id"], "app-turn");
    assert_eq!(reasoning["source"]["item_id"], "reasoning");
    let plan = segment_by_kind(segments, "plan");
    assert_eq!(plan["content"], "1. Do it");
    let tool = segment_by_kind(segments, "tool_call");
    assert_eq!(tool["status"], "complete");
    assert_eq!(tool["tool_call"]["output"], "full output");
    assert_eq!(tool["source"]["call_id"], "tool-1");

    let raw_log = spark_storage::ConversationRepository::new(&settings.data_dir)
        .read_raw_rpc_log("conversation-events", "/projects/events")
        .expect("raw log");
    assert_eq!(raw_log[0].direction, "incoming");
    assert_eq!(raw_log[0].line, "{\"event\":\"delta\"}");

    let events = service
        .read_events_after("conversation-events", "/projects/events", 0)
        .expect("events");
    assert!(events
        .iter()
        .any(|event| event["type"] == "conversation_snapshot"));

    let (failure_prepared, _snapshot) = service
        .start_turn(
            "conversation-failure",
            ConversationTurnRequest {
                project_path: "/projects/events".to_string(),
                message: "Resume".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start failure turn");
    let failed = service
        .ingest_agent_turn_output(
            "conversation-failure",
            "/projects/events",
            &failure_prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                thread_resume_failure: Some(AgentThreadResumeFailure {
                    message: "thread resume failed".to_string(),
                    error_code: Some("thread_resume_failed".to_string()),
                    details: Some(json!({"thread_id": "thread-1"})),
                }),
                ..AgentTurnOutput::default()
            },
        )
        .expect("failure output");
    let assistant_turn = failed["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == failure_prepared.assistant_turn_id)
        .expect("failed assistant turn");
    assert_eq!(assistant_turn["status"], "failed");
    assert_eq!(assistant_turn["error_code"], "thread_resume_failed");
    assert_eq!(failed["event_log"][0]["kind"], "continuity_reset");
}

#[test]
fn final_assistant_text_completes_existing_streamed_assistant_segment() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = WorkspaceConversationService::new(settings(temp.path()));
    let (prepared, _snapshot) = service
        .start_turn(
            "conversation-fallback-final-text",
            ConversationTurnRequest {
                project_path: "/projects/fallback".to_string(),
                message: "Explain the result".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");

    let snapshot = service
        .ingest_agent_turn_output(
            "conversation-fallback-final-text",
            "/projects/fallback",
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![content_delta("assistant", "Partial", "app-turn", "final")],
                final_assistant_text: Some("Complete answer".to_string()),
                ..AgentTurnOutput::default()
            },
        )
        .expect("ingest fallback text");

    let assistant_turn = snapshot["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == prepared.assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "complete");
    assert_eq!(assistant_turn["content"], "Complete answer");

    let assistant_segments = snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .filter(|segment| segment["kind"] == "assistant_message")
        .collect::<Vec<_>>();
    assert_eq!(assistant_segments.len(), 1);
    let segment = assistant_segments[0];
    assert_eq!(segment["status"], "complete");
    assert_eq!(segment["content"], "Complete answer");
    assert_eq!(segment["phase"], "final_answer");
    assert_eq!(segment["source"]["app_turn_id"], "app-turn");
    assert_eq!(segment["source"]["item_id"], "final");
}

#[test]
fn stream_error_without_final_answer_remains_failed_and_does_not_write_fallback_content() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = WorkspaceConversationService::new(settings(temp.path()));
    let (prepared, _snapshot) = service
        .start_turn(
            "conversation-stream-error",
            ConversationTurnRequest {
                project_path: "/projects/stream-error".to_string(),
                message: "Run the agent".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");

    let snapshot = service
        .ingest_agent_turn_output(
            "conversation-stream-error",
            "/projects/stream-error",
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![stream_error_event("agent crashed")],
                ..AgentTurnOutput::default()
            },
        )
        .expect("ingest stream error");

    let assistant_turn = snapshot["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == prepared.assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "failed");
    assert_eq!(assistant_turn["error"], "agent crashed");
    assert_ne!(assistant_turn["content"], "I reviewed that request.");

    let segment = segment_by_kind(
        snapshot["segments"].as_array().expect("segments"),
        "assistant_message",
    );
    assert_eq!(segment["status"], "failed");
    assert_eq!(segment["error"], "agent crashed");
    assert_eq!(segment["source"]["app_turn_id"], "app-turn");
    assert_eq!(segment["source"]["item_id"], "error-item");

    let events = service
        .read_events_after("conversation-stream-error", "/projects/stream-error", 2)
        .expect("events");
    assert!(events
        .iter()
        .any(|event| { event["type"] == "turn_upsert" && event["turn"]["status"] == "failed" }));
    assert!(events.iter().any(|event| {
        event["type"] == "segment_upsert" && event["segment"]["status"] == "failed"
    }));
    assert!(events
        .iter()
        .any(|event| event["type"] == "conversation_snapshot"));
}

#[test]
fn request_user_input_answers_expire_pending_segments_and_are_idempotent() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = WorkspaceConversationService::new(settings(temp.path()));
    let (prepared, _snapshot) = service
        .start_turn(
            "conversation-input",
            ConversationTurnRequest {
                project_path: "/projects/input".to_string(),
                message: "Need approval".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");
    let pending = service
        .ingest_agent_turn_output(
            "conversation-input",
            "/projects/input",
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![request_user_input_event()],
                ..AgentTurnOutput::default()
            },
        )
        .expect("request input");
    let request_segment = segment_by_kind(
        pending["segments"].as_array().expect("segments"),
        "request_user_input",
    );
    assert_eq!(request_segment["request_user_input"]["status"], "pending");

    let answered = service
        .submit_request_user_input_answer(
            "conversation-input",
            "decision",
            ConversationRequestUserInputAnswerRequest {
                project_path: "/projects/input".to_string(),
                answers: BTreeMap::from([("decision".to_string(), "Approve".to_string())]),
            },
        )
        .expect("answer");
    let request_segment = segment_by_kind(
        answered["segments"].as_array().expect("segments"),
        "request_user_input",
    );
    assert_eq!(request_segment["status"], "failed");
    assert_eq!(request_segment["request_user_input"]["status"], "expired");
    assert_eq!(
        request_segment["error"],
        "The requested input expired before the answer could be used."
    );
    let assistant_turn = answered["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == prepared.assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "failed");

    let idempotent = service
        .submit_request_user_input_answer(
            "conversation-input",
            "input-1",
            ConversationRequestUserInputAnswerRequest {
                project_path: "/projects/input".to_string(),
                answers: BTreeMap::from([("decision".to_string(), "Approve".to_string())]),
            },
        )
        .expect("idempotent answer");
    assert_eq!(idempotent["revision"], answered["revision"]);

    let changed = service
        .submit_request_user_input_answer(
            "conversation-input",
            "decision",
            ConversationRequestUserInputAnswerRequest {
                project_path: "/projects/input".to_string(),
                answers: BTreeMap::from([("decision".to_string(), "Reject".to_string())]),
            },
        )
        .expect_err("changed answer conflict");
    assert!(matches!(changed, WorkspaceError::Conflict(_)));

    let missing = service
        .submit_request_user_input_answer(
            "conversation-input",
            "missing",
            ConversationRequestUserInputAnswerRequest {
                project_path: "/projects/input".to_string(),
                answers: BTreeMap::from([("decision".to_string(), "Approve".to_string())]),
            },
        )
        .expect_err("missing request");
    assert!(matches!(missing, WorkspaceError::NotFound(_)));
}

fn content_delta(channel: &str, text: &str, app_turn_id: &str, item_id: &str) -> TurnStreamEvent {
    let mut event = TurnStreamEvent::content_delta(parse_channel(channel), text);
    event.source = source(app_turn_id, item_id);
    event
}

fn content_completed(
    channel: &str,
    text: &str,
    app_turn_id: &str,
    item_id: &str,
) -> TurnStreamEvent {
    TurnStreamEvent {
        kind: TurnStreamEventKind::ContentCompleted,
        channel: Some(parse_channel(channel)),
        source: source(app_turn_id, item_id),
        content_delta: Some(text.to_string()),
        message: Some(text.to_string()),
        tool_call: None,
        request_user_input: None,
        token_usage: None,
        error: None,
        phase: Some("final_answer".to_string()),
        status: None,
    }
}

fn tool_event(kind: &str, id: &str, status: &str, output: &str) -> TurnStreamEvent {
    TurnStreamEvent {
        kind: kind.parse().expect("kind"),
        channel: None,
        source: source("app-turn", "tool-1"),
        content_delta: None,
        message: None,
        tool_call: Some(json!({
            "id": id,
            "kind": "command_execution",
            "status": status,
            "title": "Run command",
            "output": output,
            "file_paths": [],
        })),
        request_user_input: None,
        token_usage: None,
        error: None,
        phase: None,
        status: None,
    }
}

fn token_usage(usage: Value) -> TurnStreamEvent {
    TurnStreamEvent {
        kind: TurnStreamEventKind::TokenUsageUpdated,
        channel: None,
        source: source("app-turn", "usage"),
        content_delta: None,
        message: None,
        tool_call: None,
        request_user_input: None,
        token_usage: Some(usage),
        error: None,
        phase: None,
        status: None,
    }
}

fn request_user_input_event() -> TurnStreamEvent {
    TurnStreamEvent {
        kind: TurnStreamEventKind::RequestUserInputRequested,
        channel: None,
        source: source("app-turn", "input-1"),
        content_delta: None,
        message: None,
        tool_call: None,
        request_user_input: Some(json!({
            "itemId": "input-1",
            "questions": [
                {
                    "id": "decision",
                    "header": "Approve",
                    "question": "Approve this change?",
                    "options": [
                        {"label": "Approve", "description": "Continue"},
                        {"label": "Reject", "description": "Stop"}
                    ],
                    "isOther": false,
                    "isSecret": false
                }
            ]
        })),
        token_usage: None,
        error: None,
        phase: None,
        status: None,
    }
}

fn stream_error_event(message: &str) -> TurnStreamEvent {
    TurnStreamEvent {
        kind: TurnStreamEventKind::Error,
        channel: Some(TurnStreamChannel::Assistant),
        source: source("app-turn", "error-item"),
        content_delta: None,
        message: Some(message.to_string()),
        tool_call: None,
        request_user_input: None,
        token_usage: None,
        error: Some(message.to_string()),
        phase: Some("final_answer".to_string()),
        status: Some("failed".to_string()),
    }
}

fn parse_channel(channel: &str) -> TurnStreamChannel {
    channel.parse().expect("channel")
}

fn source(app_turn_id: &str, item_id: &str) -> TurnStreamSource {
    TurnStreamSource {
        app_turn_id: Some(app_turn_id.to_string()),
        item_id: Some(item_id.to_string()),
        ..TurnStreamSource::default()
    }
}

fn segment_by_kind<'a>(segments: &'a [Value], kind: &str) -> &'a Value {
    segments
        .iter()
        .find(|segment| segment["kind"] == kind)
        .expect("segment kind")
}

fn settings(root: &Path) -> SparkSettings {
    SparkSettings {
        project_root: root.join("source"),
        data_dir: root.join("spark-home"),
        config_dir: root.join("spark-home/config"),
        runtime_dir: root.join("spark-home/runtime"),
        logs_dir: root.join("spark-home/logs"),
        workspace_dir: root.join("spark-home/workspace"),
        projects_dir: root.join("spark-home/workspace/projects"),
        attractor_dir: root.join("spark-home/attractor"),
        runs_dir: root.join("spark-home/attractor/runs"),
        flows_dir: root.join("flows"),
        ui_dir: None,
        project_roots: Vec::<PathBuf>::new(),
    }
}
