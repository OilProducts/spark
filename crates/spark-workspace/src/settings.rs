use serde::Deserialize;
use serde_json::{json, Value};
use spark_common::settings::{
    resolve_model_settings, ModelSettings, ModelSettingsSource, RuntimeSettings, SparkSettings,
};
use spark_storage::settings::{read_settings_document, update_settings_section, SettingsDocument};

use crate::{WorkspaceError, WorkspaceResult};
use spark_common::agent_settings::{EnvTemplate, HIDDEN_ENV_VALUE};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSettingsUpdate {
    pub expected_revision: String,
    #[serde(flatten)]
    pub section: WorkspaceSettingsSection,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "section", content = "value", rename_all = "snake_case")]
pub enum WorkspaceSettingsSection {
    Connections(spark_common::settings::ConnectionSettings),
    Providers(spark_common::provider_settings::ProviderConnections),
    /// Every agents setting but the ACP registrations, which are saved one
    /// at a time as `acp_harness`.
    Agents(spark_common::agent_settings::SessionConfig),
    /// Adds or replaces one ACP registration, or removes it when
    /// `registration` is null. Environment values sent as
    /// [`HIDDEN_ENV_VALUE`] keep their stored values.
    AcpHarness {
        id: String,
        registration: Option<spark_common::agent_settings::AcpHarness>,
    },
    LlmProfiles(Vec<unified_llm_adapter::LlmProfile>),
    ExecutionProfiles(crate::profile_settings::ExecutionProfilesUpdate),
    ClientPreferences {
        client_id: String,
        preferences: spark_common::settings::ClientPreferences,
    },
    Runtime(RuntimeSettings),
    Models(ModelSettings),
    /// `None` turns utility inference off.
    UtilityModels(Option<ModelSettings>),
    ConversationModels {
        conversation_id: String,
        project_path: String,
        #[serde(
            default,
            deserialize_with = "spark_common::settings::deserialize_nullable_patch"
        )]
        model_settings: Option<Option<ModelSettings>>,
    },
    ProjectExecution {
        project_path: String,
        #[serde(
            default,
            deserialize_with = "spark_common::settings::deserialize_nullable_patch"
        )]
        execution_profile_id: Option<Option<String>>,
    },
    ProjectModels {
        project_path: String,
        #[serde(
            default,
            deserialize_with = "spark_common::settings::deserialize_nullable_patch"
        )]
        model_settings: Option<Option<ModelSettings>>,
    },
}

