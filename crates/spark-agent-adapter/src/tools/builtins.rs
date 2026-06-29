use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};
use unified_llm_adapter::{AdapterError, Tool};

use crate::apply_patch::{self, ApplyPatchError};
use crate::environment::{CommandOptions, EnvironmentError, ExecResult, GrepOptions};
use crate::tools::{
    RegisteredTool, ToolDefinition, ToolExecution, ToolExecutionOutput, ToolRegistry,
};

pub const DEFAULT_READ_FILE_LIMIT: u64 = 2_000;
pub const OPENAI_SHELL_TIMEOUT_MS: u64 = 10_000;
pub const ANTHROPIC_SHELL_TIMEOUT_MS: u64 = 120_000;
pub const GEMINI_SHELL_TIMEOUT_MS: u64 = 10_000;
pub const ANTHROPIC_GREP_HEAD_LIMIT: u64 = 250;
pub const GEMINI_WEB_SEARCH_RESULTS: u64 = 5;
pub const GEMINI_WEB_FETCH_BYTES: u64 = 20_000;
pub const GEMINI_WEB_FETCH_TIMEOUT_MS: u64 = 10_000;

pub fn build_openai_tool_registry() -> ToolRegistry {
    build_openai_tool_registry_with_capabilities(BTreeMap::new())
}

pub(crate) fn build_openai_tool_registry_with_capabilities(
    capabilities: BTreeMap<String, bool>,
) -> ToolRegistry {
    registry_from_tools(
        vec![
            registered_read_file_tool(
                read_file_tool(
                    "path",
                    "Read a file and return numbered text lines or image data.",
                ),
                capabilities.clone(),
            ),
            registered_apply_patch_tool(apply_patch_tool()),
            registered_write_file_tool(write_file_tool("path")),
            registered_shell_tool(shell_tool(OPENAI_SHELL_TIMEOUT_MS), OPENAI_SHELL_TIMEOUT_MS),
            registered_grep_tool(grep_tool(), GrepStyle::Structured),
            registered_glob_tool(glob_tool()),
        ]
        .into_iter()
        .chain(subagent_tools().into_iter().map(RegisteredTool::from)),
    )
}

pub fn build_anthropic_tool_registry() -> ToolRegistry {
    build_anthropic_tool_registry_with_capabilities(BTreeMap::new())
}

pub(crate) fn build_anthropic_tool_registry_with_capabilities(
    capabilities: BTreeMap<String, bool>,
) -> ToolRegistry {
    registry_from_tools(
        vec![
            registered_read_file_tool(
                read_file_tool(
                    "file_path",
                    "Read a file and return line-numbered text or image data.",
                ),
                capabilities.clone(),
            ),
            registered_write_file_tool(write_file_tool("file_path")),
            registered_edit_file_tool(anthropic_edit_file_tool()),
            registered_shell_tool(
                shell_tool(ANTHROPIC_SHELL_TIMEOUT_MS),
                ANTHROPIC_SHELL_TIMEOUT_MS,
            ),
            registered_grep_tool(anthropic_grep_tool(), GrepStyle::Anthropic),
            registered_glob_tool(glob_tool()),
        ]
        .into_iter()
        .chain(subagent_tools().into_iter().map(RegisteredTool::from)),
    )
}

pub fn build_gemini_tool_registry(enable_web_search: bool, enable_web_fetch: bool) -> ToolRegistry {
    build_gemini_tool_registry_with_capabilities(
        enable_web_search,
        enable_web_fetch,
        BTreeMap::new(),
    )
}

