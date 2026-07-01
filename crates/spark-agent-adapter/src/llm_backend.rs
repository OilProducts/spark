use std::collections::BTreeMap;

use attractor_core::{FailureKind, Outcome, OutcomeStatus};
use serde_json::{json, Map, Value};
use spark_common::events::{TurnStreamEvent, TurnStreamEventKind, TurnStreamSource};
use unified_llm_adapter::{
    is_display_model_placeholder, resolve_high_level_provider_and_model, AdapterError, Client,
    HighLevelLlmResolutionInputs, LlmProfileRoute, Message, ModelCapabilities, Request, Usage,
};

use crate::agent::{
    AgentError, AgentRawLogLine, AgentRequestUserInputAnswerRequest, AgentThreadResumeFailure,
    AgentTurnBackend, AgentTurnOutput, AgentTurnRequest,
};
use crate::codergen::{
    ActiveCodergenSession, CodergenBackend, CodergenBackendOutput, CodergenBackendRequest,
    CodergenBackendResponse, CodergenError, CodergenEvent, CodergenSessionInterventionBroker,
};
use crate::codex_app_server::{
    usage_from_codex_token_payload, CodexAppServerBackend, CodexAppServerError,
    CODEX_APP_SERVER_BACKEND,
};
use crate::config::SessionConfig;
use crate::environment::ExecutionEnvironment;
use crate::events::{workspace_token_usage_payload_from_usage, EventKind, SessionEvent};
use crate::history::HistoryTurn;
use crate::profiles::{
    create_provider_profile, normalize_provider_selector as normalize_profile_selector,
};
use crate::session::Session;
use crate::session::SessionSteeringHandle;

const BACKEND_NAME: &str = "rust_unified_llm_adapter";
const PROVIDER_PLACEHOLDERS: &[&str] = &["codex default (config/profile)"];

#[derive(Clone)]
pub struct RustLlmCodergenBackend {
    client: Client,
    intervention_broker: Option<CodergenSessionInterventionBroker>,
}

impl RustLlmCodergenBackend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            intervention_broker: None,
        }
    }

    pub fn with_intervention_broker(
        client: Client,
        intervention_broker: CodergenSessionInterventionBroker,
    ) -> Self {
        Self {
            client,
            intervention_broker: Some(intervention_broker),
        }
    }
}

impl CodergenBackend for RustLlmCodergenBackend {
    fn run(
        &mut self,
        request: CodergenBackendRequest,
    ) -> Result<CodergenBackendOutput, CodergenError> {
        if is_codex_provider_selector(&request.provider) {
            return self.run_codex_app_server_codergen(request);
        }
        if request.runtime_mode.requires_agent() {
            return self.run_agent_codergen(request);
        }
        self.run_text_only_codergen(request)
    }
}

impl RustLlmCodergenBackend {
    fn run_codex_app_server_codergen(
        &mut self,
        request: CodergenBackendRequest,
    ) -> Result<CodergenBackendOutput, CodergenError> {
        let agent_request = codergen_agent_turn_request(&request);
        let steering = SessionSteeringHandle::new();
        let _active_session = self.intervention_broker.as_ref().map(|broker| {
            broker.register(ActiveCodergenSession {
                node_id: request.node_id.clone(),
                child_run_id: metadata_string(&request.metadata, "spark.runtime.run_id"),
                root_run_id: metadata_string(&request.metadata, "spark.runtime.root_run_id"),
                provider: "codex".to_string(),
                model: request.model.clone(),
                llm_profile: request.llm_profile.clone(),
                reasoning_effort: request.reasoning_effort.clone(),
                project_path: request.project_path.clone(),
                metadata: request.metadata.clone(),
                steering: steering.clone(),
            })
        });
        let output = CodexAppServerBackend::new()
            .run_agent_turn_with_steering(agent_request, Some(steering))
            .map_err(codex_app_server_codergen_error)?;
        let usage = output
            .token_usage
            .as_ref()
            .and_then(usage_from_codex_token_payload);
        let response = if let Some(failure) = output.thread_resume_failure.as_ref() {
            CodergenBackendResponse::Outcome(Outcome {
                status: OutcomeStatus::Fail,
                failure_reason: failure.message.clone(),
                retryable: Some(false),
                failure_kind: Some(FailureKind::Runtime),
                ..Outcome::new(OutcomeStatus::Fail)
            })
        } else if let Some(text) = output.final_assistant_text.as_deref().and_then(non_empty) {
            CodergenBackendResponse::Text(text.to_string())
        } else {
            CodergenBackendResponse::Outcome(Outcome {
                status: OutcomeStatus::Fail,
                failure_reason: "codex app-server completed without assistant text".to_string(),
                retryable: Some(false),
                failure_kind: Some(FailureKind::Runtime),
                ..Outcome::new(OutcomeStatus::Fail)
            })
        };
        let mut events = Vec::new();
        for event in &output.events {
            events.push(codergen_event_from_turn_stream_event(&request, event));
        }
        for raw_log_line in &output.raw_log_lines {
            events.push(CodergenEvent::new(
                "codex_app_server_raw_log_line",
                BTreeMap::from([
                    ("node_id".to_string(), json!(request.node_id.clone())),
                    (
                        "direction".to_string(),
                        json!(raw_log_line.direction.clone()),
                    ),
                    ("line".to_string(), json!(raw_log_line.line.clone())),
                ]),
            ));
        }
        events.push(CodergenEvent::new(
            "codex_app_server_request_completed",
            BTreeMap::from([
                ("backend".to_string(), json!(CODEX_APP_SERVER_BACKEND)),
                ("node_id".to_string(), json!(request.node_id.clone())),
                (
                    "provider_selector".to_string(),
                    json!(request.provider.clone()),
                ),
                ("provider".to_string(), json!("codex")),
                ("model_selector".to_string(), json!(request.model.clone())),
                ("model".to_string(), json!(request.model.clone())),
                (
                    "llm_profile".to_string(),
                    json!(request.llm_profile.clone()),
                ),
                (
                    "reasoning_effort".to_string(),
                    json!(request.reasoning_effort.clone()),
                ),
                (
                    "response_contract".to_string(),
                    json!(request.response_contract.clone()),
                ),
                (
                    "runtime_mode".to_string(),
                    json!(request.runtime_mode.clone()),
                ),
                ("token_usage".to_string(), json!(output.token_usage.clone())),
            ]),
        ));
        Ok(CodergenBackendOutput {
            response,
            events,
            usage,
        })
    }

