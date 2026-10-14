//! Claude Code CLI backend: runs an agent turn by driving `claude -p` with
//! stream-json output. Authentication comes from the Claude Code login on the
//! host (subscription or API key) — Spark never handles the credential.

use std::collections::{BTreeMap, VecDeque};
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};
use spark_common::agent_settings::NativeAgentSettings;
use spark_common::events::{
    TurnStreamChannel, TurnStreamEvent, TurnStreamEventKind, TurnStreamSource,
};
use unified_llm_adapter::Usage;
use uuid::Uuid;

use crate::agent::{
    AgentRawLogLine, AgentThreadResumeFailure, AgentTurnEventSink, AgentTurnOutput,
    AgentTurnRequest,
};

pub mod auth;
mod control;

pub const CLAUDE_CODE_BACKEND: &str = "claude_code_cli";

const CLAUDE_CODE_BIN_ENV: &str = "SPARK_CLAUDE_CODE_BIN";
const CLAUDE_CODE_PERMISSION_MODE_ENV: &str = "SPARK_CLAUDE_CODE_PERMISSION_MODE";
const CLAUDE_CODE_CONFIG_DIR_ENV: &str = "SPARK_CLAUDE_CODE_CONFIG_DIR";
const DEFAULT_PERMISSION_MODE: &str = "bypassPermissions";
const TOOL_RESULT_PREVIEW_LIMIT: usize = 4000;
/// The resumed session's cumulative `modelUsage` at the start of a turn, as
/// recorded in the previous turn's usage payload under `model_usage`.
pub const MODEL_USAGE_SNAPSHOT_METADATA_KEY: &str = "spark.runtime.claude_code.model_usage";

#[derive(Debug, Clone, PartialEq)]
pub struct ClaudeCodeError {
    pub message: String,
    pub retryable: bool,
    pub details: Option<Value>,
}

impl ClaudeCodeError {
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
            retryable: true,
            details: None,
        }
    }
}

#[derive(Debug, Default)]
pub struct ClaudeCodeBackend;

impl ClaudeCodeBackend {
    pub fn new() -> Self {
        Self
    }

    pub fn answer_request_user_input(
        &self,
        request: crate::agent::AgentRequestUserInputAnswerRequest,
    ) -> AgentTurnOutput {
        control::answer(request)
    }

    pub fn interrupt(&self, project_path: &str, conversation_id: &str) -> bool {
        control::interrupt(project_path, conversation_id)
    }

    pub fn run_agent_turn(
        &self,
        request: AgentTurnRequest,
    ) -> Result<AgentTurnOutput, ClaudeCodeError> {
        self.run_agent_turn_with_event_sink(request, None)
    }

    /// A chat turn: `AskUserQuestion` reaches the request-user-input route
    /// through the event sink. Without a sink nobody sees the question, so
    /// it is denied.
    pub fn run_agent_turn_with_event_sink(
        &self,
        request: AgentTurnRequest,
        event_sink: Option<AgentTurnEventSink>,
    ) -> Result<AgentTurnOutput, ClaudeCodeError> {
        let questions = if event_sink.is_some() {
            control::Questions::Chat
        } else {
            control::Questions::Deny
        };
        self.run(request, event_sink, questions)
    }

    /// A flow node turn: `AskUserQuestion` goes to the flow's clarification
    /// handler; without one it is denied.
    pub(crate) fn run_codergen_turn(
        &self,
        request: AgentTurnRequest,
        event_sink: Option<AgentTurnEventSink>,
        clarification_handler: Option<crate::codergen::ClarificationHandler>,
    ) -> Result<AgentTurnOutput, ClaudeCodeError> {
        let questions = match clarification_handler {
            Some(handler) => control::Questions::Flow(handler),
            None => control::Questions::Deny,
        };
        self.run(request, event_sink, questions)
    }

    /// A turn nobody can answer questions in; `AskUserQuestion` is denied.
    pub fn run_agent_turn_without_questions(
        &self,
        request: AgentTurnRequest,
        event_sink: Option<AgentTurnEventSink>,
    ) -> Result<AgentTurnOutput, ClaudeCodeError> {
        self.run(request, event_sink, control::Questions::Deny)
    }

    fn run(
        &self,
        request: AgentTurnRequest,
        event_sink: Option<AgentTurnEventSink>,
        questions: control::Questions,
    ) -> Result<AgentTurnOutput, ClaudeCodeError> {
        let resumed_session_id =
            metadata_string(&request.metadata, "spark.runtime.claude_code.session_id")
                .or_else(|| metadata_string(&request.metadata, "claude_code.session_id"));
        let mut turn = Turn::new(&request, resumed_session_id.is_some());
        let mut thread_resume_failure = None;
        if let Err(error) = self.run_with_respawn(
            &request,
            &event_sink,
            resumed_session_id.as_deref(),
            &questions,
            &mut turn,
        ) {
            // The stored session never started, so start a fresh one.
            let Some(discarded_session_id) = resumed_session_id.filter(|_| {
                turn.state.session_id.is_none() && error.message.contains("without a result event")
            }) else {
                return Err(error);
            };
            turn = Turn::new(&request, false);
            self.run_with_respawn(&request, &event_sink, None, &questions, &mut turn)?;
            thread_resume_failure = Some(AgentThreadResumeFailure {
                message: format!(
                    "claude code could not resume session {discarded_session_id}; started a fresh session"
                ),
                error_code: Some("thread_resume_failure".to_string()),
                details: Some(json!({"discarded_session_id": discarded_session_id})),
            });
        }

        let Turn {
            app_turn_id,
            state,
            events,
            raw_log_lines,
            ..
        } = turn;
        if state.is_error {
            let text = state.resolved_final_text();
            return Err(ClaudeCodeError {
                message: if auth::requires_login(&text) {
                    auth::AUTH_REQUIRED_MESSAGE.to_string()
                } else {
                    format!(
                        "claude code turn failed ({}): {text}",
                        state.result_subtype.as_deref().unwrap_or("error"),
                    )
                },
                retryable: false,
                details: state.result_payload.clone(),
            });
        }
        Ok(AgentTurnOutput {
            app_thread_id: state.session_id.clone(),
            app_turn_id: Some(app_turn_id),
            final_assistant_text: non_empty(&state.resolved_final_text()).map(str::to_string),
            token_usage: state.usage_payload.clone(),
            token_usage_breakdown: state.usage_payload,
            events,
            raw_log_lines,
            thread_resume_failure,
        })
    }

