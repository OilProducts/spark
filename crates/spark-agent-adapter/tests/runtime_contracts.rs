use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use spark_agent_adapter::{
    detect_loop, history_to_messages, tool_call_signature, AssistantTurn, CommandOptions, DirEntry,
    EnvironmentError, EnvironmentResult, EventKind, ExecResult, ExecutionEnvironment,
    ExecutionEnvironmentBackend, GrepOptions, HistoryTurn, LlmClientHandle, ProviderProfile,
    RegisteredTool, Session, SessionConfig, SessionEvent, SessionState, SteeringTurn, SystemTurn,
    ToolDefinition, ToolExecutionOutput, ToolRegistry, ToolResultsTurn, UserTurn,
    LOOP_DETECTION_WARNING,
};
use spark_common::events::{TurnStreamChannel, TurnStreamEventKind};
use unified_llm_adapter::{
    stream_events, AdapterError, AdapterErrorKind, Client, ContentPart, FinishReason, Message,
    MessageRole, ProviderAdapter, Request, Response, StreamEvent, StreamEventType, StreamEvents,
    Tool, ToolCall, ToolResult, ToolResultData, Usage,
};
use uuid::Uuid;

#[test]
fn public_runtime_contracts_are_exported_with_session_defaults() {
    let mut profile = ProviderProfile::new("fake-provider", "fake-model");
    profile.supports_reasoning = true;
    profile
        .tools
        .push(Tool::passive("lookup").expect("valid tool"));
    let environment = ExecutionEnvironment::local(".");
    let session = Session::new(
        profile.clone(),
        environment.clone(),
        SessionConfig::default(),
    );

    assert_ne!(session.id, Uuid::nil());
    assert_eq!(session.provider_profile, profile);
    assert_eq!(session.execution_environment, environment);
    assert_eq!(session.config.default_command_timeout_ms, 10_000);
    assert_eq!(session.config.max_command_timeout_ms, 600_000);
    assert!(session.config.enable_loop_detection);
    assert_eq!(session.config.loop_detection_window, 10);
    assert_eq!(session.config.max_subagent_depth, 1);
    assert_eq!(session.llm_client, LlmClientHandle::default());
    assert_eq!(session.state, SessionState::Idle);
    assert!(session.history.is_empty());
    assert!(session.steering_queue.is_empty());
    assert!(session.follow_up_queue.is_empty());
    assert!(session.active_subagents.is_empty());
    assert!(session.pending_question.is_none());
    assert!(!session.abort_signaled);
    assert!(session.provider_profile.supports("supports_reasoning"));
    assert_eq!(session.provider_profile.tool_definitions().len(), 1);

    let start_event = session.event_queue.front().expect("SESSION_START event");
    assert_eq!(start_event.kind, EventKind::SessionStart);
    assert_eq!(start_event.session_id, Some(session.id));
    assert_eq!(start_event.data["state"], json!("idle"));
}

#[test]
fn session_lifecycle_transitions_emit_observable_events_and_allow_reuse() {
    let mut session = Session::default();
    assert_eq!(
        session.next_event().expect("SESSION_START").kind,
        EventKind::SessionStart
    );

    session.mark_awaiting_input("What next?");
    assert_eq!(session.state, SessionState::AwaitingInput);

    session.submit("Answer one");
    assert_eq!(session.state, SessionState::Processing);
    let user_input = session.next_event().expect("USER_INPUT");
    assert_eq!(user_input.kind, EventKind::UserInput);
    assert_eq!(user_input.data["content"], json!("Answer one"));
    assert_eq!(user_input.data["answer_to"], json!("What next?"));
    assert!(session.pending_question.is_none());

    session.mark_natural_completion();
    assert_eq!(session.state, SessionState::Idle);
    let processing_end = session.next_event().expect("PROCESSING_END");
    assert_eq!(processing_end.kind, EventKind::ProcessingEnd);
    assert_eq!(processing_end.data["state"], json!("idle"));

    session.submit("Answer two");
    assert_eq!(session.state, SessionState::Processing);
    let user_input = session.next_event().expect("second USER_INPUT");
    assert_eq!(user_input.kind, EventKind::UserInput);
    assert_eq!(user_input.data["content"], json!("Answer two"));
    assert!(user_input.data.get("answer_to").is_none());
    assert_eq!(session.history.len(), 2);

    session.mark_turn_limit(Some(0), Some(session.history.len()));
    assert_eq!(session.state, SessionState::Idle);
    let turn_limit = session.next_event().expect("TURN_LIMIT");
    assert_eq!(turn_limit.kind, EventKind::TurnLimit);
    assert_eq!(turn_limit.data["state"], json!("idle"));
    assert_eq!(turn_limit.data["round_count"], json!(0));
    assert_eq!(turn_limit.data["total_turns"], json!(2));
    let processing_end = session.next_event().expect("limit PROCESSING_END");
    assert_eq!(processing_end.kind, EventKind::ProcessingEnd);
    assert_eq!(processing_end.data["state"], json!("idle"));
}

#[test]
fn session_end_is_emitted_once_with_structured_final_state_for_close_paths() {
    let mut explicit = Session::default();
    explicit.next_event();
    explicit.submit("hello");
    explicit.next_event();
    explicit.close();
    explicit.close();
    explicit.abort();

    let end = explicit.next_event().expect("SESSION_END");
    assert_eq!(end.kind, EventKind::SessionEnd);
    assert_eq!(end.data["state"], json!("closed"));
    assert_eq!(end.data["reason"], json!("explicit_close"));
    assert_eq!(end.data["final_state"]["state"], json!("closed"));
    assert_eq!(end.data["final_state"]["reason"], json!("explicit_close"));
    assert_eq!(end.data["final_state"]["history_turns"], json!(1));
    assert_eq!(end.data["final_state"]["abort_signaled"], json!(false));
    assert!(explicit.next_event().is_none());

    let mut aborted = Session::default();
    aborted.next_event();
    aborted.abort();
    aborted.abort();
    let error = aborted.next_event().expect("abort ERROR");
    assert_eq!(error.kind, EventKind::Error);
    assert_eq!(error.data["error"]["kind"], json!("abort"));
    assert_eq!(error.data["error"]["name"], json!("AbortError"));
    assert_eq!(error.data["error"]["message"], json!("session is aborted"));
    let end = aborted.next_event().expect("abort SESSION_END");
    assert_eq!(end.kind, EventKind::SessionEnd);
    assert_eq!(end.data["reason"], json!("abort"));
    assert_eq!(end.data["error"], error.data["error"]);
    assert_eq!(end.data["final_state"]["abort_signaled"], json!(true));
    assert_eq!(end.data["final_state"]["error"], error.data["error"]);
    assert!(aborted.next_event().is_none());

    let mut failed = Session::default();
    failed.next_event();
    failed.mark_unrecoverable_error("boom");
    failed.close();

    let error = failed.next_event().expect("ERROR");
    assert_eq!(error.kind, EventKind::Error);
    assert_eq!(error.data["error"]["kind"], json!("provider"));
    assert_eq!(error.data["error"]["name"], json!("ProviderError"));
    assert_eq!(error.data["error"]["message"], json!("boom"));
    assert_eq!(error.data["error"]["retryable"], json!(false));
    let end = failed.next_event().expect("error SESSION_END");
    assert_eq!(end.kind, EventKind::SessionEnd);
    assert_eq!(end.data["reason"], json!("unrecoverable_error"));
    assert_eq!(end.data["error"], error.data["error"]);
    assert_eq!(end.data["final_state"]["error"], error.data["error"]);
    assert!(failed.next_event().is_none());
}

#[test]
fn event_kind_serializes_to_stable_agent_event_strings() {
    assert_eq!(
        serde_json::to_value(EventKind::AssistantTextDelta).expect("serialize"),
        json!("assistant_text_delta")
    );
    assert_eq!(
        serde_json::to_value(EventKind::ModelToolCallEnd).expect("serialize"),
        json!("model_tool_call_end")
    );
    assert_eq!(
        serde_json::to_value(EventKind::ToolCallOutputDelta).expect("serialize"),
        json!("tool_call_output_delta")
    );

    let decoded: EventKind =
        serde_json::from_value(json!("assistant_reasoning_end")).expect("deserialize");
    assert_eq!(decoded, EventKind::AssistantReasoningEnd);
    let uppercase: EventKind =
        serde_json::from_value(json!("ASSISTANT_TEXT_START")).expect("deserialize uppercase");
    assert_eq!(uppercase, EventKind::AssistantTextStart);
    let custom: EventKind =
        serde_json::from_value(json!("provider_custom")).expect("deserialize custom");
    assert_eq!(custom.as_str(), "provider_custom");
}

