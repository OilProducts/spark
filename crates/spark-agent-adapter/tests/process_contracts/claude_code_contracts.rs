use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::json;
use spark_agent_adapter::{
    list_available_claude_code_models, AgentTurnBackend, AgentTurnRequest, ClaudeCodeBackend,
    ClaudeCodeModelMetadata, CodergenBackend, CodergenBackendRequest, CodergenBackendResponse,
    CodergenError, CodergenRuntimeMode, RustLlmAgentTurnBackend, RustLlmCodergenBackend,
};
use spark_common::events::TurnStreamEventKind;
use unified_llm_adapter::Client;

use super::test_support::ENV_LOCK;

struct EnvVarGuard {
    key: &'static str,
    previous: Option<String>,
}

#[test]
fn claude_code_chat_dispatch_resumes_then_retries_once_fresh() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("claude-args.log");
    let _bin_guard = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _mode_guard = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "resume-failure");
    let _log_guard = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_LOG", log_path.as_os_str());
    let mut request = agent_request(temp.path());
    request.metadata.insert(
        "spark.runtime.claude_code.session_id".to_string(),
        json!("sess-dead"),
    );

    let output = RustLlmAgentTurnBackend::new(Client::new())
        .run_turn(request)
        .expect("fresh retry succeeds");

    assert_eq!(
        output.app_thread_id.as_deref(),
        Some("sess-fake-claude-fresh")
    );
    assert_eq!(
        output
            .thread_resume_failure
            .as_ref()
            .and_then(|failure| failure.error_code.as_deref()),
        Some("thread_resume_failure")
    );
    let log = std::fs::read_to_string(log_path).expect("args log");
    assert_eq!(log.matches("-- invocation --").count(), 2);
    assert_eq!(log.matches("--resume").count(), 1);
    assert!(log.contains("sess-dead"));
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

fn fake_claude_code_bin() -> &'static str {
    env!("CARGO_BIN_EXE_spark-agent-fake-claude-code")
}

fn agent_request(project_path: &std::path::Path) -> AgentTurnRequest {
    AgentTurnRequest {
        conversation_id: "run-1:implement".to_string(),
        project_path: project_path.to_string_lossy().to_string(),
        prompt: "Prepare the change.".to_string(),
        history: Vec::new(),
        provider: Some("claude-code".to_string()),
        model: Some("claude-opus-4-8".to_string()),
        llm_profile: None,
        reasoning_effort: None,
        chat_mode: None,
        metadata: BTreeMap::new(),
    }
}

