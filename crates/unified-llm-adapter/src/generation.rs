use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::{Client, LlmProfileRoute};
use crate::errors::{AdapterError, AdapterErrorKind};
use crate::events::{
    StreamAccumulator, StreamEvent, StreamEventStream, StreamEventType, StreamEvents,
};
use crate::request::{
    ContentPart, FinishReason, FinishReasonKind, Message, MessageRole, Request, Response,
    ResponseFormat, ToolCall, ToolResult, ToolResultData, Warning,
};
use crate::resolution::{
    resolve_high_level_provider_and_model, ActiveLlmProfile, HighLevelLlmResolutionInputs,
    ModelCapabilities,
};
use crate::retry::{retry, retry_with_hooks, RetryPolicy};
use crate::structured::{validate_json_value, STRUCTURED_OUTPUT_TOOL_NAME};
use crate::timeouts::{abort_error, timeout_error, AbortSignal, TimeoutConfig};
use crate::tools::{Tool, ToolChoice, ToolInvocation, ToolRepair, ToolRepairInvocation};
use crate::usage::Usage;

#[derive(Clone)]
pub struct StopWhen(Arc<dyn Fn(&[StepResult]) -> bool + Send + Sync + 'static>);

impl StopWhen {
    pub fn new<F>(stop_when: F) -> Self
    where
        F: Fn(&[StepResult]) -> bool + Send + Sync + 'static,
    {
        Self(Arc::new(stop_when))
    }

    pub fn should_stop(&self, steps: &[StepResult]) -> bool {
        (self.0)(steps)
    }
}

impl fmt::Debug for StopWhen {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StopWhen(..)")
    }
}

impl PartialEq for StopWhen {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerateRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messages: Option<Vec<Message>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_profile: Option<ActiveLlmProfile>,
    #[serde(default)]
    pub required_capabilities: ModelCapabilities,
    #[serde(default)]
    pub tools: Vec<Tool>,
    #[serde(skip)]
    pub repair_tool_call: Option<ToolRepair>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stop_sequences: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
    #[serde(default)]
    pub provider_options: BTreeMap<String, Value>,
    #[serde(default = "default_max_tool_rounds")]
    pub max_tool_rounds: usize,
    #[serde(skip)]
    pub stop_when: Option<StopWhen>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<TimeoutConfig>,
    #[serde(skip)]
    pub abort_signal: Option<AbortSignal>,
}

impl Default for GenerateRequest {
    fn default() -> Self {
        Self {
            prompt: None,
            messages: None,
            system: None,
            model: None,
            provider: None,
            active_profile: None,
            required_capabilities: ModelCapabilities::default(),
            tools: Vec::new(),
            repair_tool_call: None,
            tool_choice: None,
            response_format: None,
            temperature: None,
            top_p: None,
            max_tokens: None,
            stop_sequences: Vec::new(),
            reasoning_effort: None,
            metadata: BTreeMap::new(),
            provider_options: BTreeMap::new(),
            max_tool_rounds: DEFAULT_MAX_TOOL_ROUNDS,
            stop_when: None,
            timeout: None,
            abort_signal: None,
        }
    }
}

impl From<Request> for GenerateRequest {
    fn from(request: Request) -> Self {
        Self {
            prompt: None,
            messages: Some(request.messages),
            system: None,
            model: non_empty_owned(request.model),
            provider: request.provider,
            active_profile: None,
            required_capabilities: ModelCapabilities::default(),
            tools: request.tools,
            repair_tool_call: None,
            tool_choice: request.tool_choice,
            response_format: request.response_format,
            temperature: request.temperature,
            top_p: request.top_p,
            max_tokens: request.max_tokens,
            stop_sequences: request.stop_sequences,
            reasoning_effort: request.reasoning_effort,
            metadata: request.metadata,
            provider_options: request.provider_options,
            max_tool_rounds: DEFAULT_MAX_TOOL_ROUNDS,
            stop_when: None,
            timeout: request.timeout,
            abort_signal: request.abort_signal,
        }
    }
}

