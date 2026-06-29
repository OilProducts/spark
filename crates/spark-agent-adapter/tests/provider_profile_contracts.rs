use serde_json::{json, Value};
use spark_agent_adapter::{
    create_anthropic_profile, create_gemini_profile, create_gemini_profile_with_options,
    create_openai_compatible_profile, create_openai_profile, create_provider_profile,
    normalize_provider_selector, GeminiProfileOptions, ProviderFamily, SessionConfig, ToolRegistry,
};
use unified_llm_adapter::Tool;

fn names(profile: &spark_agent_adapter::ProviderProfile) -> Vec<String> {
    profile.tool_registry.names()
}

fn parameters(profile: &spark_agent_adapter::ProviderProfile, name: &str) -> Value {
    profile
        .tool_registry
        .get(name)
        .map(|tool| tool.definition.parameters.clone())
        .expect("tool parameters")
}

#[test]
fn native_provider_profiles_expose_distinct_model_facing_tool_contracts() {
    let openai = create_openai_profile("gpt-5.2");
    assert_eq!(openai.id, "openai");
    assert_eq!(openai.display_name.as_deref(), Some("GPT-5.2"));
    assert_eq!(openai.context_window_size, Some(1_047_576));
    assert!(openai.supports("reasoning"));
    assert!(openai.supports("streaming"));
    assert!(openai.supports("parallel_tool_calls"));
    assert_eq!(
        names(&openai),
        [
            "read_file",
            "apply_patch",
            "write_file",
            "shell",
            "grep",
            "glob",
            "spawn_agent",
            "send_input",
            "wait",
            "close_agent",
        ]
    );
    assert_eq!(openai.tools(), openai.tool_registry.llm_definitions());
    assert!(openai.tool_registry.get("edit_file").is_none());
    assert_eq!(
        parameters(&openai, "read_file")["properties"]["path"]["type"],
        json!("string")
    );
    assert_eq!(
        parameters(&openai, "apply_patch")["properties"]["patch"]["type"],
        json!("string")
    );
    assert_eq!(
        parameters(&openai, "shell")["properties"]["timeout_ms"]["default"],
        json!(10_000)
    );

    let anthropic = create_anthropic_profile("claude-sonnet-4-5");
    assert_eq!(anthropic.id, "anthropic");
    assert_eq!(anthropic.display_name.as_deref(), Some("Claude Sonnet 4.5"));
    assert_eq!(anthropic.context_window_size, Some(200_000));
    assert_eq!(
        names(&anthropic),
        [
            "read_file",
            "write_file",
            "edit_file",
            "shell",
            "grep",
            "glob",
            "spawn_agent",
            "send_input",
            "wait",
            "close_agent",
        ]
    );
    assert!(anthropic.tool_registry.get("apply_patch").is_none());
    assert_eq!(
        parameters(&anthropic, "read_file")["properties"]["file_path"]["type"],
        json!("string")
    );
    let anthropic_edit = parameters(&anthropic, "edit_file");
    assert_eq!(
        anthropic_edit["properties"]["old_string"]["type"],
        json!("string")
    );
    assert_eq!(
        anthropic_edit["properties"]["new_string"]["type"],
        json!("string")
    );
    assert_eq!(
        parameters(&anthropic, "shell")["properties"]["timeout_ms"]["default"],
        json!(120_000)
    );

    let gemini = create_gemini_profile("gemini-3.1-pro-preview");
    assert_eq!(gemini.id, "gemini");
    assert_eq!(
        gemini.display_name.as_deref(),
        Some("Gemini 3.1 Pro Preview")
    );
    assert_eq!(gemini.context_window_size, Some(1_048_576));
    assert_eq!(
        names(&gemini),
        [
            "read_file",
            "read_many_files",
            "write_file",
            "edit_file",
            "shell",
            "grep",
            "glob",
            "list_dir",
            "spawn_agent",
            "send_input",
            "wait",
            "close_agent",
        ]
    );
    let gemini_edit = parameters(&gemini, "edit_file");
    assert_eq!(
        parameters(&gemini, "read_file")["properties"]["file_path"]["type"],
        json!("string")
    );
    assert_eq!(
        gemini_edit["properties"]["instruction"]["type"],
        json!("string")
    );
    assert_eq!(
        gemini_edit["properties"]["allow_multiple"]["default"],
        json!(false)
    );
    assert!(gemini_edit["properties"].get("replace_all").is_none());

    let gemini_with_web = create_gemini_profile_with_options(
        "gemini-3.1-pro-preview",
        GeminiProfileOptions {
            enable_web_search: true,
            enable_web_fetch: true,
        },
    );
    assert!(gemini_with_web.tool_registry.get("web_search").is_some());
    assert!(gemini_with_web.tool_registry.get("web_fetch").is_some());
}