pub fn workspace_settings(settings: &SparkSettings) -> WorkspaceResult<Value> {
    let execution_placement = attractor_api::execution_placement_settings(settings);
    let path = settings.config_dir.join("spark.toml");
    let document = read_settings_document(&path)?;
    let env = spark_common::paths::ProcessEnvironment;
    let mut result = json!({"execution_placement": execution_placement.body});
    for section in ["runtime", "models", "providers", "agents", "connections"] {
        let repair_defaults = match section {
            "runtime" => json!(RuntimeSettings::default()),
            "models" => json!(ModelSettings::default()),
            "providers" => json!(spark_common::provider_settings::ProviderConnections::default()),
            "agents" => json!(spark_common::agent_settings::SessionConfig::default()),
            _ => json!(spark_common::settings::ConnectionSettings::default()),
        };
        let view = (|| -> WorkspaceResult<Value> {
            let stored = match section {
                "runtime" => json!(document
                    .section::<RuntimeSettings>(&path, section)?
                    .unwrap_or_default()),
                "models" => json!(document.section::<ModelSettings>(&path, section)?),
                "connections" => json!(document
                    .section::<spark_common::settings::ConnectionSettings>(&path, section)?
                    .unwrap_or_default()),
                "providers" => json!(document
                    .section::<spark_common::provider_settings::ProviderConnections>(
                        &path, section
                    )?
                    .unwrap_or_default()),
                _ => json!(spark_storage::settings::agents_section(&path, &document)?.0),
            };
            let mut view = json!({"scope":"workspace", "revision":document.revision,
                "stored":stored, "effective":null, "sources":{}, "restart_fields":[], "validation_errors":[]});
            // Preserve typed stored values even when domain validation fails.
            let resolved = (|| -> WorkspaceResult<()> {
                match section {
                    "runtime" => {
                        view["effective"] = json!(RuntimeSettings {
                            runs_dir: settings.runs_dir.clone(),
                            ui_dir: settings.ui_dir.clone(),
                            project_roots: settings.project_roots.clone()
                        });
                        view["restart_fields"] = json!(["runs_dir", "ui_dir", "project_roots"]);
                        for key in ["runs_dir", "ui_dir", "project_roots"] {
                            view["sources"][key] =
                                json!(startup_source(settings, &format!("{section}.{key}")));
                        }
                        runtime_section(&path, &document)?;
                    }
                    "models" => {
                        let models = document
                            .section::<ModelSettings>(&path, section)?
                            .unwrap_or_default();
                        validate_model_settings(settings, &models)?;
                        view["effective"] = json!(models);
                    }
                    "connections" => {
                        view["running_server"] = json!(settings.connections);
                        let connections = document
                            .section::<spark_common::settings::ConnectionSettings>(&path, section)?
                            .unwrap_or_default();
                        let mut effective = settings.connections.clone();
                        let environment_target = std::env::var("SPARK_API_BASE_URL")
                            .is_ok_and(|value| !value.trim().is_empty());
                        let client_config_dir = if environment_target {
                            settings.config_dir.clone()
                        } else {
                            spark_common::settings::resolve_settings_with_env(
                                &Default::default(),
                                &env,
                            )
                            .map_err(|error| WorkspaceError::Validation(error.to_string()))?
                            .config_dir
                        };
                        let (target, source) =
                            spark_storage::settings::resolve_client_api_base_url(
                                &client_config_dir,
                                &env,
                            )?;
                        effective.client_api_base_url = Some(target);
                        view["client_config_dir"] = if environment_target {
                            Value::Null
                        } else {
                            json!(client_config_dir)
                        };
                        view["effective"] = json!(effective);
                        view["running_server"] = json!(settings.connections);
                        view["restart_fields"] = json!(["server_host", "server_port"]);
                        view["sources"] = json!({"server_host":startup_source(settings, "connections.server_host"), "server_port":startup_source(settings, "connections.server_port"),
                            "client_api_base_url": source});
                        connections
                            .validate()
                            .map_err(|error| WorkspaceError::Validation(error.to_string()))?;
                    }
                    "providers" => {
                        let providers = document
                            .section::<spark_common::provider_settings::ProviderConnections>(
                                &path, section,
                            )?
                            .unwrap_or_default();
                        let effective = providers
                            .resolve(&env)
                            .map_err(|error| WorkspaceError::Validation(error.to_string()))?;
                        view["credential_status"] = json!(effective.credential_status(&env));
                        view["effective"] = json!(effective);
                        for (provider, fields) in
                            view["effective"].as_object().cloned().unwrap_or_default()
                        {
                            for key in fields
                                .as_object()
                                .into_iter()
                                .flat_map(|fields| fields.keys())
                            {
                                let suffix = match key.as_str() {
                                    "api_key_env" => "API_KEY",
                                    "organization" => "ORG_ID",
                                    "project" => "PROJECT_ID",
                                    other => other,
                                };
                                let variable = if provider == "gemini"
                                    && key == "api_key_env"
                                    && fields[key] == "GOOGLE_API_KEY"
                                {
                                    "GOOGLE_API_KEY".to_owned()
                                } else {
                                    format!("{}_{}", provider.to_uppercase(), suffix.to_uppercase())
                                };
                                view["sources"][format!("{provider}.{key}")] =
                                    json!(value_source(&stored[&provider][key], &variable));
                            }
                        }
                    }
                    _ => {
                        let mut agents =
                            spark_storage::settings::agents_section(&path, &document)?.0;
                        agents
                            .validate()
                            .map_err(|error| WorkspaceError::Validation(error.to_string()))?;
                        agents.native = agents
                            .native
                            .resolve(&env)
                            .map_err(|error| WorkspaceError::Validation(error.to_string()))?;
                        agents.native.retain_startup_paths(&settings.agents.native);
                        view["effective"] = json!(agents);
                        view["restart_fields"] = json!([
                            "native.codex_runtime_root",
                            "native.codex_seed_dir",
                            "native.claude_config_dir"
                        ]);
                        for (key, variable) in [
                            ("codex_binary", "SPARK_CODEX_APP_SERVER_BIN"),
                            ("claude_binary", "SPARK_CLAUDE_CODE_BIN"),
                            (
                                "claude_permission_mode",
                                "SPARK_CLAUDE_CODE_PERMISSION_MODE",
                            ),
                            ("codex_jsonrpc_trace", "SPARK_DEBUG_CODEX_JSONRPC"),
                            ("agent_trace", "SPARK_DEBUG_AGENT_TRACE"),
                        ] {
                            view["sources"][format!("native.{key}")] =
                                json!(if ["codex_jsonrpc_trace", "agent_trace"].contains(&key)
                                    && std::env::var(variable).is_ok()
                                {
                                    format!("environment: {variable}")
                                } else {
                                    value_source(&stored["native"][key], variable)
                                });
                        }
                        for key in ["codex_runtime_root", "codex_seed_dir", "claude_config_dir"] {
                            view["sources"][format!("native.{key}")] =
                                json!(startup_source(settings, &format!("agents.native.{key}")));
                        }
                        view["policies"] = json!({"codex_service_tier":"standard", "codex_approval_policy":"never", "codex_sandbox":"danger-full-access"});
                    }
                }
                Ok(())
            })();
            if let Err(error) = resolved {
                view["validation_errors"] = json!([error.to_string()]);
            }
            Ok(view)
        })();
        result[section] = view.unwrap_or_else(|error| json!({"scope":"workspace", "revision":document.revision,
            "stored":document.values.get(section), "effective":null, "sources":{}, "restart_fields":[], "validation_errors":[error.to_string()]}));
        if !result[section]["validation_errors"]
            .as_array()
            .is_some_and(Vec::is_empty)
        {
            result[section]["repair_defaults"] = repair_defaults;
        }
        if section == "models" {
            result[section]["source"] = json!("workspace");
        }
    }
    result["runtime"]["active_startup"] = json!(RuntimeSettings {
        runs_dir: settings.runs_dir.clone(),
        ui_dir: settings.ui_dir.clone(),
        project_roots: settings.project_roots.clone()
    });
    result["connections"]["running_server"] = json!(settings.connections);
    // Registrations are shown apart, with their environment values hidden.
    for view in ["stored", "effective"] {
        if let Some(agents) = result["agents"][view].as_object_mut() {
            agents.remove("acp_harnesses");
        }
    }
    result["agents"]["acp_harnesses"] = acp_harnesses_view(&path, &document);
    result["agents"]["active_startup"] = json!({"codex_runtime_root":settings.agents.native.codex_runtime_root, "codex_seed_dir":settings.agents.native.codex_seed_dir, "claude_config_dir":settings.agents.native.claude_config_dir});
    let mut utility = json!({"scope":"workspace", "source":"workspace", "revision":document.revision,
        "stored":document.values.get("utility_models"), "effective":null, "sources":{}, "restart_fields":[], "validation_errors":[]});
    match utility_section(settings, &path, &document) {
        Ok(value) => {
            utility["stored"] = json!(value);
            utility["effective"] = json!(value);
        }
        Err(error) => {
            utility["validation_errors"] = json!([error.to_string()]);
            utility["repair_defaults"] = json!(ModelSettings::default());
        }
    }
    result["utility_models"] = utility;
    result["llm_profiles"] = crate::profile_settings::llm_profiles_view(&document);
    result["execution_profiles"] = crate::profile_settings::execution_profiles_view(&document);
    Ok(result)
}

