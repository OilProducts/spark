use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha1::{Digest, Sha1};
use spark_common::project::{definitions_dir, state_dir};
use spark_common::settings::SparkSettings;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use toml::Value as TomlValue;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::conversation::{open_state_database, STATE_DATABASE_FILE_NAME};
use crate::error::{Result, StorageError};
use crate::ProjectRegistry;

const SOURCE_SCHEDULE: &str = "schedule";
const SOURCE_POLL: &str = "poll";
const SOURCE_WEBHOOK: &str = "webhook";
const SOURCE_FLOW_EVENT: &str = "flow_event";
const ACTION_MODE_STATIC: &str = "static";
const WEEKDAY_ORDER: &[&str] = &["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
const TERMINAL_PIPELINE_STATUSES: &[&str] = &[
    "completed",
    "failed",
    "validation_error",
    "canceled",
    "cancelled",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerAction {
    #[serde(
        default = "default_trigger_action_mode",
        skip_serializing_if = "is_static_action_mode"
    )]
    pub mode: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub flow_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_id: Option<String>,
    /// The standing role a `role` action delivers to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub static_context: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_profile_id: Option<String>,
}

fn default_trigger_action_mode() -> String {
    ACTION_MODE_STATIC.to_string()
}

fn is_static_action_mode(value: &str) -> bool {
    value == ACTION_MODE_STATIC
}

/// A trigger's definition, read from `<org>/triggers/<id>.toml` in the org
/// repo of the project it belongs to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerDefinition {
    #[serde(skip)]
    pub revision: String,
    /// The project whose org holds the definition; derived, never stored.
    #[serde(skip)]
    pub project_path: String,
    pub id: String,
    pub name: String,
    pub protected: bool,
    pub source_type: String,
    pub action: TriggerAction,
    pub source: Map<String, Value>,
    pub created_at: String,
    pub updated_at: String,
}

/// A trigger's private activation state, in the state database. A missing
/// row means inactive; only an explicit activation arms a trigger, for the
/// definition content its `fingerprint` names.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TriggerState {
    #[serde(default)]
    pub enabled: bool,
    /// [`definition_fingerprint`] of the definition the activation validated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// The mission a `mission` action delivers to: an installation-local
    /// binding, never written to the shared definition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_id: Option<String>,
    /// Why the last activation attempt left the trigger inactive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation_error: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub last_fired_at: Option<String>,
    #[serde(default)]
    pub last_result: Option<String>,
    #[serde(default)]
    pub next_run_at: Option<String>,
    #[serde(default)]
    pub recent_history: Vec<TriggerStateHistoryEntry>,
    /// Webhook credentials live only here, never in a git repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_hash: Option<String>,
    /// `<run id>:<status>` keys of the flow events this trigger already fired
    /// for, newest last, so a re-published terminal run fires it only once.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flow_event_deliveries: Vec<String>,
}

impl TriggerState {
    /// Armed for exactly this definition's operational content.
    pub fn is_active_for(&self, definition: &TriggerDefinition) -> bool {
        self.enabled && self.fingerprint.as_deref() == Some(&definition_fingerprint(definition))
    }

    /// `definition` with this state's private bindings applied.
    pub fn bind(&self, mut definition: TriggerDefinition) -> TriggerDefinition {
        if definition.action.mode == "mission" {
            definition.action.mission_id = self.mission_id.clone();
        }
        definition
    }
}

/// Why an active trigger whose operational content changed was disarmed.
pub const DEFINITION_CHANGED: &str =
    "The definition's source or action changed; activate it again.";

