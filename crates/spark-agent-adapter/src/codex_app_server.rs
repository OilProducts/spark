use std::collections::{BTreeMap, VecDeque};
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Map, Value};
use spark_common::debug::{codex_jsonrpc_trace_enabled, CODEX_JSONRPC_TRACE_PATH_METADATA_KEY};
use spark_common::events::{
    TurnStreamChannel, TurnStreamEvent, TurnStreamEventKind, TurnStreamSource,
};
use time::OffsetDateTime;
use unified_llm_adapter::Usage;

use crate::agent::{
    AgentRawLogLine, AgentRequestUserInputAnswerRequest, AgentThreadResumeFailure,
    AgentTurnEventSink, AgentTurnOutput, AgentTurnRequest,
};
use crate::session::SessionSteeringHandle;

pub const CODEX_APP_SERVER_BACKEND: &str = "codex_app_server";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const TURN_IDLE_TIMEOUT: Duration = Duration::from_secs(300);
const CODEX_RUNTIME_ROOT_ENV: &str = "ATTRACTOR_CODEX_RUNTIME_ROOT";
const CODEX_SEED_DIR_ENV: &str = "ATTRACTOR_CODEX_SEED_DIR";
const COLLABORATION_MODE_DEFAULT: &str = "default";
const COLLABORATION_MODE_PLAN: &str = "plan";

#[derive(Debug, Clone, PartialEq)]
pub struct CodexAppServerError {
    pub message: String,
    pub retryable: bool,
    pub details: Option<Value>,
}

impl CodexAppServerError {
    fn configuration(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
            details: None,
        }
    }

    fn runtime(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
            details: None,
        }
    }

    fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

impl std::fmt::Display for CodexAppServerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CodexAppServerError {}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CodexAppServerTurnState {
    pub agent_chunks: Vec<String>,
    pub plan_chunks: Vec<String>,
    pub command_chunks: Vec<String>,
    pub final_agent_message: Option<String>,
    pub final_plan_message: Option<String>,
    pub last_token_total: Option<u64>,
    pub last_token_usage_payload: Option<Value>,
    pub turn_status: Option<String>,
    pub turn_error: Option<String>,
    pub last_error: Option<String>,
    reasoning_summary_buffer: String,
    agent_message_phases: BTreeMap<String, String>,
}

impl CodexAppServerTurnState {
    pub fn resolved_agent_text(&self) -> String {
        self.final_agent_message
            .clone()
            .unwrap_or_else(|| self.agent_chunks.join(""))
            .trim()
            .to_string()
    }

    pub fn resolved_plan_text(&self) -> String {
        self.final_plan_message
            .clone()
            .unwrap_or_else(|| self.plan_chunks.join(""))
            .trim()
            .to_string()
    }