#[test]
fn session_events_map_to_workspace_turn_stream_events_without_renaming_workspace_kinds() {
    let session_id = Uuid::new_v4();
    let text_delta = SessionEvent::new(
        EventKind::AssistantTextDelta,
        session_id,
        BTreeMap::from([
            ("delta".to_string(), json!("Hello")),
            ("response_id".to_string(), json!("resp-1")),
        ]),
    );

    let stream_event = text_delta.to_turn_stream_event().expect("mapped event");
    assert_eq!(stream_event.kind, TurnStreamEventKind::ContentDelta);
    assert_eq!(stream_event.channel, Some(TurnStreamChannel::Assistant));
    assert_eq!(stream_event.content_delta.as_deref(), Some("Hello"));
    assert_eq!(stream_event.message.as_deref(), Some("Hello"));
    assert_eq!(
        stream_event.source.session_id.as_deref(),
        Some(session_id.to_string().as_str())
    );
    assert_eq!(
        stream_event.source.raw_kind.as_deref(),
        Some("assistant_text_delta")
    );
    assert_eq!(stream_event.source.response_id.as_deref(), Some("resp-1"));

    let reasoning_end = SessionEvent::new(
        EventKind::AssistantReasoningEnd,
        session_id,
        BTreeMap::from([("text".to_string(), json!("Because"))]),
    );
    let stream_event = reasoning_end.to_turn_stream_event().expect("mapped event");
    assert_eq!(stream_event.kind, TurnStreamEventKind::ContentCompleted);
    assert_eq!(stream_event.channel, Some(TurnStreamChannel::Reasoning));
    assert_eq!(stream_event.content_delta.as_deref(), Some("Because"));

    let usage = SessionEvent::new(
        EventKind::ModelUsageUpdate,
        session_id,
        BTreeMap::from([(
            "usage".to_string(),
            json!({
                "input_tokens": 3,
                "output_tokens": 4,
                "total_tokens": 7,
                "reasoning_tokens": 2,
                "cache_read_tokens": 1
            }),
        )]),
    );
    let stream_event = usage.to_turn_stream_event().expect("usage event");
    assert_eq!(stream_event.kind, TurnStreamEventKind::TokenUsageUpdated);
    assert_eq!(
        stream_event.token_usage,
        Some(json!({
            "total": {
                "inputTokens": 3,
                "cachedInputTokens": 1,
                "outputTokens": 4,
                "reasoningOutputTokens": 2,
                "totalTokens": 7
            }
        }))
    );

    let failed_tool = SessionEvent::new(
        EventKind::ToolCallEnd,
        session_id,
        BTreeMap::from([
            ("id".to_string(), json!("call-1")),
            ("kind".to_string(), json!("command_execution")),
            ("tool_name".to_string(), json!("lookup")),
            ("output".to_string(), json!("partial output")),
            ("error".to_string(), json!("not found")),
        ]),
    );
    let stream_event = failed_tool.to_turn_stream_event().expect("tool event");
    assert_eq!(stream_event.kind, TurnStreamEventKind::ToolCallFailed);
    assert_eq!(stream_event.error, None);
    let tool_call = stream_event.tool_call.expect("tool payload");
    assert_eq!(tool_call["id"], "call-1");
    assert_eq!(tool_call["kind"], "command_execution");
    assert_eq!(tool_call["status"], "failed");
    assert_eq!(tool_call["title"], "lookup");
    assert_eq!(tool_call["output"], "partial output");
    assert_eq!(tool_call["error"], "not found");

    let request_user_input = SessionEvent::new(
        EventKind::Other("request_user_input_requested".to_string()),
        session_id,
        BTreeMap::from([(
            "request_user_input".to_string(),
            json!({"request_id": "request-1", "status": "pending"}),
        )]),
    );
    let stream_event = request_user_input
        .to_turn_stream_event()
        .expect("request user input event");
    assert_eq!(
        stream_event.kind,
        TurnStreamEventKind::RequestUserInputRequested
    );
    assert_eq!(
        stream_event.request_user_input,
        Some(json!({"request_id": "request-1", "status": "pending"}))
    );
}

#[test]
fn spark_mapping_keeps_model_proposed_tool_calls_distinct_from_execution_tool_calls() {
    let session_id = Uuid::new_v4();
    for kind in [
        EventKind::ModelToolCallStart,
        EventKind::ModelToolCallDelta,
        EventKind::ModelToolCallEnd,
    ] {
        let proposed_tool = SessionEvent::new(
            kind,
            session_id,
            BTreeMap::from([(
                "tool_call".to_string(),
                json!({"id": "proposed-1", "name": "lookup", "arguments": {"query": "rust"}}),
            )]),
        );

        assert!(proposed_tool.to_turn_stream_event().is_none());
    }

    let execution_start = SessionEvent::new(
        EventKind::ToolCallStart,
        session_id,
        BTreeMap::from([
            ("id".to_string(), json!("exec-1")),
            ("kind".to_string(), json!("command_execution")),
            ("tool_name".to_string(), json!("shell")),
            ("command".to_string(), json!("cargo test")),
        ]),
    )
    .to_turn_stream_event()
    .expect("execution start maps");
    assert_eq!(execution_start.kind, TurnStreamEventKind::ToolCallStarted);
    assert_eq!(
        execution_start.tool_call,
        Some(json!({
            "id": "exec-1",
            "kind": "command_execution",
            "status": "running",
            "title": "shell",
            "command": "cargo test",
            "output": null,
            "error": null
        }))
    );

    let execution_delta = SessionEvent::new(
        EventKind::ToolCallOutputDelta,
        session_id,
        BTreeMap::from([
            ("id".to_string(), json!("exec-1")),
            ("delta".to_string(), json!("line one\n")),
        ]),
    )
    .to_turn_stream_event()
    .expect("execution update maps");
    assert_eq!(execution_delta.kind, TurnStreamEventKind::ToolCallUpdated);
    assert_eq!(execution_delta.content_delta.as_deref(), Some("line one\n"));
    assert_eq!(
        execution_delta.tool_call,
        Some(json!({
            "id": "exec-1",
            "kind": "tool_call",
            "status": "running",
            "title": "Tool call",
            "output": "line one\n",
            "error": null
        }))
    );
}

#[test]
fn history_turns_convert_to_unified_llm_messages() {
    let user = HistoryTurn::User(UserTurn::new("User request"));
    let system = HistoryTurn::System(SystemTurn::new("System prompt"));
    let mut assistant = AssistantTurn::new(vec![ContentPart::text("Assistant reply")]);
    assistant.reasoning = Some("Internal reasoning".to_string());
    assistant
        .tool_calls
        .push(ToolCall::new("call_1", "lookup", json!({"query": "rust"})));
    let assistant = HistoryTurn::Assistant(assistant);
    let tool_results = HistoryTurn::ToolResults(ToolResultsTurn::new([ToolResultData {
        tool_call_id: "call_1".to_string(),
        content: json!("Tool output"),
        is_error: false,
        image_data: None,
        image_media_type: None,
    }]));

    let messages = history_to_messages(&[system, user, assistant, tool_results]);

    assert_eq!(messages.len(), 4);
    assert_eq!(messages[0].role, MessageRole::System);
    assert_eq!(messages[0].text(), "System prompt");
    assert_eq!(messages[1].role, MessageRole::User);
    assert_eq!(messages[1].text(), "User request");
    assert_eq!(messages[2].role, MessageRole::Assistant);
    assert_eq!(messages[2].text(), "Assistant reply");
    assert!(messages[2]
        .content
        .iter()
        .any(|part| matches!(part, ContentPart::Thinking { .. })));
    assert!(messages[2]
        .content
        .iter()
        .any(|part| matches!(part, ContentPart::ToolCall { .. })));
    assert_eq!(messages[3].role, MessageRole::Tool);
    assert_eq!(messages[3].tool_call_id.as_deref(), Some("call_1"));
}

#[test]
fn session_history_messages_preserve_turn_order_and_tool_result_types() {
    let mut session = Session::default();
    session
        .history
        .push(HistoryTurn::System(SystemTurn::new("system")));
    session
        .history
        .push(HistoryTurn::User(UserTurn::new("user")));
    session
        .history
        .push(HistoryTurn::Steering(SteeringTurn::new("steer")));
    session
        .history
        .push(HistoryTurn::ToolResults(ToolResultsTurn::new([
            ToolResult::success("call_1", json!("tool output")),
        ])));

    let messages = session.history_messages();

    assert_eq!(messages.len(), 4);
    assert_eq!(messages[0].role, MessageRole::System);
    assert_eq!(messages[0].text(), "system");
    assert_eq!(messages[1].role, MessageRole::User);
    assert_eq!(messages[1].text(), "user");
    assert_eq!(messages[2].role, MessageRole::User);
    assert_eq!(messages[2].text(), "steer");
    assert_eq!(messages[3].role, MessageRole::Tool);
    assert_eq!(messages[3].tool_call_id.as_deref(), Some("call_1"));
}

