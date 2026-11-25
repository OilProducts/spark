use std::collections::{BTreeMap, VecDeque};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
use std::time::Duration;

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::Notify;
use tokio::time::Instant;

use serde::Serialize;
use serde_json::{json, Map, Value};
use spark_common::debug::{codex_jsonrpc_trace_enabled, CODEX_JSONRPC_TRACE_PATH_METADATA_KEY};
use spark_common::events::{
    TurnStreamChannel, TurnStreamEvent, TurnStreamEventKind, TurnStreamSource,
};
use spark_common::paths::Environment;
use time::OffsetDateTime;
use unified_llm_adapter::Usage;

use crate::agent::{
    AgentRawLogLine, AgentRequestUserInputAnswerRequest, AgentThreadResumeFailure,
    AgentTurnEventSink, AgentTurnOutput, AgentTurnRequest,
};
use crate::file_changes::{self, prefixed, FileChange};
use crate::stdio_transport::{
    self, sleep_until, Blocking, Budget, Connection, End, Endpoints, Held, Outgoing, Refused,
    Status, Stop, LIMITS, PENDING_LIMIT,
};

pub mod auth;

pub const CODEX_APP_SERVER_BACKEND: &str = "codex_app_server";
/// Control-plane requests (initialize, thread/start, turn/start, …) normally
/// answer in well under a second; the bound only has to catch a wedged
/// app-server. Fifteen seconds proved too tight once startup work such as MCP
/// server launch or an OAuth token refresh sits in front of the first reply.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const TURN_IDLE_TIMEOUT: Duration = Duration::from_secs(300);
const CODEX_RUNTIME_ROOT_ENV: &str = "ATTRACTOR_CODEX_RUNTIME_ROOT";
const CODEX_SEED_DIR_ENV: &str = "ATTRACTOR_CODEX_SEED_DIR";
static CODEX_SPARK_HOME: OnceLock<PathBuf> = OnceLock::new();
static AGENT_API: OnceLock<AgentApi> = OnceLock::new();

/// The automatic owner bearer, the origin it may be sent to, and the same
/// server's loopback URL. Spark exports them, empty when unset, to every agent
/// it launches, so a value inherited from elsewhere never survives.
pub const MANAGED_AGENT_API_KEYS: [&str; 3] = [
    "SPARK_AGENT_API_TOKEN",
    "SPARK_AGENT_API_ORIGIN",
    "SPARK_AGENT_LOOPBACK_URL",
];

/// How agents reach this Spark: exported as `SPARK_API_BASE_URL` and, for a
/// non-loopback `base_url`, the `MANAGED_AGENT_API_KEYS`.
#[derive(Clone, PartialEq, Eq)]
pub struct AgentApi {
    pub base_url: String,
    /// The owner bearer for a non-loopback `base_url`; never printed.
    pub token: Option<String>,
    /// The same server's loopback URL, beside a non-loopback `base_url`.
    pub loopback_url: Option<String>,
}

/// Records, once per process, how the agents this process launches reach
/// its Spark, so the spark CLI inside agent shells needs no --base-url.
pub fn configure_agent_api(api: AgentApi) -> Result<(), String> {
    match AGENT_API.set(api.clone()) {
        Ok(()) => Ok(()),
        Err(_) if AGENT_API.get() == Some(&api) => Ok(()),
        Err(_) => Err("The agent API endpoint is already configured differently.".to_string()),
    }
}

/// Windows names are case-insensitive: drops another spelling of a managed
/// key from an assembled environment, so launching cannot reinstate its
/// inherited value over the one Spark exports.
pub(crate) fn drop_managed_key_aliases<V>(env: &mut BTreeMap<String, V>) {
    if cfg!(windows) && AGENT_API.get().is_some() {
        // As `merge_env` compares Windows names, beyond ASCII too.
        env.retain(|key, _| {
            !MANAGED_AGENT_API_KEYS
                .iter()
                .any(|managed| key != managed && key.to_uppercase() == *managed)
        });
    }
}
const COLLABORATION_MODE_DEFAULT: &str = "default";
const COLLABORATION_MODE_PLAN: &str = "plan";
/// Reasoning summary level sent when a turn does not choose one.
const DEFAULT_REASONING_SUMMARY: &str = "detailed";

pub fn configure_codex_spark_home(path: impl Into<PathBuf>) -> Result<(), String> {
    let path = path.into();
    match CODEX_SPARK_HOME.set(path.clone()) {
        Ok(()) => Ok(()),
        Err(_) if CODEX_SPARK_HOME.get() == Some(&path) => Ok(()),
        Err(_) => Err("Codex Spark home is already configured to a different path.".to_string()),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodexAppServerError {
    pub message: String,
    pub retryable: bool,
    pub details: Option<Value>,
    pub artifact: bool,
}

impl CodexAppServerError {
    fn configuration(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
            details: None,
            artifact: false,
        }
    }

    fn runtime(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            message: auth::normalize_auth_error(message),
            retryable: false,
            details: None,
            artifact: false,
        }
    }

    fn artifact(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
            details: None,
            artifact: true,
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
        self.run_agent_turn_with_event_sink(request, None)
    }

    pub fn run_agent_turn_with_event_sink(
        &self,
        request: AgentTurnRequest,
        event_sink: Option<AgentTurnEventSink>,
    ) -> Result<AgentTurnOutput, CodexAppServerError> {
        self.run_agent_turn_with_capture_observer(request, event_sink, None, None)
    }

    pub(crate) fn run_agent_turn_with_capture_observer(
        &self,
        request: AgentTurnRequest,
        event_sink: Option<AgentTurnEventSink>,
        capture_observer: Option<Arc<dyn Fn() + Send + Sync>>,
        clarification_handler: Option<crate::codergen::ClarificationHandler>,
    ) -> Result<AgentTurnOutput, CodexAppServerError> {
        let configuration = crate::config::captured_session_config(&request.metadata)
            .map_err(CodexAppServerError::configuration)?;
        let native = configuration.as_ref().map(|config| &config.native);
        let trace_path = native
            .and_then(|config| config.codex_jsonrpc_trace)
            .unwrap_or_else(codex_jsonrpc_trace_enabled)
            .then(|| {
                metadata_string(&request.metadata, &[CODEX_JSONRPC_TRACE_PATH_METADATA_KEY])
                    .map(PathBuf::from)
            })
            .flatten();
        // Registered before the handshake so a stop also ends a stuck `initialize`.
        let stop = TurnStop::new();
        interrupt_registry().lock().unwrap().insert(
            (
                request.project_path.clone(),
                request.conversation_id.clone(),
            ),
            Arc::downgrade(&stop),
        );
        let mut client = CodexAppServerClient::connect_with_settings(
            PathBuf::from(&request.project_path),
            trace_path,
            native,
            crate::agent::agent_turn_env(&request.metadata),
            Some(Arc::clone(&stop)),
            clarification_handler,
        )
        // Startup can fail before a client exists to close the turn.
        .inspect_err(|_| stop.close())?;
        client.developer_instructions = metadata_string(
            &request.metadata,
            &[crate::agent::AGENT_INSTRUCTIONS_METADATA_KEY],
        );
        let model = request
            .model
            .as_deref()
            .and_then(non_empty)
            .map(str::to_string);
        let resume_thread_id = metadata_string(
            &request.metadata,
            &[
                "spark.runtime.codex_app_server.thread_id",
                "codex_app_server.thread_id",
                "codex_thread_id",
            ],
        );
        let attachments = crate::attachments::request_attachments(&request.metadata);
        let earlier_attachments = if resume_thread_id.is_some() {
            Vec::new()
        } else {
            crate::attachments::history_attachments(&request.history)
        };
        let prompt = crate::attachments::prompt_with_attachment_references(
            &request.prompt,
            &attachments,
            &earlier_attachments,
        );
        let local_images = attachments
            .iter()
            .filter(|attachment| attachment.is_native_image())
            .map(|attachment| attachment.path.clone())
            .collect::<Vec<_>>();
        let thread_id = if let Some(thread_id) = resume_thread_id {
            match client.resume_thread(&thread_id, model.as_deref(), Some(&request.project_path)) {
                Ok(resumed) => resumed,
                // Authentication failure says nothing about the saved thread's validity.
                Err(error) if auth::requires_login(&error.message) => return Err(error),
                Err(error) => {
                    let output = thread_resume_failure_output(
                        "thread/resume",
                        AgentThreadResumeFailure {
                            message: error.message,
                            error_code: Some("codex_app_server_resume_failed".to_string()),
                            details: error.details,
                        },
                    );
                    // No turn will take a message the sink may send.
                    stop.close();
                    if let Some(sink) = event_sink.as_ref() {
                        for event in &output.events {
                            sink(event.clone());
                        }
                    }
                    return Ok(output);
                }
            }
        } else {
            client.start_thread(
                model.as_deref(),
                Some(&request.project_path),
                codex_app_server_thread_is_ephemeral(&request),
            )?
        };
        let capture_path = crate::initial_context::path_from_metadata(&request.metadata);
        let result = client.run_turn_with_capture(
            &thread_id,
            &prompt,
            &local_images,
            model.as_deref(),
            request.reasoning_effort.as_deref(),
            request
                .metadata
                .get("spark.execution.reasoning")
                .or_else(|| {
                    request
                        .metadata
                        .get("spark.execution.settings")
                        .and_then(|v| v.get("model_settings"))
                })
                .and_then(|v| v["reasoning_summary"].as_str()),
            request.chat_mode.as_deref(),
            Some(&request.project_path),
            event_sink,
            capture_path.as_deref(),
            capture_observer,
        )?;
        Ok(agent_output_from_app_server_result(result))
    }

    /// Steers the conversation's running turn with a user message through
    /// `turn/steer`. True once the app-server accepted it.
    pub fn send_message(
        &self,
        project_path: &str,
        conversation_id: &str,
        message_id: &str,
        message: &str,
    ) -> bool {
        let stop = {
            let registry = interrupt_registry().lock().unwrap();
            registry
                .get(&(project_path.to_string(), conversation_id.to_string()))
                .and_then(Weak::upgrade)
        };
        let Some(stop) = stop else {
            return false;
        };
        // Refused before it is queued: longer than one line, or no room.
        if message.len() > LIMITS.frame_bytes {
            return false;
        }
        let Some(held) = stop.steer_budget.try_admit(message.len()) else {
            return false;
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        match stop.messages.lock().unwrap().as_mut() {
            Some(messages) => messages.push_back(SteerMessage {
                id: message_id.to_string(),
                text: message.to_string(),
                accepted: sender,
                _held: held,
            }),
            // The turn has ended and takes no more messages.
            None => return false,
        }
        stop.wake.notify_one();
        // The turn loop answers once turn/steer returns; a turn that ends
        // first answers false.
        match receiver.recv_timeout(REQUEST_TIMEOUT + Duration::from_secs(1)) {
            Ok(accepted) => accepted,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => false,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Withdraw a message the turn has not taken, so it is only
                // ever queued or delivered, never both. A taken message is
                // mid turn/steer, which its own request timeout bounds.
                let mut messages = stop.messages.lock().unwrap();
                if let Some(messages) = messages.as_mut() {
                    if let Some(index) = messages.iter().position(|queued| queued.id == message_id)
                    {
                        messages.remove(index);
                        return false;
                    }
                }
                drop(messages);
                receiver.recv().unwrap_or(false)
            }
        }
    }

    /// Asks the conversation's running turn, if any, to stop.
    pub fn interrupt(&self, project_path: &str, conversation_id: &str) -> bool {
        Self::signal(project_path, conversation_id, |stop| {
            stop.interrupt.store(true, Ordering::SeqCst);
            stop.wake.notify_one();
        })
    }

    /// Ends the conversation's running turn and kills its app-server, for a
    /// turn that did not stop after its interrupt.
    pub fn terminate(&self, project_path: &str, conversation_id: &str) -> bool {
        Self::signal(project_path, conversation_id, TurnStop::terminate)
    }

    fn signal(project_path: &str, conversation_id: &str, act: impl Fn(&TurnStop)) -> bool {
        let stop = {
            let mut registry = interrupt_registry().lock().unwrap();
            registry.retain(|_, stop| stop.strong_count() > 0);
            registry
                .get(&(project_path.to_string(), conversation_id.to_string()))
                .and_then(Weak::upgrade)
        };
        stop.is_some_and(|stop| {
            act(&stop);
            true
        })
    }

    pub fn answer_request_user_input(
        &self,
        request: AgentRequestUserInputAnswerRequest,
    ) -> Result<AgentTurnOutput, CodexAppServerError> {
        let answers = match request_user_input_answer_payload(&request) {
            Ok(answers) => answers,
            Err(failure) => {
                return Ok(request_user_input_resume_failure_output(
                    request.request_id.as_str(),
                    failure,
                ));
            }
        };
        let Some(pending) = find_pending_request_user_input(&request) else {
            return Ok(request_user_input_resume_failure_output(
                request.request_id.as_str(),
                AgentThreadResumeFailure {
                    message:
                        "request-user-input answer could not resume because the live request is no longer pending."
                            .to_string(),
                    error_code: Some("request_user_input_not_pending".to_string()),
                    details: Some(json!({ "request_id": request.request_id })),
                },
            ));
        };
        pending.deliver_answers(answers.clone());
        Ok(AgentTurnOutput {
            events: vec![TurnStreamEvent {
                kind: TurnStreamEventKind::Other("request_user_input_answer_delivered".to_string()),
                channel: None,
                source: TurnStreamSource {
                    backend: Some(CODEX_APP_SERVER_BACKEND.to_string()),
                    app_thread_id: pending.app_thread_id.clone(),
                    app_turn_id: pending.app_turn_id.clone(),
                    item_id: Some(pending.request_id.clone()),
                    raw_kind: Some("request_user_input_answer_delivered".to_string()),
                    ..TurnStreamSource::default()
                },
                content_delta: None,
                message: Some("request-user-input answer delivered.".to_string()),
                tool_call: None,
                request_user_input: Some(request_user_input_response_payload(&answers)),
                token_usage: None,
                error: None,
                error_code: None,
                details: Some(json!({ "request_id": pending.request_id })),
                phase: Some("request_user_input_answer".to_string()),
                status: Some("delivered".to_string()),
            }],
            ..AgentTurnOutput::default()
        })
    }
}

