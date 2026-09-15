use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
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

use super::test_support::ENV_LOCK;

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
    let _lock = ENV_LOCK.lock().expect("env lock");
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
    let _lock = ENV_LOCK.lock().expect("env lock");
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
    let _lock = ENV_LOCK.lock().expect("env lock");
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
    let _lock = ENV_LOCK.lock().expect("env lock");
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
    let _lock = ENV_LOCK.lock().expect("env lock");
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
fn runtime_environment_never_overwrites_an_existing_runtime_auth_file() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let runtime_root = temp.path().join("runtime-codex");
    let isolated_codex_home = runtime_root.join(".codex");
    let host_codex_home = temp.path().join("host-codex-home");
    fs::create_dir_all(&host_codex_home).expect("host codex home");
    fs::create_dir_all(&isolated_codex_home).expect("runtime codex home");
    // A login (including an existing stale login) is replaced only by Codex's
    // managed sign-in flow, never by host credentials or startup migration.
    fs::write(
        isolated_codex_home.join("auth.json"),
        r#"{"owner":"runtime"}"#,
    )
    .expect("runtime auth");
    fs::write(host_codex_home.join("auth.json"), r#"{"owner":"host"}"#).expect("host auth");
    fs::write(
        host_codex_home.join("config.toml"),
        "model = \"gpt-5.6-sol\"\n",
    )
    .expect("host config");
    let _runtime_guard = EnvVarGuard::set("ATTRACTOR_CODEX_RUNTIME_ROOT", &runtime_root);
    let _seed_guard =
        EnvVarGuard::set("ATTRACTOR_CODEX_SEED_DIR", temp.path().join("missing-seed"));
    let _codex_home_guard = EnvVarGuard::set("CODEX_HOME", &host_codex_home);

    build_codex_runtime_environment().expect("runtime env");

    assert_eq!(
        fs::read_to_string(isolated_codex_home.join("auth.json")).expect("runtime auth"),
        r#"{"owner":"runtime"}"#
    );
    // Non-credential config is still seeded.
    assert!(isolated_codex_home.join("config.toml").is_file());
}

