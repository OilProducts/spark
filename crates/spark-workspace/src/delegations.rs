//! Whole-task handoff between Spark environments.
//!
//! The sending environment (A) keeps links to other environments and a
//! record of each task it hands off, written before anything is sent. Its
//! follower submits each record, follows it, and when the receiving
//! environment (B) seals it, delivers the outcome to the owner conversation's
//! inbox once. B admits each delegation ID at most once, launches it through
//! the ordinary flow or turn path, and reports its state, questions and seal.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
use spark_storage::conversation::NewInboxEvent;
pub use spark_storage::environment::DelegationListFilter;
use spark_storage::environment::{
    now, seal_out_on, DelegationIn, DelegationOut, EnvironmentLink, EnvironmentRepository,
    PROTOCOL_VERSION,
};
use spark_storage::{ConversationRepository, ConversationStore};

use crate::conversations::{
    ConversationRequestUserInputAnswerRequest, ConversationTurnRequest, ProposedPlanReviewRequest,
    RunLaunchRequest, WorkspaceConversationService,
};
use crate::inbox::{self, Inbox, InboxPost, TERMINAL_RUN_STATUSES};
use crate::{WorkspaceError, WorkspaceResult};

/// How often A follows its open delegations.
pub const FOLLOW_INTERVAL: Duration = Duration::from_secs(5);
/// The longest a failing record waits between attempts.
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// The longest one `wait` request holds.
pub const MAX_WAIT: Duration = Duration::from_secs(30);
/// The longest one request to another environment takes, answer included:
/// under the `spark` CLI's 30 s request limit, so a stalled peer's error,
/// naming it, reaches the CLI before the CLI gives up on this server.
pub const PEER_TIMEOUT: Duration = Duration::from_secs(20);
/// Provenance of a run B launched for a delegation.
pub const DELEGATION_CONTEXT_KEY: &str = "context.spark_delegation";
/// The most a linked environment's answer may hold; a larger one is refused
/// as it streams in, before it is decoded.
// ponytail: one cap for every answer; a result that outgrows it retries until
// it is raised, so raise it or page results if real ones get near it.
pub const MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;
/// The most question notifications one delegation may have outstanding in
/// its owner's inbox: not yet taken by the owner, or still open on B.
pub const MAX_OUTSTANDING_QUESTIONS: usize = 20;
const INCOMING_PATH: &str = "/workspace/api/delegations/incoming";

fn validation(message: impl Into<String>) -> WorkspaceError {
    WorkspaceError::Validation(message.into())
}

fn internal(error: impl std::fmt::Display) -> WorkspaceError {
    WorkspaceError::Internal(error.to_string())
}

fn environments(settings: &SparkSettings) -> EnvironmentRepository {
    EnvironmentRepository::new(&settings.data_dir)
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

// ---- Tasks ------------------------------------------------------------------

/// A flow task: the launch fields of B's `/runs/launch`, without model,
/// provider or profile, which come from B's project defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FlowTask {
    flow_name: String,
    summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    goal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    launch_context: Option<Value>,
}

/// A conversation task: the first message of a new chat on B.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConversationTask {
    message: String,
}

fn decode<T: serde::de::DeserializeOwned>(value: &Value) -> WorkspaceResult<T> {
    serde_json::from_value(value.clone())
        .map_err(|error| validation(format!("Invalid task: {error}")))
}

/// The digest both sides compute over what a delegation asks for.
pub fn request_digest(project_path: &str, kind: &str, request: &Value) -> String {
    let mut content = json!({"project_path": project_path, "kind": kind, "request": request});
    // Sorted, so the digest doesn't depend on map order (serde_json keeps insertion order).
    content.sort_all_objects();
    spark_common::project::sha256_hex(content.to_string().as_bytes())
}

// ---- Links ------------------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkAddRequest {
    pub origin: String,
    pub expected_id: String,
    pub code: String,
    #[serde(default)]
    pub label: String,
}

/// An origin A may call: `https://host[:port]`, or `http://` to a literal
/// loopback address (an SSH forward), with nothing after the authority.
pub fn normalize_origin(origin: &str) -> WorkspaceResult<String> {
    let url = reqwest::Url::parse(origin.trim())
        .map_err(|error| validation(format!("Invalid origin {origin:?}: {error}.")))?;
    let loopback = url
        .host_str()
        .map(|host| host.trim_start_matches('[').trim_end_matches(']'))
        .and_then(|host| host.parse::<IpAddr>().ok())
        .is_some_and(|ip| ip.is_loopback());
    match url.scheme() {
        "https" => {}
        "http" if loopback => {}
        "http" => {
            return Err(validation(
                "An http origin must be a literal loopback address, such as an SSH forward; use https otherwise.",
            ))
        }
        scheme => return Err(validation(format!("Unsupported origin scheme {scheme:?}."))),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(validation("An origin carries no user name or password."));
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err(validation(
            "An origin is a scheme, host and port only, with no path, query or fragment.",
        ));
    }
    Ok(url.origin().ascii_serialization())
}

/// Checks `origin` is the expected environment, exchanges the pairing code
/// for a token, and saves the link.
pub fn add_link(
    settings: &SparkSettings,
    store: &ConversationStore,
    request: LinkAddRequest,
) -> WorkspaceResult<EnvironmentLink> {
    let origin = normalize_origin(&request.origin)?;
    let expected = request.expected_id.trim();
    let code = request.code.trim();
    if expected.is_empty() || code.is_empty() {
        return Err(validation(
            "A link needs the expected environment ID and a pairing code.",
        ));
    }
    if expected == store.environment_id() {
        return Err(validation(
            "That is this environment's own ID; an environment cannot link to itself.",
        ));
    }
    let unreachable = |error: PeerError| match error {
        PeerError::Status(403, detail) => WorkspaceError::Forbidden(detail),
        error => WorkspaceError::ServiceUnavailable(format!("{origin}: {error}")),
    };
    let descriptor = call(Method::GET, &origin, "/environment", None, None).map_err(unreachable)?;
    let found = descriptor["environment_id"].as_str().unwrap_or_default();
    if found != expected {
        return Err(validation(format!(
            "{origin} is environment {found:?}, not {expected:?}."
        )));
    }
    if descriptor["protocol_version"] != json!(PROTOCOL_VERSION) {
        return Err(validation(format!(
            "{origin} speaks protocol {}, this environment {PROTOCOL_VERSION}; upgrade both together.",
            descriptor["protocol_version"]
        )));
    }
    let pair =
        json!({"code": code, "label": format!("Spark environment {}", store.environment_id())});
    let paired = call(
        Method::POST,
        &origin,
        "/environment/pair",
        None,
        Some(&pair),
    )
    .map_err(unreachable)?;
    let token = paired["token"].as_str().unwrap_or_default();
    if paired["environment_id"].as_str() != Some(expected) || token.is_empty() {
        return Err(internal(format!(
            "{origin} answered pairing without its token."
        )));
    }
    let link = EnvironmentLink {
        environment_id: expected.to_string(),
        origin,
        label: non_empty(Some(&request.label))
            .or_else(|| non_empty(descriptor["display_name"].as_str()))
            .unwrap_or_else(|| expected.to_string()),
        created_at: now(),
    };
    environments(settings).save_link(&link, token)?;
    Ok(link)
}

pub fn links(settings: &SparkSettings) -> WorkspaceResult<Vec<EnvironmentLink>> {
    Ok(environments(settings).links()?)
}

pub fn remove_link(settings: &SparkSettings, environment_id: &str) -> WorkspaceResult<()> {
    if environments(settings).remove_link(environment_id)? {
        Ok(())
    } else {
        Err(WorkspaceError::NotFound(format!(
            "No link to environment {environment_id}."
        )))
    }
}