    fn run_text_only_codergen(
        &mut self,
        request: CodergenBackendRequest,
    ) -> Result<CodergenBackendOutput, CodergenError> {
        let profile = normalize_optional(request.llm_profile.as_deref());
        let reasoning_effort = normalize_lower_optional(request.reasoning_effort.as_deref());
        let metadata_response_contract =
            normalize_optional(Some(request.response_contract.as_str()));
        let mut metadata = request.metadata.clone();
        metadata.extend(llm_metadata(
            "codergen",
            [
                ("node_id", Some(request.node_id.as_str())),
                ("response_contract", metadata_response_contract.as_deref()),
                ("provider_selector", Some(request.provider.as_str())),
                ("runtime_mode", Some(request.runtime_mode.mode.as_str())),
            ],
        ));
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
            metadata,
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
                    ("provider_selector".to_string(), json!(request.provider)),
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
                    (
                        "response_contract".to_string(),
                        json!(request.response_contract),
                    ),
                    (
                        "contract_repair_attempts".to_string(),
                        json!(request.contract_repair_attempts),
                    ),
                    (
                        "timeout_seconds".to_string(),
                        json!(request.timeout_seconds),
                    ),
                    ("write_contract".to_string(), json!(request.write_contract)),
                    ("runtime_mode".to_string(), json!(request.runtime_mode)),
                ]),
            )],
            usage: Some(response.usage),
        })
    }

    fn run_agent_codergen(
        &mut self,
        request: CodergenBackendRequest,
    ) -> Result<CodergenBackendOutput, CodergenError> {
        let agent_request = codergen_agent_turn_request(&request);
        let prompt = agent_request.prompt.clone();
        let mut session =
            build_agent_session(&self.client, agent_request).map_err(codergen_adapter_error)?;
        let initial_history_len = session.history.len();
        let _active_session = self.intervention_broker.as_ref().map(|broker| {
            broker.register(ActiveCodergenSession {
                node_id: request.node_id.clone(),
                child_run_id: metadata_string(&request.metadata, "spark.runtime.run_id"),
                root_run_id: metadata_string(&request.metadata, "spark.runtime.root_run_id"),
                provider: request.provider.clone(),
                model: request
                    .model
                    .clone()
                    .or_else(|| non_empty(&session.provider_profile.model).map(str::to_string)),
                llm_profile: request.llm_profile.clone(),
                reasoning_effort: request.reasoning_effort.clone(),
                project_path: request.project_path.clone(),
                metadata: request.metadata.clone(),
                steering: session.steering_handle(),
            })
        });
        let process_error = session.process_input(&self.client, prompt).err();
        session.close();
        Ok(codergen_output_from_session(
            request,
            &mut session,
            initial_history_len,
            process_error,
        ))
    }
}

