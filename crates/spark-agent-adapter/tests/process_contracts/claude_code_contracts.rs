use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::json;
use spark_agent_adapter::claude_code::auth::{
    ClaudeCodeConnection, ClaudeCodeConnectionStatus, AUTH_REQUIRED_MESSAGE,
};
use spark_agent_adapter::{
    list_available_claude_code_models, AgentAttachment, AgentTurnBackend, AgentTurnRequest,
    ClaudeCodeBackend, ClaudeCodeModelMetadata, CodergenBackend, CodergenBackendRequest,
    CodergenBackendResponse, CodergenError, CodergenRuntimeMode, HistoryTurn,
    RustLlmAgentTurnBackend, RustLlmCodergenBackend, UserTurn, AGENT_ATTACHMENTS_METADATA_KEY,
    AGENT_INSTRUCTIONS_METADATA_KEY,
};
use spark_common::events::{TurnStreamChannel, TurnStreamEventKind};
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

#[test]
fn claude_code_pins_instructions_on_every_spawn_as_a_system_prompt_file() {
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
    request.metadata.insert(
        AGENT_INSTRUCTIONS_METADATA_KEY.to_string(),
        json!("Mission frame: pinned."),
    );

    RustLlmAgentTurnBackend::new(Client::new())
        .run_turn(request)
        .expect("fresh retry succeeds");

    // Both the resumed spawn and the fresh retry carry the file.
    let log = std::fs::read_to_string(log_path).expect("args log");
    assert_eq!(log.matches("-- invocation --").count(), 2);
    assert_eq!(log.matches("--append-system-prompt-file").count(), 2);
    assert_eq!(
        log.matches("APPEND_SYSTEM_PROMPT=Mission frame: pinned.")
            .count(),
        2
    );
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
        // Background tasks hold the turn open instead of being disabled.
        "CLAUDE_CODE_DISABLE_BACKGROUND_TASKS=\n",
        "BASH_MAX_TIMEOUT_MS=3600000",
    ] {
        assert!(args.contains(expected), "missing {expected} in: {args}");
    }
    // No effort was requested, so the CLI keeps its own default.
    assert!(!args.contains("--effort"), "unexpected --effort in: {args}");
    // Spark's own binaries, the spark CLI among them, come first on the agent's PATH.
    let spark_bin_dir = std::env::current_exe()
        .expect("current exe")
        .parent()
        .expect("current exe parent")
        .canonicalize()
        .expect("canonical exe dir");
    assert!(
        args.contains(&format!("PATH_FIRST={}", spark_bin_dir.display())),
        "spark binaries not first on PATH in: {args}"
    );
}

