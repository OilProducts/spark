//! Stable ACP v1 client for registered harnesses (`acp:<id>`), driven by the
//! pinned SDK over the shared stdio transport.
//!
//! Chat conversations keep their ACP session process warm across turns until
//! idle timeout, config/instruction change, or explicit close so harness-owned
//! background work survives turn boundaries. When `end_turn` arrives while a
//! non-daemon tool call is still `in_progress`, the turn stays open until the
//! background tool and its post-wakeup response settle. Flow nodes run
//! one-shot processes after draining any open background tool calls.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::util::MatchDispatch;
use agent_client_protocol::v1::schema::{
    AuthCapabilities, AuthMethod, AuthenticateRequest, CancelNotification, ClientCapabilities,
    ContentBlock, ContentChunk, ElicitationCapabilities, ElicitationFormCapabilities,
    Implementation, InitializeRequest, LoadSessionRequest, NewSessionRequest, PermissionOptionKind,
    PromptResponse, RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    ResumeSessionRequest, SelectedPermissionOutcome, SessionConfigKind, SessionConfigOption,
    SessionConfigOptionCategory, SessionConfigOptionValue, SessionConfigSelectOptions,
    SessionNotification, SessionUpdate, SetSessionConfigOptionRequest, StopReason, SystemPrompt,
};
use agent_client_protocol::v1::{ActiveSession, ConnectionTo, SessionMessage, SessionSetup};
use agent_client_protocol::{Agent, Client, Dispatch, ErrorCode, Lines, Raw, Responder};
use futures_util::future::BoxFuture;
use futures_util::stream::FuturesUnordered;
use futures_util::{FutureExt as _, StreamExt as _};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use spark_common::agent_settings::{
    AcpHarness, AcpPermissionPolicy, EnvironmentInheritancePolicy, HarnessIdentity,
};
use spark_common::events::{
    TurnStreamChannel, TurnStreamEvent, TurnStreamEventKind, TurnStreamSource,
};
use spark_common::forms::FormReply;
use tokio::sync::mpsc::{
    error::TrySendError, Receiver, Sender, UnboundedReceiver, UnboundedSender,
};
use tokio::time::Instant;

use crate::agent::{
    AgentRawLogLine, AgentTurnEventSink, AgentTurnOutput, AgentTurnRequest,
    AGENT_INSTRUCTIONS_METADATA_KEY,
};
use crate::codergen::{ClarificationHandler, ClarificationReply};
use crate::file_changes::{self, prefixed, FileChange};
use crate::history::HistoryTurn;
use crate::stdio_transport::{self, sleep_until, Endpoints, Incoming, Outgoing, Stop, LIMITS};

pub const ACP_BACKEND: &str = "acp";
/// The session a turn continues, and the one it leaves for the next turn.
pub const ACP_SESSION_METADATA_KEY: &str = "spark.runtime.acp.session";
/// Emitted once a session is set up; its details are the session record.
pub const ACP_SESSION_ESTABLISHED: &str = "session_established";
/// How long a cancelled prompt may take to settle before its process is stopped.
const CANCEL_GRACE: Duration = Duration::from_secs(5);
/// How long a deferred end-of-turn waits for the harness to start its
/// post-tool wakeup once the last background tool call completes.
const POST_TOOL_WAKEUP_WAIT: Duration = Duration::from_secs(30);
/// How long a post-tool wakeup may go quiet after streaming an assistant
/// message before settling when no `usage_update` ends the step.
const POST_WAKEUP_STREAM_IDLE: Duration = Duration::from_secs(15);
/// How long a post-tool wakeup or continuation may go quiet while still
/// mid-step with no open assistant message (e.g. after thoughts or completed
/// tools while the model generates tool-call arguments).
// ponytail: ACP has no explicit end-of-wakeup signal; bound stuck harnesses
// without cutting off long tool-call generation after a thought summary.
const POST_WAKEUP_IN_PROGRESS_IDLE: Duration = Duration::from_secs(180);
/// Brief settle window after a post-tool `usage_update` once an assistant
/// message is open, so an immediately following tool call can still arrive
/// before the turn closes.
const POST_WAKEUP_USAGE_SETTLE: Duration = Duration::from_secs(2);
/// How long an idle pooled chat ACP process lives before being reaped.
const SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// How long an idle worker buffers unsolicited continuation updates waiting
/// for the workspace continuation handler to attach before draining them.
const CONTINUATION_ATTACH_TIMEOUT: Duration = Duration::from_secs(5);
/// How often a flow's form checks for its answer.
const FORM_POLL: Duration = Duration::from_millis(100);
/// How long a chat answer waits for the turn to take it.
const FORM_DELIVERY: Duration = Duration::from_secs(5);
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

    /// Whether this saved session may continue as `current`: every field
    /// matches exactly, except that the saved work area may carry a Windows
    /// `\\?\` prefix. Its path is never resolved again, so a checkout
    /// replaced by a symlink to another directory doesn't continue.
    fn continues_as(&self, current: &Self) -> bool {
        let cwd = spark_common::paths::strip_verbatim_prefix(PathBuf::from(&self.cwd));
        Self {
            session_id: current.session_id.clone(),
            cwd: cwd.to_string_lossy().into_owned(),
            ..self.clone()
        } == *current
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
    do_not_pool: AtomicBool,
    closed: AtomicBool,
    /// Replies for the turn's pending forms.
    replies: UnboundedSender<FormAnswer>,
    /// User messages delivered during a post-`end_turn` background wait.
    messages: UnboundedSender<LiveMessage>,
    /// True only while the turn is parked between prompts (`WaitingForTools` /
    /// `AwaitingWakeup`) and can dispatch a new `session/prompt` immediately.
    accepting_messages: Arc<AtomicBool>,
}

/// Everything a parked chat ACP process is valid for.
#[derive(Clone, PartialEq, Eq)]
struct SessionIdentity {
    record: AcpSessionRecord,
    policy: AcpPermissionPolicy,
}

impl SessionIdentity {
    fn matches(&self, plan: &Plan) -> bool {
        let Some(saved) = &plan.saved else {
            return false;
        };
        self.policy == plan.policy
            && self.record.harness == plan.harness_id
            && self.record.session_id == saved.session_id
            && self.record.cwd == plan.cwd.to_string_lossy()
            && self.record.configuration_generation == plan.generation
            && self.record.instructions_sha256 == plan.instructions_sha256
    }
}

/// A live ACP worker process and its channel for subsequent turns.
struct LiveWorker {
    /// Dropped BEFORE `connection` so `more_turns.recv()` closes and lets the
    /// worker thread exit before `Connection::drop` joins it.
    turns: UnboundedSender<TurnWork>,
    connection: stdio_transport::Connection,
    _instruction_file: Option<tempfile::NamedTempFile>,
    alive: Arc<AtomicBool>,
    /// Signaled once the calling thread has released `registry()` and parked
    /// this worker into `pool()`, so idle continuation wakeups never race
    /// ahead of parking.
    parked: Arc<tokio::sync::Notify>,
    /// True while the idle worker has buffered unsolicited post-`end_turn`
    /// activity waiting for a continuation turn to attach.
    pending_continuation: Arc<AtomicBool>,
}

impl LiveWorker {
    fn alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst) && !self.turns.is_closed()
    }
}

/// Callback invoked when a parked chat ACP session starts emitting unsolicited
/// agent activity after `end_turn`. Returns `true` if it claimed the
/// conversation and scheduled `run_continuation`.
pub type ContinuationHandler = Arc<dyn Fn(&str, &str) -> bool + Send + Sync + 'static>;

fn continuation_handlers() -> &'static Mutex<Vec<ContinuationHandler>> {
    static HANDLERS: OnceLock<Mutex<Vec<ContinuationHandler>>> = OnceLock::new();
    HANDLERS.get_or_init(Mutex::default)
}

/// Registers a handler that claims and runs unsolicited ACP continuations for
/// its owned conversations.
pub fn register_continuation_handler(handler: ContinuationHandler) {
    continuation_handlers()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .push(handler);
}

fn notify_continuation(project_path: &str, conversation_id: &str) -> bool {
    let handlers = continuation_handlers()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    handlers
        .into_iter()
        .any(|handler| handler(project_path, conversation_id))
}

/// A warm ACP session parked between chat turns.
struct PooledSession {
    worker: LiveWorker,
    identity: SessionIdentity,
}

fn pool() -> &'static Mutex<HashMap<Key, PooledSession>> {
    static POOL: OnceLock<Mutex<HashMap<Key, PooledSession>>> = OnceLock::new();
    POOL.get_or_init(Mutex::default)
}

/// The harness ID of a live parked ACP session for `(project_path, conversation_id)`.
pub fn pooled_harness(project_path: &str, conversation_id: &str) -> Option<String> {
    let key = (project_path.to_string(), conversation_id.to_string());
    let entries = pool().lock().unwrap_or_else(|error| error.into_inner());
    entries
        .get(&key)
        .filter(|session| session.worker.alive())
        .map(|session| session.identity.record.harness.clone())
}

/// Whether a parked ACP session for `(project_path, conversation_id)` has
/// buffered unsolicited post-`end_turn` activity waiting for a continuation.
pub fn has_pending_continuation(project_path: &str, conversation_id: &str) -> bool {
    let key = (project_path.to_string(), conversation_id.to_string());
    let entries = pool().lock().unwrap_or_else(|error| error.into_inner());
    entries.get(&key).is_some_and(|session| {
        session.worker.alive() && session.worker.pending_continuation.load(Ordering::SeqCst)
    })
}

/// Takes a matching warm session out of the pool for `key`, sweeping dead
/// entries and dropping any mismatched session outside the pool lock.
fn take_pooled_session(key: &Key, plan: &Plan) -> Option<LiveWorker> {
    let mut dead = Vec::new();
    let candidate = {
        let mut entries = pool().lock().unwrap_or_else(|error| error.into_inner());
        let dead_keys = entries
            .iter()
            .filter(|(entry_key, session)| *entry_key != key && !session.worker.alive())
            .map(|(entry_key, _)| entry_key.clone())
            .collect::<Vec<_>>();
        for dead_key in dead_keys {
            if let Some(session) = entries.remove(&dead_key) {
                dead.push(session);
            }
        }
        entries.remove(key)
    };
    drop(dead);
    match candidate {
        Some(pooled) if pooled.worker.alive() && pooled.identity.matches(plan) => {
            Some(pooled.worker)
        }
        Some(other) => {
            drop(other);
            None
        }
        None => None,
    }
}

/// Takes any live parked session for `key` to attach a post-`end_turn`
/// continuation.
fn take_pooled_for_continuation(key: &Key) -> Option<PooledSession> {
    let candidate = {
        let mut entries = pool().lock().unwrap_or_else(|error| error.into_inner());
        entries.remove(key)
    };
    match candidate {
        Some(pooled) if pooled.worker.alive() => Some(pooled),
        Some(other) => {
            drop(other);
            None
        }
        None => None,
    }
}

/// Parks a healthy session in the pool and wakes its worker to watch for
/// idle updates; any displaced session is dropped outside the pool lock.
fn park_pooled_session(key: Key, worker: LiveWorker, identity: SessionIdentity) {
    if !worker.alive() {
        drop(worker);
        return;
    }
    let parked = Arc::clone(&worker.parked);
    let displaced = {
        let mut entries = pool().lock().unwrap_or_else(|error| error.into_inner());
        entries.insert(key, PooledSession { worker, identity })
    };
    drop(displaced);
    parked.notify_one();
}

/// Closes any parked ACP session for `(project_path, conversation_id)`, and
/// prevents a currently running turn from being parked when it finishes.
pub fn close_session(project_path: &str, conversation_id: &str) -> bool {
    let marked = if let Some(live) = find(project_path, conversation_id) {
        live.do_not_pool.store(true, Ordering::SeqCst);
        true
    } else {
        false
    };
    let key = (project_path.to_string(), conversation_id.to_string());
    let removed = {
        let mut entries = pool().lock().unwrap_or_else(|error| error.into_inner());
        entries.remove(&key)
    };
    let had_pooled = removed.is_some();
    drop(removed);
    marked || had_pooled
}

/// A reply for one of a turn's pending forms, by its Spark request ID, and
/// where to say whether the harness got it.
struct FormAnswer {
    request_id: String,
    /// A flow's failed wait fails the turn.
    reply: Result<FormReply, String>,
    delivered: Option<std::sync::mpsc::Sender<bool>>,
}