/// Each stored ACP registration as written, with its environment values and
/// instruction-channel environment templates hidden, and why it cannot be
/// used when it cannot.
fn acp_harnesses_view(path: &std::path::Path, document: &SettingsDocument) -> Value {
    let misconfigured = spark_storage::settings::agents_section(path, document)
        .map(|(_, misconfigured)| misconfigured)
        .unwrap_or_default();
    let hide = |env: Option<&mut Value>, stand_in: Value| match env {
        Some(Value::Object(values)) => values
            .values_mut()
            .for_each(|value| *value = stand_in.clone()),
        Some(other) => *other = json!({}),
        None => {}
    };
    let registrations = stored_registrations(document)
        .into_iter()
        .flatten()
        .map(|(id, stored)| {
            let mut registration = if stored.is_table() {
                json!(stored)
            } else {
                json!({})
            };
            hide(registration.get_mut("env"), json!(HIDDEN_ENV_VALUE));
            hide(
                registration
                    .get_mut("system_instructions")
                    .and_then(|channel| channel.get_mut("env")),
                json!({ "text": HIDDEN_ENV_VALUE }),
            );
            let view = json!({"registration": registration, "error": misconfigured.get(id)});
            (id.clone(), view)
        })
        .collect::<serde_json::Map<_, _>>();
    Value::Object(registrations)
}