/// Issues a read-only GET to a linked environment using its stored bearer token.
fn linked_get(
    settings: &SparkSettings,
    environment_id: &str,
    path: &str,
) -> WorkspaceResult<Value> {
    let (link, token) = environments(settings)
        .link(environment_id)?
        .ok_or_else(|| {
            WorkspaceError::NotFound(format!(
                "No link to environment {environment_id:?}; add one with spark environment link add."
            ))
        })?;
    check_route(&link.origin, path)?;
    call(Method::GET, &link.origin, path, Some(&token), None).map_err(|error| match error {
        PeerError::Status(404, detail) => WorkspaceError::NotFound(detail),
        PeerError::Status(409, detail) => WorkspaceError::Conflict(detail),
        PeerError::Status(400 | 422, detail) => WorkspaceError::Validation(detail),
        PeerError::Status(401 | 403, detail) => WorkspaceError::Forbidden(format!(
            "The environment refused this link's token ({detail}); link it again to refresh it."
        )),
        error => WorkspaceError::ServiceUnavailable(format!("{}: {error}", link.origin)),
    })
}

fn project_query(project_path: Option<&str>) -> String {
    match non_empty(project_path) {
        Some(path) => format!("?project_path={}", encode(&path)),
        None => String::new(),
    }
}

/// Lists the projects registered on a linked environment.
pub fn link_projects(settings: &SparkSettings, environment_id: &str) -> WorkspaceResult<Value> {
    linked_get(settings, environment_id, "/workspace/api/projects")
}

/// Lists the flows available on a linked environment, optionally resolved for
/// one of its registered projects.
pub fn link_flows(
    settings: &SparkSettings,
    environment_id: &str,
    project_path: Option<&str>,
) -> WorkspaceResult<Value> {
    let path = format!("/workspace/api/flows{}", project_query(project_path));
    linked_get(settings, environment_id, &path)
}

/// Reads a flow's files (including `flow.md` and declared inputs) from a
/// linked environment.
pub fn link_flow_files(
    settings: &SparkSettings,
    environment_id: &str,
    flow_name: &str,
    project_path: Option<&str>,
) -> WorkspaceResult<Value> {
    let flow_name = flow_name.trim().trim_matches('/');
    if flow_name.is_empty() {
        return Err(validation("flow_name is required."));
    }
    let path = format!(
        "/workspace/api/flows/{}/files{}",
        route_id("flow_name", flow_name)?,
        project_query(project_path)
    );
    linked_get(settings, environment_id, &path)
}

// ---- Calls to another environment --------------------------------------------

#[derive(Debug)]
enum PeerError {
    /// No usable answer: the request may or may not have taken effect.
    Network(String),
    Status(u16, String),
}

impl std::fmt::Display for PeerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(error) => write!(f, "{error}"),
            Self::Status(status, detail) => write!(f, "answered {status}: {detail}"),
        }
    }
}

/// One JSON request to another environment. Redirects are not followed, ambient
/// proxies are bypassed, and the entire round-trip (headers plus body) shares
/// one wall-clock deadline.
fn call(
    method: Method,
    origin: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&Value>,
) -> Result<Value, PeerError> {
    call_with_timeout(method, origin, path, token, body, PEER_TIMEOUT)
}

fn call_with_timeout(
    method: Method,
    origin: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&Value>,
    timeout: Duration,
) -> Result<Value, PeerError> {
    static CLIENT: OnceLock<Result<reqwest::blocking::Client, String>> = OnceLock::new();
    let client = CLIENT
        .get_or_init(|| {
            reqwest::blocking::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| PeerError::Network(error.clone()))?
        .clone();
    let url = format!("{origin}{path}");
    let mut request = client.request(method, url).timeout(timeout);
    if let Some(token) = token {
        request = request
            .bearer_auth(token)
            // Marks the call as relayed so a loopback peer always checks the token.
            .header(reqwest::header::FORWARDED, "by=spark-peer");
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    let deadline = Instant::now() + timeout;
    let origin_owned = origin.to_string();
    let path_owned = path.to_string();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let outcome = (|| -> Result<(u16, Vec<u8>), PeerError> {
            let mut response = request.send().map_err(|error| {
                PeerError::Network(if error.is_timeout() {
                    format!("{path_owned} did not answer within {timeout:?}.")
                } else {
                    error.to_string()
                })
            })?;
            let status = response.status().as_u16();
            let too_large = || {
                PeerError::Network(format!(
                    "{origin_owned}{path_owned} answered with more than {MAX_RESPONSE_BYTES} bytes; the answer was refused."
                ))
            };
            if response
                .content_length()
                .is_some_and(|length| length > MAX_RESPONSE_BYTES)
            {
                return Err(too_large());
            }
            let mut bytes = Vec::new();
            let mut chunk = [0u8; 8192];
            loop {
                if Instant::now() >= deadline {
                    return Err(PeerError::Network(format!(
                        "{path_owned} did not answer within {timeout:?}."
                    )));
                }
                let read = response.read(&mut chunk).map_err(|error| {
                    if error.kind() == std::io::ErrorKind::TimedOut || Instant::now() >= deadline {
                        PeerError::Network(format!(
                            "{path_owned} did not answer within {timeout:?}."
                        ))
                    } else {
                        PeerError::Network(error.to_string())
                    }
                })?;
                if read == 0 {
                    break;
                }
                if (bytes.len() as u64).saturating_add(read as u64) > MAX_RESPONSE_BYTES {
                    return Err(too_large());
                }
                bytes.extend_from_slice(&chunk[..read]);
            }
            Ok((status, bytes))
        })();
        let _ = tx.send(outcome);
    });
    let remaining = deadline.saturating_duration_since(Instant::now());
    let (status, bytes) = match rx.recv_timeout(remaining) {
        Ok(result) => result?,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            return Err(PeerError::Network(format!(
                "{path} did not answer within {timeout:?}."
            )));
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            return Err(PeerError::Network(format!(
                "{path} request worker stopped unexpectedly."
            )));
        }
    };
    let text = String::from_utf8_lossy(&bytes);
    let value = serde_json::from_str::<Value>(&text);
    if (200..300).contains(&status) {
        // Every Spark route answers JSON: any other success is no answer.
        return value.map_err(|_| {
            PeerError::Network(format!(
                "{path} answered {status} with a body that is not JSON: {:?}",
                text.chars().take(200).collect::<String>()
            ))
        });
    }
    let detail = value
        .ok()
        .and_then(|value| value["detail"].as_str().map(str::to_string))
        .unwrap_or_else(|| text.chars().take(500).collect());
    Err(PeerError::Status(status, detail))
}

// ---- Outgoing delegations (A) -------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationStartRequest {
    #[serde(default)]
    pub environment_id: String,
    #[serde(default)]
    pub project_path: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub request_key: String,
    /// The calling run, if a run delegates.
    #[serde(default)]
    pub run_id: Option<String>,
    /// The calling conversation, if a conversation delegates.
    #[serde(default)]
    pub conversation_handle: Option<String>,
    #[serde(default)]
    pub flow_name: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default)]
    pub launch_context: Option<Value>,
    #[serde(default)]
    pub message: Option<String>,
}

fn task(request: &DelegationStartRequest) -> WorkspaceResult<Value> {
    let task = match request.kind.trim() {
        "flow" => {
            if request.message.is_some() {
                return Err(validation("A flow task takes no message."));
            }
            serde_json::to_value(FlowTask {
                flow_name: non_empty(request.flow_name.as_deref())
                    .ok_or_else(|| validation("A flow task requires flow_name."))?,
                summary: non_empty(request.summary.as_deref())
                    .ok_or_else(|| validation("A flow task requires summary."))?,
                goal: non_empty(request.goal.as_deref()),
                launch_context: request.launch_context.clone(),
            })
        }
        "conversation" => {
            if request.flow_name.is_some()
                || request.summary.is_some()
                || request.goal.is_some()
                || request.launch_context.is_some()
            {
                return Err(validation("A conversation task takes only a message."));
            }
            serde_json::to_value(ConversationTask {
                message: non_empty(request.message.as_deref())
                    .ok_or_else(|| validation("A conversation task requires message."))?,
            })
        }
        kind => {
            return Err(validation(format!(
                "kind must be flow or conversation, not {kind:?}."
            )))
        }
    };
    task.map_err(internal)
}

