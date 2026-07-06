use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};
use spark_agent_adapter::{
    build_codex_runtime_environment, parse_jsonrpc_line, process_codex_app_server_message,
    AgentRequestUserInputAnswerRequest, AgentTurnRequest, CodexAppServerBackend,
    CodexAppServerClient, CodexAppServerTurnState,
};
use spark_common::debug::{CODEX_JSONRPC_TRACE_PATH_METADATA_KEY, ENV_SPARK_DEBUG_CODEX_JSONRPC};
use spark_common::events::{TurnStreamChannel, TurnStreamEventKind};

static CODEX_APP_SERVER_TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

const TURN_START_PARAMS_SCHEMA: &str = r#"{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "TurnStartParams",
  "type": "object",
  "required": ["input", "threadId"],
  "properties": {
    "threadId": {"type": "string"},
    "input": {"type": "array", "items": {"type": "object"}},
    "approvalPolicy": {"type": ["string", "object", "null"]},
    "sandboxPolicy": true,
    "cwd": {"type": ["string", "null"]},
    "model": {"type": ["string", "null"]},
    "effort": {"type": ["string", "null"]},
    "collaborationMode": true,
    "summary": true,
    "serviceTier": {"type": ["string", "null"]},
    "clientUserMessageId": {"type": ["string", "null"]},
    "personality": true,
    "approvalsReviewer": {"type": ["string", "null"]},
    "outputSchema": true
  }
}"#;

const MODEL_LIST_PARAMS_SCHEMA: &str = r#"{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "ModelListParams",
  "type": "object",
  "properties": {
    "cursor": {"type": ["string", "null"]},
    "includeHidden": {"type": ["boolean", "null"]},
    "limit": {"type": ["integer", "null"], "minimum": 0}
  }
}"#;

const MODEL_LIST_RESPONSE_SCHEMA: &str = r#"{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "ModelListResponse",
  "type": "object",
  "required": ["data"],
  "properties": {
    "data": {
      "type": "array",
      "items": {
        "type": "object",
        "required": [
          "defaultReasoningEffort",
          "description",
          "displayName",
          "hidden",
          "id",
          "isDefault",
          "model",
          "supportedReasoningEfforts"
        ],
        "properties": {
          "defaultReasoningEffort": {"type": "string", "minLength": 1},
          "description": {"type": "string"},
          "displayName": {"type": "string"},
          "hidden": {"type": "boolean"},
          "id": {"type": "string"},
          "isDefault": {"type": "boolean"},
          "model": {"type": "string"},
          "supportedReasoningEfforts": {"type": "array"}
        }
      }
    },
    "nextCursor": {"type": ["string", "null"]}
  }
}"#;

