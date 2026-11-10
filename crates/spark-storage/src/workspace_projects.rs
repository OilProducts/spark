use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use spark_common::project::{build_project_id, normalize_project_path};
use time::OffsetDateTime;

use crate::conversation::{open_state_database, STATE_DATABASE_FILE_NAME};
use crate::error::{Result, StorageError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectPaths {
    pub project_id: String,
    pub project_path: String,
    pub display_name: String,
    pub root: PathBuf,
    pub conversations_dir: PathBuf,
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
    /// The Home project, rooted at the user's home folder; it can't be removed.
    #[serde(default)]
    pub is_default: bool,
    /// Derived on read: whether the project's folder is still a directory.
    #[serde(default)]
    pub folder_exists: bool,
}

pub const HOME_PROJECT_NAME: &str = "Home";

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

/// A project's private model and execution defaults, with the revision that
/// guards their edits: `"absent"` for an unregistered project.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectSettings {
    pub revision: String,
    /// The whole stored model selection, as written.
    pub model_settings: Option<Value>,
    pub execution_profile_id: Option<String>,
}

const ABSENT_REVISION: &str = "absent";
const PROJECT_COLUMNS: &str = "project_id, project_path, display_name, created_at, \
    last_opened_at, last_accessed_at, is_favorite, active_conversation_id, \
    execution_profile_id, is_default";

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

    fn connection(&self) -> Result<Connection> {
        open_state_database(&self.home_dir)
    }

    fn database_path(&self) -> PathBuf {
        self.home_dir.join(STATE_DATABASE_FILE_NAME)
    }

    /// Run `work` in one immediate transaction on a fresh connection.
    fn write<T>(&self, work: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let value = work(&tx)?;
        tx.commit()?;
        Ok(value)
    }

    pub fn ensure_project_paths(&self, project_path: &str) -> Result<ProjectPaths> {
        self.ensure_project_paths_in(&self.connection()?, project_path)
    }

    /// `ensure_project_paths` on `connection`, which a caller already inside
    /// a write transaction on the state database passes.
    pub(crate) fn ensure_project_paths_in(
        &self,
        connection: &Connection,
        project_path: &str,
    ) -> Result<ProjectPaths> {
        let project_paths = self.project_paths(project_path)?;
        for directory in [&project_paths.root, &project_paths.conversations_dir] {
            fs::create_dir_all(directory).map_err(|source| {
                StorageError::io("create project directory", directory, source)
            })?;
        }
        let now = iso_now();
        connection.execute(
            "INSERT INTO projects (project_id, project_path, display_name, created_at, \
             last_opened_at) VALUES (?1, ?2, ?3, ?4, ?4) ON CONFLICT DO NOTHING",
            params![
                project_paths.project_id,
                project_paths.project_path,
                project_paths.display_name,
                now
            ],
        )?;
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
        let project_paths = self.project_paths(project_path)?;
        self.write(|connection| {
            if initial_profile.is_some()
                && read_record(connection, &project_paths.project_id)?.is_some()
            {
                return Err(StorageError::SettingsConflict {
                    path: self.database_path(),
                });
            }
            self.ensure_project_paths_in(connection, project_path)?;
            connection.execute(
                "UPDATE projects SET last_opened_at = ?2, \
                 execution_profile_id = coalesce(?3, execution_profile_id) WHERE project_id = ?1",
                params![project_paths.project_id, iso_now(), initial_profile],
            )?;
            read_record(connection, &project_paths.project_id)?
                .ok_or_else(|| invalid_project_path(project_path, "Unable to register project."))
        })
    }

    pub fn read_project_record(&self, project_path: &str) -> Result<Option<ProjectRecord>> {
        let project_paths = self.project_paths(project_path)?;
        self.read_project_record_by_id(&project_paths.project_id)
    }

    pub fn read_project_paths_by_id(&self, project_id: &str) -> Result<Option<ProjectPaths>> {
        Ok(self.read_project_record_by_id(project_id)?.map(|record| {
            self.paths_for(record.project_id, record.project_path, record.display_name)
        }))
    }

    pub fn read_project_record_by_id(&self, project_id: &str) -> Result<Option<ProjectRecord>> {
        read_record(&self.connection()?, project_id)
    }

    pub fn list_project_records(&self) -> Result<Vec<ProjectRecord>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(&format!(
            "SELECT {PROJECT_COLUMNS} FROM projects ORDER BY project_id"
        ))?;
        let records = statement
            .query_map([], project_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(records)
    }

    pub fn update_project_record(
        &self,
        project_path: &str,
        update: ProjectRecordUpdate,
    ) -> Result<ProjectRecord> {
        if update.execution_profile_id.is_some() && update.expected_revision.is_none() {
            return Err(StorageError::SettingsValidation {
                path: self.database_path(),
                reason: "expected_revision is required to change project execution settings."
                    .into(),
            });
        }
        let project_paths = self.project_paths(project_path)?;
        let id = project_paths.project_id.clone();
        self.write(|connection| {
            if let Some(expected) = &update.expected_revision {
                if *expected != settings_revision(connection, &id)? {
                    return Err(StorageError::SettingsConflict {
                        path: self.database_path(),
                    });
                }
            }
            self.ensure_project_paths_in(connection, project_path)?;
            let mut changes: Vec<(&str, Option<String>)> = Vec::new();
            if let Some(name) = update.display_name.and_then(normalize_optional_string) {
                changes.push(("display_name", Some(name)));
            }
            if let Some(value) = update.last_accessed_at {
                changes.push((
                    "last_accessed_at",
                    value.and_then(normalize_optional_string),
                ));
            }
            if let Some(value) = update.active_conversation_id {
                changes.push((
                    "active_conversation_id",
                    value.and_then(normalize_optional_string),
                ));
            }
            if let Some(value) = update.execution_profile_id {
                changes.push((
                    "execution_profile_id",
                    value.and_then(normalize_optional_string),
                ));
                connection.execute(
                    "UPDATE projects SET settings_revision = settings_revision + 1 \
                     WHERE project_id = ?1",
                    [&id],
                )?;
            }
            for (column, value) in changes {
                connection.execute(
                    &format!("UPDATE projects SET {column} = ?2 WHERE project_id = ?1"),
                    params![id, value],
                )?;
            }
            if let Some(favorite) = update.is_favorite {
                connection.execute(
                    "UPDATE projects SET is_favorite = ?2 WHERE project_id = ?1",
                    params![id, favorite],
                )?;
            }
            read_record(connection, &id)?
                .ok_or_else(|| invalid_project_path(project_path, "Unable to register project."))
        })
    }

    /// The project's stored model and execution defaults.
    pub fn project_settings(&self, project_path: &str) -> Result<ProjectSettings> {
        let id = self.project_paths(project_path)?.project_id;
        let connection = self.connection()?;
        let row = connection
            .query_row(
                "SELECT settings_revision, model_settings, execution_profile_id FROM projects \
                 WHERE project_id = ?1",
                [&id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((revision, model_settings, execution_profile_id)) = row else {
            return Ok(ProjectSettings {
                revision: ABSENT_REVISION.into(),
                model_settings: None,
                execution_profile_id: None,
            });
        };
        let model_settings = model_settings
            .map(|text| {
                serde_json::from_str(&text).map_err(|_| StorageError::SettingsValidation {
                    path: self.database_path(),
                    reason: format!("Invalid model_settings stored for project {project_path}."),
                })
            })
            .transpose()?;
        Ok(ProjectSettings {
            revision: revision.to_string(),
            model_settings,
            execution_profile_id,
        })
    }

    /// Replace a registered project's whole model selection (`None` inherits
    /// the workspace defaults) when `expected_revision` is current.
    pub fn update_project_model_settings(
        &self,
        project_path: &str,
        expected_revision: &str,
        model_settings: Option<&Value>,
    ) -> Result<()> {
        let id = self.project_paths(project_path)?.project_id;
        let stored = model_settings.map(|value| {
            let mut value = value.clone();
            if let Some(object) = value.as_object_mut() {
                object.retain(|_, field| !field.is_null());
            }
            value.to_string()
        });
        self.write(|connection| {
            if expected_revision != settings_revision(connection, &id)?
                || expected_revision == ABSENT_REVISION
            {
                return Err(StorageError::SettingsConflict {
                    path: self.database_path(),
                });
            }
            connection.execute(
                "UPDATE projects SET model_settings = ?2, settings_revision = settings_revision + 1 \
                 WHERE project_id = ?1",
                params![id, stored],
            )?;
            Ok(())
        })
    }

    /// Registers `home_path` as the Home project, named "Home" and marked as
    /// the default, keeping any settings it already has.
    pub fn ensure_home_project(&self, home_path: &str) -> Result<ProjectRecord> {
        let id = self.project_paths(home_path)?.project_id;
        self.write(|connection| {
            self.ensure_project_paths_in(connection, home_path)?;
            connection.execute(
                "UPDATE projects SET is_default = 1, display_name = ?2 WHERE project_id = ?1",
                params![id, HOME_PROJECT_NAME],
            )?;
            read_record(connection, &id)?
                .ok_or_else(|| invalid_project_path(home_path, "Unable to register project."))
        })
    }

    /// The registered, non-Home project at `project_path`, validated for
    /// deletion. Its rows and directory go in a separate step that only the
    /// conversation store runs: rows with the tombstones, the directory once
    /// they are checkpointed ([`crate::ConversationStore::delete_project`]).
    pub fn deletable_project(&self, project_path: &str) -> Result<DeletedProjectRecord> {
        let normalized = normalize_project_path_for_storage(project_path)?
            .ok_or_else(|| invalid_project_path(project_path, "Project path is required."))?;
        let project_id = project_id_for_path(&normalized)?;
        let Some(record) = self
            .read_project_record_by_id(&project_id)?
            .filter(|record| record.project_path == normalized)
        else {
            return Err(invalid_project_path(project_path, "Unknown project."));
        };
        if record.is_default {
            return Err(invalid_project_path(
                project_path,
                "The Home project can't be removed.",
            ));
        }
        Ok(DeletedProjectRecord {
            project_id: record.project_id,
            project_path: record.project_path,
            display_name: record.display_name,
        })
    }

    pub fn project_paths(&self, project_path: &str) -> Result<ProjectPaths> {
        let normalized = normalize_project_path_for_storage(project_path)?
            .ok_or_else(|| invalid_project_path(project_path, "Project path is required."))?;
        let project_id = project_id_for_path(&normalized)?;
        let display_name = display_name_for_path(&normalized);
        Ok(self.paths_for(project_id, normalized, display_name))
    }

    fn paths_for(
        &self,
        project_id: String,
        project_path: String,
        display_name: String,
    ) -> ProjectPaths {
        let root = self.projects_root().join(&project_id);
        ProjectPaths {
            project_id,
            project_path,
            display_name,
            conversations_dir: root.join("conversations"),
            root,
        }
    }
}

fn read_record(connection: &Connection, project_id: &str) -> Result<Option<ProjectRecord>> {
    Ok(connection
        .query_row(
            &format!("SELECT {PROJECT_COLUMNS} FROM projects WHERE project_id = ?1"),
            [project_id],
            project_row,
        )
        .optional()?)
}

fn project_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectRecord> {
    let project_path: String = row.get(1)?;
    Ok(ProjectRecord {
        project_id: row.get(0)?,
        folder_exists: Path::new(&project_path).is_dir(),
        project_path,
        display_name: row.get(2)?,
        created_at: row.get(3)?,
        last_opened_at: row.get(4)?,
        last_accessed_at: row.get(5)?,
        is_favorite: row.get(6)?,
        active_conversation_id: row.get(7)?,
        execution_profile_id: row.get(8)?,
        is_default: row.get(9)?,
    })
}

fn settings_revision(connection: &Connection, project_id: &str) -> Result<String> {
    Ok(connection
        .query_row(
            "SELECT settings_revision FROM projects WHERE project_id = ?1",
            [project_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .map_or_else(
            || ABSENT_REVISION.to_string(),
            |revision| revision.to_string(),
        ))
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
