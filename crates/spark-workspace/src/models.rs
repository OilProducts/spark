use spark_common::agent_settings::NativeAgentSettings;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use spark_common::settings::SparkSettings;

use crate::errors::{WorkspaceError, WorkspaceResult};

static PROVIDER_MODELS_CACHE: Mutex<
    Option<unified_llm_adapter::model_discovery::ProviderModelCache>,
> = Mutex::new(None);
type NativeModelCache = Vec<(NativeAgentSettings, Result<Vec<ChatModelMetadata>, String>)>;
static CODEX_MODELS_CACHE: Mutex<NativeModelCache> = Mutex::new(Vec::new());
static CLAUDE_CODE_MODELS_CACHE: Mutex<NativeModelCache> = Mutex::new(Vec::new());

pub fn invalidate_model_discovery(section: &str) {
    match section {
        "providers" | "llm_profiles" => {
            if let Some(cache) = PROVIDER_MODELS_CACHE
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_mut()
            {
                cache.clear();
            }
        }
        "agents" => {
            CODEX_MODELS_CACHE
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clear();
            CLAUDE_CODE_MODELS_CACHE
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clear();
        }
        "codex" => CODEX_MODELS_CACHE
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear(),
        _ => {}
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatModelMetadata {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_profile: Option<String>,
    pub id: String,
    pub display: String,
    pub is_default: bool,
    pub supported_reasoning_efforts: Vec<String>,
    pub default_reasoning_effort: Option<String>,
    #[serde(default)]
    pub supported_thinking: Vec<String>,
    #[serde(default)]
    pub supported_reasoning_modes: Vec<String>,
    #[serde(default)]
    pub supported_reasoning_summaries: Vec<String>,

    #[serde(default)]
    pub reasoning_unverified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatModelProviderStatus {
    pub status: String,
    pub error: Option<String>,
}

pub fn chat_models(settings: &SparkSettings) -> WorkspaceResult<Value> {
    let native = native_configuration(settings)?;
    let codex_result = codex_chat_models(&native);
    chat_models_with_codex_result(settings, codex_result)
}

pub fn chat_models_with_codex_result(
    settings: &SparkSettings,
    codex_result: Result<Vec<ChatModelMetadata>, String>,
) -> WorkspaceResult<Value> {
    let (mut models, codex_status) = match codex_result {
        Ok(models) => (
            models,
            ChatModelProviderStatus {
                status: "available".to_string(),
                error: None,
            },
        ),
        Err(error) => (
            Vec::new(),
            ChatModelProviderStatus {
                status: "unavailable".to_string(),
                error: Some(error),
            },
        ),
    };
    let mut providers = serde_json::Map::new();
    providers.insert("codex".into(), serde_json::json!(codex_status));
    let mut add = |provider: &str, result: Result<Vec<ChatModelMetadata>, String>| {
        let status = match result {
            Ok(discovered) => {
                models.extend(discovered);
                ChatModelProviderStatus {
                    status: "available".into(),
                    error: None,
                }
            }
            Err(error) => ChatModelProviderStatus {
                status: "unavailable".into(),
                error: Some(error),
            },
        };
        providers.insert(provider.into(), serde_json::json!(status));
    };
    add(
        "claude-code",
        claude_code_chat_models(&native_configuration(settings)?),
    );
    let configuration = spark_storage::settings::read_execution_configuration(
        &settings.config_dir,
        &spark_common::paths::ProcessEnvironment,
    )?;
    let environment = unified_llm_adapter::ProviderEnvironment::from_env_map(
        &configuration
            .providers
            .execution_environment(&std::env::vars().collect()),
        None,
    );
    let mut cache = PROVIDER_MODELS_CACHE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    for provider in ["anthropic", "openai", "gemini", "openrouter", "litellm"] {
        if let Some(config) = environment.providers.get(provider) {
            add(
                provider,
                cache
                    .get_or_insert_with(Default::default)
                    .models(config, &unified_llm_adapter::NativeHttpTransport::new())
                    .map(|models| models.into_iter().map(unified_chat_model).collect()),
            );
        }
    }
    models.extend(configured_profile_chat_models(settings)?);
    Ok(serde_json::json!({
        "models": models,
        "provider_reasoning_efforts": unified_llm_adapter::ModelCatalog::development().provider_reasoning_efforts,
        "providers": providers,
    }))
}

pub fn native_configuration(settings: &SparkSettings) -> WorkspaceResult<NativeAgentSettings> {
    let mut configuration = spark_storage::settings::read_execution_configuration(
        &settings.config_dir,
        &spark_common::paths::ProcessEnvironment,
    )?;
    configuration
        .agents
        .native
        .retain_startup_paths(&settings.agents.native);
    spark_agent_adapter::config::capture_native_binaries(&mut configuration.agents);
    Ok(configuration.agents.native)
}

fn claude_code_chat_models(native: &NativeAgentSettings) -> Result<Vec<ChatModelMetadata>, String> {
    let mut cache = CLAUDE_CODE_MODELS_CACHE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some((_, result)) = cache.iter().find(|(captured, _)| captured == native) {
        return result.clone();
    }
    let result = spark_agent_adapter::claude_code::list_available_claude_code_models_with_settings(
        Some(native),
    )
    .map(claude_code_chat_models_from_metadata)
    .map_err(|error| error.message);
    cache.push((native.clone(), result.clone()));
    result
}

pub fn claude_code_chat_models_from_metadata(
    metadata: Vec<spark_agent_adapter::ClaudeCodeModelMetadata>,
) -> Vec<ChatModelMetadata> {
    metadata
        .into_iter()
        .map(|model| ChatModelMetadata {
            reasoning_unverified: false,
            llm_profile: None,
            provider: "claude-code".to_string(),
            // The blank id is the catalog's `default` pseudo-entry: no
            // --model flag, the CLI picks.
            is_default: model.id.is_empty(),
            id: model.id,
            display: model.display,
            // The adapter passes the chosen effort as `--effort`.
            supported_reasoning_efforts: model.supported_efforts,
            default_reasoning_effort: None,
            supported_thinking: vec![],
            supported_reasoning_modes: vec![],
            supported_reasoning_summaries: vec![],
        })
        .collect()
}

/// Codex models come from the local install itself (`model/list`), so the
/// chooser only offers what codex will actually serve.
fn codex_chat_models(native: &NativeAgentSettings) -> Result<Vec<ChatModelMetadata>, String> {
    let mut cache = CODEX_MODELS_CACHE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some((_, result)) = cache.iter().find(|(captured, _)| captured == native) {
        return result.clone();
    }
    let result = spark_agent_adapter::codex_app_server::list_available_codex_models_with_settings(
        Some(native),
    )
    .map(codex_chat_models_from_metadata)
    .map_err(|error| format!("Codex model discovery failed: {error}"));
    cache.push((native.clone(), result.clone()));
    result
}

pub fn codex_chat_models_from_metadata(
    metadata: Vec<spark_agent_adapter::CodexModelMetadata>,
) -> Vec<ChatModelMetadata> {
    let has_default = metadata.iter().any(|model| model.is_default);
    metadata
        .into_iter()
        .enumerate()
        .map(|(index, model)| ChatModelMetadata {
            reasoning_unverified: false,
            llm_profile: None,
            provider: "codex".to_string(),
            display: model.display,
            is_default: model.is_default || (!has_default && index == 0),
            // Report only what Codex reports: no levels means Default only.
            supported_reasoning_efforts: model.supported_reasoning_efforts,
            default_reasoning_effort: model.default_reasoning_effort,
            supported_thinking: vec![],
            supported_reasoning_modes: vec![],
            supported_reasoning_summaries: vec!["auto".into(), "concise".into(), "detailed".into()],
            id: model.id,
        })
        .collect()
}

fn configured_profile_chat_models(
    settings: &SparkSettings,
) -> WorkspaceResult<Vec<ChatModelMetadata>> {
    let profiles = unified_llm_adapter::public_llm_profiles(&settings.config_dir)
        .map_err(|error| WorkspaceError::ServiceUnavailable(error.to_string()))?;
    let mut models = Vec::new();
    for profile in profiles {
        let Some(profile_id) = profile.get("id").and_then(Value::as_str) else {
            continue;
        };
        let provider = profile
            .get("provider")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("openai_compatible");
        let label = profile
            .get("label")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(profile_id);
        let default_model = profile.get("default_model").and_then(Value::as_str);
        let Some(profile_models) = profile.get("models").and_then(Value::as_array) else {
            continue;
        };
        for model in profile_models.iter().filter_map(Value::as_str) {
            models.push(ChatModelMetadata {
                reasoning_unverified: false,
                llm_profile: Some(profile_id.to_string()),
                provider: provider.to_string(),
                id: model.to_string(),
                display: format!("{label} / {model}"),
                is_default: default_model == Some(model),
                supported_reasoning_efforts: profile
                    .get("reasoning_efforts")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok())
                    .unwrap_or_default(),
                default_reasoning_effort: None,
                supported_thinking: vec![],
                supported_reasoning_modes: vec![],
                supported_reasoning_summaries: vec![],
            });
        }
    }
    Ok(models)
}

fn unified_chat_model(
    discovered: unified_llm_adapter::model_discovery::DiscoveredModel,
) -> ChatModelMetadata {
    let model = discovered.info;
    ChatModelMetadata {
        reasoning_unverified: discovered.reasoning_unverified,
        llm_profile: None,
        provider: model.provider,
        id: model.id,
        display: model.display_name,
        is_default: false,
        supported_reasoning_efforts: model.reasoning_efforts,
        default_reasoning_effort: model.default_reasoning_effort,
        supported_thinking: model.supported_thinking,
        supported_reasoning_modes: model.supported_reasoning_modes,
        supported_reasoning_summaries: model.supported_reasoning_summaries,
    }
}