const TOOL_REQUEST_USER_INPUT_RESPONSE_SCHEMA: &str = r#"{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "ToolRequestUserInputResponse",
  "type": "object",
  "required": ["answers"],
  "properties": {
    "answers": {
      "type": "object",
      "additionalProperties": {
        "type": "object",
        "required": ["answers"],
        "properties": {
          "answers": {"type": "array", "items": {"type": "string"}}
        }
      }
    }
  }
}"#;

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
fn jsonrpc_line_parser_accepts_objects_and_ignores_malformed_lines() {
    assert_eq!(
        parse_jsonrpc_line(r#"{"id":1,"result":{}}"#),
        Some(json!({"id": 1, "result": {}}))
    );
    assert_eq!(parse_jsonrpc_line("not json"), None);
    assert_eq!(parse_jsonrpc_line("[1,2,3]"), None);
}

#[test]
fn initialize_and_turn_payload_contracts_match_codex_app_server_schema() {
    let initialize = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "clientInfo": {"name": "spark", "version": "0.1"},
            "capabilities": {"experimentalApi": true}
        }
    });
    assert_eq!(initialize["params"]["clientInfo"]["name"], json!("spark"));
    assert_eq!(
        initialize["params"]["capabilities"]["experimentalApi"],
        json!(true)
    );

    let turn_start = json!({
        "method": "turn/start",
        "params": {
            "threadId": "thread-1",
            "input": [{"type": "text", "text": "hello"}],
            "approvalPolicy": "never",
            "sandboxPolicy": {"type": "dangerFullAccess"},
            "cwd": "/repo",
            "model": "gpt-test",
            "collaborationMode": {
                "mode": "plan",
                "settings": {"model": "gpt-test"}
            },
            "effort": "high"
        }
    });
    assert_schema_valid(TURN_START_PARAMS_SCHEMA, &turn_start["params"]);
    assert_only_schema_declared_keys(TURN_START_PARAMS_SCHEMA, &turn_start["params"]);
    assert_eq!(turn_start["params"]["input"][0]["type"], json!("text"));
    assert_eq!(
        turn_start["params"]["sandboxPolicy"]["type"],
        json!("dangerFullAccess")
    );
    assert!(turn_start["params"].get("reasoningEffort").is_none());
    assert_eq!(
        turn_start["params"]["collaborationMode"]["mode"],
        json!("plan")
    );

    let turn_steer = json!({
        "method": "turn/steer",
        "params": {
            "threadId": "thread-1",
            "expectedTurnId": "turn-1",
            "input": [{"type": "text", "text": "adjust"}]
        }
    });
    assert_eq!(turn_steer["params"]["expectedTurnId"], json!("turn-1"));
}

#[test]
fn model_list_is_supported_and_matches_generated_schema_shape() {
    let _lock = CODEX_APP_SERVER_TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let _bin_guard = EnvVarGuard::set("SPARK_CODEX_APP_SERVER_BIN", fake_codex_app_server_bin());
    let _mode_guard = EnvVarGuard::set("SPARK_FAKE_CODEX_APP_SERVER_MODE", "model-list");
    let _runtime_guard = EnvVarGuard::set(
        "ATTRACTOR_CODEX_RUNTIME_ROOT",
        temp.path().join("codex-runtime"),
    );

    assert_schema_valid(MODEL_LIST_PARAMS_SCHEMA, &json!({"limit": 100}));
    let mut client = CodexAppServerClient::connect(temp.path().to_path_buf()).expect("connect");
    let models = client.list_models().expect("model/list");

    assert_schema_valid(MODEL_LIST_RESPONSE_SCHEMA, &models);
    assert_eq!(models["data"][0]["id"], json!("gpt-codex-test"));
}

#[test]
fn plan_mode_turn_uses_collaboration_mode_and_resolves_default_model() {
    let _lock = CODEX_APP_SERVER_TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("codex-rpc.jsonl");
    let _bin_guard = EnvVarGuard::set("SPARK_CODEX_APP_SERVER_BIN", fake_codex_app_server_bin());
    let _mode_guard = EnvVarGuard::set("SPARK_FAKE_CODEX_APP_SERVER_MODE", "default");
    let _log_guard = EnvVarGuard::set("SPARK_FAKE_CODEX_APP_SERVER_LOG", log_path.as_os_str());
    let _runtime_guard = EnvVarGuard::set(
        "ATTRACTOR_CODEX_RUNTIME_ROOT",
        temp.path().join("codex-runtime"),
    );

    let mut client = CodexAppServerClient::connect(temp.path().to_path_buf()).expect("connect");
    let thread_id = client
        .start_thread(None, Some(temp.path().to_string_lossy().as_ref()), true)
        .expect("thread/start");
    let result = client
        .run_turn(
            &thread_id,
            "Plan this",
            None,
            None,
            Some("plan"),
            Some(temp.path().to_string_lossy().as_ref()),
            None,
            None,
        )
        .expect("turn/start");

    assert_eq!(result.state.resolved_agent_text(), "Ack");
    let messages = fs::read_to_string(&log_path)
        .expect("rpc log")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("json"))
        .collect::<Vec<_>>();
    let methods = messages
        .iter()
        .filter_map(|message| message["method"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        methods,
        [
            "initialize",
            "initialized",
            "thread/start",
            "model/list",
            "turn/start"
        ]
    );
    let turn_start = messages
        .iter()
        .find(|message| message["method"] == json!("turn/start"))
        .expect("turn/start payload");
    assert_eq!(turn_start["params"]["model"], json!("gpt-codex-test"));
    assert_eq!(
        turn_start["params"]["collaborationMode"],
        json!({
            "mode": "plan",
            "settings": {"model": "gpt-codex-test"}
        })
    );
}