#[test]
fn claude_code_backend_passes_the_requested_effort_to_the_cli() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("claude-args.log");
    let _bin_guard = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _mode_guard = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "default");
    let _log_guard = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_LOG", log_path.as_os_str());
    let mut request = agent_request(temp.path());
    request.reasoning_effort = Some("high".to_string());

    ClaudeCodeBackend::new()
        .run_agent_turn(request)
        .expect("turn");

    let args = std::fs::read_to_string(&log_path).expect("args log");
    assert!(
        args.contains("--effort\nhigh\n"),
        "missing --effort high in: {args}"
    );
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
fn claude_code_usage_is_the_per_model_delta_of_cumulative_model_usage() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let _bin_guard = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _mode_guard = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "default");

    let fresh = ClaudeCodeBackend::new()
        .run_agent_turn(agent_request(temp.path()))
        .expect("fresh turn");
    let usage = fresh.token_usage.expect("fresh usage");
    assert_eq!(usage["input_tokens"], json!(12));
    assert_eq!(usage["cached_input_tokens"], json!(5));
    assert_eq!(usage["output_tokens"], json!(34));
    assert_eq!(
        usage["by_model"]["claude-opus-4-8"],
        json!({"input_tokens": 10, "cached_input_tokens": 5, "cache_creation_input_tokens": 2,
            "output_tokens": 30, "total_tokens": 40, "cost_usd": 0.5})
    );
    assert_eq!(
        usage["by_model"]["claude-haiku-4-5"]["input_tokens"],
        json!(2)
    );
    assert_eq!(fresh.token_usage_breakdown.as_ref(), Some(&usage));

    let mut resumed_request = agent_request(temp.path());
    resumed_request.metadata.insert(
        "spark.runtime.claude_code.session_id".to_string(),
        json!(fresh.app_thread_id.expect("session id")),
    );
    resumed_request.metadata.insert(
        "spark.runtime.claude_code.model_usage".to_string(),
        usage["model_usage"].clone(),
    );
    let resumed = ClaudeCodeBackend::new()
        .run_agent_turn(resumed_request)
        .expect("resumed turn")
        .token_usage
        .expect("resumed usage");
    assert_eq!(
        resumed["by_model"],
        json!({"claude-opus-4-8": {"input_tokens": 101, "cached_input_tokens": 100,
            "cache_creation_input_tokens": 0, "output_tokens": 10, "total_tokens": 111,
            "cost_usd": 0.25}})
    );
    assert_eq!(resumed["input_tokens"], json!(101));
    assert_eq!(resumed["cost_usd"], json!(0.25));
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
fn codergen_rejects_an_unrepresentable_timeout_before_starting_the_agent() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("claude-args.log");
    let _bin_guard = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _log_guard = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_LOG", log_path.as_os_str());
    for seconds in [1e100, f64::INFINITY, f64::NAN, -1.0, 0.0] {
        let error = RustLlmCodergenBackend::new(Client::new())
            .run(CodergenBackendRequest {
                node_id: "implement".to_string(),
                prompt: "Prepare the change.".to_string(),
                provider: "claude-code".to_string(),
                runtime_mode: CodergenRuntimeMode::agent(),
                project_path: Some(temp.path().to_path_buf()),
                timeout_seconds: Some(seconds),
                ..CodergenBackendRequest::default()
            })
            .expect_err("invalid timeout");
        assert!(
            error.to_string().contains("invalid node timeout"),
            "{error}"
        );
    }
    assert!(!log_path.exists(), "the agent must not start");
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
                supported_efforts: vec![],
            },
            ClaudeCodeModelMetadata {
                id: "claude-fable-5[1m]".to_string(),
                display: "Fable".to_string(),
                supported_efforts: vec!["low".to_string(), "high".to_string()],
            },
            ClaudeCodeModelMetadata {
                id: "sonnet".to_string(),
                display: "Sonnet".to_string(),
                supported_efforts: vec![],
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

/// Opt-in smoke: a background Bash task holds a real chat turn open until its
/// follow-up result, and Stop while only that task runs ends the turn.
#[test]
#[ignore = "requires an installed, logged-in claude CLI; consumes real usage"]
fn real_claude_code_background_task_holds_the_turn_and_stop_ends_it() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let prompt = "Use the Bash tool with run_in_background set to true to run `sleep 20 && echo BUILD_DONE`. \
        Then reply STARTED and end your turn without waiting. When the background task completes, reply FINISHED.";
    let request = |prompt: &str| {
        let mut request = agent_request(temp.path());
        request.model = Some("haiku".to_string());
        request.prompt = prompt.to_string();
        request
    };

    let started = std::time::Instant::now();
    let output = ClaudeCodeBackend::new()
        .run_agent_turn(request(prompt))
        .expect("background turn");
    let text = output.final_assistant_text.unwrap_or_default();
    assert!(text.contains("FINISHED"), "unexpected final text: {text}");
    assert!(started.elapsed() >= std::time::Duration::from_secs(20));

    let stop_request = request(prompt);
    let (path, id) = (
        stop_request.project_path.clone(),
        stop_request.conversation_id.clone(),
    );
    let stopped = std::sync::atomic::AtomicBool::new(false);
    let started = std::time::Instant::now();
    let output = ClaudeCodeBackend::new()
        .run_agent_turn_with_event_sink(
            stop_request,
            Some(Arc::new(move |event| {
                if event.kind == TurnStreamEventKind::TokenUsageUpdated
                    && !stopped.swap(true, std::sync::atomic::Ordering::SeqCst)
                {
                    assert!(ClaudeCodeBackend::new().interrupt(&path, &id));
                }
            })),
        )
        .expect("stopped turn");
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    let text = output.final_assistant_text.unwrap_or_default();
    assert!(
        text.contains("STARTED") && !text.contains("FINISHED"),
        "unexpected final text: {text}"
    );
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

#[test]
fn captured_native_configuration_controls_actual_claude_launch() {
    let _lock = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("args.log");
    let _log = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_LOG", &log);
    let _binary = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", "/missing/later-binary");
    let _permission = EnvVarGuard::set("SPARK_CLAUDE_CODE_PERMISSION_MODE", "bypassPermissions");
    let mut request = agent_request(temp.path());
    request.metadata.insert(
        "spark.execution.settings".into(),
        json!({"configuration": {"agents": {"native": {
            "claude_binary": fake_claude_code_bin(), "claude_permission_mode": "acceptEdits",
            "claude_config_dir": temp.path().join("captured-config")
        }}}}),
    );
    ClaudeCodeBackend::new().run_agent_turn(request).unwrap();
    let args = std::fs::read_to_string(log).unwrap();
    assert!(args.contains("acceptEdits"));
    assert!(!args.contains("bypassPermissions"));
}

fn answer_request(
    request: &AgentTurnRequest,
) -> spark_agent_adapter::AgentRequestUserInputAnswerRequest {
    spark_agent_adapter::AgentRequestUserInputAnswerRequest {
        conversation_id: request.conversation_id.clone(),
        project_path: request.project_path.clone(),
        request_id: "question-1".into(),
        assistant_turn_id: "assistant-1".into(),
        answers: BTreeMap::from([("question-1".into(), "Blue".into())]),
        request_user_input: None,
        history: Vec::new(),
        provider: request.provider.clone(),
        model: request.model.clone(),
        llm_profile: None,
        reasoning_effort: None,
        chat_mode: None,
        metadata: BTreeMap::new(),
    }
}

#[test]
fn claude_code_question_round_trip_routes_answers_and_rejects_dead_requests() {
    let _lock = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let _bin = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "question");
    let request = agent_request(temp.path());
    let answer = answer_request(&request);
    let live_answer = answer.clone();
    let output = RustLlmAgentTurnBackend::new(Client::new())
        .run_turn_with_event_sink(
            request,
            Some(Arc::new(move |event| {
                if event.kind != TurnStreamEventKind::RequestUserInputRequested {
                    return;
                }
                assert_eq!(
                    event.request_user_input.as_ref().unwrap()["request_id"],
                    "question-1"
                );
                assert_eq!(
                    event.request_user_input.as_ref().unwrap()["questions"][0]["id"],
                    "question-1"
                );
                let backend = RustLlmAgentTurnBackend::new(Client::new());
                let mut wrong = live_answer.clone();
                wrong.request_id = "unrelated".into();
                assert!(backend
                    .answer_request_user_input(wrong)
                    .unwrap()
                    .thread_resume_failure
                    .is_some());
                let mut wrong = live_answer.clone();
                wrong.conversation_id = "another-conversation".into();
                assert!(backend
                    .answer_request_user_input(wrong)
                    .unwrap()
                    .thread_resume_failure
                    .is_some());
                let delivered = backend
                    .answer_request_user_input(live_answer.clone())
                    .unwrap();
                assert!(delivered.thread_resume_failure.is_none());
                assert_eq!(
                    delivered.events[0].source.raw_kind.as_deref(),
                    Some("request_user_input_answer_delivered")
                );
                assert!(backend
                    .answer_request_user_input(live_answer.clone())
                    .unwrap()
                    .thread_resume_failure
                    .is_some());
            })),
        )
        .unwrap();
    assert_eq!(output.final_assistant_text.as_deref(), Some("All set."));
    assert!(output
        .raw_log_lines
        .iter()
        .any(|line| line.direction == "stdin" && line.line.contains("updatedInput")));
    let dead = RustLlmAgentTurnBackend::new(Client::new())
        .answer_request_user_input(answer)
        .unwrap();
    assert_eq!(
        dead.thread_resume_failure.unwrap().error_code.as_deref(),
        Some("request_user_input_not_pending")
    );
}

