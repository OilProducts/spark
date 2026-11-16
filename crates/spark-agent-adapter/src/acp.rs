//! Stable ACP v1 client for registered harnesses (`acp:<id>`), driven by the
//! pinned SDK over the shared stdio transport.
//!
//! One process per turn: initialize, create or resume the session, apply
//! required selections, then exactly one primary prompt. The SDK connection
//! runs on the transport's worker; events reach the caller's sink on the
//! calling thread, in order, and are returned exactly as delivered.

use std::collections::{BTreeMap, HashMap};
use std::io::Write as _;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::util::MatchDispatch;
use agent_client_protocol::v1::schema::{
    CancelNotification, ContentBlock, ContentChunk, Implementation, InitializeRequest,
    LoadSessionRequest, NewSessionRequest, PermissionOptionKind, PromptResponse,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    ResumeSessionRequest, SelectedPermissionOutcome, SessionConfigKind, SessionConfigOption,
    SessionConfigOptionCategory, SessionConfigOptionValue, SessionConfigSelectOptions,
    SessionNotification, SessionUpdate, SetSessionConfigOptionRequest, StopReason, SystemPrompt,
};
use agent_client_protocol::v1::{ActiveSession, ConnectionTo, SessionMessage, SessionSetup};
use agent_client_protocol::{Agent, Client, ErrorCode, Lines};
use futures_util::FutureExt as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use spark_common::agent_settings::{AcpHarness, AcpPermissionPolicy};
use spark_common::events::{
    TurnStreamChannel, TurnStreamEvent, TurnStreamEventKind, TurnStreamSource,
};
use tokio::sync::mpsc::{error::TrySendError, Receiver, Sender};
use tokio::time::Instant;

use crate::agent::{
    AgentRawLogLine, AgentTurnEventSink, AgentTurnOutput, AgentTurnRequest,
    AGENT_INSTRUCTIONS_METADATA_KEY,
};
use crate::codex_app_server::json_len;
use crate::history::HistoryTurn;
use crate::stdio_transport::{
    self, sleep_until, Budget, Endpoints, Held, Incoming, Outgoing, Stop, LIMITS,
};

pub const ACP_BACKEND: &str = "acp";
/// The session a turn continues, and the one it leaves for the next turn.
pub const ACP_SESSION_METADATA_KEY: &str = "spark.runtime.acp.session";
/// Emitted once a session is set up; its details are the session record.
pub const ACP_SESSION_ESTABLISHED: &str = "session_established";
/// How long a cancelled prompt may take to settle before its process is stopped.
const CANCEL_GRACE: Duration = Duration::from_secs(5);
/// Bound on retained diagnostics (unknown updates, policy refusals).
const MAX_LOG_LINES: usize = 200;
const MAX_LOG_LINE_BYTES: usize = 2_000;

/// Everything a later turn needs to resume a session, and to tell whether
/// it still may: the same harness, work area, configuration and instructions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpSessionRecord {
    pub harness: String,
    pub session_id: String,
    pub cwd: String,
    pub configuration_generation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions_sha256: Option<String>,
}

impl AcpSessionRecord {
    pub fn from_metadata(metadata: &BTreeMap<String, Value>) -> Option<Self> {
        serde_json::from_value(metadata.get(ACP_SESSION_METADATA_KEY)?.clone()).ok()
    }

    pub fn to_metadata(&self) -> BTreeMap<String, Value> {
        BTreeMap::from([(ACP_SESSION_METADATA_KEY.to_string(), json!(self))])
    }
}

/// A turn that failed. Once its primary prompt was sent, the harness may
/// have done the work, so it must never be resent automatically.
#[derive(Debug, Default)]
pub struct AcpFailure {
    pub message: String,
    pub prompted: bool,
    /// Every event the sink was given before the failure.
    pub events: Vec<TurnStreamEvent>,
}

/// A settled primary prompt.
pub struct AcpTurn {
    pub output: AgentTurnOutput,
    pub stop_reason: StopReason,
    /// The session the next turn may resume: none when the turn stopped
    /// before setup, or when a cancelled prompt had to be stopped.
    pub record: Option<AcpSessionRecord>,
}

// ---- live turns ----------------------------------------------------------

type Key = (String, String);

/// A running turn as other threads reach it.
struct Live {
    /// Holds one pending interrupt; more add nothing.
    interrupts: Sender<()>,
    stop: Mutex<Option<Stop>>,
    terminated: AtomicBool,
    closed: AtomicBool,
}

fn registry() -> &'static Mutex<HashMap<Key, Weak<Live>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<Key, Weak<Live>>>> = OnceLock::new();
    REGISTRY.get_or_init(Mutex::default)
}

fn find(project_path: &str, conversation_id: &str) -> Option<Arc<Live>> {
    registry()
        .lock()
        .unwrap()
        .get(&(project_path.to_string(), conversation_id.to_string()))
        .and_then(Weak::upgrade)
        .filter(|live| !live.closed.load(Ordering::SeqCst))
}

/// Removes the turn from the registry on every exit path.
struct Registered(Key, Arc<Live>);

impl Drop for Registered {
    fn drop(&mut self) {
        self.1.closed.store(true, Ordering::SeqCst);
        let mut entries = registry().lock().unwrap();
        if entries
            .get(&self.0)
            .is_some_and(|entry| entry.as_ptr() == Arc::as_ptr(&self.1))
        {
            entries.remove(&self.0);
        }
    }
}

/// Sends `session/cancel` once the prompt is out, or skips the prompt.
pub fn interrupt(project_path: &str, conversation_id: &str) -> bool {
    find(project_path, conversation_id)
        .is_some_and(|live| !matches!(live.interrupts.try_send(()), Err(TrySendError::Closed(_))))
}