#[test]
fn runtime_environment_uses_isolated_codex_home_and_seeds_from_host_home() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let runtime_root = temp.path().join("runtime-codex");
    let host_codex_home = temp.path().join("host-codex-home");
    fs::create_dir_all(&host_codex_home).expect("host codex home");
    fs::write(host_codex_home.join("auth.json"), r#"{"seed":true}"#).expect("seed auth");
    fs::write(
        host_codex_home.join("config.toml"),
        "model = \"gpt-5.6-sol\"\nservice_tier = \"fast\"\n",
    )
    .expect("seed config");
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
    assert!(!isolated_codex_home.join("auth.json").exists());
    assert_eq!(
        fs::read_to_string(isolated_codex_home.join("config.toml")).expect("seeded config"),
        "model = \"gpt-5.6-sol\"\nservice_tier = \"standard\"\n"
    );
    assert_eq!(
        fs::read_to_string(host_codex_home.join("config.toml")).expect("host config"),
        "model = \"gpt-5.6-sol\"\nservice_tier = \"fast\"\n"
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
fn app_server_completed_reasoning_items_emit_reasoning_completions_per_summary_part() {
    let mut state = CodexAppServerTurnState::default();
    let messages = [
        json!({"method": "item/reasoning/summaryTextDelta", "params": {"turnId": "turn-1", "itemId": "reason-1", "summaryIndex": 0, "delta": "**Inspecting the repo**"}}),
        json!({"method": "item/completed", "params": {"turnId": "turn-1", "item": {
            "type": "reasoning",
            "id": "reason-1",
            "summary": [
                {"type": "summary_text", "text": "**Inspecting the repo**\n\nLooked at the build files."},
                {"type": "summary_text", "text": "**Choosing a fix**\n\nSmallest coherent change wins."}
            ]
        }}}),
        json!({"method": "turn/completed", "params": {"turn": {"id": "turn-1", "status": "completed"}}}),
    ];
    let events = messages
        .iter()
        .flat_map(|message| process_codex_app_server_message(message, &mut state))
        .collect::<Vec<_>>();

    let completions = events
        .iter()
        .filter(|event| {
            event.kind == TurnStreamEventKind::ContentCompleted
                && event.channel == Some(TurnStreamChannel::Reasoning)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        completions.len(),
        2,
        "one completion per summary part: {events:?}"
    );
    assert_eq!(
        completions[0].content_delta.as_deref(),
        Some("**Inspecting the repo**\n\nLooked at the build files.")
    );
    assert_eq!(completions[0].source.item_id.as_deref(), Some("reason-1"));
    assert_eq!(completions[0].source.summary_index, Some(0));
    assert_eq!(
        completions[1].content_delta.as_deref(),
        Some("**Choosing a fix**\n\nSmallest coherent change wins.")
    );
    assert_eq!(completions[1].source.summary_index, Some(1));
}

#[test]
fn app_server_completed_reasoning_items_without_summary_emit_nothing() {
    let mut state = CodexAppServerTurnState::default();
    let message = json!({"method": "item/completed", "params": {"turnId": "turn-1", "item": {
        "type": "reasoning",
        "id": "reason-2",
        "summary": [],
        "encrypted_content": "opaque"
    }}});
    let events = process_codex_app_server_message(&message, &mut state);
    assert!(
        events.iter().all(|event| {
            !(event.kind == TurnStreamEventKind::ContentCompleted
                && event.channel == Some(TurnStreamChannel::Reasoning))
        }),
        "{events:?}"
    );
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
    let _lock = ENV_LOCK.lock().expect("env lock");
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

#[test]
fn codex_managed_login_reconnects_without_cloning_or_erasing_runtime_state() {
    use spark_agent_adapter::codex_app_server::auth::{CodexConnection, CodexLoginMethod};
    use spark_common::agent_settings::NativeAgentSettings;
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("CODEX_HOME", temp.path().join("host"));
    let log = temp.path().join("rpc.jsonl");
    let _log = EnvVarGuard::set("SPARK_FAKE_CODEX_APP_SERVER_LOG", &log);
    let _mode = EnvVarGuard::set("SPARK_FAKE_CODEX_APP_SERVER_MODE", "auth-expired");
    let native = NativeAgentSettings {
        codex_binary: Some(fake_codex_app_server_bin().into()),
        codex_runtime_root: Some(temp.path().join("runtime").to_string_lossy().into_owned()),
        codex_seed_dir: Some(temp.path().join("seed").to_string_lossy().into_owned()),
        ..Default::default()
    };
    let runtime_home = temp.path().join("runtime/.codex");
    fs::create_dir_all(runtime_home.join("sessions")).unwrap();
    fs::create_dir_all(temp.path().join("host")).unwrap();
    fs::create_dir_all(temp.path().join("seed")).unwrap();
    fs::write(temp.path().join("host/auth.json"), "host credentials").unwrap();
    fs::write(temp.path().join("seed/auth.json"), "seed credentials").unwrap();
    fs::write(runtime_home.join("auth.json"), "stale credentials").unwrap();
    fs::write(
        runtime_home.join("sessions/saved.jsonl"),
        "saved conversation",
    )
    .unwrap();
    let mut connection = CodexConnection::default();
    let expired = connection.status(temp.path(), &native).unwrap();
    assert_eq!(expired.status, "disconnected");
    assert!(expired.message.unwrap().contains("needs sign-in"));
    for method in [CodexLoginMethod::Browser, CodexLoginMethod::Device] {
        let started = connection
            .start_login(temp.path(), &native, method)
            .unwrap();
        assert_eq!(started.status, "pending");
        assert!(started
            .login_url
            .unwrap()
            .starts_with("https://auth.openai.com/"));
        assert_eq!(
            started.user_code.is_some(),
            matches!(method, CodexLoginMethod::Device)
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let status = connection.status(temp.path(), &native).unwrap();
            if status.status != "pending" {
                assert_eq!(status.status, "connected");
                assert_eq!(
                    status.account.as_ref().unwrap().email.as_deref(),
                    Some("spark@example.test")
                );
                assert!(!serde_json::to_string(&status)
                    .unwrap()
                    .contains("must-not-leak"));
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "login completion timed out"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
    assert_eq!(
        connection.status(temp.path(), &native).unwrap().status,
        "connected"
    );
    assert_eq!(
        fs::read_to_string(runtime_home.join("sessions/saved.jsonl")).unwrap(),
        "saved conversation"
    );
    assert_eq!(
        fs::read_to_string(runtime_home.join("auth.json")).unwrap(),
        "stale credentials"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("host/auth.json")).unwrap(),
        "host credentials"
    );
    let requests = read_jsonrpc_log(&log);
    assert!(requests
        .iter()
        .any(|request| request["method"] == "account/read"
            && request["params"]["refreshToken"] == true));
    assert!(!requests
        .iter()
        .any(|request| request["method"] == "account/logout"));
}

#[test]
fn codex_managed_login_handles_cancellation_decline_and_process_exit() {
    use spark_agent_adapter::codex_app_server::auth::{CodexConnection, CodexLoginMethod};
    use spark_common::agent_settings::NativeAgentSettings;
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("CODEX_HOME", temp.path().join("host"));
    let log = temp.path().join("rpc.jsonl");
    let _log = EnvVarGuard::set("SPARK_FAKE_CODEX_APP_SERVER_LOG", &log);
    let native = NativeAgentSettings {
        codex_binary: Some(fake_codex_app_server_bin().into()),
        codex_runtime_root: Some(temp.path().join("runtime").to_string_lossy().into_owned()),
        codex_seed_dir: Some(temp.path().join("seed").to_string_lossy().into_owned()),
        ..Default::default()
    };
    for mode in ["auth-pending", "auth-failure", "auth-exit"] {
        let _mode = EnvVarGuard::set("SPARK_FAKE_CODEX_APP_SERVER_MODE", mode);
        let mut connection = CodexConnection::default();
        connection
            .start_login(temp.path(), &native, CodexLoginMethod::Browser)
            .unwrap();
        if mode == "auth-pending" {
            // Duplicate clicks reuse the pending flow instead of competing for its callback port.
            connection
                .start_login(temp.path(), &native, CodexLoginMethod::Browser)
                .unwrap();
            assert_eq!(
                connection.status(temp.path(), &native).unwrap().status,
                "pending"
            );
            assert_eq!(connection.cancel_login().status, "disconnected");
            let requests = read_jsonrpc_log(&log);
            assert_eq!(
                requests
                    .iter()
                    .filter(|request| request["method"] == "account/login/start")
                    .count(),
                1
            );
            assert!(requests
                .iter()
                .any(|request| request["method"] == "account/login/cancel"));
        } else {
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            loop {
                let status = connection.status(temp.path(), &native).unwrap();
                if status.status != "pending" {
                    assert_eq!(status.status, "disconnected");
                    assert!(status.message.unwrap().contains(if mode == "auth-failure" {
                        "declined"
                    } else {
                        "exited"
                    }));
                    break;
                }
                assert!(std::time::Instant::now() < deadline);
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

#[test]
fn codex_auth_failure_during_resume_does_not_invalidate_the_saved_thread() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("CODEX_HOME", temp.path().join("host"));
    let _bin = EnvVarGuard::set("SPARK_CODEX_APP_SERVER_BIN", fake_codex_app_server_bin());
    let _runtime = EnvVarGuard::set("ATTRACTOR_CODEX_RUNTIME_ROOT", temp.path().join("runtime"));
    let _mode = EnvVarGuard::set("SPARK_FAKE_CODEX_APP_SERVER_MODE", "auth-resume-error");
    let request = AgentTurnRequest {
        conversation_id: "saved-conversation".into(),
        project_path: temp.path().to_string_lossy().into_owned(),
        prompt: "Continue".into(),
        history: vec![],
        provider: Some("codex".into()),
        model: Some("gpt-codex-test".into()),
        llm_profile: None,
        reasoning_effort: None,
        chat_mode: Some("agent".into()),
        metadata: BTreeMap::from([(
            "spark.runtime.codex_app_server.thread_id".into(),
            json!("saved-thread"),
        )]),
    };
    let error = CodexAppServerBackend::new()
        .run_agent_turn(request.clone())
        .unwrap_err();
    assert!(error.message.contains("Codex connection needs sign-in"));
    drop(_mode);
    let _mode = EnvVarGuard::set(
        "SPARK_FAKE_CODEX_APP_SERVER_MODE",
        "auth-resume-unauthorized",
    );
    let error = CodexAppServerBackend::new()
        .run_agent_turn(request.clone())
        .unwrap_err();
    assert!(error.message.contains("Codex connection needs sign-in"));
    drop(_mode);
    let _mode = EnvVarGuard::set("SPARK_FAKE_CODEX_APP_SERVER_MODE", "default");
    let resumed = CodexAppServerBackend::new()
        .run_agent_turn(request)
        .unwrap();
    assert!(resumed.thread_resume_failure.is_none());
    assert_eq!(resumed.final_assistant_text.as_deref(), Some("Ack"));
    let mut state = CodexAppServerTurnState::default();
    let events = process_codex_app_server_message(
        &json!({"method": "error", "params": {"error": {"message": "Your refresh token has expired. Please sign in again."}}}),
        &mut state,
    );
    assert_eq!(events[0].error_code.as_deref(), Some("codex_auth_required"));
    assert!(
        !spark_agent_adapter::codex_app_server::auth::requires_login(
            "Failed to refresh token: connection timed out"
        )
    );
}

#[test]
fn model_list_parses_across_payload_dialects() {
    use spark_agent_adapter::codex_models_from_list_result;

    // The shape the current codex app-server emits: data entries with
    // camelCase fields and object-shaped reasoning efforts.
    let parsed = codex_models_from_list_result(&serde_json::json!({
        "data": [
            {
                "id": "gpt-5.5",
                "displayName": "GPT-5.5",
                "isDefault": true,
                "defaultReasoningEffort": "Medium",
                "supportedReasoningEfforts": [
                    {"reasoningEffort": "Low", "description": "Fast"},
                    {"reasoningEffort": "Medium", "description": "Balanced"},
                    {"reasoningEffort": "xhigh", "description": "Max"},
                ],
            },
            {"model": "gpt-5.5-mini", "label": "GPT-5.5 Mini"},
        ],
    }));
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].id, "gpt-5.5");
    assert_eq!(parsed[0].display, "GPT-5.5");
    assert!(parsed[0].is_default);
    assert_eq!(
        parsed[0].supported_reasoning_efforts,
        vec!["low", "medium", "xhigh"]
    );
    assert_eq!(
        parsed[0].default_reasoning_effort.as_deref(),
        Some("medium")
    );
    assert_eq!(parsed[1].id, "gpt-5.5-mini");
    assert_eq!(parsed[1].display, "GPT-5.5 Mini");
    assert!(!parsed[1].is_default);

    // Snake_case under "models" with string efforts and nested reasoning.
    let parsed = codex_models_from_list_result(&serde_json::json!({
        "models": [{
            "name": "gpt-legacy",
            "display_name": "Legacy",
            "is_default": true,
            "reasoning": {"supported_efforts": ["low", "high"], "default": "high"},
        }],
    }));
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].id, "gpt-legacy");
    assert_eq!(parsed[0].supported_reasoning_efforts, vec!["low", "high"]);
    assert_eq!(parsed[0].default_reasoning_effort.as_deref(), Some("high"));

    assert!(codex_models_from_list_result(&serde_json::json!({})).is_empty());
}

#[test]
fn list_available_codex_models_queries_the_local_app_server() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let _bin_guard = EnvVarGuard::set("SPARK_CODEX_APP_SERVER_BIN", fake_codex_app_server_bin());
    let models = spark_agent_adapter::list_available_codex_models().expect("model list");
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "gpt-codex-test");
    assert_eq!(models[0].display, "Codex Test");
    assert!(models[0].is_default);
    assert_eq!(models[0].supported_reasoning_efforts, vec!["medium"]);
    assert_eq!(
        models[0].default_reasoning_effort.as_deref(),
        Some("medium")
    );
}

#[test]
fn captured_native_configuration_controls_actual_codex_launch_and_runtime_home() {
    let _lock = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", temp.path());
    let _codex_home = EnvVarGuard::set("CODEX_HOME", temp.path().join("empty-codex-home"));
    let _binary = EnvVarGuard::set("SPARK_CODEX_APP_SERVER_BIN", "/missing/later-binary");
    let _runtime = EnvVarGuard::set(
        "ATTRACTOR_CODEX_RUNTIME_ROOT",
        temp.path().join("later-runtime"),
    );
    let captured = temp.path().join("captured-runtime");
    let request: AgentTurnRequest = serde_json::from_value(json!({
        "conversation_id": "captured-native", "project_path": temp.path(), "prompt": "Hello", "history": [], "provider": "codex", "model": "gpt-codex-test",
        "metadata": {"spark.execution.settings": {"configuration": {"agents": {"native": {
            "codex_binary": fake_codex_app_server_bin(), "codex_runtime_root": captured,
            "codex_seed_dir": temp.path().join("empty-seed"), "codex_jsonrpc_trace": false
        }}}}}
    })).unwrap();
    CodexAppServerBackend::new()
        .run_agent_turn(request)
        .unwrap();
    assert!(captured.join(".codex").is_dir());
    assert!(!temp.path().join("later-runtime").exists());
    let config = std::fs::read_to_string(captured.join(".codex/config.toml")).unwrap();
    assert!(config.contains("service_tier"));
    assert!(config
        .lines()
        .any(|line| line == "service_tier = \"standard\""));
}

#[test]
fn workflow_native_tracing_uses_saved_settings_and_retains_captured_choice() {
    use spark_agent_adapter::{CodergenHandler, CodergenRequest, RustLlmCodergenBackend};
    use spark_common::debug::CODEX_JSONRPC_TRACE_FILE_NAME;
    use spark_common::paths::ProcessEnvironment;
    use spark_storage::settings::read_execution_configuration;

    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().unwrap();
    let _bin = EnvVarGuard::set("SPARK_CODEX_APP_SERVER_BIN", fake_codex_app_server_bin());
    let _mode = EnvVarGuard::set("SPARK_FAKE_CODEX_APP_SERVER_MODE", "default");
    let _runtime = EnvVarGuard::set("ATTRACTOR_CODEX_RUNTIME_ROOT", temp.path().join("runtime"));
    let _seed = EnvVarGuard::set("ATTRACTOR_CODEX_SEED_DIR", temp.path().join("empty-seed"));
    let _debug = EnvVarGuard::remove(ENV_SPARK_DEBUG_CODEX_JSONRPC);
    let config_dir = temp.path().join("config");
    fs::create_dir_all(&config_dir).unwrap();
    let core = config_dir.join("spark.toml");
    let capture = || read_execution_configuration(&config_dir, &ProcessEnvironment).unwrap();
    let graph = attractor_core::FlowDefinition::from_yaml_str(
        "schema_version: '1'\nid: trace\ntitle: Trace\nnodes:\n  task:\n    kind: agent_task\n    config:\n      kind: agent_task\n      prompt: Say hello\n    execution:\n      llm_provider: codex\n      llm_model: gpt-codex-test\n",
    ).unwrap().to_runtime_dot_graph();
    let execute = |name: &str,
                   configuration: &spark_common::settings::ExecutionConfiguration,
                   expected: bool| {
        let logs = temp.path().join(name);
        let request: CodergenRequest = serde_json::from_value(json!({
            "node_id": "task", "node": graph.nodes["task"], "graph": graph,
            "logs_root": logs, "project_path": temp.path(),
            "metadata": {"spark.execution.settings": {"configuration": configuration}}
        }))
        .unwrap();
        let result = CodergenHandler::with_backend(RustLlmCodergenBackend::new(
            unified_llm_adapter::Client::new(),
        ))
        .execute(request)
        .expect("workflow native launch");
        assert_eq!(result.response_text, "Ack");
        let trace = logs.join("task").join(CODEX_JSONRPC_TRACE_FILE_NAME);
        assert_eq!(trace.exists(), expected, "{name}");
        if expected {
            let records: Vec<Value> = fs::read_to_string(trace)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert!(records
                .iter()
                .any(|record| record["direction"] == "outgoing"));
            assert!(records
                .iter()
                .any(|record| record["direction"] == "incoming"));
        }
    };

    execute("default-disabled", &capture(), false);
    fs::write(&core, "[agents.native]\ncodex_jsonrpc_trace = true\n").unwrap();
    let enabled = capture();
    execute("saved-enabled", &enabled, true);
    fs::write(&core, "[agents.native]\ncodex_jsonrpc_trace = false\n").unwrap();
    let disabled = capture();
    execute("active-enabled", &enabled, true);
    execute("new-disabled", &disabled, false);
    fs::write(&core, "[agents.native]\ncodex_jsonrpc_trace = true\n").unwrap();
    let _off = EnvVarGuard::set(ENV_SPARK_DEBUG_CODEX_JSONRPC, "0");
    execute("environment-disabled", &capture(), false);
    drop(_off);
    fs::write(&core, "[agents.native]\ncodex_jsonrpc_trace = false\n").unwrap();
    let _on = EnvVarGuard::set(ENV_SPARK_DEBUG_CODEX_JSONRPC, "1");
    execute("environment-enabled", &capture(), true);
    execute("active-disabled", &disabled, false);
}
