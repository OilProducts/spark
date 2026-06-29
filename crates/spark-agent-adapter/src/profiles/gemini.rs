use crate::profiles::ProviderProfile;
use crate::tools::builtins;
use crate::tools::ToolRegistry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GeminiProfileOptions {
    pub enable_web_search: bool,
    pub enable_web_fetch: bool,
}

pub fn create_gemini_profile(model: impl Into<String>) -> ProviderProfile {
    create_gemini_profile_with_options(model, GeminiProfileOptions::default())
}

pub fn create_gemini_profile_with_options(
    model: impl Into<String>,
    options: GeminiProfileOptions,
) -> ProviderProfile {
    let mut profile = ProviderProfile::new("gemini", model);
    profile.system_prompt = gemini_system_prompt();
    profile.supports_streaming = true;
    profile.supports_parallel_tool_calls = true;
    profile.apply_model_metadata();
    if profile.display_name.is_none() && profile.model.is_empty() {
        profile.display_name = Some("Gemini".to_string());
    }
    profile.set_tool_registry(builtins::build_gemini_tool_registry_with_capabilities(
        options.enable_web_search,
        options.enable_web_fetch,
        profile.capabilities.clone(),
    ));
    profile
}

pub fn build_gemini_tool_registry(options: GeminiProfileOptions) -> ToolRegistry {
    builtins::build_gemini_tool_registry(options.enable_web_search, options.enable_web_fetch)
}

fn gemini_system_prompt() -> String {
    [
        "You are Spark's Gemini coding agent, aligned with gemini-cli model-facing conventions.",
        "Use read_file, read_many_files, grep, glob, list_dir, and shell to inspect the workspace before changing it.",
        "For edits, use the Gemini edit_file search-and-replace shape with an instruction plus old_string and new_string; keep full-file rewrites for cases where a replacement is not appropriate.",
        "Use read_many_files for coordinated multi-file context and list_dir when directory shape matters.",
        "Follow project instructions from AGENTS.md and Gemini-specific GEMINI.md when those instructions are present in the prompt context.",
        "Gemini safety settings, grounding configuration, and thinking configuration are provider options supplied through the Gemini request options map.",
        "Treat tool errors as recoverable feedback, keep changes focused, and update tests when behavior changes.",
        "Subagent tools are available as model-facing definitions only; do not assume a Python-backed subagent runtime.",
    ]
    .join("\n")
}