#[test]
fn provider_options_are_mapped_by_native_profile_contract() {
    let mut config = SessionConfig::default();
    config.reasoning_effort = Some("high".to_string());

    let mut openai = create_openai_profile("gpt-5.2");
    openai
        .provider_options
        .insert("temperature".to_string(), json!(0.2));
    openai
        .provider_options
        .insert("reasoning".to_string(), json!({"summary": "auto"}));
    assert_eq!(
        openai.request_provider_options(&config),
        std::collections::BTreeMap::from([(
            "openai".to_string(),
            json!({
                "temperature": 0.2,
                "reasoning": {
                    "summary": "auto",
                    "effort": "high"
                }
            })
        )])
    );

    let mut anthropic = create_anthropic_profile("claude-sonnet-4-5");
    anthropic.provider_options.insert(
        "beta_headers".to_string(),
        json!(["prompt-caching-2024-07-31"]),
    );
    assert_eq!(
        anthropic.request_provider_options(&config),
        std::collections::BTreeMap::from([(
            "anthropic".to_string(),
            json!({"beta_headers": ["prompt-caching-2024-07-31"]})
        )])
    );

    let mut gemini = create_gemini_profile("gemini-3.1-pro-preview");
    gemini.provider_options.insert(
        "safetySettings".to_string(),
        json!([{"category": "HARM_CATEGORY_DANGEROUS_CONTENT", "threshold": "BLOCK_ONLY_HIGH"}]),
    );
    gemini
        .provider_options
        .insert("groundingConfig".to_string(), json!({"enabled": true}));
    gemini.provider_options.insert(
        "thinkingConfig".to_string(),
        json!({"thinkingBudget": 1024}),
    );
    assert_eq!(
        gemini.request_provider_options(&config),
        std::collections::BTreeMap::from([(
            "gemini".to_string(),
            json!({
                "groundingConfig": {"enabled": true},
                "safetySettings": [{
                    "category": "HARM_CATEGORY_DANGEROUS_CONTENT",
                    "threshold": "BLOCK_ONLY_HIGH"
                }],
                "thinkingConfig": {"thinkingBudget": 1024}
            })
        )])
    );

    let compatible = create_openai_compatible_profile("openrouter", "anthropic/claude-sonnet-4.5");
    assert_eq!(
        compatible.request_provider_options(&config),
        std::collections::BTreeMap::from([("openrouter".to_string(), json!({}))])
    );
}