    /// Runs the turn's process; if it dies before the turn ends once its
    /// session is known, resumes that session once to finish the turn.
    fn run_with_respawn(
        &self,
        request: &AgentTurnRequest,
        event_sink: &Option<AgentTurnEventSink>,
        resume_session_id: Option<&str>,
        questions: &control::Questions,
        turn: &mut Turn,
    ) -> Result<(), ClaudeCodeError> {
        let Err(error) = self.run_process(
            request,
            event_sink,
            resume_session_id,
            None,
            questions,
            turn,
        ) else {
            return Ok(());
        };
        let Some(session_id) = turn.state.session_id.clone().filter(|_| !turn.stopping) else {
            return Err(error);
        };
        tracing::warn!(error = %error.message, "Claude Code exited mid-turn; resuming its session");
        turn.raw_log_lines.push(AgentRawLogLine {
            direction: "control".into(),
            line: format!("Claude Code control: respawning with --resume {session_id}"),
        });
        self.run_process(
            request,
            event_sink,
            Some(&session_id),
            Some(CONTINUATION_PROMPT),
            questions,
            turn,
        )
        .map_err(|_| error)
    }

    /// One Claude process. Ok once the turn has ended; Err if the process
    /// exited first.
    fn run_process(
        &self,
        request: &AgentTurnRequest,
        event_sink: &Option<AgentTurnEventSink>,
        resume_session_id: Option<&str>,
        continuation: Option<&str>,
        questions: &control::Questions,
        turn: &mut Turn,
    ) -> Result<(), ClaudeCodeError> {
        let working_dir = PathBuf::from(&request.project_path);
        if !working_dir.exists() {
            return Err(ClaudeCodeError::configuration(format!(
                "claude code working directory is unavailable in the runtime: {}",
                working_dir.display()
            )));
        }
        let configuration = crate::config::captured_session_config(&request.metadata)
            .map_err(ClaudeCodeError::configuration)?;
        let native = configuration.as_ref().map(|config| &config.native);
        let (executable, mut command) = claude_command(native);
        command
            .arg("-p")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--input-format")
            .arg("stream-json")
            .arg("--permission-prompt-tool")
            .arg("stdio")
            .arg("--include-partial-messages")
            .arg("--verbose")
            .arg("--permission-mode")
            .arg(
                native
                    .and_then(|config| config.claude_permission_mode.clone())
                    .unwrap_or_else(permission_mode),
            )
            .current_dir(&working_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(model) = request.model.as_deref().and_then(non_empty) {
            command.arg("--model").arg(model);
        }
        if let Some(effort) = request.reasoning_effort.as_deref().and_then(non_empty) {
            command.arg("--effort").arg(effort);
        }
        if let Some(session_id) = resume_session_id {
            command.arg("--resume").arg(session_id);
        }
        // Pinned instructions ride every spawn, resumed or not; the file lives
        // until this turn's process has exited.
        let instructions = metadata_string(
            &request.metadata,
            crate::agent::AGENT_INSTRUCTIONS_METADATA_KEY,
        )
        .map(|text| {
            let mut file = tempfile::NamedTempFile::new()?;
            file.write_all(text.as_bytes())?;
            Ok::<_, std::io::Error>(file)
        })
        .transpose()
        .map_err(|error| {
            ClaudeCodeError::runtime(format!("claude code instructions write failed: {error}"))
        })?;
        if let Some(file) = &instructions {
            command.arg("--append-system-prompt-file").arg(file.path());
        }
        // Background tasks hold the turn open, so keep them on even when Spark
        // itself runs under an agent that disabled them. Long gates such as
        // `just test` exceed Claude Code's 10-minute foreground cap.
        // ponytail: fixed 1h cap; make it configurable if a gate needs longer.
        command
            .env_remove("CLAUDE_CODE_DISABLE_BACKGROUND_TASKS")
            .env("BASH_DEFAULT_TIMEOUT_MS", "3600000")
            .env("BASH_MAX_TIMEOUT_MS", "3600000");

        // Attachments are read before spawning so a missing file fails cleanly.
        let content = match continuation {
            Some(text) => json!(text),
            None => {
                let (prompt_text, content) =
                    user_message_content(request, resume_session_id.is_none())?;
                crate::initial_context::capture_if_configured(&request.metadata, &prompt_text)
                    .map_err(|error| {
                        ClaudeCodeError::configuration(format!(
                            "initial context capture failed: {error}"
                        ))
                    })?;
                content
            }
        };
        let mut child = command
            .spawn()
            .map_err(|error| launch_error(&executable, error))?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ClaudeCodeError::runtime(
                "claude code did not expose its stdio",
            ));
        };
        // Shared rather than joined: a leftover grandchild may hold stderr.
        let stderr_lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let stderr_handle = std::thread::spawn({
            let stderr_lines = stderr_lines.clone();
            move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    stderr_lines.lock().unwrap().push(line);
                }
            }
        });
        let control = control::Control::register(child, stdin, request, questions.clone());
        // The prompt travels over stdin so arbitrarily large composed prompts
        // never hit argv limits.
        // A failed write closes stdin; the process exit then reports it.
        control.lock().unwrap().send_user(content);

        let (sender, receiver) = mpsc::channel();
        // Detached: a leftover grandchild may hold stdout after the turn ends.
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let started = std::time::Instant::now();
        let mut initialized = false;
        let mut exited_at: Option<std::time::Instant> = None;
        let mut abandoned = None;
        while !control.lock().unwrap().ended {
            // Checked every iteration: continuous output must not hide an
            // exit or a missing system/init.
            {
                let mut control = control.lock().unwrap();
                if exited_at.is_none() && matches!(control.child.try_wait(), Ok(Some(_))) {
                    exited_at = Some(std::time::Instant::now());
                }
                if !initialized && started.elapsed() >= Duration::from_secs(5) {
                    control.log("system/init timeout; using one-shot input");
                    control.one_shot();
                    initialized = true;
                }
                if let Err(reason) = control.poll_clarifications() {
                    control.log(&reason);
                    abandoned = Some(reason);
                    break;
                }
            }
            // A leftover grandchild may hold stdout open past the process
            // exit: stop once stdout goes quiet after the exit, or after a
            // cap if the grandchild keeps writing.
            if exited_at.is_some_and(|at| at.elapsed() >= Duration::from_secs(2)) {
                break;
            }
            let line = match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(line) => line,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if exited_at.is_some() {
                        break;
                    }
                    continue;
                }
            };
            if line.trim().is_empty() {
                continue;
            }
            turn.raw_log_lines.push(AgentRawLogLine {
                direction: "stdout".to_string(),
                line: line.clone(),
            });
            let Ok(mut message) = serde_json::from_str::<Value>(&line) else {
                control.lock().unwrap().log("malformed JSON on stdout");
                continue;
            };
            if message["type"] == "system" && message["subtype"] == "init" {
                initialized = true;
            }
            let (question, ended) = {
                let mut control = control.lock().unwrap();
                // 2.1.220 reports an interrupted stream as an error result.
                // Only an acknowledged interrupt with an explicit abort reason
                // is normal completion; genuine execution errors remain errors.
                if control.interrupted_result(&message) {
                    message["is_error"] = json!(false);
                    message["subtype"] = json!("success");
                    let (item_id, text) = latest_assistant_text(&turn.events);
                    if item_id.is_some() {
                        turn.state.last_text_item_id = item_id;
                    }
                    message["result"] = json!(non_empty(&text).unwrap_or("Stopped."));
                }
                (control.ingest(&message), control.ended)
            };
            let mut normalized = turn.state.ingest(&message, ended);
            if let Some(mut question) = question {
                question.source.app_thread_id = turn.state.session_id.clone();
                normalized.push(question);
            }
            turn.push(normalized, event_sink);
        }

        let ended = {
            let mut control = control.lock().unwrap();
            turn.stopping |= control.stopping;
            turn.raw_log_lines.append(&mut control.logs);
            let ended = control.ended;
            control.end();
            ended
        };
        let status = reap(&control)?;
        if let Some(reason) = abandoned {
            // The run stopped waiting for an answer; do not resume the session.
            turn.stopping = true;
            return Err(ClaudeCodeError {
                message: reason,
                retryable: false,
                details: None,
            });
        }
        if ended {
            // Stopped while idle: the last result, so far commentary, is final.
            if !turn.state.completed {
                if let Some(result) = turn.state.result_payload.clone() {
                    let events = turn.state.ingest_result(&result, true);
                    turn.push(events, event_sink);
                }
            }
            return Ok(());
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !stderr_handle.is_finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        let stderr_lines = std::mem::take(&mut *stderr_lines.lock().unwrap());
        let stderr_tail = stderr_lines[stderr_lines.len().saturating_sub(10)..].join("\n");
        Err(ClaudeCodeError {
            message: format!(
                "claude code exited ({status}) {}; stderr tail:\n{stderr_tail}",
                if turn.state.result_payload.is_none() {
                    "without a result event"
                } else {
                    "before the turn ended"
                }
            ),
            retryable: true,
            details: None,
        })
    }
}