/// Stops the turn's process group directly.
pub fn terminate(project_path: &str, conversation_id: &str) -> bool {
    let Some(live) = find(project_path, conversation_id) else {
        return false;
    };
    live.terminated.store(true, Ordering::SeqCst);
    if let Some(stop) = live.stop.lock().unwrap().as_ref() {
        stop.stop();
    }
    true
}

// ---- the turn ------------------------------------------------------------

/// What the worker hands the calling thread, in order. An event holds its
/// share of the sink's queue until the sink has returned.
enum Message {
    Event(TurnStreamEvent, Option<Held>),
    /// The primary prompt is about to be sent.
    Prompted,
    Done(Result<Finished, String>),
}

struct Finished {
    stop_reason: StopReason,
    final_text: Option<String>,
    usage: Option<Value>,
    /// None when the turn was interrupted before its session was set up.
    record: Option<AcpSessionRecord>,
    reset: Option<String>,
    forced: bool,
    logs: Vec<AgentRawLogLine>,
}

/// What the worker needs, fixed before launch.
struct Plan {
    harness_id: String,
    policy: AcpPermissionPolicy,
    cwd: PathBuf,
    generation: String,
    instructions: Option<String>,
    instructions_sha256: Option<String>,
    /// The registration's channel carries the instructions; never `systemPrompt`.
    channel_rendered: bool,
    /// Required config selections: the model, the effort, then the channel's.
    selections: Vec<(Selector, String)>,
    saved: Option<AcpSessionRecord>,
    prompt: String,
    /// Earlier context for a session that starts fresh.
    fresh_context: Option<String>,
}