/// Identifies the operational content of a definition (its source and
/// action, including the execution profile); descriptive fields and private
/// bindings are excluded.
pub fn definition_fingerprint(definition: &TriggerDefinition) -> String {
    let action = TriggerAction {
        mission_id: None,
        ..definition.action.clone()
    };
    let content = serde_json::json!({
        "source_type": definition.source_type,
        "source": definition.source,
        "action": action,
    });
    format!("{:x}", Sha1::digest(content.to_string().as_bytes()))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerStateHistoryEntry {
    pub timestamp: String,
    pub status: String,
    pub message: String,
    #[serde(default)]
    pub run_id: Option<String>,
}

/// One project's trigger definitions, under its `.spark/definitions`.
/// Reads never create directories there. Locks live under Spark's data
/// directory.
#[derive(Debug, Clone)]
pub struct TriggerDefinitionRepository {
    config_dir: PathBuf,
    project_path: String,
    lock_dir: PathBuf,
}

impl TriggerDefinitionRepository {
    pub fn project_path(&self) -> &str {
        &self.project_path
    }

    pub fn root_dir(&self) -> PathBuf {
        definitions_dir(Path::new(&self.project_path)).join("triggers")
    }

    pub fn definition_path(&self, trigger_id: &str) -> Result<PathBuf> {
        let id = validate_trigger_id(trigger_id)?;
        Ok(self.root_dir().join(format!("{id}.toml")))
    }

    /// The readable definitions; [`TriggerRepositories::reconcile_project`]
    /// reports the others.
    pub fn list(&self) -> Result<Vec<TriggerDefinition>> {
        let mut definitions = Vec::new();
        for trigger_id in self.definition_names()? {
            if let Ok(Some(definition)) = self.get(&trigger_id) {
                definitions.push(definition);
            }
        }
        Ok(definitions)
    }

    /// The stems of the `.toml` files among the definitions, sorted,
    /// without parsing them. A missing directory holds none; any other
    /// listing failure is an error, never an empty listing.
    pub fn definition_names(&self) -> Result<Vec<String>> {
        let root = self.root_dir();
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(StorageError::io("list trigger definitions", &root, source)),
        };
        let mut names = Vec::new();
        for entry in entries {
            let path = entry
                .map_err(|source| StorageError::io("list trigger definitions", &root, source))?
                .path();
            if path.extension().and_then(|value| value.to_str()) != Some("toml") {
                continue;
            }
            if let Some(stem) = path.file_stem().and_then(|value| value.to_str()) {
                names.push(stem.to_string());
            }
        }
        names.sort();
        Ok(names)
    }

    /// Whether the definition's file exists; an unreadable answer is an error.
    pub fn definition_present(&self, trigger_id: &str) -> Result<bool> {
        let path = self.definition_path(trigger_id)?;
        match fs::symlink_metadata(&path) {
            Ok(_) => Ok(true),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(StorageError::io("read trigger definition", &path, source)),
        }
    }

    pub fn get(&self, trigger_id: &str) -> Result<Option<TriggerDefinition>> {
        let path = self.definition_path(trigger_id)?;
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(StorageError::io("read trigger definition", &path, source)),
        };
        let payload = text
            .parse::<TomlValue>()
            .map_err(|source| StorageError::TomlRead {
                path: path.clone(),
                source,
            })?;
        let table = payload
            .as_table()
            .ok_or_else(|| invalid_trigger(&path, "Trigger definition must be a TOML table."))?;
        let mut definition = parse_trigger_definition(trigger_id, table, &path)?;
        definition.revision = format!("{:x}", Sha1::digest(text.as_bytes()));
        definition.project_path = self.project_path.clone();
        Ok(Some(definition))
    }

    /// Writes the definition and returns its new revision. `_message`
    /// describes the change; nothing records it.
    pub fn put(&self, definition: &TriggerDefinition, _message: Option<&str>) -> Result<String> {
        let _references = crate::settings::lock_profile_references(&self.config_dir)?;
        crate::settings::validate_profile_references(
            &self.config_dir,
            &serde_json::json!(definition.action),
        )?;
        let path = self.definition_path(&definition.id)?;
        let _lock = self.lock(&definition.id)?;
        let current = crate::settings::read_settings_document(&path)?;
        let expected = if definition.revision.is_empty() {
            "absent"
        } else {
            &definition.revision
        };
        if current.revision != expected {
            return Err(StorageError::SettingsConflict { path });
        }
        let payload: toml::Table = toml::from_str(&trigger_definition_toml(definition))
            .map_err(|_| invalid_trigger(&path, "Invalid trigger TOML."))?;
        // Written as normalized, which drops anything the file never holds,
        // such as webhook credentials.
        let text =
            trigger_definition_toml(&parse_trigger_definition(&definition.id, &payload, &path)?);
        fs::create_dir_all(self.root_dir())
            .map_err(|source| StorageError::io("create trigger directory", &path, source))?;
        self.replace(&path, text.as_bytes())?;
        Ok(format!("{:x}", Sha1::digest(text.as_bytes())))
    }

    /// Deletes the definition. `_message` describes the change; nothing
    /// records it.
    pub fn delete(
        &self,
        trigger_id: &str,
        expected_revision: &str,
        _message: Option<&str>,
    ) -> Result<()> {
        let path = self.definition_path(trigger_id)?;
        let _lock = self.lock(trigger_id)?;
        let current = crate::settings::read_settings_document(&path)?;
        if current.revision != expected_revision {
            return Err(StorageError::SettingsConflict { path });
        }
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(StorageError::io("delete trigger definition", &path, source)),
        }
    }

    /// Replaces `path` atomically: the new bytes are staged in
    /// `<project>/.spark/state/tmp` (on the project's filesystem), then
    /// renamed over the definition. A failed or partial write leaves the
    /// original untouched and no file among the definitions.
    fn replace(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        let project = Path::new(&self.project_path);
        let staging = state_dir(project)
            .map(|state| state.join("tmp"))
            .and_then(|tmp| {
                fs::create_dir_all(&tmp)
                    .map(|()| tmp.clone())
                    .map_err(|error| format!("Unable to create {}: {error}", tmp.display()))
            })
            .map_err(|error| {
                StorageError::io(
                    "stage trigger definition",
                    path,
                    std::io::Error::other(error),
                )
            })?;
        let mut temp = tempfile::Builder::new()
            .prefix(".spark-trigger-")
            .tempfile_in(&staging)
            .map_err(|source| StorageError::io("stage trigger definition", &staging, source))?;
        temp.write_all(bytes)
            .map_err(|source| StorageError::io("write trigger definition", path, source))?;
        temp.persist(path)
            .map_err(|error| StorageError::io("replace trigger definition", path, error.error))?;
        Ok(())
    }

    fn lock(&self, trigger_id: &str) -> Result<fs::File> {
        let id = validate_trigger_id(trigger_id)?;
        crate::settings::lock_document(&self.lock_dir.join(format!("{id}.toml")))
    }
}

