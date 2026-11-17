//! Stable ACP v1 client for registered harnesses (`acp:<id>`), driven by the
//! pinned SDK over the shared stdio transport.
//!
//! One process per turn: initialize, create or resume the session, apply
//! required selections, then exactly one primary prompt. The SDK connection
//! runs on the transport's worker; events reach the caller's sink on the
//! calling thread, in order, and are returned exactly as delivered.

use std::collections::{BTreeMap, HashMap};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::util::MatchDispatch;
use agent_client_protocol::v1::schema::{
    CancelNotification, ClientCapabilities, ContentBlock, ContentChunk, ElicitationCapabilities,
    ElicitationFormCapabilities, Implementation, InitializeRequest, LoadSessionRequest,
    NewSessionRequest, PermissionOptionKind, PromptResponse, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, ResumeSessionRequest,
    SelectedPermissionOutcome, SessionConfigKind, SessionConfigOption, SessionConfigOptionCategory,
    SessionConfigOptionValue, SessionConfigSelectOptions, SessionNotification, SessionUpdate,
    SetSessionConfigOptionRequest, StopReason, SystemPrompt,
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
use crate::history::HistoryTurn;
use crate::stdio_transport::{self, sleep_until, Endpoints, Incoming, Outgoing, Stop, LIMITS};

pub const ACP_BACKEND: &str = "acp";
/// The session a turn continues, and the one it leaves for the next turn.
pub const ACP_SESSION_METADATA_KEY: &str = "spark.runtime.acp.session";
/// Emitted once a session is set up; its details are the session record.
pub const ACP_SESSION_ESTABLISHED: &str = "session_established";
/// How long a cancelled prompt may take to settle before its process is stopped.
const CANCEL_GRACE: Duration = Duration::from_secs(5);
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
    /// Replies for the turn's pending forms.
    replies: UnboundedSender<FormAnswer>,
}

/// A reply for one of a turn's pending forms, by its Spark request ID, and
/// where to say whether the harness got it.
struct FormAnswer {
    request_id: String,
    /// A flow's failed wait fails the turn.
    reply: Result<FormReply, String>,
    delivered: Option<std::sync::mpsc::Sender<bool>>,
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

/// What the worker hands the calling thread, in order.
enum Message {
    Event(TurnStreamEvent),
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
/// What a declared instruction channel carries during discovery, which
/// never submits a prompt.
const PROBE_INSTRUCTIONS: &str = "Spark is checking this harness's capabilities. No task follows.";

/// A capability's effective state for Spark's use of a harness: `supported`,
/// `unsupported` or `unverified`, and why.
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
    /// `ready`, `unreachable` or `unauthenticated`.
    pub setup_status: String,
    /// Why setup failed; callers report it as their own status error.
    #[serde(skip)]
    pub error: Option<String>,
    pub capabilities: BTreeMap<String, Capability>,
    pub chat: Capability,
    pub flow: Capability,
    /// The temporary session's config options exactly as the harness sent
    /// them: IDs, value IDs, types, labels, order and groups.
    pub config_options: Vec<Value>,
    /// Legacy session modes, kept only when there are no config options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modes: Option<Value>,
}

/// What initialize and the temporary session reported.
struct Probed {
    system_prompt: bool,
    resume: bool,
    load: bool,
    /// The raw `session/new` result.
    session: Value,
}

struct ProbeFailure {
    unauthenticated: bool,
    message: String,
}

impl From<String> for ProbeFailure {
    fn from(message: String) -> Self {
        Self {
            unauthenticated: false,
            message,
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
    discovery(harness, probe(harness_id, harness, inheritance, cwd))
}

fn probe(
    harness_id: &str,
    harness: &AcpHarness,
    inheritance: EnvironmentInheritancePolicy,
    cwd: &Path,
) -> Result<Probed, ProbeFailure> {
    let cwd = std::fs::canonicalize(cwd)
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
    // A harness may echo what it was launched with, in an error or on stderr.
    let secrets = environment_values(&command);
    let (done, result) = std::sync::mpsc::channel();
    let connection = stdio_transport::launch(command, LIMITS, move |endpoints| {
        probe_protocol(endpoints, cwd, done)
    })
    .map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => {
            format!("executable not found: {}", harness.executable)
        }
        _ => format!("launch failed: {error}"),
    })?;
    let outcome = result.recv_timeout(PROBE_TIMEOUT);
    let stderr = connection.stderr();
    // Stops the process and waits for its worker.
    drop(connection);
    drop(instruction_file);
    let mut failure = match outcome {
        Ok(Ok(probed)) => return Ok(probed),
        Ok(Err(failure)) => failure,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => ProbeFailure::from(format!(
            "the harness did not finish setup within {}s",
            PROBE_TIMEOUT.as_secs()
        )),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            ProbeFailure::from("the ACP connection worker stopped".to_string())
        }
    };
    let lines = stderr.lines().collect::<Vec<_>>();
    let tail = lines[lines.len().saturating_sub(10)..].join("\n");
    if !tail.is_empty() && !failure.unauthenticated {
        failure.message = format!("{}; stderr tail:\n{tail}", failure.message);
    }
    failure.message = secrets
        .iter()
        .fold(failure.message, |message, value| redact(message, value));
    Err(failure)
}

