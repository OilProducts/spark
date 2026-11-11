//! Profile sections of the installation settings document, edited through
//! its shared revision/lock boundary.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
use spark_storage::settings::{read_settings_document, update_settings_sections, SettingsDocument};
use unified_llm_adapter::profiles::{parse_llm_profiles, LlmProfile, ProcessLlmProfileEnvironment};

use crate::{WorkspaceError, WorkspaceResult};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionProfilesUpdate {
    pub profiles: Vec<attractor_execution::ExecutionProfile>,
    pub default_execution_profile_id: Option<String>,
}

pub fn llm_profiles_view(document: &SettingsDocument) -> Value {
    let profiles = match parse_llm_profiles(&document.values) {
        Ok(profiles) => profiles,
        Err(error) => {
            let stored = profile_stored_values(&document.values, "llm_profiles");
            return json!({"scope":"workspace", "revision":document.revision, "stored":stored,
                "effective":null, "repair_defaults":[], "credential_status":{}, "restart_fields":[], "validation_errors":[error.to_string()]});
        }
    };
    json!({"scope": "workspace", "revision": document.revision,
        "stored": profiles.values().collect::<Vec<_>>(),
        "credential_status": profiles.iter().map(|(id, profile)|
            (id.clone(), if profile.configured(&ProcessLlmProfileEnvironment) { "configured" } else { "missing" }))
            .collect::<std::collections::BTreeMap<_, _>>(),
        "restart_fields": [], "validation_errors": []})
}

pub fn execution_profiles_view(document: &SettingsDocument) -> Value {
    let graph = match attractor_execution::profile::stored_execution_profiles(&document.values) {
        Ok(graph) => graph,
        Err(error) => {
            return json!({"scope": "workspace", "revision": document.revision,
            "stored": {"profiles":profile_stored_values(&document.values, "execution_profiles"), "default_execution_profile_id":document.values.get("execution_defaults").and_then(|value| value.get("execution_profile_id"))}, "effective":null, "repair_defaults":{"profiles":[], "default_execution_profile_id":null}, "restart_fields": [], "validation_errors": [error.to_string()]})
        }
    };
    json!({"scope": "workspace", "revision": document.revision,
        "stored": {"profiles": graph.profiles.values().collect::<Vec<_>>(),
            "default_execution_profile_id": graph.default_execution_profile_id},
        "restart_fields": [], "validation_errors": []})
}

fn profile_stored_values(values: &toml::Table, section: &str) -> Value {
    if let Some(profiles) = values.get(section) {
        if !profiles.is_table() {
            return json!(profiles);
        }
    }
    json!(values
        .get(section)
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

/// The `spark.toml` sections an LLM profile update replaces.
pub fn llm_candidate(profiles: &[LlmProfile]) -> WorkspaceResult<toml::Table> {
    let values = toml::Table::from_iter([(
        "llm_profiles".into(),
        toml::Value::Table(profiles_table(
            profiles
                .iter()
                .map(|profile| (profile.id.as_str(), profile)),
        )?),
    )]);
    parse_llm_profiles(&values).map_err(|error| WorkspaceError::Validation(error.to_string()))?;
    Ok(values)
}

/// The `spark.toml` sections an execution profile update replaces.
pub fn execution_candidate(value: &ExecutionProfilesUpdate) -> WorkspaceResult<toml::Table> {
    let mut defaults = toml::Table::new();
    if let Some(id) = &value.default_execution_profile_id {
        defaults.insert("execution_profile_id".into(), id.clone().into());
    }
    let values = toml::Table::from_iter([
        ("execution_defaults".into(), toml::Value::Table(defaults)),
        (
            "execution_profiles".into(),
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

/// Check `candidate` sections as they would land in the current document,
/// without writing.
pub fn validate_profile_sections(
    settings: &SparkSettings,
    candidate: toml::Table,
    execution: bool,
) -> WorkspaceResult<()> {
    let mut values = read_settings_document(&settings.config_dir.join("spark.toml"))?.values;
    values.extend(candidate);
    validate_profile_candidate(settings, &values, execution)
}

pub fn update_profiles(
    settings: &SparkSettings,
    revision: &str,
    candidate: toml::Table,
    execution: bool,
) -> WorkspaceResult<Value> {
    let _references = spark_storage::settings::lock_profile_references(&settings.config_dir)?;
    let path = settings.config_dir.join("spark.toml");
    let document = update_settings_sections(
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
    Ok(if execution {
        json!({"execution_profiles": execution_profiles_view(&document)})
    } else {
        json!({"llm_profiles": llm_profiles_view(&document)})
    })
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
    // The candidate document holds the installation defaults that reference profiles.
    inspect(
        json!(candidate),
        settings.config_dir.join("spark.toml").display().to_string(),
    );
    let registry = spark_storage::ProjectRegistry::new(settings.data_dir.clone());
    let projects = registry.list_project_records()?;
    let triggers = spark_storage::TriggerRepositories::from_settings(settings);
    for project in &projects {
        let stored = registry.project_settings(&project.project_path)?;
        inspect(
            json!({
                "model_settings": stored.model_settings,
                "execution_profile_id": stored.execution_profile_id,
            }),
            format!("project defaults of {}", project.project_path),
        );
        for trigger in triggers.definitions(&project.project_path)?.list()? {
            inspect(
                json!(trigger.action),
                format!("trigger {}.action in {}", trigger.id, project.project_path),
            );
        }
    }
    // Bundled flows, then each registered project's flows with its org
    // files layered over them.
    let scopes = std::iter::once((attractor_api::flow_roots(settings, None), String::new()))
        .chain(projects.iter().map(|project| {
            (
                attractor_api::flow_roots(settings, Some(&project.project_path)),
                format!(" in {}", project.project_path),
            )
        }))
        .collect::<Vec<_>>();
    for (roots, scope) in &scopes {
        let names = roots.list_flow_names().map_err(|_| {
            WorkspaceError::Validation("Cannot enumerate flows to check profile references.".into())
        })?;
        for name in names {
            let mut flow = roots
                .read_named_flow_source(&name)
                .and_then(|source| attractor_dsl::resolve_flow_roles(&source.flow, roots))
                .map_err(|error| {
                    WorkspaceError::Validation(if error.status_code() == 422 {
                        format!("Invalid flow {name}{scope}; repair it before deleting profiles.")
                    } else {
                        format!("Cannot read flow {name}{scope} to check profile references.")
                    })
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
            inspect(json!(flow), format!("flow {name}{scope}"));
        }
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