#[test]
fn claude_code_interrupt_completes_normally_and_matches_receipt_ids() {
    let _lock = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let _bin = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    for mode in [
        "interrupt",
        "interrupt-question",
        "interrupt-wrong-id",
        "interrupt-aborted",
        "legacy",
    ] {
        let _mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", mode);
        let request = agent_request(temp.path());
        let path = request.project_path.clone();
        let id = request.conversation_id.clone();
        let output = ClaudeCodeBackend::new()
            .run_agent_turn_with_event_sink(
                request,
                Some(Arc::new(move |event| {
                    let trigger = if mode == "interrupt-question" {
                        event.kind == TurnStreamEventKind::RequestUserInputRequested
                    } else {
                        event.source.raw_kind.as_deref() == Some("system_init")
                    };
                    if trigger {
                        assert_eq!(
                            ClaudeCodeBackend::new().interrupt(&path, &id),
                            mode != "legacy"
                        );
                    }
                })),
            )
            .unwrap();
        assert!(output
            .events
            .iter()
            .any(|event| event.kind == TurnStreamEventKind::TurnCompleted));
        assert!(output.thread_resume_failure.is_none());
        assert!(output.events.iter().all(|event| event.error.is_none()));
        assert_eq!(
            output
                .raw_log_lines
                .iter()
                .any(|line| line.direction == "control"),
            mode == "interrupt-wrong-id"
        );
    }
}

#[test]
fn claude_code_control_answers_every_request_despite_malformed_traffic() {
    let _lock = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let _bin = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    // The fake asserts each answer it is owed before finishing the turn.
    for mode in [
        "malformed-json",
        "malformed-request",
        "malformed-response",
        "unknown-subtype",
        "dead-question",
        "request-before-init",
        "duplicate-id",
    ] {
        let _mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", mode);
        let request = agent_request(temp.path());
        let answer = answer_request(&request);
        let output = ClaudeCodeBackend::new()
            .run_agent_turn_with_event_sink(request, Some(Arc::new(|_| {})))
            .unwrap();
        assert_eq!(output.final_assistant_text.as_deref(), Some("All set."));
        if mode.starts_with("malformed") {
            assert!(output
                .raw_log_lines
                .iter()
                .any(|line| line.direction == "control"));
        }
        assert!(ClaudeCodeBackend::new()
            .answer_request_user_input(answer)
            .thread_resume_failure
            .is_some());
    }
}

#[test]
fn claude_code_init_fallback_denies_an_outstanding_question() {
    let _lock = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let _bin = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "question-before-init");

    let started = std::time::Instant::now();
    let output = ClaudeCodeBackend::new()
        .run_agent_turn_with_event_sink(agent_request(temp.path()), Some(Arc::new(|_| {})))
        .unwrap();

    // The fake asserts it received the denial before its result.
    assert_eq!(output.final_assistant_text.as_deref(), Some("All set."));
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}

#[test]
fn claude_code_question_without_a_handler_is_denied() {
    let _lock = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let _bin = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "question-denied");
    // The flow path, and the sink-less entry points the boundary CLI uses.
    let outputs = [
        ClaudeCodeBackend::new()
            .run_agent_turn_without_questions(agent_request(temp.path()), Some(Arc::new(|_| {})))
            .unwrap(),
        ClaudeCodeBackend::new()
            .run_agent_turn(agent_request(temp.path()))
            .unwrap(),
        RustLlmAgentTurnBackend::new(Client::new())
            .run_turn(agent_request(temp.path()))
            .unwrap(),
    ];
    for output in outputs {
        assert_eq!(output.final_assistant_text.as_deref(), Some("All set."));
        assert!(output
            .events
            .iter()
            .all(|event| event.kind != TurnStreamEventKind::RequestUserInputRequested));
    }
}

#[test]
fn claude_code_plan_mode_turn_submits_exit_plan_mode_as_the_plan() {
    let _lock = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("args.log");
    let _bin = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "exit-plan-mode");
    let _log = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_LOG", &log);
    let plan_events = |output: &spark_agent_adapter::AgentTurnOutput| {
        output
            .events
            .iter()
            .filter(|event| event.channel == Some(TurnStreamChannel::Plan))
            .cloned()
            .collect::<Vec<_>>()
    };

    // The fake asserts the deny in plan mode and the allow otherwise.
    let mut request = agent_request(temp.path());
    request.chat_mode = Some("plan".into());
    // The mode is per turn, resumed or not.
    request.metadata.insert(
        "spark.runtime.claude_code.session_id".into(),
        json!("sess-plan"),
    );
    let output = ClaudeCodeBackend::new()
        .run_agent_turn_with_event_sink(request, Some(Arc::new(|_| {})))
        .unwrap();
    assert_eq!(output.final_assistant_text.as_deref(), Some("All set."));
    let plans = plan_events(&output);
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].kind, TurnStreamEventKind::ContentCompleted);
    assert_eq!(
        plans[0].content_delta.as_deref(),
        Some("# Plan\n\n- Do the thing.")
    );
    let args = std::fs::read_to_string(&log).unwrap();
    assert!(args.contains("--resume\nsess-plan"));
    assert!(args.contains("--permission-mode\nplan\n"));

    std::fs::remove_file(&log).unwrap();
    let mut request = agent_request(temp.path());
    request.chat_mode = Some("chat".into());
    let output = ClaudeCodeBackend::new()
        .run_agent_turn_with_event_sink(request, Some(Arc::new(|_| {})))
        .unwrap();
    assert_eq!(output.final_assistant_text.as_deref(), Some("All set."));
    assert!(plan_events(&output).is_empty());
    let args = std::fs::read_to_string(&log).unwrap();
    assert!(args.contains("--permission-mode\nbypassPermissions\n"));
}

