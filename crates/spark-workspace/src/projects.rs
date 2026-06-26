use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use spark_common::paths::{normalize_path, resolve_runtime_workspace_path};
use spark_common::project::normalize_project_path;
use spark_common::settings::SparkSettings;
use spark_storage::{DeletedProjectRecord, ProjectRecord, ProjectRecordUpdate, ProjectRegistry};

use crate::conversations::{ConversationSummary, WorkspaceConversationService};
use crate::errors::{WorkspaceError, WorkspaceResult};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRegistrationRequest {
    pub project_path: String,
    #[serde(default)]
    pub execution_profile_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectStateUpdate {
    pub project_path: String,
    pub last_accessed_at: Option<Option<String>>,
    pub is_favorite: Option<bool>,
    pub active_conversation_id: Option<Option<String>>,
    pub execution_profile_id: Option<Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectMetadata {
    pub name: String,
    pub directory: String,
    pub branch: Option<String>,
    pub commit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowseEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowseResponse {
    pub current_path: String,
    pub parent_path: Option<String>,
    pub roots: Vec<String>,
    pub entries: Vec<BrowseEntry>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceProjectService {
    settings: SparkSettings,
}

impl WorkspaceProjectService {
    pub fn new(settings: SparkSettings) -> Self {
        Self { settings }
    }

    pub fn settings(&self) -> &SparkSettings {
        &self.settings
    }

    pub fn list_projects(&self) -> WorkspaceResult<Vec<ProjectRecord>> {
        self.registry().list_project_records().map_err(Into::into)
    }

    pub fn register_project(
        &self,
        request: ProjectRegistrationRequest,
    ) -> WorkspaceResult<ProjectRecord> {
        let project_path = normalize_project_path_or_400(&request.project_path)?;
        let registry = self.registry();
        let mut record = registry
            .read_project_record(&project_path)?
            .ok_or_else(|| WorkspaceError::Validation("Unable to register project.".to_string()))?;
        if request.execution_profile_id.is_some() {
            let execution_profile_id = self
                .validate_project_execution_profile_id(request.execution_profile_id.as_deref())?;
            record = registry.update_project_record(
                &project_path,
                ProjectRecordUpdate {
                    execution_profile_id: Some(execution_profile_id),
                    ..ProjectRecordUpdate::default()
                },
            )?;
        }
        Ok(record)
    }

    pub fn update_project_state(
        &self,
        request: ProjectStateUpdate,
    ) -> WorkspaceResult<ProjectRecord> {
        let project_path = normalize_project_path_or_400(&request.project_path)?;
        let execution_profile_id = match request.execution_profile_id {
            Some(value) => Some(self.validate_project_execution_profile_id(value.as_deref())?),
            None => None,
        };
        self.registry()
            .update_project_record(
                &project_path,
                ProjectRecordUpdate {
                    last_accessed_at: request.last_accessed_at,
                    is_favorite: request.is_favorite,
                    active_conversation_id: request.active_conversation_id,
                    execution_profile_id,
                    ..ProjectRecordUpdate::default()
                },
            )
            .map_err(Into::into)
    }

    pub fn delete_project(&self, project_path: &str) -> WorkspaceResult<DeletedProjectRecord> {
        let project_path = normalize_project_path_or_400(project_path)?;
        self.registry()
            .delete_project_record(&project_path)
            .map_err(|error| match error {
                spark_storage::StorageError::InvalidRepositoryPath { reason, .. }
                    if reason == "Unknown project." =>
                {
                    WorkspaceError::NotFound(reason)
                }
                other => other.into(),
            })
    }

    pub fn list_project_conversations(
        &self,
        project_path: &str,
    ) -> WorkspaceResult<Vec<ConversationSummary>> {
        WorkspaceConversationService::new(self.settings.clone())
            .list_project_conversations(project_path)
    }

    pub fn project_metadata(&self, directory: &str) -> WorkspaceResult<ProjectMetadata> {
        let normalized_path = normalize_project_directory(directory)?;
        let runtime_path = resolve_runtime_workspace_path(normalized_path.to_string_lossy())
            .map_err(|error| WorkspaceError::Validation(error.to_string()))?;
        let runtime_path = PathBuf::from(runtime_path);
        Ok(ProjectMetadata {
            name: normalized_path
                .file_name()
                .and_then(|value| value.to_str())
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| normalized_path.to_string_lossy().into_owned()),
            directory: normalized_path.to_string_lossy().into_owned(),
            branch: git_output(&runtime_path, &["rev-parse", "--abbrev-ref", "HEAD"]),
            commit: git_output(&runtime_path, &["rev-parse", "HEAD"]),
        })
    }

    pub fn browse_project_directories(
        &self,
        path: Option<&str>,
    ) -> WorkspaceResult<BrowseResponse> {
        let current_path = self.normalize_browse_path(path)?;
        if !current_path.exists() {
            return Err(WorkspaceError::NotFound(format!(
                "Browse path does not exist: {}",
                current_path.display()
            )));
        }
        if !current_path.is_dir() {
            return Err(WorkspaceError::Validation(format!(
                "Browse path is not a directory: {}",
                current_path.display()
            )));
        }

        let read_dir = fs::read_dir(&current_path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::PermissionDenied {
                WorkspaceError::Forbidden(format!(
                    "Browse path is not accessible: {}",
                    current_path.display()
                ))
            } else {
                WorkspaceError::Internal(format!(
                    "Unable to read browse path {}: {source}",
                    current_path.display()
                ))
            }
        })?;
        let mut entries = Vec::new();
        for entry in read_dir {
            let entry = entry.map_err(|source| {
                WorkspaceError::Internal(format!(
                    "Unable to read browse path {}: {source}",
                    current_path.display()
                ))
            })?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let normalized = normalize_path(&path)
                .map_err(|error| WorkspaceError::Validation(error.to_string()))?;
            entries.push(BrowseEntry {
                name,
                path: normalized.to_string_lossy().into_owned(),
                is_dir: true,
            });
        }
        entries.sort_by(|left, right| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.name.cmp(&right.name))
        });
        let parent_path = current_path
            .parent()
            .filter(|parent| *parent != current_path)
            .map(|parent| parent.to_string_lossy().into_owned());
        Ok(BrowseResponse {
            current_path: current_path.to_string_lossy().into_owned(),
            parent_path,
            roots: self
                .settings
                .project_roots
                .iter()
                .map(|root| root.to_string_lossy().into_owned())
                .collect(),
            entries,
        })
    }

    pub fn chat_models(&self, project_path: &str) -> WorkspaceResult<Value> {
        let _ = normalize_project_path_or_400(project_path)?;
        crate::models::chat_models(&self.settings)
    }

    fn registry(&self) -> ProjectRegistry {
        ProjectRegistry::new(self.settings.data_dir.clone())
    }

    fn normalize_browse_path(&self, requested_path: Option<&str>) -> WorkspaceResult<PathBuf> {
        let Some(requested_path) = requested_path else {
            if let Some(root) = self.settings.project_roots.first() {
                return Ok(root.clone());
            }
            return service_home_dir()
                .map_err(|message| WorkspaceError::Validation(message))
                .and_then(|path| {
                    normalize_path(path)
                        .map_err(|error| WorkspaceError::Validation(error.to_string()))
                });
        };
        let trimmed = requested_path.trim();
        if trimmed.is_empty() {
            return Err(WorkspaceError::Validation(
                "Browse path is required.".to_string(),
            ));
        }
        let expanded = expand_tilde(Path::new(trimmed));
        if !expanded.is_absolute() {
            return Err(WorkspaceError::Validation(
                "Browse path must be absolute.".to_string(),
            ));
        }
        let normalized = normalize_path(expanded)
            .map_err(|error| WorkspaceError::Validation(error.to_string()))?;
        if !normalized.is_absolute() {
            return Err(WorkspaceError::Validation(
                "Browse path must be absolute.".to_string(),
            ));
        }
        Ok(normalized)
    }

    fn validate_project_execution_profile_id(
        &self,
        execution_profile_id: Option<&str>,
    ) -> WorkspaceResult<Option<String>> {
        let normalized_profile_id = execution_profile_id.unwrap_or("").trim().to_string();
        if normalized_profile_id.is_empty() {
            return Ok(None);
        }
        let execution_placement = attractor_api::execution_placement_settings(&self.settings).body;
        if execution_placement
            .get("validation_errors")
            .and_then(Value::as_array)
            .map(|errors| !errors.is_empty())
            .unwrap_or(false)
        {
            return Err(WorkspaceError::Validation(
                "Execution profile settings are invalid; fix execution-profiles.toml before selecting a project default."
                    .to_string(),
            ));
        }
        let Some(profiles) = execution_placement
            .get("profiles")
            .and_then(Value::as_array)
        else {
            return Err(WorkspaceError::Validation(
                "Execution profile settings did not return a profile list.".to_string(),
            ));
        };
        let profile = profiles.iter().find(|profile| {
            profile
                .get("id")
                .and_then(Value::as_str)
                .map(|id| id.trim() == normalized_profile_id)
                .unwrap_or(false)
        });
        let Some(profile) = profile else {
            return Err(WorkspaceError::Validation(format!(
                "Unknown execution profile: {normalized_profile_id}"
            )));
        };
        if profile.get("enabled").and_then(Value::as_bool) != Some(true) {
            return Err(WorkspaceError::Validation(format!(
                "Execution profile is disabled: {normalized_profile_id}"
            )));
        }
        Ok(Some(normalized_profile_id))
    }
}

