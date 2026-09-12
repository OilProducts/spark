use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use spark_common::project::{build_project_id, normalize_project_path};
use time::OffsetDateTime;
use toml::map::Map as TomlMap;

use crate::error::{Result, StorageError};
use crate::workspace_conversations::ConversationHandleRepository;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectPaths {
    pub project_id: String,
    pub project_path: String,
    pub display_name: String,
    pub root: PathBuf,
    pub project_file: PathBuf,
    pub conversations_dir: PathBuf,
    pub flow_run_requests_dir: PathBuf,
    pub flow_launches_dir: PathBuf,
    pub proposed_plans_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRecord {
    pub project_id: String,
    pub project_path: String,
    pub display_name: String,
    pub created_at: String,
    pub last_opened_at: String,
    pub last_accessed_at: Option<String>,
    pub is_favorite: bool,
    pub active_conversation_id: Option<String>,
    pub execution_profile_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeletedProjectRecord {
    pub project_id: String,
    pub project_path: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectRecordUpdate {
    pub expected_revision: Option<String>,
    pub display_name: Option<String>,
    pub last_accessed_at: Option<Option<String>>,
    pub is_favorite: Option<bool>,
    pub active_conversation_id: Option<Option<String>>,
    pub execution_profile_id: Option<Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRegistry {
    home_dir: PathBuf,
}

impl ProjectRegistry {
    pub fn new(home_dir: impl Into<PathBuf>) -> Self {
        Self {
            home_dir: home_dir.into(),
        }
    }

    pub fn home_dir(&self) -> &Path {
        &self.home_dir
    }

    pub fn workspace_root(&self) -> PathBuf {
        self.home_dir.join("workspace")
    }

    pub fn projects_root(&self) -> PathBuf {
        self.workspace_root().join("projects")
    }

    pub fn ensure_project_paths(&self, project_path: &str) -> Result<ProjectPaths> {
        self.ensure_project_paths_with_profile(project_path, None)
    }

    fn ensure_project_paths_with_profile(
        &self,
        project_path: &str,
        initial_profile: Option<&str>,
    ) -> Result<ProjectPaths> {
        let project_paths = self.project_paths(project_path)?;
        for directory in [
            &project_paths.root,
            &project_paths.conversations_dir,
            &project_paths.flow_run_requests_dir,
            &project_paths.flow_launches_dir,
            &project_paths.proposed_plans_dir,
        ] {
            fs::create_dir_all(directory).map_err(|source| {
                StorageError::io("create project directory", directory, source)
            })?;
        }

        let _lock = crate::settings::lock_document(&project_paths.project_file)?;
        let document = crate::settings::read_settings_document(&project_paths.project_file)?;
        if initial_profile.is_some() && document.revision != "absent" {
            return Err(StorageError::SettingsConflict {
                path: project_paths.project_file,
            });
        }
        let payload = document.values;
        let valid = payload.get("project_id").and_then(toml::Value::as_str)
            == Some(project_paths.project_id.as_str())
            && payload.get("project_path").and_then(toml::Value::as_str)
                == Some(project_paths.project_path.as_str())
            && ["display_name", "created_at", "last_opened_at"]
                .iter()
                .all(|key| {
                    payload
                        .get(*key)
                        .and_then(toml::Value::as_str)
                        .is_some_and(|value| !value.trim().is_empty())
                })
            && [
                "last_accessed_at",
                "active_conversation_id",
                "execution_profile_id",
            ]
            .iter()
            .all(|key| payload.get(*key).is_none_or(toml::Value::is_str))
            && payload.get("is_favorite").is_none_or(toml::Value::is_bool);
        if valid {
            return Ok(project_paths);
        }
        let now = iso_now();
        let created_at =
            read_optional_string(&payload, "created_at").unwrap_or_else(|| now.clone());
        let record = ProjectRecord {
            project_id: project_paths.project_id.clone(),
            project_path: project_paths.project_path.clone(),
            display_name: read_required_string(&payload, "display_name")
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| project_paths.display_name.clone()),
            created_at,
            last_opened_at: read_optional_string(&payload, "last_opened_at").unwrap_or(now),
            last_accessed_at: read_optional_string(&payload, "last_accessed_at"),
            is_favorite: read_optional_bool(&payload, "is_favorite", false),
            active_conversation_id: read_optional_string(&payload, "active_conversation_id"),
            execution_profile_id: initial_profile
                .map(str::to_string)
                .or_else(|| read_optional_string(&payload, "execution_profile_id")),
        };
        write_project_record(&project_paths.project_file, &record)?;
        Ok(project_paths)
    }

    pub fn register_project(&self, project_path: &str) -> Result<ProjectRecord> {
        self.register_project_with_execution_profile(project_path, None)
    }

    pub fn register_project_with_execution_profile(
        &self,
        project_path: &str,
        initial_profile: Option<&str>,
    ) -> Result<ProjectRecord> {
        let project_paths =
            self.ensure_project_paths_with_profile(project_path, initial_profile)?;
        let _lock = crate::settings::lock_document(&project_paths.project_file)?;
        let mut record = self
            .read_project_record_by_id(&project_paths.project_id)?
            .ok_or_else(|| invalid_project_path(project_path, "Unable to register project."))?;
        record.last_opened_at = iso_now();
        write_project_record(&project_paths.project_file, &record)?;
        Ok(record)
    }

    pub fn read_project_record(&self, project_path: &str) -> Result<Option<ProjectRecord>> {
        let project_paths = self.project_paths(project_path)?;
        self.read_project_record_by_id(&project_paths.project_id)
    }

    pub fn read_project_paths_by_id(&self, project_id: &str) -> Result<Option<ProjectPaths>> {
        let root = self.projects_root().join(project_id);
        let project_file = root.join("project.toml");
        if !project_file.exists() {
            return Ok(None);
        }
        let payload = read_project_payload_lossy(&project_file);
        let Some(raw_project_path) = read_required_string(&payload, "project_path") else {
            return Ok(None);
        };
        let Some(project_path) = normalize_project_path_for_storage(&raw_project_path)? else {
            return Ok(None);
        };
        let display_name = read_required_string(&payload, "display_name")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| display_name_for_path(&project_path));
        Ok(Some(ProjectPaths {
            project_id: project_id.to_string(),
            project_path,
            display_name,
            conversations_dir: root.join("conversations"),
            flow_run_requests_dir: root.join("flow-run-requests"),
            flow_launches_dir: root.join("flow-launches"),
            proposed_plans_dir: root.join("proposed-plans"),
            project_file,
            root,
        }))
    }

    pub fn read_project_record_by_id(&self, project_id: &str) -> Result<Option<ProjectRecord>> {
        let Some(project_paths) = self.read_project_paths_by_id(project_id)? else {
            return Ok(None);
        };
        let payload = read_project_payload_lossy(&project_paths.project_file);
        Ok(Some(ProjectRecord {
            project_id: project_paths.project_id,
            project_path: project_paths.project_path,
            display_name: read_required_string(&payload, "display_name")
                .filter(|value| !value.is_empty())
                .unwrap_or(project_paths.display_name),
            created_at: read_required_string(&payload, "created_at").unwrap_or_default(),
            last_opened_at: read_required_string(&payload, "last_opened_at").unwrap_or_default(),
            last_accessed_at: read_optional_string(&payload, "last_accessed_at"),
            is_favorite: read_optional_bool(&payload, "is_favorite", false),
            active_conversation_id: read_optional_string(&payload, "active_conversation_id"),
            execution_profile_id: read_optional_string(&payload, "execution_profile_id"),
        }))
    }

    pub fn list_project_records(&self) -> Result<Vec<ProjectRecord>> {
        let projects_root = self.projects_root();
        fs::create_dir_all(&projects_root).map_err(|source| {
            StorageError::io(
                "create workspace projects directory",
                &projects_root,
                source,
            )
        })?;
        let mut entries = fs::read_dir(&projects_root)
            .map_err(|source| {
                StorageError::io("read workspace projects directory", &projects_root, source)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|source| {
                StorageError::io("read workspace projects directory", &projects_root, source)
            })?;
        entries.sort_by_key(|entry| entry.path());

        let mut records = Vec::new();
        for entry in entries {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(project_id) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if let Some(record) = self.read_project_record_by_id(project_id)? {
                records.push(record);
            }
        }
        Ok(records)
    }

    pub fn update_project_record(
        &self,
        project_path: &str,
        update: ProjectRecordUpdate,
    ) -> Result<ProjectRecord> {
        if update.execution_profile_id.is_some() && update.expected_revision.is_none() {
            return Err(StorageError::SettingsValidation {
                path: self.project_paths(project_path)?.project_file,
                reason: "expected_revision is required to change project execution settings."
                    .into(),
            });
        }
        let project_paths = if update.expected_revision.is_some() {
            self.project_paths(project_path)?
        } else {
            self.ensure_project_paths(project_path)?
        };
        let _lock = crate::settings::lock_document(&project_paths.project_file)?;
        let document = crate::settings::read_settings_document(&project_paths.project_file)?;
        if update
            .expected_revision
            .as_ref()
            .is_some_and(|revision| *revision != document.revision)
        {
            return Err(StorageError::SettingsConflict {
                path: project_paths.project_file,
            });
        }
        let payload = document.values;
        let record = ProjectRecord {
            project_id: project_paths.project_id.clone(),
            project_path: project_paths.project_path.clone(),
            display_name: update
                .display_name
                .and_then(normalize_optional_string)
                .or_else(|| read_required_string(&payload, "display_name"))
                .unwrap_or_else(|| project_paths.display_name.clone()),
            created_at: read_required_string(&payload, "created_at").unwrap_or_else(iso_now),
            last_opened_at: read_required_string(&payload, "last_opened_at")
                .unwrap_or_else(iso_now),
            last_accessed_at: update
                .last_accessed_at
                .map(|value| value.and_then(normalize_optional_string))
                .unwrap_or_else(|| read_optional_string(&payload, "last_accessed_at")),
            is_favorite: update
                .is_favorite
                .unwrap_or_else(|| read_optional_bool(&payload, "is_favorite", false)),
            active_conversation_id: update
                .active_conversation_id
                .map(|value| value.and_then(normalize_optional_string))
                .unwrap_or_else(|| read_optional_string(&payload, "active_conversation_id")),
            execution_profile_id: update
                .execution_profile_id
                .map(|value| value.and_then(normalize_optional_string))
                .unwrap_or_else(|| read_optional_string(&payload, "execution_profile_id")),
        };
        write_project_record(&project_paths.project_file, &record)?;
        self.read_project_record_by_id(&project_paths.project_id)?
            .ok_or_else(|| invalid_project_path(project_path, "Unable to register project."))
    }

    pub fn delete_project_record(&self, project_path: &str) -> Result<DeletedProjectRecord> {
        let normalized = normalize_project_path_for_storage(project_path)?
            .ok_or_else(|| invalid_project_path(project_path, "Project path is required."))?;
        let project_id = project_id_for_path(&normalized)?;
        let Some(project_paths) = self.read_project_paths_by_id(&project_id)? else {
            return Err(invalid_project_path(project_path, "Unknown project."));
        };
        if project_paths.project_path != normalized {
            return Err(invalid_project_path(project_path, "Unknown project."));
        }
        let deleted = DeletedProjectRecord {
            project_id: project_paths.project_id.clone(),
            project_path: project_paths.project_path.clone(),
            display_name: project_paths.display_name.clone(),
        };
        fs::remove_dir_all(&project_paths.root).map_err(|source| {
            StorageError::io(
                "delete workspace project directory",
                &project_paths.root,
                source,
            )
        })?;
        ConversationHandleRepository::new(self.home_dir.clone())
            .remove_project_conversation_handles(&project_paths.project_id)?;
        Ok(deleted)
    }

    pub fn project_paths(&self, project_path: &str) -> Result<ProjectPaths> {
        let normalized = normalize_project_path_for_storage(project_path)?
            .ok_or_else(|| invalid_project_path(project_path, "Project path is required."))?;
        let project_id = project_id_for_path(&normalized)?;
        let display_name = display_name_for_path(&normalized);
        let root = self.projects_root().join(&project_id);
        Ok(ProjectPaths {
            project_id,
            project_path: normalized,
            display_name,
            project_file: root.join("project.toml"),
            conversations_dir: root.join("conversations"),
            flow_run_requests_dir: root.join("flow-run-requests"),
            flow_launches_dir: root.join("flow-launches"),
            proposed_plans_dir: root.join("proposed-plans"),
            root,
        })
    }
}

fn read_project_payload_lossy(path: &Path) -> TomlMap<String, toml::Value> {
    let Ok(text) = fs::read_to_string(path) else {
        return TomlMap::new();
    };
    let Ok(value) = text.parse::<toml::Value>() else {
        return TomlMap::new();
    };
    value.as_table().cloned().unwrap_or_default()
}

// Caller holds the shared document lock across reading and replacing the record.
fn write_project_record(path: &Path, record: &ProjectRecord) -> Result<()> {
    let mut extra = crate::settings::read_settings_document(path)?.values;
    for key in [
        "project_id",
        "project_path",
        "display_name",
        "created_at",
        "last_opened_at",
        "last_accessed_at",
        "is_favorite",
        "active_conversation_id",
        "execution_profile_id",
    ] {
        extra.remove(key);
    }
    let mut lines = vec![
        format!("project_id = {}", toml_string(&record.project_id)),
        format!("project_path = {}", toml_string(&record.project_path)),
        format!("display_name = {}", toml_string(&record.display_name)),
        format!("created_at = {}", toml_string(&record.created_at)),
        format!("last_opened_at = {}", toml_string(&record.last_opened_at)),
    ];
    if let Some(value) = record
        .last_accessed_at
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("last_accessed_at = {}", toml_string(value)));
    }
    lines.push(format!(
        "is_favorite = {}",
        if record.is_favorite { "true" } else { "false" }
    ));
    if let Some(value) = record
        .active_conversation_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("active_conversation_id = {}", toml_string(value)));
    }
    if let Some(value) = record
        .execution_profile_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("execution_profile_id = {}", toml_string(value)));
    }
    lines.push(String::new());
    if !extra.is_empty() {
        lines.push(
            toml::to_string(&extra).map_err(|_| StorageError::SettingsValidation {
                path: path.into(),
                reason: "Unable to serialize project settings.".into(),
            })?,
        );
    }
    crate::write_text_atomic(path, lines.join("\n"))
}

