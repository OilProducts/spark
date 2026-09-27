use std::collections::VecDeque;
use std::sync::Mutex;

use serde_json::json;
use unified_llm_adapter::model_discovery::AnthropicModelCache;
use unified_llm_adapter::{
    AdapterError, ModelCatalog, NativeCompleteRequest, NativeCompleteResponse,
    NativeCompleteTransport, ProviderConfig,
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
    let mut cache = AnthropicModelCache::default();
    let mut config = config();
    let models = cache.models(Some(&config), &transport);
    assert_eq!(models[0].display_name, "Live Opus");
    assert_eq!(
        models[0].reasoning_efforts,
        ["low", "high", "max", "future"]
    );
    assert_eq!(models[0].default_reasoning_effort.as_deref(), Some("high"));
    assert!(models[1].reasoning_efforts.is_empty());
    assert_eq!(models, cache.models(Some(&config), &transport));
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
    assert_eq!(models, cache.models(Some(&config), &transport));
    assert_eq!(transport.requests.lock().unwrap().len(), 4);
}

#[test]
fn anthropic_discovery_falls_back_without_a_key_and_caches_failed_discovery() {
    let fallback = ModelCatalog::development().list_models(Some("anthropic"));
    let mut cache = AnthropicModelCache::default();
    let transport = mock_transport(vec![NativeCompleteResponse {
        status: 401,
        headers: Default::default(),
        body: json!({"error": "unauthorized"}),
    }]);
    assert_eq!(cache.models(None, &transport), fallback);
    assert_eq!(
        cache.models(Some(&ProviderConfig::default()), &transport),
        fallback
    );
    assert!(transport.requests.lock().unwrap().is_empty());
    assert_eq!(cache.models(Some(&config()), &transport), fallback);
    assert_eq!(cache.models(Some(&config()), &transport), fallback);
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    for body in [
        json!({}),
        json!({"data": [], "has_more": false}),
        json!({"data": [], "has_more": true, "last_id": null}),
    ] {
        let transport = mock_transport(vec![NativeCompleteResponse::ok(body)]);
        assert_eq!(
            AnthropicModelCache::default().models(Some(&config()), &transport),
            fallback
        );
    }
}