#[test]
fn claude_code_background_task_holds_the_turn_until_its_follow_up_result() {
    let _lock = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let log_path = temp.path().join("claude.log");
    let _bin = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "background");
    let _log = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_LOG", log_path.as_os_str());

    let output = ClaudeCodeBackend::new()
        .run_agent_turn(agent_request(temp.path()))
        .unwrap();

    assert_eq!(
        output.final_assistant_text.as_deref(),
        Some("Build finished.")
    );
    let texts = output
        .events
        .iter()
        .filter(|event| event.kind == TurnStreamEventKind::ContentCompleted)
        .map(|event| (event.content_delta.as_deref(), event.phase.as_deref()))
        .collect::<Vec<_>>();
    assert!(texts.contains(&(Some("Started a background build."), Some("commentary"))));
    assert_eq!(
        texts.last(),
        Some(&(Some("Build finished."), Some("final_answer")))
    );
    assert_eq!(
        output
            .events
            .iter()
            .filter(|event| event.kind == TurnStreamEventKind::TurnCompleted)
            .count(),
        1
    );
    // Usage is the last cumulative modelUsage, not a sum of results.
    assert_eq!(output.token_usage.unwrap()["output_tokens"], json!(50));
    let log = std::fs::read_to_string(log_path).unwrap();
    assert!(
        log.contains("STDIN_CLOSED"),
        "stdin closed at turn end: {log}"
    );
}

#[test]
fn claude_code_interrupt_while_waiting_on_background_tasks_ends_the_turn() {
    let _lock = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let _bin = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "background-interrupt");
    let request = agent_request(temp.path());
    let (path, id) = (
        request.project_path.clone(),
        request.conversation_id.clone(),
    );

    let started = std::time::Instant::now();
    let stopped = std::sync::atomic::AtomicBool::new(false);
    let output = ClaudeCodeBackend::new()
        .run_agent_turn_with_event_sink(
            request,
            Some(Arc::new(move |event| {
                // The first result's usage arrives while only the build runs.
                if event.kind == TurnStreamEventKind::TokenUsageUpdated
                    && !stopped.swap(true, std::sync::atomic::Ordering::SeqCst)
                {
                    assert!(ClaudeCodeBackend::new().interrupt(&path, &id));
                }
            })),
        )
        .unwrap();

    assert!(started.elapsed() < std::time::Duration::from_secs(3));
    assert_eq!(
        output.final_assistant_text.as_deref(),
        Some("Started a background build.")
    );
    let last = output.events.last().unwrap();
    assert_eq!(last.kind, TurnStreamEventKind::TurnCompleted);
    assert!(last.error.is_none());
    assert!(output.events.iter().any(|event| {
        event.phase.as_deref() == Some("final_answer")
            && event.content_delta.as_deref() == Some("Started a background build.")
    }));
}

#[test]
fn claude_code_interrupt_after_a_queued_result_interrupts_the_next_model_turn() {
    let _lock = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let _bin = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "queued-interrupt");
    let request = agent_request(temp.path());
    let (path, id) = (
        request.project_path.clone(),
        request.conversation_id.clone(),
    );
    let stopped = std::sync::atomic::AtomicBool::new(false);
    let output = ClaudeCodeBackend::new()
        .run_agent_turn_with_event_sink(
            request,
            Some(Arc::new(move |event| {
                if event.kind == TurnStreamEventKind::TokenUsageUpdated
                    && !stopped.swap(true, std::sync::atomic::Ordering::SeqCst)
                {
                    assert!(ClaudeCodeBackend::new().interrupt(&path, &id));
                }
            })),
        )
        .unwrap();

    // The fake asserts it received the interrupt before stdin closed.
    assert!(output.raw_log_lines.iter().any(|line| {
        line.direction == "stdin" && line.line.contains(r#""subtype":"interrupt""#)
    }));
    let last = output.events.last().unwrap();
    assert_eq!(last.kind, TurnStreamEventKind::TurnCompleted);
    assert!(last.error.is_none());
}

#[test]
fn claude_code_message_sent_mid_turn_folds_in_and_holds_the_turn_until_delivered() {
    let _lock = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let log_path = temp.path().join("claude.log");
    let _bin = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "mid-turn-message");
    let _log = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_LOG", &log_path);
    let request = agent_request(temp.path());
    let (path, id) = (
        request.project_path.clone(),
        request.conversation_id.clone(),
    );
    let backend = || RustLlmAgentTurnBackend::new(Client::new());
    assert!(!backend().send_message(&path, &id, "note-1", "Also check the docs."));
    let sent = std::sync::atomic::AtomicBool::new(false);
    let output = ClaudeCodeBackend::new()
        .run_agent_turn_with_event_sink(
            request,
            Some(Arc::new(move |event| {
                if event.kind == TurnStreamEventKind::ContentDelta
                    && !sent.swap(true, std::sync::atomic::Ordering::SeqCst)
                {
                    assert!(backend().send_message(&path, &id, "note-1", "Also check the docs."));
                }
            })),
        )
        .unwrap();

    // The first result did not list the message, so the turn ran on to the one that did.
    assert_eq!(
        output.final_assistant_text.as_deref(),
        Some("Read your note.")
    );
    let message = output
        .raw_log_lines
        .iter()
        .filter(|line| line.direction == "stdin")
        .map(|line| serde_json::from_str::<serde_json::Value>(&line.line).unwrap())
        .find(|line| line["message"]["content"] == json!("Also check the docs."))
        .expect("mid-turn message on stdin");
    assert!(message["uuid"].as_str().is_some());
    assert!(message.get("priority").is_none());
    let log = std::fs::read_to_string(&log_path).unwrap();
    assert!(log.contains("MID_TURN=\"Also check the docs.\""), "{log}");
    assert!(log.contains("STDIN_CLOSED"), "{log}");
    assert!(!output.events.iter().any(|event| {
        event.kind == TurnStreamEventKind::Other("user_message_not_delivered".into())
    }));
}

