use crate::{WorkspaceError, WorkspaceResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
use spark_storage::{workspace_tasks::TaskRepository, ConversationRepository, ProjectRegistry};
use std::{collections::BTreeSet, path::Path};

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskReference {
    pub task_id: String,
    pub stage: Option<Stage>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunLink {
    pub run_id: String,
    pub stage: Option<Stage>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TaskFields {
    pub title: String,
    pub description: String,
    pub acceptance_criteria: String,
    pub stage: Stage,
    pub priority: u8,
    pub next_action: String,
    pub blocked: String,
    pub needs_input: String,
    pub archived: bool,
    pub conversations: Vec<String>,
    pub artifacts: Vec<String>,
    pub runs: Vec<RunLink>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskMutation {
    pub revision: Option<u64>,
    #[serde(default)]
    pub fields: serde_json::Map<String, Value>,
    #[serde(default)]
    pub note: String,
    #[serde(default = "human")]
    pub actor: String,
    pub conversation_id: Option<String>,
}
fn human() -> String {
    "human".into()
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: String,
    pub project_id: String,
    pub project_path: String,
    pub created_at: String,
    pub updated_at: String,
    pub revision: u64,
    pub fields: TaskFields,
    pub activity: Vec<Value>,
}
fn invalid(message: impl Into<String>) -> WorkspaceError {
    WorkspaceError::Validation(message.into())
}
fn decode<T: serde::de::DeserializeOwned>(value: Value) -> WorkspaceResult<T> {
    serde_json::from_value(value).map_err(|e| invalid(e.to_string()))
}

pub struct WorkspaceTaskService {
    settings: SparkSettings,
}
impl WorkspaceTaskService {
    pub fn new(settings: SparkSettings) -> Self {
        Self { settings }
    }
    fn scope(&self, project_path: &str) -> WorkspaceResult<spark_storage::ProjectPaths> {
        if project_path.trim().is_empty() {
            return Err(invalid("Project path is required"));
        }
        Ok(ProjectRegistry::new(self.settings.data_dir.clone())
            .ensure_project_paths(project_path)?)
    }
    fn runtime(&self) -> attractor_api::AttractorApiService {
        attractor_api::AttractorApiService::new(self.settings.clone())
    }
    fn project_runs(&self, project: &str) -> WorkspaceResult<Vec<Value>> {
        let response = self.runtime().list_runs();
        if response.status_code != 200 {
            return Err(WorkspaceError::Internal(
                "Unable to read project runs".into(),
            ));
        }
        Ok(response.body["runs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|run| {
                let path = run["project_path"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .or_else(|| run["working_directory"].as_str());
                path.and_then(|p| {
                    spark_common::project::normalize_project_path(p)
                        .ok()
                        .flatten()
                })
                .as_deref()
                    == Some(Path::new(project))
            })
            .cloned()
            .collect())
    }
    pub fn get(&self, project: &str, id: &str) -> WorkspaceResult<TaskRecord> {
        self.reconcile_runs(project)?;
        let scope = self.scope(project)?;
        let value = TaskRepository::new(&scope.root)
            .read(id)?
            .ok_or_else(|| WorkspaceError::NotFound("Unknown project task".into()))?;
        let task: TaskRecord = decode(value)?;
        if task.project_id != scope.project_id {
            return Err(invalid("Task belongs to another project"));
        }
        Ok(task)
    }
    pub fn list(&self, project: &str) -> WorkspaceResult<Vec<TaskRecord>> {
        self.reconcile_runs(project)?;
        let scope = self.scope(project)?;
        let mut tasks: Vec<TaskRecord> = TaskRepository::new(&scope.root)
            .list()?
            .into_iter()
            .map(decode)
            .collect::<WorkspaceResult<_>>()?;
        tasks.sort_by(|a, b| {
            (a.fields.priority, &a.created_at, &a.id).cmp(&(
                b.fields.priority,
                &b.created_at,
                &b.id,
            ))
        });
        Ok(tasks)
    }
    pub fn board(&self, project: &str) -> WorkspaceResult<Value> {
        let tasks = self.list(project)?;
        let scope = self.scope(project)?;
        let runs = self.project_runs(&scope.project_path)?;
        let mut attention = vec![];
        for task in &tasks {
            let mut ids = BTreeSet::new();
            for link in &task.fields.runs {
                // The existing question service includes descendants and owns answer storage.
                let questions = self.runtime().list_pipeline_questions(&link.run_id);
                if questions.status_code != 200 {
                    return Err(WorkspaceError::Internal(
                        "Unable to read linked run questions".into(),
                    ));
                }
                for question in questions.body["questions"].as_array().into_iter().flatten() {
                    if let Some(id) = question["run_id"].as_str() {
                        ids.insert(id.to_string());
                    }
                }
            }
            for id in ids {
                attention.push(json!({"task_id": task.id, "run_id": id}));
            }
        }
        Ok(json!({"tasks": tasks, "runs": runs, "attention": attention}))
    }
    pub fn create(&self, project: &str, mutation: TaskMutation) -> WorkspaceResult<TaskRecord> {
        if mutation.revision.is_some() {
            return Err(invalid("Creation must omit revision"));
        }
        self.mutate(
            project,
            &format!("task-{}", uuid::Uuid::new_v4()),
            mutation,
            true,
        )
    }
    pub fn update(
        &self,
        project: &str,
        id: &str,
        mutation: TaskMutation,
    ) -> WorkspaceResult<TaskRecord> {
        if mutation.revision.is_none() {
            return Err(invalid("Update requires revision"));
        }
        self.mutate(project, id, mutation, false)
    }
    /// Replay durable launch metadata after a crash, without duplicating links or history.
    pub fn reconcile_runs(&self, project: &str) -> WorkspaceResult<()> {
        let scope = self.scope(project)?;
        let runs = self.project_runs(&scope.project_path)?;
        let store = attractor_runtime::RunStore::for_settings(&self.settings);
        for run in &runs {
            let Some(id) = run["run_id"].as_str() else {
                continue;
            };
            let meta = store
                .read_run_meta(id)
                .map_err(|e| WorkspaceError::Internal(e.to_string()))?;
            let context = meta
                .and_then(|m| m.record)
                .and_then(|r| r.launch_context)
                .unwrap_or_default();
            let Some(reference) = context.get("context.spark_task").cloned() else {
                continue;
            };
            let conversation_id = context
                .get("context.spark_task_conversation")
                .and_then(Value::as_str);
            // Other launch-context callers can supply arbitrary values. Only replay
            // references to an existing task in this project.
            let Ok(reference) = decode::<TaskReference>(reference) else {
                continue;
            };
            let repo = TaskRepository::new(&scope.root);
            match repo.read(&reference.task_id) {
                Ok(Some(_)) => {}
                Ok(None) | Err(spark_storage::StorageError::InvalidRepositoryPath { .. }) => {
                    continue
                }
                Err(error) => return Err(error.into()),
            }
            repo.transact::<WorkspaceError>(&reference.task_id, |value| {
                let mut task: TaskRecord = decode(value.ok_or_else(|| invalid("Run references an unknown task"))?)?;
                if task.activity.iter().any(|a| a["associated_run_id"] == id) { return Ok(serde_json::to_value(task).unwrap()); }
                if !task.fields.runs.iter().any(|r| r.run_id == id) { task.fields.runs.push(RunLink { run_id: id.into(), stage: reference.stage }); }
                if let Some(id) = conversation_id {
                    self.validate_conversation(&scope.project_path, id)?;
                    if !task.fields.conversations.iter().any(|c| c == id) { task.fields.conversations.push(id.into()); }
                }
                task.revision += 1;
                task.updated_at = time::OffsetDateTime::now_utc().to_string();
                task.activity.push(json!({"revision": task.revision, "at": task.updated_at, "actor": "assistant", "note": "Associated approved run", "associated_run_id": id, "stage": reference.stage, "conversation_id": conversation_id}));
                Ok(serde_json::to_value(task).unwrap())
            })?;
        }
        Ok(())
    }
    fn validate_conversation(&self, project: &str, id: &str) -> WorkspaceResult<()> {
        if id.is_empty()
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(invalid("Invalid conversation ID"));
        }
        let snapshot = ConversationRepository::new(self.settings.data_dir.clone())
            .read_snapshot(id, Some(project))?;
        if !snapshot.is_some_and(|s| {
            s["conversation_id"] == id
                && s["project_path"].as_str().and_then(|p| {
                    spark_common::project::normalize_project_path(p)
                        .ok()
                        .flatten()
                }) == Some(Path::new(project).to_path_buf())
        }) {
            return Err(invalid("Unknown project conversation"));
        }
        Ok(())
    }
    fn validate(&self, project: &str, fields: &TaskFields) -> WorkspaceResult<()> {
        if fields.title.trim().is_empty() {
            return Err(invalid("Task title is required"));
        }
        if fields.priority > 3 {
            return Err(invalid(
                "Priority must be 0 (urgent), 1 (high), 2 (normal), or 3 (low)",
            ));
        }
        if fields.conversations.iter().collect::<BTreeSet<_>>().len() != fields.conversations.len()
            || fields.artifacts.iter().collect::<BTreeSet<_>>().len() != fields.artifacts.len()
            || fields
                .runs
                .iter()
                .map(|r| &r.run_id)
                .collect::<BTreeSet<_>>()
                .len()
                != fields.runs.len()
        {
            return Err(invalid("Duplicate task links are not allowed"));
        }
        for id in &fields.conversations {
            self.validate_conversation(project, id)?;
        }
        let root = Path::new(project)
            .canonicalize()
            .map_err(|_| invalid("Project directory does not exist"))?;
        for artifact in &fields.artifacts {
            let path = Path::new(artifact);
            if path.is_absolute()
                || path
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                return Err(invalid("Artifact must be a relative project path"));
            }
            let resolved = root
                .join(path)
                .canonicalize()
                .map_err(|_| invalid("Artifact does not exist"))?;
            if !resolved.starts_with(&root) || !resolved.is_file() {
                return Err(invalid("Artifact must reference a project-owned file"));
            }
        }
        if !fields.runs.is_empty() {
            let runs = self.project_runs(project)?;
            for link in &fields.runs {
                if !runs
                    .iter()
                    .any(|r| r["run_id"].as_str() == Some(&link.run_id))
                {
                    return Err(invalid("Unknown project run"));
                }
            }
        }
        Ok(())
    }
    fn mutate(
        &self,
        project: &str,
        id: &str,
        mutation: TaskMutation,
        create: bool,
    ) -> WorkspaceResult<TaskRecord> {
        let scope = self.scope(project)?;
        if mutation.actor != "human" && mutation.actor != "assistant" {
            return Err(invalid("Actor must be human or assistant"));
        }
        if let Some(id) = &mutation.conversation_id {
            self.validate_conversation(&scope.project_path, id)?;
        }
        let value = TaskRepository::new(&scope.root).transact::<WorkspaceError>(id, |previous| {
            let now = time::OffsetDateTime::now_utc().to_string();
            let mut task = match previous {
                Some(value) if !create => decode::<TaskRecord>(value)?,
                None if create => TaskRecord { id: id.into(), project_id: scope.project_id.clone(), project_path: scope.project_path.clone(), created_at: now.clone(), updated_at: now.clone(), revision: 0, fields: TaskFields { priority: 2, ..Default::default() }, activity: vec![] },
                _ => return Err(WorkspaceError::NotFound("Unknown project task".into())),
            };
            if !create && mutation.revision != Some(task.revision) { return Err(WorkspaceError::Conflict(format!("Task changed; reload revision {} and reconcile your edits", task.revision))); }
            let before = serde_json::to_value(&task.fields).unwrap();
            let mut fields = before.clone();
            for (key, value) in &mutation.fields { fields[key] = value.clone(); }
            let fields: TaskFields = decode(fields)?;
            if fields.stage == Stage::Done && (create || task.fields.stage != Stage::Done) && mutation.note.trim().is_empty() { return Err(invalid("Moving to Done requires a completion note")); }
            self.validate(&scope.project_path, &fields)?;
            task.fields = fields;
            task.updated_at = now.clone(); task.revision += 1;
            task.activity.push(json!({"revision": task.revision, "at": now, "actor": mutation.actor, "conversation_id": mutation.conversation_id, "note": mutation.note, "before": before, "after": task.fields}));
            Ok(serde_json::to_value(task).unwrap())
        })?;
        decode(value)
    }
}