#[derive(Clone, Debug)]
enum Selector {
    Category(SessionConfigOptionCategory, &'static str),
    Id(String),
}

/// Runs one primary prompt with `harness_id`'s registration, as captured in
/// the request's execution settings.
pub fn run_turn(
    request: &AgentTurnRequest,
    harness_id: &str,
    sink: Option<&AgentTurnEventSink>,
) -> Result<AcpTurn, AcpFailure> {
    let mut seen = AcpFailure::default();
    let result = turn(request, harness_id, sink, &mut seen);
    result.map_err(|message| AcpFailure { message, ..seen })
}

/// `harness_id`'s current registration in the settings the work was captured
/// from, if they can be read.
// ponytail: a worker that cannot read the captured settings path, such as one
// in a container, cannot restore harness environments; pass them in if needed.
fn live_harness(request: &AgentTurnRequest, harness_id: &str) -> Option<AcpHarness> {
    let path = std::path::Path::new(
        request.metadata.get("spark.execution.settings")?["configuration"]["config_dir"]
            .as_str()?,
    )
    .join("spark.toml");
    let agents: crate::config::SessionConfig =
        spark_storage::settings::read_settings_document(&path)
            .ok()?
            .section(&path, "agents")
            .ok()??;
    agents.acp_harnesses.get(harness_id).cloned()
}

/// The configuration generation a launch of `harness` records: its
/// registration's, plus the values it inherits now, so a changed inherited
/// environment starts fresh. Only this hash is kept, never the values.
pub fn launch_generation(
    harness: &AcpHarness,
    inheritance: spark_common::agent_settings::EnvironmentInheritancePolicy,
) -> String {
    let inherited = crate::local_environment::inherited_environment(inheritance);
    spark_common::project::sha256_hex(
        json!([harness.generation(inheritance), inherited])
            .to_string()
            .as_bytes(),
    )
}

/// Reasoning controls a turn may carry; ACP v1 has no portable equivalent.
const REASONING_CONTROLS: [&str; 4] = [
    "thinking",
    "thinking_budget_tokens",
    "reasoning_mode",
    "reasoning_summary",
];

fn turn(
    request: &AgentTurnRequest,
    harness_id: &str,
    sink: Option<&AgentTurnEventSink>,
    seen: &mut AcpFailure,
) -> Result<AcpTurn, String> {
    let config = crate::config::captured_session_config(&request.metadata)?.unwrap_or_default();
    let harness: AcpHarness = config
        .acp_harnesses
        .get(harness_id)
        .cloned()
        .ok_or_else(|| {
            format!("ACP harness {harness_id} is not registered in the agents settings.")
        })?
        .restore_env(
            live_harness(request, harness_id).as_ref(),
            config.environment_inheritance,
        )
        .map_err(|error| format!("ACP harness {harness_id}: {error}"))?;
    if request
        .chat_mode
        .as_deref()
        .is_some_and(|mode| mode.trim().eq_ignore_ascii_case("plan"))
    {
        return Err(format!(
            "Plan mode is unavailable for ACP harness {harness_id}: no read-only mode is verified for it."
        ));
    }
    // Refused, never silently ignored: a flow node's own, else the chat's.
    let reasoning = request
        .metadata
        .get("spark.execution.reasoning")
        .or_else(|| {
            request
                .metadata
                .get("spark.execution.settings")?
                .get("model_settings")
        });
    if let Some((control, value)) = reasoning.and_then(|settings| {
        REASONING_CONTROLS
            .into_iter()
            .find_map(|control| Some((control, settings.get(control).filter(|v| !v.is_null())?)))
    }) {
        return Err(format!(
            "ACP harness {harness_id} has no {control} control, so {control} {value} cannot be applied; clear it or choose another harness."
        ));
    }
    let cwd = std::fs::canonicalize(&request.project_path).map_err(|error| {
        format!(
            "ACP harness {harness_id} work area {} is unavailable: {error}",
            request.project_path
        )
    })?;
    // Exact bytes: never trimmed, so the fingerprint is the chat's own.
    let instructions = request
        .metadata
        .get(AGENT_INSTRUCTIONS_METADATA_KEY)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    let channel = harness
        .system_instructions
        .as_ref()
        .filter(|_| instructions.is_some());
    let instruction_file = channel
        .filter(|channel| channel.uses_file())
        .map(|_| {
            let mut file = tempfile::NamedTempFile::new()?;
            file.write_all(instructions.as_deref().unwrap_or_default().as_bytes())?;
            file.flush()?;
            Ok::<_, std::io::Error>(file)
        })
        .transpose()
        .map_err(|error| {
            format!("ACP harness {harness_id} instruction file write failed: {error}")
        })?;
    let (channel_env, channel_args) = match channel {
        Some(channel) => channel
            .render(
                instructions.as_deref().unwrap_or_default(),
                &instruction_file
                    .as_ref()
                    .map(|file| file.path().to_string_lossy().into_owned())
                    .unwrap_or_default(),
            )
            .map_err(|error| format!("ACP harness {harness_id} system_instructions: {error}"))?,
        None => Default::default(),
    };

    let mut selections = Vec::new();
    if let Some(model) = request
        .model
        .as_deref()
        .filter(|model| !model.trim().is_empty())
    {
        selections.push((
            Selector::Category(SessionConfigOptionCategory::Model, "model"),
            model.to_string(),
        ));
    }
    if let Some(effort) = request
        .reasoning_effort
        .as_deref()
        .filter(|effort| !effort.trim().is_empty())
    {
        selections.push((
            Selector::Category(
                SessionConfigOptionCategory::ThoughtLevel,
                "reasoning effort",
            ),
            effort.to_string(),
        ));
    }
    for (id, value) in channel.map(|channel| &channel.config).into_iter().flatten() {
        selections.push((Selector::Id(id.clone()), value.clone()));
    }

    let attachments = crate::attachments::request_attachments(&request.metadata);
    let earlier = crate::attachments::history_attachments(&request.history);
    let plan = Plan {
        harness_id: harness_id.to_string(),
        policy: harness.permission_policy,
        generation: launch_generation(&harness, config.environment_inheritance),
        instructions_sha256: instructions
            .as_deref()
            .map(|text| spark_common::project::sha256_hex(text.as_bytes())),
        instructions,
        channel_rendered: channel.is_some(),
        selections,
        saved: AcpSessionRecord::from_metadata(&request.metadata),
        prompt: crate::attachments::message_with_attachment_references(
            &request.prompt,
            &attachments,
        ),
        fresh_context: fresh_context(&request.history, &earlier),
        cwd: cwd.clone(),
    };

    // Spark's reserved bindings last, so nothing overrides them.
    let mut command = Command::new(&harness.executable);
    command
        .args(&harness.args)
        .args(channel_args)
        .current_dir(&cwd)
        .env_clear()
        .envs(crate::local_environment::inherited_environment(
            config.environment_inheritance,
        ))
        .envs(&harness.env)
        .envs(channel_env)
        .envs(crate::agent::agent_turn_env(&request.metadata));

    let (interrupts, interrupted) = tokio::sync::mpsc::channel(1);
    let live = Arc::new(Live {
        interrupts,
        stop: Mutex::new(None),
        terminated: AtomicBool::new(false),
        closed: AtomicBool::new(false),
    });
    let key = (
        request.project_path.clone(),
        request.conversation_id.clone(),
    );
    let _registered = {
        let mut entries = registry().lock().unwrap();
        if entries
            .get(&key)
            .and_then(Weak::upgrade)
            .is_some_and(|other| !other.closed.load(Ordering::SeqCst))
        {
            return Err("An ACP turn is already running for this conversation.".into());
        }
        entries.insert(key.clone(), Arc::downgrade(&live));
        Registered(key, Arc::clone(&live))
    };

    let (messages, delivered) = std::sync::mpsc::channel();
    let out = Out {
        messages,
        sink: sink.map(|_| Budget::new(&LIMITS)),
        overflowed: AtomicBool::new(false),
    };
    let connection = stdio_transport::launch(command, LIMITS, move |endpoints| {
        drive(endpoints, plan, out, interrupted)
    })
    .map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            format!(
                "ACP harness {harness_id} executable not found: {}",
                harness.executable
            )
        } else {
            format!("ACP harness {harness_id} launch failed: {error}")
        }
    })?;
    {
        let stop = connection.stop_handle();
        if live.terminated.load(Ordering::SeqCst) {
            stop.stop();
        }
        *live.stop.lock().unwrap() = Some(stop);
    }

    // The sink runs here, off the worker, which keeps servicing the connection.
    let app_turn_id = uuid::Uuid::new_v4().to_string();
    let mut done = None;
    for message in delivered {
        match message {
            Message::Event(mut event, _held) => {
                event.source.app_turn_id = Some(app_turn_id.clone());
                if let Some(sink) = sink {
                    sink(event.clone());
                }
                seen.events.push(event);
            }
            Message::Prompted => seen.prompted = true,
            Message::Done(result) => {
                done = Some(result);
                break;
            }
        }
    }
    let finished = match done {
        Some(Ok(finished)) => finished,
        failed => {
            let stderr = connection.stderr();
            let lines = stderr.lines().collect::<Vec<_>>();
            let tail = lines[lines.len().saturating_sub(10)..].join("\n");
            let message = match failed {
                Some(Err(message)) => message,
                _ => "the ACP connection worker stopped".to_string(),
            };
            return Err(if tail.is_empty() {
                format!("ACP harness {harness_id}: {message}")
            } else {
                format!("ACP harness {harness_id}: {message}; stderr tail:\n{tail}")
            });
        }
    };
    drop(connection);
    drop(instruction_file);
    // A stopped process leaves its session in an unknown state: never resume it.
    let record = finished.record.filter(|_| !finished.forced);
    let reset = match finished.forced {
        true => Some(format!(
            "ACP harness {harness_id} did not settle the cancelled prompt within {}s, so it was stopped; the next turn starts a fresh session",
            CANCEL_GRACE.as_secs()
        )),
        false => finished.reset,
    };
    let thread_resume_failure = reset.map(|message| crate::agent::AgentThreadResumeFailure {
        message,
        error_code: Some("thread_resume_failure".into()),
        details: Some(json!({"harness": harness_id})),
    });
    Ok(AcpTurn {
        output: AgentTurnOutput {
            events: std::mem::take(&mut seen.events),
            app_thread_id: record.as_ref().map(|record| record.session_id.clone()),
            app_turn_id: Some(app_turn_id),
            final_assistant_text: finished.final_text,
            token_usage: finished.usage.clone(),
            token_usage_breakdown: finished.usage,
            raw_log_lines: finished.logs,
            thread_resume_failure,
        },
        stop_reason: finished.stop_reason,
        record,
    })
}

