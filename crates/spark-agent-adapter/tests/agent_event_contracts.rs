use serde_json::json;
use std::collections::BTreeMap;

use spark_agent_adapter::{
    AgentRawLogLine, AgentRequestUserInputAnswerRequest, AgentThreadResumeFailure, AgentTurnOutput,
    AgentTurnRequest, EventKind, SessionEvent,
};
use spark_common::events::{
    TurnStreamChannel, TurnStreamEvent, TurnStreamEventKind, TurnStreamSource,
};

#[test]
fn agent_turn_output_carries_normalized_events_usage_raw_logs_and_resume_failure() {
    let mut event = TurnStreamEvent::content_delta(TurnStreamChannel::Assistant, "hello");
    event.source = TurnStreamSource {
        backend: Some("scripted".to_string()),
        app_turn_id: Some("app-turn-1".to_string()),
        item_id: Some("item-1".to_string()),
        ..TurnStreamSource::default()
    };
    let output = AgentTurnOutput {
        events: vec![event],
        final_assistant_text: Some("hello".to_string()),
        token_usage: Some(json!({"total": {"inputTokens": 3, "outputTokens": 5}})),
        token_usage_breakdown: Some(json!({"total": {"inputTokens": 3, "outputTokens": 5}})),
        raw_log_lines: vec![AgentRawLogLine {
            direction: "incoming".to_string(),
            line: "{\"type\":\"event\"}".to_string(),
        }],
        thread_resume_failure: Some(AgentThreadResumeFailure {
            message: "thread could not resume".to_string(),
            error_code: Some("thread_resume_failed".to_string()),
            details: Some(json!({"thread_id": "thread-1"})),
        }),
    };

    let encoded = serde_json::to_value(&output).expect("serialize");
    assert_eq!(encoded["events"][0]["kind"], "content_delta");
    assert_eq!(encoded["events"][0]["channel"], "assistant");
    assert_eq!(
        encoded["token_usage_breakdown"]["total"]["outputTokens"],
        json!(5)
    );
    assert_eq!(encoded["raw_log_lines"][0]["direction"], "incoming");
    assert_eq!(
        encoded["thread_resume_failure"]["error_code"],
        "thread_resume_failed"
    );

    let decoded: AgentTurnOutput = serde_json::from_value(encoded).expect("deserialize");
    assert_eq!(decoded.events.len(), 1);
    assert_eq!(decoded.raw_log_lines[0].line, "{\"type\":\"event\"}");
    assert_eq!(
        decoded
            .thread_resume_failure
            .expect("failure")
            .details
            .expect("details")["thread_id"],
        "thread-1"
    );
}

#[test]
fn session_warning_lifecycle_and_processing_events_map_to_turn_stream_events() {
    let warning = SessionEvent::without_session(
        EventKind::Warning,
        BTreeMap::from([
            ("message".to_string(), json!("Context usage at 95%.")),
            ("status".to_string(), json!("warning")),
        ]),
    )
    .to_turn_stream_event()
    .expect("warning event");
    assert_eq!(
        warning.kind,
        TurnStreamEventKind::Other("warning".to_string())
    );
    assert_eq!(warning.message.as_deref(), Some("Context usage at 95%."));
    assert_eq!(warning.status.as_deref(), Some("warning"));
    assert_eq!(
        warning.details.as_ref().unwrap()["message"],
        "Context usage at 95%."
    );
    assert_eq!(warning.source.raw_kind.as_deref(), Some("warning"));

    let session_start = SessionEvent::without_session(
        EventKind::SessionStart,
        BTreeMap::from([("status".to_string(), json!("processing"))]),
    )
    .to_turn_stream_event()
    .expect("session start event");
    assert_eq!(
        session_start.kind,
        TurnStreamEventKind::Other("session_start".to_string())
    );
    assert_eq!(session_start.status.as_deref(), Some("processing"));
    assert_eq!(
        session_start.source.raw_kind.as_deref(),
        Some("session_start")
    );

    let processing_end = SessionEvent::without_session(
        EventKind::ProcessingEnd,
        BTreeMap::from([("status".to_string(), json!("idle"))]),
    )
    .to_turn_stream_event()
    .expect("processing end event");
    assert_eq!(processing_end.kind, TurnStreamEventKind::TurnCompleted);
    assert_eq!(processing_end.status.as_deref(), Some("idle"));
    assert_eq!(processing_end.details.as_ref().unwrap()["status"], "idle");

    let session_end = SessionEvent::without_session(
        EventKind::SessionEnd,
        BTreeMap::from([("status".to_string(), json!("closed"))]),
    )
    .to_turn_stream_event()
    .expect("session end event");
    assert_eq!(
        session_end.kind,
        TurnStreamEventKind::Other("session_end".to_string())
    );
    assert_eq!(session_end.status.as_deref(), Some("closed"));
}

