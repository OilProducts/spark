use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use spark_agent_adapter::{
    AgentError, AgentRawLogLine, AgentThreadResumeFailure, AgentTurnBackend, AgentTurnOutput,
    AgentTurnRequest,
};
use spark_common::events::{
    TurnStreamChannel, TurnStreamEvent, TurnStreamEventKind, TurnStreamSource,
};
use spark_common::settings::SparkSettings;
use spark_storage::{ProjectRegistry, CONVERSATION_STATE_SCHEMA_VERSION};
use spark_workspace::{
    live::conversation_envelopes_after, ConversationRequestUserInputAnswerRequest,
    ConversationTurnRequest, WorkspaceConversationService, WorkspaceError,
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
fn start_turn_prepares_agent_request_with_persisted_history_and_selectors() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = WorkspaceConversationService::new(settings(temp.path()));

    let (first_prepared, _snapshot) = service
        .start_turn(
            "conversation-agent-request",
            ConversationTurnRequest {
                project_path: "/projects/agent-request".to_string(),
                message: "First question".to_string(),
                provider: Some("codex".to_string()),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("first turn");
    service
        .ingest_agent_turn_output(
            "conversation-agent-request",
            "/projects/agent-request",
            &first_prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                final_assistant_text: Some("First answer".to_string()),
                ..AgentTurnOutput::default()
            },
        )
        .expect("first output");

    let (prepared, snapshot) = service
        .start_turn(
            "conversation-agent-request",
            ConversationTurnRequest {
                project_path: "/projects/agent-request".to_string(),
                message: "Second question".to_string(),
                provider: Some("openrouter".to_string()),
                model: Some("openrouter/model".to_string()),
                llm_profile: Some("implementation".to_string()),
                reasoning_effort: Some("HIGH".to_string()),
                chat_mode: Some("plan".to_string()),
            },
        )
        .expect("second turn");

    let request = &prepared.agent_turn_request;
    assert_eq!(request.conversation_id, prepared.conversation_id);
    assert_eq!(request.project_path, prepared.project_path);
    assert_eq!(request.prompt, "Second question");
    assert_eq!(request.provider.as_deref(), Some("openrouter"));
    assert_eq!(request.model.as_deref(), Some("openrouter/model"));
    assert_eq!(request.llm_profile.as_deref(), Some("implementation"));
    assert_eq!(request.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(request.chat_mode.as_deref(), Some("plan"));
    assert_eq!(
        request.metadata["spark.workspace.user_turn_id"],
        json!(prepared.user_turn_id.clone())
    );
    assert_eq!(
        request.metadata["spark.workspace.assistant_turn_id"],
        json!(prepared.assistant_turn_id.clone())
    );

    let history = serde_json::to_value(&request.history).expect("history json");
    assert_eq!(history.as_array().expect("history").len(), 2);
    assert_eq!(history[0]["role"], "user");
    assert_eq!(history[0]["content"], "First question");
    assert_eq!(history[1]["role"], "assistant");
    assert_eq!(history[1]["content"], "First answer");
    assert!(history[0].get("id").is_none());
    assert!(history[0].get("status").is_none());
    assert!(history[0].get("kind").is_none());
    assert!(history[1].get("segments").is_none());

    let turns = snapshot["turns"].as_array().expect("turns");
    assert!(turns
        .iter()
        .any(|turn| turn["id"] == prepared.user_turn_id && turn["content"] == "Second question"));
    assert!(request
        .history
        .iter()
        .all(
            |turn| serde_json::to_value(turn).expect("turn json")["content"] != "Second question"
        ));

    let decoded: AgentTurnRequest =
        serde_json::from_value(serde_json::to_value(request).expect("request json"))
            .expect("deserialize request");
    assert_eq!(&decoded, request);
}

#[test]
fn start_turn_agent_history_uses_legacy_complete_turn_defaults() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = "/projects/legacy-agent-request";
    let project = ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths(project_path)
        .expect("project");
    write_state(
        &project.conversations_dir,
        "conversation-legacy-agent-request",
        json!({
            "schema_version": CONVERSATION_STATE_SCHEMA_VERSION,
            "revision": 2,
            "conversation_id": "conversation-legacy-agent-request",
            "project_path": project_path,
            "chat_mode": "chat",
            "provider": "codex",
            "model": null,
            "llm_profile": null,
            "reasoning_effort": null,
            "title": "Legacy conversation",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:02Z",
            "turns": [
                {
                    "id": "turn-legacy-user",
                    "role": "user",
                    "content": "Legacy question",
                    "timestamp": "2026-01-01T00:00:00Z"
                },
                {
                    "id": "turn-legacy-assistant",
                    "role": "assistant",
                    "content": "Legacy answer",
                    "timestamp": "2026-01-01T00:00:01Z"
                }
            ],
            "segments": []
        }),
    );

    let service = WorkspaceConversationService::new(settings);
    let (prepared, _snapshot) = service
        .start_turn(
            "conversation-legacy-agent-request",
            ConversationTurnRequest {
                project_path: project_path.to_string(),
                message: "Current question".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");

    let history = serde_json::to_value(&prepared.agent_turn_request.history).expect("history");
    assert_eq!(history.as_array().expect("history").len(), 2);
    assert_eq!(history[0]["role"], "user");
    assert_eq!(history[0]["content"], "Legacy question");
    assert_eq!(history[0]["timestamp"], "2026-01-01T00:00:00Z");
    assert_eq!(history[1]["role"], "assistant");
    assert_eq!(history[1]["content"], "Legacy answer");
    assert_eq!(history[1]["timestamp"], "2026-01-01T00:00:01Z");
}

#[test]
fn execute_turn_runs_injected_backend_and_returns_ingested_snapshot() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let backend = ScriptedAgentTurnBackend::new(vec![
        AgentTurnOutput {
            final_assistant_text: Some("First answer".to_string()),
            ..AgentTurnOutput::default()
        },
        AgentTurnOutput {
            raw_log_lines: vec![AgentRawLogLine {
                direction: "outgoing".to_string(),
                line: "{\"event\":\"backend-second\"}".to_string(),
            }],
            final_assistant_text: Some("Second answer".to_string()),
            token_usage: Some(json!({"total": {"inputTokens": 12, "outputTokens": 5}})),
            ..AgentTurnOutput::default()
        },
    ]);
    let requests = backend.requests();
    let service = WorkspaceConversationService::new_with_agent_turn_backend(
        settings.clone(),
        Arc::new(backend),
    );

    service
        .execute_turn(
            "conversation-execute",
            ConversationTurnRequest {
                project_path: "/projects/execute".to_string(),
                message: "First question".to_string(),
                provider: Some("codex".to_string()),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("first execution");
    let snapshot = service
        .execute_turn(
            "conversation-execute",
            ConversationTurnRequest {
                project_path: "/projects/execute".to_string(),
                message: "Second question".to_string(),
                provider: Some("openrouter".to_string()),
                model: Some("openrouter/model".to_string()),
                llm_profile: Some("implementation".to_string()),
                reasoning_effort: Some("HIGH".to_string()),
                chat_mode: Some("plan".to_string()),
            },
        )
        .expect("second execution");

    let requests = requests.lock().expect("requests");
    assert_eq!(requests.len(), 2);
    let request = &requests[1];
    assert_eq!(request.conversation_id, "conversation-execute");
    assert_eq!(request.project_path, "/projects/execute");
    assert_eq!(request.prompt, "Second question");
    assert_eq!(request.provider.as_deref(), Some("openrouter"));
    assert_eq!(request.model.as_deref(), Some("openrouter/model"));
    assert_eq!(request.llm_profile.as_deref(), Some("implementation"));
    assert_eq!(request.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(request.chat_mode.as_deref(), Some("plan"));
    let history = serde_json::to_value(&request.history).expect("history");
    assert_eq!(history.as_array().expect("history").len(), 2);
    assert_eq!(history[0]["role"], "user");
    assert_eq!(history[0]["content"], "First question");
    assert_eq!(history[1]["role"], "assistant");
    assert_eq!(history[1]["content"], "First answer");
    assert!(history[0].get("id").is_none());
    assert!(history[1].get("status").is_none());
    let assistant_turn_id = request.metadata["spark.workspace.assistant_turn_id"]
        .as_str()
        .expect("assistant turn id")
        .to_string();
    let user_turn_id = request.metadata["spark.workspace.user_turn_id"]
        .as_str()
        .expect("user turn id")
        .to_string();

    let turns = snapshot["turns"].as_array().expect("turns");
    assert!(turns
        .iter()
        .any(|turn| turn["id"] == user_turn_id && turn["content"] == "Second question"));
    let assistant_turn = turns
        .iter()
        .find(|turn| turn["id"] == assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "complete");
    assert_eq!(assistant_turn["content"], "Second answer");
    assert_eq!(
        assistant_turn["token_usage"],
        json!({"total": {"inputTokens": 12, "outputTokens": 5}})
    );

    let persisted = service
        .get_snapshot("conversation-execute", Some("/projects/execute"))
        .expect("persisted snapshot");
    assert_eq!(snapshot["revision"], persisted["revision"]);
    assert_eq!(snapshot["turns"], persisted["turns"]);

    let raw_log = spark_storage::ConversationRepository::new(&settings.data_dir)
        .read_raw_rpc_log("conversation-execute", "/projects/execute")
        .expect("raw log");
    assert_eq!(raw_log.last().expect("raw line").direction, "outgoing");
    assert_eq!(
        raw_log.last().expect("raw line").line,
        "{\"event\":\"backend-second\"}"
    );

    let events = service
        .read_events_after("conversation-execute", "/projects/execute", 0)
        .expect("events");
    assert!(events.iter().any(|event| {
        event["type"] == "turn_upsert"
            && event["turn"]["id"] == assistant_turn_id
            && event["turn"]["status"] == "complete"
    }));
    assert!(events
        .iter()
        .any(|event| event["type"] == "conversation_snapshot"));

    let live_envelopes =
        conversation_envelopes_after(&settings, "conversation-execute", "/projects/execute", 0)
            .expect("live envelopes");
    assert!(live_envelopes.iter().any(|envelope| {
        envelope.event_type == "conversation.turn_upsert"
            && envelope.payload["turn"]["id"] == assistant_turn_id
            && envelope.payload["turn"]["status"] == "complete"
            && envelope.payload["turn"]["content"] == "Second answer"
            && envelope.payload["turn"]["token_usage"]["total"]["inputTokens"] == json!(12)
    }));
    assert!(live_envelopes.iter().any(|envelope| {
        envelope.event_type == "conversation.snapshot"
            && envelope.payload["state"]["turns"]
                .as_array()
                .is_some_and(|turns| {
                    turns.iter().any(|turn| {
                        turn["id"] == assistant_turn_id
                            && turn["status"] == "complete"
                            && turn["token_usage"]["total"]["outputTokens"] == json!(5)
                    })
                })
    }));
}

#[test]
fn execute_turn_persists_backend_error_outputs_with_failed_shapes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let backend = ScriptedAgentTurnBackend::new(vec![
        AgentTurnOutput {
            events: vec![stream_error_event("backend stream failed")],
            final_assistant_text: Some("Do not persist this final text".to_string()),
            token_usage: Some(json!({"total": {"inputTokens": 7, "outputTokens": 1}})),
            ..AgentTurnOutput::default()
        },
        AgentTurnOutput {
            thread_resume_failure: Some(AgentThreadResumeFailure {
                message: "thread resume failed".to_string(),
                error_code: Some("thread_resume_failed".to_string()),
                details: Some(json!({"thread_id": "thread-public-execute"})),
            }),
            token_usage: Some(json!({"total": {"inputTokens": 2, "outputTokens": 0}})),
            ..AgentTurnOutput::default()
        },
    ]);
    let requests = backend.requests();
    let service = WorkspaceConversationService::new_with_agent_turn_backend(
        settings.clone(),
        Arc::new(backend),
    );

    let stream_failed = service
        .execute_turn(
            "conversation-execute-stream-failure",
            ConversationTurnRequest {
                project_path: "/projects/execute-errors".to_string(),
                message: "Run the failing stream".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("stream failure output");

    let stream_request = requests.lock().expect("requests")[0].clone();
    assert_eq!(
        stream_request.conversation_id,
        "conversation-execute-stream-failure"
    );
    assert_eq!(stream_request.project_path, "/projects/execute-errors");
    assert_eq!(stream_request.prompt, "Run the failing stream");
    let stream_assistant_turn_id = stream_request.metadata["spark.workspace.assistant_turn_id"]
        .as_str()
        .expect("assistant turn id")
        .to_string();

    let stream_assistant_turn = stream_failed["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == stream_assistant_turn_id)
        .expect("stream failed assistant turn");
    assert_eq!(stream_assistant_turn["status"], "failed");
    assert_eq!(stream_assistant_turn["error"], "backend stream failed");
    assert_ne!(
        stream_assistant_turn["content"],
        "Do not persist this final text"
    );
    assert_eq!(
        stream_assistant_turn["token_usage"],
        json!({"total": {"inputTokens": 7, "outputTokens": 1}})
    );
    let stream_segment = segment_by_kind(
        stream_failed["segments"].as_array().expect("segments"),
        "assistant_message",
    );
    assert_eq!(stream_segment["status"], "failed");
    assert_eq!(stream_segment["error"], "backend stream failed");

    let stream_events = service
        .read_events_after(
            "conversation-execute-stream-failure",
            "/projects/execute-errors",
            0,
        )
        .expect("stream events");
    assert!(stream_events.iter().any(|event| {
        event["type"] == "turn_upsert"
            && event["turn"]["id"] == stream_assistant_turn_id
            && event["turn"]["status"] == "failed"
            && event["turn"]["token_usage"]["total"]["inputTokens"] == json!(7)
    }));
    assert!(stream_events.iter().any(|event| {
        event["type"] == "segment_upsert"
            && event["segment"]["status"] == "failed"
            && event["segment"]["error"] == "backend stream failed"
    }));
    assert!(stream_events
        .iter()
        .any(|event| event["type"] == "conversation_snapshot"));

    let stream_live = conversation_envelopes_after(
        &settings,
        "conversation-execute-stream-failure",
        "/projects/execute-errors",
        0,
    )
    .expect("stream live envelopes");
    assert!(stream_live.iter().any(|envelope| {
        envelope.event_type == "conversation.turn_upsert"
            && envelope.payload["turn"]["id"] == stream_assistant_turn_id
            && envelope.payload["turn"]["status"] == "failed"
            && envelope.payload["turn"]["error"] == "backend stream failed"
    }));

    let resume_failed = service
        .execute_turn(
            "conversation-execute-resume-failure",
            ConversationTurnRequest {
                project_path: "/projects/execute-errors".to_string(),
                message: "Resume the previous thread".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("thread resume failure output");

    let resume_request = requests.lock().expect("requests")[1].clone();
    assert_eq!(
        resume_request.conversation_id,
        "conversation-execute-resume-failure"
    );
    assert_eq!(resume_request.prompt, "Resume the previous thread");
    let resume_assistant_turn_id = resume_request.metadata["spark.workspace.assistant_turn_id"]
        .as_str()
        .expect("assistant turn id")
        .to_string();

    let resume_assistant_turn = resume_failed["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == resume_assistant_turn_id)
        .expect("resume failed assistant turn");
    assert_eq!(resume_assistant_turn["status"], "failed");
    assert_eq!(resume_assistant_turn["error"], "thread resume failed");
    assert_eq!(resume_assistant_turn["error_code"], "thread_resume_failed");
    assert_eq!(
        resume_assistant_turn["token_usage"],
        json!({"total": {"inputTokens": 2, "outputTokens": 0}})
    );
    assert_eq!(resume_failed["event_log"][0]["kind"], "continuity_reset");
    assert_eq!(
        resume_failed["event_log"][0]["details"]["thread_id"],
        "thread-public-execute"
    );

    let resume_events = service
        .read_events_after(
            "conversation-execute-resume-failure",
            "/projects/execute-errors",
            0,
        )
        .expect("resume events");
    assert!(resume_events.iter().any(|event| {
        event["type"] == "turn_upsert"
            && event["turn"]["id"] == resume_assistant_turn_id
            && event["turn"]["status"] == "failed"
            && event["turn"]["error_code"] == "thread_resume_failed"
    }));
    let resume_live = conversation_envelopes_after(
        &settings,
        "conversation-execute-resume-failure",
        "/projects/execute-errors",
        0,
    )
    .expect("resume live envelopes");
    assert!(resume_live.iter().any(|envelope| {
        envelope.event_type == "conversation.turn_upsert"
            && envelope.payload["turn"]["id"] == resume_assistant_turn_id
            && envelope.payload["turn"]["status"] == "failed"
            && envelope.payload["turn"]["error_code"] == "thread_resume_failed"
    }));
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
                token_usage: Some(json!({"total": {"inputTokens": 3, "outputTokens": 0}})),
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
    assert_eq!(
        assistant_turn["token_usage"],
        json!({"total": {"inputTokens": 3, "outputTokens": 0}})
    );
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
fn stream_error_with_final_text_and_usage_stays_failed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let service = WorkspaceConversationService::new(settings(temp.path()));
    let (prepared, _snapshot) = service
        .start_turn(
            "conversation-stream-error-final-text",
            ConversationTurnRequest {
                project_path: "/projects/stream-error-final-text".to_string(),
                message: "Run the agent".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");

    let snapshot = service
        .ingest_agent_turn_output(
            "conversation-stream-error-final-text",
            "/projects/stream-error-final-text",
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![stream_error_event("backend failed")],
                final_assistant_text: Some("Do not persist as success".to_string()),
                token_usage: Some(json!({"total": {"inputTokens": 9, "outputTokens": 2}})),
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
    assert_eq!(assistant_turn["error"], "backend failed");
    assert_ne!(assistant_turn["content"], "Do not persist as success");
    assert_eq!(
        assistant_turn["token_usage"],
        json!({"total": {"inputTokens": 9, "outputTokens": 2}})
    );

    let segment = segment_by_kind(
        snapshot["segments"].as_array().expect("segments"),
        "assistant_message",
    );
    assert_eq!(segment["status"], "failed");
    assert_eq!(segment["error"], "backend failed");
    assert_ne!(segment["content"], "Do not persist as success");

    let events = service
        .read_events_after(
            "conversation-stream-error-final-text",
            "/projects/stream-error-final-text",
            0,
        )
        .expect("events");
    assert!(events.iter().any(|event| {
        event["type"] == "turn_upsert"
            && event["turn"]["id"] == prepared.assistant_turn_id
            && event["turn"]["status"] == "failed"
            && event["turn"]["token_usage"]["total"]["inputTokens"] == json!(9)
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

#[derive(Clone)]
struct ScriptedAgentTurnBackend {
    requests: Arc<Mutex<Vec<AgentTurnRequest>>>,
    outputs: Arc<Mutex<VecDeque<AgentTurnOutput>>>,
}

impl ScriptedAgentTurnBackend {
    fn new(outputs: Vec<AgentTurnOutput>) -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            outputs: Arc::new(Mutex::new(VecDeque::from(outputs))),
        }
    }

    fn requests(&self) -> Arc<Mutex<Vec<AgentTurnRequest>>> {
        Arc::clone(&self.requests)
    }
}

impl AgentTurnBackend for ScriptedAgentTurnBackend {
    fn run_turn(&self, request: AgentTurnRequest) -> Result<AgentTurnOutput, AgentError> {
        self.requests.lock().expect("requests").push(request);
        self.outputs
            .lock()
            .expect("outputs")
            .pop_front()
            .ok_or_else(|| AgentError {
                message: "No scripted agent output available.".to_string(),
                retryable: false,
                raw: None,
            })
    }
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

fn write_state(conversations_dir: &Path, conversation_id: &str, state: Value) {
    let directory = conversations_dir.join(conversation_id);
    fs::create_dir_all(&directory).expect("conversation dir");
    fs::write(
        directory.join("state.json"),
        serde_json::to_string_pretty(&state).expect("state json"),
    )
    .expect("write state");
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