const CONTINUATION_PROMPT: &str =
    "Your previous process exited before this turn finished. Continue the task where you left off.";
const EXIT_GRACE: Duration = Duration::from_secs(3);

/// Waits briefly for the ended process to exit on its own, then kills it.
fn reap(
    control: &std::sync::Mutex<control::Control>,
) -> Result<std::process::ExitStatus, ClaudeCodeError> {
    let deadline = std::time::Instant::now() + EXIT_GRACE;
    loop {
        let mut control = control.lock().unwrap();
        match control.child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if std::time::Instant::now() < deadline => {}
            _ => {
                let _ = control.child.kill();
                return control.child.wait().map_err(|error| {
                    ClaudeCodeError::runtime(format!("claude code wait failed: {error}"))
                });
            }
        }
        drop(control);
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The item id and text of the latest assistant text, including a block cut
/// off mid-delta; a stopped turn promotes it to its final answer.
pub(crate) fn latest_assistant_text(events: &[TurnStreamEvent]) -> (Option<String>, String) {
    let Some(latest) = events.iter().rev().find(|event| {
        event.channel == Some(TurnStreamChannel::Assistant)
            && matches!(
                event.kind,
                TurnStreamEventKind::ContentDelta | TurnStreamEventKind::ContentCompleted
            )
    }) else {
        return (None, String::new());
    };
    let text = if latest.kind == TurnStreamEventKind::ContentCompleted {
        latest.content_delta.clone().unwrap_or_default()
    } else {
        events
            .iter()
            .filter(|event| {
                event.kind == TurnStreamEventKind::ContentDelta
                    && event.source.item_id == latest.source.item_id
            })
            .filter_map(|event| event.content_delta.as_deref())
            .collect()
    };
    (latest.source.item_id.clone(), text)
}

/// Everything one turn has gathered, across its results and processes.
struct Turn {
    app_turn_id: String,
    state: ClaudeCodeTurnState,
    events: Vec<TurnStreamEvent>,
    raw_log_lines: Vec<AgentRawLogLine>,
    stopping: bool,
}

impl Turn {
    fn new(request: &AgentTurnRequest, resumed: bool) -> Self {
        let mut state = ClaudeCodeTurnState::default();
        if resumed {
            if let Some(snapshot) = request.metadata.get(MODEL_USAGE_SNAPSHOT_METADATA_KEY) {
                state.start_model_usage = snapshot.clone();
            }
        }
        Self {
            app_turn_id: Uuid::new_v4().to_string(),
            state,
            events: Vec::new(),
            raw_log_lines: Vec::new(),
            stopping: false,
        }
    }

    fn push(&mut self, events: Vec<TurnStreamEvent>, event_sink: &Option<AgentTurnEventSink>) {
        for mut event in events {
            event.source.app_turn_id = Some(self.app_turn_id.clone());
            if let Some(sink) = event_sink {
                sink(event.clone());
            }
            self.events.push(event);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeCodeModelMetadata {
    pub id: String,
    pub display: String,
    /// Effort levels the model accepts; empty when it reports none.
    pub supported_efforts: Vec<String>,
}

const MODEL_DISCOVERY_REQUEST_ID: &str = "spark-model-discovery";
const MODEL_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(20);

/// Queries the installed CLI's selectable model catalog over its stdio
/// control protocol (`control_request` subtype `list_models`). The protocol
/// is what the Agent SDK's `supportedModels()` uses; it is not formally
/// documented, so failures here are expected to fall back to static aliases
/// at the caller.
pub fn list_available_claude_code_models() -> Result<Vec<ClaudeCodeModelMetadata>, ClaudeCodeError>
{
    list_available_claude_code_models_with_settings(None)
}

pub fn list_available_claude_code_models_with_settings(
    native: Option<&spark_common::agent_settings::NativeAgentSettings>,
) -> Result<Vec<ClaudeCodeModelMetadata>, ClaudeCodeError> {
    let (executable, mut command) = claude_command(native);
    command
        .arg("-p")
        .arg("--verbose")
        .arg("--input-format")
        .arg("stream-json")
        .arg("--output-format")
        .arg("stream-json")
        // Discovery is project-independent; a neutral working directory keeps
        // project-level hooks and instructions out of the probe.
        .current_dir(env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let mut child = command
        .spawn()
        .map_err(|error| log_model_discovery_error(launch_error(&executable, error)))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| ClaudeCodeError::runtime("claude code did not expose stdin"))?;
    let request = json!({
        "type": "control_request",
        "request_id": MODEL_DISCOVERY_REQUEST_ID,
        "request": {"subtype": "list_models"},
    });
    writeln!(stdin, "{request}")
        .and_then(|()| stdin.flush())
        .map_err(|error| {
            let _ = child.kill();
            let _ = child.wait();
            log_model_discovery_error(ClaudeCodeError::runtime(format!(
                "claude code control request write failed: {error}"
            )))
        })?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ClaudeCodeError::runtime("claude code did not expose stdout"))?;
    // The CLI answers on its own schedule (startup hooks run first), so a
    // reader thread feeds a channel and the timeout bounds the wait; stdin
    // stays open until then so the CLI does not treat input as finished.
    let (sender, receiver) = mpsc::channel::<Value>();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if message.get("type").and_then(Value::as_str) == Some("control_response") {
                let _ = sender.send(message);
                return;
            }
        }
    });
    let outcome = receiver.recv_timeout(MODEL_DISCOVERY_TIMEOUT);
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();

    let message = outcome.map_err(|_| {
        log_model_discovery_error(ClaudeCodeError::runtime(
            "claude code exited or timed out without answering the list_models control request",
        ))
    })?;
    let response = message.get("response").cloned().unwrap_or(Value::Null);
    if response.get("request_id").and_then(Value::as_str) != Some(MODEL_DISCOVERY_REQUEST_ID) {
        return Err(log_model_discovery_error(ClaudeCodeError::runtime(
            "claude code control response did not match the list_models request",
        )));
    }
    if response.get("subtype").and_then(Value::as_str) != Some("success") {
        let detail = response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        return Err(log_model_discovery_error(ClaudeCodeError::runtime(
            format!("claude code list_models request failed: {detail}"),
        )));
    }
    Ok(claude_code_models_from_list_result(
        response.get("response").unwrap_or(&Value::Null),
    ))
}

