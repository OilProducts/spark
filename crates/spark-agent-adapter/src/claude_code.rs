//! Claude Code CLI backend: runs an agent turn by driving `claude -p` with
//! stream-json output. Authentication comes from the Claude Code login on the
//! host (subscription or API key) — Spark never handles the credential.

use std::collections::{BTreeMap, VecDeque};
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};
use spark_common::events::{
    TurnStreamChannel, TurnStreamEvent, TurnStreamEventKind, TurnStreamSource,
};
use unified_llm_adapter::Usage;
use uuid::Uuid;

use crate::agent::{
    AgentRawLogLine, AgentThreadResumeFailure, AgentTurnEventSink, AgentTurnOutput,
    AgentTurnRequest,
};

pub const CLAUDE_CODE_BACKEND: &str = "claude_code_cli";

const CLAUDE_CODE_BIN_ENV: &str = "SPARK_CLAUDE_CODE_BIN";
const CLAUDE_CODE_PERMISSION_MODE_ENV: &str = "SPARK_CLAUDE_CODE_PERMISSION_MODE";
const CLAUDE_CODE_CONFIG_DIR_ENV: &str = "SPARK_CLAUDE_CODE_CONFIG_DIR";
const DEFAULT_PERMISSION_MODE: &str = "bypassPermissions";
const TOOL_RESULT_PREVIEW_LIMIT: usize = 4000;

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

    pub fn run_agent_turn(
        &self,
        request: AgentTurnRequest,
    ) -> Result<AgentTurnOutput, ClaudeCodeError> {
        self.run_agent_turn_with_event_sink(request, None)
    }

    pub fn run_agent_turn_with_event_sink(
        &self,
        mut request: AgentTurnRequest,
        event_sink: Option<AgentTurnEventSink>,
    ) -> Result<AgentTurnOutput, ClaudeCodeError> {
        let resumed_session_id =
            metadata_string(&request.metadata, "spark.runtime.claude_code.session_id")
                .or_else(|| metadata_string(&request.metadata, "claude_code.session_id"));
        match self.run_once(&request, event_sink.clone(), resumed_session_id.as_deref()) {
            Ok(output) => Ok(output),
            Err(error)
                if resumed_session_id.is_some()
                    && error.message.contains("without a result event") =>
            {
                request
                    .metadata
                    .remove("spark.runtime.claude_code.session_id");
                request.metadata.remove("claude_code.session_id");
                let mut output = self.run_once(&request, event_sink, None)?;
                let discarded_session_id = resumed_session_id.unwrap();
                output.thread_resume_failure = Some(AgentThreadResumeFailure {
                    message: format!(
                        "claude code could not resume session {discarded_session_id}; started a fresh session"
                    ),
                    error_code: Some("thread_resume_failure".to_string()),
                    details: Some(json!({"discarded_session_id": discarded_session_id})),
                });
                Ok(output)
            }
            Err(error) => Err(error),
        }
    }

    fn run_once(
        &self,
        request: &AgentTurnRequest,
        event_sink: Option<AgentTurnEventSink>,
        resume_session_id: Option<&str>,
    ) -> Result<AgentTurnOutput, ClaudeCodeError> {
        let working_dir = PathBuf::from(&request.project_path);
        if !working_dir.exists() {
            return Err(ClaudeCodeError::configuration(format!(
                "claude code working directory is unavailable in the runtime: {}",
                working_dir.display()
            )));
        }
        let executable = claude_code_executable();
        let mut command = Command::new(&executable);
        command
            .arg("-p")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--include-partial-messages")
            .arg("--verbose")
            .arg("--permission-mode")
            .arg(permission_mode())
            .current_dir(&working_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(model) = request.model.as_deref().and_then(non_empty) {
            command.arg("--model").arg(model);
        }
        if let Some(session_id) = resume_session_id {
            command.arg("--resume").arg(session_id);
        }
        if let Some(config_dir) = env::var(CLAUDE_CODE_CONFIG_DIR_ENV)
            .ok()
            .and_then(|value| non_empty(&value).map(str::to_string))
        {
            command.env("CLAUDE_CONFIG_DIR", config_dir);
        }

        let mut child = command.spawn().map_err(|error| {
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
        })?;

        // The prompt travels over stdin so arbitrarily large composed prompts
        // never hit argv limits.
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| ClaudeCodeError::runtime("claude code did not expose stdin"))?;
        stdin
            .write_all(request.prompt.as_bytes())
            .and_then(|()| stdin.flush())
            .map_err(|error| {
                ClaudeCodeError::runtime(format!("claude code prompt write failed: {error}"))
            })?;
        drop(stdin);

        crate::initial_context::capture_if_configured(&request.metadata, &request.prompt).map_err(
            |error| {
                ClaudeCodeError::configuration(format!("initial context capture failed: {error}"))
            },
        )?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ClaudeCodeError::runtime("claude code did not expose stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| ClaudeCodeError::runtime("claude code did not expose stderr"))?;
        let stderr_handle = std::thread::spawn(move || {
            BufReader::new(stderr)
                .lines()
                .map_while(Result::ok)
                .collect::<Vec<_>>()
        });

        let app_turn_id = Uuid::new_v4().to_string();
        let mut turn = ClaudeCodeTurnState::default();
        let mut events: Vec<TurnStreamEvent> = Vec::new();
        let mut raw_log_lines: Vec<AgentRawLogLine> = Vec::new();
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            raw_log_lines.push(AgentRawLogLine {
                direction: "stdout".to_string(),
                line: line.clone(),
            });
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            for mut event in turn.ingest(&message) {
                event.source.app_turn_id = Some(app_turn_id.clone());
                if let Some(sink) = &event_sink {
                    sink(event.clone());
                }
                events.push(event);
            }
        }

        let status = child.wait().map_err(|error| {
            ClaudeCodeError::runtime(format!("claude code wait failed: {error}"))
        })?;
        let stderr_lines = stderr_handle.join().unwrap_or_default();

        if turn.result_payload.is_none() {
            let stderr_tail = stderr_lines
                .iter()
                .rev()
                .take(10)
                .rev()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            return Err(ClaudeCodeError {
                message: format!(
                    "claude code exited ({status}) without a result event; stderr tail:\n{stderr_tail}"
                ),
                retryable: true,
                details: None,
            });
        }
        if turn.is_error {
            return Err(ClaudeCodeError {
                message: format!(
                    "claude code turn failed ({}): {}",
                    turn.result_subtype.as_deref().unwrap_or("error"),
                    turn.resolved_final_text()
                ),
                retryable: false,
                details: turn.result_payload.clone(),
            });
        }

        Ok(AgentTurnOutput {
            app_thread_id: turn.session_id.clone(),
            app_turn_id: Some(app_turn_id),
            final_assistant_text: non_empty(&turn.resolved_final_text()).map(str::to_string),
            token_usage: turn.usage_payload.clone(),
            token_usage_breakdown: turn.usage_payload.clone(),
            events,
            raw_log_lines,
            thread_resume_failure: None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeCodeModelMetadata {
    pub id: String,
    pub display: String,
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
    let executable = claude_code_executable();
    let mut command = Command::new(&executable);
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
    if let Some(config_dir) = env::var(CLAUDE_CODE_CONFIG_DIR_ENV)
        .ok()
        .and_then(|value| non_empty(&value).map(str::to_string))
    {
        command.env("CLAUDE_CONFIG_DIR", config_dir);
    }

    let mut child = command.spawn().map_err(|error| {
        log_model_discovery_error(if error.kind() == std::io::ErrorKind::NotFound {
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
        })
    })?;

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
            Some(ClaudeCodeModelMetadata {
                id: id.to_string(),
                display: display.to_string(),
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
    is_error: bool,
    block_counter: usize,
    partial_block_ids: BTreeMap<usize, String>,
    pending_text_block_ids: VecDeque<String>,
    pending_reasoning_block_ids: VecDeque<String>,
    last_text_item_id: Option<String>,
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

    fn ingest(&mut self, message: &Value) -> Vec<TurnStreamEvent> {
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
            "result" => self.ingest_result(message),
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

    fn ingest_result(&mut self, message: &Value) -> Vec<TurnStreamEvent> {
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
            let item_id = self.last_text_item_id.clone().unwrap_or_else(|| {
                self.block_counter += 1;
                format!("block-{}", self.block_counter)
            });
            events.push(stream_event(
                TurnStreamEventKind::ContentCompleted,
                self.session_id.as_deref(),
                "result_text",
                |event| {
                    event.channel = Some(TurnStreamChannel::Assistant);
                    event.content_delta = Some(text.to_string());
                    event.message = Some(text.to_string());
                    event.source.item_id = Some(item_id);
                    event.phase = Some("final_answer".to_string());
                },
            ));
        }
        if let Some(usage) = message.get("usage").filter(|value| !value.is_null()) {
            let mut usage_payload = usage.clone();
            if let Some(object) = usage_payload.as_object_mut() {
                for key in ["total_cost_usd", "num_turns", "duration_ms"] {
                    if let Some(value) = message.get(key) {
                        object.insert(key.to_string(), value.clone());
                    }
                }
            }
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

pub fn usage_from_claude_code_usage_payload(payload: &Value) -> Option<Usage> {
    let object = payload.as_object()?;
    let read = |key: &str| object.get(key).and_then(Value::as_u64);
    let usage = Usage {
        input_tokens: read("input_tokens").unwrap_or(0),
        output_tokens: read("output_tokens").unwrap_or(0),
        total_tokens: 0,
        reasoning_tokens: None,
        cache_read_tokens: read("cache_read_input_tokens"),
        cache_write_tokens: read("cache_creation_input_tokens"),
        raw: Some(payload.clone()),
    };
    Some(usage.normalized())
}

fn claude_code_executable() -> PathBuf {
    env::var(CLAUDE_CODE_BIN_ENV)
        .ok()
        .and_then(|value| non_empty(&value).map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("claude"))
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