/// `message` with every occurrence of `value` replaced. A value under 4
/// bytes is replaced wherever it stands as its own word, so `PIN=abc` loses
/// its value while "abcde" and error codes keep their text.
fn redact(message: String, value: &str) -> String {
    let joined = |outer: Option<char>, inner: Option<char>| {
        outer
            .zip(inner)
            .is_some_and(|(outer, inner)| outer.is_alphanumeric() && inner.is_alphanumeric())
    };
    let (mut redacted, mut last) = (String::with_capacity(message.len()), 0);
    for (at, _) in message.match_indices(value) {
        let end = at + value.len();
        if value.len() < 4
            && (joined(message[..at].chars().next_back(), value.chars().next())
                || joined(message[end..].chars().next(), value.chars().next_back()))
        {
            continue;
        }
        redacted.push_str(&message[last..at]);
        redacted.push_str("[redacted]");
        last = end;
    }
    redacted.push_str(&message[last..]);
    redacted
}

/// The environment values `command` sets, longest first, so one that
/// contains another is hidden whole.
fn environment_values(command: &Command) -> Vec<String> {
    let mut values = command
        .get_envs()
        .filter_map(|(_, value)| value?.to_str())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    values.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    values.dedup();
    values
}

async fn probe_protocol(
    endpoints: Endpoints,
    cwd: PathBuf,
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
        .connect_with(transport, async |cx| Ok(probe_session(&cx, &cwd).await))
        .await
        .unwrap_or_else(|error| Err(describe(&error).into()))
        .map_err(|error| match failure.get() {
            Some(unusable) => unusable.clone().into(),
            None => error,
        });
    let _ = done.send(result);
}

