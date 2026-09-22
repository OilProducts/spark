use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::de::DeserializeOwned;
use sha1::{Digest, Sha1};

use crate::{write_atomic, Result, StorageError};

#[derive(Debug, Clone)]
pub struct SettingsDocument {
    pub values: toml::Table,
    pub revision: String,
}

impl SettingsDocument {
    /// Decode without reflecting authored values (which may contain secrets) in errors.
    pub fn section<T: DeserializeOwned>(&self, path: &Path, section: &str) -> Result<Option<T>> {
        self.values
            .get(section)
            .cloned()
            .map(|value| {
                value.try_into().map_err(|_| {
                    invalid(
                        path,
                        format!("Invalid {section} section; check field names and types."),
                    )
                })
            })
            .transpose()
    }
}

/// Re-read on each operation, including changes made outside the application.
pub fn read_settings_document(path: &Path) -> Result<SettingsDocument> {
    let bytes = read_bytes(path)?;
    decode(path, bytes.as_deref())
}

/// The validator checks the complete candidate document before any replacement.
/// All cooperating writers must use this boundary and its stable sidecar lock.
pub fn update_settings_section(
    path: &Path,
    expected_revision: &str,
    section: &str,
    value: Option<toml::Value>,
    validate: impl FnOnce(&toml::Table) -> Result<()>,
) -> Result<SettingsDocument> {
    update_settings_sections(path, expected_revision, [(section, value)], validate)
}

/// Patch related sections of one dedicated document under one revision and lock.
pub fn update_settings_sections<'a>(
    path: &Path,
    expected_revision: &str,
    sections: impl IntoIterator<Item = (&'a str, Option<toml::Value>)>,
    validate: impl FnOnce(&toml::Table) -> Result<()>,
) -> Result<SettingsDocument> {
    let _lock = lock_document(path)?;
    let mut document = read_settings_document(path)?;
    if document.revision != expected_revision {
        return Err(StorageError::SettingsConflict { path: path.into() });
    }
    for (section, value) in sections {
        if section.is_empty() || matches!(section, "schema_version" | "browser_migration_version") {
            return Err(invalid(
                path,
                "A settings section is required; version markers are migration-owned.",
            ));
        }
        match value {
            Some(value) => {
                document.values.insert(section.into(), value);
            }
            None => {
                document.values.remove(section);
            }
        }
    }
    validate(&document.values)?;
    persist(path, &document.values)
}

fn persist(path: &Path, values: &toml::Table) -> Result<SettingsDocument> {
    let text = toml::to_string_pretty(values)
        .map_err(|_| invalid(path, "Settings could not be serialized as TOML."))?;
    write_atomic(path, &text)?;
    decode(path, Some(text.as_bytes()))
}

fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(StorageError::io("read settings", path, error)),
    }
}

fn decode(path: &Path, bytes: Option<&[u8]>) -> Result<SettingsDocument> {
    let Some(bytes) = bytes else {
        return Ok(SettingsDocument {
            values: toml::Table::new(),
            revision: "absent".into(),
        });
    };
    let text = std::str::from_utf8(bytes).map_err(|_| invalid(path, "Expected UTF-8 TOML."))?;
    let values = toml::from_str(text).map_err(|error: toml::de::Error| {
        let position = error
            .span()
            .map(|span| format!(" near byte {}", span.start))
            .unwrap_or_default();
        invalid(
            path,
            format!("Malformed TOML{position}; check the document syntax."),
        )
    })?;
    if path.file_name().is_some_and(|name| name == "spark.toml") {
        validate_core_version(path, &values)?;
    }
    Ok(SettingsDocument {
        values,
        revision: format!("{:x}", Sha1::digest(bytes)),
    })
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    value.into()
}

/// Serialize profile deletion with reference validation and persistence. Acquire
/// this before any document lock, and retain it until the reference is durable.
/// ponytail: one workspace lock is sufficient for infrequent configuration edits.
pub fn lock_profile_references(config_dir: &Path) -> Result<File> {
    lock_document(&config_dir.join("profile-references"))
}