    pub fn resolved_command_text(&self) -> String {
        self.command_chunks.join("").trim().to_string()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodexAppServerTurnResult {
    pub thread_id: String,
    pub turn_id: String,
    pub state: CodexAppServerTurnState,
    pub events: Vec<TurnStreamEvent>,
    pub raw_log_lines: Vec<AgentRawLogLine>,
}

#[derive(Debug, Default)]
pub struct CodexAppServerBackend;

impl CodexAppServerBackend {
    pub fn new() -> Self {
        Self
    }

    pub fn run_agent_turn(
        &self,
        request: AgentTurnRequest,
    ) -> Result<AgentTurnOutput, CodexAppServerError> {
        self.run_agent_turn_with_steering_and_sink(request, None, None)
    }

    pub fn run_agent_turn_with_event_sink(
        &self,
        request: AgentTurnRequest,
        event_sink: Option<AgentTurnEventSink>,
    ) -> Result<AgentTurnOutput, CodexAppServerError> {
        self.run_agent_turn_with_steering_and_sink(request, None, event_sink)
    }

    pub fn run_agent_turn_with_steering(
        &self,
        request: AgentTurnRequest,
        steering: Option<SessionSteeringHandle>,
    ) -> Result<AgentTurnOutput, CodexAppServerError> {
        self.run_agent_turn_with_steering_and_sink(request, steering, None)
    }

    fn run_agent_turn_with_steering_and_sink(
        &self,
        request: AgentTurnRequest,
        steering: Option<SessionSteeringHandle>,
        event_sink: Option<AgentTurnEventSink>,
    ) -> Result<AgentTurnOutput, CodexAppServerError> {
        let trace_path = codex_jsonrpc_trace_enabled()
            .then(|| {
                metadata_string(&request.metadata, &[CODEX_JSONRPC_TRACE_PATH_METADATA_KEY])
                    .map(PathBuf::from)
            })
            .flatten();
        let mut client = CodexAppServerClient::connect_with_trace_path(
            PathBuf::from(&request.project_path),
            trace_path,
        )?;
        let model = request
            .model
            .as_deref()
            .and_then(non_empty)
            .map(str::to_string);
        let thread_id = if let Some(thread_id) = metadata_string(
            &request.metadata,
            &[
                "spark.runtime.codex_app_server.thread_id",
                "codex_app_server.thread_id",
                "codex_thread_id",
            ],
        ) {
            match client.resume_thread(&thread_id, model.as_deref(), Some(&request.project_path)) {
                Ok(resumed) => resumed,
                Err(error) => {
                    return Ok(thread_resume_failure_output(
                        "thread/resume",
                        AgentThreadResumeFailure {
                            message: error.message,
                            error_code: Some("codex_app_server_resume_failed".to_string()),
                            details: error.details,
                        },
                    ));
                }
            }
        } else {
            client.start_thread(
                model.as_deref(),
                Some(&request.project_path),
                codex_app_server_thread_is_ephemeral(&request),
            )?
        };
        let result = client.run_turn(
            &thread_id,
            &request.prompt,
            model.as_deref(),
            request.reasoning_effort.as_deref(),
            request.chat_mode.as_deref(),
            Some(&request.project_path),
            steering,
            event_sink,
        )?;
        Ok(agent_output_from_app_server_result(result))
    }

    pub fn answer_request_user_input(
        &self,
        request: AgentRequestUserInputAnswerRequest,
    ) -> Result<AgentTurnOutput, CodexAppServerError> {
        let thread_id = metadata_string(
            &request.metadata,
            &[
                "spark.runtime.codex_app_server.thread_id",
                "codex_app_server.thread_id",
                "codex_thread_id",
            ],
        )
        .ok_or_else(|| {
            CodexAppServerError::configuration(
                "codex app-server request-user-input answer requires a persisted thread id",
            )
        })?;
        let turn_id = metadata_string(
            &request.metadata,
            &[
                "spark.runtime.codex_app_server.turn_id",
                "codex_app_server.turn_id",
                "codex_turn_id",
            ],
        )
        .or_else(|| non_empty(&request.assistant_turn_id).map(str::to_string))
        .ok_or_else(|| {
            CodexAppServerError::configuration(
                "codex app-server request-user-input answer requires a persisted turn id",
            )
        })?;
        let answer_content = request_user_input_answer_content(
            &request.answers,
            request.request_user_input.as_ref(),
        );
        if answer_content.trim().is_empty() {
            return Ok(thread_resume_failure_output(
                request.request_id.as_str(),
                AgentThreadResumeFailure {
                    message:
                        "request-user-input answer could not resume because no answers were supplied."
                            .to_string(),
                    error_code: Some("request_user_input_missing_answers".to_string()),
                    details: Some(json!({ "request_id": request.request_id })),
                },
            ));
        }
        let trace_path = codex_jsonrpc_trace_enabled()
            .then(|| {
                metadata_string(&request.metadata, &[CODEX_JSONRPC_TRACE_PATH_METADATA_KEY])
                    .map(PathBuf::from)
            })
            .flatten();
        let mut client = CodexAppServerClient::connect_with_trace_path(
            PathBuf::from(&request.project_path),
            trace_path,
        )?;
        client.steer_turn(&thread_id, &turn_id, &answer_content)?;
        Ok(AgentTurnOutput {
            events: vec![TurnStreamEvent {
                kind: TurnStreamEventKind::Other("steering_injected".to_string()),
                channel: None,
                source: TurnStreamSource {
                    backend: Some(CODEX_APP_SERVER_BACKEND.to_string()),
                    app_thread_id: Some(thread_id),
                    app_turn_id: Some(turn_id),
                    item_id: non_empty(&request.request_id).map(str::to_string),
                    raw_kind: Some("turn_steer".to_string()),
                    ..TurnStreamSource::default()
                },
                content_delta: Some(answer_content.clone()),
                message: Some(answer_content),
                tool_call: None,
                request_user_input: None,
                token_usage: None,
                error: None,
                error_code: None,
                details: None,
                phase: Some("request_user_input_answer".to_string()),
                status: Some("delivered".to_string()),
            }],
            ..AgentTurnOutput::default()
        })
    }
}

fn codex_app_server_thread_is_ephemeral(request: &AgentTurnRequest) -> bool {
    request
        .chat_mode
        .as_deref()
        .is_some_and(|mode| mode.trim().eq_ignore_ascii_case("agent"))
}

fn agent_output_from_app_server_result(result: CodexAppServerTurnResult) -> AgentTurnOutput {
    let token_usage = result.state.last_token_usage_payload.clone();
    AgentTurnOutput {
        app_thread_id: Some(result.thread_id),
        app_turn_id: Some(result.turn_id),
        final_assistant_text: non_empty(&result.state.resolved_agent_text()).map(str::to_string),
        token_usage: token_usage.clone(),
        token_usage_breakdown: token_usage,
        raw_log_lines: result.raw_log_lines,
        events: result.events,
        thread_resume_failure: None,
    }
}

fn thread_resume_failure_output(
    item_id: &str,
    failure: AgentThreadResumeFailure,
) -> AgentTurnOutput {
    AgentTurnOutput {
        events: vec![TurnStreamEvent {
            kind: TurnStreamEventKind::Error,
            channel: None,
            source: TurnStreamSource {
                backend: Some(CODEX_APP_SERVER_BACKEND.to_string()),
                item_id: non_empty(item_id).map(str::to_string),
                raw_kind: Some("thread_resume_failure".to_string()),
                ..TurnStreamSource::default()
            },
            content_delta: None,
            message: Some(failure.message.clone()),
            tool_call: None,
            request_user_input: None,
            token_usage: None,
            error: Some(failure.message.clone()),
            error_code: failure.error_code.clone(),
            details: failure.details.clone(),
            phase: Some("thread_resume".to_string()),
            status: Some("failed".to_string()),
        }],
        thread_resume_failure: Some(failure),
        ..AgentTurnOutput::default()
    }
}

pub struct CodexAppServerClient {
    child: Child,
    stdin: ChildStdin,
    stdout: Receiver<String>,
    next_request_id: u64,
    pending_messages: VecDeque<Value>,
    pending_responses: BTreeMap<String, VecDeque<Value>>,
    trace_sink: Option<CodexJsonrpcTraceSink>,
}

impl CodexAppServerClient {
    pub fn connect(working_dir: PathBuf) -> Result<Self, CodexAppServerError> {
        Self::connect_with_trace_path(working_dir, None)
    }

    pub fn connect_with_trace_path(
        working_dir: PathBuf,
        trace_path: Option<PathBuf>,
    ) -> Result<Self, CodexAppServerError> {
        if !working_dir.exists() {
            return Err(CodexAppServerError::configuration(format!(
                "codex app-server working directory is unavailable in the runtime: {}",
                working_dir.display()
            )));
        }
        let mut command = Command::new(codex_executable());
        command
            .arg("app-server")
            .current_dir(&working_dir)
            .envs(build_codex_runtime_environment()?)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                CodexAppServerError::configuration("codex app-server not found on PATH")
            } else {
                CodexAppServerError::runtime(format!("codex app-server launch failed: {error}"))
            }
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| CodexAppServerError::runtime("codex app-server did not expose stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| {
            CodexAppServerError::runtime("codex app-server did not expose stdout")
        })?;
        let (stdout_sender, stdout_receiver) = mpsc::channel();
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                if stdout_sender.send(line).is_err() {
                    break;
                }
            }
        });
        let mut client = Self {
            child,
            stdin,
            stdout: stdout_receiver,
            next_request_id: 0,
            pending_messages: VecDeque::new(),
            pending_responses: BTreeMap::new(),
            trace_sink: trace_path.map(CodexJsonrpcTraceSink::new),
        };
        let response = client.send_request(
            "initialize",
            Some(json!({
                "clientInfo": {"name": "spark", "version": "0.1"},
                "capabilities": {"experimentalApi": true},
            })),
        )?;
        if response.get("error").is_some() {
            return Err(rpc_error("codex app-server initialize failed", &response));
        }
        client.send_json(&json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}))?;
        Ok(client)
    }

    pub fn start_thread(
        &mut self,
        model: Option<&str>,
        cwd: Option<&str>,
        ephemeral: bool,
    ) -> Result<String, CodexAppServerError> {
        let mut params = json!({
            "cwd": cwd,
            "sandbox": "danger-full-access",
            "approvalPolicy": "never",
            "ephemeral": ephemeral,
        });
        if let Some(model) = model.and_then(non_empty) {
            params["model"] = json!(model);
        }
        let response = self.send_request("thread/start", Some(params))?;
        if response.get("error").is_some() {
            return Err(rpc_error("codex app-server thread/start failed", &response));
        }
        response
            .pointer("/result/thread/id")
            .and_then(Value::as_str)
            .and_then(non_empty)
            .map(str::to_string)
            .ok_or_else(|| {
                CodexAppServerError::runtime("codex app-server did not return a thread id")
            })
    }

    pub fn resume_thread(
        &mut self,
        thread_id: &str,
        model: Option<&str>,
        cwd: Option<&str>,
    ) -> Result<String, CodexAppServerError> {
        let mut params = json!({
            "threadId": thread_id,
            "cwd": cwd,
            "sandbox": "danger-full-access",
            "approvalPolicy": "never",
        });
        if let Some(model) = model.and_then(non_empty) {
            params["model"] = json!(model);
        }
        let response = self.send_request("thread/resume", Some(params))?;
        if response.get("error").is_some() {
            return Err(rpc_error(
                "codex app-server thread/resume failed",
                &response,
            ));
        }
        response
            .pointer("/result/thread/id")
            .and_then(Value::as_str)
            .and_then(non_empty)
            .map(str::to_string)
            .ok_or_else(|| {
                CodexAppServerError::runtime(
                    "codex app-server did not return a thread id for thread/resume",
                )
            })
    }

    pub fn list_models(&mut self) -> Result<Value, CodexAppServerError> {
        let response = self.send_request("model/list", Some(json!({ "limit": 100 })))?;
        if response.get("error").is_some() {
            return Err(rpc_error("codex app-server model/list failed", &response));
        }
        Ok(response.get("result").cloned().unwrap_or_else(|| json!({})))
    }

    fn default_model(&mut self) -> Result<Option<String>, CodexAppServerError> {
        let models = self.list_models()?;
        let Some(entries) = models.get("data").and_then(Value::as_array) else {
            return Ok(None);
        };
        let fallback = entries
            .iter()
            .find_map(|entry| model_id_from_model_list_entry(entry));
        let default = entries
            .iter()
            .find(|entry| {
                entry
                    .get("isDefault")
                    .or_else(|| entry.get("is_default"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
            .and_then(model_id_from_model_list_entry);
        Ok(default.or(fallback))
    }

    pub fn run_turn(
        &mut self,
        thread_id: &str,
        prompt: &str,
        model: Option<&str>,
        reasoning_effort: Option<&str>,
        chat_mode: Option<&str>,
        cwd: Option<&str>,
        steering: Option<SessionSteeringHandle>,
        event_sink: Option<AgentTurnEventSink>,
    ) -> Result<CodexAppServerTurnResult, CodexAppServerError> {
        let mut params = json!({
            "threadId": thread_id,
            "input": [{"type": "text", "text": prompt}],
            "approvalPolicy": "never",
            "sandboxPolicy": {"type": "dangerFullAccess"},
            "cwd": cwd,
        });
        let effective_model = model.and_then(non_empty).map(str::to_string);
        if chat_mode.is_some() {
            let effective_model = match effective_model {
                Some(model) => model,
                None => self.default_model()?.ok_or_else(|| {
                    CodexAppServerError::runtime(
                        "codex app-server turn/start requires a resolved model",
                    )
                })?,
            };
            params["model"] = json!(&effective_model);
            params["collaborationMode"] =
                collaboration_mode_for_chat_mode(chat_mode, &effective_model);
        } else if let Some(model) = effective_model {
            params["model"] = json!(model);
        }
        if let Some(effort) = normalize_reasoning_effort(reasoning_effort)? {
            params["effort"] = json!(effort);
        }
        let response = self.send_request("turn/start", Some(params))?;
        if response.get("error").is_some() {
            return Err(rpc_error("codex app-server turn/start failed", &response));
        }
        let turn_id = response
            .pointer("/result/turn/id")
            .and_then(Value::as_str)
            .and_then(non_empty)
            .map(str::to_string)
            .ok_or_else(|| {
                CodexAppServerError::runtime("codex app-server did not return a turn id")
            })?;
        self.consume_turn_stream(thread_id, &turn_id, steering, event_sink)
    }

    pub fn steer_turn(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        message: &str,
    ) -> Result<(), CodexAppServerError> {
        let response = self.send_request(
            "turn/steer",
            Some(json!({
                "threadId": thread_id,
                "expectedTurnId": turn_id,
                "input": [{"type": "text", "text": message}],
            })),
        )?;
        if response.get("error").is_some() {
            return Err(rpc_error("codex app-server turn/steer failed", &response));
        }
        Ok(())
    }

    pub fn send_request(
        &mut self,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, CodexAppServerError> {
        self.next_request_id = self.next_request_id.saturating_add(1);
        let request_id = self.next_request_id;
        let mut payload = json!({"jsonrpc": "2.0", "id": request_id, "method": method});
        if let Some(params) = params {
            payload["params"] = params;
        }
        self.send_json(&payload)?;
        self.wait_for_response(request_id, REQUEST_TIMEOUT)
    }

    fn consume_turn_stream(
        &mut self,
        thread_id: &str,
        expected_turn_id: &str,
        steering: Option<SessionSteeringHandle>,
        event_sink: Option<AgentTurnEventSink>,
    ) -> Result<CodexAppServerTurnResult, CodexAppServerError> {
        let mut state = CodexAppServerTurnState::default();
        let mut events = Vec::new();
        let mut last_activity = Instant::now();
        loop {
            if let Some(handle) = steering.as_ref() {
                self.drain_steering(handle, thread_id, expected_turn_id, &mut events)?;
            }
            if last_activity.elapsed() >= TURN_IDLE_TIMEOUT {
                return Err(CodexAppServerError::runtime(
                    "codex app-server turn timed out waiting for activity",
                ));
            }
            let Some(message) = self.next_message(Duration::from_millis(100))? else {
                if self.child.try_wait().ok().flatten().is_some() {
                    return Err(CodexAppServerError::runtime(
                        "codex app-server exited before turn completion",
                    ));
                }
                continue;
            };
            last_activity = Instant::now();
            let extracted_turn_id = extract_turn_id(&message);
            if extracted_turn_id
                .as_deref()
                .is_some_and(|turn_id| turn_id != expected_turn_id)
            {
                continue;
            }
            let mut normalized = process_codex_app_server_message(&message, &mut state);
            for event in &mut normalized {
                if event.source.app_thread_id.is_none() {
                    event.source.app_thread_id = Some(thread_id.to_string());
                }
                if event.source.app_turn_id.is_none() {
                    event.source.app_turn_id = Some(expected_turn_id.to_string());
                }
            }
            let completed = normalized
                .iter()
                .any(|event| event.kind == TurnStreamEventKind::TurnCompleted)
                && extracted_turn_id.as_deref() == Some(expected_turn_id);
            if let Some(sink) = event_sink.as_ref() {
                for event in &normalized {
                    sink(event.clone());
                }
            }
            events.extend(normalized);
            if completed {
                break;
            }
        }
        if state
            .turn_status
            .as_deref()
            .is_some_and(|status| status != "completed")
        {
            return Err(CodexAppServerError::runtime(
                state
                    .turn_error
                    .clone()
                    .unwrap_or_else(|| "codex app-server turn failed".to_string()),
            ));
        }
        if let Some(error) = state.last_error.clone() {
            return Err(CodexAppServerError::runtime(error));
        }
        Ok(CodexAppServerTurnResult {
            thread_id: thread_id.to_string(),
            turn_id: expected_turn_id.to_string(),
            state,
            events,
            raw_log_lines: Vec::new(),
        })
    }

    fn drain_steering(
        &mut self,
        handle: &SessionSteeringHandle,
        thread_id: &str,
        turn_id: &str,
        events: &mut Vec<TurnStreamEvent>,
    ) -> Result<(), CodexAppServerError> {
        for steering_turn in handle.drain_queued() {
            let text = steering_turn.text();
            if text.trim().is_empty() {
                continue;
            }
            self.steer_turn(thread_id, turn_id, &text)?;
            events.push(TurnStreamEvent {
                kind: TurnStreamEventKind::Other("steering_injected".to_string()),
                channel: None,
                source: TurnStreamSource {
                    backend: Some(CODEX_APP_SERVER_BACKEND.to_string()),
                    app_thread_id: Some(thread_id.to_string()),
                    app_turn_id: Some(turn_id.to_string()),
                    raw_kind: Some("turn_steer".to_string()),
                    ..TurnStreamSource::default()
                },
                content_delta: Some(text.clone()),
                message: Some(text),
                tool_call: None,
                request_user_input: None,
                token_usage: None,
                error: None,
                error_code: None,
                details: (!steering_turn.metadata.is_empty())
                    .then(|| json!(steering_turn.metadata)),
                phase: Some("intervention".to_string()),
                status: Some("delivered".to_string()),
            });
        }
        Ok(())
    }

    fn wait_for_response(
        &mut self,
        target_id: u64,
        timeout: Duration,
    ) -> Result<Value, CodexAppServerError> {
        let started = Instant::now();
        let target_key = target_id.to_string();
        loop {
            if let Some(response) = self.pop_pending_response(&target_key) {
                return Ok(response);
            }
            if started.elapsed() >= timeout {
                return Err(CodexAppServerError::runtime(
                    "codex app-server request timed out waiting for response",
                ));
            }
            let Some(message) = self.read_message(Duration::from_millis(100))? else {
                if self.child.try_wait().ok().flatten().is_some() {
                    return Err(CodexAppServerError::runtime(
                        "codex app-server exited unexpectedly",
                    ));
                }
                continue;
            };
            if message.get("id").and_then(id_key).as_deref() == Some(target_key.as_str()) {
                return Ok(message);
            }
            self.route_incoming_message(message)?;
        }
    }

    fn next_message(&mut self, wait: Duration) -> Result<Option<Value>, CodexAppServerError> {
        if let Some(message) = self.pending_messages.pop_front() {
            return Ok(Some(message));
        }
        let Some(message) = self.read_message(wait)? else {
            return Ok(None);
        };
        if message.get("id").is_some() && message.get("method").is_some() {
            return self.handle_server_request(message).map(Some);
        }
        if message.get("id").is_some() {
            self.queue_pending_response(message);
            return Ok(None);
        }
        Ok(message.get("method").is_some().then_some(message))
    }

    fn route_incoming_message(&mut self, message: Value) -> Result<(), CodexAppServerError> {
        if message.get("id").is_some() && message.get("method").is_some() {
            let notification = self.handle_server_request(message)?;
            self.pending_messages.push_back(notification);
        } else if message.get("id").is_some() {
            self.queue_pending_response(message);
        } else if message.get("method").is_some() {
            self.pending_messages.push_back(message);
        }
        Ok(())
    }

    fn handle_server_request(&mut self, message: Value) -> Result<Value, CodexAppServerError> {
        let request_id = message.get("id").cloned().unwrap_or(Value::Null);
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let result = if matches!(
            method,
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval"
        ) {
            json!({"decision": "acceptForSession"})
        } else if method == "item/tool/requestUserInput" {
            let empty = Map::new();
            request_user_input_empty_response(
                message
                    .get("params")
                    .and_then(Value::as_object)
                    .unwrap_or(&empty),
            )
        } else {
            Value::Null
        };
        if result.is_null() {
            self.send_json(&json!({
                "jsonrpc": "2.0",
                "id": request_id,
                "error": {"code": -32000, "message": format!("Unsupported request: {method}")},
            }))?;
        } else {
            self.send_json(&json!({"jsonrpc": "2.0", "id": request_id, "result": result}))?;
        }
        Ok(json!({
            "jsonrpc": message.get("jsonrpc").cloned().unwrap_or_else(|| json!("2.0")),
            "method": method,
            "params": message.get("params").cloned().unwrap_or_else(|| json!({})),
        }))
    }

    fn send_json(&mut self, payload: &Value) -> Result<(), CodexAppServerError> {
        let line = serde_json::to_string(payload).map_err(|error| {
            CodexAppServerError::runtime(format!("codex app-server request encode failed: {error}"))
        })?;
        self.append_trace_line("outgoing", &line)?;
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|error| {
                CodexAppServerError::runtime(format!(
                    "codex app-server stdin write failed: {error}"
                ))
            })
    }

    fn read_message(&mut self, wait: Duration) -> Result<Option<Value>, CodexAppServerError> {
        match self.stdout.recv_timeout(wait) {
            Ok(line) => {
                self.append_trace_line("incoming", &line)?;
                Ok(parse_jsonrpc_line(&line))
            }
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => Ok(None),
        }
    }

    fn append_trace_line(&self, direction: &str, line: &str) -> Result<(), CodexAppServerError> {
        if let Some(sink) = self.trace_sink.as_ref() {
            sink.append(direction, line)?;
        }
        Ok(())
    }

    fn pop_pending_response(&mut self, id: &str) -> Option<Value> {
        let responses = self.pending_responses.get_mut(id)?;
        let response = responses.pop_front();
        if responses.is_empty() {
            self.pending_responses.remove(id);
        }
        response
    }

    fn queue_pending_response(&mut self, message: Value) {
        if let Some(key) = message.get("id").and_then(id_key) {
            self.pending_responses
                .entry(key)
                .or_default()
                .push_back(message);
        }
    }
}

#[derive(Debug, Clone)]
struct CodexJsonrpcTraceSink {
    path: PathBuf,
}

impl CodexJsonrpcTraceSink {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn append(&self, direction: &str, line: &str) -> Result<(), CodexAppServerError> {
        spark_storage::append_jsonl_record(
            &self.path,
            &CodexJsonrpcTraceLine {
                timestamp: iso_now(),
                direction,
                line,
            },
        )
        .map_err(|error| {
            CodexAppServerError::runtime(format!("codex app-server trace write failed: {error}"))
        })
    }
}

#[derive(Serialize)]
struct CodexJsonrpcTraceLine<'a> {
    timestamp: String,
    direction: &'a str,
    line: &'a str,
}

impl Drop for CodexAppServerClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn parse_jsonrpc_line(line: &str) -> Option<Value> {
    serde_json::from_str::<Value>(line)
        .ok()
        .filter(Value::is_object)
}

pub fn process_codex_app_server_message(
    message: &Value,
    state: &mut CodexAppServerTurnState,
) -> Vec<TurnStreamEvent> {
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return Vec::new();
    };
    let params = message
        .get("params")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut events = Vec::new();
    match method {
        "item/commandExecution/requestApproval" => {
            events.push(
                event(
                    TurnStreamEventKind::ToolCallStarted,
                    "command_approval_requested",
                )
                .content(extract_command_text(&params))
                .tool(Value::Object(params.clone()))
                .item_id(object_text(&params, &["itemId"]))
                .build(),
            );
        }
        "item/fileChange/requestApproval" => {
            events.push(
                event(
                    TurnStreamEventKind::ToolCallStarted,
                    "file_change_approval_requested",
                )
                .tool(Value::Object(params.clone()))
                .item_id(object_text(&params, &["itemId"]))
                .build(),
            );
        }
        "item/tool/requestUserInput" => {
            events.push(
                event(
                    TurnStreamEventKind::RequestUserInputRequested,
                    "request_user_input_requested",
                )
                .request_user_input(Value::Object(params.clone()))
                .item_id(object_text(&params, &["itemId"]))
                .build(),
            );
        }
        "item/started" => {
            if let Some(item) = params.get("item").and_then(Value::as_object) {
                remember_agent_message_phase(item, state);
                if is_context_compaction_item(item) {
                    events.push(
                        event(
                            TurnStreamEventKind::ContextCompactionStarted,
                            "context_compaction_started",
                        )
                        .tool(Value::Object(item.clone()))
                        .item_id(object_text(item, &["id"]))
                        .build(),
                    );
                } else if is_tool_item(item) {
                    events.push(
                        event(TurnStreamEventKind::ToolCallStarted, "tool_item_started")
                            .tool(Value::Object(item.clone()))
                            .item_id(object_text(item, &["id"]))
                            .build(),
                    );
                }
            }
        }
        "item/completed" => {
            if let Some(item) = params.get("item").and_then(Value::as_object) {
                let item_id = object_text(item, &["id"]);
                if let Some(text) = extract_agent_message_text_from_item(item) {
                    let phase = remember_agent_message_phase(item, state)
                        .or_else(|| agent_message_phase(item));
                    state.final_agent_message = Some(text.clone());
                    events.push(
                        event(
                            TurnStreamEventKind::ContentCompleted,
                            "assistant_message_completed",
                        )
                        .channel(TurnStreamChannel::Assistant)
                        .content(Some(text))
                        .tool(Value::Object(item.clone()))
                        .item_id(item_id)
                        .phase(phase)
                        .build(),
                    );
                } else if let Some(text) = extract_plan_text_from_item(item) {
                    state.final_plan_message = Some(text.clone());
                    events.push(
                        event(TurnStreamEventKind::ContentCompleted, "plan_completed")
                            .channel(TurnStreamChannel::Plan)
                            .content(Some(text))
                            .tool(Value::Object(item.clone()))
                            .item_id(item_id)
                            .build(),
                    );
                } else if is_context_compaction_item(item) {
                    events.push(
                        event(
                            TurnStreamEventKind::ContextCompactionCompleted,
                            "context_compaction_completed",
                        )
                        .tool(Value::Object(item.clone()))
                        .item_id(item_id)
                        .build(),
                    );
                } else if is_tool_item(item) {
                    events.push(
                        event(
                            TurnStreamEventKind::ToolCallCompleted,
                            "tool_item_completed",
                        )
                        .tool(Value::Object(item.clone()))
                        .item_id(item_id)
                        .build(),
                    );
                }
            }
        }
        "item/agentMessage/delta" => {
            if let Some(delta) = object_text(&params, &["delta"]) {
                state.agent_chunks.push(delta.clone());
                let item_id = object_text(&params, &["itemId"]);
                let phase = item_id
                    .as_deref()
                    .and_then(|id| state.agent_message_phases.get(id).cloned());
                events.push(
                    event(TurnStreamEventKind::ContentDelta, "assistant_delta")
                        .channel(TurnStreamChannel::Assistant)
                        .content(Some(delta))
                        .item_id(item_id)
                        .phase(phase)
                        .build(),
                );
            }
        }
        "item/plan/delta" => {
            if let Some(delta) = object_text(&params, &["delta"]) {
                state.plan_chunks.push(delta.clone());
                events.push(
                    event(TurnStreamEventKind::ContentDelta, "plan_delta")
                        .channel(TurnStreamChannel::Plan)
                        .content(Some(delta))
                        .item_id(object_text(&params, &["itemId"]))
                        .build(),
                );
            }
        }
        "item/reasoning/summaryTextDelta" => {
            if let Some(delta) = object_text(&params, &["delta"]) {
                state.reasoning_summary_buffer.push_str(&delta);
                events.push(
                    event(TurnStreamEventKind::ContentDelta, "reasoning_delta")
                        .channel(TurnStreamChannel::Reasoning)
                        .content(Some(delta))
                        .item_id(object_text(&params, &["itemId"]))
                        .summary_index(params.get("summaryIndex").and_then(Value::as_u64))
                        .build(),
                );
            }
        }
        "item/reasoning/summaryPartAdded" => {
            if let Some(part) = params.get("part").and_then(Value::as_object) {
                if let Some(mut text) = object_text(part, &["text", "summaryText", "summary_text"])
                {
                    if !state.reasoning_summary_buffer.is_empty()
                        && text.starts_with(&state.reasoning_summary_buffer)
                    {
                        text = text[state.reasoning_summary_buffer.len()..].to_string();
                    }
                    state.reasoning_summary_buffer.clear();
                    if !text.is_empty() {
                        events.push(
                            event(TurnStreamEventKind::ContentDelta, "reasoning_delta")
                                .channel(TurnStreamChannel::Reasoning)
                                .content(Some(text))
                                .item_id(object_text(&params, &["itemId"]))
                                .summary_index(params.get("summaryIndex").and_then(Value::as_u64))
                                .build(),
                        );
                    }
                }
            }
        }
        "item/commandExecution/outputDelta" => {
            if let Some(delta) = object_text(&params, &["delta"]) {
                state.command_chunks.push(delta.clone());
                events.push(
                    event(TurnStreamEventKind::ToolCallUpdated, "command_output_delta")
                        .content(Some(delta))
                        .item_id(object_text(&params, &["itemId"]))
                        .build(),
                );
            }
        }
        "thread/tokenUsage/updated" => {
            if let Some(token_usage) = params.get("tokenUsage").cloned() {
                state.last_token_usage_payload = Some(token_usage.clone());
                state.last_token_total = token_usage
                    .pointer("/total/totalTokens")
                    .and_then(Value::as_u64);
                events.push(
                    event(
                        TurnStreamEventKind::TokenUsageUpdated,
                        "token_usage_updated",
                    )
                    .token_usage(token_usage)
                    .build(),
                );
            }
        }
        "thread/compacted" => {
            events.push(
                event(
                    TurnStreamEventKind::ContextCompactionCompleted,
                    "thread_compacted",
                )
                .build(),
            );
        }
        "error" => {
            let error = params
                .get("error")
                .and_then(Value::as_object)
                .and_then(|error| object_text(error, &["message"]))
                .or_else(|| object_text(&params, &["message"]))
                .unwrap_or_else(|| "codex app-server error".to_string());
            state.turn_status = Some("failed".to_string());
            state.turn_error = Some(error.clone());
            state.last_error = Some(error.clone());
            events.push(
                event(TurnStreamEventKind::Error, "error")
                    .error(error)
                    .build(),
            );
        }
        "turn/completed" => {
            let turn = params
                .get("turn")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let status = object_text(&turn, &["status"]);
            state.turn_status = status.clone();
            if status
                .as_deref()
                .is_some_and(|status| status != "completed")
            {
                let error = turn
                    .get("error")
                    .and_then(Value::as_object)
                    .and_then(|error| object_text(error, &["message"]))
                    .or_else(|| state.turn_error.clone())
                    .unwrap_or_else(|| {
                        format!(
                            "turn ended with status '{}'",
                            status.as_deref().unwrap_or("failed")
                        )
                    });
                state.turn_error = Some(error.clone());
                state.last_error = Some(error);
            }
            events.push(
                event(TurnStreamEventKind::TurnCompleted, "turn_completed")
                    .status(status)
                    .error_opt(state.turn_error.clone())
                    .build(),
            );
        }
        _ => {}
    }
    events
}

#[derive(Default)]
struct EventBuilder {
    kind: Option<TurnStreamEventKind>,
    raw_kind: Option<String>,
    channel: Option<TurnStreamChannel>,
    content_delta: Option<String>,
    tool_call: Option<Value>,
    request_user_input: Option<Value>,
    item_id: Option<String>,
    summary_index: Option<u64>,
    phase: Option<String>,
    status: Option<String>,
    error: Option<String>,
    token_usage: Option<Value>,
}

fn event(kind: TurnStreamEventKind, raw_kind: &str) -> EventBuilder {
    EventBuilder {
        kind: Some(kind),
        raw_kind: Some(raw_kind.to_string()),
        ..EventBuilder::default()
    }
}

impl EventBuilder {
    fn channel(mut self, channel: TurnStreamChannel) -> Self {
        self.channel = Some(channel);
        self
    }

    fn content(mut self, content: Option<String>) -> Self {
        self.content_delta = content;
        self
    }

    fn tool(mut self, tool_call: Value) -> Self {
        self.tool_call = Some(tool_call);
        self
    }

    fn request_user_input(mut self, request_user_input: Value) -> Self {
        self.request_user_input = Some(request_user_input);
        self
    }

    fn item_id(mut self, item_id: Option<String>) -> Self {
        self.item_id = item_id;
        self
    }

    fn summary_index(mut self, summary_index: Option<u64>) -> Self {
        self.summary_index = summary_index;
        self
    }

    fn phase(mut self, phase: Option<String>) -> Self {
        self.phase = phase;
        self
    }

    fn status(mut self, status: Option<String>) -> Self {
        self.status = status;
        self
    }

    fn error(mut self, error: String) -> Self {
        self.error = Some(error);
        self
    }

    fn error_opt(mut self, error: Option<String>) -> Self {
        self.error = error;
        self
    }

    fn token_usage(mut self, token_usage: Value) -> Self {
        self.token_usage = Some(token_usage);
        self
    }

    fn build(self) -> TurnStreamEvent {
        TurnStreamEvent {
            kind: self.kind.expect("event kind"),
            channel: self.channel,
            source: TurnStreamSource {
                backend: Some(CODEX_APP_SERVER_BACKEND.to_string()),
                item_id: self.item_id,
                summary_index: self.summary_index,
                raw_kind: self.raw_kind,
                ..TurnStreamSource::default()
            },
            message: self.content_delta.clone().or_else(|| self.error.clone()),
            content_delta: self.content_delta,
            tool_call: self.tool_call,
            request_user_input: self.request_user_input,
            token_usage: self.token_usage,
            error: self.error,
            error_code: None,
            details: None,
            phase: self.phase,
            status: self.status,
        }
    }
}

pub fn build_codex_runtime_environment() -> Result<BTreeMap<String, String>, CodexAppServerError> {
    let mut env_map = env::vars().collect::<BTreeMap<_, _>>();
    let original_home = env_map
        .get("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let original_codex_home = env_map
        .get("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| original_home.join(".codex"));
    let runtime_root = runtime_root(&env_map);
    ensure_dir(&runtime_root)?;
    let codex_home = runtime_root.join(".codex");
    let xdg_config_home = env_map
        .get("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| runtime_root.join(".config"));
    let xdg_data_home = env_map
        .get("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| runtime_root.join(".local/share"));
    for directory in [&runtime_root, &codex_home, &xdg_config_home, &xdg_data_home] {
        ensure_dir(directory)?;
    }

    let explicit_seed = env_map
        .get(CODEX_SEED_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/codex-seed"));
    let mut seed_candidates = Vec::new();
    for candidate in [explicit_seed, original_codex_home] {
        if candidate != codex_home && !seed_candidates.contains(&candidate) {
            seed_candidates.push(candidate);
        }
    }
    for file_name in ["auth.json", "config.toml"] {
        if let Some(source) = seed_candidates
            .iter()
            .map(|candidate| candidate.join(file_name))
            .find(|candidate| candidate.is_file())
        {
            let _ = copy_file_if_changed(&source, &codex_home.join(file_name));
        }
    }
    for candidate in seed_candidates {
        let _ = copy_tree_contents(
            &candidate.join("plugins").join("cache"),
            &codex_home.join("plugins").join("cache"),
        );
    }

    env_map.insert(
        "HOME".to_string(),
        runtime_root.to_string_lossy().into_owned(),
    );
    env_map.insert(
        "CODEX_HOME".to_string(),
        codex_home.to_string_lossy().into_owned(),
    );
    env_map.insert(
        "XDG_CONFIG_HOME".to_string(),
        xdg_config_home.to_string_lossy().into_owned(),
    );
    env_map.insert(
        "XDG_DATA_HOME".to_string(),
        xdg_data_home.to_string_lossy().into_owned(),
    );
    prepend_first_party_tool_bins_to_path(&mut env_map);
    Ok(env_map)
}

fn runtime_root(env_map: &BTreeMap<String, String>) -> PathBuf {
    env_map
        .get(CODEX_RUNTIME_ROOT_ENV)
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env_map
                .get("SPARK_HOME")
                .filter(|value| !value.trim().is_empty())
                .map(|spark_home| PathBuf::from(spark_home).join("runtime").join("codex"))
        })
        .or_else(|| {
            env_map.get("HOME").map(|home| {
                PathBuf::from(home)
                    .join(".spark")
                    .join("runtime")
                    .join("codex")
            })
        })
        .unwrap_or_else(|| PathBuf::from(".spark").join("runtime").join("codex"))
}

fn codex_executable() -> String {
    env::var("SPARK_CODEX_APP_SERVER_BIN")
        .ok()
        .and_then(|value| non_empty(&value).map(str::to_string))
        .unwrap_or_else(|| "codex".to_string())
}

fn ensure_dir(path: &Path) -> Result<(), CodexAppServerError> {
    fs::create_dir_all(path).map_err(|error| {
        CodexAppServerError::configuration(format!(
            "codex runtime directory '{}' could not be created: {error}",
            path.display()
        ))
    })
}

fn copy_file_if_changed(source: &Path, destination: &Path) -> std::io::Result<()> {
    if destination.is_file() && fs::read(source).ok() == fs::read(destination).ok() {
        return Ok(());
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, destination).map(|_| ())
}

fn copy_tree_contents(source: &Path, destination: &Path) -> std::io::Result<()> {
    if !source.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree_contents(&entry.path(), &target)?;
        } else if entry.file_type()?.is_file() {
            copy_file_if_changed(&entry.path(), &target)?;
        }
    }
    Ok(())
}

fn prepend_first_party_tool_bins_to_path(env_map: &mut BTreeMap<String, String>) {
    let tool_bin_dirs = first_party_tool_bin_dirs();
    if tool_bin_dirs.is_empty() {
        return;
    }
    let mut path_entries = tool_bin_dirs
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    if let Some(existing_path) = env_map.get("PATH").and_then(|value| non_empty(value)) {
        path_entries.push(existing_path.to_string());
    }
    env_map.insert("PATH".to_string(), path_entries.join(path_separator()));
}

fn first_party_tool_bin_dirs() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(executable) = env::current_exe() {
        if let Some(parent) = executable.parent() {
            candidates.push(parent.to_path_buf());
        }
    }
    let mut seen = Vec::<String>::new();
    let mut directories = Vec::new();
    for candidate in candidates {
        let normalized = candidate.canonicalize().unwrap_or(candidate);
        if !normalized.is_dir() {
            continue;
        }
        let key = normalized.to_string_lossy().to_string();
        if seen.iter().any(|entry| entry == &key) {
            continue;
        }
        seen.push(key);
        directories.push(normalized);
    }
    directories
}

fn path_separator() -> &'static str {
    if cfg!(windows) {
        ";"
    } else {
        ":"
    }
}

