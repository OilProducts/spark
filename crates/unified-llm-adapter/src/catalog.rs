use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

static CATALOG: OnceLock<ModelCatalog> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub provider: String,
    pub display_name: String,
    pub context_window: Option<i64>,
    pub supports_tools: bool,
    pub supports_vision: bool,
    pub supports_reasoning: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_cost_per_million: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_cost_per_million: Option<f64>,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelCatalog {
    models: Vec<ModelInfo>,
}

impl ModelCatalog {
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        let models: Vec<ModelInfo> = serde_json::from_str(text)?;
        Ok(Self { models })
    }

    pub fn development() -> &'static Self {
        CATALOG.get_or_init(|| {
            let resource = spark_assets::models::model_catalog_resource();
            let text = resource
                .text()
                .expect("development unified LLM model catalog must be UTF-8");
            Self::from_json(text).expect("development unified LLM model catalog must be valid")
        })
    }

    pub fn list_models(&self, provider: Option<&str>) -> Vec<ModelInfo> {
        let Some(provider) = provider else {
            return self.models.clone();
        };
        let provider = normalize(provider);
        self.models
            .iter()
            .filter(|model| normalize(&model.provider) == provider)
            .cloned()
            .collect()
    }

    pub fn get_model_info(&self, model_id: &str) -> Option<ModelInfo> {
        let model_id = normalize(model_id);
        self.models
            .iter()
            .find(|model| {
                normalize(&model.id) == model_id
                    || model
                        .aliases
                        .iter()
                        .any(|alias| normalize(alias) == model_id)
            })
            .cloned()
    }

    pub fn get_latest_model(&self, provider: &str, capability: Option<&str>) -> Option<ModelInfo> {
        if !is_native_default_provider(provider) {
            return None;
        }
        self.list_models(Some(provider))
            .into_iter()
            .find(|model| model_supports(model, capability))
    }
}

pub fn get_model_info(model_id: &str) -> Option<ModelInfo> {
    ModelCatalog::development().get_model_info(model_id)
}

pub fn list_models(provider: Option<&str>) -> Vec<ModelInfo> {
    ModelCatalog::development().list_models(provider)
}

pub fn get_latest_model(provider: &str, capability: Option<&str>) -> Option<ModelInfo> {
    ModelCatalog::development().get_latest_model(provider, capability)
}

fn model_supports(model: &ModelInfo, capability: Option<&str>) -> bool {
    let Some(capability) = capability else {
        return true;
    };
    match normalize(capability).trim_start_matches("supports_") {
        "tools" => model.supports_tools,
        "vision" => model.supports_vision,
        "reasoning" => model.supports_reasoning,
        "structured" | "structured_output" => model.supports_tools,
        _ => false,
    }
}

fn is_native_default_provider(provider: &str) -> bool {
    matches!(
        normalize(provider).as_str(),
        "openai" | "anthropic" | "gemini"
    )
}

fn normalize(value: &str) -> String {
    value.trim().to_lowercase()
}