fn codergen_agent_turn_request(request: &CodergenBackendRequest) -> AgentTurnRequest {
    let mut metadata = request.metadata.clone();
    metadata.insert(
        "spark.runtime.codergen.node_id".to_string(),
        json!(request.node_id.clone()),
    );
    metadata.insert(
        "spark.runtime.codergen.response_contract".to_string(),
        json!(request.response_contract.clone()),
    );
    metadata.insert(
        "spark.runtime.codergen.contract_repair_attempts".to_string(),
        json!(request.contract_repair_attempts),
    );
    metadata.insert(
        "spark.runtime.codergen.runtime_mode".to_string(),
        json!(request.runtime_mode.clone()),
    );
    metadata.insert(
        "spark.runtime.codergen.write_contract".to_string(),
        json!(request.write_contract.clone()),
    );
    if let Some(timeout_seconds) = request.timeout_seconds {
        metadata.insert(
            "spark.runtime.codergen.timeout_seconds".to_string(),
            json!(timeout_seconds),
        );
    }
    if let Some(repair_attempt) = request.repair_attempt {
        metadata.insert(
            "spark.runtime.codergen.repair_attempt".to_string(),
            json!(repair_attempt),
        );
    }

    AgentTurnRequest {
        conversation_id: codergen_conversation_id(request),
        project_path: codergen_project_path(request),
        prompt: request.prompt.clone(),
        history: Vec::new(),
        provider: normalize_optional(Some(request.provider.as_str())),
        model: request
            .model
            .as_deref()
            .and_then(non_empty)
            .map(str::to_string),
        llm_profile: request
            .llm_profile
            .as_deref()
            .and_then(non_empty)
            .map(str::to_string),
        reasoning_effort: normalize_lower_optional(request.reasoning_effort.as_deref()),
        chat_mode: Some("agent".to_string()),
        metadata,
    }
}

fn codergen_output_from_session(
    request: CodergenBackendRequest,
    session: &mut Session,
    initial_history_len: usize,
    process_error: Option<AdapterError>,
) -> CodergenBackendOutput {
    let (final_assistant_text, assistant_usage) =
        codergen_final_assistant_text_and_usage(session, initial_history_len);
    let session_events = drain_session_events(session);
    let event_usage = session_events
        .iter()
        .rev()
        .find_map(usage_from_session_event);
    let usage = assistant_usage.or(event_usage);
    let response = if let Some(error) = process_error.as_ref() {
        CodergenBackendResponse::Outcome(Outcome {
            status: OutcomeStatus::Fail,
            failure_reason: format_adapter_error(error),
            retryable: Some(error.retryable),
            failure_kind: Some(FailureKind::Runtime),
            ..Outcome::new(OutcomeStatus::Fail)
        })
    } else if let Some(text) = final_assistant_text.as_deref().and_then(non_empty) {
        CodergenBackendResponse::Text(text.to_string())
    } else {
        CodergenBackendResponse::Outcome(Outcome {
            status: OutcomeStatus::Fail,
            failure_reason: "agent-backed codergen completed without assistant text".to_string(),
            retryable: Some(false),
            failure_kind: Some(FailureKind::Runtime),
            ..Outcome::new(OutcomeStatus::Fail)
        })
    };

    let mut events = Vec::new();
    for event in &session_events {
        events.push(codergen_event_from_session_event(&request, session, event));
        if let Some(raw_log_line) = raw_log_line_from_event(event) {
            events.push(CodergenEvent::new(
                "rust_agent_raw_log_line",
                BTreeMap::from([
                    ("node_id".to_string(), json!(request.node_id.clone())),
                    ("direction".to_string(), json!(raw_log_line.direction)),
                    ("line".to_string(), json!(raw_log_line.line)),
                ]),
            ));
        }
    }
    if !session_events
        .iter()
        .any(|event| event.kind == EventKind::ModelUsageUpdate)
    {
        if let Some(usage) = usage.as_ref() {
            events.push(codergen_usage_event_from_assistant_turn(
                &request, session, usage,
            ));
        }
    }
    events.push(CodergenEvent::new(
        "rust_agent_adapter_request_completed",
        codergen_completion_event_payload(&request, session, usage.as_ref()),
    ));

    CodergenBackendOutput {
        response,
        events,
        usage,
    }
}

fn codergen_event_from_session_event(
    request: &CodergenBackendRequest,
    session: &Session,
    event: &SessionEvent,
) -> CodergenEvent {
    let mut payload = codergen_session_metadata_payload(request, session);
    payload.insert("kind".to_string(), json!(event.kind.as_str()));
    payload.insert(
        "category".to_string(),
        json!(codergen_session_event_category(&event.kind)),
    );
    payload.insert(
        "session_event".to_string(),
        serde_json::to_value(event).unwrap_or_else(|_| json!({})),
    );
    if let Some(turn_stream_event) = event.to_turn_stream_event() {
        if let Some(tool_call) = turn_stream_event.tool_call.as_ref() {
            payload.insert("tool_event".to_string(), tool_call.clone());
        }
        payload.insert(
            "turn_stream_event".to_string(),
            serde_json::to_value(turn_stream_event).unwrap_or_else(|_| json!({})),
        );
    }
    CodergenEvent::new("rust_agent_session_event", payload)
}