#[test]
fn process_input_builds_low_level_request_and_records_text_completion() {
    let adapter = Arc::new(ScriptedAdapter::with_complete_responses(vec![
        assistant_response("resp-1", "Assistant reply", Some("Assistant thinking")),
    ]));
    let client_adapter: Arc<dyn ProviderAdapter> = adapter.clone();
    let client =
        Client::from_adapters(vec![client_adapter], Some("fake-provider")).expect("client");
    let mut profile = ProviderProfile::new("fake-provider", "fake-model");
    profile.system_prompt = "Session system prompt".to_string();
    profile.tools.push(
        Tool::passive_with_schema(
            "lookup",
            Some("Lookup values".to_string()),
            Some(json!({"type": "object"})),
        )
        .expect("valid tool"),
    );
    profile
        .provider_options
        .insert("temperature".to_string(), json!(0.2));
    let mut session = Session::new(
        profile,
        ExecutionEnvironment::local("."),
        SessionConfig {
            reasoning_effort: Some("high".to_string()),
            ..SessionConfig::default()
        },
    );
    assert_eq!(
        session.next_event().expect("SESSION_START").kind,
        EventKind::SessionStart
    );

    session.mark_awaiting_input("What next?");
    session.queue_steering("preloaded steering");
    session
        .process_input(&client, "Answer one")
        .expect("process input");

    let calls = adapter.complete_requests();
    assert_eq!(calls.len(), 1);
    let request = &calls[0];
    assert_eq!(request.provider.as_deref(), Some("fake-provider"));
    assert_eq!(request.model, "fake-model");
    assert_eq!(request.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(
        request.provider_options,
        BTreeMap::from([("fake-provider".to_string(), json!({"temperature": 0.2}))])
    );
    assert_eq!(request.tools.len(), 1);
    assert_eq!(request.tools[0].name, "lookup");
    assert_eq!(
        request
            .tool_choice
            .as_ref()
            .map(|choice| choice.mode.as_str()),
        Some("auto")
    );
    assert_eq!(
        request
            .tool_choice
            .as_ref()
            .and_then(|choice| choice.tool_name.as_deref()),
        None
    );
    assert_eq!(request.messages.len(), 3);
    assert_eq!(request.messages[0].role, MessageRole::System);
    let system_prompt = request.messages[0].text();
    assert!(system_prompt.contains("<provider_base_instructions>"));
    assert!(system_prompt.contains("Session system prompt"));
    assert!(system_prompt.contains("<environment>"));
    assert!(system_prompt.contains("Working directory:"));
    assert!(system_prompt.contains("<tools>"));
    assert!(system_prompt.contains("lookup"));
    assert!(system_prompt.contains("Lookup values"));
    assert_eq!(request.messages[1], Message::user("Answer one"));
    assert_eq!(request.messages[2], Message::user("preloaded steering"));

    assert_eq!(session.state, SessionState::Idle);
    assert!(session.pending_question.is_none());
    assert!(session.steering_queue.is_empty());
    assert_eq!(
        session
            .history
            .iter()
            .map(|turn| match turn {
                HistoryTurn::User(_) => "UserTurn",
                HistoryTurn::Steering(_) => "SteeringTurn",
                HistoryTurn::Assistant(_) => "AssistantTurn",
                HistoryTurn::System(_) => "SystemTurn",
                HistoryTurn::ToolResults(_) => "ToolResultsTurn",
            })
            .collect::<Vec<_>>(),
        vec!["UserTurn", "SteeringTurn", "AssistantTurn"]
    );
    let HistoryTurn::Assistant(assistant_turn) = &session.history[2] else {
        panic!("assistant turn recorded");
    };
    assert_eq!(assistant_turn.text(), "Assistant reply");
    assert_eq!(
        assistant_turn.reasoning.as_deref(),
        Some("Assistant thinking")
    );
    assert_eq!(assistant_turn.response_id.as_deref(), Some("resp-1"));
    assert_eq!(
        assistant_turn.usage,
        Some(Usage {
            input_tokens: 3,
            output_tokens: 5,
            total_tokens: 8,
            ..Usage::default()
        })
    );
    assert!(assistant_turn.timestamp <= time::OffsetDateTime::now_utc());

    let user_input = session.next_event().expect("USER_INPUT");
    assert_eq!(user_input.kind, EventKind::UserInput);
    assert_eq!(user_input.data["content"], json!("Answer one"));
    assert_eq!(user_input.data["answer_to"], json!("What next?"));
    let steering = session.next_event().expect("STEERING_INJECTED");
    assert_eq!(steering.kind, EventKind::SteeringInjected);
    assert_eq!(steering.data["content"], json!("preloaded steering"));
    let text_start = session.next_event().expect("ASSISTANT_TEXT_START");
    assert_eq!(text_start.kind, EventKind::AssistantTextStart);
    assert_eq!(text_start.data["response_id"], json!("resp-1"));
    let text_delta = session.next_event().expect("ASSISTANT_TEXT_DELTA");
    assert_eq!(text_delta.kind, EventKind::AssistantTextDelta);
    assert_eq!(text_delta.data["delta"], json!("Assistant reply"));
    assert_eq!(text_delta.data["response_id"], json!("resp-1"));
    let text_end = session.next_event().expect("ASSISTANT_TEXT_END");
    assert_eq!(text_end.kind, EventKind::AssistantTextEnd);
    assert_eq!(text_end.data["text"], json!("Assistant reply"));
    assert_eq!(text_end.data["reasoning"], json!("Assistant thinking"));
    assert_eq!(text_end.data["response_id"], json!("resp-1"));
    let processing_end = session.next_event().expect("PROCESSING_END");
    assert_eq!(processing_end.kind, EventKind::ProcessingEnd);
    assert_eq!(processing_end.data["state"], json!("idle"));
    assert!(session.next_event().is_none());
}

#[test]
fn process_input_delays_follow_ups_until_natural_completion_and_preserves_history() {
    let adapter = Arc::new(ScriptedAdapter::with_complete_responses(vec![
        assistant_response("resp-question", "Need more info?", None),
        assistant_response("resp-answer", "Answered", None),
        assistant_response("resp-follow-up", "Follow-up handled", None),
    ]));
    let client_adapter: Arc<dyn ProviderAdapter> = adapter.clone();
    let client =
        Client::from_adapters(vec![client_adapter], Some("fake-provider")).expect("client");
    let mut profile = ProviderProfile::new("fake-provider", "fake-model");
    profile.system_prompt = "Session system prompt".to_string();
    let mut session = Session::new(
        profile,
        ExecutionEnvironment::default(),
        SessionConfig::default(),
    );
    session.system_prompt_snapshot = "sys".to_string();
    session.next_event();

    session.follow_up("Queued follow-up");
    session
        .process_input(&client, "Initial input")
        .expect("first input");

    assert_eq!(adapter.complete_requests().len(), 1);
    assert_eq!(session.state, SessionState::AwaitingInput);
    assert_eq!(session.pending_question.as_deref(), Some("Need more info?"));
    assert_eq!(session.follow_up_queue.len(), 1);
    assert_eq!(turn_names(&session.history), ["UserTurn", "AssistantTurn"]);
    let first_events = drain_events(&mut session);
    assert!(!first_events
        .iter()
        .any(|event| event.kind == EventKind::ProcessingEnd));

    session
        .process_input(&client, "Yes please")
        .expect("answer and queued follow-up");

    let calls = adapter.complete_requests();
    assert_eq!(calls.len(), 3);
    assert_eq!(
        calls[0].messages,
        vec![Message::system("sys"), Message::user("Initial input")]
    );
    assert_eq!(
        calls[1].messages,
        [
            vec![Message::system("sys")],
            history_to_messages(&session.history[..3])
        ]
        .concat()
    );
    assert_eq!(
        calls[2].messages,
        [
            vec![Message::system("sys")],
            history_to_messages(&session.history[..5])
        ]
        .concat()
    );

    assert_eq!(session.state, SessionState::Idle);
    assert!(session.pending_question.is_none());
    assert!(session.follow_up_queue.is_empty());
    assert_eq!(
        session
            .history
            .iter()
            .map(|turn| match turn {
                HistoryTurn::User(turn) => turn.text(),
                HistoryTurn::Assistant(turn) => turn.text(),
                HistoryTurn::Steering(turn) => turn.text(),
                HistoryTurn::System(turn) => turn.text(),
                HistoryTurn::ToolResults(_) => "<tool-results>".to_string(),
            })
            .collect::<Vec<_>>(),
        [
            "Initial input",
            "Need more info?",
            "Yes please",
            "Answered",
            "Queued follow-up",
            "Follow-up handled",
        ]
    );

    let events = drain_events(&mut session);
    let kinds = events
        .iter()
        .map(|event| event.kind.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            EventKind::UserInput,
            EventKind::AssistantTextStart,
            EventKind::AssistantTextDelta,
            EventKind::AssistantTextEnd,
            EventKind::UserInput,
            EventKind::AssistantTextStart,
            EventKind::AssistantTextDelta,
            EventKind::AssistantTextEnd,
            EventKind::ProcessingEnd,
        ]
    );
    assert_eq!(events[0].data["content"], json!("Yes please"));
    assert_eq!(events[0].data["answer_to"], json!("Need more info?"));
    assert_eq!(events[4].data["content"], json!("Queued follow-up"));
    assert!(events[4].data.get("answer_to").is_none());
}

#[test]
fn process_input_executes_tool_rounds_and_injects_mid_tool_steering_before_next_request() {
    let adapter = Arc::new(ScriptedAdapter::with_complete_responses(vec![
        tool_call_response("resp-tool"),
        assistant_response("resp-final", "All done", None),
    ]));
    let client_adapter: Arc<dyn ProviderAdapter> = adapter.clone();
    let client =
        Client::from_adapters(vec![client_adapter], Some("fake-provider")).expect("client");
    let mut profile = ProviderProfile::new("fake-provider", "fake-model");
    profile.system_prompt = "Session system prompt".to_string();
    let mut registry = ToolRegistry::new();
    registry.register(RegisteredTool::new_with_executor(
        ToolDefinition::new(
            "lookup",
            "Lookup values",
            Some(json!({
                "type": "object",
                "properties": {"query": {"type": "string"}},
                "required": ["query"],
                "additionalProperties": false
            })),
        )
        .expect("tool definition"),
        Arc::new(|invocation| {
            assert_eq!(invocation.tool_call_id, "call-1");
            assert_eq!(invocation.arguments, json!({"query": "rust"}));
            assert!(invocation.host_controls.steer("tool steering"));
            Ok(ToolExecutionOutput::success(Value::String(
                "tool result".to_string(),
            )))
        }),
    ));
    profile.set_tool_registry(registry);
    let mut session = Session::new(
        profile,
        ExecutionEnvironment::default(),
        SessionConfig {
            reasoning_effort: Some("high".to_string()),
            ..SessionConfig::default()
        },
    );
    session.system_prompt_snapshot = "sys".to_string();
    session.next_event();

    session.steer("preloaded steering");
    session
        .process_input(&client, "Question")
        .expect("tool input");

    let calls = adapter.complete_requests();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].reasoning_effort.as_deref(), Some("high"));
    assert_eq!(
        calls[0].messages,
        vec![
            Message::system("sys"),
            Message::user("Question"),
            Message::user("preloaded steering"),
        ]
    );
    assert_eq!(
        calls[1].messages,
        [
            vec![Message::system("sys")],
            history_to_messages(&session.history[..5])
        ]
        .concat()
    );
    assert_eq!(calls[1].reasoning_effort.as_deref(), Some("high"));

    assert_eq!(
        turn_names(&session.history),
        [
            "UserTurn",
            "SteeringTurn",
            "AssistantTurn",
            "ToolResultsTurn",
            "SteeringTurn",
            "AssistantTurn",
        ]
    );
    let HistoryTurn::ToolResults(tool_results) = &session.history[3] else {
        panic!("tool results recorded");
    };
    assert_eq!(tool_results.result_list[0].content, json!("tool result"));
    let HistoryTurn::Steering(steering) = &session.history[4] else {
        panic!("tool steering recorded");
    };
    assert_eq!(steering.text(), "tool steering");

    let events = drain_events(&mut session);
    let kinds = events
        .iter()
        .map(|event| event.kind.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            EventKind::UserInput,
            EventKind::SteeringInjected,
            EventKind::AssistantTextStart,
            EventKind::AssistantTextDelta,
            EventKind::AssistantTextEnd,
            EventKind::ModelToolCallEnd,
            EventKind::ToolCallStart,
            EventKind::ToolCallEnd,
            EventKind::SteeringInjected,
            EventKind::AssistantTextStart,
            EventKind::AssistantTextDelta,
            EventKind::AssistantTextEnd,
            EventKind::ProcessingEnd,
        ]
    );
    assert_eq!(events[1].data["content"], json!("preloaded steering"));
    assert_eq!(events[6].data["tool_call_id"], json!("call-1"));
    assert_eq!(events[7].data["output"], json!("tool result"));
    assert_eq!(events[8].data["content"], json!("tool steering"));
    assert_eq!(
        events.last().expect("processing end").data["state"],
        json!("idle")
    );
}