/// Flow agent turns run in throwaway threads, except a session node's,
/// which a later node resumes.
fn codex_app_server_thread_is_ephemeral(request: &AgentTurnRequest) -> bool {
    request
        .chat_mode
        .as_deref()
        .is_some_and(|mode| mode.trim().eq_ignore_ascii_case("agent"))
        && !request
            .metadata
            .contains_key(crate::codergen::SESSION_NAME_METADATA_KEY)
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

/// A synchronous handle on one app-server. Its connection's worker owns the
/// JSON-RPC state and keeps servicing the app-server between calls.
pub struct CodexAppServerClient {
    /// Declared before `connection`: the worker runs until this closes, and
    /// dropping the connection waits for the worker.
    commands: UnboundedSender<(Command, Held)>,
    command_budget: Budget,
    controls: Arc<TurnStop>,
    /// Flow questions go to a clarification handler instead of a person.
    clarifies: bool,
    /// Pinned system instructions sent as `developerInstructions` on
    /// `thread/start` and `thread/resume`.
    developer_instructions: Option<String>,
    executable: PathBuf,
    connection: Connection,
}

impl CodexAppServerClient {
    pub fn connect(working_dir: PathBuf) -> Result<Self, CodexAppServerError> {
        Self::connect_with_trace_path(working_dir, None)
    }

    pub fn connect_with_trace_path(
        working_dir: PathBuf,
        trace_path: Option<PathBuf>,
    ) -> Result<Self, CodexAppServerError> {
        Self::connect_with_settings(working_dir, trace_path, None, BTreeMap::new(), None, None)
    }

    /// `controls` are a registered turn's; a standalone client gets its own.
    /// `turn_env` adds to the runtime environment the turn's own bindings.
    fn connect_with_settings(
        working_dir: PathBuf,
        trace_path: Option<PathBuf>,
        native: Option<&spark_common::agent_settings::NativeAgentSettings>,
        turn_env: BTreeMap<String, OsString>,
        controls: Option<Arc<TurnStop>>,
        clarification_handler: Option<crate::codergen::ClarificationHandler>,
    ) -> Result<Self, CodexAppServerError> {
        let controls = controls.unwrap_or_else(TurnStop::new);
        if !working_dir.exists() {
            return Err(CodexAppServerError::configuration(format!(
                "codex app-server working directory is unavailable in the runtime: {}",
                working_dir.display()
            )));
        }
        let executable = codex_program(native);
        let mut command = std::process::Command::new(&executable);
        command
            // Models that return raw reasoning stream it as item/reasoning/textDelta.
            .arg("-c")
            .arg("show_raw_agent_reasoning=true")
            .arg("app-server")
            .current_dir(&working_dir)
            .envs(build_codex_runtime_environment_with_settings(native)?)
            .envs(turn_env);
        let launch_failed = |error: std::io::Error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                CodexAppServerError::configuration(format!(
                    "codex app-server executable not found: {} (install Codex, or set its program \
                     in Settings > Models & accounts > Agent harnesses)",
                    executable.display()
                ))
            } else {
                CodexAppServerError::runtime(format!(
                    "codex app-server launch failed for {}: {error}",
                    executable.display()
                ))
            }
        };
        let (completions, completed) = unbounded_channel();
        let clarifies = clarification_handler.is_some();
        let clarification = match clarification_handler {
            Some(handler) => Some((handler, Blocking::spawn(&LIMITS).map_err(launch_failed)?)),
            None => None,
        };
        let trace = match trace_path {
            Some(path) => Some(Trace {
                sink: CodexJsonrpcTraceSink::new(path),
                work: Blocking::spawn(&LIMITS).map_err(launch_failed)?,
                failure: Arc::default(),
                completions: completions.clone(),
            }),
            None => None,
        };
        let (commands, received) = unbounded_channel();
        let worker = Worker {
            controls: Arc::clone(&controls),
            clarification,
            trace,
            completions,
            next_id: 0,
            pending: BTreeMap::new(),
            backlog: VecDeque::new(),
            backlog_budget: Budget::new(&LIMITS),
            turn: None,
            waiter: None,
            chat_question: None,
            flow_questions: Vec::new(),
            flow_due: None,
            next_question: 0,
            fenced: Vec::new(),
            stalled: Vec::new(),
            ended: None,
        };
        // Its own process group, so a forced stop also ends the agent's tools.
        let connection = stdio_transport::launch(command, LIMITS, move |endpoints| {
            worker.run(endpoints, received, completed)
        })
        .map_err(launch_failed)?;
        controls.attach(connection.stop_handle());
        let mut client = Self {
            commands,
            command_budget: Budget::new(&LIMITS),
            controls,
            clarifies,
            developer_instructions: None,
            executable,
            connection,
        };
        let response = client
            .send_request(
                "initialize",
                Some(json!({
                    "clientInfo": {"name": "spark", "version": "0.1"},
                    "capabilities": {"experimentalApi": true},
                })),
            )
            .map_err(|error| client.with_diagnostics(error))?;
        if response.get("error").is_some() {
            return Err(
                client.with_diagnostics(rpc_error("codex app-server initialize failed", &response))
            );
        }
        client
            .notify("initialized")
            .map_err(|error| client.with_diagnostics(error))?;
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
        if self.clarifies {
            params["config"] = json!({"features.default_mode_request_user_input": true});
        }
        if let Some(instructions) = &self.developer_instructions {
            params["developerInstructions"] = json!(instructions);
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
        if let Some(instructions) = &self.developer_instructions {
            params["developerInstructions"] = json!(instructions);
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
        let response = self
            .send_request("model/list", Some(json!({ "limit": 100 })))
            .map_err(|error| self.with_diagnostics(error))?;
        if response.get("error").is_some() {
            return Err(
                self.with_diagnostics(rpc_error("codex app-server model/list failed", &response))
            );
        }
        Ok(response.get("result").cloned().unwrap_or_else(|| json!({})))
    }

    fn with_diagnostics(&self, mut error: CodexAppServerError) -> CodexAppServerError {
        let stderr = Some(self.connection.stderr()).filter(|value| !value.trim().is_empty());
        let cause = error.details.take();
        error.details = Some(json!({
            "cause": cause,
            "executable": self.executable,
            "stderr": stderr,
        }));
        error
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
        event_sink: Option<AgentTurnEventSink>,
    ) -> Result<CodexAppServerTurnResult, CodexAppServerError> {
        self.run_turn_with_capture(
            thread_id,
            prompt,
            &[],
            model,
            reasoning_effort,
            None,
            chat_mode,
            cwd,
            event_sink,
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn run_turn_with_capture(
        &mut self,
        thread_id: &str,
        prompt: &str,
        local_images: &[String],
        model: Option<&str>,
        reasoning_effort: Option<&str>,
        reasoning_summary: Option<&str>,
        chat_mode: Option<&str>,
        cwd: Option<&str>,
        event_sink: Option<AgentTurnEventSink>,
        capture_path: Option<&Path>,
        capture_observer: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> Result<CodexAppServerTurnResult, CodexAppServerError> {
        // An image-only turn sends no empty text item.
        let mut input = Vec::new();
        if !prompt.is_empty() || local_images.is_empty() {
            input.push(json!({"type": "text", "text": prompt}));
        }
        input.extend(
            local_images
                .iter()
                .map(|path| json!({"type": "localImage", "path": path})),
        );
        let mut params = json!({
            "threadId": thread_id,
            "input": input,
            "approvalPolicy": "never",
            "sandboxPolicy": {"type": "dangerFullAccess"},
            "cwd": cwd,
        });
        let effort = normalize_reasoning_effort(reasoning_effort)?;
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
                collaboration_mode_for_chat_mode(chat_mode, &effective_model, effort.as_deref());
        } else {
            if let Some(model) = effective_model {
                params["model"] = json!(model);
            }
            if let Some(effort) = effort {
                params["effort"] = json!(effort);
            }
        }
        // Without an explicit level Codex falls back to its own default (auto),
        // bypassing model_reasoning_summary in config.toml, so Spark's default
        // stays detailed summaries; an explicitly chosen level wins.
        params["summary"] = json!(reasoning_summary.unwrap_or(DEFAULT_REASONING_SUMMARY));
        if let Some(path) = capture_path {
            crate::initial_context::write_if_absent(path, prompt).map_err(|error| {
                CodexAppServerError::artifact(format!(
                    "persist initial LLM context at {}: {error}",
                    path.display()
                ))
            })?;
            if let Some(observer) = capture_observer {
                observer();
            }
        }
        let (messages, received) = mpsc::channel();
        let bytes = json_len(&params);
        self.submit(
            Command::Turn {
                thread_id: thread_id.to_string(),
                params,
                sink: event_sink.is_some(),
                messages,
            },
            bytes,
        )?;
        // This thread is the sink's, so the sink has seen every event before
        // the turn returns, and the worker never waits on a callback.
        for message in received {
            match message {
                TurnMessage::Event(event) => {
                    if let Some(sink) = event_sink.as_ref() {
                        sink(event);
                    }
                }
                TurnMessage::Done(result) => return result,
            }
        }
        Err(connection_lost())
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
        let (reply, response) = mpsc::channel();
        let bytes = params.as_ref().map_or(0, json_len);
        self.submit(
            Command::Request {
                method: method.to_string(),
                params,
                reply,
            },
            bytes,
        )?;
        response.recv().map_err(|_| connection_lost())?
    }

    fn notify(&mut self, method: &str) -> Result<(), CodexAppServerError> {
        let (reply, written) = mpsc::channel();
        self.submit(
            Command::Notify {
                method: method.to_string(),
                reply,
            },
            0,
        )?;
        written.recv().map_err(|_| connection_lost())?
    }

    /// The next notification, or a server request as one, if one arrives
    /// within `wait`; why not once the connection has ended and none is left.
    fn next_message(&mut self, wait: Duration) -> Result<Option<Value>, Ended> {
        let (reply, next) = mpsc::channel();
        self.submit(Command::Next { wait, reply }, 0)
            .map_err(Ended::Failed)?;
        next.recv().map_err(|_| Ended::Failed(connection_lost()))?
    }

    /// Hands the worker a command, unless its queue is full.
    fn submit(&self, command: Command, bytes: usize) -> Result<(), CodexAppServerError> {
        let held = self
            .command_budget
            .try_admit(bytes)
            .ok_or_else(|| overloaded("its command queue"))?;
        self.commands
            .send((command, held))
            .map_err(|_| connection_lost())
    }
}

impl Drop for CodexAppServerClient {
    fn drop(&mut self) {
        // Takes no more messages. The connection, dropped after the command
        // sender, kills a still-live app-server at once.
        self.controls.close();
    }
}

/// Settles one facade call.
type Reply<T> = mpsc::Sender<Result<T, CodexAppServerError>>;

/// What a client asks of its worker.
enum Command {
    Request {
        method: String,
        params: Option<Value>,
        reply: Reply<Value>,
    },
    /// A notification, settled once admitted for writing.
    Notify { method: String, reply: Reply<()> },
    /// Starts a turn and streams it back.
    Turn {
        thread_id: String,
        params: Value,
        sink: bool,
        messages: mpsc::Sender<TurnMessage>,
    },
    Next {
        wait: Duration,
        reply: mpsc::Sender<Result<Option<Value>, Ended>>,
    },
}

/// What a turn hands its caller, in order: events for the sink, then its
/// result.
enum TurnMessage {
    Event(TurnStreamEvent),
    Done(Result<CodexAppServerTurnResult, CodexAppServerError>),
}

/// Why a connection ended. The first cause wins.
#[derive(Clone, Debug)]
enum Ended {
    /// Stdout ended: `None` once the app-server exited, else the read failure.
    Stdout(Option<String>),
    Terminated,
    Failed(CodexAppServerError),
}

impl Ended {
    /// The error for a caller; `exited` says what an exit cut short.
    fn error(&self, exited: &str) -> CodexAppServerError {
        match self {
            Self::Stdout(read_error) => stream_end_error(read_error.clone(), exited),
            Self::Terminated => CodexAppServerError::runtime(
                "codex app-server was terminated after its turn did not stop",
            ),
            Self::Failed(error) => error.clone(),
        }
    }
}

const REQUEST_EXITED: &str = "codex app-server exited unexpectedly";
const TURN_EXITED: &str = "codex app-server exited before turn completion";
const REQUEST_TIMED_OUT: &str = "codex app-server request timed out waiting for response";
/// How often a flow question's answer is looked for.
const FLOW_POLL: Duration = Duration::from_millis(100);

fn connection_lost() -> CodexAppServerError {
    CodexAppServerError::runtime("codex app-server connection worker stopped")
}

fn overloaded(queue: &str) -> CodexAppServerError {
    CodexAppServerError::runtime(format!("codex app-server overloaded: {queue} is full"))
}

/// Serialized size, for queue accounting.
pub(crate) fn json_len(value: &impl Serialize) -> usize {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    let _ = serde_json::to_writer(&mut count, value);
    count.0
}

/// Writes the JSON-RPC trace in order on its own thread. Replies pass
/// through it, so a caller hears back only once every line traced before
/// its reply is written, or of the failure that stopped the trace.
struct Trace {
    sink: CodexJsonrpcTraceSink,
    work: Blocking,
    failure: Arc<Mutex<Option<CodexAppServerError>>>,
    completions: UnboundedSender<Completion>,
}

impl Trace {
    /// Queues `line`; a full queue fails the trace, and so the connection.
    fn line(&self, direction: &'static str, line: String) -> Result<(), CodexAppServerError> {
        let (sink, failure, completions) = (
            self.sink.clone(),
            Arc::clone(&self.failure),
            self.completions.clone(),
        );
        self.work
            .run(line.len(), move || {
                if lock(&failure).is_some() {
                    return;
                }
                // Unlocked meanwhile: the worker reads it while storage blocks.
                if let Err(error) = sink.append(direction, &line) {
                    lock(&failure).get_or_insert(error.clone());
                    let _ = completions.send(Completion::TraceFailed(error));
                }
            })
            .map_err(|_| self.overflowed())
    }

    /// Later lines are dropped, not written out of order.
    fn overflowed(&self) -> CodexAppServerError {
        let error = overloaded("its trace queue");
        lock(&self.failure).get_or_insert(error.clone());
        error
    }

    fn failure(&self) -> Option<CodexAppServerError> {
        lock(&self.failure).clone()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A reply waiting behind the trace, taken by whichever runs it first: the
/// trace thread, or the connection's end.
type Fenced = Mutex<Option<Box<dyn FnOnce(Option<CodexAppServerError>) + Send>>>;

/// Blocking work finished off the runtime.
enum Completion {
    /// The clarification handler took a flow question.
    Asked {
        key: u64,
        poll: Result<crate::codergen::ClarificationPoll, String>,
    },
    Polled {
        key: u64,
        poll: crate::codergen::ClarificationPoll,
        answers: Result<Option<BTreeMap<String, String>>, String>,
    },
    TraceFailed(CodexAppServerError),
}

struct Pending {
    deadline: Instant,
    call: Call,
}

/// Who waits on a request's response.
enum Call {
    Facade(Reply<Value>),
    TurnStart,
    Steer(SteerMessage),
    Interrupt,
}

struct FlowQuestion {
    key: u64,
    request_id: Value,
    /// `None` until the handler returns, and while a poll runs.
    poll: Option<crate::codergen::ClarificationPoll>,
}

struct Turn {
    thread_id: String,
    /// Known once turn/start answers.
    turn_id: Option<String>,
    state: CodexAppServerTurnState,
    /// The turn's record: every event but thinking progress.
    events: Vec<TurnStreamEvent>,
    messages: mpsc::Sender<TurnMessage>,
    /// The caller has a sink.
    sink: bool,
    /// Renewed only by provider output; suspended while a question waits on
    /// a person, with a full window again once none does.
    idle_deadline: Option<Instant>,
    stopping: bool,
    /// A turn/steer is outstanding: output read meanwhile waits behind it.
    steering: bool,
    /// Set once the turn has ended; sent once the sink has the events before it.
    result: Option<Result<CodexAppServerTurnResult, CodexAppServerError>>,
}

impl Turn {
    /// Hands `event` to the sink, never waiting on it: a slow sink falls
    /// behind rather than failing the turn or stalling the worker it may be
    /// waiting on.
    fn sequence(&mut self, event: TurnStreamEvent) {
        // ponytail: unbounded, like the turn's record, which already holds
        // every event but thinking progress.
        if self.sink {
            let _ = self.messages.send(TurnMessage::Event(event.clone()));
        }
        // Thinking progress is live-only: it is never part of the turn's record.
        if event.thinking_state().is_none() {
            self.events.push(event);
        }
    }

    fn finish(&mut self, result: Result<CodexAppServerTurnResult, CodexAppServerError>) {
        self.result.get_or_insert(result);
    }

    fn outcome(&mut self) -> Result<CodexAppServerTurnResult, CodexAppServerError> {
        let state = std::mem::take(&mut self.state);
        if state.turn_status.as_deref().is_some_and(|status| {
            status != "completed" && !(self.stopping && status == "interrupted")
        }) {
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
            thread_id: self.thread_id.clone(),
            turn_id: self.turn_id.clone().unwrap_or_default(),
            state,
            events: std::mem::take(&mut self.events),
            raw_log_lines: Vec::new(),
        })
    }
}

/// The app-server's JSON-RPC state, owned by its connection's worker: the
/// sole reader of its output and writer of its input, which also keeps
/// deadlines and controls between facade calls.
struct Worker {
    controls: Arc<TurnStop>,
    clarification: Option<(crate::codergen::ClarificationHandler, Blocking)>,
    trace: Option<Trace>,
    completions: UnboundedSender<Completion>,
    next_id: u64,
    pending: BTreeMap<u64, Pending>,
    /// Notifications no turn or caller has taken yet: read before a turn
    /// starts, while a steer is outstanding, or between calls.
    backlog: VecDeque<(Value, Option<Held>)>,
    backlog_budget: Budget,
    turn: Option<Turn>,
    waiter: Option<(Instant, mpsc::Sender<Result<Option<Value>, Ended>>)>,
    chat_question: Option<PendingRequestUserInputResponse>,
    flow_questions: Vec<FlowQuestion>,
    flow_due: Option<Instant>,
    next_question: u64,
    /// Replies queued behind the trace, settled directly once the connection ends.
    fenced: Vec<Weak<Fenced>>,
    /// Timed-out steers, answered false once the app-server has exited.
    stalled: Vec<SteerMessage>,
    ended: Option<Ended>,
}

impl Worker {
    /// Runs until the client is gone.
    async fn run(
        mut self,
        endpoints: Endpoints,
        mut commands: UnboundedReceiver<(Command, Held)>,
        mut completed: UnboundedReceiver<Completion>,
    ) {
        let Endpoints {
            mut stdout,
            stdin,
            mut status,
            stop,
        } = endpoints;
        let mut io = Io { stdin, stop };
        loop {
            let deadline = self.next_deadline();
            let reading = self.ended.is_none();
            let wake = Arc::clone(&self.controls.wake);
            tokio::select! {
                command = commands.recv() => match command {
                    Some((command, _held)) => self.command(&mut io, command).await,
                    None => {
                        // Dropping a stalled steer answers false: not before the stopped
                        // app-server has exited (bounded by its termination grace).
                        if !self.stalled.is_empty() {
                            let _ = status.wait_for(|status| status.exited.is_some()).await;
                        }
                        return;
                    }
                },
                line = stdout.next(), if reading => match line {
                    Some(line) => self.line(&mut io, line.text).await,
                    None => {
                        let read_error = match stdout.end() {
                            Some(End::Failed(error)) => Some(error.clone()),
                            _ => None,
                        };
                        self.end(&io, Ended::Stdout(read_error)).await;
                    }
                },
                Some(completion) = completed.recv() => self.completion(&mut io, completion).await,
                () = wake.notified() => {}
                () = sleep_until(deadline) => {}
                Ok(()) = status.changed() => {
                    let status = status.borrow_and_update().clone();
                    self.status(&io, status).await;
                }
            }
            self.advance(&mut io).await;
        }
    }

    fn next_deadline(&self) -> Option<Instant> {
        let idle = self
            .turn
            .as_ref()
            .filter(|turn| turn.result.is_none())
            .and_then(|turn| turn.idle_deadline);
        let flow = self.flow_due.filter(|_| !self.flow_questions.is_empty());
        self.pending
            .values()
            .map(|pending| pending.deadline)
            .chain(idle)
            .chain(flow)
            .chain(self.waiter.as_ref().map(|(deadline, _)| *deadline))
            .min()
    }

    async fn command(&mut self, io: &mut Io, command: Command) {
        match command {
            Command::Request {
                method,
                params,
                reply,
            } => match &self.ended {
                Some(ended) => {
                    let error = ended.error(REQUEST_EXITED);
                    self.reply(reply, Err(error)).await;
                }
                None => self.request(io, &method, params, Call::Facade(reply)).await,
            },
            Command::Notify { method, reply } => {
                let result = match &self.ended {
                    Some(ended) => Err(ended.error(REQUEST_EXITED)),
                    None => {
                        self.write(
                            io,
                            json!({"jsonrpc": "2.0", "method": method, "params": {}}),
                        )
                        .await
                    }
                };
                self.reply(reply, result).await;
            }
            Command::Turn {
                thread_id,
                params,
                sink,
                messages,
            } => {
                let result = self
                    .ended
                    .as_ref()
                    .map(|ended| Err(ended.error(TURN_EXITED)));
                self.turn = Some(Turn {
                    thread_id,
                    turn_id: None,
                    state: CodexAppServerTurnState::default(),
                    events: Vec::new(),
                    messages,
                    sink,
                    idle_deadline: None,
                    stopping: false,
                    steering: false,
                    result,
                });
                if self.ended.is_none() {
                    self.request(io, "turn/start", Some(params), Call::TurnStart)
                        .await;
                }
            }
            Command::Next { wait, reply } => {
                self.waiter = Some((Instant::now() + wait, reply));
            }
        }
    }

    async fn line(&mut self, io: &mut Io, text: String) {
        if let Some(Err(error)) = self
            .trace
            .as_ref()
            .map(|trace| trace.line("incoming", text.clone()))
        {
            return self.end(io, Ended::Failed(error)).await;
        }
        let bytes = text.len();
        // Malformed lines are skipped.
        let Some(message) = parse_jsonrpc_line(&text) else {
            return;
        };
        drop(text);
        if message.get("id").is_some() && message.get("method").is_some() {
            let notification = self.server_request(io, message).await;
            self.notification(io, notification, bytes).await;
        } else if message.get("id").is_some() {
            self.response(message).await;
        } else if message.get("method").is_some() {
            self.notification(io, message, bytes).await;
        }
    }

    async fn notification(&mut self, io: &Io, message: Value, bytes: usize) {
        // A turn's backlog grows only across one turn/start or turn/steer
        // round trip, whose reply is read past it: it is never refused.
        if self.turn.is_some() {
            self.backlog.push_back((message, None));
            return;
        }
        match self.backlog_budget.try_admit(bytes) {
            Some(held) => self.backlog.push_back((message, Some(held))),
            None => {
                self.end(io, Ended::Failed(overloaded("its notification queue")))
                    .await
            }
        }
    }

    /// Answers what can be answered now, and returns the request shaped as a
    /// notification for the turn's events.
    async fn server_request(&mut self, io: &mut Io, message: Value) -> Value {
        let request_id = message.get("id").cloned().unwrap_or(Value::Null);
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        match method {
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                self.respond(
                    io,
                    json!({"jsonrpc": "2.0", "id": request_id, "result": {"decision": "acceptForSession"}}),
                )
                .await
            }
            // Answered once a person, through the workspace route, or the
            // flow's run record has an answer.
            "item/tool/requestUserInput" => self.ask(io, &message).await,
            _ => {
                self.respond(
                    io,
                    json!({
                        "jsonrpc": "2.0",
                        "id": request_id,
                        "error": {"code": -32000, "message": format!("Unsupported request: {method}")},
                    }),
                )
                .await
            }
        }
        json!({
            "jsonrpc": message.get("jsonrpc").cloned().unwrap_or_else(|| json!("2.0")),
            "method": method,
            "params": message.get("params").cloned().unwrap_or_else(|| json!({})),
        })
    }

    async fn ask(&mut self, io: &Io, message: &Value) {
        if self.clarification.is_none() {
            self.chat_question = Some(register_request_user_input_response(
                message,
                Arc::clone(&self.controls.wake),
            ));
            return;
        }
        if self.flow_questions.len() >= PENDING_LIMIT {
            return self
                .end(io, Ended::Failed(overloaded("its question list")))
                .await;
        }
        let request_id = message.get("id").cloned().unwrap_or(Value::Null);
        let mut params = message.get("params").cloned().unwrap_or_else(|| json!({}));
        params["agent_request_id"] = request_id.clone();
        self.next_question += 1;
        let key = self.next_question;
        self.flow_questions.push(FlowQuestion {
            key,
            request_id,
            poll: None,
        });
        let Some((handler, work)) = &self.clarification else {
            return;
        };
        let (handler, completions) = (Arc::clone(handler), self.completions.clone());
        let queued = work.run(json_len(&params), move || {
            let poll = handler(params);
            let _ = completions.send(Completion::Asked { key, poll });
        });
        if queued.is_err() {
            self.end(io, Ended::Failed(overloaded("its clarification queue")))
                .await;
        }
    }

    async fn response(&mut self, message: Value) {
        let id = message
            .get("id")
            .and_then(id_key)
            .and_then(|key| key.parse::<u64>().ok());
        let Some(pending) = id.and_then(|id| self.pending.remove(&id)) else {
            tracing::debug!(id = ?message.get("id"), "codex app-server sent an unexpected response");
            return;
        };
        match pending.call {
            Call::Facade(reply) => self.reply(reply, Ok(message)).await,
            Call::TurnStart => {
                let Some(turn) = self.turn.as_mut() else {
                    return;
                };
                if message.get("error").is_some() {
                    return turn.finish(Err(rpc_error(
                        "codex app-server turn/start failed",
                        &message,
                    )));
                }
                match message
                    .pointer("/result/turn/id")
                    .and_then(Value::as_str)
                    .and_then(non_empty)
                {
                    Some(turn_id) => {
                        turn.turn_id = Some(turn_id.to_string());
                        turn.idle_deadline = Some(Instant::now() + TURN_IDLE_TIMEOUT);
                    }
                    None => turn.finish(Err(CodexAppServerError::runtime(
                        "codex app-server did not return a turn id",
                    ))),
                }
            }
            Call::Steer(steer) => {
                let accepted = message.get("error").is_none();
                if !accepted {
                    let error = rpc_error("codex app-server turn/steer failed", &message);
                    tracing::warn!(error = %error.message, "codex app-server turn/steer failed");
                }
                if let Some(turn) = self.turn.as_mut() {
                    turn.steering = false;
                    if accepted {
                        let delivered = delivered_event(turn, steer.id, steer.text);
                        turn.sequence(delivered);
                    }
                }
                // Once sequenced; never waiting on the sink, which may be the caller.
                let _ = steer.accepted.send(accepted);
            }
            Call::Interrupt => {
                if message.get("error").is_some() {
                    tracing::warn!(%message, "codex app-server turn/interrupt failed");
                }
            }
        }
    }

    async fn completion(&mut self, io: &mut Io, completion: Completion) {
        match completion {
            Completion::Asked { key, poll } => {
                // A question that expired meanwhile stays expired.
                let Some(question) = self.flow_questions.iter_mut().find(|q| q.key == key) else {
                    return;
                };
                match poll {
                    Ok(poll) => {
                        question.poll = Some(poll);
                        self.flow_due.get_or_insert_with(Instant::now);
                    }
                    Err(error) => {
                        self.end(io, Ended::Failed(CodexAppServerError::runtime(error)))
                            .await
                    }
                }
            }
            Completion::Polled { key, poll, answers } => {
                let Some(index) = self.flow_questions.iter().position(|q| q.key == key) else {
                    return;
                };
                match answers {
                    Ok(Some(answers)) => {
                        let question = self.flow_questions.remove(index);
                        self.respond(
                            io,
                            json!({"jsonrpc": "2.0", "id": question.request_id, "result": request_user_input_response_payload(&answers)}),
                        )
                        .await;
                    }
                    Ok(None) => self.flow_questions[index].poll = Some(poll),
                    Err(error) => {
                        self.end(io, Ended::Failed(CodexAppServerError::runtime(error)))
                            .await
                    }
                }
            }
            Completion::TraceFailed(error) => self.end(io, Ended::Failed(error)).await,
        }
    }

    async fn status(&mut self, io: &Io, status: Status) {
        // Process death expires questions at once, though a descendant may
        // hold stdout open; a late answer cannot revive them.
        if status.exited.is_some() {
            self.chat_question = None;
            self.flow_questions.clear();
            for steer in self.stalled.drain(..) {
                let _ = steer.accepted.send(false);
            }
        }
        if status.write_failure.is_some() {
            self.end(
                io,
                Ended::Failed(CodexAppServerError::runtime(
                    "codex app-server stdin write failed",
                )),
            )
            .await;
        }
    }

    /// Rechecks every control and deadline; wakes carry no state.
    async fn advance(&mut self, io: &mut Io) {
        if self.controls.terminate.load(Ordering::SeqCst) {
            self.end(io, Ended::Terminated).await;
        }
        let now = Instant::now();
        let expired = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.deadline <= now)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for id in expired {
            let Some(pending) = self.pending.remove(&id) else {
                continue;
            };
            let error = CodexAppServerError::runtime(REQUEST_TIMED_OUT);
            match pending.call {
                // Its line may wait unread in the pipe, or half written, beyond
                // recall: the app-server is stopped, and the sender hears
                // false only once it has exited and can read nothing more.
                Call::Steer(steer) => {
                    tracing::warn!(error = %error.message, "codex app-server turn/steer failed");
                    self.stalled.push(steer);
                    self.end(io, Ended::Failed(error)).await;
                }
                call => self.settle(call, error).await,
            }
        }
        if self
            .waiter
            .as_ref()
            .is_some_and(|(deadline, _)| *deadline <= now)
        {
            if let Some((_, reply)) = self.waiter.take() {
                let _ = reply.send(Ok(None));
            }
        }
        if self.ended.is_none() {
            self.control_turn(io, now).await;
            self.answer_chat_question(io).await;
            self.poll_flow_questions(io, now).await;
            self.take_steers(io).await;
        }
        self.pump().await;
    }

    async fn control_turn(&mut self, io: &mut Io, now: Instant) {
        let Some(turn) = self.turn.as_mut().filter(|turn| turn.result.is_none()) else {
            return;
        };
        let Some(turn_id) = turn.turn_id.clone() else {
            return;
        };
        if !turn.stopping && self.controls.interrupt.load(Ordering::SeqCst) {
            turn.stopping = true;
            let params = json!({"threadId": turn.thread_id, "turnId": turn_id});
            self.request(io, "turn/interrupt", Some(params), Call::Interrupt)
                .await;
        }
        let asking = self.chat_question.is_some() || !self.flow_questions.is_empty();
        let Some(turn) = self.turn.as_mut().filter(|turn| turn.result.is_none()) else {
            return;
        };
        if turn.stopping {
            // A stopped turn leaves its question unanswered and takes no messages.
            self.chat_question = None;
            let messages = self
                .controls
                .messages
                .lock()
                .unwrap()
                .as_mut()
                .map(std::mem::take)
                .unwrap_or_default();
            for message in messages {
                let _ = message.accepted.send(false);
            }
        }
        if asking {
            turn.idle_deadline = None;
        } else if turn.idle_deadline.is_none() {
            turn.idle_deadline = Some(now + TURN_IDLE_TIMEOUT);
        }
        if turn.idle_deadline.is_some_and(|deadline| now >= deadline) {
            turn.finish(Err(CodexAppServerError::runtime(
                "codex app-server turn timed out waiting for activity",
            )));
        }
    }

    async fn answer_chat_question(&mut self, io: &mut Io) {
        let Some(question) = self.chat_question.as_ref() else {
            return;
        };
        let Some(answers) = question.pending.take_answers() else {
            return;
        };
        let id = question.jsonrpc_request_id.clone();
        self.chat_question = None;
        self.respond(
            io,
            json!({"jsonrpc": "2.0", "id": id, "result": request_user_input_response_payload(&answers)}),
        )
        .await;
    }

    /// ponytail: a flow question's answer and its run's status live in the
    /// run record and journal, which another process writes, so they are
    /// polled every 100 ms while one is pending, on their own schedule that
    /// output cannot delay. Removing this needs cross-process notification.
    async fn poll_flow_questions(&mut self, io: &Io, now: Instant) {
        if self.flow_questions.is_empty() {
            self.flow_due = None;
            return;
        }
        if self.flow_due.is_some_and(|due| now < due) {
            return;
        }
        // Not accumulated: a question whose poll still runs skips this round.
        self.flow_due = Some(now + FLOW_POLL);
        let Some((_, work)) = &self.clarification else {
            return;
        };
        let mut queued = Ok(());
        for question in &mut self.flow_questions {
            let Some(mut poll) = question.poll.take() else {
                continue;
            };
            let (key, completions) = (question.key, self.completions.clone());
            queued = work.run(0, move || {
                let answers = crate::codergen::ClarificationReply::polled_answers(poll());
                let _ = completions.send(Completion::Polled { key, poll, answers });
            });
            if queued.is_err() {
                break;
            }
        }
        if queued.is_err() {
            self.end(io, Ended::Failed(overloaded("its clarification queue")))
                .await;
        }
    }

    /// Sends queued messages through turn/steer one at a time while the turn
    /// runs, however far behind its sink is.
    async fn take_steers(&mut self, io: &mut Io) {
        loop {
            let Some(turn) = self.turn.as_mut() else {
                return;
            };
            if turn.result.is_some() || turn.stopping || turn.steering {
                return;
            }
            let Some(turn_id) = turn.turn_id.clone() else {
                return;
            };
            let Some(message) = self
                .controls
                .messages
                .lock()
                .unwrap()
                .as_mut()
                .and_then(VecDeque::pop_front)
            else {
                return;
            };
            turn.steering = true;
            let params = json!({
                "threadId": turn.thread_id,
                "expectedTurnId": turn_id,
                "input": [{"type": "text", "text": message.text}],
            });
            // A message that could not be written settled false; take the next.
            self.request(io, "turn/steer", Some(params), Call::Steer(message))
                .await;
        }
    }

    /// Hands the turn's caller its result once it has one; feeds
    /// notifications to the turn or a waiting caller.
    async fn pump(&mut self) {
        loop {
            if let Some(turn) = self.turn.as_mut() {
                if turn.result.is_some() {
                    let turn = self.turn.take().expect("turn");
                    // Its questions end with it, and so do its live messages,
                    // before its sink takes the rest: one it never took was
                    // not delivered, and a sink may be waiting on one.
                    self.chat_question = None;
                    self.flow_questions.clear();
                    self.controls.close();
                    let (messages, result) = (turn.messages, turn.result.expect("result"));
                    self.deliver(move |failure| {
                        let _ = messages.send(TurnMessage::Done(failure.map_or(result, Err)));
                    });
                    continue;
                }
                if turn.turn_id.is_none() || turn.steering {
                    return;
                }
                let Some((message, _held)) = self.backlog.pop_front() else {
                    return;
                };
                self.turn_message(message);
                continue;
            }
            let Some((deadline, reply)) = self.waiter.take() else {
                return;
            };
            let next = match (self.backlog.pop_front(), &self.ended) {
                (Some((message, _held)), _) => Ok(Some(message)),
                (None, Some(ended)) => Err(ended.clone()),
                (None, None) => {
                    self.waiter = Some((deadline, reply));
                    return;
                }
            };
            self.deliver(move |failure| {
                let _ = reply.send(failure.map_or(next, |error| Err(Ended::Failed(error))));
            });
        }
    }

    fn turn_message(&mut self, message: Value) {
        let Some(turn) = self.turn.as_mut() else {
            return;
        };
        let Some(expected_turn_id) = turn.turn_id.clone() else {
            return;
        };
        if turn.idle_deadline.is_some() {
            turn.idle_deadline = Some(Instant::now() + TURN_IDLE_TIMEOUT);
        }
        let extracted_turn_id = extract_turn_id(&message);
        if extracted_turn_id
            .as_deref()
            .is_some_and(|turn_id| turn_id != expected_turn_id)
        {
            return;
        }
        let mut normalized = process_codex_app_server_message(&message, &mut turn.state);
        for event in &mut normalized {
            if event.source.app_thread_id.is_none() {
                event.source.app_thread_id = Some(turn.thread_id.clone());
            }
            if event.source.app_turn_id.is_none() {
                event.source.app_turn_id = Some(expected_turn_id.clone());
            }
        }
        let completed = normalized
            .iter()
            .any(|event| event.kind == TurnStreamEventKind::TurnCompleted)
            && extracted_turn_id.as_deref() == Some(expected_turn_id.as_str());
        // A turn stopped on request ends normally with what it had said.
        if completed && turn.stopping && turn.state.turn_status.as_deref() == Some("interrupted") {
            turn.state.turn_error = None;
            turn.state.last_error = None;
            let (source, text) =
                crate::events::latest_assistant_text(&turn.events).unwrap_or_default();
            let item_id = source.item_id;
            let text = non_empty(&text).unwrap_or("Stopped.").to_string();
            turn.state.final_agent_message = Some(text.clone());
            let index = normalized
                .iter()
                .position(|event| event.kind == TurnStreamEventKind::TurnCompleted)
                .unwrap();
            normalized[index].error = None;
            let mut answer = event(TurnStreamEventKind::ContentCompleted, "turn_interrupted")
                .channel(TurnStreamChannel::Assistant)
                .content(Some(text))
                .item_id(item_id.or_else(|| Some(format!("{expected_turn_id}-stopped"))))
                .phase(Some("final_answer".to_string()))
                .build();
            answer.source.app_thread_id = Some(turn.thread_id.clone());
            answer.source.app_turn_id = Some(expected_turn_id.clone());
            normalized.insert(index, answer);
        }
        for event in normalized {
            turn.sequence(event);
        }
        if completed {
            let result = turn.outcome();
            turn.finish(result);
        }
    }

    /// Writes a request and registers who waits on its response.
    async fn request(&mut self, io: &mut Io, method: &str, params: Option<Value>, call: Call) {
        if self.pending.len() >= PENDING_LIMIT {
            let error = overloaded("its pending request list");
            self.end(io, Ended::Failed(error.clone())).await;
            return self.settle(call, error).await;
        }
        self.next_id += 1;
        let id = self.next_id;
        let mut payload = json!({"jsonrpc": "2.0", "id": id, "method": method});
        if let Some(params) = params {
            payload["params"] = params;
        }
        match self.write(io, payload).await {
            // The deadline starts once the request is taken, writer progress included.
            Ok(()) => {
                self.pending.insert(
                    id,
                    Pending {
                        deadline: Instant::now() + REQUEST_TIMEOUT,
                        call,
                    },
                );
            }
            Err(error) => self.settle(call, error).await,
        }
    }

    /// Answers a server request; failing to is fatal.
    async fn respond(&mut self, io: &mut Io, payload: Value) {
        if let Err(error) = self.write(io, payload).await {
            self.end(io, Ended::Failed(error)).await;
        }
    }

    async fn write(&mut self, io: &Io, payload: Value) -> Result<(), CodexAppServerError> {
        let line = serde_json::to_string(&payload).map_err(|error| {
            CodexAppServerError::runtime(format!("codex app-server request encode failed: {error}"))
        })?;
        let traced = self.trace.is_some().then(|| line.clone());
        let refused = match io.stdin.send(line) {
            Ok(_written) => {
                let Some((trace, line)) = self.trace.as_ref().zip(traced) else {
                    return Ok(());
                };
                let Err(error) = trace.line("outgoing", line) else {
                    return Ok(());
                };
                self.end(io, Ended::Failed(error.clone())).await;
                return Err(error);
            }
            Err(refused) => refused,
        };
        let error = match refused {
            Refused::TooLarge => {
                // Nothing was written, so the connection goes on.
                return Err(CodexAppServerError::runtime(format!(
                    "codex app-server message exceeds the {} MiB line limit",
                    LIMITS.frame_bytes >> 20
                )));
            }
            Refused::Full => overloaded("its stdin queue"),
            Refused::Closed => CodexAppServerError::runtime("codex app-server stdin write failed"),
        };
        self.end(io, Ended::Failed(error.clone())).await;
        Err(error)
    }

    /// Settles a request that will get no response.
    async fn settle(&mut self, call: Call, error: CodexAppServerError) {
        match call {
            // Not behind the trace: storage cannot hold up a timeout or an end.
            Call::Facade(reply) => {
                let _ = reply.send(Err(error));
            }
            Call::TurnStart | Call::Interrupt => {
                if let Some(turn) = self.turn.as_mut() {
                    turn.finish(Err(error));
                }
            }
            Call::Steer(steer) => {
                tracing::warn!(error = %error.message, "codex app-server turn/steer failed");
                if let Some(turn) = self.turn.as_mut() {
                    turn.steering = false;
                }
                let _ = steer.accepted.send(false);
            }
        }
    }

    /// The one terminal transition: takes no more work, expires questions,
    /// settles every waiting caller once and stops the app-server.
    async fn end(&mut self, io: &Io, cause: Ended) {
        if self.ended.is_some() {
            return;
        }
        // A forced stop kills the app-server, so whatever followed is that stop.
        let cause = if self.controls.terminate.load(Ordering::SeqCst) {
            Ended::Terminated
        } else {
            cause
        };
        self.ended = Some(cause.clone());
        io.stop.stop();
        self.controls.close();
        let failure = self.trace.as_ref().and_then(Trace::failure);
        for fenced in std::mem::take(&mut self.fenced) {
            let deliver = fenced.upgrade().and_then(|fenced| lock(&fenced).take());
            if let Some(deliver) = deliver {
                deliver(failure.clone());
            }
        }
        self.chat_question = None;
        self.flow_questions.clear();
        for (_, pending) in std::mem::take(&mut self.pending) {
            self.settle(pending.call, cause.error(REQUEST_EXITED)).await;
        }
        // A turn that already ended keeps its result.
        if let Some(turn) = self.turn.as_mut() {
            turn.finish(Err(cause.error(TURN_EXITED)));
        }
    }

    async fn reply<T: Send + 'static>(
        &mut self,
        reply: Reply<T>,
        result: Result<T, CodexAppServerError>,
    ) {
        self.deliver(move |failure| {
            let _ = reply.send(failure.map_or(result, Err));
        });
    }

    /// Runs `deliver` once every trace line before it is written, with the
    /// trace's failure if it has failed; at once if the connection has
    /// ended, or ends, first, so blocked storage holds up no ended caller.
    fn deliver(&mut self, deliver: impl FnOnce(Option<CodexAppServerError>) + Send + 'static) {
        let Some(trace) = &self.trace else {
            return deliver(None);
        };
        if self.ended.is_some() {
            return deliver(trace.failure());
        }
        let fenced: Arc<Fenced> = Arc::new(Mutex::new(Some(Box::new(deliver))));
        let (queued, failure) = (Arc::clone(&fenced), Arc::clone(&trace.failure));
        let accepted = trace.work.run(0, move || {
            let failure = lock(&failure).clone();
            let deliver = lock(&queued).take();
            if let Some(deliver) = deliver {
                deliver(failure);
            }
        });
        match accepted {
            Ok(()) => {
                self.fenced.retain(|fenced| fenced.strong_count() > 0);
                self.fenced.push(Arc::downgrade(&fenced));
            }
            // The trace has failed, and its completion ends the connection.
            Err(_) => {
                let error = trace.overflowed();
                let _ = self
                    .completions
                    .send(Completion::TraceFailed(error.clone()));
                let deliver = lock(&fenced).take();
                if let Some(deliver) = deliver {
                    deliver(Some(error));
                }
            }
        }
    }
}

/// The worker's ends of the app-server's stdin and stop.
struct Io {
    stdin: Outgoing,
    stop: Stop,
}

fn delivered_event(turn: &Turn, message_id: String, text: String) -> TurnStreamEvent {
    TurnStreamEvent {
        kind: TurnStreamEventKind::Other("user_message_delivered".to_string()),
        channel: None,
        source: TurnStreamSource {
            backend: Some(CODEX_APP_SERVER_BACKEND.to_string()),
            app_thread_id: Some(turn.thread_id.clone()),
            app_turn_id: turn.turn_id.clone(),
            item_id: Some(message_id),
            raw_kind: Some("turn_steer".to_string()),
            ..TurnStreamSource::default()
        },
        content_delta: None,
        message: Some(text),
        tool_call: None,
        request_user_input: None,
        token_usage: None,
        error: None,
        error_code: None,
        details: None,
        phase: None,
        status: Some("delivered".to_string()),
    }
}

#[derive(Debug)]
struct PendingRequestUserInputResponse {
    jsonrpc_request_id: Value,
    pending: Arc<PendingRequestUserInput>,
}

impl Drop for PendingRequestUserInputResponse {
    fn drop(&mut self) {
        unregister_request_user_input(&self.pending);
    }
}

#[derive(Debug)]
struct PendingRequestUserInput {
    request_id: String,
    app_thread_id: Option<String>,
    app_turn_id: Option<String>,
    aliases: Vec<String>,
    state: Mutex<PendingRequestUserInputState>,
    /// The asking client's worker, woken once an answer is stored.
    wake: Arc<Notify>,
}

#[derive(Debug, Default)]
struct PendingRequestUserInputState {
    answers: Option<BTreeMap<String, String>>,
}

impl PendingRequestUserInput {
    fn take_answers(&self) -> Option<BTreeMap<String, String>> {
        self.state.lock().ok()?.answers.take()
    }

    fn deliver_answers(&self, answers: BTreeMap<String, String>) {
        if let Ok(mut state) = self.state.lock() {
            state.answers = Some(answers);
        }
        self.wake.notify_one();
    }
}

/// A queued turn/steer message, holding its share of the steer queue.
struct SteerMessage {
    id: String,
    text: String,
    accepted: mpsc::Sender<bool>,
    _held: Held,
}

/// A turn's controls, shared with `CodexAppServerBackend`'s live calls.
struct TurnStop {
    interrupt: AtomicBool,
    terminate: AtomicBool,
    /// Messages for turn/steer. `None` once the turn has ended.
    messages: Mutex<Option<VecDeque<SteerMessage>>>,
    steer_budget: Budget,
    /// Makes the worker recheck these controls; carries no state.
    wake: Arc<Notify>,
    /// Kills the app-server directly, past its worker and queues.
    connection: Mutex<Option<Stop>>,
}

impl TurnStop {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            interrupt: AtomicBool::new(false),
            terminate: AtomicBool::new(false),
            messages: Mutex::new(Some(VecDeque::new())),
            steer_budget: Budget::new(&LIMITS),
            wake: Arc::new(Notify::new()),
            connection: Mutex::new(None),
        })
    }

    fn attach(&self, stop: Stop) {
        let mut connection = self.connection.lock().unwrap();
        if self.terminate.load(Ordering::SeqCst) {
            stop.stop();
        }
        *connection = Some(stop);
    }

    fn terminate(&self) {
        let connection = self.connection.lock().unwrap();
        self.terminate.store(true, Ordering::SeqCst);
        if let Some(stop) = connection.as_ref() {
            stop.stop();
        }
        drop(connection);
        self.wake.notify_one();
    }

    /// Takes no more messages; one the turn never took was not delivered.
    fn close(&self) {
        let untaken = self
            .messages
            .lock()
            .ok()
            .and_then(|mut messages| messages.take());
        for message in untaken.unwrap_or_default() {
            let _ = message.accepted.send(false);
        }
    }
}

