//! Every conversation's inbox. Run reports, trigger deliveries and messages
//! are appended to the conversation store and reach the conversation as
//! turns: one batch per turn, at most one turn in flight. A mission or
//! standing role gates delivery to its conversation; a chat has none (see
//! `process_conversation`). A run's owner conversation is fixed at launch in
//! `context.spark_owner`, and child runs report to their root's owner.
use crate::conversations::{InboxDelivery, PreparedConversationTurn, WorkspaceConversationService};
use crate::live::LiveEnvelope;
use crate::{WorkspaceError, WorkspaceResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use spark_agent_adapter::AgentTurnBackend;
use spark_common::settings::SparkSettings;
use spark_storage::conversation::NewInboxEvent;
use spark_storage::{ConversationRepository, ConversationStore};
use std::sync::Arc;

pub use spark_storage::conversation::{InboxEvent, InboxRun as RosterEntry, TERMINAL_RUN_STATUSES};
pub const OWNER_CONTEXT_KEY: &str = "context.spark_owner";

/// A mission's or role's inbox as its responses show it, read from its
/// conversation's inbox; never stored in the record.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Inbox {
    pub runs: Vec<RosterEntry>,
    /// Sequence of the newest inbox event delivered to the conversation.
    pub cursor: u64,
    /// Sequence of the newest inbox event.
    pub event_seq: u64,
}
impl Inbox {
    pub fn in_flight(&self) -> usize {
        self.runs.iter().filter(|run| run.in_flight()).count()
    }
    /// The inbox of `conversation_id`; empty while the conversation has none.
    pub(crate) fn read(
        store: &Arc<ConversationStore>,
        conversation_id: &str,
    ) -> WorkspaceResult<Self> {
        let repository = ConversationRepository::new(store.clone());
        let (event_seq, cursor) = repository.inbox_sequence(conversation_id)?;
        Ok(Self {
            runs: repository.inbox_runs(conversation_id)?,
            cursor: cursor as u64,
            event_seq: event_seq as u64,
        })
    }
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

/// What a server installs so every conversation in its process runs inbox
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
    store: &Arc<ConversationStore>,
    runtime: &InboxRuntime,
) -> WorkspaceConversationService {
    WorkspaceConversationService::new_with_agent_turn_backend(
        settings.clone(),
        store.clone(),
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

/// Appends an event to a live conversation's inbox unless its id is already
/// there; returns whether it was new.
pub(crate) fn append(
    store: &Arc<ConversationStore>,
    conversation_id: &str,
    post: InboxPost,
    default_source: &str,
) -> WorkspaceResult<bool> {
    let event = NewInboxEvent {
        id: post
            .id
            .filter(|id| !id.trim().is_empty())
            .unwrap_or_else(|| format!("event-{}", uuid::Uuid::new_v4())),
        kind: post.kind,
        source: post
            .source
            .filter(|source| !source.trim().is_empty())
            .unwrap_or_else(|| default_source.into()),
        payload: post.payload,
    };
    Ok(ConversationRepository::new(store.clone())
        .append_inbox_event(conversation_id, event)?
        .is_some())
}

/// A message from one conversation to another, both named by handle.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadMessageRequest {
    /// The sender's handle.
    pub conversation_handle: String,
    pub message: String,
}

/// Stores `request` as a `thread.message` event in the inbox of the
/// conversation with handle `to`, then processes that conversation. Every
/// call is a new message. Returns the stored event's recipient.
pub fn send_message(
    settings: &SparkSettings,
    store: &Arc<ConversationStore>,
    to: &str,
    request: ThreadMessageRequest,
) -> WorkspaceResult<Value> {
    let message = request.message.trim();
    if request.conversation_handle.trim().is_empty() {
        return Err(invalid("Sender conversation handle is required"));
    }
    if message.is_empty() {
        return Err(invalid("Message is required"));
    }
    let repository = ConversationRepository::new(store.clone());
    let resolve = |handle: &str| {
        repository
            .find_conversation_by_handle(handle)?
            .ok_or_else(|| {
                WorkspaceError::NotFound(format!("Unknown conversation handle: {handle}"))
            })
    };
    let recipient = resolve(to)?;
    let sender = resolve(&request.conversation_handle)?;
    let missions = crate::missions::WorkspaceMissionService::new(settings.clone(), store.clone());
    let roles = crate::roles::WorkspaceRoleService::new(settings.clone(), store.clone());
    let label = match missions.conversation_mission(&sender.project_path, &sender.conversation_id) {
        Some(mission) => Some(format!("mission {:?}", mission.fields.title)),
        None => roles
            .conversation_role(&sender.project_path, &sender.conversation_id)
            .map(|role| format!("standing role {}", role.name)),
    };
    let mut identity = json!({
        "conversation_id": sender.conversation_id,
        "project_path": sender.project_path,
        "conversation_handle": sender.conversation_handle,
    });
    if let Some(label) = label {
        identity["label"] = json!(label);
    }
    let post = InboxPost {
        id: None,
        kind: "thread.message".into(),
        source: Some(sender.conversation_id.clone()),
        payload: json!({"sender": identity, "message": message}),
    };
    if !append(
        store,
        &recipient.conversation_id,
        post,
        &sender.conversation_id,
    )? {
        return Err(WorkspaceError::Internal(
            "The message was not stored".into(),
        ));
    }
    // Stored is accepted; a delivery failure is the recipient's to show.
    let _ = process_conversation(settings, store, &recipient.conversation_id);
    Ok(json!({
        "ok": true,
        "conversation_id": recipient.conversation_id,
        "conversation_handle": recipient.conversation_handle,
    }))
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

/// A launch context with whatever owner the caller supplied replaced by the
/// server-derived `owner` conversation; without one the run is unowned.
pub(crate) fn owned_launch_context(
    mut context: Map<String, Value>,
    owner: Option<&str>,
) -> Map<String, Value> {
    context.remove(OWNER_CONTEXT_KEY);
    if let Some(conversation_id) = owner {
        context.insert(
            OWNER_CONTEXT_KEY.into(),
            json!({ "conversation_id": conversation_id }),
        );
    }
    context
}

/// Adds a run launched from `conversation_id` to its roster, then reconciles
/// it at once so a run that already finished still reports. Delivers
/// nothing: the caller processes the conversation once it holds no lock.
pub(crate) fn register_launch(
    settings: &SparkSettings,
    store: &Arc<ConversationStore>,
    conversation_id: &str,
    run_id: &str,
    flow_name: &str,
    summary: &str,
) -> WorkspaceResult<()> {
    ConversationRepository::new(store.clone()).register_inbox_run(
        conversation_id,
        run_id,
        flow_name,
        summary,
    )?;
    if let Some(paths) = run_paths(settings, run_id)? {
        reconcile_run(settings, store, conversation_id, &paths, false)?;
    }
    Ok(())
}

/// Delivers a conversation's pending inbox events through its gate: a
/// mission or standing role under its own rules, a chat always. Takes the
/// mission or role record lock, so callers hold no record, turn or storage
/// lock.
pub fn process_conversation(
    settings: &SparkSettings,
    store: &Arc<ConversationStore>,
    conversation_id: &str,
) -> WorkspaceResult<()> {
    let Some(project) =
        ConversationRepository::new(store.clone()).conversation_project_path(conversation_id)?
    else {
        return Ok(());
    };
    let missions = crate::missions::WorkspaceMissionService::new(settings.clone(), store.clone());
    if missions
        .conversation_mission(&project, conversation_id)
        .is_some()
    {
        return missions.process(&project, conversation_id).map(drop);
    }
    let roles = crate::roles::WorkspaceRoleService::new(settings.clone(), store.clone());
    if let Some(role) = roles.conversation_role(&project, conversation_id) {
        return roles.process(&project, &role.name).map(drop);
    }
    deliver_inbox(settings, store, conversation_id, &project, None).map(drop)
}

/// Sends the pending inbox events, after `begin` when given, to the
/// conversation: an idle one starts a turn with them, a running one takes
/// them live, and otherwise they wait in the inbox for the next turn.
/// Messages held or queued in the conversation start first. Returns whether
/// a turn started. Runs under the gate's record lock, if any.
pub(crate) fn deliver_inbox(
    settings: &SparkSettings,
    store: &Arc<ConversationStore>,
    conversation_id: &str,
    project_path: &str,
    begin: Option<&str>,
) -> WorkspaceResult<bool> {
    let Some(runtime) = runtime(settings) else {
        return Ok(false);
    };
    let conversations = conversations(settings, store, &runtime);
    let through_seq = ConversationRepository::new(store.clone())
        .pending_inbox_events(conversation_id)?
        .last()
        .map_or(0, |event| event.seq);
    let delivery = if through_seq == 0 && begin.is_none() {
        InboxDelivery::Waiting
    } else {
        match conversations.deliver_inbox_batch(conversation_id, project_path, through_seq, begin) {
            Ok(delivery) => delivery,
            // The events stay pending for the next processing opportunity.
            Err(error) => {
                conversations.append_notice(
                    conversation_id,
                    project_path,
                    &format!(
                        "Could not start a turn for this conversation's inbox: {error}. Its events wait for the next turn."
                    ),
                )?;
                return Ok(false);
            }
        }
    };
    let prepared = match delivery {
        InboxDelivery::Started(prepared) => prepared,
        InboxDelivery::Joined => return Ok(false),
        InboxDelivery::Waiting => {
            match conversations.start_queued_turn(conversation_id, project_path)? {
                Some((prepared, _)) => prepared,
                None => return Ok(false),
            }
        }
    };
    complete_in_background(runtime, conversations, prepared);
    Ok(true)
}

/// Completes a started turn and each message queued behind it on its own
/// thread; its completion then processes the inbox again. Committed changes
/// reach live clients through the store's commit notices; only transient
/// stream deltas are published here.
fn complete_in_background(
    runtime: InboxRuntime,
    conversations: WorkspaceConversationService,
    prepared: PreparedConversationTurn,
) {
    std::thread::spawn(move || {
        let publish = runtime.publish.clone();
        let (conversation, project_path) = (
            prepared.conversation_id.clone(),
            prepared.project_path.clone(),
        );
        let _ =
            conversations.complete_started_turn_with_progress_payloads(prepared, move |payload| {
                publish(crate::live::conversation_event_envelope(
                    &conversation,
                    &project_path,
                    payload,
                ));
            });
    });
}

/// A run tree's owner conversation and root record.
#[derive(Clone)]
pub(crate) struct RunOwner {
    pub conversation_id: String,
    pub paths: attractor_runtime::paths::RunRootPaths,
}

/// The reserved owner context: `{"conversation_id": ...}`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnerContext {
    conversation_id: String,
}

fn internal(error: attractor_runtime::RuntimeStorageError) -> WorkspaceError {
    WorkspaceError::Internal(error.to_string())
}

fn run_paths(
    settings: &SparkSettings,
    run_id: &str,
) -> WorkspaceResult<Option<attractor_runtime::paths::RunRootPaths>> {
    attractor_runtime::RunStore::for_settings(settings)
        .find_run_root(run_id)
        .map_err(internal)
}

fn record_owner(record: &attractor_core::RunRecord) -> Option<String> {
    let owner = record.launch_context.as_ref()?.get(OWNER_CONTEXT_KEY)?;
    serde_json::from_value::<OwnerContext>(owner.clone())
        .ok()
        .map(|owner| owner.conversation_id)
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
    let owner = record_owner(&record).map(|conversation_id| RunOwner {
        conversation_id,
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

/// Hands a run tree's state to the conversation that owns it, and returns
/// whether its inbox changed. A report whose owner no longer exists is
/// discarded, never redirected.
pub fn deliver_run_events(
    settings: &SparkSettings,
    store: &Arc<ConversationStore>,
    run_id: &str,
) -> WorkspaceResult<bool> {
    let Some(owner) = run_owner(settings, run_id)? else {
        return Ok(false);
    };
    if ConversationRepository::new(store.clone())
        .conversation_project_path(&owner.conversation_id)?
        .is_none()
    {
        eprintln!(
            "Run {run_id} reports to conversation {}, which no longer exists; its report is discarded.",
            owner.conversation_id
        );
        return Ok(false);
    }
    let changed = reconcile_run(settings, store, &owner.conversation_id, &owner.paths, false)?;
    if changed {
        process_conversation(settings, store, &owner.conversation_id)?;
    }
    Ok(changed)
}

/// Startup recovery, once orphaned turns are settled: restores owned root
/// runs a crash left out of their roster (not finished runs already
/// reported, which a roster may have trimmed), reconciles every in-flight
/// roster run, then processes every live conversation with pending events.
pub fn recover(settings: &SparkSettings, store: &Arc<ConversationStore>) -> WorkspaceResult<()> {
    let repository = ConversationRepository::new(store.clone());
    let runs = attractor_runtime::RunStore::for_settings(settings);
    for record in runs.list_run_records().map_err(internal)? {
        // Continued runs keep their source's owner but were never launched
        // from the conversation.
        if record.parent_run_id.is_some() || record.continued_from_run_id.is_some() {
            continue;
        }
        let Some(conversation_id) = record_owner(&record) else {
            continue;
        };
        if repository
            .conversation_project_path(&conversation_id)?
            .is_some()
        {
            let status = attractor_runtime::normalize_run_status(record.status.trim());
            let reported = TERMINAL_RUN_STATUSES
                .contains(&status.as_str())
                .then(|| report_id(&record.run_id, &status));
            let _ = repository.restore_inbox_run(
                &conversation_id,
                &record.run_id,
                &record.flow_name,
                record.title.as_deref().unwrap_or_default(),
                Some(record.started_at.trim()).filter(|started| !started.is_empty()),
                reported.as_deref(),
            );
        }
    }
    for (conversation_id, run_id) in repository.in_flight_inbox_runs()? {
        if let Some(paths) = run_paths(settings, &run_id)? {
            let _ = reconcile_run(settings, store, &conversation_id, &paths, true);
        }
    }
    for (conversation_id, _) in repository.pending_inbox_conversations()? {
        let _ = process_conversation(settings, store, &conversation_id);
    }
    Ok(())
}

/// The id of a run tree's terminal or recovery-wait report.
fn report_id(root_id: &str, status: &str) -> String {
    format!("{root_id}:{status}")
}

/// Reconciles an owned run tree into its owner's inbox: appends its
/// questions once per owning run and question id, its terminal or
/// recovery-wait event once, and its roster status. `recovering` accepts
/// terminal runs whose result is still pending (after a restart no executor
/// remains to replace it). Returns whether the inbox changed.
pub(crate) fn reconcile_run(
    settings: &SparkSettings,
    store: &Arc<ConversationStore>,
    conversation_id: &str,
    paths: &attractor_runtime::paths::RunRootPaths,
    recovering: bool,
) -> WorkspaceResult<bool> {
    let runs = attractor_runtime::RunStore::for_settings(settings);
    let root_id = &paths.run_id;
    let mut changed = false;
    let Some(record) = runs.read_run_record(paths).map_err(internal)? else {
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
            store,
            conversation_id,
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
            || !runs
                .read_result(paths)
                .map_err(internal)?
                .is_some_and(|result| result.state == "pending")
        {
            changed |= append(
                store,
                conversation_id,
                InboxPost {
                    id: Some(report_id(root_id, &status)),
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
    changed |= ConversationRepository::new(store.clone()).set_inbox_run_status(
        conversation_id,
        root_id,
        roster_status,
    )?;
    Ok(changed)
}
