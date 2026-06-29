use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::config::SessionConfig;
use crate::events::{EventKind, SessionEvent};
use unified_llm_adapter::{
    AdapterError, AdapterErrorKind, Client, Message, Request, Response, StreamAccumulator,
    StreamEventType, ToolChoice,
};

use crate::history::{
    history_to_messages, AssistantTurn, HistoryTurn, SteeringTurn, TurnContent, UserTurn,
};
use crate::profiles::ProviderProfile;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionEnvironment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<PathBuf>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

impl Default for ExecutionEnvironment {
    fn default() -> Self {
        Self {
            working_dir: None,
            env: BTreeMap::new(),
            metadata: BTreeMap::new(),
        }
    }
}

impl ExecutionEnvironment {
    pub fn local(working_dir: impl Into<PathBuf>) -> Self {
        Self {
            working_dir: Some(working_dir.into()),
            ..Self::default()
        }
    }
}

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
    pub active_subagents: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_question: Option<String>,
    #[serde(default)]
    pub abort_signaled: bool,
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
            pending_question: None,
            abort_signaled: false,
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
            ..Request::default()
        }
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

        self.start_processing_input(content);
        self.drain_steering_queue();

        let round_count = 0;
        if self.limit_reached_before_model_request(round_count) {
            return Ok(());
        }

        let system_prompt = self
            .provider_profile
            .build_system_prompt(&self.execution_environment);
        let request = self.build_request(system_prompt);
        let model_output = match self.model_response(client, request) {
            Ok(model_output) => model_output,
            Err(error) => {
                self.mark_unrecoverable_error(error.message.clone());
                return Err(error);
            }
        };
        let stream_error = model_output.stream_error.clone();
        self.record_model_response(model_output);
        if let Some(error) = stream_error {
            self.mark_unrecoverable_error(error.message.clone());
            return Err(error);
        }

        let recorded_tool_rounds = self
            .history
            .last()
            .and_then(|turn| match turn {
                HistoryTurn::Assistant(assistant) if !assistant.tool_calls.is_empty() => Some(1),
                _ => None,
            })
            .unwrap_or(0);
        if recorded_tool_rounds > 0
            && self.config.max_tool_rounds_per_input > 0
            && recorded_tool_rounds >= self.config.max_tool_rounds_per_input
        {
            self.mark_turn_limit(Some(recorded_tool_rounds), Some(self.history.len()));
            return Ok(());
        }

        self.mark_natural_completion();
        Ok(())
    }

    pub fn emit_event(&mut self, mut event: SessionEvent) {
        if event.session_id.is_none() {
            event.session_id = Some(self.id);
        }
        self.event_queue.push_back(event);
    }

    pub fn emit_kind(&mut self, kind: EventKind, data: BTreeMap<String, Value>) {
        self.emit_event(SessionEvent::new(kind, self.id, data));
    }

    pub fn next_event(&mut self) -> Option<SessionEvent> {
        self.event_queue.pop_front()
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

    pub fn queue_follow_up(&mut self, content: impl Into<crate::history::TurnContent>) {
        self.follow_up_queue.push_back(UserTurn::new(content));
    }

    pub fn close(&mut self) {
        self.close_with_reason("explicit_close", None);
    }

    pub fn abort(&mut self) {
        self.abort_signaled = true;
        self.close_with_reason("abort", None);
    }

    pub fn mark_unrecoverable_error(&mut self, error: impl Into<String>) {
        if self.state == SessionState::Closed {
            return;
        }
        let error = error.into();
        self.emit_kind(
            EventKind::Error,
            BTreeMap::from([("error".to_string(), Value::String(error.clone()))]),
        );
        self.close_with_reason("unrecoverable_error", Some(error));
    }

    fn close_with_reason(&mut self, reason: &'static str, error: Option<String>) {
        if self.state == SessionState::Closed {
            return;
        }
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
        error: Option<String>,
    ) -> BTreeMap<String, Value> {
        let mut final_state = json!({
            "state": state_value(self.state),
            "reason": reason,
            "abort_signaled": self.abort_signaled,
            "history_turns": self.history.len(),
            "active_subagents": self.active_subagents.len(),
            "pending_question": self.pending_question.clone(),
        });
        if let Some(error) = error {
            if let Some(object) = final_state.as_object_mut() {
                object.insert("error".to_string(), Value::String(error));
            }
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

    fn drain_steering_queue(&mut self) {
        while let Some(steering_turn) = self.steering_queue.pop_front() {
            let text = steering_turn.text();
            self.history.push(HistoryTurn::Steering(steering_turn));
            self.emit_kind(
                EventKind::SteeringInjected,
                BTreeMap::from([("content".to_string(), Value::String(text))]),
            );
        }
    }

    fn limit_reached_before_model_request(&mut self, round_count: u32) -> bool {
        if self.abort_signaled {
            self.mark_turn_limit(Some(round_count), Some(self.history.len()));
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
                    stream_error = Some(error);
                    break;
                }
            };

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
                    stream_error = Some(event.error.clone().unwrap_or_else(|| {
                        AdapterError::new(AdapterErrorKind::Stream, "model stream error")
                    }));
                    break;
                }
                StreamEventType::StreamStart
                | StreamEventType::ProviderEvent
                | StreamEventType::Custom(_) => {}
            }
        }
        if let Err(error) = stream.close() {
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

    fn record_model_response(&mut self, model_output: ModelResponseOutput) {
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
    let id = profile.id.trim();
    (!id.is_empty()).then(|| id.to_string())
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

fn non_empty_string(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn session_state_error(message: impl Into<String>) -> AdapterError {
    AdapterError::new(AdapterErrorKind::InvalidRequest, message)
}