/// Earlier conversation as ordinary context for a session that starts
/// fresh. Pinned instructions are never part of it.
fn fresh_context(
    history: &[HistoryTurn],
    earlier: &[crate::attachments::AgentAttachment],
) -> Option<String> {
    let mut lines = Vec::new();
    for turn in history {
        let (who, text) = match turn {
            HistoryTurn::User(turn) => ("User", turn.text()),
            HistoryTurn::Assistant(turn) => ("Assistant", turn.text()),
            _ => continue,
        };
        if !text.trim().is_empty() {
            lines.push(format!("{who}: {text}"));
        }
    }
    let mut context = String::new();
    if !lines.is_empty() {
        context = format!(
            "Earlier in this conversation (context only; reply to the new message below):\n\n{}",
            lines.join("\n\n")
        );
    }
    let context = crate::attachments::prompt_with_attachment_references(&context, &[], earlier);
    (!context.is_empty()).then_some(context)
}

// ---- the worker ----------------------------------------------------------

/// Sends events and the result to the calling thread, never waiting on the
/// sink: a sink a full queue behind fails the turn instead.
struct Out {
    messages: std::sync::mpsc::Sender<Message>,
    /// The sink's queue, if the caller has a sink.
    sink: Option<Budget>,
    overflowed: AtomicBool,
}

impl Out {
    fn event(&self, event: TurnStreamEvent) {
        let held = match &self.sink {
            Some(_) if self.overflowed.load(Ordering::SeqCst) => return,
            Some(budget) => match budget.try_admit(json_len(&event)) {
                Some(held) => Some(held),
                None => return self.overflowed.store(true, Ordering::SeqCst),
            },
            None => None,
        };
        let _ = self.messages.send(Message::Event(event, held));
    }

    fn overloaded(&self) -> Result<(), String> {
        match self.overflowed.load(Ordering::SeqCst) {
            true => Err("overloaded: the event queue is full".to_string()),
            false => Ok(()),
        }
    }
}

/// The first reason the harness's output became unusable.
type Failure = Arc<OnceLock<String>>;

/// The transport's stdout as the SDK's line stream: a read failure or a line
/// that is not a JSON-RPC message is an error, EOF or drained output ends it.
fn incoming(
    stdout: Incoming,
    failure: Failure,
) -> impl futures_util::Stream<Item = std::io::Result<String>> {
    futures_util::stream::unfold(Some(stdout), move |stdout| {
        let failure = Arc::clone(&failure);
        async move {
            let mut stdout = stdout?;
            let error = match stdout.next().await {
                // ponytail: the SDK parses each line again; one parse would need our own transport.
                Some(line) => match agent_client_protocol::TransportFrame::parse_json(&line.text) {
                    agent_client_protocol::TransportFrame::Malformed { error, .. } => format!(
                        "the harness wrote malformed protocol output: {}",
                        error.message
                    ),
                    _ => return Some((Ok(line.text), Some(stdout))),
                },
                None => match stdout.end() {
                    Some(stdio_transport::End::Failed(error)) => error.clone(),
                    _ => return None,
                },
            };
            let _ = failure.set(error.clone());
            Some((Err(std::io::Error::other(error)), None))
        }
    })
}

/// The transport's stdin as the SDK's line sink: each line is written and
/// flushed before the next is taken, and a refused or failed write is an error.
fn outgoing(stdin: Outgoing) -> impl futures_util::Sink<String, Error = std::io::Error> {
    futures_util::sink::unfold(stdin, |stdin, line: String| async move {
        let written = stdin.send(line).map_err(|refused| {
            std::io::Error::other(format!("stdin refused the line: {refused:?}"))
        })?;
        written
            .await
            .map_err(|_| std::io::Error::other("the stdin writer stopped"))??;
        Ok(stdin)
    })
}

async fn drive(endpoints: Endpoints, plan: Plan, out: Out, interrupts: Receiver<()>) {
    let Endpoints {
        stdout,
        stdin,
        status: _,
        stop,
    } = endpoints;
    let failure = Failure::default();
    let transport = Lines::new(
        Box::pin(outgoing(stdin)),
        Box::pin(incoming(stdout, Arc::clone(&failure))),
    );
    let result = Client
        .v1()
        .name("spark")
        .connect_with(transport, async |cx| {
            Ok(Turn::new(&plan, &out, stop).run(cx, interrupts).await)
        })
        .await
        .unwrap_or_else(|error| Err(describe(&error)));
    // Unusable output explains a failed turn better than the closed connection;
    // a sink that missed events never sees the turn succeed.
    let result = result
        .map_err(|error| failure.get().cloned().unwrap_or(error))
        .and_then(|finished| out.overloaded().map(|()| finished));
    let _ = out.messages.send(Message::Done(result));
}

