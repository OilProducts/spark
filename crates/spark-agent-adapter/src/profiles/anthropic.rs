use crate::profiles::ProviderProfile;
use crate::tools::builtins;
use crate::tools::ToolRegistry;

pub fn create_anthropic_profile(model: impl Into<String>) -> ProviderProfile {
    let mut profile = ProviderProfile::new("anthropic", model);
    profile.system_prompt = anthropic_system_prompt();
    profile.supports_streaming = true;
    profile.supports_parallel_tool_calls = true;
    profile.apply_model_metadata();
    if profile.display_name.is_none() && profile.model.is_empty() {
        profile.display_name = Some("Anthropic".to_string());
    }
    profile.set_tool_registry(builtins::build_anthropic_tool_registry_with_capabilities(
        profile.capabilities.clone(),
    ));
    profile
}

pub fn build_anthropic_tool_registry() -> ToolRegistry {
    builtins::build_anthropic_tool_registry()
}

fn anthropic_system_prompt() -> String {
    [
        "You are Spark's Anthropic coding agent, aligned with Claude Code model-facing conventions.",
        "Use the available tools to read and understand files before editing them.",
        "For existing-file edits, prefer edit_file with old_string and new_string. The old_string must match exactly and be unique unless the tool request explicitly allows multiple replacements.",
        "Do not use apply_patch as the Anthropic native edit path; use write_file only for new files or deliberate full-file rewrites.",
        "Shell commands use the Anthropic profile's longer default timeout unless a tool call supplies timeout_ms.",
        "Follow project instructions from AGENTS.md and Anthropic-specific CLAUDE.md when those instructions are present in the prompt context.",
        "Use tool errors as recoverable feedback, ask for missing information only when needed, and avoid pretending that an edit or command succeeded.",
        "Preserve observable behavior, keep changes readable, and update tests when behavior changes.",
        "Use subagent tools through the normal tool-calling path when a bounded child investigation is useful.",
    ]
    .join("\n")
}