#[test]
fn context_usage_stays_quiet_at_the_provider_threshold_boundary() {
    let adapter = Arc::new(ScriptedAdapter::with_complete_responses(vec![
        assistant_response("resp-boundary", "Boundary reply", None),
    ]));
    let client_adapter: Arc<dyn ProviderAdapter> = adapter.clone();
    let client =
        Client::from_adapters(vec![client_adapter], Some("fake-provider")).expect("client");
    let mut profile = ProviderProfile::new("fake-provider", "fake-model");
    profile.context_window_size = Some(100);
    let mut session = Session::new(
        profile,
        ExecutionEnvironment::default(),
        SessionConfig::default(),
    );
    session.system_prompt_snapshot = "sys".to_string();
    session.next_event();

    session
        .process_input(&client, "x".repeat(317))
        .expect("process input");

    let calls = adapter.complete_requests();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].messages[0], Message::system("sys"));
    assert_eq!(calls[0].messages[1], Message::user("x".repeat(317)));

    let events = drain_events(&mut session);
    assert!(!events.iter().any(|event| event.kind == EventKind::Warning));
    assert_eq!(session.state, SessionState::Idle);
    assert_eq!(session.history.len(), 2);
}

#[test]
fn context_usage_warning_includes_structured_estimate_without_mutating_history_or_state() {
    let adapter = Arc::new(ScriptedAdapter::with_complete_responses(vec![
        assistant_response("resp-warning", "Warning reply", None),
    ]));
    let client_adapter: Arc<dyn ProviderAdapter> = adapter.clone();
    let client =
        Client::from_adapters(vec![client_adapter], Some("fake-provider")).expect("client");
    let mut profile = ProviderProfile::new("fake-provider", "fake-model");
    profile.context_window_size = Some(100);
    let mut session = Session::new(
        profile,
        ExecutionEnvironment::default(),
        SessionConfig::default(),
    );
    session.system_prompt_snapshot = "sys".to_string();
    session.next_event();

    session
        .process_input(&client, "x".repeat(397))
        .expect("process input");

    let calls = adapter.complete_requests();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].messages[0], Message::system("sys"));
    assert_eq!(calls[0].messages[1], Message::user("x".repeat(397)));

    let events = drain_events(&mut session);
    let kinds = events
        .iter()
        .map(|event| event.kind.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            EventKind::UserInput,
            EventKind::Warning,
            EventKind::AssistantTextStart,
            EventKind::AssistantTextDelta,
            EventKind::AssistantTextEnd,
            EventKind::ProcessingEnd,
        ]
    );
    let warning = &events[1];
    assert_eq!(
        warning.data["message"],
        json!("Context usage at ~100% of context window")
    );
    assert_eq!(warning.data["usage"]["approximate_characters"], json!(400));
    assert_json_f64(&warning.data["usage"]["approximate_tokens"], 100.0);
    assert_json_f64(&warning.data["usage"]["threshold_tokens"], 80.0);
    assert_json_f64(&warning.data["usage"]["threshold_ratio"], 0.8);
    assert_json_f64(&warning.data["usage"]["usage_ratio"], 1.0);
    assert_eq!(warning.data["usage"]["context_window_size"], json!(100));

    assert_eq!(session.state, SessionState::Idle);
    assert!(session.pending_question.is_none());
    assert_eq!(
        session
            .history
            .iter()
            .map(|turn| match turn {
                HistoryTurn::User(_) => "UserTurn",
                HistoryTurn::Steering(_) => "SteeringTurn",
                HistoryTurn::Assistant(_) => "AssistantTurn",
                HistoryTurn::System(_) => "SystemTurn",
                HistoryTurn::ToolResults(_) => "ToolResultsTurn",
            })
            .collect::<Vec<_>>(),
        vec!["UserTurn", "AssistantTurn"]
    );
}

#[test]
fn process_input_uses_stream_directly_when_profile_supports_streaming() {
    let adapter = Arc::new(ScriptedAdapter::with_stream_events(vec![vec![
        StreamEvent::text_delta("streamed "),
        StreamEvent::text_delta("reply"),
        StreamEvent::finish(
            FinishReason::Stop,
            Some(Usage {
                input_tokens: 2,
                output_tokens: 4,
                total_tokens: 6,
                ..Usage::default()
            }),
        ),
    ]]));
    let client_adapter: Arc<dyn ProviderAdapter> = adapter.clone();
    let client =
        Client::from_adapters(vec![client_adapter], Some("fake-provider")).expect("client");
    let mut profile = ProviderProfile::new("fake-provider", "fake-model");
    profile.system_prompt = "Session system prompt".to_string();
    profile.supports_streaming = true;
    let mut session = Session::new(
        profile,
        ExecutionEnvironment::default(),
        SessionConfig::default(),
    );
    session.next_event();

    session
        .process_input(&client, "Question")
        .expect("process input");

    assert!(adapter.complete_requests().is_empty());
    assert_eq!(adapter.stream_requests().len(), 1);
    let HistoryTurn::Assistant(assistant_turn) = session.history.last().expect("assistant") else {
        panic!("assistant turn recorded");
    };
    assert_eq!(assistant_turn.text(), "streamed reply");
    assert_eq!(
        assistant_turn.usage,
        Some(Usage {
            input_tokens: 2,
            output_tokens: 4,
            total_tokens: 6,
            ..Usage::default()
        })
    );
}

#[test]
fn process_input_streaming_emits_typed_session_events_in_stream_order() {
    let adapter = Arc::new(ScriptedAdapter::with_stream_events(vec![vec![
        StreamEvent {
            response: Some(Response {
                id: "resp-1".to_string(),
                model: "fake-model".to_string(),
                provider: "fake-provider".to_string(),
                ..Response::default()
            }),
            ..StreamEvent::new(StreamEventType::StreamStart)
        },
        StreamEvent {
            delta: Some("Hello ".to_string()),
            ..StreamEvent::new(StreamEventType::TextStart)
        },
        StreamEvent::text_delta("world"),
        StreamEvent {
            reasoning_delta: Some("think ".to_string()),
            ..StreamEvent::new(StreamEventType::ReasoningStart)
        },
        StreamEvent {
            reasoning_delta: Some("more".to_string()),
            ..StreamEvent::new(StreamEventType::ReasoningDelta)
        },
        StreamEvent::new(StreamEventType::ReasoningEnd),
        StreamEvent {
            tool_call: Some(ToolCall::new("call-1", "lookup", json!({"query": "rust"}))),
            ..StreamEvent::new(StreamEventType::ToolCallStart)
        },
        StreamEvent {
            delta: Some("{\"query\":\"rust\"}".to_string()),
            tool_call: Some(ToolCall::from_raw_arguments(
                "call-1",
                "lookup",
                "{\"query\":\"rust\"}",
            )),
            ..StreamEvent::new(StreamEventType::ToolCallDelta)
        },
        StreamEvent {
            tool_call: Some(ToolCall::new("call-1", "lookup", json!({"query": "rust"}))),
            ..StreamEvent::new(StreamEventType::ToolCallEnd)
        },
        StreamEvent::finish(
            FinishReason::Stop,
            Some(Usage {
                input_tokens: 2,
                output_tokens: 4,
                total_tokens: 6,
                ..Usage::default()
            }),
        ),
    ]]));
    let client_adapter: Arc<dyn ProviderAdapter> = adapter.clone();
    let client =
        Client::from_adapters(vec![client_adapter], Some("fake-provider")).expect("client");
    let mut profile = ProviderProfile::new("fake-provider", "fake-model");
    profile.supports_streaming = true;
    let mut session = Session::new(
        profile,
        ExecutionEnvironment::default(),
        SessionConfig {
            max_tool_rounds_per_input: 1,
            ..SessionConfig::default()
        },
    );
    session.next_event();

    session
        .process_input(&client, "Question")
        .expect("process input");

    let events = drain_events(&mut session);
    let kinds = events
        .iter()
        .map(|event| event.kind.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            EventKind::UserInput,
            EventKind::AssistantTextStart,
            EventKind::AssistantTextDelta,
            EventKind::AssistantTextDelta,
            EventKind::AssistantReasoningStart,
            EventKind::AssistantReasoningDelta,
            EventKind::AssistantReasoningDelta,
            EventKind::AssistantReasoningEnd,
            EventKind::ModelToolCallStart,
            EventKind::ModelToolCallDelta,
            EventKind::ModelToolCallEnd,
            EventKind::AssistantTextEnd,
            EventKind::ModelUsageUpdate,
            EventKind::ToolCallStart,
            EventKind::ToolCallEnd,
            EventKind::TurnLimit,
            EventKind::ProcessingEnd,
        ]
    );
    assert!(!kinds.contains(&EventKind::ToolCallOutputDelta));

    assert_eq!(events[1].data["response_id"], json!("resp-1"));
    assert_eq!(events[2].data["delta"], json!("Hello "));
    assert_eq!(events[3].data["delta"], json!("world"));
    assert_eq!(events[7].data["text"], json!("think more"));
    assert_eq!(events[8].data["tool_call"]["id"], json!("call-1"));
    assert_eq!(events[9].data["delta"], json!("{\"query\":\"rust\"}"));
    assert_eq!(events[10].data["tool_call"]["name"], json!("lookup"));
    assert_eq!(events[11].data["text"], json!("Hello world"));
    assert_eq!(events[11].data["reasoning"], json!("think more"));
    assert_eq!(events[12].data["usage"]["total_tokens"], json!(6));
    assert_eq!(events[13].data["tool_call_id"], json!("call-1"));
    assert_eq!(events[13].data["tool_name"], json!("lookup"));
    assert_eq!(events[14].data["tool_call_id"], json!("call-1"));
    assert_eq!(events[14].data["tool_name"], json!("lookup"));
    assert_eq!(events[14].data["error"], json!("Unknown tool: lookup"));
    assert_eq!(events[15].data["round_count"], json!(1));
    assert_eq!(events[15].data["total_turns"], json!(3));

    let HistoryTurn::Assistant(assistant_turn) = &session.history[1] else {
        panic!("assistant turn recorded");
    };
    assert_eq!(assistant_turn.text(), "Hello world");
    assert_eq!(assistant_turn.reasoning.as_deref(), Some("think more"));
    assert_eq!(assistant_turn.tool_calls[0].id, "call-1");
    let HistoryTurn::ToolResults(tool_results) = session.history.last().expect("tool results")
    else {
        panic!("tool results recorded");
    };
    assert_eq!(tool_results.result_list[0].tool_call_id, "call-1");
    assert!(tool_results.result_list[0].is_error);
}

