use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use spark_agent_adapter::{
    ActiveCodergenSession, AgentRequestUserInputAnswerRequest, AgentTurnBackend, AgentTurnRequest,
    AssistantTurn, CodergenBackend, CodergenBackendRequest, CodergenChildInterventionRequest,
    CodergenRuntimeMode, CodergenSessionInterventionBroker, ExecutionEnvironment, HistoryTurn,
    ProviderProfile, RustLlmAgentTurnBackend, RustLlmCodergenBackend, Session, SessionConfig,
    SessionState, UserTurn,
};
use spark_common::events::{TurnStreamChannel, TurnStreamEventKind};
use unified_llm_adapter::{
    stream_events, ActiveLlmProfile, AdapterError, AdapterErrorKind, Client, ContentPart,
    FinishReason, Message, MessageRole, ProviderAdapter, Request, Response, StreamEvent,
    StreamEventType, StreamEvents, ToolCall, Usage,
};

static CODEX_APP_SERVER_TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

fn tool_names(request: &Request) -> Vec<String> {
    request.tools.iter().map(|tool| tool.name.clone()).collect()
}

fn tool_parameters(request: &Request, name: &str) -> Value {
    request
        .tools
        .iter()
        .find(|tool| tool.name == name)
        .and_then(|tool| tool.parameters.clone())
        .expect("tool parameters")
}

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

fn fake_codex_app_server_bin() -> &'static str {
    env!("CARGO_BIN_EXE_spark-agent-fake-codex-app-server")
}

fn wait_for_logged_method(log_path: &Path, method: &str) {
    let started = Instant::now();
    loop {
        if started.elapsed() > Duration::from_secs(5) {
            panic!("timed out waiting for logged method {method}");
        }
        if fs::read_to_string(log_path)
            .ok()
            .into_iter()
            .flat_map(|content| {
                content
                    .lines()
                    .map(|line| line.to_string())
                    .collect::<Vec<_>>()
            })
            .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
            .any(|message| message["method"] == json!(method))
        {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn agent_turn_request_round_trips_selector_metadata_and_history() {
    let request = AgentTurnRequest {
        conversation_id: "conversation-contract".to_string(),
        project_path: "/repo".to_string(),
        prompt: "Continue from here".to_string(),
        history: vec![
            HistoryTurn::User(UserTurn::new("Previous question")),
            HistoryTurn::Assistant(AssistantTurn::new("Previous answer")),
        ],
        provider: Some("openrouter".to_string()),
        model: Some("openrouter/model".to_string()),
        llm_profile: Some("implementation".to_string()),
        reasoning_effort: Some("HIGH".to_string()),
        chat_mode: Some("agent".to_string()),
        metadata: BTreeMap::from([("caller".to_string(), json!("workspace"))]),
    };

    let encoded = serde_json::to_value(&request).expect("serialize request");
    assert_eq!(encoded["conversation_id"], "conversation-contract");
    assert_eq!(encoded["project_path"], "/repo");
    assert_eq!(encoded["prompt"], "Continue from here");
    assert_eq!(encoded["provider"], "openrouter");
    assert_eq!(encoded["model"], "openrouter/model");
    assert_eq!(encoded["llm_profile"], "implementation");
    assert_eq!(encoded["reasoning_effort"], "HIGH");
    assert_eq!(encoded["chat_mode"], "agent");
    assert_eq!(encoded["metadata"]["caller"], "workspace");
    assert_eq!(encoded["history"][0]["role"], "user");
    assert_eq!(encoded["history"][0]["content"], "Previous question");
    assert_eq!(encoded["history"][1]["role"], "assistant");
    assert_eq!(encoded["history"][1]["content"], "Previous answer");

    let decoded: AgentTurnRequest = serde_json::from_value(encoded).expect("deserialize request");
    assert_eq!(decoded, request);
}

#[test]
fn request_user_input_answer_request_round_trips_resume_lifecycle_fields() {
    let request = AgentRequestUserInputAnswerRequest {
        conversation_id: "conversation-answer".to_string(),
        project_path: "/repo".to_string(),
        request_id: "request-1".to_string(),
        assistant_turn_id: "assistant-turn-1".to_string(),
        answers: BTreeMap::from([
            ("constraints".to_string(), "Keep scope narrow".to_string()),
            ("path_choice".to_string(), "Inline card".to_string()),
        ]),
        request_user_input: Some(json!({
            "request_id": "request-1",
            "status": "pending",
            "questions": [
                {
                    "id": "path_choice",
                    "header": "Path",
                    "question": "Which path should I take?",
                    "question_type": "MULTIPLE_CHOICE",
                    "options": [{"label": "Inline card"}],
                    "allow_other": true,
                    "is_secret": false
                }
            ]
        })),
        history: vec![
            HistoryTurn::User(UserTurn::new("Previous question")),
            HistoryTurn::Assistant(AssistantTurn::new("Previous answer")),
        ],
        provider: Some("codex".to_string()),
        model: Some("gpt-agent".to_string()),
        llm_profile: Some("implementation".to_string()),
        reasoning_effort: Some("HIGH".to_string()),
        chat_mode: Some("agent".to_string()),
        metadata: BTreeMap::from([("caller".to_string(), json!("workspace"))]),
    };

    let encoded = serde_json::to_value(&request).expect("serialize answer request");
    assert_eq!(encoded["conversation_id"], "conversation-answer");
    assert_eq!(encoded["project_path"], "/repo");
    assert_eq!(encoded["request_id"], "request-1");
    assert_eq!(encoded["assistant_turn_id"], "assistant-turn-1");
    assert_eq!(encoded["answers"]["path_choice"], "Inline card");
    assert_eq!(encoded["request_user_input"]["status"], "pending");
    assert_eq!(encoded["history"][0]["role"], "user");
    assert_eq!(encoded["provider"], "codex");
    assert_eq!(encoded["model"], "gpt-agent");
    assert_eq!(encoded["llm_profile"], "implementation");
    assert_eq!(encoded["reasoning_effort"], "HIGH");
    assert_eq!(encoded["chat_mode"], "agent");
    assert_eq!(encoded["metadata"]["caller"], "workspace");

    let decoded: AgentRequestUserInputAnswerRequest =
        serde_json::from_value(encoded).expect("deserialize answer request");
    assert_eq!(decoded, request);
}

#[test]
fn codergen_backend_enters_rust_unified_llm_adapter_boundary() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingAdapter::new("openai", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], None).expect("client");
    let mut backend = RustLlmCodergenBackend::new(client);

    let output = backend
        .run(CodergenBackendRequest {
            node_id: "task".to_string(),
            prompt: "Write the runtime note".to_string(),
            context: BTreeMap::new(),
            response_contract: "status_envelope".to_string(),
            contract_repair_attempts: 0,
            timeout_seconds: None,
            write_contract: Default::default(),
            provider: "OpenAI".to_string(),
            model: Some("gpt-boundary".to_string()),
            llm_profile: None,
            reasoning_effort: Some("HIGH".to_string()),
            repair_attempt: None,
            runtime_mode: Default::default(),
            project_path: None,
            metadata: BTreeMap::new(),
        })
        .expect("codergen output");

    assert_eq!(output.response_text(), "adapter response for gpt-boundary");
    assert_eq!(output.usage.expect("usage").total_tokens, 7);
    let requests = calls.lock().expect("calls");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.provider.as_deref(), Some("openai"));
    assert_eq!(request.model, "gpt-boundary");
    assert_eq!(
        request.messages,
        vec![Message::user("Write the runtime note")]
    );
    assert_eq!(request.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(
        request.metadata["spark.runtime.backend"],
        json!("rust_unified_llm_adapter")
    );
    assert_eq!(request.metadata["spark.runtime.source"], json!("codergen"));
    assert_eq!(request.metadata["spark.runtime.provider"], json!("openai"));
    assert_eq!(
        request.metadata["spark.runtime.model"],
        json!("gpt-boundary")
    );
    assert!(!request.metadata.contains_key("spark.runtime.llm_profile"));
    assert_eq!(
        request.metadata["spark.runtime.reasoning_effort"],
        json!("high")
    );
    assert_eq!(
        request.metadata["spark.runtime.response_contract"],
        json!("status_envelope")
    );
    assert_eq!(
        request.metadata["spark.runtime.provider_selector"],
        json!("OpenAI")
    );
}

#[test]
fn codergen_backend_routes_codex_selector_through_app_server() {
    let _lock = CODEX_APP_SERVER_TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("rpc-log.jsonl");
    let _bin_guard = EnvVarGuard::set("SPARK_CODEX_APP_SERVER_BIN", fake_codex_app_server_bin());
    let _mode_guard = EnvVarGuard::set("SPARK_FAKE_CODEX_APP_SERVER_MODE", "default");
    let _log_guard = EnvVarGuard::set("SPARK_FAKE_CODEX_APP_SERVER_LOG", log_path.as_os_str());
    let runtime_root = temp.path().join("codex-runtime");
    let _runtime_guard = EnvVarGuard::set("ATTRACTOR_CODEX_RUNTIME_ROOT", runtime_root.as_os_str());
    let client = Client::new();
    let mut backend = RustLlmCodergenBackend::new(client);

    let output = backend
        .run(CodergenBackendRequest {
            node_id: "task".to_string(),
            prompt: "Write the runtime note".to_string(),
            context: BTreeMap::new(),
            response_contract: String::new(),
            contract_repair_attempts: 0,
            timeout_seconds: None,
            write_contract: Default::default(),
            provider: "codex".to_string(),
            model: Some("gpt-codex-test".to_string()),
            llm_profile: None,
            reasoning_effort: Some("HIGH".to_string()),
            repair_attempt: None,
            runtime_mode: Default::default(),
            project_path: Some(temp.path().to_path_buf()),
            metadata: BTreeMap::new(),
        })
        .expect("codergen output");

    assert_eq!(output.response_text(), "Ack");
    assert_eq!(output.usage.expect("usage").total_tokens, 3);
    assert!(output.events.iter().any(|event| {
        event.event_type == "codex_app_server_session_event"
            && event.payload["turn_stream_event"]["source"]["backend"] == json!("codex_app_server")
            && event.payload["turn_stream_event"]["source"]["app_thread_id"] == json!("thread-test")
            && event.payload["turn_stream_event"]["source"]["app_turn_id"] == json!("turn-test")
    }));
    let messages = fs::read_to_string(&log_path)
        .expect("rpc log")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("json"))
        .collect::<Vec<_>>();
    let methods = messages
        .iter()
        .filter_map(|message| message["method"].as_str().map(str::to_string))
        .collect::<Vec<_>>();
    assert_eq!(
        methods,
        ["initialize", "initialized", "thread/start", "turn/start"]
    );
    let turn_start = messages
        .iter()
        .find(|message| message["method"] == json!("turn/start"))
        .expect("turn/start payload");
    let thread_start = messages
        .iter()
        .find(|message| message["method"] == json!("thread/start"))
        .expect("thread/start payload");
    assert_eq!(thread_start["params"]["ephemeral"], json!(true));
    assert_eq!(turn_start["params"]["effort"], json!("high"));
    assert!(turn_start["params"].get("reasoningEffort").is_none());
    assert_eq!(turn_start["params"]["model"], json!("gpt-codex-test"));
    assert_eq!(
        turn_start["params"]["collaborationMode"],
        json!({
            "mode": "default",
            "settings": {"model": "gpt-codex-test"}
        })
    );
}