#[test]
fn claude_code_reports_a_mid_turn_message_the_stopped_turn_never_used() {
    let _lock = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let _bin = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "mid-turn-stop");
    let request = agent_request(temp.path());
    let (path, id) = (
        request.project_path.clone(),
        request.conversation_id.clone(),
    );
    let sent = std::sync::atomic::AtomicBool::new(false);
    let output = ClaudeCodeBackend::new()
        .run_agent_turn_with_event_sink(
            request,
            Some(Arc::new(move |event| {
                if event.kind == TurnStreamEventKind::ContentDelta
                    && !sent.swap(true, std::sync::atomic::Ordering::SeqCst)
                {
                    let backend = ClaudeCodeBackend::new();
                    assert!(backend.send_message(&path, &id, "note-1", "Too late."));
                    assert!(backend.interrupt(&path, &id));
                    // A stopping turn takes no more messages.
                    assert!(!backend.send_message(&path, &id, "note-2", "Later."));
                }
            })),
        )
        .unwrap();

    let not_delivered = output
        .events
        .iter()
        .filter(|event| {
            event.kind == TurnStreamEventKind::Other("user_message_not_delivered".into())
        })
        .collect::<Vec<_>>();
    assert_eq!(not_delivered.len(), 1);
    assert_eq!(not_delivered[0].source.item_id.as_deref(), Some("note-1"));
}

#[test]
fn claude_code_crash_after_init_resumes_the_session_once() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _bin = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    // crash-orphan leaves a descendant holding stdout and stderr open;
    // crash-chatty-orphan's descendant also keeps writing to stdout.
    for mode in ["crash-after-init", "crash-orphan", "crash-chatty-orphan"] {
        let temp = tempfile::tempdir().unwrap();
        let log_path = temp.path().join("claude.log");
        let _mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", mode);
        let _log = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_LOG", log_path.as_os_str());

        let started = std::time::Instant::now();
        let output = ClaudeCodeBackend::new()
            .run_agent_turn(agent_request(temp.path()))
            .unwrap();

        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "{mode}"
        );
        assert_eq!(output.final_assistant_text.as_deref(), Some("Recovered."));
        assert_eq!(output.app_thread_id.as_deref(), Some("sess-fake-claude-1"));
        assert!(output.thread_resume_failure.is_none());
        // The first process's output stays in the turn.
        assert!(output
            .events
            .iter()
            .any(|event| event.content_delta.as_deref() == Some("Working.")));
        let log = std::fs::read_to_string(log_path).unwrap();
        assert_eq!(log.matches("-- invocation --").count(), 2);
        assert!(log.contains("--resume\nsess-fake-claude-1"));
        assert!(log.contains("USER_CONTENT=\"Your previous process exited"));
    }
}

#[test]
fn claude_code_interrupt_does_not_hide_api_failures_and_process_death_expires_questions() {
    let _lock = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let _bin = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    for mode in ["interrupt-error", "dead-question-exit"] {
        let _mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", mode);
        let request = agent_request(temp.path());
        let answer = answer_request(&request);
        let path = request.project_path.clone();
        let id = request.conversation_id.clone();
        let error = ClaudeCodeBackend::new()
            .run_agent_turn_with_event_sink(
                request,
                Some(Arc::new(move |event| {
                    if mode == "interrupt-error"
                        && event.source.raw_kind.as_deref() == Some("system_init")
                    {
                        assert!(ClaudeCodeBackend::new().interrupt(&path, &id));
                    }
                })),
            )
            .unwrap_err();
        assert!(error.message.contains(if mode == "interrupt-error" {
            "error_during_execution"
        } else {
            "without a result event"
        }));
        assert_eq!(
            ClaudeCodeBackend::new()
                .answer_request_user_input(answer)
                .thread_resume_failure
                .unwrap()
                .error_code
                .as_deref(),
            Some("request_user_input_not_pending")
        );
    }
}

fn sign_in_settings(
    config_dir: &std::path::Path,
) -> spark_common::agent_settings::NativeAgentSettings {
    spark_common::agent_settings::NativeAgentSettings {
        claude_binary: Some(fake_claude_code_bin().into()),
        claude_config_dir: Some(config_dir.to_string_lossy().into()),
        ..Default::default()
    }
}