fn codergen_event_from_turn_stream_event(
    request: &CodergenBackendRequest,
    event: &TurnStreamEvent,
) -> CodergenEvent {
    let mut payload = BTreeMap::from([
        ("node_id".to_string(), json!(request.node_id.clone())),
        (
            "backend".to_string(),
            json!(CODEX_APP_SERVER_BACKEND.to_string()),
        ),
        (
            "provider_selector".to_string(),
            json!(request.provider.clone()),
        ),
        ("provider".to_string(), json!("codex")),
        ("model_selector".to_string(), json!(request.model.clone())),
        (
            "reasoning_effort".to_string(),
            json!(request.reasoning_effort.clone()),
        ),
        ("kind".to_string(), json!(event.kind.as_str())),
        (
            "category".to_string(),
            json!(codergen_turn_stream_event_category(event)),
        ),
        (
            "turn_stream_event".to_string(),
            serde_json::to_value(event).unwrap_or_else(|_| json!({})),
        ),
    ]);
    if let Some(tool_call) = event.tool_call.as_ref() {
        payload.insert("tool_event".to_string(), tool_call.clone());
    }
    if let Some(token_usage) = event.token_usage.as_ref() {
        payload.insert("token_usage".to_string(), token_usage.clone());
    }
    CodergenEvent::new("codex_app_server_session_event", payload)
}

fn codergen_turn_stream_event_category(event: &TurnStreamEvent) -> &'static str {
    match event.kind {
        TurnStreamEventKind::ContentDelta | TurnStreamEventKind::ContentCompleted => {
            match event.channel {
                Some(spark_common::events::TurnStreamChannel::Plan) => "plan",
                Some(spark_common::events::TurnStreamChannel::Reasoning) => "reasoning",
                _ => "assistant_text",
            }
        }
        TurnStreamEventKind::ToolCallStarted
        | TurnStreamEventKind::ToolCallUpdated
        | TurnStreamEventKind::ToolCallCompleted
        | TurnStreamEventKind::ToolCallFailed => "tool_execution",
        TurnStreamEventKind::TokenUsageUpdated => "usage",
        TurnStreamEventKind::RequestUserInputRequested => "request_user_input",
        TurnStreamEventKind::ContextCompactionStarted
        | TurnStreamEventKind::ContextCompactionCompleted => "context_compaction",
        TurnStreamEventKind::TurnCompleted => "processing",
        TurnStreamEventKind::Error => "error",
        TurnStreamEventKind::Other(_) => "other",
    }
}

fn codergen_usage_event_from_assistant_turn(
    request: &CodergenBackendRequest,
    session: &Session,
    usage: &Usage,
) -> CodergenEvent {
    let usage_value =
        serde_json::to_value(usage.clone().normalized()).unwrap_or_else(|_| json!({}));
    let mut payload = codergen_session_metadata_payload(request, session);
    payload.insert(
        "kind".to_string(),
        json!(EventKind::ModelUsageUpdate.as_str()),
    );
    payload.insert("category".to_string(), json!("usage"));
    payload.insert("token_usage".to_string(), usage_value.clone());
    payload.insert("derived_from".to_string(), json!("assistant_turn_usage"));
    payload.insert(
        "session_event".to_string(),
        json!({
            "kind": EventKind::ModelUsageUpdate.as_str(),
            "session_id": session.session_id().to_string(),
            "data": {"usage": usage_value},
        }),
    );
    CodergenEvent::new("rust_agent_session_event", payload)
}

fn codergen_session_event_category(kind: &EventKind) -> &'static str {
    match kind {
        EventKind::SessionStart | EventKind::SessionEnd => "lifecycle",
        EventKind::UserInput => "user_input",
        EventKind::ProcessingEnd | EventKind::TurnLimit | EventKind::LoopDetection => "processing",
        EventKind::AssistantTextStart
        | EventKind::AssistantTextDelta
        | EventKind::AssistantTextEnd => "assistant_text",
        EventKind::AssistantReasoningStart
        | EventKind::AssistantReasoningDelta
        | EventKind::AssistantReasoningEnd => "reasoning",
        EventKind::ModelToolCallStart
        | EventKind::ModelToolCallDelta
        | EventKind::ModelToolCallEnd => "model_tool_call",
        EventKind::ToolCallStart | EventKind::ToolCallOutputDelta | EventKind::ToolCallEnd => {
            "tool_execution"
        }
        EventKind::SteeringInjected => "steering",
        EventKind::ModelUsageUpdate => "usage",
        EventKind::Warning => "warning",
        EventKind::Error => "error",
        EventKind::Other(_) => "other",
    }
}

