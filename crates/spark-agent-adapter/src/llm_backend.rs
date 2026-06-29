use std::collections::BTreeMap;

use serde_json::{json, Map, Value};
use unified_llm_adapter::{
    is_display_model_placeholder, resolve_high_level_provider_and_model, AdapterError, Client,
    HighLevelLlmResolutionInputs, LlmProfileRoute, Message, ModelCapabilities, Request,
};

use crate::agent::{
    AgentError, AgentRawLogLine, AgentThreadResumeFailure, AgentTurnBackend, AgentTurnOutput,
    AgentTurnRequest,
};
use crate::codergen::{
    CodergenBackend, CodergenBackendOutput, CodergenBackendRequest, CodergenBackendResponse,
    CodergenError, CodergenEvent,
};
use crate::config::SessionConfig;
use crate::environment::ExecutionEnvironment;
use crate::events::{workspace_token_usage_payload_from_usage, SessionEvent};
use crate::history::HistoryTurn;
use crate::profiles::{
    create_provider_profile, normalize_provider_selector as normalize_profile_selector,
};
use crate::session::Session;

const BACKEND_NAME: &str = "rust_unified_llm_adapter";
const PROVIDER_PLACEHOLDERS: &[&str] = &["codex default (config/profile)"];

#[derive(Clone)]
pub struct RustLlmCodergenBackend {
    client: Client,
}