#[derive(Clone)]
struct PreparedGenerateRequest {
    request: Request,
    max_tool_rounds: usize,
    stop_when: Option<StopWhen>,
    repair_tool_call: Option<ToolRepair>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepResult {
    pub request: Request,
    pub response: Response,
    pub text: String,
    pub reasoning: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub tool_results: Vec<ToolResult>,
    pub finish_reason: FinishReason,
    pub usage: Usage,
    pub warnings: Vec<Warning>,
}

pub type GenerateStep = StepResult;

#[derive(Debug, Clone, PartialEq)]
pub struct GenerateResult {
    pub text: String,
    pub reasoning: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub tool_results: Vec<ToolResult>,
    pub finish_reason: FinishReason,
    pub usage: Usage,
    pub total_usage: Usage,
    pub response: Response,
    pub warnings: Vec<Warning>,
    pub steps: Vec<StepResult>,
    pub output: Option<Value>,
}

pub fn generate<I>(client: &Client, input: I) -> Result<GenerateResult, AdapterError>
where
    I: Into<GenerateRequest>,
{
    generate_with_policy(client, input, &RetryPolicy::default())
}

pub fn generate_with_policy<I>(
    client: &Client,
    input: I,
    policy: &RetryPolicy,
) -> Result<GenerateResult, AdapterError>
where
    I: Into<GenerateRequest>,
{
    let prepared = build_generate_request(client, input.into())?;
    generate_active_steps_with_policy(client, prepared, policy)
}

pub fn generate_with_policy_and_hooks<I, R, S>(
    client: &Client,
    input: I,
    policy: &RetryPolicy,
    random_multiplier: R,
    sleeper: S,
) -> Result<GenerateResult, AdapterError>
where
    I: Into<GenerateRequest>,
    R: FnMut() -> f64,
    S: FnMut(f64),
{
    let prepared = build_generate_request(client, input.into())?;
    generate_active_steps_with_policy_and_hooks(
        client,
        prepared,
        policy,
        random_multiplier,
        sleeper,
    )
}

pub fn stream<I>(client: &Client, input: I) -> Result<StreamResult, AdapterError>
where
    I: Into<GenerateRequest>,
{
    stream_with_policy(client, input, &RetryPolicy::default())
}

pub fn stream_with_policy<I>(
    client: &Client,
    input: I,
    policy: &RetryPolicy,
) -> Result<StreamResult, AdapterError>
where
    I: Into<GenerateRequest>,
{
    stream_with_policy_and_hooks(
        client,
        input,
        policy,
        default_stream_random_multiplier,
        default_stream_sleep,
    )
}

pub fn stream_with_policy_and_hooks<I, R, S>(
    client: &Client,
    input: I,
    policy: &RetryPolicy,
    random_multiplier: R,
    sleeper: S,
) -> Result<StreamResult, AdapterError>
where
    I: Into<GenerateRequest>,
    R: FnMut() -> f64 + Send + 'static,
    S: FnMut(f64) + Send + 'static,
{
    let prepared = build_generate_request(client, input.into())?;
    let control = OperationControl::from_request(&prepared.request);
    control.check_before("stream")?;
    let stream_control = control.clone();
    let events = ActiveToolStream::new(
        client.clone(),
        prepared,
        policy.clone(),
        random_multiplier,
        sleeper,
    )?;
    Ok(StreamResult::new_with_control(events, stream_control))
}

pub fn generate_steps_with_policy<N>(
    client: &Client,
    initial_request: Request,
    policy: &RetryPolicy,
    mut next_request: N,
) -> Result<GenerateResult, AdapterError>
where
    N: FnMut(&[StepResult]) -> Result<Option<Request>, AdapterError>,
{
    let mut current_request = initial_request;
    let mut steps = Vec::new();
    let control = OperationControl::from_request(&current_request);

    loop {
        let request_for_call = current_request.clone();
        let response = retry(policy, || {
            control.check_before("generation")?;
            control.check_step_before("generation step")?;
            let started_at = Instant::now();
            let response = client.complete(request_for_call.clone())?;
            control.check_after_step(started_at, "generation step")?;
            Ok(response)
        })?;
        steps.push(StepResult::from_request_response(current_request, response));

        control.check_before("tool execution")?;
        let Some(next) = next_request(&steps)? else {
            control.check_before("generation")?;
            return finish_generation(steps);
        };
        control.check_before("generation")?;
        current_request = control.apply_to_request(next);
    }
}

pub fn generate_steps_with_policy_and_hooks<N, R, S>(
    client: &Client,
    initial_request: Request,
    policy: &RetryPolicy,
    mut next_request: N,
    mut random_multiplier: R,
    mut sleeper: S,
) -> Result<GenerateResult, AdapterError>
where
    N: FnMut(&[StepResult]) -> Result<Option<Request>, AdapterError>,
    R: FnMut() -> f64,
    S: FnMut(f64),
{
    let mut current_request = initial_request;
    let mut steps = Vec::new();
    let control = OperationControl::from_request(&current_request);

    loop {
        let request_for_call = current_request.clone();
        let response = retry_with_hooks(
            policy,
            || {
                control.check_before("generation")?;
                control.check_step_before("generation step")?;
                let started_at = Instant::now();
                let response = client.complete(request_for_call.clone())?;
                control.check_after_step(started_at, "generation step")?;
                Ok(response)
            },
            &mut random_multiplier,
            &mut sleeper,
        )?;
        steps.push(StepResult::from_request_response(current_request, response));

        control.check_before("tool execution")?;
        let Some(next) = next_request(&steps)? else {
            control.check_before("generation")?;
            return finish_generation(steps);
        };
        control.check_before("generation")?;
        current_request = control.apply_to_request(next);
    }
}

fn generate_active_steps_with_policy(
    client: &Client,
    initial_request: PreparedGenerateRequest,
    policy: &RetryPolicy,
) -> Result<GenerateResult, AdapterError> {
    let PreparedGenerateRequest {
        request: mut current_request,
        max_tool_rounds,
        stop_when,
        repair_tool_call,
    } = initial_request;
    let mut steps = Vec::new();
    let mut tool_rounds_executed = 0;
    let control = OperationControl::from_request(&current_request);

    loop {
        let request_for_call = current_request.clone();
        let response = retry(policy, || {
            control.check_before("generation")?;
            control.check_step_before("generation step")?;
            let started_at = Instant::now();
            let response = client.complete(request_for_call.clone())?;
            control.check_after_step(started_at, "generation step")?;
            Ok(response)
        })?;

        control.check_before("tool execution")?;
        let tool_results = execute_active_tool_round(
            &current_request,
            &response,
            tool_rounds_executed,
            max_tool_rounds,
            repair_tool_call.as_ref(),
        )?;
        let next_request = next_tool_round_request(&current_request, &response, &tool_results);
        let should_continue = next_request.is_some();
        steps.push(StepResult::from_request_response_with_tool_results(
            current_request,
            response,
            tool_results,
        ));

        if !should_continue {
            control.check_before("generation")?;
            return finish_generation(steps);
        }
        if stop_when
            .as_ref()
            .is_some_and(|stop_when| stop_when.should_stop(&steps))
        {
            control.check_before("generation")?;
            return finish_generation(steps);
        }

        tool_rounds_executed += 1;
        control.check_before("generation")?;
        current_request = control.apply_to_request(next_request.expect("checked above"));
    }
}

fn generate_active_steps_with_policy_and_hooks<R, S>(
    client: &Client,
    initial_request: PreparedGenerateRequest,
    policy: &RetryPolicy,
    mut random_multiplier: R,
    mut sleeper: S,
) -> Result<GenerateResult, AdapterError>
where
    R: FnMut() -> f64,
    S: FnMut(f64),
{
    let PreparedGenerateRequest {
        request: mut current_request,
        max_tool_rounds,
        stop_when,
        repair_tool_call,
    } = initial_request;
    let mut steps = Vec::new();
    let mut tool_rounds_executed = 0;
    let control = OperationControl::from_request(&current_request);

    loop {
        let request_for_call = current_request.clone();
        let response = retry_with_hooks(
            policy,
            || {
                control.check_before("generation")?;
                control.check_step_before("generation step")?;
                let started_at = Instant::now();
                let response = client.complete(request_for_call.clone())?;
                control.check_after_step(started_at, "generation step")?;
                Ok(response)
            },
            &mut random_multiplier,
            &mut sleeper,
        )?;

        control.check_before("tool execution")?;
        let tool_results = execute_active_tool_round(
            &current_request,
            &response,
            tool_rounds_executed,
            max_tool_rounds,
            repair_tool_call.as_ref(),
        )?;
        let next_request = next_tool_round_request(&current_request, &response, &tool_results);
        let should_continue = next_request.is_some();
        steps.push(StepResult::from_request_response_with_tool_results(
            current_request,
            response,
            tool_results,
        ));

        if !should_continue {
            control.check_before("generation")?;
            return finish_generation(steps);
        }
        if stop_when
            .as_ref()
            .is_some_and(|stop_when| stop_when.should_stop(&steps))
        {
            control.check_before("generation")?;
            return finish_generation(steps);
        }

        tool_rounds_executed += 1;
        control.check_before("generation")?;
        current_request = control.apply_to_request(next_request.expect("checked above"));
    }
}

#[derive(Debug, Clone)]
struct OperationControl {
    abort_signal: Option<AbortSignal>,
    timeout: Option<TimeoutConfig>,
    started_at: Instant,
}

impl OperationControl {
    fn none() -> Self {
        Self {
            abort_signal: None,
            timeout: None,
            started_at: Instant::now(),
        }
    }