/// One project's trigger activation rows.
#[derive(Debug, Clone)]
pub struct TriggerRuntimeStateRepository {
    home: PathBuf,
    project_id: String,
}

impl TriggerRuntimeStateRepository {
    fn connection(&self) -> Result<Connection> {
        open_state_database(&self.home)
    }

    fn read_in(&self, connection: &Connection, trigger_id: &str) -> Result<Option<TriggerState>> {
        let id = validate_trigger_id(trigger_id)?;
        let text: Option<String> = connection
            .query_row(
                "SELECT state FROM trigger_activations WHERE project_id = ?1 AND trigger_id = ?2",
                params![self.project_id, id],
                |row| row.get(0),
            )
            .optional()?;
        text.map(|text| self.parse(&text)).transpose()
    }

    /// Malformed private state is an error, never an inactive default.
    fn parse(&self, text: &str) -> Result<TriggerState> {
        serde_json::from_str(text).map_err(|source| StorageError::JsonRead {
            path: self.home.join(STATE_DATABASE_FILE_NAME),
            source,
        })
    }

    fn write_in(
        &self,
        connection: &Connection,
        trigger_id: &str,
        state: &TriggerState,
    ) -> Result<()> {
        let text = serde_json::to_string(state).map_err(|source| StorageError::JsonWrite {
            path: self.home.join(STATE_DATABASE_FILE_NAME),
            source,
        })?;
        connection.execute(
            "INSERT INTO trigger_activations (project_id, trigger_id, state) VALUES (?1, ?2, ?3) \
             ON CONFLICT DO UPDATE SET state = excluded.state",
            params![self.project_id, trigger_id, text],
        )?;
        Ok(())
    }

    /// The stored state, or `None` when the trigger has no activation row.
    pub fn get(&self, trigger_id: &str) -> Result<Option<TriggerState>> {
        self.read_in(&self.connection()?, trigger_id)
    }

    /// The stored state; a trigger without a row is inactive.
    pub fn load(&self, trigger_id: &str) -> Result<TriggerState> {
        Ok(self.get(trigger_id)?.unwrap_or_default())
    }

    /// Replaces the whole state. Writers that change part of it use
    /// [`Self::update`], so they never save a stale snapshot.
    pub fn save(&self, trigger_id: &str, state: &TriggerState) -> Result<()> {
        self.update(trigger_id, |current| *current = state.clone())
            .map(|_| ())
    }

    /// Reads, changes and writes the state (creating the row) in one
    /// immediate transaction, so concurrent writers serialize.
    pub fn update<F>(&self, trigger_id: &str, update: F) -> Result<TriggerState>
    where
        F: FnOnce(&mut TriggerState),
    {
        self.transact(trigger_id, true, update)
            .map(Option::unwrap_or_default)
    }

    /// [`Self::update`] of an existing row only: claims and outcomes, which
    /// must never recreate an activation removed meanwhile.
    pub fn update_existing<F>(&self, trigger_id: &str, update: F) -> Result<Option<TriggerState>>
    where
        F: FnOnce(&mut TriggerState),
    {
        self.transact(trigger_id, false, update)
    }

    fn transact<F>(&self, trigger_id: &str, create: bool, update: F) -> Result<Option<TriggerState>>
    where
        F: FnOnce(&mut TriggerState),
    {
        let id = validate_trigger_id(trigger_id)?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Project removal deletes existing rows in its own transaction, so
        // only a new row needs the registration check.
        let mut state = match self.read_in(&tx, id)? {
            Some(state) => state,
            None if create => {
                crate::workspace_projects::require_registered(&tx, &self.project_id)?;
                TriggerState::default()
            }
            None => return Ok(None),
        };
        update(&mut state);
        self.write_in(&tx, id, &state)?;
        tx.commit()?;
        Ok(Some(state))
    }

    /// Deletes the activation, its credentials and its delivery claims.
    pub fn delete(&self, trigger_id: &str) -> Result<()> {
        let id = validate_trigger_id(trigger_id)?;
        self.connection()?.execute(
            "DELETE FROM trigger_activations WHERE project_id = ?1 AND trigger_id = ?2",
            params![self.project_id, id],
        )?;
        Ok(())
    }
}

