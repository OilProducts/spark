//! Missions: project records of an intended outcome that own runs and react to
//! their events. One inbox per mission, processed sequentially under the
//! project mission lock; hooks react mechanically and reaction runs supply
//! judgment. Only reaction directives write `state`.
use crate::conversations::WorkspaceConversationService;
use crate::{WorkspaceError, WorkspaceResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use spark_common::settings::SparkSettings;
use spark_storage::{workspace_missions::MissionRepository, ProjectRegistry};

pub const DEFAULT_REACTION_FLOW: &str = "missions/react.yaml";
const TERMINAL_RUN_STATUSES: &[&str] = &["completed", "failed", "validation_error", "canceled"];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    #[default]
    Backlog,
    Planning,
    Ready,
    InProgress,
    Review,
    Done,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Launch {
        flow_name: String,
        label: String,
        #[serde(default)]
        context: Map<String, Value>,
    },
    Close {
        status: CloseStatus,
        #[serde(default)]
        reason: String,
    },
    SetState {
        markdown: String,
    },
    Ignore,
    Reason,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hook {
    pub on: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(rename = "do")]
    pub action: Action,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Budget {
    pub concurrent_runs: usize,
    pub total_runs: usize,
    pub reactions: usize,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            concurrent_runs: 4,
            total_runs: 25,
            reactions: 10,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MissionFields {
    pub title: String,
    pub description: String,
    pub stage: Stage,
    pub archived: bool,
    pub reaction_flow: String,
    pub hooks: Vec<Hook>,
    pub budget: Budget,
}
impl Default for MissionFields {
    fn default() -> Self {
        Self {
            title: String::new(),
            description: String::new(),
            stage: Stage::default(),
            archived: false,
            reaction_flow: DEFAULT_REACTION_FLOW.into(),
            hooks: vec![],
            budget: Budget::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionMutation {
    pub revision: Option<u64>,
    #[serde(default)]
    pub fields: Map<String, Value>,
    /// Optional YAML document of fields (hooks and budget are edited as text);
    /// explicit `fields` entries win.
    #[serde(default)]
    pub yaml: Option<String>,
    #[serde(default)]
    pub note: String,
    #[serde(default = "human")]
    pub actor: String,
}
fn human() -> String {
    "human".into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Work,
    Reaction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RosterEntry {
    pub run_id: String,
    pub label: String,
    pub role: Role,
    pub launched_at: String,
    pub launched_by_event: String,
    pub status: String,
}
impl RosterEntry {
    fn in_flight(&self) -> bool {
        !TERMINAL_RUN_STATUSES.contains(&self.status.as_str())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Substate {
    #[default]
    Idle,
    Running,
    Reasoning,
    Waiting,
    Attention,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Execution {
    pub substate: Substate,
    #[serde(default)]
    pub reason: String,
}

/// A blocker that outlives one processing pass: `waiting` on a budget (with the
/// refused actions to retry) or `attention` until a human message or Resume.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hold {
    pub substate: Substate,
    pub reason: String,
    #[serde(default)]
    pub actions: Vec<Action>,
    #[serde(default)]
    pub event: String,
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
    /// `human` for a human close or cancel, `mission` for a reaction or hook.
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

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionCloseRequest {
    #[serde(default)]
    pub status: Option<CloseStatus>,
    #[serde(default)]
    pub reason: String,
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
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub runs: Vec<RosterEntry>,
    #[serde(default)]
    pub execution: Execution,
    #[serde(default)]
    pub cursor: u64,
    /// Sequence of the newest inbox event, so appends change the record.
    #[serde(default)]
    pub event_seq: u64,
    #[serde(default)]
    pub closed: Option<Closed>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub hold: Option<Hold>,
    /// Events routed to `reason` that the next reaction will receive as one batch.
    #[serde(default)]
    pub pending_events: Vec<MissionEvent>,
    /// The batch handed to the in-flight reaction; restored if it fails.
    #[serde(default)]
    pub reaction_events: Vec<MissionEvent>,
}
impl MissionRecord {
    fn reaction_in_flight(&self) -> Option<&RosterEntry> {
        self.runs
            .iter()
            .find(|run| run.role == Role::Reaction && run.in_flight())
    }
    fn record_activity(&mut self, actor: &str, note: &str, before: Value) {
        let now = now();
        self.updated_at = now.clone();
        self.revision += 1;
        self.activity.push(json!({"revision": self.revision, "at": now, "actor": actor, "note": note, "before": before, "after": self.fields}));
    }
    fn set_stage(&mut self, stage: Stage, note: &str) {
        let before = serde_json::to_value(&self.fields).unwrap();
        self.fields.stage = stage;
        self.record_activity("mission", note, before);
    }
    fn note(&mut self, actor: &str, note: &str) {
        let before = serde_json::to_value(&self.fields).unwrap();
        self.record_activity(actor, note, before);
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
fn validate_fields(fields: &MissionFields) -> WorkspaceResult<()> {
    if fields.title.trim().is_empty() {
        return Err(invalid("Mission title is required"));
    }
    if fields.reaction_flow.trim().is_empty() {
        return Err(invalid("Mission reaction_flow is required"));
    }
    for hook in &fields.hooks {
        if hook.on.trim().is_empty() {
            return Err(invalid("Hook `on` is required"));
        }
        match &hook.action {
            Action::SetState { .. } => {
                return Err(invalid("Hooks cannot set state; only reactions write it"))
            }
            Action::Launch {
                flow_name, label, ..
            } if flow_name.trim().is_empty() || label.trim().is_empty() => {
                return Err(invalid("Hook launch requires flow_name and label"))
            }
            _ => {}
        }
    }
    Ok(())
}
fn run_event_status(kind: &str, payload: &Value) -> Option<String> {
    let status = payload.get("status").and_then(Value::as_str);
    match kind {
        "run.completed" => Some("completed".into()),
        "run.failed" => Some(status.unwrap_or("failed").into()),
        "run.canceled" => Some("canceled".into()),
        "run.waiting" => Some("waiting".into()),
        _ => None,
    }
}

pub struct WorkspaceMissionService {
    settings: SparkSettings,
    run_event_observer: Option<attractor_runtime::RunEventObserver>,
}
impl WorkspaceMissionService {
    pub fn new(settings: SparkSettings) -> Self {
        Self {
            settings,
            run_event_observer: None,
        }
    }
    pub fn with_run_event_observer(
        mut self,
        observer: attractor_runtime::RunEventObserver,
    ) -> Self {
        self.run_event_observer = Some(observer);
        self
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
        let mission: MissionRecord = decode(value)?;
        if mission.project_id != scope.project_id {
            return Err(invalid("Mission belongs to another project"));
        }
        Ok(mission)
    }
    pub fn get(&self, project: &str, id: &str) -> WorkspaceResult<MissionRecord> {
        let scope = self.scope(project)?;
        Self::load(&MissionRepository::new(&scope.root), &scope, id)
    }
    pub fn list(&self, project: &str) -> WorkspaceResult<Vec<MissionRecord>> {
        let scope = self.scope(project)?;
        let mut missions: Vec<MissionRecord> = MissionRepository::new(&scope.root)
            .list()?
            .into_iter()
            .map(decode)
            .collect::<WorkspaceResult<_>>()?;
        if missions.iter().any(|m| m.project_id != scope.project_id) {
            return Err(invalid("Mission belongs to another project"));
        }
        missions.sort_by(|a, b| (&a.created_at, &a.id).cmp(&(&b.created_at, &b.id)));
        Ok(missions)
    }
    pub fn board(&self, project: &str) -> WorkspaceResult<Value> {
        Ok(json!({"missions": self.list(project)?}))
    }
    pub fn events(
        &self,
        project: &str,
        id: &str,
        after: u64,
    ) -> WorkspaceResult<Vec<MissionEvent>> {
        let scope = self.scope(project)?;
        let repo = MissionRepository::new(&scope.root);
        Self::load(&repo, &scope, id)?;
        Ok(Self::read_events(&repo, id)?
            .into_iter()
            .filter(|event| event.seq > after)
            .collect())
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
        self.mutate(project, id, mutation, false)?;
        // Raising a budget or editing hooks resumes processing.
        self.process(project, id)
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
        let mut changes = match mutation.yaml.as_deref() {
            Some(text) => match serde_yaml::from_str::<Value>(text)
                .map_err(|e| invalid(format!("Invalid mission YAML: {e}")))?
            {
                Value::Object(map) => map,
                Value::Null => Map::new(),
                _ => return Err(invalid("Mission YAML must be a mapping of fields")),
            },
            None => Map::new(),
        };
        changes.extend(mutation.fields.clone());
        let value = MissionRepository::new(&scope.root).transact::<WorkspaceError>(id, |previous| {
            let now = now();
            let mut mission = match previous {
                Some(value) if !create => decode::<MissionRecord>(value)?,
                None if create => decode::<MissionRecord>(json!({"id": id, "project_id": scope.project_id, "project_path": scope.project_path, "created_at": now, "updated_at": now, "revision": 0, "fields": {}, "activity": []}))?,
                _ => return Err(WorkspaceError::NotFound("Unknown project mission".into())),
            };
            if !create && mutation.revision != Some(mission.revision) { return Err(WorkspaceError::Conflict(format!("Mission changed; reload revision {} and reconcile your edits", mission.revision))); }
            if mission.project_id != scope.project_id { return Err(invalid("Mission belongs to another project")); }
            let before = serde_json::to_value(&mission.fields).unwrap();
            let mut fields = before.clone();
            for (key, value) in &changes { fields[key] = value.clone(); }
            let fields: MissionFields = decode(fields)?;
            validate_fields(&fields)?;
            mission.fields = fields;
            mission.record_activity(&mutation.actor, &mutation.note, before);
            Ok(serde_json::to_value(mission).unwrap())
        })?;
        decode(value)
    }

    /// Runs `work` on the mission under the project lock, then processes the
    /// inbox and persists the record.
    fn with_mission(
        &self,
        project: &str,
        id: &str,
        work: impl FnOnce(&MissionRepository, &mut MissionRecord) -> WorkspaceResult<()>,
    ) -> WorkspaceResult<MissionRecord> {
        let scope = self.scope(project)?;
        MissionRepository::new(&scope.root).locked(|repo| {
            let mut mission = Self::load(repo, &scope, id)?;
            let before = serde_json::to_value(&mission).unwrap();
            work(repo, &mut mission)?;
            self.process_locked(repo, &mut mission)?;
            let after = serde_json::to_value(&mission).unwrap();
            if before != after {
                repo.write(id, &after)?;
            }
            Ok(mission)
        })
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
        let kind = post.kind.trim();
        if kind.is_empty()
            || !kind
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(&c))
        {
            return Err(invalid(
                "Event kind must be a non-empty dotted lowercase name",
            ));
        }
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
            kind: kind.into(),
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
    pub fn post_event(
        &self,
        project: &str,
        id: &str,
        post: MissionEventPost,
    ) -> WorkspaceResult<MissionRecord> {
        self.with_mission(project, id, |repo, mission| {
            Self::append(repo, mission, post, "human").map(|_| ())
        })
    }

    pub fn start(&self, project: &str, id: &str) -> WorkspaceResult<MissionRecord> {
        self.with_mission(project, id, |repo, mission| {
            if mission.started_at.is_some() {
                return Err(WorkspaceError::Conflict("Mission already started".into()));
            }
            mission.started_at = Some(now());
            mission.set_stage(Stage::InProgress, "Started mission");
            let post = MissionEventPost {
                id: Some(format!("{}:started", mission.id)),
                kind: "mission.started".into(),
                source: Some("human".into()),
                payload: json!({"objective": mission.fields.description}),
            };
            Self::append(repo, mission, post, "human").map(|_| ())
        })
    }
    pub fn pause(&self, project: &str, id: &str) -> WorkspaceResult<MissionRecord> {
        self.with_mission(project, id, |_, mission| {
            if !mission.paused {
                mission.paused = true;
                mission.note("human", "Paused mission");
            }
            Ok(())
        })
    }
    pub fn resume(&self, project: &str, id: &str) -> WorkspaceResult<MissionRecord> {
        self.with_mission(project, id, |_, mission| {
            mission.paused = false;
            mission.note("human", "Resumed mission");
            self.clear_attention(mission);
            Ok(())
        })
    }
    pub fn cancel(&self, project: &str, id: &str) -> WorkspaceResult<MissionRecord> {
        self.with_mission(project, id, |_, mission| {
            let runtime = attractor_api::AttractorApiService::new(self.settings.clone());
            for run in mission.runs.iter().filter(|run| run.in_flight()) {
                runtime.cancel_pipeline_route(&run.run_id);
            }
            close(
                mission,
                CloseStatus::Canceled,
                "Canceled by human".into(),
                "human",
            );
            Ok(())
        })
    }
    pub fn close(
        &self,
        project: &str,
        id: &str,
        request: MissionCloseRequest,
    ) -> WorkspaceResult<MissionRecord> {
        self.with_mission(project, id, |_, mission| {
            let reason = if request.reason.trim().is_empty() {
                "Closed by human".to_string()
            } else {
                request.reason
            };
            close(
                mission,
                request.status.unwrap_or(CloseStatus::Done),
                reason,
                "human",
            );
            Ok(())
        })
    }

    /// Turns a mission-owned run's state into inbox events: a changed
    /// `context.mission.signal` becomes `run.signal`, and a terminal or waiting
    /// status becomes one event keyed by run id plus status.
    pub fn deliver_run_events(&self, run_id: &str) -> WorkspaceResult<Option<MissionRecord>> {
        self.deliver(run_id, false)
    }

    /// `recovering` delivers terminal runs whose result is still pending: after a
    /// restart no executor remains to replace it.
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
                        signal: None,
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
        let signal = store
            .read_checkpoint(&owner.paths)
            .map_err(internal)?
            .and_then(|checkpoint| checkpoint.context.get("context.mission.signal").cloned())
            .filter(|value| !value.is_null() && owner.signal.as_ref() != Some(value));
        let status = attractor_runtime::normalize_run_status(record.status.trim());
        let kind = match status.as_str() {
            "completed" => Some("run.completed"),
            "failed" | "validation_error" => Some("run.failed"),
            "canceled" => Some("run.canceled"),
            // ponytail: roster keeps `waiting` until terminal and a second gate on the
            // same run dedupes by event id; track resumes if gate-heavy flows need it.
            "waiting" => Some("run.waiting"),
            _ => None,
        }
        .filter(|_| owner.status.as_ref() != Some(&status));
        let result_pending = !recovering
            && kind.is_some_and(|kind| kind != "run.waiting")
            && store
                .read_result(&owner.paths)
                .map_err(internal)?
                .is_some_and(|result| result.state == "pending");
        let kind = kind.filter(|_| !result_pending);
        if signal.is_none() && kind.is_none() {
            return Ok(None);
        }
        let mut changed = false;
        let mission = self.with_mission(&owner.project, &owner.mission_id, |repo, mission| {
            if let Some(signal) = &signal {
                let signals: Vec<_> = Self::read_events(repo, &mission.id)?
                    .into_iter()
                    .filter(|event| event.kind == "run.signal" && event.source == run_id)
                    .collect();
                if signals.last().map(|event| &event.payload) != Some(signal) {
                    changed |= Self::append(
                        repo,
                        mission,
                        MissionEventPost {
                            id: Some(format!("{run_id}:signal:{}", signals.len() + 1)),
                            kind: "run.signal".into(),
                            source: Some(run_id.into()),
                            payload: signal.clone(),
                        },
                        run_id,
                    )?;
                }
            }
            if let Some(kind) = kind {
                changed |= Self::append(
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
            }
            Ok(())
        })?;
        if let Some(Some(delivered)) = run_owners().lock().unwrap().get_mut(&key) {
            delivered.signal = signal.or(delivered.signal.take());
            if kind.is_some() {
                delivered.status = Some(status);
            }
        }
        Ok(changed.then_some(mission))
    }

    /// Restart recovery: post missing terminal events for owned runs and resume
    /// inbox processing for every open mission.
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

    fn process_locked(
        &self,
        repo: &MissionRepository,
        mission: &mut MissionRecord,
    ) -> WorkspaceResult<()> {
        let events: Vec<_> = Self::read_events(repo, &mission.id)?
            .into_iter()
            .filter(|event| event.seq > mission.cursor)
            .collect();
        // Roster status tracks delivery even while processing is held.
        for event in &events {
            if let Some(status) = run_event_status(&event.kind, &event.payload) {
                if let Some(run) = mission.runs.iter_mut().find(|r| r.run_id == event.source) {
                    if run.in_flight() {
                        run.status = status;
                    }
                }
            }
        }
        if mission.started_at.is_some() && mission.closed.is_none() && !mission.paused {
            self.reduce(mission, events);
        }
        settle(mission);
        Ok(())
    }

    fn reduce(&self, mission: &mut MissionRecord, events: Vec<MissionEvent>) {
        if let Some(hold) = mission
            .hold
            .take_if(|hold| hold.substate == Substate::Waiting)
        {
            self.apply(mission, hold.actions, &hold.event);
            if mission.hold.is_some() {
                return;
            }
        }
        for event in events {
            if mission.closed.is_some()
                || mission
                    .hold
                    .as_ref()
                    .is_some_and(|hold| hold.substate == Substate::Waiting)
            {
                break;
            }
            mission.cursor = event.seq;
            self.handle(mission, event);
        }
        self.maybe_react(mission);
    }

    fn handle(&self, mission: &mut MissionRecord, event: MissionEvent) {
        let run = mission
            .runs
            .iter()
            .find(|run| run.run_id == event.source)
            .cloned();
        if let Some(run) = run.as_ref().filter(|run| run.role == Role::Reaction) {
            match event.kind.as_str() {
                "run.completed" => match self.read_directive(&run.run_id) {
                    Ok(actions) => {
                        mission.reaction_events.clear();
                        self.apply(mission, actions, &event.id);
                    }
                    Err(reason) => reaction_failed(mission, reason),
                },
                "run.failed" | "run.canceled" => reaction_failed(
                    mission,
                    format!("Reaction run {} ended as {}", run.run_id, event.kind),
                ),
                _ => {}
            }
            return;
        }
        if event.kind == "human.message" {
            self.clear_attention(mission);
        }
        let status = event.payload.get("status").and_then(Value::as_str);
        let action = mission
            .fields
            .hooks
            .iter()
            .find(|hook| {
                hook.on == event.kind
                    && hook
                        .label
                        .as_ref()
                        .is_none_or(|label| run.as_ref().is_some_and(|run| &run.label == label))
                    && hook
                        .status
                        .as_deref()
                        .is_none_or(|expected| Some(expected) == status)
            })
            .map_or(Action::Reason, |hook| hook.action.clone());
        match action {
            Action::Ignore => {}
            Action::Reason => mission.pending_events.push(event),
            action => self.apply(mission, vec![action], &event.id),
        }
    }

    fn read_directive(&self, run_id: &str) -> Result<Vec<Action>, String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Directive {
            actions: Vec<Action>,
        }
        let store = attractor_runtime::RunStore::for_settings(&self.settings);
        let value = store
            .find_run_root(run_id)
            .ok()
            .flatten()
            .and_then(|paths| store.read_checkpoint(&paths).ok().flatten())
            .and_then(|checkpoint| checkpoint.context.get("context.mission.directive").cloned())
            .ok_or_else(|| format!("Reaction run {run_id} wrote no context.mission.directive"))?;
        // Agents may write the directive as a JSON string.
        let value = match value {
            Value::String(text) => serde_json::from_str(&text).unwrap_or(Value::String(text)),
            value => value,
        };
        let directive: Directive = serde_json::from_value(value)
            .map_err(|e| format!("Reaction run {run_id} wrote an invalid directive: {e}"))?;
        if directive
            .actions
            .iter()
            .any(|action| matches!(action, Action::Ignore | Action::Reason))
        {
            return Err(format!(
                "Reaction run {run_id} directive may only launch, set_state, or close"
            ));
        }
        Ok(directive.actions)
    }

    fn apply(&self, mission: &mut MissionRecord, actions: Vec<Action>, event: &str) {
        for (index, action) in actions.iter().enumerate() {
            if mission.closed.is_some() {
                return;
            }
            match action {
                Action::Launch {
                    flow_name,
                    label,
                    context,
                } => {
                    // Queue behind actions an earlier hold already deferred.
                    if mission
                        .hold
                        .as_ref()
                        .is_some_and(|hold| !hold.actions.is_empty())
                    {
                        hold(
                            mission,
                            Substate::Waiting,
                            String::new(),
                            actions[index..].to_vec(),
                            event,
                        );
                        return;
                    }
                    if let Some(limit) = over_budget(mission, Role::Work) {
                        hold(
                            mission,
                            Substate::Waiting,
                            format!("Budget limit reached: {limit}"),
                            actions[index..].to_vec(),
                            event,
                        );
                        return;
                    }
                    if let Err(error) = self.launch(
                        mission,
                        flow_name,
                        label,
                        Role::Work,
                        context.clone(),
                        event,
                    ) {
                        // The rest of the directive runs once a human clears this.
                        hold(
                            mission,
                            Substate::Attention,
                            format!("Launch of {label} failed: {error}"),
                            actions[index + 1..].to_vec(),
                            event,
                        );
                        return;
                    }
                }
                Action::SetState { markdown } => mission.state = markdown.clone(),
                Action::Close { status, reason } => {
                    close(mission, *status, reason.clone(), "mission")
                }
                Action::Ignore | Action::Reason => {}
            }
        }
    }

    /// A human Resume or message clears attention and applies the actions it deferred.
    fn clear_attention(&self, mission: &mut MissionRecord) {
        if let Some(held) = mission
            .hold
            .take_if(|hold| hold.substate == Substate::Attention)
        {
            self.apply(mission, held.actions, &held.event);
        }
    }

    fn maybe_react(&self, mission: &mut MissionRecord) {
        if mission.closed.is_some()
            || mission.hold.is_some()
            || mission.pending_events.is_empty()
            || mission.reaction_in_flight().is_some()
        {
            return;
        }
        let event = mission.pending_events.last().unwrap().id.clone();
        if let Some(limit) = over_budget(mission, Role::Reaction) {
            hold(
                mission,
                Substate::Waiting,
                format!("Budget limit reached: {limit}"),
                vec![],
                &event,
            );
            return;
        }
        let context = Map::from_iter([(
            "context.mission".to_string(),
            json!({
                "id": mission.id,
                "title": mission.fields.title,
                "objective": mission.fields.description,
                "state": mission.state,
                "runs": mission.runs,
                "events": mission.pending_events,
            }),
        )]);
        let flow = mission.fields.reaction_flow.clone();
        match self.launch(mission, &flow, "reaction", Role::Reaction, context, &event) {
            Ok(()) => mission.reaction_events = std::mem::take(&mut mission.pending_events),
            Err(error) => hold(
                mission,
                Substate::Attention,
                format!("Reaction launch failed: {error}"),
                vec![],
                &event,
            ),
        }
    }

    fn launch(
        &self,
        mission: &mut MissionRecord,
        flow_name: &str,
        label: &str,
        role: Role,
        mut context: Map<String, Value>,
        event: &str,
    ) -> Result<(), String> {
        context.insert(
            "context.spark_mission".into(),
            json!({"mission_id": mission.id, "label": label, "role": role}),
        );
        let mut service = WorkspaceConversationService::new(self.settings.clone());
        if let Some(observer) = &self.run_event_observer {
            service = service.with_run_event_observer(observer.clone());
        }
        let run_id = service.launch_workspace_flow(
            &mission.project_path,
            flow_name,
            &json!({
                "flow_name": flow_name,
                "summary": format!("Mission {} launched {label}.", mission.id),
                "project_path": mission.project_path,
                "launch_context": context,
            }),
        )?;
        mission.runs.push(RosterEntry {
            run_id,
            label: label.into(),
            role,
            launched_at: now(),
            launched_by_event: event.into(),
            status: "running".into(),
        });
        Ok(())
    }
}

/// A run's owning mission and what delivery already posted for it.
#[derive(Clone)]
struct RunOwner {
    mission_id: String,
    project: String,
    paths: attractor_runtime::paths::RunRootPaths,
    signal: Option<Value>,
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

fn over_budget(mission: &MissionRecord, role: Role) -> Option<String> {
    let budget = mission.fields.budget;
    let in_flight = mission.runs.iter().filter(|run| run.in_flight()).count();
    let reactions = mission
        .runs
        .iter()
        .filter(|run| run.role == Role::Reaction)
        .count();
    if in_flight >= budget.concurrent_runs {
        Some(format!("concurrent_runs ({})", budget.concurrent_runs))
    } else if mission.runs.len() >= budget.total_runs {
        Some(format!("total_runs ({})", budget.total_runs))
    } else if role == Role::Reaction && reactions >= budget.reactions {
        Some(format!("reactions ({})", budget.reactions))
    } else {
        None
    }
}

fn reaction_failed(mission: &mut MissionRecord, reason: String) {
    let mut batch = std::mem::take(&mut mission.reaction_events);
    batch.append(&mut mission.pending_events);
    mission.pending_events = batch;
    hold(mission, Substate::Attention, reason, vec![], "");
}

/// Sets or extends the mission hold. Attention outranks a budget wait and keeps
/// its reason until a human clears it; deferred actions accumulate in order.
fn hold(
    mission: &mut MissionRecord,
    substate: Substate,
    reason: String,
    actions: Vec<Action>,
    event: &str,
) {
    match &mut mission.hold {
        Some(existing) => {
            if substate == Substate::Attention || existing.substate != Substate::Attention {
                existing.substate = substate;
                if !reason.is_empty() {
                    existing.reason = reason;
                }
            }
            existing.actions.extend(actions);
        }
        None => {
            mission.hold = Some(Hold {
                substate,
                reason,
                actions,
                event: event.into(),
            })
        }
    }
}

fn close(mission: &mut MissionRecord, status: CloseStatus, reason: String, actor: &str) {
    if mission.closed.is_some() {
        return;
    }
    mission.closed = Some(Closed {
        status,
        reason: reason.clone(),
        at: now(),
        actor: actor.into(),
    });
    mission.hold = None;
    let note = format!("Closed as {}: {reason}", json!(status).as_str().unwrap());
    if status == CloseStatus::Done && mission.fields.stage != Stage::Review {
        let before = serde_json::to_value(&mission.fields).unwrap();
        mission.fields.stage = Stage::Review;
        mission.record_activity(actor, &note, before);
    } else {
        mission.note(actor, &note);
    }
}

fn settle(mission: &mut MissionRecord) {
    let in_flight = mission.runs.iter().filter(|run| run.in_flight()).count();
    mission.execution = if let Some(closed) = &mission.closed {
        Execution {
            // Only failures a reaction or hook closed on need a human.
            substate: if closed.status == CloseStatus::Done || closed.actor == "human" {
                Substate::Idle
            } else {
                Substate::Attention
            },
            reason: closed.reason.clone(),
        }
    } else if let Some(hold) = &mission.hold {
        Execution {
            substate: hold.substate,
            reason: hold.reason.clone(),
        }
    } else if let Some(reaction) = mission.reaction_in_flight() {
        if reaction.status == "waiting" {
            Execution {
                substate: Substate::Waiting,
                reason: "Reaction is waiting on a human decision".into(),
            }
        } else {
            Execution {
                substate: Substate::Reasoning,
                reason: String::new(),
            }
        }
    } else if in_flight > 0 {
        Execution {
            substate: Substate::Running,
            reason: format!("{in_flight} run(s) in flight"),
        }
    } else {
        Execution::default()
    };
    if mission.paused {
        mission.execution.reason = if mission.execution.reason.is_empty() {
            "Paused".into()
        } else {
            format!("Paused; {}", mission.execution.reason)
        };
    }
}