fn describe(error: &agent_client_protocol::Error) -> String {
    if agent_client_protocol::is_incoming_transport_closed(error)
        || agent_client_protocol::is_connection_terminated(error)
    {
        "the harness closed the connection".to_string()
    } else {
        match &error.data {
            Some(data) => format!("{} ({}): {data}", error.message, error.code.value()),
            None => format!("{} ({})", error.message, error.code.value()),
        }
    }
}

fn is_auth_error(error: &agent_client_protocol::Error) -> bool {
    error.code == ErrorCode::AUTH_REQUIRED
}

struct Turn<'a> {
    plan: &'a Plan,
    out: &'a Out,
    stop: Stop,
    translate: Translate,
}

type Session = ActiveSession<'static, Agent>;

/// A session ready for the primary prompt.
struct Ready {
    session: Session,
    record: AcpSessionRecord,
    reset: Option<String>,
    resumed: bool,
}

impl<'a> Turn<'a> {
    fn new(plan: &'a Plan, out: &'a Out, stop: Stop) -> Self {
        Self {
            plan,
            out,
            stop,
            translate: Translate::new(&plan.harness_id),
        }
    }

    async fn run(
        mut self,
        cx: ConnectionTo<Agent>,
        mut interrupts: Receiver<()>,
    ) -> Result<Finished, String> {
        // An interrupt before the prompt is sent skips the prompt.
        let ready = tokio::select! {
            biased;
            Some(()) = interrupts.recv() => None,
            ready = self.setup(&cx) => Some(ready?),
        };
        let Some(Ready {
            mut session,
            record,
            reset,
            resumed,
        }) = ready
        else {
            return Ok(self.finish(StopReason::Cancelled, None, None, None, false));
        };
        let mut text = self.plan.prompt.clone();
        if let Some(context) = self.plan.fresh_context.as_ref().filter(|_| !resumed) {
            text = format!("{context}\n\n---\n\n{text}");
        }
        let _ = self.out.messages.send(Message::Prompted);
        session
            .send_prompt(text)
            .map_err(|error| format!("session/prompt failed: {}", describe(&error)))?;
        self.prompt(&mut session, interrupts, record, reset).await
    }

    /// Initializes, then resumes, loads or creates the session and applies
    /// the required selections.
    async fn setup(&mut self, cx: &ConnectionTo<Agent>) -> Result<Ready, String> {
        let plan = self.plan;
        let id = &plan.harness_id;
        let init = cx
            .send_request(
                InitializeRequest::new(ProtocolVersion::V1)
                    .client_info(Implementation::new("spark", env!("CARGO_PKG_VERSION"))),
            )
            .block_task()
            .await
            .map_err(|error| format!("initialize failed: {}", describe(&error)))?;
        if init.protocol_version != ProtocolVersion::V1 {
            return Err(format!(
                "the harness negotiated ACP protocol version {}, but Spark speaks version 1",
                init.protocol_version.as_u16()
            ));
        }
        let capabilities = init.agent_capabilities;
        let system_prompt = match &plan.instructions {
            Some(text) if !plan.channel_rendered => {
                if capabilities.session_capabilities.system_prompt.is_none() {
                    return Err(format!(
                        "this work carries pinned system instructions, but ACP harness {id} \
                         advertises no systemPrompt and its registration declares no \
                         system_instructions channel"
                    ));
                }
                Some(SystemPrompt::new(vec![ContentBlock::from(text.clone())]))
            }
            _ => None,
        };

        let current = AcpSessionRecord {
            harness: id.clone(),
            session_id: String::new(),
            cwd: plan.cwd.to_string_lossy().into_owned(),
            configuration_generation: plan.generation.clone(),
            instructions_sha256: plan.instructions_sha256.clone(),
        };
        let mut reset = None;
        let mut restored = None;
        if let Some(saved) = &plan.saved {
            let matches = AcpSessionRecord {
                session_id: saved.session_id.clone(),
                ..current.clone()
            } == *saved;
            let restore = if !matches {
                Err("its harness, work area, configuration or instructions changed".to_string())
            } else if capabilities.session_capabilities.resume.is_some() {
                let request = ResumeSessionRequest::new(saved.session_id.clone(), &plan.cwd)
                    .system_prompt(system_prompt.clone());
                match cx
                    .resume_session_from(request)
                    .block_task()
                    .start_session()
                    .await
                {
                    Ok(SessionSetup {
                        session, response, ..
                    }) => Ok((session, response.config_options.unwrap_or_default())),
                    Err(error) => Err(reset_reason(&error, "resume")?),
                }
            } else if capabilities.load_session {
                let request = LoadSessionRequest::new(saved.session_id.clone(), &plan.cwd)
                    .system_prompt(system_prompt.clone());
                match cx
                    .load_session_from(request)
                    .block_task()
                    .start_session()
                    .await
                {
                    Ok(SessionSetup {
                        mut session,
                        response,
                        ..
                    }) => {
                        // Replay arrived before the response: none of it is new.
                        while session.read_update().now_or_never().is_some() {}
                        Ok((session, response.config_options.unwrap_or_default()))
                    }
                    Err(error) => Err(reset_reason(&error, "load")?),
                }
            } else {
                Err("the harness cannot resume or load sessions".to_string())
            };
            match restore {
                Ok(setup) => restored = Some(setup),
                Err(reason) => {
                    let message = format!(
                        "ACP harness {id} could not continue session {}: {reason}; started a fresh session",
                        saved.session_id
                    );
                    self.out.event(self.translate.event(
                        TurnStreamEventKind::Other("continuity_reset".into()),
                        "continuity_reset",
                        |event| {
                            event.message = Some(message.clone());
                            event.error_code = Some("thread_resume_failure".into());
                        },
                    ));
                    reset = Some(message);
                }
            }
        }
        let resumed = restored.is_some();
        let (session, mut options) = match restored {
            Some(setup) => setup,
            None => {
                let request =
                    NewSessionRequest::new(&plan.cwd).system_prompt(system_prompt.clone());
                cx.build_session_from(request)
                    .block_task()
                    .start_session()
                    .await
                    .map(
                        |SessionSetup {
                             session, response, ..
                         }| {
                            (session, response.config_options.unwrap_or_default())
                        },
                    )
                    .map_err(|error| setup_error(&error, "session/new"))?
            }
        };
        let record = AcpSessionRecord {
            session_id: session.session_id().0.to_string(),
            ..current
        };
        self.translate.session_id = Some(record.session_id.clone());
        self.out.event(self.translate.event(
            TurnStreamEventKind::Other(ACP_SESSION_ESTABLISHED.into()),
            ACP_SESSION_ESTABLISHED,
            |event| {
                event.details = Some(json!(record));
                event.status = Some(if resumed { "resumed" } else { "new" }.into());
            },
        ));

        self.select(&session, &mut options).await?;
        Ok(Ready {
            session,
            record,
            reset,
            resumed,
        })
    }