pub(crate) fn build_gemini_tool_registry_with_capabilities(
    enable_web_search: bool,
    enable_web_fetch: bool,
    capabilities: BTreeMap<String, bool>,
) -> ToolRegistry {
    let mut tools = vec![
        registered_read_file_tool(
            read_file_tool(
                "file_path",
                "Read a file and return line-numbered text or image data.",
            ),
            capabilities,
        ),
        registered_read_many_files_tool(read_many_files_tool()),
        registered_write_file_tool(write_file_tool("file_path")),
        registered_edit_file_tool(gemini_edit_file_tool()),
        registered_shell_tool(shell_tool(GEMINI_SHELL_TIMEOUT_MS), GEMINI_SHELL_TIMEOUT_MS),
        registered_grep_tool(grep_tool(), GrepStyle::Structured),
        registered_glob_tool(glob_tool()),
        registered_list_dir_tool(list_dir_tool()),
    ];
    if enable_web_search {
        tools.push(RegisteredTool::from(web_search_tool()));
    }
    if enable_web_fetch {
        tools.push(RegisteredTool::from(web_fetch_tool()));
    }
    tools.extend(subagent_tools().into_iter().map(RegisteredTool::from));
    registry_from_tools(tools)
}

pub fn build_openai_compatible_tool_registry() -> ToolRegistry {
    build_openai_tool_registry()
}

pub fn subagent_tools() -> Vec<Tool> {
    vec![
        object_tool(
            "spawn_agent",
            "Spawn a child agent session and start it on a task.",
            json!({
                "task": {"type": "string", "minLength": 1},
                "working_dir": {"type": "string", "minLength": 1},
                "model": {"type": "string"},
                "max_turns": {"type": "integer", "minimum": 0}
            }),
            &["task"],
        ),
        object_tool(
            "send_input",
            "Send a follow-up message to a running child agent.",
            json!({
                "agent_id": {"type": "string", "minLength": 1},
                "message": {"type": "string", "minLength": 1}
            }),
            &["agent_id", "message"],
        ),
        object_tool(
            "wait",
            "Wait for a child agent to finish and return its result.",
            json!({
                "agent_id": {"type": "string", "minLength": 1}
            }),
            &["agent_id"],
        ),
        object_tool(
            "close_agent",
            "Close a child agent session and cancel any running work.",
            json!({
                "agent_id": {"type": "string", "minLength": 1}
            }),
            &["agent_id"],
        ),
    ]
}

fn registry_from_tools(tools: impl IntoIterator<Item = RegisteredTool>) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    for tool in tools {
        registry.register(tool);
    }
    registry
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GrepStyle {
    Structured,
    Anthropic,
}

fn registered_read_file_tool(
    tool: Tool,
    default_capabilities: BTreeMap<String, bool>,
) -> RegisteredTool {
    registered_tool(tool, move |execution| {
        let Some(path) = path_argument(&execution.arguments, &["path", "file_path"]) else {
            return Ok(tool_error("Missing required argument: path"));
        };
        let Some(offset) = integer_argument(&execution.arguments, "offset", 1, 1) else {
            return Ok(tool_error("offset must be at least 1"));
        };
        let limit = integer_argument(
            &execution.arguments,
            "limit",
            DEFAULT_READ_FILE_LIMIT as usize,
            0,
        );
        let Some(limit) = limit else {
            return Ok(tool_error("limit must be non-negative"));
        };

        match execution
            .execution_environment
            .read_file(&path, Some(offset), Some(limit))
        {
            Ok(content) => Ok(ToolExecutionOutput::success(Value::String(
                format_numbered_text(&content, offset),
            ))),
            Err(EnvironmentError::InvalidUtf8(_))
                if supports_image_results(&execution, &default_capabilities) =>
            {
                match execution.execution_environment.read_file_bytes(&path) {
                    Ok(payload) => {
                        let Some(media_type) = guess_image_media_type(&path, &payload) else {
                            return Ok(tool_error(format!("Binary file not supported: {path}")));
                        };
                        Ok(ToolExecutionOutput::success(Value::String(format!(
                            "Read image file: {path} ({media_type}, {} bytes)",
                            payload.len()
                        )))
                        .with_image(payload, Some(media_type.to_string())))
                    }
                    Err(error) => Ok(tool_error(file_error_message(&path, error))),
                }
            }
            Err(error) => Ok(tool_error(file_error_message(&path, error))),
        }
    })
}

