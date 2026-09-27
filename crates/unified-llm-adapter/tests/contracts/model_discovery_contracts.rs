use std::collections::VecDeque;
use std::sync::Mutex;

use serde_json::json;
use unified_llm_adapter::model_discovery::ProviderModelCache;
use unified_llm_adapter::{
    AdapterError, NativeCompleteRequest, NativeCompleteResponse, NativeCompleteTransport,
    ProviderConfig,
};

struct Transport {
    replies: Mutex<VecDeque<NativeCompleteResponse>>,
    requests: Mutex<Vec<NativeCompleteRequest>>,
}
impl NativeCompleteTransport for Transport {
    fn complete(
        &self,
        request: NativeCompleteRequest,
    ) -> Result<NativeCompleteResponse, AdapterError> {
        self.requests.lock().unwrap().push(request);
        Ok(self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected discovery call"))
    }
}
fn mock_transport(replies: Vec<NativeCompleteResponse>) -> Transport {
    Transport {
        replies: Mutex::new(replies.into()),
        requests: Mutex::new(vec![]),
    }
}
fn config() -> ProviderConfig {
    ProviderConfig {
        provider: "anthropic".into(),
        api_key: Some("key".into()),
        base_url: Some("https://example.test/v1".into()),
        ..ProviderConfig::default()
    }
}

#[test]
fn anthropic_discovery_maps_efforts_paginates_and_caches_per_configuration() {
    let first = NativeCompleteResponse::ok(json!({"data": [{
        "id": "claude-opus-4-6", "display_name": "Live Opus", "max_input_tokens": 200000,
        "capabilities": {"effort": {"max": {"supported": true}, "high": {"supported": true}, "low": {"supported": true}, "medium": {"supported": false}, "future": {"supported": true}}}
    }], "has_more": true, "last_id": "opus/id"}));
    let second = NativeCompleteResponse::ok(
        json!({"data": [{"id": "haiku", "display_name": "Live Haiku", "capabilities": {"effort": {"low": {"supported": false}}}}], "has_more": false, "last_id": "haiku"}),
    );
    let transport = mock_transport(vec![first.clone(), second.clone(), first, second]);
    let mut cache = ProviderModelCache::default();
    let mut config = config();
    let models = cache.models(&config, &transport).unwrap();
    assert_eq!(models[0].info.display_name, "Live Opus");
    assert_eq!(models[0].info.context_window, Some(200000));
    assert_eq!(
        models[0].info.reasoning_efforts,
        ["low", "high", "max", "future"]
    );
    assert_eq!(
        models[0].info.default_reasoning_effort.as_deref(),
        Some("high")
    );
    assert!(models[1].info.reasoning_efforts.is_empty());
    assert_eq!(models, cache.models(&config, &transport).unwrap());
    {
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].url, "https://example.test/v1/models?limit=1000");
        assert_eq!(requests[0].headers["x-api-key"], "key");
        assert_eq!(requests[0].headers["anthropic-version"], "2023-06-01");
        assert!(requests[1].url.contains("after_id=opus%2Fid"));
    }
    config.api_key = Some("rotated".into());
    assert_eq!(models, cache.models(&config, &transport).unwrap());
    assert_eq!(transport.requests.lock().unwrap().len(), 4);
}

#[test]
fn discovery_caches_failures_without_catalog_fallback() {
    let mut cache = ProviderModelCache::default();
    let transport = mock_transport(vec![NativeCompleteResponse {
        status: 401,
        headers: Default::default(),
        body: json!({"error": "unauthorized key"}),
    }]);
    let error = cache.models(&config(), &transport).unwrap_err();
    assert!(error.contains("401"));
    assert!(error.contains("unauthorized [redacted]"));
    assert_eq!(cache.models(&config(), &transport).unwrap_err(), error);
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    for body in [
        json!({}),
        json!({"data": [], "has_more": true, "last_id": null}),
        json!({"data": [{"id":""}]}),
    ] {
        let transport = mock_transport(vec![NativeCompleteResponse::ok(body)]);
        assert!(ProviderModelCache::default()
            .models(&config(), &transport)
            .is_err());
    }
    let transport = mock_transport(vec![NativeCompleteResponse::ok(json!({"data": []}))]);
    assert!(ProviderModelCache::default()
        .models(&config(), &transport)
        .unwrap()
        .is_empty());
}

