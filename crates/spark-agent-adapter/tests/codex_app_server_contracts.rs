use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{json, Value};
use spark_agent_adapter::{
    build_codex_runtime_environment, parse_jsonrpc_line, process_codex_app_server_message,
    CodexAppServerClient, CodexAppServerTurnState,
};
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
    assert!(turn_start["params"].get("collaborationMode").is_none());

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

fn fake_codex_app_server_bin() -> &'static str {
    env!("CARGO_BIN_EXE_spark-agent-fake-codex-app-server")
}