fn registered_write_file_tool(tool: Tool) -> RegisteredTool {
    registered_tool(tool, |execution| {
        let Some(path) = path_argument(&execution.arguments, &["path", "file_path"]) else {
            return Ok(tool_error("Missing required argument: path"));
        };
        let Some(content) = string_argument(&execution.arguments, &["content", "text"]) else {
            return Ok(tool_error("Missing required argument: content"));
        };
        let bytes_written = content.len();
        match execution.execution_environment.write_file(&path, &content) {
            Ok(()) => Ok(ToolExecutionOutput::success(json!({
                "path": path,
                "bytes_written": bytes_written
            }))),
            Err(error) => Ok(tool_error(write_error_message(&path, error))),
        }
    })
}

fn registered_apply_patch_tool(tool: Tool) -> RegisteredTool {
    registered_tool(tool, |execution| {
        let Some(patch) = string_argument(&execution.arguments, &["patch"]) else {
            return Ok(tool_error(
                "Patch parse error: missing required argument: patch",
            ));
        };
        if patch.trim().is_empty() {
            return Ok(tool_error(
                "Patch parse error: missing required argument: patch",
            ));
        }
        match apply_patch::apply_patch(&execution.execution_environment, &patch) {
            Ok(results) => Ok(ToolExecutionOutput::success(Value::Array(results))),
            Err(ApplyPatchError::Parse(message)) => {
                Ok(tool_error(format!("Patch parse error: {message}")))
            }
            Err(ApplyPatchError::Apply(message)) => Ok(tool_error(message)),
        }
    })
}

fn registered_edit_file_tool(tool: Tool) -> RegisteredTool {
    registered_tool(tool, |execution| {
        let Some(path) = path_argument(&execution.arguments, &["path", "file_path"]) else {
            return Ok(tool_error("Missing required argument: path"));
        };
        let Some(old_string) = string_argument(&execution.arguments, &["old_string", "old"]) else {
            return Ok(tool_error("Missing required argument: old_string"));
        };
        if old_string.is_empty() {
            return Ok(tool_error("old_string must not be empty"));
        }
        let Some(new_string) = string_argument(&execution.arguments, &["new_string", "new"]) else {
            return Ok(tool_error("Missing required argument: new_string"));
        };
        let replace_all = match replacement_all_argument(&execution.arguments) {
            Ok(value) => value,
            Err(message) => return Ok(tool_error(message)),
        };
        let current = match execution.execution_environment.read_file(&path, None, None) {
            Ok(content) => content,
            Err(error) => return Ok(tool_error(file_error_message(&path, error))),
        };
        let occurrences = current.matches(&old_string).count();
        if occurrences == 0 {
            return Ok(tool_error(format!("old_string not found in {path}")));
        }
        if !replace_all && occurrences != 1 {
            return Ok(tool_error(format!(
                "old_string is not unique in {path}: {occurrences} matches"
            )));
        }
        let updated = if replace_all {
            current.replace(&old_string, &new_string)
        } else {
            current.replacen(&old_string, &new_string, 1)
        };
        let bytes_written = updated.len();
        match execution.execution_environment.write_file(&path, &updated) {
            Ok(()) => Ok(ToolExecutionOutput::success(json!({
                "path": path,
                "replacements": if replace_all { occurrences } else { 1 },
                "bytes_written": bytes_written,
                "replace_all": replace_all
            }))),
            Err(error) => Ok(tool_error(write_error_message(&path, error))),
        }
    })
}

fn registered_shell_tool(tool: Tool, provider_default_timeout_ms: u64) -> RegisteredTool {
    registered_tool(tool, move |execution| {
        let Some(command) = string_argument(&execution.arguments, &["command"]) else {
            return Ok(tool_error("Missing required argument: command"));
        };
        if command.trim().is_empty() {
            return Ok(tool_error("Missing required argument: command"));
        }
        let timeout_ms = match optional_u64_argument(&execution.arguments, "timeout_ms", 1) {
            Some(Some(value)) => value,
            Some(None) => return Ok(tool_error("timeout_ms must be at least 1")),
            None => provider_default_timeout_ms,
        }
        .min(execution.config.max_command_timeout_ms);
        let working_dir = path_argument(&execution.arguments, &["working_dir"]).map(PathBuf::from);
        let options = CommandOptions {
            timeout_ms: Some(timeout_ms),
            working_dir,
            env_vars: BTreeMap::new(),
        };
        match execution
            .execution_environment
            .exec_command(&command, options)
        {
            Ok(result) => Ok(exec_result_output(result)),
            Err(error) => Ok(tool_error(shell_error_message(&command, error))),
        }
    })
}