/// Maps a `list_models` result (`{"models": [...]}`) onto the metadata the
/// chooser needs. The `default` pseudo-entry maps to an empty id: a blank
/// model means "the CLI's default" on this provider, so the catalog's own
/// label ends up on the state a fresh conversation is actually in.
pub fn claude_code_models_from_list_result(result: &Value) -> Vec<ClaudeCodeModelMetadata> {
    let Some(entries) = result.get("models").and_then(Value::as_array) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let value = entry
                .get("value")
                .and_then(Value::as_str)
                .and_then(non_empty)?;
            let id = if value == "default" { "" } else { value };
            let display = entry
                .get("displayName")
                .and_then(Value::as_str)
                .and_then(non_empty)
                .unwrap_or(value);
            let supported_efforts = entry
                .get("supportedEffortLevels")
                .and_then(Value::as_array)
                .map(|levels| {
                    levels
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            Some(ClaudeCodeModelMetadata {
                id: id.to_string(),
                display: display.to_string(),
                supported_efforts,
            })
        })
        .collect()
}

fn log_model_discovery_error(error: ClaudeCodeError) -> ClaudeCodeError {
    tracing::warn!(
        error = %error.message,
        "Claude Code model discovery failed"
    );
    error
}

/// The turn's text and its stream-json `content`: the plain prompt string
/// for text-only turns, else text plus base64 image blocks. Files that are
/// not native images travel as local references in the text, as do earlier
/// attachments when the session starts fresh.
fn user_message_content(
    request: &AgentTurnRequest,
    fresh_session: bool,
) -> Result<(String, Value), ClaudeCodeError> {
    use base64::Engine as _;

    let attachments = crate::attachments::request_attachments(&request.metadata);
    let earlier = if fresh_session {
        crate::attachments::history_attachments(&request.history)
    } else {
        Vec::new()
    };
    let text = crate::attachments::prompt_with_attachment_references(
        &request.prompt,
        &attachments,
        &earlier,
    );
    let images = attachments
        .iter()
        .filter(|attachment| attachment.is_native_image())
        .collect::<Vec<_>>();
    if images.is_empty() {
        return Ok((text.clone(), Value::String(text)));
    }
    let mut blocks = Vec::new();
    if !text.is_empty() {
        blocks.push(json!({"type": "text", "text": text}));
    }
    for image in images {
        let bytes = std::fs::read(&image.path).map_err(|error| {
            ClaudeCodeError::configuration(format!(
                "claude code could not read attachment {}: {error}",
                image.filename
            ))
        })?;
        blocks.push(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": image.media_type,
                "data": base64::engine::general_purpose::STANDARD.encode(bytes),
            },
        }));
    }
    Ok((text, Value::Array(blocks)))
}