fn read_required_string(payload: &TomlMap<String, toml::Value>, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(toml::Value::as_str)
        .map(str::to_string)
}

fn read_optional_string(payload: &TomlMap<String, toml::Value>, key: &str) -> Option<String> {
    read_required_string(payload, key).and_then(normalize_optional_string)
}

fn read_optional_bool(payload: &TomlMap<String, toml::Value>, key: &str, default: bool) -> bool {
    payload
        .get(key)
        .and_then(toml::Value::as_bool)
        .unwrap_or(default)
}

fn normalize_optional_string(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn normalize_project_path_for_storage(value: &str) -> Result<Option<String>> {
    normalize_project_path(value)
        .map(|value| value.map(|path| path.to_string_lossy().into_owned()))
        .map_err(|error| invalid_project_path(value, error.to_string()))
}

fn project_id_for_path(value: &str) -> Result<String> {
    build_project_id(value).map_err(|error| invalid_project_path(value, error.to_string()))
}

fn display_name_for_path(value: &str) -> String {
    Path::new(value)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

fn invalid_project_path(path: impl AsRef<str>, reason: impl Into<String>) -> StorageError {
    StorageError::InvalidRepositoryPath {
        path: PathBuf::from(path.as_ref()),
        reason: reason.into(),
    }
}

fn iso_now() -> String {
    let now = OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

fn toml_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            _ => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
}
