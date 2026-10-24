//! The inbox missions and standing roles share: run events, trigger
//! deliveries, and messages land in a record's inbox and reach its
//! conversation as turns, one batch per turn and at most one turn in flight,
//! under the record store's project lock. A run's owner is fixed at launch in
//! `context.spark_owner`.
use crate::conversations::{
    ConversationTurnRequest, RunLaunchRequest, WorkspaceConversationService,
};
use crate::live::LiveEnvelope;
use crate::{WorkspaceError, WorkspaceResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use spark_agent_adapter::AgentTurnBackend;
use spark_common::settings::SparkSettings;
use spark_storage::workspace_records::RecordRepository;
use std::sync::Arc;

pub(crate) const TERMINAL_RUN_STATUSES: &[&str] =
    &["completed", "failed", "validation_error", "canceled"];
pub const OWNER_CONTEXT_KEY: &str = "context.spark_owner";

/// Flattened into the owning record.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Inbox {
    pub runs: Vec<RosterEntry>,
    /// Sequence of the newest inbox event delivered to the conversation.
    pub cursor: u64,
    /// Sequence of the newest inbox event, so appends change the record.
    pub event_seq: u64,
}
impl Inbox {
    pub fn in_flight(&self) -> usize {
        self.runs.iter().filter(|run| run.in_flight()).count()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RosterEntry {
    pub run_id: String,
    #[serde(default)]
    pub flow_name: String,
    #[serde(default)]
    pub summary: String,
    pub launched_at: String,
    pub status: String,
}
impl RosterEntry {
    pub fn in_flight(&self) -> bool {
        !TERMINAL_RUN_STATUSES.contains(&self.status.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InboxEvent {
    pub seq: u64,
    pub id: String,
    pub at: String,
    pub kind: String,
    pub source: String,
    #[serde(default)]
    pub payload: Value,
}

/// An event to append; a repeated `id` is dropped as a duplicate.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InboxPost {
    #[serde(default)]
    pub id: Option<String>,
    pub kind: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub payload: Value,
}

/// Who a run reports to: `{"kind": "mission" | "role", "id": ...}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Owner {
    Mission(String),
    Role(String),
}

/// What a server installs so every mission and role in its process runs
/// turns through its agent backend and publishes live updates. Without one,
/// inbox events stay pending.
#[derive(Clone)]
pub struct InboxRuntime {
    pub agent_turn_backend: Arc<dyn AgentTurnBackend>,
    pub publish: Arc<dyn Fn(LiveEnvelope) + Send + Sync>,
}

// ponytail: keyed by data dir so tests with separate homes can share a process.
fn runtimes(
) -> &'static std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, InboxRuntime>> {
    static RUNTIMES: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, InboxRuntime>>,
    > = std::sync::OnceLock::new();
    RUNTIMES.get_or_init(Default::default)
}
pub fn install_runtime(settings: &SparkSettings, runtime: InboxRuntime) {
    runtimes()
        .lock()
        .unwrap()
        .insert(settings.data_dir.clone(), runtime);
}
pub(crate) fn runtime(settings: &SparkSettings) -> Option<InboxRuntime> {
    runtimes().lock().unwrap().get(&settings.data_dir).cloned()
}
pub(crate) fn conversations(
    settings: &SparkSettings,
    runtime: &InboxRuntime,
) -> WorkspaceConversationService {
    WorkspaceConversationService::new_with_agent_turn_backend(
        settings.clone(),
        runtime.agent_turn_backend.clone(),
    )
}

pub(crate) fn now() -> String {
    time::OffsetDateTime::now_utc().to_string()
}
pub(crate) fn invalid(message: impl Into<String>) -> WorkspaceError {
    WorkspaceError::Validation(message.into())
}
pub(crate) fn decode<T: serde::de::DeserializeOwned>(value: Value) -> WorkspaceResult<T> {
    serde_json::from_value(value).map_err(|e| invalid(e.to_string()))
}

pub(crate) fn read_events(repo: &RecordRepository, id: &str) -> WorkspaceResult<Vec<InboxEvent>> {
    repo.read_events(id)?.into_iter().map(decode).collect()
}

/// Appends an event unless its id was already delivered; returns whether it was new.
pub(crate) fn append(
    repo: &RecordRepository,
    id: &str,
    inbox: &mut Inbox,
    post: InboxPost,
    default_source: &str,
) -> WorkspaceResult<bool> {
    let events = read_events(repo, id)?;
    let event_id = post
        .id
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| format!("event-{}", uuid::Uuid::new_v4()));
    // ponytail: linear duplicate scan; index ids if inboxes grow large.
    if events.iter().any(|event| event.id == event_id) {
        return Ok(false);
    }
    let event = InboxEvent {
        seq: events.last().map_or(0, |event| event.seq) + 1,
        id: event_id,
        at: now(),
        kind: post.kind,
        source: post
            .source
            .filter(|source| !source.trim().is_empty())
            .unwrap_or_else(|| default_source.into()),
        payload: post.payload,
    };
    repo.append_event(id, &serde_json::to_value(&event).unwrap())?;
    inbox.event_seq = event.seq;
    Ok(true)
}

/// The event a trigger delivery appends.
pub(crate) fn trigger_post(request: &spark_triggers::TriggerActivationRequest) -> InboxPost {
    InboxPost {
        // A retried delivery carries the same id and is skipped as a duplicate.
        id: request
            .delivery_id
            .as_ref()
            .map(|delivery| format!("trigger:{}:{delivery}", request.trigger_id)),
        kind: "trigger.fired".into(),
        source: Some(request.trigger_id.clone()),
        payload: json!({"trigger_id": request.trigger_id, "trigger_name": request.trigger_name, "source_type": request.source_type, "source_payload": request.source_payload}),
    }
}

/// Launches a flow for the inbox's owner: records `owner` in the launch
/// context so the run's events come back, and adds the run to the roster.
pub(crate) fn launch_recorded(
    inbox: &mut Inbox,
    owner: &Owner,
    mut request: RunLaunchRequest,
    start: impl FnOnce(RunLaunchRequest) -> WorkspaceResult<Value>,
) -> WorkspaceResult<Value> {
    let mut context = match request.launch_context.take() {
        Some(Value::Object(context)) => context,
        _ => Map::new(),
    };
    context.insert(OWNER_CONTEXT_KEY.into(), json!(owner));
    request.launch_context = Some(Value::Object(context));
    let (flow_name, summary) = (request.flow_name.clone(), request.summary.clone());
    let response = start(request)?;
    inbox.runs.push(RosterEntry {
        run_id: response["run_id"].as_str().unwrap_or_default().to_string(),
        flow_name,
        summary,
        launched_at: now(),
        status: "running".into(),
    });
    Ok(response)
}

/// Sends pending inbox events, after `begin` when given, as the next turn of
/// `conversation`. While a turn runs they go into it if it takes them, else
/// they wait in the inbox for the next turn. `after` runs once a started turn
/// ends, to deliver what arrived meanwhile. Returns whether a turn started.
/// Must run under the record store's lock.
#[allow(clippy::too_many_arguments)]
pub(crate) fn deliver_inbox(
    settings: &SparkSettings,
    repo: &RecordRepository,
    id: &str,
    conversation: &str,
    project_path: &str,
    inbox: &mut Inbox,
    begin: Option<String>,
    after: impl FnOnce() + Send + 'static,
) -> WorkspaceResult<bool> {
    let Some(runtime) = runtime(settings) else {
        return Ok(false);
    };
    if begin.is_none() && inbox.event_seq <= inbox.cursor {
        return Ok(false);
    }
    let conversations = conversations(settings, &runtime);
    let lines: Vec<String> = read_events(repo, id)?
        .iter()
        .filter(|event| event.seq > inbox.cursor)
        .map(|event| render_event(inbox, event))
        .collect();
    let message = begin
        .into_iter()
        .chain(lines)
        .collect::<Vec<_>>()
        .join("\n\n");
    if conversations.turn_in_flight(conversation, project_path) {
        if conversations.deliver_message(conversation, project_path, &message)? {
            inbox.cursor = inbox.event_seq;
        }
        return Ok(false);
    }
    inbox.cursor = inbox.event_seq;
    let request = ConversationTurnRequest {
        project_path: project_path.to_string(),
        message: message.clone(),
        ..Default::default()
    };
    let (prepared, snapshot) = match conversations.send_turn(conversation, request) {
        Ok((Some(prepared), snapshot)) => (prepared, snapshot),
        // Queued behind a turn that started meanwhile; it starts after it.
        Ok((None, _)) => return Ok(false),
        // Only the user can move an inbox whose turn cannot start.
        Err(error) => {
            conversations.append_notice(
                conversation,
                project_path,
                &format!("Could not start a turn: {error}\n\n{message}"),
            )?;
            return Ok(false);
        }
    };
    if let Ok(envelope) =
        crate::live::conversation_snapshot_envelope(settings, conversation, project_path)
    {
        (runtime.publish)(envelope);
    }
    let settings = settings.clone();
    std::thread::spawn(move || {
        let publish = runtime.publish.clone();
        let (conversation, project_path) = (
            prepared.conversation_id.clone(),
            prepared.project_path.clone(),
        );
        let (progress_conversation, progress_project) =
            (conversation.clone(), project_path.clone());
        let _ = conversations.complete_started_turn_with_progress_payloads(
            prepared,
            snapshot,
            move |payload| {
                let revision = payload["revision"].as_i64().unwrap_or(0);
                publish(crate::live::conversation_event_envelope(
                    &progress_conversation,
                    &progress_project,
                    payload,
                    revision,
                ));
            },
        );
        if let Ok(envelope) =
            crate::live::conversation_snapshot_envelope(&settings, &conversation, &project_path)
        {
            (runtime.publish)(envelope);
        }
        after();
    });
    Ok(true)
}

/// A run tree's owner and root record.
#[derive(Clone)]
pub(crate) struct RunOwner {
    pub owner: Owner,
    pub project: String,
    pub paths: attractor_runtime::paths::RunRootPaths,
}

fn internal(error: attractor_runtime::RuntimeStorageError) -> WorkspaceError {
    WorkspaceError::Internal(error.to_string())
}

/// The owner recorded at the launch of `run_id`'s root, if any.
pub(crate) fn run_owner(
    settings: &SparkSettings,
    run_id: &str,
) -> WorkspaceResult<Option<RunOwner>> {
    let key = (settings.runs_dir.clone(), run_id.to_string());
    if let Some(owner) = run_owners().lock().unwrap().get(&key).cloned() {
        return Ok(owner);
    }
    let store = attractor_runtime::RunStore::for_settings(settings);
    let Some(paths) = store.find_run_root(run_id).map_err(internal)? else {
        return Ok(None);
    };
    let Some(record) = store.read_run_record(&paths).map_err(internal)? else {
        return Ok(None);
    };
    let root_id = record.root_run_id.as_deref().unwrap_or(run_id);
    let Some(paths) = store.find_run_root(root_id).map_err(internal)? else {
        return Ok(None);
    };
    let Some(record) = store.read_run_record(&paths).map_err(internal)? else {
        return Ok(None);
    };
    let owner = record
        .launch_context
        .as_ref()
        .and_then(|context| context.get(OWNER_CONTEXT_KEY))
        .and_then(|owner| serde_json::from_value::<Owner>(owner.clone()).ok())
        .map(|owner| RunOwner {
            owner,
            project: if record.project_path.trim().is_empty() {
                record.working_directory.clone()
            } else {
                record.project_path.clone()
            },
            paths,
        });
    run_owners().lock().unwrap().insert(key, owner.clone());
    Ok(owner)
}

/// Ownership is fixed at launch, so unowned runs are answered from memory.
// ponytail: one entry per observed run for the process lifetime, like the publisher's cursors.
#[allow(clippy::type_complexity)]
fn run_owners() -> &'static std::sync::Mutex<
    std::collections::HashMap<(std::path::PathBuf, String), Option<RunOwner>>,
> {
    static OWNERS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<(std::path::PathBuf, String), Option<RunOwner>>>,
    > = std::sync::OnceLock::new();
    OWNERS.get_or_init(Default::default)
}

/// Hands a run tree's state to the mission or role that owns it.
pub fn deliver_run_events(settings: &SparkSettings, run_id: &str) -> WorkspaceResult<()> {
    match run_owner(settings, run_id)?.map(|owner| owner.owner) {
        Some(Owner::Mission(_)) => crate::missions::WorkspaceMissionService::new(settings.clone())
            .deliver_run_events(run_id)
            .map(|_| ()),
        Some(Owner::Role(_)) => crate::roles::WorkspaceRoleService::new(settings.clone())
            .deliver_run_events(run_id)
            .map(|_| ()),
        None => Ok(()),
    }
}

/// Reconciles an owned run tree into the inbox: appends its questions once
/// per owning run and question id, its terminal or recovery-wait event once,
/// and its roster status. `recovering` accepts terminal runs whose result is
/// still pending (after a restart no executor remains to replace it).
/// Returns whether the inbox changed.
pub(crate) fn reconcile_run(
    settings: &SparkSettings,
    repo: &RecordRepository,
    id: &str,
    inbox: &mut Inbox,
    paths: &attractor_runtime::paths::RunRootPaths,
    recovering: bool,
) -> WorkspaceResult<bool> {
    let store = attractor_runtime::RunStore::for_settings(settings);
    let root_id = &paths.run_id;
    let mut changed = false;
    let Some(record) = store.read_run_record(paths).map_err(internal)? else {
        return Ok(false);
    };
    let status = attractor_runtime::normalize_run_status(record.status.trim());
    // Unanswered gates remain in the event log after termination, but can no longer block work.
    let questions = if TERMINAL_RUN_STATUSES.contains(&status.as_str()) {
        vec![]
    } else {
        let response = attractor_api::AttractorApiService::new(settings.clone())
            .list_pipeline_questions(root_id);
        if response.status_code != 200 {
            return Err(WorkspaceError::Internal(response.body.to_string()));
        }
        response.body["questions"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    };
    for mut question in questions.iter().cloned() {
        question["root_run_id"] = json!(root_id);
        let question_id = question["question_id"].as_str().unwrap_or_default();
        let source = question["run_id"].as_str().unwrap_or(root_id).to_string();
        changed |= append(
            repo,
            id,
            inbox,
            InboxPost {
                id: Some(format!("question:{source}:{question_id}")),
                kind: "run.question".into(),
                source: Some(source.clone()),
                payload: question,
            },
            &source,
        )?;
    }
    // Gate records can still say waiting while their answer is being consumed.
    // Recovery decisions are the only non-question waits.
    let recovery_wait = status == "waiting"
        && record.outcome_reason_code.as_deref() == Some("recovery_decision_required");
    let roster_status = if !questions.is_empty() || recovery_wait {
        "waiting"
    } else if status == "waiting" {
        "running"
    } else {
        &status
    };
    let kind = match status.as_str() {
        "completed" => Some("run.completed"),
        "failed" | "validation_error" => Some("run.failed"),
        "canceled" => Some("run.canceled"),
        "waiting" if questions.is_empty() && recovery_wait => Some("run.waiting"),
        _ => None,
    };
    if let Some(kind) = kind {
        if recovering
            || kind == "run.waiting"
            || !store
                .read_result(paths)
                .map_err(internal)?
                .is_some_and(|result| result.state == "pending")
        {
            changed |= append(
                repo,
                id,
                inbox,
                InboxPost {
                    id: Some(format!("{root_id}:{status}")),
                    kind: kind.into(),
                    source: Some(root_id.clone()),
                    payload: json!({"status": status, "flow_name": record.flow_name, "outcome": record.outcome, "error": record.last_error}),
                },
                root_id,
            )?;
        } else {
            return Ok(changed);
        }
    }
    if let Some(run) = inbox.runs.iter_mut().find(|run| run.run_id == *root_id) {
        if run.in_flight() && run.status != roster_status {
            run.status = roster_status.into();
            changed = true;
        }
    }
    Ok(changed)
}

/// One line per event: the run's flow, summary, and status, and its id to
/// inspect it by; or the message a human or another role sent.
pub(crate) fn render_event(inbox: &Inbox, event: &InboxEvent) -> String {
    let payload = &event.payload;
    match event.kind.as_str() {
        "trigger.fired" => {
            let data = serde_json::to_string(&payload["source_payload"]).unwrap_or_default();
            let excerpt: String = data.chars().take(4096).collect();
            return format!(
                "Trigger fired: {} ({})\nExternal data (full payload: {} bytes{}):\n> {}",
                payload["trigger_name"],
                payload["source_type"],
                data.len(),
                if excerpt.len() < data.len() {
                    "; truncated"
                } else {
                    ""
                },
                excerpt
            );
        }
        "human.message" => {
            return format!("User: {}", payload["message"].as_str().unwrap_or_default())
        }
        "role.message" => {
            return format!(
                "Message from {}: {}",
                payload["from"].as_str().unwrap_or("another conversation"),
                payload["message"].as_str().unwrap_or_default()
            )
        }
        "mission.closed" => {
            return format!(
                "Mission {} ({}) closed as {}: {}",
                payload["title"],
                payload["mission_id"].as_str().unwrap_or_default(),
                payload["status"].as_str().unwrap_or_default(),
                payload["reason"].as_str().unwrap_or_default()
            )
        }
        "run.question" => return format!("Run question: {payload}"),
        "mission.updated" => {
            let mut lines = vec![
                "The user updated this mission; these replace the values in your instructions."
                    .to_string(),
            ];
            if let Some(title) = payload["title"].as_str() {
                lines.push(format!("Title: {title}"));
            }
            if let Some(objective) = payload["objective"].as_str() {
                lines.push(format!("Objective:\n{objective}"));
            }
            if let Some(budget) = payload.get("budget") {
                lines.push(format!(
                    "Budget: {} concurrent runs, {} runs in total.",
                    budget["concurrent_runs"], budget["total_runs"]
                ));
            }
            return lines.join("\n");
        }
        _ => {}
    }
    let run = inbox.runs.iter().find(|run| run.run_id == event.source);
    let flow = run
        .map(|run| run.flow_name.as_str())
        .filter(|flow| !flow.is_empty())
        .or(payload["flow_name"].as_str())
        .unwrap_or("unknown flow");
    let summary = run.map_or("", |run| run.summary.as_str());
    let outcome = match event.kind.as_str() {
        "run.waiting" => "is waiting on a recovery decision".to_string(),
        _ => format!("ended {}", payload["status"].as_str().unwrap_or("")),
    };
    let error = payload["error"]
        .as_str()
        .filter(|error| !error.trim().is_empty())
        .map(|error| format!(": {error}"))
        .unwrap_or_default();
    format!(
        "Run {} ({flow}, {summary:?}) {outcome}{error}.",
        event.source
    )
}