/// Accumulates one `claude -p` stream-json turn and maps each protocol
/// message onto the TurnStreamEvent vocabulary the journal already renders.
#[derive(Debug, Default)]
struct ClaudeCodeTurnState {
    session_id: Option<String>,
    assistant_texts: Vec<String>,
    result_text: Option<String>,
    result_subtype: Option<String>,
    result_payload: Option<Value>,
    usage_payload: Option<Value>,
    start_model_usage: Value,
    is_error: bool,
    /// The final result has been reported; earlier results are commentary.
    completed: bool,
    block_counter: usize,
    partial_block_ids: BTreeMap<usize, String>,
    pending_text_block_ids: VecDeque<String>,
    pending_reasoning_block_ids: VecDeque<String>,
    last_text_item_id: Option<String>,
    /// The item the latest result's text was reported under.
    result_item_id: Option<String>,
    tool_calls: BTreeMap<String, Value>,
}

impl ClaudeCodeTurnState {
    fn resolved_final_text(&self) -> String {
        self.result_text
            .clone()
            .unwrap_or_else(|| self.assistant_texts.join("\n"))
            .trim()
            .to_string()
    }

    /// `last` marks a result that ends the turn.
    fn ingest(&mut self, message: &Value, last: bool) -> Vec<TurnStreamEvent> {
        let message_type = message.get("type").and_then(Value::as_str).unwrap_or("");
        if let Some(session_id) = message.get("session_id").and_then(Value::as_str) {
            self.session_id
                .get_or_insert_with(|| session_id.to_string());
        }
        match message_type {
            "system" => self.ingest_system(message),
            "assistant" => self.ingest_assistant(message),
            "stream_event" => self.ingest_stream_event(message),
            "user" => self.ingest_user(message),
            "result" => self.ingest_result(message, last),
            other => vec![stream_event(
                TurnStreamEventKind::Other(other.to_string()),
                self.session_id.as_deref(),
                other,
                |event| {
                    event.details = Some(message.clone());
                },
            )],
        }
    }

    fn ingest_system(&mut self, message: &Value) -> Vec<TurnStreamEvent> {
        let subtype = message
            .get("subtype")
            .and_then(Value::as_str)
            .unwrap_or("system");
        let raw_kind = format!("system_{subtype}");
        vec![stream_event(
            TurnStreamEventKind::Other(raw_kind.clone()),
            self.session_id.as_deref(),
            &raw_kind,
            |event| {
                event.message = message
                    .get("model")
                    .and_then(Value::as_str)
                    .map(|model| format!("claude code session started (model {model})"));
                event.details = Some(message.clone());
            },
        )]
    }