type InterruptRegistry = Mutex<BTreeMap<(String, String), Weak<TurnStop>>>;
fn interrupt_registry() -> &'static InterruptRegistry {
    static REGISTRY: OnceLock<InterruptRegistry> = OnceLock::new();
    REGISTRY.get_or_init(Mutex::default)
}

fn request_user_input_registry() -> &'static Mutex<BTreeMap<String, Arc<PendingRequestUserInput>>> {
    static REGISTRY: OnceLock<Mutex<BTreeMap<String, Arc<PendingRequestUserInput>>>> =
        OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn register_request_user_input_response(
    message: &Value,
    wake: Arc<Notify>,
) -> PendingRequestUserInputResponse {
    let params = message
        .get("params")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let jsonrpc_request_id = message.get("id").cloned().unwrap_or(Value::Null);
    let request_id = request_user_input_request_id(&params)
        .or_else(|| {
            jsonrpc_request_id
                .as_str()
                .and_then(non_empty)
                .map(str::to_string)
        })
        .or_else(|| jsonrpc_request_id.as_u64().map(|id| id.to_string()))
        .unwrap_or_else(|| "request-user-input".to_string());
    let app_thread_id = object_text(&params, &["threadId", "thread_id"]);
    let app_turn_id = object_text(&params, &["turnId", "turn_id"]);
    let aliases = request_user_input_aliases(&request_id, &params);
    let pending = Arc::new(PendingRequestUserInput {
        request_id,
        app_thread_id,
        app_turn_id,
        aliases,
        state: Mutex::new(PendingRequestUserInputState::default()),
        wake,
    });
    if let Ok(mut registry) = request_user_input_registry().lock() {
        for key in request_user_input_registry_keys(&pending) {
            registry.insert(key, Arc::clone(&pending));
        }
    }
    PendingRequestUserInputResponse {
        jsonrpc_request_id,
        pending,
    }
}