fn request_user_input_empty_response(params: &Map<String, Value>) -> Value {
    let answers = params
        .get("questions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|question| {
            let question = question.as_object()?;
            let id = object_text(question, &["id"])?;
            Some((id, json!({ "answers": [] })))
        })
        .collect::<Map<_, _>>();
    json!({ "answers": answers })
}

fn extract_turn_id(message: &Value) -> Option<String> {
    let params = message.get("params")?.as_object()?;
    object_text(params, &["turnId"]).or_else(|| {
        params
            .get("turn")
            .and_then(Value::as_object)
            .and_then(|turn| object_text(turn, &["id"]))
    })
}

fn extract_command_text(payload: &Map<String, Value>) -> Option<String> {
    for key in [
        "command",
        "commandLine",
        "command_line",
        "cmd",
        "commandText",
    ] {
        match payload.get(key) {
            Some(Value::Array(parts)) => {
                let command = parts
                    .iter()
                    .filter_map(as_non_empty_string)
                    .collect::<Vec<_>>()
                    .join(" ");
                if !command.is_empty() {
                    return Some(command);
                }
            }
            Some(value) => {
                if let Some(text) = as_non_empty_string(value) {
                    return Some(text);
                }
            }
            None => {}
        }
    }
    payload
        .get("command")
        .and_then(Value::as_object)
        .and_then(extract_command_text)
}