/// A definition that cannot run, or a project whose definitions could not
/// be listed. Neither is evidence that anything was removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TriggerIssue {
    pub project_path: String,
    pub trigger_id: Option<String>,
    pub error: String,
}

/// One reconciliation pass: the readable definitions, and what could not
/// be read.
#[derive(Debug, Clone, Default)]
pub struct TriggerReconciliation {
    pub definitions: Vec<TriggerDefinition>,
    pub issues: Vec<TriggerIssue>,
}

/// Triggers across projects: each project's definitions under
/// `.spark/definitions` and their activation rows in the state database.
#[derive(Debug, Clone)]
pub struct TriggerRepositories {
    config_dir: PathBuf,
    data_dir: PathBuf,
}

impl TriggerRepositories {
    pub fn new(config_dir: impl Into<PathBuf>, data_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
            data_dir: data_dir.into(),
        }
    }

    pub fn from_settings(settings: &SparkSettings) -> Self {
        Self::new(settings.config_dir.clone(), settings.data_dir.clone())
    }

    pub fn definitions(&self, project_path: &str) -> Result<TriggerDefinitionRepository> {
        let paths = ProjectRegistry::new(self.data_dir.clone()).project_paths(project_path)?;
        Ok(TriggerDefinitionRepository {
            config_dir: self.config_dir.clone(),
            project_path: paths.project_path,
            // Revision locks of the authored definitions. Kept outside the
            // project root that removal deletes, so a definition write racing
            // a removal neither blocks its cleanup nor recreates the root.
            lock_dir: self.data_dir.join("locks/triggers").join(&paths.project_id),
        })
    }

    pub fn runtime_state(&self, project_path: &str) -> Result<TriggerRuntimeStateRepository> {
        let paths = ProjectRegistry::new(self.data_dir.clone()).project_paths(project_path)?;
        Ok(TriggerRuntimeStateRepository {
            home: self.data_dir.clone(),
            project_id: paths.project_id,
        })
    }

    /// Every registered project's readable trigger definitions.
    pub fn list_all(&self) -> Result<Vec<TriggerDefinition>> {
        let mut definitions = Vec::new();
        for project in ProjectRegistry::new(self.data_dir.clone()).list_project_records()? {
            definitions.extend(self.definitions(&project.project_path)?.list()?);
        }
        Ok(definitions)
    }

    /// The project and trigger whose webhook key is `webhook_key`.
    pub fn find_webhook_key(&self, webhook_key: &str) -> Result<Option<(String, String)>> {
        let connection = open_state_database(&self.data_dir)?;
        Ok(connection
            .query_row(
                "SELECT p.project_path, t.trigger_id FROM trigger_activations t \
                 JOIN projects p ON p.project_id = t.project_id \
                 WHERE json_valid(t.state) AND json_extract(t.state, '$.webhook_key') = ?1",
                [webhook_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    /// Reconciles every registered project; see [`Self::reconcile_project`].
    pub fn reconcile(&self) -> Result<TriggerReconciliation> {
        let mut all = TriggerReconciliation::default();
        for project in ProjectRegistry::new(self.data_dir.clone()).list_project_records()? {
            let project = self.reconcile_project(&project.project_path)?;
            all.definitions.extend(project.definitions);
            all.issues.extend(project.issues);
        }
        Ok(all)
    }

    /// Compares a project's definition files with its activation rows. A
    /// successful listing without a definition's file deletes its row
    /// (activation, credentials and delivery claims); an active row whose
    /// definition's operational content changed is deactivated. An
    /// unavailable project or unlistable directory changes nothing, and an
    /// unreadable or invalid definition is reported, keeping its row.
    pub fn reconcile_project(&self, project_path: &str) -> Result<TriggerReconciliation> {
        let repository = self.definitions(project_path)?;
        let result = Self::scan(&repository);
        if result.issues.iter().any(|issue| issue.trigger_id.is_none()) {
            return Ok(result);
        }
        self.reconcile_rows(&repository, result)
    }

    /// The definitions that cannot run in `project_path`, or in every
    /// registered project, and the projects whose definitions could not be
    /// listed. Reads only.
    pub fn issues(&self, project_path: Option<&str>) -> Result<Vec<TriggerIssue>> {
        let projects = match project_path {
            Some(path) => vec![path.to_string()],
            None => ProjectRegistry::new(self.data_dir.clone())
                .list_project_records()?
                .into_iter()
                .map(|project| project.project_path)
                .collect(),
        };
        let mut issues = Vec::new();
        for project in projects {
            issues.extend(Self::scan(&self.definitions(&project)?).issues);
        }
        Ok(issues)
    }

    /// Reads a project's definitions, reporting what cannot be read.
    fn scan(repository: &TriggerDefinitionRepository) -> TriggerReconciliation {
        let project_path = repository.project_path.clone();
        let issue = |trigger_id: Option<&str>, error: String| TriggerIssue {
            project_path: project_path.clone(),
            trigger_id: trigger_id.map(str::to_string),
            error,
        };
        let mut result = TriggerReconciliation::default();
        if !Path::new(&project_path).is_dir() {
            result
                .issues
                .push(issue(None, "Project folder is unavailable.".into()));
            return result;
        }
        let names = match repository.definition_names() {
            Ok(names) => names,
            Err(error) => {
                result.issues.push(issue(None, error.to_string()));
                return result;
            }
        };
        for name in &names {
            match repository.get(name) {
                Ok(Some(definition)) => result.definitions.push(definition),
                Ok(None) => {}
                Err(error) => result.issues.push(issue(Some(name), error.to_string())),
            }
        }
        result
    }

    /// Deletes the rows of definitions confirmed removed and deactivates
    /// rows whose definition's operational content changed.
    fn reconcile_rows(
        &self,
        repository: &TriggerDefinitionRepository,
        mut result: TriggerReconciliation,
    ) -> Result<TriggerReconciliation> {
        let project_path = repository.project_path.clone();
        let issue = |trigger_id: Option<&str>, error: String| TriggerIssue {
            project_path: project_path.clone(),
            trigger_id: trigger_id.map(str::to_string),
            error,
        };
        let states = self.runtime_state(&project_path)?;
        let mut connection = states.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let rows = {
            let mut statement =
                tx.prepare("SELECT trigger_id FROM trigger_activations WHERE project_id = ?1")?;
            let rows = statement
                .query_map([&states.project_id], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        for trigger_id in rows {
            // Rechecked under the write lock: a definition written after the
            // listing, before its row, exists by now.
            let present = match validate_trigger_id(&trigger_id) {
                Ok(_) => repository.definition_present(&trigger_id),
                Err(_) => Ok(false),
            };
            match present {
                Ok(false) => {
                    tx.execute(
                        "DELETE FROM trigger_activations WHERE project_id = ?1 AND trigger_id = ?2",
                        params![states.project_id, trigger_id],
                    )?;
                    continue;
                }
                Ok(true) => {}
                Err(error) => {
                    result
                        .issues
                        .push(issue(Some(&trigger_id), error.to_string()));
                    continue;
                }
            }
            let Ok(Some(definition)) = repository.get(&trigger_id) else {
                continue;
            };
            // Malformed state is that trigger's error, never an inactive default.
            let mut state = match states.read_in(&tx, &trigger_id) {
                Ok(state) => state.unwrap_or_default(),
                Err(error) => {
                    result
                        .issues
                        .push(issue(Some(&trigger_id), error.to_string()));
                    continue;
                }
            };
            if state.enabled && !state.is_active_for(&definition) {
                state.enabled = false;
                state.activation_error = Some(DEFINITION_CHANGED.into());
                states.write_in(&tx, &trigger_id, &state)?;
            }
        }
        tx.commit()?;
        Ok(result)
    }
}

pub fn normalize_trigger_action_payload(payload: &Map<String, Value>) -> Result<TriggerAction> {
    normalize_trigger_action_payload_at(payload, Path::new("trigger definition"))
}

pub fn normalize_trigger_source_payload(
    source_type: &str,
    payload: &Map<String, Value>,
) -> Result<Map<String, Value>> {
    normalize_trigger_source_payload_at(source_type, payload, Path::new("trigger definition"))
}

fn parse_trigger_definition(
    trigger_id: &str,
    payload: &toml::map::Map<String, TomlValue>,
    path: &Path,
) -> Result<TriggerDefinition> {
    let action_payload = payload
        .get("action")
        .and_then(|value| value.as_table())
        .ok_or_else(|| {
            invalid_trigger(
                path,
                "Trigger definition must include [action] and [source] sections.",
            )
        })?;
    let source_payload = payload
        .get("source")
        .and_then(|value| value.as_table())
        .ok_or_else(|| {
            invalid_trigger(
                path,
                "Trigger definition must include [action] and [source] sections.",
            )
        })?;
    let mut action =
        normalize_trigger_action_payload_at(&parse_action_payload(action_payload), path)?;
    // The shared file never binds a mission; the activation row does.
    action.mission_id = None;
    let source_type = toml_scalar_to_string(payload.get("source_type"))
        .trim()
        .to_string();
    let source = normalize_trigger_source_payload_at(
        &source_type,
        &parse_source(source_payload, path)?,
        path,
    )?;
    Ok(TriggerDefinition {
        revision: String::new(),
        project_path: String::new(),
        id: trigger_id.to_string(),
        name: toml_scalar_to_string(payload.get("name"))
            .trim()
            .to_string(),
        protected: payload
            .get("protected")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        source_type,
        action,
        source,
        created_at: toml_scalar_to_string(payload.get("created_at")),
        updated_at: toml_scalar_to_string(payload.get("updated_at")),
    })
}

fn parse_action_payload(payload: &toml::map::Map<String, TomlValue>) -> Map<String, Value> {
    payload
        .iter()
        .map(|(key, value)| (key.clone(), toml_value_to_json(value)))
        .collect()
}

fn parse_source(
    payload: &toml::map::Map<String, TomlValue>,
    path: &Path,
) -> Result<Map<String, Value>> {
    let mut source = Map::new();
    for (key, value) in payload {
        if let Some(json_key) = key.strip_suffix("_json") {
            if let Some(text) = value.as_str() {
                let parsed = serde_json::from_str::<Value>(text).map_err(|_| {
                    invalid_trigger(path, format!("Invalid JSON value for source.{key}"))
                })?;
                source.insert(json_key.to_string(), parsed);
                continue;
            }
        }
        source.insert(key.clone(), toml_value_to_json(value));
    }
    Ok(source)
}

fn normalize_trigger_action_payload_at(
    payload: &Map<String, Value>,
    path: &Path,
) -> Result<TriggerAction> {
    let mode = payload
        .get("mode")
        .and_then(|value| value.as_str())
        .unwrap_or(ACTION_MODE_STATIC)
        .trim()
        .to_ascii_lowercase();
    if !matches!(mode.as_str(), ACTION_MODE_STATIC | "mission" | "role") {
        return Err(invalid_trigger(
            path,
            "Trigger action mode must be static, mission, or role.",
        ));
    }
    let flow_name = payload
        .get("flow_name")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if mode == ACTION_MODE_STATIC && flow_name.is_empty() {
        return Err(invalid_trigger(
            path,
            "Trigger action requires a flow_name.",
        ));
    }
    let mission_id = payload
        .get("mission_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    // A mission is a private activation binding: optional here, required
    // to activate.
    let role = payload
        .get("role")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|role| !role.is_empty())
        .map(str::to_string)
        .filter(|_| mode == "role");
    let mission_id = mission_id.filter(|_| mode == "mission");
    if mode == "role" && role.is_none() {
        return Err(invalid_trigger(path, "Role actions require role."));
    }
    let flow_name = if mode != ACTION_MODE_STATIC {
        String::new()
    } else {
        flow_name
    };
    let static_context = match payload.get("static_context") {
        Some(Value::Object(object)) => object.clone(),
        Some(Value::Null) | None => match payload
            .get("static_context_json")
            .and_then(|value| value.as_str())
        {
            Some(text) => serde_json::from_str::<Value>(text)
                .ok()
                .and_then(|value| value.as_object().cloned())
                .ok_or_else(|| {
                    invalid_trigger(
                        path,
                        "Trigger action static_context_json must be valid JSON.",
                    )
                })?,
            None => Map::new(),
        },
        Some(_) => {
            return Err(invalid_trigger(
                path,
                "Trigger action static_context must be a JSON object.",
            ))
        }
    };
    let execution_profile_id = payload
        .get("execution_profile_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    Ok(TriggerAction {
        mode,
        mission_id,
        role,
        flow_name,
        static_context,
        execution_profile_id,
    })
}

fn normalize_trigger_source_payload_at(
    source_type: &str,
    payload: &Map<String, Value>,
    path: &Path,
) -> Result<Map<String, Value>> {
    match source_type.trim() {
        SOURCE_SCHEDULE => normalize_schedule_source(payload, path),
        SOURCE_POLL => normalize_poll_source(payload, path),
        // Webhook credentials live in runtime state, not the definition.
        SOURCE_WEBHOOK => Ok(Map::new()),
        SOURCE_FLOW_EVENT => normalize_flow_event_source(payload, path),
        other => Err(invalid_trigger(
            path,
            format!("Unsupported trigger source type: {other}"),
        )),
    }
}

fn normalize_schedule_source(
    payload: &Map<String, Value>,
    path: &Path,
) -> Result<Map<String, Value>> {
    let kind = payload
        .get("kind")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if !["once", "interval", "weekly"].contains(&kind.as_str()) {
        return Err(invalid_trigger(
            path,
            "Schedule triggers require kind=once|interval|weekly.",
        ));
    }
    let mut source = Map::new();
    source.insert("kind".to_string(), Value::String(kind.clone()));
    match kind.as_str() {
        "once" => {
            let run_at = payload
                .get("run_at")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .trim();
            if run_at.is_empty() {
                return Err(invalid_trigger(
                    path,
                    "One-shot schedule triggers require run_at.",
                ));
            }
            if OffsetDateTime::parse(run_at, &Rfc3339).is_err() {
                return Err(invalid_trigger(
                    path,
                    "One-shot schedule run_at must be an RFC 3339 timestamp with an offset or Z, such as 2030-01-01T10:00:00Z.",
                ));
            }
            source.insert("run_at".to_string(), Value::String(run_at.to_string()));
        }
        "interval" => {
            source.insert(
                "interval_seconds".to_string(),
                Value::from(coerce_positive_int(
                    payload.get("interval_seconds"),
                    "interval_seconds",
                    path,
                )?),
            );
        }
        "weekly" => {
            let raw_weekdays = payload
                .get("weekdays")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            let mut weekdays = raw_weekdays
                .iter()
                .map(|value| {
                    json_value_to_python_string(value)
                        .trim()
                        .to_ascii_lowercase()
                })
                .collect::<Vec<_>>();
            if weekdays.is_empty()
                || weekdays
                    .iter()
                    .any(|weekday| !WEEKDAY_ORDER.contains(&weekday.as_str()))
            {
                return Err(invalid_trigger(
                    path,
                    "Weekly schedule triggers require weekdays using mon..sun.",
                ));
            }
            weekdays.sort_by_key(|weekday| {
                WEEKDAY_ORDER
                    .iter()
                    .position(|candidate| *candidate == weekday)
                    .unwrap_or(usize::MAX)
            });
            weekdays.dedup();
            source.insert(
                "weekdays".to_string(),
                Value::Array(weekdays.into_iter().map(Value::String).collect()),
            );
            source.insert(
                "hour".to_string(),
                Value::from(coerce_int_range(payload.get("hour"), "hour", 0, 23, path)?),
            );
            source.insert(
                "minute".to_string(),
                Value::from(coerce_int_range(
                    payload.get("minute"),
                    "minute",
                    0,
                    59,
                    path,
                )?),
            );
        }
        _ => {}
    }
    Ok(source)
}

fn normalize_poll_source(payload: &Map<String, Value>, path: &Path) -> Result<Map<String, Value>> {
    let url = payload
        .get("url")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(invalid_trigger(
            path,
            "Poll triggers require an http(s) url.",
        ));
    }
    let interval_seconds =
        coerce_positive_int(payload.get("interval_seconds"), "interval_seconds", path)?;
    let items_path = payload
        .get("items_path")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let item_id_path = payload
        .get("item_id_path")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if items_path.is_empty() || item_id_path.is_empty() {
        return Err(invalid_trigger(
            path,
            "Poll triggers require items_path and item_id_path.",
        ));
    }
    let headers = payload
        .get("headers")
        .and_then(|value| value.as_object())
        .map(|object| {
            object
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        Value::String(json_value_to_python_string(value)),
                    )
                })
                .collect()
        })
        .unwrap_or_else(Map::new);
    Ok(Map::from_iter([
        ("url".to_string(), Value::String(url)),
        (
            "interval_seconds".to_string(),
            Value::from(interval_seconds),
        ),
        ("items_path".to_string(), Value::String(items_path)),
        ("item_id_path".to_string(), Value::String(item_id_path)),
        ("headers".to_string(), Value::Object(headers)),
    ]))
}