fn registered_grep_tool(tool: Tool, style: GrepStyle) -> RegisteredTool {
    registered_tool(tool, move |execution| {
        let Some(pattern) = string_argument(&execution.arguments, &["pattern"]) else {
            return Ok(tool_error("Missing required argument: pattern"));
        };
        if pattern.trim().is_empty() {
            return Ok(tool_error("Missing required argument: pattern"));
        }
        let path = match execution.arguments.get("path") {
            Some(_) => match path_argument(&execution.arguments, &["path"]) {
                Some(path) => path,
                None => return Ok(tool_error("Missing required argument: path")),
            },
            None => ".".to_string(),
        };
        let glob_filter = string_argument(&execution.arguments, &["glob_filter", "glob"]);
        if execution.arguments.get("glob_filter").is_some() && glob_filter.is_none() {
            return Ok(tool_error("glob_filter must be a string"));
        }
        if execution.arguments.get("glob").is_some() && glob_filter.is_none() {
            return Ok(tool_error("glob must be a string"));
        }
        let case_insensitive = bool_argument(
            &execution.arguments,
            if style == GrepStyle::Anthropic {
                "-i"
            } else {
                "case_insensitive"
            },
            false,
        );
        let Some(case_insensitive) = case_insensitive else {
            return Ok(tool_error("case_insensitive must be a boolean"));
        };
        let default_max_results = if style == GrepStyle::Anthropic {
            10_000
        } else {
            100
        };
        let max_results = integer_argument(
            &execution.arguments,
            if style == GrepStyle::Anthropic {
                "head_limit"
            } else {
                "max_results"
            },
            default_max_results,
            1,
        );
        let Some(max_results) = max_results else {
            return Ok(tool_error("max_results must be at least 1"));
        };
        let options = GrepOptions {
            glob_filter,
            case_insensitive,
            max_results,
        };
        let output = match execution
            .execution_environment
            .grep(&pattern, &path, &options)
        {
            Ok(output) => output,
            Err(EnvironmentError::InvalidInput(message)) => {
                return Ok(tool_error(format!("Invalid regex pattern: {message}")));
            }
            Err(error) => return Ok(tool_error(search_error_message(&path, error))),
        };
        let matches = parse_grep_matches(&output);
        if style == GrepStyle::Anthropic {
            let output_mode = string_argument(&execution.arguments, &["output_mode"])
                .unwrap_or_else(|| "files_with_matches".to_string());
            return Ok(ToolExecutionOutput::success(anthropic_grep_payload(
                &matches,
                &output_mode,
            )));
        }
        Ok(ToolExecutionOutput::success(json!({"matches": matches})))
    })
}

fn registered_glob_tool(tool: Tool) -> RegisteredTool {
    registered_tool(tool, |execution| {
        let Some(pattern) = string_argument(&execution.arguments, &["pattern"]) else {
            return Ok(tool_error("Missing required argument: pattern"));
        };
        if pattern.trim().is_empty() {
            return Ok(tool_error("Missing required argument: pattern"));
        }
        let path = match execution.arguments.get("path") {
            Some(_) => match path_argument(&execution.arguments, &["path"]) {
                Some(path) => path,
                None => return Ok(tool_error("Missing required argument: path")),
            },
            None => ".".to_string(),
        };
        match execution.execution_environment.glob(&pattern, &path) {
            Ok(matches) => Ok(ToolExecutionOutput::success(json!(matches))),
            Err(EnvironmentError::InvalidInput(message)) => {
                Ok(tool_error(format!("Invalid glob pattern: {message}")))
            }
            Err(error) => Ok(tool_error(search_error_message(&path, error))),
        }
    })
}