fn run_record(
    settings: &SparkSettings,
    run_id: &str,
) -> WorkspaceResult<Option<attractor_core::RunRecord>> {
    let runs = attractor_runtime::RunStore::for_settings(settings);
    let Some(paths) = runs.find_run_root(run_id).map_err(internal)? else {
        return Ok(None);
    };
    runs.read_run_record(&paths).map_err(internal)
}

/// The caller's request-key scope and the conversation its outcome reports
/// to. Callers identify themselves, as a conversation's run start does;
/// the checks prevent accidents, not a caller that lies.
fn caller(
    settings: &SparkSettings,
    store: &Arc<ConversationStore>,
    request: &DelegationStartRequest,
) -> WorkspaceResult<(String, Option<String>)> {
    match (
        non_empty(request.run_id.as_deref()),
        non_empty(request.conversation_handle.as_deref()),
    ) {
        (Some(_), Some(_)) => Err(validation("Name one caller: a run or a conversation.")),
        (Some(run_id), None) => {
            let record = run_record(settings, &run_id)?
                .ok_or_else(|| WorkspaceError::NotFound(format!("Unknown run: {run_id}")))?;
            let status = attractor_runtime::normalize_run_status(record.status.trim());
            if TERMINAL_RUN_STATUSES.contains(&status.as_str()) {
                return Err(WorkspaceError::Conflict(format!(
                    "Run {run_id} has ended ({status}); it cannot delegate."
                )));
            }
            if record.execution_mode != "native" {
                return Err(WorkspaceError::Forbidden(format!(
                    "Run {run_id} is not a native run; only native runs can delegate."
                )));
            }
            let owner = inbox::run_owner(settings, &run_id)?.map(|owner| owner.conversation_id);
            Ok((format!("run:{run_id}"), owner))
        }
        (None, Some(handle)) => {
            let repository = ConversationRepository::new(store.clone());
            let unknown =
                || WorkspaceError::NotFound(format!("Unknown conversation handle: {handle}"));
            let found = repository
                .find_conversation_by_handle(&handle)?
                .ok_or_else(unknown)?;
            let head = repository
                .read_head(&found.conversation_id, None)?
                .ok_or_else(unknown)?;
            if head["chat_mode"] == "plan" {
                return Err(WorkspaceError::Forbidden(
                    "A conversation in plan mode cannot delegate work.".to_string(),
                ));
            }
            Ok((
                format!("conversation:{}", found.conversation_id),
                Some(found.conversation_id),
            ))
        }
        (None, None) => Ok(("owner".to_string(), None)),
    }
}

/// Records a delegation to a linked environment before anything is sent; the
/// follower submits it. The same caller and request key return the earlier
/// record when the request matches, and conflict otherwise, even once its
/// link is removed; only a new record needs a link.
pub fn start(
    settings: &SparkSettings,
    store: &Arc<ConversationStore>,
    request: DelegationStartRequest,
) -> WorkspaceResult<DelegationOut> {
    let request_key = request.request_key.trim().to_string();
    if request_key.is_empty() {
        return Err(validation(
            "request_key is required: name the task, and reuse the name when retrying.",
        ));
    }
    let environment_id = request.environment_id.trim().to_string();
    let project_path = request.project_path.trim().to_string();
    if project_path.is_empty() {
        return Err(validation(
            "project_path is required: a project registered on the other environment.",
        ));
    }
    let task = task(&request)?;
    let kind = request.kind.trim().to_string();
    let repository = environments(settings);
    let (caller, owner_conversation_id) = caller(settings, store, &request)?;
    let request_sha256 = request_digest(&project_path, &kind, &task);
    let stored = if let Some(earlier) = repository.out_by_key(&caller, &request_key)? {
        earlier
    } else if repository.link(&environment_id)?.is_none() {
        return Err(WorkspaceError::NotFound(format!(
            "No link to environment {environment_id:?}; add one with spark environment link add."
        )));
    } else {
        repository.create_out(&DelegationOut {
            delegation_id: uuid::Uuid::new_v4().to_string(),
            environment_id: environment_id.clone(),
            project_path,
            kind,
            request: task,
            request_sha256: request_sha256.clone(),
            caller,
            request_key: request_key.clone(),
            owner_conversation_id,
            phase: "created".to_string(),
            target_id: None,
            last_state: None,
            result: None,
            paused: None,
            cancel_requested_at: None,
            cancel_acknowledged_at: None,
            created_at: String::new(),
            updated_at: String::new(),
            submitted_at: None,
        })?
    };
    if stored.request_sha256 != request_sha256 || stored.environment_id != environment_id {
        return Err(WorkspaceError::Conflict(format!(
            "Request key {request_key:?} already names a different task, delegation {}.",
            stored.delegation_id
        )));
    }
    Ok(stored)
}

pub fn list(
    settings: &SparkSettings,
    filter: DelegationListFilter,
) -> WorkspaceResult<Vec<DelegationOut>> {
    let filter = DelegationListFilter {
        environment_id: non_empty(filter.environment_id.as_deref()),
        phase: non_empty(filter.phase.as_deref()),
        ..filter
    };
    if let Some(phase) = filter.phase.as_deref() {
        if !matches!(phase, "created" | "accepted" | "sealed") {
            return Err(validation(format!(
                "phase must be created, accepted or sealed, not {phase:?}."
            )));
        }
    }
    Ok(environments(settings).list_out(&filter)?)
}

pub fn get(settings: &SparkSettings, delegation_id: &str) -> WorkspaceResult<DelegationOut> {
    environments(settings)
        .out(delegation_id)?
        .ok_or_else(|| WorkspaceError::NotFound(format!("Unknown delegation: {delegation_id}")))
}

/// Records the owner's cancellation; the follower sends it to B until B
/// acknowledges, and keeps following until B seals the task. A paused
/// delegation (unlinked, revoked or refused by B) that was never submitted
/// is sealed locally as `canceled` right away, since B never saw it; one B
/// may have keeps its cancel pending, sent once a link resumes it.
pub fn cancel(
    settings: &SparkSettings,
    store: &Arc<ConversationStore>,
    delegation_id: &str,
) -> WorkspaceResult<DelegationOut> {
    environments(settings).request_cancel_out(delegation_id)?;
    // Read after the cancel is recorded, so a pause that committed first is
    // seen. Storage checks the pause and submission again as it seals, so a
    // relink that commits before the seal leaves the record open.
    let record = get(settings, delegation_id)?;
    if record.phase == "sealed" {
        return Ok(record);
    }
    if let Some(reason) = record.paused.as_deref() {
        seal_canceled_unsubmitted(settings, store, &record, reason)?;
    }
    get(settings, delegation_id)
}

fn seal_canceled_unsubmitted(
    settings: &SparkSettings,
    store: &Arc<ConversationStore>,
    record: &DelegationOut,
    reason: &str,
) -> WorkspaceResult<bool> {
    let target_kind = if record.kind == "flow" {
        "run"
    } else {
        "conversation"
    };
    let state = json!({
        "target_kind": target_kind,
        "target_id": record.target_id,
    });
    let result = json!({
        "status": "canceled",
        "detail": format!("Canceled locally while paused: {reason}"),
    });
    seal_out_and_notify(settings, store, record, &state, result, true)
}