fn settled_status(
    connection: &mut ClaudeCodeConnection,
    native: &spark_common::agent_settings::NativeAgentSettings,
    done: impl Fn(&ClaudeCodeConnectionStatus) -> bool,
) -> ClaudeCodeConnectionStatus {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let status = connection.status(native).unwrap();
        if done(&status) || std::time::Instant::now() > deadline {
            return status;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[test]
fn claude_code_sign_in_relays_the_link_and_pasted_code_to_the_cli() {
    let temp = tempfile::tempdir().unwrap();
    let native = sign_in_settings(temp.path());
    let mut connection = ClaudeCodeConnection::default();
    assert_eq!(connection.status(&native).unwrap().status, "disconnected");

    let (sender, completed) = std::sync::mpsc::channel();
    let pending = connection
        .start_login(&native, move |status| {
            let _ = sender.send(status.status);
        })
        .unwrap();
    assert_eq!(pending.status, "pending");
    // Terminal hyperlink escapes around the printed link are dropped.
    assert_eq!(
        pending.login_url.as_deref(),
        Some("https://claude.com/cai/oauth/authorize?code=true&state=fake")
    );
    assert!(connection.submit_code("good#state\nextra").is_err());

    connection.submit_code("missing-state").unwrap();
    let rejected = settled_status(&mut connection, &native, |status| status.message.is_some());
    assert_eq!(rejected.status, "pending");
    assert!(rejected.message.unwrap().starts_with("Invalid code"));

    connection.submit_code("  good#state \n").unwrap();
    assert_eq!(
        completed
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap(),
        "connected"
    );
    let connected = connection.status(&native).unwrap();
    assert_eq!(connected.status, "connected");
    let account = connected.account.unwrap();
    assert_eq!(account.email.as_deref(), Some("spark@example.test"));
    assert_eq!(account.plan.as_deref(), Some("max"));
    assert!(connected.login_url.is_none());
    // Afterwards the CLI itself is asked, and no login is waiting for a code.
    assert_eq!(connection.status(&native).unwrap().status, "connected");
    assert!(connection.submit_code("good#state").is_err());
}

#[test]
fn claude_code_sign_in_reports_cli_failures_and_cancels_without_signing_in() {
    let temp = tempfile::tempdir().unwrap();
    let native = sign_in_settings(temp.path());
    let mut connection = ClaudeCodeConnection::default();

    connection.start_login(&native, |_| {}).unwrap();
    connection.submit_code("wrong#state").unwrap();
    let failed = settled_status(&mut connection, &native, |status| {
        status.status != "pending"
    });
    assert_eq!(failed.status, "disconnected");
    assert_eq!(
        failed.message.as_deref(),
        Some("Login failed: Invalid authorization code")
    );

    connection.start_login(&native, |_| {}).unwrap();
    let cancelled = connection.cancel_login();
    assert_eq!(cancelled.status, "disconnected");
    assert!(connection.submit_code("good#state").is_err());
    assert_eq!(connection.status(&native).unwrap().status, "disconnected");
    assert!(!temp.path().join("fake-signed-in").exists());
}

#[test]
fn configured_claude_config_dir_reaches_auth_status_login_and_turns() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let config_dir = temp.path().join("claude-config");
    std::fs::create_dir_all(&config_dir).unwrap();
    let log = temp.path().join("claude-args.log");
    let _log = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_LOG", &log);
    let _binary = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", "/missing/env-binary");
    let native = sign_in_settings(&config_dir);

    // The fake CLI keeps its sign-in under CLAUDE_CONFIG_DIR.
    let mut connection = ClaudeCodeConnection::default();
    connection.start_login(&native, |_| {}).unwrap();
    connection.submit_code("good#state").unwrap();
    settled_status(&mut connection, &native, |status| {
        status.status == "connected"
    });
    assert!(config_dir.join("fake-signed-in").exists());
    assert_eq!(
        ClaudeCodeConnection::default()
            .status(&native)
            .unwrap()
            .status,
        "connected"
    );

    let mut request = agent_request(temp.path());
    request.metadata.insert(
        "spark.execution.settings".into(),
        json!({"configuration": {"agents": {"native": native}}}),
    );
    ClaudeCodeBackend::new().run_agent_turn(request).unwrap();
    let args = std::fs::read_to_string(log).unwrap();
    assert!(
        args.contains(&format!("CLAUDE_CONFIG_DIR={}\n", config_dir.display())),
        "{args}"
    );
}

#[test]
fn default_claude_config_dir_is_spark_owned_and_needs_its_own_sign_in() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let spark_home = temp.path().join("spark-home");
    let config_dir = spark_home.join("runtime/claude");
    let _spark_home = EnvVarGuard::set("SPARK_HOME", &spark_home);
    let _configured = EnvVarGuard::set("SPARK_CLAUDE_CODE_CONFIG_DIR", "");
    let _bin = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let log = temp.path().join("claude-args.log");
    let _log = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_LOG", &log);
    let native = spark_common::agent_settings::NativeAgentSettings {
        claude_binary: Some(fake_claude_code_bin().into()),
        ..Default::default()
    };

    // A fresh Spark-owned dir is not signed in, even if ~/.claude is.
    let mut connection = ClaudeCodeConnection::default();
    assert_eq!(connection.status(&native).unwrap().status, "disconnected");
    assert!(config_dir.is_dir());
    let _mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "not-logged-in");
    let error = ClaudeCodeBackend::new()
        .run_agent_turn(agent_request(temp.path()))
        .expect_err("not signed in");
    assert_eq!(error.message, AUTH_REQUIRED_MESSAGE);
    let args = std::fs::read_to_string(&log).unwrap();
    assert!(
        args.contains(&format!("CLAUDE_CONFIG_DIR={}\n", config_dir.display())),
        "{args}"
    );

    // Sign-in runs `claude auth login` against that same dir.
    connection.start_login(&native, |_| {}).unwrap();
    connection.submit_code("good#state").unwrap();
    settled_status(&mut connection, &native, |status| {
        status.status == "connected"
    });
    assert!(config_dir.join("fake-signed-in").exists());
}

#[test]
fn claude_code_auth_failure_asks_for_sign_in_and_keeps_the_cli_reason() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let _bin_guard = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _mode_guard = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "auth-error");

    let error = ClaudeCodeBackend::new()
        .run_agent_turn(agent_request(temp.path()))
        .expect_err("auth failure");
    assert_eq!(error.message, AUTH_REQUIRED_MESSAGE);
    assert_eq!(
        error.details.unwrap()["result"],
        "Failed to authenticate: OAuth session expired and could not be refreshed"
    );
}