fn unregister_request_user_input(pending: &Arc<PendingRequestUserInput>) {
    if let Ok(mut registry) = request_user_input_registry().lock() {
        registry.retain(|_, value| !Arc::ptr_eq(value, pending));
    }
}

fn request_user_input_request_id(params: &Map<String, Value>) -> Option<String> {
    params
        .get("request_id")
        .or_else(|| params.get("requestId"))
        .or_else(|| params.get("itemId"))
        .and_then(Value::as_str)
        .and_then(non_empty)
        .map(str::to_string)
}

fn request_user_input_aliases(request_id: &str, params: &Map<String, Value>) -> Vec<String> {
    let mut aliases = Vec::new();
    push_unique_non_empty(&mut aliases, request_id);
    if let Some(item_id) = params
        .get("itemId")
        .and_then(Value::as_str)
        .and_then(non_empty)
    {
        push_unique_non_empty(&mut aliases, item_id);
    }
    if let Some(questions) = params.get("questions").and_then(Value::as_array) {
        for question in questions {
            if let Some(id) = question
                .get("id")
                .and_then(Value::as_str)
                .and_then(non_empty)
            {
                push_unique_non_empty(&mut aliases, id);
            }
        }
    }
    aliases
}

fn push_unique_non_empty(values: &mut Vec<String>, value: &str) {
    let Some(value) = non_empty(value) else {
        return;
    };
    if !values.iter().any(|existing| existing == &value) {
        values.push(value.to_string());
    }
}