    fn ingest_assistant(&mut self, message: &Value) -> Vec<TurnStreamEvent> {
        let mut events = Vec::new();
        let blocks = message
            .pointer("/message/content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for block in blocks {
            match block.get("type").and_then(Value::as_str).unwrap_or("") {
                "text" => {
                    let text = block.get("text").and_then(Value::as_str).unwrap_or("");
                    if text.trim().is_empty() {
                        continue;
                    }
                    let item_id = self.completed_item_id(TurnStreamChannel::Assistant);
                    self.last_text_item_id = Some(item_id.clone());
                    self.assistant_texts.push(text.to_string());
                    events.push(stream_event(
                        TurnStreamEventKind::ContentCompleted,
                        self.session_id.as_deref(),
                        "assistant_text",
                        |event| {
                            event.channel = Some(TurnStreamChannel::Assistant);
                            event.content_delta = Some(text.to_string());
                            event.message = Some(text.to_string());
                            event.source.item_id = Some(item_id);
                            event.phase = Some("commentary".to_string());
                        },
                    ));
                }
                "thinking" => {
                    let thinking = block.get("thinking").and_then(Value::as_str).unwrap_or("");
                    if thinking.trim().is_empty() {
                        continue;
                    }
                    let item_id = self.completed_item_id(TurnStreamChannel::Reasoning);
                    events.push(stream_event(
                        TurnStreamEventKind::ContentCompleted,
                        self.session_id.as_deref(),
                        "assistant_thinking",
                        |event| {
                            event.channel = Some(TurnStreamChannel::Reasoning);
                            event.content_delta = Some(thinking.to_string());
                            event.message = Some(thinking.to_string());
                            event.source.item_id = Some(item_id);
                            event.phase = Some("commentary".to_string());
                        },
                    ));
                }
                "tool_use" => {
                    let tool_call = canonical_tool_call(&block);
                    if let Some(id) = block.get("id").and_then(Value::as_str) {
                        self.tool_calls.insert(id.to_string(), tool_call.clone());
                    }
                    events.push(stream_event(
                        TurnStreamEventKind::ToolCallStarted,
                        self.session_id.as_deref(),
                        "tool_use",
                        |event| {
                            event.tool_call = Some(tool_call);
                            event.status = Some("started".to_string());
                        },
                    ));
                }
                _ => {}
            }
        }
        events
    }

    fn ingest_stream_event(&mut self, message: &Value) -> Vec<TurnStreamEvent> {
        let event = message.get("event").unwrap_or(message);
        if event.get("type").and_then(Value::as_str) == Some("message_start") {
            self.partial_block_ids.clear();
            return Vec::new();
        }
        if event.get("type").and_then(Value::as_str) != Some("content_block_delta") {
            return Vec::new();
        }
        let Some(index) = event
            .get("index")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
        else {
            return Vec::new();
        };
        let delta = event.get("delta").unwrap_or(&Value::Null);
        let (channel, text, raw_kind) = match delta.get("type").and_then(Value::as_str) {
            Some("text_delta") => (
                TurnStreamChannel::Assistant,
                delta.get("text").and_then(Value::as_str),
                "assistant_text_delta",
            ),
            Some("thinking_delta") => (
                TurnStreamChannel::Reasoning,
                delta.get("thinking").and_then(Value::as_str),
                "assistant_thinking_delta",
            ),
            _ => return Vec::new(),
        };
        let Some(text) = text.filter(|text| !text.is_empty()) else {
            return Vec::new();
        };
        let (item_id, is_new) = self.item_id_for_partial(index);
        if is_new {
            match channel {
                TurnStreamChannel::Assistant => {
                    self.pending_text_block_ids.push_back(item_id.clone())
                }
                TurnStreamChannel::Reasoning => {
                    self.pending_reasoning_block_ids.push_back(item_id.clone())
                }
                _ => {}
            }
        }
        vec![stream_event(
            TurnStreamEventKind::ContentDelta,
            self.session_id.as_deref(),
            raw_kind,
            |stream_event| {
                stream_event.channel = Some(channel);
                stream_event.content_delta = Some(text.to_string());
                stream_event.message = Some(text.to_string());
                stream_event.source.item_id = Some(item_id);
                stream_event.phase = Some("commentary".to_string());
            },
        )]
    }

    fn item_id_for_partial(&mut self, index: usize) -> (String, bool) {
        if let Some(item_id) = self.partial_block_ids.get(&index) {
            return (item_id.clone(), false);
        }
        self.block_counter += 1;
        let item_id = format!("block-{}", self.block_counter);
        self.partial_block_ids.insert(index, item_id.clone());
        (item_id, true)
    }

    fn completed_item_id(&mut self, channel: TurnStreamChannel) -> String {
        let pending = match channel {
            TurnStreamChannel::Assistant => self.pending_text_block_ids.pop_front(),
            TurnStreamChannel::Reasoning => self.pending_reasoning_block_ids.pop_front(),
            _ => None,
        };
        pending.unwrap_or_else(|| {
            self.block_counter += 1;
            format!("block-{}", self.block_counter)
        })
    }