fn codergen_completion_event_payload(
    request: &CodergenBackendRequest,
    session: &Session,
    usage: Option<&Usage>,
) -> BTreeMap<String, Value> {
    let mut payload = codergen_session_metadata_payload(request, session);
    payload.insert("backend".to_string(), json!(BACKEND_NAME));
    payload.insert("session_state".to_string(), json!(session.state));
    payload.insert(
        "contract_repair_attempts".to_string(),
        json!(request.contract_repair_attempts),
    );
    payload.insert("token_usage".to_string(), json!(usage));
    payload
}

fn codergen_session_metadata_payload(
    request: &CodergenBackendRequest,
    session: &Session,
) -> BTreeMap<String, Value> {
    let metadata = &session.execution_environment.metadata;
    BTreeMap::from([
        ("node_id".to_string(), json!(request.node_id.clone())),
        (
            "session_id".to_string(),
            json!(session.session_id().to_string()),
        ),
        (
            "provider_selector".to_string(),
            json!(request.provider.clone()),
        ),
        (
            "provider".to_string(),
            json!(metadata_string(metadata, "spark.runtime.provider")
                .or_else(|| session.provider_profile.request_provider_id())),
        ),
        ("model_selector".to_string(), json!(request.model.clone())),
        (
            "model".to_string(),
            json!(metadata_string(metadata, "spark.runtime.model")
                .or_else(|| non_empty(&session.provider_profile.model).map(str::to_string))),
        ),
        (
            "llm_profile_selector".to_string(),
            json!(request.llm_profile.clone()),
        ),
        (
            "llm_profile".to_string(),
            json!(metadata_string(metadata, "spark.runtime.llm_profile")),
        ),
        (
            "reasoning_effort".to_string(),
            json!(metadata_string(metadata, "spark.runtime.reasoning_effort")
                .or_else(|| session.config.reasoning_effort.clone())
                .or_else(|| request.reasoning_effort.clone())),
        ),
        (
            "runtime_mode".to_string(),
            json!(request.runtime_mode.clone()),
        ),
        (
            "response_contract".to_string(),
            json!(request.response_contract.clone()),
        ),
        (
            "timeout_seconds".to_string(),
            json!(request.timeout_seconds),
        ),
        (
            "write_contract".to_string(),
            json!(request.write_contract.clone()),
        ),
    ])
}

fn codergen_final_assistant_text_and_usage(
    session: &Session,
    initial_history_len: usize,
) -> (Option<String>, Option<Usage>) {
    session
        .history
        .iter()
        .skip(initial_history_len)
        .rev()
        .find_map(|turn| match turn {
            HistoryTurn::Assistant(assistant) => {
                let text = assistant.text();
                if non_empty(&text).is_some() {
                    Some((Some(text), assistant.usage.clone()))
                } else {
                    None
                }
            }
            _ => None,
        })
        .unwrap_or((None, None))
}

fn drain_session_events(session: &mut Session) -> Vec<SessionEvent> {
    let mut events = Vec::new();
    while let Some(event) = session.next_event() {
        events.push(event);
    }
    events
}

fn usage_from_session_event(event: &SessionEvent) -> Option<Usage> {
    if event.kind != EventKind::ModelUsageUpdate {
        return None;
    }
    event
        .data
        .get("token_usage")
        .or_else(|| event.data.get("usage"))
        .and_then(usage_from_token_usage_payload)
}

fn codergen_project_path(request: &CodergenBackendRequest) -> String {
    request
        .project_path
        .as_ref()
        .map(|path| path.to_string_lossy().trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| ".".to_string())
}

fn codergen_conversation_id(request: &CodergenBackendRequest) -> String {
    metadata_string(&request.metadata, "spark.runtime.conversation_id").unwrap_or_else(|| {
        metadata_string(&request.metadata, "spark.runtime.run_id")
            .map(|run_id| format!("{run_id}:{}", request.node_id))
            .unwrap_or_else(|| format!("codergen:{}", request.node_id))
    })
}

fn metadata_string(metadata: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(Value::as_str)
        .and_then(non_empty)
        .map(str::to_string)
}