/// A user message to dispatch as a new `session/prompt` while the turn is
/// waiting between prompts (post-`end_turn` background tool wait).
struct LiveMessage {
    message_id: String,
    message: String,
    delivered: std::sync::mpsc::Sender<bool>,
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

/// Sends `reply` to the harness waiting on the running turn's form
/// `request_id`. False when no such form is pending: it was answered, or its
/// prompt, connection or turn has ended, and a stored request never revives it.
pub fn answer(
    project_path: &str,
    conversation_id: &str,
    request_id: &str,
    reply: FormReply,
) -> bool {
    let Some(live) = find(project_path, conversation_id) else {
        return false;
    };
    let (delivered, received) = std::sync::mpsc::channel();
    let answer = FormAnswer {
        request_id: request_id.to_string(),
        reply: Ok(reply),
        delivered: Some(delivered),
    };
    live.replies.send(answer).is_ok() && received.recv_timeout(FORM_DELIVERY).unwrap_or(false)
}

/// Sends `message` into a running ACP turn when it is waiting between prompts
/// (a post-`end_turn` background tool wait), returning `true` once
/// `session/prompt` was dispatched. Returns `false` while a prompt is already
/// in flight or no turn is active, leaving the caller to queue it.
pub fn send_message(
    project_path: &str,
    conversation_id: &str,
    message_id: &str,
    message: &str,
) -> bool {
    let Some(live) = find(project_path, conversation_id) else {
        return false;
    };
    if !live.accepting_messages.load(Ordering::SeqCst) {
        return false;
    }
    let (delivered, received) = std::sync::mpsc::channel();
    let pending = LiveMessage {
        message_id: message_id.to_string(),
        message: message.to_string(),
        delivered,
    };
    live.messages.send(pending).is_ok() && received.recv_timeout(FORM_DELIVERY).unwrap_or(false)
}

/// Stops the turn's process group directly, and closes any parked session.
pub fn terminate(project_path: &str, conversation_id: &str) -> bool {
    let mut stopped = false;
    if let Some(live) = find(project_path, conversation_id) {
        live.terminated.store(true, Ordering::SeqCst);
        live.do_not_pool.store(true, Ordering::SeqCst);
        if let Some(stop) = live.stop.lock().unwrap().as_ref() {
            stop.stop();
        }
        stopped = true;
    }
    let key = (project_path.to_string(), conversation_id.to_string());
    let removed = {
        let mut entries = pool().lock().unwrap_or_else(|error| error.into_inner());
        entries.remove(&key)
    };
    let had_pooled = removed.is_some();
    drop(removed);
    stopped || had_pooled
}

// ---- the turn ------------------------------------------------------------

/// What the worker hands the calling thread, in order.
enum Message {
    Event(TurnStreamEvent),
    /// The primary prompt is about to be sent.
    Prompted,
    Done(Result<Finished, String>),
    /// A pooled worker exited before starting this turn, returning it intact.
    Rejected(TurnWork),
}

/// Everything one prompted turn hands to a new or pooled session worker.
struct PromptWork {
    plan: Plan,
    out: Out,
    interrupts: Receiver<()>,
    replies: UnboundedSender<FormAnswer>,
    answered: UnboundedReceiver<FormAnswer>,
    live_messages: UnboundedReceiver<LiveMessage>,
    accepting_messages: Arc<AtomicBool>,
}

/// Everything an unsolicited post-`end_turn` continuation hands to a parked
/// session worker.
struct ContinuationWork {
    out: Out,
    interrupts: Receiver<()>,
    replies: UnboundedSender<FormAnswer>,
    answered: UnboundedReceiver<FormAnswer>,
    live_messages: UnboundedReceiver<LiveMessage>,
    accepting_messages: Arc<AtomicBool>,
}

enum TurnWork {
    Prompt(PromptWork),
    Continuation(ContinuationWork),
}

impl TurnWork {
    fn out_sender(&self) -> std::sync::mpsc::Sender<Message> {
        match self {
            Self::Prompt(work) => work.out.messages.clone(),
            Self::Continuation(work) => work.out.messages.clone(),
        }
    }
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
    /// The registration's display name, shown with the forms it asks.
    harness_label: String,
    /// A flow's: forms go to it instead of waiting for a chat answer.
    clarification: Option<ClarificationHandler>,
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

impl Plan {
    /// Records `options`, a full set the harness acknowledged, for choosers.
    /// An empty set is a response that omitted them, so it changes nothing.
    fn acknowledge(&self, options: &[SessionConfigOption]) {
        if options.is_empty() {
            return;
        }
        let options = options
            .iter()
            .filter_map(|option| serde_json::to_value(option).ok())
            .collect();
        ACKNOWLEDGED
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(
                (
                    self.cwd.clone(),
                    self.harness_id.clone(),
                    self.generation.clone(),
                ),
                options,
            );
    }
}

#[derive(Clone, Debug)]
enum Selector {
    Category(SessionConfigOptionCategory, &'static str),
    /// A config ID, and who asked for it.
    Id(String, &'static str),
}

/// Runs one primary prompt with `harness_id`'s registration, as captured in
/// the request's execution settings. A flow passes its `clarification`
/// handler; a chat's forms wait for [`answer`].
pub fn run_turn(
    request: &AgentTurnRequest,
    harness_id: &str,
    sink: Option<&AgentTurnEventSink>,
    clarification: Option<ClarificationHandler>,
) -> Result<AcpTurn, AcpFailure> {
    let mut seen = AcpFailure::default();
    let result = turn(request, harness_id, sink, clarification, &mut seen);
    result.map_err(|message| AcpFailure { message, ..seen })
}

/// Attaches to a parked chat ACP session that emitted unsolicited post-`end_turn`
/// activity, streaming its updates through `sink` until the wakeup settles.
/// Returns `Ok(None)` if no parked session is available.
pub fn run_continuation(
    project_path: &str,
    conversation_id: &str,
    sink: Option<&AgentTurnEventSink>,
) -> Result<Option<AcpTurn>, AcpFailure> {
    let mut seen = AcpFailure::default();
    let result = continuation(project_path, conversation_id, sink, &mut seen);
    result.map_err(|message| AcpFailure { message, ..seen })
}

fn continuation(
    project_path: &str,
    conversation_id: &str,
    sink: Option<&AgentTurnEventSink>,
    seen: &mut AcpFailure,
) -> Result<Option<AcpTurn>, String> {
    let key = (project_path.to_string(), conversation_id.to_string());
    let (interrupts, interrupted) = tokio::sync::mpsc::channel(1);
    let (replies, answered) = tokio::sync::mpsc::unbounded_channel();
    let (messages_tx, live_messages) = tokio::sync::mpsc::unbounded_channel();
    let accepting_messages = Arc::new(AtomicBool::new(false));
    let live = Arc::new(Live {
        interrupts,
        stop: Mutex::new(None),
        terminated: AtomicBool::new(false),
        do_not_pool: AtomicBool::new(false),
        closed: AtomicBool::new(false),
        replies: replies.clone(),
        messages: messages_tx,
        accepting_messages: Arc::clone(&accepting_messages),
    });
    let registered = {
        let mut entries = registry().lock().unwrap();
        if entries
            .get(&key)
            .and_then(Weak::upgrade)
            .is_some_and(|other| !other.closed.load(Ordering::SeqCst))
        {
            return Ok(None);
        }
        entries.insert(key.clone(), Arc::downgrade(&live));
        Registered(key.clone(), Arc::clone(&live))
    };

    let Some(pooled) = take_pooled_for_continuation(&key) else {
        return Ok(None);
    };
    let harness_id = pooled.identity.record.harness.clone();
    let identity = pooled.identity;
    let worker = pooled.worker;
    attach_stop(&live, &worker.connection);

    let (messages, delivered) = std::sync::mpsc::channel();
    let work = TurnWork::Continuation(ContinuationWork {
        out: Out { messages },
        interrupts: interrupted,
        replies,
        answered,
        live_messages,
        accepting_messages,
    });
    if worker.turns.send(work).is_err() {
        drop(worker);
        return Ok(None);
    }

    let app_turn_id = uuid::Uuid::new_v4().to_string();
    let mut done: Option<Result<Finished, String>> = None;
    for message in delivered {
        match message {
            Message::Event(mut event) => {
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
            Message::Rejected(_) => {
                drop(worker);
                return Ok(None);
            }
        }
    }

    let finished = match done {
        Some(Ok(finished)) => finished,
        failed => {
            let stderr = worker.connection.stderr();
            let lines = stderr.lines().collect::<Vec<_>>();
            let tail = lines[lines.len().saturating_sub(10)..].join("\n");
            drop(worker);
            let message = match failed {
                Some(Err(message)) => message,
                _ => "the ACP connection worker stopped".to_string(),
            };
            let message = if tail.is_empty() {
                format!("ACP harness {harness_id}: {message}")
            } else {
                format!("ACP harness {harness_id}: {message}; stderr tail:\n{tail}")
            };
            return Err(message);
        }
    };
    let record = finished.record.filter(|_| !finished.forced);
    let should_park = !finished.forced
        && !live.terminated.load(Ordering::SeqCst)
        && !live.do_not_pool.load(Ordering::SeqCst)
        && record.is_some();
    drop(registered);
    if should_park {
        park_pooled_session(
            key,
            worker,
            SessionIdentity {
                record: record.clone().unwrap(),
                policy: identity.policy,
            },
        );
    } else {
        drop(worker);
    }
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
    Ok(Some(AcpTurn {
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
    }))
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
    let document = spark_storage::settings::read_settings_document(&path).ok()?;
    let (agents, _) = spark_storage::settings::agents_section(&path, &document).ok()?;
    agents.acp_harnesses.get(harness_id).cloned()
}

/// Whether `program` names an executable file: a path that exists, or a bare
/// name found on `path` (else Spark's own `PATH`), honoring `PATHEXT` on
/// Windows. A relative path is taken as found, since where it resolves depends
/// on the launch.
pub fn program_found(program: &str, path: Option<&str>) -> bool {
    let candidate = Path::new(program);
    if candidate.is_absolute() {
        return candidate.is_file();
    }
    if candidate.components().count() > 1 {
        return true;
    }
    crate::local_environment::find_executable_on_path(program, path.map(std::ffi::OsStr::new))
        .is_some()
}

/// `harness`'s program in `cwd` (else Spark's own directory) with the
/// environment it inherits under `inheritance`, its own `env`, then
/// `bindings`, applied in that order. A program its child finds is named as
/// found; launch no other, since the system's own search would look past the
/// child's `PATH`, as Windows does through Spark's, or in the work area for an
/// empty entry.
fn harness_command(
    harness: &AcpHarness,
    inheritance: EnvironmentInheritancePolicy,
    cwd: Option<&Path>,
    bindings: impl IntoIterator<Item = (String, OsString)>,
) -> Command {
    let env = crate::local_environment::inherited_environment(inheritance)
        .into_iter()
        .chain(harness.env.clone())
        .map(|(key, value)| (key, OsString::from(value)))
        .chain(bindings)
        .collect::<Vec<_>>();
    let command = |program: &std::ffi::OsStr| {
        let mut command = Command::new(program);
        command.env_clear().envs(env.iter().cloned());
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        command
    };
    let named = command(harness.executable.as_ref());
    // So setup and turns run the file their child finds, such as a project's
    // npm `.cmd` shim, which a Windows launch of a bare name would not try.
    match crate::local_environment::child_program(&named) {
        Some(program) => command(program.as_os_str()),
        None => named,
    }
}

/// Whether `command` from [`harness_command`] runs a program its child finds.
fn found(command: &Command) -> bool {
    crate::local_environment::child_program(command).is_some()
}

/// The configuration generation a launch of `harness` records: its
/// registration's, plus the values it inherits now, so a changed inherited
/// environment starts fresh. Only this hash is kept, never the values.
pub fn launch_generation(
    harness: &AcpHarness,
    inheritance: EnvironmentInheritancePolicy,
) -> String {
    let inherited = crate::local_environment::inherited_environment(inheritance);
    spark_common::project::sha256_hex(
        json!([harness.generation(inheritance), inherited])
            .to_string()
            .as_bytes(),
    )
}

// ---- discovery -----------------------------------------------------------

/// How long discovery waits for a harness to initialize and open its
/// temporary session before reporting it unreachable.
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a sign-in waits for the user to finish what the harness asks,
/// such as a browser sign-in.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(180);
/// What a declared instruction channel carries during discovery, which
/// never submits a prompt.
const PROBE_INSTRUCTIONS: &str = "Spark is checking this harness's capabilities. No task follows.";

/// A capability's effective state for Spark's use of a harness: `supported`,
/// `unsupported` or `unverified`, or for flows `limited`, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Capability {
    pub state: String,
    pub reason: String,
}

impl Capability {
    fn new(state: &str, reason: impl Into<String>) -> Self {
        Self {
            state: state.to_string(),
            reason: reason.into(),
        }
    }

    pub fn supported(&self) -> bool {
        self.state == "supported"
    }
}

/// What a prompt-free probe of a registered harness found. Discovery is
/// advisory: each turn's own negotiation decides. Setup failures stay apart
/// from capabilities, since a harness that responds may still be unable to
/// run a particular request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AcpDiscovery {
    /// `ready`, `unreachable`, `unauthenticated` or `not_installed`.
    pub setup_status: String,
    /// Why setup failed; callers report it as their own status error.
    #[serde(skip)]
    pub error: Option<String>,
    /// The sign-in methods the harness offers through ACP `authenticate`,
    /// once it answered initialize but setup failed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub auth_methods: Vec<AcpAuthMethod>,
    pub capabilities: BTreeMap<String, Capability>,
    /// The temporary session's config options exactly as the harness sent
    /// them: IDs, value IDs, types, labels, order and groups.
    pub config_options: Vec<Value>,
    /// Legacy session modes, kept only when there are no config options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modes: Option<Value>,
}

/// A sign-in method a harness offers: one Spark starts with [`sign_in`],
/// or, with `terminal`, one the user runs in a terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AcpAuthMethod {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The command to run, as words: the variables the method sets as
    /// `NAME=value`, then the harness's program, arguments and the method's
    /// own arguments.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<Vec<String>>,
}

/// What initialize and the temporary session reported.
struct Probed {
    resume: bool,
    load: bool,
    /// The raw `session/new` result.
    session: Value,
}

#[derive(Default)]
struct ProbeFailure {
    unauthenticated: bool,
    /// The connection failed rather than the harness answering, as when it
    /// quits.
    lost: bool,
    /// The program could not be found to launch.
    missing: bool,
    message: String,
    auth_methods: Vec<AcpAuthMethod>,
}

impl From<String> for ProbeFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            ..Default::default()
        }
    }
}

/// Probes `harness` in `cwd` without a prompt: initialize, then one
/// temporary session for its config options. The process is stopped before
/// this returns.
pub fn discover(
    harness_id: &str,
    harness: &AcpHarness,
    inheritance: EnvironmentInheritancePolicy,
    cwd: &Path,
) -> AcpDiscovery {
    discovery(probe(harness_id, harness, inheritance, cwd, None))
}

/// Signs in to `harness` with the method `method_id` it offered, waiting for
/// the user to finish what the harness asks, then probes it as [`discover`]
/// does. The harness keeps its own sign-in, as it would from its own login.
pub fn sign_in(
    harness_id: &str,
    harness: &AcpHarness,
    inheritance: EnvironmentInheritancePolicy,
    cwd: &Path,
    method_id: &str,
) -> AcpDiscovery {
    discovery(probe(
        harness_id,
        harness,
        inheritance,
        cwd,
        Some(method_id),
    ))
}