#[test]
fn process_input_streaming_error_records_partial_turn_before_error_close() {
    let stream_error = AdapterError::new(AdapterErrorKind::Stream, "boom");
    let adapter = Arc::new(ScriptedAdapter::with_stream_events(vec![vec![
        StreamEvent {
            response: Some(Response {
                id: "resp-err".to_string(),
                model: "fake-model".to_string(),
                provider: "fake-provider".to_string(),
                ..Response::default()
            }),
            ..StreamEvent::new(StreamEventType::StreamStart)
        },
        StreamEvent::text_delta("partial"),
        StreamEvent {
            error: Some(stream_error.clone()),
            raw: Some(json!({"error": "boom"})),
            ..StreamEvent::new(StreamEventType::Error)
        },
    ]]));
    let client_adapter: Arc<dyn ProviderAdapter> = adapter.clone();
    let client =
        Client::from_adapters(vec![client_adapter], Some("fake-provider")).expect("client");
    let mut profile = ProviderProfile::new("fake-provider", "fake-model");
    profile.supports_streaming = true;
    let mut session = Session::new(
        profile,
        ExecutionEnvironment::default(),
        SessionConfig::default(),
    );
    session.next_event();

    let error = session
        .process_input(&client, "Question")
        .expect_err("stream error");

    assert_eq!(error.kind, AdapterErrorKind::Stream);
    assert_eq!(error.message, "boom");
    assert_eq!(session.state, SessionState::Closed);

    let events = drain_events(&mut session);
    let kinds = events
        .iter()
        .map(|event| event.kind.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            EventKind::UserInput,
            EventKind::AssistantTextStart,
            EventKind::AssistantTextDelta,
            EventKind::AssistantTextEnd,
            EventKind::Error,
            EventKind::SessionEnd,
        ]
    );
    assert_eq!(events[2].data["delta"], json!("partial"));
    assert_eq!(events[3].data["text"], json!("partial"));
    assert_eq!(events[4].data["error"]["kind"], json!("stream"));
    assert_eq!(events[4].data["error"]["name"], json!("StreamError"));
    assert_eq!(events[4].data["error"]["message"], json!("boom"));
    assert_eq!(events[5].data["error"], events[4].data["error"]);
    assert_eq!(
        events[5].data["final_state"]["error"],
        events[4].data["error"]
    );

    assert_eq!(
        session
            .history
            .iter()
            .map(|turn| match turn {
                HistoryTurn::User(_) => "UserTurn",
                HistoryTurn::Steering(_) => "SteeringTurn",
                HistoryTurn::Assistant(_) => "AssistantTurn",
                HistoryTurn::System(_) => "SystemTurn",
                HistoryTurn::ToolResults(_) => "ToolResultsTurn",
            })
            .collect::<Vec<_>>(),
        vec!["UserTurn", "AssistantTurn"]
    );
    let HistoryTurn::Assistant(assistant_turn) = session.history.last().expect("assistant") else {
        panic!("assistant turn recorded");
    };
    assert_eq!(assistant_turn.text(), "partial");
}

#[test]
fn process_input_limits_stop_before_or_after_the_low_level_request() {
    let adapter = Arc::new(ScriptedAdapter::with_complete_responses(vec![
        tool_call_response("resp-tool"),
    ]));
    let client_adapter: Arc<dyn ProviderAdapter> = adapter.clone();
    let client =
        Client::from_adapters(vec![client_adapter], Some("fake-provider")).expect("client");
    let mut limited_turns = Session::new(
        ProviderProfile::new("fake-provider", "fake-model"),
        ExecutionEnvironment::default(),
        SessionConfig {
            max_turns: 1,
            ..SessionConfig::default()
        },
    );
    limited_turns.next_event();

    limited_turns
        .process_input(&client, "Question")
        .expect("turn-limited input");

    assert!(adapter.complete_requests().is_empty());
    assert_eq!(
        limited_turns.next_event().expect("USER_INPUT").kind,
        EventKind::UserInput
    );
    let turn_limit = limited_turns.next_event().expect("TURN_LIMIT");
    assert_eq!(turn_limit.kind, EventKind::TurnLimit);
    assert_eq!(turn_limit.data["round_count"], json!(0));
    assert_eq!(turn_limit.data["total_turns"], json!(1));
    assert_eq!(
        limited_turns.next_event().expect("PROCESSING_END").kind,
        EventKind::ProcessingEnd
    );
    assert_eq!(limited_turns.state, SessionState::Idle);

    let mut limited_tool_rounds = Session::new(
        ProviderProfile::new("fake-provider", "fake-model"),
        ExecutionEnvironment::default(),
        SessionConfig {
            max_tool_rounds_per_input: 1,
            ..SessionConfig::default()
        },
    );
    limited_tool_rounds.next_event();

    limited_tool_rounds
        .process_input(&client, "Needs a tool")
        .expect("tool-limited input");

    assert_eq!(adapter.complete_requests().len(), 1);
    let mut observed_turn_limit = None;
    while let Some(event) = limited_tool_rounds.next_event() {
        if event.kind == EventKind::TurnLimit {
            observed_turn_limit = Some(event);
            break;
        }
    }
    let turn_limit = observed_turn_limit.expect("TURN_LIMIT");
    assert_eq!(turn_limit.data["round_count"], json!(1));
    assert_eq!(turn_limit.data["total_turns"], json!(3));
    let processing_end = limited_tool_rounds.next_event().expect("PROCESSING_END");
    assert_eq!(processing_end.kind, EventKind::ProcessingEnd);
    assert_eq!(limited_tool_rounds.state, SessionState::Idle);
    assert_eq!(
        turn_names(&limited_tool_rounds.history),
        ["UserTurn", "AssistantTurn", "ToolResultsTurn"]
    );
    let HistoryTurn::ToolResults(tool_results) =
        limited_tool_rounds.history.last().expect("tool results")
    else {
        panic!("tool results recorded");
    };
    assert_eq!(tool_results.result_list[0].tool_call_id, "call-1");
    assert!(tool_results.result_list[0].is_error);
}

#[test]
fn abort_emits_structured_error_before_session_end_and_runs_cleanup_once() {
    let cleanup_calls = Arc::new(Mutex::new(0));
    let environment =
        ExecutionEnvironment::from_backend(CleanupCountingBackend::new(cleanup_calls.clone()));
    let mut session = Session::new(
        ProviderProfile::new("fake-provider", "fake-model"),
        environment,
        SessionConfig::default(),
    );
    session.next_event();

    session.abort();
    session.abort();
    session.close();

    assert_eq!(session.state, SessionState::Closed);
    assert_eq!(*cleanup_calls.lock().expect("cleanup calls"), 1);
    let events = drain_events(&mut session);
    assert_eq!(
        events
            .iter()
            .map(|event| event.kind.clone())
            .collect::<Vec<_>>(),
        vec![EventKind::Error, EventKind::SessionEnd]
    );
    assert_eq!(events[0].data["error"]["kind"], json!("abort"));
    assert_eq!(events[0].data["error"]["name"], json!("AbortError"));
    assert_eq!(
        events[0].data["error"]["message"],
        json!("session is aborted")
    );
    assert_eq!(events[1].data["reason"], json!("abort"));
    assert_eq!(events[1].data["error"], events[0].data["error"]);
    assert_eq!(
        events[1].data["final_state"]["error"],
        events[0].data["error"]
    );
    assert!(!events
        .iter()
        .any(|event| event.kind == EventKind::TurnLimit));
}

