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
    AgentTurnBackend, AgentTurnEventSink, AgentTurnOutput, AgentTurnRequest,
};
use crate::claude_code::{
    usage_from_claude_code_usage_payload, ClaudeCodeBackend, CLAUDE_CODE_BACKEND,
    MODEL_USAGE_SNAPSHOT_METADATA_KEY, STRUCTURED_OUTPUT_RETRIES_EXHAUSTED_SUBTYPE,
};
use crate::codergen::{
    ActiveCodergenSession, CodergenBackend, CodergenBackendOutput, CodergenBackendRequest,
    CodergenBackendResponse, CodergenError, CodergenEvent, CodergenEventSink,
    CodergenSessionInterventionBroker,
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

/// Request metadata naming the native conversation a contract repair continues.
const NATIVE_RESUME_METADATA_KEY: &str = "spark.runtime.native.session_id";

#[derive(Clone)]
pub struct RustLlmCodergenBackend {
    client: Client,
    clarification_handler: Option<crate::codergen::ClarificationHandler>,
    cancel_check: Option<crate::codergen::CancelCheck>,
    intervention_broker: Option<CodergenSessionInterventionBroker>,
    /// The last native attempt's conversation, by session id, which its
    /// contract repair continues.
    resume: Option<(String, Vec<HistoryTurn>)>,
}

impl RustLlmCodergenBackend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            clarification_handler: None,
            cancel_check: None,
            intervention_broker: None,
            resume: None,
        }
    }

    pub fn with_intervention_broker(
        client: Client,
        intervention_broker: CodergenSessionInterventionBroker,
    ) -> Self {
        Self {
            client,
            clarification_handler: None,
            cancel_check: None,
            intervention_broker: Some(intervention_broker),
            resume: None,
        }
    }
}

impl CodergenBackend for RustLlmCodergenBackend {
    fn set_clarification_handler(&mut self, handler: crate::codergen::ClarificationHandler) {
        self.clarification_handler = Some(handler);
    }

    fn set_cancel_check(&mut self, check: crate::codergen::CancelCheck) {
        self.cancel_check = Some(check);
    }

    fn run(
        &mut self,
        request: CodergenBackendRequest,
    ) -> Result<CodergenBackendOutput, CodergenError> {
        self.run_with_event_sink(request, None)
    }

    fn run_with_event_sink(
        &mut self,
        request: CodergenBackendRequest,
        event_sink: Option<CodergenEventSink>,
    ) -> Result<CodergenBackendOutput, CodergenError> {
        if is_codex_provider_selector(&request.provider) {
            return self.run_codex_app_server_codergen(request, event_sink);
        }
        if is_claude_code_provider_selector(&request.provider) {
            return self.run_claude_code_codergen(request, event_sink);
        }
        if request.runtime_mode.requires_agent() {
            return self.run_agent_codergen(request, event_sink);
        }
        // Text-only completions have no incremental stream.
        self.run_text_only_codergen(request)
    }
}

impl RustLlmCodergenBackend {
    /// The conversation a repair attempt continues: the one its request names.
    fn resumed_history(&mut self, request: &CodergenBackendRequest) -> Vec<HistoryTurn> {
        let id = metadata_string(&request.metadata, NATIVE_RESUME_METADATA_KEY);
        match self.resume.take() {
            Some((resume_id, history)) if Some(&resume_id) == id.as_ref() => history,
            _ => Vec::new(),
        }
    }

    fn keep_for_repair(&mut self, history: Vec<HistoryTurn>) -> BTreeMap<String, Value> {
        let id = uuid::Uuid::new_v4().to_string();
        self.resume = Some((id.clone(), history));
        BTreeMap::from([(NATIVE_RESUME_METADATA_KEY.to_string(), json!(id))])
    }

    /// Lets run steering reach this node's agent turn as a live message,
    /// keyed like the turn itself.
    fn register_live_turn(
        &self,
        request: &CodergenBackendRequest,
        agent_request: &AgentTurnRequest,
        provider: &str,
        model: Option<String>,
        steering: SessionSteeringHandle,
    ) -> Option<crate::codergen::ActiveCodergenSessionGuard> {
        self.intervention_broker.as_ref().map(|broker| {
            broker.register(ActiveCodergenSession {
                node_id: request.node_id.clone(),
                child_run_id: metadata_string(&request.metadata, "spark.runtime.run_id"),
                root_run_id: metadata_string(&request.metadata, "spark.runtime.root_run_id"),
                provider: provider.to_string(),
                model,
                llm_profile: request.llm_profile.clone(),
                reasoning_effort: request.reasoning_effort.clone(),
                project_path: request.project_path.clone(),
                metadata: request.metadata.clone(),
                steering,
                live_turn: Some((
                    agent_request.project_path.clone(),
                    agent_request.conversation_id.clone(),
                )),
            })
        })
    }

