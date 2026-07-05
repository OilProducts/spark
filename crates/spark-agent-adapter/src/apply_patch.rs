use std::path::Path;

use serde_json::{json, Value};

use crate::environment::{EnvironmentError, ExecutionEnvironment};

const BEGIN_MARKER: &str = "*** Begin Patch";
const END_MARKER: &str = "*** End Patch";
const END_OF_FILE_MARKER: &str = "*** End of File";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyPatchError {
    Parse(String),
    Apply(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PatchHunk {
    context_hint: Option<String>,
    lines: Vec<PatchLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PatchLine {
    kind: PatchLineKind,
    text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PatchLineKind {
    Context,
    Delete,
    Add,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PatchOperation {
    Add {
        path: String,
        lines: Vec<String>,
    },
    Delete {
        path: String,
    },
    Update {
        path: String,
        move_to: Option<String>,
        hunks: Vec<PatchHunk>,
    },
}

pub fn apply_patch(
    environment: &ExecutionEnvironment,
    patch_text: &str,
) -> Result<Vec<Value>, ApplyPatchError> {
    let operations = parse_patch_text(patch_text).map_err(ApplyPatchError::Parse)?;
    apply_operations(environment, &operations).map_err(ApplyPatchError::Apply)
}

fn parse_patch_text(patch_text: &str) -> Result<Vec<PatchOperation>, String> {
    let lines = patch_text.lines().collect::<Vec<_>>();
    if lines.first().copied() != Some(BEGIN_MARKER) {
        return Err("missing *** Begin Patch marker".to_string());
    }

    let mut operations = Vec::new();
    let mut index = 1;
    let mut saw_end_marker = false;
    while index < lines.len() {
        let line = lines[index];
        if line == END_MARKER {
            saw_end_marker = true;
            index += 1;
            break;
        }
        if line.starts_with("*** Add File:") {
            let (operation, next_index) = parse_add_operation(&lines, index)?;
            operations.push(operation);
            index = next_index;
            continue;
        }
        if line.starts_with("*** Delete File:") {
            let (operation, next_index) = parse_delete_operation(&lines, index)?;
            operations.push(operation);
            index = next_index;
            continue;
        }
        if line.starts_with("*** Update File:") {
            let (operation, next_index) = parse_update_operation(&lines, index)?;
            operations.push(operation);
            index = next_index;
            continue;
        }
        if line.trim().is_empty() {
            return Err("unexpected blank line in patch".to_string());
        }
        return Err(format!("unexpected patch line: {line}"));
    }

    if !saw_end_marker {
        return Err("missing *** End Patch marker".to_string());
    }
    if operations.is_empty() {
        return Err("patch must contain at least one operation".to_string());
    }
    if lines[index..].iter().any(|line| !line.trim().is_empty()) {
        return Err("unexpected content after *** End Patch marker".to_string());
    }
    Ok(operations)
}

fn parse_add_operation(lines: &[&str], index: usize) -> Result<(PatchOperation, usize), String> {
    let path = header_path(lines[index], "*** Add File:")?;
    let mut index = index + 1;
    let mut added_lines = Vec::new();
    while index < lines.len() {
        let line = lines[index];
        if line.starts_with("***") {
            break;
        }
        if !line.starts_with('+') {
            return Err("add file lines must start with +".to_string());
        }
        added_lines.push(line[1..].to_string());
        index += 1;
    }
    Ok((
        PatchOperation::Add {
            path,
            lines: added_lines,
        },
        index,
    ))
}

fn parse_delete_operation(lines: &[&str], index: usize) -> Result<(PatchOperation, usize), String> {
    Ok((
        PatchOperation::Delete {
            path: header_path(lines[index], "*** Delete File:")?,
        },
        index + 1,
    ))
}

fn parse_update_operation(lines: &[&str], index: usize) -> Result<(PatchOperation, usize), String> {
    let path = header_path(lines[index], "*** Update File:")?;
    let mut index = index + 1;
    let mut move_to = None;
    if index < lines.len() && lines[index].starts_with("*** Move to:") {
        move_to = Some(header_path(lines[index], "*** Move to:")?);
        index += 1;
    }

    let mut hunks = Vec::new();
    while index < lines.len() {
        let line = lines[index];
        if line == END_OF_FILE_MARKER {
            index += 1;
            break;
        }
        if line.starts_with("***") {
            break;
        }
        if !line.starts_with("@@") {
            return Err("update file hunks must begin with @@".to_string());
        }
        let (hunk, next_index) = parse_hunk(lines, index)?;
        hunks.push(hunk);
        index = next_index;
    }

    if hunks.is_empty() && move_to.is_none() {
        return Err("update file must contain at least one hunk".to_string());
    }
    Ok((
        PatchOperation::Update {
            path,
            move_to,
            hunks,
        },
        index,
    ))
}

fn parse_hunk(lines: &[&str], index: usize) -> Result<(PatchHunk, usize), String> {
    let header = lines[index];
    if !header.starts_with("@@") {
        return Err("expected hunk header starting with @@".to_string());
    }

    let mut context_hint = &header[2..];
    if let Some(stripped) = context_hint.strip_prefix(' ') {
        context_hint = stripped;
    }
    let context_hint = if context_hint.trim().is_empty() {
        None
    } else {
        Some(context_hint.to_string())
    };

    let mut index = index + 1;
    let mut hunk_lines = Vec::new();
    while index < lines.len() {
        let line = lines[index];
        if line.starts_with("@@") || line.starts_with("***") {
            break;
        }
        if line.is_empty() {
            return Err("invalid empty hunk line".to_string());
        }
        let (prefix, text) = line.split_at(1);
        let kind = match prefix {
            " " => PatchLineKind::Context,
            "-" => PatchLineKind::Delete,
            "+" => PatchLineKind::Add,
            _ => return Err(format!("invalid hunk line: {line}")),
        };
        hunk_lines.push(PatchLine {
            kind,
            text: text.to_string(),
        });
        index += 1;
    }

    if hunk_lines.is_empty() {
        return Err("hunk must contain at least one line".to_string());
    }
    Ok((
        PatchHunk {
            context_hint,
            lines: hunk_lines,
        },
        index,
    ))
}

fn header_path(line: &str, prefix: &str) -> Result<String, String> {
    let Some(mut path) = line.strip_prefix(prefix) else {
        return Err(format!("expected {prefix:?}"));
    };
    if let Some(stripped) = path.strip_prefix(' ') {
        path = stripped;
    }
    if path.trim().is_empty() {
        return Err(format!("missing path after {prefix:?}"));
    }
    Ok(path.to_string())
}

fn apply_operations(
    environment: &ExecutionEnvironment,
    operations: &[PatchOperation],
) -> Result<Vec<Value>, String> {
    let mut results = Vec::new();
    for operation in operations {
        let result = match operation {
            PatchOperation::Add { path, lines } => add_file(environment, path, lines)?,
            PatchOperation::Delete { path } => delete_file(environment, path)?,
            PatchOperation::Update {
                path,
                move_to,
                hunks,
            } => update_file(environment, path, move_to.as_deref(), hunks)?,
        };
        results.push(result);
    }
    Ok(results)
}

fn add_file(
    environment: &ExecutionEnvironment,
    path: &str,
    lines: &[String],
) -> Result<Value, String> {
    if environment.file_exists(path) {
        return Err(format!("File already exists: {path}"));
    }

    let content = join_text_content(lines, "\n", !lines.is_empty());
    environment
        .write_file(path, &content)
        .map_err(|error| write_error_message(path, error))?;

    let written = read_text_content(environment, path)?;
    if written != content {
        return Err(format!("Patch verification failed: {path}"));
    }

    Ok(json!({"operation": "add", "path": path}))
}

fn delete_file(environment: &ExecutionEnvironment, path: &str) -> Result<Value, String> {
    if !environment.file_exists(path) {
        return Err(format!("File not found: {path}"));
    }
    if environment.is_directory(path) {
        return Err(format!("Is a directory: {path}"));
    }

    environment
        .delete_file(path)
        .map_err(|error| delete_error_message(path, error))?;
    if environment.file_exists(path) {
        return Err(format!("Patch verification failed: {path}"));
    }

    Ok(json!({"operation": "delete", "path": path}))
}

fn update_file(
    environment: &ExecutionEnvironment,
    path: &str,
    move_to: Option<&str>,
    hunks: &[PatchHunk],
) -> Result<Value, String> {
    if hunks.is_empty() {
        let Some(destination_path) = move_to.filter(|destination| *destination != path) else {
            return Err("update file must contain at least one hunk".to_string());
        };
        return rename_only(environment, path, destination_path);
    }

    let current_content = read_text_content(environment, path)?;
    let (current_lines, separator, trailing_newline) = split_text_content(&current_content);
    let mut updated_lines = current_lines;

    for hunk in hunks {
        updated_lines = apply_hunk(updated_lines, hunk, path)?;
    }

    let updated_content = join_text_content(&updated_lines, separator, trailing_newline);
    if let Some(destination_path) = move_to.filter(|destination| *destination != path) {
        validate_rename_preconditions(environment, path, destination_path)?;
    }

    environment
        .write_file(path, &updated_content)
        .map_err(|error| write_error_message(path, error))?;

    let written = read_text_content(environment, path)?;
    if written != updated_content {
        return Err(format!("Patch verification failed: {path}"));
    }

    if let Some(destination_path) = move_to.filter(|destination| *destination != path) {
        rename_file_after_preflight(environment, path, destination_path)?;
        let renamed_content = read_text_content(environment, destination_path)?;
        if renamed_content != updated_content
            || environment.file_exists(path)
            || !environment.file_exists(destination_path)
        {
            return Err(format!(
                "Patch verification failed: {path} -> {destination_path}"
            ));
        }
        return Ok(json!({
            "operation": "update+rename",
            "path": path,
            "new_path": destination_path,
            "hunks": hunks.len()
        }));
    }

    Ok(json!({"operation": "update", "path": path, "hunks": hunks.len()}))
}

fn rename_only(
    environment: &ExecutionEnvironment,
    source_path: &str,
    destination_path: &str,
) -> Result<Value, String> {
    validate_rename_preconditions(environment, source_path, destination_path)?;
    let original_content = read_bytes_content(environment, source_path)?;
    rename_file_after_preflight(environment, source_path, destination_path)?;

    let renamed_content = read_bytes_content(environment, destination_path)?;
    if renamed_content != original_content
        || environment.file_exists(source_path)
        || !environment.file_exists(destination_path)
    {
        return Err(format!(
            "Patch verification failed: {source_path} -> {destination_path}"
        ));
    }

    Ok(json!({
        "operation": "rename",
        "path": source_path,
        "new_path": destination_path
    }))
}

fn validate_rename_preconditions(
    environment: &ExecutionEnvironment,
    source_path: &str,
    destination_path: &str,
) -> Result<(), String> {
    if !environment.file_exists(source_path) {
        return Err(format!("File not found: {source_path}"));
    }
    if environment.is_directory(source_path) {
        return Err(format!("Is a directory: {source_path}"));
    }
    if environment.file_exists(destination_path) {
        return Err(format!("File already exists: {destination_path}"));
    }
    Ok(())
}

fn rename_file_after_preflight(
    environment: &ExecutionEnvironment,
    source_path: &str,
    destination_path: &str,
) -> Result<(), String> {
    environment
        .rename_file(source_path, destination_path)
        .map_err(|error| rename_error_message(source_path, destination_path, error))
}

fn apply_hunk(
    current_lines: Vec<String>,
    hunk: &PatchHunk,
    path: &str,
) -> Result<Vec<String>, String> {
    let before_lines = hunk
        .lines
        .iter()
        .filter(|line| line.kind != PatchLineKind::Add)
        .map(|line| line.text.clone())
        .collect::<Vec<_>>();
    let after_lines = hunk
        .lines
        .iter()
        .filter(|line| line.kind != PatchLineKind::Delete)
        .map(|line| line.text.clone())
        .collect::<Vec<_>>();
    if before_lines.is_empty() {
        return Err(format!("Patch apply error: empty hunk in {path}"));
    }

    let start = match_start(
        &current_lines,
        &before_lines,
        hunk.context_hint.as_deref(),
        path,
    )?;
    let end = start + before_lines.len();
    let mut updated =
        Vec::with_capacity(current_lines.len() - before_lines.len() + after_lines.len());
    updated.extend_from_slice(&current_lines[..start]);
    updated.extend(after_lines);
    updated.extend_from_slice(&current_lines[end..]);
    Ok(updated)
}

fn match_start(
    haystack: &[String],
    needle: &[String],
    hint: Option<&str>,
    path: &str,
) -> Result<usize, String> {
    let exact_candidates = find_sequence_matches(haystack, needle, false);
    if exact_candidates.len() == 1 {
        return Ok(exact_candidates[0]);
    }
    if exact_candidates.len() > 1 {
        return choose_candidate_with_hint(haystack, &exact_candidates, needle.len(), hint)
            .ok_or_else(|| format!("Patch apply error: ambiguous hunk match in {path}"));
    }

    let fuzzy_candidates = find_sequence_matches(haystack, needle, true);
    if fuzzy_candidates.len() == 1 {
        return Ok(fuzzy_candidates[0]);
    }
    if fuzzy_candidates.len() > 1 {
        return choose_candidate_with_hint(haystack, &fuzzy_candidates, needle.len(), hint)
            .ok_or_else(|| format!("Patch apply error: ambiguous hunk match in {path}"));
    }

    Err(format!(
        "Patch apply error: unable to locate hunk in {path}"
    ))
}

fn find_sequence_matches(haystack: &[String], needle: &[String], fuzzy: bool) -> Vec<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return Vec::new();
    }

    let mut matches = Vec::new();
    for start in 0..=haystack.len() - needle.len() {
        if needle
            .iter()
            .enumerate()
            .all(|(offset, expected)| line_matches(&haystack[start + offset], expected, fuzzy))
        {
            matches.push(start);
        }
    }
    matches
}

fn line_matches(candidate: &str, expected: &str, fuzzy: bool) -> bool {
    if fuzzy {
        normalize_fuzzy_text(candidate) == normalize_fuzzy_text(expected)
    } else {
        candidate == expected
    }
}

fn choose_candidate_with_hint(
    haystack: &[String],
    candidates: &[usize],
    match_length: usize,
    hint: Option<&str>,
) -> Option<usize> {
    let hint = hint?;
    let positions = hint_positions(haystack, hint);
    if positions.is_empty() {
        return None;
    }

    let mut best_start = None;
    let mut best_score = None;
    for &start in candidates {
        let end = start + match_length;
        let score = if positions
            .iter()
            .any(|position| start <= *position && *position < end)
        {
            (0usize, 0usize, start)
        } else if let Some(nearest_before) = positions
            .iter()
            .copied()
            .filter(|position| *position < start)
            .max()
        {
            (1usize, start - nearest_before, start)
        } else if let Some(nearest_after) = positions
            .iter()
            .copied()
            .filter(|position| *position >= end)
            .min()
        {
            (2usize, nearest_after - end, start)
        } else {
            (3usize, start, start)
        };

        if best_score.map_or(true, |current| score < current) {
            best_score = Some(score);
            best_start = Some(start);
        } else if Some(score) == best_score {
            best_start = None;
        }
    }
    best_start
}

fn hint_positions(haystack: &[String], hint: &str) -> Vec<usize> {
    let exact_matches = haystack
        .iter()
        .enumerate()
        .filter_map(|(index, line)| if line == hint { Some(index) } else { None })
        .collect::<Vec<_>>();
    if !exact_matches.is_empty() {
        return exact_matches;
    }

    let fuzzy_hint = normalize_fuzzy_text(hint);
    haystack
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            if normalize_fuzzy_text(line) == fuzzy_hint {
                Some(index)
            } else {
                None
            }
        })
        .collect()
}

fn read_text_content(environment: &ExecutionEnvironment, path: &str) -> Result<String, String> {
    let value = environment
        .read_file(path, None, None)
        .map_err(|error| read_error_message(path, error))?;
    if is_binary_text(&value) {
        return Err(format!("Binary file not supported: {path}"));
    }
    Ok(value)
}

fn read_bytes_content(environment: &ExecutionEnvironment, path: &str) -> Result<Vec<u8>, String> {
    environment
        .read_file_bytes(path)
        .map_err(|error| read_error_message(path, error))
}

fn split_text_content(content: &str) -> (Vec<String>, &str, bool) {
    let separator = if content.contains("\r\n") {
        "\r\n"
    } else if content.contains('\n') {
        "\n"
    } else if content.contains('\r') {
        "\r"
    } else {
        "\n"
    };
    let trailing_newline =
        !content.is_empty() && (content.ends_with('\n') || content.ends_with('\r'));
    if content.is_empty() {
        return (Vec::new(), separator, trailing_newline);
    }

    let mut lines = content
        .split(separator)
        .map(str::to_string)
        .collect::<Vec<_>>();
    if trailing_newline && lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    (lines, separator, trailing_newline)
}

fn join_text_content(lines: &[String], separator: &str, trailing_newline: bool) -> String {
    let mut text = lines.join(separator);
    if trailing_newline && !lines.is_empty() {
        text.push_str(separator);
    }
    text
}

fn normalize_fuzzy_text(value: &str) -> String {
    let mut normalized = String::new();
    for character in value.chars() {
        if let Some(replacement) = normalize_fuzzy_char(character) {
            normalized.push_str(replacement);
        } else {
            normalized.push(character);
        }
    }
    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_fuzzy_char(character: char) -> Option<&'static str> {
    match character {
        '\u{00a0}' | '\u{2007}' | '\u{2009}' | '\u{202f}' | '\u{3000}' => Some(" "),
        '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
        | '\u{2212}' => Some("-"),
        '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}' => Some("'"),
        '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{201f}' => Some("\""),
        '\u{2026}' => Some("..."),
        _ => None,
    }
}

fn is_binary_text(value: &str) -> bool {
    value.chars().any(|character| {
        let codepoint = character as u32;
        codepoint == 0
            || (codepoint < 32 && !matches!(character, '\t' | '\n' | '\r'))
            || (0x7f..=0x9f).contains(&codepoint)
    })
}

fn read_error_message(path: &str, error: EnvironmentError) -> String {
    match error {
        EnvironmentError::FileNotFound(_) => format!("File not found: {path}"),
        EnvironmentError::PermissionDenied(_) => format!("Permission denied: {path}"),
        EnvironmentError::IsDirectory(_) | EnvironmentError::NotDirectory(_) => {
            format!("Is a directory: {path}")
        }
        EnvironmentError::InvalidUtf8(_) => format!("Binary file not supported: {path}"),
        other => format!("Patch apply error: failed to read {path}: {other}"),
    }
}

fn write_error_message(path: &str, error: EnvironmentError) -> String {
    match error {
        EnvironmentError::FileNotFound(_) => format!("File not found: {path}"),
        EnvironmentError::PermissionDenied(_) => format!("Permission denied: {path}"),
        EnvironmentError::IsDirectory(_) | EnvironmentError::NotDirectory(_) => {
            format!("Is a directory: {path}")
        }
        other => format!("Patch apply error: failed to write {path}: {other}"),
    }
}

fn delete_error_message(path: &str, error: EnvironmentError) -> String {
    match error {
        EnvironmentError::FileNotFound(_) => format!("File not found: {path}"),
        EnvironmentError::PermissionDenied(_) => format!("Permission denied: {path}"),
        EnvironmentError::IsDirectory(_) | EnvironmentError::NotDirectory(_) => {
            format!("Is a directory: {path}")
        }
        EnvironmentError::AlreadyExists(_) => format!("File already exists: {path}"),
        other => format!("Patch apply error: failed to delete {path}: {other}"),
    }
}

fn rename_error_message(
    source_path: &str,
    destination_path: &str,
    error: EnvironmentError,
) -> String {
    match error {
        EnvironmentError::FileNotFound(path) => {
            if path_matches(&path, destination_path) {
                format!("File not found: {destination_path}")
            } else {
                format!("File not found: {source_path}")
            }
        }
        EnvironmentError::PermissionDenied(_) => format!("Permission denied: {source_path}"),
        EnvironmentError::IsDirectory(_) | EnvironmentError::NotDirectory(_) => {
            format!("Is a directory: {source_path}")
        }
        EnvironmentError::AlreadyExists(_) => format!("File already exists: {destination_path}"),
        other => format!(
            "Patch apply error: failed to rename {source_path} to {destination_path}: {other}"
        ),
    }
}

fn path_matches(path: &Path, display_path: &str) -> bool {
    path == Path::new(display_path) || path.to_string_lossy().replace('\\', "/") == display_path
}