#[test]
fn discovery_filters_and_uses_live_then_notes_then_unverified_levels() {
    let cases = [
        (
            "openai",
            json!({"data": [
                {"id":"gpt-5.2"}, {"id":"future-model"}, {"id":"text-embedding-3-small"}, {"id":"omni-moderation-latest"},
                {"id":"text-moderation-latest"}, {"id":"whisper-1"}, {"id":"tts-1"}, {"id":"dall-e-3"}, {"id":"gpt-image-1"},
                {"id":"gpt-realtime"}, {"id":"gpt-audio"}, {"id":"gpt-4o-audio-preview"}, {"id":"gpt-4o-mini-realtime-preview"},
                {"id":"gpt-4o-transcribe"}, {"id":"gpt-4o-mini-transcribe"}, {"id":"gpt-4o-mini-tts"}, {"id":"sora-2"}
            ]}),
            vec!["gpt-5.2", "future-model"],
        ),
        (
            "gemini",
            json!({"models": [
                {"name":"models/gemini-3.1-pro-preview", "supportedGenerationMethods":["generateContent"], "thinking":true},
                {"name":"models/future", "supportedGenerationMethods":["generateContent"], "thinking":true},
                {"name":"models/gemini-3-flash-preview", "supportedGenerationMethods":["generateContent"], "thinking":false},
                {"name":"models/embedding", "supportedGenerationMethods":["embedContent"], "thinking":true}
            ]}),
            vec!["gemini-3.1-pro-preview", "future", "gemini-3-flash-preview"],
        ),
        (
            "openrouter",
            json!({"data": [
                {"id":"openai/gpt-5.2", "reasoning":{"supported_efforts":["future", "low"], "default_effort":"future"}},
                {"id":"unknown"}, {"id":"empty", "reasoning":{"supported_efforts":[]}}
            ]}),
            vec!["openai/gpt-5.2", "unknown", "empty"],
        ),
        (
            "litellm",
            json!({"data": [{"id":"gpt-5.2"}, {"id":"live", "reasoning":{"supported_efforts":["high"], "default_effort":"high"}}]}),
            vec!["gpt-5.2", "live"],
        ),
    ];
    for (provider, body, ids) in cases {
        let config = ProviderConfig {
            provider: provider.into(),
            ..config()
        };
        let transport = mock_transport(vec![NativeCompleteResponse::ok(body)]);
        let models = ProviderModelCache::default()
            .models(&config, &transport)
            .unwrap();
        assert_eq!(
            models
                .iter()
                .map(|model| model.info.id.as_str())
                .collect::<Vec<_>>(),
            ids
        );
        match provider {
            "openai" | "gemini" => {
                let notes = unified_llm_adapter::get_model_info(ids[0]).unwrap();
                assert_eq!(models[0].info.reasoning_efforts, notes.reasoning_efforts);
                assert_eq!(
                    models[0].info.input_cost_per_million,
                    notes.input_cost_per_million
                );
                assert_eq!(models[0].info.aliases, notes.aliases);
                assert!(!models[0].reasoning_unverified);
                assert!(models[1].reasoning_unverified);
                if provider == "gemini" {
                    assert!(models[2].info.reasoning_efforts.is_empty());
                }
            }
            "openrouter" => {
                assert_eq!(models[0].info.reasoning_efforts, ["future", "low"]);
                assert_eq!(
                    models[0].info.default_reasoning_effort.as_deref(),
                    Some("future")
                );
                assert!(!models[0].reasoning_unverified);
                assert!(models[1].reasoning_unverified);
                assert!(models[2].info.reasoning_efforts.is_empty());
            }
            "litellm" => {
                assert!(models[0].info.reasoning_efforts.is_empty());
                assert!(!models[0].reasoning_unverified);
                assert_eq!(models[1].info.reasoning_efforts, ["high"]);
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn discovery_preserves_callable_alias_and_gemini_pagination() {
    let notes = unified_llm_adapter::list_models(Some("openai"))
        .into_iter()
        .find(|model| !model.aliases.is_empty())
        .unwrap();
    let alias = &notes.aliases[0];
    let transport = mock_transport(vec![NativeCompleteResponse::ok(
        json!({"data":[{"id":alias}]}),
    )]);
    let models = ProviderModelCache::default()
        .models(
            &ProviderConfig {
                provider: "openai".into(),
                ..config()
            },
            &transport,
        )
        .unwrap();
    assert_eq!(&models[0].info.id, alias);
    assert_eq!(
        models[0].info.input_cost_per_million,
        notes.input_cost_per_million
    );
    let transport = mock_transport(vec![
        NativeCompleteResponse::ok(json!({"models":[], "nextPageToken":"page/2"})),
        NativeCompleteResponse::ok(json!({"models":[]})),
    ]);
    ProviderModelCache::default()
        .models(
            &ProviderConfig {
                provider: "gemini".into(),
                ..config()
            },
            &transport,
        )
        .unwrap();
    assert!(transport.requests.lock().unwrap()[1]
        .url
        .contains("pageToken=page%2F2"));
}

#[test]
fn reasoning_controls_use_live_capabilities_then_notes_without_adding_models() {
    for (provider, entries, expected) in [
        (
            "anthropic",
            json!([
                {"id":"claude-opus-4-6", "capabilities":{"thinking":{"types":{"adaptive":{"supported":true},"enabled":{"supported":false}}}}},
                {"id":"claude-opus-5-5", "capabilities":{"thinking":{"types":{"adaptive":{"supported":true}}}}},
                {"id":"unknown", "capabilities":{"thinking":{"types":{"enabled":{"supported":true},"disabled":{"supported":true}}}}}
            ]),
            vec![
                vec!["adaptive", "off"],
                vec!["adaptive"],
                vec!["budget", "off"],
            ],
        ),
        (
            "openrouter",
            json!([
                {"id":"optional", "reasoning":{"mandatory":false,"supports_max_tokens":true}},
                {"id":"required", "reasoning":{"mandatory":true}},
                {"id":"unknown"}
            ]),
            vec![vec!["off", "budget"], vec![], vec![]],
        ),
    ] {
        let mut cfg = config();
        cfg.provider = provider.into();
        let transport = mock_transport(vec![NativeCompleteResponse::ok(json!({"data":entries}))]);
        let models = ProviderModelCache::default()
            .models(&cfg, &transport)
            .unwrap();
        assert_eq!(models.len(), expected.len());
        for (model, expected) in models.iter().zip(expected) {
            assert_eq!(model.info.supported_thinking, expected);
            assert!(model.info.supported_reasoning_modes.is_empty());
            assert!(model.info.supported_reasoning_summaries.is_empty());
        }
    }
    let mut cfg = config();
    cfg.provider = "openai".into();
    let transport = mock_transport(vec![NativeCompleteResponse::ok(
        json!({"data":[{"id":"gpt-6-astra"},{"id":"gpt-5.6-sol"},{"id":"unknown"}]}),
    )]);
    let models = ProviderModelCache::default()
        .models(&cfg, &transport)
        .unwrap();
    assert_eq!(models.len(), 3);
    for model in &models[..2] {
        assert_eq!(model.info.supported_reasoning_modes, ["standard", "pro"]);
        assert_eq!(
            model.info.supported_reasoning_summaries,
            ["auto", "detailed"]
        );
    }
    assert!(models[2].info.supported_reasoning_modes.is_empty());
    assert!(models[2].info.supported_reasoning_summaries.is_empty());
}