fn probe(
    harness_id: &str,
    harness: &AcpHarness,
    inheritance: EnvironmentInheritancePolicy,
    cwd: &Path,
    sign_in: Option<&str>,
) -> Result<Probed, ProbeFailure> {
    let cwd = spark_common::paths::canonicalize_clean(cwd)
        .map_err(|error| format!("work area {} is unavailable: {error}", cwd.display()))?;
    // A declared channel launches as it would for chat, with harmless text.
    let instructions = harness
        .system_instructions
        .as_ref()
        .map(|_| PROBE_INSTRUCTIONS);
    let (command, instruction_file) = launch_command(
        harness_id,
        harness,
        inheritance,
        &cwd,
        instructions,
        &BTreeMap::new(),
    )?;
    let (done, result) = std::sync::mpsc::channel();
    let method = sign_in.map(str::to_string);
    let program = std::iter::once(&harness.executable)
        .chain(&harness.args)
        .cloned()
        .collect();
    // Only a program its child cannot find is missing; one that exists but
    // cannot start, such as a script whose interpreter is gone, failed.
    if !found(&command) {
        return Err(ProbeFailure {
            missing: true,
            message: format!("executable not found: {}", harness.executable),
            ..Default::default()
        });
    }
    let connection = stdio_transport::launch(command, LIMITS, move |endpoints| {
        probe_protocol(endpoints, cwd, program, method, done)
    })
    .map_err(|error| format!("launch failed: {error}"))?;
    let (timeout, step) = match sign_in {
        Some(_) => (SIGN_IN_TIMEOUT, "signing in"),
        None => (PROBE_TIMEOUT, "setup"),
    };
    let outcome = match sign_in {
        Some(_) => wait_for_sign_in(harness_id, &result),
        None => result.recv_timeout(timeout),
    };
    // A harness that quit explains a lost connection better than the
    // connection's own error.
    let exited = matches!(outcome, Ok(Err(ref failure)) if failure.lost)
        .then(|| connection.exit_code_within(Duration::from_millis(500)))
        .flatten();
    let stderr = connection.stderr();
    // Stops the process and waits for its worker.
    drop(connection);
    drop(instruction_file);
    let mut failure = match (outcome, exited) {
        (Ok(Ok(probed)), _) => return Ok(probed),
        (Ok(Err(_)), Some(exited)) => ProbeFailure::from(format!(
            "the harness quit during setup with exit code {exited}"
        )),
        (Ok(Err(failure)), None) => failure,
        (Err(std::sync::mpsc::RecvTimeoutError::Timeout), _) => ProbeFailure::from(format!(
            "the harness did not finish {step} within {}s",
            timeout.as_secs()
        )),
        (Err(std::sync::mpsc::RecvTimeoutError::Disconnected), _) => {
            ProbeFailure::from("the ACP connection worker stopped".to_string())
        }
    };
    let lines = stderr.lines().collect::<Vec<_>>();
    let tail = lines[lines.len().saturating_sub(10)..].join("\n");
    if !tail.is_empty() && !failure.unauthenticated {
        failure.message = format!("{}; stderr tail:\n{tail}", failure.message);
    }
    Err(failure)
}

/// Sign-ins in progress, each with its harness ID and the flag that
/// cancels it.
static SIGNING_IN: Mutex<Vec<(String, Arc<AtomicBool>)>> = Mutex::new(Vec::new());

/// Cancels the sign-ins in progress for `harness_id`, stopping the harness;
/// whether there was one.
pub fn cancel_sign_in(harness_id: &str) -> bool {
    let signing = SIGNING_IN.lock().unwrap_or_else(|error| error.into_inner());
    let mut found = false;
    for (_, cancelled) in signing.iter().filter(|(id, _)| id == harness_id) {
        cancelled.store(true, Ordering::Relaxed);
        found = true;
    }
    found
}

/// What a sign-in found, waiting until [`SIGN_IN_TIMEOUT`] unless it is
/// cancelled with [`cancel_sign_in`].
fn wait_for_sign_in(
    harness_id: &str,
    result: &std::sync::mpsc::Receiver<Result<Probed, ProbeFailure>>,
) -> Result<Result<Probed, ProbeFailure>, std::sync::mpsc::RecvTimeoutError> {
    use std::sync::mpsc::RecvTimeoutError::Timeout;
    let cancelled = Arc::new(AtomicBool::new(false));
    SIGNING_IN
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .push((harness_id.to_string(), Arc::clone(&cancelled)));
    let started = std::time::Instant::now();
    let outcome = loop {
        match result.recv_timeout(Duration::from_millis(100)) {
            Err(Timeout) if cancelled.load(Ordering::Relaxed) => {
                break Ok(Err(ProbeFailure {
                    unauthenticated: true,
                    message: "sign-in was cancelled".into(),
                    ..Default::default()
                }))
            }
            Err(Timeout) if started.elapsed() < SIGN_IN_TIMEOUT => {}
            outcome => break outcome,
        }
    };
    SIGNING_IN
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .retain(|(_, flag)| !Arc::ptr_eq(flag, &cancelled));
    outcome
}

async fn probe_protocol(
    endpoints: Endpoints,
    cwd: PathBuf,
    program: Vec<String>,
    sign_in: Option<String>,
    done: std::sync::mpsc::Sender<Result<Probed, ProbeFailure>>,
) {
    let Endpoints { stdout, stdin, .. } = endpoints;
    let failure = Failure::default();
    let transport = Lines::new(
        Box::pin(outgoing(stdin)),
        Box::pin(incoming(stdout, Arc::clone(&failure))),
    );
    let result = Client
        .v1()
        .name("spark")
        .connect_with(transport, async |cx| {
            Ok(probe_session(&cx, &cwd, &program, sign_in.as_deref()).await)
        })
        .await
        .unwrap_or_else(|error| {
            Err(ProbeFailure {
                lost: true,
                message: describe(&error),
                ..Default::default()
            })
        })
        .map_err(|error| match failure.get() {
            Some(unusable) => unusable.clone().into(),
            None => error,
        });
    let _ = done.send(result);
}

/// `program` is the harness's program and arguments, which a terminal
/// sign-in method extends.
async fn probe_session(
    cx: &ConnectionTo<Agent>,
    cwd: &Path,
    program: &[String],
    sign_in: Option<&str>,
) -> Result<Probed, ProbeFailure> {
    let failed = |error: agent_client_protocol::Error, method: &str| ProbeFailure {
        unauthenticated: is_auth_error(&error),
        lost: agent_client_protocol::is_incoming_transport_closed(&error)
            || agent_client_protocol::is_connection_terminated(&error),
        // Shown where the user signs in, so without the turn's pointer there.
        message: match is_auth_error(&error) {
            true => auth_error(&error),
            false => setup_error(&error, method),
        },
        ..Default::default()
    };
    let init = cx
        .send_request(
            InitializeRequest::new(ProtocolVersion::V1)
                .client_info(Implementation::new("spark", env!("CARGO_PKG_VERSION")))
                // Spark shows a terminal method as the command for the user to run.
                .client_capabilities(
                    ClientCapabilities::new().auth(AuthCapabilities::new().terminal(true)),
                ),
        )
        .block_task()
        .await
        .map_err(|error| failed(error, "initialize"))?;
    if init.protocol_version != ProtocolVersion::V1 {
        return Err(format!(
            "the harness negotiated ACP protocol version {}, but Spark speaks version 1",
            init.protocol_version.as_u16()
        )
        .into());
    }
    let auth_methods = init
        .auth_methods
        .iter()
        .filter_map(|method| match method {
            AuthMethod::Agent(method) => Some(AcpAuthMethod {
                id: method.id.to_string(),
                name: method.name.clone(),
                description: method.description.clone(),
                terminal: None,
            }),
            // Spark has no terminal to run it in; the user runs it in theirs.
            AuthMethod::Terminal(method) => Some(AcpAuthMethod {
                id: method.id.to_string(),
                name: method.name.clone(),
                description: method.description.clone(),
                terminal: Some(
                    BTreeMap::from_iter(&method.env)
                        .into_iter()
                        .map(|(name, value)| format!("{name}={value}"))
                        .chain(program.iter().chain(&method.args).cloned())
                        .collect(),
                ),
            }),
            _ => None,
        })
        .collect::<Vec<_>>();
    if let Some(method) = auth_methods
        .iter()
        .find(|method| Some(method.id.as_str()) == sign_in && method.terminal.is_some())
    {
        return Err(format!("{} is run in a terminal, not by Spark", method.name).into());
    }
    let failed = |error, method: &str| ProbeFailure {
        auth_methods: auth_methods.clone(),
        ..failed(error, method)
    };
    if let Some(method) = sign_in {
        cx.send_request(AuthenticateRequest::new(method.to_string()))
            .block_task()
            .await
            .map_err(|error| failed(error, "authenticate"))?;
    }
    // Options and modes belong to a session; this one never gets a prompt.
    // Sent untyped, so options of types Spark does not know are kept.
    let request = agent_client_protocol::UntypedMessage::new(
        "session/new",
        NewSessionRequest::new(cwd.to_path_buf()),
    )
    .map_err(|error| describe(&error))?;
    let session = cx
        .send_request(request)
        .block_task()
        .await
        .map_err(|error| failed(error, "session/new"))?;
    let capabilities = init.agent_capabilities;
    Ok(Probed {
        resume: capabilities.session_capabilities.resume.is_some(),
        load: capabilities.load_session,
        session,
    })
}

/// Each harness's config options as a session last acknowledged them, by
/// work area, harness ID and launch generation: what its choosers offer now,
/// values that depend on an earlier choice included.
// ponytail: one entry per work area and registration, never pruned.
type AcknowledgedKey = (PathBuf, String, String);
static ACKNOWLEDGED: Mutex<BTreeMap<AcknowledgedKey, Vec<Value>>> = Mutex::new(BTreeMap::new());

/// The options `harness_id` last acknowledged in `cwd` under `generation`.
pub fn acknowledged_options(cwd: &Path, harness_id: &str, generation: &str) -> Option<Vec<Value>> {
    let key = (
        spark_common::paths::canonicalize_clean(cwd).ok()?,
        harness_id.to_string(),
        generation.to_string(),
    );
    ACKNOWLEDGED
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&key)
        .cloned()
}

/// A select option's values in order, with grouped values flattened.
pub fn select_values(option: &Value) -> Vec<&Value> {
    option["options"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|entry| match entry["options"].as_array() {
            Some(group) => group.iter().collect(),
            None => vec![entry],
        })
        .collect()
}

/// The first select option of `category`, such as `model`.
pub fn select_option<'a>(options: &'a [Value], category: &str) -> Option<&'a Value> {
    options
        .iter()
        .find(|option| option["category"] == category && option["type"] == "select")
}

/// Spark's effective support, from what the probe saw and what this backend
/// implements.
fn discovery(probed: Result<Probed, ProbeFailure>) -> AcpDiscovery {
    let unsupported = |reason: &str| Capability::new("unsupported", reason);
    // Fixed by this implementation, whatever a harness reports.
    let mut capabilities = BTreeMap::from([
        ("plan_mode".to_string(), unsupported("Spark can't confirm that an ACP agent's plan mode only reads.")),
        ("steering".to_string(), unsupported("A message sent during a turn waits for the next turn.")),
        ("child_intervention".to_string(), unsupported("Spark can't step into an ACP agent's subagents.")),
        ("forms".to_string(), Capability::new("supported", "Spark shows the agent's questions and forms in chats and flows, and declines one it can't show.")),
        ("per_prompt_usage".to_string(), Capability::new("unverified", "Counted when the agent reports it; otherwise unknown.")),
    ]);
    let probed = match probed {
        Ok(probed) => probed,
        Err(failure) => {
            for name in ["session_resume", "model_selection", "reasoning_effort"] {
                capabilities.insert(
                    name.to_string(),
                    Capability::new("unverified", "Known once the agent is ready."),
                );
            }
            return AcpDiscovery {
                setup_status: if failure.unauthenticated {
                    "unauthenticated"
                } else if failure.missing {
                    "not_installed"
                } else {
                    "unreachable"
                }
                .to_string(),
                error: Some(failure.message),
                auth_methods: failure.auth_methods,
                capabilities,
                config_options: Vec::new(),
                modes: None,
            };
        }
    };
    let options = probed.session["configOptions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    capabilities.insert(
        "session_resume".to_string(),
        if probed.resume {
            Capability::new("supported", "The agent can resume a session.")
        } else if probed.load {
            Capability::new("supported", "The agent can reload a session.")
        } else {
            unsupported("Each turn starts a fresh session, with earlier messages as context.")
        },
    );
    capabilities.insert(
        "model_selection".to_string(),
        match select_option(&options, "model") {
            Some(_) => Capability::new("supported", "The agent offers a choice of model."),
            None => unsupported("The agent offers no choice of model; turns use its default."),
        },
    );
    capabilities.insert(
        "reasoning_effort".to_string(),
        match select_option(&options, "thought_level") {
            Some(_) => Capability::new(
                "supported",
                "The agent offers a choice of reasoning effort.",
            ),
            None => unsupported("The agent offers no choice of reasoning effort."),
        },
    );
    AcpDiscovery {
        setup_status: "ready".to_string(),
        error: None,
        auth_methods: Vec::new(),
        capabilities,
        modes: options
            .is_empty()
            .then(|| {
                probed
                    .session
                    .get("modes")
                    .filter(|modes| !modes.is_null())
                    .cloned()
            })
            .flatten(),
        config_options: options,
    }
}