fn stored_registrations(document: &SettingsDocument) -> Option<&toml::Table> {
    document
        .values
        .get("agents")?
        .get("acp_harnesses")?
        .as_table()
}

/// The current document, if it is still the one the client edited.
fn current_document(
    path: &std::path::Path,
    expected_revision: &str,
) -> WorkspaceResult<SettingsDocument> {
    let document = read_settings_document(path)?;
    if document.revision != expected_revision {
        return Err(spark_storage::StorageError::SettingsConflict { path: path.into() }.into());
    }
    Ok(document)
}

/// `registration` for `id`, with hidden environment values taken from the
/// stored registration of the same ID, checked.
fn acp_registration(
    document: &SettingsDocument,
    id: &str,
    mut registration: spark_common::agent_settings::AcpHarness,
) -> WorkspaceResult<spark_common::agent_settings::AcpHarness> {
    let invalid =
        |reason: String| WorkspaceError::Validation(format!("ACP harness {id:?}: {reason}"));
    spark_common::agent_settings::validate_harness_id(id).map_err(invalid)?;
    let stored = stored_registrations(document).and_then(|stored| stored.get(id));
    let saved = |section: &[&str], key: &str| {
        section
            .iter()
            .fold(stored, |value, part| {
                value.and_then(|value| value.get(*part))
            })
            .and_then(|value| value.get(key))
            .cloned()
            .ok_or_else(|| {
                invalid(format!(
                    "enter a value for {key}; its saved value was not found."
                ))
            })
    };
    for (key, value) in &mut registration.env {
        if value == HIDDEN_ENV_VALUE {
            *value = saved(&["env"], key)?
                .as_str()
                .ok_or_else(|| invalid(format!("enter a value for {key}.")))?
                .to_owned();
        }
    }
    if let Some(channel) = registration.system_instructions.as_mut() {
        for (key, template) in &mut channel.env {
            if *template == EnvTemplate::Text(HIDDEN_ENV_VALUE.into()) {
                *template = saved(&["system_instructions", "env"], key)?
                    .try_into()
                    .map_err(|_| invalid(format!("enter a value for {key}.")))?;
            }
        }
    }
    registration.validate().map_err(invalid)?;
    Ok(registration)
}

fn startup_source(settings: &SparkSettings, key: &str) -> String {
    format!(
        "{}; retained until restart",
        settings
            .startup_sources
            .get(key)
            .map(String::as_str)
            .unwrap_or("startup selection")
    )
}

fn value_source(stored: &Value, variable: &str) -> String {
    spark_common::settings::setting_source(
        false,
        &spark_common::paths::ProcessEnvironment,
        variable,
        !stored.is_null(),
    )
}

fn runtime_section(
    path: &std::path::Path,
    document: &SettingsDocument,
) -> WorkspaceResult<RuntimeSettings> {
    let runtime: RuntimeSettings = document.section(path, "runtime")?.unwrap_or_default();
    runtime
        .validate()
        .map_err(|error| WorkspaceError::Validation(error.to_string()))?;
    Ok(runtime)
}