#[test]
fn claude_code_backend_maps_stream_json_to_turn_events_and_final_text() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("claude-args.log");
    let _bin_guard = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _mode_guard = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "default");
    let _log_guard = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_LOG", log_path.as_os_str());

    let output = ClaudeCodeBackend::new()
        .run_agent_turn(agent_request(temp.path()))
        .expect("turn");

    assert_eq!(output.final_assistant_text.as_deref(), Some("All set."));
    assert_eq!(output.app_thread_id.as_deref(), Some("sess-fake-claude-1"));
    let app_turn_id = output.app_turn_id.as_deref().expect("app turn id");
    assert!(uuid::Uuid::parse_str(app_turn_id).is_ok());
    assert!(output
        .events
        .iter()
        .all(|event| { event.source.app_turn_id.as_deref() == Some(app_turn_id) }));
    let kinds = output
        .events
        .iter()
        .map(|event| event.kind.as_str().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            "system_init",
            "content_delta",
            "content_completed",
            "content_delta",
            "content_completed",
            "tool_call_started",
            "tool_call_completed",
            "content_delta",
            "content_completed",
            "tool_call_started",
            "tool_call_completed",
            "content_delta",
            "content_completed",
            "content_completed",
            "token_usage_updated",
            "turn_completed",
        ],
    );
    let content_events = output
        .events
        .iter()
        .filter(|event| event.kind == TurnStreamEventKind::ContentCompleted)
        .collect::<Vec<_>>();
    assert_eq!(
        content_events
            .iter()
            .map(|event| event.source.item_id.as_deref())
            .collect::<Vec<_>>(),
        vec![
            Some("block-1"),
            Some("block-2"),
            Some("block-3"),
            Some("block-4"),
            Some("block-4")
        ]
    );
    assert_eq!(
        content_events.last().unwrap().phase.as_deref(),
        Some("final_answer")
    );
    assert_eq!(
        content_events.last().unwrap().content_delta.as_deref(),
        Some("All set.")
    );
    let deltas = output
        .events
        .iter()
        .filter(|event| event.kind == TurnStreamEventKind::ContentDelta)
        .collect::<Vec<_>>();
    for (delta, completed) in deltas.iter().zip(&content_events) {
        assert_eq!(delta.source.item_id, completed.source.item_id);
    }
    assert_eq!(deltas.len(), 4);
    assert_eq!(
        deltas.last().unwrap().source.item_id,
        content_events.last().unwrap().source.item_id
    );
    let tool_started = output
        .events
        .iter()
        .find(|event| event.kind == TurnStreamEventKind::ToolCallStarted)
        .expect("tool call event");
    assert_eq!(
        tool_started.tool_call.as_ref().unwrap()["name"],
        json!("Bash")
    );
    assert_eq!(
        output.token_usage.as_ref().unwrap()["input_tokens"],
        json!(12)
    );

    // The CLI was invoked headless with stream-json output and the model flag.
    let args = std::fs::read_to_string(&log_path).expect("args log");
    for expected in [
        "-p",
        "stream-json",
        "--include-partial-messages",
        "--verbose",
        "--permission-mode",
        "--model",
    ] {
        assert!(args.contains(expected), "missing {expected} in: {args}");
    }
}

#[test]
fn claude_code_tool_calls_emit_canonical_payloads_through_completion() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let _bin_guard = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _mode_guard = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "tool-payloads");

    let output = ClaudeCodeBackend::new()
        .run_agent_turn(agent_request(temp.path()))
        .expect("turn");
    let payload = |kind, id| {
        output
            .events
            .iter()
            .find(|event| {
                event.kind == kind
                    && event
                        .tool_call
                        .as_ref()
                        .and_then(|call| call["id"].as_str())
                        == Some(id)
            })
            .and_then(|event| event.tool_call.as_ref())
            .expect("tool payload")
    };

    let described = payload(TurnStreamEventKind::ToolCallStarted, "toolu_bash_described");
    assert_eq!(described["kind"], "command_execution");
    assert_eq!(described["command"], "cargo test");
    assert_eq!(described["title"], "Run the tests");
    let completed = payload(
        TurnStreamEventKind::ToolCallCompleted,
        "toolu_bash_described",
    );
    assert_eq!(completed["kind"], described["kind"]);
    assert_eq!(completed["command"], described["command"]);
    assert_eq!(completed["title"], described["title"]);
    assert_eq!(completed["output"], "tests passed");
    assert_eq!(completed["is_error"], false);

    assert_eq!(
        payload(TurnStreamEventKind::ToolCallStarted, "toolu_bash_plain")["title"],
        "printf first"
    );
    for kind in [
        TurnStreamEventKind::ToolCallStarted,
        TurnStreamEventKind::ToolCallCompleted,
    ] {
        assert_eq!(
            payload(kind, "toolu_read")["file_paths"],
            json!(["/tmp/example.rs"])
        );
    }
    let unknown = payload(TurnStreamEventKind::ToolCallCompleted, "toolu_unknown");
    assert_eq!(unknown["kind"], "dynamic_tool");
    assert_eq!(unknown["title"], "McpWidget");
    assert_eq!(unknown["output"], "done");
}

