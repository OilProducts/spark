use std::collections::BTreeMap;

use crate::config::SessionConfig;

pub const DEFAULT_TOOL_OUTPUT_LIMITS: &[(&str, u64)] = &[
    ("apply_patch", 10_000),
    ("edit_file", 10_000),
    ("glob", 20_000),
    ("grep", 20_000),
    ("read_file", 50_000),
    ("shell", 30_000),
    ("spawn_agent", 20_000),
    ("write_file", 1_000),
];

pub const DEFAULT_TRUNCATION_MODES: &[(&str, &str)] = &[
    ("apply_patch", "tail"),
    ("edit_file", "tail"),
    ("glob", "tail"),
    ("grep", "tail"),
    ("read_file", "head_tail"),
    ("shell", "head_tail"),
    ("spawn_agent", "head_tail"),
    ("write_file", "tail"),
];

pub const DEFAULT_TOOL_LINE_LIMITS: &[(&str, u64)] =
    &[("glob", 500), ("grep", 200), ("shell", 256)];

const HEAD_TAIL_WARNING: &str = "[WARNING: Tool output was truncated. {removed} characters were removed from the middle. The full output is available in the event stream. If you need to see specific parts, re-run the tool with more targeted parameters.]";
const TAIL_WARNING: &str = "[WARNING: Tool output was truncated. First {removed} characters were removed. The full output is available in the event stream.]";

pub fn truncate_tool_output(output: &str, tool_name: &str, config: &SessionConfig) -> String {
    let char_limits = default_output_limits();
    let line_limits = default_line_limits();
    let modes = default_truncation_modes();

    let mut output = if let Some(max_chars) = config
        .tool_output_limits
        .get(tool_name)
        .copied()
        .or_else(|| char_limits.get(tool_name).copied())
    {
        let mode = modes.get(tool_name).copied().unwrap_or("tail");
        truncate_output(output, max_chars, mode)
    } else {
        output.to_string()
    };

    if let Some(max_lines) = config
        .line_limits
        .get(tool_name)
        .copied()
        .or_else(|| line_limits.get(tool_name).copied())
    {
        output = truncate_lines(&output, max_lines);
    }

    output
}

pub fn truncate_output(output: &str, max_chars: u64, mode: &str) -> String {
    assert!(max_chars >= 1, "max_chars must be at least 1");
    let chars = output.chars().collect::<Vec<_>>();
    let max_chars = usize::try_from(max_chars).unwrap_or(usize::MAX);
    if chars.len() <= max_chars {
        return output.to_string();
    }

    let removed = chars.len() - max_chars;
    match mode {
        "head_tail" => {
            let head_chars = max_chars / 2;
            let tail_chars = max_chars - head_chars;
            let head = chars[..head_chars].iter().collect::<String>();
            let tail = chars[chars.len() - tail_chars..].iter().collect::<String>();
            format!(
                "{head}\n\n{}\n\n{tail}",
                HEAD_TAIL_WARNING.replace("{removed}", &removed.to_string())
            )
        }
        "tail" => {
            let tail = chars[chars.len() - max_chars..].iter().collect::<String>();
            format!(
                "{}\n\n{tail}",
                TAIL_WARNING.replace("{removed}", &removed.to_string())
            )
        }
        other => panic!("unsupported truncation mode: {other}"),
    }
}

pub fn truncate_lines(output: &str, max_lines: u64) -> String {
    assert!(max_lines >= 1, "max_lines must be at least 1");
    let lines = output.split('\n').collect::<Vec<_>>();
    let max_lines = usize::try_from(max_lines).unwrap_or(usize::MAX);
    if lines.len() <= max_lines {
        return output.to_string();
    }

    let head_count = max_lines / 2;
    let tail_count = max_lines - head_count;
    let omitted = lines.len() - head_count - tail_count;
    let mut output = Vec::with_capacity(max_lines + 1);
    output.extend(lines[..head_count].iter().copied().map(str::to_string));
    output.push(format!("[... {omitted} lines omitted ...]"));
    output.extend(
        lines[lines.len() - tail_count..]
            .iter()
            .copied()
            .map(str::to_string),
    );
    output.join("\n")
}

fn default_output_limits() -> BTreeMap<&'static str, u64> {
    DEFAULT_TOOL_OUTPUT_LIMITS.iter().copied().collect()
}

fn default_truncation_modes() -> BTreeMap<&'static str, &'static str> {
    DEFAULT_TRUNCATION_MODES.iter().copied().collect()
}

fn default_line_limits() -> BTreeMap<&'static str, u64> {
    DEFAULT_TOOL_LINE_LIMITS.iter().copied().collect()
}
