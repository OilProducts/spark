use std::collections::BTreeMap;

use serde_json::{json, Map, Value};
use spark_common::events::{TurnStreamEvent, TurnStreamEventKind, TurnStreamSource};
use unified_llm_adapter::{
    is_display_model_placeholder, resolve_high_level_provider_and_model, AdapterError, Client,
    HighLevelLlmResolutionInputs, LlmProfileRoute, Message, ModelCapabilities, Request,
};

use crate::agent::{
    AgentError, AgentRawLogLine, AgentRequestUserInputAnswerRequest, AgentThreadResumeFailure,
    AgentTurnBackend, AgentTurnOutput, AgentTurnRequest,
};
use crate::codergen::{
    CodergenBackend, CodergenBackendOutput, CodergenBackendRequest, CodergenBackendResponse,
    CodergenError, CodergenEvent,
};
use crate::config::SessionConfig;
use crate::environment::ExecutionEnvironment;
use crate::events::{workspace_token_usage_payload_from_usage, EventKind, SessionEvent};
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
