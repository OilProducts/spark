use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use spark_common::settings::{
    resolve_model_settings, ModelSettings, ModelSettingsSource, RuntimeSettings, SparkSettings,
};
use spark_storage::settings::{read_settings_document, update_settings_section, SettingsDocument};

use crate::{WorkspaceError, WorkspaceResult};

#[derive(Debug, Serialize)]
pub struct RuntimeSettingsView {
    pub scope: &'static str,
    pub revision: String,
    pub stored: RuntimeSettings,
    pub effective: RuntimeSettings,
    pub restart_fields: [&'static str; 4],
}

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
    Agents(spark_common::agent_settings::SessionConfig),
    LlmProfiles(Vec<unified_llm_adapter::LlmProfile>),
    ExecutionProfiles(crate::profile_settings::ExecutionProfilesUpdate),
    ImportClientPreferences {
        client_id: String,
        preferences: spark_common::settings::ClientPreferences,
    },
    ClientPreferences {
        client_id: String,
        preferences: spark_common::settings::ClientPreferences,
    },
    Runtime(RuntimeSettings),
    Models(ModelSettings),
    ImportModels(ModelSettings),
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
    let mut execution = spark_storage::settings::read_execution_configuration(
        &settings.config_dir,
        &spark_common::paths::ProcessEnvironment,
    )?;
    execution
        .agents
        .native
        .retain_startup_paths(&settings.agents.native);
    let connections = document
        .section::<spark_common::settings::ConnectionSettings>(&path, "connections")?
        .unwrap_or_default();
    connections
        .validate()
        .map_err(|error| WorkspaceError::Validation(error.to_string()))?;
    let stored = runtime_section(&path, &document)?;
    let models = document
        .section::<ModelSettings>(&path, "models")?
        .unwrap_or_default();
    validate_model_settings(settings, &models)?;
    let effective = RuntimeSettings {
        runs_dir: Some(settings.runs_dir.clone()),
        flows_dir: Some(settings.flows_dir.clone()),
        ui_dir: settings.ui_dir.clone(),
        project_roots: settings.project_roots.clone(),
    };
    Ok(json!({
        "execution_placement": execution_placement.body,
        "providers": {"scope": "workspace", "revision": document.revision,
            "stored": document.section::<spark_common::provider_settings::ProviderConnections>(&path, "providers")?.unwrap_or_default(),
            "effective": execution.providers, "credential_status": execution.providers.credential_status(&spark_common::paths::ProcessEnvironment),
            "restart_fields": [], "validation_errors": []},
        "agents": {"scope": "workspace", "revision": document.revision,
            "stored": document.section::<spark_common::agent_settings::SessionConfig>(&path, "agents")?.unwrap_or_default(), "effective": execution.agents,
            "policies": {"codex_service_tier": "standard", "codex_approval_policy": "never", "codex_sandbox": "danger-full-access"},
            "restart_fields": ["native.codex_runtime_root", "native.codex_seed_dir", "native.claude_config_dir"], "validation_errors": []},
        "connections": {"scope": "workspace", "revision": document.revision,
            "stored": connections, "effective": settings.connections,
            "restart_fields": ["server_host", "server_port"], "validation_errors": []},
        "llm_profiles": crate::profile_settings::llm_profiles_view(settings)?,
        "execution_profiles": crate::profile_settings::execution_profiles_view(settings)?,
        "models": {
            "scope": "workspace", "revision": document.revision,
            "stored": document.section::<ModelSettings>(&path, "models")?,
            "effective": models, "source": "workspace", "restart_fields": [],
        },
        "runtime": RuntimeSettingsView {
            scope: "workspace",
            revision: document.revision,
            stored,
            effective,
            restart_fields: ["runs_dir", "flows_dir", "ui_dir", "project_roots"],
        },
    }))
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
        WorkspaceSettingsSection::Agents(value) => value
            .validate()
            .map_err(|error| WorkspaceError::Validation(error.to_string())),
        WorkspaceSettingsSection::Connections(value) => value
            .validate()
            .map_err(|error| WorkspaceError::Validation(error.to_string())),
        WorkspaceSettingsSection::LlmProfiles(value) => {
            crate::profile_settings::llm_candidate(value).map(|_| ())
        }
        WorkspaceSettingsSection::ExecutionProfiles(value) => {
            crate::profile_settings::execution_candidate(value).map(|_| ())
        }
        WorkspaceSettingsSection::ClientPreferences {
            client_id,
            preferences,
        }
        | WorkspaceSettingsSection::ImportClientPreferences {
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
        WorkspaceSettingsSection::Models(value) | WorkspaceSettingsSection::ImportModels(value) => {
            validate_model_settings(settings, value)
        }
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
    request: WorkspaceSettingsUpdate,
) -> WorkspaceResult<Value> {
    // Delegated project/conversation/profile operations acquire this themselves.
    let _references = if matches!(
        &request.section,
        WorkspaceSettingsSection::Models(_)
            | WorkspaceSettingsSection::ImportModels(_)
            | WorkspaceSettingsSection::ProjectModels { .. }
    ) {
        Some(spark_storage::settings::lock_profile_references(
            &settings.config_dir,
        )?)
    } else {
        None
    };
    let importing = matches!(&request.section, WorkspaceSettingsSection::ImportModels(_));
    if importing {
        let path = settings.config_dir.join("spark.toml");
        if read_settings_document(&path)?.values.contains_key("models") {
            return workspace_settings(settings);
        }
    }
    validate_workspace_settings_update(settings, &request)?;
    let path = settings.config_dir.join("spark.toml");
    let (section, value) = match request.section {
        WorkspaceSettingsSection::Providers(value) => ("providers", toml::Value::try_from(value)),
        WorkspaceSettingsSection::Agents(value) => ("agents", toml::Value::try_from(value)),
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
        WorkspaceSettingsSection::ImportClientPreferences {
            client_id,
            preferences,
        } => {
            let path =
                spark_storage::settings::client_settings_path(&settings.config_dir, &client_id)?;
            let document = spark_storage::settings::import_client_preferences(
                &path,
                &request.expected_revision,
                &preferences,
            )?;
            return client_preferences_view(&path, &client_id, &document);
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
        WorkspaceSettingsSection::Models(value) | WorkspaceSettingsSection::ImportModels(value) => {
            ("models", toml::Value::try_from(value))
        }
        WorkspaceSettingsSection::ConversationModels {
            conversation_id,
            project_path,
            model_settings,
        } => {
            let service = crate::WorkspaceConversationService::new(settings.clone());
            if model_settings.is_none() {
                let snapshot = service.get_snapshot(&conversation_id, Some(&project_path))?;
                if snapshot["settings"]["models"]["revision"].as_str()
                    != Some(request.expected_revision.as_str())
                {
                    return Err(WorkspaceError::Conflict(
                        "Conversation settings changed; reload before saving.".into(),
                    ));
                }
                return Ok(snapshot["settings"].clone());
            }
            let snapshot = service.update_conversation_settings(
                &conversation_id,
                crate::ConversationSettingsUpdate {
                    project_path,
                    model_settings,
                    expected_revision: Some(request.expected_revision),
                    ..Default::default()
                },
            )?;
            return Ok(snapshot["settings"].clone());
        }
        WorkspaceSettingsSection::ProjectModels {
            project_path,
            model_settings,
        } => {
            let paths = spark_storage::ProjectRegistry::new(&settings.data_dir)
                .project_paths(&project_path)?;
            if !paths.project_file.exists() {
                return Err(WorkspaceError::NotFound(
                    "Register the project before editing its defaults.".into(),
                ));
            }
            let Some(model_settings) = model_settings else {
                let document = read_settings_document(&paths.project_file)?;
                if document.revision != request.expected_revision {
                    return Err(WorkspaceError::Conflict(
                        "Project settings changed; reload before saving.".into(),
                    ));
                }
                return project_model_settings_view(settings, &project_path);
            };
            let value = model_settings
                .map(toml::Value::try_from)
                .transpose()
                .map_err(|_| WorkspaceError::Validation("Invalid model settings.".into()))?;
            update_settings_section(
                &paths.project_file,
                &request.expected_revision,
                "model_settings",
                value,
                |_| Ok(()),
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
            let document = SettingsDocument {
                values: values.clone(),
                revision: String::new(),
            };
            spark_storage::settings::validate_core_sections(&path, values)?;
            let models: ModelSettings = document.section(&path, "models")?.unwrap_or_default();
            validate_model_settings(settings, &models).map_err(|error| {
                spark_storage::StorageError::SettingsValidation {
                    path: path.clone(),
                    reason: error.to_string(),
                }
            })?;
            if importing {
                spark_storage::settings::backup_settings_before_import(&path)?;
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
    if !matches!(
        values.get("browser_migration_version"),
        None | Some(toml::Value::Integer(0 | 1))
    ) {
        return Err(spark_storage::StorageError::SettingsValidation {
            path: path.into(),
            reason:
                "Unsupported browser preference migration version; use a compatible Spark binary."
                    .into(),
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
            "editor_sidebar_width": stored.editor_sidebar_width.unwrap_or(288),
            "home_sidebar_primary_split_ratio": stored.home_sidebar_primary_split_ratio,
            "show_advanced_controls": stored.show_advanced_controls.unwrap_or(false),
            "expand_child_flows": stored.expand_child_flows.unwrap_or(false),
            "graph_settings_open": stored.graph_settings_open.unwrap_or(false),
            "runs_scope": stored.runs_scope.clone().unwrap_or(spark_common::settings::PresentationScope::Active),
            "run_presentation": stored.run_presentation,
            "flow_node_positions": stored.flow_node_positions,
            "flow_edge_ports": stored.flow_edge_ports,
            "triggers_scope": stored.triggers_scope.clone().unwrap_or(spark_common::settings::PresentationScope::All)},
        "browser_migration_version": document.values.get("browser_migration_version").and_then(toml::Value::as_integer).unwrap_or(0),
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

pub fn project_model_settings_view(
    settings: &SparkSettings,
    project_path: &str,
) -> WorkspaceResult<Value> {
    let path = spark_storage::ProjectRegistry::new(&settings.data_dir)
        .project_paths(project_path)?
        .project_file;
    let document = read_settings_document(&path)?;
    let stored: Option<ModelSettings> = document.section(&path, "model_settings")?;
    let workspace = workspace_model_settings(settings)?;
    let (effective, source) = resolve_model_settings(&workspace, stored.as_ref(), None)
        .map_err(|error| WorkspaceError::Validation(error.to_string()))?;
    validate_model_settings(settings, effective)?;
    let stored_execution: Option<String> = document.section(&path, "execution_profile_id")?;
    let default_execution = attractor_api::execution_placement_settings(settings).body
        ["default_execution_profile_id"]
        .clone();
    let effective_execution = stored_execution
        .as_ref()
        .map(|id| json!(id))
        .unwrap_or(default_execution);
    Ok(
        json!({"models": {"scope": "project", "project_path": project_path,
        "revision": document.revision, "stored": stored, "effective": effective,
        "source": source, "restart_fields": []},
        "execution": {"scope": "project", "project_path": project_path,
        "revision": document.revision, "stored": stored_execution,
        "effective": effective_execution, "source": if stored_execution.is_some() { "project" } else { "workspace" }, "restart_fields": []}}),
    )
}

pub fn conversation_model_settings(
    settings: &SparkSettings,
    project_path: &str,
    conversation: Option<&ModelSettings>,
) -> WorkspaceResult<(ModelSettings, ModelSettingsSource)> {
    let (defaults, defaults_source) =
        spark_agent_adapter::config::read_project_model_defaults(settings, project_path)
            .map_err(WorkspaceError::Validation)?;
    let (effective, source) = if let Some(conversation) = conversation {
        conversation
            .validate()
            .map_err(|error| WorkspaceError::Validation(error.to_string()))?;
        (conversation, ModelSettingsSource::Conversation)
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