    fn ingest_user(&mut self, message: &Value) -> Vec<TurnStreamEvent> {
        let mut events = Vec::new();
        let blocks = message
            .pointer("/message/content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for block in blocks {
            if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let is_error = block
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let id = block.get("tool_use_id").cloned().unwrap_or(Value::Null);
            let mut tool_call = block
                .get("tool_use_id")
                .and_then(Value::as_str)
                .and_then(|id| self.tool_calls.get(id).cloned())
                .unwrap_or_else(|| json!({"id": id}));
            if let Some(payload) = tool_call.as_object_mut() {
                payload.insert("output".to_string(), tool_result_preview(&block));
                payload.insert("is_error".to_string(), Value::Bool(is_error));
            }
            events.push(stream_event(
                if is_error {
                    TurnStreamEventKind::ToolCallFailed
                } else {
                    TurnStreamEventKind::ToolCallCompleted
                },
                self.session_id.as_deref(),
                "tool_result",
                |event| {
                    event.tool_call = Some(tool_call);
                    event.status = Some(if is_error { "failed" } else { "completed" }.to_string());
                },
            ));
        }
        events
    }

    fn ingest_result(&mut self, message: &Value, last: bool) -> Vec<TurnStreamEvent> {
        // Each result claims the latest assistant text once; a later result
        // with no new text gets its own item, while promoting the same result
        // on a stop reuses its item.
        let item_id = self.last_text_item_id.take().or_else(|| {
            self.result_item_id
                .clone()
                .filter(|_| self.result_payload.as_ref() == Some(message))
        });
        self.completed = last;
        self.result_payload = Some(message.clone());
        self.result_subtype = message
            .get("subtype")
            .and_then(Value::as_str)
            .map(str::to_string);
        self.is_error = message
            .get("is_error")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.result_text = message
            .get("result")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut events = Vec::new();
        if let Some(text) = self.result_text.as_deref().and_then(non_empty) {
            let item_id = item_id.unwrap_or_else(|| {
                self.block_counter += 1;
                format!("block-{}", self.block_counter)
            });
            self.result_item_id = Some(item_id.clone());
            events.push(stream_event(
                TurnStreamEventKind::ContentCompleted,
                self.session_id.as_deref(),
                "result_text",
                |event| {
                    event.channel = Some(TurnStreamChannel::Assistant);
                    event.content_delta = Some(text.to_string());
                    event.message = Some(text.to_string());
                    event.source.item_id = Some(item_id);
                    event.phase =
                        Some(if last { "final_answer" } else { "commentary" }.to_string());
                },
            ));
        }
        if let Some(model_usage) = message.get("modelUsage").filter(|value| value.is_object()) {
            let usage_payload =
                usage_payload_from_model_usage(model_usage, &self.start_model_usage);
            self.usage_payload = Some(usage_payload.clone());
            events.push(stream_event(
                TurnStreamEventKind::TokenUsageUpdated,
                self.session_id.as_deref(),
                "result_usage",
                |event| {
                    event.token_usage = Some(usage_payload.clone());
                },
            ));
        }
        if !last {
            return events;
        }
        events.push(stream_event(
            TurnStreamEventKind::TurnCompleted,
            self.session_id.as_deref(),
            "result",
            |event| {
                event.status = self.result_subtype.clone();
                if self.is_error {
                    event.error = Some(self.resolved_final_text());
                    event.error_code = self.result_subtype.clone();
                }
            },
        ));
        events
    }
}

fn canonical_tool_call(block: &Value) -> Value {
    let name = block.get("name").and_then(Value::as_str).unwrap_or("");
    let input = block.get("input").cloned().unwrap_or(Value::Null);
    let mut payload = serde_json::Map::from_iter([
        (
            "id".to_string(),
            block.get("id").cloned().unwrap_or(Value::Null),
        ),
        ("name".to_string(), Value::String(name.to_string())),
        ("input".to_string(), input.clone()),
    ]);
    let input_string = |key| input.get(key).and_then(Value::as_str).and_then(non_empty);

    match name {
        "Bash" => {
            let command = input_string("command");
            payload.insert("kind".to_string(), json!("command_execution"));
            if let Some(command) = command {
                payload.insert("command".to_string(), json!(command));
            }
            let title = input_string("description")
                .or_else(|| {
                    command
                        .and_then(|command| command.lines().next())
                        .and_then(non_empty)
                })
                .unwrap_or(name);
            payload.insert("title".to_string(), json!(title));
        }
        "Read" | "Write" | "Edit" | "NotebookEdit" => {
            let path = input_string("file_path").or_else(|| input_string("notebook_path"));
            if let Some(path) = path {
                payload.insert("file_paths".to_string(), json!([path]));
            }
            payload.insert("title".to_string(), json!(name));
        }
        "Glob" | "Grep" => {
            if let Some(path) = input_string("path") {
                payload.insert("file_paths".to_string(), json!([path]));
            }
            payload.insert("title".to_string(), json!(name));
        }
        _ => {
            payload.insert("kind".to_string(), json!("dynamic_tool"));
            payload.insert("title".to_string(), json!(name));
        }
    }
    Value::Object(payload)
}

