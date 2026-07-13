use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::config::SessionConfig;
use crate::context::{context_usage_warning_payload, estimate_context_usage, ContextUsageEstimate};
use crate::environment::ExecutionEnvironment;
use crate::events::{EventKind, SessionEvent};
use unified_llm_adapter::{
    AbortController, AbortSignal, AdapterError, AdapterErrorKind, Client, Message, Request,
    Response, StreamAccumulator, StreamEventType, ToolChoice,
};

use crate::history::{
    history_to_messages, AssistantTurn, HistoryTurn, SteeringTurn, TurnContent, UserTurn,
};
use crate::profiles::ProviderProfile;
use crate::subagents::{
    close_active_subagents, is_subagent_tool_name, SubAgentHandle, SubAgentToolRuntime,
    SubAgentWorker,
};
use crate::tools::{ToolDispatchContext, ToolDispatchEvent, ToolHostControlHook, ToolHostControls};
use unified_llm_adapter::ToolCall;

pub const LOOP_DETECTION_WARNING: &str = "Repeated tool-call loop detected. Try a different approach instead of repeating the same tool calls.";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmClientHandle {
    #[serde(default)]
    pub backend: String,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Clone)]
struct ModelResponseOutput {
    response: Response,
    emitted_assistant_events: bool,
    emitted_model_tool_events: bool,
    stream_error: Option<AdapterError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ModelRequestContext {
    provider: Option<String>,
    model: Option<String>,
}

impl ModelRequestContext {
    fn from_request(request: &Request) -> Self {
        Self {
            provider: request
                .provider
                .as_deref()
                .and_then(non_empty)
                .map(str::to_string),
            model: non_empty(&request.model).map(str::to_string),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolCallSignature {
    pub name: String,
    pub arguments_hash: String,
}

impl Default for LlmClientHandle {
    fn default() -> Self {
        Self {
            backend: "unified_llm_adapter".to_string(),
            metadata: BTreeMap::new(),
        }
    }
}

impl LlmClientHandle {
    pub fn new(backend: impl Into<String>) -> Self {
        Self {
            backend: backend.into(),
            ..Self::default()
        }
    }
}

#[derive(Debug)]
struct SessionSteeringState {
    queue: VecDeque<SteeringTurn>,
    accepting: bool,
}

impl Default for SessionSteeringState {
    fn default() -> Self {
        Self {
            queue: VecDeque::new(),
            accepting: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionSteeringHandle {
    state: Arc<Mutex<SessionSteeringState>>,
}

impl Default for SessionSteeringHandle {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(SessionSteeringState::default())),
        }
    }
}

impl PartialEq for SessionSteeringHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }
}

impl SessionSteeringHandle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn queue_steering(&self, content: impl Into<TurnContent>) -> bool {
        let mut state = self.state.lock().expect("session steering queue lock");
        if !state.accepting {
            return false;
        }
        state.queue.push_back(SteeringTurn::new(content));
        true
    }

    pub fn queue_steering_with_metadata(
        &self,
        content: impl Into<TurnContent>,
        metadata: impl Into<BTreeMap<String, Value>>,
    ) -> bool {
        let mut state = self.state.lock().expect("session steering queue lock");
        if !state.accepting {
            return false;
        }
        state
            .queue
            .push_back(SteeringTurn::with_metadata(content, metadata));
        true
    }

    pub fn drain_queued(&self) -> Vec<SteeringTurn> {
        self.state
            .lock()
            .expect("session steering queue lock")
            .queue
            .drain(..)
            .collect()
    }

    fn drain_and_close_if_empty(&self) -> Vec<SteeringTurn> {
        let mut state = self.state.lock().expect("session steering queue lock");
        if state.queue.is_empty() {
            state.accepting = false;
            return Vec::new();
        }
        state.queue.drain(..).collect()
    }

    pub(crate) fn close(&self) {
        self.state
            .lock()
            .expect("session steering queue lock")
            .accepting = false;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Idle,
    Processing,
    AwaitingInput,
    Closed,
}

impl Default for SessionState {
    fn default() -> Self {
        Self::Idle
    }
}

/// Transient wiring, not session state: ignored by Debug/PartialEq/serde.
#[derive(Clone, Default)]
pub(crate) struct SessionEventObserver(
    pub(crate) Option<std::sync::Arc<dyn Fn(&SessionEvent) + Send + Sync>>,
);

impl std::fmt::Debug for SessionEventObserver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("SessionEventObserver")
            .field(&self.0.is_some())
            .finish()
    }
}

impl PartialEq for SessionEventObserver {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub provider_profile: ProviderProfile,
    pub execution_environment: ExecutionEnvironment,
    pub config: SessionConfig,
    pub state: SessionState,
    #[serde(default)]
    pub history: Vec<HistoryTurn>,
    #[serde(default)]
    pub event_queue: VecDeque<SessionEvent>,
    #[serde(default)]
    pub llm_client: LlmClientHandle,
    #[serde(default)]
    pub steering_queue: VecDeque<SteeringTurn>,
    #[serde(default)]
    pub follow_up_queue: VecDeque<UserTurn>,
    #[serde(default)]
    pub active_subagents: BTreeMap<String, SubAgentHandle>,
    /// Live observer fired for every emitted event, before it enters the
    /// queue — the seam that lets chat sinks and run journals stream while
    /// process_input is still executing. Post-hoc queue drains are unchanged.
    #[serde(default, skip)]
    pub(crate) event_observer: SessionEventObserver,
    #[serde(default, skip)]
    pub(crate) active_subagent_workers: BTreeMap<String, SubAgentWorker>,
    #[serde(default, skip)]
    pub(crate) external_steering: Option<SessionSteeringHandle>,
    #[serde(default)]
    pub system_prompt_snapshot: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_question: Option<String>,
    #[serde(default)]
    pub abort_signaled: bool,
    #[serde(default, skip)]
    abort_controller: AbortController,
    #[serde(default, skip)]
    cleanup_state: SessionCleanupState,
}