fn normalize_flow_event_source(
    payload: &Map<String, Value>,
    path: &Path,
) -> Result<Map<String, Value>> {
    let flow_name = payload
        .get("flow_name")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| Value::String(value.to_string()))
        .unwrap_or(Value::Null);
    let statuses = payload
        .get("statuses")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|value| {
            json_value_to_python_string(&value)
                .trim()
                .to_ascii_lowercase()
        })
        .collect::<Vec<_>>();
    let normalized_statuses = statuses
        .iter()
        .filter(|status| TERMINAL_PIPELINE_STATUSES.contains(&status.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !statuses.is_empty() && normalized_statuses.is_empty() {
        return Err(invalid_trigger(
            path,
            "Flow-event triggers require terminal statuses when statuses are provided.",
        ));
    }
    Ok(Map::from_iter([
        ("flow_name".to_string(), flow_name),
        (
            "statuses".to_string(),
            Value::Array(normalized_statuses.into_iter().map(Value::String).collect()),
        ),
    ]))
}

fn coerce_positive_int(value: Option<&Value>, field_name: &str, path: &Path) -> Result<i64> {
    let number = coerce_int(value, field_name, path)?;
    if number <= 0 {
        return Err(invalid_trigger(
            path,
            format!("{field_name} must be greater than zero."),
        ));
    }
    Ok(number)
}

fn coerce_int_range(
    value: Option<&Value>,
    field_name: &str,
    minimum: i64,
    maximum: i64,
    path: &Path,
) -> Result<i64> {
    let number = coerce_int(value, field_name, path)?;
    if number < minimum || number > maximum {
        return Err(invalid_trigger(
            path,
            format!("{field_name} must be between {minimum} and {maximum}."),
        ));
    }
    Ok(number)
}

fn coerce_int(value: Option<&Value>, field_name: &str, path: &Path) -> Result<i64> {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .ok_or_else(|| invalid_trigger(path, format!("{field_name} must be an integer."))),
        _ => Err(invalid_trigger(
            path,
            format!("{field_name} must be an integer."),
        )),
    }
}