fn usage_from_token_usage_payload(payload: &Value) -> Option<Usage> {
    let total = payload.get("total").unwrap_or(payload);
    let input_tokens = value_u64(total, &["inputTokens", "input_tokens"])
        .or_else(|| value_u64(payload, &["inputTokens", "input_tokens"]))
        .unwrap_or(0);
    let output_tokens = value_u64(total, &["outputTokens", "output_tokens"])
        .or_else(|| value_u64(payload, &["outputTokens", "output_tokens"]))
        .unwrap_or(0);
    let total_tokens = value_u64(total, &["totalTokens", "total_tokens"])
        .or_else(|| value_u64(payload, &["totalTokens", "total_tokens"]))
        .unwrap_or(input_tokens + output_tokens);
    let cache_read_tokens = value_u64(total, &["cachedInputTokens", "cache_read_tokens"])
        .or_else(|| value_u64(payload, &["cachedInputTokens", "cache_read_tokens"]));
    let cache_write_tokens = value_u64(total, &["cacheWriteTokens", "cache_write_tokens"])
        .or_else(|| value_u64(payload, &["cacheWriteTokens", "cache_write_tokens"]));
    let reasoning_tokens = value_u64(total, &["reasoningOutputTokens", "reasoning_tokens"])
        .or_else(|| value_u64(payload, &["reasoningOutputTokens", "reasoning_tokens"]));
    if input_tokens == 0
        && output_tokens == 0
        && total_tokens == 0
        && cache_read_tokens.is_none()
        && cache_write_tokens.is_none()
        && reasoning_tokens.is_none()
    {
        return None;
    }
    Some(
        Usage {
            input_tokens,
            output_tokens,
            total_tokens,
            reasoning_tokens,
            cache_read_tokens,
            cache_write_tokens,
            raw: Some(payload.clone()),
            ..Usage::default()
        }
        .normalized(),
    )
}

fn value_u64(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| value.get(*key)?.as_u64())
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
        if is_codex_request_provider(request.provider.as_deref(), request.llm_profile.as_deref()) {
            return CodexAppServerBackend::new()
                .run_agent_turn(request)
                .map_err(codex_app_server_agent_error);
        }
        let prompt = request.prompt.clone();
        let mut session =
            build_agent_session(&self.client, request).map_err(agent_adapter_error)?;
        let initial_history_len = session.history.len();
        if let Err(error) = session.process_input(&self.client, prompt) {
            let output = agent_output_from_session(&mut session, initial_history_len);
            if agent_turn_output_has_failure(&output) {
                return Ok(output);
            }
            return Err(agent_adapter_error(error));
        }

        Ok(agent_output_from_session(&mut session, initial_history_len))
    }

    fn answer_request_user_input(
        &self,
        request: AgentRequestUserInputAnswerRequest,
    ) -> Result<AgentTurnOutput, AgentError> {
        if is_codex_request_provider(request.provider.as_deref(), request.llm_profile.as_deref()) {
            return CodexAppServerBackend::new()
                .answer_request_user_input(request)
                .map_err(codex_app_server_agent_error);
        }
        let continuation = match request_user_input_answer_continuation(&request) {
            Ok(continuation) => continuation,
            Err(failure) => {
                return Ok(request_user_input_resume_failure_output(
                    request.request_id.as_str(),
                    failure,
                ));
            }
        };

        let AgentRequestUserInputAnswerRequest {
            conversation_id,
            project_path,
            request_id,
            assistant_turn_id,
            answers,
            request_user_input,
            history,
            provider,
            model,
            llm_profile,
            reasoning_effort,
            chat_mode,
            mut metadata,
        } = request;
        metadata.insert(
            "spark.runtime.request_user_input.request_id".to_string(),
            json!(request_id),
        );
        metadata.insert(
            "spark.runtime.request_user_input.assistant_turn_id".to_string(),
            json!(assistant_turn_id),
        );
        metadata.insert(
            "spark.runtime.request_user_input.answers".to_string(),
            json!(answers),
        );
        if let Some(request_user_input) = request_user_input {
            metadata.insert(
                "spark.runtime.request_user_input".to_string(),
                request_user_input,
            );
        }

        let mut session = build_agent_session_for_source(
            &self.client,
            AgentTurnRequest {
                conversation_id,
                project_path,
                prompt: String::new(),
                history,
                provider,
                model,
                llm_profile,
                reasoning_effort,
                chat_mode,
                metadata,
            },
            "request_user_input_answer",
        )
        .map_err(agent_adapter_error)?;
        let initial_history_len = session.history.len();
        session.mark_awaiting_input(continuation.pending_question);
        if let Err(error) = session.process_input(&self.client, continuation.answer_content) {
            let output = agent_output_from_session(&mut session, initial_history_len);
            if agent_turn_output_has_failure(&output) {
                return Ok(output);
            }
            return Err(agent_adapter_error(error));
        }

        Ok(agent_output_from_session(&mut session, initial_history_len))
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
    build_agent_session_for_source(client, request, "agent_turn")
}