fn registered_read_many_files_tool(tool: Tool) -> RegisteredTool {
    registered_tool(tool, |execution| {
        let Some(paths) = execution
            .arguments
            .get("paths")
            .and_then(|value| value.as_array())
        else {
            return Ok(tool_error("Missing required argument: paths"));
        };
        if paths.is_empty() {
            return Ok(tool_error("paths must not be empty"));
        }
        let mut files = Vec::new();
        for path_value in paths {
            let Some(path) = path_value.as_str().filter(|value| !value.trim().is_empty()) else {
                return Ok(tool_error("paths must be strings or paths"));
            };
            let content = match execution.execution_environment.read_file(
                path,
                Some(1),
                Some(DEFAULT_READ_FILE_LIMIT as usize),
            ) {
                Ok(content) => content,
                Err(error) => return Ok(tool_error(file_error_message(path, error))),
            };
            files.push(json!({
                "path": path,
                "content": format_numbered_text(&content, 1)
            }));
        }
        Ok(ToolExecutionOutput::success(json!({
            "count": files.len(),
            "files": files
        })))
    })
}

fn registered_list_dir_tool(tool: Tool) -> RegisteredTool {
    registered_tool(tool, |execution| {
        let Some(path) = path_argument(&execution.arguments, &["path", "directory"]) else {
            return Ok(tool_error("Missing required argument: path"));
        };
        let Some(depth) = integer_argument(&execution.arguments, "depth", 0, 0) else {
            return Ok(tool_error("depth must be non-negative"));
        };
        match execution.execution_environment.list_directory(&path, depth) {
            Ok(entries) => Ok(ToolExecutionOutput::success(json!({
                "path": path,
                "depth": depth,
                "count": entries.len(),
                "entries": entries
            }))),
            Err(EnvironmentError::NotDirectory(_)) => {
                Ok(tool_error(format!("Not a directory: {path}")))
            }
            Err(error) => Ok(tool_error(file_error_message(&path, error))),
        }
    })
}

fn registered_tool(
    tool: Tool,
    executor: impl Fn(ToolExecution) -> Result<ToolExecutionOutput, AdapterError>
        + Send
        + Sync
        + 'static,
) -> RegisteredTool {
    let definition =
        ToolDefinition::try_from(tool).expect("built-in tool must convert to definition");
    RegisteredTool::new_with_executor(definition, Arc::new(executor))
}

fn tool_error(message: impl Into<String>) -> ToolExecutionOutput {
    ToolExecutionOutput::error(Value::String(message.into()))
}

fn string_argument(arguments: &Value, names: &[&str]) -> Option<String> {
    let object = arguments.as_object()?;
    for name in names {
        let Some(value) = object.get(*name) else {
            continue;
        };
        let Some(text) = value.as_str() else {
            return None;
        };
        return Some(text.to_string());
    }
    None
}

fn path_argument(arguments: &Value, names: &[&str]) -> Option<String> {
    string_argument(arguments, names).filter(|value| !value.trim().is_empty())
}

fn integer_argument(
    arguments: &Value,
    name: &str,
    default: usize,
    minimum: usize,
) -> Option<usize> {
    let Some(value) = arguments.as_object().and_then(|object| object.get(name)) else {
        return Some(default);
    };
    let value = value.as_u64()?;
    let value = usize::try_from(value).ok()?;
    if value < minimum {
        return None;
    }
    Some(value)
}

fn optional_u64_argument(arguments: &Value, name: &str, minimum: u64) -> Option<Option<u64>> {
    let value = arguments.as_object().and_then(|object| object.get(name))?;
    let value = value.as_u64()?;
    if value < minimum {
        return Some(None);
    }
    Some(Some(value))
}

fn bool_argument(arguments: &Value, name: &str, default: bool) -> Option<bool> {
    let Some(value) = arguments.as_object().and_then(|object| object.get(name)) else {
        return Some(default);
    };
    value.as_bool()
}