pub fn validate_workspace_settings_update(
    settings: &SparkSettings,
    request: &WorkspaceSettingsUpdate,
) -> WorkspaceResult<()> {
    match &request.section {
        WorkspaceSettingsSection::Providers(value) => value
            .validate()
            .map_err(|error| WorkspaceError::Validation(error.to_string())),
        WorkspaceSettingsSection::Agents(value) => {
            if !value.acp_harnesses.is_empty() {
                return Err(WorkspaceError::Validation(
                    "Save ACP harnesses one at a time, with the acp_harness section.".into(),
                ));
            }
            value
                .validate()
                .map_err(|error| WorkspaceError::Validation(error.to_string()))
        }
        WorkspaceSettingsSection::AcpHarness { id, registration } => {
            let path = settings.config_dir.join("spark.toml");
            let document = read_settings_document(&path)?;
            match registration {
                Some(registration) => {
                    acp_registration(&document, id, registration.clone()).map(|_| ())
                }
                None => Ok(()),
            }
        }
        WorkspaceSettingsSection::Connections(value) => value
            .validate()
            .map_err(|error| WorkspaceError::Validation(error.to_string())),
        WorkspaceSettingsSection::LlmProfiles(value) => {
            crate::profile_settings::validate_profile_sections(
                settings,
                crate::profile_settings::llm_candidate(value)?,
                false,
            )
        }
        WorkspaceSettingsSection::ExecutionProfiles(value) => {
            crate::profile_settings::validate_profile_sections(
                settings,
                crate::profile_settings::execution_candidate(value)?,
                true,
            )
        }
        WorkspaceSettingsSection::ClientPreferences {
            client_id,
            preferences,
        } => spark_common::settings::validate_client_id(client_id)
            .and_then(|_| preferences.validate())
            .map_err(|error| WorkspaceError::Validation(error.to_string())),
        WorkspaceSettingsSection::ProjectExecution {
            execution_profile_id,
            ..
        } => {
            if let Some(value) = execution_profile_id {
                crate::WorkspaceProjectService::new(settings.clone())
                    .validate_project_execution_profile_id(value.as_deref())?;
            }
            Ok(())
        }
        WorkspaceSettingsSection::Models(value) => validate_model_settings(settings, value),
        WorkspaceSettingsSection::UtilityModels(value) => value
            .as_ref()
            .map(|value| validate_model_settings(settings, value))
            .transpose()
            .map(|_| ()),
        WorkspaceSettingsSection::ProjectModels { model_settings, .. }
        | WorkspaceSettingsSection::ConversationModels { model_settings, .. } => model_settings
            .as_ref()
            .and_then(Option::as_ref)
            .map(|value| validate_model_settings(settings, value))
            .transpose()
            .map(|_| ()),
        WorkspaceSettingsSection::Runtime(value) => value
            .validate()
            .map_err(|error| WorkspaceError::Validation(error.to_string())),
    }
}