    fn run_codex_app_server_codergen(
        &mut self,
        request: CodergenBackendRequest,
        event_sink: Option<CodergenEventSink>,
    ) -> Result<CodergenBackendOutput, CodergenError> {
        let mut agent_request = codergen_agent_turn_request(&request);
        if self.clarification_handler.is_some() {
            agent_request.prompt.push_str("\n\nInspect available evidence before asking for clarification. Use request_user_input for unresolved questions about the user’s intent that affect the outcome, then continue the same task after the answer.");
        }
        // Live prefix contract: the stream sink receives exactly the events
        // the output batch starts with (same mapping, same order). The same
        // mapped event also feeds the thread-local transcript sink.
        let sink_request = request.clone();
        let stream_sink = event_sink;
        let capture_event =
            crate::initial_context::path_from_metadata(&request.metadata).map(|_| {
                CodergenEvent::new(
                    "initial_context_captured",
                    BTreeMap::from([
                        ("node_id".to_string(), json!(request.node_id.clone())),
                        (
                            "context_capture_kind".to_string(),
                            json!("codex_turn_input"),
                        ),
                    ]),
                )
            });
        let capture_sink =
            stream_sink
                .clone()
                .zip(capture_event.clone())
                .map(|(sink, capture_event)| {
                    std::sync::Arc::new(move || sink(capture_event.clone()))
                        as std::sync::Arc<dyn Fn() + Send + Sync>
                });
        let turn_event_sink: Option<AgentTurnEventSink> =
            Some(std::sync::Arc::new(move |turn_event: TurnStreamEvent| {
                if let Some(sink) = &stream_sink {
                    sink(codergen_event_from_turn_stream_event(
                        &sink_request,
                        &turn_event,
                    ));
                }
            }) as AgentTurnEventSink);
        let _active_session = self.register_live_turn(
            &request,
            &agent_request,
            "codex",
            request.model.clone(),
            SessionSteeringHandle::new(),
        );
        let stop = NodeStop::watch(&request, &agent_request, self.cancel_check.clone())?;
        let result = CodexAppServerBackend::new().run_agent_turn_with_capture_observer(
            agent_request,
            turn_event_sink,
            capture_sink,
            stop.pausing(self.clarification_handler.clone()),
        );
        if let Some(output) = stop.finish() {
            return Ok(output);
        }
        let output = result.map_err(|error| {
            if error.artifact {
                CodergenError::Artifact(error.message)
            } else {
                codex_app_server_codergen_error(error)
            }
        })?;
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
        let mut events = capture_event.into_iter().collect::<Vec<_>>();
        for event in &output.events {
            let codergen_event = codergen_event_from_turn_stream_event(&request, event);
            events.push(codergen_event);
        }
        let completed_event = CodergenEvent::new(
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
                (
                    "context_capture_kind".to_string(),
                    json!("codex_turn_input"),
                ),
            ]),
        );
        events.push(completed_event);
        Ok(CodergenBackendOutput {
            response,
            events,
            usage,
            resume_metadata: BTreeMap::new(),
        })
    }

    fn run_claude_code_codergen(
        &mut self,
        request: CodergenBackendRequest,
        event_sink: Option<CodergenEventSink>,
    ) -> Result<CodergenBackendOutput, CodergenError> {
        let mut agent_request = codergen_agent_turn_request(&request);
        if self.clarification_handler.is_some() {
            agent_request.prompt.push_str("\n\nInspect available evidence before asking for clarification. Use AskUserQuestion for unresolved questions about the user’s intent that affect the outcome, then continue the same task after the answer.");
        }
        let capture_event =
            crate::initial_context::path_from_metadata(&request.metadata).map(|_| {
                CodergenEvent::new(
                    "initial_context_captured",
                    BTreeMap::from([
                        ("node_id".to_string(), json!(request.node_id.clone())),
                        (
                            "context_capture_kind".to_string(),
                            json!("claude_code_turn_input"),
                        ),
                    ]),
                )
            });
        if let (Some(sink), Some(capture_event)) = (event_sink.as_ref(), capture_event.as_ref()) {
            sink(capture_event.clone());
        }
        // Live prefix contract: the sink receives exactly the events the
        // output batch starts with (same mapping, same order).
        let sink_request = request.clone();
        let stream_sink = event_sink;
        let turn_event_sink: Option<AgentTurnEventSink> = stream_sink.as_ref().map(|sink| {
            let sink = sink.clone();
            std::sync::Arc::new(move |turn_event: TurnStreamEvent| {
                sink(claude_code_codergen_event_from_turn_stream_event(
                    &sink_request,
                    &turn_event,
                ));
            }) as AgentTurnEventSink
        });
        let _active_session = self.register_live_turn(
            &request,
            &agent_request,
            "claude-code",
            request.model.clone(),
            SessionSteeringHandle::new(),
        );
        let stop = NodeStop::watch(&request, &agent_request, self.cancel_check.clone())?;
        let result = ClaudeCodeBackend::new().run_codergen_turn(
            agent_request,
            turn_event_sink,
            stop.pausing(self.clarification_handler.clone()),
        );
        if let Some(output) = stop.finish() {
            return Ok(output);
        }
        let output = result.map_err(|error| CodergenError::Backend(error.message))?;
        let usage = output
            .token_usage
            .as_ref()
            .and_then(usage_from_claude_code_usage_payload);
        // A turn that exhausted its structured-output retries is a contract
        // violation the parser reports, so it is repaired like one.
        let structured_output_failed = output.events.iter().any(|event| {
            event.status.as_deref() == Some(STRUCTURED_OUTPUT_RETRIES_EXHAUSTED_SUBTYPE)
        });
        // Any text such a turn left behind failed the schema, so it is ignored.
        let response = if structured_output_failed {
            CodergenBackendResponse::Text(String::new())
        } else if let Some(text) = output.final_assistant_text.as_deref().and_then(non_empty) {
            CodergenBackendResponse::Text(text.to_string())
        } else {
            CodergenBackendResponse::Outcome(Outcome {
                status: OutcomeStatus::Fail,
                failure_reason: "claude code completed without assistant text".to_string(),
                retryable: Some(false),
                failure_kind: Some(FailureKind::Runtime),
                ..Outcome::new(OutcomeStatus::Fail)
            })
        };
        let mut events = capture_event.into_iter().collect::<Vec<_>>();
        for event in &output.events {
            events.push(claude_code_codergen_event_from_turn_stream_event(
                &request, event,
            ));
        }
        events.push(CodergenEvent::new(
            "claude_code_request_completed",
            BTreeMap::from([
                ("backend".to_string(), json!(CLAUDE_CODE_BACKEND)),
                ("node_id".to_string(), json!(request.node_id.clone())),
                (
                    "provider_selector".to_string(),
                    json!(request.provider.clone()),
                ),
                ("provider".to_string(), json!("claude-code")),
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
                (
                    "context_capture_kind".to_string(),
                    json!("claude_code_turn_input"),
                ),
            ]),
        ));
        let mut resume_metadata = BTreeMap::new();
        if let Some(session_id) = output.app_thread_id.as_deref().and_then(non_empty) {
            resume_metadata.insert(
                "spark.runtime.claude_code.session_id".to_string(),
                json!(session_id),
            );
            // The session's cumulative usage, which the resumed attempt subtracts.
            if let Some(model_usage) = output
                .token_usage
                .as_ref()
                .and_then(|usage| usage.get("model_usage"))
            {
                resume_metadata.insert(
                    MODEL_USAGE_SNAPSHOT_METADATA_KEY.to_string(),
                    model_usage.clone(),
                );
            }
        }
        Ok(CodergenBackendOutput {
            response,
            events,
            usage,
            resume_metadata,
        })
    }

    fn run_text_only_codergen(
        &mut self,
        request: CodergenBackendRequest,
    ) -> Result<CodergenBackendOutput, CodergenError> {
        let profile = normalize_optional(request.llm_profile.as_deref());
        let reasoning_effort = normalize_optional(request.reasoning_effort.as_deref());
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
        let mut history = self.resumed_history(&request);
        history.push(HistoryTurn::User(crate::history::UserTurn::new(
            request.prompt.clone(),
        )));
        let llm_request = build_llm_request(
            &self.client,
            crate::history::history_to_messages(&history),
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
        crate::initial_context::capture_if_configured(&request.metadata, &request.prompt)
            .map_err(|source| CodergenError::Artifact(source.to_string()))?;
        let mut llm_request = llm_request;
        llm_request.request.response_format = status_envelope_schema(&request).map(|json_schema| {
            unified_llm_adapter::ResponseFormat::JsonSchema {
                json_schema,
                strict: false,
            }
        });
        let agent_request = codergen_agent_turn_request(&request);
        let controller = unified_llm_adapter::AbortController::new();
        llm_request.request.abort_signal = Some(controller.signal());
        let key = (
            agent_request.project_path.as_str(),
            agent_request.conversation_id.as_str(),
        );
        let _turn = NativeTurn::register(key, None, move || controller.abort("Stopped."));
        let stop = NodeStop::watch(&request, &agent_request, self.cancel_check.clone())?;
        let result = self.client.complete(llm_request.request);
        if let Some(output) = stop.finish() {
            return Ok(output);
        }
        let response = result.map_err(codergen_adapter_error)?;
        let text = unified_llm_adapter::structured_response_text(&response);
        history.push(HistoryTurn::Assistant(crate::history::AssistantTurn::new(
            text.clone(),
        )));
        let resume_metadata = self.keep_for_repair(history);

        Ok(CodergenBackendOutput {
            response: CodergenBackendResponse::Text(text),
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
                    (
                        "context_capture_kind".to_string(),
                        json!("assembled_messages"),
                    ),
                ]),
            )],
            usage: Some(response.usage),
            resume_metadata,
        })
    }

    fn run_agent_codergen(
        &mut self,
        request: CodergenBackendRequest,
        event_sink: Option<CodergenEventSink>,
    ) -> Result<CodergenBackendOutput, CodergenError> {
        let mut agent_request = codergen_agent_turn_request(&request);
        agent_request.history = self.resumed_history(&request);
        let mut prompt = agent_request.prompt.clone();
        let mut session = build_agent_session(&self.client, agent_request.clone())
            .map_err(codergen_adapter_error)?;
        session.interaction.final_answer_schema = status_envelope_schema(&request);
        if self.clarification_handler.is_some() {
            prompt.push_str("\n\nInspect available evidence before asking for clarification. Use request_user_input for unresolved questions about the user’s intent that affect the outcome, then continue the same task after the answer.");
        }
        if let Some(sink) = event_sink {
            // Live prefix contract: emit the same per-event mapping the
            // post-run drain produces, in the same order — including events
            // already queued by session construction.
            let metadata_base = codergen_session_metadata_payload(&request, &session);
            let node_id = request.node_id.clone();
            let observer: std::sync::Arc<dyn Fn(&SessionEvent) + Send + Sync> =
                std::sync::Arc::new(move |event: &SessionEvent| {
                    for codergen_event in
                        codergen_events_for_session_event(&metadata_base, &node_id, event)
                    {
                        sink(codergen_event);
                    }
                });
            for event in session.event_queue.iter() {
                observer(event);
            }
            session.set_event_observer(observer);
        }
        let initial_history_len = session.history.len();
        let _turn = NativeTurn::register_session(
            (&agent_request.project_path, &agent_request.conversation_id),
            &mut session,
        );
        let _active_session = self.register_live_turn(
            &request,
            &agent_request,
            &request.provider,
            request
                .model
                .clone()
                .or_else(|| non_empty(&session.provider_profile.model).map(str::to_string)),
            session.steering_handle(),
        );
        let stop = NodeStop::watch(&request, &agent_request, self.cancel_check.clone())?;
        session.interaction.questions = stop.pausing(self.clarification_handler.clone());
        let process_error = session.process_input(&self.client, prompt).err();
        session.close();
        if let Some(output) = stop.finish() {
            return Ok(output);
        }
        if let Some(error) = process_error
            .as_ref()
            .filter(|error| error.error_code.as_deref() == Some("initial_context_artifact"))
        {
            return Err(CodergenError::Artifact(error.message.clone()));
        }
        let mut output =
            codergen_output_from_session(request, &mut session, initial_history_len, process_error);
        output.resume_metadata = self.keep_for_repair(std::mem::take(&mut session.history));
        Ok(output)
    }
}