/// A prompt's ACP usage in Spark's convention. ACP counts each token type
/// once, so its total is their sum; Spark's input includes cache reads and
/// writes, and its output includes thinking.
fn spark_usage(usage: &agent_client_protocol::v1::schema::Usage) -> Value {
    let cached = usage.cached_read_tokens.unwrap_or(0);
    let creation = usage.cached_write_tokens.unwrap_or(0);
    let input = usage.input_tokens + cached + creation;
    let output = usage.output_tokens + usage.thought_tokens.unwrap_or(0);
    json!({
        "input_tokens": input,
        "cached_input_tokens": cached,
        "cache_creation_input_tokens": creation,
        "output_tokens": output,
        "reasoning_tokens": usage.thought_tokens,
        "total_tokens": usage.total_tokens.max(input + output),
    })
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
    clarification: Option<ClarificationHandler>,
    seen: &mut AcpFailure,
) -> Result<AcpTurn, String> {
    let config = crate::config::captured_session_config(&request.metadata)?.unwrap_or_default();
    let harness: AcpHarness = config
        .acp_harnesses
        .get(harness_id)
        .cloned()
        .ok_or_else(|| {
            format!("ACP harness {harness_id} is not registered, or its registration is invalid; check Settings > Models & accounts > Agent harnesses.")
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
    let cwd = spark_common::paths::canonicalize_clean(&request.project_path).map_err(|error| {
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
    let persist = !request.metadata.contains_key("spark.runtime.run_id") && clarification.is_none();

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
    // Session controls chosen for this harness; ones chosen for another
    // provider, such as a flow's launch default a node overrides, are not its.
    let controls = reasoning
        .filter(|settings| {
            settings["provider"]
                .as_str()
                .and_then(|provider| HarnessIdentity::parse(provider).ok().flatten())
                == Some(HarnessIdentity::Acp(harness_id.to_string()))
        })
        .and_then(|settings| settings.get("session_config"))
        .and_then(Value::as_object);
    for (id, value) in controls.into_iter().flatten() {
        let value = value
            .as_str()
            .ok_or_else(|| format!("session control {id:?} must name a value ID"))?;
        selections.push((
            Selector::Id(id.clone(), "chosen in its session controls"),
            value.to_string(),
        ));
    }
    for (id, value) in channel.map(|channel| &channel.config).into_iter().flatten() {
        selections.push((
            Selector::Id(id.clone(), "required by its system_instructions"),
            value.clone(),
        ));
    }

    let attachments = crate::attachments::request_attachments(&request.metadata);
    let earlier = crate::attachments::history_attachments(&request.history);
    let plan = Plan {
        harness_id: harness_id.to_string(),
        harness_label: harness.name.clone(),
        clarification,
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

    let (interrupts, interrupted) = tokio::sync::mpsc::channel(1);
    let (replies, answered) = tokio::sync::mpsc::unbounded_channel();
    let (messages_tx, live_messages) = tokio::sync::mpsc::unbounded_channel();
    let accepting_messages = Arc::new(AtomicBool::new(false));
    let live = Arc::new(Live {
        interrupts,
        stop: Mutex::new(None),
        terminated: AtomicBool::new(false),
        do_not_pool: AtomicBool::new(false),
        closed: AtomicBool::new(false),
        replies: replies.clone(),
        messages: messages_tx,
        accepting_messages: Arc::clone(&accepting_messages),
    });
    let key = (
        request.project_path.clone(),
        request.conversation_id.clone(),
    );
    let registered = {
        let mut entries = registry().lock().unwrap();
        if entries
            .get(&key)
            .and_then(Weak::upgrade)
            .is_some_and(|other| !other.closed.load(Ordering::SeqCst))
        {
            return Err("An ACP turn is already running for this conversation.".into());
        }
        entries.insert(key.clone(), Arc::downgrade(&live));
        Registered(key.clone(), Arc::clone(&live))
    };

    let app_turn_id = uuid::Uuid::new_v4().to_string();
    let (messages, delivered) = std::sync::mpsc::channel();
    let mut pending = Some((
        PromptWork {
            plan,
            out: Out { messages },
            interrupts: interrupted,
            replies,
            answered,
            live_messages,
            accepting_messages,
        },
        delivered,
    ));

    // Reuse a warm chat session when its identity still matches; fall back to
    // a fresh process if the pooled worker exited right before taking the turn.
    let mut worker: Option<LiveWorker> = None;
    let mut done: Option<Result<Finished, String>> = None;
    if persist {
        let pooled = pending
            .as_ref()
            .and_then(|(work, _)| take_pooled_session(&key, &work.plan));
        if let Some(pooled) = pooled {
            let (work, delivered) = pending.take().unwrap();
            attach_stop(&live, &pooled.connection);
            match pooled.turns.send(TurnWork::Prompt(work)) {
                Ok(()) => {
                    let mut rejected = None;
                    for message in delivered {
                        match message {
                            Message::Event(mut event) => {
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
                            Message::Rejected(TurnWork::Prompt(returned)) => {
                                rejected = Some(returned);
                                break;
                            }
                            Message::Rejected(TurnWork::Continuation(_)) => break,
                        }
                    }
                    if let Some(mut returned) = rejected {
                        drop(pooled);
                        let (messages, fresh_delivered) = std::sync::mpsc::channel();
                        returned.out = Out { messages };
                        pending = Some((returned, fresh_delivered));
                    } else {
                        worker = Some(pooled);
                    }
                }
                Err(error) => {
                    drop(pooled);
                    let TurnWork::Prompt(mut returned) = error.0 else {
                        unreachable!("sent PromptWork");
                    };
                    let (messages, fresh_delivered) = std::sync::mpsc::channel();
                    returned.out = Out { messages };
                    pending = Some((returned, fresh_delivered));
                }
            }
        }
    }

    let worker = match worker {
        Some(worker) => worker,
        None => {
            let (work, delivered) = pending.take().unwrap();
            let (command, instruction_file) = launch_command(
                harness_id,
                &harness,
                config.environment_inheritance,
                &cwd,
                work.plan.instructions.as_deref(),
                &request.metadata,
            )?;
            if !found(&command) {
                return Err(format!(
                    "ACP harness {harness_id} executable not found: {}",
                    harness.executable
                ));
            }
            let (turns_tx, turns_rx) = tokio::sync::mpsc::unbounded_channel();
            let alive = Arc::new(AtomicBool::new(true));
            let parked = Arc::new(tokio::sync::Notify::new());
            let pending_continuation = Arc::new(AtomicBool::new(false));
            let alive_worker = Arc::clone(&alive);
            let parked_worker = Arc::clone(&parked);
            let pending_worker = Arc::clone(&pending_continuation);
            let worker_key = key.clone();
            let connection = stdio_transport::launch(command, LIMITS, move |endpoints| {
                drive_session(
                    endpoints,
                    worker_key,
                    work,
                    turns_rx,
                    persist,
                    alive_worker,
                    parked_worker,
                    pending_worker,
                )
            })
            .map_err(|error| format!("ACP harness {harness_id} launch failed: {error}"))?;
            attach_stop(&live, &connection);
            let launched = LiveWorker {
                connection,
                _instruction_file: instruction_file,
                turns: turns_tx,
                alive,
                parked,
                pending_continuation,
            };
            // The sink runs here, off the worker, which keeps servicing the connection.
            for message in delivered {
                match message {
                    Message::Event(mut event) => {
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
                    Message::Rejected(_) => break,
                }
            }
            launched
        }
    };

    let finished = match done {
        Some(Ok(finished)) => finished,
        failed => {
            let stderr = worker.connection.stderr();
            let lines = stderr.lines().collect::<Vec<_>>();
            let tail = lines[lines.len().saturating_sub(10)..].join("\n");
            drop(worker);
            let message = match failed {
                Some(Err(message)) => message,
                _ => "the ACP connection worker stopped".to_string(),
            };
            let message = if tail.is_empty() {
                format!("ACP harness {harness_id}: {message}")
            } else {
                format!("ACP harness {harness_id}: {message}; stderr tail:\n{tail}")
            };
            return Err(message);
        }
    };
    // A stopped process leaves its session in an unknown state: never resume it.
    let record = finished.record.filter(|_| !finished.forced);
    let should_park = persist
        && !finished.forced
        && !live.terminated.load(Ordering::SeqCst)
        && !live.do_not_pool.load(Ordering::SeqCst)
        && record.is_some();
    drop(registered);
    if should_park {
        park_pooled_session(
            key,
            worker,
            SessionIdentity {
                record: record.clone().unwrap(),
                policy: harness.permission_policy,
            },
        );
    } else {
        drop(worker);
    }
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

/// The command that launches `harness` in `cwd`, carrying `instructions`
/// through its declared channel, and the instruction file that must outlive
/// the process. Spark's reserved bindings come last, so nothing overrides them.
fn launch_command(
    harness_id: &str,
    harness: &AcpHarness,
    inheritance: EnvironmentInheritancePolicy,
    cwd: &Path,
    instructions: Option<&str>,
    metadata: &BTreeMap<String, Value>,
) -> Result<(Command, Option<tempfile::NamedTempFile>), String> {
    let channel = harness.system_instructions.as_ref().zip(instructions);
    let instruction_file = channel
        .filter(|(channel, _)| channel.uses_file())
        .map(|(_, text)| {
            let mut file = tempfile::NamedTempFile::new()?;
            file.write_all(text.as_bytes())?;
            file.flush()?;
            Ok::<_, std::io::Error>(file)
        })
        .transpose()
        .map_err(|error| {
            format!("ACP harness {harness_id} instruction file write failed: {error}")
        })?;
    let (channel_env, channel_args) = match channel {
        Some((channel, text)) => channel
            .render(
                text,
                &instruction_file
                    .as_ref()
                    .map(|file| file.path().to_string_lossy().into_owned())
                    .unwrap_or_default(),
            )
            .map_err(|error| format!("ACP harness {harness_id} system_instructions: {error}"))?,
        None => Default::default(),
    };
    let mut command = harness_command(
        harness,
        inheritance,
        Some(cwd),
        channel_env
            .into_iter()
            .map(|(key, value)| (key, OsString::from(value)))
            .chain(crate::agent::agent_turn_env(metadata)),
    );
    command.args(&harness.args).args(channel_args);
    // Windows runs a batch file through cmd.exe, which takes no line break in
    // an argument, as multiline instructions would bring.
    let program = Path::new(command.get_program());
    if cfg!(windows)
        && crate::local_environment::is_batch_file(program)
        && command
            .get_args()
            .any(|arg| arg.to_string_lossy().contains(['\r', '\n']))
    {
        return Err(format!(
            "ACP harness {harness_id} runs the batch file {}, to which Windows cannot pass \
             an argument with line breaks, such as multiline instructions. Pass them through \
             {{{{instructions_file}}}} or an environment variable instead.",
            program.display()
        ));
    }
    Ok((command, instruction_file))
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
/// sink: a slow sink falls behind rather than failing the turn or stalling
/// the worker it may be waiting on.
struct Out {
    // ponytail: unbounded, like the turn's record, which already holds every
    // event.
    messages: std::sync::mpsc::Sender<Message>,
}

impl Out {
    fn event(&self, event: TurnStreamEvent) {
        let _ = self.messages.send(Message::Event(event));
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

fn attach_stop(live: &Live, connection: &stdio_transport::Connection) {
    let stop = connection.stop_handle();
    if live.terminated.load(Ordering::SeqCst) {
        stop.stop();
    }
    *live.stop.lock().unwrap() = Some(stop);
}

fn is_continuation_trigger(dispatch: &Dispatch) -> bool {
    match dispatch {
        Dispatch::Request(request, _) => matches!(
            request.method(),
            "session/request_permission" | "elicitation/create"
        ),
        Dispatch::Notification(notification) => {
            notification.method() == "session/update"
                && matches!(
                    notification.params()["update"]["sessionUpdate"].as_str(),
                    Some(
                        "agent_message_chunk"
                            | "agent_thought_chunk"
                            | "tool_call"
                            | "tool_call_update"
                            | "plan"
                    )
                )
        }
        Dispatch::Response(_, _) => false,
    }
}

#[allow(clippy::too_many_arguments)]
async fn drive_session(
    endpoints: Endpoints,
    key: Key,
    first_work: PromptWork,
    more_turns: UnboundedReceiver<TurnWork>,
    persist: bool,
    alive: Arc<AtomicBool>,
    parked: Arc<tokio::sync::Notify>,
    pending_continuation: Arc<AtomicBool>,
) {
    let Endpoints {
        stdout,
        stdin,
        mut status,
        stop,
    } = endpoints;
    let failure = Failure::default();
    let transport = Lines::new(
        Box::pin(outgoing(stdin)),
        Box::pin(incoming(stdout, Arc::clone(&failure))),
    );
    // Holds the active turn's output channel if `connect_with` itself fails
    // before the closure can deliver `Message::Done`.
    let active_out: Arc<Mutex<Option<Out>>> = Arc::new(Mutex::new(Some(Out {
        messages: first_work.out.messages.clone(),
    })));
    let active_out_closure = Arc::clone(&active_out);
    let failure_closure = Arc::clone(&failure);
    let more_turns_cell = Arc::new(Mutex::new(Some(more_turns)));
    let more_turns_closure = Arc::clone(&more_turns_cell);
    let pending_closure = Arc::clone(&pending_continuation);
    let session_result = Client
        .v1()
        .name("spark")
        .connect_with(transport, async move |cx| {
            let mut more_turns = more_turns_closure.lock().unwrap().take().unwrap();
            let mut runner = Turn::new(first_work, stop);
            let established = runner.run_first(&cx, &failure_closure).await;
            if let Ok(Some((mut session, mut options, record))) = established {
                active_out_closure.lock().unwrap().take();
                if persist {
                    let mut buffered: Vec<Dispatch> = Vec::new();
                    let mut attach_deadline: Option<Instant> = None;
                    let mut wait_for_park = true;
                    // Idle loop between chat turns: keep servicing stdout so
                    // background tasks survive, and wake a continuation turn
                    // whenever the harness emits unsolicited agent work.
                    loop {
                        if wait_for_park {
                            let next = tokio::select! {
                                biased;
                                () = parked.notified() => None,
                                next = more_turns.recv() => match next {
                                    Some(work) => Some(work),
                                    None => break,
                                },
                                _ = status.wait_for(|s| s.exited.is_some() || s.write_failure.is_some()) => break,
                            };
                            wait_for_park = false;
                            if let Some(next_work) = next {
                                pending_closure.store(false, Ordering::SeqCst);
                                attach_deadline = None;
                                let pending_buffered = std::mem::take(&mut buffered);
                                *active_out_closure.lock().unwrap() = Some(Out {
                                    messages: next_work.out_sender(),
                                });
                                let keep = match next_work {
                                    TurnWork::Prompt(work) => {
                                        runner
                                            .run_next(
                                                work,
                                                pending_buffered,
                                                &mut session,
                                                &mut options,
                                                &record,
                                                &failure_closure,
                                            )
                                            .await
                                    }
                                    TurnWork::Continuation(work) => {
                                        runner
                                            .run_continuation(
                                                work,
                                                pending_buffered,
                                                &mut session,
                                                &mut options,
                                                &record,
                                                &failure_closure,
                                            )
                                            .await
                                    }
                                };
                                active_out_closure.lock().unwrap().take();
                                if !keep {
                                    break;
                                }
                                wait_for_park = true;
                                continue;
                            }
                        }

                        let idle_deadline = (buffered.is_empty()
                            && !runner.has_open_background_tools())
                        .then(|| Instant::now() + SESSION_IDLE_TIMEOUT);
                        tokio::select! {
                            next = more_turns.recv() => match next {
                                Some(next_work) => {
                                    pending_closure.store(false, Ordering::SeqCst);
                                    attach_deadline = None;
                                    let pending_buffered = std::mem::take(&mut buffered);
                                    *active_out_closure.lock().unwrap() = Some(Out {
                                        messages: next_work.out_sender(),
                                    });
                                    let keep = match next_work {
                                        TurnWork::Prompt(work) => {
                                            runner
                                                .run_next(
                                                    work,
                                                    pending_buffered,
                                                    &mut session,
                                                    &mut options,
                                                    &record,
                                                    &failure_closure,
                                                )
                                                .await
                                        }
                                        TurnWork::Continuation(work) => {
                                            runner
                                                .run_continuation(
                                                    work,
                                                    pending_buffered,
                                                    &mut session,
                                                    &mut options,
                                                    &record,
                                                    &failure_closure,
                                                )
                                                .await
                                        }
                                    };
                                    active_out_closure.lock().unwrap().take();
                                    if !keep {
                                        break;
                                    }
                                    wait_for_park = true;
                                }
                                None => break,
                            },
                            update = session.read_update() => match update {
                                Ok(SessionMessage::Message(dispatch)) => {
                                    let raw = dispatch.into_raw();
                                    if !buffered.is_empty() {
                                        buffered.push(raw);
                                    } else if is_continuation_trigger(&raw) {
                                        pending_closure.store(true, Ordering::SeqCst);
                                        if notify_continuation(&key.0, &key.1) {
                                            buffered.push(raw);
                                            attach_deadline =
                                                Some(Instant::now() + CONTINUATION_ATTACH_TIMEOUT);
                                        } else {
                                            pending_closure.store(false, Ordering::SeqCst);
                                            if runner
                                                .handle_idle_raw_dispatch(raw, &mut options)
                                                .await
                                                .is_err()
                                            {
                                                break;
                                            }
                                        }
                                    } else if runner
                                        .handle_idle_raw_dispatch(raw, &mut options)
                                        .await
                                        .is_err()
                                    {
                                        break;
                                    }
                                }
                                Ok(_) => {}
                                Err(_) => break,
                            },
                            () = sleep_until(attach_deadline) => {
                                attach_deadline = None;
                                pending_closure.store(false, Ordering::SeqCst);
                                for raw in std::mem::take(&mut buffered) {
                                    if runner
                                        .handle_idle_raw_dispatch(raw, &mut options)
                                        .await
                                        .is_err()
                                    {
                                        break;
                                    }
                                }
                            }
                            // Exit the idle loop immediately if the harness
                            // process dies or stdout/stdin closes between turns.
                            _ = status.wait_for(|s| s.exited.is_some() || s.write_failure.is_some()) => break,
                            () = sleep_until(idle_deadline) => break,
                        }
                    }
                }
            } else {
                active_out_closure.lock().unwrap().take();
            }
            pending_closure.store(false, Ordering::SeqCst);
            more_turns.close();
            while let Ok(work) = more_turns.try_recv() {
                let sender = work.out_sender();
                let _ = sender.send(Message::Rejected(work));
            }
            Ok(())
        })
        .await;
    alive.store(false, Ordering::SeqCst);
    pending_continuation.store(false, Ordering::SeqCst);
    if let Some(mut more_turns) = more_turns_cell.lock().unwrap().take() {
        more_turns.close();
        while let Ok(work) = more_turns.try_recv() {
            let sender = work.out_sender();
            let _ = sender.send(Message::Rejected(work));
        }
    }
    let pending_out = active_out.lock().unwrap().take();
    if let Some(out) = pending_out {
        let error = match session_result {
            Err(error) => failure.get().cloned().unwrap_or_else(|| describe(&error)),
            Ok(()) => failure
                .get()
                .cloned()
                .unwrap_or_else(|| "the ACP connection worker stopped".to_string()),
        };
        let _ = out.messages.send(Message::Done(Err(error)));
    }
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

struct Turn {
    plan: Plan,
    out: Out,
    interrupts: Receiver<()>,
    answered: UnboundedReceiver<FormAnswer>,
    stop: Stop,
    translate: Translate,
    /// Tool call IDs whose permission request Spark denied or cancelled, so
    /// deferred end-of-turn never waits on them.
    denied_tools: HashSet<String>,
    /// This launch's connection, which every form it asks is tied to.
    connection: String,
    /// Forms the harness waits on, by Spark request ID.
    forms: HashMap<String, PendingForm>,
    /// Where a flow's answered forms come back.
    replies: UnboundedSender<FormAnswer>,
    /// User messages arriving while this turn is live.
    live_messages: UnboundedReceiver<LiveMessage>,
    /// Shared flag telling `send_message` when the turn is between prompts.
    accepting_messages: Arc<AtomicBool>,
}

/// A harness's `elicitation/create` waiting for its reply.
struct PendingForm {
    responder: Responder<Value, Raw>,
    /// The question as asked, for its expiry.
    request: Value,
    /// Tells a flow's wait the form is no longer pending.
    gone: Arc<AtomicBool>,
}

/// Withdraws every form still pending on any exit, such as a lost connection.
impl Drop for Turn {
    fn drop(&mut self) {
        self.close_live_messages();
        self.expire_forms();
    }
}

type Session = ActiveSession<'static, Agent>;

/// A session ready for the primary prompt.
struct Ready {
    session: Session,
    options: Vec<SessionConfigOption>,
    record: AcpSessionRecord,
    reset: Option<String>,
    resumed: bool,
    /// Instructions the first prompt of a fresh session carries.
    in_prompt: bool,
}

/// State after `PromptResponse(EndTurn)` arrived while background tool calls
/// were still `in_progress`.
#[derive(Debug, PartialEq, Eq)]
enum DeferredEnd {
    /// At least one non-daemon tool call is still `in_progress`.
    WaitingForTools,
    /// All open tool calls settled; waiting for the harness's post-tool wakeup.
    AwaitingWakeup(Instant),
    /// Post-tool wakeup is streaming; settles after quiet/usage_update grace.
    StreamingWakeup(Instant),
}

impl DeferredEnd {
    fn deadline(&self) -> Option<Instant> {
        match self {
            Self::WaitingForTools => None,
            Self::AwaitingWakeup(deadline) | Self::StreamingWakeup(deadline) => Some(*deadline),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdateKind {
    Tool,
    Usage,
    AssistantMessage,
    Progress,
    Other,
}

fn update_kind(update: &SessionUpdate) -> UpdateKind {
    match update {
        SessionUpdate::ToolCall(_) | SessionUpdate::ToolCallUpdate(_) => UpdateKind::Tool,
        SessionUpdate::UsageUpdate(_) => UpdateKind::Usage,
        SessionUpdate::AgentMessageChunk(_) => UpdateKind::AssistantMessage,
        SessionUpdate::AgentThoughtChunk(_) | SessionUpdate::Plan(_) => UpdateKind::Progress,
        _ => UpdateKind::Other,
    }
}

fn advance_deferred_at(
    current: DeferredEnd,
    seen_kind: Option<UpdateKind>,
    has_open_background_tools: bool,
    has_open_assistant: bool,
    now: Instant,
) -> DeferredEnd {
    if has_open_background_tools {
        return DeferredEnd::WaitingForTools;
    }
    match (current, seen_kind) {
        (DeferredEnd::WaitingForTools, _) => {
            DeferredEnd::AwaitingWakeup(now + POST_TOOL_WAKEUP_WAIT)
        }
        (_, Some(UpdateKind::Usage)) if has_open_assistant => {
            DeferredEnd::StreamingWakeup(now + POST_WAKEUP_USAGE_SETTLE)
        }
        (DeferredEnd::StreamingWakeup(_), Some(UpdateKind::Usage)) => {
            DeferredEnd::StreamingWakeup(now + POST_WAKEUP_IN_PROGRESS_IDLE)
        }
        (_, Some(UpdateKind::AssistantMessage)) => {
            DeferredEnd::StreamingWakeup(now + POST_WAKEUP_STREAM_IDLE)
        }
        (_, Some(UpdateKind::Progress | UpdateKind::Tool)) => {
            DeferredEnd::StreamingWakeup(now + POST_WAKEUP_IN_PROGRESS_IDLE)
        }
        (state, _) => state,
    }
}

impl Turn {
    fn new(work: PromptWork, stop: Stop) -> Self {
        let harness_id = work.plan.harness_id.clone();
        Self {
            plan: work.plan,
            out: work.out,
            interrupts: work.interrupts,
            answered: work.answered,
            stop,
            translate: Translate::new(&harness_id),
            denied_tools: HashSet::new(),
            connection: uuid::Uuid::new_v4().to_string(),
            forms: HashMap::new(),
            replies: work.replies,
            live_messages: work.live_messages,
            accepting_messages: work.accepting_messages,
        }
    }

    fn can_accept_live_message(deferred: Option<&DeferredEnd>, cancelled: bool) -> bool {
        !cancelled
            && matches!(
                deferred,
                Some(DeferredEnd::WaitingForTools | DeferredEnd::AwaitingWakeup(_))
            )
    }

    fn close_live_messages(&mut self) {
        self.accepting_messages.store(false, Ordering::SeqCst);
        self.live_messages.close();
        while let Ok(live_message) = self.live_messages.try_recv() {
            let _ = live_message.delivered.send(false);
        }
    }

    fn has_open_background_tools(&self) -> bool {
        self.translate.has_open_background_tools(&self.denied_tools)
    }

    async fn run_first(
        &mut self,
        cx: &ConnectionTo<Agent>,
        failure: &Failure,
    ) -> Result<Option<(Session, Vec<SessionConfigOption>, AcpSessionRecord)>, ()> {
        let mut interrupts =
            std::mem::replace(&mut self.interrupts, tokio::sync::mpsc::channel(1).1);
        // An interrupt before the prompt is sent skips the prompt.
        let ready = tokio::select! {
            biased;
            Some(()) = interrupts.recv() => Ok(None),
            ready = self.setup(cx) => ready.map(Some),
        };
        self.interrupts = interrupts;
        let ready = match ready {
            Ok(Some(ready)) => ready,
            Ok(None) => {
                let finished = self.finish(StopReason::Cancelled, None, None, None, false);
                let _ = self.out.messages.send(Message::Done(Ok(finished)));
                return Ok(None);
            }
            Err(error) => {
                let error = failure.get().cloned().unwrap_or(error);
                let _ = self.out.messages.send(Message::Done(Err(error)));
                return Err(());
            }
        };
        let Ready {
            mut session,
            mut options,
            record,
            reset,
            resumed,
            in_prompt,
        } = ready;
        let mut text = self.plan.prompt.clone();
        if let Some(context) = self.plan.fresh_context.as_ref().filter(|_| !resumed) {
            text = format!("{context}\n\n---\n\n{text}");
        }
        // ponytail: instructions sent as prompt text weigh less than a system
        // prompt and may fade in a long or compacted session.
        if let Some(instructions) = self
            .plan
            .instructions
            .as_ref()
            .filter(|_| in_prompt && !resumed)
        {
            text = format!("<instructions>\n{instructions}\n</instructions>\n\n{text}");
        }
        let _ = self.out.messages.send(Message::Prompted);
        if let Err(error) = session
            .send_prompt(text)
            .map_err(|error| format!("session/prompt failed: {}", describe(&error)))
        {
            let error = failure.get().cloned().unwrap_or(error);
            let _ = self.out.messages.send(Message::Done(Err(error)));
            return Err(());
        }
        let result = self
            .prompt(&mut session, &mut options, record.clone(), reset)
            .await
            .map_err(|error| failure.get().cloned().unwrap_or(error));
        self.close_live_messages();
        if result.is_err() {
            self.expire_forms();
        }
        let keep = result
            .as_ref()
            .is_ok_and(|finished| !finished.forced && finished.record.is_some());
        let _ = self.out.messages.send(Message::Done(result));
        Ok(keep.then_some((session, options, record)))
    }

    async fn run_next(
        &mut self,
        work: PromptWork,
        buffered: Vec<Dispatch>,
        session: &mut Session,
        options: &mut Vec<SessionConfigOption>,
        record: &AcpSessionRecord,
        failure: &Failure,
    ) -> bool {
        self.close_live_messages();
        self.expire_forms();
        self.plan = work.plan;
        self.out = work.out;
        self.interrupts = work.interrupts;
        self.answered = work.answered;
        self.replies = work.replies;
        self.live_messages = work.live_messages;
        self.accepting_messages = work.accepting_messages;
        self.denied_tools.clear();
        self.translate.begin_turn();

        if self.interrupts.try_recv().is_ok() {
            let finished = self.finish(
                StopReason::Cancelled,
                None,
                Some(record.clone()),
                None,
                false,
            );
            let _ = self.out.messages.send(Message::Done(Ok(finished)));
            return true;
        }
        self.out.event(self.translate.event(
            TurnStreamEventKind::Other(ACP_SESSION_ESTABLISHED.into()),
            ACP_SESSION_ESTABLISHED,
            |event| {
                event.details = Some(json!(record));
                event.status = Some("resumed".into());
            },
        ));
        self.plan.acknowledge(options);
        if let Err(error) = self.select(session, options).await {
            let error = failure.get().cloned().unwrap_or(error);
            let _ = self.out.messages.send(Message::Done(Err(error)));
            return false;
        }
        let _ = self.out.messages.send(Message::Prompted);
        if let Err(error) = session
            .send_prompt(self.plan.prompt.clone())
            .map_err(|error| format!("session/prompt failed: {}", describe(&error)))
        {
            let error = failure.get().cloned().unwrap_or(error);
            let _ = self.out.messages.send(Message::Done(Err(error)));
            return false;
        }
        let result = self
            .prompt_with_initial(session, options, record.clone(), None, None, buffered)
            .await
            .map_err(|error| failure.get().cloned().unwrap_or(error));
        self.close_live_messages();
        if result.is_err() {
            self.expire_forms();
        }
        let keep = result
            .as_ref()
            .is_ok_and(|finished| !finished.forced && finished.record.is_some());
        let _ = self.out.messages.send(Message::Done(result));
        keep
    }

    async fn run_continuation(
        &mut self,
        work: ContinuationWork,
        buffered: Vec<Dispatch>,
        session: &mut Session,
        options: &mut Vec<SessionConfigOption>,
        record: &AcpSessionRecord,
        failure: &Failure,
    ) -> bool {
        self.close_live_messages();
        self.expire_forms();
        self.out = work.out;
        self.interrupts = work.interrupts;
        self.answered = work.answered;
        self.replies = work.replies;
        self.live_messages = work.live_messages;
        self.accepting_messages = work.accepting_messages;
        self.denied_tools.clear();
        self.translate.begin_turn();

        self.out.event(self.translate.event(
            TurnStreamEventKind::Other(ACP_SESSION_ESTABLISHED.into()),
            ACP_SESSION_ESTABLISHED,
            |event| {
                event.details = Some(json!(record));
                event.status = Some("resumed".into());
            },
        ));
        let initial_deferred = Some(DeferredEnd::StreamingWakeup(
            Instant::now() + POST_WAKEUP_IN_PROGRESS_IDLE,
        ));
        let result = self
            .prompt_with_initial(
                session,
                options,
                record.clone(),
                None,
                initial_deferred,
                buffered,
            )
            .await
            .map_err(|error| failure.get().cloned().unwrap_or(error));
        self.close_live_messages();
        if result.is_err() {
            self.expire_forms();
        }
        let keep = result
            .as_ref()
            .is_ok_and(|finished| !finished.forced && finished.record.is_some());
        let _ = self.out.messages.send(Message::Done(result));
        keep
    }

    /// Services notifications and requests while parked idle between turns so
    /// the harness stays healthy and stdout never stalls.
    async fn handle_idle_raw_dispatch(
        &mut self,
        dispatch: Dispatch,
        options: &mut Vec<SessionConfigOption>,
    ) -> Result<(), String> {
        let dispatch = match dispatch {
            Dispatch::Request(request, responder) if request.method() == "elicitation/create" => {
                let error = agent_client_protocol::Error::new(
                    ErrorCode::INVALID_REQUEST,
                    "no active turn to answer elicitation",
                );
                let _ = responder.respond_with_error(error);
                return Ok(());
            }
            dispatch => dispatch,
        };
        let policy = self.plan.policy;
        let (translate, plan, denied_tools) =
            (&mut self.translate, &self.plan, &mut self.denied_tools);
        MatchDispatch::new(dispatch)
            .if_notification(async |notification: SessionNotification| {
                if let SessionUpdate::ConfigOptionUpdate(update) = &notification.update {
                    *options = update.config_options.clone();
                    plan.acknowledge(options);
                }
                let _ = translate.update(notification.update);
                Ok(())
            })
            .await
            .if_request(async |request: RequestPermissionRequest, responder| {
                let (response, _, denied) = permission(&request, policy);
                if denied {
                    denied_tools.insert(request.tool_call.tool_call_id.0.to_string());
                }
                responder.respond(response)
            })
            .await
            .otherwise_ignore()
            .map_err(|error| format!("malformed message from the harness: {}", describe(&error)))
    }

    /// Initializes, then resumes, loads or creates the session and applies
    /// the required selections.
    async fn setup(&mut self, cx: &ConnectionTo<Agent>) -> Result<Ready, String> {
        let id = self.plan.harness_id.clone();
        let init = cx
            .send_request(
                InitializeRequest::new(ProtocolVersion::V1)
                    .client_info(Implementation::new("spark", env!("CARGO_PKG_VERSION")))
                    .client_capabilities(ClientCapabilities::new().elicitation(
                        ElicitationCapabilities::new().form(ElicitationFormCapabilities::new()),
                    )),
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
        // Without a channel or ACP system prompts, the instructions lead the
        // first prompt of each fresh session instead.
        let in_prompt = self.plan.instructions.is_some()
            && !self.plan.channel_rendered
            && capabilities.session_capabilities.system_prompt.is_none();
        let system_prompt = self
            .plan
            .instructions
            .as_ref()
            .filter(|_| !self.plan.channel_rendered && !in_prompt)
            .map(|text| SystemPrompt::new(vec![ContentBlock::from(text.clone())]));

        let current = AcpSessionRecord {
            harness: id.clone(),
            session_id: String::new(),
            cwd: self.plan.cwd.to_string_lossy().into_owned(),
            configuration_generation: self.plan.generation.clone(),
            instructions_sha256: self.plan.instructions_sha256.clone(),
        };
        let mut reset = None;
        let mut restored = None;
        if let Some(saved) = &self.plan.saved {
            let restore = if !saved.continues_as(&current) {
                Err("its harness, work area, configuration or instructions changed".to_string())
            } else if capabilities.session_capabilities.resume.is_some() {
                let request = ResumeSessionRequest::new(saved.session_id.clone(), &self.plan.cwd)
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
                let request = LoadSessionRequest::new(saved.session_id.clone(), &self.plan.cwd)
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
                    NewSessionRequest::new(&self.plan.cwd).system_prompt(system_prompt.clone());
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

        self.plan.acknowledge(&options);
        self.select(&session, &mut options).await?;
        Ok(Ready {
            session,
            options,
            record,
            reset,
            resumed,
            in_prompt,
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
                    Selector::Id(id, _) => option.id.0.as_ref() == id,
                })
                .ok_or_else(|| match &selector {
                    Selector::Category(_, label) => format!(
                        "the harness offers no {label} selection, so {label} {value:?} cannot be applied"
                    ),
                    Selector::Id(id, source) => format!(
                        "the harness offers no config option {id:?} {source}"
                    ),
                })?;
            let config_id = option.id.0.to_string();
            let SessionConfigKind::Select(select) = &option.kind else {
                return Err(format!(
                    "config option {config_id:?} is not a select option"
                ));
            };
            let offered: Vec<&str> = match &select.options {
                SessionConfigSelectOptions::Ungrouped(options) => options
                    .iter()
                    .map(|option| option.value.0.as_ref())
                    .collect(),
                SessionConfigSelectOptions::Grouped(groups) => groups
                    .iter()
                    .flat_map(|group| &group.options)
                    .map(|option| option.value.0.as_ref())
                    .collect(),
                _ => Vec::new(),
            };
            if !offered.contains(&value.as_str()) {
                return Err(format!(
                    "config option {config_id:?} does not offer the value {value:?}; it offers {offered:?}. Choose one of those for this chat or flow step, or run it on another harness"
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
                self.plan.acknowledge(options);
            }
            chosen.insert(config_id, value);
        }
        // A later acknowledgement may change or drop an earlier selection;
        // the prompt runs only under the state every selection asked for.
        for (config_id, value) in &chosen {
            let current = options
                .iter()
                .find(|option| option.id.0.as_ref() == config_id)
                .and_then(|option| match &option.kind {
                    SessionConfigKind::Select(select) => Some(select.current_value.0.to_string()),
                    _ => None,
                });
            match current {
                Some(current) if current == *value => {}
                Some(current) => {
                    return Err(format!(
                        "config option {config_id:?} is {current:?} after the later selections, not the selected {value:?}"
                    ))
                }
                None => {
                    return Err(format!(
                        "the harness no longer offers config option {config_id:?} after the later selections, so {value:?} does not hold"
                    ))
                }
            }
        }
        Ok(())
    }

    /// Runs the primary prompt to its response, servicing updates, permission
    /// requests, forms and interrupts meanwhile. If `end_turn` arrives while a
    /// non-daemon tool call is still `in_progress`, stays open until the tool
    /// and any post-tool wakeup settle.
    async fn prompt(
        &mut self,
        session: &mut Session,
        options: &mut Vec<SessionConfigOption>,
        record: AcpSessionRecord,
        reset: Option<String>,
    ) -> Result<Finished, String> {
        self.prompt_with_initial(session, options, record, reset, None, Vec::new())
            .await
    }

    async fn handle_turn_raw_dispatch(
        &mut self,
        dispatch: Dispatch,
        options: &mut Vec<SessionConfigOption>,
        withdrawn: &mut FuturesUnordered<BoxFuture<'static, String>>,
    ) -> Result<Option<UpdateKind>, String> {
        let dispatch = match dispatch {
            Dispatch::Request(request, responder) if request.method() == "elicitation/create" => {
                if let Some((request_id, cancellation)) = self.elicit(request.params(), responder) {
                    withdrawn.push(Box::pin(async move {
                        cancellation.cancelled().await;
                        request_id
                    }));
                }
                return Ok(None);
            }
            dispatch => dispatch,
        };
        let (translate, out, plan, denied_tools) = (
            &mut self.translate,
            &self.out,
            &self.plan,
            &mut self.denied_tools,
        );
        let policy = plan.policy;
        let mut seen_kind = None;
        MatchDispatch::new(dispatch)
            .if_notification(async |notification: SessionNotification| {
                seen_kind = Some(update_kind(&notification.update));
                if let SessionUpdate::ConfigOptionUpdate(update) = &notification.update {
                    *options = update.config_options.clone();
                    plan.acknowledge(options);
                }
                for event in translate.update(notification.update) {
                    out.event(event);
                }
                Ok(())
            })
            .await
            .if_request(async |request: RequestPermissionRequest, responder| {
                let (response, refused, denied) = permission(&request, policy);
                if denied {
                    denied_tools.insert(request.tool_call.tool_call_id.0.to_string());
                }
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
        Ok(seen_kind)
    }

    fn advance_deferred(&self, current: DeferredEnd, seen_kind: Option<UpdateKind>) -> DeferredEnd {
        advance_deferred_at(
            current,
            seen_kind,
            self.has_open_background_tools(),
            self.translate.has_open_assistant(),
            Instant::now(),
        )
    }

    async fn prompt_with_initial(
        &mut self,
        session: &mut Session,
        options: &mut Vec<SessionConfigOption>,
        record: AcpSessionRecord,
        reset: Option<String>,
        initial_deferred: Option<DeferredEnd>,
        buffered: Vec<Dispatch>,
    ) -> Result<Finished, String> {
        let connection = session.connection().clone();
        let session_id = session.session_id().clone();
        let mut cancelled = false;
        let mut cancel_deadline = None;
        let mut forced = false;
        let mut deferred = initial_deferred;
        let mut prompt_usage: Option<Value> = None;
        // Forms the harness withdrew with `$/cancel_request`.
        let mut withdrawn = FuturesUnordered::<BoxFuture<'static, String>>::new();
        for dispatch in buffered {
            let seen_kind = self
                .handle_turn_raw_dispatch(dispatch, options, &mut withdrawn)
                .await?;
            if let Some(current) = deferred.take() {
                deferred = Some(self.advance_deferred(current, seen_kind));
            }
        }
        loop {
            self.accepting_messages.store(
                Self::can_accept_live_message(deferred.as_ref(), cancelled),
                Ordering::SeqCst,
            );
            let deferred_deadline = deferred
                .as_ref()
                .filter(|_| !cancelled)
                .and_then(DeferredEnd::deadline);
            tokio::select! {
                update = session.read_update() => match update {
                    Ok(SessionMessage::Message(dispatch)) => {
                        let seen_kind = self
                            .handle_turn_raw_dispatch(dispatch.into_raw(), options, &mut withdrawn)
                            .await?;
                        if let Some(current) = deferred.take() {
                            if cancelled {
                                deferred = Some(current);
                                if !self.has_open_background_tools() {
                                    return Ok(self.finish(
                                        StopReason::Cancelled,
                                        prompt_usage,
                                        Some(record),
                                        reset,
                                        forced,
                                    ));
                                }
                            } else {
                                deferred = Some(self.advance_deferred(current, seen_kind));
                            }
                        }
                    }
                    Ok(SessionMessage::PromptResponse(Ok(response))) => {
                        let PromptResponse { stop_reason, usage, .. } = response;
                        let usage = usage.as_ref().map(spark_usage);
                        if let Some(usage) = &usage {
                            self.out.event(self.translate.event(
                                TurnStreamEventKind::TokenUsageUpdated,
                                "prompt_usage",
                                |event| event.token_usage = Some(usage.clone()),
                            ));
                        }
                        prompt_usage = usage;
                        if stop_reason == StopReason::EndTurn
                            && !cancelled
                            && self.has_open_background_tools()
                        {
                            // Keep the turn open while harness-owned background
                            // tools finish; any message so far is commentary.
                            for event in self.translate.close(false) {
                                self.out.event(event);
                            }
                            deferred = Some(DeferredEnd::WaitingForTools);
                            continue;
                        }
                        return Ok(self.finish(
                            stop_reason,
                            prompt_usage,
                            Some(record),
                            reset,
                            forced,
                        ));
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
                Some(answer) = self.answered.recv() => self.deliver(answer)?,
                Some(live_message) = self.live_messages.recv() => {
                    let accepted = if Self::can_accept_live_message(deferred.as_ref(), cancelled) {
                        for event in self.translate.close(false) {
                            self.out.event(event);
                        }
                        match session.send_prompt(live_message.message.clone()) {
                            Ok(()) => {
                                self.out.event(self.translate.event(
                                    TurnStreamEventKind::Other("user_message_delivered".into()),
                                    "user_message_delivered",
                                    |event| {
                                        event.source.item_id = Some(live_message.message_id);
                                        event.message = Some(live_message.message);
                                        event.status = Some("delivered".into());
                                    },
                                ));
                                deferred = None;
                                true
                            }
                            Err(_) => false,
                        }
                    } else {
                        false
                    };
                    let _ = live_message.delivered.send(accepted);
                }
                Some(request_id) = withdrawn.next(), if !withdrawn.is_empty() => {
                    // Dropping the responder sends the cancelled request's error.
                    if let Some(PendingForm { request, gone, .. }) = self.forms.remove(&request_id) {
                        self.expired(request, gone);
                    }
                }
                Some(()) = self.interrupts.recv(), if !cancelled => {
                    cancelled = true;
                    self.expire_forms();
                    if deferred.is_some() {
                        let _ = connection.send_notification(CancelNotification::new(session_id.clone()));
                        if !self.has_open_background_tools() {
                            return Ok(self.finish(
                                StopReason::Cancelled,
                                prompt_usage,
                                Some(record),
                                reset,
                                false,
                            ));
                        }
                        cancel_deadline = Some(Instant::now() + CANCEL_GRACE);
                    } else {
                        cancel_deadline = Some(Instant::now() + CANCEL_GRACE);
                        connection
                            .send_notification(CancelNotification::new(session_id.clone()))
                            .map_err(|error| format!("session/cancel failed: {}", describe(&error)))?;
                    }
                }
                () = sleep_until(deferred_deadline) => {
                    return Ok(self.finish(
                        StopReason::EndTurn,
                        prompt_usage,
                        Some(record),
                        reset,
                        forced,
                    ));
                }
                () = sleep_until(cancel_deadline) => {
                    cancel_deadline = None;
                    forced = true;
                    self.stop.stop();
                    if deferred.is_some() {
                        return Ok(self.finish(
                            StopReason::Cancelled,
                            prompt_usage,
                            Some(record),
                            reset,
                            true,
                        ));
                    }
                }
            }
        }
    }

    /// Asks the user a harness's form: a flow through its clarification
    /// handler, a chat as a pending question [`answer`] replies to. Returns
    /// the pending form's Spark request ID and its cancellation, or `None`
    /// when the request was refused: only form mode is advertised, and a
    /// schema Spark cannot show fails explicitly rather than in part.
    fn elicit(
        &mut self,
        params: &Value,
        responder: Responder<Value, Raw>,
    ) -> Option<(String, agent_client_protocol::RequestCancellation)> {
        let refuse = |responder: Responder<Value, Raw>, message: String| {
            let error = agent_client_protocol::Error::new(ErrorCode::INVALID_PARAMS, message);
            let _ = responder.respond_with_error(error);
        };
        if params["mode"] != "form" {
            refuse(responder, "Spark supports only form elicitation".into());
            return None;
        }
        let schema = &params["requestedSchema"];
        if let Err(reason) = spark_common::forms::check_schema(schema) {
            let message = format!(
                "ACP harness {} asked for a form Spark cannot show: {reason}",
                self.plan.harness_id
            );
            self.out.event(self.translate.event(
                TurnStreamEventKind::Other("warning".into()),
                "elicitation_unsupported",
                |event| event.message = Some(message.clone()),
            ));
            self.translate.log(message.clone());
            refuse(responder, message);
            return None;
        }
        let request_id = format!("acp-form-{}", uuid::Uuid::new_v4());
        // The form names its Spark request ID, so a person sees which request they answer.
        let form = json!({
            "request_id": request_id,
            "harness": HarnessIdentity::Acp(self.plan.harness_id.clone()).selector(),
            "harness_label": self.plan.harness_label,
            "message": params["message"],
            "schema": schema,
        });
        let request = json!({"request_id": request_id, "questions": [], "form": form});
        let correlation = json!({
            "connection": self.connection,
            "session_id": params["sessionId"],
            "tool_call_id": params["toolCallId"],
            "rpc_request_id": responder.raw_id(),
        });
        self.out.event(self.translate.event(
            TurnStreamEventKind::RequestUserInputRequested,
            "elicitation_requested",
            |event| {
                event.request_user_input = Some(request.clone());
                event.source.item_id = Some(request_id.clone());
                event.details = Some(correlation);
            },
        ));
        let gone = Arc::new(AtomicBool::new(false));
        if let Some(handler) = self.plan.clarification.clone() {
            let (replies, gone, request_id) =
                (self.replies.clone(), Arc::clone(&gone), request_id.clone());
            // Polled off the worker, which keeps servicing the connection.
            std::thread::spawn(move || {
                let waited = (|| {
                    let mut poll = handler(json!({"agent_request_id": request_id, "form": form}))?;
                    while !gone.load(Ordering::SeqCst) {
                        match poll()? {
                            Some(ClarificationReply::Form(reply)) => return Ok(Some(reply)),
                            Some(ClarificationReply::Answers(_)) => {
                                return Err("a form was answered with question answers".to_string())
                            }
                            None => std::thread::sleep(FORM_POLL),
                        }
                    }
                    Ok(None)
                })();
                if let Some(reply) = waited.transpose() {
                    let _ = replies.send(FormAnswer {
                        request_id,
                        reply,
                        delivered: None,
                    });
                }
            });
        }
        let cancellation = responder.cancellation();
        self.forms.insert(
            request_id.clone(),
            PendingForm {
                responder,
                request,
                gone,
            },
        );
        Some((request_id, cancellation))
    }

    /// Sends a reply to its pending form. A reply for no pending form, being
    /// stale or a duplicate, is refused and changes nothing.
    fn deliver(&mut self, answer: FormAnswer) -> Result<(), String> {
        let FormAnswer {
            request_id,
            reply,
            delivered,
        } = answer;
        let Some(form) = self.forms.remove(&request_id) else {
            if let Some(delivered) = delivered {
                let _ = delivered.send(false);
            }
            return Ok(());
        };
        form.gone.store(true, Ordering::SeqCst);
        let reply = reply.map_err(|error| format!("the form's answer failed: {error}"))?;
        let sent = form.responder.respond(json!(reply)).is_ok();
        if let Some(delivered) = delivered {
            let _ = delivered.send(sent);
        }
        Ok(())
    }

    /// Cancels every pending form with the harness and expires its question.
    fn expire_forms(&mut self) {
        for (_, form) in std::mem::take(&mut self.forms) {
            let PendingForm {
                responder,
                request,
                gone,
            } = form;
            let _ = responder.respond(json!(FormReply::Cancel));
            self.expired(request, gone);
        }
    }

    /// Reports a form that can no longer be answered.
    fn expired(&mut self, mut request: Value, gone: Arc<AtomicBool>) {
        gone.store(true, Ordering::SeqCst);
        request["status"] = json!("expired");
        let request_id = request["request_id"].as_str().map(str::to_string);
        self.out.event(self.translate.event(
            TurnStreamEventKind::RequestUserInputRequested,
            "elicitation_expired",
            |event| {
                event.request_user_input = Some(request);
                event.source.item_id = request_id;
                event.status = Some("expired".into());
            },
        ));
    }

    fn finish(
        &mut self,
        stop_reason: StopReason,
        usage: Option<Value>,
        record: Option<AcpSessionRecord>,
        reset: Option<String>,
        forced: bool,
    ) -> Finished {
        self.expire_forms();
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

/// What the harness said when it asked to be signed in, without the
/// protocol's error code.
fn auth_error(error: &agent_client_protocol::Error) -> String {
    match &error.data {
        Some(data) => format!(
            "the agent needs you to sign in. It said: {}: {data}",
            error.message
        ),
        None => format!("the agent needs you to sign in. It said: {}", error.message),
    }
}

fn setup_error(error: &agent_client_protocol::Error, method: &str) -> String {
    if is_auth_error(error) {
        format!(
            "{}. Sign in from Settings > Models & accounts > Agent harnesses, then retry",
            auth_error(error)
        )
    } else {
        format!("{method} failed: {}", describe(error))
    }
}

/// The policy's answer: an offered option of the allowed (or, for `deny`,
/// rejecting) kinds, else the cancellation outcome with a diagnostic, and
/// whether permission was denied/cancelled.
fn permission(
    request: &RequestPermissionRequest,
    policy: AcpPermissionPolicy,
) -> (RequestPermissionResponse, Option<String>, bool) {
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
            policy == AcpPermissionPolicy::Deny,
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
            true,
        ),
    }
}

// ---- event translation ---------------------------------------------------

/// An assistant or reasoning message being streamed.
struct Open {
    channel: TurnStreamChannel,
    item_id: String,
    /// Whether `item_id` is the agent's own message id.
    named: bool,
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

fn is_in_progress_tool(raw: &Value) -> bool {
    matches!(
        raw["status"].as_str(),
        Some("in_progress" | "inProgress" | "running")
    )
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

    /// Resets per-turn state before another prompt on the same live session,
    /// keeping only still-open tool calls (such as long-lived daemons).
    fn begin_turn(&mut self) {
        self.open = None;
        self.last_assistant = None;
        self.logs.clear();
        self.tool_calls.retain(|_, raw| is_in_progress_tool(raw));
    }

    /// Whether any non-daemon, non-denied tool call is still `in_progress`.
    fn has_open_background_tools(&self, denied: &HashSet<String>) -> bool {
        self.tool_calls.iter().any(|(id, raw)| {
            !denied.contains(id)
                && is_in_progress_tool(raw)
                && raw["rawInput"]["IsDaemon"].as_bool() != Some(true)
        })
    }

    /// Whether an assistant answer message is currently open and uninterrupted.
    fn has_open_assistant(&self) -> bool {
        self.open
            .as_ref()
            .is_some_and(|open| open.channel == TurnStreamChannel::Assistant)
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
                let mut events = self.interrupt();
                let id = call.tool_call_id.0.to_string();
                let raw = serde_json::to_value(&call).unwrap_or_default();
                self.tool_calls.insert(id.clone(), raw);
                events.push(self.tool_event(&id, true));
                events
            }
            SessionUpdate::ToolCallUpdate(update) => {
                let mut events = self.interrupt();
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
                let mut events = self.interrupt();
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
        let message_id = chunk.message_id.map(|id| id.0.to_string());
        let mut events = Vec::new();
        // A new channel or a new message id starts a new message.
        if self.open.as_ref().is_some_and(|open| {
            open.channel != channel
                || message_id
                    .as_ref()
                    .is_some_and(|id| !open.named || *id != open.item_id)
        }) {
            events = self.close(false);
        }
        let open = self.open.get_or_insert_with(|| {
            self.items += 1;
            Open {
                channel: channel.clone(),
                named: message_id.is_some(),
                item_id: message_id.unwrap_or_else(|| format!("message-{}", self.items)),
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

    /// Completes the open message a tool call or plan interrupts. A message
    /// the agent named continues around them until its id changes.
    fn interrupt(&mut self) -> Vec<TurnStreamEvent> {
        match &self.open {
            Some(open) if open.named => Vec::new(),
            _ => self.close(false),
        }
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
        let raw_title = raw["title"].as_str().unwrap_or("tool");
        let is_execute = raw["kind"] == "execute";
        let command = raw["rawInput"]["command"]
            .as_str()
            .or_else(|| raw["rawInput"]["CommandLine"].as_str())
            .or_else(|| raw["rawInput"]["cmd"].as_str())
            .or_else(|| {
                is_execute
                    .then_some(raw_title)
                    .filter(|title| !title.trim().is_empty())
            });
        let title = if is_execute {
            raw["rawInput"]["toolSummary"]
                .as_str()
                .map(str::trim)
                .filter(|summary| !summary.is_empty())
                .or_else(|| {
                    raw_title
                        .lines()
                        .map(str::trim)
                        .find(|line| !line.is_empty())
                })
                .unwrap_or("tool")
                .to_string()
        } else {
            raw_title.to_string()
        };
        let mut tool_call = json!({
            "id": id,
            "name": raw["name"].as_str().unwrap_or(&title),
            "title": title,
            "kind": if is_execute { "command_execution" } else { "dynamic_tool" },
            "acp_kind": raw["kind"].as_str().unwrap_or("other"),
            "status": status,
            "input": raw["rawInput"],
        });
        if let Some(command) = command {
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
        // Agents such as Gemini CLI report results only as text content.
        let content = raw["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item["content"]["text"].as_str())
            .collect::<Vec<_>>();
        if !raw["rawOutput"].is_null() {
            tool_call["output"] = raw["rawOutput"].clone();
        } else if !content.is_empty() {
            tool_call["output"] = json!(content.join("\n"));
        }
        // A diff without its old text is a new file.
        let diffs = raw["content"].as_array().into_iter().flatten();
        let changes = diffs
            .filter(|item| item["type"] == "diff")
            .filter_map(|item| {
                let path = item["path"].as_str()?;
                let new = item["newText"].as_str()?;
                Some(match item["oldText"].as_str() {
                    Some(old) => FileChange::compare(path, old, new),
                    None => FileChange::new(path, None, prefixed(new, '+')),
                })
            })
            .collect();
        if let Some(changes) = file_changes::payload(changes) {
            tool_call["file_changes"] = changes;
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

/// How long `agy -p /usage --output-format json` may take before Spark stops it.
const USAGE_TIMEOUT: Duration = Duration::from_secs(15);

/// Whether a registered ACP harness runs the Antigravity CLI (`agy`).
pub fn is_antigravity_harness(harness: &AcpHarness) -> bool {
    Path::new(&harness.executable)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.eq_ignore_ascii_case("agy"))
}

/// The signed-in Antigravity account's plan limits, from `agy -p /usage --output-format json`.
/// The command makes no model call (`num_turns: 0`).
// ponytail: special case for Antigravity until ACP standardizes plan quota windows;
// an unknown response shape reports an error instead of a wrong number.
pub fn read_antigravity_usage(
    harness: &AcpHarness,
    inheritance: EnvironmentInheritancePolicy,
) -> Result<Vec<crate::UsageWindow>, String> {
    let mut command = harness_command(harness, inheritance, None, []);
    if !found(&command) {
        return Err(format!(
            "Could not launch {}: not found",
            harness.executable
        ));
    }
    crate::local_environment::configure_process_group(&mut command);
    command
        .args(["-p", "/usage", "--output-format", "json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = crate::local_environment::spawn_process_group(command)
        .map_err(|error| format!("Could not launch {}: {error}", harness.executable))?;
    let group = child.id();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(child.wait_with_output());
    });
    let output = match receiver.recv_timeout(USAGE_TIMEOUT) {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            return Err(format!("Could not read Antigravity usage: {error}"));
        }
        Err(_) => {
            crate::local_environment::terminate_process_group(group, None);
            return Err(format!(
                "Antigravity did not report its usage within {}s.",
                USAGE_TIMEOUT.as_secs()
            ));
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let trimmed_stdout = stdout.trim();
    if trimmed_stdout.is_empty() {
        let detail = stderr.trim();
        return Err(if detail.is_empty() {
            "Antigravity exited without reporting its usage.".into()
        } else {
            format!("Antigravity could not read usage: {detail}")
        });
    }
    let payload: Value = serde_json::from_str(trimmed_stdout).map_err(|_| {
        let detail = stderr.trim();
        if !output.status.success() && !detail.is_empty() {
            format!("Antigravity could not read usage: {detail}")
        } else {
            "Antigravity reported usage in an unknown format.".into()
        }
    })?;
    parse_antigravity_usage(&payload, stderr.trim())
}

fn parse_antigravity_usage(
    payload: &Value,
    stderr: &str,
) -> Result<Vec<crate::UsageWindow>, String> {
    if payload.get("status").and_then(Value::as_str) != Some("SUCCESS") {
        let message = payload
            .get("error")
            .and_then(|error| {
                error
                    .as_str()
                    .or_else(|| error.get("message").and_then(Value::as_str))
            })
            .or_else(|| payload.get("response").and_then(Value::as_str))
            .map(str::trim)
            .filter(|message| !message.is_empty())
            .or_else(|| (!stderr.is_empty()).then_some(stderr))
            .unwrap_or("unknown error");
        return Err(format!("Antigravity could not read usage: {message}"));
    }
    let invalid = || "Antigravity reported usage in an unknown format.".to_string();
    let groups = payload
        .pointer("/command/data/groups")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let mut windows = Vec::new();
    for group in groups {
        let group_name = group
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .ok_or_else(invalid)?;
        let buckets = group
            .get("buckets")
            .and_then(Value::as_array)
            .ok_or_else(invalid)?;
        for bucket in buckets {
            let span = match bucket.get("window").and_then(Value::as_str).map(str::trim) {
                Some("weekly") => "Weekly".to_string(),
                Some(hours)
                    if hours.ends_with('h') && hours[..hours.len() - 1].parse::<u64>().is_ok() =>
                {
                    format!("{}-hour", &hours[..hours.len() - 1])
                }
                Some(other) if !other.is_empty() => {
                    let other = other.replace('_', " ");
                    let mut chars = other.chars();
                    chars.next().map_or(String::new(), |first| {
                        first.to_uppercase().chain(chars).collect()
                    })
                }
                _ => bucket
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .ok_or_else(invalid)?
                    .to_string(),
            };
            let remaining = bucket
                .get("remaining_fraction")
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite())
                .ok_or_else(invalid)?;
            let resets_at = bucket
                .get("reset_time")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            windows.push(crate::UsageWindow {
                label: format!("{span} · {group_name}"),
                used_percent: ((1.0 - remaining) * 100.0).clamp(0.0, 100.0),
                resets_at,
            });
        }
    }
    Ok(windows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deferred_end_uses_in_progress_idle_until_an_assistant_message_is_open() {
        let now = Instant::now();
        let streaming = DeferredEnd::StreamingWakeup(now);

        // Thoughts, plans, and completed tools keep the long in-progress bound.
        for kind in [UpdateKind::Progress, UpdateKind::Tool] {
            assert_eq!(
                advance_deferred_at(
                    DeferredEnd::StreamingWakeup(now),
                    Some(kind),
                    false,
                    false,
                    now
                ),
                DeferredEnd::StreamingWakeup(now + POST_WAKEUP_IN_PROGRESS_IDLE),
            );
        }

        // Usage updates without an open assistant answer do not shrink to the 2s settle window.
        assert_eq!(
            advance_deferred_at(streaming, Some(UpdateKind::Usage), false, false, now),
            DeferredEnd::StreamingWakeup(now + POST_WAKEUP_IN_PROGRESS_IDLE),
        );
        let awaiting = DeferredEnd::AwaitingWakeup(now + POST_TOOL_WAKEUP_WAIT);
        assert_eq!(
            advance_deferred_at(
                DeferredEnd::AwaitingWakeup(now + POST_TOOL_WAKEUP_WAIT),
                Some(UpdateKind::Usage),
                false,
                false,
                now
            ),
            awaiting,
        );

        // Once an assistant message is streaming, idle uses 15s and usage_update settles in 2s.
        assert_eq!(
            advance_deferred_at(
                DeferredEnd::StreamingWakeup(now),
                Some(UpdateKind::AssistantMessage),
                false,
                true,
                now
            ),
            DeferredEnd::StreamingWakeup(now + POST_WAKEUP_STREAM_IDLE),
        );
        assert_eq!(
            advance_deferred_at(
                DeferredEnd::StreamingWakeup(now),
                Some(UpdateKind::Usage),
                false,
                true,
                now
            ),
            DeferredEnd::StreamingWakeup(now + POST_WAKEUP_USAGE_SETTLE),
        );

        // Any open background tool always takes precedence.
        assert_eq!(
            advance_deferred_at(
                DeferredEnd::StreamingWakeup(now),
                Some(UpdateKind::Usage),
                true,
                true,
                now
            ),
            DeferredEnd::WaitingForTools,
        );
    }

    /// A probe and every live session request name the work area alike: its
    /// conventional canonical path, whatever spelling the caller used, and
    /// options a live session acknowledges reach the chooser asking under
    /// another spelling. A saved session continues in the same directory, and
    /// resets once the checkout path is replaced by a symlink to another one.
    /// The agent is a shell script.
    #[cfg(unix)]
    #[test]
    fn probe_and_live_sessions_send_one_conventional_work_area() {
        const AGENT: &str = r#"
log=$1
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$log"
  id=${line#*\"id\":}
  id=${id%%[,\}]*}
  IFS= read -r capabilities < "$2"
  case $line in
    *'"method":"initialize"'*) result="{\"protocolVersion\":1,\"agentCapabilities\":$capabilities}" ;;
    *'"method":"session/new"'*) result='{"sessionId":"shell-session","configOptions":[{"id":"model","name":"Model","type":"select","currentValue":"small","options":[{"value":"small","name":"Small"}]}]}' ;;
    *'"method":"session/resume"'* | *'"method":"session/load"'*) result='{}' ;;
    *'"method":"session/prompt"'*) result='{"stopReason":"end_turn"}' ;;
    *) continue ;;
  esac
  printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$id" "$result"
done
"#;
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let alias = temp.path().join("alias");
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let cwd = spark_common::paths::canonicalize_clean(&real).unwrap();
        let (agent, log, capabilities) = (
            temp.path().join("agent.sh"),
            temp.path().join("acp.log"),
            temp.path().join("capabilities"),
        );
        std::fs::write(&agent, AGENT).unwrap();
        let harness = json!({
            "name": "Shell agent",
            "executable": "/bin/sh",
            "args": [agent, log, capabilities],
        });
        let request = |saved: Option<&AcpSessionRecord>| AgentTurnRequest {
            conversation_id: "work-area".to_string(),
            project_path: alias.to_string_lossy().into_owned(),
            prompt: "Hi.".to_string(),
            history: Vec::new(),
            provider: Some("acp:shell".to_string()),
            model: None,
            llm_profile: None,
            reasoning_effort: None,
            chat_mode: None,
            metadata: BTreeMap::from([
                (
                    "spark.execution.settings".to_string(),
                    json!({"configuration": {"agents": {"acp_harnesses": {"shell": harness}}}}),
                ),
                // A flow node's turn: nothing parks, so each turn launches anew.
                ("spark.runtime.run_id".to_string(), json!("run")),
            ])
            .into_iter()
            .chain(saved.map(AcpSessionRecord::to_metadata).unwrap_or_default())
            .collect(),
        };
        let run = |saved, agent_capabilities: &str| {
            std::fs::write(&capabilities, agent_capabilities).unwrap();
            run_turn(&request(saved), "shell", None, None)
                .map_err(|failure| failure.message)
                .unwrap()
        };

        std::fs::write(&capabilities, "{}").unwrap();
        let shell: AcpHarness = serde_json::from_value(harness.clone()).unwrap();
        discover("shell", &shell, Default::default(), &alias);
        let fresh = run(None, "{}").record.unwrap();
        assert_eq!(Path::new(&fresh.cwd), cwd);
        assert!(acknowledged_options(
            &alias,
            "shell",
            &launch_generation(&shell, Default::default())
        )
        .is_some());
        let resume = r#"{"sessionCapabilities":{"resume":{}}}"#;
        for agent_capabilities in [resume, r#"{"loadSession":true}"#] {
            let turn = run(Some(&fresh), agent_capabilities);
            assert!(turn.output.thread_resume_failure.is_none());
            assert_eq!(turn.record.as_ref(), Some(&fresh));
        }
        // The checkout becomes a symlink to another one: a fresh session.
        let other = temp.path().join("other");
        std::fs::create_dir(&other).unwrap();
        std::fs::remove_dir(&real).unwrap();
        std::os::unix::fs::symlink(&other, &real).unwrap();
        let other = spark_common::paths::canonicalize_clean(&other).unwrap();
        let moved = run(Some(&fresh), resume);
        assert!(moved.output.thread_resume_failure.is_some());
        assert_eq!(Path::new(&moved.record.unwrap().cwd), other);

        let sent = std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|message| {
                message["method"].as_str().is_some_and(|method| {
                    method.starts_with("session/") && method != "session/prompt"
                })
            })
            .map(|message| (message["method"].clone(), message["params"]["cwd"].clone()))
            .collect::<Vec<_>>();
        let expected = [
            ("session/new", &cwd),
            ("session/new", &cwd),
            ("session/resume", &cwd),
            ("session/load", &cwd),
            ("session/new", &other),
        ]
        .map(|(method, cwd)| (json!(method), json!(cwd)));
        assert_eq!(sent, expected);
    }

    /// Only a Windows `\\?\` prefix may differ between a saved session's
    /// work area and the current one; every other field must match.
    #[test]
    fn a_saved_session_continues_only_where_a_verbatim_prefix_differs() {
        let current = AcpSessionRecord {
            harness: "shell".into(),
            session_id: String::new(),
            cwd: r"C:\proj".into(),
            configuration_generation: "1".into(),
            instructions_sha256: None,
        };
        let saved = |cwd: &str| AcpSessionRecord {
            session_id: "session".into(),
            cwd: cwd.into(),
            ..current.clone()
        };
        assert!(saved(r"C:\proj").continues_as(&current));
        assert!(saved(r"\\?\C:\proj").continues_as(&current));
        assert!(!saved(r"\\?\C:\other").continues_as(&current));
        let kept = AcpSessionRecord {
            cwd: r"\\?\C:\proj.".into(),
            ..current.clone()
        };
        assert!(saved(r"\\?\C:\proj.").continues_as(&kept));
        assert!(!saved(r"C:\proj.").continues_as(&kept));
        let regenerated = AcpSessionRecord {
            configuration_generation: "2".into(),
            ..current.clone()
        };
        assert!(!saved(r"\\?\C:\proj").continues_as(&regenerated));
    }

    /// The program is the one the child's own `PATH` finds: `Path` is bound
    /// after `PATH` in a registration, a channel's binding comes last, and a
    /// relative entry resolves from the work area.
    #[cfg(windows)]
    #[test]
    fn a_windows_launch_names_the_program_on_the_path_its_child_receives() {
        let temp = tempfile::tempdir().unwrap();
        let dirs = ["a", "b", "c"].map(|name| {
            let dir = temp.path().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("agent.cmd"), "").unwrap();
            dir
        });
        let harness: AcpHarness = serde_json::from_value(json!({
            "name": "Agent",
            "executable": "agent",
            "env": {"PATH": dirs[0], "Path": dirs[1]},
        }))
        .unwrap();
        let launch = |bindings: Vec<(String, OsString)>| {
            let command = harness_command(
                &harness,
                EnvironmentInheritancePolicy::InheritNone,
                None,
                bindings,
            );
            let path = command
                .get_envs()
                .filter(|(key, _)| key.eq_ignore_ascii_case("PATH"))
                .filter_map(|(_, value)| value.map(std::ffi::OsStr::to_owned))
                .collect::<Vec<_>>();
            (command.get_program().to_owned(), path)
        };
        let named = |(program, path): (OsString, Vec<OsString>), dir: &Path| {
            assert!(
                program.eq_ignore_ascii_case(dir.join("agent.cmd")),
                "{program:?}"
            );
            assert_eq!(path, [dir.as_os_str()]);
        };

        named(launch(Vec::new()), dirs[1].as_path());
        named(
            launch(vec![("PATH".into(), dirs[2].clone().into())]),
            dirs[2].as_path(),
        );

        let project = temp.path().join("project");
        let bin = project.join("node_modules").join(".bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("agent.cmd"), "").unwrap();
        let local = AcpHarness {
            env: BTreeMap::from([("PATH".to_string(), r"node_modules\.bin".to_string())]),
            ..harness.clone()
        };
        let command = harness_command(
            &local,
            EnvironmentInheritancePolicy::InheritNone,
            Some(&project),
            [],
        );
        assert!(
            command
                .get_program()
                .eq_ignore_ascii_case(bin.join("agent.cmd")),
            "{:?}",
            command.get_program()
        );
    }
}