pub fn update_workspace_settings(
    settings: &SparkSettings,
    store: &std::sync::Arc<spark_storage::ConversationStore>,
    request: WorkspaceSettingsUpdate,
) -> WorkspaceResult<Value> {
    // Delegated project/conversation/profile operations acquire this themselves.
    let _references = if matches!(
        &request.section,
        WorkspaceSettingsSection::Models(_)
            | WorkspaceSettingsSection::UtilityModels(_)
            | WorkspaceSettingsSection::ProjectModels { .. }
    ) {
        Some(spark_storage::settings::lock_profile_references(
            &settings.config_dir,
        )?)
    } else {
        None
    };
    validate_workspace_settings_update(settings, &request)?;
    let path = settings.config_dir.join("spark.toml");
    let (section, value) = match request.section {
        WorkspaceSettingsSection::Providers(value) => ("providers", toml::Value::try_from(value)),
        WorkspaceSettingsSection::Agents(value) => {
            // The registrations stay as stored, valid or not.
            let registrations =
                stored_registrations(&current_document(&path, &request.expected_revision)?)
                    .cloned();
            let mut agents = toml::Value::try_from(value);
            if let (Ok(toml::Value::Table(agents)), Some(registrations)) =
                (agents.as_mut(), registrations)
            {
                agents.insert("acp_harnesses".into(), toml::Value::Table(registrations));
            }
            ("agents", agents)
        }
        WorkspaceSettingsSection::AcpHarness { id, registration } => {
            let document = current_document(&path, &request.expected_revision)?;
            let mut agents = document
                .values
                .get("agents")
                .and_then(toml::Value::as_table)
                .cloned()
                .unwrap_or_default();
            let mut registrations = stored_registrations(&document).cloned().unwrap_or_default();
            match registration {
                Some(registration) => {
                    let registration = acp_registration(&document, &id, registration)?;
                    registrations.insert(
                        id,
                        toml::Value::try_from(registration).map_err(|_| {
                            WorkspaceError::Validation("Invalid ACP harness.".into())
                        })?,
                    );
                }
                None => {
                    if registrations.remove(&id).is_none() {
                        return Err(WorkspaceError::NotFound(format!(
                            "ACP harness {id} is not registered."
                        )));
                    }
                }
            }
            if registrations.is_empty() {
                agents.remove("acp_harnesses");
            } else {
                agents.insert("acp_harnesses".into(), toml::Value::Table(registrations));
            }
            ("agents", Ok(toml::Value::Table(agents)))
        }
        WorkspaceSettingsSection::Connections(value) => {
            ("connections", toml::Value::try_from(value))
        }
        WorkspaceSettingsSection::LlmProfiles(value) => {
            return crate::profile_settings::update_profiles(
                settings,
                &request.expected_revision,
                crate::profile_settings::llm_candidate(&value)?,
                false,
            )
        }
        WorkspaceSettingsSection::ExecutionProfiles(value) => {
            return crate::profile_settings::update_profiles(
                settings,
                &request.expected_revision,
                crate::profile_settings::execution_candidate(&value)?,
                true,
            )
        }
        WorkspaceSettingsSection::ClientPreferences {
            client_id,
            preferences,
        } => {
            let path =
                spark_storage::settings::client_settings_path(&settings.config_dir, &client_id)?;
            let document = update_settings_section(
                &path,
                &request.expected_revision,
                "preferences",
                Some(toml::Value::try_from(preferences).map_err(|_| {
                    WorkspaceError::Validation("Invalid client preferences.".into())
                })?),
                |values| validate_client_document(&path, values),
            )?;
            return client_preferences_view(&path, &client_id, &document);
        }
        WorkspaceSettingsSection::ProjectExecution {
            project_path,
            execution_profile_id,
        } => {
            if execution_profile_id.is_none() {
                let view = project_model_settings_view(settings, &project_path)?;
                if view["execution"]["revision"].as_str()
                    != Some(request.expected_revision.as_str())
                {
                    return Err(WorkspaceError::Conflict(
                        "Project settings changed; reload before saving.".into(),
                    ));
                }
                return Ok(view);
            }
            crate::WorkspaceProjectService::new(settings.clone()).update_project_state(
                store,
                crate::ProjectStateUpdate {
                    project_path: project_path.clone(),
                    execution_profile_id,
                    expected_revision: Some(request.expected_revision),
                    ..Default::default()
                },
            )?;
            return project_model_settings_view(settings, &project_path);
        }
        WorkspaceSettingsSection::Runtime(value) => ("runtime", toml::Value::try_from(value)),
        WorkspaceSettingsSection::Models(value) => ("models", toml::Value::try_from(value)),
        WorkspaceSettingsSection::UtilityModels(value) => {
            let value = value
                .map(toml::Value::try_from)
                .transpose()
                .map_err(|_| WorkspaceError::Validation("Invalid model settings.".into()))?;
            // Validated above while holding the profile reference lock.
            update_settings_section(
                &path,
                &request.expected_revision,
                "utility_models",
                value,
                |values| spark_storage::settings::validate_core_version(&path, values),
            )?;
            return workspace_settings(settings);
        }
        WorkspaceSettingsSection::ConversationModels {
            conversation_id,
            project_path,
            model_settings,
        } => {
            let service = crate::WorkspaceConversationService::new(settings.clone(), store.clone());
            if model_settings.is_none() {
                let view = service.conversation_settings_view(&conversation_id, &project_path)?;
                if view["models"]["revision"].as_str() != Some(request.expected_revision.as_str()) {
                    return Err(WorkspaceError::Conflict(
                        "Conversation settings changed; reload before saving.".into(),
                    ));
                }
                return Ok(view);
            }
            service.update_conversation_settings(
                &conversation_id,
                crate::ConversationSettingsUpdate {
                    project_path: project_path.clone(),
                    model_settings,
                    expected_revision: Some(request.expected_revision),
                    ..Default::default()
                },
            )?;
            return service.conversation_settings_view(&conversation_id, &project_path);
        }
        WorkspaceSettingsSection::ProjectModels {
            project_path,
            model_settings,
        } => {
            let registry = spark_storage::ProjectRegistry::new(&settings.data_dir);
            if registry.read_project_record(&project_path)?.is_none() {
                return Err(WorkspaceError::NotFound(
                    "Register the project before editing its defaults.".into(),
                ));
            }
            let Some(model_settings) = model_settings else {
                if registry.project_settings(&project_path)?.revision != request.expected_revision {
                    return Err(WorkspaceError::Conflict(
                        "Project settings changed; reload before saving.".into(),
                    ));
                }
                return project_model_settings_view(settings, &project_path);
            };
            // Validated above while holding the profile reference lock.
            registry.update_project_model_settings(
                &project_path,
                &request.expected_revision,
                model_settings.map(|value| json!(value)).as_ref(),
            )?;
            return project_model_settings_view(settings, &project_path);
        }
    };
    let value =
        value.map_err(|_| WorkspaceError::Validation("Invalid settings section.".into()))?;
    update_settings_section(
        &path,
        &request.expected_revision,
        section,
        Some(value),
        |values| {
            spark_storage::settings::validate_core_version(&path, values)?;
            let candidate = toml::Table::from_iter([(section.to_owned(), values[section].clone())]);
            spark_storage::settings::validate_core_sections(&path, &candidate)?;
            if section == "models" {
                let document = SettingsDocument {
                    values: candidate,
                    revision: String::new(),
                };
                let models: ModelSettings = document.section(&path, "models")?.unwrap_or_default();
                validate_model_settings(settings, &models).map_err(|error| {
                    spark_storage::StorageError::SettingsValidation {
                        path: path.clone(),
                        reason: error.to_string(),
                    }
                })?;
            }
            Ok(())
        },
    )?;
    workspace_settings(settings)
}

