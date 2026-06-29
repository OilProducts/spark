use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use spark_agent_adapter::{
    AgentTurnBackend, AgentTurnRequest, CodergenBackend, CodergenBackendRequest,
    RustLlmAgentTurnBackend, RustLlmCodergenBackend,
};
use spark_common::events::{TurnStreamChannel, TurnStreamEventKind};
use unified_llm_adapter::{
    stream_events, ActiveLlmProfile, AdapterError, Client, FinishReason, Message, ProviderAdapter,
    Request, Response, StreamEvent, StreamEvents, Usage,
};

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
}

#[test]
fn codergen_backend_routes_codex_selector_through_openai_compatible() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(RecordingAdapter::new(
        "openai_compatible",
        Arc::clone(&calls),
    ));
    let client = Client::from_adapters(vec![adapter], None).expect("client");
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
            model: Some("openai:gpt-5.2".to_string()),
            llm_profile: None,
            reasoning_effort: None,
            repair_attempt: None,
        })
        .expect("codergen output");

    assert_eq!(
        output.response_text(),
        "adapter response for openai:gpt-5.2"
    );
    let requests = calls.lock().expect("calls");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].provider.as_deref(), Some("openai_compatible"));
    assert_eq!(
        requests[0].metadata["spark.runtime.provider"],
        json!("openai_compatible")
    );
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
    assert!(output.raw_log_lines.is_empty());
    assert!(output.thread_resume_failure.is_none());
    assert_eq!(
        output
            .events
            .iter()
            .map(|event| event.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            TurnStreamEventKind::ContentDelta,
            TurnStreamEventKind::ContentCompleted,
            TurnStreamEventKind::TokenUsageUpdated,
            TurnStreamEventKind::TurnCompleted
        ]
    );
    assert_eq!(output.events[0].channel, Some(TurnStreamChannel::Assistant));
    assert_eq!(
        output.events[0].content_delta.as_deref(),
        Some("adapter response for gpt-agent")
    );
    assert_eq!(
        output.events[0].source.backend.as_deref(),
        Some("agent_session")
    );
    assert_eq!(
        output.events[0].source.raw_kind.as_deref(),
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
            provider: Some("codex".to_string()),
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
        ("codex", "openai_compatible"),
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
fn codergen_backend_uses_profile_default_before_building_low_level_request() {
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

    backend
        .run(CodergenBackendRequest {
            node_id: "task".to_string(),
            prompt: "Use the profile default".to_string(),
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
        })
        .expect("codergen output");

    let requests = calls.lock().expect("calls");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].provider.as_deref(), Some("openai_compatible"));
    assert_eq!(requests[0].model, "local-default");
    assert_eq!(
        requests[0].metadata["spark.runtime.llm_profile"],
        json!("implementation")
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
            provider: "codex".to_string(),
            model: None,
            llm_profile: Some("implementation".to_string()),
            reasoning_effort: None,
            repair_attempt: None,
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