fn replacement_all_argument(arguments: &Value) -> Result<bool, &'static str> {
    let Some(object) = arguments.as_object() else {
        return Ok(false);
    };
    if let Some(value) = object.get("replace_all") {
        return value.as_bool().ok_or("replace_all must be a boolean");
    }
    if let Some(value) = object.get("allow_multiple") {
        return value.as_bool().ok_or("allow_multiple must be a boolean");
    }
    Ok(false)
}

fn format_numbered_text(content: &str, starting_line: usize) -> String {
    content
        .lines()
        .enumerate()
        .map(|(index, line)| format!("{:03} | {}", starting_line + index, line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn exec_result_output(result: ExecResult) -> ToolExecutionOutput {
    let is_error = result.exit_code != 0 || result.timed_out;
    let value = json!({
        "stdout": result.stdout,
        "stderr": result.stderr,
        "exit_code": result.exit_code,
        "timed_out": result.timed_out,
        "duration_ms": result.duration_ms
    });
    if is_error {
        ToolExecutionOutput::error(value)
    } else {
        ToolExecutionOutput::success(value)
    }
}

fn parse_grep_matches(output: &str) -> Vec<Value> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, ':');
            let path = parts.next()?;
            let line_number = parts.next()?.parse::<u64>().ok()?;
            let line_text = parts.next().unwrap_or_default().trim_end_matches('\r');
            Some(json!({
                "path": path,
                "line_number": line_number,
                "line": line_text
            }))
        })
        .collect()
}

fn supports_image_results(
    execution: &ToolExecution,
    default_capabilities: &BTreeMap<String, bool>,
) -> bool {
    capability_enabled(&execution.capabilities, default_capabilities, "vision")
        || capability_enabled(&execution.capabilities, default_capabilities, "multimodal")
        || capability_enabled(&execution.capabilities, default_capabilities, "image")
}

fn capability_enabled(
    capabilities: &BTreeMap<String, bool>,
    default_capabilities: &BTreeMap<String, bool>,
    name: &str,
) -> bool {
    let normalized_name = normalize_capability_name(name);
    if let Some(enabled) = lookup_capability(capabilities, &normalized_name) {
        return enabled;
    }
    lookup_capability(default_capabilities, &normalized_name).unwrap_or(false)
}

fn lookup_capability(capabilities: &BTreeMap<String, bool>, normalized_name: &str) -> Option<bool> {
    capabilities
        .iter()
        .find(|(key, _)| normalize_capability_name(key) == normalized_name)
        .map(|(_, enabled)| *enabled)
}

fn normalize_capability_name(value: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase().replace('-', "_");
    normalized
        .strip_prefix("supports_")
        .unwrap_or(normalized.as_str())
        .to_string()
}

fn guess_image_media_type(path: &str, payload: &[u8]) -> Option<&'static str> {
    if payload.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if payload.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }
    if payload.starts_with(b"GIF87a") || payload.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if payload.len() >= 12 && payload.starts_with(b"RIFF") && &payload[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if payload.starts_with(b"BM") {
        return Some("image/bmp");
    }

    let lower_path = path.to_ascii_lowercase();
    match lower_path.rsplit('.').next() {
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        Some("bmp") => Some("image/bmp"),
        _ => None,
    }
}

fn anthropic_grep_payload(matches: &[Value], output_mode: &str) -> Value {
    match output_mode {
        "content" => json!({"matches": matches}),
        "count" => {
            let mut order = Vec::<String>::new();
            let mut counts = BTreeMap::<String, u64>::new();
            for item in matches {
                let Some(path) = item.get("path").and_then(|value| value.as_str()) else {
                    continue;
                };
                if !counts.contains_key(path) {
                    order.push(path.to_string());
                }
                *counts.entry(path.to_string()).or_insert(0) += 1;
            }
            json!({
                "files": order
                    .into_iter()
                    .map(|path| json!({"path": path, "count": counts[&path]}))
                    .collect::<Vec<_>>()
            })
        }
        _ => {
            let mut seen = BTreeSet::<String>::new();
            let mut paths = Vec::<String>::new();
            for item in matches {
                let Some(path) = item.get("path").and_then(|value| value.as_str()) else {
                    continue;
                };
                if seen.insert(path.to_string()) {
                    paths.push(path.to_string());
                }
            }
            json!(paths)
        }
    }
}

