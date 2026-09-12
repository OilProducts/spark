pub use spark_common::agent_settings::SessionConfig;

pub fn captured_session_config(
    metadata: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Result<Option<SessionConfig>, String> {
    let Some(configuration) = metadata
        .get("spark.execution.settings")
        .and_then(|value| value.get("configuration"))
    else {
        return Ok(None);
    };
    let config: SessionConfig = serde_json::from_value(configuration["agents"].clone())
        .map_err(|_| "Invalid captured agent configuration.".to_string())?;
    config
        .validate()
        .map_err(|_| "Invalid captured agent configuration.".to_string())?;
    Ok(Some(config))
}

pub fn capture_native_binaries(config: &mut SessionConfig) {
    config.native.codex_binary.get_or_insert_with(|| {
        crate::codex_app_server::codex_executable()
            .to_string_lossy()
            .into_owned()
    });
    config.native.claude_binary.get_or_insert_with(|| {
        crate::claude_code::claude_code_executable()
            .to_string_lossy()
            .into_owned()
    });
}

pub fn validate_model_settings(
    settings: &spark_common::settings::SparkSettings,
    value: &spark_common::settings::ModelSettings,
) -> Result<(), String> {
    value.validate().map_err(|error| error.to_string())?;
    if let Some(provider) = value.provider.as_deref() {
        let normalized = validate_provider(provider)?;
        if normalized != provider {
            return Err("Use the canonical provider identifier.".into());
        }
        if let Some(model) = value.model.as_deref() {
            let models = unified_llm_adapter::list_models(None);
            if models.iter().any(|entry| entry.id == model)
                && !models
                    .iter()
                    .any(|entry| entry.id == model && entry.provider == provider)
            {
                return Err("The model belongs to a different provider.".into());
            }
        }
        if matches!(provider, "openrouter" | "litellm" | "openai_compatible")
            && value.model.is_none()
        {
            return Err(format!("Provider {provider} requires an explicit model."));
        }
    }
    if let Some(profile_id) = value.llm_profile.as_deref() {
        let profile = unified_llm_adapter::get_llm_profile(&settings.config_dir, profile_id)
            .map_err(|_| {
                "Unable to load the selected LLM profile; check llm-profiles.toml.".to_string()
            })?;
        validate_profile_model(&profile, value.model.as_deref())?;
    }
    if let Some(effort) = value.reasoning_effort.as_deref() {
        validate_reasoning_effort(effort)?;
    }
    Ok(())
}

pub fn validate_provider(value: &str) -> Result<String, String> {
    let normalized = value.trim().to_lowercase();
    let normalized = if normalized.is_empty() {
        "codex".to_string()
    } else {
        normalized
    };
    match normalized.as_str() {
        "codex" | "claude-code" | "openai" | "anthropic" | "gemini" | "openrouter" | "litellm"
        | "openai_compatible" => Ok(normalized),
        _ => Err(
            "Provider must be blank or one of: codex, claude-code, openai, anthropic, gemini, openrouter, litellm, openai_compatible."
                .to_string(),
        ),
    }
}

pub fn validate_reasoning_effort(value: &str) -> Result<String, String> {
    let normalized = value.trim().to_lowercase();
    if normalized.is_empty() || matches!(normalized.as_str(), "low" | "medium" | "high" | "xhigh") {
        Ok(normalized)
    } else {
        Err("Reasoning effort must be blank or one of: low, medium, high, xhigh.".to_string())
    }
}

/// Resolve persisted defaults and apply the same domain validation used by settings edits.
pub fn read_project_model_defaults(
    settings: &spark_common::settings::SparkSettings,
    project_path: &str,
) -> Result<
    (
        spark_common::settings::ModelSettings,
        spark_common::settings::ModelSettingsSource,
    ),
    String,
> {
    let defaults = spark_storage::settings::read_project_model_defaults(settings, project_path)
        .map_err(|error| error.to_string())?;
    validate_model_settings(settings, &defaults.0)?;
    Ok(defaults)
}

/// Validate a selection against either stored or candidate profile contents.
pub fn validate_profile_model(
    profile: &unified_llm_adapter::LlmProfile,
    model: Option<&str>,
) -> Result<(), String> {
    if model.is_some_and(|model| !profile.models.iter().any(|entry| entry == model)) {
        return Err("Select a model supported by the LLM profile.".into());
    }
    if model.is_none() && profile.default_model.is_none() {
        return Err("The LLM profile has no default model; select a model.".into());
    }
    Ok(())
}