fn build_agent_session_for_source(
    client: &Client,
    request: AgentTurnRequest,
    source: &str,
) -> Result<Session, AdapterError> {
    let AgentTurnRequest {
        conversation_id,
        project_path,
        prompt: _,
        history,
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
        source,
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

    let mut session = Session::new(profile, execution_environment, config);
    session.history = history;
    Ok(session)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RequestUserInputAnswerContinuation {
    pending_question: String,
    answer_content: String,
}

fn request_user_input_answer_continuation(
    request: &AgentRequestUserInputAnswerRequest,
) -> Result<RequestUserInputAnswerContinuation, AgentThreadResumeFailure> {
    let request_id = non_empty(&request.request_id).ok_or_else(|| {
        request_user_input_resume_failure(
            "request-user-input answer could not resume because the request id is empty.",
            "request_user_input_missing_request_id",
            json!({}),
        )
    })?;
    let answers = normalize_request_user_input_answer_map(&request.answers);
    if answers.is_empty() {
        return Err(request_user_input_resume_failure(
            "request-user-input answer could not resume because no answers were supplied.",
            "request_user_input_missing_answers",
            json!({ "request_id": request_id }),
        ));
    }

    if let Some(payload_id) = request
        .request_user_input
        .as_ref()
        .and_then(request_user_input_payload_request_id)
    {
        if payload_id != request_id {
            return Err(request_user_input_resume_failure(
                "request-user-input answer could not resume because the persisted request id does not match the answer request.",
                "request_user_input_id_mismatch",
                json!({
                    "request_id": request_id,
                    "payload_request_id": payload_id,
                }),
            ));
        }
    }

    if matches!(
        request
            .request_user_input
            .as_ref()
            .and_then(request_user_input_payload_status)
            .as_deref(),
        Some("expired")
    ) {
        return Err(request_user_input_resume_failure(
            "request-user-input answer could not resume because the persisted request is no longer pending.",
            "request_user_input_not_pending",
            json!({ "request_id": request_id }),
        ));
    }

    let answer_content =
        request_user_input_answer_content(&answers, request.request_user_input.as_ref());
    if answer_content.trim().is_empty() {
        return Err(request_user_input_resume_failure(
            "request-user-input answer could not resume because the normalized answers were empty.",
            "request_user_input_missing_answers",
            json!({ "request_id": request_id }),
        ));
    }

    Ok(RequestUserInputAnswerContinuation {
        pending_question: request_user_input_pending_question(
            request_id,
            request.request_user_input.as_ref(),
        ),
        answer_content,
    })
}

fn normalize_request_user_input_answer_map(
    answers: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    answers
        .iter()
        .filter_map(|(key, value)| {
            let key = non_empty(key)?;
            let value = non_empty(value)?;
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

fn request_user_input_payload_request_id(payload: &Value) -> Option<String> {
    let object = payload.as_object()?;
    object
        .get("request_id")
        .or_else(|| object.get("itemId"))
        .and_then(Value::as_str)
        .and_then(non_empty)
        .map(str::to_string)
}

fn request_user_input_payload_status(payload: &Value) -> Option<String> {
    payload
        .as_object()?
        .get("status")
        .and_then(Value::as_str)
        .and_then(non_empty)
        .map(|status| status.to_ascii_lowercase())
}

fn request_user_input_pending_question(request_id: &str, payload: Option<&Value>) -> String {
    let prompts = request_user_input_questions(payload)
        .into_iter()
        .filter_map(|(_, prompt)| non_empty(&prompt).map(str::to_string))
        .collect::<Vec<_>>();
    match prompts.len() {
        0 => format!("User input requested for {request_id}."),
        1 => prompts[0].clone(),
        count => format!("{count} questions need user input."),
    }
}

fn request_user_input_answer_content(
    answers: &BTreeMap<String, String>,
    payload: Option<&Value>,
) -> String {
    let questions = request_user_input_questions(payload);
    let mut consumed_question_ids = Vec::new();
    let mut lines = Vec::new();
    for (question_id, prompt) in questions {
        if let Some(answer) = answers.get(&question_id) {
            lines.push(request_user_input_answer_line(
                question_id.as_str(),
                prompt.as_str(),
                answer.as_str(),
            ));
            consumed_question_ids.push(question_id);
        }
    }
    for (question_id, answer) in answers {
        if consumed_question_ids
            .iter()
            .any(|consumed| consumed == question_id)
        {
            continue;
        }
        lines.push(request_user_input_answer_line(
            question_id.as_str(),
            "",
            answer.as_str(),
        ));
    }
    lines.join("\n\n")
}

fn request_user_input_questions(payload: Option<&Value>) -> Vec<(String, String)> {
    payload
        .and_then(Value::as_object)
        .and_then(|object| object.get("questions"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, question)| {
            let object = question.as_object()?;
            let question_id = object
                .get("id")
                .and_then(Value::as_str)
                .and_then(non_empty)
                .map(str::to_string)
                .unwrap_or_else(|| format!("question-{}", index + 1));
            let prompt = object
                .get("question")
                .and_then(Value::as_str)
                .and_then(non_empty)
                .map(str::to_string)
                .unwrap_or_default();
            Some((question_id, prompt))
        })
        .collect()
}

fn request_user_input_answer_line(question_id: &str, prompt: &str, answer: &str) -> String {
    if let Some(prompt) = non_empty(prompt) {
        format!("{prompt}\nAnswer: {answer}")
    } else {
        format!("{question_id}: {answer}")
    }
}

fn request_user_input_resume_failure(
    message: impl Into<String>,
    error_code: impl Into<String>,
    details: Value,
) -> AgentThreadResumeFailure {
    AgentThreadResumeFailure {
        message: message.into(),
        error_code: Some(error_code.into()),
        details: Some(details),
    }
}

fn request_user_input_resume_failure_output(
    request_id: &str,
    failure: AgentThreadResumeFailure,
) -> AgentTurnOutput {
    AgentTurnOutput {
        events: vec![TurnStreamEvent {
            kind: TurnStreamEventKind::Error,
            channel: None,
            source: TurnStreamSource {
                backend: Some(BACKEND_NAME.to_string()),
                item_id: non_empty(request_id).map(str::to_string),
                raw_kind: Some("request_user_input_resume_failure".to_string()),
                ..TurnStreamSource::default()
            },
            content_delta: None,
            message: Some(failure.message.clone()),
            tool_call: None,
            request_user_input: None,
            token_usage: None,
            error_code: failure.error_code.clone(),
            details: failure.details.clone(),
            error: Some(failure.message.clone()),
            phase: Some("request_user_input_answer".to_string()),
            status: Some("failed".to_string()),
        }],
        thread_resume_failure: Some(failure),
        ..AgentTurnOutput::default()
    }
}

fn agent_output_from_session(session: &mut Session, initial_history_len: usize) -> AgentTurnOutput {
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
                output.token_usage_breakdown = Some(token_usage.clone());
            }
            output.events.push(turn_stream_event);
        }
    }

    if let Some((assistant, text)) = session
        .history
        .iter()
        .skip(initial_history_len)
        .rev()
        .find_map(|turn| match turn {
            HistoryTurn::Assistant(assistant) => {
                let text = assistant.text();
                if non_empty(&text).is_some() {
                    Some((assistant, text))
                } else {
                    None
                }
            }
            _ => None,
        })
    {
        output.final_assistant_text = Some(text);
        if output.token_usage.is_none() {
            output.token_usage = assistant
                .usage
                .as_ref()
                .and_then(workspace_token_usage_payload_from_usage);
        }
        if output.token_usage_breakdown.is_none() {
            output.token_usage_breakdown = output.token_usage.clone();
        }
    }

    output
}

fn agent_turn_output_has_failure(output: &AgentTurnOutput) -> bool {
    output.thread_resume_failure.is_some()
        || output
            .events
            .iter()
            .any(|event| event.kind == spark_common::events::TurnStreamEventKind::Error)
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
    if event.kind == EventKind::Error {
        return thread_resume_failure_from_error_event(event);
    }

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

fn thread_resume_failure_from_error_event(
    event: &SessionEvent,
) -> Option<AgentThreadResumeFailure> {
    let nested = event.data.get("error").and_then(Value::as_object);
    let message = object_string(nested, &["message"])
        .or_else(|| data_string(&event.data, &["message", "error"]))
        .unwrap_or_else(|| "agent turn failed".to_string());
    let error_code = object_string(nested, &["error_code", "code", "kind", "name"])
        .or_else(|| data_string(&event.data, &["error_code", "code", "error_kind", "name"]));
    let details = Some(Value::Object(event.data.clone().into_iter().collect()));

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

fn codex_app_server_codergen_error(error: CodexAppServerError) -> CodergenError {
    CodergenError::Backend(format!("codex app-server error: {}", error.message))
}

fn agent_adapter_error(error: AdapterError) -> AgentError {
    AgentError {
        message: format_adapter_error(&error),
        retryable: error.retryable,
        raw: serde_json::to_value(error).ok(),
    }
}

fn codex_app_server_agent_error(error: CodexAppServerError) -> AgentError {
    AgentError {
        message: error.message,
        retryable: error.retryable,
        raw: error.details,
    }
}

fn format_adapter_error(error: &AdapterError) -> String {
    format!("{}: {}", error.kind.spec_error_name(), error.message)
}

fn is_codex_request_provider(provider: Option<&str>, _llm_profile: Option<&str>) -> bool {
    provider.is_some_and(is_codex_provider_selector)
}

fn is_codex_provider_selector(provider: &str) -> bool {
    provider.trim().eq_ignore_ascii_case("codex")
}
