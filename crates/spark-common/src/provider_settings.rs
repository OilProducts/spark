use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{paths::Environment, Result, SparkCommonError};

pub const PROVIDERS: [&str; 6] = [
    "openai",
    "anthropic",
    "gemini",
    "openrouter",
    "litellm",
    "openai_compatible",
];

/// Authored connections and execution snapshots contain references, never credentials.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConnection {
    pub base_url: Option<String>,
    pub api_key_env: Option<String>,
    pub organization: Option<String>,
    pub project: Option<String>,
    pub http_referer: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProviderConnections(pub BTreeMap<String, ProviderConnection>);

impl ProviderConnections {
    pub fn validate(&self) -> Result<()> {
        let invalid = || {
            SparkCommonError::SettingsValidation("Invalid provider connection: use a supported provider, an HTTP(S) endpoint without credentials/query/fragment, and an environment-variable credential reference. Organization/project apply to OpenAI; attribution applies to OpenRouter.".into())
        };
        for (provider, value) in &self.0 {
            if !PROVIDERS.contains(&provider.as_str())
                || (provider != "openai"
                    && (value.organization.is_some() || value.project.is_some()))
                || (provider != "openrouter"
                    && (value.http_referer.is_some() || value.title.is_some()))
            {
                return Err(invalid());
            }
            if let Some(reference) = &value.api_key_env {
                if reference.is_empty()
                    || !reference.bytes().enumerate().all(|(i, c)| {
                        c == b'_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit())
                    })
                {
                    return Err(invalid());
                }
            }
            for value in [&value.base_url, &value.http_referer].into_iter().flatten() {
                let url = url::Url::parse(value).map_err(|_| invalid())?;
                if !matches!(url.scheme(), "http" | "https")
                    || url.host_str().is_none()
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || url.query().is_some()
                    || url.fragment().is_some()
                {
                    return Err(invalid());
                }
            }
            if [&value.organization, &value.project, &value.title]
                .into_iter()
                .flatten()
                .any(|value| value.trim().is_empty() || value.chars().any(char::is_control))
            {
                return Err(invalid());
            }
        }
        Ok(())
    }

    /// Resolve environment precedence once at the work boundary, retaining only references.
    pub fn resolve(&self, env: &impl Environment) -> Result<Self> {
        self.validate()?;
        let mut resolved = self.clone();
        for provider in PROVIDERS {
            let value = resolved.0.entry(provider.into()).or_default();
            let prefix = provider.to_uppercase();
            let nonempty = |key: &str| env.get_var(key).filter(|v| !v.trim().is_empty());
            value.base_url = nonempty(&format!("{prefix}_BASE_URL")).or(value.base_url.take());
            let standard_key = format!("{prefix}_API_KEY");
            value.api_key_env = Some(if nonempty(&standard_key).is_some() {
                standard_key
            } else if provider == "gemini" && nonempty("GOOGLE_API_KEY").is_some() {
                "GOOGLE_API_KEY".into()
            } else {
                value.api_key_env.take().unwrap_or(standard_key)
            });
            if provider == "openai" {
                value.organization = nonempty("OPENAI_ORG_ID").or(value.organization.take());
                value.project = nonempty("OPENAI_PROJECT_ID").or(value.project.take());
            }
            if provider == "openrouter" {
                value.http_referer =
                    nonempty("OPENROUTER_HTTP_REFERER").or(value.http_referer.take());
                value.title = nonempty("OPENROUTER_TITLE").or(value.title.take());
            }
        }
        resolved.validate()?;
        Ok(resolved)
    }

    /// Feed the existing adapters. This transient map may contain secrets; never serialize it.
    pub fn execution_environment(
        &self,
        env: &BTreeMap<String, String>,
    ) -> BTreeMap<String, String> {
        let mut result = env.clone();
        for (provider, value) in &self.0 {
            let prefix = provider.to_uppercase();
            for (key, value) in [
                (format!("{prefix}_BASE_URL"), value.base_url.clone()),
                (
                    format!("{prefix}_API_KEY"),
                    value
                        .api_key_env
                        .as_ref()
                        .and_then(|key| env.get(key).cloned()),
                ),
            ] {
                result.remove(&key);
                if let Some(value) = value {
                    result.insert(key, value);
                }
            }
            if provider == "gemini" {
                result.remove("GOOGLE_API_KEY");
            }
            for (key, value) in match provider.as_str() {
                "openai" => vec![
                    ("OPENAI_ORG_ID", &value.organization),
                    ("OPENAI_PROJECT_ID", &value.project),
                ],
                "openrouter" => vec![
                    ("OPENROUTER_HTTP_REFERER", &value.http_referer),
                    ("OPENROUTER_TITLE", &value.title),
                ],
                _ => vec![],
            } {
                result.remove(key);
                if let Some(value) = value {
                    result.insert(key.into(), value.clone());
                }
            }
        }
        result
    }

    pub fn credential_status(&self, env: &impl Environment) -> BTreeMap<String, bool> {
        self.0
            .iter()
            .map(|(name, value)| {
                (
                    name.clone(),
                    value
                        .api_key_env
                        .as_ref()
                        .and_then(|key| env.get_var(key))
                        .is_some_and(|v| !v.trim().is_empty()),
                )
            })
            .collect()
    }
}