fn trigger_definition_toml(definition: &TriggerDefinition) -> String {
    let mut lines = vec![
        format!("id = {}", toml_string(&definition.id)),
        format!("name = {}", toml_string(&definition.name)),
        format!("protected = {}", toml_bool(definition.protected)),
        format!("source_type = {}", toml_string(&definition.source_type)),
        format!("created_at = {}", toml_string(&definition.created_at)),
        format!("updated_at = {}", toml_string(&definition.updated_at)),
        String::new(),
        "[action]".to_string(),
    ];
    if definition.action.mode != ACTION_MODE_STATIC {
        lines.push(format!("mode = {}", toml_string(&definition.action.mode)));
    }
    if let Some(role) = &definition.action.role {
        lines.push(format!("role = {}", toml_string(role)));
    }
    if !definition.action.flow_name.is_empty() {
        lines.push(format!(
            "flow_name = {}",
            toml_string(&definition.action.flow_name)
        ));
    }
    if !definition.action.static_context.is_empty() {
        lines.push(format!(
            "static_context_json = {}",
            toml_string(&json_string_python_style(&Value::Object(
                definition.action.static_context.clone()
            )))
        ));
    }
    if let Some(execution_profile_id) = definition.action.execution_profile_id.as_deref() {
        lines.push(format!(
            "execution_profile_id = {}",
            toml_string(execution_profile_id)
        ));
    }
    lines.push(String::new());
    lines.push("[source]".to_string());
    let mut source_keys = definition.source.keys().cloned().collect::<Vec<_>>();
    source_keys.sort();
    for key in source_keys {
        let Some(value) = definition.source.get(&key) else {
            continue;
        };
        if value.is_null() {
            continue;
        }
        lines.extend(toml_source_line(&key, value));
    }
    lines.push(String::new());
    lines.join("\n")
}