#[test]
fn process_input_provider_error_emits_structured_payload_closes_and_runs_cleanup() {
    let cleanup_calls = Arc::new(Mutex::new(0));
    let mut provider_error = AdapterError::provider(
        AdapterErrorKind::RateLimit,
        "rate limit",
        Some("fake-provider".to_string()),
    );
    provider_error.status_code = Some(429);
    provider_error.error_code = Some("rate_limit".to_string());
    provider_error.retry_after = Some(2.5);
    provider_error.raw = Some(json!({"error": {"message": "rate limit", "code": "rate_limit"}}));
    let adapter = Arc::new(ErrorAdapter::new(provider_error.clone()));
    let client_adapter: Arc<dyn ProviderAdapter> = adapter.clone();
    let client =
        Client::from_adapters(vec![client_adapter], Some("fake-provider")).expect("client");
    let environment =
        ExecutionEnvironment::from_backend(CleanupCountingBackend::new(cleanup_calls.clone()));
    let mut session = Session::new(
        ProviderProfile::new("fake-provider", "fake-model"),
        environment,
        SessionConfig::default(),
    );
    session.next_event();

    let error = session
        .process_input(&client, "Question")
        .expect_err("provider error");

    assert_eq!(error, provider_error);
    assert_eq!(session.state, SessionState::Closed);
    assert_eq!(*cleanup_calls.lock().expect("cleanup calls"), 1);
    assert_eq!(adapter.complete_requests().len(), 1);
    let events = drain_events(&mut session);
    assert_eq!(
        events
            .iter()
            .map(|event| event.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            EventKind::UserInput,
            EventKind::Error,
            EventKind::SessionEnd
        ]
    );
    assert_eq!(events[1].data["error"]["kind"], json!("rate_limit"));
    assert_eq!(events[1].data["error"]["name"], json!("RateLimitError"));
    assert_eq!(events[1].data["error"]["message"], json!("rate limit"));
    assert_eq!(events[1].data["error"]["provider"], json!("fake-provider"));
    assert_eq!(events[1].data["error"]["status_code"], json!(429));
    assert_eq!(events[1].data["error"]["error_code"], json!("rate_limit"));
    assert_eq!(events[1].data["error"]["retryable"], json!(true));
    assert_eq!(events[1].data["error"]["retry_after"], json!(2.5));
    assert_eq!(
        events[1].data["error"]["raw"],
        json!({"error": {"message": "rate limit", "code": "rate_limit"}})
    );
    assert_eq!(events[2].data["reason"], json!("unrecoverable_error"));
    assert_eq!(events[2].data["error"], events[1].data["error"]);
    assert_eq!(
        events[2].data["final_state"]["error"],
        events[1].data["error"]
    );
}

#[test]
fn process_input_abort_signaled_guard_closes_without_turn_limit_or_idle() {
    let cleanup_calls = Arc::new(Mutex::new(0));
    let adapter = Arc::new(ScriptedAdapter::with_complete_responses(vec![
        assistant_response("resp-unused", "unused", None),
    ]));
    let client_adapter: Arc<dyn ProviderAdapter> = adapter.clone();
    let client =
        Client::from_adapters(vec![client_adapter], Some("fake-provider")).expect("client");
    let environment =
        ExecutionEnvironment::from_backend(CleanupCountingBackend::new(cleanup_calls.clone()));
    let mut session = Session::new(
        ProviderProfile::new("fake-provider", "fake-model"),
        environment,
        SessionConfig::default(),
    );
    session.next_event();
    session.abort_signaled = true;

    session
        .process_input(&client, "Question")
        .expect("abort guard stops input");

    assert_eq!(session.state, SessionState::Closed);
    assert_eq!(*cleanup_calls.lock().expect("cleanup calls"), 1);
    assert!(adapter.complete_requests().is_empty());
    let events = drain_events(&mut session);
    assert_eq!(
        events
            .iter()
            .map(|event| event.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            EventKind::UserInput,
            EventKind::Error,
            EventKind::SessionEnd
        ]
    );
    assert_eq!(events[1].data["error"]["kind"], json!("abort"));
    assert_eq!(events[2].data["reason"], json!("abort"));
    assert_eq!(events[2].data["final_state"]["abort_signaled"], json!(true));
    assert!(!events
        .iter()
        .any(|event| event.kind == EventKind::TurnLimit));
    assert!(!events
        .iter()
        .any(|event| event.kind == EventKind::ProcessingEnd));
}

#[test]
fn tool_call_signatures_ignore_call_ids_and_hash_stable_arguments() {
    let first = ToolCall::new("call-1", "lookup", json!({"b": 2, "a": {"y": 2, "x": 1}}));
    let second = ToolCall::from_raw_arguments("call-2", "lookup", r#"{"a":{"x":1,"y":2},"b":2}"#);
    let different_arguments =
        ToolCall::new("call-3", "lookup", json!({"b": 2, "a": {"y": 3, "x": 1}}));
    let different_name = ToolCall::new(
        "call-4",
        "summarize",
        json!({"a": {"x": 1, "y": 2}, "b": 2}),
    );

    let first_signature = tool_call_signature(&first);
    let second_signature = tool_call_signature(&second);

    assert_eq!(first_signature, second_signature);
    assert_eq!(first_signature.name, "lookup");
    assert_eq!(
        first_signature.arguments_hash,
        second_signature.arguments_hash
    );
    assert_ne!(
        first_signature.arguments_hash,
        tool_call_signature(&different_arguments).arguments_hash
    );
    assert_ne!(first_signature, tool_call_signature(&different_name));
}

#[test]
fn tool_call_signatures_hash_nested_json_strings_as_strings() {
    let nested_json_string = ToolCall::new(
        "call-1",
        "lookup",
        json!({"payload": "{\"x\":1}", "stable": true}),
    );
    let raw_arguments_with_nested_json_string = ToolCall::from_raw_arguments(
        "call-2",
        "lookup",
        r#"{"stable":true,"payload":"{\"x\":1}"}"#,
    );
    let nested_object = ToolCall::new(
        "call-3",
        "lookup",
        json!({"payload": {"x": 1}, "stable": true}),
    );

    let nested_string_signature = tool_call_signature(&nested_json_string);
    assert_eq!(
        nested_string_signature,
        tool_call_signature(&raw_arguments_with_nested_json_string)
    );
    assert_ne!(
        nested_string_signature.arguments_hash,
        tool_call_signature(&nested_object).arguments_hash
    );
}

#[test]
fn detect_loop_matches_repeating_tool_call_patterns_of_length_one_two_and_three() {
    assert!(detect_loop(
        &history_from_tool_calls(vec![
            ToolCall::new("call-1", "lookup", json!({"value": 1})),
            ToolCall::new("call-2", "lookup", json!({"value": 1})),
        ]),
        2
    ));
    assert!(detect_loop(
        &history_from_tool_calls(vec![
            ToolCall::new("call-1", "lookup", json!({"value": 1})),
            ToolCall::new("call-2", "summarize", json!({"value": 2})),
            ToolCall::new("call-3", "lookup", json!({"value": 1})),
            ToolCall::new("call-4", "summarize", json!({"value": 2})),
        ]),
        4
    ));
    assert!(detect_loop(
        &history_from_tool_calls(vec![
            ToolCall::new("call-1", "lookup", json!({"value": 1})),
            ToolCall::new("call-2", "summarize", json!({"value": 2})),
            ToolCall::new("call-3", "expand", json!({"value": 3})),
            ToolCall::new("call-4", "lookup", json!({"value": 1})),
            ToolCall::new("call-5", "summarize", json!({"value": 2})),
            ToolCall::new("call-6", "expand", json!({"value": 3})),
        ]),
        6
    ));
    assert!(detect_loop(
        &history_from_tool_calls(
            (1..=10)
                .map(|index| {
                    if index % 2 == 1 {
                        ToolCall::new(format!("call-{index}"), "lookup", json!({"value": 1}))
                    } else {
                        ToolCall::new(format!("call-{index}"), "summarize", json!({"value": 2}))
                    }
                })
                .collect::<Vec<_>>()
        ),
        SessionConfig::default().loop_detection_window
    ));

    assert!(!detect_loop(
        &history_from_tool_calls(vec![
            ToolCall::new("call-1", "lookup", json!({"value": 1})),
            ToolCall::new("call-2", "lookup", json!({"value": 1})),
        ]),
        SessionConfig::default().loop_detection_window
    ));
    assert!(!detect_loop(
        &history_from_tool_calls(vec![
            ToolCall::new("call-1", "tool_1", json!({"value": 1})),
            ToolCall::new("call-2", "tool_2", json!({"value": 2})),
            ToolCall::new("call-3", "tool_3", json!({"value": 3})),
            ToolCall::new("call-4", "tool_4", json!({"value": 4})),
            ToolCall::new("call-5", "tool_5", json!({"value": 5})),
            ToolCall::new("call-6", "tool_6", json!({"value": 6})),
            ToolCall::new("call-7", "tool_7", json!({"value": 7})),
            ToolCall::new("call-8", "tool_8", json!({"value": 8})),
            ToolCall::new("call-9", "lookup", json!({"value": 1})),
            ToolCall::new("call-10", "lookup", json!({"value": 1})),
        ]),
        SessionConfig::default().loop_detection_window
    ));
    assert!(!detect_loop(
        &history_from_tool_calls(vec![
            ToolCall::new("call-1", "lookup", json!({"value": 1})),
            ToolCall::new("call-2", "summarize", json!({"value": 2})),
            ToolCall::new("call-3", "lookup", json!({"value": 1})),
            ToolCall::new("call-4", "summarize", json!({"value": 99})),
        ]),
        4
    ));
    assert!(!detect_loop(
        &history_from_tool_calls(vec![
            ToolCall::new("call-1", "lookup", json!({"value": 1})),
            ToolCall::new("call-2", "summarize", json!({"value": 2})),
            ToolCall::new("call-3", "expand", json!({"value": 3})),
            ToolCall::new("call-4", "lookup", json!({"value": 1})),
            ToolCall::new("call-5", "summarize", json!({"value": 2})),
            ToolCall::new("call-6", "expand", json!({"value": 3})),
            ToolCall::new("call-7", "lookup", json!({"value": 1})),
            ToolCall::new("call-8", "summarize", json!({"value": 2})),
            ToolCall::new("call-9", "expand", json!({"value": 3})),
            ToolCall::new("call-10", "lookup", json!({"value": 99})),
        ]),
        SessionConfig::default().loop_detection_window
    ));
    assert!(!detect_loop(
        &history_from_tool_calls(vec![
            ToolCall::new("call-1", "lookup", json!({"value": 1})),
            ToolCall::new("call-2", "lookup", json!({"value": 1})),
        ]),
        1
    ));
}

#[test]
fn process_input_emits_loop_detection_and_steers_recovery_when_enabled_by_default() {
    let adapter = Arc::new(ScriptedAdapter::with_complete_responses(vec![
        tool_call_response_with_call(
            "resp-1",
            "Need tool",
            ToolCall::new("call-1", "lookup", json!({"value": 1})),
        ),
        tool_call_response_with_call(
            "resp-2",
            "Need tool again",
            ToolCall::new("call-2", "lookup", json!({"value": 1})),
        ),
        assistant_response("resp-3", "All done", None),
    ]));
    let client_adapter: Arc<dyn ProviderAdapter> = adapter.clone();
    let client =
        Client::from_adapters(vec![client_adapter], Some("fake-provider")).expect("client");
    let mut session = Session::new(
        profile_with_lookup_tool(),
        ExecutionEnvironment::default(),
        SessionConfig {
            loop_detection_window: 2,
            ..SessionConfig::default()
        },
    );
    session.next_event();

    session
        .process_input(&client, "Question")
        .expect("process input");

    let calls = adapter.complete_requests();
    assert_eq!(calls.len(), 3);
    assert_eq!(
        calls[2].messages.last().expect("recovery steering"),
        &Message::user(LOOP_DETECTION_WARNING)
    );
    assert_eq!(session.state, SessionState::Idle);
    assert_eq!(
        turn_names(&session.history),
        [
            "UserTurn",
            "AssistantTurn",
            "ToolResultsTurn",
            "AssistantTurn",
            "ToolResultsTurn",
            "SteeringTurn",
            "AssistantTurn",
        ]
    );
    let HistoryTurn::Steering(steering) = &session.history[5] else {
        panic!("loop detection steering recorded");
    };
    assert_eq!(steering.text(), LOOP_DETECTION_WARNING);
    let HistoryTurn::Assistant(assistant) = session.history.last().expect("final assistant") else {
        panic!("final assistant recorded");
    };
    assert_eq!(assistant.text(), "All done");

    let events = drain_events(&mut session);
    let kinds = events
        .iter()
        .map(|event| event.kind.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            EventKind::UserInput,
            EventKind::AssistantTextStart,
            EventKind::AssistantTextDelta,
            EventKind::AssistantTextEnd,
            EventKind::ModelToolCallEnd,
            EventKind::ToolCallStart,
            EventKind::ToolCallEnd,
            EventKind::AssistantTextStart,
            EventKind::AssistantTextDelta,
            EventKind::AssistantTextEnd,
            EventKind::ModelToolCallEnd,
            EventKind::ToolCallStart,
            EventKind::ToolCallEnd,
            EventKind::LoopDetection,
            EventKind::AssistantTextStart,
            EventKind::AssistantTextDelta,
            EventKind::AssistantTextEnd,
            EventKind::ProcessingEnd,
        ]
    );
    assert_eq!(events[13].data["message"], json!(LOOP_DETECTION_WARNING));
}