fn validate_client_document(
    path: &std::path::Path,
    values: &toml::Table,
) -> spark_storage::Result<()> {
    if !matches!(
        values.get("schema_version"),
        None | Some(toml::Value::Integer(1))
    ) {
        return Err(spark_storage::StorageError::SettingsValidation {
            path: path.into(),
            reason: "Unsupported client settings version; use a compatible Spark binary.".into(),
        });
    }
    let document = SettingsDocument {
        values: values.clone(),
        revision: String::new(),
    };
    let preferences: spark_common::settings::ClientPreferences =
        document.section(path, "preferences")?.unwrap_or_default();
    preferences
        .validate()
        .map_err(|error| spark_storage::StorageError::SettingsValidation {
            path: path.into(),
            reason: error.to_string(),
        })
}

fn client_preferences_view(
    path: &std::path::Path,
    client_id: &str,
    document: &SettingsDocument,
) -> WorkspaceResult<Value> {
    validate_client_document(path, &document.values)?;
    let stored: spark_common::settings::ClientPreferences =
        document.section(path, "preferences")?.unwrap_or_default();
    Ok(json!({"preferences": {
        "scope": "client", "client_id": client_id, "revision": document.revision,
        "effective": {"editor_mode": stored.editor_mode.clone().unwrap_or_default(),
            "editor_sidebar_width": stored.editor_sidebar_width.unwrap_or(336),
            "home_sidebar_primary_split_ratio": stored.home_sidebar_primary_split_ratio,
            "show_advanced_controls": stored.show_advanced_controls.unwrap_or(false),
            "expand_child_flows": stored.expand_child_flows.unwrap_or(false),
            "graph_settings_open": stored.graph_settings_open.unwrap_or(false),
            "show_thinking": stored.show_thinking.unwrap_or(true),
            "runs_scope": stored.runs_scope.clone().unwrap_or(spark_common::settings::PresentationScope::Active),
            "run_presentation": stored.run_presentation,
            "flow_node_positions": stored.flow_node_positions,
            "flow_edge_ports": stored.flow_edge_ports,
            "triggers_scope": stored.triggers_scope.clone().unwrap_or(spark_common::settings::PresentationScope::All),
            "appearance": stored.appearance.clone().unwrap_or_default()},
        "stored": stored, "restart_fields": [], "validation_errors": [],
    }}))
}

pub fn client_settings(settings: &SparkSettings, client_id: &str) -> WorkspaceResult<Value> {
    let path = spark_storage::settings::client_settings_path(&settings.config_dir, client_id)?;
    let document = read_settings_document(&path)?;
    client_preferences_view(&path, client_id, &document)
}

/// Load on every read/start so inherited settings never become server-start defaults.
pub fn workspace_model_settings(settings: &SparkSettings) -> WorkspaceResult<ModelSettings> {
    let path = settings.config_dir.join("spark.toml");
    let group = read_settings_document(&path)?
        .section::<ModelSettings>(&path, "models")?
        .unwrap_or_default();
    validate_model_settings(settings, &group)?;
    Ok(group)
}

/// The configured utility model, or `None` when utility inference is off.
pub fn workspace_utility_model_settings(
    settings: &SparkSettings,
) -> WorkspaceResult<Option<ModelSettings>> {
    let path = settings.config_dir.join("spark.toml");
    utility_section(settings, &path, &read_settings_document(&path)?)
}

fn utility_section(
    settings: &SparkSettings,
    path: &std::path::Path,
    document: &SettingsDocument,
) -> WorkspaceResult<Option<ModelSettings>> {
    let group = document.section::<ModelSettings>(path, "utility_models")?;
    if let Some(group) = &group {
        validate_model_settings(settings, group)?;
    }
    Ok(group)
}

