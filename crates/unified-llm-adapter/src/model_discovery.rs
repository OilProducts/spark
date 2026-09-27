//! Provider lists establish availability; the bundled catalog only enriches listed IDs.
use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

use crate::{
    AdapterTimeout, ModelCatalog, ModelInfo, NativeCompleteRequest, NativeCompleteTransport,
    ProviderConfig,
};

#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveredModel {
    pub info: ModelInfo,
    pub reasoning_unverified: bool,
}

#[derive(Default)]
pub struct ProviderModelCache {
    cached: Vec<(ProviderConfig, Result<Vec<DiscoveredModel>, String>)>,
}

impl ProviderModelCache {
    pub fn clear(&mut self) {
        self.cached.clear();
    }

    /// The caller serializes discovery and invalidation, including failed requests.
    pub fn models(
        &mut self,
        config: &ProviderConfig,
        transport: &dyn NativeCompleteTransport,
    ) -> Result<Vec<DiscoveredModel>, String> {
        if let Some((_, result)) = self.cached.iter().find(|(previous, _)| previous == config) {
            return result.clone();
        }
        let result = discover(config, transport).map_err(|error| {
            // Provider errors may echo credentials. Never send those back to the picker.
            match config.api_key.as_deref().filter(|key| !key.is_empty()) {
                Some(key) => error.replace(key, "[redacted]"),
                None => error,
            }
        });
        self.cached.push((config.clone(), result.clone()));
        result
    }
}