#[test]
fn claude_code_multi_block_completions_consume_partial_ids_in_order() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let _bin_guard = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _mode_guard = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "multi-block");

    let output = ClaudeCodeBackend::new()
        .run_agent_turn(agent_request(temp.path()))
        .expect("turn");
    let ids = output
        .events
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                TurnStreamEventKind::ContentDelta | TurnStreamEventKind::ContentCompleted
            )
        })
        .map(|event| event.source.item_id.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![
            Some("block-1"),
            Some("block-2"),
            Some("block-1"),
            Some("block-2"),
            Some("block-2")
        ]
    );
}

#[test]
fn claude_code_completions_mint_ids_without_partial_events() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let _bin_guard = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _mode_guard = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "no-partials");

    let output = ClaudeCodeBackend::new()
        .run_agent_turn(agent_request(temp.path()))
        .expect("turn");
    assert!(output
        .events
        .iter()
        .all(|event| event.kind != TurnStreamEventKind::ContentDelta));
    let ids = output
        .events
        .iter()
        .filter(|event| event.kind == TurnStreamEventKind::ContentCompleted)
        .map(|event| event.source.item_id.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![
            Some("block-1"),
            Some("block-2"),
            Some("block-3"),
            Some("block-4"),
            Some("block-4")
        ]
    );
}

#[test]
fn claude_code_result_only_and_empty_result_contracts() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let _bin_guard = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());

    let result_mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "result-only");
    let output = ClaudeCodeBackend::new()
        .run_agent_turn(agent_request(temp.path()))
        .expect("result-only turn");
    let final_event = output
        .events
        .iter()
        .find(|event| event.phase.as_deref() == Some("final_answer"))
        .expect("final answer event");
    assert_eq!(final_event.source.item_id.as_deref(), Some("block-1"));
    assert_eq!(
        final_event.content_delta.as_deref(),
        Some("Result without blocks.")
    );
    drop(result_mode);

    let _empty_mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "empty-result");
    let output = ClaudeCodeBackend::new()
        .run_agent_turn(agent_request(temp.path()))
        .expect("empty-result turn");
    assert!(output
        .events
        .iter()
        .all(|event| event.phase.as_deref() != Some("final_answer")));
    assert_eq!(output.final_assistant_text, None);
}

#[test]
fn claude_code_codergen_streams_prefix_and_reports_completed_event_with_usage() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let _bin_guard = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _mode_guard = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "default");

    let client = Client::new();
    let mut backend = RustLlmCodergenBackend::new(client);
    let streamed = Arc::new(Mutex::new(Vec::new()));
    let sink_events = Arc::clone(&streamed);
    let request = CodergenBackendRequest {
        node_id: "implement".to_string(),
        prompt: "Prepare the change.".to_string(),
        provider: "claude-code".to_string(),
        model: Some("claude-opus-4-8".to_string()),
        runtime_mode: CodergenRuntimeMode::agent(),
        project_path: Some(temp.path().to_path_buf()),
        ..CodergenBackendRequest::default()
    };
    let output = backend
        .run_with_event_sink(
            request,
            Some(Arc::new(move |event| {
                sink_events.lock().expect("sink lock").push(event);
            })),
        )
        .expect("codergen output");

    assert_eq!(
        output.response,
        CodergenBackendResponse::Text("All set.".to_string())
    );
    let usage = output.usage.expect("usage mapped");
    assert_eq!(usage.input_tokens, 12);
    assert_eq!(usage.output_tokens, 34);
    assert_eq!(usage.cache_read_tokens, Some(5));

    let event_types = output
        .events
        .iter()
        .map(|event| event.event_type.as_str())
        .collect::<Vec<_>>();
    assert!(event_types
        .iter()
        .all(|event_type| *event_type == "claude_code_session_event"
            || *event_type == "claude_code_request_completed"));
    assert_eq!(
        *event_types.last().expect("terminal event"),
        "claude_code_request_completed"
    );
    let completed = output.events.last().expect("completed event");
    assert_eq!(completed.payload["provider"], json!("claude-code"));
    assert_eq!(completed.payload["token_usage"]["output_tokens"], json!(34));

    // Live prefix contract: the sink saw exactly the leading events of the
    // returned batch, in order.
    let streamed = streamed.lock().expect("streamed lock");
    assert!(!streamed.is_empty());
    assert_eq!(streamed.len(), output.events.len() - 1);
    for (streamed_event, batch_event) in streamed.iter().zip(output.events.iter()) {
        assert_eq!(streamed_event, batch_event);
    }
}