fn stream_event(
    kind: TurnStreamEventKind,
    session_id: Option<&str>,
    raw_kind: &str,
    fill: impl FnOnce(&mut TurnStreamEvent),
) -> TurnStreamEvent {
    let mut event = TurnStreamEvent {
        kind,
        channel: None,
        source: TurnStreamSource {
            backend: Some(CLAUDE_CODE_BACKEND.to_string()),
            app_thread_id: session_id.map(str::to_string),
            raw_kind: Some(raw_kind.to_string()),
            ..TurnStreamSource::default()
        },
        content_delta: None,
        message: None,
        tool_call: None,
        request_user_input: None,
        token_usage: None,
        error: None,
        error_code: None,
        details: None,
        phase: None,
        status: None,
    };
    fill(&mut event);
    event
}

fn tool_result_preview(block: &Value) -> Value {
    let text = match block.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    if text.chars().count() > TOOL_RESULT_PREVIEW_LIMIT {
        let truncated: String = text.chars().take(TOOL_RESULT_PREVIEW_LIMIT).collect();
        json!(format!("{truncated}\n… [truncated]"))
    } else {
        json!(text)
    }
}

/// `modelUsage` is cumulative per Claude session, so a turn's usage is the
/// per-model delta against the session's snapshot at turn start. Input
/// includes cache reads and creation; cached input is the cache reads. The
/// cumulative snapshot rides along as `model_usage` for the next resumed turn.
fn usage_payload_from_model_usage(model_usage: &Value, start: &Value) -> Value {
    let read = |entry: &Value, key: &str| entry.get(key).and_then(Value::as_f64).unwrap_or(0.0);
    let mut by_model = serde_json::Map::new();
    let (mut input, mut cached, mut creation, mut output, mut cost) = (0, 0, 0, 0, 0.0);
    for (model, current) in model_usage.as_object().into_iter().flatten() {
        let delta = |key| (read(current, key) - read(&start[model.as_str()], key)).max(0.0);
        let model_cached = delta("cacheReadInputTokens") as u64;
        let model_creation = delta("cacheCreationInputTokens") as u64;
        let model_input = delta("inputTokens") as u64 + model_cached + model_creation;
        let model_output = delta("outputTokens") as u64;
        let model_cost = delta("costUSD");
        if model_input + model_output == 0 && model_cost == 0.0 {
            continue;
        }
        input += model_input;
        cached += model_cached;
        creation += model_creation;
        output += model_output;
        cost += model_cost;
        by_model.insert(
            model.clone(),
            json!({
                "input_tokens": model_input,
                "cached_input_tokens": model_cached,
                "cache_creation_input_tokens": model_creation,
                "output_tokens": model_output,
                "total_tokens": model_input + model_output,
                "cost_usd": model_cost,
            }),
        );
    }
    json!({
        "input_tokens": input,
        "cached_input_tokens": cached,
        "cache_creation_input_tokens": creation,
        "output_tokens": output,
        "total_tokens": input + output,
        "cost_usd": cost,
        "by_model": by_model,
        "model_usage": model_usage,
    })
}

pub fn usage_from_claude_code_usage_payload(payload: &Value) -> Option<Usage> {
    let object = payload.as_object()?;
    let read = |key: &str| object.get(key).and_then(Value::as_u64);
    let usage = Usage {
        input_tokens: read("input_tokens").unwrap_or(0),
        output_tokens: read("output_tokens").unwrap_or(0),
        total_tokens: 0,
        reasoning_tokens: None,
        cache_read_tokens: read("cached_input_tokens"),
        cache_write_tokens: read("cache_creation_input_tokens"),
        raw: Some(payload.clone()),
    };
    Some(usage.normalized())
}

pub(crate) fn claude_code_executable() -> PathBuf {
    env::var(CLAUDE_CODE_BIN_ENV)
        .ok()
        .and_then(|value| non_empty(&value).map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("claude"))
}

/// The CLI as every Spark launch sees it: the configured binary and config
/// dir, which is where Claude Code keeps its sign-in.
fn claude_command(native: Option<&NativeAgentSettings>) -> (PathBuf, Command) {
    let executable = native
        .and_then(|config| config.claude_binary.as_ref())
        .map(PathBuf::from)
        .unwrap_or_else(claude_code_executable);
    let mut command = Command::new(&executable);
    // The agent's shell runs the spark CLI against this Spark, as Codex sessions do.
    command.envs(crate::codex_app_server::spark_agent_environment(
        env::var("PATH").ok().as_deref(),
    ));
    if let Some(config_dir) = match native {
        Some(config) => config.claude_config_dir.clone(),
        None => env::var(CLAUDE_CODE_CONFIG_DIR_ENV)
            .ok()
            .and_then(|value| non_empty(&value).map(str::to_string)),
    } {
        command.env("CLAUDE_CONFIG_DIR", config_dir);
    }
    (executable, command)
}

fn launch_error(executable: &Path, error: std::io::Error) -> ClaudeCodeError {
    if error.kind() == std::io::ErrorKind::NotFound {
        ClaudeCodeError::configuration(format!(
            "claude code executable not found: {} (install Claude Code and log in, \
             or set {CLAUDE_CODE_BIN_ENV})",
            executable.display()
        ))
    } else {
        ClaudeCodeError::runtime(format!(
            "claude code launch failed for {}: {error}",
            executable.display()
        ))
    }
}

fn permission_mode() -> String {
    env::var(CLAUDE_CODE_PERMISSION_MODE_ENV)
        .ok()
        .and_then(|value| non_empty(&value).map(str::to_string))
        .unwrap_or_else(|| DEFAULT_PERMISSION_MODE.to_string())
}

fn metadata_string(metadata: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(Value::as_str)
        .and_then(non_empty)
        .map(str::to_string)
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}