fn extract_agent_message_text_from_item(item: &Map<String, Value>) -> Option<String> {
    let item_type = object_text(item, &["type"])?.to_ascii_lowercase();
    if !matches!(item_type.as_str(), "agentmessage" | "agent_message") {
        return None;
    }
    if let Some(content) = item.get("content").and_then(Value::as_array) {
        let text = content
            .iter()
            .filter_map(Value::as_object)
            .filter(|entry| {
                object_text(entry, &["type"])
                    .is_some_and(|entry_type| entry_type.eq_ignore_ascii_case("text"))
            })
            .filter_map(|entry| entry.get("text").map(value_to_string))
            .collect::<Vec<_>>()
            .join("");
        if !text.trim().is_empty() {
            return Some(text.trim().to_string());
        }
    }
    object_text(item, &["text", "message", "contentText", "content_text"])
}

fn extract_plan_text_from_item(item: &Map<String, Value>) -> Option<String> {
    let item_type = object_text(item, &["type"])?
        .to_ascii_lowercase()
        .replace('_', "");
    if !matches!(item_type.as_str(), "plan" | "proposedplan") {
        return None;
    }
    object_text(
        item,
        &[
            "text",
            "planText",
            "plan_text",
            "markdown",
            "contentText",
            "content_text",
        ],
    )
    .or_else(|| {
        let text = item
            .get("content")?
            .as_array()?
            .iter()
            .filter_map(Value::as_object)
            .filter(|entry| {
                object_text(entry, &["type"]).is_some_and(|entry_type| {
                    matches!(
                        entry_type.to_ascii_lowercase().as_str(),
                        "text" | "markdown"
                    )
                })
            })
            .filter_map(|entry| entry.get("text").map(value_to_string))
            .filter_map(|text| non_empty(&text).map(str::to_string))
            .collect::<Vec<_>>()
            .join("\n\n");
        (!text.is_empty()).then_some(text)
    })
}

