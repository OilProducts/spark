use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use spark_agent_adapter::{
    AgentError, AgentRawLogLine, AgentRequestUserInputAnswerRequest, AgentThreadResumeFailure,
    AgentTurnBackend, AgentTurnEventSink, AgentTurnOutput, AgentTurnRequest,
};
use spark_common::debug::{
    CODEX_JSONRPC_TRACE_FILE_NAME, CODEX_JSONRPC_TRACE_PATH_METADATA_KEY,
    ENV_SPARK_DEBUG_CODEX_JSONRPC,
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

struct EnvVarGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, previous }
    }

    fn remove(key: &'static str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

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
    // Mode-change turn, user turn, assistant turn, plus one settings-change
    // journal entry.
    assert_eq!(snapshot["revision"], 4);

    let events = service
        .read_events_after("conversation-turn", "/projects/turns", 0)
        .expect("events");
    assert_eq!(
        events
            .iter()
            .map(|event| event["type"].as_str().expect("type"))
            .collect::<Vec<_>>(),
        vec![
            "turn_upsert",
            "turn_upsert",
            "turn_upsert",
            "conversation_snapshot"
        ]
    );
    assert_eq!(
        events
            .iter()
            .map(|event| event["revision"].as_i64().expect("revision"))
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
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
fn start_turn_passes_codex_jsonrpc_trace_path_only_in_debug_mode() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = WorkspaceConversationService::new(settings.clone());

    let _debug_guard = EnvVarGuard::remove(ENV_SPARK_DEBUG_CODEX_JSONRPC);
    let (default_prepared, _) = service
        .start_turn(
            "conversation-trace-default",
            ConversationTurnRequest {
                project_path: "/projects/trace-default".to_string(),
                message: "Default trace".to_string(),
                provider: Some("codex".to_string()),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start default turn");
    assert!(default_prepared
        .agent_turn_request
        .metadata
        .get(CODEX_JSONRPC_TRACE_PATH_METADATA_KEY)
        .is_none());

    drop(_debug_guard);
    let _debug_guard = EnvVarGuard::set(ENV_SPARK_DEBUG_CODEX_JSONRPC, "1");
    let (debug_prepared, _) = service
        .start_turn(
            "conversation-trace-debug",
            ConversationTurnRequest {
                project_path: "/projects/trace-debug".to_string(),
                message: "Debug trace".to_string(),
                provider: Some("codex".to_string()),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start debug turn");
    let trace_path = debug_prepared
        .agent_turn_request
        .metadata
        .get(CODEX_JSONRPC_TRACE_PATH_METADATA_KEY)
        .and_then(Value::as_str)
        .expect("trace path");
    assert!(trace_path.ends_with(&format!(
        "/conversation-trace-debug/{CODEX_JSONRPC_TRACE_FILE_NAME}"
    )));
    assert!(!Path::new(trace_path).exists());
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
            token_usage_breakdown: Some(json!({
                "total": {"inputTokens": 12, "outputTokens": 5},
                "last": {"inputTokens": 4, "outputTokens": 2}
            })),
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
    assert_eq!(
        assistant_turn["token_usage_breakdown"]["last"]["inputTokens"],
        json!(4)
    );

    let persisted = service
        .get_snapshot("conversation-execute", Some("/projects/execute"))
        .expect("persisted snapshot");
    assert_eq!(snapshot["revision"], persisted["revision"]);
    assert_eq!(snapshot["turns"], persisted["turns"]);

    let raw_log = spark_storage::ConversationRepository::new(&settings.data_dir)
        .read_codex_jsonrpc_trace("conversation-execute", "/projects/execute")
        .expect("raw log");
    assert!(
        raw_log.is_empty(),
        "Codex JSON-RPC traces are disabled unless SPARK_DEBUG_CODEX_JSONRPC=1"
    );

    let events = service
        .read_events_after("conversation-execute", "/projects/execute", 0)
        .expect("events");
    assert!(events.iter().any(|event| {
        event["type"] == "turn_upsert"
            && event["turn"]["id"] == assistant_turn_id
            && event["turn"]["status"] == "complete"
    }));

    let live_envelopes =
        conversation_envelopes_after(&settings, "conversation-execute", "/projects/execute", 0)
            .expect("live envelopes");
    assert!(live_envelopes.iter().any(|envelope| {
        envelope.event_type == "conversation.turn_upsert"
            && envelope.payload["turn"]["id"] == assistant_turn_id
            && envelope.payload["turn"]["status"] == "complete"
            && envelope.payload["turn"]["content"] == "Second answer"
            && envelope.payload["turn"]["token_usage"]["total"]["inputTokens"] == json!(12)
            && envelope.payload["turn"]["token_usage"]["total"]["outputTokens"] == json!(5)
            && envelope.payload["turn"]["token_usage_breakdown"]["last"]["outputTokens"] == json!(2)
    }));
}

#[test]
fn execute_turn_persists_backend_error_outputs_with_failed_shapes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let backend = ScriptedAgentTurnBackend::new(vec![
        AgentTurnOutput {
            events: vec![structured_stream_error_event(
                "backend stream failed",
                "rate_limit_exceeded",
                json!({
                    "provider": "openai",
                    "status_code": 429,
                    "raw": {"error": {"code": "rate_limit_exceeded"}}
                }),
            )],
            final_assistant_text: Some("Do not persist this final text".to_string()),
            token_usage: Some(json!({"total": {"inputTokens": 7, "outputTokens": 1}})),
            token_usage_breakdown: Some(json!({
                "total": {"inputTokens": 7, "outputTokens": 1},
                "last": {"inputTokens": 7, "outputTokens": 1}
            })),
            ..AgentTurnOutput::default()
        },
        AgentTurnOutput {
            raw_log_lines: vec![AgentRawLogLine {
                direction: "incoming".to_string(),
                line: "{\"event\":\"thread-resume-failed\"}".to_string(),
            }],
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
    assert_eq!(stream_assistant_turn["error_code"], "rate_limit_exceeded");
    assert_eq!(stream_assistant_turn["details"]["provider"], "openai");
    assert_ne!(
        stream_assistant_turn["content"],
        "Do not persist this final text"
    );
    assert_eq!(
        stream_assistant_turn["token_usage"],
        json!({"total": {"inputTokens": 7, "outputTokens": 1}})
    );
    assert_eq!(
        stream_assistant_turn["token_usage_breakdown"]["last"]["outputTokens"],
        json!(1)
    );
    let stream_segment = segment_by_kind(
        stream_failed["segments"].as_array().expect("segments"),
        "assistant_message",
    );
    assert_eq!(stream_segment["status"], "failed");
    assert_eq!(stream_segment["error"], "backend stream failed");
    assert_eq!(stream_segment["error_code"], "rate_limit_exceeded");
    assert_eq!(stream_segment["details"]["status_code"], json!(429));

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
            && event["turn"]["error_code"] == "rate_limit_exceeded"
    }));
    assert!(stream_events.iter().any(|event| {
        event["type"] == "segment_upsert"
            && event["segment"]["status"] == "failed"
            && event["segment"]["error"] == "backend stream failed"
            && event["segment"]["details"]["raw"]["error"]["code"] == "rate_limit_exceeded"
    }));

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
            && envelope.payload["turn"]["error_code"] == "rate_limit_exceeded"
            && envelope.payload["turn"]["details"]["provider"] == "openai"
            && envelope.payload["turn"]["token_usage_breakdown"]["last"]["outputTokens"] == json!(1)
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
        resume_assistant_turn["details"]["thread_id"],
        "thread-public-execute"
    );
    assert_eq!(
        resume_assistant_turn["token_usage"],
        json!({"total": {"inputTokens": 2, "outputTokens": 0}})
    );
    assert_eq!(resume_failed["event_log"][0]["kind"], "continuity_reset");
    assert_eq!(
        resume_failed["event_log"][0]["details"]["thread_id"],
        "thread-public-execute"
    );
    let resume_raw_log = spark_storage::ConversationRepository::new(&settings.data_dir)
        .read_codex_jsonrpc_trace(
            "conversation-execute-resume-failure",
            "/projects/execute-errors",
        )
        .expect("resume raw log");
    assert!(
        resume_raw_log.is_empty(),
        "Codex JSON-RPC traces are disabled unless SPARK_DEBUG_CODEX_JSONRPC=1"
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
            && event["turn"]["details"]["thread_id"] == "thread-public-execute"
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
            && envelope.payload["turn"]["details"]["thread_id"] == "thread-public-execute"
    }));
    assert!(resume_live.iter().any(|envelope| {
        envelope.event_type == "conversation.snapshot"
            && envelope.payload["state"]["event_log"][0]["details"]["thread_id"]
                == "thread-public-execute"
    }));
}

#[test]
fn execute_turn_starts_fresh_codex_thread_after_resume_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let backend = ScriptedAgentTurnBackend::new(vec![
        AgentTurnOutput {
            final_assistant_text: Some("Started".to_string()),
            app_thread_id: Some("thread-stale".to_string()),
            app_turn_id: Some("turn-started".to_string()),
            ..AgentTurnOutput::default()
        },
        AgentTurnOutput {
            thread_resume_failure: Some(AgentThreadResumeFailure {
                message: "thread resume failed".to_string(),
                error_code: Some("codex_app_server_resume_failed".to_string()),
                details: Some(json!({"thread_id": "thread-stale"})),
            }),
            ..AgentTurnOutput::default()
        },
        AgentTurnOutput {
            final_assistant_text: Some("Recovered".to_string()),
            app_thread_id: Some("thread-fresh".to_string()),
            app_turn_id: Some("turn-recovered".to_string()),
            ..AgentTurnOutput::default()
        },
    ]);
    let requests = backend.requests();
    let service =
        WorkspaceConversationService::new_with_agent_turn_backend(settings, Arc::new(backend));

    service
        .execute_turn(
            "conversation-resume-reset",
            ConversationTurnRequest {
                project_path: "/projects/resume-reset".to_string(),
                message: "Start".to_string(),
                provider: Some("codex".to_string()),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start output");
    service
        .execute_turn(
            "conversation-resume-reset",
            ConversationTurnRequest {
                project_path: "/projects/resume-reset".to_string(),
                message: "Resume stale".to_string(),
                provider: Some("codex".to_string()),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("resume failure output");
    let recovered = service
        .execute_turn(
            "conversation-resume-reset",
            ConversationTurnRequest {
                project_path: "/projects/resume-reset".to_string(),
                message: "Start fresh".to_string(),
                provider: Some("codex".to_string()),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("fresh output");

    let requests = requests.lock().expect("requests");
    assert_eq!(
        requests[1].metadata["spark.runtime.codex_app_server.thread_id"],
        json!("thread-stale")
    );
    assert!(
        !requests[2]
            .metadata
            .contains_key("spark.runtime.codex_app_server.thread_id"),
        "next explicit turn should not retry the stale app-server thread id"
    );
    assert_eq!(
        recovered["turns"].as_array().unwrap().last().unwrap()["content"],
        "Recovered"
    );
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
            agent_event(
                "session_start",
                "processing",
                json!({"state": "processing"}),
            ),
            content_delta("assistant", "Hel", "app-turn", "final"),
            content_completed("assistant", "Hello", "app-turn", "final"),
            content_delta("reasoning", "Because", "app-turn", "reasoning"),
            content_completed("reasoning", "Because.", "app-turn", "reasoning"),
            content_delta("plan", "1. Do it", "app-turn", "plan"),
            content_completed("plan", "1. Do it", "app-turn", "plan"),
            model_tool_event("model_tool_call_start", "tool-1", "proposed", None),
            model_tool_event(
                "model_tool_call_delta",
                "tool-1",
                "streaming",
                Some("{\"query\":\"rust\"}"),
            ),
            model_tool_event("model_tool_call_end", "tool-1", "completed", None),
            tool_event("tool_call_started", "tool-1", "running", "partial"),
            tool_event("tool_call_completed", "tool-1", "completed", "full output"),
            token_usage(json!({"total": {"inputTokens": 10, "outputTokens": 4}})),
            agent_warning_event("Context usage at 95%."),
            processing_completed_event(),
            agent_event("session_end", "closed", json!({"state": "closed"})),
        ],
        app_thread_id: None,
        app_turn_id: None,
        final_assistant_text: Some("Hello".to_string()),
        token_usage: Some(json!({"total": {"inputTokens": 10, "outputTokens": 4}})),
        token_usage_breakdown: Some(json!({
            "total": {"inputTokens": 10, "outputTokens": 4},
            "last": {"inputTokens": 2, "outputTokens": 1}
        })),
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
    assert_eq!(
        assistant_turn["token_usage_breakdown"]["last"]["outputTokens"],
        json!(1)
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
    let model_tool = segment_by_kind(segments, "model_tool_call");
    assert_eq!(model_tool["status"], "complete");
    assert_eq!(model_tool["tool_call"]["id"], "tool-1");
    assert_eq!(model_tool["tool_call"]["kind"], "model_tool_call");
    assert_eq!(model_tool["tool_call"]["name"], "lookup");
    assert_eq!(
        model_tool["tool_call"]["arguments"],
        json!({"query": "rust"})
    );
    assert_eq!(model_tool["source"]["call_id"], "tool-1");
    assert_eq!(model_tool["source"]["raw_kind"], "model_tool_call_end");
    assert_eq!(model_tool["source"]["response_id"], "resp-1");
    let tool = segment_by_kind(segments, "tool_call");
    assert_eq!(tool["status"], "complete");
    assert_eq!(tool["tool_call"]["kind"], "command_execution");
    assert_eq!(tool["tool_call"]["title"], "Run command");
    assert_eq!(tool["tool_call"]["command"], "cargo test");
    assert_eq!(tool["tool_call"]["output"], "full output");
    assert_eq!(tool["source"]["call_id"], "tool-1");
    assert!(segments.iter().any(|segment| {
        segment["kind"] == "agent_event"
            && segment["category"] == "lifecycle"
            && segment["event_kind"] == "session_start"
            && segment["event_status"] == "processing"
            && segment["details"]["state"] == "processing"
    }));
    assert!(segments.iter().any(|segment| {
        segment["kind"] == "agent_event"
            && segment["category"] == "warning"
            && segment["event_kind"] == "warning"
            && segment["message"] == "Context usage at 95%."
            && segment["details"]["message"] == "Context usage at 95%."
    }));
    assert!(segments.iter().any(|segment| {
        segment["kind"] == "agent_event"
            && segment["category"] == "processing"
            && segment["event_kind"] == "processing_end"
            && segment["event_status"] == "idle"
            && segment["details"]["state"] == "idle"
    }));
    assert!(segments.iter().any(|segment| {
        segment["kind"] == "agent_event"
            && segment["category"] == "lifecycle"
            && segment["event_kind"] == "session_end"
            && segment["event_status"] == "closed"
            && segment["details"]["state"] == "closed"
    }));

    let raw_log = spark_storage::ConversationRepository::new(&settings.data_dir)
        .read_codex_jsonrpc_trace("conversation-events", "/projects/events")
        .expect("raw log");
    assert!(
        raw_log.is_empty(),
        "Codex JSON-RPC traces are disabled unless SPARK_DEBUG_CODEX_JSONRPC=1"
    );

    let events = service
        .read_events_after("conversation-events", "/projects/events", 0)
        .expect("events");
    assert!(events
        .iter()
        .any(|event| event["type"] == "conversation_snapshot"));

    let live_envelopes =
        conversation_envelopes_after(&settings, "conversation-events", "/projects/events", 0)
            .expect("live envelopes");
    assert!(live_envelopes.iter().any(|envelope| {
        envelope.event_type == "conversation.turn_upsert"
            && envelope.payload["turn"]["id"] == prepared.assistant_turn_id
            && envelope.payload["turn"]["token_usage_breakdown"]["last"]["outputTokens"] == json!(1)
    }));
    assert!(live_envelopes.iter().any(|envelope| {
        envelope.event_type == "conversation.segment_upsert"
            && envelope.payload["segment"]["kind"] == "assistant_message"
            && envelope.payload["segment"]["content"] == "Hello"
    }));
    assert!(!live_envelopes.iter().any(|envelope| {
        envelope.event_type == "conversation.segment_upsert"
            && envelope.payload["segment"]["kind"] == "assistant_message"
            && envelope.payload["segment"]["content"] == "Hel"
    }));
    assert!(live_envelopes.iter().any(|envelope| {
        envelope.event_type == "conversation.segment_upsert"
            && envelope.payload["segment"]["kind"] == "reasoning"
            && envelope.payload["segment"]["content"] == "Because."
            && envelope.payload["segment"]["source"]["item_id"] == "reasoning"
    }));
    assert!(!live_envelopes.iter().any(|envelope| {
        envelope.event_type == "conversation.segment_upsert"
            && envelope.payload["segment"]["kind"] == "reasoning"
            && envelope.payload["segment"]["content"] == "Because"
    }));
    assert!(!live_envelopes.iter().any(|envelope| {
        envelope.event_type == "conversation.segment_upsert"
            && envelope.payload["segment"]["kind"] == "model_tool_call"
            && envelope.payload["segment"]["source"]["raw_kind"] == "model_tool_call_delta"
    }));
    assert!(live_envelopes.iter().any(|envelope| {
        envelope.event_type == "conversation.segment_upsert"
            && envelope.payload["segment"]["kind"] == "model_tool_call"
            && envelope.payload["segment"]["status"] == "complete"
            && envelope.payload["segment"]["tool_call"]["id"] == "tool-1"
            && envelope.payload["segment"]["source"]["raw_kind"] == "model_tool_call_end"
            && envelope.payload["segment"]["source"]["response_id"] == "resp-1"
    }));
    assert!(live_envelopes.iter().any(|envelope| {
        envelope.event_type == "conversation.segment_upsert"
            && envelope.payload["segment"]["kind"] == "tool_call"
            && envelope.payload["segment"]["status"] == "complete"
            && envelope.payload["segment"]["tool_call"]["output"] == "full output"
            && envelope.payload["segment"]["source"]["call_id"] == "tool-1"
    }));
    assert!(live_envelopes.iter().any(|envelope| {
        envelope.event_type == "conversation.segment_upsert"
            && envelope.payload["segment"]["kind"] == "agent_event"
            && envelope.payload["segment"]["event_kind"] == "warning"
            && envelope.payload["segment"]["message"] == "Context usage at 95%."
    }));
    assert!(live_envelopes.iter().any(|envelope| {
        envelope.event_type == "conversation.segment_upsert"
            && envelope.payload["segment"]["kind"] == "agent_event"
            && envelope.payload["segment"]["event_kind"] == "processing_end"
            && envelope.payload["segment"]["event_status"] == "idle"
    }));
    assert!(live_envelopes.iter().any(|envelope| {
        envelope.event_type == "conversation.snapshot"
            && envelope.payload["state"]["segments"]
                .as_array()
                .is_some_and(|segments| {
                    segments.iter().any(|segment| {
                        segment["kind"] == "agent_event"
                            && segment["event_kind"] == "session_end"
                            && segment["event_status"] == "closed"
                    })
                })
    }));

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
    assert_eq!(assistant_turn["details"]["thread_id"], "thread-1");
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
}

#[test]
fn late_segment_updates_get_new_journal_revisions_without_changing_transcript_order() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let service = WorkspaceConversationService::new(settings);
    let (prepared, _snapshot) = service
        .start_turn(
            "conversation-late-segment",
            ConversationTurnRequest {
                project_path: "/projects/late-segment".to_string(),
                message: "Run a tool then explain".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");

    service
        .ingest_agent_turn_output(
            "conversation-late-segment",
            "/projects/late-segment",
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![
                    tool_event("tool_call_started", "tool-1", "running", "partial output"),
                    content_completed("assistant", "Tool is running.", "app-turn", "final"),
                ],
                final_assistant_text: Some("Tool is running.".to_string()),
                ..AgentTurnOutput::default()
            },
        )
        .expect("initial ingest");

    let completed = service
        .ingest_agent_turn_output(
            "conversation-late-segment",
            "/projects/late-segment",
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![tool_event(
                    "tool_call_completed",
                    "tool-1",
                    "completed",
                    "full output",
                )],
                final_assistant_text: Some("Tool is running.".to_string()),
                ..AgentTurnOutput::default()
            },
        )
        .expect("late tool completion");

    let segments = completed["segments"].as_array().expect("segments");
    let tool = segment_by_kind(segments, "tool_call");
    let assistant = segment_by_kind(segments, "assistant_message");
    assert_eq!(tool["status"], "complete");
    assert_eq!(tool["tool_call"]["output"], "full output");
    assert!(
        tool["order"].as_i64().expect("tool order")
            < assistant["order"].as_i64().expect("assistant order")
    );

    let events = service
        .read_events_after("conversation-late-segment", "/projects/late-segment", 0)
        .expect("events");
    let assistant_revision = events
        .iter()
        .find(|event| {
            event["type"] == "segment_upsert"
                && event["segment"]["kind"] == "assistant_message"
                && event["segment"]["content"] == "Tool is running."
        })
        .and_then(|event| event["revision"].as_i64())
        .expect("assistant revision");
    let completed_tool_revision = events
        .iter()
        .filter(|event| {
            event["type"] == "segment_upsert"
                && event["segment"]["kind"] == "tool_call"
                && event["segment"]["status"] == "complete"
        })
        .filter_map(|event| event["revision"].as_i64())
        .max()
        .expect("completed tool revision");
    assert!(completed_tool_revision > assistant_revision);
    let completed_tool_event = events
        .iter()
        .find(|event| event["revision"].as_i64() == Some(completed_tool_revision))
        .expect("completed tool event");
    assert_eq!(completed_tool_event["segment"]["order"], tool["order"]);
}

#[test]
fn execute_turn_streams_transient_events_without_persisting_delta_payloads() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let backend = ScriptedAgentTurnBackend::with_stream_events(
        vec![AgentTurnOutput {
            events: vec![content_completed("assistant", "Hello", "app-turn", "final")],
            app_thread_id: Some("thread-live".to_string()),
            app_turn_id: Some("app-turn".to_string()),
            final_assistant_text: Some("Hello".to_string()),
            ..AgentTurnOutput::default()
        }],
        vec![vec![
            content_delta("assistant", "Hel", "app-turn", "final"),
            content_delta("reasoning", "Thinking", "app-turn", "reasoning"),
        ]],
    );
    let service = WorkspaceConversationService::new_with_agent_turn_backend(
        settings.clone(),
        Arc::new(backend),
    );
    let progress_payloads = Arc::new(Mutex::new(Vec::new()));
    let progress_payloads_for_callback = Arc::clone(&progress_payloads);

    let snapshot = service
        .execute_turn_with_progress_payloads(
            "conversation-live-stream",
            ConversationTurnRequest {
                project_path: "/projects/live-stream".to_string(),
                message: "Stream this".to_string(),
                provider: Some("codex".to_string()),
                chat_mode: Some("chat".to_string()),
                ..ConversationTurnRequest::default()
            },
            move |payload| {
                progress_payloads_for_callback
                    .lock()
                    .expect("progress payloads")
                    .push(payload);
            },
        )
        .expect("execute turn");

    let live_payloads = progress_payloads.lock().expect("progress payloads");
    // Committed start events keep the journal wire shape.
    assert!(live_payloads.iter().any(|payload| {
        payload["type"] == "turn_upsert"
            && payload["turn"]["role"] == "user"
            && payload["turn"]["content"] == "Stream this"
    }));
    // Streamed updates are transient deltas: coalesced bodies, a per-turn
    // stream sequence, the committed base revision, and no durable revision.
    assert!(live_payloads.iter().any(|payload| {
        payload["type"] == "stream_delta"
            && payload["delta_kind"] == "turn_delta"
            && payload["turn"]["role"] == "assistant"
            && payload["turn"]["status"] == "streaming"
            && payload["turn"]["content"] == "Hel"
            && payload.get("revision").is_none()
    }));
    assert!(live_payloads.iter().any(|payload| {
        payload["type"] == "stream_delta"
            && payload["delta_kind"] == "segment_delta"
            && payload["segment"]["kind"] == "assistant_message"
            && payload["segment"]["status"] == "streaming"
            && payload["segment"]["content"] == "Hel"
    }));
    assert!(live_payloads.iter().any(|payload| {
        payload["type"] == "stream_delta"
            && payload["delta_kind"] == "segment_delta"
            && payload["segment"]["kind"] == "reasoning"
            && payload["segment"]["status"] == "streaming"
            && payload["segment"]["content"] == "Thinking"
    }));
    let stream_sequences = live_payloads
        .iter()
        .filter(|payload| payload["type"] == "stream_delta")
        .map(|payload| payload["stream_sequence"].as_i64().expect("sequence"))
        .collect::<Vec<_>>();
    let mut sorted_sequences = stream_sequences.clone();
    sorted_sequences.sort_unstable();
    sorted_sequences.dedup();
    assert_eq!(
        stream_sequences, sorted_sequences,
        "stream sequences strictly increase"
    );
    drop(live_payloads);

    let assistant_turn = snapshot["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["role"] == "assistant")
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "complete");
    assert_eq!(assistant_turn["content"], "Hello");

    let events = service
        .read_events_after("conversation-live-stream", "/projects/live-stream", 0)
        .expect("events");
    assert!(!events.iter().any(|event| {
        event["type"] == "segment_upsert"
            && event["segment"]["kind"] == "assistant_message"
            && event["segment"]["content"] == "Hel"
    }));
    assert!(events.iter().any(|event| {
        event["type"] == "segment_upsert"
            && event["segment"]["kind"] == "assistant_message"
            && event["segment"]["content"] == "Hello"
    }));
}

#[test]
fn request_user_input_answers_call_backend_lifecycle_and_ingest_output() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let backend = ScriptedAgentTurnBackend::with_answer_outputs(
        Vec::new(),
        vec![AgentTurnOutput {
            raw_log_lines: vec![AgentRawLogLine {
                direction: "incoming".to_string(),
                line: "{\"event\":\"request-user-input-answer\"}".to_string(),
            }],
            events: vec![
                content_delta("assistant", "Approved ", "app-answer", "final-answer"),
                content_completed(
                    "assistant",
                    "Approved after input.",
                    "app-answer",
                    "final-answer",
                ),
                token_usage(json!({"total": {"inputTokens": 7, "outputTokens": 3}})),
            ],
            final_assistant_text: Some("Approved after input.".to_string()),
            token_usage: Some(json!({"total": {"inputTokens": 7, "outputTokens": 3}})),
            ..AgentTurnOutput::default()
        }],
    );
    let answer_requests = backend.answer_requests();
    let service = WorkspaceConversationService::new_with_agent_turn_backend(
        settings.clone(),
        Arc::new(backend),
    );
    let (prepared, _snapshot) = service
        .start_turn(
            "conversation-input",
            ConversationTurnRequest {
                project_path: "/projects/input".to_string(),
                message: "Need approval".to_string(),
                provider: Some("openrouter".to_string()),
                model: Some("openrouter/model".to_string()),
                llm_profile: Some("implementation".to_string()),
                reasoning_effort: Some("HIGH".to_string()),
                chat_mode: Some("chat".to_string()),
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
    assert_eq!(request_segment["status"], "complete");
    assert_eq!(request_segment["request_user_input"]["status"], "answered");
    assert_eq!(
        request_segment["request_user_input"]["answers"]["decision"],
        "Approve"
    );
    let assistant_turn = answered["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == prepared.assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "complete");
    assert_eq!(assistant_turn["content"], "Approved after input.");
    assert_eq!(
        assistant_turn["token_usage"]["total"]["inputTokens"],
        json!(7)
    );
    assert!(answered["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .any(|segment| segment["kind"] == "assistant_message"
            && segment["content"] == "Approved after input."));
    let raw_log = spark_storage::ConversationRepository::new(&settings.data_dir)
        .read_codex_jsonrpc_trace("conversation-input", "/projects/input")
        .expect("raw log");
    assert!(
        raw_log.is_empty(),
        "Codex JSON-RPC traces are disabled unless SPARK_DEBUG_CODEX_JSONRPC=1"
    );

    let answer_requests = answer_requests.lock().expect("answer requests");
    assert_eq!(answer_requests.len(), 1);
    let answer_request = &answer_requests[0];
    assert_eq!(answer_request.conversation_id, "conversation-input");
    assert_eq!(answer_request.project_path, "/projects/input");
    assert_eq!(answer_request.request_id, "input-1");
    assert_eq!(answer_request.assistant_turn_id, prepared.assistant_turn_id);
    assert_eq!(answer_request.answers["decision"], "Approve");
    assert_eq!(
        answer_request.request_user_input.as_ref().unwrap()["status"],
        "answered"
    );
    assert_eq!(
        answer_request.request_user_input.as_ref().unwrap()["answers"]["decision"],
        "Approve"
    );
    assert_eq!(answer_request.history.len(), 1);
    match &answer_request.history[0] {
        spark_agent_adapter::HistoryTurn::User(turn) => {
            assert_eq!(turn.text(), "Need approval");
        }
        other => panic!("unexpected history turn: {other:?}"),
    }
    assert_eq!(answer_request.provider.as_deref(), Some("openrouter"));
    assert_eq!(answer_request.model.as_deref(), Some("openrouter/model"));
    assert_eq!(
        answer_request.llm_profile.as_deref(),
        Some("implementation")
    );
    assert_eq!(answer_request.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(answer_request.chat_mode.as_deref(), Some("chat"));
    assert_eq!(
        answer_request.metadata["spark.workspace.assistant_turn_id"],
        json!(prepared.assistant_turn_id)
    );
    assert_eq!(
        answer_request.metadata["spark.workspace.request_user_input.lookup_id"],
        json!("decision")
    );
    assert_eq!(
        answer_request.metadata["spark.runtime.codex_app_server.thread_id"],
        json!("app-thread")
    );
    assert_eq!(
        answer_request.metadata["spark.runtime.codex_app_server.turn_id"],
        json!("app-turn")
    );
    drop(answer_requests);

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

#[test]
fn request_user_input_live_delivery_keeps_assistant_turn_streaming_until_original_turn_finishes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let backend = ScriptedAgentTurnBackend::with_answer_outputs(
        Vec::new(),
        vec![AgentTurnOutput {
            events: vec![TurnStreamEvent {
                kind: TurnStreamEventKind::Other("request_user_input_answer_delivered".to_string()),
                channel: None,
                source: TurnStreamSource {
                    backend: Some("codex_app_server".to_string()),
                    app_thread_id: Some("app-thread".to_string()),
                    app_turn_id: Some("app-turn".to_string()),
                    item_id: Some("input-1".to_string()),
                    raw_kind: Some("request_user_input_answer_delivered".to_string()),
                    ..TurnStreamSource::default()
                },
                content_delta: None,
                message: Some("request-user-input answer delivered.".to_string()),
                tool_call: None,
                request_user_input: None,
                token_usage: None,
                error: None,
                error_code: None,
                details: None,
                phase: Some("request_user_input_answer".to_string()),
                status: Some("delivered".to_string()),
            }],
            ..AgentTurnOutput::default()
        }],
    );
    let service = WorkspaceConversationService::new_with_agent_turn_backend(
        settings(temp.path()),
        Arc::new(backend),
    );
    let (prepared, _snapshot) = service
        .start_turn(
            "conversation-input-live",
            ConversationTurnRequest {
                project_path: "/projects/input-live".to_string(),
                message: "Need approval".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");
    service
        .ingest_agent_turn_output(
            "conversation-input-live",
            "/projects/input-live",
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![request_user_input_event()],
                ..AgentTurnOutput::default()
            },
        )
        .expect("request input");

    let answered = service
        .submit_request_user_input_answer(
            "conversation-input-live",
            "decision",
            ConversationRequestUserInputAnswerRequest {
                project_path: "/projects/input-live".to_string(),
                answers: BTreeMap::from([("decision".to_string(), "Approve".to_string())]),
            },
        )
        .expect("live answer");
    let request_segment = segment_by_kind(
        answered["segments"].as_array().expect("segments"),
        "request_user_input",
    );
    assert_eq!(request_segment["status"], "complete");
    assert_eq!(request_segment["request_user_input"]["status"], "answered");
    assert_eq!(
        request_segment["request_user_input"]["answers"]["decision"],
        "Approve"
    );
    let assistant_turn = answered["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == prepared.assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "streaming");
    assert_eq!(assistant_turn["content"], "");
    assert!(!answered["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .any(|segment| segment["kind"] == "assistant_message"));
}

#[test]
fn request_user_input_live_request_is_answerable_before_original_turn_finishes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let backend = BlockingRequestInputBackend::new();
    let service = WorkspaceConversationService::new_with_agent_turn_backend(
        settings(temp.path()),
        Arc::new(backend.clone()),
    );
    let (prepared, started_snapshot) = service
        .start_turn(
            "conversation-input-live-pending",
            ConversationTurnRequest {
                project_path: "/projects/input-live-pending".to_string(),
                message: "Need approval".to_string(),
                provider: Some("codex".to_string()),
                chat_mode: Some("plan".to_string()),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");
    let assistant_turn_id = prepared.assistant_turn_id.clone();
    let progress_payloads = Arc::new(Mutex::new(Vec::new()));
    let progress_payloads_for_callback = Arc::clone(&progress_payloads);
    let service_for_turn = service.clone();

    let turn_handle = std::thread::spawn(move || {
        service_for_turn.complete_started_turn_with_progress_payloads(
            prepared,
            started_snapshot,
            move |payload| {
                progress_payloads_for_callback
                    .lock()
                    .expect("progress payloads")
                    .push(payload);
            },
        )
    });

    backend.wait_for_request();
    assert!(
        progress_payloads
            .lock()
            .expect("progress payloads")
            .iter()
            .any(|payload| {
                payload["type"] == "segment_upsert"
                    && payload["segment"]["kind"] == "request_user_input"
                    && payload["segment"]["request_user_input"]["status"] == "pending"
            }),
        "request-user-input card should be emitted live before the turn completes"
    );

    let answered = service
        .submit_request_user_input_answer(
            "conversation-input-live-pending",
            "decision",
            ConversationRequestUserInputAnswerRequest {
                project_path: "/projects/input-live-pending".to_string(),
                answers: BTreeMap::from([("decision".to_string(), "Approve".to_string())]),
            },
        )
        .expect("live pending answer");
    let request_segment = segment_by_kind(
        answered["segments"].as_array().expect("segments"),
        "request_user_input",
    );
    assert_eq!(request_segment["request_user_input"]["status"], "answered");
    assert_eq!(
        request_segment["request_user_input"]["answers"]["decision"],
        "Approve"
    );
    let assistant_turn = answered["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "streaming");

    let completed = turn_handle
        .join()
        .expect("turn thread")
        .expect("turn completes after answer");
    let completed_assistant_turn = completed["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(completed_assistant_turn["status"], "complete");
    assert_eq!(completed_assistant_turn["content"], "Approved.");
    assert_eq!(backend.answer_requests().lock().expect("answers").len(), 1);

    let completed_segments = completed["segments"].as_array().expect("segments");
    assert!(completed_segments.iter().any(|segment| {
        segment["kind"] == "request_user_input"
            && segment["request_user_input"]["request_id"] == "input-1"
            && segment["request_user_input"]["status"] == "answered"
    }));
    assert!(completed_segments.iter().any(|segment| {
        segment["kind"] == "request_user_input"
            && segment["request_user_input"]["request_id"] == "input-2"
            && segment["request_user_input"]["status"] == "pending"
    }));

    let events = service
        .read_events_after(
            "conversation-input-live-pending",
            "/projects/input-live-pending",
            0,
        )
        .expect("events");
    let revisions = events
        .iter()
        .map(|event| event["revision"].as_i64().expect("revision"))
        .collect::<Vec<_>>();
    assert!(revisions.windows(2).all(|window| window[0] < window[1]));
    let answer_revision = events
        .iter()
        .find(|event| {
            event["type"] == "segment_upsert"
                && event["segment"]["kind"] == "request_user_input"
                && event["segment"]["request_user_input"]["request_id"] == "input-1"
                && event["segment"]["request_user_input"]["status"] == "answered"
        })
        .and_then(|event| event["revision"].as_i64())
        .expect("answer revision");
    let after_answer = service
        .read_events_after(
            "conversation-input-live-pending",
            "/projects/input-live-pending",
            answer_revision,
        )
        .expect("events after answer");
    assert!(after_answer.iter().any(|event| {
        event["type"] == "segment_upsert"
            && event["segment"]["kind"] == "request_user_input"
            && event["segment"]["request_user_input"]["request_id"] == "input-2"
            && event["segment"]["request_user_input"]["status"] == "pending"
    }));
}

#[test]
fn request_user_input_answers_expire_when_backend_lifecycle_cannot_resume() {
    let temp = tempfile::tempdir().expect("tempdir");
    let backend = ScriptedAgentTurnBackend::with_answer_outputs(
        Vec::new(),
        vec![AgentTurnOutput {
            thread_resume_failure: Some(AgentThreadResumeFailure {
                message: "request-user-input answer could not resume.".to_string(),
                error_code: Some("request_user_input_not_pending".to_string()),
                details: Some(json!({"request_id": "input-1"})),
            }),
            ..AgentTurnOutput::default()
        }],
    );
    let service = WorkspaceConversationService::new_with_agent_turn_backend(
        settings(temp.path()),
        Arc::new(backend),
    );
    let (prepared, _snapshot) = service
        .start_turn(
            "conversation-input-expired",
            ConversationTurnRequest {
                project_path: "/projects/input-expired".to_string(),
                message: "Need approval".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");
    service
        .ingest_agent_turn_output(
            "conversation-input-expired",
            "/projects/input-expired",
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![request_user_input_event()],
                ..AgentTurnOutput::default()
            },
        )
        .expect("request input");

    let answered = service
        .submit_request_user_input_answer(
            "conversation-input-expired",
            "decision",
            ConversationRequestUserInputAnswerRequest {
                project_path: "/projects/input-expired".to_string(),
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
    assert_eq!(
        assistant_turn["error"],
        "The requested input expired before the answer could be used."
    );

    let idempotent = service
        .submit_request_user_input_answer(
            "conversation-input-expired",
            "input-1",
            ConversationRequestUserInputAnswerRequest {
                project_path: "/projects/input-expired".to_string(),
                answers: BTreeMap::from([("decision".to_string(), "Approve".to_string())]),
            },
        )
        .expect("idempotent answer");
    assert_eq!(idempotent["revision"], answered["revision"]);

    let changed = service
        .submit_request_user_input_answer(
            "conversation-input-expired",
            "decision",
            ConversationRequestUserInputAnswerRequest {
                project_path: "/projects/input-expired".to_string(),
                answers: BTreeMap::from([("decision".to_string(), "Reject".to_string())]),
            },
        )
        .expect_err("changed answer conflict");
    assert!(matches!(changed, WorkspaceError::Conflict(_)));
}

#[test]
fn request_user_input_answer_backend_errors_return_error_without_expiring_request() {
    let temp = tempfile::tempdir().expect("tempdir");
    let backend = ScriptedAgentTurnBackend::with_answer_outputs(Vec::new(), Vec::new());
    let answer_requests = backend.answer_requests();
    let service = WorkspaceConversationService::new_with_agent_turn_backend(
        settings(temp.path()),
        Arc::new(backend),
    );
    let (prepared, _snapshot) = service
        .start_turn(
            "conversation-input-backend-error",
            ConversationTurnRequest {
                project_path: "/projects/input-backend-error".to_string(),
                message: "Need approval".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("start turn");
    service
        .ingest_agent_turn_output(
            "conversation-input-backend-error",
            "/projects/input-backend-error",
            &prepared.assistant_turn_id,
            "chat",
            AgentTurnOutput {
                events: vec![request_user_input_event()],
                ..AgentTurnOutput::default()
            },
        )
        .expect("request input");

    let error = service
        .submit_request_user_input_answer(
            "conversation-input-backend-error",
            "decision",
            ConversationRequestUserInputAnswerRequest {
                project_path: "/projects/input-backend-error".to_string(),
                answers: BTreeMap::from([("decision".to_string(), "Approve".to_string())]),
            },
        )
        .expect_err("backend answer error");
    assert!(matches!(
        error,
        WorkspaceError::Internal(message)
            if message == "No scripted agent answer output available."
    ));
    let answer_requests = answer_requests.lock().expect("answer requests");
    assert_eq!(answer_requests.len(), 1);
    assert_eq!(answer_requests[0].request_id, "input-1");
    drop(answer_requests);

    let snapshot = service
        .get_snapshot(
            "conversation-input-backend-error",
            Some("/projects/input-backend-error"),
        )
        .expect("stored snapshot");
    let request_segment = segment_by_kind(
        snapshot["segments"].as_array().expect("segments"),
        "request_user_input",
    );
    assert_eq!(request_segment["status"], "complete");
    assert_eq!(request_segment["request_user_input"]["status"], "answered");
    assert_eq!(
        request_segment["request_user_input"]["answers"]["decision"],
        "Approve"
    );
    assert!(request_segment.get("error").is_none());
    let assistant_turn = snapshot["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .find(|turn| turn["id"] == prepared.assistant_turn_id)
        .expect("assistant turn");
    assert_eq!(assistant_turn["status"], "streaming");
    assert!(assistant_turn.get("error").is_none());
}

#[derive(Clone)]
struct BlockingRequestInputBackend {
    requests: Arc<Mutex<Vec<AgentTurnRequest>>>,
    answer_requests: Arc<Mutex<Vec<AgentRequestUserInputAnswerRequest>>>,
    request_emitted: Arc<(Mutex<bool>, Condvar)>,
    answer_delivered: Arc<(Mutex<bool>, Condvar)>,
}

impl BlockingRequestInputBackend {
    fn new() -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            answer_requests: Arc::new(Mutex::new(Vec::new())),
            request_emitted: Arc::new((Mutex::new(false), Condvar::new())),
            answer_delivered: Arc::new((Mutex::new(false), Condvar::new())),
        }
    }

    fn wait_for_request(&self) {
        let (lock, cvar) = &*self.request_emitted;
        let emitted = lock.lock().expect("request emitted");
        let (emitted, timeout) = cvar
            .wait_timeout_while(emitted, Duration::from_secs(5), |emitted| !*emitted)
            .expect("request emitted wait");
        assert!(*emitted, "request-user-input event was not emitted");
        assert!(
            !timeout.timed_out(),
            "timed out waiting for request-user-input"
        );
    }

    fn answer_requests(&self) -> Arc<Mutex<Vec<AgentRequestUserInputAnswerRequest>>> {
        Arc::clone(&self.answer_requests)
    }
}

impl AgentTurnBackend for BlockingRequestInputBackend {
    fn run_turn(&self, request: AgentTurnRequest) -> Result<AgentTurnOutput, AgentError> {
        self.run_turn_with_event_sink(request, None)
    }

    fn run_turn_with_event_sink(
        &self,
        request: AgentTurnRequest,
        event_sink: Option<AgentTurnEventSink>,
    ) -> Result<AgentTurnOutput, AgentError> {
        self.requests.lock().expect("requests").push(request);
        if let Some(sink) = event_sink.as_ref() {
            sink(request_user_input_event());
        }
        {
            let (lock, cvar) = &*self.request_emitted;
            *lock.lock().expect("request emitted") = true;
            cvar.notify_all();
        }
        let (lock, cvar) = &*self.answer_delivered;
        let delivered = lock.lock().expect("answer delivered");
        let (delivered, timeout) = cvar
            .wait_timeout_while(delivered, Duration::from_secs(5), |delivered| !*delivered)
            .expect("answer delivered wait");
        if !*delivered || timeout.timed_out() {
            return Err(AgentError {
                message: "Timed out waiting for request-user-input answer.".to_string(),
                retryable: false,
                raw: None,
            });
        }
        if let Some(sink) = event_sink.as_ref() {
            sink(request_user_input_event_with("input-2", "decision-2"));
        }
        Ok(AgentTurnOutput {
            events: vec![content_completed(
                "assistant",
                "Approved.",
                "app-turn",
                "final",
            )],
            app_thread_id: Some("thread-live".to_string()),
            app_turn_id: Some("app-turn".to_string()),
            final_assistant_text: Some("Approved.".to_string()),
            ..AgentTurnOutput::default()
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
        let (lock, cvar) = &*self.answer_delivered;
        *lock.lock().expect("answer delivered") = true;
        cvar.notify_all();
        Ok(AgentTurnOutput {
            events: vec![TurnStreamEvent {
                kind: TurnStreamEventKind::Other("request_user_input_answer_delivered".to_string()),
                channel: None,
                source: TurnStreamSource {
                    backend: Some("codex_app_server".to_string()),
                    app_thread_id: Some("app-thread".to_string()),
                    app_turn_id: Some("app-turn".to_string()),
                    item_id: Some("input-1".to_string()),
                    raw_kind: Some("request_user_input_answer_delivered".to_string()),
                    ..TurnStreamSource::default()
                },
                content_delta: None,
                message: Some("request-user-input answer delivered.".to_string()),
                tool_call: None,
                request_user_input: None,
                token_usage: None,
                error: None,
                error_code: None,
                details: None,
                phase: Some("request_user_input_answer".to_string()),
                status: Some("delivered".to_string()),
            }],
            ..AgentTurnOutput::default()
        })
    }
}

#[derive(Clone)]
struct ScriptedAgentTurnBackend {
    requests: Arc<Mutex<Vec<AgentTurnRequest>>>,
    outputs: Arc<Mutex<VecDeque<AgentTurnOutput>>>,
    stream_events: Arc<Mutex<VecDeque<Vec<TurnStreamEvent>>>>,
    answer_requests: Arc<Mutex<Vec<AgentRequestUserInputAnswerRequest>>>,
    answer_outputs: Arc<Mutex<VecDeque<AgentTurnOutput>>>,
}

impl ScriptedAgentTurnBackend {
    fn new(outputs: Vec<AgentTurnOutput>) -> Self {
        Self::with_answer_outputs(outputs, Vec::new())
    }

    fn with_stream_events(
        outputs: Vec<AgentTurnOutput>,
        stream_events: Vec<Vec<TurnStreamEvent>>,
    ) -> Self {
        let backend = Self::with_answer_outputs(outputs, Vec::new());
        *backend.stream_events.lock().expect("stream events") = VecDeque::from(stream_events);
        backend
    }

    fn with_answer_outputs(
        outputs: Vec<AgentTurnOutput>,
        answer_outputs: Vec<AgentTurnOutput>,
    ) -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            outputs: Arc::new(Mutex::new(VecDeque::from(outputs))),
            stream_events: Arc::new(Mutex::new(VecDeque::new())),
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

    fn run_turn_with_event_sink(
        &self,
        request: AgentTurnRequest,
        event_sink: Option<AgentTurnEventSink>,
    ) -> Result<AgentTurnOutput, AgentError> {
        if let Some(sink) = event_sink {
            if let Some(events) = self
                .stream_events
                .lock()
                .expect("stream events")
                .pop_front()
            {
                for event in events {
                    sink(event);
                }
            }
        }
        self.run_turn(request)
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
        error_code: None,
        details: None,
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
            "command": "cargo test",
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

fn model_tool_event(kind: &str, id: &str, status: &str, delta: Option<&str>) -> TurnStreamEvent {
    let mut tool_call = json!({
        "id": id,
        "kind": "model_tool_call",
        "status": status,
        "name": "lookup",
        "title": "lookup",
        "arguments": {"query": "rust"},
    });
    if let Some(delta) = delta {
        tool_call["delta"] = json!(delta);
    }

    TurnStreamEvent {
        kind: TurnStreamEventKind::Other(kind.to_string()),
        channel: None,
        source: TurnStreamSource {
            app_turn_id: Some("app-turn".to_string()),
            item_id: Some(id.to_string()),
            response_id: Some("resp-1".to_string()),
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
        error_code: None,
        details: None,
        phase: None,
        status: None,
    }
}

fn request_user_input_event() -> TurnStreamEvent {
    request_user_input_event_with("input-1", "decision")
}

fn request_user_input_event_with(item_id: &str, question_id: &str) -> TurnStreamEvent {
    TurnStreamEvent {
        kind: TurnStreamEventKind::RequestUserInputRequested,
        channel: None,
        source: source("app-turn", item_id),
        content_delta: None,
        message: None,
        tool_call: None,
        request_user_input: Some(json!({
            "itemId": item_id,
            "questions": [
                {
                    "id": question_id,
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
        error_code: None,
        details: None,
        phase: None,
        status: None,
    }
}

fn agent_event(kind: &str, status: &str, details: Value) -> TurnStreamEvent {
    TurnStreamEvent {
        kind: TurnStreamEventKind::Other(kind.to_string()),
        channel: None,
        source: TurnStreamSource {
            backend: Some("agent_session".to_string()),
            app_turn_id: Some("app-turn".to_string()),
            item_id: Some(kind.to_string()),
            raw_kind: Some(kind.to_string()),
            ..TurnStreamSource::default()
        },
        content_delta: None,
        message: None,
        tool_call: None,
        request_user_input: None,
        token_usage: None,
        error: None,
        error_code: None,
        details: Some(details),
        phase: None,
        status: Some(status.to_string()),
    }
}

fn agent_warning_event(message: &str) -> TurnStreamEvent {
    let mut event = agent_event("warning", "warning", json!({"message": message}));
    event.message = Some(message.to_string());
    event
}

fn processing_completed_event() -> TurnStreamEvent {
    TurnStreamEvent {
        kind: TurnStreamEventKind::TurnCompleted,
        channel: None,
        source: TurnStreamSource {
            backend: Some("agent_session".to_string()),
            app_turn_id: Some("app-turn".to_string()),
            item_id: Some("processing".to_string()),
            raw_kind: Some("processing_end".to_string()),
            ..TurnStreamSource::default()
        },
        content_delta: None,
        message: None,
        tool_call: None,
        request_user_input: None,
        token_usage: None,
        error: None,
        error_code: None,
        details: Some(json!({"state": "idle"})),
        phase: Some("turn".to_string()),
        status: Some("idle".to_string()),
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
        error_code: None,
        details: None,
        phase: Some("final_answer".to_string()),
        status: Some("failed".to_string()),
    }
}

fn structured_stream_error_event(
    message: &str,
    error_code: &str,
    details: Value,
) -> TurnStreamEvent {
    let mut event = stream_error_event(message);
    event.error_code = Some(error_code.to_string());
    event.details = Some(details);
    event
}

fn parse_channel(channel: &str) -> TurnStreamChannel {
    channel.parse().expect("channel")
}

fn source(app_turn_id: &str, item_id: &str) -> TurnStreamSource {
    TurnStreamSource {
        app_thread_id: Some("app-thread".to_string()),
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

#[test]
fn runtime_session_records_thread_continuity_and_tombstones_on_resume_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let backend = ScriptedAgentTurnBackend::new(vec![
        AgentTurnOutput {
            app_thread_id: Some("thread-alpha".to_string()),
            final_assistant_text: Some("First answer".to_string()),
            ..AgentTurnOutput::default()
        },
        AgentTurnOutput {
            thread_resume_failure: Some(AgentThreadResumeFailure {
                message: "thread resume failed".to_string(),
                error_code: Some("thread_resume_failed".to_string()),
                details: Some(json!({"thread_id": "thread-alpha"})),
            }),
            ..AgentTurnOutput::default()
        },
        AgentTurnOutput {
            final_assistant_text: Some("Fresh thread answer".to_string()),
            ..AgentTurnOutput::default()
        },
    ]);
    let requests = backend.requests();
    let service = WorkspaceConversationService::new_with_agent_turn_backend(
        settings.clone(),
        Arc::new(backend),
    );
    let repository = spark_storage::ConversationRepository::new(&settings.data_dir);

    service
        .execute_turn(
            "conversation-runtime-session",
            ConversationTurnRequest {
                project_path: "/projects/runtime-session".to_string(),
                message: "Establish the thread".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("first turn");
    let first_assistant_turn_id = requests.lock().expect("requests")[0].metadata
        ["spark.workspace.assistant_turn_id"]
        .as_str()
        .expect("assistant turn id")
        .to_string();
    let session = repository
        .read_runtime_session(
            "conversation-runtime-session",
            Some("/projects/runtime-session"),
        )
        .expect("read session")
        .expect("session written after thread id observed");
    assert_eq!(session.provider, "codex_app_server");
    assert_eq!(session.thread_id.as_deref(), Some("thread-alpha"));
    assert_eq!(
        session.last_turn_id.as_deref(),
        Some(first_assistant_turn_id.as_str())
    );
    assert!(!session.resume_failed);
    assert!(!session.established_at.is_empty());

    service
        .execute_turn(
            "conversation-runtime-session",
            ConversationTurnRequest {
                project_path: "/projects/runtime-session".to_string(),
                message: "Resume the previous thread".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("resume failure turn");
    assert_eq!(
        requests.lock().expect("requests")[1].metadata["spark.runtime.codex_app_server.thread_id"],
        json!("thread-alpha")
    );
    let tombstoned = repository
        .read_runtime_session(
            "conversation-runtime-session",
            Some("/projects/runtime-session"),
        )
        .expect("read tombstone")
        .expect("session still present");
    assert!(tombstoned.resume_failed);
    assert_eq!(
        tombstoned.thread_id.as_deref(),
        Some("thread-alpha"),
        "failed thread id is kept for debugging"
    );

    service
        .execute_turn(
            "conversation-runtime-session",
            ConversationTurnRequest {
                project_path: "/projects/runtime-session".to_string(),
                message: "Start over".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("fresh thread turn");
    assert!(
        !requests.lock().expect("requests")[2]
            .metadata
            .contains_key("spark.runtime.codex_app_server.thread_id"),
        "tombstoned continuity must not resume the failed thread"
    );
}

#[test]
fn runtime_session_absent_falls_back_to_transcript_turn_scan() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let backend = ScriptedAgentTurnBackend::new(vec![
        AgentTurnOutput {
            app_thread_id: Some("thread-legacy".to_string()),
            final_assistant_text: Some("First answer".to_string()),
            ..AgentTurnOutput::default()
        },
        AgentTurnOutput {
            final_assistant_text: Some("Second answer".to_string()),
            ..AgentTurnOutput::default()
        },
    ]);
    let requests = backend.requests();
    let service = WorkspaceConversationService::new_with_agent_turn_backend(
        settings.clone(),
        Arc::new(backend),
    );
    let repository = spark_storage::ConversationRepository::new(&settings.data_dir);

    service
        .execute_turn(
            "conversation-legacy-continuity",
            ConversationTurnRequest {
                project_path: "/projects/runtime-session".to_string(),
                message: "Establish the thread".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("first turn");

    // Conversations that predate the runtime-session sidecar have only the
    // transcript turn stamps; continuity must still resume via the turn scan.
    let session_path = repository
        .conversation_session_path(
            "conversation-legacy-continuity",
            Some("/projects/runtime-session"),
        )
        .expect("session path")
        .expect("conversation root");
    assert!(session_path.exists());
    fs::remove_file(&session_path).expect("remove session sidecar");

    service
        .execute_turn(
            "conversation-legacy-continuity",
            ConversationTurnRequest {
                project_path: "/projects/runtime-session".to_string(),
                message: "Continue without a sidecar".to_string(),
                ..ConversationTurnRequest::default()
            },
        )
        .expect("second turn");
    assert_eq!(
        requests.lock().expect("requests")[1].metadata["spark.runtime.codex_app_server.thread_id"],
        json!("thread-legacy")
    );

    // The successful turn scan self-heals: the sidecar is materialized so the
    // next continuity read never touches the transcript.
    let healed = repository
        .read_runtime_session(
            "conversation-legacy-continuity",
            Some("/projects/runtime-session"),
        )
        .expect("read healed session")
        .expect("healed session");
    assert_eq!(healed.thread_id.as_deref(), Some("thread-legacy"));
    assert!(!healed.resume_failed);
}
