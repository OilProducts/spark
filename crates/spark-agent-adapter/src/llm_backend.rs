use std::collections::BTreeMap;

use serde_json::{json, Value};
use unified_llm_adapter::{
    is_display_model_placeholder, resolve_high_level_provider_and_model, AdapterError, Client,
    HighLevelLlmResolutionInputs, LlmProfileRoute, Message, ModelCapabilities, Request, Usage,
};

use crate::agent::{AgentError, AgentTurnBackend, AgentTurnOutput, AgentTurnRequest};
use crate::codergen::{
    CodergenBackend, CodergenBackendOutput, CodergenBackendRequest, CodergenBackendResponse,
    CodergenError, CodergenEvent,
};

const BACKEND_NAME: &str = "rust_unified_llm_adapter";
const PROVIDER_PLACEHOLDERS: &[&str] = &["codex", "codex default (config/profile)"];

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
                provider: normalize_provider_selector(&request.provider),
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
        let profile = normalize_optional(request.llm_profile.as_deref());
        let reasoning_effort = normalize_lower_optional(request.reasoning_effort.as_deref());
        let metadata_chat_mode = normalize_optional(request.chat_mode.as_deref());
        let mut metadata = request.metadata.clone();
        metadata.extend(llm_metadata(
            "agent_turn",
            [
                ("conversation_id", Some(request.conversation_id.as_str())),
                ("project_path", Some(request.project_path.as_str())),
                ("chat_mode", metadata_chat_mode.as_deref()),
            ],
        ));
        let llm_request = build_llm_request(
            &self.client,
            vec![Message::user(request.prompt)],
            RequestSelection {
                provider: normalize_provider_selector(request.provider.as_deref().unwrap_or("")),
                model: normalize_model_selector(request.model.as_deref()),
                llm_profile: profile,
                reasoning_effort,
                required_capabilities: ModelCapabilities::default(),
            },
            metadata,
        )
        .map_err(agent_adapter_error)?;
        let response = self
            .client
            .complete(llm_request.request)
            .map_err(agent_adapter_error)?;

        Ok(AgentTurnOutput {
            events: Vec::new(),
            final_assistant_text: Some(response.text()),
            token_usage: usage_json(&response.usage),
            raw_log_lines: Vec::new(),
            thread_resume_failure: None,
        })
    }
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

fn normalize_provider_selector(provider: &str) -> Option<String> {
    let provider = provider.trim();
    if provider.is_empty()
        || PROVIDER_PLACEHOLDERS
            .iter()
            .any(|placeholder| provider.eq_ignore_ascii_case(placeholder))
    {
        return None;
    }
    Some(provider.to_ascii_lowercase())
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

fn usage_json(usage: &Usage) -> Option<Value> {
    serde_json::to_value(usage).ok()
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