fn request_user_input_registry_keys(pending: &PendingRequestUserInput) -> Vec<String> {
    pending
        .aliases
        .iter()
        .flat_map(|alias| {
            [
                scoped_request_user_input_key(
                    pending.app_thread_id.as_deref(),
                    pending.app_turn_id.as_deref(),
                    alias,
                ),
                alias_request_user_input_key(alias),
            ]
        })
        .collect()
}

fn scoped_request_user_input_key(
    app_thread_id: Option<&str>,
    app_turn_id: Option<&str>,
    alias: &str,
) -> String {
    format!(
        "{}\u{0}{}\u{0}{}",
        app_thread_id.unwrap_or(""),
        app_turn_id.unwrap_or(""),
        alias
    )
}

fn alias_request_user_input_key(alias: &str) -> String {
    format!("\u{0}\u{0}{alias}")
}

fn find_pending_request_user_input(
    request: &AgentRequestUserInputAnswerRequest,
) -> Option<Arc<PendingRequestUserInput>> {
    let alias = non_empty(&request.request_id)?;
    let app_thread_id = metadata_string(
        &request.metadata,
        &[
            "spark.runtime.codex_app_server.thread_id",
            "codex_app_server.thread_id",
            "codex_thread_id",
        ],
    );
    let app_turn_id = metadata_string(
        &request.metadata,
        &[
            "spark.runtime.codex_app_server.turn_id",
            "codex_app_server.turn_id",
            "codex_turn_id",
        ],
    )
    .or_else(|| non_empty(&request.assistant_turn_id).map(str::to_string));
    let keys = [
        scoped_request_user_input_key(app_thread_id.as_deref(), app_turn_id.as_deref(), alias),
        alias_request_user_input_key(alias),
    ];
    let registry = request_user_input_registry().lock().ok()?;
    keys.iter().find_map(|key| registry.get(key).cloned())
}

fn request_user_input_response_payload(answers: &BTreeMap<String, String>) -> Value {
    let answers = answers
        .iter()
        .map(|(question_id, answer)| {
            (
                question_id.clone(),
                json!({
                    "answers": [answer],
                }),
            )
        })
        .collect::<Map<_, _>>();
    json!({ "answers": answers })
}

