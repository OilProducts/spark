use crate::profiles::ProviderProfile;
use crate::tools::builtins;
use crate::tools::ToolRegistry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFamily {
    OpenAI,
    Anthropic,
    Gemini,
    OpenAICompatible,
}

pub fn create_provider_profile(selector: &str, model: impl Into<String>) -> ProviderProfile {
    match normalize_provider_selector(selector).family {
        ProviderFamily::OpenAI => create_openai_profile(model),
        ProviderFamily::Anthropic => crate::profiles::anthropic::create_anthropic_profile(model),
        ProviderFamily::Gemini => crate::profiles::gemini::create_gemini_profile(model),
        ProviderFamily::OpenAICompatible => create_openai_compatible_profile(selector, model),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedProviderSelector {
    pub id: String,
    pub family: ProviderFamily,
}

pub fn normalize_provider_selector(selector: &str) -> NormalizedProviderSelector {
    let normalized = selector.trim().to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "openai" => NormalizedProviderSelector {
            id: "openai".to_string(),
            family: ProviderFamily::OpenAI,
        },
        "anthropic" | "claude" | "claude_code" => NormalizedProviderSelector {
            id: "anthropic".to_string(),
            family: ProviderFamily::Anthropic,
        },
        "gemini" | "google" | "google_gemini" => NormalizedProviderSelector {
            id: "gemini".to_string(),
            family: ProviderFamily::Gemini,
        },
        "openrouter" => NormalizedProviderSelector {
            id: "openrouter".to_string(),
            family: ProviderFamily::OpenAICompatible,
        },
        "litellm" => NormalizedProviderSelector {
            id: "litellm".to_string(),
            family: ProviderFamily::OpenAICompatible,
        },
        "codex" | "openai_compatible" | "compatible" => NormalizedProviderSelector {
            id: "openai_compatible".to_string(),
            family: ProviderFamily::OpenAICompatible,
        },
        _ => NormalizedProviderSelector {
            id: normalized,
            family: ProviderFamily::OpenAICompatible,
        },
    }
}

pub fn create_openai_profile(model: impl Into<String>) -> ProviderProfile {
    let mut profile = ProviderProfile::new("openai", model);
    profile.system_prompt = openai_system_prompt();
    profile.supports_streaming = true;
    profile.supports_parallel_tool_calls = true;
    profile.apply_model_metadata();
    profile.set_tool_registry(builtins::build_openai_tool_registry_with_capabilities(
        profile.capabilities.clone(),
    ));
    profile
}

pub fn create_openai_compatible_profile(
    selector: &str,
    model: impl Into<String>,
) -> ProviderProfile {
    let normalized = normalize_provider_selector(selector);
    let mut profile = ProviderProfile::new(normalized.id, model);
    profile.system_prompt = openai_compatible_system_prompt();
    profile.display_name = match profile.id.as_str() {
        "openrouter" => Some("OpenRouter".to_string()),
        "litellm" => Some("LiteLLM".to_string()),
        "openai_compatible" => Some("OpenAI Compatible".to_string()),
        _ => profile.display_name,
    };
    profile.capabilities.insert("reasoning".to_string(), false);
    profile.supports_reasoning = false;
    profile.set_tool_registry(build_openai_compatible_tool_registry());
    profile
}

pub fn build_openai_tool_registry() -> ToolRegistry {
    builtins::build_openai_tool_registry()
}

pub fn build_openai_compatible_tool_registry() -> ToolRegistry {
    builtins::build_openai_compatible_tool_registry()
}

fn openai_system_prompt() -> String {
    [
        "You are Spark's OpenAI coding agent, aligned with codex-rs model-facing conventions.",
        "Use the available tools to inspect real project state before making claims or edits.",
        "For targeted file changes, prefer apply_patch and use the *** Begin Patch / *** End Patch patch format. Keep write_file for new files or deliberate full-file rewrites.",
        "Run shell commands only when they help validate or inspect behavior, and treat the default shell timeout as a short command budget.",
        "Follow project instructions from AGENTS.md and OpenAI-specific .codex/instructions.md when those instructions are present in the prompt context.",
        "When a tool reports an error, use the error data to recover or explain the blocker instead of inventing results.",
        "Preserve observable behavior, keep diffs focused, and update tests when behavior changes.",
        "Subagent tools are available as model-facing definitions only; do not assume a Python-backed subagent runtime.",
    ]
    .join("\n")
}

fn openai_compatible_system_prompt() -> String {
    [
        "You are Spark's OpenAI-compatible coding agent, using OpenAI-style tool calling conventions.",
        "Use read_file, grep, glob, and shell to inspect the workspace before acting.",
        "Prefer apply_patch for targeted edits and write_file only for new files or complete rewrites.",
        "Keep changes focused, preserve observable behavior, and validate the result with the most relevant tests or commands.",
        "Treat OpenAI-compatible providers such as OpenRouter and LiteLLM as Spark compatibility extensions, not as native Anthropic or Gemini profiles.",
        "Subagent tools are exposed as definitions only in this milestone and must not invoke Python agent code.",
    ]
    .join("\n")
}