impl RustLlmCodergenBackend {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

impl CodergenBackend for RustLlmCodergenBackend {
    fn run(
        &mut self,
        request: CodergenBackendRequest,
    ) -> Result<CodergenBackendOutput, CodergenError> {
        let profile = normalize_optional(request.llm_profile.as_deref());
        let reasoning_effort = normalize_lower_optional(request.reasoning_effort.as_deref());
        let metadata_response_contract =
            normalize_optional(Some(request.response_contract.as_str()));
        let llm_request = build_llm_request(
            &self.client,
            vec![Message::user(request.prompt.clone())],
            RequestSelection {
                provider: normalize_request_provider_selector(&request.provider),
                model: normalize_model_selector(request.model.as_deref()),
                llm_profile: profile.clone(),
                reasoning_effort: reasoning_effort.clone(),
                required_capabilities: capabilities_for_codergen(&request),
            },
            llm_metadata(
                "codergen",
                [
                    ("node_id", Some(request.node_id.as_str())),
                    ("response_contract", metadata_response_contract.as_deref()),
                ],
            ),
        )
        .map_err(codergen_adapter_error)?;
        let response = self
            .client
            .complete(llm_request.request)
            .map_err(codergen_adapter_error)?;

        Ok(CodergenBackendOutput {
            response: CodergenBackendResponse::Text(response.text()),
            events: vec![CodergenEvent::new(
                "rust_llm_adapter_request_completed",
                BTreeMap::from([
                    ("backend".to_string(), json!(BACKEND_NAME)),
                    ("node_id".to_string(), json!(request.node_id)),
                    ("provider".to_string(), json!(response.provider)),
                    ("model".to_string(), json!(response.model)),
                    (
                        "llm_profile".to_string(),
                        json!(llm_request.llm_profile.clone()),
                    ),
                    (
                        "reasoning_effort".to_string(),
                        json!(llm_request.reasoning_effort.clone()),
                    ),
                ]),
            )],
            usage: Some(response.usage),
        })
    }
}

#[derive(Clone)]
pub struct RustLlmAgentTurnBackend {
    client: Client,
}

impl RustLlmAgentTurnBackend {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

impl AgentTurnBackend for RustLlmAgentTurnBackend {
    fn run_turn(&self, request: AgentTurnRequest) -> Result<AgentTurnOutput, AgentError> {
        let prompt = request.prompt.clone();
        let mut session =
            build_agent_session(&self.client, request).map_err(agent_adapter_error)?;
        session
            .process_input(&self.client, prompt)
            .map_err(agent_adapter_error)?;

        Ok(agent_output_from_session(&mut session))
    }
}

#[derive(Debug, Clone)]
struct AgentSessionSelection {
    provider: Option<String>,
    model: Option<String>,
    llm_profile: Option<String>,
    reasoning_effort: Option<String>,
}

fn build_agent_session(
    client: &Client,
    request: AgentTurnRequest,
) -> Result<Session, AdapterError> {
    let AgentTurnRequest {
        conversation_id,
        project_path,
        prompt: _,
        provider,
        model,
        llm_profile,
        reasoning_effort,
        chat_mode,
        metadata,
    } = request;

    let selection = AgentSessionSelection {
        provider: normalize_request_provider_selector(provider.as_deref().unwrap_or("")),
        model: normalize_model_selector(model.as_deref()),
        llm_profile: normalize_optional(llm_profile.as_deref()),
        reasoning_effort: normalize_lower_optional(reasoning_effort.as_deref()),
    };
    let mut metadata = metadata;
    let metadata_chat_mode = normalize_optional(chat_mode.as_deref());
    let metadata_provider = normalize_optional(provider.as_deref());
    let metadata_model = normalize_optional(model.as_deref());
    let metadata_llm_profile = normalize_optional(llm_profile.as_deref());
    metadata.extend(llm_metadata(
        "agent_turn",
        [
            ("conversation_id", Some(conversation_id.as_str())),
            ("project_path", Some(project_path.as_str())),
            ("chat_mode", metadata_chat_mode.as_deref()),
            ("provider_selector", metadata_provider.as_deref()),
            ("model_selector", metadata_model.as_deref()),
            ("llm_profile_selector", metadata_llm_profile.as_deref()),
        ],
    ));
    let llm_request = build_llm_request(
        client,
        Vec::new(),
        RequestSelection {
            provider: selection.provider.clone(),
            model: selection.model.clone(),
            llm_profile: selection.llm_profile.clone(),
            reasoning_effort: selection.reasoning_effort.clone(),
            required_capabilities: ModelCapabilities::default(),
        },
        metadata,
    )?;

    let request_provider = llm_request.request.provider.clone().unwrap_or_default();
    let mut profile = create_provider_profile(
        &llm_request.provider_profile_selector,
        llm_request.request.model.clone(),
    );
    if !request_provider.trim().is_empty() && request_provider.trim() != profile.id {
        profile.request_provider = Some(request_provider);
    }
    profile.provider_options = llm_request.request.provider_options.clone();
    let execution_environment =
        ExecutionEnvironment::local(project_path).with_metadata(llm_request.request.metadata);
    let config = SessionConfig {
        reasoning_effort: selection.reasoning_effort,
        ..SessionConfig::default()
    };

    Ok(Session::new(profile, execution_environment, config))
}

fn agent_output_from_session(session: &mut Session) -> AgentTurnOutput {
    let mut output = AgentTurnOutput::default();
    while let Some(event) = session.next_event() {
        if let Some(raw_line) = raw_log_line_from_event(&event) {
            output.raw_log_lines.push(raw_line);
        }
        if let Some(failure) = thread_resume_failure_from_event(&event) {
            output.thread_resume_failure = Some(failure);
        }
        if let Some(turn_stream_event) = event.to_turn_stream_event() {
            if let Some(token_usage) = turn_stream_event.token_usage.as_ref() {
                output.token_usage = Some(token_usage.clone());
            }
            output.events.push(turn_stream_event);
        }
    }

    if let Some(assistant) = session.history.iter().rev().find_map(|turn| match turn {
        HistoryTurn::Assistant(assistant) => Some(assistant),
        _ => None,
    }) {
        output.final_assistant_text = Some(assistant.text());
        if output.token_usage.is_none() {
            output.token_usage = assistant
                .usage
                .as_ref()
                .and_then(workspace_token_usage_payload_from_usage);
        }
    }

    output
}

fn raw_log_line_from_event(event: &SessionEvent) -> Option<AgentRawLogLine> {
    if event.kind.as_str() != "raw_log_line" && !event.data.contains_key("raw_log_line") {
        return None;
    }

    let nested = event.data.get("raw_log_line").and_then(Value::as_object);
    let direction = object_string(nested, &["direction"])
        .or_else(|| data_string(&event.data, &["direction"]))
        .unwrap_or_else(|| "incoming".to_string());
    let line = object_string(nested, &["line"])
        .or_else(|| data_string(&event.data, &["line"]))
        .or_else(|| event.data.get("raw").map(Value::to_string))?;

    Some(AgentRawLogLine { direction, line })
}

fn thread_resume_failure_from_event(event: &SessionEvent) -> Option<AgentThreadResumeFailure> {
    if event.kind.as_str() != "thread_resume_failure"
        && !event.data.contains_key("thread_resume_failure")
    {
        return None;
    }

    let nested = event
        .data
        .get("thread_resume_failure")
        .and_then(Value::as_object);
    let message = object_string(nested, &["message"])
        .or_else(|| data_string(&event.data, &["message", "error"]))
        .unwrap_or_else(|| "thread could not resume".to_string());
    let error_code = object_string(nested, &["error_code"])
        .or_else(|| data_string(&event.data, &["error_code"]));
    let details = nested
        .and_then(|object| object.get("details").cloned())
        .or_else(|| event.data.get("details").cloned());

    Some(AgentThreadResumeFailure {
        message,
        error_code,
        details,
    })
}

#[derive(Debug, Clone)]
struct RequestSelection {
    provider: Option<String>,
    model: Option<String>,
    llm_profile: Option<String>,
    reasoning_effort: Option<String>,
    required_capabilities: ModelCapabilities,
}

struct BuiltLlmRequest {
    request: Request,
    llm_profile: Option<String>,
    reasoning_effort: Option<String>,
    provider_profile_selector: String,
}

fn build_llm_request(
    client: &Client,
    messages: Vec<Message>,
    selection: RequestSelection,
    mut metadata: BTreeMap<String, Value>,
) -> Result<BuiltLlmRequest, AdapterError> {
    let selected_profile = selected_profile_for_request(
        client,
        selection.provider.as_deref(),
        selection.llm_profile.as_deref(),
    )?;
    let active_profile = selected_profile
        .as_ref()
        .map(LlmProfileRoute::active_profile);
    let provider = selected_profile
        .as_ref()
        .map(|profile| profile.provider.clone())
        .or_else(|| {
            selection
                .provider
                .as_deref()
                .and_then(|provider| client.routed_provider_for_selector(provider))
        });
    let client_default_provider = if selected_profile.is_some() {
        None
    } else {
        client
            .default_provider()
            .and_then(|provider| client.routed_provider_for_selector(provider))
    };
    let resolved = resolve_high_level_provider_and_model(&HighLevelLlmResolutionInputs {
        provider,
        model: selection.model,
        active_profile,
        client_default_provider,
        required_capabilities: selection.required_capabilities,
    })?;
    let provider_profile_selector = resolved.provider.clone();
    let request_provider = selected_profile
        .as_ref()
        .map(|profile| profile.id.clone())
        .unwrap_or_else(|| resolved.provider.clone());
    let selected_profile_id = selected_profile.map(|profile| profile.id);
    metadata.extend(llm_selection_metadata([
        ("provider", Some(resolved.provider.as_str())),
        ("model", Some(resolved.model.as_str())),
        ("llm_profile", selected_profile_id.as_deref()),
        ("reasoning_effort", selection.reasoning_effort.as_deref()),
    ]));
    Ok(BuiltLlmRequest {
        request: Request {
            model: resolved.model,
            messages,
            provider: Some(request_provider),
            reasoning_effort: selection.reasoning_effort.clone(),
            metadata,
            ..Request::default()
        },
        llm_profile: selected_profile_id,
        reasoning_effort: selection.reasoning_effort,
        provider_profile_selector,
    })
}

fn selected_profile_for_request(
    client: &Client,
    provider: Option<&str>,
    llm_profile: Option<&str>,
) -> Result<Option<LlmProfileRoute>, AdapterError> {
    if let Some(profile_id) = llm_profile.and_then(non_empty) {
        return client.require_llm_profile(profile_id).map(Some);
    }
    if let Some(profile_id) = provider.and_then(non_empty) {
        if let Some(profile) = client.llm_profile(profile_id) {
            return Ok(Some(profile));
        }
    } else if let Some(default_provider) = client.default_provider() {
        if let Some(profile) = client.llm_profile(default_provider) {
            return Ok(Some(profile));
        }
    }
    Ok(None)
}

fn capabilities_for_codergen(request: &CodergenBackendRequest) -> ModelCapabilities {
    if request
        .reasoning_effort
        .as_deref()
        .and_then(non_empty)
        .is_some()
    {
        ModelCapabilities::reasoning()
    } else {
        ModelCapabilities::default()
    }
}

fn normalize_request_provider_selector(provider: &str) -> Option<String> {
    let provider = provider.trim();
    if provider.is_empty()
        || PROVIDER_PLACEHOLDERS
            .iter()
            .any(|placeholder| provider.eq_ignore_ascii_case(placeholder))
    {
        return None;
    }
    if is_builtin_provider_alias(provider) {
        return Some(normalize_profile_selector(provider).id);
    }
    Some(provider.to_ascii_lowercase())
}

fn is_builtin_provider_alias(provider: &str) -> bool {
    matches!(
        provider
            .trim()
            .to_ascii_lowercase()
            .replace('-', "_")
            .as_str(),
        "openai"
            | "anthropic"
            | "claude"
            | "claude_code"
            | "gemini"
            | "google"
            | "google_gemini"
            | "openrouter"
            | "litellm"
            | "codex"
            | "openai_compatible"
            | "compatible"
    )
}

fn normalize_model_selector(model: Option<&str>) -> Option<String> {
    let model = model.and_then(non_empty)?;
    if is_display_model_placeholder(model) {
        return None;
    }
    Some(model.to_string())
}

fn normalize_optional(value: Option<&str>) -> Option<String> {
    value.and_then(|value| non_empty(value).map(str::to_string))
}

fn normalize_lower_optional(value: Option<&str>) -> Option<String> {
    normalize_optional(value).map(|value| value.to_ascii_lowercase())
}

fn non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn data_string(data: &BTreeMap<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let value = data.get(*key)?;
        match value {
            Value::Null => None,
            Value::String(text) => Some(text.clone()),
            other => Some(other.to_string()),
        }
    })
}