    fn from_request(request: &Request) -> Self {
        Self {
            abort_signal: request.abort_signal.clone(),
            timeout: request.timeout,
            started_at: Instant::now(),
        }
    }

    fn check_before(&self, scope: &str) -> Result<(), AdapterError> {
        self.check_abort(scope)?;
        self.check_total(scope)
    }

    fn check_after_step(&self, step_started_at: Instant, scope: &str) -> Result<(), AdapterError> {
        self.check_abort(scope)?;
        if let Some(timeout) = self.timeout.and_then(|timeout| timeout.per_step) {
            check_elapsed_timeout(step_started_at, timeout, scope)?;
        }
        self.check_total(scope)
    }

    fn check_step_before(&self, scope: &str) -> Result<(), AdapterError> {
        if let Some(timeout) = self.timeout.and_then(|timeout| timeout.per_step) {
            check_zero_timeout(timeout, scope)?;
        }
        Ok(())
    }

    fn check_stream_read_before(&self) -> Result<(), AdapterError> {
        self.check_before("stream")?;
        if let Some(timeout) = self.timeout.and_then(|timeout| timeout.per_step) {
            check_elapsed_timeout(self.started_at, timeout, "stream step")?;
        }
        if let Some(timeout) = self.timeout.and_then(|timeout| timeout.stream_read) {
            check_zero_timeout(timeout, "stream_read")?;
        }
        Ok(())
    }

    fn check_stream_read_after(&self, read_started_at: Instant) -> Result<(), AdapterError> {
        if let Some(timeout) = self.timeout.and_then(|timeout| timeout.stream_read) {
            check_elapsed_timeout(read_started_at, timeout, "stream_read")?;
        }
        if let Some(timeout) = self.timeout.and_then(|timeout| timeout.per_step) {
            check_elapsed_timeout(self.started_at, timeout, "stream step")?;
        }
        self.check_before("stream")
    }

    fn check_abort(&self, scope: &str) -> Result<(), AdapterError> {
        let Some(signal) = self.abort_signal.as_ref() else {
            return Ok(());
        };
        if signal.aborted() {
            return Err(abort_error(scope, signal.reason()));
        }
        Ok(())
    }

    fn check_total(&self, scope: &str) -> Result<(), AdapterError> {
        if let Some(timeout) = self.timeout.and_then(|timeout| timeout.total) {
            check_elapsed_timeout(self.started_at, timeout, scope)?;
        }
        Ok(())
    }