fn file_error_message(path: &str, error: EnvironmentError) -> String {
    match error {
        EnvironmentError::FileNotFound(_) => format!("File not found: {path}"),
        EnvironmentError::PermissionDenied(_) => format!("Permission denied: {path}"),
        EnvironmentError::IsDirectory(_) | EnvironmentError::NotDirectory(_) => {
            format!("Is a directory: {path}")
        }
        EnvironmentError::InvalidUtf8(_) => format!("Binary file not supported: {path}"),
        other => format!("Failed to read file: {path}: {other}"),
    }
}

fn write_error_message(path: &str, error: EnvironmentError) -> String {
    match error {
        EnvironmentError::FileNotFound(_) => format!("File not found: {path}"),
        EnvironmentError::PermissionDenied(_) => format!("Permission denied: {path}"),
        EnvironmentError::IsDirectory(_) | EnvironmentError::NotDirectory(_) => {
            format!("Is a directory: {path}")
        }
        other => format!("Failed to write file: {path}: {other}"),
    }
}

fn search_error_message(path: &str, error: EnvironmentError) -> String {
    match error {
        EnvironmentError::FileNotFound(_) => format!("File not found: {path}"),
        EnvironmentError::PermissionDenied(_) => format!("Permission denied: {path}"),
        EnvironmentError::IsDirectory(_) | EnvironmentError::NotDirectory(_) => {
            format!("Is a directory: {path}")
        }
        other => format!("Search failed: {path}: {other}"),
    }
}

fn shell_error_message(command: &str, error: EnvironmentError) -> String {
    match error {
        EnvironmentError::FileNotFound(_) => format!("File not found: {command}"),
        EnvironmentError::PermissionDenied(_) => format!("Permission denied: {command}"),
        EnvironmentError::IsDirectory(_) | EnvironmentError::NotDirectory(_) => {
            format!("Is a directory: {command}")
        }
        other => format!("Failed to execute command: {other}"),
    }
}

fn read_file_tool(path_field: &str, description: &str) -> Tool {
    object_tool(
        "read_file",
        description,
        json!({
            path_field: {"type": "string", "minLength": 1},
            "offset": {"type": "integer", "minimum": 1, "default": 1},
            "limit": {"type": "integer", "minimum": 0, "default": DEFAULT_READ_FILE_LIMIT}
        }),
        &[path_field],
    )
}

fn write_file_tool(path_field: &str) -> Tool {
    object_tool(
        "write_file",
        "Write a file and report how many bytes were written.",
        json!({
            path_field: {"type": "string", "minLength": 1},
            "content": {"type": "string"}
        }),
        &[path_field, "content"],
    )
}

fn anthropic_edit_file_tool() -> Tool {
    object_tool(
        "edit_file",
        "Edit a file by replacing exact text.",
        json!({
            "file_path": {"type": "string", "minLength": 1},
            "old_string": {"type": "string", "minLength": 1},
            "new_string": {"type": "string"},
            "replace_all": {"type": "boolean", "default": false}
        }),
        &["file_path", "old_string", "new_string"],
    )
}

fn gemini_edit_file_tool() -> Tool {
    object_tool(
        "edit_file",
        "Edit a file with search-and-replace semantics.",
        json!({
            "file_path": {"type": "string", "minLength": 1},
            "instruction": {"type": "string", "minLength": 1},
            "old_string": {"type": "string", "minLength": 1},
            "new_string": {"type": "string"},
            "allow_multiple": {"type": "boolean", "default": false}
        }),
        &["file_path", "instruction", "old_string", "new_string"],
    )
}

fn apply_patch_tool() -> Tool {
    object_tool(
        "apply_patch",
        "Apply code changes using the v4a patch format.",
        json!({
            "patch": {"type": "string", "minLength": 1}
        }),
        &["patch"],
    )
}

