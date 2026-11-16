use spark_agent_adapter::acp::{self, AcpDiscovery};
use spark_common::agent_settings::{NativeAgentSettings, SessionConfig};
use std::path::{Path, PathBuf};
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
/// ACP discoveries by work area, harness ID, launch generation and
/// implementation version.
type AcpDiscoveryKey = (PathBuf, String, String, &'static str);
static ACP_DISCOVERY_CACHE: Mutex<Vec<(AcpDiscoveryKey, AcpDiscovery)>> = Mutex::new(Vec::new());

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
            ACP_DISCOVERY_CACHE
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clear();
        }
        "codex" => CODEX_MODELS_CACHE
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear(),
        "claude_code" => CLAUDE_CODE_MODELS_CACHE
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatModelProviderStatus {
    pub status: String,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// What discovery found for an ACP harness.
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub harness: Option<AcpDiscovery>,
}

/// The chooser's models and provider statuses, with ACP harnesses probed in
/// `work_area`, where their work would run.
pub fn chat_models(settings: &SparkSettings, work_area: &Path) -> WorkspaceResult<Value> {
    let native = native_configuration(settings)?;
    let codex_result = codex_chat_models(&native);
    chat_models_with_codex_result(settings, work_area, codex_result)
}

pub fn chat_models_with_codex_result(
    settings: &SparkSettings,
    work_area: &Path,
    codex_result: Result<Vec<ChatModelMetadata>, String>,
) -> WorkspaceResult<Value> {
    let mut models = Vec::new();
    let mut providers = serde_json::Map::new();
    let mut add = |provider: &str,
                   label: Option<&str>,
                   result: Result<Vec<ChatModelMetadata>, String>,
                   harness: Option<AcpDiscovery>| {
        let (status, error) = match result {
            Ok(discovered) => {
                models.extend(discovered);
                ("available", None)
            }
            Err(error) => ("unavailable", Some(error)),
        };
        let status = ChatModelProviderStatus {
            status: status.into(),
            error,
            label: label.map(str::to_string),
            harness,
        };
        providers.insert(provider.into(), serde_json::json!(status));
    };
    add("codex", Some("Codex"), codex_result, None);
    add(
        "claude-code",
        Some("Claude Code"),
        claude_code_chat_models(&native_configuration(settings)?),
        None,
    );
    let configuration = spark_storage::settings::read_execution_configuration(
        &settings.config_dir,
        &spark_common::paths::ProcessEnvironment,
    )?;
    for (id, harness, discovery) in acp_discoveries(&configuration.agents, work_area) {
        let provider = format!("acp:{id}");
        let result = match discovery.setup_status.as_str() {
            "ready" => Ok(acp_chat_models(&provider, &discovery.config_options)),
            _ => Err(discovery.error.clone().unwrap_or_default()),
        };
        add(&provider, Some(&harness.name), result, Some(discovery));
    }
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
                None,
                cache
                    .get_or_insert_with(Default::default)
                    .models(config, &unified_llm_adapter::NativeHttpTransport::new())
                    .map(|models| models.into_iter().map(unified_chat_model).collect()),
                None,
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

/// Each registered harness with its discovery in `work_area`, probing those
/// not cached, in parallel.
fn acp_discoveries<'a>(
    agents: &'a SessionConfig,
    work_area: &Path,
) -> Vec<(
    &'a String,
    &'a spark_common::agent_settings::AcpHarness,
    AcpDiscovery,
)> {
    let mut cache = ACP_DISCOVERY_CACHE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let keyed = agents
        .acp_harnesses
        .iter()
        .map(|(id, harness)| {
            let generation = acp::launch_generation(harness, agents.environment_inheritance);
            let key = (
                work_area.to_path_buf(),
                id.clone(),
                generation,
                env!("CARGO_PKG_VERSION"),
            );
            (id, harness, key)
        })
        .collect::<Vec<_>>();
    let probed = std::thread::scope(|scope| {
        keyed
            .iter()
            .filter(|(_, _, key)| !cache.iter().any(|(cached, _)| cached == key))
            .map(|(id, harness, key)| {
                let probe = scope.spawn(|| {
                    acp::discover(id, harness, agents.environment_inheritance, work_area)
                });
                (key.clone(), probe)
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|(key, probe)| (key, probe.join().expect("ACP discovery probe panicked")))
            .collect::<Vec<_>>()
    });
    // Only ready discoveries are kept, so installing or signing in to a
    // harness shows the next time models are listed.
    cache.extend(
        probed
            .iter()
            .filter(|(_, discovery)| discovery.setup_status == "ready")
            .cloned(),
    );
    keyed
        .into_iter()
        .filter_map(|(id, harness, key)| {
            let (_, discovery) = probed
                .iter()
                .chain(cache.iter())
                .find(|(cached, _)| *cached == key)?;
            let mut discovery = discovery.clone();
            // A session's acknowledged state supersedes the probe's.
            if let Some(options) = acp::acknowledged_options(work_area, id, &key.2)
                .filter(|_| discovery.setup_status == "ready")
            {
                discovery.config_options = options;
                discovery.modes = None;
            }
            Some((id, harness, discovery))
        })
        .collect()
}

/// An ACP harness's models are its model selector's values, in order. With
/// no selector, one "Harness default" entry leaves the choice to the harness.
/// Efforts are its thought-level value IDs, exactly as offered.
fn acp_chat_models(provider: &str, options: &[Value]) -> Vec<ChatModelMetadata> {
    let thought_level = acp::select_option(options, "thought_level");
    let efforts = thought_level
        .map(|option| {
            acp::select_values(option)
                .into_iter()
                .filter_map(|value| value["value"].as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let default_effort = thought_level
        .and_then(|option| option["currentValue"].as_str())
        .map(str::to_string);
    let model = |id: &str, display: &str, is_default: bool| ChatModelMetadata {
        provider: provider.to_string(),
        llm_profile: None,
        id: id.to_string(),
        display: display.to_string(),
        is_default,
        supported_reasoning_efforts: efforts.clone(),
        default_reasoning_effort: default_effort.clone(),
        supported_thinking: vec![],
        supported_reasoning_modes: vec![],
        supported_reasoning_summaries: vec![],
        reasoning_unverified: false,
    };
    match acp::select_option(options, "model") {
        Some(selector) => acp::select_values(selector)
            .into_iter()
            .filter_map(|value| {
                let id = value["value"].as_str()?;
                let display = value["name"].as_str().unwrap_or(id);
                Some(model(id, display, selector["currentValue"] == id))
            })
            .collect(),
        None => vec![model("", "Harness default", true)],
    }
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