#[test]
fn claude_code_sends_images_as_base64_blocks_and_files_as_references() {
    use base64::Engine as _;

    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("claude-args.log");
    let _bin_guard = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _mode_guard = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "default");
    let _log_guard = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_LOG", log_path.as_os_str());
    let attachment = |name: &str, media_type: &str, bytes: &[u8]| {
        let path = temp.path().join(name);
        std::fs::write(&path, bytes).expect("attachment bytes");
        AgentAttachment {
            id: format!("att-{name}"),
            filename: name.to_string(),
            media_type: media_type.to_string(),
            size_bytes: bytes.len() as u64,
            path: path.to_string_lossy().to_string(),
        }
    };
    let screenshot = attachment("shot.png", "image/png", b"\x89PNG\r\n\x1a\nfake");
    let animation = attachment("loop.gif", "image/gif", b"GIF89afake");
    let log = attachment("build.log", "text/plain", b"error");
    let earlier = attachment("spec.pdf", "application/pdf", b"%PDF-1.4");
    let mut earlier_turn = UserTurn::new("");
    earlier_turn.attachments = vec![earlier.clone()];

    // Image-only message on a fresh session.
    let mut request = agent_request(temp.path());
    request.prompt = String::new();
    request.history = vec![HistoryTurn::User(earlier_turn)];
    request.metadata.insert(
        AGENT_ATTACHMENTS_METADATA_KEY.to_string(),
        json!([screenshot, animation, log]),
    );
    RustLlmAgentTurnBackend::new(Client::new())
        .run_turn(request)
        .expect("attachment turn");
    // Text-only input stays a plain string.
    RustLlmAgentTurnBackend::new(Client::new())
        .run_turn(agent_request(temp.path()))
        .expect("text turn");

    let contents = std::fs::read_to_string(&log_path)
        .expect("log")
        .lines()
        .filter_map(|line| line.strip_prefix("USER_CONTENT="))
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("content json"))
        .collect::<Vec<_>>();
    assert_eq!(contents.len(), 2);
    let blocks = contents[0].as_array().expect("content blocks");
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[0]["type"], "text");
    let text = blocks[0]["text"].as_str().expect("text");
    assert!(text.starts_with("Attached files"));
    assert!(text.contains(&log.path));
    assert!(text.contains(&format!("Files attached earlier in this conversation (local copies):\n- spec.pdf (application/pdf, 8 bytes): {}", earlier.path)));
    assert!(!text.contains("shot.png"));
    assert_eq!(
        blocks[1],
        json!({"type": "image", "source": {"type": "base64", "media_type": "image/png",
            "data": base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\nfake")}})
    );
    assert_eq!(blocks[2]["source"]["media_type"], "image/gif");
    assert_eq!(contents[1], json!("Prepare the change."));
}

fn envelope_node_request(
    project_path: &std::path::Path,
    response_contract: Option<&str>,
) -> spark_agent_adapter::CodergenRequest {
    use attractor_core::{DotAttribute, DotValue, DotValueType};
    let source = json!({"schema_version":"1", "id":"flow",
        "nodes":{"start":{"kind":"start"},
            "review":{"kind":"agent_task", "config":{"kind":"agent_task", "prompt":"Review the change."}},
            "fix":{"kind":"agent_task", "config":{"kind":"agent_task", "prompt":"fix"}},
            "end":{"kind":"exit"}},
        "edges":[{"from":"start","to":"review"},
            {"from":"review","to":"end","label":"Approve"},
            {"from":"review","to":"fix","label":"Rework"},
            {"from":"fix","to":"end"}]
    });
    let mut graph = attractor_dsl::parse_flow_definition(&source.to_string())
        .unwrap()
        .to_runtime_dot_graph();
    let node = graph.nodes.get_mut("review").unwrap();
    let mut attrs = vec![
        ("llm_provider", "claude-code"),
        ("spark.writes_context", r#"["context.review.notes"]"#),
    ];
    attrs.extend(response_contract.map(|value| ("codergen.response_contract", value)));
    for (key, value) in attrs {
        node.attrs.insert(
            key.to_string(),
            DotAttribute {
                key: key.to_string(),
                value: DotValue::String(value.to_string()),
                value_type: DotValueType::String,
                line: 0,
            },
        );
    }
    spark_agent_adapter::CodergenRequest {
        node_id: "review".to_string(),
        node: node.clone(),
        graph,
        context: Default::default(),
        logs_root: None,
        fallback_model: None,
        fallback_provider: None,
        fallback_profile: None,
        fallback_reasoning_effort: None,
        project_path: Some(project_path.to_path_buf()),
        metadata: BTreeMap::new(),
    }
}

/// Each logged invocation's argument list.
fn logged_invocations(log_path: &std::path::Path) -> Vec<Vec<String>> {
    std::fs::read_to_string(log_path)
        .expect("args log")
        .split("-- invocation --")
        .filter(|chunk| chunk.contains("-p"))
        .map(|chunk| chunk.lines().map(str::to_string).collect())
        .collect()
}

fn arg_after(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].clone())
}

#[test]
fn claude_code_status_envelope_node_passes_its_schema_and_routes_from_the_structured_result() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("claude-args.log");
    let _bin_guard = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _mode_guard = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "structured");
    let _log_guard = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_LOG", log_path.as_os_str());
    let request = envelope_node_request(temp.path(), Some("status_envelope"));
    let expected_schema = spark_agent_adapter::status_envelope_json_schema(
        &request.graph,
        "review",
        &attractor_core::resolve_context_write_contract(&request.node.attrs),
    );

    let execution = spark_agent_adapter::CodergenHandler::with_backend(
        RustLlmCodergenBackend::new(Client::new()),
    )
    .execute(request)
    .expect("execution");

    assert_eq!(
        execution.outcome.status,
        attractor_core::OutcomeStatus::Success
    );
    assert_eq!(execution.outcome.preferred_label, "Approve");
    assert_eq!(execution.repair_attempts, 0);
    let invocations = logged_invocations(&log_path);
    assert_eq!(invocations.len(), 1);
    let schema = arg_after(&invocations[0], "--json-schema").expect("--json-schema");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&schema).unwrap(),
        expected_schema
    );
    assert_eq!(
        expected_schema["properties"]["preferred_label"]["enum"],
        json!(["Approve", "Rework"])
    );

    // A node without the envelope contract passes no schema.
    std::fs::remove_file(&log_path).unwrap();
    spark_agent_adapter::CodergenHandler::with_backend(RustLlmCodergenBackend::new(Client::new()))
        .execute(envelope_node_request(temp.path(), None))
        .expect("execution");
    let invocations = logged_invocations(&log_path);
    assert_eq!(invocations.len(), 1);
    assert!(!invocations[0].iter().any(|arg| arg == "--json-schema"));
}