    /// Applies each required selection against the harness's current options,
    /// using the full state each acknowledgement returns.
    async fn select(
        &mut self,
        session: &Session,
        options: &mut Vec<SessionConfigOption>,
    ) -> Result<(), String> {
        let mut chosen = BTreeMap::<String, String>::new();
        for (selector, value) in self.plan.selections.clone() {
            let option = options
                .iter()
                .find(|option| match &selector {
                    Selector::Category(category, _) => option.category.as_ref() == Some(category),
                    Selector::Id(id) => option.id.0.as_ref() == id,
                })
                .ok_or_else(|| match &selector {
                    Selector::Category(_, label) => format!(
                        "the harness offers no {label} selection, so {label} {value:?} cannot be applied"
                    ),
                    Selector::Id(id) => format!(
                        "the harness offers no config option {id:?} required by its system_instructions"
                    ),
                })?;
            let config_id = option.id.0.to_string();
            let SessionConfigKind::Select(select) = &option.kind else {
                return Err(format!(
                    "config option {config_id:?} is not a select option"
                ));
            };
            let offered = match &select.options {
                SessionConfigSelectOptions::Ungrouped(options) => options
                    .iter()
                    .any(|option| option.value.0.as_ref() == value),
                SessionConfigSelectOptions::Grouped(groups) => groups
                    .iter()
                    .flat_map(|group| &group.options)
                    .any(|option| option.value.0.as_ref() == value),
                _ => false,
            };
            if !offered {
                return Err(format!(
                    "config option {config_id:?} does not offer the value {value:?}"
                ));
            }
            if let Some(previous) = chosen
                .get(&config_id)
                .filter(|previous| **previous != value)
            {
                return Err(format!(
                    "config option {config_id:?} is required as both {previous:?} and {value:?}"
                ));
            }
            if select.current_value.0.as_ref() != value {
                let response = session
                    .connection()
                    .send_request(SetSessionConfigOptionRequest::new(
                        session.session_id().clone(),
                        config_id.clone(),
                        SessionConfigOptionValue::value_id(value.clone()),
                    ))
                    .block_task()
                    .await
                    .map_err(|error| {
                        format!(
                            "setting config option {config_id:?} failed: {}",
                            describe(&error)
                        )
                    })?;
                *options = response.config_options;
            }
            chosen.insert(config_id, value);
        }
        Ok(())
    }

    /// Runs the primary prompt to its response, servicing updates, permission
    /// requests and interrupts meanwhile.
    async fn prompt(
        mut self,
        session: &mut Session,
        mut interrupts: Receiver<()>,
        record: AcpSessionRecord,
        reset: Option<String>,
    ) -> Result<Finished, String> {
        let connection = session.connection().clone();
        let session_id = session.session_id().clone();
        let mut cancelled = false;
        let mut cancel_deadline = None;
        let mut forced = false;
        loop {
            tokio::select! {
                update = session.read_update() => match update {
                    Ok(SessionMessage::Message(dispatch)) => {
                        let (translate, out, policy) = (&mut self.translate, self.out, self.plan.policy);
                        MatchDispatch::new(dispatch.into_raw())
                            .if_notification(async |notification: SessionNotification| {
                                for event in translate.update(notification.update) {
                                    out.event(event);
                                }
                                Ok(())
                            })
                            .await
                            .if_request(async |request: RequestPermissionRequest, responder| {
                                let (response, refused) = permission(&request, policy);
                                if let Some(message) = refused {
                                    out.event(translate.event(
                                        TurnStreamEventKind::Other("warning".into()),
                                        "permission_policy_refused",
                                        |event| event.message = Some(message.clone()),
                                    ));
                                    translate.log(message);
                                }
                                responder.respond(response)
                            })
                            .await
                            .otherwise_ignore()
                            .map_err(|error| format!("malformed message from the harness: {}", describe(&error)))?;
                    }
                    Ok(SessionMessage::PromptResponse(Ok(response))) => {
                        let PromptResponse { stop_reason, usage, .. } = response;
                        let usage = usage.map(|usage| json!(usage));
                        if let Some(usage) = &usage {
                            self.out.event(self.translate.event(
                                TurnStreamEventKind::TokenUsageUpdated,
                                "prompt_usage",
                                |event| event.token_usage = Some(usage.clone()),
                            ));
                        }
                        return Ok(self.finish(stop_reason, usage, Some(record), reset, forced));
                    }
                    Ok(SessionMessage::PromptResponse(Err(error))) if cancelled => {
                        // The cancelled prompt ended with its process.
                        let _ = error;
                        return Ok(self.finish(StopReason::Cancelled, None, Some(record), reset, true));
                    }
                    Ok(SessionMessage::PromptResponse(Err(error))) => {
                        return Err(format!("session/prompt failed: {}", describe(&error)));
                    }
                    Ok(_) => {}
                    Err(error) => return Err(describe(&error)),
                },
                Some(()) = interrupts.recv(), if !cancelled => {
                    cancelled = true;
                    cancel_deadline = Some(Instant::now() + CANCEL_GRACE);
                    connection
                        .send_notification(CancelNotification::new(session_id.clone()))
                        .map_err(|error| format!("session/cancel failed: {}", describe(&error)))?;
                }
                () = sleep_until(cancel_deadline) => {
                    cancel_deadline = None;
                    forced = true;
                    self.stop.stop();
                }
            }
            self.out.overloaded()?;
        }
    }

