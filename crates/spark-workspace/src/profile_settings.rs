//! Dedicated profile documents, edited through the shared revision/lock boundary.
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
    let profiles = match parse_llm_profiles(&document.values) {
        Ok(profiles) => profiles,
        Err(error) => {
            let stored = profile_stored_values(&document.values);
            return Ok(
                json!({"scope":"workspace", "revision":document.revision, "stored":stored,
                "effective":null, "repair_defaults":[], "credential_status":{}, "restart_fields":[], "validation_errors":[error.to_string()]}),
            );
        }
    };
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
            "stored": {"profiles":profile_stored_values(&document.values), "default_execution_profile_id":document.values.get("defaults").and_then(|value| value.get("execution_profile_id"))}, "effective":null, "repair_defaults":{"profiles":[], "default_execution_profile_id":null}, "restart_fields": [], "validation_errors": [error.to_string()]}))
        }
    };
    Ok(json!({"scope": "workspace", "revision": document.revision,
        "stored": {"profiles": graph.profiles.values().collect::<Vec<_>>(),
            "default_execution_profile_id": graph.default_execution_profile_id},
        "restart_fields": [], "validation_errors": []}))
}

fn profile_stored_values(values: &toml::Table) -> Value {
    if let Some(profiles) = values.get("profiles") {
        if !profiles.is_table() {
            return json!(profiles);
        }
    }
    json!(values
        .get("profiles")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flat_map(|profiles| profiles.iter())
        .map(|(id, value)| {
            let mut value = json!(value);
            if let Some(fields) = value.as_object_mut() {
                fields.insert("id".into(), json!(id));
            }
            value
        })
        .collect::<Vec<_>>())
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
            validate_profile_candidate(settings, values, execution).map_err(|error| {
                spark_storage::StorageError::SettingsValidation {
                    path: path.clone(),
                    reason: error.to_string(),
                }
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

pub fn validate_profile_candidate(
    settings: &SparkSettings,
    candidate: &toml::Table,
    execution: bool,
) -> WorkspaceResult<()> {
    let llm = if execution {
        Default::default()
    } else {
        parse_llm_profiles(candidate)
            .map_err(|error| WorkspaceError::Validation(error.to_string()))?
    };
    let graph = if execution {
        let graph = attractor_execution::profile::parse_execution_profiles(candidate)
            .map_err(|error| WorkspaceError::Validation(error.to_string()))?;
        graph
            .validate_default()
            .map_err(|error| WorkspaceError::Validation(error.to_string()))?;
        Some(graph)
    } else {
        None
    };
    let mut references = Vec::new();
    let key = if execution {
        "execution_profile_id"
    } else {
        "llm_profile"
    };
    let mut inspect = |value: Value, label: String| {
        find_references(
            &value,
            &label,
            key,
            &|id, fields| {
                if let Some(graph) = &graph {
                    return graph
                        .profiles
                        .get(id)
                        .is_none_or(|profile| !profile.enabled);
                }
                let Some(profile) = llm.get(id) else {
                    return true;
                };
                let model = fields
                    .get("model")
                    .or_else(|| fields.get("llm_model"))
                    .or_else(|| fields.get("_attractor.runtime.launch_model"))
                    .and_then(Value::as_str);
                spark_agent_adapter::config::validate_profile_model(profile, model).is_err()
            },
            &mut references,
        )
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
        let mut flow = attractor_dsl::parse_flow_definition(&source).map_err(|_| {
            WorkspaceError::Validation(format!(
                "Invalid flow {name}; repair it before deleting profiles."
            ))
        })?;
        if !execution {
            for node in flow.nodes.values_mut() {
                let selection = node.execution.get_or_insert_with(Default::default);
                let inputs = unified_llm_adapter::LlmResolutionInputs {
                    node_model: selection.llm_model.clone(),
                    node_profile: selection.llm_profile.clone(),
                    fallback_model: flow.defaults.llm_model.clone(),
                    fallback_profile: flow.defaults.llm_profile.clone(),
                    ..Default::default()
                };
                // Resolve authored inheritance only; runtime captures are not references.
                let context = Default::default();
                selection.llm_profile =
                    unified_llm_adapter::resolve_effective_llm_profile(&inputs, &context);
                selection.llm_model =
                    unified_llm_adapter::resolve_effective_llm_model(&inputs, &context);
            }
        }
        inspect(json!(flow), format!("flow {name}"));
    }
    if references.is_empty() {
        Ok(())
    } else {
        Err(WorkspaceError::Validation(format!(
            "Candidate profiles invalidate references. Update these configurations first: {}",
            references.join(", ")
        )))
    }
}

fn find_references(
    value: &Value,
    label: &str,
    key: &str,
    invalid: &impl Fn(&str, &serde_json::Map<String, Value>) -> bool,
    result: &mut Vec<String>,
) {
    match value {
        Value::Object(fields) => {
            for (field, value) in fields {
                let path = format!("{label}.{field}");
                if (field == key
                    || (key == "llm_profile"
                        && field == unified_llm_adapter::RUNTIME_LAUNCH_PROFILE_KEY))
                    && value.as_str().is_some_and(|id| invalid(id.trim(), fields))
                {
                    result.push(path);
                } else {
                    find_references(value, &path, key, invalid, result);
                }
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                find_references(value, &format!("{label}[{index}]"), key, invalid, result);
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