fn discover(
    config: &ProviderConfig,
    transport: &dyn NativeCompleteTransport,
) -> Result<Vec<DiscoveredModel>, String> {
    let provider = config.provider.as_str();
    let base = config
        .base_url
        .as_deref()
        .unwrap_or("https://api.anthropic.com/v1")
        .trim_end_matches('/');
    let base = base.strip_suffix("/responses").unwrap_or(base);
    let mut url = reqwest::Url::parse(base).map_err(|error| error.to_string())?;
    url.set_path(&format!("{}/models", url.path().trim_end_matches('/')));
    let mut headers = BTreeMap::new();
    if let Some(key) = &config.api_key {
        match provider {
            "anthropic" => {
                headers.insert("x-api-key".into(), key.clone());
            }
            "gemini" => {
                headers.insert("x-goog-api-key".into(), key.clone());
            }
            _ => {
                headers.insert("Authorization".into(), format!("Bearer {key}"));
            }
        }
    }
    if provider == "anthropic" {
        headers.insert("anthropic-version".into(), "2023-06-01".into());
        url.query_pairs_mut().append_pair("limit", "1000");
    }
    for (option, header) in [
        ("organization", "OpenAI-Organization"),
        ("project", "OpenAI-Project"),
        ("HTTP-Referer", "HTTP-Referer"),
        ("X-Title", "X-Title"),
    ] {
        if let Some(value) = config.options.get(option) {
            headers.insert(header.into(), value.clone());
        }
    }
    let mut models = Vec::new();
    let mut cursors = BTreeSet::new();
    loop {
        let response = transport
            .complete(NativeCompleteRequest {
                provider: provider.into(),
                method: "GET".into(),
                url: url.to_string(),
                headers: headers.clone(),
                timeout: AdapterTimeout {
                    request: 10.0,
                    ..AdapterTimeout::default()
                },
                abort_signal: None,
                body: json!(null),
            })
            .map_err(|error| format!("{provider} model discovery failed: {error}"))?;
        if !(200..300).contains(&response.status) {
            return Err(format!(
                "{provider} model discovery failed (HTTP {}): {}",
                response.status, response.body
            ));
        }
        let field = if provider == "gemini" {
            "models"
        } else {
            "data"
        };
        let entries = response.body[field]
            .as_array()
            .ok_or_else(|| format!("{provider} model discovery: missing {field} array"))?;
        for model in entries {
            let id = model[if provider == "gemini" { "name" } else { "id" }]
                .as_str()
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(|| format!("{provider} model discovery: missing model ID"))?;
            let id = if provider == "gemini" {
                id.strip_prefix("models/").unwrap_or(id)
            } else {
                id
            };
            if provider == "openai" && !supports_responses(id) {
                continue;
            }
            if provider == "gemini"
                && !model["supportedGenerationMethods"]
                    .as_array()
                    .is_some_and(|methods| methods.iter().any(|method| method == "generateContent"))
            {
                continue;
            }
            models.push(enrich(provider, id, model));
        }
        let cursor = match provider {
            "anthropic" if response.body["has_more"] == true => Some((
                "after_id",
                response.body["last_id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or("Anthropic model discovery: missing pagination cursor")?,
            )),
            "gemini" => response.body["nextPageToken"]
                .as_str()
                .filter(|token| !token.is_empty())
                .map(|token| ("pageToken", token)),
            _ => None,
        };
        let Some((key, cursor)) = cursor else {
            break;
        };
        if !cursors.insert(cursor.to_string()) {
            return Err(format!(
                "{provider} model discovery: repeated pagination cursor"
            ));
        }
        url.query_pairs_mut().clear().append_pair(key, cursor);
        if provider == "anthropic" {
            url.query_pairs_mut().append_pair("limit", "1000");
        }
    }
    Ok(models)
}

fn supports_responses(id: &str) -> bool {
    // ponytail: OpenAI's Model object has no capability field. Filter documented non-Responses ID prefixes; extend when new families ship.
    ![
        "text-embedding-",
        "text-moderation-",
        "omni-moderation-",
        "whisper-",
        "tts-",
        "dall-e-",
        "gpt-image-",
        "chatgpt-image-",
        "gpt-realtime",
        "gpt-audio",
        "gpt-transcribe",
        "gpt-live-transcribe",
        "gpt-4o-realtime",
        "gpt-4o-mini-realtime",
        "gpt-4o-audio",
        "gpt-4o-mini-audio",
        "gpt-4o-transcribe",
        "gpt-4o-mini-transcribe",
        "gpt-4o-tts",
        "gpt-4o-mini-tts",
        "sora-",
    ]
    .iter()
    .any(|prefix| id.starts_with(prefix))
}

fn enrich(provider: &str, id: &str, live: &Value) -> DiscoveredModel {
    let catalog = ModelCatalog::development();
    let notes = catalog.get_model_info(id);
    let mut info = notes.clone().unwrap_or_else(|| ModelInfo {
        id: id.into(),
        provider: provider.into(),
        display_name: id.into(),
        context_window: None,
        supports_tools: false,
        supports_vision: false,
        reasoning_efforts: vec![],
        default_reasoning_effort: None,
        supported_thinking: vec![],
        supported_reasoning_modes: vec![],
        supported_reasoning_summaries: vec![],
        max_output: None,
        input_cost_per_million: None,
        output_cost_per_million: None,
        aliases: vec![],
    });
    // Notes enrich only provider-listed IDs. Never infer controls across providers.
    if provider != info.provider || matches!(provider, "gemini" | "litellm" | "openai_compatible") {
        info.supported_thinking.clear();
        info.supported_reasoning_modes.clear();
        info.supported_reasoning_summaries.clear();
    }
    if provider == "anthropic" {
        // Spark notes from the thinking guide. Opus 5 can reject off above high;
        // leave that model/effort combination to the provider, like other settings.
        if matches!(
            id,
            "claude-opus-5"
                | "claude-sonnet-5"
                | "claude-opus-4-8"
                | "claude-opus-4-7"
                | "claude-sonnet-4-6"
        ) {
            info.supported_thinking.push("off".into());
        }
        if let Some(types) = live.pointer("/capabilities/thinking/types") {
            let off = types
                .pointer("/disabled/supported")
                .and_then(Value::as_bool)
                .unwrap_or_else(|| info.supported_thinking.iter().any(|v| v == "off"));
            info.supported_thinking = [("adaptive", "adaptive"), ("enabled", "budget")]
                .into_iter()
                .filter(|(key, _)| types[*key]["supported"] == true)
                .map(|(_, value)| value.to_string())
                .collect();
            if off {
                info.supported_thinking.push("off".into());
            }
        }
    } else if provider == "openrouter" {
        info.supported_thinking.clear();
        info.supported_reasoning_modes.clear();
        info.supported_reasoning_summaries.clear();
        if let Some(reasoning) = live.get("reasoning").filter(|v| v.is_object()) {
            if reasoning["mandatory"] != true {
                info.supported_thinking.push("off".into());
            }
            if reasoning["supports_max_tokens"] == true && !id.starts_with("google/") {
                info.supported_thinking.push("budget".into());
            }
        }
    }
    // Spark notes: the reasoning guide documents these callable GPT-5.6 IDs.
    // These notes enrich live entries only; they never add catalog models.
    if provider == "openai"
        && matches!(
            id,
            "gpt-5.6" | "gpt-5.6-sol" | "gpt-5.6-terra" | "gpt-5.6-luna"
        )
    {
        info.supported_reasoning_modes = vec!["standard".into(), "pro".into()];
        info.supported_reasoning_summaries = vec!["auto".into(), "detailed".into()];
    }
    // Preserve the provider's callable ID even when it matched a catalog alias.
    info.id = id.into();
    info.provider = provider.into();
    info.display_name = live["display_name"]
        .as_str()
        .or_else(|| live["displayName"].as_str())
        .or_else(|| live["name"].as_str().filter(|_| provider != "gemini"))
        .unwrap_or(&info.display_name)
        .into();
    info.context_window = live["max_input_tokens"]
        .as_i64()
        .or_else(|| live["inputTokenLimit"].as_i64())
        .or(info.context_window);
    info.max_output = live["max_tokens"]
        .as_i64()
        .or_else(|| live["outputTokenLimit"].as_i64())
        .or(info.max_output);
    info.supports_tools = live
        .pointer("/capabilities/function_calling/supported")
        .and_then(Value::as_bool)
        .unwrap_or(info.supports_tools);
    info.supports_vision = live
        .pointer("/capabilities/image_input/supported")
        .and_then(Value::as_bool)
        .unwrap_or(info.supports_vision);
    let levels = catalog
        .provider_reasoning_efforts
        .get(provider)
        .cloned()
        .unwrap_or_default();
    let live_efforts = if provider == "anthropic" {
        live.pointer("/capabilities/effort")
            .and_then(Value::as_object)
            .map(|efforts| {
                let mut result = Vec::new();
                for level in levels.iter().chain(efforts.keys()) {
                    if efforts
                        .get(level)
                        .and_then(|value| value["supported"].as_bool())
                        == Some(true)
                        && !result.contains(level)
                    {
                        result.push(level.clone());
                    }
                }
                result
            })
    } else if matches!(provider, "openrouter" | "litellm") {
        live.get("reasoning")
            .and_then(Value::as_object)
            .map(|reasoning| match reasoning.get("supported_efforts") {
                Some(Value::Null) if provider == "openrouter" => levels.clone(),
                Some(Value::Array(efforts)) => efforts
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect(),
                _ => Vec::new(),
            })
    } else {
        None
    };
    let mut reasoning_unverified = false;
    if provider == "gemini" && live["thinking"] != true {
        info.reasoning_efforts.clear();
    } else if let Some(efforts) = live_efforts {
        info.reasoning_efforts = efforts;
        if let Some(default) = live
            .pointer("/reasoning/default_effort")
            .and_then(Value::as_str)
        {
            info.default_reasoning_effort = Some(default.into());
        }
    } else if provider == "litellm" {
        // /v1/models is OpenAI-compatible and documents no effort levels. Keep profile-declared levels on profile entries instead of guessing from proxy aliases.
        info.reasoning_efforts.clear();
    } else if notes.is_none() {
        info.reasoning_efforts = levels;
        reasoning_unverified = !info.reasoning_efforts.is_empty();
    }
    info.default_reasoning_effort = info
        .default_reasoning_effort
        .filter(|effort| info.reasoning_efforts.contains(effort));
    DiscoveredModel {
        info,
        reasoning_unverified,
    }
}
