use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use spark_agent_adapter::{
    AgentError, AgentRequestUserInputAnswerRequest, AgentThreadResumeFailure, AgentTurnBackend,
    AgentTurnOutput, AgentTurnRequest,
};
use spark_common::events::{
    TurnStreamChannel, TurnStreamEvent, TurnStreamEventKind, TurnStreamSource,
};
use spark_common::settings::SparkSettings;
use spark_http::{build_app_with_agent_turn_backend, build_app_with_rust_llm_client};
use spark_workspace::{ConversationTurnRequest, WorkspaceConversationService};
use tokio::time::{sleep, Duration};
use tower::ServiceExt;
use unified_llm_adapter::{
    stream_events, ActiveLlmProfile, AdapterError, Client, FinishReason, Message, ProviderAdapter,
    Request as LlmRequest, Response, StreamEvent, StreamEvents, Usage,
};

#[tokio::test]
async fn conversation_turn_route_executes_injected_backend_and_preserves_validation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let backend = ScriptedAgentTurnBackend::new(vec![
        AgentTurnOutput {
            final_assistant_text: Some("Previous route answer.".to_string()),
            ..AgentTurnOutput::default()
        },
        AgentTurnOutput {
            raw_log_lines: vec![spark_agent_adapter::AgentRawLogLine {
                direction: "outgoing".to_string(),
                line: "{\"event\":\"http-route-turn\"}".to_string(),
            }],
            events: vec![
                content_completed(
                    TurnStreamChannel::Assistant,
                    "Scripted route answer.",
                    "app-route-turn",
                    "final-answer",
                ),
                content_delta(
                    TurnStreamChannel::Reasoning,
                    "Checking route contracts",
                    "app-route-turn",
                    "reasoning",
                ),
                content_completed(
                    TurnStreamChannel::Reasoning,
                    "Checking route contracts.",
                    "app-route-turn",
                    "reasoning",
                ),
                model_tool_event("model_tool_call_start", "route-tool-1", "proposed", None),
                model_tool_event(
                    "model_tool_call_delta",
                    "route-tool-1",
                    "streaming",
                    Some("{\"query\":\"http route\"}"),
                ),
                model_tool_event("model_tool_call_end", "route-tool-1", "completed", None),
                tool_event(
                    TurnStreamEventKind::ToolCallStarted,
                    "route-tool-1",
                    "running",
                    "partial route output",
                ),
                tool_event(
                    TurnStreamEventKind::ToolCallCompleted,
                    "route-tool-1",
                    "completed",
                    "full route output",
                ),
                request_user_input_event(),
            ],
            final_assistant_text: Some("Scripted route answer.".to_string()),
            token_usage: Some(json!({"total": {"inputTokens": 8, "outputTokens": 4}})),
            ..AgentTurnOutput::default()
        },
    ]);
    let backend_requests = backend.requests();
    let app = build_app_with_agent_turn_backend(settings.clone(), Arc::new(backend));

    let first = request_json(
        app.clone(),
        "POST",
        "/workspace/api/conversations/conversation-http-turn/turns",
        Some(json!({
            "project_path": "/projects/http-turn",
            "message": "Prime route history",
            "provider": "codex",
            "chat_mode": "chat"
        })),
    )
    .await;
    assert_eq!(first.0, StatusCode::OK);
    wait_for_conversation_snapshot(
        &settings,
        "conversation-http-turn",
        "/projects/http-turn",
        |snapshot| {
            snapshot["turns"]
                .as_array()
                .expect("turns")
                .iter()
                .any(|turn| {
                    turn["role"] == "assistant"
                        && turn["status"] == "complete"
                        && turn["content"] == "Previous route answer."
                })
        },
    )
    .await;

    let response = request_json(
        app.clone(),
        "POST",
        "/workspace/api/conversations/conversation-http-turn/turns",
        Some(json!({
            "project_path": "/projects/http-turn",
            "message": "Ship it",
            "provider": "openai",
            "model": "gpt-5",
            "llm_profile": "frontier",
            "reasoning_effort": "HIGH",
            "chat_mode": "chat"
        })),
    )
    .await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(response.1["conversation_id"], "conversation-http-turn");
    assert_eq!(response.1["provider"], "openai");
    assert_eq!(response.1["model"], "gpt-5");
    assert_eq!(response.1["llm_profile"], "frontier");
    assert_eq!(response.1["reasoning_effort"], "high");
    assert_eq!(response.1["turns"].as_array().expect("turns").len(), 4);
    let assistant_turn = response.1["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["role"] == "assistant" && turn["status"] == "pending")
        .expect("assistant turn");
    assert_eq!(assistant_turn["content"], "");

    let final_snapshot = wait_for_conversation_snapshot(
        &settings,
        "conversation-http-turn",
        "/projects/http-turn",
        |snapshot| {
            snapshot["turns"]
                .as_array()
                .expect("turns")
                .iter()
                .any(|turn| {
                    turn["role"] == "assistant"
                        && turn["status"] == "complete"
                        && turn["content"] == "Scripted route answer."
                })
        },
    )
    .await;
    let assistant_turn = final_snapshot["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["role"] == "assistant" && turn["content"] == "Scripted route answer.")
        .expect("assistant turn");
    assert_eq!(
        assistant_turn["token_usage"]["total"]["inputTokens"],
        json!(8)
    );
    assert!(final_snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .any(|segment| segment["kind"] == "assistant_message"
            && segment["content"] == "Scripted route answer."));
    assert!(final_snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .any(|segment| segment["kind"] == "request_user_input"
            && segment["request_user_input"]["status"] == "pending"));
    assert!(final_snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .any(|segment| segment["kind"] == "reasoning"
            && segment["content"] == "Checking route contracts."
            && segment["source"]["item_id"] == "reasoning"));
    assert!(final_snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .any(|segment| segment["kind"] == "model_tool_call"
            && segment["status"] == "complete"
            && segment["tool_call"]["id"] == "route-tool-1"
            && segment["tool_call"]["arguments"] == json!({"query": "http route"})
            && segment["source"]["raw_kind"] == "model_tool_call_end"));
    assert!(final_snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .any(|segment| segment["kind"] == "tool_call"
            && segment["status"] == "complete"
            && segment["tool_call"]["id"] == "route-tool-1"
            && segment["tool_call"]["output"] == "full route output"));

    let backend_requests = backend_requests.lock().expect("backend requests");
    assert_eq!(backend_requests.len(), 2);
    let backend_request = &backend_requests[1];
    assert_eq!(backend_request.conversation_id, "conversation-http-turn");
    assert_eq!(backend_request.project_path, "/projects/http-turn");
    assert!(
        backend_request
            .prompt
            .starts_with("You are the Spark workspace assistant."),
        "new-thread turns carry the assistant frame: {}",
        backend_request.prompt
    );
    assert!(
        backend_request
            .prompt
            .ends_with("Latest user message:\nShip it"),
        "{}",
        backend_request.prompt
    );
    assert_eq!(backend_request.provider.as_deref(), Some("openai"));
    assert_eq!(backend_request.model.as_deref(), Some("gpt-5"));
    assert_eq!(backend_request.llm_profile.as_deref(), Some("frontier"));
    assert_eq!(backend_request.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(backend_request.chat_mode.as_deref(), Some("chat"));
    let user_turn_id = backend_request.metadata["spark.workspace.user_turn_id"]
        .as_str()
        .expect("user turn id");
    let assistant_turn_id = backend_request.metadata["spark.workspace.assistant_turn_id"]
        .as_str()
        .expect("assistant turn id");
    assert!(response.1["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .any(|turn| turn["id"] == user_turn_id && turn["content"] == "Ship it"));
    assert_eq!(assistant_turn["id"], assistant_turn_id);
    let history = serde_json::to_value(&backend_request.history).expect("history");
    assert_eq!(history.as_array().expect("history").len(), 2);
    assert_eq!(history[0]["role"], "user");
    assert_eq!(history[0]["content"], "Prime route history");
    assert_eq!(history[1]["role"], "assistant");
    assert_eq!(history[1]["content"], "Previous route answer.");
    assert!(history[0].get("id").is_none());
    assert!(history[1].get("status").is_none());
    drop(backend_requests);

    let project = spark_storage::ProjectRegistry::new(&settings.data_dir)
        .ensure_project_paths("/projects/http-turn")
        .expect("project");
    assert!(project
        .conversations_dir
        .join("conversation-http-turn/conversation.json")
        .exists());
    assert!(project
        .conversations_dir
        .join("conversation-http-turn/transcript.json")
        .exists());
    assert!(project
        .conversations_dir
        .join("conversation-http-turn/journal.jsonl")
        .exists());
    assert!(!project
        .conversations_dir
        .join("conversation-http-turn/state.json")
        .exists());
    let persisted = WorkspaceConversationService::new(settings.clone())
        .get_snapshot("conversation-http-turn", Some("/projects/http-turn"))
        .expect("persisted snapshot");
    assert_eq!(persisted["turns"], final_snapshot["turns"]);
    assert_eq!(persisted["segments"], final_snapshot["segments"]);
    let raw_log = spark_storage::ConversationRepository::new(&settings.data_dir)
        .read_codex_jsonrpc_trace("conversation-http-turn", "/projects/http-turn")
        .expect("raw log");
    assert!(
        raw_log.is_empty(),
        "Codex JSON-RPC traces are disabled unless SPARK_DEBUG_CODEX_JSONRPC=1"
    );

    WorkspaceConversationService::new(settings.clone())
        .start_turn(
            "conversation-http-conflict",
            ConversationTurnRequest {
                project_path: "/projects/http-turn".to_string(),
                message: "Leave pending".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("pending turn");
    let conflict = request_json(
        app.clone(),
        "POST",
        "/workspace/api/conversations/conversation-http-conflict/turns",
        Some(json!({"project_path": "/projects/http-turn", "message": "Again"})),
    )
    .await;
    assert_eq!(conflict.0, StatusCode::CONFLICT);
    assert!(conflict.1["detail"]
        .as_str()
        .expect("detail")
        .contains("assistant turn is still in progress"));

    let empty = request_json(
        app.clone(),
        "POST",
        "/workspace/api/conversations/conversation-empty/turns",
        Some(json!({"project_path": "/projects/http-turn", "message": "   "})),
    )
    .await;
    assert_eq!(empty.0, StatusCode::BAD_REQUEST);
    assert_eq!(empty.1, json!({"detail": "Message is required."}));

    let missing_model = request_json(
        app.clone(),
        "POST",
        "/workspace/api/conversations/conversation-model/turns",
        Some(json!({"project_path": "/projects/http-turn", "message": "Hi", "provider": "openrouter"})),
    )
    .await;
    assert_eq!(missing_model.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        missing_model.1,
        json!({"detail": "Provider openrouter requires an explicit model."})
    );

    let malformed = request_text(
        app,
        "POST",
        "/workspace/api/conversations/conversation-http-turn/turns",
        "{not-json",
        Some("application/json"),
    )
    .await;
    assert_eq!(malformed.0, StatusCode::BAD_REQUEST);
    assert!(malformed.2.starts_with("application/json"));
}

#[tokio::test]
async fn conversation_turn_route_uses_rust_llm_client_backend_for_openai_compatible_profile() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(RecordingAdapter::new(
        "openai_compatible",
        Arc::clone(&calls),
    ));
    let client = Client::new()
        .with_llm_profile_adapter(
            "frontier",
            ActiveLlmProfile::new("openai_compatible", Some("profile-default".to_string())),
            adapter,
        )
        .expect("client");
    let app = build_app_with_rust_llm_client(settings.clone(), client);

    let response = request_json(
        app,
        "POST",
        "/workspace/api/conversations/conversation-http-rust-agent/turns",
        Some(json!({
            "project_path": "/projects/http-rust-agent",
            "message": "Route through the Rust agent backend.",
            "provider": "openai_compatible",
            "model": "gpt-route-agent",
            "llm_profile": "frontier",
            "reasoning_effort": "HIGH",
            "chat_mode": "chat"
        })),
    )
    .await;

    assert_eq!(response.0, StatusCode::OK);
    let turns = response.1["turns"].as_array().expect("turns");
    let assistant_turn = turns
        .iter()
        .find(|turn| turn["role"] == "assistant")
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "pending");
    assert_eq!(assistant_turn["content"], "");

    let final_snapshot = wait_for_conversation_snapshot(
        &settings,
        "conversation-http-rust-agent",
        "/projects/http-rust-agent",
        |snapshot| {
            snapshot["turns"]
                .as_array()
                .expect("turns")
                .iter()
                .any(|turn| {
                    turn["role"] == "assistant"
                        && turn["status"] == "complete"
                        && turn["content"] == "route adapter response for gpt-route-agent"
                })
        },
    )
    .await;
    let assistant_turn = final_snapshot["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["role"] == "assistant")
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "complete");
    assert_eq!(
        assistant_turn["content"],
        "route adapter response for gpt-route-agent"
    );

    let calls = calls.lock().expect("calls");
    assert_eq!(calls.len(), 1);
    let request = &calls[0];
    assert_eq!(request.provider.as_deref(), Some("openai_compatible"));
    assert_eq!(request.model, "gpt-route-agent");
    let user_message = request.messages.last().expect("user message");
    assert!(
        user_message
            .text()
            .ends_with("Latest user message:\nRoute through the Rust agent backend."),
        "{:?}",
        user_message
    );
    assert_eq!(request.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(
        request.metadata["spark.runtime.provider"],
        json!("openai_compatible")
    );
    assert_eq!(
        request.metadata["spark.runtime.model"],
        json!("gpt-route-agent")
    );
    assert_eq!(
        request.metadata["spark.runtime.llm_profile"],
        json!("frontier")
    );
    assert_eq!(
        request.metadata["spark.runtime.reasoning_effort"],
        json!("high")
    );
    assert_eq!(
        request.metadata["spark.runtime.provider_selector"],
        json!("openai_compatible")
    );
    assert_eq!(request.metadata["spark.runtime.chat_mode"], json!("chat"));
}

#[tokio::test]
async fn conversation_turn_route_persists_structured_backend_error_output() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let app = build_app_with_agent_turn_backend(
        settings.clone(),
        Arc::new(ScriptedAgentTurnBackend::new(vec![AgentTurnOutput {
            events: vec![stream_error_event("route backend stream failed")],
            token_usage: Some(json!({"total": {"inputTokens": 9, "outputTokens": 1}})),
            ..AgentTurnOutput::default()
        }])),
    );

    let response = request_json(
        app,
        "POST",
        "/workspace/api/conversations/conversation-http-output-error/turns",
        Some(json!({
            "project_path": "/projects/http-output-error",
            "message": "Preserve this failed output"
        })),
    )
    .await;

    assert_eq!(response.0, StatusCode::OK);
    let started_assistant_turn = response.1["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["role"] == "assistant")
        .expect("assistant turn");
    assert_eq!(started_assistant_turn["status"], "pending");
    assert_eq!(started_assistant_turn["content"], "");

    let final_snapshot = wait_for_conversation_snapshot(
        &settings,
        "conversation-http-output-error",
        "/projects/http-output-error",
        |snapshot| {
            snapshot["turns"]
                .as_array()
                .expect("turns")
                .iter()
                .any(|turn| {
                    turn["role"] == "assistant"
                        && turn["status"] == "failed"
                        && turn["error"] == "route backend stream failed"
                })
        },
    )
    .await;
    let assistant_turn = final_snapshot["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["role"] == "assistant")
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "failed");
    assert_eq!(assistant_turn["error"], "route backend stream failed");
    assert_eq!(
        assistant_turn["token_usage"],
        json!({"total": {"inputTokens": 9, "outputTokens": 1}})
    );
    assert!(final_snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .any(|segment| segment["kind"] == "assistant_message"
            && segment["status"] == "failed"
            && segment["error"] == "route backend stream failed"));
    let persisted = WorkspaceConversationService::new(settings.clone())
        .get_snapshot(
            "conversation-http-output-error",
            Some("/projects/http-output-error"),
        )
        .expect("persisted snapshot");
    assert_eq!(persisted["turns"], final_snapshot["turns"]);
    assert_eq!(persisted["segments"], final_snapshot["segments"]);

    let events = wait_for_conversation_events(
        &settings,
        "conversation-http-output-error",
        "/projects/http-output-error",
        0,
        |events| {
            events.iter().any(|event| {
                event["type"] == "turn_upsert"
                    && event["turn"]["status"] == "failed"
                    && event["turn"]["error"] == "route backend stream failed"
            }) && events.iter().any(|event| {
                event["type"] == "segment_upsert"
                    && event["segment"]["status"] == "failed"
                    && event["segment"]["error"] == "route backend stream failed"
            })
        },
    )
    .await;
    assert!(events.iter().any(|event| {
        event["type"] == "turn_upsert"
            && event["turn"]["status"] == "failed"
            && event["turn"]["error"] == "route backend stream failed"
    }));
    assert!(events.iter().any(|event| {
        event["type"] == "segment_upsert"
            && event["segment"]["status"] == "failed"
            && event["segment"]["error"] == "route backend stream failed"
    }));
}

#[tokio::test]
async fn conversation_turn_route_persists_thread_resume_failure_details() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let app = build_app_with_agent_turn_backend(
        settings.clone(),
        Arc::new(ScriptedAgentTurnBackend::new(vec![AgentTurnOutput {
            token_usage: Some(json!({"total": {"inputTokens": 5, "outputTokens": 0}})),
            thread_resume_failure: Some(AgentThreadResumeFailure {
                message: "route thread resume failed".to_string(),
                error_code: Some("thread_resume_failed".to_string()),
                details: Some(json!({"thread_id": "thread-http-resume"})),
            }),
            ..AgentTurnOutput::default()
        }])),
    );

    let response = request_json(
        app,
        "POST",
        "/workspace/api/conversations/conversation-http-resume-failure/turns",
        Some(json!({
            "project_path": "/projects/http-resume-failure",
            "message": "Resume this thread"
        })),
    )
    .await;

    assert_eq!(response.0, StatusCode::OK);
    let started_assistant_turn = response.1["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["role"] == "assistant")
        .expect("assistant turn");
    assert_eq!(started_assistant_turn["status"], "pending");

    let final_snapshot = wait_for_conversation_snapshot(
        &settings,
        "conversation-http-resume-failure",
        "/projects/http-resume-failure",
        |snapshot| {
            snapshot["turns"]
                .as_array()
                .expect("turns")
                .iter()
                .any(|turn| {
                    turn["role"] == "assistant"
                        && turn["status"] == "failed"
                        && turn["error_code"] == "thread_resume_failed"
                })
        },
    )
    .await;
    let assistant_turn = final_snapshot["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["role"] == "assistant")
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "failed");
    assert_eq!(assistant_turn["error"], "route thread resume failed");
    assert_eq!(assistant_turn["error_code"], "thread_resume_failed");
    assert_eq!(
        assistant_turn["token_usage"],
        json!({"total": {"inputTokens": 5, "outputTokens": 0}})
    );
    assert_eq!(final_snapshot["event_log"][0]["kind"], "continuity_reset");
    assert_eq!(
        final_snapshot["event_log"][0]["details"]["thread_id"],
        "thread-http-resume"
    );
    let persisted = WorkspaceConversationService::new(settings.clone())
        .get_snapshot(
            "conversation-http-resume-failure",
            Some("/projects/http-resume-failure"),
        )
        .expect("persisted snapshot");
    assert_eq!(persisted["turns"], final_snapshot["turns"]);
    assert_eq!(persisted["event_log"], final_snapshot["event_log"]);

    let events = wait_for_conversation_events(
        &settings,
        "conversation-http-resume-failure",
        "/projects/http-resume-failure",
        0,
        |events| {
            events.iter().any(|event| {
                event["type"] == "conversation_snapshot_ref" && event.get("state").is_none()
            })
        },
    )
    .await;
    assert!(events.iter().any(|event| {
        event["type"] == "turn_upsert"
            && event["turn"]["status"] == "failed"
            && event["turn"]["error_code"] == "thread_resume_failed"
    }));
    // The workflow-event commit journals a slim snapshot ref (durable
    // event_log content is asserted through the persisted snapshot above).
    assert!(events.iter().any(|event| {
        event["type"] == "conversation_snapshot_ref" && event.get("state").is_none()
    }));
}

async fn wait_for_conversation_events<F>(
    settings: &SparkSettings,
    conversation_id: &str,
    project_path: &str,
    revision: i64,
    predicate: F,
) -> Vec<Value>
where
    F: Fn(&[Value]) -> bool,
{
    let service = WorkspaceConversationService::new(settings.clone());
    let mut last_events = Vec::new();
    for _ in 0..100 {
        let events = service
            .read_events_after(conversation_id, project_path, revision)
            .expect("events");
        if predicate(&events) {
            return events;
        }
        last_events = events;
        sleep(Duration::from_millis(10)).await;
    }
    panic!(
        "conversation events did not satisfy predicate: {}",
        serde_json::to_string_pretty(&last_events).expect("events json")
    );
}

#[tokio::test]
async fn conversation_turn_route_returns_workspace_error_for_backend_trait_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let backend = ScriptedAgentTurnBackend::new(Vec::new());
    let backend_requests = backend.requests();
    let app = build_app_with_agent_turn_backend(settings.clone(), Arc::new(backend));

    let response = request_json(
        app,
        "POST",
        "/workspace/api/conversations/conversation-http-backend-error/turns",
        Some(json!({
            "project_path": "/projects/http-backend-error",
            "message": "Backend should fail"
        })),
    )
    .await;

    assert_eq!(response.0, StatusCode::OK);
    let started_assistant_turn = response.1["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["role"] == "assistant")
        .expect("assistant turn");
    assert_eq!(started_assistant_turn["status"], "pending");

    let final_snapshot = wait_for_conversation_snapshot(
        &settings,
        "conversation-http-backend-error",
        "/projects/http-backend-error",
        |snapshot| {
            snapshot["turns"]
                .as_array()
                .expect("turns")
                .iter()
                .any(|turn| {
                    turn["role"] == "assistant"
                        && turn["status"] == "failed"
                        && turn["error"] == "No scripted agent output available."
                })
        },
    )
    .await;
    assert!(final_snapshot["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .any(|turn| turn["role"] == "assistant"
            && turn["status"] == "failed"
            && turn["error"] == "No scripted agent output available."));
    let backend_requests = backend_requests.lock().expect("backend requests");
    assert_eq!(backend_requests.len(), 1);
    assert!(
        backend_requests[0]
            .prompt
            .ends_with("Latest user message:\nBackend should fail"),
        "{}",
        backend_requests[0].prompt
    );
}

#[tokio::test]
async fn request_user_input_answer_route_continues_pending_requests() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = WorkspaceConversationService::new(settings.clone());
    let (prepared, _) = service
        .start_turn(
            "conversation-http-input",
            ConversationTurnRequest {
                project_path: "/projects/http-input".to_string(),
                message: "Need input".to_string(),
                provider: Some("openrouter".to_string()),
                model: Some("openrouter/route-input".to_string()),
                llm_profile: Some("implementation".to_string()),
                reasoning_effort: Some("HIGH".to_string()),
                chat_mode: Some("chat".to_string()),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start");
    service
        .ingest_agent_turn_output(
            "conversation-http-input",
            "/projects/http-input",
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![request_user_input_event()],
                ..AgentTurnOutput::default()
            },
        )
        .expect("pending request");
    let backend = ScriptedAgentTurnBackend::with_answer_outputs(
        Vec::new(),
        vec![AgentTurnOutput {
            final_assistant_text: Some("Route answer after input.".to_string()),
            token_usage: Some(json!({"total": {"inputTokens": 4, "outputTokens": 2}})),
            ..AgentTurnOutput::default()
        }],
    );
    let answer_requests = backend.answer_requests();
    let app = build_app_with_agent_turn_backend(settings, Arc::new(backend));

    let answered = request_json(
        app.clone(),
        "POST",
        "/workspace/api/conversations/conversation-http-input/request-user-input/decision/answer",
        Some(json!({
            "project_path": "/projects/http-input",
            "answers": {"decision": "Approve"}
        })),
    )
    .await;
    assert_eq!(answered.0, StatusCode::OK);
    let segment = answered.1["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .find(|segment| segment["kind"] == "request_user_input")
        .expect("request segment");
    assert_eq!(segment["request_user_input"]["status"], "answered");
    assert_eq!(segment["status"], "complete");
    let assistant_turn = answered.1["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == prepared.assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "complete");
    assert_eq!(assistant_turn["content"], "Route answer after input.");
    assert_eq!(
        assistant_turn["token_usage"]["total"]["inputTokens"],
        json!(4)
    );
    let answer_requests = answer_requests.lock().expect("answer requests");
    assert_eq!(answer_requests.len(), 1);
    assert_eq!(
        answer_requests[0].conversation_id,
        "conversation-http-input"
    );
    assert_eq!(answer_requests[0].project_path, "/projects/http-input");
    assert_eq!(answer_requests[0].request_id, "input-1");
    assert_eq!(
        answer_requests[0].assistant_turn_id,
        prepared.assistant_turn_id
    );
    assert_eq!(answer_requests[0].answers["decision"], "Approve");
    assert_eq!(answer_requests[0].provider.as_deref(), Some("openrouter"));
    assert_eq!(
        answer_requests[0].model.as_deref(),
        Some("openrouter/route-input")
    );
    assert_eq!(
        answer_requests[0].llm_profile.as_deref(),
        Some("implementation")
    );
    assert_eq!(answer_requests[0].reasoning_effort.as_deref(), Some("high"));
    assert_eq!(answer_requests[0].chat_mode.as_deref(), Some("chat"));
    drop(answer_requests);

    let changed = request_json(
        app.clone(),
        "POST",
        "/workspace/api/conversations/conversation-http-input/request-user-input/decision/answer",
        Some(json!({
            "project_path": "/projects/http-input",
            "answers": {"decision": "Reject"}
        })),
    )
    .await;
    assert_eq!(changed.0, StatusCode::CONFLICT);

    let missing = request_json(
        app,
        "POST",
        "/workspace/api/conversations/conversation-http-input/request-user-input/missing/answer",
        Some(json!({
            "project_path": "/projects/http-input",
            "answers": {"decision": "Approve"}
        })),
    )
    .await;
    assert_eq!(missing.0, StatusCode::NOT_FOUND);
    assert_eq!(
        missing.1,
        json!({"detail": "Unknown conversation input request: missing"})
    );
}

#[tokio::test]
async fn request_user_input_answer_route_returns_backend_errors_without_expiring_request() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = WorkspaceConversationService::new(settings.clone());
    let (prepared, _) = service
        .start_turn(
            "conversation-http-input-backend-error",
            ConversationTurnRequest {
                project_path: "/projects/http-input-backend-error".to_string(),
                message: "Need input".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start");
    service
        .ingest_agent_turn_output(
            "conversation-http-input-backend-error",
            "/projects/http-input-backend-error",
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![request_user_input_event()],
                ..AgentTurnOutput::default()
            },
        )
        .expect("pending request");
    let backend = ScriptedAgentTurnBackend::with_answer_outputs(Vec::new(), Vec::new());
    let answer_requests = backend.answer_requests();
    let app = build_app_with_agent_turn_backend(settings.clone(), Arc::new(backend));

    let response = request_json(
        app,
        "POST",
        "/workspace/api/conversations/conversation-http-input-backend-error/request-user-input/decision/answer",
        Some(json!({
            "project_path": "/projects/http-input-backend-error",
            "answers": {"decision": "Approve"}
        })),
    )
    .await;

    assert_eq!(response.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response.1,
        json!({"detail": "No scripted agent answer output available."})
    );
    assert!(response.1.get("segments").is_none());
    let answer_requests = answer_requests.lock().expect("answer requests");
    assert_eq!(answer_requests.len(), 1);
    assert_eq!(answer_requests[0].request_id, "input-1");
    drop(answer_requests);

    let snapshot = WorkspaceConversationService::new(settings)
        .get_snapshot(
            "conversation-http-input-backend-error",
            Some("/projects/http-input-backend-error"),
        )
        .expect("stored snapshot");
    let segment = snapshot["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .find(|segment| segment["kind"] == "request_user_input")
        .expect("request segment");
    assert_eq!(segment["status"], "complete");
    assert_eq!(segment["request_user_input"]["status"], "answered");
    assert_eq!(
        segment["request_user_input"]["answers"]["decision"],
        "Approve"
    );
    assert!(segment.get("error").is_none());
    let assistant_turn = snapshot["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == prepared.assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "streaming");
    assert!(assistant_turn.get("error").is_none());
}

async fn request_json(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    let request = if let Some(body) = body {
        builder = builder.header("content-type", "application/json");
        builder
            .body(Body::from(body.to_string()))
            .expect("request body")
    } else {
        builder.body(Body::empty()).expect("request body")
    };
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = serde_json::from_slice::<Value>(&bytes).expect("json");
    (status, value, content_type)
}

async fn wait_for_conversation_snapshot<F>(
    settings: &SparkSettings,
    conversation_id: &str,
    project_path: &str,
    predicate: F,
) -> Value
where
    F: Fn(&Value) -> bool,
{
    let service = WorkspaceConversationService::new(settings.clone());
    let mut last_snapshot = None;
    for _ in 0..100 {
        let snapshot = service
            .get_snapshot(conversation_id, Some(project_path))
            .expect("snapshot");
        if predicate(&snapshot) {
            return snapshot;
        }
        last_snapshot = Some(snapshot);
        sleep(Duration::from_millis(10)).await;
    }
    panic!(
        "conversation snapshot did not satisfy predicate: {}",
        serde_json::to_string_pretty(&last_snapshot).expect("snapshot json")
    );
}

async fn request_text(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: &str,
    content_type: Option<&str>,
) -> (StatusCode, String, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(content_type) = content_type {
        builder = builder.header("content-type", content_type);
    }
    let request = builder
        .body(Body::from(body.to_string()))
        .expect("request body");
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (
        status,
        String::from_utf8(bytes.to_vec()).expect("utf-8"),
        content_type,
    )
}

#[derive(Clone)]
struct ScriptedAgentTurnBackend {
    requests: Arc<Mutex<Vec<AgentTurnRequest>>>,
    outputs: Arc<Mutex<VecDeque<AgentTurnOutput>>>,
    answer_requests: Arc<Mutex<Vec<AgentRequestUserInputAnswerRequest>>>,
    answer_outputs: Arc<Mutex<VecDeque<AgentTurnOutput>>>,
}

impl ScriptedAgentTurnBackend {
    fn new(outputs: Vec<AgentTurnOutput>) -> Self {
        Self::with_answer_outputs(outputs, Vec::new())
    }

    fn with_answer_outputs(
        mut outputs: Vec<AgentTurnOutput>,
        mut answer_outputs: Vec<AgentTurnOutput>,
    ) -> Self {
        for (index, output) in outputs.iter_mut().enumerate() {
            add_final_answer_event(output, &format!("scripted-turn-{index}"));
        }
        for (index, output) in answer_outputs.iter_mut().enumerate() {
            add_final_answer_event(output, &format!("scripted-answer-{index}"));
        }
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            outputs: Arc::new(Mutex::new(VecDeque::from(outputs))),
            answer_requests: Arc::new(Mutex::new(Vec::new())),
            answer_outputs: Arc::new(Mutex::new(VecDeque::from(answer_outputs))),
        }
    }

    fn requests(&self) -> Arc<Mutex<Vec<AgentTurnRequest>>> {
        Arc::clone(&self.requests)
    }

    fn answer_requests(&self) -> Arc<Mutex<Vec<AgentRequestUserInputAnswerRequest>>> {
        Arc::clone(&self.answer_requests)
    }
}

fn add_final_answer_event(output: &mut AgentTurnOutput, app_turn_id: &str) {
    if output
        .events
        .iter()
        .any(|event| event.kind == TurnStreamEventKind::Error)
    {
        return;
    }
    let Some(text) = output.final_assistant_text.clone() else {
        return;
    };
    if output.events.iter().any(|event| {
        event.kind == TurnStreamEventKind::ContentCompleted
            && event.channel == Some(TurnStreamChannel::Assistant)
            && event.phase.as_deref() == Some("final_answer")
    }) {
        return;
    }
    output.events.push(content_completed(
        TurnStreamChannel::Assistant,
        &text,
        app_turn_id,
        "final",
    ));
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

    fn answer_request_user_input(
        &self,
        request: AgentRequestUserInputAnswerRequest,
    ) -> Result<AgentTurnOutput, AgentError> {
        self.answer_requests
            .lock()
            .expect("answer requests")
            .push(request);
        self.answer_outputs
            .lock()
            .expect("answer outputs")
            .pop_front()
            .ok_or_else(|| AgentError {
                message: "No scripted agent answer output available.".to_string(),
                retryable: false,
                raw: None,
            })
    }
}

struct RecordingAdapter {
    name: &'static str,
    calls: Arc<Mutex<Vec<LlmRequest>>>,
}

impl RecordingAdapter {
    fn new(name: &'static str, calls: Arc<Mutex<Vec<LlmRequest>>>) -> Self {
        Self { name, calls }
    }
}

impl ProviderAdapter for RecordingAdapter {
    fn name(&self) -> &str {
        self.name
    }

    fn complete(&self, request: LlmRequest) -> Result<Response, AdapterError> {
        self.calls.lock().expect("calls").push(request.clone());
        Ok(Response {
            model: request.model.clone(),
            provider: request.provider.clone().unwrap_or_default(),
            message: Message::assistant(format!("route adapter response for {}", request.model)),
            finish_reason: FinishReason::Stop,
            usage: Usage {
                input_tokens: 3,
                output_tokens: 4,
                total_tokens: 7,
                ..Usage::default()
            },
            ..Response::default()
        })
    }

    fn stream(&self, request: LlmRequest) -> Result<StreamEvents, AdapterError> {
        self.calls.lock().expect("calls").push(request.clone());
        Ok(stream_events(
            vec![
                Ok(StreamEvent::text_delta(format!(
                    "route adapter response for {}",
                    request.model
                ))),
                Ok(StreamEvent::finish(
                    FinishReason::Stop,
                    Some(Usage {
                        input_tokens: 3,
                        output_tokens: 4,
                        total_tokens: 7,
                        ..Usage::default()
                    }),
                )),
            ]
            .into_iter(),
        ))
    }
}

fn request_user_input_event() -> TurnStreamEvent {
    TurnStreamEvent {
        kind: TurnStreamEventKind::RequestUserInputRequested,
        channel: None,
        source: TurnStreamSource {
            app_turn_id: Some("app-turn".to_string()),
            item_id: Some("input-1".to_string()),
            ..TurnStreamSource::default()
        },
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
                    ]
                }
            ]
        })),
        token_usage: None,
        error: None,
        error_code: None,
        details: None,
        phase: None,
        status: None,
    }
}

fn content_delta(
    channel: TurnStreamChannel,
    text: &str,
    app_turn_id: &str,
    item_id: &str,
) -> TurnStreamEvent {
    let mut event = TurnStreamEvent::content_delta(channel, text);
    event.source = source(app_turn_id, item_id);
    event
}

fn content_completed(
    channel: TurnStreamChannel,
    text: &str,
    app_turn_id: &str,
    item_id: &str,
) -> TurnStreamEvent {
    TurnStreamEvent {
        kind: TurnStreamEventKind::ContentCompleted,
        channel: Some(channel),
        source: source(app_turn_id, item_id),
        content_delta: Some(text.to_string()),
        message: Some(text.to_string()),
        tool_call: None,
        request_user_input: None,
        token_usage: None,
        error: None,
        error_code: None,
        details: None,
        phase: Some("final_answer".to_string()),
        status: None,
    }
}

fn model_tool_event(kind: &str, id: &str, status: &str, delta: Option<&str>) -> TurnStreamEvent {
    let mut tool_call = json!({
        "id": id,
        "kind": "model_tool_call",
        "status": status,
        "name": "lookup",
        "title": "lookup",
        "arguments": {"query": "http route"},
    });
    if let Some(delta) = delta {
        tool_call["delta"] = json!(delta);
    }

    TurnStreamEvent {
        kind: TurnStreamEventKind::Other(kind.to_string()),
        channel: None,
        source: TurnStreamSource {
            app_turn_id: Some("app-route-turn".to_string()),
            item_id: Some(id.to_string()),
            response_id: Some("resp-route".to_string()),
            raw_kind: Some(kind.to_string()),
            ..TurnStreamSource::default()
        },
        content_delta: delta.map(str::to_string),
        message: None,
        tool_call: Some(tool_call),
        request_user_input: None,
        token_usage: None,
        error: None,
        error_code: None,
        details: None,
        phase: None,
        status: Some(status.to_string()),
    }
}

fn tool_event(kind: TurnStreamEventKind, id: &str, status: &str, output: &str) -> TurnStreamEvent {
    TurnStreamEvent {
        kind,
        channel: None,
        source: source("app-route-turn", id),
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
        error_code: None,
        details: None,
        phase: None,
        status: None,
    }
}

fn stream_error_event(message: &str) -> TurnStreamEvent {
    TurnStreamEvent {
        kind: TurnStreamEventKind::Error,
        channel: Some(TurnStreamChannel::Assistant),
        source: TurnStreamSource {
            app_turn_id: Some("app-turn".to_string()),
            item_id: Some("error-item".to_string()),
            ..TurnStreamSource::default()
        },
        content_delta: None,
        message: Some(message.to_string()),
        tool_call: None,
        request_user_input: None,
        token_usage: None,
        error: Some(message.to_string()),
        error_code: None,
        details: None,
        phase: Some("final_answer".to_string()),
        status: Some("failed".to_string()),
    }
}

fn source(app_turn_id: &str, item_id: &str) -> TurnStreamSource {
    TurnStreamSource {
        app_turn_id: Some(app_turn_id.to_string()),
        item_id: Some(item_id.to_string()),
        ..TurnStreamSource::default()
    }
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