#[test]
fn codex_app_server_trace_file_is_debug_only_and_uses_jsonl_records() {
    let _lock = CODEX_APP_SERVER_TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let _bin_guard = EnvVarGuard::set("SPARK_CODEX_APP_SERVER_BIN", fake_codex_app_server_bin());
    let _mode_guard = EnvVarGuard::set("SPARK_FAKE_CODEX_APP_SERVER_MODE", "default");
    let _runtime_guard = EnvVarGuard::set(
        "ATTRACTOR_CODEX_RUNTIME_ROOT",
        temp.path().join("codex-runtime"),
    );
    let trace_path = temp.path().join("codex-jsonrpc-trace.jsonl");
    let mut metadata = std::collections::BTreeMap::new();
    metadata.insert(
        CODEX_JSONRPC_TRACE_PATH_METADATA_KEY.to_string(),
        json!(trace_path.to_string_lossy().to_string()),
    );
    let request = AgentTurnRequest {
        conversation_id: "conversation-trace".to_string(),
        project_path: temp.path().to_string_lossy().to_string(),
        prompt: "Trace this".to_string(),
        history: Vec::new(),
        provider: Some("codex".to_string()),
        model: Some("gpt-codex-test".to_string()),
        llm_profile: None,
        reasoning_effort: None,
        chat_mode: Some("agent".to_string()),
        metadata,
    };

    let _debug_guard = EnvVarGuard::remove(ENV_SPARK_DEBUG_CODEX_JSONRPC);
    let output = CodexAppServerBackend::new()
        .run_agent_turn(request.clone())
        .expect("turn without debug");
    assert_eq!(output.final_assistant_text.as_deref(), Some("Ack"));
    assert!(!trace_path.exists());

    drop(_debug_guard);
    let _debug_guard = EnvVarGuard::set(ENV_SPARK_DEBUG_CODEX_JSONRPC, "1");
    let output = CodexAppServerBackend::new()
        .run_agent_turn(request)
        .expect("turn with debug");
    assert_eq!(output.final_assistant_text.as_deref(), Some("Ack"));
    let records = fs::read_to_string(&trace_path)
        .expect("trace")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("trace json"))
        .collect::<Vec<_>>();
    assert!(records.iter().any(|record| {
        record["direction"] == json!("outgoing")
            && record["line"]
                .as_str()
                .is_some_and(|line| line.contains(r#""method":"turn/start""#))
            && record["timestamp"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
    }));
    assert!(records.iter().any(|record| {
        record["direction"] == json!("incoming")
            && record["line"]
                .as_str()
                .is_some_and(|line| line.contains(r#""method":"turn/completed""#))
    }));
}

#[test]
fn request_user_input_response_shape_matches_generated_schema() {
    let response = json!({
        "answers": {
            "choice": {"answers": ["Inline card"]},
            "notes": {"answers": []}
        }
    });
    assert_schema_valid(TOOL_REQUEST_USER_INPUT_RESPONSE_SCHEMA, &response);
}

#[test]
fn app_server_request_user_input_blocks_until_backend_answer_is_submitted() {
    let _lock = CODEX_APP_SERVER_TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("codex-rpc.jsonl");
    let _bin_guard = EnvVarGuard::set("SPARK_CODEX_APP_SERVER_BIN", fake_codex_app_server_bin());
    let _mode_guard = EnvVarGuard::set("SPARK_FAKE_CODEX_APP_SERVER_MODE", "request-user-input");
    let _log_guard = EnvVarGuard::set("SPARK_FAKE_CODEX_APP_SERVER_LOG", log_path.as_os_str());
    let _runtime_guard = EnvVarGuard::set(
        "ATTRACTOR_CODEX_RUNTIME_ROOT",
        temp.path().join("codex-runtime"),
    );
    let project_path = temp.path().to_string_lossy().into_owned();
    let (event_sender, event_receiver) = mpsc::channel();
    let request = AgentTurnRequest {
        conversation_id: "conversation-input".to_string(),
        project_path: project_path.clone(),
        prompt: "Ask me".to_string(),
        history: Vec::new(),
        provider: Some("codex".to_string()),
        model: Some("gpt-codex-test".to_string()),
        llm_profile: None,
        reasoning_effort: None,
        chat_mode: Some("agent".to_string()),
        metadata: BTreeMap::new(),
    };

    let run_handle = thread::spawn(move || {
        CodexAppServerBackend::new().run_agent_turn_with_event_sink(
            request,
            Some(Arc::new(move |event| {
                if event.kind == TurnStreamEventKind::RequestUserInputRequested {
                    let _ = event_sender.send(event);
                }
            })),
        )
    });

    let request_event = event_receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("pending request event");
    assert_eq!(
        request_event.request_user_input.as_ref().unwrap()["questions"][0]["id"],
        json!("choice")
    );
    let messages_before_answer = read_jsonrpc_log(&log_path);
    assert!(
        !messages_before_answer
            .iter()
            .any(|message| message["id"] == json!("server-request-1")
                && message.get("result").is_some()),
        "requestUserInput should not be answered before user input"
    );

    let delivery = CodexAppServerBackend::new()
        .answer_request_user_input(AgentRequestUserInputAnswerRequest {
            conversation_id: "conversation-input".to_string(),
            project_path,
            request_id: "choice".to_string(),
            assistant_turn_id: "assistant-turn-1".to_string(),
            answers: BTreeMap::from([("choice".to_string(), "A".to_string())]),
            request_user_input: None,
            history: Vec::new(),
            provider: Some("codex".to_string()),
            model: Some("gpt-codex-test".to_string()),
            llm_profile: None,
            reasoning_effort: None,
            chat_mode: Some("agent".to_string()),
            metadata: BTreeMap::from([
                (
                    "spark.runtime.codex_app_server.thread_id".to_string(),
                    json!("thread-test"),
                ),
                (
                    "spark.runtime.codex_app_server.turn_id".to_string(),
                    json!("turn-test"),
                ),
            ]),
        })
        .expect("answer delivery");
    assert!(delivery.thread_resume_failure.is_none());
    assert!(delivery.events.iter().any(|event| {
        event.source.raw_kind.as_deref() == Some("request_user_input_answer_delivered")
    }));

    let output = run_handle
        .join()
        .expect("turn thread")
        .expect("turn output");
    assert_eq!(output.final_assistant_text.as_deref(), Some("Ack"));
    let messages_after_answer = read_jsonrpc_log(&log_path);
    let request_user_input_response = messages_after_answer
        .iter()
        .find(|message| {
            message["id"] == json!("server-request-1") && message.get("result").is_some()
        })
        .expect("request-user-input response");
    assert_eq!(
        request_user_input_response["result"],
        json!({"answers": {"choice": {"answers": ["A"]}}})
    );
}

#[test]
fn runtime_environment_prepends_first_party_tool_bin_to_path() {
    let _lock = CODEX_APP_SERVER_TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let original_path = format!(
        "/usr/local/bin{}{}",
        std::path::MAIN_SEPARATOR,
        "placeholder"
    );
    let _runtime_guard = EnvVarGuard::set(
        "ATTRACTOR_CODEX_RUNTIME_ROOT",
        temp.path().join("codex-runtime"),
    );
    let _seed_guard =
        EnvVarGuard::set("ATTRACTOR_CODEX_SEED_DIR", temp.path().join("missing-seed"));
    let _path_guard = EnvVarGuard::set("PATH", &original_path);

    let env = build_codex_runtime_environment().expect("runtime env");
    let path = env.get("PATH").expect("PATH");
    let entries = std::env::split_paths(path).collect::<Vec<_>>();
    let current_exe_parent = std::env::current_exe()
        .expect("current exe")
        .parent()
        .expect("current exe parent")
        .to_path_buf();

    assert_eq!(entries.first(), Some(&current_exe_parent));
    assert_eq!(entries.last(), Some(&PathBuf::from(original_path)));
}

#[test]
fn runtime_environment_uses_isolated_codex_home_and_seeds_from_host_home() {
    let _lock = CODEX_APP_SERVER_TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let runtime_root = temp.path().join("runtime-codex");
    let host_codex_home = temp.path().join("host-codex-home");
    fs::create_dir_all(&host_codex_home).expect("host codex home");
    fs::write(host_codex_home.join("auth.json"), r#"{"seed":true}"#).expect("seed auth");
    let _runtime_guard = EnvVarGuard::set("ATTRACTOR_CODEX_RUNTIME_ROOT", &runtime_root);
    let _seed_guard =
        EnvVarGuard::set("ATTRACTOR_CODEX_SEED_DIR", temp.path().join("missing-seed"));
    let _codex_home_guard = EnvVarGuard::set("CODEX_HOME", &host_codex_home);

    let env = build_codex_runtime_environment().expect("runtime env");
    let isolated_codex_home = runtime_root.join(".codex");

    assert_eq!(
        env.get("CODEX_HOME").map(PathBuf::from),
        Some(isolated_codex_home.clone())
    );
    assert_ne!(isolated_codex_home, host_codex_home);
    assert_eq!(
        fs::read_to_string(isolated_codex_home.join("auth.json")).expect("seeded auth"),
        r#"{"seed":true}"#
    );
}

#[test]
fn app_server_error_notification_reads_generated_error_message_shape() {
    let mut state = CodexAppServerTurnState::default();
    let events = process_codex_app_server_message(
        &json!({
            "method": "error",
            "params": {"error": {"message": "schema shaped failure"}}
        }),
        &mut state,
    );

    assert_eq!(state.turn_error.as_deref(), Some("schema shaped failure"));
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, TurnStreamEventKind::Error);
    assert_eq!(events[0].error.as_deref(), Some("schema shaped failure"));
}

#[test]
fn app_server_notifications_normalize_assistant_plan_reasoning_tool_usage_and_completion() {
    let mut state = CodexAppServerTurnState::default();
    let messages = [
        json!({"method": "item/agentMessage/delta", "params": {"turnId": "turn-1", "itemId": "msg-1", "delta": "Ack"}}),
        json!({"method": "item/plan/delta", "params": {"turnId": "turn-1", "itemId": "plan-1", "delta": "1. Patch\n"}}),
        json!({"method": "item/reasoning/summaryTextDelta", "params": {"turnId": "turn-1", "itemId": "reason-1", "summaryIndex": 0, "delta": "Thinking"}}),
        json!({"method": "item/commandExecution/outputDelta", "params": {"turnId": "turn-1", "itemId": "cmd-1", "delta": "ok\n"}}),
        json!({"method": "thread/tokenUsage/updated", "params": {"turnId": "turn-1", "tokenUsage": {"total": {"inputTokens": 2, "cachedInputTokens": 0, "outputTokens": 1, "reasoningOutputTokens": 1, "totalTokens": 3}}}}),
        json!({"method": "item/completed", "params": {"turnId": "turn-1", "item": {"type": "AgentMessage", "id": "msg-1", "content": [{"type": "Text", "text": "Ack"}], "phase": "final_answer"}}}),
        json!({"method": "turn/completed", "params": {"turn": {"id": "turn-1", "status": "completed"}}}),
    ];
    let events = messages
        .iter()
        .flat_map(|message| process_codex_app_server_message(message, &mut state))
        .collect::<Vec<_>>();

    assert_eq!(state.resolved_agent_text(), "Ack");
    assert_eq!(state.resolved_plan_text(), "1. Patch");
    assert_eq!(state.resolved_command_text(), "ok");
    assert_eq!(state.last_token_total, Some(3));
    assert!(events.iter().any(|event| {
        event.kind == TurnStreamEventKind::ContentDelta
            && event.channel == Some(TurnStreamChannel::Assistant)
            && event.content_delta.as_deref() == Some("Ack")
            && event.source.backend.as_deref() == Some("codex_app_server")
    }));
    assert!(events
        .iter()
        .any(|event| event.channel == Some(TurnStreamChannel::Plan)));
    assert!(events
        .iter()
        .any(|event| event.channel == Some(TurnStreamChannel::Reasoning)));
    assert!(events
        .iter()
        .any(|event| event.kind == TurnStreamEventKind::ToolCallUpdated));
    assert!(events
        .iter()
        .any(|event| event.kind == TurnStreamEventKind::TokenUsageUpdated));
    assert!(events
        .iter()
        .any(|event| event.kind == TurnStreamEventKind::TurnCompleted));
}

#[test]
fn app_server_tool_items_emit_frontend_renderable_tool_call_payloads() {
    let mut state = CodexAppServerTurnState::default();
    let events = recorded_tool_notification_messages()
        .iter()
        .flat_map(|message| process_codex_app_server_message(message, &mut state))
        .collect::<Vec<_>>();

    let started = events
        .iter()
        .find(|event| event.kind == TurnStreamEventKind::ToolCallStarted)
        .and_then(|event| event.tool_call.as_ref())
        .expect("started tool call");
    assert_eq!(started["id"], json!("cmd-1"));
    assert_eq!(started["kind"], json!("command_execution"));
    assert_eq!(started["status"], json!("running"));
    assert_eq!(started["title"], json!("Run command"));
    assert_eq!(started["command"], json!("cargo test"));
    assert_eq!(started["output"], Value::Null);

    let completed_command = events
        .iter()
        .filter(|event| event.kind == TurnStreamEventKind::ToolCallCompleted)
        .find_map(|event| {
            let tool_call = event.tool_call.as_ref()?;
            (tool_call["kind"] == "command_execution").then_some(tool_call)
        })
        .expect("completed command tool call");
    assert_eq!(completed_command["id"], json!("cmd-1"));
    assert_eq!(completed_command["status"], json!("completed"));
    assert_eq!(completed_command["title"], json!("Run command"));
    assert_eq!(completed_command["command"], json!("cargo test"));
    assert_eq!(completed_command["output"], json!("test result: ok\n"));

    let file_change = events
        .iter()
        .filter(|event| event.kind == TurnStreamEventKind::ToolCallCompleted)
        .find_map(|event| {
            let tool_call = event.tool_call.as_ref()?;
            (tool_call["kind"] == "file_change").then_some(tool_call)
        })
        .expect("file change tool call");
    assert_eq!(file_change["id"], json!("file-1"));
    assert_eq!(file_change["status"], json!("completed"));
    assert_eq!(file_change["title"], json!("Apply file changes"));
    assert_eq!(
        file_change["file_paths"],
        json!(["src/lib.rs", "tests/lib_contracts.rs"])
    );
}

#[test]
fn app_server_tool_approval_requests_emit_normalized_tool_call_payloads() {
    let mut state = CodexAppServerTurnState::default();
    let events = recorded_tool_notification_messages()
        .iter()
        .filter(|message| {
            message["method"]
                .as_str()
                .is_some_and(|method| method.ends_with("/requestApproval"))
        })
        .flat_map(|message| process_codex_app_server_message(message, &mut state))
        .collect::<Vec<_>>();

    let command = events[0].tool_call.as_ref().expect("command tool call");
    assert_eq!(command["id"], json!("cmd-approve"));
    assert_eq!(command["kind"], json!("command_execution"));
    assert_eq!(command["status"], json!("running"));
    assert_eq!(command["title"], json!("Run command"));
    assert_eq!(command["command"], json!("cargo fmt --all"));

    let file_change = events[1].tool_call.as_ref().expect("file tool call");
    assert_eq!(file_change["id"], json!("file-approve"));
    assert_eq!(file_change["kind"], json!("file_change"));
    assert_eq!(file_change["status"], json!("running"));
    assert_eq!(file_change["title"], json!("Apply file changes"));
    assert_eq!(file_change["file_paths"], json!(["Cargo.toml"]));
}

#[test]
fn fake_app_server_tool_trace_emits_normalized_frontend_renderable_tool_events() {
    let _lock = CODEX_APP_SERVER_TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture_path = recorded_tool_notification_fixture_path();
    let _bin_guard = EnvVarGuard::set("SPARK_CODEX_APP_SERVER_BIN", fake_codex_app_server_bin());
    let _mode_guard = EnvVarGuard::set("SPARK_FAKE_CODEX_APP_SERVER_MODE", "tool-calls");
    let _trace_guard = EnvVarGuard::set(
        "SPARK_FAKE_CODEX_APP_SERVER_TOOL_TRACE",
        fixture_path.as_os_str(),
    );
    let _runtime_guard = EnvVarGuard::set(
        "ATTRACTOR_CODEX_RUNTIME_ROOT",
        temp.path().join("codex-runtime"),
    );
    let output = CodexAppServerBackend::new()
        .run_agent_turn(AgentTurnRequest {
            conversation_id: "conversation-tool-trace".to_string(),
            project_path: temp.path().to_string_lossy().into_owned(),
            prompt: "Run the trace".to_string(),
            history: Vec::new(),
            provider: Some("codex".to_string()),
            model: Some("gpt-codex-test".to_string()),
            llm_profile: None,
            reasoning_effort: None,
            chat_mode: Some("agent".to_string()),
            metadata: BTreeMap::new(),
        })
        .expect("tool trace turn");

    let tool_calls = output
        .events
        .iter()
        .filter_map(|event| event.tool_call.as_ref())
        .collect::<Vec<_>>();
    assert!(tool_calls.iter().any(|tool_call| {
        tool_call["id"] == json!("cmd-1")
            && tool_call["kind"] == json!("command_execution")
            && tool_call["title"] == json!("Run command")
            && tool_call["command"] == json!("cargo test")
            && tool_call["output"] == json!("test result: ok\n")
    }));
    assert!(tool_calls.iter().any(|tool_call| {
        tool_call["id"] == json!("cmd-approve")
            && tool_call["kind"] == json!("command_execution")
            && tool_call["title"] == json!("Run command")
            && tool_call["command"] == json!("cargo fmt --all")
    }));
    assert!(tool_calls.iter().any(|tool_call| {
        tool_call["id"] == json!("file-1")
            && tool_call["kind"] == json!("file_change")
            && tool_call["title"] == json!("Apply file changes")
            && tool_call["file_paths"] == json!(["src/lib.rs", "tests/lib_contracts.rs"])
    }));
    assert!(tool_calls.iter().any(|tool_call| {
        tool_call["id"] == json!("file-approve")
            && tool_call["kind"] == json!("file_change")
            && tool_call["title"] == json!("Apply file changes")
            && tool_call["file_paths"] == json!(["Cargo.toml"])
    }));
}

#[test]
fn app_server_notifications_preserve_stream_delta_whitespace() {
    let mut state = CodexAppServerTurnState::default();
    let messages = [
        json!({"method": "item/agentMessage/delta", "params": {"turnId": "turn-1", "itemId": "msg-1", "delta": "Hello "}}),
        json!({"method": "item/agentMessage/delta", "params": {"turnId": "turn-1", "itemId": "msg-1", "delta": " world"}}),
        json!({"method": "item/agentMessage/delta", "params": {"turnId": "turn-1", "itemId": "msg-1", "delta": " "}}),
        json!({"method": "item/plan/delta", "params": {"turnId": "turn-1", "itemId": "plan-1", "delta": "Plan step \n"}}),
        json!({"method": "item/reasoning/summaryTextDelta", "params": {"turnId": "turn-1", "itemId": "reason-1", "summaryIndex": 0, "delta": "Thinking "}}),
        json!({"method": "item/reasoning/summaryTextDelta", "params": {"turnId": "turn-1", "itemId": "reason-1", "summaryIndex": 0, "delta": " more"}}),
        json!({"method": "item/commandExecution/outputDelta", "params": {"turnId": "turn-1", "itemId": "cmd-1", "delta": "ok \n"}}),
    ];
    let events = messages
        .iter()
        .flat_map(|message| process_codex_app_server_message(message, &mut state))
        .collect::<Vec<_>>();

    assert_eq!(state.agent_chunks, ["Hello ", " world", " "]);
    assert_eq!(state.plan_chunks, ["Plan step \n"]);
    assert_eq!(state.command_chunks, ["ok \n"]);

    let assistant_deltas = events
        .iter()
        .filter(|event| {
            event.kind == TurnStreamEventKind::ContentDelta
                && event.channel == Some(TurnStreamChannel::Assistant)
        })
        .map(|event| event.content_delta.as_deref().unwrap_or(""))
        .collect::<Vec<_>>();
    assert_eq!(assistant_deltas, ["Hello ", " world", " "]);

    let reasoning_deltas = events
        .iter()
        .filter(|event| {
            event.kind == TurnStreamEventKind::ContentDelta
                && event.channel == Some(TurnStreamChannel::Reasoning)
        })
        .map(|event| event.content_delta.as_deref().unwrap_or(""))
        .collect::<Vec<_>>();
    assert_eq!(reasoning_deltas, ["Thinking ", " more"]);

    let command_delta = events
        .iter()
        .find(|event| event.kind == TurnStreamEventKind::ToolCallUpdated)
        .and_then(|event| event.content_delta.as_deref());
    assert_eq!(command_delta, Some("ok \n"));
}

#[test]
fn app_server_request_user_input_notification_preserves_payload() {
    let mut state = CodexAppServerTurnState::default();
    let events = process_codex_app_server_message(
        &json!({
            "method": "item/tool/requestUserInput",
            "params": {
                "itemId": "input-1",
                "questions": [{"id": "choice", "question": "Pick one"}]
            }
        }),
        &mut state,
    );

    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].kind,
        TurnStreamEventKind::RequestUserInputRequested
    );
    assert_eq!(events[0].source.item_id.as_deref(), Some("input-1"));
    assert_eq!(
        events[0]
            .request_user_input
            .as_ref()
            .and_then(Value::as_object)
            .unwrap()["questions"][0]["id"],
        json!("choice")
    );
}

fn recorded_tool_notification_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../test-fixtures/compat/agent/codex-app-server-tool-notifications.jsonl")
}

fn recorded_tool_notification_messages() -> Vec<Value> {
    fs::read_to_string(recorded_tool_notification_fixture_path())
        .expect("tool notification fixture")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("tool notification json"))
        .collect()
}

fn assert_schema_valid(schema: &str, instance: &Value) {
    let schema = serde_json::from_str::<Value>(schema).expect("schema json");
    let validator = jsonschema::validator_for(&schema).expect("schema validator");
    assert!(
        validator.is_valid(instance),
        "instance did not match schema: {instance}"
    );
}

fn assert_only_schema_declared_keys(schema: &str, instance: &Value) {
    let schema = serde_json::from_str::<Value>(schema).expect("schema json");
    let declared = schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("schema properties");
    let instance = instance.as_object().expect("instance object");
    let unknown = instance
        .keys()
        .filter(|key| !declared.contains_key(*key))
        .cloned()
        .collect::<Vec<_>>();
    assert!(unknown.is_empty(), "unknown schema keys: {unknown:?}");
}

fn read_jsonrpc_log(path: &std::path::Path) -> Vec<Value> {
    fs::read_to_string(path)
        .expect("rpc log")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("json"))
        .collect()
}

fn fake_codex_app_server_bin() -> &'static str {
    env!("CARGO_BIN_EXE_spark-agent-fake-codex-app-server")
}