/// Check authored profile references while holding `lock_profile_references`.
/// Domain validators remain responsible for model compatibility and policies.
pub fn validate_profile_references(config_dir: &Path, value: &serde_json::Value) -> Result<()> {
    match value {
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                let filename = match key.as_str() {
                    "llm_profile" | "_attractor.runtime.launch_profile" => {
                        Some("llm-profiles.toml")
                    }
                    "execution_profile_id" => Some("execution-profiles.toml"),
                    _ => None,
                };
                if let (Some(filename), Some(id)) = (filename, value.as_str()) {
                    let id = id.trim();
                    if id.is_empty() {
                        continue;
                    }
                    let path = config_dir.join(filename);
                    let document = read_settings_document(&path)?;
                    if filename == "execution-profiles.toml"
                        && document.revision == "absent"
                        && id == "native"
                    {
                        continue;
                    }
                    if !document
                        .values
                        .get("profiles")
                        .and_then(toml::Value::as_table)
                        .is_some_and(|profiles| profiles.keys().any(|key| key.trim() == id))
                    {
                        return Err(invalid(
                            &path,
                            format!(
                                "Unknown profile reference in {key}; select an existing profile."
                            ),
                        ));
                    }
                } else {
                    validate_profile_references(config_dir, value)?;
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                validate_profile_references(config_dir, value)?;
            }
        }
        _ => (),
    }
    Ok(())
}

pub(crate) fn lock_document(path: &Path) -> Result<File> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|error| StorageError::io("create settings directory", parent, error))?;
    }
    let lock_path = sidecar(path, ".lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| StorageError::io("open settings lock", &lock_path, error))?;
    file.lock_exclusive()
        .map_err(|error| StorageError::io("lock settings", &lock_path, error))?;
    Ok(file)
}

fn invalid(path: &Path, reason: impl Into<String>) -> StorageError {
    StorageError::SettingsValidation {
        path: path.into(),
        reason: reason.into(),
    }
}

pub fn client_settings_path(config_dir: &Path, client_id: &str) -> Result<PathBuf> {
    spark_common::settings::validate_client_id(client_id)
        .map_err(|error| invalid(config_dir, error.to_string()))?;
    Ok(config_dir.join("clients").join(format!("{client_id}.toml")))
}

/// Platform bootstrap identity is independent of the server's ephemeral address.
pub fn load_or_create_client_identity(path: &Path) -> Result<String> {
    let _lock = lock_document(path)?;
    if let Some(bytes) = read_bytes(path)? {
        let id = std::str::from_utf8(&bytes)
            .map_err(|_| invalid(path, "Invalid client identity; expected UTF-8."))?;
        spark_common::settings::validate_client_id(id)
            .map_err(|error| invalid(path, error.to_string()))?;
        return Ok(id.to_owned());
    }
    let id = format!("desktop-{:032x}", rand::random::<u128>());
    write_atomic(path, &id)?;
    Ok(id)
}

/// Every bootstrap reads core settings through this boundary before loading
/// persisted runtime choices. A fresh install creates the current document.
pub fn load_core_settings(core: &Path) -> Result<SettingsDocument> {
    let legacy = core.with_file_name("ui-defaults.json");
    if legacy.exists() {
        return Err(invalid(&legacy, "The legacy defaults import was removed; delete this file or move its values into spark.toml."));
    }
    let _lock = lock_document(core)?;
    let document = read_settings_document(core)?;
    if document.revision == "absent" {
        return persist(
            core,
            &toml::Table::from_iter([("schema_version".to_owned(), 1.into())]),
        );
    }
    validate_core_sections(core, &document.values)?;
    Ok(document)
}

fn backup_migration_source(path: &Path, suffix: &str, bytes: &[u8]) -> Result<()> {
    let backup = sidecar(path, suffix);
    match read_bytes(&backup)? {
        Some(existing) if existing != bytes => Err(invalid(
            path,
            "Migration backup differs from the original; resolve the backup before retrying.",
        )),
        Some(_) => Ok(()),
        None => write_atomic(&backup, bytes),
    }
}

pub fn validate_core_sections(path: &Path, values: &toml::Table) -> Result<()> {
    validate_core_version(path, values)?;
    let document = SettingsDocument {
        values: values.clone(),
        revision: String::new(),
    };
    document
        .section::<spark_common::settings::ConnectionSettings>(path, "connections")?
        .unwrap_or_default()
        .validate()
        .map_err(|error| invalid(path, error.to_string()))?;
    document
        .section::<spark_common::provider_settings::ProviderConnections>(path, "providers")?
        .unwrap_or_default()
        .validate()
        .map_err(|error| invalid(path, error.to_string()))?;
    document
        .section::<spark_common::agent_settings::SessionConfig>(path, "agents")?
        .unwrap_or_default()
        .validate()
        .map_err(|error| invalid(path, error.to_string()))?;
    let runtime: spark_common::settings::RuntimeSettings =
        document.section(path, "runtime")?.unwrap_or_default();
    runtime
        .validate()
        .map_err(|error| invalid(path, error.to_string()))?;
    let models: spark_common::settings::ModelSettings =
        document.section(path, "models")?.unwrap_or_default();
    models
        .validate()
        .map_err(|error| invalid(path, error.to_string()))?;
    validate_desktop_section(path, values)
}

