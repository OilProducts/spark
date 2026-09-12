//! Dedicated profile documents, edited through the shared revision/lock boundary.
use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
use spark_storage::settings::{read_settings_document, update_settings_sections};
use unified_llm_adapter::profiles::{parse_llm_profiles, LlmProfile, ProcessLlmProfileEnvironment};

use crate::{WorkspaceError, WorkspaceResult};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionProfilesUpdate {
    pub profiles: Vec<attractor_execution::ExecutionProfile>,
    pub default_execution_profile_id: Option<String>,
}

pub fn llm_profiles_view(settings: &SparkSettings) -> WorkspaceResult<Value> {
    let path = settings.config_dir.join("llm-profiles.toml");
    let document = read_settings_document(&path)?;
    let profiles = parse_llm_profiles(&document.values)
        .map_err(|error| WorkspaceError::Validation(error.to_string()))?;
    Ok(json!({"scope": "workspace", "revision": document.revision,
        "stored": profiles.values().collect::<Vec<_>>(),
        "credential_status": profiles.iter().map(|(id, profile)|
            (id.clone(), if profile.configured(&ProcessLlmProfileEnvironment) { "configured" } else { "missing" }))
            .collect::<std::collections::BTreeMap<_, _>>(),
        "restart_fields": [], "validation_errors": []}))
}

pub fn execution_profiles_view(settings: &SparkSettings) -> WorkspaceResult<Value> {
    let path = settings.config_dir.join("execution-profiles.toml");
    let document = read_settings_document(&path)?;
    let parsed = if document.revision == "absent" {
        Ok(attractor_execution::ExecutionProfileGraph {
            profiles: std::collections::BTreeMap::from([(
                "native".into(),
                attractor_execution::ExecutionProfile::implementation_native(),
            )]),
            synthesized_native_default: true,
            ..Default::default()
        })
    } else {
        attractor_execution::profile::parse_execution_profiles(&document.values).and_then(|graph| {
            graph.validate_default()?;
            Ok(graph)
        })
    };
    let graph = match parsed {
        Ok(graph) => graph,
        Err(error) => {
            return Ok(json!({"scope": "workspace", "revision": document.revision,
            "stored": null, "restart_fields": [], "validation_errors": [error.to_string()]}))
        }
    };
    Ok(json!({"scope": "workspace", "revision": document.revision,
        "stored": {"profiles": graph.profiles.values().collect::<Vec<_>>(),
            "default_execution_profile_id": graph.default_execution_profile_id},
        "restart_fields": [], "validation_errors": []}))
}

fn profiles_table<'a, T: Serialize + 'a>(
    profiles: impl Iterator<Item = (&'a str, &'a T)>,
) -> WorkspaceResult<toml::Table> {
    let mut table = toml::Table::new();
    for (id, profile) in profiles {
        if id.trim().is_empty() || id != id.trim() {
            return Err(WorkspaceError::Validation(
                "Profile IDs must be nonempty without surrounding whitespace.".into(),
            ));
        }
        let mut value = toml::Value::try_from(profile).map_err(|_| {
            WorkspaceError::Validation(
                "Profile fields must be representable as TOML; metadata cannot contain null."
                    .into(),
            )
        })?;
        value.as_table_mut().expect("profile struct").remove("id");
        if table.insert(id.into(), value).is_some() {
            return Err(WorkspaceError::Validation(
                "Profile IDs must be unique.".into(),
            ));
        }
    }
    Ok(table)
}

pub fn llm_candidate(profiles: &[LlmProfile]) -> WorkspaceResult<toml::Table> {
    let values = toml::Table::from_iter([(
        "profiles".into(),
        toml::Value::Table(profiles_table(
            profiles
                .iter()
                .map(|profile| (profile.id.as_str(), profile)),
        )?),
    )]);
    parse_llm_profiles(&values).map_err(|error| WorkspaceError::Validation(error.to_string()))?;
    Ok(values)
}

pub fn execution_candidate(value: &ExecutionProfilesUpdate) -> WorkspaceResult<toml::Table> {
    let mut defaults = toml::Table::new();
    if let Some(id) = &value.default_execution_profile_id {
        defaults.insert("execution_profile_id".into(), id.clone().into());
    }
    let values = toml::Table::from_iter([
        ("defaults".into(), toml::Value::Table(defaults)),
        (
            "profiles".into(),
            toml::Value::Table(profiles_table(
                value
                    .profiles
                    .iter()
                    .map(|profile| (profile.id.as_str(), profile)),
            )?),
        ),
    ]);
    attractor_execution::profile::parse_execution_profiles(&values)
        .and_then(|graph| graph.validate_default())
        .map_err(|error| WorkspaceError::Validation(error.to_string()))?;
    Ok(values)
}