#[test]
fn claude_code_model_discovery_queries_catalog_over_stdio_and_maps_default_to_blank() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("claude-args.log");
    let _bin_guard = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _log_guard = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_LOG", log_path.as_os_str());

    let models = list_available_claude_code_models().expect("model catalog");

    assert_eq!(
        models,
        vec![
            ClaudeCodeModelMetadata {
                id: String::new(),
                display: "Default (recommended)".to_string(),
            },
            ClaudeCodeModelMetadata {
                id: "claude-fable-5[1m]".to_string(),
                display: "Fable".to_string(),
            },
            ClaudeCodeModelMetadata {
                id: "sonnet".to_string(),
                display: "Sonnet".to_string(),
            },
        ],
    );
    let args = std::fs::read_to_string(&log_path).expect("args log");
    for expected in ["-p", "--input-format", "stream-json"] {
        assert!(args.contains(expected), "missing {expected} in: {args}");
    }
}

/// Opt-in smoke against the real Claude Code CLI and its login. Run with:
/// `cargo test -p spark-agent-adapter --test process_contracts real_claude_code -- --ignored`
#[test]
#[ignore = "requires an installed, logged-in claude CLI; consumes real usage"]
fn real_claude_code_cli_completes_a_text_only_turn() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let _mode_guard = EnvVarGuard::set("SPARK_CLAUDE_CODE_PERMISSION_MODE", "default");

    let mut request = agent_request(temp.path());
    request.model = None; // user's default model
    request.prompt =
        "Reply with exactly the text SPARK_SMOKE_OK and nothing else. Do not use any tools."
            .to_string();
    let output = ClaudeCodeBackend::new()
        .run_agent_turn(request)
        .expect("real claude turn");

    let text = output.final_assistant_text.expect("assistant text");
    assert!(
        text.contains("SPARK_SMOKE_OK"),
        "unexpected assistant text: {text}"
    );
    assert!(output.token_usage.is_some(), "usage payload missing");
    assert!(output.app_thread_id.is_some(), "session id missing");
}

/// Opt-in smoke: verifies the undocumented `list_models` control request
/// still answers on the installed CLI. Run with the same command as above.
#[test]
#[ignore = "requires an installed, logged-in claude CLI"]
fn real_claude_code_cli_lists_models_over_control_protocol() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let models = list_available_claude_code_models().expect("real model catalog");
    assert!(!models.is_empty(), "empty model catalog from real CLI");
}

#[test]
fn claude_code_error_result_surfaces_as_backend_error() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let _bin_guard = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _mode_guard = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "error");

    let client = Client::new();
    let mut backend = RustLlmCodergenBackend::new(client);
    let request = CodergenBackendRequest {
        node_id: "implement".to_string(),
        prompt: "Prepare the change.".to_string(),
        provider: "claude-code".to_string(),
        runtime_mode: CodergenRuntimeMode::agent(),
        project_path: Some(temp.path().to_path_buf()),
        ..CodergenBackendRequest::default()
    };
    let error = backend.run(request).expect_err("backend error");
    match error {
        CodergenError::Backend(message) => {
            assert!(
                message.contains("error_during_execution") && message.contains("simulated failure"),
                "unexpected error message: {message}"
            );
        }
        other => panic!("expected backend error, got: {other:?}"),
    }
}
