use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;
use toml::value::Table;

use crate::errors::{AdapterError, AdapterErrorKind};
use crate::openai_compatible::OpenAICompatibleRequestConfig;

pub const PROFILE_CONFIG_FILE: &str = "llm-profiles.toml";
const SUPPORTED_PROFILE_PROVIDERS: &[&str] = &["openai_compatible"];
const SUPPORTED_PROFILE_KEYS: &[&str] = &[
    "api_key_env",
    "base_url",
    "default_model",
    "label",
    "models",
    "provider",
];

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct LlmProfileConfigurationError {
    message: String,
}

impl LlmProfileConfigurationError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn into_adapter_error(self) -> AdapterError {
        AdapterError::new(AdapterErrorKind::Configuration, self.message)
    }
}

impl From<LlmProfileConfigurationError> for AdapterError {
    fn from(error: LlmProfileConfigurationError) -> Self {
        error.into_adapter_error()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmProfile {
    pub id: String,
    pub provider: String,
    pub base_url: String,
    pub models: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
}

impl LlmProfile {
    pub fn configured(&self, env: &impl LlmProfileEnvironment) -> bool {
        match self.api_key_env.as_deref() {
            None => true,
            Some(key) => env
                .get_env(key)
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false),
        }
    }

    pub fn api_key_with_env(
        &self,
        env: &impl LlmProfileEnvironment,
    ) -> Result<Option<String>, LlmProfileConfigurationError> {
        let Some(key) = self.api_key_env.as_deref() else {
            return Ok(None);
        };
        env.get_env(key)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(Some)
            .ok_or_else(|| {
                LlmProfileConfigurationError::new(format!(
                    "LLM profile '{}' requires environment variable {key} to be non-empty.",
                    self.id
                ))
            })
    }

    pub fn openai_compatible_request_config_with_env(
        &self,
        env: &impl LlmProfileEnvironment,
    ) -> Result<OpenAICompatibleRequestConfig, LlmProfileConfigurationError> {
        Ok(OpenAICompatibleRequestConfig {
            api_key: self.api_key_with_env(env)?,
            base_url: Some(self.base_url.clone()),
            require_api_key: self.api_key_env.is_some(),
            ..OpenAICompatibleRequestConfig::default()
        })
    }

    pub fn to_public_value(&self, env: &impl LlmProfileEnvironment) -> Value {
        json!({
            "id": self.id,
            "label": self.label,
            "provider": self.provider,
            "models": self.models,
            "default_model": self.default_model,
            "configured": self.configured(env),
        })
    }
}

pub trait LlmProfileEnvironment {
    fn get_env(&self, key: &str) -> Option<String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessLlmProfileEnvironment;

impl LlmProfileEnvironment for ProcessLlmProfileEnvironment {
    fn get_env(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
}

impl LlmProfileEnvironment for BTreeMap<String, String> {
    fn get_env(&self, key: &str) -> Option<String> {
        self.get(key).cloned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmProfileConfigRoot {
    config_dir: PathBuf,
}

impl LlmProfileConfigRoot {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }
}

pub fn llm_profiles_path(config_dir: impl AsRef<Path>) -> PathBuf {
    config_dir.as_ref().join(PROFILE_CONFIG_FILE)
}

pub fn load_llm_profiles(
    config_dir: impl AsRef<Path>,
) -> Result<BTreeMap<String, LlmProfile>, LlmProfileConfigurationError> {
    let path = llm_profiles_path(config_dir);
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let raw_text = fs::read_to_string(&path).map_err(|_| {
        LlmProfileConfigurationError::new(format!(
            "Unable to read LLM profile config: {}",
            path.display()
        ))
    })?;
    let raw = raw_text.parse::<toml::Value>().map_err(|source| {
        LlmProfileConfigurationError::new(format!("Invalid LLM profile config: {source}"))
    })?;
    let table = raw.as_table().cloned().unwrap_or_default();
    let Some(profiles_raw) = table.get("profiles") else {
        return Ok(BTreeMap::new());
    };
    let Some(profiles_raw) = profiles_raw.as_table() else {
        return Err(LlmProfileConfigurationError::new(
            "LLM profile config must contain a [profiles] table.",
        ));
    };

    let mut profiles = BTreeMap::new();
    for (profile_id, profile_raw) in profiles_raw {
        let normalized_id =
            require_non_empty_text(profile_id.as_str(), &format!("profile id {profile_id:?}"))?;
        let Some(profile_raw) = profile_raw.as_table() else {
            return Err(LlmProfileConfigurationError::new(format!(
                "LLM profile '{normalized_id}' must be a table."
            )));
        };
        profiles.insert(
            normalized_id.clone(),
            parse_profile(&normalized_id, profile_raw)?,
        );
    }
    Ok(profiles)
}

pub fn get_llm_profile(
    config_dir: impl AsRef<Path>,
    profile_id: &str,
) -> Result<LlmProfile, LlmProfileConfigurationError> {
    get_llm_profile_with_env(config_dir, profile_id, &ProcessLlmProfileEnvironment)
}

pub fn get_llm_profile_with_env(
    config_dir: impl AsRef<Path>,
    profile_id: &str,
    _env: &impl LlmProfileEnvironment,
) -> Result<LlmProfile, LlmProfileConfigurationError> {
    let normalized_id = profile_id.trim();
    if normalized_id.is_empty() {
        return Err(LlmProfileConfigurationError::new(
            "LLM profile id is required.",
        ));
    }
    load_llm_profiles(config_dir)?
        .remove(normalized_id)
        .ok_or_else(|| {
            LlmProfileConfigurationError::new(format!(
                "LLM profile '{normalized_id}' was not found."
            ))
        })
}

pub fn public_llm_profiles(
    config_dir: impl AsRef<Path>,
) -> Result<Vec<Value>, LlmProfileConfigurationError> {
    public_llm_profiles_with_env(config_dir, &ProcessLlmProfileEnvironment)
}

pub fn public_llm_profiles_with_env(
    config_dir: impl AsRef<Path>,
    env: &impl LlmProfileEnvironment,
) -> Result<Vec<Value>, LlmProfileConfigurationError> {
    Ok(load_llm_profiles(config_dir)?
        .values()
        .map(|profile| profile.to_public_value(env))
        .collect())
}

fn parse_profile(
    profile_id: &str,
    raw: &Table,
) -> Result<LlmProfile, LlmProfileConfigurationError> {
    reject_unknown_profile_keys(profile_id, raw)?;

    let provider = require_non_empty_text(
        raw.get("provider"),
        &format!("LLM profile '{profile_id}' provider"),
    )?
    .to_lowercase();
    if !SUPPORTED_PROFILE_PROVIDERS.contains(&provider.as_str()) {
        return Err(LlmProfileConfigurationError::new(format!(
            "LLM profile '{profile_id}' has unsupported provider '{provider}'; supported providers: openai_compatible."
        )));
    }

    let base_url = require_non_empty_text(
        raw.get("base_url"),
        &format!("LLM profile '{profile_id}' base_url"),
    )?;
    let models_raw = raw
        .get("models")
        .and_then(|value| value.as_array())
        .ok_or_else(|| {
            LlmProfileConfigurationError::new(format!(
                "LLM profile '{profile_id}' models must be a non-empty list."
            ))
        })?;
    let models = models_raw
        .iter()
        .map(|value| require_non_empty_text(value, &format!("LLM profile '{profile_id}' model")))
        .collect::<Result<Vec<_>, _>>()?;
    if models.is_empty() {
        return Err(LlmProfileConfigurationError::new(format!(
            "LLM profile '{profile_id}' must declare at least one model."
        )));
    }

    let default_model = optional_text(raw.get("default_model"))?;
    if let Some(default_model) = default_model.as_deref() {
        if !models.iter().any(|model| model == default_model) {
            return Err(LlmProfileConfigurationError::new(format!(
                "LLM profile '{profile_id}' default_model '{default_model}' is not listed in models."
            )));
        }
    }

    Ok(LlmProfile {
        id: profile_id.to_string(),
        provider,
        base_url,
        models,
        label: optional_text(raw.get("label"))?,
        api_key_env: optional_text(raw.get("api_key_env"))?,
        default_model,
    })
}

fn reject_unknown_profile_keys(
    profile_id: &str,
    raw: &Table,
) -> Result<(), LlmProfileConfigurationError> {
    for key in raw.keys() {
        if !SUPPORTED_PROFILE_KEYS.contains(&key.as_str()) {
            return Err(LlmProfileConfigurationError::new(format!(
                "LLM profile '{profile_id}' has unsupported key '{key}'; supported keys: api_key_env, base_url, default_model, label, models, provider."
            )));
        }
    }
    Ok(())
}

fn optional_text(
    value: Option<&toml::Value>,
) -> Result<Option<String>, LlmProfileConfigurationError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(value) = value.as_str() else {
        return Err(LlmProfileConfigurationError::new(
            "LLM profile text values must be strings.",
        ));
    };
    let normalized = value.trim();
    if normalized.is_empty() {
        Ok(None)
    } else {
        Ok(Some(normalized.to_string()))
    }
}

fn require_non_empty_text<T>(value: T, label: &str) -> Result<String, LlmProfileConfigurationError>
where
    T: RequiredTextInput,
{
    value.require_text(label)
}

trait RequiredTextInput {
    fn require_text(self, label: &str) -> Result<String, LlmProfileConfigurationError>;
}

impl RequiredTextInput for &str {
    fn require_text(self, label: &str) -> Result<String, LlmProfileConfigurationError> {
        let normalized = self.trim();
        if normalized.is_empty() {
            Err(LlmProfileConfigurationError::new(format!(
                "{label} is required."
            )))
        } else {
            Ok(normalized.to_string())
        }
    }
}

impl RequiredTextInput for &toml::Value {
    fn require_text(self, label: &str) -> Result<String, LlmProfileConfigurationError> {
        let Some(value) = self.as_str() else {
            return Err(LlmProfileConfigurationError::new(format!(
                "{label} is required."
            )));
        };
        require_non_empty_text(value, label)
    }
}

impl RequiredTextInput for Option<&toml::Value> {
    fn require_text(self, label: &str) -> Result<String, LlmProfileConfigurationError> {
        let Some(value) = self else {
            return Err(LlmProfileConfigurationError::new(format!(
                "{label} is required."
            )));
        };
        value.require_text(label)
    }
}