fn is_tool_item(item: &Map<String, Value>) -> bool {
    object_text(item, &["type"])
        .is_some_and(|item_type| matches!(item_type.as_str(), "commandExecution" | "fileChange"))
}

fn is_context_compaction_item(item: &Map<String, Value>) -> bool {
    object_text(item, &["type"]).is_some_and(|item_type| {
        item_type.to_ascii_lowercase().replace('_', "") == "contextcompaction"
    })
}

fn remember_agent_message_phase(
    item: &Map<String, Value>,
    state: &mut CodexAppServerTurnState,
) -> Option<String> {
    let item_type = object_text(item, &["type"])?.to_ascii_lowercase();
    if !matches!(item_type.as_str(), "agentmessage" | "agent_message") {
        return None;
    }
    let item_id = object_text(item, &["id"])?;
    let phase = agent_message_phase(item)?;
    state.agent_message_phases.insert(item_id, phase.clone());
    Some(phase)
}

fn agent_message_phase(item: &Map<String, Value>) -> Option<String> {
    object_text(item, &["phase"]).map(|phase| phase.to_ascii_lowercase())
}

fn model_id_from_model_list_entry(entry: &Value) -> Option<String> {
    entry
        .get("id")
        .or_else(|| entry.get("model"))
        .and_then(Value::as_str)
        .and_then(non_empty)
        .map(str::to_string)
}