pub fn update_profiles(
    settings: &SparkSettings,
    revision: &str,
    candidate: toml::Table,
    execution: bool,
) -> WorkspaceResult<Value> {
    let _references = spark_storage::settings::lock_profile_references(&settings.config_dir)?;
    let filename = if execution {
        "execution-profiles.toml"
    } else {
        "llm-profiles.toml"
    };
    let path = settings.config_dir.join(filename);
    update_settings_sections(
        &path,
        revision,
        candidate
            .iter()
            .map(|(key, value)| (key.as_str(), Some(value.clone()))),
        |values| {
            let validate = || -> WorkspaceResult<()> {
                if execution {
                    attractor_execution::profile::parse_execution_profiles(values)
                        .and_then(|graph| graph.validate_default())
                        .map_err(|error| WorkspaceError::Validation(error.to_string()))?;
                } else {
                    parse_llm_profiles(values)
                        .map_err(|error| WorkspaceError::Validation(error.to_string()))?;
                }
                let old = read_settings_document(&path)?;
                let old_ids = old
                    .values
                    .get("profiles")
                    .and_then(toml::Value::as_table)
                    .into_iter()
                    .flat_map(|profiles| profiles.keys())
                    .map(|id| id.trim().to_owned())
                    .chain((execution && old.revision == "absent").then(|| "native".to_owned()));
                let removed: BTreeSet<_> = old_ids
                    .filter(|id| {
                        !values["profiles"]
                            .as_table()
                            .expect("validated profiles")
                            .contains_key(id)
                    })
                    .collect();
                if !removed.is_empty() {
                    let references = profile_references(settings, &removed, execution)?;
                    if !references.is_empty() {
                        return Err(WorkspaceError::Validation(format!(
                            "Cannot delete referenced profiles. Update these references first: {}",
                            references.join(", ")
                        )));
                    }
                }
                Ok(())
            };
            validate().map_err(|error| spark_storage::StorageError::SettingsValidation {
                path: path.clone(),
                reason: error.to_string(),
            })
        },
    )?;
    let section = if execution {
        "execution_profiles"
    } else {
        "llm_profiles"
    };
    let view = if execution {
        execution_profiles_view(settings)?
    } else {
        llm_profiles_view(settings)?
    };
    Ok(json!({section: view}))
}

fn profile_references(
    settings: &SparkSettings,
    removed: &BTreeSet<String>,
    execution: bool,
) -> WorkspaceResult<Vec<String>> {
    let mut references = Vec::new();
    let key = if execution {
        "execution_profile_id"
    } else {
        "llm_profile"
    };
    let mut inspect = |value: Value, label: String| {
        find_references(&value, &label, key, removed, &mut references)
    };
    let core = settings.config_dir.join("spark.toml");
    inspect(
        json!(read_settings_document(&core)?.values),
        core.display().to_string(),
    );
    // Inspect authored metadata only, including unregistered project directories.
    // Turns, checkpoints, runtime sessions and historical snapshots are not references.
    visit_metadata(&settings.projects_dir, &mut |path| {
        if path.file_name().is_some_and(|name| name == "project.toml") {
            inspect(
                json!(read_settings_document(path)?.values),
                path.display().to_string(),
            );
        } else if path
            .file_name()
            .is_some_and(|name| name == "conversation.json")
        {
            let bytes = std::fs::read(path).map_err(|error| {
                WorkspaceError::Validation(format!(
                    "Cannot check references in {}: {error}",
                    path.display()
                ))
            })?;
            let record: Value = serde_json::from_slice(&bytes).map_err(|_| {
                WorkspaceError::Validation(format!(
                    "Invalid conversation metadata in {}; repair it before deleting profiles.",
                    path.display()
                ))
            })?;
            inspect(
                record.get("model_settings").cloned().unwrap_or(Value::Null),
                format!("{}.model_settings", path.display()),
            );
        }
        Ok(())
    })?;
    for trigger in spark_storage::list_trigger_definitions(&settings.config_dir)? {
        inspect(
            json!(trigger.action),
            format!("trigger {}.action", trigger.id),
        );
    }
    for name in attractor_api::list_logical_flow_names(&settings.flows_dir).map_err(|_| {
        WorkspaceError::Validation("Cannot enumerate flows to check profile references.".into())
    })? {
        let source =
            attractor_dsl::load_flow_content(&settings.flows_dir, &name).map_err(|_| {
                WorkspaceError::Validation(format!(
                    "Cannot read flow {name} to check profile references."
                ))
            })?;
        let flow = attractor_dsl::parse_flow_definition(&source).map_err(|_| {
            WorkspaceError::Validation(format!(
                "Invalid flow {name}; repair it before deleting profiles."
            ))
        })?;
        inspect(json!(flow), format!("flow {name}"));
    }
    Ok(references)
}

fn find_references(
    value: &Value,
    label: &str,
    key: &str,
    removed: &BTreeSet<String>,
    result: &mut Vec<String>,
) {
    match value {
        Value::Object(fields) => {
            for (field, value) in fields {
                let path = format!("{label}.{field}");
                if (field == key
                    || (key == "llm_profile"
                        && field == unified_llm_adapter::RUNTIME_LAUNCH_PROFILE_KEY))
                    && value.as_str().is_some_and(|id| removed.contains(id.trim()))
                {
                    result.push(path);
                } else {
                    find_references(value, &path, key, removed, result);
                }
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                find_references(value, &format!("{label}[{index}]"), key, removed, result);
            }
        }
        _ => (),
    }
}

fn visit_metadata(
    projects_dir: &Path,
    visit: &mut impl FnMut(&Path) -> WorkspaceResult<()>,
) -> WorkspaceResult<()> {
    for project in subdirectories(projects_dir)? {
        visit(&project.join("project.toml"))?;
        for conversation in subdirectories(&project.join("conversations"))? {
            let path = conversation.join("conversation.json");
            if path.exists() {
                visit(&path)?;
            }
        }
    }
    Ok(())
}

fn subdirectories(path: &Path) -> WorkspaceResult<Vec<std::path::PathBuf>> {
    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => {
            return Err(WorkspaceError::Validation(format!(
                "Cannot check profile references in {}.",
                path.display()
            )))
        }
    };
    let mut directories = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| {
            WorkspaceError::Validation("Cannot enumerate profile references.".into())
        })?;
        let kind = entry
            .file_type()
            .map_err(|_| WorkspaceError::Validation("Cannot inspect profile references.".into()))?;
        if kind.is_dir() {
            directories.push(entry.path());
        }
    }
    Ok(directories)
}