    fn apply_to_request(&self, mut request: Request) -> Request {
        request.timeout = self.timeout;
        request.abort_signal = self.abort_signal.clone();
        request
    }
}

fn check_elapsed_timeout(
    started_at: Instant,
    timeout: f64,
    scope: &str,
) -> Result<(), AdapterError> {
    check_zero_timeout(timeout, scope)?;
    if started_at.elapsed() >= Duration::from_secs_f64(timeout) {
        return Err(timeout_error(scope, Some(timeout)));
    }
    Ok(())
}

fn check_zero_timeout(timeout: f64, scope: &str) -> Result<(), AdapterError> {
    if timeout <= 0.0 {
        return Err(timeout_error(scope, Some(timeout)));
    }
    Ok(())
}

struct ActiveToolStream<R, S>
where
    R: FnMut() -> f64,
    S: FnMut(f64),
{
    client: Client,
    current_request: Request,
    max_tool_rounds: usize,
    stop_when: Option<StopWhen>,
    repair_tool_call: Option<ToolRepair>,
    policy: RetryPolicy,
    random_multiplier: R,
    sleeper: S,
    control: OperationControl,
    current_stream: Option<StreamEvents>,
    current_accumulator: StreamAccumulator,
    steps: Vec<StepResult>,
    tool_rounds_executed: usize,
    step_attempt: u32,
    step_yielded_any: bool,
    pending_events: VecDeque<Result<StreamEvent, AdapterError>>,
    needs_stream_open: bool,
    finished: bool,
}

impl<R, S> ActiveToolStream<R, S>
where
    R: FnMut() -> f64,
    S: FnMut(f64),
{
    fn new(
        client: Client,
        prepared: PreparedGenerateRequest,
        policy: RetryPolicy,
        random_multiplier: R,
        sleeper: S,
    ) -> Result<StreamEvents, AdapterError>
    where
        R: Send + 'static,
        S: Send + 'static,
    {
        let control = OperationControl::from_request(&prepared.request);
        let PreparedGenerateRequest {
            request,
            max_tool_rounds,
            stop_when,
            repair_tool_call,
        } = prepared;
        let mut stream = Self {
            client,
            current_request: request,
            max_tool_rounds,
            stop_when,
            repair_tool_call,
            policy,
            random_multiplier,
            sleeper,
            control,
            current_stream: None,
            current_accumulator: StreamAccumulator::default(),
            steps: Vec::new(),
            tool_rounds_executed: 0,
            step_attempt: 0,
            step_yielded_any: false,
            pending_events: VecDeque::new(),
            needs_stream_open: true,
            finished: false,
        };
        stream.open_current_stream_from_attempt(0)?;
        Ok(Box::new(stream))
    }

    fn open_current_stream_from_attempt(&mut self, mut attempt: u32) -> Result<(), AdapterError> {
        loop {
            match self.open_current_stream_once() {
                Ok(events) => {
                    self.current_stream = Some(events);
                    self.current_accumulator = StreamAccumulator::default();
                    self.step_attempt = attempt;
                    self.step_yielded_any = false;
                    self.needs_stream_open = false;
                    return Ok(());
                }
                Err(error) => {
                    if !self.policy.is_retryable_error(&error) || attempt >= self.policy.max_retries
                    {
                        return Err(error);
                    }
                    let multiplier = if self.policy.jitter {
                        Some((self.random_multiplier)())
                    } else {
                        None
                    };
                    let Some(delay) =
                        self.policy
                            .calculate_delay(attempt, Some(&error), multiplier)
                    else {
                        return Err(error);
                    };
                    self.policy.notify_retry(&error, attempt, delay);
                    (self.sleeper)(delay);
                    attempt += 1;
                }
            }
        }
    }

    fn open_current_stream_once(&self) -> Result<StreamEvents, AdapterError> {
        self.control.check_before("stream")?;
        self.control.check_step_before("stream step")?;
        let opened_at = Instant::now();
        let events = self
            .client
            .stream(self.control.apply_to_request(self.current_request.clone()))?;
        self.control.check_after_step(opened_at, "stream step")?;
        Ok(events)
    }

    fn handle_stream_error(
        &mut self,
        error: AdapterError,
    ) -> Option<Result<StreamEvent, AdapterError>> {
        if self.step_yielded_any
            || !self.policy.is_retryable_error(&error)
            || self.step_attempt >= self.policy.max_retries
        {
            self.finished = true;
            let _ = self.close_current_stream();
            return Some(Err(error));
        }

        let multiplier = if self.policy.jitter {
            Some((self.random_multiplier)())
        } else {
            None
        };
        let Some(delay) = self
            .policy
            .calculate_delay(self.step_attempt, Some(&error), multiplier)
        else {
            self.finished = true;
            let _ = self.close_current_stream();
            return Some(Err(error));
        };

        self.policy.notify_retry(&error, self.step_attempt, delay);
        (self.sleeper)(delay);
        self.step_attempt += 1;
        let _ = self.close_current_stream();
        match self.open_current_stream_from_attempt(self.step_attempt) {
            Ok(()) => None,
            Err(error) => {
                self.finished = true;
                Some(Err(error))
            }
        }
    }

    fn finish_current_step(&mut self) -> Option<Result<StreamEvent, AdapterError>> {
        if let Err(error) = self.close_current_stream() {
            self.finished = true;
            return Some(Err(error));
        }

        let response = self.current_accumulator.finalize();
        let tool_results = match execute_active_tool_round(
            &self.current_request,
            &response,
            self.tool_rounds_executed,
            self.max_tool_rounds,
            self.repair_tool_call.as_ref(),
        ) {
            Ok(tool_results) => tool_results,
            Err(error) => {
                self.finished = true;
                return Some(Err(error));
            }
        };
        let next_request = next_tool_round_request(&self.current_request, &response, &tool_results);
        let should_continue = next_request.is_some();
        let step = StepResult::from_request_response_with_tool_results(
            self.current_request.clone(),
            response,
            tool_results,
        );
        self.steps.push(step);

        if !should_continue
            || self
                .stop_when
                .as_ref()
                .is_some_and(|stop_when| stop_when.should_stop(&self.steps))
        {
            self.finished = true;
            return None;
        }

        self.tool_rounds_executed += 1;
        self.current_request = self
            .control
            .apply_to_request(next_request.expect("checked above"));
        self.needs_stream_open = true;
        self.pending_events.push_back(Ok(step_finish_event(
            self.steps.last().expect("step just pushed"),
        )));
        None
    }

    fn close_current_stream(&mut self) -> Result<(), AdapterError> {
        let Some(stream) = self.current_stream.as_mut() else {
            return Ok(());
        };
        let result = stream.close();
        self.current_stream = None;
        result
    }
}

impl<R, S> Iterator for ActiveToolStream<R, S>
where
    R: FnMut() -> f64,
    S: FnMut(f64),
{
    type Item = Result<StreamEvent, AdapterError>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(event) = self.pending_events.pop_front() {
            return Some(event);
        }
        if self.finished {
            return None;
        }
        if self.needs_stream_open {
            if let Err(error) = self.open_current_stream_from_attempt(0) {
                self.finished = true;
                return Some(Err(error));
            }
        }

        loop {
            let stream = self.current_stream.as_mut()?;
            match stream.next() {
                Some(Ok(event)) => {
                    self.step_yielded_any = true;
                    self.current_accumulator.push(event.clone());
                    return Some(Ok(event));
                }
                Some(Err(error)) => {
                    if let Some(result) = self.handle_stream_error(error) {
                        return Some(result);
                    }
                    continue;
                }
                None => {
                    if let Some(result) = self.finish_current_step() {
                        return Some(result);
                    }
                    if let Some(event) = self.pending_events.pop_front() {
                        return Some(event);
                    }
                    if self.finished {
                        return None;
                    }
                    if self.needs_stream_open {
                        if let Err(error) = self.open_current_stream_from_attempt(0) {
                            self.finished = true;
                            return Some(Err(error));
                        }
                    }
                }
            }
        }
    }
}

