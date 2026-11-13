use std::fmt::Write as _;

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    pub(crate) fn stdout(exit_code: i32, stdout: impl Into<String>) -> Self {
        Self {
            exit_code,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }
}

pub(crate) fn success_json(payload: &Value) -> CommandOutput {
    let mut stdout = serde_json::to_string_pretty(payload).expect("serializing JSON cannot fail");
    stdout.push('\n');
    CommandOutput::stdout(0, stdout)
}

/// The one JSON object every failure writes to stderr. Transport and HTTP
/// failures name the server they called and where that target came from.
pub(crate) fn failure(
    message: impl AsRef<str>,
    exit_code: i32,
    status_code: Option<u16>,
    target: Option<(&str, &str)>,
) -> CommandOutput {
    let string =
        |value: &str| serde_json::to_string(value).expect("serializing a string cannot fail");
    let mut stderr = format!(
        "{{\"ok\": false, \"error\": {}, \"exit_code\": {exit_code}",
        string(message.as_ref())
    );
    if let Some(status_code) = status_code {
        let _ = write!(stderr, ", \"status_code\": {status_code}");
    }
    if let Some((target, source)) = target {
        let _ = write!(
            stderr,
            ", \"target\": {}, \"target_source\": {}",
            string(target),
            string(source)
        );
    }
    stderr.push_str("}\n");
    CommandOutput {
        exit_code,
        stdout: String::new(),
        stderr,
    }
}

pub(crate) fn json_error(message: impl AsRef<str>, exit_code: i32) -> CommandOutput {
    failure(message, exit_code, None, None)
}

pub(crate) fn usage_error(message: impl AsRef<str>) -> CommandOutput {
    failure(message, crate::EXIT_USAGE_ERROR, None, None)
}

pub(crate) fn write_process_output(output: &CommandOutput) {
    use std::io::Write;

    if !output.stdout.is_empty() {
        let _ = std::io::stdout().write_all(output.stdout.as_bytes());
    }
    if !output.stderr.is_empty() {
        let _ = std::io::stderr().write_all(output.stderr.as_bytes());
    }
}