fn normalize_project_path_or_400(project_path: &str) -> WorkspaceResult<String> {
    normalize_project_path(project_path)
        .map_err(|error| WorkspaceError::Validation(error.to_string()))?
        .map(|path| path.to_string_lossy().into_owned())
        .ok_or_else(|| WorkspaceError::Validation("Project path is required.".to_string()))
}

fn normalize_project_directory(directory: &str) -> WorkspaceResult<PathBuf> {
    let requested_path = directory.trim();
    if requested_path.is_empty() {
        return Err(WorkspaceError::Validation(
            "Project directory path is required.".to_string(),
        ));
    }
    let expanded = expand_tilde(Path::new(requested_path));
    if !expanded.is_absolute() {
        return Err(WorkspaceError::Validation(
            "Project directory path must be absolute.".to_string(),
        ));
    }
    normalize_path(expanded).map_err(|error| WorkspaceError::Validation(error.to_string()))
}

fn git_output(directory: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn expand_tilde(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if text == "~" {
        return service_home_dir().unwrap_or_else(|_| path.to_path_buf());
    }
    if let Some(rest) = text.strip_prefix("~/") {
        if let Ok(home) = service_home_dir() {
            return home.join(rest);
        }
    }
    path.to_path_buf()
}

fn service_home_dir() -> std::result::Result<PathBuf, String> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set.".to_string())
}