fn toml_source_line(key: &str, value: &Value) -> Vec<String> {
    match value {
        Value::Bool(value) => vec![format!("{key} = {}", toml_bool(*value))],
        Value::Number(value) => vec![format!("{key} = {value}")],
        Value::Array(values) => vec![format!(
            "{key} = [{}]",
            values
                .iter()
                .map(|value| toml_string(&json_value_to_python_string(value)))
                .collect::<Vec<_>>()
                .join(", ")
        )],
        Value::Object(_) => vec![format!(
            "{key}_json = {}",
            toml_string(&json_string_python_style(value))
        )],
        Value::String(value) => vec![format!("{key} = {}", toml_string(value))],
        Value::Null => Vec::new(),
    }
}

fn toml_value_to_json(value: &TomlValue) -> Value {
    match value {
        TomlValue::String(value) => Value::String(value.clone()),
        TomlValue::Integer(value) => Value::from(*value),
        TomlValue::Float(value) => Value::from(*value),
        TomlValue::Boolean(value) => Value::from(*value),
        TomlValue::Datetime(value) => Value::String(value.to_string()),
        TomlValue::Array(values) => Value::Array(values.iter().map(toml_value_to_json).collect()),
        TomlValue::Table(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), toml_value_to_json(value)))
                .collect(),
        ),
    }
}