fn pause_or_seal_canceled(
    settings: &SparkSettings,
    store: &Arc<ConversationStore>,
    delegation_id: &str,
    reason: &str,
) -> Result<(), String> {
    let repository = environments(settings);
    let _ = repository.pause_out(delegation_id, reason);
    let Ok(Some(current)) = repository.out(delegation_id) else {
        return Ok(());
    };
    if current.phase != "sealed"
        && current.paused.is_some()
        && current.cancel_requested_at.is_some()
        && current.submitted_at.is_none()
    {
        seal_canceled_unsubmitted(settings, store, &current, reason).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A run question takes `selected_value`; a conversation request takes
/// `answers` by question ID, or `selected_value` when it asks one question.
/// A question that carries a `form` takes only `form`: one typed reply.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationAnswerRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub selected_value: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub answers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form: Option<spark_common::forms::FormReply>,
}

/// Forwards an answer to B once. A lost response is not replayed: the caller
/// reads the delegation to see whether the question closed.
pub fn answer(
    settings: &SparkSettings,
    delegation_id: &str,
    question_id: &str,
    request: DelegationAnswerRequest,
) -> WorkspaceResult<Value> {
    let path = format!(
        "{INCOMING_PATH}/{}/questions/{}/answer",
        route_id("delegation_id", delegation_id)?,
        route_id("question_id", question_id)?
    );
    let record = get(settings, delegation_id)?;
    if record.phase != "accepted" {
        return Err(WorkspaceError::Conflict(format!(
            "Delegation {delegation_id} has no open questions ({}).",
            record.phase
        )));
    }
    let (link, token) = environments(settings)
        .link(&record.environment_id)?
        .ok_or_else(|| {
            WorkspaceError::Conflict(format!("No link to environment {}.", record.environment_id))
        })?;
    check_route(&link.origin, &path)?;
    let body = serde_json::to_value(&request).map_err(internal)?;
    call(Method::POST, &link.origin, &path, Some(&token), Some(&body)).map_err(|error| match error {
        PeerError::Status(404, detail) => WorkspaceError::NotFound(detail),
        PeerError::Status(409, detail) => WorkspaceError::Conflict(detail),
        PeerError::Status(400 | 422, detail) => WorkspaceError::Validation(detail),
        PeerError::Status(401 | 403, detail) => WorkspaceError::Forbidden(detail),
        error => WorkspaceError::ServiceUnavailable(format!(
            "The answer's outcome is unknown ({error}); it was not resent. Read the delegation to see whether the question closed."
        )),
    })
}

/// An identifier for one segment of a peer's route, percent-encoded. Refused
/// when a `/`-separated part of it is empty, `.` or `..`, or it holds a
/// backslash or control character, so it never names another route.
fn route_id(name: &str, id: &str) -> WorkspaceResult<String> {
    if id.split('/').any(|part| matches!(part, "" | "." | ".."))
        || id.chars().any(|c| c == '\\' || c.is_control())
    {
        return Err(validation(format!(
            "{name} {id:?} is not a valid identifier."
        )));
    }
    Ok(encode(id))
}

/// Refuses, before anything is sent, a route that does not parse back to
/// exactly `origin` and `path`: one that normalizing would move to another
/// route or origin.
fn check_route(origin: &str, path: &str) -> WorkspaceResult<()> {
    let url = format!("{origin}{path}");
    match reqwest::Url::parse(&url) {
        Ok(parsed) if parsed.as_str() == url => Ok(()),
        _ => Err(validation(format!(
            "{path:?} does not stay on its route at {origin}."
        ))),
    }
}

/// Percent-encodes one path segment.
fn encode(segment: &str) -> String {
    segment
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            byte => format!("%{byte:02X}"),
        })
        .collect()
}

/// A's follower: one per server process. Errors back off per record.
#[derive(Default)]
pub struct Follower {
    backoff: HashMap<String, (Instant, Duration)>,
    /// Fault injection for contracts: the worker following this environment
    /// panics.
    #[doc(hidden)]
    pub panic_environment: Option<String>,
    /// Fault injection for contracts: following this delegation panics.
    #[doc(hidden)]
    pub panic_delegation: Option<String>,
}

impl Follower {
    /// One pass over every open record whose follow is due. Environments are
    /// polled in parallel so one unreachable peer does not stall healthy
    /// ones, and each record is followed on its own, so one failing or
    /// panicking record backs off alone. Returns the diagnostics it printed
    /// for records that failed and for records and workers that panicked;
    /// their records back off.
    // ponytail: scoped threads per tick, one request per due record; move to
    // independent per-environment tasks if a slow peer delaying the next tick
    // ever matters.
    pub fn tick(
        &mut self,
        settings: &SparkSettings,
        store: &Arc<ConversationStore>,
    ) -> Vec<String> {
        let Ok(records) = environments(settings).followed_out() else {
            return Vec::new();
        };
        self.backoff
            .retain(|id, _| records.iter().any(|record| &record.delegation_id == id));
        let now = Instant::now();
        let mut by_env: BTreeMap<String, Vec<DelegationOut>> = BTreeMap::new();
        for record in records {
            if self
                .backoff
                .get(&record.delegation_id)
                .is_some_and(|(due, _)| *due > now)
            {
                continue;
            }
            by_env
                .entry(record.environment_id.clone())
                .or_default()
                .push(record);
        }
        let (panic_environment, panic_delegation) = (
            self.panic_environment.as_deref(),
            self.panic_delegation.as_deref(),
        );
        let lost = || Err::<(), _>("The follower panicked.".to_string());
        let follow_env = |env: &str, records: &[DelegationOut]| {
            if panic_environment == Some(env) {
                panic!("Injected follower panic for environment {env}.");
            }
            let mut reported = Vec::new();
            let outcomes: Vec<_> = records
                .iter()
                .map(|record| {
                    let id = &record.delegation_id;
                    // A panic loses only this record's outcome.
                    let followed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        if panic_delegation == Some(id.as_str()) {
                            panic!("Injected follower panic for delegation {id}.");
                        }
                        follow(settings, store, record)
                    }));
                    let outcome = match followed {
                        Ok(Err(error)) => {
                            reported.push(format!(
                                "Following delegation {id} on environment {env} failed; it \
                                 retries after a backoff: {error}"
                            ));
                            Err(error)
                        }
                        Ok(outcome) => outcome,
                        Err(_) => {
                            reported.push(format!(
                                "The delegation follower panicked following delegation {id} on \
                                 environment {env}; it retries after a backoff."
                            ));
                            lost()
                        }
                    };
                    (id.clone(), outcome)
                })
                .collect();
            (outcomes, reported)
        };
        let mut diagnostics = Vec::new();
        let outcomes = std::thread::scope(|scope| {
            let workers: Vec<_> = by_env
                .iter()
                .map(|(env, records)| (env, records, scope.spawn(|| follow_env(env, records))))
                .collect();
            let mut outcomes = Vec::new();
            for (env, records, worker) in workers {
                if let Ok((followed, reported)) = worker.join() {
                    outcomes.extend(followed);
                    diagnostics.extend(reported);
                    continue;
                }
                // A panic outside any one record loses the worker's outcomes:
                // report it, and back its records off.
                let ids: Vec<_> = records.iter().map(|r| r.delegation_id.clone()).collect();
                diagnostics.push(format!(
                    "The delegation follower for environment {env} panicked; delegations {} \
                     retry after a backoff.",
                    ids.join(", ")
                ));
                outcomes.extend(ids.into_iter().map(|id| (id, lost())));
            }
            outcomes
        });
        for diagnostic in &diagnostics {
            eprintln!("{diagnostic}");
        }
        let finished = Instant::now();
        for (id, outcome) in outcomes {
            match outcome {
                Ok(()) => {
                    self.backoff.remove(&id);
                }
                Err(_) => {
                    let delay = self
                        .backoff
                        .get(&id)
                        .map_or(FOLLOW_INTERVAL, |(_, delay)| (*delay * 2).min(MAX_BACKOFF));
                    self.backoff.insert(id, (finished + delay, delay));
                }
            }
        }
        diagnostics
    }
}