fn request_user_input_answer_payload(
    request: &AgentRequestUserInputAnswerRequest,
) -> Result<BTreeMap<String, String>, AgentThreadResumeFailure> {
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
    Ok(answers)
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
        .or_else(|| object.get("requestId"))
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
                backend: Some(CODEX_APP_SERVER_BACKEND.to_string()),
                item_id: non_empty(request_id).map(str::to_string),
                raw_kind: Some("request_user_input_resume_failure".to_string()),
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
            phase: Some("request_user_input_answer".to_string()),
            status: Some("failed".to_string()),
        }],
        thread_resume_failure: Some(failure),
        ..AgentTurnOutput::default()
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
                .tool(command_execution_tool_call_payload(&params, "running"))
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
                .tool(file_change_tool_call_payload(&params, "running"))
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
                if is_reasoning_item(item) {
                    events.push(codex_thinking_event(true));
                } else if is_context_compaction_item(item) {
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
                            .tool(tool_call_payload_from_codex_item(item, "running"))
                            .item_id(object_text(item, &["id"]))
                            .build(),
                    );
                }
            }
        }
        "item/completed" => {
            if let Some(item) = params.get("item").and_then(Value::as_object) {
                let item_id = object_text(item, &["id"]);
                if is_reasoning_item(item) {
                    events.push(codex_thinking_event(false));
                    if let Some(text) = reasoning_raw_text_from_item(item) {
                        events.push(
                            event(TurnStreamEventKind::ContentCompleted, "reasoning_completed")
                                .channel(TurnStreamChannel::Reasoning)
                                .content(Some(text))
                                .item_id(item_id.as_deref().map(raw_reasoning_item_id))
                                .build(),
                        );
                    }
                }
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
                } else if let Some(parts) = reasoning_summary_parts_from_item(item) {
                    state.reasoning_summary_buffer.clear();
                    for (summary_index, text) in parts.into_iter().enumerate() {
                        events.push(
                            event(TurnStreamEventKind::ContentCompleted, "reasoning_completed")
                                .channel(TurnStreamChannel::Reasoning)
                                .content(Some(text))
                                .item_id(item_id.clone())
                                .summary_index(Some(summary_index as u64))
                                .build(),
                        );
                    }
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
                    let tool_call = tool_call_payload_from_codex_item(item, "completed");
                    // A failed or declined item ends as a failed call, matching its status.
                    let kind = if tool_call["status"] == "failed" {
                        TurnStreamEventKind::ToolCallFailed
                    } else {
                        TurnStreamEventKind::ToolCallCompleted
                    };
                    events.push(
                        event(kind, "tool_item_completed")
                            .tool(tool_call)
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
        "item/reasoning/textDelta" => {
            if let Some(delta) = object_text(&params, &["delta"]) {
                events.push(
                    event(TurnStreamEventKind::ContentDelta, "reasoning_delta")
                        .channel(TurnStreamChannel::Reasoning)
                        .content(Some(delta))
                        .item_id(
                            object_text(&params, &["itemId"])
                                .as_deref()
                                .map(raw_reasoning_item_id),
                        )
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
            if let Some(mut token_usage) = params.get("tokenUsage").cloned() {
                // The latest request's input is what the context holds now.
                if let Some(window) = token_usage["modelContextWindow"].as_u64() {
                    let mut context = json!({"window_tokens": window});
                    if let Some(used) = token_usage
                        .pointer("/last/inputTokens")
                        .and_then(Value::as_u64)
                    {
                        context["used_tokens"] = json!(used);
                    }
                    token_usage["context"] = context;
                }
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
                .and_then(auth::error_message)
                .or_else(|| object_text(&params, &["message"]))
                .unwrap_or_else(|| "codex app-server error".to_string());
            // Codex retries this one itself; the turn goes on.
            if params.get("willRetry").and_then(Value::as_bool) == Some(true) {
                events.push(
                    event(
                        TurnStreamEventKind::Other("warning".into()),
                        "error_will_retry",
                    )
                    .content(Some(format!("{error}; retrying")))
                    .build(),
                );
                return events;
            }
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
                    .and_then(auth::error_message)
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
        self.error = Some(auth::normalize_auth_error(error));
        self
    }

    fn error_opt(mut self, error: Option<String>) -> Self {
        self.error = error.map(auth::normalize_auth_error);
        self
    }

    fn token_usage(mut self, token_usage: Value) -> Self {
        self.token_usage = Some(token_usage);
        self
    }

    fn build(self) -> TurnStreamEvent {
        let error_code = self
            .error
            .as_deref()
            .filter(|error| auth::requires_login(error))
            .map(|_| "codex_auth_required".to_string());
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
            error_code,
            details: None,
            phase: self.phase,
            status: self.status,
        }
    }
}

pub fn build_codex_runtime_environment() -> Result<BTreeMap<String, OsString>, CodexAppServerError>
{
    build_codex_runtime_environment_with_settings(None)
}

pub(crate) fn build_codex_runtime_environment_with_settings(
    native: Option<&spark_common::agent_settings::NativeAgentSettings>,
) -> Result<BTreeMap<String, OsString>, CodexAppServerError> {
    let mut env_map = process_environment();
    if let Some(native) = native {
        for (key, value) in [
            (CODEX_RUNTIME_ROOT_ENV, &native.codex_runtime_root),
            (CODEX_SEED_DIR_ENV, &native.codex_seed_dir),
        ] {
            env_map.remove(key);
            if let Some(value) = value {
                env_map.insert(key.into(), value.into());
            }
        }
    }
    let existing_path_key = env_map
        .keys()
        .find(|key| crate::local_environment::is_path_key(key))
        .cloned();
    let path = existing_path_key
        .as_deref()
        .and_then(|key| env_map.remove(key));
    drop_managed_key_aliases(&mut env_map);
    env_map.extend(spark_agent_environment(
        path.as_deref().and_then(OsStr::to_str),
    ));
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

    // Only an explicit seed dir seeds Spark's Codex home; the user's own
    // CODEX_HOME or ~/.codex never reaches it.
    let seed = env_map
        .get(CODEX_SEED_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/codex-seed"));
    // OAuth credentials must come from a login in this home. Even a one-time
    // copy shares the host's refresh token and can invalidate either session.
    if seed != codex_home {
        let _ = copy_file_if_changed(&seed.join("config.toml"), &codex_home.join("config.toml"));
        let _ = copy_tree_contents(
            &seed.join("plugins").join("cache"),
            &codex_home.join("plugins").join("cache"),
        );
    }
    force_standard_service_tier(&codex_home.join("config.toml")).map_err(|error| {
        CodexAppServerError::configuration(format!(
            "codex runtime config could not default to the standard service tier: {error}"
        ))
    })?;

    // On Windows these replace an inherited spelling such as `UserProfile`,
    // which would otherwise reach the child after them and win.
    crate::local_environment::merge_env(
        &mut env_map,
        [
            ("HOME", runtime_root.clone()),
            ("USERPROFILE", runtime_root),
            ("CODEX_HOME", codex_home),
            ("XDG_CONFIG_HOME", xdg_config_home),
            ("XDG_DATA_HOME", xdg_data_home),
        ]
        .map(|(key, path)| (key.to_string(), path.into_os_string())),
        cfg!(windows),
    );
    Ok(env_map)
}

/// This process's environment with every value byte for byte, such as a
/// non-UTF-8 HOME. A variable whose name is not UTF-8 is left out.
pub(crate) fn process_environment() -> BTreeMap<String, OsString> {
    env::vars_os()
        .filter_map(|(key, value)| Some((key.into_string().ok()?, value)))
        .collect()
}

fn runtime_root(env_map: &BTreeMap<String, OsString>) -> PathBuf {
    AgentEnvironment(env_map)
        .get_var_os(CODEX_RUNTIME_ROOT_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| spark_runtime_dir(env_map, "codex"))
}

/// An agent's own home under Spark's: `<SPARK_HOME>/runtime/<agent>`, else
/// `<home>/.spark/runtime/<agent>`, else `.spark/runtime/<agent>`.
pub(crate) fn spark_runtime_dir(env_map: &BTreeMap<String, OsString>, agent: &str) -> PathBuf {
    let env = AgentEnvironment(env_map);
    env.get_var_os("SPARK_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            spark_common::paths::user_home_dir_with_env(&env).map(|home| home.join(".spark"))
        })
        .unwrap_or_else(|| PathBuf::from(".spark"))
        .join("runtime")
        .join(agent)
}

/// An agent's environment map, in which a blank value counts as unset.
struct AgentEnvironment<'a>(&'a BTreeMap<String, OsString>);

impl Environment for AgentEnvironment<'_> {
    fn get_var(&self, key: &str) -> Option<String> {
        self.get_var_os(key)?.into_string().ok()
    }

    fn get_var_os(&self, key: &str) -> Option<OsString> {
        let value = self.0.get(key)?;
        (!value.to_string_lossy().trim().is_empty()).then(|| value.clone())
    }
}

/// A model the local codex install can actually serve, per `model/list`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CodexModelMetadata {
    pub id: String,
    pub display: String,
    pub is_default: bool,
    pub supported_reasoning_efforts: Vec<String>,
    pub default_reasoning_effort: Option<String>,
}

/// Ask the local codex app-server which models it serves. Spawns a short
/// lived app-server process; callers cache the result.
pub fn list_available_codex_models() -> Result<Vec<CodexModelMetadata>, CodexAppServerError> {
    list_available_codex_models_with_settings(None)
}

pub fn list_available_codex_models_with_settings(
    native: Option<&spark_common::agent_settings::NativeAgentSettings>,
) -> Result<Vec<CodexModelMetadata>, CodexAppServerError> {
    let working_dir = env::current_dir().unwrap_or_else(|_| env::temp_dir());
    let mut client = CodexAppServerClient::connect_with_settings(
        working_dir,
        None,
        native,
        BTreeMap::new(),
        None,
        None,
    )
    .map_err(log_model_discovery_error)?;
    let result = client
        .list_models()
        .map_err(|error| log_model_discovery_error(error))?;
    Ok(codex_models_from_list_result(&result))
}

fn log_model_discovery_error(error: CodexAppServerError) -> CodexAppServerError {
    tracing::warn!(
        error = %error,
        details = ?error.details,
        "Codex model discovery failed"
    );
    error
}

/// Parse a `model/list` result across the payload dialects codex has used:
/// entries under `data` or `models`, camelCase or snake_case fields, and
/// reasoning efforts as plain strings or `{reasoningEffort, description}`
/// objects.
pub fn codex_models_from_list_result(result: &Value) -> Vec<CodexModelMetadata> {
    let entries = result
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| result.get("models").and_then(Value::as_array));
    let Some(entries) = entries else {
        return Vec::new();
    };
    let mut models = Vec::new();
    for entry in entries {
        let Some(id) = first_string(entry, &["id", "model", "name"]) else {
            continue;
        };
        let display = first_string(
            entry,
            &["display", "displayName", "display_name", "label", "name"],
        )
        .unwrap_or_else(|| id.clone());
        let is_default = entry
            .get("isDefault")
            .or_else(|| entry.get("is_default"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let reasoning = entry.get("reasoning").cloned().unwrap_or(Value::Null);
        let efforts_value = first_value(
            entry,
            &[
                "supportedReasoningEfforts",
                "supported_reasoning_efforts",
                "reasoningEfforts",
                "reasoning_efforts",
            ],
        )
        .or_else(|| {
            first_value(
                &reasoning,
                &["supportedEfforts", "supported_efforts", "efforts"],
            )
        });
        let supported_reasoning_efforts = efforts_value
            .as_ref()
            .and_then(|value| value.as_array())
            .map(|items| {
                let mut efforts = Vec::new();
                for item in items {
                    let Some(effort) = reasoning_effort_from_value(item) else {
                        continue;
                    };
                    if !efforts.contains(&effort) {
                        efforts.push(effort);
                    }
                }
                efforts
            })
            .unwrap_or_default();
        let default_reasoning_effort = first_value(
            entry,
            &[
                "defaultReasoningEffort",
                "default_reasoning_effort",
                "reasoningEffort",
                "reasoning_effort",
            ],
        )
        .or_else(|| first_value(&reasoning, &["defaultEffort", "default_effort", "default"]))
        .as_ref()
        .and_then(reasoning_effort_from_value);
        models.push(CodexModelMetadata {
            id,
            display,
            is_default,
            supported_reasoning_efforts,
            default_reasoning_effort,
        });
    }
    models
}

fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    })
}

fn first_value(value: &Value, keys: &[&str]) -> Option<Value> {
    keys.iter().find_map(|key| value.get(*key).cloned())
}