async fn probe_session(cx: &ConnectionTo<Agent>, cwd: &Path) -> Result<Probed, ProbeFailure> {
    let failed = |error: agent_client_protocol::Error, method: &str| ProbeFailure {
        unauthenticated: is_auth_error(&error),
        message: setup_error(&error, method),
    };
    let init = cx
        .send_request(
            InitializeRequest::new(ProtocolVersion::V1)
                .client_info(Implementation::new("spark", env!("CARGO_PKG_VERSION"))),
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
        system_prompt: capabilities.session_capabilities.system_prompt.is_some(),
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
        std::fs::canonicalize(cwd).ok()?,
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
fn discovery(harness: &AcpHarness, probed: Result<Probed, ProbeFailure>) -> AcpDiscovery {
    let unsupported = |reason: &str| Capability::new("unsupported", reason);
    // Fixed by this implementation, whatever a harness reports.
    let mut capabilities = BTreeMap::from([
        ("plan_mode".to_string(), unsupported("No read-only mode is verified for ACP harnesses; a harness mode named plan carries no such guarantee.")),
        ("steering".to_string(), unsupported("ACP v1 has no portable way to accept a message during a prompt, so messages wait for the next turn.")),
        ("child_intervention".to_string(), unsupported("Spark cannot intervene in an ACP harness's child work.")),
        ("forms".to_string(), Capability::new("supported", "Spark advertises form elicitation and answers it in chats and flows; a schema it cannot show is refused.")),
        ("per_prompt_usage".to_string(), Capability::new("unverified", "Counted only when the harness reports usage on a prompt's response; otherwise unknown.")),
    ]);
    let probed = match probed {
        Ok(probed) => probed,
        Err(failure) => {
            for name in [
                "pinned_instructions",
                "session_resume",
                "model_selection",
                "reasoning_effort",
            ] {
                capabilities.insert(
                    name.to_string(),
                    Capability::new("unverified", "Discovery could not set up the harness."),
                );
            }
            let unknown = Capability::new(
                "unverified",
                "Discovery could not set up the harness; see its setup error.",
            );
            return AcpDiscovery {
                setup_status: if failure.unauthenticated {
                    "unauthenticated"
                } else {
                    "unreachable"
                }
                .to_string(),
                error: Some(failure.message),
                capabilities,
                chat: unknown.clone(),
                flow: unknown,
                config_options: Vec::new(),
                modes: None,
            };
        }
    };
    let options = probed.session["configOptions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let offered = |id: &str, value: &str| {
        options.iter().any(|option| {
            option["id"] == id
                && option["type"] == "select"
                && select_values(option)
                    .iter()
                    .any(|entry| entry["value"] == value)
        })
    };
    let pinned = match &harness.system_instructions {
        Some(channel) => match channel.config.iter().find(|(id, value)| !offered(id, value)) {
            Some((id, value)) => unsupported(&format!(
                "Its system_instructions channel requires config option {id:?} set to {value:?}, which the harness does not offer."
            )),
            None => Capability::new("supported", "Its registration declares a system_instructions channel."),
        },
        None if probed.system_prompt => {
            Capability::new("supported", "The harness advertises systemPrompt.")
        }
        None => unsupported("The harness advertises no systemPrompt and its registration declares no system_instructions channel."),
    };
    let chat = match pinned.supported() {
        true => Capability::new(
            "supported",
            "Chat pins its instructions through a supported channel.",
        ),
        false => unsupported(&format!("Chat pins instructions. {}", pinned.reason)),
    };
    capabilities.insert("pinned_instructions".to_string(), pinned);
    capabilities.insert(
        "session_resume".to_string(),
        if probed.resume {
            Capability::new("supported", "The harness advertises session/resume.")
        } else if probed.load {
            Capability::new("supported", "The harness advertises session/load.")
        } else {
            unsupported("Each turn starts a fresh session, with earlier messages as context.")
        },
    );
    capabilities.insert(
        "model_selection".to_string(),
        match select_option(&options, "model") {
            Some(_) => Capability::new("supported", "The harness offers a model selector."),
            None => unsupported("The harness offers no model selector; turns use its default."),
        },
    );
    capabilities.insert(
        "reasoning_effort".to_string(),
        match select_option(&options, "thought_level") {
            Some(_) => Capability::new("supported", "The harness offers a thought-level selector."),
            None => unsupported("The harness offers no thought-level selector."),
        },
    );
    AcpDiscovery {
        setup_status: "ready".to_string(),
        error: None,
        capabilities,
        chat,
        flow: Capability::new(
            "supported",
            "Flow steps pin no instructions; a step that requires steering or child intervention is refused.",
        ),
        modes: options
            .is_empty()
            .then(|| probed.session.get("modes").filter(|modes| !modes.is_null()).cloned())
            .flatten(),
        config_options: options,
    }
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
    let (command, instruction_file) = launch_command(
        harness_id,
        &harness,
        config.environment_inheritance,
        &cwd,
        instructions.as_deref(),
        &request.metadata,
    )?;

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
    let live = Arc::new(Live {
        interrupts,
        stop: Mutex::new(None),
        terminated: AtomicBool::new(false),
        closed: AtomicBool::new(false),
        replies: replies.clone(),
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
    let out = Out { messages };
    let connection = stdio_transport::launch(command, LIMITS, move |endpoints| {
        drive(endpoints, plan, out, interrupted, (replies, answered))
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
    let mut command = Command::new(&harness.executable);
    command
        .args(&harness.args)
        .args(channel_args)
        .current_dir(cwd)
        .env_clear()
        .envs(crate::local_environment::inherited_environment(inheritance))
        .envs(&harness.env)
        .envs(channel_env)
        .envs(crate::agent::agent_turn_env(metadata));
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

async fn drive(
    endpoints: Endpoints,
    plan: Plan,
    out: Out,
    interrupts: Receiver<()>,
    (replies, answered): (UnboundedSender<FormAnswer>, UnboundedReceiver<FormAnswer>),
) {
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
            Ok(Turn::new(&plan, &out, stop, replies)
                .run(cx, interrupts, answered)
                .await)
        })
        .await
        .unwrap_or_else(|error| Err(describe(&error)));
    // Unusable output explains a failed turn better than the closed connection.
    let result = result.map_err(|error| failure.get().cloned().unwrap_or(error));
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
    /// This launch's connection, which every form it asks is tied to.
    connection: String,
    /// Forms the harness waits on, by Spark request ID.
    forms: HashMap<String, PendingForm>,
    /// Where a flow's answered forms come back.
    replies: UnboundedSender<FormAnswer>,
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
impl Drop for Turn<'_> {
    fn drop(&mut self) {
        self.expire_forms();
    }
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
    fn new(plan: &'a Plan, out: &'a Out, stop: Stop, replies: UnboundedSender<FormAnswer>) -> Self {
        Self {
            plan,
            out,
            stop,
            translate: Translate::new(&plan.harness_id),
            connection: uuid::Uuid::new_v4().to_string(),
            forms: HashMap::new(),
            replies,
        }
    }

    async fn run(
        mut self,
        cx: ConnectionTo<Agent>,
        mut interrupts: Receiver<()>,
        answered: UnboundedReceiver<FormAnswer>,
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
        self.prompt(&mut session, interrupts, answered, record, reset)
            .await
    }

    /// Initializes, then resumes, loads or creates the session and applies
    /// the required selections.
    async fn setup(&mut self, cx: &ConnectionTo<Agent>) -> Result<Ready, String> {
        let plan = self.plan;
        let id = &plan.harness_id;
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

        self.plan.acknowledge(&options);
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
    /// requests, forms and interrupts meanwhile.
    async fn prompt(
        mut self,
        session: &mut Session,
        mut interrupts: Receiver<()>,
        mut answered: UnboundedReceiver<FormAnswer>,
        record: AcpSessionRecord,
        reset: Option<String>,
    ) -> Result<Finished, String> {
        let connection = session.connection().clone();
        let session_id = session.session_id().clone();
        let mut cancelled = false;
        let mut cancel_deadline = None;
        let mut forced = false;
        // Forms the harness withdrew with `$/cancel_request`.
        let mut withdrawn = FuturesUnordered::<BoxFuture<'static, String>>::new();
        loop {
            tokio::select! {
                update = session.read_update() => match update {
                    Ok(SessionMessage::Message(dispatch)) => {
                        let dispatch = match dispatch.into_raw() {
                            Dispatch::Request(request, responder)
                                if request.method() == "elicitation/create" =>
                            {
                                if let Some((request_id, cancellation)) =
                                    self.elicit(request.params(), responder)
                                {
                                    withdrawn.push(Box::pin(async move {
                                        cancellation.cancelled().await;
                                        request_id
                                    }));
                                }
                                continue;
                            }
                            dispatch => dispatch,
                        };
                        let (translate, out, plan) = (&mut self.translate, self.out, &self.plan);
                        let policy = plan.policy;
                        MatchDispatch::new(dispatch)
                            .if_notification(async |notification: SessionNotification| {
                                if let SessionUpdate::ConfigOptionUpdate(update) = &notification.update {
                                    plan.acknowledge(&update.config_options);
                                }
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
                Some(answer) = answered.recv() => self.deliver(answer)?,
                Some(request_id) = withdrawn.next(), if !withdrawn.is_empty() => {
                    // Dropping the responder sends the cancelled request's error.
                    if let Some(PendingForm { request, gone, .. }) = self.forms.remove(&request_id) {
                        self.expired(request, gone);
                    }
                }
                Some(()) = interrupts.recv(), if !cancelled => {
                    cancelled = true;
                    self.expire_forms();
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