impl<R, S> StreamEventStream for ActiveToolStream<R, S>
where
    R: FnMut() -> f64 + Send,
    S: FnMut(f64) + Send,
{
    fn close(&mut self) -> Result<(), AdapterError> {
        self.finished = true;
        self.pending_events.clear();
        self.close_current_stream()
    }
}

fn step_finish_event(step: &StepResult) -> StreamEvent {
    StreamEvent {
        r#type: StreamEventType::Custom("step_finish".to_string()),
        finish_reason: Some(step.finish_reason.clone()),
        usage: Some(step.usage.clone()),
        response: Some(step.response.clone()),
        ..StreamEvent::new(StreamEventType::Custom("step_finish".to_string()))
    }
}

pub struct StreamResult {
    events: StreamEvents,
    accumulator: StreamAccumulator,
    control: OperationControl,
    finished: bool,
    closed: bool,
    terminal_error: Option<AdapterError>,
}

impl StreamResult {
    pub fn new(events: StreamEvents) -> Self {
        Self::new_with_control(events, OperationControl::none())
    }

    fn new_with_control(events: StreamEvents, control: OperationControl) -> Self {
        Self {
            events,
            accumulator: StreamAccumulator::default(),
            control,
            finished: false,
            closed: false,
            terminal_error: None,
        }
    }

    pub fn partial_response(&self) -> Response {
        self.accumulator.response.clone()
    }

    pub fn response(&mut self) -> Result<Response, AdapterError> {
        while let Some(event) = self.next_accumulated() {
            event?;
        }
        if let Some(error) = self.terminal_error.clone() {
            return Err(error);
        }
        Ok(self.accumulator.finalize())
    }

    pub fn text_stream(&mut self) -> TextStream<'_> {
        TextStream { result: self }
    }

    pub fn close(&mut self) -> Result<(), AdapterError> {
        self.finished = true;
        self.accumulator.finalize();
        self.close_events()
    }

    pub fn accumulator(&self) -> &StreamAccumulator {
        &self.accumulator
    }

    fn next_accumulated(&mut self) -> Option<Result<StreamEvent, AdapterError>> {
        if self.finished {
            return None;
        }

        if let Err(error) = self.control.check_stream_read_before() {
            return self.finish_with_terminal_error(error);
        }

        let read_started_at = Instant::now();
        match self.events.next() {
            Some(Ok(event)) => {
                if let Err(error) = self.control.check_stream_read_after(read_started_at) {
                    return self.finish_with_terminal_error(error);
                }
                self.accumulator.push(event.clone());
                Some(Ok(event))
            }
            Some(Err(error)) => self.finish_with_terminal_error(error),
            None => {
                self.finished = true;
                self.accumulator.finalize();
                match self.close_events() {
                    Ok(()) => None,
                    Err(error) => {
                        self.terminal_error = Some(error.clone());
                        Some(Err(error))
                    }
                }
            }
        }
    }

    fn finish_with_terminal_error(
        &mut self,
        error: AdapterError,
    ) -> Option<Result<StreamEvent, AdapterError>> {
        self.finished = true;
        self.terminal_error = Some(error.clone());
        let _ = self.close_events();
        Some(Err(error))
    }

    fn close_events(&mut self) -> Result<(), AdapterError> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        self.events.close()
    }
}

impl Drop for StreamResult {
    fn drop(&mut self) {
        let _ = self.close_events();
    }
}

impl Iterator for StreamResult {
    type Item = Result<StreamEvent, AdapterError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_accumulated()
    }
}

impl futures_core::Stream for StreamResult {
    type Item = Result<StreamEvent, AdapterError>;

    fn poll_next(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.as_mut().get_mut().next_accumulated())
    }
}

pub struct TextStream<'a> {
    result: &'a mut StreamResult,
}

impl Iterator for TextStream<'_> {
    type Item = Result<String, AdapterError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let before = self.result.partial_response().text();
            let event = self.result.next_accumulated()?;
            let event = match event {
                Ok(event) => event,
                Err(error) => return Some(Err(error)),
            };
            let after = self.result.partial_response().text();
            if after.len() > before.len() && after.starts_with(&before) {
                return Some(Ok(after[before.len()..].to_string()));
            }
            if matches!(
                event.r#type,
                StreamEventType::TextStart | StreamEventType::TextDelta
            ) {
                if let Some(delta) = event.delta {
                    if !delta.is_empty() {
                        return Some(Ok(delta));
                    }
                }
            }
        }
    }
}

impl StepResult {
    fn from_request_response(request: Request, response: Response) -> Self {
        Self {
            text: response.text(),
            reasoning: response.reasoning(),
            tool_calls: response.tool_calls(),
            tool_results: tool_results_from_response(&response),
            finish_reason: response.finish_reason.clone(),
            usage: response.usage.clone(),
            warnings: response.warnings.clone(),
            request,
            response,
        }
    }