#[test]
fn codergen_backend_agent_mode_uses_rust_session_boundary() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(RecordingAdapter::new(
        "openai_compatible",
        Arc::clone(&calls),
    ));
    let client = Client::from_adapters(vec![adapter], None).expect("client");
    let mut backend = RustLlmCodergenBackend::new(client);

    let output = backend
        .run(CodergenBackendRequest {
            node_id: "agent_task".to_string(),
            prompt: "Inspect the workspace before answering".to_string(),
            context: BTreeMap::new(),
            response_contract: "status_envelope".to_string(),
            contract_repair_attempts: 1,
            timeout_seconds: Some(30.0),
            write_contract: Default::default(),
            provider: "openai-compatible".to_string(),
            model: Some("agent-model".to_string()),
            llm_profile: None,
            reasoning_effort: Some("HIGH".to_string()),
            repair_attempt: None,
            runtime_mode: CodergenRuntimeMode {
                requires_tools: true,
                requires_steering: true,
                requires_child_intervention: true,
                requires_session_events: true,
                ..CodergenRuntimeMode::agent()
            },
            project_path: Some("/repo".into()),
            metadata: BTreeMap::from([("caller".to_string(), json!("codergen-test"))]),
        })
        .expect("agent codergen output");

    assert_eq!(output.response_text(), "adapter response for agent-model");
    assert_eq!(output.usage.expect("usage").total_tokens, 7);
    let session_event_categories = output
        .events
        .iter()
        .filter(|event| event.event_type == "rust_agent_session_event")
        .map(|event| event.payload["category"].as_str().unwrap_or(""))
        .collect::<Vec<_>>();
    assert!(session_event_categories.contains(&"lifecycle"));
    assert!(session_event_categories.contains(&"user_input"));
    assert!(session_event_categories.contains(&"assistant_text"));
    assert!(session_event_categories.contains(&"usage"));
    assert!(session_event_categories.contains(&"processing"));
    assert!(output.events.iter().any(|event| {
        event.event_type == "rust_agent_session_event"
            && event.payload["kind"] == json!("session_start")
    }));
    assert!(output.events.iter().any(|event| {
        event.event_type == "rust_agent_session_event"
            && event.payload["kind"] == json!("session_end")
    }));
    assert!(output.events.iter().any(|event| {
        event.event_type == "rust_agent_session_event"
            && event
                .payload
                .get("turn_stream_event")
                .is_some_and(|turn_stream_event| {
                    turn_stream_event["kind"] == json!("content_completed")
                        && turn_stream_event["channel"] == json!("assistant")
                })
    }));
    let completion = output
        .events
        .iter()
        .find(|event| event.event_type == "rust_agent_adapter_request_completed")
        .expect("completion event");
    assert_eq!(
        completion.payload["provider_selector"],
        json!("openai-compatible")
    );
    assert_eq!(completion.payload["provider"], json!("openai_compatible"));
    assert_eq!(completion.payload["model_selector"], json!("agent-model"));
    assert_eq!(completion.payload["model"], json!("agent-model"));
    assert_eq!(completion.payload["reasoning_effort"], json!("high"));
    assert_eq!(
        completion.payload["response_contract"],
        json!("status_envelope")
    );
    assert_eq!(completion.payload["timeout_seconds"], json!(30.0));
    assert_eq!(completion.payload["runtime_mode"]["mode"], json!("agent"));
    let requests = calls.lock().expect("calls");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.provider.as_deref(), Some("openai_compatible"));
    assert_eq!(request.model, "agent-model");
    assert_eq!(request.messages.len(), 2);
    assert!(request.messages[0].text().contains("OpenAI-compatible"));
    assert_eq!(
        request.messages[1],
        Message::user("Inspect the workspace before answering")
    );
    assert_eq!(
        tool_names(request),
        [
            "read_file",
            "apply_patch",
            "write_file",
            "shell",
            "grep",
            "glob",
            "spawn_agent",
            "send_input",
            "wait",
            "close_agent",
        ]
    );
    assert_eq!(request.metadata["caller"], json!("codergen-test"));
    assert_eq!(
        request.metadata["spark.runtime.source"],
        json!("agent_turn")
    );
    assert_eq!(
        request.metadata["spark.runtime.codergen.node_id"],
        json!("agent_task")
    );
    assert_eq!(
        request.metadata["spark.runtime.codergen.response_contract"],
        json!("status_envelope")
    );
    assert_eq!(
        request.metadata["spark.runtime.codergen.timeout_seconds"],
        json!(30.0)
    );
    assert_eq!(
        request.metadata["spark.runtime.codergen.runtime_mode"]["mode"],
        json!("agent")
    );
    assert_eq!(
        request.metadata["spark.runtime.codergen.write_contract"]["allowed_keys"],
        json!([])
    );
}

#[test]
fn codergen_backend_agent_mode_delivers_child_intervention_to_active_session() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let intervention_results = Arc::new(Mutex::new(Vec::new()));
    let broker = CodergenSessionInterventionBroker::default();
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(InterventionDuringStreamAdapter::new(
        "openai",
        Arc::clone(&calls),
        broker.clone(),
        Arc::clone(&intervention_results),
    ));
    let client = Client::from_adapters(vec![adapter], None).expect("client");
    let mut backend = RustLlmCodergenBackend::with_intervention_broker(client, broker.clone());

    let output = backend
        .run(CodergenBackendRequest {
            node_id: "agent_task".to_string(),
            prompt: "Run the implementation task".to_string(),
            context: BTreeMap::new(),
            response_contract: String::new(),
            contract_repair_attempts: 0,
            timeout_seconds: None,
            write_contract: Default::default(),
            provider: "OpenAI".to_string(),
            model: Some("gpt-steer".to_string()),
            llm_profile: None,
            reasoning_effort: Some("medium".to_string()),
            repair_attempt: None,
            runtime_mode: CodergenRuntimeMode::agent(),
            project_path: Some("/repo".into()),
            metadata: BTreeMap::from([
                ("caller".to_string(), json!("intervention-test")),
                ("spark.runtime.run_id".to_string(), json!("child-1")),
                ("spark.runtime.root_run_id".to_string(), json!("root-1")),
            ]),
        })
        .expect("agent codergen output");

    assert_eq!(output.response_text(), "final answer after steering");
    let results = intervention_results.lock().expect("intervention results");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, "delivered");
    assert_eq!(results[0].delivery_mode, "rust_boundary_codergen_turn");
    assert_eq!(results[0].reason, "scope check");
    assert_eq!(results[0].target_node_id.as_deref(), Some("agent_task"));
    drop(results);

    let requests = calls.lock().expect("calls");
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].messages.last(),
        Some(&Message::user("Please keep the current change bounded."))
    );
    drop(requests);

    let steering_event = output
        .events
        .iter()
        .find(|event| {
            event.event_type == "rust_agent_session_event"
                && event.payload["kind"] == json!("steering_injected")
        })
        .expect("steering session event");
    let data = &steering_event.payload["session_event"]["data"];
    assert_eq!(
        data["content"],
        json!("Please keep the current change bounded.")
    );
    assert_eq!(
        data["message"],
        json!("Please keep the current change bounded.")
    );
    assert_eq!(data["child_run_id"], json!("child-1"));
    assert_eq!(data["parent_run_id"], json!("parent-1"));
    assert_eq!(data["parent_node_id"], json!("manager"));
    assert_eq!(data["root_run_id"], json!("root-1"));
    assert_eq!(data["target_node_id"], json!("agent_task"));
    assert_eq!(data["reason"], json!("scope check"));
    assert_eq!(data["source"], json!("manager_loop"));
    assert_eq!(data["cycle"], json!(3));
    assert_eq!(data["provider"], json!("OpenAI"));
    assert_eq!(data["model"], json!("gpt-steer"));
    assert_eq!(data["metadata"]["caller"], json!("intervention-test"));

    let inactive = broker.request_child_intervention(CodergenChildInterventionRequest {
        child_run_id: "child-1".to_string(),
        message: "late".to_string(),
        parent_run_id: "parent-1".to_string(),
        parent_node_id: "manager".to_string(),
        root_run_id: "root-1".to_string(),
        reason: String::new(),
        source: "manager_loop".to_string(),
        cycle: None,
        target_node_id: Some("agent_task".to_string()),
        provider: None,
        model: None,
        llm_profile: None,
        reasoning_effort: None,
    });
    assert_eq!(inactive.status, "rejected");
    assert_eq!(inactive.reason, "backend_steering_unsupported");
}

#[test]
fn codergen_backend_agent_mode_rejects_stale_child_run_intervention() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let intervention_results = Arc::new(Mutex::new(Vec::new()));
    let broker = CodergenSessionInterventionBroker::default();
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(InterventionDuringStreamAdapter::with_request(
            "openai",
            Arc::clone(&calls),
            broker.clone(),
            Arc::clone(&intervention_results),
            intervention_request("stale-child", "root-1", Some("agent_task")),
        ));
    let client = Client::from_adapters(vec![adapter], None).expect("client");
    let mut backend = RustLlmCodergenBackend::with_intervention_broker(client, broker);

    let output = backend
        .run(CodergenBackendRequest {
            node_id: "agent_task".to_string(),
            prompt: "Run the implementation task".to_string(),
            context: BTreeMap::new(),
            response_contract: String::new(),
            contract_repair_attempts: 0,
            timeout_seconds: None,
            write_contract: Default::default(),
            provider: "OpenAI".to_string(),
            model: Some("gpt-steer".to_string()),
            llm_profile: None,
            reasoning_effort: Some("medium".to_string()),
            repair_attempt: None,
            runtime_mode: CodergenRuntimeMode::agent(),
            project_path: Some("/repo".into()),
            metadata: BTreeMap::from([
                ("spark.runtime.run_id".to_string(), json!("child-1")),
                ("spark.runtime.root_run_id".to_string(), json!("root-1")),
            ]),
        })
        .expect("agent codergen output");

    assert_eq!(
        output.response_text(),
        "intermediate answer before steering"
    );
    let results = intervention_results.lock().expect("intervention results");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, "rejected");
    assert_eq!(results[0].reason, "backend_steering_unsupported");
    assert!(results[0].message.contains("intervention child run"));
    assert_eq!(calls.lock().expect("calls").len(), 1);
    assert!(!output.events.iter().any(|event| {
        event.event_type == "rust_agent_session_event"
            && event.payload["kind"] == json!("steering_injected")
    }));
}

