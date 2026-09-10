use crate::{WorkspaceError, WorkspaceResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
use spark_storage::{workspace_tasks::TaskRepository, ProjectRegistry};

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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TaskFields {
    pub title: String,
    pub description: String,
    pub stage: Stage,
    pub archived: bool,
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
    pub fn get(&self, project: &str, id: &str) -> WorkspaceResult<TaskRecord> {
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
        let scope = self.scope(project)?;
        let mut tasks: Vec<TaskRecord> = TaskRepository::new(&scope.root)
            .list()?
            .into_iter()
            .map(decode)
            .collect::<WorkspaceResult<_>>()?;
        if tasks.iter().any(|task| task.project_id != scope.project_id) {
            return Err(invalid("Task belongs to another project"));
        }
        tasks.sort_by(|a, b| (&a.created_at, &a.id).cmp(&(&b.created_at, &b.id)));
        Ok(tasks)
    }
    pub fn board(&self, project: &str) -> WorkspaceResult<Value> {
        let tasks = self.list(project)?;
        Ok(json!({"tasks": tasks}))
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
        let value = TaskRepository::new(&scope.root).transact::<WorkspaceError>(id, |previous| {
            let now = time::OffsetDateTime::now_utc().to_string();
            let mut task = match previous {
                Some(value) if !create => decode::<TaskRecord>(value)?,
                None if create => TaskRecord { id: id.into(), project_id: scope.project_id.clone(), project_path: scope.project_path.clone(), created_at: now.clone(), updated_at: now.clone(), revision: 0, fields: TaskFields::default(), activity: vec![] },
                _ => return Err(WorkspaceError::NotFound("Unknown project task".into())),
            };
            if !create && mutation.revision != Some(task.revision) { return Err(WorkspaceError::Conflict(format!("Task changed; reload revision {} and reconcile your edits", task.revision))); }
            if task.project_id != scope.project_id { return Err(invalid("Task belongs to another project")); }
            let before = serde_json::to_value(&task.fields).unwrap();
            let mut fields = before.clone();
            for (key, value) in &mutation.fields { fields[key] = value.clone(); }
            let fields: TaskFields = decode(fields)?;
            if fields.title.trim().is_empty() { return Err(invalid("Task title is required")); }
            task.fields = fields;
            task.updated_at = now.clone(); task.revision += 1;
            task.activity.push(json!({"revision": task.revision, "at": now, "actor": mutation.actor, "note": mutation.note, "before": before, "after": task.fields}));
            Ok(serde_json::to_value(task).unwrap())
        })?;
        decode(value)
    }
}