pub fn validate_desktop_section(path: &Path, values: &toml::Table) -> Result<()> {
    SettingsDocument {
        values: values.clone(),
        revision: String::new(),
    }
    .section::<spark_common::settings::DesktopSettings>(path, "desktop")?;
    Ok(())
}

pub fn update_desktop_settings(
    path: &Path,
    expected_revision: &str,
    value: &spark_common::settings::DesktopSettings,
) -> Result<SettingsDocument> {
    update_settings_section(
        path,
        expected_revision,
        "desktop",
        Some(toml::Value::try_from(value).map_err(|_| invalid(path, "Invalid Desktop settings."))?),
        |values| validate_core_sections(path, values),
    )
}

/// Validate the core version at every entry point, including non-Desktop reads/writes.
pub fn validate_core_version(path: &Path, values: &toml::Table) -> Result<()> {
    match values.get("schema_version") {
        None | Some(toml::Value::Integer(1)) => Ok(()),
        Some(found) => Err(invalid(
            path,
            format!(
                "Unsupported settings schema_version {found}; expected 1. Use a compatible Spark binary."
            ),
        )),
    }
}

/// Called while the document update lock is held, after validating the candidate.
pub fn backup_settings_before_import(path: &Path) -> Result<()> {
    let Some(bytes) = read_bytes(path)? else {
        return Ok(());
    };
    let backup = sidecar(path, ".browser-defaults-v0.bak");
    match read_bytes(&backup)? {
        Some(existing) if existing != bytes => Err(invalid(
            path,
            "Settings import backup differs; resolve the backup before retrying.",
        )),
        Some(_) => Ok(()),
        None => write_atomic(&backup, &bytes),
    }
}

/// Resolve authored defaults at launch time, including edits made since startup.
/// Domain callers validate provider/profile compatibility before starting work.
pub fn read_project_model_defaults(
    settings: &spark_common::settings::SparkSettings,
    project_path: &str,
) -> Result<(
    spark_common::settings::ModelSettings,
    spark_common::settings::ModelSettingsSource,
)> {
    use spark_common::settings::{resolve_model_settings, ModelSettings};
    let core = settings.config_dir.join("spark.toml");
    let workspace: ModelSettings = read_settings_document(&core)?
        .section(&core, "models")?
        .unwrap_or_default();
    let project_file = crate::ProjectRegistry::new(&settings.data_dir)
        .project_paths(project_path)?
        .project_file;
    let project: Option<ModelSettings> =
        read_settings_document(&project_file)?.section(&project_file, "model_settings")?;
    let (group, source) = resolve_model_settings(&workspace, project.as_ref(), None)
        .map_err(|_| invalid(&core, "Invalid model defaults; select exactly one provider or profile and nonempty optional fields."))?;
    Ok((group.clone(), source))
}

pub use spark_common::settings::ExecutionConfiguration;

pub fn read_execution_configuration(
    config_dir: &Path,
    env: &impl spark_common::paths::Environment,
) -> Result<ExecutionConfiguration> {
    let path = config_dir.join("spark.toml");
    let document = read_settings_document(&path)?;
    validate_core_sections(&path, &document.values)?;
    let providers: spark_common::provider_settings::ProviderConnections =
        document.section(&path, "providers")?.unwrap_or_default();
    let mut agents: spark_common::agent_settings::SessionConfig =
        document.section(&path, "agents")?.unwrap_or_default();
    agents.native = agents
        .native
        .resolve(env)
        .map_err(|error| invalid(&path, error.to_string()))?;
    let stored_runtime = document.section(&path, "runtime")?.unwrap_or_default();
    let runtime = spark_common::settings::resolve_settings_with_persisted(
        &spark_common::settings::SettingsOverrides {
            data_dir: config_dir.parent().map(Path::to_path_buf),
            ..Default::default()
        },
        env,
        &stored_runtime,
    )
    .map_err(|error| invalid(&path, error.to_string()))?;
    Ok(ExecutionConfiguration {
        config_dir: config_dir.to_path_buf(),
        runtime: spark_common::settings::RuntimeSettings {
            runs_dir: Some(runtime.runs_dir),
            flows_dir: Some(runtime.flows_dir),
            ui_dir: runtime.ui_dir,
            project_roots: runtime.project_roots,
        },
        providers: providers
            .resolve(env)
            .map_err(|error| invalid(&path, error.to_string()))?,
        agents,
    })
}