#[derive(Debug, Clone)]
struct SessionCleanupState {
    completed: Arc<AtomicBool>,
}

impl Default for SessionCleanupState {
    fn default() -> Self {
        Self {
            completed: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl PartialEq for SessionCleanupState {
    fn eq(&self, other: &Self) -> bool {
        self.completed.load(Ordering::SeqCst) == other.completed.load(Ordering::SeqCst)
    }
}

impl SessionCleanupState {
    fn cleanup(&self, environment: &ExecutionEnvironment) {
        if !self.completed.swap(true, Ordering::SeqCst) {
            let _ = environment.cleanup();
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionAbortHandle {
    signal: AbortSignal,
    execution_environment: ExecutionEnvironment,
    cleanup_state: SessionCleanupState,
}

impl SessionAbortHandle {
    pub fn abort(&self) {
        self.signal.abort("session is aborted");
        self.cleanup_state.cleanup(&self.execution_environment);
    }

    pub fn abort_with_reason(&self, reason: impl Into<String>) {
        self.signal.abort(reason.into());
        self.cleanup_state.cleanup(&self.execution_environment);
    }

    pub fn aborted(&self) -> bool {
        self.signal.aborted()
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new(
            ProviderProfile::default(),
            ExecutionEnvironment::default(),
            SessionConfig::default(),
        )
    }
}

impl Session {
    pub fn new(
        provider_profile: ProviderProfile,
        execution_environment: ExecutionEnvironment,
        config: SessionConfig,
    ) -> Self {
        let system_prompt_snapshot = provider_profile.build_system_prompt(&execution_environment);
        let mut session = Self {
            id: Uuid::new_v4(),
            provider_profile,
            execution_environment,
            config,
            state: SessionState::Idle,
            history: Vec::new(),
            event_queue: VecDeque::new(),
            llm_client: LlmClientHandle::default(),
            steering_queue: VecDeque::new(),
            follow_up_queue: VecDeque::new(),
            active_subagents: BTreeMap::new(),
            active_subagent_workers: BTreeMap::new(),
            event_observer: SessionEventObserver::default(),
            external_steering: None,
            system_prompt_snapshot,
            pending_question: None,
            abort_signaled: false,
            abort_controller: AbortController::new(),
            cleanup_state: SessionCleanupState::default(),
        };
        session.emit_kind(EventKind::SessionStart, state_payload(session.state));
        session
    }

    pub fn with_profile(provider_profile: ProviderProfile) -> Self {
        Self::new(
            provider_profile,
            ExecutionEnvironment::default(),
            SessionConfig::default(),
        )
    }

    pub fn profile(&self) -> &ProviderProfile {
        &self.provider_profile
    }

    pub fn environment(&self) -> &ExecutionEnvironment {
        &self.execution_environment
    }

    pub fn session_id(&self) -> Uuid {
        self.id
    }

    pub fn history_messages(&self) -> Vec<Message> {
        history_to_messages(&self.history)
    }

    pub fn build_request(&self, system_prompt: impl Into<String>) -> Request {
        let mut messages = Vec::with_capacity(self.history.len() + 1);
        messages.push(Message::system(system_prompt));
        messages.extend(self.history_messages());

        Request {
            model: self.provider_profile.model.clone(),
            messages,
            provider: provider_id(&self.provider_profile),
            tools: self.provider_profile.tool_definitions(),
            tool_choice: Some(ToolChoice::auto()),
            reasoning_effort: self.config.reasoning_effort.clone(),
            metadata: self.execution_environment.metadata.clone(),
            provider_options: self.provider_profile.request_provider_options(&self.config),
            abort_signal: Some(self.abort_signal()),
            ..Request::default()
        }
    }

    pub fn context_usage_estimate(&self, request: &Request) -> Option<ContextUsageEstimate> {
        let context_window_size = self.provider_profile.context_window_size?;
        (context_window_size > 0)
            .then(|| estimate_context_usage(&request.messages, context_window_size))
    }

    pub fn check_context_usage(&mut self, request: &Request) -> bool {
        let Some(estimate) = self.context_usage_estimate(request) else {
            return false;
        };
        if !estimate.exceeds_threshold {
            return false;
        }
        self.emit_kind(EventKind::Warning, context_usage_warning_payload(&estimate));
        true
    }

    pub fn process_input(
        &mut self,
        client: &Client,
        content: impl Into<TurnContent>,
    ) -> Result<(), AdapterError> {
        if self.state == SessionState::Closed {
            return Err(session_state_error("session is closed"));
        }
        if self.state == SessionState::Processing {
            return Err(session_state_error("session is already processing input"));
        }
        if !matches!(self.state, SessionState::Idle | SessionState::AwaitingInput) {
            return Err(session_state_error("session is not ready for input"));
        }

        let system_prompt = self.current_system_prompt();
        let mut current_input = Some(content.into());
        while let Some(input) = current_input.take() {
            self.start_processing_input(input);
            self.drain_steering_queue();

            let mut round_count = 0;
            loop {
                if self.limit_reached_before_model_request(round_count) {
                    return Ok(());
                }

                let request = self.build_request(system_prompt.clone());
                self.check_context_usage(&request);
                let request_context = ModelRequestContext::from_request(&request);
                let model_output = match self.model_response(client, request) {
                    Ok(model_output) => model_output,
                    Err(error) => {
                        self.mark_model_request_error(&error, Some(&request_context));
                        return Err(error);
                    }
                };
                let stream_error = model_output.stream_error.clone();
                let assistant_turn = self.record_model_response(model_output);
                if let Some(error) = stream_error {
                    self.mark_model_request_error(&error, Some(&request_context));
                    return Err(error);
                }
                if self.abort_requested() {
                    self.close_for_abort();
                    return Ok(());
                }

                if assistant_turn.tool_calls.is_empty() {
                    if self.drain_steering_queue_before_completion() {
                        round_count += 1;
                        continue;
                    }
                    if self.assistant_response_is_open_question(&assistant_turn) {
                        self.mark_awaiting_input(assistant_turn.text());
                        return Ok(());
                    }
                    break;
                }

                round_count += 1;
                let tool_results = self.execute_tool_calls(client, assistant_turn.tool_calls);
                if self.abort_requested() {
                    self.close_for_abort();
                    return Ok(());
                }
                self.history.push(HistoryTurn::ToolResults(
                    crate::history::ToolResultsTurn::new(tool_results),
                ));
                self.drain_steering_queue();
                self.maybe_emit_loop_detection_warning();
            }

            self.mark_natural_completion();
            let Some(follow_up) = self.follow_up_queue.pop_front() else {
                return Ok(());
            };
            current_input = Some(follow_up.content);
        }
        Ok(())
    }

    pub fn emit_event(&mut self, mut event: SessionEvent) {
        if event.session_id.is_none() {
            event.session_id = Some(self.id);
        }
        if let Some(observer) = &self.event_observer.0 {
            observer(&event);
        }
        self.event_queue.push_back(event);
    }

    pub fn set_event_observer(
        &mut self,
        observer: std::sync::Arc<dyn Fn(&SessionEvent) + Send + Sync>,
    ) {
        self.event_observer = SessionEventObserver(Some(observer));
    }

    fn current_system_prompt(&self) -> String {
        if self.system_prompt_snapshot.is_empty() {
            return self
                .provider_profile
                .build_system_prompt(&self.execution_environment);
        }
        self.system_prompt_snapshot.clone()
    }

    pub fn emit_kind(&mut self, kind: EventKind, data: BTreeMap<String, Value>) {
        self.emit_event(SessionEvent::new(kind, self.id, data));
    }

    pub fn next_event(&mut self) -> Option<SessionEvent> {
        self.event_queue.pop_front()
    }

    pub fn abort_handle(&self) -> SessionAbortHandle {
        SessionAbortHandle {
            signal: self.abort_signal(),
            execution_environment: self.execution_environment.clone(),
            cleanup_state: self.cleanup_state.clone(),
        }
    }

    fn abort_signal(&self) -> AbortSignal {
        self.abort_controller.signal()
    }

    fn abort_requested(&self) -> bool {
        self.abort_signaled || self.abort_controller.signal.aborted()
    }

    pub fn mark_processing(&mut self) {
        if self.state != SessionState::Closed {
            self.state = SessionState::Processing;
        }
    }

    pub fn mark_idle(&mut self) {
        if self.state != SessionState::Closed {
            self.state = SessionState::Idle;
            self.pending_question = None;
        }
    }

    pub fn mark_awaiting_input(&mut self, question: impl Into<String>) {
        if self.state != SessionState::Closed {
            self.state = SessionState::AwaitingInput;
            self.pending_question = Some(question.into());
        }
    }

    pub fn submit(&mut self, content: impl Into<TurnContent>) {
        self.submit_user_input(content);
    }

    pub fn submit_user_input(&mut self, content: impl Into<TurnContent>) {
        if matches!(self.state, SessionState::Closed | SessionState::Processing) {
            return;
        }

        let answer_to = self.pending_question.take();
        let user_turn = UserTurn::new(content);
        let mut payload =
            BTreeMap::from([("content".to_string(), Value::String(user_turn.text()))]);
        if let Some(question) = answer_to {
            payload.insert("answer_to".to_string(), Value::String(question));
        }
        self.history.push(HistoryTurn::User(user_turn));
        self.state = SessionState::Processing;
        self.emit_kind(EventKind::UserInput, payload);
    }

    pub fn mark_natural_completion(&mut self) {
        if self.state == SessionState::Closed {
            return;
        }
        self.state = SessionState::Idle;
        self.pending_question = None;
        self.emit_kind(EventKind::ProcessingEnd, state_payload(self.state));
    }

    pub fn mark_turn_limit(&mut self, round_count: Option<u32>, total_turns: Option<usize>) {
        if self.state == SessionState::Closed {
            return;
        }
        self.state = SessionState::Idle;
        self.pending_question = None;

        let mut payload = state_payload(self.state);
        if let Some(round_count) = round_count {
            payload.insert("round_count".to_string(), json!(round_count));
        }
        if let Some(total_turns) = total_turns {
            payload.insert("total_turns".to_string(), json!(total_turns));
        }
        self.emit_kind(EventKind::TurnLimit, payload);
        self.emit_kind(EventKind::ProcessingEnd, state_payload(self.state));
    }

    pub fn queue_steering(&mut self, content: impl Into<crate::history::TurnContent>) {
        self.steering_queue.push_back(SteeringTurn::new(content));
    }

    pub fn queue_steering_with_metadata(
        &mut self,
        content: impl Into<crate::history::TurnContent>,
        metadata: impl Into<BTreeMap<String, Value>>,
    ) {
        self.steering_queue
            .push_back(SteeringTurn::with_metadata(content, metadata));
    }

    pub fn steer(&mut self, content: impl Into<crate::history::TurnContent>) {
        self.queue_steering(content);
    }

    pub fn attach_steering_handle(&mut self, handle: SessionSteeringHandle) {
        self.external_steering = Some(handle);
    }

    pub fn steering_handle(&mut self) -> SessionSteeringHandle {
        if let Some(handle) = self.external_steering.as_ref() {
            return handle.clone();
        }
        let handle = SessionSteeringHandle::new();
        self.external_steering = Some(handle.clone());
        handle
    }

    pub fn queue_follow_up(&mut self, content: impl Into<crate::history::TurnContent>) {
        self.follow_up_queue.push_back(UserTurn::new(content));
    }

    pub fn follow_up(&mut self, content: impl Into<crate::history::TurnContent>) {
        self.queue_follow_up(content);
    }

    pub fn close(&mut self) {
        self.close_with_reason("explicit_close", None);
    }

    pub fn abort(&mut self) {
        if self.state == SessionState::Closed {
            return;
        }
        self.abort_signaled = true;
        self.abort_controller.abort("session is aborted");
        self.close_for_abort();
    }

    pub fn mark_unrecoverable_error(&mut self, error: impl Into<String>) {
        let mut error = AdapterError::new(AdapterErrorKind::Provider, error.into());
        error.retryable = false;
        self.mark_unrecoverable_adapter_error(&error, None);
    }

    fn mark_unrecoverable_adapter_error(
        &mut self,
        error: &AdapterError,
        context: Option<&ModelRequestContext>,
    ) {
        if self.state == SessionState::Closed {
            return;
        }
        let reason = "unrecoverable_error";
        let error_payload = adapter_error_value(error, context);
        let final_state =
            self.final_state_value(SessionState::Closed, reason, Some(error_payload.clone()));
        self.emit_kind(
            EventKind::Error,
            error_event_payload(error, error_payload.clone(), Some(final_state)),
        );
        self.cleanup_execution_environment();
        self.close_with_reason(reason, Some(error_payload));
    }

    fn mark_model_request_error(
        &mut self,
        error: &AdapterError,
        context: Option<&ModelRequestContext>,
    ) {
        if error.kind == AdapterErrorKind::Abort || self.abort_requested() {
            self.close_for_abort_with_error(error);
        } else if is_recoverable_model_error(error) {
            self.mark_recoverable_model_error(error, context);
        } else {
            self.mark_unrecoverable_adapter_error(error, context);
        }
    }

    fn mark_recoverable_model_error(
        &mut self,
        error: &AdapterError,
        context: Option<&ModelRequestContext>,
    ) {
        if self.state == SessionState::Closed {
            return;
        }
        self.emit_kind(EventKind::Warning, warning_event_payload(error, context));
        self.mark_natural_completion();
    }

    fn close_for_abort(&mut self) {
        let error = self.abort_adapter_error();
        self.close_for_abort_with_error(&error);
    }

    fn close_for_abort_with_error(&mut self, error: &AdapterError) {
        if self.state == SessionState::Closed {
            return;
        }
        self.abort_signaled = true;
        self.abort_controller.abort(error.message.clone());
        let reason = "abort";
        let error_payload = adapter_error_value(error, None);
        let final_state =
            self.final_state_value(SessionState::Closed, reason, Some(error_payload.clone()));
        self.emit_kind(
            EventKind::Error,
            error_event_payload(error, error_payload.clone(), Some(final_state)),
        );
        self.cleanup_execution_environment();
        self.close_with_reason(reason, Some(error_payload));
    }

    fn abort_adapter_error(&self) -> AdapterError {
        let message = self
            .abort_controller
            .signal
            .reason()
            .filter(|reason| !reason.trim().is_empty())
            .unwrap_or_else(|| "session is aborted".to_string());
        AdapterError::new(AdapterErrorKind::Abort, message)
    }

    fn cleanup_execution_environment(&self) {
        self.cleanup_state.cleanup(&self.execution_environment);
    }

    fn close_with_reason(&mut self, reason: &'static str, error: Option<Value>) {
        if let Some(external_steering) = self.external_steering.as_ref() {
            external_steering.close();
        }
        if self.state == SessionState::Closed {
            return;
        }
        close_active_subagents(
            &mut self.active_subagents,
            &mut self.active_subagent_workers,
        );
        self.state = SessionState::Closed;
        self.pending_question = None;
        self.emit_kind(
            EventKind::SessionEnd,
            self.final_state_payload(reason, error),
        );
    }

    fn final_state_payload(
        &self,
        reason: &'static str,
        error: Option<Value>,
    ) -> BTreeMap<String, Value> {
        let final_state = self.final_state_value(self.state, reason, error.clone());
        if let Some(error) = error {
            return BTreeMap::from([
                (
                    "state".to_string(),
                    Value::String(state_value(self.state).to_string()),
                ),
                ("reason".to_string(), Value::String(reason.to_string())),
                ("error".to_string(), error),
                ("final_state".to_string(), final_state),
            ]);
        }

        BTreeMap::from([
            (
                "state".to_string(),
                Value::String(state_value(self.state).to_string()),
            ),
            ("reason".to_string(), Value::String(reason.to_string())),
            ("final_state".to_string(), final_state),
        ])
    }

    fn final_state_value(
        &self,
        state: SessionState,
        reason: &'static str,
        error: Option<Value>,
    ) -> Value {
        let mut final_state = json!({
            "state": state_value(state),
            "reason": reason,
            "abort_signaled": self.abort_signaled,
            "history_turns": self.history.len(),
            "active_subagents": self.active_subagents.len(),
            "pending_question": self.pending_question.clone(),
        });
        if let Some(error) = error {
            if let Some(object) = final_state.as_object_mut() {
                object.insert("error".to_string(), error.clone());
            }
        }
        final_state
    }

    fn start_processing_input(&mut self, content: impl Into<TurnContent>) {
        let answer_to = self.pending_question.take();
        let user_turn = UserTurn::new(content);
        let mut payload =
            BTreeMap::from([("content".to_string(), Value::String(user_turn.text()))]);
        if let Some(question) = answer_to {
            payload.insert("answer_to".to_string(), Value::String(question));
        }

        self.history.push(HistoryTurn::User(user_turn));
        self.state = SessionState::Processing;
        self.emit_kind(EventKind::UserInput, payload);
    }

    fn drain_steering_queue(&mut self) -> bool {
        self.drain_steering_queue_inner(false)
    }

    fn drain_steering_queue_before_completion(&mut self) -> bool {
        self.drain_steering_queue_inner(true)
    }

    fn drain_steering_queue_inner(&mut self, close_external_if_empty: bool) -> bool {
        let has_local_steering = !self.steering_queue.is_empty();
        if let Some(external_steering) = self.external_steering.as_ref() {
            let steering = if close_external_if_empty && !has_local_steering {
                external_steering.drain_and_close_if_empty()
            } else {
                external_steering.drain_queued()
            };
            self.steering_queue.extend(steering);
        }
        let mut drained = false;
        while let Some(steering_turn) = self.steering_queue.pop_front() {
            let text = steering_turn.text();
            let mut payload = BTreeMap::from([("content".to_string(), Value::String(text))]);
            payload.extend(steering_turn.metadata.clone());
            self.history.push(HistoryTurn::Steering(steering_turn));
            self.emit_kind(EventKind::SteeringInjected, payload);
            drained = true;
        }
        drained
    }

    fn maybe_emit_loop_detection_warning(&mut self) -> bool {
        if !self.config.enable_loop_detection {
            return false;
        }
        if !detect_loop(&self.history, self.config.loop_detection_window) {
            return false;
        }

        self.history.push(HistoryTurn::Steering(SteeringTurn::new(
            LOOP_DETECTION_WARNING,
        )));
        self.emit_kind(
            EventKind::LoopDetection,
            BTreeMap::from([(
                "message".to_string(),
                Value::String(LOOP_DETECTION_WARNING.to_string()),
            )]),
        );
        true
    }

    fn limit_reached_before_model_request(&mut self, round_count: u32) -> bool {
        if self.abort_requested() {
            self.close_for_abort();
            return true;
        }

        if self.config.max_tool_rounds_per_input > 0
            && round_count >= self.config.max_tool_rounds_per_input
        {
            self.mark_turn_limit(Some(round_count), Some(self.history.len()));
            return true;
        }

        if self.config.max_turns > 0 && self.history.len() >= self.config.max_turns as usize {
            self.mark_turn_limit(Some(round_count), Some(self.history.len()));
            return true;
        }

        false
    }

    fn model_response(
        &mut self,
        client: &Client,
        request: Request,
    ) -> Result<ModelResponseOutput, AdapterError> {
        let initial_context = crate::initial_context::assembled_message_text(&request.messages);
        crate::initial_context::capture_if_configured(&request.metadata, &initial_context)
            .map_err(|source| {
                let mut error = AdapterError::new(
                    unified_llm_adapter::AdapterErrorKind::InvalidRequest,
                    format!("persist initial LLM context: {source}"),
                );
                error.error_code = Some("initial_context_artifact".to_string());
                error
            })?;
        if self.provider_profile.supports("streaming") {
            self.stream_response(client, request)
        } else {
            client
                .complete(request)
                .map(|response| ModelResponseOutput {
                    response,
                    emitted_assistant_events: false,
                    emitted_model_tool_events: false,
                    stream_error: None,
                })
        }
    }

    fn stream_response(
        &mut self,
        client: &Client,
        request: Request,
    ) -> Result<ModelResponseOutput, AdapterError> {
        let model = request.model.clone();
        let provider = request.provider.clone().unwrap_or_default();
        let abort_signal = request.abort_signal.clone();
        let mut stream = client.stream(request)?;
        let mut accumulator = StreamAccumulator::default();
        let mut response_id: Option<String> = None;
        let mut assistant_text_started = false;
        let mut assistant_reasoning_started = false;
        let mut assistant_reasoning_ended = false;
        let mut emitted_model_tool_events = false;
        let mut pending_usage_events = Vec::new();
        let mut stream_error = None;

        for event in stream.by_ref() {
            let event = match event {
                Ok(event) => event,
                Err(error) => {
                    if error.kind == AdapterErrorKind::Abort
                        || abort_signal.as_ref().is_some_and(AbortSignal::aborted)
                    {
                        let _ = stream.close();
                        return Err(error);
                    }
                    stream_error = Some(error);
                    break;
                }
            };

            if abort_signal.as_ref().is_some_and(AbortSignal::aborted) {
                let _ = stream.close();
                return Err(self.abort_adapter_error());
            }

            accumulator.push(event.clone());
            let current_response_id = non_empty_string(accumulator.response.id.clone());
            if current_response_id.is_some() {
                response_id = current_response_id;
            }

            match &event.r#type {
                StreamEventType::TextStart => {
                    self.emit_stream_session_events(&event, response_id.as_deref(), None, None);
                    assistant_text_started = true;
                }
                StreamEventType::TextDelta => {
                    if !assistant_text_started {
                        self.emit_assistant_text_start(response_id.as_deref());
                        assistant_text_started = true;
                    }
                    self.emit_stream_session_events(&event, response_id.as_deref(), None, None);
                }
                StreamEventType::TextEnd => {
                    if !assistant_text_started {
                        self.emit_assistant_text_start(response_id.as_deref());
                        assistant_text_started = true;
                    }
                }
                StreamEventType::ReasoningStart => {
                    self.emit_stream_session_events(&event, response_id.as_deref(), None, None);
                    assistant_reasoning_started = true;
                }
                StreamEventType::ReasoningDelta => {
                    if !assistant_reasoning_started {
                        self.emit_kind(
                            EventKind::AssistantReasoningStart,
                            response_payload(response_id.as_deref()),
                        );
                        assistant_reasoning_started = true;
                    }
                    self.emit_stream_session_events(&event, response_id.as_deref(), None, None);
                }
                StreamEventType::ReasoningEnd => {
                    if !assistant_reasoning_started {
                        self.emit_kind(
                            EventKind::AssistantReasoningStart,
                            response_payload(response_id.as_deref()),
                        );
                        assistant_reasoning_started = true;
                    }
                    let reasoning = accumulator.response.reasoning();
                    self.emit_stream_session_events(
                        &event,
                        response_id.as_deref(),
                        None,
                        reasoning.as_deref(),
                    );
                    assistant_reasoning_ended = true;
                }
                StreamEventType::ToolCallStart
                | StreamEventType::ToolCallDelta
                | StreamEventType::ToolCallEnd => {
                    emitted_model_tool_events |=
                        self.emit_stream_session_events(&event, response_id.as_deref(), None, None);
                }
                StreamEventType::Finish => {
                    pending_usage_events.extend(
                        SessionEvent::from_stream_event(&event, self.id, response_id.as_deref())
                            .into_iter()
                            .filter(|event| event.kind == EventKind::ModelUsageUpdate),
                    );
                }
                StreamEventType::Error => {
                    pending_usage_events.extend(
                        SessionEvent::from_stream_event(&event, self.id, response_id.as_deref())
                            .into_iter()
                            .filter(|event| event.kind == EventKind::ModelUsageUpdate),
                    );
                    let error = event.error.clone().unwrap_or_else(|| {
                        AdapterError::new(AdapterErrorKind::Stream, "model stream error")
                    });
                    if error.kind == AdapterErrorKind::Abort {
                        let _ = stream.close();
                        return Err(error);
                    }
                    stream_error = Some(error);
                    break;
                }
                StreamEventType::StreamStart
                | StreamEventType::ProviderEvent
                | StreamEventType::Custom(_) => {}
            }
        }
        if abort_signal.as_ref().is_some_and(AbortSignal::aborted) {
            let _ = stream.close();
            return Err(self.abort_adapter_error());
        }
        if let Err(error) = stream.close() {
            if error.kind == AdapterErrorKind::Abort {
                return Err(error);
            }
            if stream_error.is_none() {
                stream_error = Some(error);
            }
        }
        let mut response = accumulator.finalize();
        if response.model.is_empty() {
            response.model = model;
        }
        if response.provider.is_empty() {
            response.provider = provider;
        }

        if response_id.is_none() {
            response_id = non_empty_string(response.id.clone());
        }
        let response_text = response.text();
        let reasoning = response.reasoning();
        if assistant_reasoning_started && !assistant_reasoning_ended {
            self.emit_kind(
                EventKind::AssistantReasoningEnd,
                reasoning_end_payload(response_id.as_deref(), reasoning.as_deref()),
            );
        }
        if assistant_text_started {
            self.emit_assistant_text_end(
                &response_text,
                reasoning.as_deref(),
                response_id.as_deref(),
            );
        } else {
            self.emit_assistant_text_events(
                &response_text,
                reasoning.as_deref(),
                response_id.as_deref(),
            );
        }
        for usage_event in pending_usage_events {
            self.emit_event(usage_event);
        }

        Ok(ModelResponseOutput {
            response,
            emitted_assistant_events: true,
            emitted_model_tool_events,
            stream_error,
        })
    }

    fn record_model_response(&mut self, model_output: ModelResponseOutput) -> AssistantTurn {
        let response = model_output.response;
        let response_text = response.text();
        let response_id = non_empty_string(response.id.clone());
        let reasoning = response.reasoning();
        if !model_output.emitted_assistant_events {
            self.emit_assistant_text_events(
                &response_text,
                reasoning.as_deref(),
                response_id.as_deref(),
            );
        }

        let mut assistant_turn = AssistantTurn::new(response_text);
        assistant_turn.tool_calls = response.tool_calls();
        assistant_turn.reasoning = reasoning;
        assistant_turn.usage = Some(response.usage.clone());
        assistant_turn.response_id = response_id;
        assistant_turn.finish_reason = Some(response.finish_reason.clone());
        assistant_turn.raw = response.raw.clone();
        assistant_turn.warnings = response.warnings.clone();

        if !model_output.emitted_model_tool_events {
            for tool_call in &assistant_turn.tool_calls {
                self.emit_kind(
                    EventKind::ModelToolCallEnd,
                    BTreeMap::from([(
                        "tool_call".to_string(),
                        serde_json::to_value(tool_call).expect("tool call is serializable"),
                    )]),
                );
            }
        }

        self.history.push(HistoryTurn::Assistant(assistant_turn));
        match self.history.last().expect("assistant turn was just pushed") {
            HistoryTurn::Assistant(assistant_turn) => assistant_turn.clone(),
            _ => unreachable!("assistant turn was just pushed"),
        }
    }

    fn execute_tool_calls(
        &mut self,
        client: &Client,
        tool_calls: impl IntoIterator<Item = ToolCall>,
    ) -> Vec<unified_llm_adapter::ToolResult> {
        let tool_calls = tool_calls.into_iter().collect::<Vec<_>>();
        let has_subagent_calls = tool_calls
            .iter()
            .any(|tool_call| is_subagent_tool_name(&tool_call.name));
        let events = Arc::new(Mutex::new(Vec::<ToolDispatchEvent>::new()));
        let captured_events = events.clone();
        let queued_steering = Arc::new(Mutex::new(Vec::<SteeringTurn>::new()));
        let captured_steering = queued_steering.clone();
        let queued_follow_ups = Arc::new(Mutex::new(Vec::<UserTurn>::new()));
        let captured_follow_ups = queued_follow_ups.clone();
        let steering_hook: ToolHostControlHook = Arc::new(move |content| {
            captured_steering
                .lock()
                .expect("tool queued steering")
                .push(SteeringTurn::new(content));
        });
        let follow_up_hook: ToolHostControlHook = Arc::new(move |content| {
            captured_follow_ups
                .lock()
                .expect("tool queued follow-up")
                .push(UserTurn::new(content));
        });
        let registry = self.provider_profile.registry();
        let subagent_runtime = SubAgentToolRuntime::from_session(self, client.clone());
        let results = registry.dispatch_many(
            tool_calls,
            ToolDispatchContext {
                execution_environment: self.execution_environment.clone(),
                messages: self.history_messages(),
                config: self.config.clone(),
                capabilities: self.provider_profile.capability_flags().clone(),
                supports_parallel_tool_calls: self.provider_profile.supports("parallel_tool_calls")
                    && !has_subagent_calls,
                host_controls: ToolHostControls::new(Some(steering_hook), Some(follow_up_hook)),
                subagent_runtime: Some(subagent_runtime.clone()),
                event_hook: Some(Arc::new(move |event| {
                    captured_events
                        .lock()
                        .expect("tool dispatch events")
                        .push(event);
                })),
                ..ToolDispatchContext::default()
            },
        );
        subagent_runtime.restore_into(self);

        let events = events.lock().expect("tool dispatch events").clone();
        for event in events {
            self.emit_kind(event.kind, event.data);
        }
        if self.abort_requested() {
            return results;
        }
        self.steering_queue.extend(
            queued_steering
                .lock()
                .expect("tool queued steering")
                .iter()
                .cloned(),
        );
        self.follow_up_queue.extend(
            queued_follow_ups
                .lock()
                .expect("tool queued follow-up")
                .iter()
                .cloned(),
        );

        results
    }

    fn assistant_response_is_open_question(&self, assistant_turn: &AssistantTurn) -> bool {
        let response_text = assistant_turn.text();
        let response_text = response_text.trim_end();
        !response_text.is_empty() && response_text.ends_with('?')
    }

    fn emit_stream_session_events(
        &mut self,
        event: &unified_llm_adapter::StreamEvent,
        response_id: Option<&str>,
        accumulated_text: Option<&str>,
        accumulated_reasoning: Option<&str>,
    ) -> bool {
        let events = SessionEvent::from_stream_event_with_accumulated(
            event,
            self.id,
            response_id,
            accumulated_text,
            accumulated_reasoning,
        );
        let emitted = !events.is_empty();
        for event in events {
            self.emit_event(event);
        }
        emitted
    }

    fn emit_assistant_text_events(
        &mut self,
        text: &str,
        reasoning: Option<&str>,
        response_id: Option<&str>,
    ) {
        self.emit_assistant_text_start(response_id);
        self.emit_assistant_text_delta(text, response_id);
        self.emit_assistant_text_end(text, reasoning, response_id);
    }

    fn emit_assistant_text_start(&mut self, response_id: Option<&str>) {
        self.emit_kind(EventKind::AssistantTextStart, response_payload(response_id));
    }

    fn emit_assistant_text_delta(&mut self, delta: &str, response_id: Option<&str>) {
        let mut payload = response_payload(response_id);
        payload.insert("delta".to_string(), Value::String(delta.to_string()));
        self.emit_kind(EventKind::AssistantTextDelta, payload);
    }

    fn emit_assistant_text_end(
        &mut self,
        text: &str,
        reasoning: Option<&str>,
        response_id: Option<&str>,
    ) {
        let mut payload = response_payload(response_id);
        payload.insert("text".to_string(), Value::String(text.to_string()));
        payload.insert(
            "reasoning".to_string(),
            reasoning
                .map(|value| Value::String(value.to_string()))
                .unwrap_or(Value::Null),
        );
        self.emit_kind(EventKind::AssistantTextEnd, payload);
    }
}

fn state_payload(state: SessionState) -> BTreeMap<String, Value> {
    BTreeMap::from([(
        "state".to_string(),
        Value::String(state_value(state).to_string()),
    )])
}

fn state_value(state: SessionState) -> &'static str {
    match state {
        SessionState::Idle => "idle",
        SessionState::Processing => "processing",
        SessionState::AwaitingInput => "awaiting_input",
        SessionState::Closed => "closed",
    }
}

fn provider_id(profile: &ProviderProfile) -> Option<String> {
    profile.request_provider_id()
}

fn response_payload(response_id: Option<&str>) -> BTreeMap<String, Value> {
    response_id
        .filter(|value| !value.is_empty())
        .map(|value| {
            BTreeMap::from([("response_id".to_string(), Value::String(value.to_string()))])
        })
        .unwrap_or_default()
}

fn reasoning_end_payload(
    response_id: Option<&str>,
    reasoning: Option<&str>,
) -> BTreeMap<String, Value> {
    let mut payload = response_payload(response_id);
    payload.insert(
        "text".to_string(),
        Value::String(reasoning.unwrap_or_default().to_string()),
    );
    payload
}

fn non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn non_empty_string(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn session_state_error(message: impl Into<String>) -> AdapterError {
    AdapterError::new(AdapterErrorKind::InvalidRequest, message)
}

fn is_recoverable_model_error(error: &AdapterError) -> bool {
    error.kind == AdapterErrorKind::ContextLength || is_delegated_transient_provider_error(error)
}

fn is_delegated_transient_provider_error(error: &AdapterError) -> bool {
    matches!(
        error.kind,
        AdapterErrorKind::RateLimit | AdapterErrorKind::Network
    ) || matches!(error.status_code, Some(429 | 500..=503))
        || (error.kind == AdapterErrorKind::Server
            && error
                .status_code
                .map(|status_code| (500..=503).contains(&status_code))
                .unwrap_or(true))
}

fn warning_event_payload(
    error: &AdapterError,
    context: Option<&ModelRequestContext>,
) -> BTreeMap<String, Value> {
    let error_payload = adapter_error_value(error, context);
    let mut payload = BTreeMap::from([
        ("message".to_string(), Value::String(error.message.clone())),
        ("error".to_string(), error_payload.clone()),
    ]);
    copy_error_summary_fields(&mut payload, &error_payload);
    payload
}

fn error_event_payload(
    error: &AdapterError,
    error_payload: Value,
    final_state: Option<Value>,
) -> BTreeMap<String, Value> {
    let mut payload = BTreeMap::from([
        ("message".to_string(), Value::String(error.message.clone())),
        ("error".to_string(), error_payload.clone()),
    ]);
    copy_error_summary_fields(&mut payload, &error_payload);
    if let Some(final_state) = final_state {
        payload.insert("final_state".to_string(), final_state);
    }
    payload
}

fn copy_error_summary_fields(payload: &mut BTreeMap<String, Value>, error_payload: &Value) {
    let Some(error_object) = error_payload.as_object() else {
        return;
    };
    for (source_key, target_key) in [
        ("kind", "error_kind"),
        ("name", "name"),
        ("code", "code"),
        ("error_code", "error_code"),
        ("provider", "provider"),
        ("model", "model"),
        ("retryable", "retryable"),
        ("status_code", "status_code"),
        ("retry_after", "retry_after"),
    ] {
        if let Some(value) = error_object.get(source_key) {
            payload.insert(target_key.to_string(), value.clone());
        }
    }
    if !payload.contains_key("error_code") {
        if let Some(code) = error_object.get("code") {
            payload.insert("error_code".to_string(), code.clone());
        }
    }
}

fn adapter_error_value(error: &AdapterError, context: Option<&ModelRequestContext>) -> Value {
    let mut value = serde_json::to_value(error).expect("adapter error is serializable");
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "name".to_string(),
            Value::String(error.kind.spec_error_name().to_string()),
        );
        let kind_value = serde_json::to_value(error.kind).expect("error kind is serializable");
        let code = error
            .error_code
            .clone()
            .or_else(|| kind_value.as_str().map(str::to_string));
        if let Some(code) = code {
            object
                .entry("code".to_string())
                .or_insert(Value::String(code));
        }
        if error.provider.is_none() {
            if let Some(provider) = context.and_then(|context| context.provider.clone()) {
                object.insert("provider".to_string(), Value::String(provider));
            }
        }
        if let Some(model) = context.and_then(|context| context.model.clone()) {
            object.insert("model".to_string(), Value::String(model));
        }
    }
    value
}

pub fn detect_loop(history: &[HistoryTurn], loop_detection_window: u32) -> bool {
    if loop_detection_window <= 1 {
        return false;
    }

    let signatures = tool_call_signatures(history);
    let window = loop_detection_window as usize;
    if signatures.len() < window {
        return false;
    }

    let recent_signatures = &signatures[signatures.len() - window..];
    for pattern_length in 1..=3 {
        if pattern_length * 2 > window {
            continue;
        }
        if window % pattern_length != 0 {
            continue;
        }
        let pattern = &recent_signatures[..pattern_length];
        if recent_signatures
            .iter()
            .enumerate()
            .all(|(index, signature)| signature == &pattern[index % pattern_length])
        {
            return true;
        }
    }

    false
}

pub fn tool_call_signature(tool_call: &ToolCall) -> ToolCallSignature {
    let arguments = if tool_call.arguments.is_null() {
        tool_call
            .raw_arguments
            .as_deref()
            .map(canonicalize_raw_arguments)
            .unwrap_or(Value::Null)
    } else {
        canonicalize_argument_value(&tool_call.arguments)
    };
    ToolCallSignature {
        name: tool_call.name.clone(),
        arguments_hash: stable_digest(&stable_json(&arguments)),
    }
}

fn tool_call_signatures(history: &[HistoryTurn]) -> Vec<ToolCallSignature> {
    history
        .iter()
        .flat_map(|turn| match turn {
            HistoryTurn::Assistant(assistant_turn) => assistant_turn.tool_calls.as_slice(),
            _ => &[],
        })
        .map(tool_call_signature)
        .collect()
}

fn canonicalize_raw_arguments(raw_arguments: &str) -> Value {
    serde_json::from_str::<Value>(raw_arguments)
        .map(|value| canonicalize_argument_value(&value))
        .unwrap_or_else(|_| Value::String(raw_arguments.to_string()))
}

fn canonicalize_argument_value(value: &Value) -> Value {
    match value {
        Value::String(_) => value.clone(),
        Value::Array(values) => {
            Value::Array(values.iter().map(canonicalize_argument_value).collect())
        }
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), canonicalize_argument_value(value)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn stable_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string()),
        Value::Array(values) => {
            let items = values.iter().map(stable_json).collect::<Vec<_>>().join(",");
            format!("[{items}]")
        }
        Value::Object(object) => {
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            let items = entries
                .into_iter()
                .map(|(key, value)| {
                    let key = serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string());
                    format!("{key}:{}", stable_json(value))
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{items}}}")
        }
    }
}

fn stable_digest(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