#[test]
fn codergen_backend_agent_mode_rejects_target_mismatch_intervention() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let intervention_results = Arc::new(Mutex::new(Vec::new()));
    let broker = CodergenSessionInterventionBroker::default();
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(InterventionDuringStreamAdapter::with_request(
            "openai",
            Arc::clone(&calls),
            broker.clone(),
            Arc::clone(&intervention_results),
            intervention_request("child-1", "root-1", Some("other_task")),
        ));
    let client = Client::from_adapters(vec![adapter], None).expect("client");
    let mut backend = RustLlmCodergenBackend::with_intervention_broker(client, broker);

    let output = backend
        .run(CodergenBackendRequest {
            node_id: "agent_task".to_string(),
            prompt: "Run the implementation task".to_string(),
            context: BTreeMap::new(),
            response_contract: String::new(),
            contract_repair_attempts: 0,
            timeout_seconds: None,
            write_contract: Default::default(),
            provider: "OpenAI".to_string(),
            model: Some("gpt-steer".to_string()),
            llm_profile: None,
            reasoning_effort: Some("medium".to_string()),
            repair_attempt: None,
            runtime_mode: CodergenRuntimeMode::agent(),
            project_path: Some("/repo".into()),
            metadata: BTreeMap::from([
                ("spark.runtime.run_id".to_string(), json!("child-1")),
                ("spark.runtime.root_run_id".to_string(), json!("root-1")),
            ]),
        })
        .expect("agent codergen output");

    assert_eq!(
        output.response_text(),
        "intermediate answer before steering"
    );
    let results = intervention_results.lock().expect("intervention results");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, "rejected");
    assert_eq!(results[0].reason, "backend_steering_unsupported");
    assert_eq!(results[0].target_node_id.as_deref(), Some("other_task"));
    assert!(results[0].message.contains("target node"));
    assert_eq!(calls.lock().expect("calls").len(), 1);
    assert!(!output.events.iter().any(|event| {
        event.event_type == "rust_agent_session_event"
            && event.payload["kind"] == json!("steering_injected")
    }));
}

#[test]
fn codergen_broker_routes_overlapping_active_sessions_by_child_root_and_target() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingAdapter::new("openai", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], None).expect("client");
    let broker = CodergenSessionInterventionBroker::default();
    let mut first_session = codergen_intervention_session("child-1", "root-1", "gpt-first");
    let mut second_session = codergen_intervention_session("child-2", "root-1", "gpt-second");
    let _first_guard = broker.register(active_codergen_session(
        &mut first_session,
        "first_task",
        "child-1",
        "root-1",
        "gpt-first",
    ));
    let _second_guard = broker.register(active_codergen_session(
        &mut second_session,
        "second_task",
        "child-2",
        "root-1",
        "gpt-second",
    ));

    let first_result = broker.request_child_intervention(intervention_request(
        "child-1",
        "root-1",
        Some("first_task"),
    ));
    let second_result = broker.request_child_intervention(intervention_request(
        "child-2",
        "root-1",
        Some("second_task"),
    ));

    assert_eq!(first_result.status, "delivered");
    assert_eq!(second_result.status, "delivered");
    first_session
        .process_input(&client, "start first task")
        .expect("first session completes");
    second_session
        .process_input(&client, "start second task")
        .expect("second session completes");

    let calls = calls.lock().expect("calls");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].model, "gpt-first");
    assert_eq!(
        calls[0].messages.last(),
        Some(&Message::user("Please keep the current change bounded."))
    );
    assert_eq!(calls[1].model, "gpt-second");
    assert_eq!(
        calls[1].messages.last(),
        Some(&Message::user("Please keep the current change bounded."))
    );
}

#[test]
fn codergen_broker_rejects_late_intervention_after_close_error_or_abort_while_registered() {
    #[derive(Clone, Copy)]
    enum TerminalAction {
        Close,
        Error,
        Abort,
    }

    let cases = [
        ("explicit_close", TerminalAction::Close),
        ("unrecoverable_error", TerminalAction::Error),
        ("abort", TerminalAction::Abort),
    ];
    for (case, action) in cases {
        let child_run_id = format!("child-{case}");
        let mut session = codergen_intervention_session(&child_run_id, "root-1", "gpt-stale");
        let broker = CodergenSessionInterventionBroker::default();
        let _guard = broker.register(active_codergen_session(
            &mut session,
            "agent_task",
            &child_run_id,
            "root-1",
            "gpt-stale",
        ));

        match action {
            TerminalAction::Close => session.close(),
            TerminalAction::Error => session.mark_unrecoverable_error("provider failed"),
            TerminalAction::Abort => session.abort(),
        }

        let late = broker.request_child_intervention(intervention_request(
            &child_run_id,
            "root-1",
            Some("agent_task"),
        ));
        assert_eq!(late.status, "rejected", "{case}");
        assert_eq!(late.reason, "backend_steering_unsupported", "{case}");
        assert!(
            late.message.contains("no longer accepting steering"),
            "{case}: {}",
            late.message
        );
    }
}

#[test]
fn codergen_broker_rejects_late_intervention_after_session_completion_while_registered() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingAdapter::new("openai", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], None).expect("client");
    let mut session = Session::new(
        ProviderProfile::new("openai", "gpt-late"),
        ExecutionEnvironment::default().with_metadata(BTreeMap::from([
            ("spark.runtime.run_id".to_string(), json!("child-1")),
            ("spark.runtime.root_run_id".to_string(), json!("root-1")),
        ])),
        SessionConfig::default(),
    );
    let broker = CodergenSessionInterventionBroker::default();
    let _guard = broker.register(ActiveCodergenSession {
        node_id: "agent_task".to_string(),
        child_run_id: Some("child-1".to_string()),
        root_run_id: Some("root-1".to_string()),
        provider: "openai".to_string(),
        model: Some("gpt-late".to_string()),
        llm_profile: None,
        reasoning_effort: None,
        project_path: None,
        metadata: BTreeMap::new(),
        steering: session.steering_handle(),
    });

    session
        .process_input(&client, "finish the task")
        .expect("session completes");
    assert_eq!(session.state, SessionState::Idle);

    let late = broker.request_child_intervention(intervention_request(
        "child-1",
        "root-1",
        Some("agent_task"),
    ));
    assert_eq!(late.status, "rejected");
    assert_eq!(late.reason, "backend_steering_unsupported");
    assert!(late.message.contains("no longer accepting steering"));
    assert_eq!(calls.lock().expect("calls").len(), 1);
    assert!(!session
        .history
        .iter()
        .any(|turn| matches!(turn, HistoryTurn::Steering(_))));
}

#[test]
fn codergen_backend_agent_mode_propagates_session_model_usage_update() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingAdapter::new("openai", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], None).expect("client");
    let mut backend = RustLlmCodergenBackend::new(client);

    let output = backend
        .run(CodergenBackendRequest {
            node_id: "agent_stream_usage".to_string(),
            prompt: "Use the streaming session path".to_string(),
            context: BTreeMap::new(),
            response_contract: String::new(),
            contract_repair_attempts: 0,
            timeout_seconds: None,
            write_contract: Default::default(),
            provider: "OpenAI".to_string(),
            model: Some("gpt-stream".to_string()),
            llm_profile: None,
            reasoning_effort: None,
            repair_attempt: None,
            runtime_mode: CodergenRuntimeMode::agent(),
            project_path: None,
            metadata: BTreeMap::new(),
        })
        .expect("agent codergen output");

    assert_eq!(output.response_text(), "adapter response for gpt-stream");
    assert_eq!(output.usage.expect("usage").total_tokens, 7);
    let usage_event = output
        .events
        .iter()
        .find(|event| {
            event.event_type == "rust_agent_session_event"
                && event.payload["kind"] == json!("model_usage_update")
        })
        .expect("session usage event");
    assert!(usage_event.payload.get("derived_from").is_none());
    assert_eq!(
        usage_event.payload["session_event"]["data"]["usage"]["total_tokens"],
        json!(7)
    );
    assert_eq!(
        usage_event.payload["turn_stream_event"]["token_usage"]["total"]["totalTokens"],
        json!(7)
    );
    assert_eq!(calls.lock().expect("calls").len(), 1);
}

#[test]
fn codergen_backend_agent_mode_uses_model_usage_update_when_final_text_is_empty() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(UsageOnlyStreamAdapter::new("openai", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], None).expect("client");
    let mut backend = RustLlmCodergenBackend::new(client);

    let output = backend
        .run(CodergenBackendRequest {
            node_id: "agent_usage_only".to_string(),
            prompt: "Return usage without content".to_string(),
            context: BTreeMap::new(),
            response_contract: String::new(),
            contract_repair_attempts: 0,
            timeout_seconds: None,
            write_contract: Default::default(),
            provider: "OpenAI".to_string(),
            model: Some("gpt-usage-only".to_string()),
            llm_profile: None,
            reasoning_effort: None,
            repair_attempt: None,
            runtime_mode: CodergenRuntimeMode::agent(),
            project_path: None,
            metadata: BTreeMap::new(),
        })
        .expect("agent codergen output");

    let spark_agent_adapter::codergen::CodergenBackendResponse::Outcome(outcome) = output.response
    else {
        panic!("empty final text should produce an observable failure outcome");
    };
    assert_eq!(outcome.status, attractor_core::OutcomeStatus::Fail);
    assert!(outcome
        .failure_reason
        .contains("completed without assistant text"));
    let usage = output.usage.expect("usage from model_usage_update");
    assert_eq!(usage.input_tokens, 5);
    assert_eq!(usage.total_tokens, 5);
    assert_eq!(usage.cache_write_tokens, Some(6));
    assert_eq!(
        usage.raw.as_ref().expect("raw usage")["cache_write_tokens"],
        json!(6)
    );
    assert_eq!(calls.lock().expect("calls").len(), 1);
}

#[test]
fn codergen_backend_agent_mode_keeps_tool_failures_model_visible_and_distinguishes_tool_events() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(ToolFailureThenAnswerAdapter::new(
        "openai_compatible",
        Arc::clone(&calls),
    ));
    let client = Client::from_adapters(vec![adapter], None).expect("client");
    let mut backend = RustLlmCodergenBackend::new(client);
    let temp = tempfile::tempdir().expect("tempdir");

    let output = backend
        .run(CodergenBackendRequest {
            node_id: "agent_tool_task".to_string(),
            prompt: "Read the missing file and report back".to_string(),
            context: BTreeMap::new(),
            response_contract: "status_envelope".to_string(),
            contract_repair_attempts: 1,
            timeout_seconds: Some(30.0),
            write_contract: Default::default(),
            provider: "openai-compatible".to_string(),
            model: Some("agent-tool-model".to_string()),
            llm_profile: None,
            reasoning_effort: Some("low".to_string()),
            repair_attempt: None,
            runtime_mode: CodergenRuntimeMode::agent(),
            project_path: Some(temp.path().to_path_buf()),
            metadata: BTreeMap::new(),
        })
        .expect("agent codergen output");

    assert_eq!(output.response_text(), "tool failure was visible");
    let requests = calls.lock().expect("calls");
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].messages.last().expect("first user message"),
        &Message::user("Read the missing file and report back")
    );
    let second_request = &requests[1];
    assert_eq!(second_request.messages.len(), 4);
    assert!(second_request.messages.iter().any(|message| {
        message.role == MessageRole::Assistant
            && message.content.iter().any(|part| {
                matches!(
                    part,
                    ContentPart::ToolCall { tool_call }
                        if tool_call.id == "call-missing" && tool_call.name == "read_file"
                )
            })
    }));
    assert!(second_request.messages.iter().any(|message| {
        message.role == MessageRole::Tool
            && message.content.iter().any(|part| {
                matches!(
                    part,
                    ContentPart::ToolResult { tool_result }
                        if tool_result.tool_call_id == "call-missing" && tool_result.is_error
                )
            })
    }));

    let model_tool_event = output
        .events
        .iter()
        .find(|event| {
            event.event_type == "rust_agent_session_event"
                && event.payload["category"] == json!("model_tool_call")
        })
        .expect("model tool event");
    assert_eq!(
        model_tool_event.payload["tool_event"]["kind"],
        json!("model_tool_call")
    );
    assert_eq!(
        model_tool_event.payload["tool_event"]["name"],
        json!("read_file")
    );
    let execution_tool_event = output
        .events
        .iter()
        .find(|event| {
            event.event_type == "rust_agent_session_event"
                && event.payload["category"] == json!("tool_execution")
                && event.payload["kind"] == json!("tool_call_end")
        })
        .expect("tool execution event");
    assert_eq!(
        execution_tool_event.payload["tool_event"]["kind"],
        json!("tool_call")
    );
    assert_eq!(
        execution_tool_event.payload["tool_event"]["status"],
        json!("failed")
    );
    assert!(
        execution_tool_event.payload["session_event"]["data"]["error"]
            .to_string()
            .contains("missing.txt")
    );
}