fn shell_tool(default_timeout_ms: u64) -> Tool {
    object_tool(
        "shell",
        "Run a shell command and return stdout, stderr, and exit metadata.",
        json!({
            "command": {"type": "string", "minLength": 1},
            "description": {"type": "string"},
            "timeout_ms": {
                "type": "integer",
                "minimum": 1,
                "default": default_timeout_ms
            }
        }),
        &["command"],
    )
}

fn grep_tool() -> Tool {
    object_tool(
        "grep",
        "Search files with a regex and return structured matches.",
        json!({
            "pattern": {"type": "string", "minLength": 1},
            "path": {"type": "string", "minLength": 1},
            "glob_filter": {"type": "string", "minLength": 1},
            "case_insensitive": {"type": "boolean", "default": false},
            "max_results": {"type": "integer", "minimum": 1, "default": 100}
        }),
        &["pattern"],
    )
}

fn anthropic_grep_tool() -> Tool {
    object_tool(
        "grep",
        "Search files with a regex and return matching content, file paths, or per-file counts.",
        json!({
            "pattern": {"type": "string", "minLength": 1},
            "path": {"type": "string", "minLength": 1},
            "glob": {"type": "string", "minLength": 1},
            "type": {"type": "string", "minLength": 1},
            "output_mode": {
                "type": "string",
                "enum": ["content", "files_with_matches", "count"],
                "default": "files_with_matches"
            },
            "-i": {"type": "boolean", "default": false},
            "-n": {"type": "boolean", "default": true},
            "multiline": {"type": "boolean", "default": false},
            "head_limit": {
                "type": "integer",
                "minimum": 0,
                "default": ANTHROPIC_GREP_HEAD_LIMIT
            },
            "offset": {"type": "integer", "minimum": 0, "default": 0}
        }),
        &["pattern"],
    )
}

fn glob_tool() -> Tool {
    object_tool(
        "glob",
        "Find files matching a glob pattern.",
        json!({
            "pattern": {"type": "string", "minLength": 1},
            "path": {"type": "string", "minLength": 1}
        }),
        &["pattern"],
    )
}

fn read_many_files_tool() -> Tool {
    object_tool(
        "read_many_files",
        "Read several files and return numbered text for each one.",
        json!({
            "paths": {
                "type": "array",
                "items": {"type": "string", "minLength": 1},
                "minItems": 1
            }
        }),
        &["paths"],
    )
}

fn list_dir_tool() -> Tool {
    object_tool(
        "list_dir",
        "List a directory and return structured entries.",
        json!({
            "path": {"type": "string", "minLength": 1},
            "depth": {"type": "integer", "minimum": 0, "default": 0}
        }),
        &["path"],
    )
}

fn web_search_tool() -> Tool {
    object_tool(
        "web_search",
        "Search the web for up-to-date information.",
        json!({
            "query": {"type": "string", "minLength": 1},
            "max_results": {
                "type": "integer",
                "minimum": 1,
                "default": GEMINI_WEB_SEARCH_RESULTS
            },
            "timeout_ms": {
                "type": "integer",
                "minimum": 1,
                "default": GEMINI_WEB_FETCH_TIMEOUT_MS
            }
        }),
        &["query"],
    )
}

fn web_fetch_tool() -> Tool {
    object_tool(
        "web_fetch",
        "Fetch and extract content from a URL.",
        json!({
            "url": {"type": "string", "minLength": 1},
            "max_bytes": {
                "type": "integer",
                "minimum": 1,
                "default": GEMINI_WEB_FETCH_BYTES
            },
            "timeout_ms": {
                "type": "integer",
                "minimum": 1,
                "default": GEMINI_WEB_FETCH_TIMEOUT_MS
            }
        }),
        &["url"],
    )
}

fn object_tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Tool {
    Tool::passive_with_schema(
        name,
        Some(description.to_string()),
        Some(json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        })),
    )
    .expect("built-in tool definitions must be valid")
}