#[test]
fn agent_turn_requests_preserve_spark_payload_fields_across_serialization() {
    let mut metadata = BTreeMap::new();
    metadata.insert("spark.workspace.user_turn_id".to_string(), json!("user-1"));
    metadata.insert(
        "spark.runtime.provider_selector".to_string(),
        json!("codex"),
    );

    let request = AgentTurnRequest {
        conversation_id: "conversation-m7".to_string(),
        project_path: "/projects/m7".to_string(),
        prompt: "Continue the milestone.".to_string(),
        history: vec![],
        provider: Some("codex".to_string()),
        model: Some("profile-model".to_string()),
        llm_profile: Some("team-profile".to_string()),
        reasoning_effort: Some("high".to_string()),
        chat_mode: Some("plan".to_string()),
        metadata: metadata.clone(),
    };

    let encoded = serde_json::to_value(&request).expect("serialize request");
    assert_eq!(encoded["conversation_id"], "conversation-m7");
    assert_eq!(encoded["project_path"], "/projects/m7");
    assert_eq!(encoded["prompt"], "Continue the milestone.");
    assert_eq!(encoded["provider"], "codex");
    assert_eq!(encoded["model"], "profile-model");
    assert_eq!(encoded["llm_profile"], "team-profile");
    assert_eq!(encoded["reasoning_effort"], "high");
    assert_eq!(encoded["chat_mode"], "plan");
    assert_eq!(
        encoded["metadata"]["spark.workspace.user_turn_id"],
        "user-1"
    );

    let decoded: AgentTurnRequest = serde_json::from_value(encoded).expect("deserialize request");
    assert_eq!(decoded.conversation_id, request.conversation_id);
    assert_eq!(decoded.provider, request.provider);
    assert_eq!(decoded.metadata, metadata);
}

#[test]
fn request_user_input_answer_requests_preserve_resume_context() {
    let answer_request = AgentRequestUserInputAnswerRequest {
        conversation_id: "conversation-m7".to_string(),
        project_path: "/projects/m7".to_string(),
        request_id: "request-1".to_string(),
        assistant_turn_id: "assistant-1".to_string(),
        answers: BTreeMap::from([("decision".to_string(), "Continue".to_string())]),
        request_user_input: Some(json!({
            "request_id": "request-1",
            "status": "answered",
            "questions": [{"id": "decision", "question": "Continue?"}],
        })),
        history: vec![],
        provider: Some("codex".to_string()),
        model: Some("profile-model".to_string()),
        llm_profile: Some("team-profile".to_string()),
        reasoning_effort: Some("medium".to_string()),
        chat_mode: Some("chat".to_string()),
        metadata: BTreeMap::from([(
            "spark.workspace.request_user_input.lookup_id".to_string(),
            json!("decision"),
        )]),
    };

    let encoded = serde_json::to_value(&answer_request).expect("serialize answer request");
    assert_eq!(encoded["request_id"], "request-1");
    assert_eq!(encoded["assistant_turn_id"], "assistant-1");
    assert_eq!(encoded["answers"]["decision"], "Continue");
    assert_eq!(encoded["request_user_input"]["status"], "answered");
    assert_eq!(encoded["provider"], "codex");
    assert_eq!(encoded["model"], "profile-model");
    assert_eq!(encoded["llm_profile"], "team-profile");
    assert_eq!(encoded["reasoning_effort"], "medium");
    assert_eq!(encoded["chat_mode"], "chat");

    let decoded: AgentRequestUserInputAnswerRequest =
        serde_json::from_value(encoded).expect("deserialize answer request");
    assert_eq!(decoded.request_id, "request-1");
    assert_eq!(decoded.answers["decision"], "Continue");
    assert_eq!(
        decoded.metadata["spark.workspace.request_user_input.lookup_id"],
        json!("decision")
    );
}