#[test]
fn process_input_waits_for_configured_loop_detection_window_before_steering_by_default() {
    let adapter = Arc::new(ScriptedAdapter::with_complete_responses(vec![
        tool_call_response_with_call(
            "resp-1",
            "Need tool",
            ToolCall::new("call-1", "lookup", json!({"value": 1})),
        ),
        tool_call_response_with_call(
            "resp-2",
            "Need tool again",
            ToolCall::new("call-2", "lookup", json!({"value": 1})),
        ),
        assistant_response("resp-3", "All done", None),
    ]));
    let client_adapter: Arc<dyn ProviderAdapter> = adapter.clone();
    let client =
        Client::from_adapters(vec![client_adapter], Some("fake-provider")).expect("client");
    let mut session = Session::new(
        profile_with_lookup_tool(),
        ExecutionEnvironment::default(),
        SessionConfig::default(),
    );
    session.next_event();

    session
        .process_input(&client, "Question")
        .expect("process input");

    assert_eq!(adapter.complete_requests().len(), 3);
    assert_eq!(session.state, SessionState::Idle);
    assert_eq!(
        turn_names(&session.history),
        [
            "UserTurn",
            "AssistantTurn",
            "ToolResultsTurn",
            "AssistantTurn",
            "ToolResultsTurn",
            "AssistantTurn",
        ]
    );
    assert!(!session
        .history
        .iter()
        .any(|turn| matches!(turn, HistoryTurn::Steering(_))));
    let HistoryTurn::Assistant(assistant) = session.history.last().expect("final assistant") else {
        panic!("final assistant recorded");
    };
    assert_eq!(assistant.text(), "All done");

    let events = drain_events(&mut session);
    assert!(!events
        .iter()
        .any(|event| event.kind == EventKind::LoopDetection));
}

#[test]
fn process_input_skips_loop_detection_when_disabled() {
    let adapter = Arc::new(ScriptedAdapter::with_complete_responses(vec![
        tool_call_response_with_call(
            "resp-1",
            "Need tool",
            ToolCall::new("call-1", "lookup", json!({"value": 1})),
        ),
        tool_call_response_with_call(
            "resp-2",
            "Need tool again",
            ToolCall::new("call-2", "lookup", json!({"value": 1})),
        ),
        assistant_response("resp-3", "All done", None),
    ]));
    let client_adapter: Arc<dyn ProviderAdapter> = adapter.clone();
    let client =
        Client::from_adapters(vec![client_adapter], Some("fake-provider")).expect("client");
    let mut session = Session::new(
        profile_with_lookup_tool(),
        ExecutionEnvironment::default(),
        SessionConfig {
            enable_loop_detection: false,
            loop_detection_window: 2,
            ..SessionConfig::default()
        },
    );
    session.next_event();

    session
        .process_input(&client, "Question")
        .expect("process input");

    assert_eq!(adapter.complete_requests().len(), 3);
    assert_eq!(session.state, SessionState::Idle);
    assert_eq!(
        turn_names(&session.history),
        [
            "UserTurn",
            "AssistantTurn",
            "ToolResultsTurn",
            "AssistantTurn",
            "ToolResultsTurn",
            "AssistantTurn",
        ]
    );
    assert!(!session
        .history
        .iter()
        .any(|turn| matches!(turn, HistoryTurn::Steering(_))));

    let events = drain_events(&mut session);
    assert!(!events
        .iter()
        .any(|event| event.kind == EventKind::LoopDetection));
}

#[test]
fn process_input_reads_reasoning_effort_when_each_request_is_built() {
    let adapter = Arc::new(ScriptedAdapter::with_complete_responses(vec![
        assistant_response("resp-1", "First", None),
        assistant_response("resp-2", "Second", None),
        assistant_response("resp-3", "Third", None),
        assistant_response("resp-4", "Fourth", None),
        assistant_response("resp-5", "Fifth", None),
    ]));
    let client_adapter: Arc<dyn ProviderAdapter> = adapter.clone();
    let client =
        Client::from_adapters(vec![client_adapter], Some("fake-provider")).expect("client");
    let mut profile = ProviderProfile::new("openai", "fake-model");
    profile.request_provider = Some("fake-provider".to_string());
    profile
        .provider_options
        .insert("temperature".to_string(), json!(0.2));
    profile
        .provider_options
        .insert("reasoning".to_string(), json!({"summary": "auto"}));
    let mut session = Session::new(
        profile,
        ExecutionEnvironment::default(),
        SessionConfig {
            reasoning_effort: Some("low".to_string()),
            ..SessionConfig::default()
        },
    );
    session.next_event();

    session
        .process_input(&client, "First input")
        .expect("first input");
    session.config.reasoning_effort = Some("high".to_string());
    session
        .process_input(&client, "Second input")
        .expect("second input");
    session.config.reasoning_effort = Some("medium".to_string());
    session
        .process_input(&client, "Third input")
        .expect("third input");
    session.config.reasoning_effort = None;
    session
        .process_input(&client, "Fourth input")
        .expect("fourth input");
    session.config.reasoning_effort = Some("provider-extension".to_string());
    session
        .process_input(&client, "Fifth input")
        .expect("fifth input");

    let calls = adapter.complete_requests();
    assert_eq!(calls.len(), 5);
    assert_eq!(
        calls
            .iter()
            .map(|request| request.reasoning_effort.as_deref())
            .collect::<Vec<_>>(),
        [
            Some("low"),
            Some("high"),
            Some("medium"),
            None,
            Some("provider-extension"),
        ]
    );
    assert_eq!(
        calls
            .iter()
            .map(|request| request.provider.as_deref())
            .collect::<Vec<_>>(),
        [Some("fake-provider"); 5]
    );
    assert_eq!(
        calls[0].provider_options["openai"]["reasoning"],
        json!({"summary": "auto", "effort": "low"})
    );
    assert_eq!(
        calls[1].provider_options["openai"]["reasoning"],
        json!({"summary": "auto", "effort": "high"})
    );
    assert_eq!(
        calls[2].provider_options["openai"]["reasoning"],
        json!({"summary": "auto", "effort": "medium"})
    );
    assert_eq!(
        calls[3].provider_options["openai"]["reasoning"],
        json!({"summary": "auto"})
    );
    assert_eq!(
        calls[4].provider_options["openai"]["reasoning"],
        json!({"summary": "auto", "effort": "provider-extension"})
    );
    assert_eq!(calls[1].messages[0], calls[0].messages[0]);
    assert!(calls[1].messages[0].text().contains("<environment>"));
    assert_eq!(calls[1].messages[1], Message::user("First input"));
    assert_eq!(calls[1].messages[2], Message::assistant("First"));
    assert_eq!(calls[1].messages[3], Message::user("Second input"));
    assert_eq!(calls[4].messages[7], Message::user("Fourth input"));
    assert_eq!(calls[4].messages[8], Message::assistant("Fourth"));
    assert_eq!(calls[4].messages[9], Message::user("Fifth input"));
}