fn reasoning_effort_from_value(value: &Value) -> Option<String> {
    let text = match value {
        Value::String(text) => Some(text.as_str()),
        Value::Object(_) => value
            .get("reasoningEffort")
            .or_else(|| value.get("reasoning_effort"))
            .or_else(|| value.get("effort"))
            .or_else(|| value.get("id"))
            .or_else(|| value.get("value"))
            .and_then(Value::as_str),
        _ => None,
    }?;
    let normalized = text.trim().to_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

pub(crate) fn codex_executable() -> PathBuf {
    let explicit = env::var("SPARK_CODEX_APP_SERVER_BIN")
        .ok()
        .and_then(|value| non_empty(&value).map(str::to_string));
    let path = env::var_os("PATH");
    resolve_codex_executable(
        explicit.as_deref(),
        path.as_deref(),
        &bundled_codex_candidates(),
    )
}

fn resolve_codex_executable(
    explicit: Option<&str>,
    path: Option<&OsStr>,
    bundled_candidates: &[PathBuf],
) -> PathBuf {
    if let Some(explicit) = explicit.and_then(non_empty) {
        return PathBuf::from(explicit);
    }
    if let Some(codex) = find_complete_codex_on_path(path) {
        return codex;
    }
    if let Some(codex) = bundled_candidates
        .iter()
        .find_map(|candidate| complete_codex_candidate(candidate))
    {
        return codex;
    }
    // An npm install on Windows has only shims, which a launch of `codex.exe`
    // never finds.
    if cfg!(windows) {
        if let Some(shim) = find_codex_shim(path, &crate::local_environment::own_pathext()) {
            return shim;
        }
    }
    PathBuf::from(codex_binary_name())
}

/// The first `codex.bat` or `codex.cmd` on `path`, directory by directory in
/// `pathext`'s order, as a bare `codex` finds it past an incomplete `codex.exe`.
fn find_codex_shim(path: Option<&OsStr>, pathext: &OsStr) -> Option<PathBuf> {
    let pathext = pathext.to_string_lossy();
    let shims = pathext
        .split(';')
        .filter(|extension| {
            [".bat", ".cmd"]
                .iter()
                .any(|batch| extension.eq_ignore_ascii_case(batch))
        })
        .map(|extension| format!("codex{extension}"))
        .collect::<Vec<_>>();
    crate::local_environment::path_directories(path?).find_map(|directory| {
        shims
            .iter()
            .map(|shim| directory.join(shim))
            .find(|shim| shim.is_file())
            .map(crate::local_environment::launched)
    })
}

/// The Codex program every launch runs: the configured one, else
/// [`codex_executable`], named as discovery finds it, so a bare `codex` from
/// Settings or `SPARK_CODEX_APP_SERVER_BIN` runs a Windows `codex.cmd` shim.
pub(crate) fn codex_program(
    native: Option<&spark_common::agent_settings::NativeAgentSettings>,
) -> PathBuf {
    crate::local_environment::launch_program(
        native
            .and_then(|config| config.codex_binary.as_ref())
            .map(PathBuf::from)
            .unwrap_or_else(codex_executable),
    )
}

fn find_complete_codex_on_path(path: Option<&OsStr>) -> Option<PathBuf> {
    crate::local_environment::path_directories(path?)
        .map(|directory| directory.join(codex_binary_name()))
        .find_map(|candidate| complete_codex_candidate(&candidate))
}

fn complete_codex_candidate(candidate: &Path) -> Option<PathBuf> {
    if has_code_mode_host(candidate) {
        return Some(candidate.to_path_buf());
    }
    let canonical = spark_common::paths::canonicalize_clean(candidate).ok()?;
    has_code_mode_host(&canonical).then_some(canonical)
}

fn has_code_mode_host(codex: &Path) -> bool {
    codex.is_file()
        && codex
            .parent()
            .map(|parent| parent.join(code_mode_host_binary_name()).is_file())
            .unwrap_or(false)
}

fn codex_binary_name() -> &'static str {
    if cfg!(windows) {
        "codex.exe"
    } else {
        "codex"
    }
}

fn code_mode_host_binary_name() -> &'static str {
    if cfg!(windows) {
        "codex-code-mode-host.exe"
    } else {
        "codex-code-mode-host"
    }
}

fn bundled_codex_candidates() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let relative_paths = [
            "Codex.app/Contents/Resources/codex",
            "ChatGPT.app/Contents/Resources/codex",
        ];
        let mut candidates = relative_paths
            .iter()
            .map(|relative| PathBuf::from("/Applications").join(relative))
            .collect::<Vec<_>>();
        if let Some(home) = env::var_os("HOME").filter(|value| !value.is_empty()) {
            let applications = PathBuf::from(home).join("Applications");
            candidates.extend(
                relative_paths
                    .iter()
                    .map(|relative| applications.join(relative)),
            );
        }
        candidates
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
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

