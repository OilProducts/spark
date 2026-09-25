//! Missions: a project objective plus a long-lived agent conversation that owns
//! the mission's runs. Run events and human messages land in the mission inbox
//! and reach the conversation as turns, one batch per turn and at most one turn
//! in flight, under the project mission lock. Status is derived on read.
use crate::conversations::{
    ConversationTurnRequest, RunLaunchRequest, WorkspaceConversationService,
};
use crate::live::LiveEnvelope;
use crate::{WorkspaceError, WorkspaceResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use spark_agent_adapter::AgentTurnBackend;
use spark_common::settings::SparkSettings;
use spark_storage::{workspace_missions::MissionRepository, ProjectRegistry};
use std::sync::Arc;

const TERMINAL_RUN_STATUSES: &[&str] = &["completed", "failed", "validation_error", "canceled"];
const EDITABLE_FIELDS: &[&str] = &["title", "description", "archived", "budget"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Budget {
    pub concurrent_runs: usize,
    pub total_runs: usize,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            concurrent_runs: 4,
            total_runs: 25,
        }
    }
}

/// Loading ignores fields earlier releases stored; mutations accept only
/// `EDITABLE_FIELDS`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MissionFields {
    pub title: String,
    pub description: String,
    pub archived: bool,
    pub budget: Budget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionMutation {
    pub revision: Option<u64>,
    #[serde(default)]
    pub fields: Map<String, Value>,
    #[serde(default)]
    pub note: String,
    #[serde(default = "human")]
    pub actor: String,
}
fn human() -> String {
    "human".into()
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
    fn in_flight(&self) -> bool {
        !TERMINAL_RUN_STATUSES.contains(&self.status.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseStatus {
    Done,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Closed {
    pub status: CloseStatus,
    pub reason: String,
    pub at: String,
    /// `human` or `assistant`.
    #[serde(default)]
    pub actor: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MissionEvent {
    pub seq: u64,
    pub id: String,
    pub at: String,
    pub kind: String,
    pub source: String,
    #[serde(default)]
    pub payload: Value,
}

/// A message typed into the mission transcript.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionEventPost {
    #[serde(default)]
    pub id: Option<String>,
    pub kind: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionCloseRequest {
    #[serde(default)]
    pub status: Option<CloseStatus>,
    #[serde(default)]
    pub reason: String,
    #[serde(default = "human")]
    pub actor: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionStatus {
    #[default]
    Draft,
    Running,
    NeedsYou,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionRecord {
    pub id: String,
    pub project_id: String,
    pub project_path: String,
    pub created_at: String,
    pub updated_at: String,
    pub revision: u64,
    pub fields: MissionFields,
    pub activity: Vec<Value>,
    /// The mission's conversation, created on Start.
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub runs: Vec<RosterEntry>,
    /// Sequence of the newest inbox event delivered to the conversation.
    #[serde(default)]
    pub cursor: u64,
    /// Sequence of the newest inbox event, so appends change the record.
    #[serde(default)]
    pub event_seq: u64,
    #[serde(default)]
    pub closed: Option<Closed>,
    #[serde(default)]
    pub started_at: Option<String>,
    /// Derived on read; never stored.
    #[serde(default, skip_deserializing)]
    pub status: MissionStatus,
}
impl MissionRecord {
    fn note(&mut self, actor: &str, note: &str) {
        let before = serde_json::to_value(&self.fields).unwrap();
        self.record_activity(actor, note, before);
    }
    fn record_activity(&mut self, actor: &str, note: &str, before: Value) {
        let now = now();
        self.updated_at = now.clone();
        self.revision += 1;
        self.activity.push(json!({"revision": self.revision, "at": now, "actor": actor, "note": note, "before": before, "after": self.fields}));
    }
}

fn now() -> String {
    time::OffsetDateTime::now_utc().to_string()
}
fn invalid(message: impl Into<String>) -> WorkspaceError {
    WorkspaceError::Validation(message.into())
}
fn decode<T: serde::de::DeserializeOwned>(value: Value) -> WorkspaceResult<T> {
    serde_json::from_value(value).map_err(|e| invalid(e.to_string()))
}
fn decode_record(value: Value) -> WorkspaceResult<MissionRecord> {
    // Finished cards from the stage board must not reappear as drafts.
    let finished = value["started_at"].is_null()
        && matches!(value["fields"]["stage"].as_str(), Some("done" | "review"));
    let mut mission: MissionRecord = decode(value)?;
    mission.fields.archived |= finished;
    Ok(mission)
}
fn stored(mission: &MissionRecord) -> Value {
    let mut value = serde_json::to_value(mission).unwrap();
    value.as_object_mut().unwrap().remove("status");
    value
}

/// What a server installs so every mission service in its process runs turns
/// through its agent backend and publishes live updates. Without one, inbox
/// events stay pending.
#[derive(Clone)]
pub struct MissionRuntime {
    pub agent_turn_backend: Arc<dyn AgentTurnBackend>,
    pub publish: Arc<dyn Fn(LiveEnvelope) + Send + Sync>,
}

// ponytail: keyed by data dir so tests with separate homes can share a process.
fn runtimes(
) -> &'static std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, MissionRuntime>> {
    static RUNTIMES: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, MissionRuntime>>,
    > = std::sync::OnceLock::new();
    RUNTIMES.get_or_init(Default::default)
}
pub fn install_runtime(settings: &SparkSettings, runtime: MissionRuntime) {
    runtimes()
        .lock()
        .unwrap()
        .insert(settings.data_dir.clone(), runtime);
}

#[derive(Clone)]
pub struct WorkspaceMissionService {
    settings: SparkSettings,
}
impl WorkspaceMissionService {
    pub fn new(settings: SparkSettings) -> Self {
        Self { settings }
    }
    fn runtime(&self) -> Option<MissionRuntime> {
        runtimes()
            .lock()
            .unwrap()
            .get(&self.settings.data_dir)
            .cloned()
    }
    fn conversations(&self, runtime: &MissionRuntime) -> WorkspaceConversationService {
        WorkspaceConversationService::new_with_agent_turn_backend(
            self.settings.clone(),
            runtime.agent_turn_backend.clone(),
        )
    }
    fn scope(&self, project_path: &str) -> WorkspaceResult<spark_storage::ProjectPaths> {
        if project_path.trim().is_empty() {
            return Err(invalid("Project path is required"));
        }
        Ok(ProjectRegistry::new(self.settings.data_dir.clone())
            .ensure_project_paths(project_path)?)
    }
    fn load(
        repo: &MissionRepository,
        scope: &spark_storage::ProjectPaths,
        id: &str,
    ) -> WorkspaceResult<MissionRecord> {
        let value = repo
            .read(id)?
            .ok_or_else(|| WorkspaceError::NotFound("Unknown project mission".into()))?;
        let mission = decode_record(value)?;
        if mission.project_id != scope.project_id {
            return Err(invalid("Mission belongs to another project"));
        }
        Ok(mission)
    }
    /// Draft until started and Closed once closed. An open mission Needs you
    /// when a run waits on a human gate or nothing is in flight or pending;
    /// otherwise it is Running.
    fn with_status(&self, mut mission: MissionRecord) -> MissionRecord {
        mission.status = if mission.closed.is_some() {
            MissionStatus::Closed
        } else if mission.started_at.is_none() {
            MissionStatus::Draft
        } else if mission.runs.iter().any(|run| run.status == "waiting") {
            MissionStatus::NeedsYou
        } else if mission.event_seq > mission.cursor
            || mission.runs.iter().any(RosterEntry::in_flight)
            || mission.conversation_id.as_deref().is_some_and(|id| {
                WorkspaceConversationService::new(self.settings.clone())
                    .turn_in_flight(id, &mission.project_path)
            })
        {
            MissionStatus::Running
        } else {
            MissionStatus::NeedsYou
        };
        mission
    }
    fn publish(&self, mission: &MissionRecord) {
        if let Some(runtime) = self.runtime() {
            (runtime.publish)(crate::live::mission_upsert_envelope(mission));
        }
    }
    pub fn get(&self, project: &str, id: &str) -> WorkspaceResult<MissionRecord> {
        let scope = self.scope(project)?;
        Self::load(&MissionRepository::new(&scope.root), &scope, id).map(|m| self.with_status(m))
    }
    pub fn list(&self, project: &str) -> WorkspaceResult<Vec<MissionRecord>> {
        let scope = self.scope(project)?;
        let mut missions: Vec<MissionRecord> = MissionRepository::new(&scope.root)
            .list()?
            .into_iter()
            .map(decode_record)
            .collect::<WorkspaceResult<_>>()?;
        if missions.iter().any(|m| m.project_id != scope.project_id) {
            return Err(invalid("Mission belongs to another project"));
        }
        missions.sort_by(|a, b| (&a.created_at, &a.id).cmp(&(&b.created_at, &b.id)));
        Ok(missions.into_iter().map(|m| self.with_status(m)).collect())
    }
    pub fn board(&self, project: &str) -> WorkspaceResult<Value> {
        Ok(json!({"missions": self.list(project)?}))
    }
    /// The mission whose conversation this is, if any: a mission's
    /// conversation id is the mission id.
    pub(crate) fn conversation_mission(
        &self,
        project: &str,
        conversation_id: &str,
    ) -> Option<MissionRecord> {
        let scope = self.scope(project).ok()?;
        Self::load(
            &MissionRepository::new(&scope.root),
            &scope,
            conversation_id,
        )
        .ok()
    }
    fn read_events(repo: &MissionRepository, id: &str) -> WorkspaceResult<Vec<MissionEvent>> {
        repo.read_events(id)?.into_iter().map(decode).collect()
    }

    pub fn create(
        &self,
        project: &str,
        mutation: MissionMutation,
    ) -> WorkspaceResult<MissionRecord> {
        if mutation.revision.is_some() {
            return Err(invalid("Creation must omit revision"));
        }
        self.mutate(
            project,
            &format!("mission-{}", uuid::Uuid::new_v4()),
            mutation,
            true,
        )
    }
    pub fn update(
        &self,
        project: &str,
        id: &str,
        mutation: MissionMutation,
    ) -> WorkspaceResult<MissionRecord> {
        if mutation.revision.is_none() {
            return Err(invalid("Update requires revision"));
        }
        self.mutate(project, id, mutation, false)
    }
    fn mutate(
        &self,
        project: &str,
        id: &str,
        mutation: MissionMutation,
        create: bool,
    ) -> WorkspaceResult<MissionRecord> {
        let scope = self.scope(project)?;
        if mutation.actor != "human" && mutation.actor != "assistant" {
            return Err(invalid("Actor must be human or assistant"));
        }
        if let Some(key) = mutation
            .fields
            .keys()
            .find(|key| !EDITABLE_FIELDS.contains(&key.as_str()))
        {
            return Err(invalid(format!("Mission field `{key}` is not editable")));
        }
        let value = MissionRepository::new(&scope.root).transact::<WorkspaceError>(id, |previous| {
            let now = now();
            let mut mission = match previous {
                Some(value) if !create => decode_record(value)?,
                None if create => decode_record(json!({"id": id, "project_id": scope.project_id, "project_path": scope.project_path, "created_at": now, "updated_at": now, "revision": 0, "fields": {}, "activity": []}))?,
                _ => return Err(WorkspaceError::NotFound("Unknown project mission".into())),
            };
            if !create && mutation.revision != Some(mission.revision) { return Err(WorkspaceError::Conflict(format!("Mission changed; reload revision {} and reconcile your edits", mission.revision))); }
            if mission.project_id != scope.project_id { return Err(invalid("Mission belongs to another project")); }
            let before = serde_json::to_value(&mission.fields).unwrap();
            let mut fields = before.clone();
            for (key, value) in &mutation.fields { fields[key] = value.clone(); }
            let fields: MissionFields = decode(fields)?;
            if fields.title.trim().is_empty() { return Err(invalid("Mission title is required")); }
            mission.fields = fields;
            mission.record_activity(&mutation.actor, &mutation.note, before);
            Ok(stored(&mission))
        })?;
        let mission = self.with_status(decode_record(value)?);
        self.publish(&mission);
        Ok(mission)
    }

    /// Runs `work` on the mission under the project lock, delivers pending
    /// inbox events when `deliver` is set, persists, and publishes.
    fn locked(
        &self,
        project: &str,
        id: &str,
        deliver: bool,
        work: impl FnOnce(&MissionRepository, &mut MissionRecord) -> WorkspaceResult<()>,
    ) -> WorkspaceResult<MissionRecord> {
        let scope = self.scope(project)?;
        let mission = MissionRepository::new(&scope.root).locked::<_, WorkspaceError>(|repo| {
            let mut mission = Self::load(repo, &scope, id)?;
            let before = stored(&mission);
            work(repo, &mut mission)?;
            if deliver {
                self.deliver_pending(repo, &mut mission, None)?;
            }
            let after = stored(&mission);
            if before != after {
                repo.write(id, &after)?;
            }
            Ok(mission)
        })?;
        let mission = self.with_status(mission);
        self.publish(&mission);
        Ok(mission)
    }
    fn with_mission(
        &self,
        project: &str,
        id: &str,
        work: impl FnOnce(&MissionRepository, &mut MissionRecord) -> WorkspaceResult<()>,
    ) -> WorkspaceResult<MissionRecord> {
        self.locked(project, id, true, work)
    }
    pub fn process(&self, project: &str, id: &str) -> WorkspaceResult<MissionRecord> {
        self.with_mission(project, id, |_, _| Ok(()))
    }

    /// Appends an event unless its id was already delivered; returns whether it was new.
    fn append(
        repo: &MissionRepository,
        mission: &mut MissionRecord,
        post: MissionEventPost,
        default_source: &str,
    ) -> WorkspaceResult<bool> {
        let events = Self::read_events(repo, &mission.id)?;
        let id = post
            .id
            .filter(|id| !id.trim().is_empty())
            .unwrap_or_else(|| format!("event-{}", uuid::Uuid::new_v4()));
        // ponytail: linear duplicate scan; index ids if inboxes grow large.
        if events.iter().any(|event| event.id == id) {
            return Ok(false);
        }
        let event = MissionEvent {
            seq: events.last().map_or(0, |event| event.seq) + 1,
            id,
            at: now(),
            kind: post.kind,
            source: post
                .source
                .filter(|source| !source.trim().is_empty())
                .unwrap_or_else(|| default_source.into()),
            payload: post.payload,
        };
        repo.append_event(&mission.id, &serde_json::to_value(&event).unwrap())?;
        mission.event_seq = event.seq;
        Ok(true)
    }
    /// Posts a message typed into the transcript; it reaches the agent as the
    /// next turn.
    pub fn post_event(
        &self,
        project: &str,
        id: &str,
        post: MissionEventPost,
    ) -> WorkspaceResult<MissionRecord> {
        if post.kind != "human.message" {
            return Err(invalid("Only human.message events may be posted"));
        }
        if post.payload["message"]
            .as_str()
            .is_none_or(|message| message.trim().is_empty())
        {
            return Err(invalid("Message is required"));
        }
        self.with_mission(project, id, |repo, mission| {
            Self::append(repo, mission, post, "human").map(|_| ())
        })
    }

    pub fn start(&self, project: &str, id: &str) -> WorkspaceResult<MissionRecord> {
        self.locked(project, id, false, |repo, mission| {
            if mission.started_at.is_some() {
                return Err(WorkspaceError::Conflict("Mission already started".into()));
            }
            if mission.closed.is_some() {
                return Err(WorkspaceError::Conflict("Mission is closed".into()));
            }
            mission.started_at = Some(now());
            mission.conversation_id = Some(mission.id.clone());
            mission.note("human", "Started mission");
            let objective = if mission.fields.description.trim().is_empty() {
                &mission.fields.title
            } else {
                &mission.fields.description
            };
            let begin = format!("Objective:\n{objective}\n\nBegin work on this mission.");
            self.deliver_pending(repo, mission, Some(begin))
        })
    }
    pub fn cancel(&self, project: &str, id: &str) -> WorkspaceResult<MissionRecord> {
        self.with_mission(project, id, |_, mission| {
            let runtime = attractor_api::AttractorApiService::new(self.settings.clone());
            for run in mission.runs.iter().filter(|run| run.in_flight()) {
                runtime.cancel_pipeline_route(&run.run_id);
            }
            if let (Some(runtime), Some(conversation)) =
                (self.runtime(), mission.conversation_id.as_deref())
            {
                let _ = self
                    .conversations(&runtime)
                    .interrupt_turn(conversation, &mission.project_path);
            }
            self.close_locked(
                mission,
                CloseStatus::Canceled,
                "Canceled by human".into(),
                "human",
            )
        })
    }
    pub fn close(
        &self,
        project: &str,
        id: &str,
        request: MissionCloseRequest,
    ) -> WorkspaceResult<MissionRecord> {
        if request.actor != "human" && request.actor != "assistant" {
            return Err(invalid("Actor must be human or assistant"));
        }
        self.with_mission(project, id, |_, mission| {
            let reason = if request.reason.trim().is_empty() {
                "Closed".to_string()
            } else {
                request.reason
            };
            self.close_locked(
                mission,
                request.status.unwrap_or(CloseStatus::Done),
                reason,
                &request.actor,
            )
        })
    }
    /// Closes once and marks the close in the transcript.
    fn close_locked(
        &self,
        mission: &mut MissionRecord,
        status: CloseStatus,
        reason: String,
        actor: &str,
    ) -> WorkspaceResult<()> {
        if mission.closed.is_some() {
            return Ok(());
        }
        let note = format!("Closed as {}: {reason}", json!(status).as_str().unwrap());
        mission.closed = Some(Closed {
            status,
            reason,
            at: now(),
            actor: actor.into(),
        });
        mission.note(actor, &note);
        if let Some(conversation) = &mission.conversation_id {
            WorkspaceConversationService::new(self.settings.clone()).append_notice(
                conversation,
                &mission.project_path,
                &note,
            )?;
        }
        Ok(())
    }

    /// Launches a flow for the mission's agent: refuses a closed mission or a
    /// launch over budget, and injects `context.spark_mission` so the run's
    /// events come back here.
    pub(crate) fn launch(
        &self,
        project: &str,
        id: &str,
        mut request: RunLaunchRequest,
        start: impl FnOnce(RunLaunchRequest) -> WorkspaceResult<Value>,
    ) -> WorkspaceResult<Value> {
        let mut response = Value::Null;
        self.with_mission(project, id, |_, mission| {
            if mission.closed.is_some() {
                return Err(WorkspaceError::Conflict(
                    "Mission is closed; it launches nothing more.".into(),
                ));
            }
            if let Some(limit) = over_budget(mission) {
                return Err(WorkspaceError::Conflict(format!(
                    "Mission budget reached: {limit}. Wait for a run to finish, or ask the user to raise the budget."
                )));
            }
            let mut context = match request.launch_context.take() {
                Some(Value::Object(context)) => context,
                _ => Map::new(),
            };
            context.insert(
                "context.spark_mission".into(),
                json!({"mission_id": mission.id}),
            );
            request.launch_context = Some(Value::Object(context));
            let (flow_name, summary) = (request.flow_name.clone(), request.summary.clone());
            response = start(request)?;
            let run_id = response["run_id"].as_str().unwrap_or_default().to_string();
            mission.runs.push(RosterEntry {
                run_id,
                flow_name,
                summary,
                launched_at: now(),
                status: "running".into(),
            });
            Ok(())
        })?;
        Ok(response)
    }

    /// Sends pending inbox events, after `begin` when given, as the next turn
    /// unless a turn is in flight. Must run under the project lock.
    fn deliver_pending(
        &self,
        repo: &MissionRepository,
        mission: &mut MissionRecord,
        begin: Option<String>,
    ) -> WorkspaceResult<()> {
        let Some(runtime) = self.runtime() else {
            return Ok(());
        };
        if mission.started_at.is_none()
            || mission.closed.is_some()
            || (begin.is_none() && mission.event_seq <= mission.cursor)
        {
            return Ok(());
        }
        let conversation = mission
            .conversation_id
            .get_or_insert_with(|| mission.id.clone())
            .clone();
        let conversations = self.conversations(&runtime);
        if conversations.turn_in_flight(&conversation, &mission.project_path) {
            return Ok(());
        }
        let lines: Vec<String> = Self::read_events(repo, &mission.id)?
            .iter()
            .filter(|event| event.seq > mission.cursor)
            .map(|event| render_event(mission, event))
            .collect();
        mission.cursor = mission.event_seq;
        let message = begin
            .into_iter()
            .chain(lines)
            .collect::<Vec<_>>()
            .join("\n\n");
        let request = ConversationTurnRequest {
            project_path: mission.project_path.clone(),
            message: message.clone(),
            ..Default::default()
        };
        let (prepared, snapshot) = match conversations.start_turn(&conversation, request) {
            Ok(started) => started,
            // Only the user can move a mission whose turn cannot start.
            Err(error) => {
                return conversations.append_notice(
                    &conversation,
                    &mission.project_path,
                    &format!("Could not start a turn: {error}\n\n{message}"),
                )
            }
        };
        if let Ok(envelope) = crate::live::conversation_snapshot_envelope(
            &self.settings,
            &conversation,
            &mission.project_path,
        ) {
            (runtime.publish)(envelope);
        }
        let service = self.clone();
        let (project, id) = (mission.project_path.clone(), mission.id.clone());
        std::thread::spawn(move || {
            let publish = runtime.publish.clone();
            let (conversation, project_path) = (
                prepared.conversation_id.clone(),
                prepared.project_path.clone(),
            );
            let _ = conversations.complete_started_turn_with_progress_payloads(
                prepared,
                snapshot,
                move |payload| {
                    let revision = payload["revision"].as_i64().unwrap_or(0);
                    publish(crate::live::conversation_event_envelope(
                        &conversation,
                        &project_path,
                        payload,
                        revision,
                    ));
                },
            );
            if let Ok(envelope) =
                crate::live::conversation_snapshot_envelope(&service.settings, &id, &project)
            {
                (runtime.publish)(envelope);
            }
            // Events that arrived during the turn go out as the next one.
            let _ = service.process(&project, &id);
        });
        Ok(())
    }

    /// Turns a mission-owned run's terminal or waiting status into one inbox
    /// event keyed by run id plus status.
    pub fn deliver_run_events(&self, run_id: &str) -> WorkspaceResult<Option<MissionRecord>> {
        self.deliver(run_id, false)
    }

    /// `recovering` accepts terminal runs whose result is still pending (after
    /// a restart no executor remains to replace it) and leaves delivery to the
    /// caller.
    fn deliver(&self, run_id: &str, recovering: bool) -> WorkspaceResult<Option<MissionRecord>> {
        let internal =
            |e: attractor_runtime::RuntimeStorageError| WorkspaceError::Internal(e.to_string());
        let store = attractor_runtime::RunStore::for_settings(&self.settings);
        let key = (self.settings.runs_dir.clone(), run_id.to_string());
        let cached = run_owners().lock().unwrap().get(&key).cloned();
        let owner = match cached {
            Some(owner) => owner,
            None => {
                let Some(paths) = store.find_run_root(run_id).map_err(internal)? else {
                    return Ok(None);
                };
                let Some(record) = store.read_run_record(&paths).map_err(internal)? else {
                    return Ok(None);
                };
                let owner = record
                    .launch_context
                    .as_ref()
                    .and_then(|context| context.get("context.spark_mission"))
                    .and_then(|link| link.get("mission_id"))
                    .and_then(Value::as_str)
                    .map(|mission_id| RunOwner {
                        mission_id: mission_id.into(),
                        project: if record.project_path.trim().is_empty() {
                            record.working_directory.clone()
                        } else {
                            record.project_path.clone()
                        },
                        paths,
                        status: None,
                    });
                run_owners()
                    .lock()
                    .unwrap()
                    .insert(key.clone(), owner.clone());
                owner
            }
        };
        let Some(owner) = owner else {
            return Ok(None);
        };
        let Some(record) = store.read_run_record(&owner.paths).map_err(internal)? else {
            return Ok(None);
        };
        let status = attractor_runtime::normalize_run_status(record.status.trim());
        let kind = match status.as_str() {
            "completed" => "run.completed",
            "failed" | "validation_error" => "run.failed",
            "canceled" => "run.canceled",
            // ponytail: roster keeps `waiting` until terminal and a second gate on the
            // same run dedupes by event id; track resumes if gate-heavy flows need it.
            "waiting" => "run.waiting",
            _ => return Ok(None),
        };
        if owner.status.as_ref() == Some(&status) {
            return Ok(None);
        }
        if !recovering
            && kind != "run.waiting"
            && store
                .read_result(&owner.paths)
                .map_err(internal)?
                .is_some_and(|result| result.state == "pending")
        {
            return Ok(None);
        }
        let mut changed = false;
        let mission = self.locked(&owner.project, &owner.mission_id, !recovering, |repo, mission| {
            changed = Self::append(
                repo,
                mission,
                MissionEventPost {
                    id: Some(format!("{run_id}:{status}")),
                    kind: kind.into(),
                    source: Some(run_id.into()),
                    payload: json!({"status": status, "flow_name": record.flow_name, "outcome": record.outcome, "error": record.last_error}),
                },
                run_id,
            )?;
            if let Some(run) = mission.runs.iter_mut().find(|run| run.run_id == run_id) {
                if run.in_flight() {
                    run.status = status.clone();
                }
            }
            Ok(())
        })?;
        if let Some(Some(delivered)) = run_owners().lock().unwrap().get_mut(&key) {
            delivered.status = Some(status);
        }
        Ok(changed.then_some(mission))
    }

    /// Restart recovery: fail turns the previous process left in flight, post
    /// missing terminal events for owned runs, then deliver anything pending.
    pub fn recover(&self) -> WorkspaceResult<Vec<MissionRecord>> {
        let mut recovered = vec![];
        for project in
            ProjectRegistry::new(self.settings.data_dir.clone()).list_project_records()?
        {
            let Ok(missions) = self.list(&project.project_path) else {
                continue;
            };
            for mission in missions {
                if mission.started_at.is_none() || mission.closed.is_some() {
                    continue;
                }
                if let Some(conversation) = &mission.conversation_id {
                    let _ = WorkspaceConversationService::new(self.settings.clone())
                        .abandon_active_turn(conversation, &mission.project_path);
                }
                for run in mission.runs.iter().filter(|run| run.in_flight()) {
                    let _ = self.deliver(&run.run_id, true);
                }
                if let Ok(mission) = self.process(&project.project_path, &mission.id) {
                    recovered.push(mission);
                }
            }
        }
        Ok(recovered)
    }
}

/// One line per event: the run's flow, summary, and status, and its id to
/// inspect it by; or the message a human typed.
fn render_event(mission: &MissionRecord, event: &MissionEvent) -> String {
    let payload = &event.payload;
    if event.kind == "human.message" {
        return format!("User: {}", payload["message"].as_str().unwrap_or_default());
    }
    let run = mission.runs.iter().find(|run| run.run_id == event.source);
    let flow = run
        .map(|run| run.flow_name.as_str())
        .filter(|flow| !flow.is_empty())
        .or(payload["flow_name"].as_str())
        .unwrap_or("unknown flow");
    let summary = run.map_or("", |run| run.summary.as_str());
    let outcome = match event.kind.as_str() {
        "run.waiting" => "is waiting on a human gate".to_string(),
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

/// The mission frame pinned as system instructions on every turn.
pub(crate) fn mission_frame(mission: &MissionRecord, handle: &str) -> String {
    let budget = mission.fields.budget;
    format!(
        "You are the agent for a Spark mission. You own this mission's work until it is closed.\n\n\
        Mission title: {title}\n\
        Objective:\n{objective}\n\n\
        Work in the project directly, and launch Spark flows when a flow fits the work. Every run you launch reports back to this conversation: when runs complete, fail, are canceled, or wait on a human gate, you receive a new turn listing them. Never poll or sleep waiting for runs; end your turn instead. Launches count against the mission budget ({concurrent} concurrent runs, {total} runs in total); a refused launch names the limit it hit.\n\n\
        When the objective is met or cannot be met, close the mission: `spark mission close --project {project} --id {id} --status done|failed|canceled --reason <text>`. When only the user can decide something, ask and end your turn; their reply arrives as a new turn.\n\n\
        {control}\n\n\
        Mission ID: {id}\n\
        Conversation handle: {handle}\n\
        Project path: {project}",
        title = mission.fields.title,
        objective = mission.fields.description,
        concurrent = budget.concurrent_runs,
        total = budget.total_runs,
        project = mission.project_path,
        id = mission.id,
        control = crate::conversations::spark_control_surface(handle),
    )
}

/// A run's owning mission and the status delivery already posted for it.
#[derive(Clone)]
struct RunOwner {
    mission_id: String,
    project: String,
    paths: attractor_runtime::paths::RunRootPaths,
    status: Option<String>,
}

/// Ownership is fixed at launch, so unowned runs are answered from memory.
// ponytail: one entry per observed run for the process lifetime, like the publisher's cursors.
fn run_owners() -> &'static std::sync::Mutex<
    std::collections::HashMap<(std::path::PathBuf, String), Option<RunOwner>>,
> {
    static OWNERS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<(std::path::PathBuf, String), Option<RunOwner>>>,
    > = std::sync::OnceLock::new();
    OWNERS.get_or_init(Default::default)
}

fn over_budget(mission: &MissionRecord) -> Option<String> {
    let budget = mission.fields.budget;
    let in_flight = mission.runs.iter().filter(|run| run.in_flight()).count();
    if in_flight >= budget.concurrent_runs {
        Some(format!("concurrent_runs ({})", budget.concurrent_runs))
    } else if mission.runs.len() >= budget.total_runs {
        Some(format!("total_runs ({})", budget.total_runs))
    } else {
        None
    }
}
