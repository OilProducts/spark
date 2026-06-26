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

    pub(crate) fn stderr(exit_code: i32, stderr: impl Into<String>) -> Self {
        Self {
            exit_code,
            stdout: String::new(),
            stderr: stderr.into(),
        }
    }
}

pub(crate) fn success_json(payload: &Value) -> CommandOutput {
    let mut stdout = serde_json::to_string_pretty(payload).expect("serializing JSON cannot fail");
    stdout.push('\n');
    CommandOutput::stdout(0, stdout)
}

pub(crate) fn json_error(message: impl AsRef<str>, exit_code: i32) -> CommandOutput {
    let encoded_message =
        serde_json::to_string(message.as_ref()).expect("serializing a string cannot fail");
    CommandOutput::stderr(
        exit_code,
        format!("{{\"ok\": false, \"error\": {encoded_message}}}\n"),
    )
}

pub(crate) fn http_error(status_code: u16, message: impl AsRef<str>) -> CommandOutput {
    let encoded_message =
        serde_json::to_string(message.as_ref()).expect("serializing a string cannot fail");
    CommandOutput::stderr(
        crate::http_status_exit_code(status_code),
        format!(
            "{{\"ok\": false, \"status_code\": {status_code}, \"error\": {encoded_message}}}\n"
        ),
    )
}

pub(crate) fn usage_error(message: impl AsRef<str>) -> CommandOutput {
    CommandOutput::stderr(
        crate::EXIT_USAGE_ERROR,
        format!(
            "usage: spark [-h] {{convo,run,flow,trigger}} ...\n\
spark: error: {}\n",
            message.as_ref()
        ),
    )
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
