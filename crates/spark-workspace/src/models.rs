use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use spark_common::settings::SparkSettings;

use crate::errors::{WorkspaceError, WorkspaceResult};

const REASONING_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max", "ultra"];

/// The codex model list comes from a spawned app-server process, so it is
/// cached briefly; the installed model set changes on codex upgrades, not
/// per request.
const CODEX_MODELS_CACHE_TTL: Duration = Duration::from_secs(300);
static CODEX_MODELS_CACHE: Mutex<Option<(Instant, Vec<ChatModelMetadata>)>> = Mutex::new(None);

/// Claude Code models come from the CLI's stdio control protocol, cached on
/// the same rationale as codex. Discovery failures (CLI missing, logged out,
/// probe timeout) fall back to the static aliases — valid `--model` values —
/// so a transient failure never blanks the picker. The resolved list is
/// cached either way so a broken CLI is not re-probed per request.
// ponytail: second copy of the codex model-cache pattern; extract a shared
// helper when a third provider needs one.
const CLAUDE_CODE_MODELS_CACHE_TTL: Duration = Duration::from_secs(300);
static CLAUDE_CODE_MODELS_CACHE: Mutex<Option<(Instant, Vec<ChatModelMetadata>)>> =
    Mutex::new(None);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatModelMetadata {
    pub provider: String,
    pub id: String,
    pub display: String,
    pub is_default: bool,
    pub supported_reasoning_efforts: Vec<String>,
    pub default_reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatModelProviderStatus {
    pub status: String,
    pub error: Option<String>,
}

pub fn chat_models(settings: &SparkSettings) -> WorkspaceResult<Value> {
    let codex_result = codex_chat_models();
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
    models.extend(claude_code_chat_models());
    models.extend(public_unified_chat_models());
    models.extend(configured_profile_chat_models(settings)?);
    Ok(serde_json::json!({
        "models": models,
        "providers": {
            "codex": codex_status,
        },
    }))
}

fn claude_code_chat_models() -> Vec<ChatModelMetadata> {
    if let Ok(cache) = CLAUDE_CODE_MODELS_CACHE.lock() {
        if let Some((fetched_at, models)) = cache.as_ref() {
            if fetched_at.elapsed() < CLAUDE_CODE_MODELS_CACHE_TTL {
                return models.clone();
            }
        }
    }
    let models = spark_agent_adapter::list_available_claude_code_models()
        .map(claude_code_chat_models_from_metadata)
        .ok()
        .filter(|models| !models.is_empty())
        .unwrap_or_else(claude_code_static_alias_models);
    if let Ok(mut cache) = CLAUDE_CODE_MODELS_CACHE.lock() {
        *cache = Some((Instant::now(), models.clone()));
    }
    models
}

fn claude_code_static_alias_models() -> Vec<ChatModelMetadata> {
    ["opus", "sonnet", "haiku"]
        .into_iter()
        .map(|id| ChatModelMetadata {
            provider: "claude-code".to_string(),
            id: id.to_string(),
            display: id.to_string(),
            is_default: false,
            supported_reasoning_efforts: Vec::new(),
            default_reasoning_effort: None,
        })
        .collect()
}

pub fn claude_code_chat_models_from_metadata(
    metadata: Vec<spark_agent_adapter::ClaudeCodeModelMetadata>,
) -> Vec<ChatModelMetadata> {
    metadata
        .into_iter()
        .map(|model| ChatModelMetadata {
            provider: "claude-code".to_string(),
            // The blank id is the catalog's `default` pseudo-entry: no
            // --model flag, the CLI picks.
            is_default: model.id.is_empty(),
            id: model.id,
            display: model.display,
            // The CLI backend only passes --model; it has no way to apply a
            // reasoning effort, so none is advertised even when the catalog
            // reports effort support.
            supported_reasoning_efforts: Vec::new(),
            default_reasoning_effort: None,
        })
        .collect()
}

/// Codex models come from the local install itself (`model/list`), so the
/// chooser only offers what codex will actually serve.
fn codex_chat_models() -> Result<Vec<ChatModelMetadata>, String> {
    if let Ok(cache) = CODEX_MODELS_CACHE.lock() {
        if let Some((fetched_at, models)) = cache.as_ref() {
            if fetched_at.elapsed() < CODEX_MODELS_CACHE_TTL {
                return Ok(models.clone());
            }
        }
    }
    let live = spark_agent_adapter::list_available_codex_models()
        .map(codex_chat_models_from_metadata)
        .map_err(|error| format!("Codex model discovery failed: {error}"))?;
    if let Ok(mut cache) = CODEX_MODELS_CACHE.lock() {
        *cache = Some((Instant::now(), live.clone()));
    }
    Ok(live)
}

pub fn codex_chat_models_from_metadata(
    metadata: Vec<spark_agent_adapter::CodexModelMetadata>,
) -> Vec<ChatModelMetadata> {
    let has_default = metadata.iter().any(|model| model.is_default);
    metadata
        .into_iter()
        .enumerate()
        .map(|(index, model)| ChatModelMetadata {
            provider: "codex".to_string(),
            display: model.display,
            is_default: model.is_default || (!has_default && index == 0),
            supported_reasoning_efforts: if model.supported_reasoning_efforts.is_empty() {
                REASONING_EFFORTS
                    .iter()
                    .map(|effort| effort.to_string())
                    .collect()
            } else {
                model.supported_reasoning_efforts
            },
            default_reasoning_effort: model
                .default_reasoning_effort
                .or_else(|| Some("medium".to_string())),
            id: model.id,
        })
        .collect()
}

pub fn public_unified_chat_models() -> Vec<ChatModelMetadata> {
    unified_llm_adapter::list_models(None)
        .into_iter()
        .filter(|model| {
            matches!(
                model.provider.as_str(),
                "openai" | "anthropic" | "gemini" | "openrouter" | "litellm"
            )
        })
        .map(|model| ChatModelMetadata {
            provider: model.provider,
            id: model.id,
            display: model.display_name,
            is_default: false,
            supported_reasoning_efforts: reasoning_efforts(model.supports_reasoning),
            default_reasoning_effort: default_reasoning_effort(model.supports_reasoning),
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
                provider: provider.to_string(),
                id: model.to_string(),
                display: format!("{label} / {model}"),
                is_default: default_model == Some(model),
                supported_reasoning_efforts: Vec::new(),
                default_reasoning_effort: None,
            });
        }
    }
    Ok(models)
}

fn reasoning_efforts(supported: bool) -> Vec<String> {
    if supported {
        REASONING_EFFORTS
            .iter()
            .map(|value| (*value).to_string())
            .collect()
    } else {
        Vec::new()
    }
}

fn default_reasoning_effort(supported: bool) -> Option<String> {
    supported.then(|| "medium".to_string())
}