#[test]
fn codergen_backend_agent_mode_surfaces_provider_failures_with_session_error_events() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(AuthFailingAdapter::new(
        "openai_compatible",
        Arc::clone(&calls),
    ));
    let client = Client::from_adapters(vec![adapter], None).expect("client");
    let mut backend = RustLlmCodergenBackend::new(client);

    let output = backend
        .run(CodergenBackendRequest {
            node_id: "agent_auth_task".to_string(),
            prompt: "Use the agent path".to_string(),
            context: BTreeMap::new(),
            response_contract: "status_envelope".to_string(),
            contract_repair_attempts: 1,
            timeout_seconds: Some(10.0),
            write_contract: Default::default(),
            provider: "openai-compatible".to_string(),
            model: Some("agent-auth-model".to_string()),
            llm_profile: None,
            reasoning_effort: None,
            repair_attempt: None,
            runtime_mode: CodergenRuntimeMode::agent(),
            project_path: None,
            metadata: BTreeMap::new(),
        })
        .expect("provider failure is returned as codergen failure output");

    let spark_agent_adapter::codergen::CodergenBackendResponse::Outcome(outcome) = output.response
    else {
        panic!("provider failure should be a failure outcome");
    };
    assert_eq!(outcome.status, attractor_core::OutcomeStatus::Fail);
    assert_eq!(outcome.retryable, Some(false));
    assert!(outcome
        .failure_reason
        .contains("AuthenticationError: missing API key"));
    let error_event = output
        .events
        .iter()
        .find(|event| {
            event.event_type == "rust_agent_session_event"
                && event.payload["category"] == json!("error")
        })
        .expect("session error event");
    assert_eq!(
        error_event.payload["session_event"]["data"]["error_kind"],
        json!("authentication")
    );
    assert_eq!(
        error_event.payload["session_event"]["data"]["retryable"],
        json!(false)
    );
    assert_eq!(
        error_event.payload["session_event"]["data"]["provider"],
        json!("openai_compatible")
    );
    assert!(output.events.iter().any(|event| {
        event.event_type == "rust_agent_session_event"
            && event.payload["kind"] == json!("session_end")
            && event.payload["session_event"]["data"]["reason"] == json!("unrecoverable_error")
    }));
    assert_eq!(calls.lock().expect("calls").len(), 1);
}

#[test]
fn agent_turn_backend_builds_session_and_preserves_metadata_and_output_contract() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingAdapter::new("openai", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], Some("OPENAI")).expect("client");
    let backend = RustLlmAgentTurnBackend::new(client);

    let output = backend
        .run_turn(AgentTurnRequest {
            conversation_id: "conversation-1".to_string(),
            project_path: "/repo".to_string(),
            prompt: "Plan the next step".to_string(),
            history: Vec::new(),
            provider: None,
            model: Some("gpt-agent".to_string()),
            llm_profile: None,
            reasoning_effort: Some("HIGH".to_string()),
            chat_mode: Some("agent".to_string()),
            metadata: BTreeMap::from([("caller".to_string(), json!("workspace"))]),
        })
        .expect("agent output");

    assert_eq!(
        output.final_assistant_text.as_deref(),
        Some("adapter response for gpt-agent")
    );
    let token_usage = output.token_usage.expect("usage");
    assert_eq!(
        token_usage,
        json!({
            "total": {
                "inputTokens": 3,
                "cachedInputTokens": 3,
                "outputTokens": 4,
                "totalTokens": 7
            }
        })
    );
    assert!(token_usage.get("total_tokens").is_none());
    assert_eq!(
        output.token_usage_breakdown.as_ref(),
        Some(&json!({
            "total": {
                "inputTokens": 3,
                "cachedInputTokens": 3,
                "outputTokens": 4,
                "totalTokens": 7
            }
        }))
    );
    assert!(output.raw_log_lines.is_empty());
    assert!(output.thread_resume_failure.is_none());
    assert_eq!(
        output
            .events
            .iter()
            .map(|event| event.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            TurnStreamEventKind::Other("session_start".to_string()),
            TurnStreamEventKind::Other("assistant_text_start".to_string()),
            TurnStreamEventKind::ContentDelta,
            TurnStreamEventKind::ContentCompleted,
            TurnStreamEventKind::TokenUsageUpdated,
            TurnStreamEventKind::TurnCompleted
        ]
    );
    assert_eq!(
        output.events[0].source.raw_kind.as_deref(),
        Some("session_start")
    );
    assert_eq!(output.events[1].channel, Some(TurnStreamChannel::Assistant));
    assert_eq!(
        output.events[1].source.raw_kind.as_deref(),
        Some("assistant_text_start")
    );
    assert_eq!(output.events[2].channel, Some(TurnStreamChannel::Assistant));
    assert_eq!(
        output.events[2].content_delta.as_deref(),
        Some("adapter response for gpt-agent")
    );
    assert_eq!(
        output.events[2].source.backend.as_deref(),
        Some("agent_session")
    );
    assert_eq!(
        output.events[2].source.raw_kind.as_deref(),
        Some("assistant_text_delta")
    );
    let requests = calls.lock().expect("calls");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.provider.as_deref(), Some("openai"));
    assert_eq!(request.model, "gpt-agent");
    assert_eq!(request.messages.len(), 2);
    assert!(!request.messages[0].text().trim().is_empty());
    assert!(request.messages[0].text().contains("OpenAI coding agent"));
    assert!(request.messages[0].text().contains("apply_patch"));
    assert_eq!(request.messages[1], Message::user("Plan the next step"));
    assert_eq!(
        tool_names(request),
        [
            "read_file",
            "apply_patch",
            "write_file",
            "shell",
            "grep",
            "glob",
            "spawn_agent",
            "send_input",
            "wait",
            "close_agent",
        ]
    );
    assert_eq!(
        request.provider_options,
        BTreeMap::from([(
            "openai".to_string(),
            json!({"reasoning": {"effort": "high"}})
        )])
    );
    assert_eq!(request.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(request.metadata["caller"], json!("workspace"));
    assert_eq!(
        request.metadata["spark.runtime.source"],
        json!("agent_turn")
    );
    assert_eq!(request.metadata["spark.runtime.provider"], json!("openai"));
    assert_eq!(request.metadata["spark.runtime.model"], json!("gpt-agent"));
    assert!(!request.metadata.contains_key("spark.runtime.llm_profile"));
    assert_eq!(
        request.metadata["spark.runtime.reasoning_effort"],
        json!("high")
    );
    assert_eq!(
        request.metadata["spark.runtime.conversation_id"],
        json!("conversation-1")
    );
    assert_eq!(
        request.metadata["spark.runtime.project_path"],
        json!("/repo")
    );
    assert_eq!(request.metadata["spark.runtime.chat_mode"], json!("agent"));
    assert_eq!(
        request.metadata["spark.runtime.model_selector"],
        json!("gpt-agent")
    );
}

#[test]
fn agent_turn_backend_seeds_session_from_request_history_before_current_prompt() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingAdapter::new("openai", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], Some("OPENAI")).expect("client");
    let backend = RustLlmAgentTurnBackend::new(client);

    backend
        .run_turn(AgentTurnRequest {
            conversation_id: "conversation-history".to_string(),
            project_path: "/repo".to_string(),
            prompt: "Current question".to_string(),
            history: vec![
                HistoryTurn::User(UserTurn::new("Previous question")),
                HistoryTurn::Assistant(AssistantTurn::new("Previous answer")),
            ],
            provider: Some("openai".to_string()),
            model: Some("gpt-agent".to_string()),
            llm_profile: None,
            reasoning_effort: None,
            chat_mode: Some("agent".to_string()),
            metadata: BTreeMap::new(),
        })
        .expect("agent output");

    let requests = calls.lock().expect("calls");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.messages.len(), 4);
    assert_eq!(request.messages[0].role, MessageRole::System);
    assert_eq!(request.messages[1], Message::user("Previous question"));
    assert_eq!(request.messages[2], Message::assistant("Previous answer"));
    assert_eq!(request.messages[3], Message::user("Current question"));
}