fn assistant_response(id: &str, text: &str, reasoning: Option<&str>) -> Response {
    let mut content = vec![ContentPart::text(text)];
    if let Some(reasoning) = reasoning {
        content.push(ContentPart::Thinking {
            thinking: unified_llm_adapter::ThinkingData {
                text: reasoning.to_string(),
                signature: None,
                redacted: false,
                source_provider: None,
                source_model: None,
            },
        });
    }
    Response {
        id: id.to_string(),
        model: "fake-model".to_string(),
        provider: "fake-provider".to_string(),
        message: assistant_message(content),
        finish_reason: FinishReason::Stop,
        usage: Usage {
            input_tokens: 3,
            output_tokens: 5,
            total_tokens: 8,
            ..Usage::default()
        },
        ..Response::default()
    }
}

fn tool_call_response_with_call(id: &str, text: &str, tool_call: ToolCall) -> Response {
    Response {
        id: id.to_string(),
        model: "fake-model".to_string(),
        provider: "fake-provider".to_string(),
        message: assistant_message(vec![
            ContentPart::text(text),
            ContentPart::ToolCall { tool_call },
        ]),
        finish_reason: FinishReason::ToolCalls,
        ..Response::default()
    }
}

fn tool_call_response(id: &str) -> Response {
    tool_call_response_with_call(
        id,
        "Need a tool",
        ToolCall::new("call-1", "lookup", json!({"query": "rust"})),
    )
}

fn profile_with_lookup_tool() -> ProviderProfile {
    let mut profile = ProviderProfile::new("fake-provider", "fake-model");
    let mut registry = ToolRegistry::new();
    registry.register(RegisteredTool::new_with_executor(
        ToolDefinition::new(
            "lookup",
            "Lookup values",
            Some(json!({
                "type": "object",
                "properties": {"value": {"type": "integer"}},
                "required": ["value"],
                "additionalProperties": false
            })),
        )
        .expect("tool definition"),
        Arc::new(|_| {
            Ok(ToolExecutionOutput::success(Value::String(
                "tool result".to_string(),
            )))
        }),
    ));
    profile.set_tool_registry(registry);
    profile
}

fn history_from_tool_calls(tool_calls: impl IntoIterator<Item = ToolCall>) -> Vec<HistoryTurn> {
    let mut history = Vec::new();
    for tool_call in tool_calls {
        let mut assistant_turn = AssistantTurn::new("tool");
        assistant_turn.tool_calls = vec![tool_call.clone()];
        history.push(HistoryTurn::Assistant(assistant_turn));
        history.push(HistoryTurn::ToolResults(ToolResultsTurn::new(vec![
            ToolResult::success(tool_call.id, Value::String("tool result".to_string())),
        ])));
    }
    history
}

fn assistant_message(content: Vec<ContentPart>) -> Message {
    Message {
        role: MessageRole::Assistant,
        content,
        name: None,
        tool_call_id: None,
        provider_metadata: BTreeMap::new(),
    }
}

fn drain_events(session: &mut Session) -> Vec<SessionEvent> {
    let mut events = Vec::new();
    while let Some(event) = session.next_event() {
        events.push(event);
    }
    events
}

fn turn_names(history: &[HistoryTurn]) -> Vec<&'static str> {
    history
        .iter()
        .map(|turn| match turn {
            HistoryTurn::User(_) => "UserTurn",
            HistoryTurn::Steering(_) => "SteeringTurn",
            HistoryTurn::Assistant(_) => "AssistantTurn",
            HistoryTurn::System(_) => "SystemTurn",
            HistoryTurn::ToolResults(_) => "ToolResultsTurn",
        })
        .collect()
}

fn assert_json_f64(value: &Value, expected: f64) {
    let actual = value.as_f64().expect("JSON number");
    assert!(
        (actual - expected).abs() < f64::EPSILON,
        "expected {expected}, got {actual}"
    );
}

#[derive(Default)]
struct ScriptedAdapter {
    complete_requests: Mutex<Vec<Request>>,
    stream_requests: Mutex<Vec<Request>>,
    complete_responses: Mutex<VecDeque<Response>>,
    stream_responses: Mutex<VecDeque<Vec<StreamEvent>>>,
}

impl ScriptedAdapter {
    fn with_complete_responses(responses: Vec<Response>) -> Self {
        Self {
            complete_responses: Mutex::new(responses.into_iter().collect()),
            ..Self::default()
        }
    }

    fn with_stream_events(responses: Vec<Vec<StreamEvent>>) -> Self {
        Self {
            stream_responses: Mutex::new(responses.into_iter().collect()),
            ..Self::default()
        }
    }

    fn complete_requests(&self) -> Vec<Request> {
        self.complete_requests
            .lock()
            .expect("complete requests")
            .clone()
    }

    fn stream_requests(&self) -> Vec<Request> {
        self.stream_requests
            .lock()
            .expect("stream requests")
            .clone()
    }
}

impl ProviderAdapter for ScriptedAdapter {
    fn name(&self) -> &str {
        "fake-provider"
    }

    fn complete(&self, request: Request) -> Result<Response, AdapterError> {
        self.complete_requests
            .lock()
            .expect("complete requests")
            .push(request.clone());
        self.complete_responses
            .lock()
            .expect("complete responses")
            .pop_front()
            .ok_or_else(|| AdapterError::new(AdapterErrorKind::Configuration, "missing response"))
    }

    fn stream(&self, request: Request) -> Result<StreamEvents, AdapterError> {
        self.stream_requests
            .lock()
            .expect("stream requests")
            .push(request);
        let events = self
            .stream_responses
            .lock()
            .expect("stream responses")
            .pop_front()
            .ok_or_else(|| AdapterError::new(AdapterErrorKind::Configuration, "missing stream"))?;
        Ok(stream_events(events.into_iter().map(Ok)))
    }
}

#[derive(Debug)]
struct ErrorAdapter {
    error: AdapterError,
    complete_requests: Mutex<Vec<Request>>,
    stream_requests: Mutex<Vec<Request>>,
}

impl ErrorAdapter {
    fn new(error: AdapterError) -> Self {
        Self {
            error,
            complete_requests: Mutex::new(Vec::new()),
            stream_requests: Mutex::new(Vec::new()),
        }
    }

    fn complete_requests(&self) -> Vec<Request> {
        self.complete_requests
            .lock()
            .expect("complete requests")
            .clone()
    }
}

impl ProviderAdapter for ErrorAdapter {
    fn name(&self) -> &str {
        "fake-provider"
    }

    fn complete(&self, request: Request) -> Result<Response, AdapterError> {
        self.complete_requests
            .lock()
            .expect("complete requests")
            .push(request);
        Err(self.error.clone())
    }

    fn stream(&self, request: Request) -> Result<StreamEvents, AdapterError> {
        self.stream_requests
            .lock()
            .expect("stream requests")
            .push(request);
        Err(self.error.clone())
    }
}

#[derive(Debug)]
struct CleanupCountingBackend {
    cleanup_calls: Arc<Mutex<usize>>,
}

impl CleanupCountingBackend {
    fn new(cleanup_calls: Arc<Mutex<usize>>) -> Self {
        Self { cleanup_calls }
    }
}

impl ExecutionEnvironmentBackend for CleanupCountingBackend {
    fn read_file(
        &self,
        path: &Path,
        _offset: Option<usize>,
        _limit: Option<usize>,
    ) -> EnvironmentResult<String> {
        Err(EnvironmentError::FileNotFound(path.to_path_buf()))
    }

    fn read_file_bytes(&self, path: &Path) -> EnvironmentResult<Vec<u8>> {
        Err(EnvironmentError::FileNotFound(path.to_path_buf()))
    }

    fn write_file(&self, _path: &Path, _content: &str) -> EnvironmentResult<()> {
        Ok(())
    }

    fn file_exists(&self, _path: &Path) -> bool {
        false
    }

    fn is_directory(&self, _path: &Path) -> bool {
        false
    }

    fn delete_file(&self, _path: &Path) -> EnvironmentResult<()> {
        Ok(())
    }

    fn rename_file(&self, _source_path: &Path, _destination_path: &Path) -> EnvironmentResult<()> {
        Ok(())
    }

    fn list_directory(&self, _path: &Path, _depth: usize) -> EnvironmentResult<Vec<DirEntry>> {
        Ok(Vec::new())
    }

    fn exec_command(
        &self,
        _command: &str,
        _options: CommandOptions,
    ) -> EnvironmentResult<ExecResult> {
        Ok(ExecResult::default())
    }

    fn grep(
        &self,
        _pattern: &str,
        _path: &Path,
        _options: &GrepOptions,
    ) -> EnvironmentResult<String> {
        Ok(String::new())
    }

    fn glob(&self, _pattern: &str, _path: &Path) -> EnvironmentResult<Vec<String>> {
        Ok(Vec::new())
    }

    fn initialize(&self) -> EnvironmentResult<()> {
        Ok(())
    }

    fn cleanup(&self) -> EnvironmentResult<()> {
        *self.cleanup_calls.lock().expect("cleanup calls") += 1;
        Ok(())
    }

    fn working_directory(&self) -> String {
        "workspace".to_string()
    }

    fn platform(&self) -> String {
        "test".to_string()
    }

    fn os_version(&self) -> String {
        "test".to_string()
    }
}
