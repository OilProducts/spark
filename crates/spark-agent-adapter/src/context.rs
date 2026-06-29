use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use time::OffsetDateTime;
use unified_llm_adapter::{get_model_info, ContentPart, Message};

use crate::environment::{CommandOptions, ExecutionEnvironment};
use crate::profiles::{instruction_provider_family, InstructionProviderFamily, ProviderProfile};
use crate::project_docs;

pub const CONTEXT_WARNING_THRESHOLD_RATIO: f64 = 0.8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentContext {
    pub working_directory: String,
    pub is_git_repository: bool,
    pub current_branch: String,
    pub modified_count: usize,
    pub untracked_count: usize,
    pub recent_commit_messages: Vec<String>,
    pub platform: String,
    pub os_version: String,
    pub today: String,
    pub model_display_name: String,
    pub knowledge_cutoff: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextUsageEstimate {
    pub approximate_characters: u64,
    pub approximate_tokens: f64,
    pub threshold_tokens: f64,
    pub context_window_size: u64,
    pub threshold_ratio: f64,
    pub usage_ratio: f64,
    pub exceeds_threshold: bool,
}

pub fn estimate_context_usage(
    messages: &[Message],
    context_window_size: u64,
) -> ContextUsageEstimate {
    let approximate_characters = context_character_count(messages);
    let approximate_tokens = approximate_characters as f64 / 4.0;
    let threshold_tokens = context_window_size as f64 * CONTEXT_WARNING_THRESHOLD_RATIO;
    let usage_ratio = if context_window_size == 0 {
        0.0
    } else {
        approximate_tokens / context_window_size as f64
    };
    let exceeds_threshold = context_window_size > 0
        && approximate_characters.saturating_mul(5) > context_window_size.saturating_mul(16);

    ContextUsageEstimate {
        approximate_characters,
        approximate_tokens,
        threshold_tokens,
        context_window_size,
        threshold_ratio: CONTEXT_WARNING_THRESHOLD_RATIO,
        usage_ratio,
        exceeds_threshold,
    }
}

pub fn context_usage_warning_payload(estimate: &ContextUsageEstimate) -> BTreeMap<String, Value> {
    let percent = (estimate.usage_ratio * 100.0).round() as u64;
    BTreeMap::from([
        (
            "message".to_string(),
            Value::String(format!("Context usage at ~{percent}% of context window")),
        ),
        (
            "usage".to_string(),
            json!({
                "approximate_characters": estimate.approximate_characters,
                "approximate_tokens": estimate.approximate_tokens,
                "threshold_tokens": estimate.threshold_tokens,
                "threshold_ratio": estimate.threshold_ratio,
                "context_window_size": estimate.context_window_size,
                "usage_ratio": estimate.usage_ratio,
            }),
        ),
    ])
}

pub fn snapshot_environment_context(
    profile: &ProviderProfile,
    environment: &ExecutionEnvironment,
) -> EnvironmentContext {
    let working_directory = environment.working_directory();
    let is_git_repository = is_git_repository(environment, &working_directory);
    let (current_branch, modified_count, untracked_count, recent_commit_messages) =
        if is_git_repository {
            let (modified_count, untracked_count) =
                git_status_counts(environment, &working_directory);
            (
                current_branch(environment, &working_directory),
                modified_count,
                untracked_count,
                recent_commit_messages(environment, &working_directory),
            )
        } else {
            ("unknown".to_string(), 0, 0, Vec::new())
        };

    EnvironmentContext {
        working_directory,
        is_git_repository,
        current_branch,
        modified_count,
        untracked_count,
        recent_commit_messages,
        platform: environment.platform(),
        os_version: environment.os_version(),
        today: OffsetDateTime::now_utc().date().to_string(),
        model_display_name: model_display_name(profile),
        knowledge_cutoff: knowledge_cutoff(profile),
    }
}

pub fn build_environment_context_block(context: &EnvironmentContext) -> String {
    let mut lines = vec![
        "<environment>".to_string(),
        format!("Working directory: {}", context.working_directory),
        format!(
            "Is git repository: {}",
            context.is_git_repository.to_string().to_ascii_lowercase()
        ),
        format!("Git branch: {}", context.current_branch),
        format!("Modified files: {}", context.modified_count),
        format!("Untracked files: {}", context.untracked_count),
    ];
    if context.recent_commit_messages.is_empty() {
        lines.push("Recent commit messages: none".to_string());
    } else {
        lines.push("Recent commit messages:".to_string());
        lines.extend(
            context
                .recent_commit_messages
                .iter()
                .map(|message| format!("- {message}")),
        );
    }
    lines.extend([
        format!("Platform: {}", context.platform),
        format!("OS version: {}", context.os_version),
        format!("Today's date: {}", context.today),
        format!("Model: {}", context.model_display_name),
        format!("Knowledge cutoff: {}", context.knowledge_cutoff),
        "</environment>".to_string(),
    ]);
    lines.join("\n")
}

pub fn build_provider_base_instructions(profile: &ProviderProfile) -> String {
    let mut lines = vec!["<provider_base_instructions>".to_string()];
    if profile.system_prompt.trim().is_empty() {
        lines.extend(default_provider_base_lines(instruction_provider_family(
            profile,
        )));
    } else {
        lines.extend(profile.system_prompt.lines().map(str::to_string));
    }
    lines.push("</provider_base_instructions>".to_string());
    lines.join("\n")
}

pub fn build_tool_descriptions(profile: &ProviderProfile) -> String {
    let tools = profile.tools();
    let mut lines = vec!["<tools>".to_string()];
    if tools.is_empty() {
        lines.push("No tools are registered for this profile.".to_string());
    } else {
        for tool in tools {
            lines.push(format!(
                "- {}: {}",
                tool.name,
                tool.description.unwrap_or_default()
            ));
            if let Some(parameters) = tool.parameters {
                lines.push(format!(
                    "  Parameters: {}",
                    serde_json::to_string(&parameters).unwrap_or_else(|_| parameters.to_string())
                ));
            }
            if !tool.provider_metadata.is_empty() {
                let metadata = Value::Object(tool.provider_metadata.into_iter().collect());
                lines.push(format!(
                    "  Metadata: {}",
                    serde_json::to_string(&metadata).unwrap_or_else(|_| metadata.to_string())
                ));
            }
        }
    }
    lines.push("</tools>".to_string());
    lines.join("\n")
}

pub fn build_system_prompt(
    profile: &ProviderProfile,
    environment: &ExecutionEnvironment,
) -> String {
    build_system_prompt_with_user_overrides(profile, environment, None)
}

pub fn build_system_prompt_with_user_overrides(
    profile: &ProviderProfile,
    environment: &ExecutionEnvironment,
    user_overrides: Option<&str>,
) -> String {
    let context = snapshot_environment_context(profile, environment);
    let project_documents = project_docs::discover_project_documents(environment, profile);
    build_system_prompt_from_snapshot(profile, &context, &project_documents, user_overrides)
}

pub fn build_system_prompt_from_snapshot(
    profile: &ProviderProfile,
    context: &EnvironmentContext,
    project_documents: &project_docs::ProjectDocuments,
    user_overrides: Option<&str>,
) -> String {
    let mut layers = vec![
        build_provider_base_instructions(profile),
        build_environment_context_block(context),
        build_tool_descriptions(profile),
    ];

    let project_documents_text = project_docs::render_project_documents(project_documents);
    if !project_documents_text.trim().is_empty() {
        layers.push(format!(
            "<project_instructions>\n{}\n</project_instructions>",
            project_documents_text
        ));
    }

    if let Some(user_overrides) = render_user_overrides(user_overrides) {
        layers.push(user_overrides);
    }

    layers.join("\n\n")
}

fn render_user_overrides(user_overrides: Option<&str>) -> Option<String> {
    let text = user_overrides?.trim();
    if text.is_empty() {
        return None;
    }
    Some(format!("<user_overrides>\n{text}\n</user_overrides>"))
}

fn context_character_count(messages: &[Message]) -> u64 {
    messages
        .iter()
        .flat_map(|message| message.content.iter())
        .map(content_part_character_count)
        .sum()
}

fn content_part_character_count(part: &ContentPart) -> u64 {
    match part {
        ContentPart::Text { text } => text.chars().count() as u64,
        ContentPart::Thinking { thinking } | ContentPart::RedactedThinking { thinking } => {
            thinking.text.chars().count() as u64
        }
        ContentPart::ToolCall { tool_call } => {
            tool_call.id.chars().count() as u64
                + tool_call.name.chars().count() as u64
                + tool_call
                    .raw_arguments
                    .as_deref()
                    .map(|value| value.chars().count() as u64)
                    .unwrap_or_else(|| value_character_count(&tool_call.arguments))
        }
        ContentPart::ToolResult { tool_result } => {
            tool_result.tool_call_id.chars().count() as u64
                + value_character_count(&tool_result.content)
        }
        ContentPart::Image { image } => {
            option_character_count(image.url.as_deref())
                + image.data.as_ref().map(Vec::len).unwrap_or(0) as u64
                + option_character_count(image.media_type.as_deref())
                + option_character_count(image.detail.as_deref())
        }
        ContentPart::Audio { audio } => {
            option_character_count(audio.url.as_deref())
                + audio.data.as_ref().map(Vec::len).unwrap_or(0) as u64
                + option_character_count(audio.media_type.as_deref())
        }
        ContentPart::Document { document } => {
            option_character_count(document.url.as_deref())
                + document.data.as_ref().map(Vec::len).unwrap_or(0) as u64
                + option_character_count(document.media_type.as_deref())
                + option_character_count(document.file_name.as_deref())
        }
        ContentPart::Custom { raw, .. }
        | ContentPart::Raw { raw, .. }
        | ContentPart::Provider { raw } => value_character_count(raw),
    }
}

fn option_character_count(value: Option<&str>) -> u64 {
    value.map(|value| value.chars().count() as u64).unwrap_or(0)
}

fn value_character_count(value: &Value) -> u64 {
    match value {
        Value::Null => 0,
        Value::Bool(value) => value.to_string().chars().count() as u64,
        Value::Number(value) => value.to_string().chars().count() as u64,
        Value::String(value) => value.chars().count() as u64,
        Value::Array(values) => values.iter().map(value_character_count).sum(),
        Value::Object(values) => values.values().map(value_character_count).sum(),
    }
}

fn default_provider_base_lines(provider_family: Option<InstructionProviderFamily>) -> Vec<String> {
    match provider_family {
        Some(InstructionProviderFamily::OpenAI) => vec![
            "Provider identity: OpenAI coding agent.".to_string(),
            "Tool usage: inspect before editing, use the available tools deliberately, and prefer apply_patch for targeted edits.".to_string(),
            "Project instruction conventions: AGENTS.md is always loaded; deeper project docs override shallower ones; .codex/instructions.md is OpenAI-only.".to_string(),
            "Coding guidance: preserve observable behavior, update tests when behavior changes, and keep diffs minimal.".to_string(),
        ],
        Some(InstructionProviderFamily::Anthropic) => vec![
            "Provider identity: Anthropic coding agent.".to_string(),
            "Tool usage: read before edit, use read_file to inspect existing files, and prefer edit_file for targeted replacements.".to_string(),
            "Project instruction conventions: AGENTS.md is always loaded; deeper project docs override shallower ones; CLAUDE.md is Anthropic-only.".to_string(),
            "Coding guidance: preserve observable behavior, update tests when behavior changes, and keep edits readable.".to_string(),
        ],
        Some(InstructionProviderFamily::Gemini) => vec![
            "Provider identity: Gemini coding agent.".to_string(),
            "Tool usage: inspect before editing, use tools intentionally, and keep changes focused.".to_string(),
            "Project instruction conventions: AGENTS.md is always loaded; deeper project docs override shallower ones; GEMINI.md is Gemini-only.".to_string(),
            "Coding guidance: preserve observable behavior, update tests when behavior changes, and keep edits readable.".to_string(),
        ],
        None => vec![
            "Provider identity: Unified LLM coding agent.".to_string(),
            "Tool usage: inspect before editing, use the available tools deliberately, and keep the request path concise.".to_string(),
            "Project instruction conventions: AGENTS.md is always loaded and deeper project docs override shallower ones.".to_string(),
            "Coding guidance: preserve observable behavior, update tests when behavior changes, and keep edits readable.".to_string(),
        ],
    }
}

fn is_git_repository(environment: &ExecutionEnvironment, working_directory: &str) -> bool {
    exec_command_candidates(
        environment,
        "git rev-parse --is-inside-work-tree",
        working_directory,
    )
    .as_deref()
        == Some("true")
}

fn current_branch(environment: &ExecutionEnvironment, working_directory: &str) -> String {
    exec_command_candidates(environment, "git branch --show-current", working_directory)
        .or_else(|| {
            exec_command_candidates(
                environment,
                "git rev-parse --abbrev-ref HEAD",
                working_directory,
            )
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn git_status_counts(
    environment: &ExecutionEnvironment,
    working_directory: &str,
) -> (usize, usize) {
    let Some(output) =
        exec_command_candidates(environment, "git status --porcelain=v1", working_directory)
    else {
        return (0, 0);
    };

    output.lines().fold((0, 0), |(modified, untracked), line| {
        if line.starts_with("??") {
            (modified, untracked + 1)
        } else if line.trim().is_empty() {
            (modified, untracked)
        } else {
            (modified + 1, untracked)
        }
    })
}

fn recent_commit_messages(
    environment: &ExecutionEnvironment,
    working_directory: &str,
) -> Vec<String> {
    exec_command_candidates(
        environment,
        "git log -n 5 --pretty=format:%s",
        working_directory,
    )
    .map(|output| {
        output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}

fn exec_command_candidates(
    environment: &ExecutionEnvironment,
    command: &str,
    working_directory: &str,
) -> Option<String> {
    let mut candidates = vec![working_directory.to_string()];
    let resolved = resolve_path_text(working_directory)
        .to_string_lossy()
        .to_string();
    if !candidates.contains(&resolved) {
        candidates.push(resolved);
    }

    candidates.into_iter().find_map(|candidate| {
        let result = environment
            .exec_command(
                command,
                CommandOptions {
                    working_dir: Some(PathBuf::from(candidate)),
                    ..CommandOptions::default()
                },
            )
            .ok()?;
        if result.exit_code != 0 {
            return None;
        }
        let output = result.stdout.trim();
        (!output.is_empty()).then(|| output.to_string())
    })
}

fn resolve_path_text(path_text: &str) -> PathBuf {
    let path = PathBuf::from(path_text);
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn model_display_name(profile: &ProviderProfile) -> String {
    profile
        .display_name
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .or_else(|| {
            (!profile.model.trim().is_empty()).then(|| {
                get_model_info(&profile.model)
                    .map(|model| model.display_name)
                    .unwrap_or_else(|| profile.model.clone())
            })
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn knowledge_cutoff(profile: &ProviderProfile) -> String {
    profile
        .knowledge_cutoff_date
        .as_ref()
        .or(profile.knowledge_cutoff.as_ref())
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| "unknown".to_string())
}