/// Moves one record forward: sends a pending cancellation, submits a record
/// B has not acknowledged, or polls an accepted one. Returns an error only
/// for a failure worth retrying; a refusal pauses the record.
fn follow(
    settings: &SparkSettings,
    store: &Arc<ConversationStore>,
    record: &DelegationOut,
) -> Result<(), String> {
    if let Some(reason) = record.paused.as_deref() {
        if record.cancel_requested_at.is_some() && record.submitted_at.is_none() {
            seal_canceled_unsubmitted(settings, store, record, reason)
                .map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    let repository = environments(settings);
    let id = &record.delegation_id;
    let Some((link, token)) = repository
        .link(&record.environment_id)
        .map_err(|e| e.to_string())?
    else {
        return pause_or_seal_canceled(
            settings,
            store,
            id,
            "No link to this environment; link it again to resume.",
        );
    };
    let path = format!("{INCOMING_PATH}/{id}");
    let peer = |method: Method, path: &str, body: Option<Value>| {
        call(method, &link.origin, path, Some(&token), body.as_ref())
    };
    // A created record with a cancel is never submitted: its cancel is sent
    // again until B's answer moves it on, even once acknowledged.
    let cancel = record.cancel_requested_at.is_some()
        && (record.cancel_acknowledged_at.is_none() || record.phase == "created");
    let response = if cancel {
        // Sent first, so a task B never admitted is tombstoned, not launched.
        let body = json!({"origin_environment_id": store.environment_id()});
        peer(Method::POST, &format!("{path}/cancel"), Some(body))
    } else if record.phase == "created" {
        // Noted first, so a cancel while paused knows B may have it.
        if !repository.submit_out(id).map_err(|e| e.to_string())? {
            return Ok(());
        }
        let body = json!({
            "delegation_id": id,
            "origin_environment_id": store.environment_id(),
            "project_path": record.project_path,
            "kind": record.kind,
            "request": record.request,
            "request_sha256": record.request_sha256,
        });
        peer(Method::POST, INCOMING_PATH, Some(body))
    } else {
        peer(Method::GET, &path, None)
    };
    let state = match response {
        Ok(state) => state,
        Err(PeerError::Network(error)) => return Err(error),
        Err(PeerError::Status(status, detail)) if status >= 500 || status == 429 => {
            return Err(detail)
        }
        Err(PeerError::Status(status, detail)) => {
            let reason = match status {
                401 | 403 => format!("The environment refused this link's token ({detail}); link it again to resume."),
                404 => format!("The environment does not know this delegation ({detail})."),
                _ => format!("The environment refused this delegation ({status}: {detail})."),
            };
            return pause_or_seal_canceled(settings, store, id, &reason);
        }
    };
    apply(settings, store, record, &state).map_err(|error| error.to_string())?;
    // Only once B's answer is stored, so a failed apply resends the cancel.
    if cancel {
        repository
            .acknowledge_cancel_out(id)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn delegation_inbox_payload(record: &DelegationOut, state: &Value) -> Value {
    json!({
        "delegation_id": record.delegation_id,
        "environment_id": record.environment_id,
        "project_path": record.project_path,
        "kind": record.kind,
        "request_key": record.request_key,
        "target_kind": state["target_kind"],
        "target_id": state["target_id"],
    })
}

fn live_owner<'a>(
    conversations: &ConversationRepository,
    record: &'a DelegationOut,
) -> WorkspaceResult<Option<&'a str>> {
    let Some(owner) = record.owner_conversation_id.as_deref() else {
        return Ok(None);
    };
    Ok(conversations
        .conversation_project_path(owner)?
        .map(|_| owner))
}

/// Seals an outgoing record (see `seal_out_on` for `unsubmitted`) and
/// stores its one `delegation.result` inbox event in the same transaction
/// when its owner conversation still exists, then delivers it. Once both
/// are committed a delivery failure is only reported: the event stays in
/// the inbox for the next delivery or startup recovery. Returns whether it
/// sealed.
fn seal_out_and_notify(
    settings: &SparkSettings,
    store: &Arc<ConversationStore>,
    record: &DelegationOut,
    state: &Value,
    result: Value,
    unsubmitted: bool,
) -> WorkspaceResult<bool> {
    let id = &record.delegation_id;
    let conversations = ConversationRepository::new(store.clone());
    let Some(owner) = live_owner(&conversations, record)? else {
        return Ok(environments(settings).seal_out(id, &result, unsubmitted)?);
    };
    let mut payload = delegation_inbox_payload(record, state);
    payload["result"] = result.clone();
    let mut sealed = false;
    conversations.append_inbox_event_with(
        owner,
        NewInboxEvent {
            id: format!("delegation:{id}:result"),
            kind: "delegation.result".to_string(),
            source: id.clone(),
            payload,
        },
        |connection| {
            sealed = seal_out_on(connection, id, &result, unsubmitted)?;
            Ok(sealed)
        },
    )?;
    if sealed {
        if let Err(error) = inbox::process_conversation(settings, store, owner) {
            eprintln!(
                "Delegation {id} sealed, but delivering its result to conversation {owner} failed: {error}. The event waits in its inbox."
            );
        }
    }
    Ok(sealed)
}

/// Stores B's view of a record: its target once B acknowledges, new
/// questions in the owner's inbox, and the seal with its inbox event. A view
/// that would leave more than `MAX_OUTSTANDING_QUESTIONS` question
/// notifications outstanding is refused before anything is stored, so the
/// record backs off and keeps its last view.
fn apply(
    settings: &SparkSettings,
    store: &Arc<ConversationStore>,
    record: &DelegationOut,
    state: &Value,
) -> WorkspaceResult<()> {
    let repository = environments(settings);
    let id = &record.delegation_id;
    let sealed = state["phase"] == "sealed";
    let conversations = ConversationRepository::new(store.clone());
    let owner = if sealed {
        None
    } else {
        live_owner(&conversations, record)?
    };
    let prefix = format!("delegation:{id}:question:");
    // B's open questions, each under its notification ID. Run questions carry
    // `run_id` so sibling child runs sharing a gate ID do not collide.
    let questions: Vec<(String, &Value)> = state["questions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|question| {
            let qid = question["question_id"].as_str()?;
            let key = match non_empty(question["run_id"].as_str()) {
                Some(run_id) => format!("{run_id}:{qid}"),
                None => qid.to_string(),
            };
            Some((format!("{prefix}{key}"), question))
        })
        .collect();
    if let Some(owner) = owner {
        let pending = conversations.pending_inbox_events(owner)?;
        let outstanding: BTreeSet<&str> = questions
            .iter()
            .map(|(event_id, _)| event_id.as_str())
            .chain(pending.iter().map(|event| event.id.as_str()))
            .filter(|event_id| event_id.starts_with(&prefix))
            .collect();
        if outstanding.len() > MAX_OUTSTANDING_QUESTIONS {
            return Err(WorkspaceError::Conflict(format!(
                "The environment's view of delegation {id} would leave {} questions outstanding \
                 in its owner's inbox, more than {MAX_OUTSTANDING_QUESTIONS}; it was not stored. \
                 It is taken once the owner has taken earlier questions and they have closed.",
                outstanding.len()
            )));
        }
    }
    if record.phase == "created" {
        if let Some(target_id) = non_empty(state["target_id"].as_str()) {
            repository.accept_out(id, &target_id)?;
        }
    }
    repository.set_out_state(id, state)?;
    if sealed {
        let result = state.get("result").cloned().unwrap_or(Value::Null);
        return seal_out_and_notify(settings, store, record, state, result, false).map(drop);
    }
    let Some(owner) = owner else {
        return Ok(());
    };
    let mut payload = delegation_inbox_payload(record, state);
    let mut changed = false;
    for (event_id, question) in questions {
        payload["question"] = question.clone();
        changed |= inbox::append(
            store,
            owner,
            InboxPost {
                id: Some(event_id),
                kind: "delegation.question".to_string(),
                source: Some(id.clone()),
                payload: payload.clone(),
            },
            id,
        )?;
    }
    if changed {
        inbox::process_conversation(settings, store, owner)?;
    }
    Ok(())
}

// ---- Incoming delegations (B) -------------------------------------------------

/// What A submits.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IncomingDelegation {
    pub delegation_id: String,
    pub origin_environment_id: String,
    pub project_path: String,
    pub kind: String,
    pub request: Value,
    pub request_sha256: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IncomingCancel {
    #[serde(default)]
    pub origin_environment_id: String,
}

/// The canonical form of a delegation ID A sends, which must be a UUID.
fn delegation_uuid(delegation_id: &str) -> WorkspaceResult<String> {
    Ok(uuid::Uuid::parse_str(delegation_id.trim())
        .map_err(|_| validation("delegation_id must be a UUID."))?
        .to_string())
}

/// Serializes admission, cancellation, sealing and startup launch.
fn incoming_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// Admits a delegation at most once under its ID and launches it: an
/// existing admission with the same request, or a cancellation tombstone,
/// is returned as is. A task that fails validation is sealed, not launched.
pub fn admit(
    conversations: &WorkspaceConversationService,
    request: IncomingDelegation,
) -> WorkspaceResult<Value> {
    let delegation_id = delegation_uuid(&request.delegation_id)?;
    if request.request_sha256
        != request_digest(&request.project_path, &request.kind, &request.request)
    {
        return Err(validation("request_sha256 does not match the request."));
    }
    let _lock = incoming_lock();
    let repository = environments(conversations.settings());
    if let Some(existing) = repository.in_(&delegation_id)? {
        // A tombstone has no request.
        if !existing.request_sha256.is_empty() && existing.request_sha256 != request.request_sha256
        {
            return Err(WorkspaceError::Conflict(format!(
                "Delegation {delegation_id} was admitted with a different request."
            )));
        }
        if existing.phase == "accepted" {
            launch(conversations, &existing)?;
        }
        return state(conversations, &delegation_id);
    }
    let now = now();
    let mut row = DelegationIn {
        delegation_id: delegation_id.clone(),
        origin_environment_id: request.origin_environment_id.clone(),
        project_path: request.project_path.clone(),
        kind: request.kind.clone(),
        request: request.request.clone(),
        request_sha256: request.request_sha256.clone(),
        target_kind: match request.kind.as_str() {
            "flow" => "run",
            "conversation" => "conversation",
            _ => "",
        }
        .to_string(),
        target_id: uuid::Uuid::new_v4().to_string(),
        phase: "accepted".to_string(),
        result: None,
        created_at: now.clone(),
        sealed_at: None,
        canceled_at: None,
    };
    match validate(conversations, &request) {
        Ok(()) => {
            let row = repository.admit_in(&row)?;
            launch(conversations, &row)?;
        }
        Err(error) => {
            row.phase = "sealed".to_string();
            row.result = Some(json!({"status": "validation_error", "error": error.to_string()}));
            row.sealed_at = Some(now);
            repository.admit_in(&row)?;
        }
    }
    state(conversations, &delegation_id)
}

/// The checks the ordinary launch or turn makes, before anything starts.
fn validate(
    conversations: &WorkspaceConversationService,
    request: &IncomingDelegation,
) -> WorkspaceResult<()> {
    let registered = crate::WorkspaceProjectService::new(conversations.settings().clone())
        .list_projects()?
        .iter()
        .any(|project| project.project_path == request.project_path);
    if !registered {
        return Err(validation(format!(
            "{} is not a project registered on this environment.",
            request.project_path
        )));
    }
    match request.kind.as_str() {
        "flow" => {
            let task: FlowTask = decode(&request.request)?;
            conversations.validate_run_launch(&RunLaunchRequest {
                flow_name: task.flow_name,
                summary: task.summary,
                goal: task.goal,
                launch_context: task.launch_context,
                project_path: Some(request.project_path.clone()),
                ..RunLaunchRequest::default()
            })
        }
        "conversation" => {
            let task: ConversationTask = decode(&request.request)?;
            if task.message.trim().is_empty() {
                return Err(validation("A conversation task requires a message."));
            }
            Ok(())
        }
        kind => Err(validation(format!("Unknown delegation kind {kind:?}."))),
    }
}

fn target_exists(
    conversations: &WorkspaceConversationService,
    row: &DelegationIn,
) -> WorkspaceResult<bool> {
    Ok(match row.target_kind.as_str() {
        "run" => run_record(conversations.settings(), &row.target_id)?.is_some(),
        _ => ConversationRepository::new(conversations.store().clone())
            .conversation_project_path(&row.target_id)?
            .is_some(),
    })
}

/// Launches an admitted row's target under its fixed ID through the
/// ordinary path, then marks it launched. A target that already exists is
/// only marked; a launch that fails without creating it seals the task.
fn launch(conversations: &WorkspaceConversationService, row: &DelegationIn) -> WorkspaceResult<()> {
    let repository = environments(conversations.settings());
    let launched = if target_exists(conversations, row)? {
        Ok(())
    } else if row.target_kind == "run" {
        decode::<FlowTask>(&row.request).and_then(|task| {
            let mut context = task
                .launch_context
                .as_ref()
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            context.insert(
                DELEGATION_CONTEXT_KEY.to_string(),
                json!({"environment_id": row.origin_environment_id, "delegation_id": row.delegation_id}),
            );
            conversations
                .launch_workspace_run(RunLaunchRequest {
                    flow_name: task.flow_name,
                    summary: task.summary,
                    goal: task.goal,
                    launch_context: Some(Value::Object(context)),
                    project_path: Some(row.project_path.clone()),
                    run_id: Some(row.target_id.clone()),
                    ..RunLaunchRequest::default()
                })
                .map(drop)
        })
    } else {
        decode::<ConversationTask>(&row.request).and_then(|task| {
            let (prepared, _) = conversations.send_turn(
                &row.target_id,
                ConversationTurnRequest {
                    project_path: row.project_path.clone(),
                    message: task.message,
                    client_turn_id: Some(row.delegation_id.clone()),
                    chat_mode: Some("chat".to_string()),
                    ..ConversationTurnRequest::default()
                },
            )?;
            if let Some(prepared) = prepared {
                let publish = inbox::runtime(conversations.settings())
                    .map(|runtime| runtime.publish)
                    .unwrap_or_else(|| Arc::new(|_| {}));
                inbox::complete_in_background(publish, conversations.clone(), prepared);
            }
            Ok(())
        })
    };
    match launched {
        Ok(()) => {
            repository.mark_in_launched(&row.delegation_id)?;
        }
        Err(_) if target_exists(conversations, row)? => {
            repository.mark_in_launched(&row.delegation_id)?;
        }
        Err(error) => {
            repository.seal_in(
                &row.delegation_id,
                &json!({"status": "failed", "error": error.to_string()}),
            )?;
        }
    }
    Ok(())
}

/// Startup: launches every admitted row a previous process left unlaunched,
/// once, after run and inbox recovery.
pub fn launch_accepted(conversations: &WorkspaceConversationService) -> WorkspaceResult<()> {
    let _lock = incoming_lock();
    for row in environments(conversations.settings()).accepted_in()? {
        launch(conversations, &row)?;
    }
    Ok(())
}

/// B's view of a delegation: `{delegation_id, phase, target_kind,
/// target_id, status, questions}` and, once sealed, `result`. The first
/// view that meets the seal rule writes the seal; later activity in the
/// target never changes it.
pub fn incoming_state(
    conversations: &WorkspaceConversationService,
    delegation_id: &str,
) -> WorkspaceResult<Value> {
    let _lock = incoming_lock();
    state(conversations, delegation_id)
}

fn state(
    conversations: &WorkspaceConversationService,
    delegation_id: &str,
) -> WorkspaceResult<Value> {
    let repository = environments(conversations.settings());
    let mut row = repository
        .in_(delegation_id)?
        .ok_or_else(|| WorkspaceError::NotFound(format!("Unknown delegation: {delegation_id}")))?;
    let (status, questions) = match row.phase.as_str() {
        "sealed" => (
            row.result
                .as_ref()
                .map_or(Value::Null, |r| r["status"].clone()),
            vec![],
        ),
        "accepted" => (json!("accepted"), vec![]),
        _ => {
            let (status, questions, result) = if row.target_kind == "run" {
                run_view(conversations, &row.target_id)?
            } else {
                conversation_view(conversations, &row)?
            };
            if let Some(result) = result {
                row = repository.seal_in(delegation_id, &result)?.unwrap_or(row);
                (
                    row.result
                        .as_ref()
                        .map_or(Value::Null, |r| r["status"].clone()),
                    vec![],
                )
            } else {
                (json!(status), questions)
            }
        }
    };
    let mut state = json!({
        "delegation_id": row.delegation_id,
        "phase": row.phase,
        "target_kind": row.target_kind,
        "target_id": row.target_id,
        "status": status,
        "questions": questions,
    });
    if let Some(result) = row.result {
        state["result"] = result;
    }
    Ok(state)
}

/// A delegated run: its status, its tree's open questions, and its result
/// once it is terminal and no executor can still replace a pending one.
fn run_view(
    conversations: &WorkspaceConversationService,
    run_id: &str,
) -> WorkspaceResult<(String, Vec<Value>, Option<Value>)> {
    let settings = conversations.settings();
    let runs = attractor_runtime::RunStore::for_settings(settings);
    let record = match runs.find_run_root(run_id).map_err(internal)? {
        Some(paths) => runs
            .read_run_record(&paths)
            .map_err(internal)?
            .map(|r| (paths, r)),
        None => None,
    };
    let Some((paths, record)) = record else {
        return Ok((
            "failed".to_string(),
            vec![],
            Some(json!({"status": "failed", "error": "The delegated run no longer exists."})),
        ));
    };
    let status = attractor_runtime::normalize_run_status(record.status.trim());
    let api = conversations.runtime_api_service();
    if TERMINAL_RUN_STATUSES.contains(&status.as_str()) {
        let settled = !attractor_api::run_has_live_executor(run_id)
            || runs
                .read_result(&paths)
                .map_err(internal)?
                .is_some_and(|result| result.state != "pending");
        if !settled {
            return Ok((status, vec![], None));
        }
        let response = api.get_pipeline_result(run_id);
        if response.status_code != 200 {
            return Err(internal(response.body));
        }
        return Ok((status, vec![], Some(response.body)));
    }
    let response = api.list_pipeline_questions(run_id);
    if response.status_code != 200 {
        return Err(internal(response.body));
    }
    let questions = response.body["questions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    Ok((status, questions, None))
}

/// A delegated conversation: `running` while a turn is in progress,
/// `waiting` on an open question, plan review or in-flight roster run, else
/// `idle`. Its result, once an assistant turn has ended and nothing is in
/// progress, pending or open, is the last assistant turn. Once canceled,
/// nothing waits on its owner: open questions expire, plans awaiting review
/// are rejected, and a turn that failed reports `canceled`. Its transcript,
/// roster and pending inbox events are read in one transaction, so a run
/// whose report has just started a turn is never seen ended without it.
// ponytail: reads the whole conversation each poll; read a summary if delegated chats grow long.
fn conversation_view(
    conversations: &WorkspaceConversationService,
    row: &DelegationIn,
) -> WorkspaceResult<(String, Vec<Value>, Option<Value>)> {
    let conversation_id = row.target_id.as_str();
    let store = conversations.store();
    let repository = ConversationRepository::new(store.clone());
    if row.canceled_at.is_some() {
        end_waits(conversations, &repository, conversation_id)?;
    }
    let Some((snapshot, runs, pending)) = repository.read_snapshot_with_inbox(conversation_id)?
    else {
        return Ok((
            "failed".to_string(),
            vec![],
            Some(json!({"status": "failed", "error": "The delegated conversation was deleted."})),
        ));
    };
    let items = |key: &str| snapshot[key].as_array().cloned().unwrap_or_default();
    let turns = items("turns");
    let running = turns.iter().any(|turn| {
        matches!(
            turn["status"].as_str(),
            Some("pending" | "streaming" | "queued")
        )
    });
    let questions: Vec<Value> = items("segments")
        .into_iter()
        .filter(|segment| segment["kind"] == "request_user_input" && segment["status"] == "pending")
        .filter_map(|segment| {
            let request = &segment["request_user_input"];
            let question_id = conversation_question_id(&segment)?;
            let mut question = json!({
                "question_id": question_id,
                "questions": request["questions"],
            });
            if request["form"].is_object() {
                question["form"] = request["form"].clone();
            }
            Some(question)
        })
        .collect();
    let review = items("proposed_plans")
        .iter()
        .any(|plan| plan["status"] == "pending_review");
    let in_flight = runs.iter().any(|run| run.in_flight());
    let status = if running {
        "running"
    } else if !questions.is_empty() || review || in_flight {
        "waiting"
    } else {
        "idle"
    };
    let last = turns.iter().rev().find(|turn| {
        turn["role"] == "assistant"
            && matches!(turn["status"].as_str(), Some("complete" | "failed"))
    });
    let result = match last {
        Some(turn) if status == "idle" && pending.is_empty() => {
            let canceled = row.canceled_at.is_some() && turn["status"] == "failed";
            Some(json!({
                "status": if canceled { json!("canceled") } else { turn["status"].clone() },
                "text": turn["content"],
                "error": turn.get("error").cloned().unwrap_or(Value::Null),
            }))
        }
        _ => None,
    };
    Ok((status.to_string(), questions, result))
}

/// A conversation request's question ID in a delegation: its adapter's
/// `request_id`, or, without one, the ID of the segment that stores it, so
/// two requests never share one.
fn conversation_question_id(segment: &Value) -> Option<&str> {
    [&segment["request_user_input"]["request_id"], &segment["id"]]
        .into_iter()
        .find_map(|id| id.as_str().filter(|id| !id.is_empty()))
}

/// Ends what a canceled conversation waits for from its owner, through the
/// ordinary paths: its open questions expire, so the turn waiting for them
/// ends, and its plans awaiting review are rejected. A live execution is
/// interrupted separately; its later writes are dropped once its turn
/// settles.
fn end_waits(
    conversations: &WorkspaceConversationService,
    repository: &ConversationRepository,
    conversation_id: &str,
) -> WorkspaceResult<()> {
    let Some(project_path) = repository.conversation_project_path(conversation_id)? else {
        return Ok(());
    };
    let Some(snapshot) = repository.read_snapshot(conversation_id, None)? else {
        return Ok(());
    };
    for segment in snapshot["segments"].as_array().into_iter().flatten() {
        if segment["kind"] == "request_user_input" && segment["status"] == "pending" {
            if let Some(segment_id) = segment["id"].as_str() {
                repository.expire_question(conversation_id, &project_path, segment_id)?;
            }
        }
    }
    for plan in snapshot["proposed_plans"].as_array().into_iter().flatten() {
        if plan["status"] == "pending_review" {
            if let Some(plan_id) = plan["id"].as_str() {
                conversations.review_proposed_plan(
                    conversation_id,
                    plan_id,
                    ProposedPlanReviewRequest {
                        project_path: project_path.clone(),
                        disposition: "rejected".to_string(),
                        review_note: Some("The delegation was canceled.".to_string()),
                    },
                )?;
            }
        }
    }
    Ok(())
}

/// Cancels a delegation: an unknown ID becomes a `canceled` tombstone, an
/// unlaunched admission seals `canceled`, and a launched target that has not
/// already ended is marked canceled and stopped; it then seals with whatever
/// outcome it reaches.
pub fn cancel_incoming(
    conversations: &WorkspaceConversationService,
    delegation_id: &str,
    request: IncomingCancel,
) -> WorkspaceResult<Value> {
    let delegation_id = &delegation_uuid(delegation_id)?;
    let _lock = incoming_lock();
    let repository = environments(conversations.settings());
    match repository.in_(delegation_id)? {
        None => {
            repository.tombstone_in(delegation_id, request.origin_environment_id.trim())?;
        }
        Some(row) if row.phase == "accepted" => {
            repository.seal_in(delegation_id, &json!({"status": "canceled"}))?;
        }
        Some(row) if row.phase == "launched" => {
            // A target that ended before the cancel keeps its outcome.
            let current = state(conversations, delegation_id)?;
            if current["phase"] == "sealed" {
                return Ok(current);
            }
            repository.cancel_in(delegation_id)?;
            let api = conversations.runtime_api_service();
            if row.target_kind == "run" {
                // An ended run refuses; its seal stands.
                let _ = api.cancel_pipeline_route(&row.target_id);
            } else {
                let _ = conversations.interrupt_turn(&row.target_id, &row.project_path);
                for run in Inbox::read(conversations.store(), &row.target_id)?.runs {
                    if run.in_flight() {
                        let _ = api.cancel_pipeline_route(&run.run_id);
                    }
                }
            }
        }
        Some(_) => {}
    }
    state(conversations, delegation_id)
}

/// Answers an open question of a launched delegation through the ordinary
/// answer path, after checking the question belongs to its target.
pub fn answer_incoming(
    conversations: &WorkspaceConversationService,
    delegation_id: &str,
    question_id: &str,
    request: DelegationAnswerRequest,
) -> WorkspaceResult<Value> {
    let row = environments(conversations.settings())
        .in_(delegation_id)?
        .ok_or_else(|| WorkspaceError::NotFound(format!("Unknown delegation: {delegation_id}")))?;
    if row.phase != "launched" {
        return Err(WorkspaceError::Conflict(format!(
            "Delegation {delegation_id} has no open questions ({}).",
            row.phase
        )));
    }
    let given = [
        !request.selected_value.is_empty(),
        !request.answers.is_empty(),
        request.form.is_some(),
    ];
    if given.into_iter().filter(|given| *given).count() != 1 {
        return Err(validation(
            "Give exactly one of selected_value, answers or form.",
        ));
    }
    let unknown = || {
        WorkspaceError::NotFound(format!(
            "Question {question_id} is not open in delegation {delegation_id}."
        ))
    };
    let requested_run_id = non_empty(request.run_id.as_deref());
    if row.target_kind == "run" {
        if !request.answers.is_empty() {
            return Err(validation("A run question takes selected_value or form."));
        }
        let api = conversations.runtime_api_service();
        let questions = api.list_pipeline_questions(&row.target_id).body;
        let matching: Vec<&Value> = questions["questions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|question| question["question_id"] == question_id)
            .collect();
        let question = match requested_run_id {
            Some(target_run_id) => matching
                .into_iter()
                .find(|question| question["run_id"].as_str() == Some(target_run_id.as_str()))
                .ok_or_else(unknown)?,
            None => match matching.as_slice() {
                [] => return Err(unknown()),
                [only] => *only,
                _ => {
                    return Err(WorkspaceError::Conflict(format!(
                        "Question {question_id} is open in multiple runs of delegation {delegation_id}; specify run_id."
                    )));
                }
            },
        };
        let run_id = question["run_id"].as_str().unwrap_or(&row.target_id);
        let response = api.answer_pipeline_question(
            run_id,
            question_id,
            attractor_api::HumanAnswerRequest {
                selected_value: request.selected_value,
                note: String::new(),
                form: request.form,
            },
        );
        let detail = response.body["detail"].to_string();
        return match response.status_code {
            // A refused answer leaves the question open.
            200..=299 if response.body["status"] == "validation_error" => Err(validation(
                response.body["error"]
                    .as_str()
                    .unwrap_or("The answer was refused."),
            )),
            200..=299 => Ok(response.body),
            404 => Err(WorkspaceError::NotFound(detail)),
            400 | 422 => Err(WorkspaceError::Validation(detail)),
            _ => Err(WorkspaceError::Internal(detail)),
        };
    }
    if requested_run_id.is_some() {
        return Err(validation(
            "A conversation question does not accept run_id.",
        ));
    }
    let snapshot = ConversationRepository::new(conversations.store().clone())
        .read_snapshot(&row.target_id, None)?
        .ok_or_else(unknown)?;
    let segment = snapshot["segments"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|segment| {
            segment["kind"] == "request_user_input"
                && segment["status"] == "pending"
                && conversation_question_id(segment) == Some(question_id)
        })
        .ok_or_else(unknown)?;
    // Answered under its segment, the one stored request this question names.
    let segment_id = segment["id"].as_str().ok_or_else(unknown)?;
    if segment["request_user_input"]["form"].is_object() && request.form.is_none() {
        return Err(validation(format!(
            "Request {question_id} is a form: reply with form."
        )));
    }
    let answers = if request.form.is_some() || !request.answers.is_empty() {
        request.answers
    } else {
        let questions = segment["request_user_input"]["questions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let [question] = questions.as_slice() else {
            return Err(validation(format!(
                "Request {question_id} asks {} questions; give answers by question ID.",
                questions.len()
            )));
        };
        BTreeMap::from([(
            question["id"].as_str().unwrap_or_default().to_string(),
            request.selected_value,
        )])
    };
    conversations.submit_request_user_input_answer(
        &row.target_id,
        segment_id,
        ConversationRequestUserInputAnswerRequest {
            project_path: row.project_path.clone(),
            answers,
            form: request.form,
        },
    )?;
    Ok(json!({"question_id": question_id, "answered": true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;

    #[test]
    fn call_with_timeout_enforces_one_deadline_across_a_slow_streaming_body_and_marks_relay() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen_forwarded = Arc::new(Mutex::new(String::new()));
        let seen_clone = seen_forwarded.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut req = [0u8; 2048];
            let n = stream.read(&mut req).unwrap_or(0);
            *seen_clone.lock().unwrap() = String::from_utf8_lossy(&req[..n]).to_lowercase();
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n",
            );
            let _ = stream.flush();
            for _ in 0..40 {
                if stream.write_all(b"1\r\n \r\n").is_err() || stream.flush().is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });

        let started = Instant::now();
        let result = call_with_timeout(
            Method::GET,
            &format!("http://{addr}"),
            "/slow",
            Some("test-token"),
            None,
            Duration::from_millis(200),
        );
        let elapsed = started.elapsed();

        assert!(
            matches!(&result, Err(PeerError::Network(msg)) if msg.contains("did not answer within")),
            "expected deadline timeout, got {result:?}"
        );
        assert!(
            elapsed < Duration::from_secs(1),
            "slow streaming body outlasted deadline: {elapsed:?}"
        );
        let headers = seen_forwarded.lock().unwrap().clone();
        assert!(
            headers.contains("forwarded: by=spark-peer"),
            "expected relay marker in headers: {headers}"
        );
        let _ = server.join();
    }
}