#[test]
fn agent_turn_backend_answers_request_user_input_through_rust_session_lifecycle() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingAdapter::new("openai", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], Some("OPENAI")).expect("client");
    let backend = RustLlmAgentTurnBackend::new(client);

    let output = backend
        .answer_request_user_input(AgentRequestUserInputAnswerRequest {
            conversation_id: "conversation-answer".to_string(),
            project_path: "/repo".to_string(),
            request_id: "request-1".to_string(),
            assistant_turn_id: "assistant-turn-1".to_string(),
            answers: BTreeMap::from([
                ("constraints".to_string(), "Keep scope narrow".to_string()),
                ("path_choice".to_string(), "Inline card".to_string()),
            ]),
            request_user_input: Some(json!({
                "request_id": "request-1",
                "status": "answered",
                "questions": [
                    {
                        "id": "path_choice",
                        "header": "Path",
                        "question": "Which path should I take?",
                        "question_type": "MULTIPLE_CHOICE",
                        "options": [{"label": "Inline card"}],
                        "allow_other": true,
                        "is_secret": false
                    },
                    {
                        "id": "constraints",
                        "header": "Constraints",
                        "question": "What constraints matter?",
                        "question_type": "FREEFORM",
                        "options": [],
                        "allow_other": false,
                        "is_secret": false
                    }
                ]
            })),
            history: vec![
                HistoryTurn::User(UserTurn::new("Use request_user_input.")),
                HistoryTurn::Assistant(AssistantTurn::new("Which path should I take?")),
            ],
            provider: None,
            model: Some("gpt-agent".to_string()),
            llm_profile: None,
            reasoning_effort: Some("HIGH".to_string()),
            chat_mode: Some("agent".to_string()),
            metadata: BTreeMap::from([("caller".to_string(), json!("workspace"))]),
        })
        .expect("answer output");

    assert_eq!(
        output.final_assistant_text.as_deref(),
        Some("adapter response for gpt-agent")
    );
    assert!(output.thread_resume_failure.is_none());
    assert_eq!(
        output
            .events
            .iter()
            .map(|event| event.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            TurnStreamEventKind::Other("session_start".to_string()),
            TurnStreamEventKind::Other("assistant_text_start".to_string()),
            TurnStreamEventKind::ContentDelta,
            TurnStreamEventKind::ContentCompleted,
            TurnStreamEventKind::TokenUsageUpdated,
            TurnStreamEventKind::TurnCompleted
        ]
    );
    let requests = calls.lock().expect("calls");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.provider.as_deref(), Some("openai"));
    assert_eq!(request.model, "gpt-agent");
    assert_eq!(request.messages.len(), 4);
    assert_eq!(
        request.messages[1],
        Message::user("Use request_user_input.")
    );
    assert_eq!(
        request.messages[2],
        Message::assistant("Which path should I take?")
    );
    assert_eq!(request.messages[3].role, MessageRole::User);
    let answer_message = request.messages[3].text();
    assert!(answer_message.contains("Inline card"));
    assert!(answer_message.contains("Keep scope narrow"));
    assert_eq!(
        request.metadata["spark.runtime.source"],
        json!("request_user_input_answer")
    );
    assert_eq!(
        request.metadata["spark.runtime.request_user_input.request_id"],
        json!("request-1")
    );
    assert_eq!(
        request.metadata["spark.runtime.request_user_input.assistant_turn_id"],
        json!("assistant-turn-1")
    );
    assert_eq!(
        request.metadata["spark.runtime.request_user_input.answers"]["path_choice"],
        json!("Inline card")
    );
    assert_eq!(
        request.metadata["spark.runtime.request_user_input.answers"]["constraints"],
        json!("Keep scope narrow")
    );
    assert_eq!(
        request.metadata["spark.runtime.request_user_input"]["questions"][0]["question"],
        json!("Which path should I take?")
    );
    assert_eq!(
        request.metadata["spark.runtime.request_user_input"]["questions"][1]["question"],
        json!("What constraints matter?")
    );
    assert_eq!(
        request.metadata["spark.runtime.conversation_id"],
        json!("conversation-answer")
    );
    assert_eq!(
        request.metadata["spark.runtime.project_path"],
        json!("/repo")
    );
    assert_eq!(request.metadata["spark.runtime.chat_mode"], json!("agent"));
    assert_eq!(
        request.metadata["spark.runtime.reasoning_effort"],
        json!("high")
    );
    assert_eq!(request.metadata["caller"], json!("workspace"));
}

#[test]
fn agent_turn_backend_answer_preserves_profile_selector_semantics() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(RecordingAdapter::new(
        "openai_compatible",
        Arc::clone(&calls),
    ));
    let client = Client::new()
        .with_llm_profile_adapter(
            "implementation",
            ActiveLlmProfile::new("openai_compatible", Some("local-default".to_string())),
            adapter,
        )
        .expect("profile client");
    let backend = RustLlmAgentTurnBackend::new(client);

    backend
        .answer_request_user_input(AgentRequestUserInputAnswerRequest {
            conversation_id: "conversation-profile-answer".to_string(),
            project_path: "/repo".to_string(),
            request_id: "request-1".to_string(),
            assistant_turn_id: "assistant-turn-profile".to_string(),
            answers: BTreeMap::from([("path_choice".to_string(), "Inline card".to_string())]),
            request_user_input: Some(json!({
                "itemId": "request-1",
                "questions": [
                    {
                        "id": "path_choice",
                        "question": "Which path should I take?",
                        "options": [{"label": "Inline card"}]
                    }
                ]
            })),
            history: vec![HistoryTurn::Assistant(AssistantTurn::new(
                "Which path should I take?",
            ))],
            provider: None,
            model: None,
            llm_profile: Some("implementation".to_string()),
            reasoning_effort: None,
            chat_mode: Some("chat".to_string()),
            metadata: BTreeMap::new(),
        })
        .expect("answer output");

    let requests = calls.lock().expect("calls");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.provider.as_deref(), Some("openai_compatible"));
    assert_eq!(request.model, "local-default");
    assert_eq!(
        tool_names(request),
        [
            "read_file",
            "apply_patch",
            "write_file",
            "shell",
            "grep",
            "glob",
            "spawn_agent",
            "send_input",
            "wait",
            "close_agent",
        ]
    );
    assert_eq!(
        request.provider_options,
        BTreeMap::from([("openai_compatible".to_string(), json!({}))])
    );
    assert_eq!(
        request.metadata["spark.runtime.source"],
        json!("request_user_input_answer")
    );
    assert_eq!(
        request.metadata["spark.runtime.llm_profile"],
        json!("implementation")
    );
    assert_eq!(
        request.metadata["spark.runtime.llm_profile_selector"],
        json!("implementation")
    );
    assert!(request
        .metadata
        .get("spark.runtime.provider_selector")
        .is_none());
}

#[test]
fn agent_turn_backend_starts_codex_workspace_chat_threads_as_durable() {
    let _lock = CODEX_APP_SERVER_TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("rpc-log.jsonl");
    let _bin_guard = EnvVarGuard::set("SPARK_CODEX_APP_SERVER_BIN", fake_codex_app_server_bin());
    let _mode_guard = EnvVarGuard::set("SPARK_FAKE_CODEX_APP_SERVER_MODE", "default");
    let _log_guard = EnvVarGuard::set("SPARK_FAKE_CODEX_APP_SERVER_LOG", log_path.as_os_str());
    let _runtime_guard = EnvVarGuard::set(
        "ATTRACTOR_CODEX_RUNTIME_ROOT",
        temp.path().join("codex-runtime"),
    );
    let backend = RustLlmAgentTurnBackend::new(Client::new());

    let output = backend
        .run_turn(AgentTurnRequest {
            conversation_id: "conversation-chat".to_string(),
            project_path: temp.path().to_string_lossy().into_owned(),
            prompt: "hello".to_string(),
            history: Vec::new(),
            provider: Some("codex".to_string()),
            model: Some("gpt-codex-test".to_string()),
            llm_profile: None,
            reasoning_effort: None,
            chat_mode: Some("chat".to_string()),
            metadata: BTreeMap::new(),
        })
        .expect("codex chat output");

    assert_eq!(output.final_assistant_text.as_deref(), Some("Ack"));
    let messages = fs::read_to_string(&log_path)
        .expect("rpc log")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("json"))
        .collect::<Vec<_>>();
    let thread_start = messages
        .iter()
        .find(|message| message["method"] == json!("thread/start"))
        .expect("thread/start payload");
    assert_eq!(thread_start["params"]["ephemeral"], json!(false));
}

#[test]
fn agent_turn_backend_answer_reports_recoverable_resume_failure_as_output() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingAdapter::new("openai", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], Some("OPENAI")).expect("client");
    let backend = RustLlmAgentTurnBackend::new(client);

    let output = backend
        .answer_request_user_input(AgentRequestUserInputAnswerRequest {
            conversation_id: "conversation-answer".to_string(),
            project_path: "/repo".to_string(),
            request_id: "request-1".to_string(),
            assistant_turn_id: "assistant-turn-1".to_string(),
            answers: BTreeMap::from([("path_choice".to_string(), "Inline card".to_string())]),
            request_user_input: Some(json!({
                "request_id": "request-2",
                "status": "pending",
                "questions": [{"id": "path_choice", "question": "Which path?"}]
            })),
            history: Vec::new(),
            provider: None,
            model: Some("gpt-agent".to_string()),
            llm_profile: None,
            reasoning_effort: None,
            chat_mode: Some("agent".to_string()),
            metadata: BTreeMap::new(),
        })
        .expect("recoverable resume failure output");

    assert!(calls.lock().expect("calls").is_empty());
    let failure = output
        .thread_resume_failure
        .as_ref()
        .expect("thread resume failure");
    assert_eq!(
        failure.error_code.as_deref(),
        Some("request_user_input_id_mismatch")
    );
    assert_eq!(
        failure.details.as_ref().unwrap()["request_id"],
        json!("request-1")
    );
    assert_eq!(output.events.len(), 1);
    assert_eq!(output.events[0].kind, TurnStreamEventKind::Error);
    assert_eq!(
        output.events[0].source.item_id.as_deref(),
        Some("request-1")
    );
    assert_eq!(output.events[0].status.as_deref(), Some("failed"));
}

#[test]
fn agent_turn_backend_does_not_reuse_prior_history_as_current_final_text_after_stream_error() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(ErroringStreamAdapter::new("openai", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], Some("OPENAI")).expect("client");
    let backend = RustLlmAgentTurnBackend::new(client);

    let output = backend
        .run_turn(AgentTurnRequest {
            conversation_id: "conversation-history-error".to_string(),
            project_path: "/repo".to_string(),
            prompt: "Current question".to_string(),
            history: vec![
                HistoryTurn::User(UserTurn::new("Previous question")),
                HistoryTurn::Assistant(AssistantTurn::new("Previous answer")),
            ],
            provider: Some("openai".to_string()),
            model: Some("gpt-agent".to_string()),
            llm_profile: None,
            reasoning_effort: None,
            chat_mode: Some("agent".to_string()),
            metadata: BTreeMap::new(),
        })
        .expect("agent output");

    assert_eq!(calls.lock().expect("calls").len(), 1);
    assert_eq!(output.final_assistant_text, None);
    assert!(output
        .events
        .iter()
        .any(|event| event.kind == TurnStreamEventKind::Error));
    assert_eq!(
        output.token_usage,
        Some(
            json!({"total": {"inputTokens": 1, "cachedInputTokens": 0, "outputTokens": 1, "totalTokens": 2}})
        )
    );
}

#[test]
fn agent_turn_backend_routes_llm_profile_through_session_boundary() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(RecordingAdapter::new(
        "openai_compatible",
        Arc::clone(&calls),
    ));
    let client = Client::new()
        .with_llm_profile_adapter(
            "implementation",
            ActiveLlmProfile::new("openai_compatible", Some("local-default".to_string())),
            adapter,
        )
        .expect("profile client");
    let backend = RustLlmAgentTurnBackend::new(client);

    let output = backend
        .run_turn(AgentTurnRequest {
            conversation_id: "conversation-2".to_string(),
            project_path: "/repo".to_string(),
            prompt: "Use the selected profile".to_string(),
            history: Vec::new(),
            provider: None,
            model: None,
            llm_profile: Some("implementation".to_string()),
            reasoning_effort: None,
            chat_mode: Some("chat".to_string()),
            metadata: BTreeMap::new(),
        })
        .expect("agent output");

    assert_eq!(
        output.final_assistant_text.as_deref(),
        Some("adapter response for local-default")
    );
    let requests = calls.lock().expect("calls");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.provider.as_deref(), Some("openai_compatible"));
    assert_eq!(request.model, "local-default");
    assert_eq!(request.messages.len(), 2);
    assert!(!request.messages[0].text().trim().is_empty());
    assert!(request.messages[0].text().contains("OpenAI-compatible"));
    assert!(request.messages[0].text().contains("apply_patch"));
    assert_eq!(
        request.messages[1],
        Message::user("Use the selected profile")
    );
    assert_eq!(
        tool_names(request),
        [
            "read_file",
            "apply_patch",
            "write_file",
            "shell",
            "grep",
            "glob",
            "spawn_agent",
            "send_input",
            "wait",
            "close_agent",
        ]
    );
    assert_eq!(
        request.metadata["spark.runtime.provider"],
        json!("openai_compatible")
    );
    assert_eq!(
        request.metadata["spark.runtime.llm_profile"],
        json!("implementation")
    );
    assert_eq!(
        request.metadata["spark.runtime.llm_profile_selector"],
        json!("implementation")
    );
    assert_eq!(
        request.metadata["spark.runtime.conversation_id"],
        json!("conversation-2")
    );
}