#[test]
fn claude_code_envelope_repairs_resume_the_first_attempts_session() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("claude-args.log");
    let _bin_guard = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _log_guard = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_LOG", log_path.as_os_str());
    for (mode, reason) in [
        ("structured-violation", "context.undeclared"),
        ("structured-retries", "empty response"),
        // Exhaustion wins over any envelope-looking text the turn left behind.
        ("structured-retries-result-text", "empty response"),
        ("structured-retries-assistant-text", "empty response"),
    ] {
        let _mode_guard = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", mode);
        let _ = std::fs::remove_file(&log_path);

        let execution = spark_agent_adapter::CodergenHandler::with_backend(
            RustLlmCodergenBackend::new(Client::new()),
        )
        .execute(envelope_node_request(temp.path(), Some("status_envelope")))
        .expect("execution");

        assert_eq!(
            execution.outcome.status,
            attractor_core::OutcomeStatus::Success,
            "{mode}"
        );
        assert_eq!(execution.outcome.preferred_label, "Approve", "{mode}");
        assert_eq!(execution.repair_attempts, 1, "{mode}");
        assert!(
            execution.contract_violations[0].reason.contains(reason),
            "{mode}: {:?}",
            execution.contract_violations
        );
        let invocations = logged_invocations(&log_path);
        assert_eq!(invocations.len(), 2, "{mode}");
        assert_eq!(arg_after(&invocations[0], "--resume"), None, "{mode}");
        assert_eq!(
            arg_after(&invocations[1], "--resume").as_deref(),
            Some("sess-fake-claude-1"),
            "{mode}"
        );
        // The repair keeps the schema, and its message is the short in-session one.
        assert!(invocations[1].iter().any(|arg| arg == "--json-schema"));
        let log = std::fs::read_to_string(&log_path).unwrap();
        let repair_message = log
            .lines()
            .filter(|line| line.starts_with("USER_CONTENT="))
            .nth(1)
            .expect("repair message");
        assert!(
            repair_message.contains("Validation error"),
            "{repair_message}"
        );
        assert!(
            !repair_message.contains("Review the change."),
            "{repair_message}"
        );

        // Run usage counts both attempts; the resumed one only its delta.
        let completed = execution
            .events
            .iter()
            .filter(|event| event.event_type == "claude_code_request_completed")
            .map(|event| event.payload["token_usage"]["output_tokens"].clone())
            .collect::<Vec<_>>();
        assert_eq!(completed, vec![json!(10), json!(5)], "{mode}");
    }
}

#[test]
fn claude_code_envelope_repairs_stop_at_the_limit_when_retries_stay_exhausted() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("claude-args.log");
    let _bin_guard = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _log_guard = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_LOG", log_path.as_os_str());
    let _mode_guard = EnvVarGuard::set(
        "SPARK_FAKE_CLAUDE_CODE_MODE",
        "structured-retries-result-text-always",
    );

    let execution = spark_agent_adapter::CodergenHandler::with_backend(
        RustLlmCodergenBackend::new(Client::new()),
    )
    .execute(envelope_node_request(temp.path(), Some("status_envelope")))
    .expect("execution");

    assert_eq!(
        execution.outcome.status,
        attractor_core::OutcomeStatus::Fail
    );
    assert_eq!(
        execution.repair_attempts,
        spark_agent_adapter::codergen::DEFAULT_CONTRACT_REPAIR_ATTEMPTS
    );
    assert_eq!(execution.contract_violations.len(), 2);
    let invocations = logged_invocations(&log_path);
    assert_eq!(invocations.len(), 2);
    assert_eq!(
        arg_after(&invocations[1], "--resume").as_deref(),
        Some("sess-fake-claude-1")
    );
}

/// Opt-in smoke: one status-envelope node against the real CLI's
/// `--json-schema`. Run with the same command as the other real tests.
#[test]
#[ignore = "requires an installed, logged-in claude CLI; consumes real usage"]
fn real_claude_code_status_envelope_node_returns_a_schema_valid_outcome() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let _mode_guard = EnvVarGuard::set("SPARK_CLAUDE_CODE_PERMISSION_MODE", "default");
    let mut request = envelope_node_request(temp.path(), Some("status_envelope"));
    request.node.attrs.get_mut("prompt").expect("prompt").value = attractor_core::DotValue::String(
        "Do not use any tools. Finish with outcome success and preferred_label Rework.".to_string(),
    );

    let execution = spark_agent_adapter::CodergenHandler::with_backend(
        RustLlmCodergenBackend::new(Client::new()),
    )
    .execute(request)
    .expect("execution");

    assert_eq!(
        execution.outcome.status,
        attractor_core::OutcomeStatus::Success
    );
    assert_eq!(execution.outcome.preferred_label, "Rework");
    assert!(execution.contract_violations.is_empty());
}

#[test]
fn claude_code_status_frames_become_compaction_and_warnings_and_the_turn_records_context_usage() {
    let _lock = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let log_path = temp.path().join("claude-args.log");
    let _bin = EnvVarGuard::set("SPARK_CLAUDE_CODE_BIN", fake_claude_code_bin());
    let _mode = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_MODE", "status-events");
    let _log = EnvVarGuard::set("SPARK_FAKE_CLAUDE_CODE_LOG", log_path.as_os_str());

    let output = ClaudeCodeBackend::new()
        .run_agent_turn(agent_request(temp.path()))
        .unwrap();

    let shown = output
        .events
        .iter()
        .filter(|event| {
            !matches!(&event.kind, TurnStreamEventKind::Other(kind) if kind.starts_with("system_"))
        })
        .map(|event| (event.kind.as_str(), event.message.as_deref()))
        .collect::<Vec<_>>();
    assert_eq!(
        &shown[..4],
        [
            (
                "warning",
                Some("API error 529 (overloaded); retrying in 4s (attempt 2 of 10)")
            ),
            // `allowed` shows nothing.
            (
                "warning",
                Some("Rate limit warning: 5-hour window at 82%, resets 15:00 UTC")
            ),
            ("context_compaction_started", None),
            (
                "context_compaction_completed",
                Some("Context compacted (21.6k \u{2192} 1.9k tokens).")
            ),
        ]
    );
    // The compaction summary is neither a user nor an assistant message.
    assert!(!output.events.iter().any(|event| event
        .message
        .as_deref()
        .is_some_and(|text| text.contains("being continued"))));
    assert_eq!(output.final_assistant_text.as_deref(), Some("Done."));
    assert_eq!(
        output.token_usage.as_ref().unwrap()["context"],
        json!({"used_tokens": 24000, "window_tokens": 1000000})
    );
    let log = std::fs::read_to_string(log_path).unwrap();
    assert!(log.contains("GET_CONTEXT_USAGE"), "{log}");
}