#[test]
fn provider_profiles_build_provider_specific_prompt_topics_without_string_parity() {
    let environment = spark_agent_adapter::ExecutionEnvironment::default();

    let openai_prompt = create_openai_profile("gpt-5.2").build_system_prompt(&environment);
    assert!(openai_prompt.contains("OpenAI coding agent"));
    assert!(openai_prompt.contains("codex-rs"));
    assert!(openai_prompt.contains("apply_patch"));
    assert!(openai_prompt.contains(".codex/instructions.md"));
    assert!(openai_prompt.contains("tool reports an error"));

    let anthropic_prompt =
        create_anthropic_profile("claude-sonnet-4-5").build_system_prompt(&environment);
    assert!(anthropic_prompt.contains("Anthropic coding agent"));
    assert!(anthropic_prompt.contains("Claude Code"));
    assert!(anthropic_prompt.contains("edit_file"));
    assert!(anthropic_prompt.contains("old_string"));
    assert!(anthropic_prompt.contains("CLAUDE.md"));

    let gemini_prompt =
        create_gemini_profile("gemini-3.1-pro-preview").build_system_prompt(&environment);
    assert!(gemini_prompt.contains("Gemini coding agent"));
    assert!(gemini_prompt.contains("gemini-cli"));
    assert!(gemini_prompt.contains("read_many_files"));
    assert!(gemini_prompt.contains("GEMINI.md"));
    assert!(gemini_prompt.contains("safety settings"));
    assert!(gemini_prompt.contains("grounding configuration"));
    assert!(gemini_prompt.contains("thinking configuration"));

    let compatible_prompt =
        create_openai_compatible_profile("openrouter", "anthropic/claude-sonnet-4.5")
            .build_system_prompt(&environment);
    assert!(compatible_prompt.contains("OpenAI-compatible coding agent"));
    assert!(compatible_prompt.contains("apply_patch"));
    assert!(compatible_prompt.contains("OpenRouter"));
    assert!(compatible_prompt.contains("Spark compatibility extensions"));
}

#[test]
fn spark_provider_selectors_normalize_without_erasing_native_contracts() {
    assert_eq!(normalize_provider_selector("openai").id, "openai");
    assert_eq!(
        normalize_provider_selector("anthropic").family,
        ProviderFamily::Anthropic
    );
    assert_eq!(
        normalize_provider_selector("gemini").family,
        ProviderFamily::Gemini
    );
    assert_eq!(normalize_provider_selector("codex").id, "openai_compatible");
    assert_eq!(normalize_provider_selector("openrouter").id, "openrouter");
    assert_eq!(normalize_provider_selector("litellm").id, "litellm");
    assert_eq!(
        normalize_provider_selector("openai-compatible").id,
        "openai_compatible"
    );

    let native_openai = create_provider_profile("openai", "gpt-5.2");
    assert_eq!(native_openai.id, "openai");
    assert!(native_openai.supports("reasoning"));
    assert!(native_openai.tool_registry.get("apply_patch").is_some());

    let codex = create_provider_profile("codex", "local-model");
    assert_eq!(codex.id, "openai_compatible");
    assert!(!codex.supports("reasoning"));
    assert!(codex.tool_registry.get("apply_patch").is_some());

    let openrouter = create_provider_profile("openrouter", "anthropic/claude-sonnet-4.5");
    assert_eq!(openrouter.id, "openrouter");
    assert!(!openrouter.supports("reasoning"));
    assert!(openrouter.tool_registry.get("apply_patch").is_some());
}

#[test]
fn custom_tool_registration_is_latest_wins_by_name_after_profile_creation() {
    let mut profile = create_openai_profile("gpt-5.2");
    let first = Tool::passive_with_schema(
        "read_file",
        Some("first custom read".to_string()),
        Some(json!({"type": "object"})),
    )
    .unwrap();
    let second = Tool::passive_with_schema(
        "read_file",
        Some("second custom read".to_string()),
        Some(json!({"type": "object"})),
    )
    .unwrap();

    let previous = profile.tool_registry.register(first);
    assert!(previous.is_some());
    let previous = profile.tool_registry.register(second);
    assert_eq!(
        previous.map(|tool| tool.definition.description),
        Some("first custom read".to_string())
    );

    assert_eq!(
        profile
            .tools()
            .into_iter()
            .filter(|tool| tool.name == "read_file")
            .count(),
        1
    );
    assert_eq!(
        profile
            .tool_registry
            .get("read_file")
            .map(|tool| tool.definition.description.as_str()),
        Some("second custom read")
    );

    let mut registry = ToolRegistry::new();
    registry.register(Tool::passive("lookup").unwrap());
    registry.register(
        Tool::passive_with_schema("lookup", Some("replacement".to_string()), None).unwrap(),
    );
    assert_eq!(registry.names(), ["lookup"]);
    assert_eq!(
        registry
            .get("lookup")
            .map(|tool| tool.definition.description.as_str()),
        Some("replacement")
    );
}