#[test]
fn agent_turn_backend_uses_anthropic_native_profile_at_adapter_boundary() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingAdapter::new("anthropic", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], None).expect("client");
    let backend = RustLlmAgentTurnBackend::new(client);

    backend
        .run_turn(AgentTurnRequest {
            conversation_id: "conversation-anthropic".to_string(),
            project_path: "/repo".to_string(),
            prompt: "Edit the file".to_string(),
            history: Vec::new(),
            provider: Some("Claude-Code".to_string()),
            model: Some("claude-sonnet-4-5".to_string()),
            llm_profile: None,
            reasoning_effort: None,
            chat_mode: Some("agent".to_string()),
            metadata: BTreeMap::new(),
        })
        .expect("agent output");

    let requests = calls.lock().expect("calls");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.provider.as_deref(), Some("anthropic"));
    assert_eq!(request.model, "claude-sonnet-4-5");
    assert!(request.messages[0]
        .text()
        .contains("Anthropic coding agent"));
    assert!(request.messages[0].text().contains("old_string"));
    assert_eq!(
        tool_names(request),
        [
            "read_file",
            "write_file",
            "edit_file",
            "shell",
            "grep",
            "glob",
            "spawn_agent",
            "send_input",
            "wait",
            "close_agent",
        ]
    );
    assert!(request.tools.iter().all(|tool| tool.name != "apply_patch"));
    assert_eq!(
        tool_parameters(request, "edit_file")["properties"]["old_string"]["type"],
        json!("string")
    );
    assert_eq!(
        tool_parameters(request, "shell")["properties"]["timeout_ms"]["default"],
        json!(120_000)
    );
    assert_eq!(
        request.provider_options,
        BTreeMap::from([("anthropic".to_string(), json!({}))])
    );
}

#[test]
fn agent_turn_backend_uses_gemini_native_profile_at_adapter_boundary() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingAdapter::new("gemini", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], None).expect("client");
    let backend = RustLlmAgentTurnBackend::new(client);

    backend
        .run_turn(AgentTurnRequest {
            conversation_id: "conversation-gemini".to_string(),
            project_path: "/repo".to_string(),
            prompt: "Inspect several files".to_string(),
            history: Vec::new(),
            provider: Some("Google-Gemini".to_string()),
            model: Some("gemini-3.1-pro-preview".to_string()),
            llm_profile: None,
            reasoning_effort: None,
            chat_mode: Some("agent".to_string()),
            metadata: BTreeMap::new(),
        })
        .expect("agent output");

    let requests = calls.lock().expect("calls");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.provider.as_deref(), Some("gemini"));
    assert_eq!(request.model, "gemini-3.1-pro-preview");
    assert!(request.messages[0].text().contains("Gemini coding agent"));
    assert!(request.messages[0].text().contains("GEMINI.md"));
    assert!(request.messages[0]
        .text()
        .contains("thinking configuration"));
    assert_eq!(
        tool_names(request),
        [
            "read_file",
            "read_many_files",
            "write_file",
            "edit_file",
            "shell",
            "grep",
            "glob",
            "list_dir",
            "spawn_agent",
            "send_input",
            "wait",
            "close_agent",
        ]
    );
    assert!(request.tools.iter().all(|tool| tool.name != "apply_patch"));
    assert_eq!(
        tool_parameters(request, "read_many_files")["properties"]["paths"]["type"],
        json!("array")
    );
    assert_eq!(
        tool_parameters(request, "list_dir")["properties"]["depth"]["default"],
        json!(0)
    );
    assert_eq!(
        request.provider_options,
        BTreeMap::from([("gemini".to_string(), json!({}))])
    );
}

#[test]
fn agent_turn_backend_normalizes_openai_compatible_selectors_at_adapter_boundary() {
    for (selector, provider) in [
        ("openai-compatible", "openai_compatible"),
        ("openrouter", "openrouter"),
        ("litellm", "litellm"),
    ] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let adapter: Arc<dyn ProviderAdapter> =
            Arc::new(RecordingAdapter::new(provider, Arc::clone(&calls)));
        let client = Client::from_adapters(vec![adapter], None).expect("client");
        let backend = RustLlmAgentTurnBackend::new(client);

        backend
            .run_turn(AgentTurnRequest {
                conversation_id: format!("conversation-{selector}"),
                project_path: "/repo".to_string(),
                prompt: "Use the compatibility selector".to_string(),
                history: Vec::new(),
                provider: Some(selector.to_string()),
                model: Some("explicit-model".to_string()),
                llm_profile: None,
                reasoning_effort: Some("HIGH".to_string()),
                chat_mode: Some("agent".to_string()),
                metadata: BTreeMap::new(),
            })
            .expect("agent output");

        let requests = calls.lock().expect("calls");
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.provider.as_deref(), Some(provider));
        assert!(request.messages[0].text().contains("OpenAI-compatible"));
        assert_eq!(
            tool_names(request),
            [
                "read_file",
                "apply_patch",
                "write_file",
                "shell",
                "grep",
                "glob",
                "spawn_agent",
                "send_input",
                "wait",
                "close_agent",
            ]
        );
        assert_eq!(
            request.provider_options,
            BTreeMap::from([(provider.to_string(), json!({}))])
        );
        assert_eq!(request.metadata["spark.runtime.provider"], json!(provider));
    }
}

#[test]
fn agent_turn_backend_preserves_configured_profile_ids_before_provider_alias_normalization() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(RecordingAdapter::new(
        "openai_compatible",
        Arc::clone(&calls),
    ));
    let client = Client::new()
        .with_llm_profile_adapter(
            "team-profile",
            ActiveLlmProfile::new("openai_compatible", Some("team-model".to_string())),
            adapter,
        )
        .expect("profile client");
    let backend = RustLlmAgentTurnBackend::new(client);

    backend
        .run_turn(AgentTurnRequest {
            conversation_id: "conversation-team-profile".to_string(),
            project_path: "/repo".to_string(),
            prompt: "Use the configured profile id".to_string(),
            history: Vec::new(),
            provider: Some("Team-Profile".to_string()),
            model: None,
            llm_profile: None,
            reasoning_effort: None,
            chat_mode: Some("agent".to_string()),
            metadata: BTreeMap::new(),
        })
        .expect("agent output");

    let requests = calls.lock().expect("calls");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.provider.as_deref(), Some("openai_compatible"));
    assert_eq!(request.model, "team-model");
    assert_eq!(
        request.metadata["spark.runtime.llm_profile"],
        json!("team-profile")
    );
}

#[test]
fn agent_turn_backend_keys_openai_provider_options_by_native_profile_for_configured_routes() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingAdapter::new("openai", Arc::clone(&calls)));
    let client = Client::new()
        .with_llm_profile_adapter(
            "team-openai",
            ActiveLlmProfile::new("openai", Some("gpt-team".to_string())),
            adapter,
        )
        .expect("profile client");
    let backend = RustLlmAgentTurnBackend::new(client);

    backend
        .run_turn(AgentTurnRequest {
            conversation_id: "conversation-team-openai".to_string(),
            project_path: "/repo".to_string(),
            prompt: "Use the configured OpenAI profile".to_string(),
            history: Vec::new(),
            provider: None,
            model: None,
            llm_profile: Some("team-openai".to_string()),
            reasoning_effort: Some("HIGH".to_string()),
            chat_mode: Some("agent".to_string()),
            metadata: BTreeMap::new(),
        })
        .expect("agent output");

    let requests = calls.lock().expect("calls");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.provider.as_deref(), Some("openai"));
    assert_eq!(request.model, "gpt-team");
    assert!(request.messages[0].text().contains("OpenAI coding agent"));
    assert_eq!(request.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(
        request.provider_options,
        BTreeMap::from([(
            "openai".to_string(),
            json!({"reasoning": {"effort": "high"}})
        )])
    );
    assert_eq!(request.metadata["spark.runtime.provider"], json!("openai"));
    assert_eq!(
        request.metadata["spark.runtime.llm_profile"],
        json!("team-openai")
    );
}

#[test]
fn agent_turn_backend_keys_anthropic_and_gemini_provider_options_by_native_profile_for_configured_routes(
) {
    for (profile_id, provider, model, prompt_marker) in [
        (
            "team-anthropic",
            "anthropic",
            "claude-sonnet-4-5",
            "Anthropic coding agent",
        ),
        (
            "team-gemini",
            "gemini",
            "gemini-3.1-pro-preview",
            "Gemini coding agent",
        ),
    ] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let adapter: Arc<dyn ProviderAdapter> =
            Arc::new(RecordingAdapter::new(provider, Arc::clone(&calls)));
        let client = Client::new()
            .with_llm_profile_adapter(
                profile_id,
                ActiveLlmProfile::new(provider, Some(model.to_string())),
                adapter,
            )
            .expect("profile client");
        let backend = RustLlmAgentTurnBackend::new(client);

        backend
            .run_turn(AgentTurnRequest {
                conversation_id: format!("conversation-{profile_id}"),
                project_path: "/repo".to_string(),
                prompt: "Use the configured native profile".to_string(),
                history: Vec::new(),
                provider: None,
                model: None,
                llm_profile: Some(profile_id.to_string()),
                reasoning_effort: None,
                chat_mode: Some("agent".to_string()),
                metadata: BTreeMap::new(),
            })
            .expect("agent output");

        let requests = calls.lock().expect("calls");
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.provider.as_deref(), Some(provider));
        assert_eq!(request.model, model);
        assert!(request.messages[0].text().contains(prompt_marker));
        assert_eq!(
            request.provider_options,
            BTreeMap::from([(provider.to_string(), json!({}))])
        );
        assert_eq!(request.metadata["spark.runtime.provider"], json!(provider));
        assert_eq!(
            request.metadata["spark.runtime.llm_profile"],
            json!(profile_id)
        );
    }
}