/// One-time browser import under the client document lock. Existing fields win.
pub fn import_client_preferences(
    path: &Path,
    expected_revision: &str,
    preferences: &spark_common::settings::ClientPreferences,
) -> Result<SettingsDocument> {
    preferences
        .validate()
        .map_err(|error| invalid(path, error.to_string()))?;
    let _lock = lock_document(path)?;
    let mut document = read_settings_document(path)?;
    if !matches!(
        document.values.get("schema_version"),
        None | Some(toml::Value::Integer(1))
    ) {
        return Err(invalid(path, "Unsupported client settings version."));
    }
    if !matches!(
        document.values.get("browser_migration_version"),
        None | Some(toml::Value::Integer(0 | 1))
    ) {
        return Err(invalid(
            path,
            "Unsupported browser preference migration version.",
        ));
    }

    match document
        .values
        .get("browser_migration_version")
        .and_then(toml::Value::as_integer)
        .unwrap_or(0)
    {
        1 => return Ok(document),
        0 => (),
        _ => {
            return Err(invalid(
                path,
                "Unsupported browser preference migration version.",
            ))
        }
    }
    if document.revision != expected_revision {
        return Err(StorageError::SettingsConflict { path: path.into() });
    }
    let existing: spark_common::settings::ClientPreferences =
        document.section(path, "preferences")?.unwrap_or_default();
    existing
        .validate()
        .map_err(|error| invalid(path, error.to_string()))?;
    let mut imported = toml::Value::try_from(preferences)
        .map_err(|_| invalid(path, "Invalid client preferences."))?
        .as_table()
        .cloned()
        .unwrap_or_default();
    if let Some(existing) = document
        .values
        .get("preferences")
        .and_then(toml::Value::as_table)
    {
        for (key, value) in existing {
            if matches!(key.as_str(), "flow_node_positions" | "flow_edge_ports") {
                if let (Some(incoming), Some(authoritative)) = (
                    imported.get_mut(key).and_then(toml::Value::as_table_mut),
                    value.as_table(),
                ) {
                    incoming.extend(authoritative.clone());
                    continue;
                }
            }
            imported.insert(key.clone(), value.clone());
        }
    }
    let combined: spark_common::settings::ClientPreferences = toml::Value::Table(imported.clone())
        .try_into()
        .map_err(|_| invalid(path, "Invalid client preferences."))?;
    combined
        .validate()
        .map_err(|error| invalid(path, error.to_string()))?;
    if let Some(bytes) = read_bytes(path)? {
        backup_migration_source(path, ".browser-import-v0.bak", &bytes)?;
    }
    document
        .values
        .insert("preferences".into(), toml::Value::Table(imported));
    document
        .values
        .insert("browser_migration_version".into(), 1.into());
    persist(path, &document.values)
}

/// Target for a new CLI using this configuration home (an explicit CLI flag wins before this).
pub fn resolve_client_api_base_url(
    config_dir: &Path,
    env: &impl spark_common::paths::Environment,
) -> Result<(String, &'static str)> {
    if let Some(value) = env
        .get_var("SPARK_API_BASE_URL")
        .filter(|value| !value.trim().is_empty())
    {
        return Ok((value.trim().to_owned(), "environment: SPARK_API_BASE_URL"));
    }
    let path = config_dir.join("spark.toml");
    let connections: spark_common::settings::ConnectionSettings = read_settings_document(&path)?
        .section(&path, "connections")?
        .unwrap_or_default();
    connections
        .validate()
        .map_err(|error| invalid(&path, error.to_string()))?;
    Ok(match connections.client_api_base_url {
        Some(value) => (value, "stored"),
        None => (
            spark_common::source_checkout::DEFAULT_API_BASE_URL.into(),
            "built-in default",
        ),
    })
}