fn collaboration_mode_for_chat_mode(chat_mode: Option<&str>, model: &str) -> Value {
    let mode = match chat_mode.map(str::trim).map(str::to_ascii_lowercase) {
        Some(mode) if mode == COLLABORATION_MODE_PLAN => COLLABORATION_MODE_PLAN,
        _ => COLLABORATION_MODE_DEFAULT,
    };
    json!({
        "mode": mode,
        "settings": {
            "model": model,
        },
    })
}

fn normalize_reasoning_effort(value: Option<&str>) -> Result<Option<String>, CodexAppServerError> {
    let Some(value) = value.and_then(non_empty) else {
        return Ok(None);
    };
    let normalized = value.to_ascii_lowercase();
    if matches!(normalized.as_str(), "low" | "medium" | "high" | "xhigh") {
        return Ok(Some(normalized));
    }
    Err(CodexAppServerError::configuration(
        "reasoning_effort must be blank or one of: low, medium, high, xhigh",
    ))
}

fn request_user_input_answer_content(
    answers: &BTreeMap<String, String>,
    payload: Option<&Value>,
) -> String {
    let questions = payload
        .and_then(Value::as_object)
        .and_then(|payload| payload.get("questions"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, question)| {
            let question = question.as_object()?;
            let id =
                object_text(question, &["id"]).unwrap_or_else(|| format!("question-{}", index + 1));
            let prompt = object_text(question, &["question"]).unwrap_or_default();
            Some((id, prompt))
        })
        .collect::<Vec<_>>();
    let mut consumed = Vec::new();
    let mut lines = Vec::new();
    for (id, prompt) in questions {
        if let Some(answer) = answers.get(&id).and_then(|answer| non_empty(answer)) {
            if prompt.trim().is_empty() {
                lines.push(format!("{id}: {answer}"));
            } else {
                lines.push(format!("{prompt}\nAnswer: {answer}"));
            }
            consumed.push(id);
        }
    }
    for (id, answer) in answers {
        if consumed.iter().any(|consumed| consumed == id) {
            continue;
        }
        if let Some(answer) = non_empty(answer) {
            lines.push(format!("{id}: {answer}"));
        }
    }
    lines.join("\n\n")
}

