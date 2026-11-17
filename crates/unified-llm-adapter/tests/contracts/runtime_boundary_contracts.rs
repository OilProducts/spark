use std::collections::BTreeMap;

use serde_json::json;
use unified_llm_adapter::{
    resolve_effective_llm_model, resolve_effective_llm_profile, resolve_effective_llm_provider,
    resolve_effective_reasoning_effort, resolve_high_level_provider_and_model, ActiveLlmProfile,
    HighLevelLlmResolutionInputs, LlmResolutionInputs, ModelCapabilities,
    DISPLAY_MODEL_PLACEHOLDER, RUNTIME_LAUNCH_MODEL_KEY, RUNTIME_LAUNCH_PROFILE_KEY,
    RUNTIME_LAUNCH_PROVIDER_KEY, RUNTIME_LAUNCH_REASONING_EFFORT_KEY,
};

#[test]
fn llm_resolution_applies_launch_before_fallback_and_omits_model_placeholder() {
    let context = BTreeMap::from([
        (
            RUNTIME_LAUNCH_MODEL_KEY.to_string(),
            json!(DISPLAY_MODEL_PLACEHOLDER),
        ),
        (RUNTIME_LAUNCH_PROVIDER_KEY.to_string(), json!("Gemini")),
        (
            RUNTIME_LAUNCH_PROFILE_KEY.to_string(),
            json!("launch-profile"),
        ),
        (
            RUNTIME_LAUNCH_REASONING_EFFORT_KEY.to_string(),
            json!("MEDIUM"),
        ),
    ]);
    let inputs = LlmResolutionInputs {
        node_model: Some(DISPLAY_MODEL_PLACEHOLDER.to_string()),
        node_reasoning_effort: Some("HIGH".to_string()),
        node_reasoning_is_default_placeholder: true,
        fallback_model: Some("backend-model".to_string()),
        fallback_provider: Some("OpenAI".to_string()),
        fallback_profile: Some("backend-profile".to_string()),
        fallback_reasoning_effort: Some("LOW".to_string()),
        ..LlmResolutionInputs::default()
    };

    assert_eq!(
        resolve_effective_llm_model(&inputs, &context).as_deref(),
        Some("backend-model")
    );
    assert_eq!(resolve_effective_llm_provider(&inputs, &context), "gemini");
    assert_eq!(
        resolve_effective_llm_profile(&inputs, &context).as_deref(),
        Some("launch-profile")
    );
    assert_eq!(
        resolve_effective_reasoning_effort(&inputs, &context).as_deref(),
        Some("MEDIUM")
    );
}

/// ACP thought-level values are opaque IDs: only native providers' efforts
/// are normalized.
#[test]
fn acp_reasoning_effort_ids_bypass_provider_normalization() {
    let effort = |provider: &str| {
        let context = BTreeMap::from([
            (RUNTIME_LAUNCH_PROVIDER_KEY.to_string(), json!(provider)),
            (
                RUNTIME_LAUNCH_REASONING_EFFORT_KEY.to_string(),
                json!("Deep/Think-2"),
            ),
        ]);
        resolve_effective_reasoning_effort(&LlmResolutionInputs::default(), &context)
    };
    assert_eq!(effort("acp:fake").as_deref(), Some("Deep/Think-2"));
    assert_eq!(effort("codex").as_deref(), Some("deep/think-2"));
    assert_eq!(effort("claude-code").as_deref(), Some("deep/think-2"));
}

#[test]
fn high_level_resolution_omits_display_model_placeholder_before_request_building() {
    let resolved = resolve_high_level_provider_and_model(&HighLevelLlmResolutionInputs {
        provider: Some("openai_compatible".to_string()),
        model: Some(DISPLAY_MODEL_PLACEHOLDER.to_string()),
        active_profile: Some(ActiveLlmProfile::new(
            "openai_compatible",
            Some("profile-default".to_string()),
        )),
        client_default_provider: None,
        required_capabilities: ModelCapabilities::default(),
    })
    .expect("resolved model");

    assert_eq!(resolved.provider, "openai_compatible");
    assert_eq!(resolved.model, "profile-default");
}