#[test]
fn codergen_backend_routes_provider_profile_selector_through_openai_compatible() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(RecordingAdapter::new(
        "openai_compatible",
        Arc::clone(&calls),
    ));
    let client = Client::new()
        .with_llm_profile_adapter(
            "implementation",
            ActiveLlmProfile::new("openai_compatible", Some("local-default".to_string())),
            adapter,
        )
        .expect("profile client");
    let mut backend = RustLlmCodergenBackend::new(client);

    let output = backend
        .run(CodergenBackendRequest {
            node_id: "task".to_string(),
            prompt: "Use the selected profile".to_string(),
            context: BTreeMap::new(),
            response_contract: String::new(),
            contract_repair_attempts: 0,
            timeout_seconds: None,
            write_contract: Default::default(),
            provider: "Implementation".to_string(),
            model: Some("local-explicit".to_string()),
            llm_profile: None,
            reasoning_effort: None,
            repair_attempt: None,
            runtime_mode: Default::default(),
            project_path: None,
            metadata: BTreeMap::new(),
        })
        .expect("codergen output");

    assert_eq!(
        output.response_text(),
        "adapter response for local-explicit"
    );
    let requests = calls.lock().expect("calls");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.provider.as_deref(), Some("openai_compatible"));
    assert_eq!(request.model, "local-explicit");
    assert_eq!(
        request.metadata["spark.runtime.provider"],
        json!("openai_compatible")
    );
    assert_eq!(
        request.metadata["spark.runtime.llm_profile"],
        json!("implementation")
    );
}

#[test]
fn codergen_backend_routes_codex_provider_with_profile_through_app_server() {
    let _lock = CODEX_APP_SERVER_TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("rpc-log.jsonl");
    let _bin_guard = EnvVarGuard::set("SPARK_CODEX_APP_SERVER_BIN", fake_codex_app_server_bin());
    let _mode_guard = EnvVarGuard::set("SPARK_FAKE_CODEX_APP_SERVER_MODE", "default");
    let _log_guard = EnvVarGuard::set("SPARK_FAKE_CODEX_APP_SERVER_LOG", log_path.as_os_str());
    let runtime_root = temp.path().join("codex-runtime");
    let _runtime_guard = EnvVarGuard::set("ATTRACTOR_CODEX_RUNTIME_ROOT", runtime_root.as_os_str());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(RecordingAdapter::new(
        "openai_compatible",
        Arc::clone(&calls),
    ));
    let client = Client::new()
        .with_llm_profile_adapter(
            "implementation",
            ActiveLlmProfile::new("openai_compatible", Some("local-default".to_string())),
            adapter,
        )
        .expect("profile client");
    let mut backend = RustLlmCodergenBackend::new(client);

    let output = backend
        .run(CodergenBackendRequest {
            node_id: "task".to_string(),
            prompt: "Use Codex even with a profile selector".to_string(),
            context: BTreeMap::new(),
            response_contract: String::new(),
            contract_repair_attempts: 0,
            timeout_seconds: None,
            write_contract: Default::default(),
            provider: "codex".to_string(),
            model: None,
            llm_profile: Some("implementation".to_string()),
            reasoning_effort: None,
            repair_attempt: None,
            runtime_mode: Default::default(),
            project_path: Some(temp.path().to_path_buf()),
            metadata: BTreeMap::new(),
        })
        .expect("codergen output");

    assert_eq!(output.response_text(), "Ack");
    assert!(calls.lock().expect("calls").is_empty());
    let messages = fs::read_to_string(&log_path)
        .expect("rpc log")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("json"))
        .collect::<Vec<_>>();
    let methods = messages
        .iter()
        .filter_map(|message| message["method"].as_str().map(str::to_string))
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
            "mode": "default",
            "settings": {"model": "gpt-codex-test"}
        })
    );
}

#[test]
fn codergen_backend_delivers_child_intervention_to_codex_app_server_turn() {
    let _lock = CODEX_APP_SERVER_TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("rpc-log.jsonl");
    let _bin_guard = EnvVarGuard::set("SPARK_CODEX_APP_SERVER_BIN", fake_codex_app_server_bin());
    let _mode_guard = EnvVarGuard::set("SPARK_FAKE_CODEX_APP_SERVER_MODE", "steerable");
    let _log_guard = EnvVarGuard::set("SPARK_FAKE_CODEX_APP_SERVER_LOG", log_path.as_os_str());
    let runtime_root = temp.path().join("codex-runtime");
    let _runtime_guard = EnvVarGuard::set("ATTRACTOR_CODEX_RUNTIME_ROOT", runtime_root.as_os_str());
    let broker = CodergenSessionInterventionBroker::default();
    let mut backend =
        RustLlmCodergenBackend::with_intervention_broker(Client::new(), broker.clone());
    let project_path = temp.path().to_path_buf();

    let run_handle = thread::spawn(move || {
        backend.run(CodergenBackendRequest {
            node_id: "task".to_string(),
            prompt: "Wait for intervention".to_string(),
            context: BTreeMap::new(),
            response_contract: String::new(),
            contract_repair_attempts: 0,
            timeout_seconds: None,
            write_contract: Default::default(),
            provider: "codex".to_string(),
            model: Some("gpt-codex-test".to_string()),
            llm_profile: None,
            reasoning_effort: None,
            repair_attempt: None,
            runtime_mode: CodergenRuntimeMode::agent(),
            project_path: Some(project_path),
            metadata: BTreeMap::from([
                ("spark.runtime.run_id".to_string(), json!("child-1")),
                ("spark.runtime.root_run_id".to_string(), json!("root-1")),
            ]),
        })
    });

    wait_for_logged_method(&log_path, "turn/start");
    let intervention = broker.request_child_intervention(CodergenChildInterventionRequest {
        child_run_id: "child-1".to_string(),
        message: "Use the intervention".to_string(),
        parent_run_id: "parent-1".to_string(),
        parent_node_id: "manager".to_string(),
        root_run_id: "root-1".to_string(),
        reason: "operator".to_string(),
        source: "test".to_string(),
        cycle: None,
        target_node_id: Some("task".to_string()),
        provider: None,
        model: None,
        llm_profile: None,
        reasoning_effort: None,
    });

    assert_eq!(intervention.status, "delivered");
    assert_eq!(intervention.delivery_mode, "codex_app_server_turn");
    let output = run_handle
        .join()
        .expect("codex codergen thread")
        .expect("codex codergen output");
    assert_eq!(output.response_text(), "Steered");
    let messages = fs::read_to_string(&log_path)
        .expect("rpc log")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("json"))
        .collect::<Vec<_>>();
    let turn_steer = messages
        .iter()
        .find(|message| message["method"] == json!("turn/steer"))
        .expect("turn/steer");
    assert_eq!(turn_steer["params"]["threadId"], json!("thread-steer"));
    assert_eq!(turn_steer["params"]["expectedTurnId"], json!("turn-steer"));
    assert_eq!(
        turn_steer["params"]["input"][0]["text"],
        json!("Use the intervention")
    );
}

#[test]
fn codergen_backend_requires_explicit_model_or_profile_default_for_profiles() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(RecordingAdapter::new(
        "openai_compatible",
        Arc::clone(&calls),
    ));
    let client = Client::new()
        .with_llm_profile_adapter(
            "implementation",
            ActiveLlmProfile::new("openai_compatible", None),
            adapter,
        )
        .expect("profile client");
    let mut backend = RustLlmCodergenBackend::new(client);

    let error = backend
        .run(CodergenBackendRequest {
            node_id: "task".to_string(),
            prompt: "Need a model".to_string(),
            context: BTreeMap::new(),
            response_contract: String::new(),
            contract_repair_attempts: 0,
            timeout_seconds: None,
            write_contract: Default::default(),
            provider: "implementation".to_string(),
            model: None,
            llm_profile: None,
            reasoning_effort: None,
            repair_attempt: None,
            runtime_mode: Default::default(),
            project_path: None,
            metadata: BTreeMap::new(),
        })
        .expect_err("configuration error");

    assert!(error
        .to_string()
        .contains("ConfigurationError: No model configured"));
    assert!(calls.lock().expect("calls").is_empty());
}

#[test]
fn codergen_backend_reports_missing_profile_configuration() {
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(RecordingAdapter::new(
        "openai",
        Arc::new(Mutex::new(Vec::new())),
    ));
    let client = Client::from_adapters(vec![adapter], None).expect("client");
    let mut backend = RustLlmCodergenBackend::new(client);

    let error = backend
        .run(CodergenBackendRequest {
            node_id: "task".to_string(),
            prompt: "Use missing profile".to_string(),
            context: BTreeMap::new(),
            response_contract: String::new(),
            contract_repair_attempts: 0,
            timeout_seconds: None,
            write_contract: Default::default(),
            provider: "OpenAI".to_string(),
            model: Some("gpt-boundary".to_string()),
            llm_profile: Some("missing".to_string()),
            reasoning_effort: None,
            repair_attempt: None,
            runtime_mode: Default::default(),
            project_path: None,
            metadata: BTreeMap::new(),
        })
        .expect_err("missing profile");

    assert!(error
        .to_string()
        .contains("ConfigurationError: LLM profile 'missing' was not found."));
}

trait ResponseText {
    fn response_text(&self) -> String;
}

impl ResponseText for spark_agent_adapter::CodergenBackendOutput {
    fn response_text(&self) -> String {
        match &self.response {
            spark_agent_adapter::codergen::CodergenBackendResponse::Text(text) => text.clone(),
            spark_agent_adapter::codergen::CodergenBackendResponse::Boolean(value) => {
                value.to_string()
            }
            spark_agent_adapter::codergen::CodergenBackendResponse::Outcome(outcome) => {
                outcome.notes.clone()
            }
        }
    }
}

struct RecordingAdapter {
    name: &'static str,
    calls: Arc<Mutex<Vec<Request>>>,
}

impl RecordingAdapter {
    fn new(name: &'static str, calls: Arc<Mutex<Vec<Request>>>) -> Self {
        Self { name, calls }
    }
}

impl ProviderAdapter for RecordingAdapter {
    fn name(&self) -> &str {
        self.name
    }

    fn complete(&self, request: Request) -> Result<Response, AdapterError> {
        self.calls.lock().expect("calls").push(request.clone());
        Ok(Response {
            model: request.model.clone(),
            provider: request.provider.clone().unwrap_or_default(),
            message: Message::assistant(format!("adapter response for {}", request.model)),
            finish_reason: FinishReason::Stop,
            usage: Usage {
                input_tokens: 3,
                output_tokens: 4,
                total_tokens: 7,
                cache_read_tokens: Some(5),
                ..Usage::default()
            },
            ..Response::default()
        })
    }

    fn stream(&self, request: Request) -> Result<StreamEvents, AdapterError> {
        self.calls.lock().expect("calls").push(request.clone());
        Ok(stream_events(
            vec![
                Ok(StreamEvent::text_delta(format!(
                    "adapter response for {}",
                    request.model
                ))),
                Ok(StreamEvent::finish(
                    FinishReason::Stop,
                    Some(Usage {
                        input_tokens: 3,
                        output_tokens: 4,
                        total_tokens: 7,
                        cache_read_tokens: Some(5),
                        ..Usage::default()
                    }),
                )),
            ]
            .into_iter(),
        ))
    }
}

struct InterventionDuringStreamAdapter {
    name: &'static str,
    calls: Arc<Mutex<Vec<Request>>>,
    broker: CodergenSessionInterventionBroker,
    intervention_results: Arc<Mutex<Vec<spark_agent_adapter::CodergenChildInterventionResult>>>,
    request: CodergenChildInterventionRequest,
}