    fn finish(
        &mut self,
        stop_reason: StopReason,
        usage: Option<Value>,
        record: Option<AcpSessionRecord>,
        reset: Option<String>,
        forced: bool,
    ) -> Finished {
        let end_turn = stop_reason == StopReason::EndTurn;
        for event in self.translate.close(end_turn) {
            self.out.event(event);
        }
        // Only an answered prompt completes here; the caller settles any other end.
        if end_turn {
            self.out.event(self.translate.event(
                TurnStreamEventKind::TurnCompleted,
                "prompt_response",
                |event| event.status = Some(stop_label(&stop_reason)),
            ));
        }
        Finished {
            final_text: self.translate.last_assistant.clone().filter(|_| end_turn),
            stop_reason,
            usage,
            record,
            reset,
            forced,
            logs: std::mem::take(&mut self.translate.logs),
        }
    }
}

/// The stop reason's wire name, such as `end_turn`.
pub fn stop_label(reason: &StopReason) -> String {
    serde_json::to_value(reason)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{reason:?}"))
}

/// Why a rejected restore resets continuity; a login requirement is a setup
/// failure instead.
fn reset_reason(error: &agent_client_protocol::Error, method: &str) -> Result<String, String> {
    if is_auth_error(error) {
        return Err(setup_error(error, method));
    }
    Ok(format!(
        "session/{method} was rejected: {}",
        describe(error)
    ))
}

fn setup_error(error: &agent_client_protocol::Error, method: &str) -> String {
    if is_auth_error(error) {
        "the harness requires authentication; sign in with the harness's own login, then retry"
            .to_string()
    } else {
        format!("{method} failed: {}", describe(error))
    }
}

/// The policy's answer: an offered option of the allowed (or, for `deny`,
/// rejecting) kinds, else the cancellation outcome with a diagnostic.
fn permission(
    request: &RequestPermissionRequest,
    policy: AcpPermissionPolicy,
) -> (RequestPermissionResponse, Option<String>) {
    let kinds = match policy {
        AcpPermissionPolicy::Allow => [
            PermissionOptionKind::AllowOnce,
            PermissionOptionKind::AllowAlways,
        ],
        AcpPermissionPolicy::Deny => [
            PermissionOptionKind::RejectOnce,
            PermissionOptionKind::RejectAlways,
        ],
    };
    let chosen = kinds.iter().find_map(|kind| {
        request
            .options
            .iter()
            .find(|option| option.kind == *kind)
            .map(|option| option.option_id.clone())
    });
    match chosen {
        Some(option_id) => (
            RequestPermissionResponse::new(RequestPermissionOutcome::Selected(
                SelectedPermissionOutcome::new(option_id),
            )),
            None,
        ),
        None => (
            RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled),
            Some(format!(
                "permission request for tool call {} offered no option the {} policy accepts; answered cancelled",
                request.tool_call.tool_call_id.0,
                match policy {
                    AcpPermissionPolicy::Allow => "allow",
                    AcpPermissionPolicy::Deny => "deny",
                }
            )),
        ),
    }
}

// ---- event translation ---------------------------------------------------

/// An assistant or reasoning message being streamed.
struct Open {
    channel: TurnStreamChannel,
    item_id: String,
    text: String,
}

/// Maps session updates onto Spark's turn events, accumulating each
/// message's text and every tool call's state.
struct Translate {
    harness: String,
    session_id: Option<String>,
    open: Option<Open>,
    last_assistant: Option<String>,
    tool_calls: BTreeMap<String, Value>,
    items: u64,
    logs: Vec<AgentRawLogLine>,
}

impl Translate {
    fn new(harness: &str) -> Self {
        Self {
            harness: harness.to_string(),
            session_id: None,
            open: None,
            last_assistant: None,
            tool_calls: BTreeMap::new(),
            items: 0,
            logs: Vec::new(),
        }
    }