/// How long a stopped agent turn may run on after its interrupt.
const STOP_GRACE: std::time::Duration = std::time::Duration::from_secs(10);

/// Interrupts a node's agent turn when its run is canceled or its `timeout`
/// elapses. Claude, Codex and native turns all stop through their turn
/// interrupt, so one watcher serves every backend.
struct NodeStop {
    reason: std::sync::Arc<std::sync::Mutex<Option<Outcome>>>,
    waits: std::sync::Arc<std::sync::Mutex<HumanWaits>>,
    done: std::sync::Arc<std::sync::atomic::AtomicBool>,
    watcher: Option<std::thread::JoinHandle<()>>,
}

impl NodeStop {
    fn watch(
        request: &CodergenBackendRequest,
        agent_request: &AgentTurnRequest,
        cancel_check: Option<crate::codergen::CancelCheck>,
    ) -> Result<Self, CodergenError> {
        let reason = std::sync::Arc::new(std::sync::Mutex::new(None));
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let waits = std::sync::Arc::default();
        let timeout = request.timeout_seconds;
        let deadline = timeout
            .map(|seconds| {
                std::time::Duration::try_from_secs_f64(seconds)
                    .ok()
                    .filter(|limit| !limit.is_zero())
                    .and_then(|limit| std::time::Instant::now().checked_add(limit))
                    .ok_or_else(|| {
                        CodergenError::Backend(format!(
                            "invalid node timeout {seconds} seconds: it must be a positive, representable duration"
                        ))
                    })
            })
            .transpose()?;
        if cancel_check.is_none() && deadline.is_none() {
            return Ok(Self {
                reason,
                waits,
                done,
                watcher: None,
            });
        }
        let (project_path, conversation_id) = (
            agent_request.project_path.clone(),
            agent_request.conversation_id.clone(),
        );
        let watcher = std::thread::spawn({
            let (reason, waits, done) = (reason.clone(), waits.clone(), done.clone());
            move || {
                let mut stopped_at = None;
                while !done.load(std::sync::atomic::Ordering::SeqCst) {
                    let mut slot = reason.lock().unwrap();
                    if slot.is_none() {
                        if cancel_check.as_ref().is_some_and(|canceled| canceled()) {
                            *slot = Some(Outcome {
                                status: OutcomeStatus::Fail,
                                failure_reason: "Run canceled while the node was running"
                                    .to_string(),
                                retryable: Some(false),
                                failure_kind: Some(FailureKind::Runtime),
                                ..Outcome::new(OutcomeStatus::Fail)
                            });
                        } else if deadline.is_some_and(|at| {
                            // Time spent waiting on a human answer does not count.
                            let waits = waits.lock().unwrap();
                            waits.open == 0
                                && at
                                    .checked_add(waits.total)
                                    .is_some_and(|at| std::time::Instant::now() >= at)
                        }) {
                            *slot = Some(Outcome {
                                status: OutcomeStatus::Fail,
                                failure_reason: format!(
                                    "Agent node timed out after {} seconds",
                                    timeout.unwrap_or_default()
                                ),
                                failure_kind: Some(FailureKind::Runtime),
                                ..Outcome::new(OutcomeStatus::Fail)
                            });
                        }
                    }
                    // Repeated until the turn ends: the process may not
                    // have registered its interrupt yet. A turn that cannot
                    // take an interrupt, or is still running a grace period
                    // after one, is terminated.
                    if slot.is_some() {
                        let stopped = *stopped_at.get_or_insert_with(std::time::Instant::now);
                        let interrupted = stopped.elapsed() < STOP_GRACE
                            && RustLlmAgentTurnBackend::interrupt(&project_path, &conversation_id);
                        if !interrupted {
                            RustLlmAgentTurnBackend::terminate(&project_path, &conversation_id);
                        }
                    }
                    drop(slot);
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
            }
        });
        Ok(Self {
            reason,
            waits,
            done,
            watcher: Some(watcher),
        })
    }

    /// The node's clarification handler, with its timeout paused while a
    /// question waits for its answer: a human gate never times out.
    fn pausing(
        &self,
        handler: Option<crate::codergen::ClarificationHandler>,
    ) -> Option<crate::codergen::ClarificationHandler> {
        let waits = self.waits.clone();
        handler.map(|handler| {
            std::sync::Arc::new(move |params| {
                let mut poll = handler(params)?;
                let mut wait = Some(HumanWait::open(waits.clone()));
                Ok(Box::new(move || {
                    let answer = poll();
                    if !matches!(answer, Ok(None)) {
                        drop(wait.take());
                    }
                    answer
                }) as crate::codergen::ClarificationPoll)
            }) as crate::codergen::ClarificationHandler
        })
    }

    /// Stops watching; a stopped node's output is its cancel or timeout failure.
    fn finish(mut self) -> Option<CodergenBackendOutput> {
        self.done.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(watcher) = self.watcher.take() {
            let _ = watcher.join();
        }
        let outcome = self.reason.lock().unwrap().take()?;
        Some(CodergenBackendOutput {
            response: CodergenBackendResponse::Outcome(outcome),
            events: Vec::new(),
            usage: None,
            resume_metadata: BTreeMap::new(),
        })
    }
}

/// A node's open human waits and the time it has spent in them.
#[derive(Default)]
struct HumanWaits {
    open: usize,
    since: Option<std::time::Instant>,
    total: std::time::Duration,
}

/// One question waiting for its answer, closed when answered or dropped.
struct HumanWait(std::sync::Arc<std::sync::Mutex<HumanWaits>>);

impl HumanWait {
    fn open(waits: std::sync::Arc<std::sync::Mutex<HumanWaits>>) -> Self {
        let mut state = waits.lock().unwrap();
        if state.open == 0 {
            state.since = Some(std::time::Instant::now());
        }
        state.open += 1;
        drop(state);
        Self(waits)
    }
}

impl Drop for HumanWait {
    fn drop(&mut self) {
        let mut state = self.0.lock().unwrap();
        state.open -= 1;
        if state.open == 0 {
            if let Some(since) = state.since.take() {
                state.total += since.elapsed();
            }
        }
    }
}

fn status_envelope_schema(request: &CodergenBackendRequest) -> Option<Value> {
    request
        .metadata
        .get(crate::codergen::STATUS_ENVELOPE_SCHEMA_METADATA_KEY)
        .filter(|schema| schema.is_object())
        .cloned()
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
        reasoning_effort: normalize_optional(request.reasoning_effort.as_deref()).map(|value| {
            if is_codex_provider_selector(&request.provider)
                || is_claude_code_provider_selector(&request.provider)
            {
                value.to_ascii_lowercase()
            } else {
                value
            }
        }),
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
    let final_assistant_text = codergen_final_assistant_text(session, initial_history_len);
    let session_events = drain_session_events(session);
    // Every request in the node, tool rounds included.
    let usage = (session.usage_total != Usage::default())
        .then(|| session.usage_total.clone())
        .or_else(|| {
            session_events
                .iter()
                .rev()
                .find_map(usage_from_session_event)
        });
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

    let metadata_base = codergen_session_metadata_payload(&request, session);
    let mut events = Vec::new();
    for event in &session_events {
        events.extend(codergen_events_for_session_event(
            &metadata_base,
            &request.node_id,
            event,
        ));
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
    let completed_event = CodergenEvent::new(
        "rust_agent_adapter_request_completed",
        codergen_completion_event_payload(&request, session, usage.as_ref()),
    );
    events.push(completed_event);

    CodergenBackendOutput {
        response,
        events,
        usage,
        resume_metadata: BTreeMap::new(),
    }
}

fn codergen_events_for_session_event(
    metadata_base: &BTreeMap<String, Value>,
    node_id: &str,
    event: &SessionEvent,
) -> Vec<CodergenEvent> {
    let mut events = vec![codergen_event_from_session_event_with_metadata(
        metadata_base,
        event,
    )];
    if let Some(raw_log_line) = raw_log_line_from_event(event) {
        events.push(CodergenEvent::new(
            "rust_agent_raw_log_line",
            BTreeMap::from([
                ("node_id".to_string(), json!(node_id)),
                ("direction".to_string(), json!(raw_log_line.direction)),
                ("line".to_string(), json!(raw_log_line.line)),
            ]),
        ));
    }
    events
}

fn codergen_event_from_session_event_with_metadata(
    metadata_base: &BTreeMap<String, Value>,
    event: &SessionEvent,
) -> CodergenEvent {
    let mut payload = metadata_base.clone();
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
        // The node's running total, which live run usage reads.
        if let Some(token_usage) = turn_stream_event.token_usage.as_ref() {
            payload.insert("token_usage".to_string(), token_usage.clone());
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

fn claude_code_codergen_event_from_turn_stream_event(
    request: &CodergenBackendRequest,
    event: &TurnStreamEvent,
) -> CodergenEvent {
    let mut payload = BTreeMap::from([
        ("node_id".to_string(), json!(request.node_id.clone())),
        (
            "backend".to_string(),
            json!(CLAUDE_CODE_BACKEND.to_string()),
        ),
        (
            "provider_selector".to_string(),
            json!(request.provider.clone()),
        ),
        ("provider".to_string(), json!("claude-code")),
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
    CodergenEvent::new("claude_code_session_event", payload)
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
    payload.insert(
        "context_capture_kind".to_string(),
        json!("assembled_messages"),
    );
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

fn codergen_final_assistant_text(session: &Session, initial_history_len: usize) -> Option<String> {
    session
        .history
        .iter()
        .skip(initial_history_len)
        .rev()
        .find_map(|turn| match turn {
            HistoryTurn::Assistant(assistant) => {
                let text = assistant.text();
                non_empty(&text).is_some().then_some(text)
            }
            _ => None,
        })
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

impl RustLlmAgentTurnBackend {
    /// A conversation runs one turn at a time, so at most one backend has it live.
    fn interrupt(project_path: &str, conversation_id: &str) -> bool {
        ClaudeCodeBackend::new().interrupt(project_path, conversation_id)
            || CodexAppServerBackend::new().interrupt(project_path, conversation_id)
            || NativeTurn::find(project_path, conversation_id).is_some_and(|turn| turn.stop())
    }

    /// A native turn runs in this process, so it ends the way it is
    /// interrupted: its request is aborted and its commands killed.
    fn terminate(project_path: &str, conversation_id: &str) -> bool {
        ClaudeCodeBackend::new().terminate(project_path, conversation_id)
            || CodexAppServerBackend::new().terminate(project_path, conversation_id)
            || NativeTurn::find(project_path, conversation_id).is_some_and(|turn| turn.stop())
    }

    /// The one path that sends a message into a live agent turn, for chat
    /// and run steering alike.
    pub(crate) fn send_live_message(
        project_path: &str,
        conversation_id: &str,
        message_id: &str,
        message: &str,
    ) -> bool {
        ClaudeCodeBackend::new().send_message(project_path, conversation_id, message_id, message)
            || CodexAppServerBackend::new().send_message(
                project_path,
                conversation_id,
                message_id,
                message,
            )
            || NativeTurn::find(project_path, conversation_id).is_some_and(|turn| {
                turn.steering
                    .as_ref()
                    .is_some_and(|steering| steering.queue_live_message(message_id, message))
            })
    }
}

type NativeTurnKey = (String, String);

/// A native turn running in this process, reachable by
/// `(project_path, conversation_id)` like a Claude or Codex turn.
type AnswerSender = std::sync::mpsc::Sender<BTreeMap<String, String>>;

struct NativeTurn {
    steering: Option<SessionSteeringHandle>,
    abort: Box<dyn Fn() + Send + Sync>,
    stopped: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The turn's unanswered chat questions, by request id.
    questions: std::sync::Arc<std::sync::Mutex<BTreeMap<String, AnswerSender>>>,
}

impl NativeTurn {
    fn registry() -> &'static std::sync::Mutex<
        std::collections::HashMap<NativeTurnKey, std::sync::Weak<NativeTurn>>,
    > {
        static TURNS: std::sync::OnceLock<
            std::sync::Mutex<std::collections::HashMap<NativeTurnKey, std::sync::Weak<NativeTurn>>>,
        > = std::sync::OnceLock::new();
        TURNS.get_or_init(Default::default)
    }

    /// Registered while the returned handle lives.
    fn register(
        (project_path, conversation_id): (&str, &str),
        steering: Option<SessionSteeringHandle>,
        abort: impl Fn() + Send + Sync + 'static,
    ) -> std::sync::Arc<Self> {
        let turn = std::sync::Arc::new(Self {
            steering,
            abort: Box::new(abort),
            stopped: Default::default(),
            questions: Default::default(),
        });
        let mut turns = Self::registry().lock().unwrap();
        turns.retain(|_, turn| turn.strong_count() > 0);
        turns.insert(
            (project_path.to_string(), conversation_id.to_string()),
            std::sync::Arc::downgrade(&turn),
        );
        turn
    }

    fn register_session(key: (&str, &str), session: &mut Session) -> std::sync::Arc<Self> {
        let abort = session.abort_handle();
        Self::register(key, Some(session.steering_handle()), move || {
            abort.abort_with_reason("Stopped.")
        })
    }

    fn find(project_path: &str, conversation_id: &str) -> Option<std::sync::Arc<Self>> {
        Self::registry()
            .lock()
            .unwrap()
            .get(&(project_path.to_string(), conversation_id.to_string()))
            .and_then(std::sync::Weak::upgrade)
    }

    /// Chat questions wait for the answer the request-user-input route
    /// delivers to this turn.
    fn question_handler(&self) -> crate::codergen::ClarificationHandler {
        let questions = self.questions.clone();
        std::sync::Arc::new(move |params: Value| {
            let request_id = params["agent_request_id"]
                .as_str()
                .ok_or("question request has no id")?
                .to_string();
            let (sender, answers) = std::sync::mpsc::channel();
            questions.lock().unwrap().insert(request_id, sender);
            Ok(Box::new(move || match answers.try_recv() {
                Ok(answers) => Ok(Some(answers)),
                Err(std::sync::mpsc::TryRecvError::Empty) => Ok(None),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Err("The question is no longer pending.".to_string())
                }
            }) as crate::codergen::ClarificationPoll)
        })
    }

    fn answer(&self, request_id: &str, answers: &BTreeMap<String, String>) -> bool {
        let sender = self.questions.lock().unwrap().remove(request_id);
        sender.is_some_and(|sender| sender.send(answers.clone()).is_ok())
    }

    fn stop(&self) -> bool {
        self.stopped
            .store(true, std::sync::atomic::Ordering::SeqCst);
        (self.abort)();
        true
    }
}

/// A stopped chat turn completes with its latest answer text, or
/// "Stopped.", instead of failing on the abort.
fn settle_stopped_turn(output: &mut AgentTurnOutput, event_sink: Option<&AgentTurnEventSink>) {
    output
        .events
        .retain(|event| event.kind != TurnStreamEventKind::Error);
    output.thread_resume_failure = None;
    let (source, text) = crate::events::latest_assistant_text(&output.events).unwrap_or_default();
    let text = non_empty(&text).unwrap_or("Stopped.").to_string();
    let source = TurnStreamSource {
        backend: Some("agent_session".to_string()),
        ..source
    };
    let answer = TurnStreamEvent {
        kind: TurnStreamEventKind::ContentCompleted,
        channel: Some(spark_common::events::TurnStreamChannel::Assistant),
        source: TurnStreamSource {
            raw_kind: Some("turn_interrupted".to_string()),
            ..source.clone()
        },
        content_delta: Some(text.clone()),
        message: Some(text.clone()),
        tool_call: None,
        request_user_input: None,
        token_usage: None,
        error: None,
        error_code: None,
        details: None,
        phase: Some("final_answer".to_string()),
        status: None,
    };
    let completed = TurnStreamEvent {
        kind: TurnStreamEventKind::TurnCompleted,
        channel: None,
        content_delta: None,
        message: None,
        phase: None,
        status: Some("interrupted".to_string()),
        source: TurnStreamSource {
            raw_kind: Some("turn_interrupted".to_string()),
            item_id: None,
            ..source
        },
        ..answer.clone()
    };
    for event in [answer, completed] {
        if let Some(sink) = event_sink {
            sink(event.clone());
        }
        output.events.push(event);
    }
    output.final_assistant_text = Some(text);
}

impl AgentTurnBackend for RustLlmAgentTurnBackend {
    fn interrupt_turn(&self, project_path: &str, conversation_id: &str) -> bool {
        Self::interrupt(project_path, conversation_id)
    }

    fn send_message(
        &self,
        project_path: &str,
        conversation_id: &str,
        message_id: &str,
        message: &str,
    ) -> bool {
        Self::send_live_message(project_path, conversation_id, message_id, message)
    }

    fn run_turn(&self, request: AgentTurnRequest) -> Result<AgentTurnOutput, AgentError> {
        self.run_turn_with_event_sink(request, None)
    }

    fn run_turn_with_event_sink(
        &self,
        request: AgentTurnRequest,
        event_sink: Option<AgentTurnEventSink>,
    ) -> Result<AgentTurnOutput, AgentError> {
        if is_codex_request_provider(request.provider.as_deref(), request.llm_profile.as_deref()) {
            return CodexAppServerBackend::new()
                .run_agent_turn_with_event_sink(request, event_sink)
                .map_err(codex_app_server_agent_error);
        }
        if request
            .provider
            .as_deref()
            .is_some_and(is_claude_code_provider_selector)
        {
            return ClaudeCodeBackend::new()
                .run_agent_turn_with_event_sink(request, event_sink)
                .map_err(claude_code_agent_error);
        }
        let attachments = crate::attachments::request_attachments(&request.metadata);
        let prompt = request.prompt.clone();
        let (project_path, conversation_id) = (
            request.project_path.clone(),
            request.conversation_id.clone(),
        );
        let session = build_agent_session(&self.client, request).map_err(agent_adapter_error)?;
        let input = crate::attachments::native_turn_input(
            &prompt,
            &attachments,
            session.provider_profile.supports("vision"),
        );
        self.run_native_turn(
            (&project_path, &conversation_id),
            session,
            input,
            event_sink,
        )
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
        if request
            .provider
            .as_deref()
            .is_some_and(is_claude_code_provider_selector)
        {
            return Ok(ClaudeCodeBackend::new().answer_request_user_input(request));
        }
        if NativeTurn::find(&request.project_path, &request.conversation_id)
            .is_some_and(|turn| turn.answer(&request.request_id, &request.answers))
        {
            let mut delivered = TurnStreamEvent::new(TurnStreamEventKind::Other(
                "request_user_input_answer_delivered".to_string(),
            ))
            .expect("answer delivery is a valid turn event");
            delivered.source = TurnStreamSource {
                backend: Some(BACKEND_NAME.to_string()),
                item_id: Some(request.request_id.clone()),
                raw_kind: Some("request_user_input_answer_delivered".to_string()),
                ..TurnStreamSource::default()
            };
            delivered.status = Some("delivered".to_string());
            return Ok(AgentTurnOutput {
                events: vec![delivered],
                ..AgentTurnOutput::default()
            });
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
                conversation_id: conversation_id.clone(),
                project_path: project_path.clone(),
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
        session.mark_awaiting_input(continuation.pending_question);
        self.run_native_turn(
            (&project_path, &conversation_id),
            session,
            continuation.answer_content,
            None,
        )
    }
}

impl RustLlmAgentTurnBackend {
    /// Runs one native turn, registered so it can be steered and stopped.
    fn run_native_turn(
        &self,
        key: (&str, &str),
        mut session: Session,
        input: impl Into<crate::history::TurnContent>,
        event_sink: Option<AgentTurnEventSink>,
    ) -> Result<AgentTurnOutput, AgentError> {
        let turn = NativeTurn::register_session(key, &mut session);
        if let Some(event_sink) = event_sink.clone() {
            // Only a streamed turn shows its questions to someone who can answer.
            session.interaction.questions = Some(turn.question_handler());
            // Mirror the codex live+batch shape: stream each session event to
            // the sink as it is emitted, while the drained queue still feeds
            // the durable output batch afterwards.
            let stopped = turn.stopped.clone();
            let observer: std::sync::Arc<dyn Fn(&SessionEvent) + Send + Sync> =
                std::sync::Arc::new(move |event: &SessionEvent| {
                    // A stopped turn's abort is not a failure.
                    if event.kind == EventKind::Error
                        && stopped.load(std::sync::atomic::Ordering::SeqCst)
                    {
                        return;
                    }
                    if let Some(turn_event) = event.to_turn_stream_event() {
                        event_sink(turn_event);
                    }
                });
            // Session construction already queued lifecycle events
            // (session_start); replay them so the sink sees the full turn.
            for event in session.event_queue.iter() {
                observer(event);
            }
            session.set_event_observer(observer);
        }
        let initial_history_len = session.history.len();
        let result = session.process_input(&self.client, input);
        session.finish_steering();
        let mut output = agent_output_from_session(&mut session, initial_history_len);
        if session.abort_signaled && turn.stopped.load(std::sync::atomic::Ordering::SeqCst) {
            settle_stopped_turn(&mut output, event_sink.as_ref());
            return Ok(output);
        }
        if let Err(error) = result {
            if agent_turn_output_has_failure(&output) {
                return Ok(output);
            }
            return Err(agent_adapter_error(error));
        }
        Ok(output)
    }
}

/// Output cap for one utility call; a reasoning budget is added on top.
pub const UTILITY_MAX_OUTPUT_TOKENS: u64 = 4096;
const UTILITY_MAX_TEXT_CHARS: usize = 8192;

/// One stateless model call for Spark housekeeping: no tools, no history, and
/// no persisted session. `metadata` carries `spark.execution.settings` as a
/// chat turn does, so reasoning controls and native agent settings resolve
/// the same way.
#[derive(Debug, Clone, Default)]
pub struct UtilityCall {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub llm_profile: Option<String>,
    pub reasoning_effort: Option<String>,
    pub instructions: String,
    pub input: String,
    pub metadata: BTreeMap<String, Value>,
    pub timeout: std::time::Duration,
}

/// Returns the model's trimmed text, or an error on provider failure,
/// timeout, or empty output.
pub fn run_utility_call(client: &Client, call: &UtilityCall) -> Result<String, String> {
    let provider = call.provider.as_deref().unwrap_or("");
    let text = if is_codex_provider_selector(provider) || is_claude_code_provider_selector(provider)
    {
        // A fresh empty directory keeps project instructions, settings and
        // files out of the call; it is removed when the call returns.
        let work_dir = tempfile::tempdir()
            .map_err(|error| format!("utility call directory failed: {error}"))?;
        let output = work_dir.path().join("output.txt");
        let command = utility_cli_command(call, work_dir.path(), &output)?;
        let stdout = run_utility_process(command, &call.input, call.timeout)?;
        if is_codex_provider_selector(provider) {
            std::fs::read_to_string(&output)
                .map_err(|error| format!("codex utility call wrote no output: {error}"))?
        } else {
            stdout
        }
    } else {
        let mut request =
            utility_llm_request(client, call).map_err(|e| format_adapter_error(&e))?;
        // Streaming honours the abort signal mid-request, so a timeout cancels
        // the HTTP call rather than abandoning it.
        let controller = unified_llm_adapter::AbortController::new();
        request.abort_signal = Some(controller.signal());
        let (done, finished) = std::sync::mpsc::channel::<()>();
        let timeout = call.timeout;
        std::thread::spawn(move || {
            if let Err(std::sync::mpsc::RecvTimeoutError::Timeout) = finished.recv_timeout(timeout)
            {
                controller.abort(utility_timeout_message(timeout));
            }
        });
        let mut accumulator = unified_llm_adapter::StreamAccumulator::default();
        let result = client.stream(request).and_then(|events| {
            for event in events {
                accumulator.push(event?);
            }
            Ok(accumulator.finalize().text())
        });
        drop(done);
        result.map_err(|error| format_adapter_error(&error))?
    };
    let text = text.trim();
    if text.is_empty() {
        return Err("utility model returned no text".into());
    }
    Ok(text.chars().take(UTILITY_MAX_TEXT_CHARS).collect())
}

/// Builds the single completion an API provider or LLM profile receives.
pub fn utility_llm_request(client: &Client, call: &UtilityCall) -> Result<Request, AdapterError> {
    let mut built = build_llm_request(
        client,
        vec![
            Message::system(call.instructions.clone()),
            Message::user(call.input.clone()),
        ],
        RequestSelection {
            provider: normalize_request_provider_selector(call.provider.as_deref().unwrap_or("")),
            model: normalize_model_selector(call.model.as_deref()),
            llm_profile: normalize_optional(call.llm_profile.as_deref()),
            reasoning_effort: normalize_optional(call.reasoning_effort.as_deref()),
            required_capabilities: ModelCapabilities::default(),
        },
        call.metadata.clone(),
    )?;
    built.request.max_tokens = Some(
        UTILITY_MAX_OUTPUT_TOKENS
            + built
                .request
                .thinking_budget_tokens
                .filter(|_| built.request.thinking.as_deref() == Some("budget"))
                .unwrap_or(0),
    );
    Ok(built.request)
}

/// Builds the Claude Code or Codex one-shot command: prompt on stdin, no
/// tools, no session persistence, run in `work_dir` outside any project.
pub fn utility_cli_command(
    call: &UtilityCall,
    work_dir: &std::path::Path,
    output: &std::path::Path,
) -> Result<std::process::Command, String> {
    let native =
        crate::config::captured_session_config(&call.metadata)?.map(|config| config.native);
    let model = call.model.as_deref().and_then(non_empty);
    let effort = call.reasoning_effort.as_deref().and_then(non_empty);
    let mut command;
    if is_codex_provider_selector(call.provider.as_deref().unwrap_or("")) {
        command = std::process::Command::new(
            native
                .as_ref()
                .and_then(|config| config.codex_binary.as_ref())
                .map(std::path::PathBuf::from)
                .unwrap_or_else(crate::codex_app_server::codex_executable),
        );
        command
            .envs(
                crate::codex_app_server::build_codex_runtime_environment_with_settings(
                    native.as_ref(),
                )
                .map_err(|error| error.message)?,
            )
            .args([
                "exec",
                "--ephemeral",
                "--sandbox",
                "read-only",
                "--skip-git-repo-check",
                "--ignore-rules",
                "-C",
            ])
            .arg(work_dir)
            .arg("-o")
            .arg(output)
            .arg("-c")
            .arg(format!(
                "developer_instructions={}",
                serde_json::to_string(&call.instructions).unwrap_or_default()
            ));
        if let Some(model) = model {
            command.arg("-m").arg(model);
        }
        if let Some(effort) = effort {
            command
                .arg("-c")
                .arg(format!("model_reasoning_effort={effort:?}"));
        }
        if let Some(summary) = call
            .metadata
            .get("spark.execution.settings")
            .and_then(|settings| settings["model_settings"]["reasoning_summary"].as_str())
        {
            command
                .arg("-c")
                .arg(format!("model_reasoning_summary={summary:?}"));
        }
        command.arg("-");
    } else {
        command = std::process::Command::new(
            native
                .as_ref()
                .and_then(|config| config.claude_binary.as_ref())
                .map(std::path::PathBuf::from)
                .unwrap_or_else(crate::claude_code::claude_code_executable),
        );
        command
            .args([
                "-p",
                // No user, project or local settings: hooks, permissions and
                // plugins stay out of the call; sign-in is unaffected.
                "--setting-sources",
                "",
                "--tools",
                "",
                "--strict-mcp-config",
                "--no-session-persistence",
                "--output-format",
                "text",
                "--system-prompt",
            ])
            .arg(&call.instructions)
            .env(
                "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
                UTILITY_MAX_OUTPUT_TOKENS.to_string(),
            );
        if let Some(model) = model {
            command.arg("--model").arg(model);
        }
        if let Some(effort) = effort {
            command.arg("--effort").arg(effort);
        }
        command.env(
            "CLAUDE_CONFIG_DIR",
            crate::claude_code::claude_config_dir(native.as_ref()),
        );
    }
    command.current_dir(work_dir);
    Ok(command)
}

fn run_utility_process(
    mut command: std::process::Command,
    input: &str,
    timeout: std::time::Duration,
) -> Result<String, String> {
    use std::io::{Read, Write};
    use std::process::Stdio;
    let program = command.get_program().to_string_lossy().into_owned();
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("utility model launch failed for {program}: {error}"))?;
    let mut stdin = child.stdin.take();
    let input = input.to_owned();
    std::thread::spawn(move || {
        if let Some(stdin) = stdin.as_mut() {
            let _ = stdin.write_all(input.as_bytes());
        }
    });
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_string(&mut text);
            }
            text
        })
    };
    let stdout = read(child.stdout.take().map(|pipe| Box::new(pipe) as _));
    let stderr = read(child.stderr.take().map(|pipe| Box::new(pipe) as _));
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("utility model wait failed: {error}"))?
        {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(utility_timeout_message(timeout));
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let stdout = stdout.join().unwrap_or_default();
    if !status.success() {
        let stderr = stderr.join().unwrap_or_default();
        let detail = non_empty(&stderr)
            .or_else(|| non_empty(&stdout))
            .unwrap_or("no output");
        return Err(format!(
            "utility model command failed ({status}): {}",
            detail.chars().take(500).collect::<String>()
        ));
    }
    Ok(stdout)
}

fn utility_timeout_message(timeout: std::time::Duration) -> String {
    format!(
        "utility model call timed out after {}s",
        timeout.as_secs_f64()
    )
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
        reasoning_effort: normalize_optional(reasoning_effort.as_deref()),
    };
    let mut metadata = metadata;
    let instructions = metadata
        .get(crate::agent::AGENT_INSTRUCTIONS_METADATA_KEY)
        .and_then(Value::as_str)
        .map(str::to_string);
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
    if llm_request.vision {
        profile.apply_profile_vision(unified_llm_adapter::get_model_info(&profile.model).as_ref());
    }
    let mut config: SessionConfig = llm_request
        .request
        .metadata
        .get("spark.execution.settings")
        .and_then(|snapshot| snapshot.get("configuration"))
        .and_then(|configuration| configuration.get("agents"))
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()
        .map_err(|_| {
            AdapterError::new(
                unified_llm_adapter::AdapterErrorKind::Configuration,
                "Invalid captured agent configuration.",
            )
        })?
        .unwrap_or_default();
    config.validate().map_err(|_| {
        AdapterError::new(
            unified_llm_adapter::AdapterErrorKind::Configuration,
            "Invalid captured agent configuration.",
        )
    })?;
    config.reasoning_effort = selection.reasoning_effort;
    let execution_environment = ExecutionEnvironment::from_backend(
        crate::local_environment::LocalExecutionEnvironment::with_options(
            project_path,
            config.default_command_timeout_ms,
            config.max_command_timeout_ms,
            config.environment_inheritance,
        ),
    )
    .with_metadata(llm_request.request.metadata);

    let plan_mode = metadata_chat_mode
        .as_deref()
        .is_some_and(|mode| mode.eq_ignore_ascii_case("plan"));
    if plan_mode {
        let mut registry = profile.registry();
        for name in registry.names() {
            if !crate::interaction::READ_ONLY_TOOLS.contains(&name.as_str()) {
                registry.unregister(&name);
            }
        }
        profile.set_tool_registry(registry);
    }
    let mut session = Session::new(profile, execution_environment, config);
    session.history = history;
    if plan_mode {
        session.interaction.plan_mode = true;
        session.system_prompt_snapshot = format!(
            "{}\n\n{}",
            session.system_prompt_snapshot,
            crate::interaction::PLAN_INSTRUCTIONS
        );
    }
    if let Some(instructions) = instructions {
        session.system_prompt_snapshot = format!(
            "{}\n\n<spark_instructions>\n{instructions}\n</spark_instructions>",
            session.system_prompt_snapshot
        );
    }
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
    /// The selected llm-profiles.toml profile declares `vision = true`.
    vision: bool,
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
    let vision = selected_profile
        .as_ref()
        .is_some_and(|profile| profile.vision);
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
        }
        .with_reasoning_settings(),
        llm_profile: selected_profile_id,
        reasoning_effort: selection.reasoning_effort,
        provider_profile_selector,
        vision,
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

fn claude_code_agent_error(error: crate::claude_code::ClaudeCodeError) -> AgentError {
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

fn is_claude_code_provider_selector(provider: &str) -> bool {
    let normalized = provider.trim();
    normalized.eq_ignore_ascii_case("claude-code") || normalized.eq_ignore_ascii_case("claude_code")
}

#[cfg(test)]
mod captured_settings_tests {
    use super::*;

    #[test]
    fn real_session_builder_consumes_captured_limits() {
        let client = Client::from_env_map(
            &BTreeMap::from([(
                "OPENAI_COMPATIBLE_BASE_URL".into(),
                "http://localhost:10001".into(),
            )]),
            None,
        )
        .unwrap();
        let request: AgentTurnRequest = serde_json::from_value(json!({
            "conversation_id": "capture", "project_path": ".", "prompt": "test", "history": [],
            "provider": "openai_compatible", "model": "test-model",
            "metadata": {"spark.execution.settings": {"configuration": {"agents": {"max_turns": 3, "default_command_timeout_ms": 123, "max_command_timeout_ms": 456, "tool_output_limits": {"shell": 789}, "enable_loop_detection": false}}}}
        })).unwrap();
        let session = build_agent_session(&client, request).unwrap();
        assert_eq!(session.config.max_turns, 3);
        assert_eq!(session.config.default_command_timeout_ms, 123);
        assert_eq!(session.config.tool_output_limits["shell"], 789);
        assert!(!session.config.enable_loop_detection);
    }
}