pub fn project_model_settings_view(
    settings: &SparkSettings,
    project_path: &str,
) -> WorkspaceResult<Value> {
    let stored =
        spark_storage::ProjectRegistry::new(&settings.data_dir).project_settings(project_path)?;
    let source = if stored.model_settings.is_some() {
        "project"
    } else {
        "workspace"
    };
    let mut models = json!({"scope":"project", "project_path":project_path, "revision":stored.revision,
        "stored":stored.model_settings, "effective":null, "source":source, "restart_fields":[], "validation_errors":[]});
    let resolved = (|| -> WorkspaceResult<Value> {
        let stored: Option<ModelSettings> = stored
            .model_settings
            .clone()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|_| {
                WorkspaceError::Validation(
                    "Invalid project model_settings; check field names and types.".into(),
                )
            })?;
        let workspace = workspace_model_settings(settings)?;
        let (effective, _) = resolve_model_settings(&workspace, stored.as_ref(), None)
            .map_err(|error| WorkspaceError::Validation(error.to_string()))?;
        validate_model_settings(settings, effective)?;
        Ok(json!(effective))
    })();
    match resolved {
        Ok(effective) => models["effective"] = effective,
        Err(error) => {
            models["validation_errors"] = json!([error.to_string()]);
            models["repair_defaults"] = json!(ModelSettings::default());
        }
    }
    let mut execution = json!({"scope":"project", "project_path":project_path, "revision":stored.revision,
        "stored":stored.execution_profile_id, "effective":null, "source":if stored.execution_profile_id.is_some() { "project" } else { "workspace" }, "restart_fields":[], "validation_errors":[]});
    let resolved = (|| -> WorkspaceResult<Value> {
        let selection = attractor_execution::resolve_execution_profile_by_id(
            settings,
            None,
            stored.execution_profile_id.as_deref(),
            None,
        )
        .map_err(|error| WorkspaceError::Validation(error.to_string()))?;
        Ok(json!(selection.selected_profile_id))
    })();
    match resolved {
        Ok(effective) => execution["effective"] = effective,
        Err(error) => execution["validation_errors"] = json!([error.to_string()]),
    }
    Ok(json!({"models":models, "execution":execution}))
}

/// A conversation's own settings win, then its standing role's file
/// (`role`, unset fields `None`), then project defaults.
pub fn conversation_model_settings(
    settings: &SparkSettings,
    project_path: &str,
    role: Option<&ModelSettings>,
    conversation: Option<&ModelSettings>,
) -> WorkspaceResult<(ModelSettings, ModelSettingsSource)> {
    let (defaults, defaults_source) =
        spark_agent_adapter::config::read_project_model_defaults(settings, project_path)
            .map_err(WorkspaceError::Validation)?;
    // A role naming its provider or profile starts from it, not from the project's model.
    let role = role.map(|role| {
        let mut group = if role.provider.is_some() || role.llm_profile.is_some() {
            ModelSettings {
                provider: role.provider.clone(),
                llm_profile: role.llm_profile.clone(),
                model: None,
                reasoning_effort: None,
                thinking: None,
                thinking_budget_tokens: None,
                reasoning_mode: None,
                reasoning_summary: None,
                session_config: Default::default(),
            }
        } else {
            defaults.clone()
        };
        group.model = role.model.clone().or(group.model);
        group.reasoning_effort = role.reasoning_effort.clone().or(group.reasoning_effort);
        group.thinking = role.thinking.clone().or(group.thinking);
        group.thinking_budget_tokens = role.thinking_budget_tokens.or(group.thinking_budget_tokens);
        group.reasoning_mode = role.reasoning_mode.clone().or(group.reasoning_mode);
        group.reasoning_summary = role.reasoning_summary.clone().or(group.reasoning_summary);
        group
    });
    let (effective, source) = if let Some(conversation) = conversation {
        conversation
            .validate()
            .map_err(|error| WorkspaceError::Validation(error.to_string()))?;
        (conversation, ModelSettingsSource::Conversation)
    } else if let Some(role) = &role {
        role.validate()
            .map_err(|error| WorkspaceError::Validation(error.to_string()))?;
        (role, ModelSettingsSource::Role)
    } else {
        (&defaults, defaults_source)
    };
    validate_model_settings(settings, effective)?;
    Ok((effective.clone(), source))
}

pub fn validate_model_settings(
    settings: &SparkSettings,
    value: &ModelSettings,
) -> WorkspaceResult<()> {
    spark_agent_adapter::config::validate_model_settings(settings, value)
        .map_err(WorkspaceError::Validation)
}