    fn event(
        &self,
        kind: TurnStreamEventKind,
        raw_kind: &str,
        fill: impl FnOnce(&mut TurnStreamEvent),
    ) -> TurnStreamEvent {
        let mut event = TurnStreamEvent {
            kind,
            channel: None,
            source: TurnStreamSource {
                backend: Some(ACP_BACKEND.to_string()),
                session_id: Some(self.harness.clone()),
                app_thread_id: self.session_id.clone(),
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

    fn log(&mut self, line: String) {
        if self.logs.len() < MAX_LOG_LINES {
            let mut line = line;
            if line.len() > MAX_LOG_LINE_BYTES {
                let mut end = MAX_LOG_LINE_BYTES;
                while !line.is_char_boundary(end) {
                    end -= 1;
                }
                line.truncate(end);
            }
            self.logs.push(AgentRawLogLine {
                direction: "diagnostic".into(),
                line,
            });
        }
    }

    fn update(&mut self, update: SessionUpdate) -> Vec<TurnStreamEvent> {
        match update {
            SessionUpdate::AgentMessageChunk(chunk) => {
                self.chunk(TurnStreamChannel::Assistant, chunk)
            }
            SessionUpdate::AgentThoughtChunk(chunk) => {
                self.chunk(TurnStreamChannel::Reasoning, chunk)
            }
            SessionUpdate::ToolCall(call) => {
                let mut events = self.close(false);
                let id = call.tool_call_id.0.to_string();
                let raw = serde_json::to_value(&call).unwrap_or_default();
                self.tool_calls.insert(id.clone(), raw);
                events.push(self.tool_event(&id, true));
                events
            }
            SessionUpdate::ToolCallUpdate(update) => {
                let mut events = self.close(false);
                let id = update.tool_call_id.0.to_string();
                let fields = serde_json::to_value(&update.fields).unwrap_or_default();
                let started = !self.tool_calls.contains_key(&id);
                let raw = self
                    .tool_calls
                    .entry(id.clone())
                    .or_insert_with(|| json!({"toolCallId": id}));
                if let (Some(raw), Some(fields)) = (raw.as_object_mut(), fields.as_object()) {
                    for (key, value) in fields {
                        if !value.is_null() {
                            raw.insert(key.clone(), value.clone());
                        }
                    }
                }
                events.push(self.tool_event(&id, started));
                events
            }
            SessionUpdate::Plan(plan) => {
                let mut events = self.close(false);
                let entries = serde_json::to_value(&plan.entries).unwrap_or_default();
                events.push(self.event(
                    TurnStreamEventKind::Other("plan_updated".into()),
                    "plan",
                    |event| event.details = Some(json!({"entries": entries})),
                ));
                events
            }
            SessionUpdate::UsageUpdate(usage) => {
                // Context occupancy and session cost, never request consumption.
                let details = serde_json::to_value(&usage).unwrap_or_default();
                vec![self.event(
                    TurnStreamEventKind::Other("context_usage_updated".into()),
                    "usage_update",
                    |event| event.details = Some(details),
                )]
            }
            other => {
                let line = serde_json::to_string(&other).unwrap_or_default();
                self.log(format!("session/update not translated: {line}"));
                Vec::new()
            }
        }
    }

    fn chunk(&mut self, channel: TurnStreamChannel, chunk: ContentChunk) -> Vec<TurnStreamEvent> {
        let ContentBlock::Text(text) = chunk.content else {
            self.log("non-text message content not translated".into());
            return Vec::new();
        };
        let mut events = Vec::new();
        if self
            .open
            .as_ref()
            .is_some_and(|open| open.channel != channel)
        {
            events = self.close(false);
        }
        let open = self.open.get_or_insert_with(|| {
            self.items += 1;
            Open {
                channel: channel.clone(),
                item_id: chunk
                    .message_id
                    .map(|id| id.0.to_string())
                    .unwrap_or_else(|| format!("message-{}", self.items)),
                text: String::new(),
            }
        });
        open.text.push_str(&text.text);
        let item_id = open.item_id.clone();
        events.push(self.event(
            TurnStreamEventKind::ContentDelta,
            "message_chunk",
            |event| {
                event.channel = Some(channel);
                event.content_delta = Some(text.text.clone());
                event.source.item_id = Some(item_id);
            },
        ));
        events
    }

    /// Completes the open message; an assistant message that ends the turn
    /// is its final answer.
    fn close(&mut self, final_answer: bool) -> Vec<TurnStreamEvent> {
        let Some(open) = self.open.take() else {
            return Vec::new();
        };
        let is_assistant = open.channel == TurnStreamChannel::Assistant;
        if is_assistant {
            self.last_assistant = Some(open.text.clone());
        }
        vec![
            self.event(TurnStreamEventKind::ContentCompleted, "message", |event| {
                event.content_delta = Some(open.text.clone());
                event.message = Some(open.text);
                event.source.item_id = Some(open.item_id);
                if is_assistant {
                    event.phase = Some(
                        if final_answer {
                            "final_answer"
                        } else {
                            "commentary"
                        }
                        .into(),
                    );
                }
                event.channel = Some(open.channel);
            }),
        ]
    }

    fn tool_event(&self, id: &str, started: bool) -> TurnStreamEvent {
        let raw = &self.tool_calls[id];
        let status = raw["status"].as_str().unwrap_or("pending").to_string();
        let title = raw["title"].as_str().unwrap_or("tool").to_string();
        let mut tool_call = json!({
            "id": id,
            "name": raw["name"].as_str().unwrap_or(&title),
            "title": title,
            "kind": if raw["kind"] == "execute" { "command_execution" } else { "dynamic_tool" },
            "acp_kind": raw["kind"].as_str().unwrap_or("other"),
            "status": status,
            "input": raw["rawInput"],
        });
        if let Some(command) = raw["rawInput"]["command"].as_str() {
            tool_call["command"] = json!(command);
        }
        let paths = raw["locations"]
            .as_array()
            .map(|locations| {
                locations
                    .iter()
                    .filter_map(|l| l["path"].as_str())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !paths.is_empty() {
            tool_call["file_paths"] = json!(paths);
        }
        if !raw["rawOutput"].is_null() {
            tool_call["output"] = raw["rawOutput"].clone();
        }
        let kind = match status.as_str() {
            "completed" => TurnStreamEventKind::ToolCallCompleted,
            "failed" => TurnStreamEventKind::ToolCallFailed,
            _ if started => TurnStreamEventKind::ToolCallStarted,
            _ => TurnStreamEventKind::ToolCallUpdated,
        };
        self.event(kind, "tool_call", |event| {
            event.tool_call = Some(tool_call);
            event.status = Some(status);
            event.source.item_id = Some(id.to_string());
        })
    }
}
