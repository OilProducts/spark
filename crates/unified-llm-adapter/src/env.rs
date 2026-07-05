use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const PROVIDER_REGISTRATION_ORDER: [&str; 6] = [
    "openai",
    "anthropic",
    "gemini",
    "openrouter",
    "litellm",
    "openai_compatible",
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default)]
    pub options: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderEnvironment {
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_provider: Option<String>,
}

impl ProviderEnvironment {
    pub fn from_process_env(explicit_default: Option<&str>) -> Self {
        let env = std::env::vars().collect::<BTreeMap<_, _>>();
        Self::from_env_map(&env, explicit_default)
    }

    pub fn from_env_map(env: &BTreeMap<String, String>, explicit_default: Option<&str>) -> Self {
        let mut providers = BTreeMap::new();

        if let Some(api_key) = env_value(env, "OPENAI_API_KEY") {
            let mut options = BTreeMap::new();
            insert_if_present(&mut options, env, "OPENAI_ORG_ID", "organization");
            insert_if_present(&mut options, env, "OPENAI_PROJECT_ID", "project");
            providers.insert(
                "openai".to_string(),
                ProviderConfig {
                    provider: "openai".to_string(),
                    api_key: Some(api_key),
                    base_url: Some(normalize_openai_base_url(env_value(env, "OPENAI_BASE_URL"))),
                    options,
                },
            );
        }

        if let Some(api_key) = env_value(env, "ANTHROPIC_API_KEY") {
            providers.insert(
                "anthropic".to_string(),
                ProviderConfig {
                    provider: "anthropic".to_string(),
                    api_key: Some(api_key),
                    base_url: env_value(env, "ANTHROPIC_BASE_URL")
                        .map(normalize_anthropic_base_url),
                    options: BTreeMap::new(),
                },
            );
        }

        let gemini_api_key =
            env_value(env, "GEMINI_API_KEY").or_else(|| env_value(env, "GOOGLE_API_KEY"));
        if let Some(api_key) = gemini_api_key {
            providers.insert(
                "gemini".to_string(),
                ProviderConfig {
                    provider: "gemini".to_string(),
                    api_key: Some(api_key),
                    base_url: Some(normalize_gemini_base_url(env_value(env, "GEMINI_BASE_URL"))),
                    options: BTreeMap::new(),
                },
            );
        }

        if let Some(api_key) = env_value(env, "OPENROUTER_API_KEY") {
            let mut options = BTreeMap::new();
            insert_if_present(&mut options, env, "OPENROUTER_HTTP_REFERER", "HTTP-Referer");
            insert_if_present(&mut options, env, "OPENROUTER_TITLE", "X-Title");
            providers.insert(
                "openrouter".to_string(),
                ProviderConfig {
                    provider: "openrouter".to_string(),
                    api_key: Some(api_key),
                    base_url: Some(normalize_openai_compatible_base_url(
                        env_value(env, "OPENROUTER_BASE_URL"),
                        "https://openrouter.ai/api/v1",
                    )),
                    options,
                },
            );
        }

        if let Some(base_url) = env_value(env, "LITELLM_BASE_URL") {
            providers.insert(
                "litellm".to_string(),
                ProviderConfig {
                    provider: "litellm".to_string(),
                    api_key: env_value(env, "LITELLM_API_KEY"),
                    base_url: Some(normalize_openai_compatible_base_url(
                        Some(base_url),
                        "https://api.openai.com",
                    )),
                    options: BTreeMap::new(),
                },
            );
        }

        if let Some(base_url) = env_value(env, "OPENAI_COMPATIBLE_BASE_URL") {
            let mut options = BTreeMap::new();
            options.insert("require_api_key".to_string(), "false".to_string());
            providers.insert(
                "openai_compatible".to_string(),
                ProviderConfig {
                    provider: "openai_compatible".to_string(),
                    api_key: env_value(env, "OPENAI_COMPATIBLE_API_KEY"),
                    base_url: Some(normalize_openai_compatible_base_url(
                        Some(base_url),
                        "https://api.openai.com",
                    )),
                    options,
                },
            );
        }

        let default_provider = explicit_default.map(normalize_provider).or_else(|| {
            PROVIDER_REGISTRATION_ORDER
                .into_iter()
                .find(|provider| providers.contains_key(*provider))
                .map(str::to_string)
        });

        Self {
            providers,
            default_provider,
        }
    }
}