    fn from_request_response_with_tool_results(
        request: Request,
        response: Response,
        tool_results: Vec<ToolResult>,
    ) -> Self {
        Self {
            text: response.text(),
            reasoning: response.reasoning(),
            tool_calls: response.tool_calls(),
            tool_results,
            finish_reason: response.finish_reason.clone(),
            usage: response.usage.clone(),
            warnings: response.warnings.clone(),
            request,
            response,
        }
    }
}

const DEFAULT_MAX_TOOL_ROUNDS: usize = 1;

fn default_max_tool_rounds() -> usize {
    DEFAULT_MAX_TOOL_ROUNDS
}

fn execute_active_tool_round(
    request: &Request,
    response: &Response,
    tool_rounds_executed: usize,
    max_tool_rounds: usize,
    repair_tool_call: Option<&ToolRepair>,
) -> Result<Vec<ToolResult>, AdapterError> {
    let tool_calls = response.tool_calls();
    if !should_execute_active_tools(
        request,
        response,
        &tool_calls,
        tool_rounds_executed,
        max_tool_rounds,
    ) {
        return Ok(Vec::new());
    }

    execute_active_tool_calls(request, tool_calls, repair_tool_call)
}

fn should_execute_active_tools(
    request: &Request,
    response: &Response,
    tool_calls: &[ToolCall],
    tool_rounds_executed: usize,
    max_tool_rounds: usize,
) -> bool {
    if tool_calls.is_empty()
        || response.finish_reason.reason != FinishReasonKind::ToolCalls
        || tool_rounds_executed >= max_tool_rounds
        || has_passive_tool_call(request, tool_calls)
        || has_sdk_structured_output_tool_call(request, tool_calls)
    {
        return false;
    }

    true
}

fn has_sdk_structured_output_tool_call(request: &Request, tool_calls: &[ToolCall]) -> bool {
    request.response_format.is_some()
        && !request
            .tools
            .iter()
            .any(|tool| tool.name == STRUCTURED_OUTPUT_TOOL_NAME)
        && tool_calls
            .iter()
            .any(|tool_call| tool_call.name == STRUCTURED_OUTPUT_TOOL_NAME)
}

fn has_passive_tool_call(request: &Request, tool_calls: &[ToolCall]) -> bool {
    tool_calls.iter().any(|tool_call| {
        request
            .tools
            .iter()
            .find(|tool| tool.name == tool_call.name)
            .is_some_and(Tool::is_passive)
    })
}

fn execute_active_tool_calls(
    request: &Request,
    tool_calls: Vec<ToolCall>,
    repair_tool_call: Option<&ToolRepair>,
) -> Result<Vec<ToolResult>, AdapterError> {
    thread::scope(|scope| {
        let handles = tool_calls
            .into_iter()
            .map(|tool_call| {
                let tool_call_id = tool_call.id.clone();
                (
                    tool_call_id,
                    scope.spawn(move || {
                        execute_active_tool_call(request, tool_call, repair_tool_call)
                    }),
                )
            })
            .collect::<Vec<_>>();
        let mut results = Vec::with_capacity(handles.len());
        let mut control_flow_error = None;

        for (tool_call_id, handle) in handles {
            match handle.join() {
                Ok(Ok(result)) => {
                    results.push(result);
                }
                Ok(Err(error)) => {
                    control_flow_error.get_or_insert(error);
                }
                Err(_) => {
                    results.push(ToolResult::error(
                        tool_call_id,
                        Value::String("tool handler panicked".to_string()),
                    ));
                }
            }
        }

        if let Some(error) = control_flow_error {
            Err(error)
        } else {
            Ok(results)
        }
    })
}

fn execute_active_tool_call(
    request: &Request,
    tool_call: ToolCall,
    repair_tool_call: Option<&ToolRepair>,
) -> Result<ToolResult, AdapterError> {
    let Some(tool) = request
        .tools
        .iter()
        .find(|tool| tool.name == tool_call.name)
    else {
        return Ok(ToolResult::error(
            tool_call.id,
            Value::String(format!("Unknown tool '{}'", tool_call.name)),
        ));
    };
    if tool.is_passive() {
        return Ok(ToolResult::error(
            tool_call.id,
            Value::String(format!("Tool '{}' has no execute handler", tool.name)),
        ));
    }
    let tool_call_id = tool_call.id.clone();
    let tool_call =
        match prepare_tool_call_for_execution(tool, tool_call, request, repair_tool_call) {
            Ok(tool_call) => tool_call,
            Err(error) if is_control_flow_error(&error) => return Err(error),
            Err(error) => return Ok(ToolResult::error(tool_call_id, error.message)),
        };
    let invocation = ToolInvocation::new(
        tool_call.clone(),
        request.messages.clone(),
        request.abort_signal.clone(),
    );
    match tool.execute(invocation) {
        Ok(result) => Ok(result),
        Err(error) if is_control_flow_error(&error) => Err(error),
        Err(error) => Ok(ToolResult::error(tool_call.id, error.message)),
    }
}

fn prepare_tool_call_for_execution(
    tool: &Tool,
    mut tool_call: ToolCall,
    request: &Request,
    repair_tool_call: Option<&ToolRepair>,
) -> Result<ToolCall, AdapterError> {
    let arguments = match parse_and_validate_tool_arguments(tool, &tool_call) {
        Ok(arguments) => arguments,
        Err(error) => repair_tool_arguments(tool, &tool_call, request, repair_tool_call, error)?,
    };
    tool_call.arguments = arguments;
    Ok(tool_call)
}

fn parse_and_validate_tool_arguments(
    tool: &Tool,
    tool_call: &ToolCall,
) -> Result<Value, AdapterError> {
    let arguments = parse_tool_call_arguments(tool_call)?;
    validate_tool_call_arguments(tool, &arguments)?;
    Ok(arguments)
}

