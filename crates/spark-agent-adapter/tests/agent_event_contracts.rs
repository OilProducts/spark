use serde_json::json;
use spark_agent_adapter::{AgentRawLogLine, AgentThreadResumeFailure, AgentTurnOutput};
use spark_common::events::{TurnStreamChannel, TurnStreamEvent, TurnStreamSource};

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