fn env_value(env: &BTreeMap<String, String>, key: &str) -> Option<String> {
    env.get(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn insert_if_present(
    options: &mut BTreeMap<String, String>,
    env: &BTreeMap<String, String>,
    key: &str,
    option_key: &str,
) {
    if let Some(value) = env_value(env, key) {
        options.insert(option_key.to_string(), value);
    }
}

fn normalize_provider(provider: &str) -> String {
    provider.trim().to_lowercase()
}

fn normalize_openai_base_url(base_url: Option<String>) -> String {
    let base = base_url.unwrap_or_else(|| "https://api.openai.com".to_string());
    let mut normalized = trim_url_path_suffix(&base, &[]);
    if !normalized.ends_with("/v1") && !normalized.ends_with("/responses") {
        normalized = append_url_path_segment(&normalized, "v1");
    }
    normalized
}

fn normalize_anthropic_base_url(base_url: String) -> String {
    let mut normalized = trim_url_path_suffix(&base_url, &["/messages"]);
    if !normalized.ends_with("/v1") {
        normalized = append_url_path_segment(&normalized, "v1");
    }
    normalized
}

fn normalize_gemini_base_url(base_url: Option<String>) -> String {
    let base = base_url.unwrap_or_else(|| "https://generativelanguage.googleapis.com".to_string());
    let mut normalized =
        trim_url_path_suffix(&base, &["/generateContent", "/streamGenerateContent"]);
    if let Some(index) = normalized.find("/models/") {
        normalized.truncate(index);
        normalized = normalized.trim_end_matches('/').to_string();
    }
    if normalized.ends_with("/v1beta") {
        normalized
    } else if normalized.ends_with("/v1") {
        format!("{}v1beta", normalized.trim_end_matches("v1"))
    } else {
        append_url_path_segment(&normalized, "v1beta")
    }
}

fn normalize_openai_compatible_base_url(
    base_url: Option<String>,
    default_base_url: &str,
) -> String {
    let base = base_url.unwrap_or_else(|| default_base_url.to_string());
    let mut normalized = trim_url_path_suffix(&base, &["/chat/completions", "/responses"]);
    if !normalized.ends_with("/v1") {
        normalized = append_url_path_segment(&normalized, "v1");
    }
    normalized
}

fn trim_url_path_suffix(value: &str, suffixes: &[&str]) -> String {
    let (mut base, suffix) = split_url_query_or_fragment(value.trim());
    base = base.trim_end_matches('/').to_string();
    for suffix_to_strip in suffixes {
        if base.ends_with(suffix_to_strip) {
            base.truncate(base.len() - suffix_to_strip.len());
            base = base.trim_end_matches('/').to_string();
            break;
        }
    }
    format!("{base}{suffix}")
}

fn append_url_path_segment(value: &str, segment: &str) -> String {
    let (base, suffix) = split_url_query_or_fragment(value);
    format!("{}/{}{}", base.trim_end_matches('/'), segment, suffix)
}

fn split_url_query_or_fragment(value: &str) -> (String, String) {
    let query = value.find('?');
    let fragment = value.find('#');
    let split_at = match (query, fragment) {
        (Some(query), Some(fragment)) => query.min(fragment),
        (Some(query), None) => query,
        (None, Some(fragment)) => fragment,
        (None, None) => return (value.to_string(), String::new()),
    };
    (value[..split_at].to_string(), value[split_at..].to_string())
}