fn parse_tool_call_arguments(tool_call: &ToolCall) -> Result<Value, AdapterError> {
    if let Some(raw_arguments) = tool_call.raw_arguments.as_deref() {
        let raw_arguments = raw_arguments.trim();
        if raw_arguments.is_empty() {
            return Ok(Value::Object(serde_json::Map::new()));
        }
        return serde_json::from_str(raw_arguments).map_err(|error| {
            invalid_tool_call_error(format!(
                "Invalid JSON arguments for tool '{}': {error}",
                tool_call.name
            ))
        });
    }

    if let Value::String(raw_arguments) = &tool_call.arguments {
        let raw_arguments = raw_arguments.trim();
        if raw_arguments.is_empty() {
            return Ok(Value::Object(serde_json::Map::new()));
        }
        return serde_json::from_str(raw_arguments).map_err(|error| {
            invalid_tool_call_error(format!(
                "Invalid JSON arguments for tool '{}': {error}",
                tool_call.name
            ))
        });
    }

    Ok(tool_call.arguments.clone())
}

fn validate_tool_call_arguments(tool: &Tool, arguments: &Value) -> Result<(), AdapterError> {
    let Some(parameters) = tool.parameters.as_ref() else {
        return Ok(());
    };
    validate_json_value(arguments, parameters, "$").map_err(|message| {
        invalid_tool_call_error(format!(
            "Invalid arguments for tool '{}': {message}",
            tool.name
        ))
    })
}

fn repair_tool_arguments(
    tool: &Tool,
    tool_call: &ToolCall,
    request: &Request,
    repair_tool_call: Option<&ToolRepair>,
    original_error: AdapterError,
) -> Result<Value, AdapterError> {
    let Some(repair_tool_call) = repair_tool_call else {
        return Err(original_error);
    };
    let invocation = ToolRepairInvocation::new(
        tool_call.clone(),
        tool.clone(),
        original_error.message.clone(),
        request.messages.clone(),
        request.abort_signal.clone(),
    );
    let repaired_arguments = repair_tool_call.repair(invocation)?;
    validate_tool_call_arguments(tool, &repaired_arguments).map_err(|error| {
        invalid_tool_call_error(format!(
            "Invalid repaired arguments for tool '{}': {}",
            tool.name, error.message
        ))
    })?;
    Ok(repaired_arguments)
}

fn invalid_tool_call_error(message: impl Into<String>) -> AdapterError {
    AdapterError::new(AdapterErrorKind::InvalidToolCall, message)
}

fn next_tool_round_request(
    request: &Request,
    response: &Response,
    tool_results: &[ToolResult],
) -> Option<Request> {
    if tool_results.is_empty() {
        return None;
    }

    let mut next = request.clone();
    let tool_calls = response.tool_calls();
    let mut messages = request.messages.clone();
    messages.push(assistant_message_for_tool_round(response, &tool_calls));
    messages.extend(tool_results.iter().map(tool_result_message));
    next.messages = messages;
    Some(next)
}