fn rpc_error(prefix: &str, response: &Value) -> CodexAppServerError {
    let message = response
        .pointer("/error/message")
        .and_then(Value::as_str)
        .and_then(non_empty)
        .map(|message| format!("{prefix}: {message}"))
        .unwrap_or_else(|| prefix.to_string());
    CodexAppServerError::runtime(message).with_details(response.clone())
}

fn id_key(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn metadata_string(metadata: &BTreeMap<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| metadata.get(*key).and_then(as_non_empty_string))
}

fn object_text(object: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(as_non_empty_string))
}

fn as_non_empty_string(value: &Value) -> Option<String> {
    let value = value_to_string(value);
    (!value.is_empty()).then_some(value)
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        other => other.to_string(),
    }
}

fn non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn iso_now() -> String {
    let now = OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

pub fn usage_from_codex_token_payload(payload: &Value) -> Option<Usage> {
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
    if input_tokens == 0 && output_tokens == 0 && total_tokens == 0 {
        return None;
    }
    Some(
        Usage {
            input_tokens,
            output_tokens,
            total_tokens,
            reasoning_tokens: value_u64(total, &["reasoningOutputTokens", "reasoning_tokens"]),
            cache_read_tokens: value_u64(total, &["cachedInputTokens", "cache_read_tokens"]),
            cache_write_tokens: value_u64(total, &["cacheWriteTokens", "cache_write_tokens"]),
            raw: Some(payload.clone()),
        }
        .normalized(),
    )
}

fn value_u64(value: &Value, keys: &[&str]) -> Option<u64> {
    let object = value.as_object()?;
    keys.iter().find_map(|key| object.get(*key)?.as_u64())
}