fn toml_scalar_to_string(value: Option<&TomlValue>) -> String {
    match value {
        Some(TomlValue::String(value)) => value.clone(),
        Some(TomlValue::Integer(value)) => value.to_string(),
        Some(TomlValue::Float(value)) => value.to_string(),
        Some(TomlValue::Boolean(value)) => value.to_string(),
        Some(TomlValue::Datetime(value)) => value.to_string(),
        Some(_) | None => String::new(),
    }
}

fn json_value_to_python_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::Null => "None".to_string(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

fn json_string_python_style(value: &Value) -> String {
    match value {
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort();
            let parts = keys
                .into_iter()
                .map(|key| {
                    format!(
                        "{}: {}",
                        serde_json::to_string(key).expect("json key"),
                        json_string_python_style(&object[key])
                    )
                })
                .collect::<Vec<_>>();
            format!("{{{}}}", parts.join(", "))
        }
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(json_string_python_style)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::String(value) => serde_json::to_string(value).expect("json string"),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null => "null".to_string(),
    }
}

fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn toml_bool(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

fn validate_trigger_id(trigger_id: &str) -> Result<&str> {
    let id = trigger_id.trim();
    if id.is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id == "."
        || id == ".."
        || id.contains("..")
    {
        return Err(StorageError::InvalidRepositoryPath {
            path: PathBuf::from(trigger_id),
            reason: "Trigger id must be a file name.".to_string(),
        });
    }
    Ok(id)
}

fn invalid_trigger(path: &Path, reason: impl Into<String>) -> StorageError {
    StorageError::InvalidRepositoryPath {
        path: path.to_path_buf(),
        reason: reason.into(),
    }
}