impl InterventionDuringStreamAdapter {
    fn new(
        name: &'static str,
        calls: Arc<Mutex<Vec<Request>>>,
        broker: CodergenSessionInterventionBroker,
        intervention_results: Arc<Mutex<Vec<spark_agent_adapter::CodergenChildInterventionResult>>>,
    ) -> Self {
        Self::with_request(
            name,
            calls,
            broker,
            intervention_results,
            intervention_request("child-1", "root-1", Some("agent_task")),
        )
    }

    fn with_request(
        name: &'static str,
        calls: Arc<Mutex<Vec<Request>>>,
        broker: CodergenSessionInterventionBroker,
        intervention_results: Arc<Mutex<Vec<spark_agent_adapter::CodergenChildInterventionResult>>>,
        request: CodergenChildInterventionRequest,
    ) -> Self {
        Self {
            name,
            calls,
            broker,
            intervention_results,
            request,
        }
    }
}

fn intervention_request(
    child_run_id: &str,
    root_run_id: &str,
    target_node_id: Option<&str>,
) -> CodergenChildInterventionRequest {
    CodergenChildInterventionRequest {
        child_run_id: child_run_id.to_string(),
        message: "Please keep the current change bounded.".to_string(),
        parent_run_id: "parent-1".to_string(),
        parent_node_id: "manager".to_string(),
        root_run_id: root_run_id.to_string(),
        reason: "scope check".to_string(),
        source: "manager_loop".to_string(),
        cycle: Some(3),
        target_node_id: target_node_id.map(str::to_string),
        provider: None,
        model: None,
        llm_profile: None,
        reasoning_effort: None,
    }
}

fn codergen_intervention_session(child_run_id: &str, root_run_id: &str, model: &str) -> Session {
    Session::new(
        ProviderProfile::new("openai", model),
        ExecutionEnvironment::default().with_metadata(BTreeMap::from([
            ("spark.runtime.run_id".to_string(), json!(child_run_id)),
            ("spark.runtime.root_run_id".to_string(), json!(root_run_id)),
        ])),
        SessionConfig::default(),
    )
}

fn active_codergen_session(
    session: &mut Session,
    node_id: &str,
    child_run_id: &str,
    root_run_id: &str,
    model: &str,
) -> ActiveCodergenSession {
    ActiveCodergenSession {
        node_id: node_id.to_string(),
        child_run_id: Some(child_run_id.to_string()),
        root_run_id: Some(root_run_id.to_string()),
        provider: "openai".to_string(),
        model: Some(model.to_string()),
        llm_profile: None,
        reasoning_effort: None,
        project_path: None,
        metadata: BTreeMap::new(),
        steering: session.steering_handle(),
    }
}

impl ProviderAdapter for InterventionDuringStreamAdapter {
    fn name(&self) -> &str {
        self.name
    }

    fn complete(&self, request: Request) -> Result<Response, AdapterError> {
        let call_index = {
            let mut calls = self.calls.lock().expect("calls");
            calls.push(request.clone());
            calls.len()
        };
        if call_index == 1 {
            let result = self.broker.request_child_intervention(self.request.clone());
            self.intervention_results
                .lock()
                .expect("intervention results")
                .push(result);
            return Ok(Response {
                model: request.model.clone(),
                provider: request.provider.clone().unwrap_or_default(),
                message: Message::assistant("intermediate answer before steering"),
                finish_reason: FinishReason::Stop,
                usage: Usage {
                    input_tokens: 2,
                    output_tokens: 3,
                    total_tokens: 5,
                    ..Usage::default()
                },
                ..Response::default()
            });
        }

        Ok(Response {
            model: request.model.clone(),
            provider: request.provider.clone().unwrap_or_default(),
            message: Message::assistant("final answer after steering"),
            finish_reason: FinishReason::Stop,
            usage: Usage {
                input_tokens: 4,
                output_tokens: 5,
                total_tokens: 9,
                ..Usage::default()
            },
            ..Response::default()
        })
    }

    fn stream(&self, request: Request) -> Result<StreamEvents, AdapterError> {
        self.complete(request).map(|response| {
            stream_events(
                vec![
                    Ok(StreamEvent::text_delta(response.text())),
                    Ok(StreamEvent::finish(
                        response.finish_reason,
                        Some(response.usage),
                    )),
                ]
                .into_iter(),
            )
        })
    }
}

struct ToolFailureThenAnswerAdapter {
    name: &'static str,
    calls: Arc<Mutex<Vec<Request>>>,
}

impl ToolFailureThenAnswerAdapter {
    fn new(name: &'static str, calls: Arc<Mutex<Vec<Request>>>) -> Self {
        Self { name, calls }
    }
}

impl ProviderAdapter for ToolFailureThenAnswerAdapter {
    fn name(&self) -> &str {
        self.name
    }

    fn complete(&self, request: Request) -> Result<Response, AdapterError> {
        let call_index = {
            let mut calls = self.calls.lock().expect("calls");
            calls.push(request.clone());
            calls.len()
        };
        if call_index == 1 {
            return Ok(Response {
                model: request.model.clone(),
                provider: request.provider.clone().unwrap_or_default(),
                message: Message::assistant("Need to inspect a file"),
                tool_calls: vec![ToolCall::new(
                    "call-missing",
                    "read_file",
                    json!({"path": "missing.txt"}),
                )],
                finish_reason: FinishReason::ToolCalls,
                usage: Usage {
                    input_tokens: 2,
                    output_tokens: 3,
                    total_tokens: 5,
                    ..Usage::default()
                },
                ..Response::default()
            });
        }

        Ok(Response {
            model: request.model.clone(),
            provider: request.provider.clone().unwrap_or_default(),
            message: Message::assistant("tool failure was visible"),
            finish_reason: FinishReason::Stop,
            usage: Usage {
                input_tokens: 4,
                output_tokens: 5,
                total_tokens: 9,
                ..Usage::default()
            },
            ..Response::default()
        })
    }

    fn stream(&self, request: Request) -> Result<StreamEvents, AdapterError> {
        self.complete(request).map(|response| {
            stream_events(
                vec![
                    Ok(StreamEvent::text_delta(response.text())),
                    Ok(StreamEvent::finish(
                        response.finish_reason,
                        Some(response.usage),
                    )),
                ]
                .into_iter(),
            )
        })
    }
}

struct AuthFailingAdapter {
    name: &'static str,
    calls: Arc<Mutex<Vec<Request>>>,
}

impl AuthFailingAdapter {
    fn new(name: &'static str, calls: Arc<Mutex<Vec<Request>>>) -> Self {
        Self { name, calls }
    }
}

impl ProviderAdapter for AuthFailingAdapter {
    fn name(&self) -> &str {
        self.name
    }

    fn complete(&self, request: Request) -> Result<Response, AdapterError> {
        self.calls.lock().expect("calls").push(request);
        let mut error = AdapterError::provider(
            AdapterErrorKind::Authentication,
            "missing API key",
            Some(self.name.to_string()),
        );
        error.error_code = Some("missing_api_key".to_string());
        Err(error)
    }

    fn stream(&self, request: Request) -> Result<StreamEvents, AdapterError> {
        self.complete(request)
            .map(|_| stream_events(Vec::new().into_iter()))
    }
}

struct UsageOnlyStreamAdapter {
    name: &'static str,
    calls: Arc<Mutex<Vec<Request>>>,
}

impl UsageOnlyStreamAdapter {
    fn new(name: &'static str, calls: Arc<Mutex<Vec<Request>>>) -> Self {
        Self { name, calls }
    }
}

impl ProviderAdapter for UsageOnlyStreamAdapter {
    fn name(&self) -> &str {
        self.name
    }

    fn complete(&self, request: Request) -> Result<Response, AdapterError> {
        self.calls.lock().expect("calls").push(request.clone());
        Ok(Response {
            model: request.model.clone(),
            provider: request.provider.clone().unwrap_or_default(),
            finish_reason: FinishReason::Stop,
            usage: Usage {
                input_tokens: 5,
                output_tokens: 0,
                total_tokens: 5,
                cache_write_tokens: Some(6),
                ..Usage::default()
            },
            ..Response::default()
        })
    }

    fn stream(&self, request: Request) -> Result<StreamEvents, AdapterError> {
        self.calls.lock().expect("calls").push(request);
        Ok(stream_events(
            vec![Ok(StreamEvent::finish(
                FinishReason::Stop,
                Some(Usage {
                    input_tokens: 5,
                    output_tokens: 0,
                    total_tokens: 5,
                    cache_write_tokens: Some(6),
                    ..Usage::default()
                }),
            ))]
            .into_iter(),
        ))
    }
}

struct ErroringStreamAdapter {
    name: &'static str,
    calls: Arc<Mutex<Vec<Request>>>,
}

impl ErroringStreamAdapter {
    fn new(name: &'static str, calls: Arc<Mutex<Vec<Request>>>) -> Self {
        Self { name, calls }
    }
}

impl ProviderAdapter for ErroringStreamAdapter {
    fn name(&self) -> &str {
        self.name
    }

    fn complete(&self, request: Request) -> Result<Response, AdapterError> {
        self.calls.lock().expect("calls").push(request.clone());
        Ok(Response {
            model: request.model.clone(),
            provider: request.provider.clone().unwrap_or_default(),
            message: Message::assistant("unused non-streaming response"),
            finish_reason: FinishReason::Stop,
            ..Response::default()
        })
    }

    fn stream(&self, request: Request) -> Result<StreamEvents, AdapterError> {
        self.calls.lock().expect("calls").push(request);
        Ok(stream_events(
            vec![Ok(StreamEvent {
                error: Some(AdapterError::new(AdapterErrorKind::Stream, "stream failed")),
                usage: Some(Usage {
                    input_tokens: 1,
                    output_tokens: 1,
                    total_tokens: 2,
                    ..Usage::default()
                }),
                raw: Some(json!({"error": "stream failed"})),
                ..StreamEvent::new(StreamEventType::Error)
            })]
            .into_iter(),
        ))
    }
}

#[test]
fn non_codex_turns_stream_the_same_events_the_batch_returns() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> =
        Arc::new(RecordingAdapter::new("openai", Arc::clone(&calls)));
    let client = Client::from_adapters(vec![adapter], Some("OPENAI")).expect("client");
    let backend = RustLlmAgentTurnBackend::new(client);

    let streamed = Arc::new(Mutex::new(Vec::new()));
    let sink_events = Arc::clone(&streamed);
    let output = backend
        .run_turn_with_event_sink(
            AgentTurnRequest {
                conversation_id: "conversation-stream".to_string(),
                project_path: "/repo".to_string(),
                prompt: "Stream this turn".to_string(),
                history: Vec::new(),
                provider: None,
                model: Some("gpt-agent".to_string()),
                llm_profile: None,
                reasoning_effort: None,
                chat_mode: Some("agent".to_string()),
                metadata: BTreeMap::new(),
            },
            Some(Arc::new(move |event| {
                sink_events.lock().expect("streamed").push(event);
            })),
        )
        .expect("agent output");

    let streamed = streamed.lock().expect("streamed").clone();
    assert!(
        !streamed.is_empty(),
        "the live sink must receive events during a non-codex turn",
    );
    // Live-plus-batch parity, mirroring the codex backend: every event the
    // durable batch carries was also delivered to the sink, in order.
    assert_eq!(streamed, output.events);
}
