use std::collections::BTreeMap;

use unified_llm_adapter::{Client, Message, Request, StreamAccumulator, StreamEventType};

const LIVE_PROVIDERS: &[&str] = &[
    "openai",
    "anthropic",
    "gemini",
    "openrouter",
    "openai_compatible",
];

#[test]
fn live_rust_http_transport_complete_and_stream_for_selected_providers() {
    if !live_enabled() {
        return;
    }

    for provider in selected_live_providers() {
        let Some(case) = LiveCase::from_process_env(provider) else {
            eprintln!("skipping {provider} live smoke: missing credentials or endpoint");
            continue;
        };
        let client = Client::from_env_map(&case.env, Some(provider)).unwrap_or_else(|error| {
            panic!("{provider} live smoke client should be configurable: {error:?}")
        });
        let request = Request {
            model: case.model,
            messages: vec![Message::user("Reply with exactly: rust transport ok")],
            temperature: Some(0.0),
            max_tokens: Some(32),
            ..Request::default()
        };

        let response = client
            .complete(request.clone())
            .unwrap_or_else(|error| panic!("{provider} live complete failed: {error:?}"));
        let events = client
            .stream(request)
            .unwrap_or_else(|error| panic!("{provider} live stream open failed: {error:?}"))
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_else(|error| panic!("{provider} live stream read failed: {error:?}"));
        let stream_response = StreamAccumulator::from_events(events.clone()).response;

        assert_eq!(response.provider, provider);
        assert_eq!(stream_response.provider, provider);
        assert!(
            !response.text().trim().is_empty(),
            "{provider} live complete returned empty text"
        );
        assert!(
            !stream_response.text().trim().is_empty(),
            "{provider} live stream returned empty text"
        );
        assert_eq!(
            events.first().map(|event| &event.r#type),
            Some(&StreamEventType::StreamStart),
            "{provider} live stream should start with stream_start"
        );
        assert!(
            events
                .iter()
                .any(|event| event.r#type == StreamEventType::TextDelta),
            "{provider} live stream should yield a text delta"
        );
        assert_eq!(
            events.last().map(|event| &event.r#type),
            Some(&StreamEventType::Finish),
            "{provider} live stream should finish"
        );
    }
}

struct LiveCase {
    env: BTreeMap<String, String>,
    model: String,
}

impl LiveCase {
    fn from_process_env(provider: &str) -> Option<Self> {
        match provider {
            "openai" => {
                let mut env = BTreeMap::new();
                env.insert("OPENAI_API_KEY".to_string(), env_value("OPENAI_API_KEY")?);
                insert_process_env(&mut env, "OPENAI_BASE_URL");
                Some(Self {
                    env,
                    model: model_value("UNIFIED_LLM_ADAPTER_LIVE_OPENAI_MODEL", "gpt-5.2"),
                })
            }
            "anthropic" => {
                let mut env = BTreeMap::new();
                env.insert(
                    "ANTHROPIC_API_KEY".to_string(),
                    env_value("ANTHROPIC_API_KEY")?,
                );
                insert_process_env(&mut env, "ANTHROPIC_BASE_URL");
                Some(Self {
                    env,
                    model: model_value(
                        "UNIFIED_LLM_ADAPTER_LIVE_ANTHROPIC_MODEL",
                        "claude-sonnet-4-5",
                    ),
                })
            }
            "gemini" => {
                let mut env = BTreeMap::new();
                if let Some(api_key) = env_value("GEMINI_API_KEY") {
                    env.insert("GEMINI_API_KEY".to_string(), api_key);
                } else {
                    env.insert("GOOGLE_API_KEY".to_string(), env_value("GOOGLE_API_KEY")?);
                }
                insert_process_env(&mut env, "GEMINI_BASE_URL");
                Some(Self {
                    env,
                    model: model_value(
                        "UNIFIED_LLM_ADAPTER_LIVE_GEMINI_MODEL",
                        "gemini-3.1-pro-preview",
                    ),
                })
            }
            "openrouter" => {
                let mut env = BTreeMap::new();
                env.insert(
                    "OPENROUTER_API_KEY".to_string(),
                    env_value("OPENROUTER_API_KEY")?,
                );
                insert_process_env(&mut env, "OPENROUTER_BASE_URL");
                insert_process_env(&mut env, "OPENROUTER_HTTP_REFERER");
                insert_process_env(&mut env, "OPENROUTER_TITLE");
                Some(Self {
                    env,
                    model: model_value(
                        "UNIFIED_LLM_ADAPTER_LIVE_OPENROUTER_MODEL",
                        "openai/gpt-4o-mini",
                    ),
                })
            }
            "openai_compatible" => {
                let mut env = BTreeMap::new();
                let base_url = env_value("OPENAI_COMPATIBLE_BASE_URL")
                    .or_else(|| env_value("OPENAI_BASE_URL"))
                    .or_else(|| {
                        env_value("OPENAI_API_KEY").map(|_| "https://api.openai.com".to_string())
                    })?;
                env.insert("OPENAI_COMPATIBLE_BASE_URL".to_string(), base_url);
                if let Some(api_key) =
                    env_value("OPENAI_COMPATIBLE_API_KEY").or_else(|| env_value("OPENAI_API_KEY"))
                {
                    env.insert("OPENAI_COMPATIBLE_API_KEY".to_string(), api_key);
                }
                Some(Self {
                    env,
                    model: env_value("UNIFIED_LLM_ADAPTER_LIVE_OPENAI_COMPATIBLE_MODEL")
                        .or_else(|| env_value("UNIFIED_LLM_ADAPTER_LIVE_CHAT_MODEL"))
                        .unwrap_or_else(|| "gpt-4o-mini".to_string()),
                })
            }
            _ => None,
        }
    }
}

fn live_enabled() -> bool {
    matches!(
        env_value("UNIFIED_LLM_ADAPTER_RUN_LIVE")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes"
    )
}

fn selected_live_providers() -> Vec<&'static str> {
    let Some(raw) = env_value("UNIFIED_LLM_ADAPTER_LIVE_PROVIDERS") else {
        return LIVE_PROVIDERS.to_vec();
    };
    let requested = raw
        .split(',')
        .map(|provider| provider.trim().to_ascii_lowercase())
        .filter(|provider| !provider.is_empty())
        .collect::<Vec<_>>();
    LIVE_PROVIDERS
        .iter()
        .copied()
        .filter(|provider| requested.iter().any(|requested| requested == provider))
        .collect()
}

fn model_value(env_name: &str, fallback: &str) -> String {
    env_value(env_name).unwrap_or_else(|| fallback.to_string())
}

fn insert_process_env(env: &mut BTreeMap<String, String>, name: &str) {
    if let Some(value) = env_value(name) {
        env.insert(name.to_string(), value);
    }
}

fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