fn force_standard_service_tier(path: &Path) -> std::io::Result<()> {
    let contents = fs::read_to_string(path).unwrap_or_default();
    let mut found = false;
    let mut normalized = Vec::new();
    let mut in_top_level = true;

    for line in contents.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            in_top_level = false;
        }
        if in_top_level
            && trimmed
                .split_once('=')
                .is_some_and(|(key, _)| key.trim() == "service_tier")
        {
            normalized.push(r#"service_tier = "standard""#);
            found = true;
        } else {
            normalized.push(line);
        }
    }
    if !found {
        normalized.insert(0, r#"service_tier = "standard""#);
    }

    let mut output = normalized.join("\n");
    output.push('\n');
    if output != contents {
        fs::write(path, output)?;
    }
    Ok(())
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

/// What every agent shell needs for the `spark` CLI to reach this Spark: its
/// home, its API, and the app's own binaries (the CLI among them) first on
/// PATH. Codex and Claude Code sessions get the same, as do flow tool commands.
pub fn spark_agent_environment(path: Option<&str>) -> Vec<(String, OsString)> {
    let mut vars = Vec::new();
    if let Some(spark_home) = CODEX_SPARK_HOME.get() {
        vars.push(("SPARK_HOME".to_string(), spark_home.into()));
    }
    if let Some(api) = AGENT_API.get() {
        vars.push((
            "SPARK_API_BASE_URL".to_string(),
            api.base_url.clone().into(),
        ));
        let origin = api.token.as_ref().map(|_| api.base_url.clone());
        for (key, value) in MANAGED_AGENT_API_KEYS.into_iter().zip([
            api.token.clone(),
            origin,
            api.loopback_url.clone(),
        ]) {
            vars.push((key.to_string(), value.unwrap_or_default().into()));
        }
    }
    // Each tool directory is one entry, quoted on Windows when its name holds
    // `;`, and is left off when no entry can name it. `path` follows as given,
    // so an empty entry still names the launch's own directory on Unix.
    let tools = env::join_paths(first_party_tool_bin_dirs())
        .ok()
        .filter(|tools| !tools.is_empty());
    let path = match (tools, path) {
        (Some(mut tools), Some(path)) => {
            tools.push(path_separator());
            tools.push(path);
            Some(tools)
        }
        (tools, path) => tools.or_else(|| path.map(OsString::from)),
    };
    vars.extend(path.map(|path| ("PATH".to_string(), path)));
    vars
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
        let normalized = spark_common::paths::canonicalize_clean(&candidate).unwrap_or(candidate);
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

fn tool_call_payload_from_codex_item(item: &Map<String, Value>, default_status: &str) -> Value {
    match object_text(item, &["type"]).as_deref() {
        Some("commandExecution") => command_execution_tool_call_payload(item, default_status),
        Some("fileChange") => file_change_tool_call_payload(item, default_status),
        Some("collabAgentToolCall") => subagent_tool_call_payload(item, default_status),
        _ => Value::Object(item.clone()),
    }
}

fn command_execution_tool_call_payload(
    payload: &Map<String, Value>,
    default_status: &str,
) -> Value {
    let mut tool_call = Map::new();
    tool_call.insert(
        "id".to_string(),
        Value::String(
            object_text(payload, &["id"])
                .or_else(|| object_text(payload, &["itemId"]))
                .unwrap_or_else(|| "command_execution".to_string()),
        ),
    );
    tool_call.insert(
        "kind".to_string(),
        Value::String("command_execution".to_string()),
    );
    tool_call.insert(
        "status".to_string(),
        Value::String(normalize_tool_status(payload, default_status)),
    );
    tool_call.insert(
        "title".to_string(),
        Value::String("Run command".to_string()),
    );
    if let Some(command) = extract_command_text(payload) {
        tool_call.insert("command".to_string(), Value::String(command));
    }
    tool_call.insert(
        "output".to_string(),
        object_text(payload, &["aggregatedOutput", "aggregated_output"])
            .map(Value::String)
            .unwrap_or(Value::Null),
    );
    Value::Object(tool_call)
}

/// A collab agent call, shown like any backend's subagent call. Items from
/// the child thread arrive on their own thread and are not nested here.
fn subagent_tool_call_payload(item: &Map<String, Value>, default_status: &str) -> Value {
    let mut tool_call = Map::from_iter([
        (
            "id".to_string(),
            json!(object_text(item, &["id"]).unwrap_or_else(|| "subagent".to_string())),
        ),
        ("kind".to_string(), json!("subagent")),
        (
            "status".to_string(),
            json!(normalize_tool_status(item, default_status)),
        ),
        (
            "title".to_string(),
            json!(object_text(item, &["tool"]).unwrap_or_else(|| "Subagent".to_string())),
        ),
    ]);
    if let Some(prompt) = object_text(item, &["prompt"]) {
        tool_call.insert("prompt".to_string(), json!(prompt));
    }
    Value::Object(tool_call)
}

fn file_change_tool_call_payload(payload: &Map<String, Value>, default_status: &str) -> Value {
    let mut tool_call = Map::new();
    tool_call.insert(
        "id".to_string(),
        Value::String(
            object_text(payload, &["id"])
                .or_else(|| object_text(payload, &["itemId"]))
                .unwrap_or_else(|| "file_change".to_string()),
        ),
    );
    tool_call.insert("kind".to_string(), Value::String("file_change".to_string()));
    tool_call.insert(
        "status".to_string(),
        Value::String(normalize_tool_status(payload, default_status)),
    );
    tool_call.insert(
        "title".to_string(),
        Value::String("Apply file changes".to_string()),
    );
    tool_call.insert(
        "file_paths".to_string(),
        Value::Array(
            extract_file_paths(payload)
                .into_iter()
                .map(Value::String)
                .collect(),
        ),
    );
    if let Some(changes) = file_changes::payload(codex_file_changes(payload)) {
        tool_call.insert("file_changes".to_string(), changes);
    }
    Value::Object(tool_call)
}

/// A native `fileChange` item's `changes`: update diffs as reported, added
/// and deleted files' contents as `+`/`-` lines.
fn codex_file_changes(payload: &Map<String, Value>) -> Vec<FileChange> {
    let changes = payload.get("changes").and_then(Value::as_array);
    changes
        .into_iter()
        .flatten()
        .filter_map(|change| {
            let path = change
                .get("path")
                .and_then(Value::as_str)
                .and_then(non_empty)?;
            let diff = change.get("diff").and_then(Value::as_str)?;
            let kind = change.get("kind").unwrap_or(&Value::Null);
            let move_to = kind["move_path"].as_str().and_then(non_empty);
            Some(match (kind["type"].as_str(), move_to) {
                (Some("add"), _) => FileChange::new(path, Some("add"), prefixed(diff, '+')),
                (Some("delete"), _) => FileChange::new(path, Some("delete"), prefixed(diff, '-')),
                (Some("update"), Some(to)) => {
                    let trailer = format!("\n\nMoved to: {to}");
                    let diff = diff.strip_suffix(&trailer).unwrap_or(diff);
                    let mut change = FileChange::new(to, Some("move"), diff.to_string());
                    change.move_from = Some(path.to_string());
                    change
                }
                (Some("update"), None) => FileChange::new(path, Some("update"), diff.to_string()),
                _ => FileChange::new(path, None, diff.to_string()),
            })
        })
        .collect()
}

fn normalize_tool_status(payload: &Map<String, Value>, default_status: &str) -> String {
    let raw_status = object_text(payload, &["status", "state"])
        .unwrap_or_else(|| default_status.to_string())
        .to_ascii_lowercase();
    match raw_status.as_str() {
        "running" | "in_progress" | "inprogress" | "started" | "pending" => "running",
        "failed" | "failure" | "error" | "errored" | "cancelled" | "canceled" | "declined" => {
            "failed"
        }
        "completed" | "complete" | "succeeded" | "success" | "done" => "completed",
        _ => default_status,
    }
    .to_string()
}

fn extract_file_paths(payload: &Map<String, Value>) -> Vec<String> {
    let mut paths = Vec::new();
    for key in ["file_paths", "filePaths", "paths"] {
        collect_file_paths(payload.get(key), &mut paths);
    }
    for key in ["changes", "edits", "files"] {
        collect_file_paths(payload.get(key), &mut paths);
    }
    paths.sort();
    paths.dedup();
    paths
}

fn collect_file_paths(value: Option<&Value>, paths: &mut Vec<String>) {
    match value {
        Some(Value::String(path)) => push_file_path(path, paths),
        Some(Value::Array(entries)) => {
            for entry in entries {
                collect_file_path_value(entry, paths);
            }
        }
        Some(Value::Object(object)) => {
            for key in ["path", "filePath", "file_path", "filename", "name"] {
                if let Some(path) = object.get(key).and_then(as_non_empty_string) {
                    push_file_path(&path, paths);
                    return;
                }
            }
        }
        _ => {}
    }
}

fn collect_file_path_value(value: &Value, paths: &mut Vec<String>) {
    match value {
        Value::String(path) => push_file_path(path, paths),
        Value::Object(object) => collect_file_paths(Some(&Value::Object(object.clone())), paths),
        _ => {}
    }
}

fn push_file_path(path: &str, paths: &mut Vec<String>) {
    if let Some(path) = non_empty(path) {
        paths.push(path.to_string());
    }
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

fn is_reasoning_item(item: &Map<String, Value>) -> bool {
    object_text(item, &["type"]).is_some_and(|kind| kind.eq_ignore_ascii_case("reasoning"))
}

/// Raw reasoning renders apart from the same item's summary parts.
fn raw_reasoning_item_id(item_id: &str) -> String {
    format!("{item_id}:raw")
}

fn reasoning_raw_text_from_item(item: &Map<String, Value>) -> Option<String> {
    let text = item
        .get("content")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join("\n\n");
    (!text.trim().is_empty()).then_some(text)
}

fn codex_thinking_event(active: bool) -> TurnStreamEvent {
    let mut event = TurnStreamEvent::thinking_progress(active, None);
    event.source.backend = Some(CODEX_APP_SERVER_BACKEND.to_string());
    event.source.raw_kind = Some(
        if active {
            "item/started"
        } else {
            "item/completed"
        }
        .to_string(),
    );
    event
}

/// Extracts the summary part texts from a completed reasoning item. Returns
/// None for non-reasoning items; reasoning items whose summary is empty yield
/// no completion events.
fn reasoning_summary_parts_from_item(item: &Map<String, Value>) -> Option<Vec<String>> {
    let item_type = object_text(item, &["type"])?.to_ascii_lowercase();
    if item_type != "reasoning" {
        return None;
    }
    let mut parts = item
        .get("summary")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| match entry {
                    Value::String(text) => Some(text.clone()),
                    Value::Object(part) => {
                        object_text(part, &["text", "summaryText", "summary_text"])
                    }
                    _ => None,
                })
                .filter(|text| !text.trim().is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if parts.is_empty() {
        if let Some(text) = object_text(item, &["text"]).filter(|text| !text.trim().is_empty()) {
            parts.push(text);
        }
    }
    (!parts.is_empty()).then_some(parts)
}

fn is_tool_item(item: &Map<String, Value>) -> bool {
    object_text(item, &["type"]).is_some_and(|item_type| {
        matches!(
            item_type.as_str(),
            "commandExecution" | "fileChange" | "collabAgentToolCall"
        )
    })
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

// Codex runs a collaboration-mode turn at the mode's settings, ignoring a
// top-level `effort`, so a mode-bearing turn carries its effort here.
fn collaboration_mode_for_chat_mode(
    chat_mode: Option<&str>,
    model: &str,
    effort: Option<&str>,
) -> Value {
    let mode = match chat_mode.map(str::trim).map(str::to_ascii_lowercase) {
        Some(mode) if mode == COLLABORATION_MODE_PLAN => COLLABORATION_MODE_PLAN,
        _ => COLLABORATION_MODE_DEFAULT,
    };
    let mut settings = json!({"model": model});
    if let Some(effort) = effort {
        settings["reasoning_effort"] = json!(effort);
    }
    json!({"mode": mode, "settings": settings})
}

fn normalize_reasoning_effort(value: Option<&str>) -> Result<Option<String>, CodexAppServerError> {
    let Some(value) = value.and_then(non_empty) else {
        return Ok(None);
    };
    Ok(Some(
        crate::config::validate_reasoning_effort(value, None)
            .map_err(CodexAppServerError::configuration)?,
    ))
}

fn rpc_error(prefix: &str, response: &Value) -> CodexAppServerError {
    let message = response
        .get("error")
        .and_then(auth::error_message)
        .filter(|message| !message.trim().is_empty())
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

/// Why no more output or replies can come once stdout has ended.
fn stream_end_error(read_error: Option<String>, exited: &str) -> CodexAppServerError {
    CodexAppServerError::runtime(match read_error {
        Some(error) => format!("codex app-server stdout read failed: {error}"),
        None => exited.to_string(),
    })
}

fn value_u64(value: &Value, keys: &[&str]) -> Option<u64> {
    let object = value.as_object()?;
    keys.iter().find_map(|key| object.get(*key)?.as_u64())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_codex(directory: &Path, include_host: bool) -> PathBuf {
        fs::create_dir_all(directory).expect("codex directory");
        let codex = directory.join(codex_binary_name());
        fs::write(&codex, b"codex").expect("codex binary");
        if include_host {
            fs::write(directory.join(code_mode_host_binary_name()), b"host")
                .expect("code-mode host");
        }
        codex
    }

    #[test]
    fn codex_resolution_prefers_explicit_override() {
        let resolved = resolve_codex_executable(
            Some("/custom/codex"),
            Some(OsStr::new("/ignored")),
            &[PathBuf::from("/bundled/codex")],
        );
        assert_eq!(resolved, PathBuf::from("/custom/codex"));
    }

    #[test]
    fn codex_resolution_uses_complete_path_installation_first() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path_codex = create_codex(&temp.path().join("path-bin"), true);
        let bundled_codex = create_codex(&temp.path().join("bundled"), true);
        let path = env::join_paths([path_codex.parent().expect("path parent")]).expect("PATH");

        let resolved = resolve_codex_executable(None, Some(&path), &[bundled_codex]);

        assert_eq!(resolved, path_codex);
    }

    #[test]
    fn codex_resolution_uses_bundled_pair_when_path_host_is_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path_codex = create_codex(&temp.path().join("path-bin"), false);
        let bundled_codex = create_codex(&temp.path().join("bundled"), true);
        let path = env::join_paths([path_codex.parent().expect("path parent")]).expect("PATH");

        let resolved =
            resolve_codex_executable(None, Some(&path), std::slice::from_ref(&bundled_codex));

        assert_eq!(resolved, bundled_codex);
    }

    #[test]
    fn codex_resolution_keeps_legacy_path_fallback_without_complete_pair() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path_codex = create_codex(&temp.path().join("path-bin"), false);
        let path = env::join_paths([path_codex.parent().expect("path parent")]).expect("PATH");

        let resolved = resolve_codex_executable(None, Some(&path), &[]);

        assert_eq!(resolved, PathBuf::from(codex_binary_name()));
    }

    /// Shims are taken directory by directory, each in `PATHEXT`'s order.
    #[test]
    fn codex_shims_follow_path_then_pathext_order() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (first, second) = (temp.path().join("first"), temp.path().join("second"));
        for shim in [
            first.join("codex"),
            second.join("codex.bat"),
            second.join("codex.cmd"),
        ] {
            fs::create_dir_all(shim.parent().unwrap()).expect("shim dir");
            fs::write(shim, "").expect("shim");
        }
        let path = env::join_paths([&first, &second]).expect("PATH");
        // Lowercase, so a case-insensitive file system returns the files as named.
        let find = |pathext: &str| find_codex_shim(Some(&path), OsStr::new(pathext));

        assert_eq!(find(".exe;.cmd;.bat"), Some(second.join("codex.cmd")));
        assert_eq!(find(".exe;.bat;.cmd"), Some(second.join("codex.bat")));
        fs::write(first.join("codex.cmd"), "").expect("earlier shim");
        assert_eq!(find(".exe;.bat;.cmd"), Some(first.join("codex.cmd")));
        assert_eq!(find(".exe"), None);
    }

    /// Discovery searches an empty `PATH` entry only as a launch does, so on
    /// Windows a `codex.bat` in Spark's own directory, which a launch never
    /// searches, diverts nothing from a later shim. A child takes that
    /// directory, which this process cannot change for its other tests.
    #[test]
    fn codex_discovery_takes_an_empty_path_entry_as_a_launch_does() {
        const PATH_VAR: &str = "SPARK_CODEX_EMPTY_PATH_ENTRY_TEST";
        let Some(path) = env::var_os(PATH_VAR) else {
            let temp = tempfile::tempdir().expect("tempdir");
            let bin = temp.path().join("bin");
            fs::create_dir_all(&bin).expect("bin dir");
            fs::write(temp.path().join("codex.bat"), "").expect("stray shim");
            fs::write(bin.join("codex.cmd"), "").expect("shim");
            let output = std::process::Command::new(env::current_exe().unwrap())
                .args([
                    "--exact",
                    "codex_app_server::tests::codex_discovery_takes_an_empty_path_entry_as_a_launch_does",
                    "--nocapture",
                ])
                .current_dir(temp.path())
                .env(PATH_VAR, env::join_paths([Path::new(""), &bin]).unwrap())
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success() && stdout.contains("1 passed"),
                "{stdout}\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        };
        // Lowercase, so a case-insensitive file system returns the files as named.
        let shim = find_codex_shim(Some(&path), OsStr::new(".bat;.cmd")).expect("shim");
        if cfg!(windows) {
            let bin = env::split_paths(&path).last().unwrap();
            assert_eq!(shim, bin.join("codex.cmd"));
            let resolved = resolve_codex_executable(None, Some(&path), &[]);
            let bare = crate::local_environment::find_executable_on_path("codex", Some(&path))
                .expect("bare codex");
            for found in [resolved, bare] {
                assert!(found.as_os_str().eq_ignore_ascii_case(&shim), "{found:?}");
            }
        } else {
            assert_eq!(shim, Path::new("codex.bat"));
        }
    }

    #[cfg(windows)]
    #[test]
    fn codex_resolution_falls_back_to_an_npm_shim_over_an_incomplete_codex_on_windows() {
        let temp = tempfile::tempdir().expect("tempdir");
        let incomplete = create_codex(&temp.path().join("incomplete"), false);
        let bin = temp.path().join("npm");
        fs::create_dir_all(&bin).expect("npm dir");
        // npm writes an extensionless sh script beside the shim.
        for name in ["codex", "codex.cmd"] {
            fs::write(bin.join(name), "").expect("shim");
        }
        let path = env::join_paths([incomplete.parent().unwrap(), &bin]).expect("PATH");

        let resolved = resolve_codex_executable(None, Some(&path), &[]);

        // A shim takes PATHEXT's case, which an NTFS lookup ignores.
        let same = |found: &Path, expected: PathBuf| {
            assert!(
                found.as_os_str().eq_ignore_ascii_case(&expected),
                "{found:?}"
            );
        };
        same(&resolved, bin.join("codex.cmd"));
        // An earlier batch shim comes first, as a bare `codex` finds it.
        let earlier = temp.path().join("earlier");
        fs::create_dir_all(&earlier).expect("earlier dir");
        fs::write(earlier.join("codex.bat"), "").expect("batch shim");
        let path = env::join_paths([&earlier, &bin]).expect("PATH");
        let resolved = resolve_codex_executable(None, Some(&path), &[]);
        same(&resolved, earlier.join("codex.bat"));
        let bare = crate::local_environment::find_executable_on_path("codex", Some(&path))
            .expect("bare codex");
        same(&bare, resolved);
        // A complete bundle still wins.
        let complete = create_codex(&temp.path().join("complete"), true);
        let path = env::join_paths([&bin, complete.parent().unwrap()]).expect("PATH");
        assert_eq!(resolve_codex_executable(None, Some(&path), &[]), complete);
    }

    #[test]
    fn runtime_dirs_skip_blank_home_values() {
        let vars = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(key, value)| (key.to_string(), OsString::from(value)))
                .collect::<BTreeMap<_, _>>()
        };
        let runtime = |home: &str, agent: &str| {
            PathBuf::from(home)
                .join(".spark")
                .join("runtime")
                .join(agent)
        };
        let profile = vars(&[("HOME", "   "), ("USERPROFILE", r"C:\Users\chris")]);
        assert_eq!(
            spark_runtime_dir(&profile, "codex"),
            runtime(r"C:\Users\chris", "codex")
        );
        let drive = vars(&[
            ("SPARK_HOME", " "),
            ("HOME", "\t"),
            ("USERPROFILE", "  "),
            ("HOMEDRIVE", "D:"),
            ("HOMEPATH", r"\Users\builder"),
        ]);
        assert_eq!(
            spark_runtime_dir(&drive, "claude"),
            runtime(r"D:\Users\builder", "claude")
        );
        let blank = vars(&[
            ("HOME", " "),
            ("USERPROFILE", " "),
            ("HOMEDRIVE", " "),
            ("HOMEPATH", r"\Users\builder"),
        ]);
        assert_eq!(spark_runtime_dir(&blank, "codex"), runtime("", "codex"));
        let spark = vars(&[("SPARK_HOME", "/srv/spark"), ("HOME", "/home/chris")]);
        assert_eq!(
            spark_runtime_dir(&spark, "codex"),
            PathBuf::from("/srv/spark/runtime/codex")
        );
    }

    /// Runs in a child process: HOME and the configured Spark home are
    /// process-wide.
    #[cfg(unix)]
    #[test]
    fn agents_launch_with_the_exact_non_utf8_paths_spark_prepares() {
        use std::os::unix::ffi::OsStrExt;

        const CHILD: &str = "SPARK_AGENT_NON_UTF8_HOME_TEST";
        if env::var_os(CHILD).is_none() {
            let temp = tempfile::tempdir().expect("tempdir");
            let root = temp.path().canonicalize().expect("canonical tempdir");
            let output = std::process::Command::new(env::current_exe().unwrap())
                .args([
                    "--exact",
                    "codex_app_server::tests::agents_launch_with_the_exact_non_utf8_paths_spark_prepares",
                    "--nocapture",
                ])
                // No SPARK_HOME: Spark's home is `~/.spark`.
                .env_clear()
                .env(CHILD, "1")
                .env("HOME", root.join(OsStr::from_bytes(b"home-\xff")))
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success() && stdout.contains("1 passed"),
                "{stdout}\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let home = PathBuf::from(env::var_os("HOME").unwrap());
        assert!(home.to_str().is_none());
        // As spark-server configures it at startup.
        let spark_home =
            spark_common::settings::resolve_data_dir(&spark_common::paths::ProcessEnvironment)
                .unwrap();
        assert_eq!(spark_home, home.join(".spark"));
        configure_codex_spark_home(&spark_home).unwrap();
        let runtime = spark_home.join("runtime");
        let exact = |path: &Path| Some(path.as_os_str().to_owned());

        // Every turn binds the Spark home for its agent processes.
        let turn = crate::agent::agent_turn_env(&BTreeMap::new());
        assert_eq!(turn.get("SPARK_HOME").cloned(), exact(&spark_home));

        // Claude Code's command, as launched.
        let (_, command) = crate::claude_code::claude_command(None);
        let claude = command
            .get_envs()
            .map(|(key, value)| (key.to_owned(), value.map(OsStr::to_owned)))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(claude[OsStr::new("SPARK_HOME")], exact(&spark_home));
        assert_eq!(
            claude[OsStr::new("CLAUDE_CONFIG_DIR")],
            exact(&runtime.join("claude"))
        );

        // Codex's launch environment, and the dirs prepared for it.
        let codex = runtime.join("codex");
        match build_codex_runtime_environment() {
            Ok(env) => {
                for (key, path) in [
                    ("SPARK_HOME", spark_home.clone()),
                    ("HOME", codex.clone()),
                    ("USERPROFILE", codex.clone()),
                    ("CODEX_HOME", codex.join(".codex")),
                    ("XDG_CONFIG_HOME", codex.join(".config")),
                    ("XDG_DATA_HOME", codex.join(".local/share")),
                ] {
                    assert_eq!(env.get(key).cloned(), exact(&path), "{key}");
                }
                assert!(codex.join(".codex/config.toml").is_file());
            }
            // APFS stores only UTF-8 names, so no such home can exist there.
            Err(error) if cfg!(target_os = "macos") => {
                assert!(error.message.contains("could not be created"), "{error:?}")
            }
            Err(error) => panic!("{error:?}"),
        }
    }
}