fn object_string(object: Option<&Map<String, Value>>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let value = object?.get(*key)?;
        match value {
            Value::Null => None,
            Value::String(text) => Some(text.clone()),
            other => Some(other.to_string()),
        }
    })
}

fn llm_metadata<const N: usize>(
    source: &str,
    entries: [(&'static str, Option<&str>); N],
) -> BTreeMap<String, Value> {
    let mut metadata = BTreeMap::from([
        ("spark.runtime.backend".to_string(), json!(BACKEND_NAME)),
        ("spark.runtime.source".to_string(), json!(source)),
    ]);
    for (key, value) in entries {
        if let Some(value) = value.and_then(non_empty) {
            metadata.insert(format!("spark.runtime.{key}"), json!(value));
        }
    }
    metadata
}

fn llm_selection_metadata<const N: usize>(
    entries: [(&'static str, Option<&str>); N],
) -> BTreeMap<String, Value> {
    entries
        .into_iter()
        .filter_map(|(key, value)| {
            value
                .and_then(non_empty)
                .map(|value| (format!("spark.runtime.{key}"), json!(value)))
        })
        .collect()
}

fn codergen_adapter_error(error: AdapterError) -> CodergenError {
    CodergenError::Backend(format_adapter_error(&error))
}

fn agent_adapter_error(error: AdapterError) -> AgentError {
    AgentError {
        message: format_adapter_error(&error),
        retryable: error.retryable,
        raw: serde_json::to_value(error).ok(),
    }
}

fn format_adapter_error(error: &AdapterError) -> String {
    format!("{}: {}", error.kind.spec_error_name(), error.message)
}