fn assistant_message_for_tool_round(response: &Response, tool_calls: &[ToolCall]) -> Message {
    let mut message = response.message.clone();
    let missing_tool_calls = tool_calls
        .iter()
        .filter(|tool_call| {
            !message.content.iter().any(|part| {
                matches!(
                    part,
                    ContentPart::ToolCall { tool_call: existing } if existing.id == tool_call.id
                )
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    message.content.extend(
        missing_tool_calls
            .into_iter()
            .map(|tool_call| ContentPart::ToolCall { tool_call }),
    );
    message
}

fn tool_result_message(result: &ToolResult) -> Message {
    let tool_call_id = result.tool_call_id.clone();
    Message {
        role: MessageRole::Tool,
        content: vec![ContentPart::ToolResult {
            tool_result: ToolResultData {
                tool_call_id: tool_call_id.clone(),
                content: result.content.clone(),
                is_error: result.is_error,
                image_data: result.image_data.clone(),
                image_media_type: result.image_media_type.clone(),
            },
        }],
        name: None,
        tool_call_id: Some(tool_call_id),
        provider_metadata: BTreeMap::new(),
    }
}

fn is_control_flow_error(error: &AdapterError) -> bool {
    matches!(
        error.kind,
        AdapterErrorKind::Abort | AdapterErrorKind::RequestTimeout
    )
}

fn finish_generation(steps: Vec<StepResult>) -> Result<GenerateResult, AdapterError> {
    let response = steps
        .last()
        .map(|step| step.response.clone())
        .expect("generation records at least one step before finishing");
    let tool_results = steps
        .last()
        .map(|step| step.tool_results.clone())
        .unwrap_or_default();
    let total_usage = steps
        .iter()
        .map(|step| step.usage.clone())
        .fold(Usage::default(), |total, usage| total + usage);
    let warnings = steps
        .iter()
        .flat_map(|step| step.warnings.clone())
        .collect::<Vec<_>>();
    Ok(GenerateResult {
        text: response.text(),
        reasoning: response.reasoning(),
        tool_calls: response.tool_calls(),
        tool_results,
        finish_reason: response.finish_reason.clone(),
        usage: response.usage.clone(),
        total_usage,
        response,
        warnings,
        steps,
        output: None,
    })
}

fn build_generate_request(
    client: &Client,
    input: GenerateRequest,
) -> Result<PreparedGenerateRequest, AdapterError> {
    if let Some(timeout) = input.timeout {
        timeout.validate().map_err(invalid_request_error)?;
    }
    if let Some(signal) = input.abort_signal.as_ref() {
        if signal.aborted() {
            return Err(abort_error("generation", signal.reason()));
        }
    }
    let messages = normalize_messages(input.prompt, input.messages, input.system)?;
    let required_capabilities = input
        .required_capabilities
        .union(capabilities_from_request_parts(
            &messages,
            &input.tools,
            input.response_format.as_ref(),
            input.reasoning_effort.as_deref(),
        ));
    let selected_profile = selected_profile_for_high_level_request(
        client,
        input.provider.as_deref(),
        input.active_profile.is_some(),
    );
    let active_profile = selected_profile
        .as_ref()
        .map(LlmProfileRoute::active_profile)
        .or(input.active_profile);
    let provider = selected_profile
        .as_ref()
        .map(|profile| profile.provider.clone())
        .or(input.provider);
    let client_default_provider = if selected_profile.is_some() {
        None
    } else {
        client.default_provider().map(str::to_string)
    };
    let resolved = resolve_high_level_provider_and_model(&HighLevelLlmResolutionInputs {
        provider,
        model: input.model,
        active_profile,
        client_default_provider,
        required_capabilities,
    })?;
    let has_tools = !input.tools.is_empty();
    let request_provider = selected_profile
        .as_ref()
        .map(|profile| profile.id.clone())
        .unwrap_or_else(|| resolved.provider.clone());
    let mut metadata = input.metadata;
    if let Some(profile) = selected_profile.as_ref() {
        metadata.insert(
            "spark.runtime.provider".to_string(),
            Value::String(resolved.provider.clone()),
        );
        metadata.insert(
            "spark.runtime.model".to_string(),
            Value::String(resolved.model.clone()),
        );
        metadata.insert(
            "spark.runtime.llm_profile".to_string(),
            Value::String(profile.id.clone()),
        );
    }

    Ok(PreparedGenerateRequest {
        request: Request {
            model: resolved.model,
            messages,
            provider: Some(request_provider),
            tools: input.tools,
            tool_choice: default_tool_choice(input.tool_choice, has_tools),
            response_format: input.response_format,
            temperature: input.temperature,
            top_p: input.top_p,
            max_tokens: input.max_tokens,
            stop_sequences: input.stop_sequences,
            reasoning_effort: input.reasoning_effort,
            metadata,
            provider_options: input.provider_options,
            timeout: input.timeout,
            abort_signal: input.abort_signal,
        },
        max_tool_rounds: input.max_tool_rounds,
        stop_when: input.stop_when,
        repair_tool_call: input.repair_tool_call,
    })
}

fn selected_profile_for_high_level_request(
    client: &Client,
    provider: Option<&str>,
    has_active_profile: bool,
) -> Option<LlmProfileRoute> {
    if let Some(provider) = provider.and_then(non_empty_text) {
        return client.llm_profile(provider);
    }
    if has_active_profile {
        return None;
    }
    client
        .default_provider()
        .and_then(|provider| client.llm_profile(provider))
}

fn default_tool_choice(tool_choice: Option<ToolChoice>, has_tools: bool) -> Option<ToolChoice> {
    tool_choice.or_else(|| has_tools.then(ToolChoice::auto))
}

fn default_stream_sleep(delay: f64) {
    if delay <= 0.0 || !delay.is_finite() {
        return;
    }
    thread::sleep(Duration::from_secs_f64(delay));
}

fn default_stream_random_multiplier() -> f64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos())
        .unwrap_or(0);
    0.5 + (f64::from(nanos) / 999_999_999.0)
}

fn normalize_messages(
    prompt: Option<String>,
    messages: Option<Vec<Message>>,
    system: Option<String>,
) -> Result<Vec<Message>, AdapterError> {
    if prompt.is_some() && messages.is_some() {
        return Err(invalid_request_error(
            "generate and stream accept either prompt or messages, not both",
        ));
    }
    if prompt.is_none() && messages.is_none() {
        return Err(invalid_request_error(
            "either prompt or messages must be provided",
        ));
    }

    let mut normalized = Vec::new();
    if let Some(system) = non_empty_optional(system) {
        normalized.push(Message::system(system));
    }

    match (prompt, messages) {
        (Some(prompt), None) => normalized.push(Message::user(prompt)),
        (None, Some(messages)) => normalized.extend(messages),
        (None, None) => unreachable!("prompt/messages absence checked above"),
        (Some(_), Some(_)) => unreachable!("prompt/messages conflict checked above"),
    }
    Ok(normalized)
}

fn capabilities_from_request_parts(
    messages: &[Message],
    tools: &[Tool],
    response_format: Option<&ResponseFormat>,
    reasoning_effort: Option<&str>,
) -> ModelCapabilities {
    let mut capabilities = ModelCapabilities::default();
    if !tools.is_empty() {
        capabilities = capabilities.union(ModelCapabilities::tools());
    }
    if response_format.is_some() {
        capabilities = capabilities.union(ModelCapabilities::structured_output());
    }
    if messages.iter().any(message_has_vision_content) {
        capabilities = capabilities.union(ModelCapabilities::vision());
    }
    if reasoning_effort
        .map(str::trim)
        .is_some_and(|reasoning_effort| !reasoning_effort.is_empty())
    {
        capabilities = capabilities.union(ModelCapabilities::reasoning());
    }
    capabilities
}

fn message_has_vision_content(message: &Message) -> bool {
    message
        .content
        .iter()
        .any(|part| matches!(part, ContentPart::Image { .. }))
}

fn tool_results_from_response(response: &Response) -> Vec<ToolResult> {
    if response.message.role != MessageRole::Tool {
        return Vec::new();
    }

    response
        .message
        .content
        .iter()
        .filter_map(|part| match part {
            ContentPart::ToolResult { tool_result } => Some(tool_result.clone().into()),
            _ => None,
        })
        .collect()
}

fn non_empty_owned(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn non_empty_text(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn non_empty_optional(value: Option<String>) -> Option<String> {
    value.and_then(non_empty_owned)
}

fn invalid_request_error(message: impl Into<String>) -> AdapterError {
    AdapterError::new(AdapterErrorKind::InvalidRequest, message)
}
