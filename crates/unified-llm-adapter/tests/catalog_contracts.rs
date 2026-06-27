use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::json;
use unified_llm_adapter::{
    get_latest_model, get_model_info, list_models, stream_events, Client, FinishReason, Message,
    ModelCatalog, ProviderAdapter, Request, Response, StreamEvents,
};

#[test]
fn public_catalog_api_exposes_model_metadata_and_filters_capabilities() {
    let gpt = get_model_info("gpt5.2").expect("gpt alias");
    assert_eq!(gpt.id, "gpt-5.2");
    assert_eq!(gpt.provider, "openai");
    assert_eq!(gpt.display_name, "GPT-5.2");
    assert_eq!(gpt.context_window, Some(1_047_576));
    assert_eq!(gpt.max_output, Some(128_000));
    assert!(gpt.supports_tools);
    assert!(gpt.supports_vision);
    assert!(gpt.supports_reasoning);
    assert_eq!(gpt.input_cost_per_million, Some(1.75));
    assert_eq!(gpt.output_cost_per_million, Some(14.0));
    assert!(gpt.aliases.contains(&"gpt5.2".to_string()));

    assert_eq!(
        list_models(Some("OpenAI"))
            .into_iter()
            .map(|model| model.id)
            .collect::<Vec<_>>(),
        vec!["gpt-5.2", "gpt-5.2-mini", "gpt-5.2-codex"]
    );
    assert_eq!(
        get_latest_model("openai", Some("supports_tools"))
            .map(|model| model.id)
            .as_deref(),
        Some("gpt-5.2")
    );
    assert_eq!(
        get_latest_model("anthropic", Some("reasoning"))
            .map(|model| model.id)
            .as_deref(),
        Some("claude-opus-4-6")
    );
    assert_eq!(
        get_latest_model("gemini", Some("vision"))
            .map(|model| model.id)
            .as_deref(),
        Some("gemini-3.1-pro-preview")
    );
    assert_eq!(get_latest_model("openai", Some("audio")), None);
}

#[test]
fn latest_model_defaults_are_native_provider_only_even_if_resource_contains_compatible_models() {
    let catalog = ModelCatalog::from_json(
        &json!([
            {
                "id": "anthropic/claude-sonnet-4.5",
                "provider": "openrouter",
                "display_name": "OpenRouter Sonnet",
                "context_window": 200000,
                "max_output": 64000,
                "supports_tools": true,
                "supports_vision": true,
                "supports_reasoning": true,
                "input_cost_per_million": null,
                "output_cost_per_million": null,
                "aliases": ["openrouter-sonnet"]
            },
            {
                "id": "team-router-model",
                "provider": "litellm",
                "display_name": "Team Router Model",
                "context_window": null,
                "max_output": null,
                "supports_tools": true,
                "supports_vision": false,
                "supports_reasoning": false,
                "input_cost_per_million": null,
                "output_cost_per_million": null,
                "aliases": []
            },
            {
                "id": "local-compatible-model",
                "provider": "openai_compatible",
                "display_name": "Local Compatible Model",
                "context_window": 32768,
                "max_output": null,
                "supports_tools": false,
                "supports_vision": false,
                "supports_reasoning": false,
                "input_cost_per_million": null,
                "output_cost_per_million": null,
                "aliases": []
            }
        ])
        .to_string(),
    )
    .expect("catalog json");

    assert_eq!(
        catalog
            .get_model_info("openrouter-sonnet")
            .map(|model| model.id),
        Some("anthropic/claude-sonnet-4.5".to_string())
    );
    assert_eq!(catalog.list_models(Some("openrouter")).len(), 1);
    assert_eq!(catalog.get_latest_model("openrouter", None), None);
    assert_eq!(catalog.get_latest_model("litellm", Some("tools")), None);
    assert_eq!(catalog.get_latest_model("openai_compatible", None), None);
}

#[test]
fn unknown_model_ids_are_advisory_and_pass_through_request_routing() {
    assert_eq!(get_model_info("not-in-catalog-2026-06-26"), None);

    let adapter: Arc<dyn ProviderAdapter> = Arc::new(EchoModelAdapter);
    let client = Client::from_providers(
        BTreeMap::from([("openai".to_string(), adapter)]),
        Some("openai"),
    )
    .expect("client");

    let response = client
        .complete(Request {
            model: "not-in-catalog-2026-06-26".to_string(),
            messages: vec![Message::user("hello")],
            ..Request::default()
        })
        .expect("unknown model should route");

    assert_eq!(response.model, "not-in-catalog-2026-06-26");
    assert_eq!(response.provider, "openai");
    assert_eq!(response.text(), "not-in-catalog-2026-06-26");
}

struct EchoModelAdapter;

impl ProviderAdapter for EchoModelAdapter {
    fn name(&self) -> &str {
        "openai"
    }

    fn complete(&self, request: Request) -> Result<Response, unified_llm_adapter::AdapterError> {
        Ok(Response {
            model: request.model.clone(),
            provider: request.provider.unwrap_or_default(),
            message: Message::assistant(request.model),
            finish_reason: FinishReason::Stop,
            ..Response::default()
        })
    }

    fn stream(&self, _request: Request) -> Result<StreamEvents, unified_llm_adapter::AdapterError> {
        Ok(stream_events(Vec::new().into_iter()))
    }
}
