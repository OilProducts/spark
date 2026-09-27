//! Anthropic is the native provider that publishes per-model effort capabilities.
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    AdapterTimeout, ModelCatalog, ModelInfo, NativeCompleteRequest, NativeCompleteTransport,
    ProviderConfig,
};

#[derive(Default)]
pub struct AnthropicModelCache {
    cached: Option<(Instant, ProviderConfig, Vec<ModelInfo>)>,
}

impl AnthropicModelCache {
    /// Cache failures as catalog results too, so a broken endpoint is not probed on every picker open.
    pub fn models(
        &mut self,
        config: Option<&ProviderConfig>,
        transport: &dyn NativeCompleteTransport,
    ) -> Vec<ModelInfo> {
        let fallback = || ModelCatalog::development().list_models(Some("anthropic"));
        let Some(config) = config.filter(|config| {
            config
                .api_key
                .as_ref()
                .is_some_and(|key| !key.trim().is_empty())
        }) else {
            return fallback();
        };
        if let Some((time, previous, models)) = &self.cached {
            if previous == config && time.elapsed() < Duration::from_secs(300) {
                return models.clone();
            }
        }
        let models = discover(config, transport).unwrap_or_else(fallback);
        self.cached = Some((Instant::now(), config.clone(), models.clone()));
        models
    }
}

#[derive(Deserialize)]
struct ModelsPage {
    data: Vec<DiscoveredModel>,
    has_more: bool,
    last_id: Option<String>,
}

#[derive(Deserialize)]
struct DiscoveredModel {
    id: String,
    display_name: String,
    #[serde(default)]
    capabilities: Value,
    max_input_tokens: Option<i64>,
    max_tokens: Option<i64>,
}

fn discover(
    config: &ProviderConfig,
    transport: &dyn NativeCompleteTransport,
) -> Option<Vec<ModelInfo>> {
    let base = config
        .base_url
        .as_deref()
        .unwrap_or("https://api.anthropic.com")
        .trim_end_matches('/');
    let base = base.strip_suffix("/v1").unwrap_or(base);
    let mut url = reqwest::Url::parse(&format!("{base}/v1/models")).ok()?;
    url.query_pairs_mut().append_pair("limit", "1000");
    let mut models = Vec::new();
    let mut cursors = BTreeSet::new();
    loop {
        let response = transport
            .complete(NativeCompleteRequest {
                provider: "anthropic".into(),
                method: "GET".into(),
                url: url.to_string(),
                headers: BTreeMap::from([
                    ("x-api-key".into(), config.api_key.clone()?),
                    ("anthropic-version".into(), "2023-06-01".into()),
                ]),
                timeout: AdapterTimeout {
                    request: 10.0,
                    ..AdapterTimeout::default()
                },
                abort_signal: None,
                body: json!(null),
            })
            .ok()?;
        if !(200..300).contains(&response.status) {
            return None;
        }
        let page: ModelsPage = serde_json::from_value(response.body).ok()?;
        for model in page.data {
            if model.id.trim().is_empty() {
                return None;
            }
            let catalog = ModelCatalog::development().get_model_info(&model.id);
            let efforts = model.capabilities.get("effort").and_then(Value::as_object);
            let mut reasoning_efforts = Vec::new();
            if let Some(efforts) = efforts {
                // Known provider ordering first; retain future published levels too.
                for level in ModelCatalog::development().provider_reasoning_efforts["anthropic"]
                    .iter()
                    .map(String::as_str)
                    .chain(efforts.keys().map(String::as_str))
                {
                    if efforts
                        .get(level)
                        .and_then(|value| value.get("supported"))
                        .and_then(Value::as_bool)
                        == Some(true)
                        && !reasoning_efforts.iter().any(|value| value == level)
                    {
                        reasoning_efforts.push(level.to_string());
                    }
                }
            }
            let default_reasoning_effort = catalog
                .as_ref()
                .and_then(|model| model.default_reasoning_effort.clone())
                .filter(|effort| reasoning_efforts.contains(effort));
            models.push(ModelInfo {
                id: model.id,
                provider: "anthropic".into(),
                display_name: model.display_name,
                context_window: model.max_input_tokens,
                max_output: model.max_tokens,
                supports_tools: model
                    .capabilities
                    .pointer("/function_calling/supported")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                supports_vision: model
                    .capabilities
                    .pointer("/image_input/supported")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                reasoning_efforts,
                default_reasoning_effort,
                input_cost_per_million: catalog
                    .as_ref()
                    .and_then(|model| model.input_cost_per_million),
                output_cost_per_million: catalog
                    .as_ref()
                    .and_then(|model| model.output_cost_per_million),
                aliases: catalog.map(|model| model.aliases).unwrap_or_default(),
            });
        }
        if !page.has_more {
            break;
        }
        let cursor = page.last_id.filter(|id| !id.is_empty())?;
        if !cursors.insert(cursor.clone()) {
            return None;
        }
        url.query_pairs_mut()
            .clear()
            .append_pair("limit", "1000")
            .append_pair("after_id", &cursor);
    }
    (!models.is_empty()).then_some(models)
}
