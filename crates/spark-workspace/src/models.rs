use serde::{Deserialize, Serialize};
use serde_json::Value;
use spark_common::settings::SparkSettings;

use crate::errors::{WorkspaceError, WorkspaceResult};

const REASONING_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatModelMetadata {
    pub provider: String,
    pub id: String,
    pub display: String,
    pub is_default: bool,
    pub supported_reasoning_efforts: Vec<String>,
    pub default_reasoning_effort: Option<String>,
}

pub fn chat_models(settings: &SparkSettings) -> WorkspaceResult<Value> {
    let mut models = Vec::new();
    if let Some(model) = unified_llm_adapter::get_model_info("codex") {
        models.push(ChatModelMetadata {
            provider: "codex".to_string(),
            id: model.id,
            display: model.display_name,
            is_default: true,
            supported_reasoning_efforts: reasoning_efforts(model.supports_reasoning),
            default_reasoning_effort: default_reasoning_effort(model.supports_reasoning),
        });
    }
    models.extend(public_unified_chat_models());
    models.extend(configured_profile_chat_models(settings)?);
    Ok(serde_json::json!({ "models": models }))
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
