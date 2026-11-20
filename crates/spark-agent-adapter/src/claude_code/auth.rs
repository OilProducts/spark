//! Claude Code sign-in through the CLI's own `claude auth` commands. Spark
//! relays the sign-in link and a pasted code; it never reads or stores tokens.

use super::{claude_command, launch_error, ClaudeCodeError};
use crate::local_environment::{spawn_process_group, terminate_process_group};
use serde::Serialize;
use serde_json::Value;
use spark_common::agent_settings::NativeAgentSettings;
use std::io::{BufRead, BufReader, Write};
use std::process::{ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const AUTH_REQUIRED_MESSAGE: &str =
    "Claude Code needs sign-in. Sign in to Claude, then retry your message or run.";

const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
/// How long `claude auth status` may take before Spark reports it as stuck.
const STATUS_TIMEOUT: Duration = Duration::from_secs(10);

/// Matches the CLI's own auth errors ("Failed to authenticate: …",
/// "Not logged in · Please run /login"), not agent text that mentions logins.
pub fn requires_login(message: &str) -> bool {
    let message = message.trim().to_ascii_lowercase();
    message.starts_with("failed to authenticate")
        || message.contains("please run /login")
        || message.contains("claude code needs sign-in")
}

#[derive(Clone, Debug, Serialize)]
pub struct ClaudeCodeAccount {
    pub kind: String,
    pub email: Option<String>,
    pub plan: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ClaudeCodeConnectionStatus {
    pub status: &'static str,
    pub account: Option<ClaudeCodeAccount>,
    pub login_url: Option<String>,
    pub message: Option<String>,
}

impl ClaudeCodeConnectionStatus {
    fn disconnected(message: Option<String>) -> Self {
        Self {
            status: "disconnected",
            account: None,
            login_url: None,
            message,
        }
    }
}

/// One pending sign-in per Spark server. `claude auth login` opens the browser
/// itself and finishes through a local callback; the link it prints instead
/// ends on a page with a code, which the user pastes back through Spark.
#[derive(Default)]
pub struct ClaudeCodeConnection {
    pending: Option<PendingLogin>,
}

struct PendingLogin {
    /// The settings it was started with.
    native: NativeAgentSettings,
    status: Arc<Mutex<ClaudeCodeConnectionStatus>>,
    stdin: Option<ChildStdin>,
    cancelled: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for PendingLogin {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl ClaudeCodeConnection {
    /// Ends a sign-in started with another program or configuration
    /// directory than `native`'s, which would sign that one in instead.
    fn end_stale(&mut self, native: &NativeAgentSettings) {
        let key = |native: &NativeAgentSettings| {
            (
                native.claude_binary.clone(),
                native.claude_config_dir.clone(),
            )
        };
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| key(&pending.native) != key(native))
        {
            self.pending = None;
        }
    }

    pub fn status(
        &mut self,
        native: &NativeAgentSettings,
    ) -> Result<ClaudeCodeConnectionStatus, ClaudeCodeError> {
        self.end_stale(native);
        if let Some(pending) = &self.pending {
            let status = lock(&pending.status)?.clone();
            if status.status != "pending" {
                self.pending = None;
            }
            return Ok(status);
        }
        read_status(native)
    }

    pub fn start_login(
        &mut self,
        native: &NativeAgentSettings,
        completed: impl FnOnce(&ClaudeCodeConnectionStatus) + Send + 'static,
    ) -> Result<ClaudeCodeConnectionStatus, ClaudeCodeError> {
        self.end_stale(native);
        if let Some(pending) = &self.pending {
            let status = lock(&pending.status)?.clone();
            if status.status == "pending" {
                return Ok(status);
            }
        }
        self.pending = None;
        let (executable, mut command) = claude_command(Some(native));
        command
            .args(["auth", "login"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child =
            spawn_process_group(command).map_err(|error| launch_error(&executable, error))?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let status = Arc::new(Mutex::new(ClaudeCodeConnectionStatus {
            status: "pending",
            account: None,
            login_url: None,
            message: None,
        }));
        // The CLI reports problems such as a mistyped code on stderr and keeps
        // waiting, so the latest line is the message to show.
        let stderr_status = Arc::clone(&status);
        let stderr_reader = thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if !line.trim().is_empty() {
                    if let Ok(mut status) = stderr_status.lock() {
                        status.message = Some(line.trim().to_string());
                    }
                }
            }
        });
        let (url_sender, url_receiver) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(url) = login_url(&line) {
                    let _ = url_sender.send(url);
                }
            }
        });
        let Ok(url) = url_receiver.recv_timeout(Duration::from_secs(30)) else {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stderr_reader.join();
            let detail = lock(&status)?.message.clone().unwrap_or_default();
            return Err(ClaudeCodeError::runtime(
                format!("Claude Code did not start sign-in. {detail}")
                    .trim()
                    .to_string(),
            ));
        };
        lock(&status)?.login_url = Some(url);
        let snapshot = lock(&status)?.clone();

        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_status = Arc::clone(&status);
        let worker_cancelled = Arc::clone(&cancelled);
        let started_with = native.clone();
        let native = native.clone();
        let worker = thread::spawn(move || {
            let started = Instant::now();
            let exit = loop {
                if worker_cancelled.load(Ordering::Relaxed) || started.elapsed() >= LOGIN_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                match child.try_wait() {
                    Ok(Some(exit)) => break Some(exit.success()),
                    Ok(None) => thread::sleep(Duration::from_millis(100)),
                    Err(_) => break Some(false),
                }
            };
            let _ = stderr_reader.join();
            let next = match exit {
                None => ClaudeCodeConnectionStatus::disconnected(Some(
                    if worker_cancelled.load(Ordering::Relaxed) {
                        "Sign-in cancelled."
                    } else {
                        "Sign-in timed out. Please sign in again."
                    }
                    .into(),
                )),
                Some(true) => read_status(&native).unwrap_or_else(|error| {
                    ClaudeCodeConnectionStatus::disconnected(Some(error.message))
                }),
                Some(false) => ClaudeCodeConnectionStatus::disconnected(Some(
                    worker_status
                        .lock()
                        .ok()
                        .and_then(|status| status.message.clone())
                        .unwrap_or_else(|| "Claude sign-in failed. Please try again.".into()),
                )),
            };
            if let Ok(mut current) = worker_status.lock() {
                *current = next.clone();
            }
            completed(&next);
        });
        self.pending = Some(PendingLogin {
            native: started_with,
            status,
            stdin,
            cancelled,
            worker: Some(worker),
        });
        Ok(snapshot)
    }

    /// Passes `code` to the sign-in started with `native`'s program.
    pub fn submit_code(
        &mut self,
        native: &NativeAgentSettings,
        code: &str,
    ) -> Result<ClaudeCodeConnectionStatus, ClaudeCodeError> {
        self.end_stale(native);
        let code = code.trim();
        // One line only: anything more would feed the CLI extra input.
        if code.is_empty() || code.contains(['\n', '\r']) {
            return Err(ClaudeCodeError::configuration(
                "Paste the full code shown after sign-in.",
            ));
        }
        let pending = self
            .pending
            .as_mut()
            .filter(|pending| {
                pending
                    .status
                    .lock()
                    .is_ok_and(|status| status.status == "pending")
            })
            .ok_or_else(|| {
                ClaudeCodeError::configuration(
                    "No Claude sign-in is waiting for a code. Sign in again.",
                )
            })?;
        let stdin = pending.stdin.as_mut().ok_or_else(|| {
            ClaudeCodeError::runtime("Claude Code sign-in is not accepting codes.")
        })?;
        lock(&pending.status)?.message = None;
        writeln!(stdin, "{code}")
            .and_then(|()| stdin.flush())
            .map_err(|error| {
                ClaudeCodeError::runtime(format!("Claude Code did not accept the code: {error}"))
            })?;
        Ok(lock(&pending.status)?.clone())
    }

    pub fn cancel_login(&mut self) -> ClaudeCodeConnectionStatus {
        self.pending = None;
        ClaudeCodeConnectionStatus::disconnected(Some(
            "Sign-in cancelled. Existing credentials were kept.".into(),
        ))
    }
}

fn read_status(
    native: &NativeAgentSettings,
) -> Result<ClaudeCodeConnectionStatus, ClaudeCodeError> {
    let (executable, mut command) = claude_command(Some(native));
    // Exits 1 when signed out but still prints the report.
    command
        .args(["auth", "status", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = spawn_process_group(command).map_err(|error| launch_error(&executable, error))?;
    let group = child.id();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(child.wait_with_output());
    });
    let output = match receiver.recv_timeout(STATUS_TIMEOUT) {
        Ok(output) => output.map_err(|error| launch_error(&executable, error))?,
        Err(_) => {
            terminate_process_group(group, None);
            return Err(ClaudeCodeError::runtime(format!(
                "Claude Code ({}) did not report its sign-in status within {}s.",
                executable.display(),
                STATUS_TIMEOUT.as_secs()
            )));
        }
    };
    let report = serde_json::from_slice::<Value>(&output.stdout)
        .ok()
        .filter(|report| report.get("loggedIn").is_some_and(Value::is_boolean))
        .ok_or_else(|| {
            ClaudeCodeError::runtime(format!(
                "Claude Code did not report its sign-in status. {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        })?;
    let text = |key: &str| report.get(key).and_then(Value::as_str).map(str::to_owned);
    if report["loggedIn"] != true {
        return Ok(ClaudeCodeConnectionStatus::disconnected(None));
    }
    Ok(ClaudeCodeConnectionStatus {
        status: "connected",
        account: Some(ClaudeCodeAccount {
            kind: text("authMethod").unwrap_or_else(|| "unknown".into()),
            email: text("email"),
            plan: text("subscriptionType"),
        }),
        login_url: None,
        message: None,
    })
}

/// The printed fallback link, minus any terminal hyperlink escapes.
fn login_url(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let url = line[start..]
        .split(|c: char| c.is_whitespace() || c.is_control())
        .next()?;
    Some(url.to_string())
}

fn lock(
    status: &Mutex<ClaudeCodeConnectionStatus>,
) -> Result<std::sync::MutexGuard<'_, ClaudeCodeConnectionStatus>, ClaudeCodeError> {
    status.lock().map_err(|_| {
        ClaudeCodeError::runtime(
            "Claude Code connection state is unavailable. Restart Spark and try again.",
        )
    })
}

/// The signed-in account's plan limits, from the CLI's `get_usage` control
/// request. The request makes no model call.
// ponytail: Claude Code marks get_usage experimental; a changed shape reports
// an error here instead of a wrong number.
pub fn read_usage(
    native: &NativeAgentSettings,
) -> Result<Vec<crate::UsageWindow>, ClaudeCodeError> {
    let (executable, mut command) = claude_command(Some(native));
    command
        .args([
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
        ])
        .args([
            "--verbose",
            "--no-session-persistence",
            "--strict-mcp-config",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child =
        spawn_process_group(command).map_err(|error| launch_error(&executable, error))?;
    let group = child.id();
    let request = r#"{"type":"control_request","request_id":"spark-usage","request":{"subtype":"get_usage","skip_behaviors":true}}"#;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = writeln!(stdin, "{request}").and_then(|()| stdin.flush());
        // stdin stays open until the reply arrives: closing it ends the session first.
        let reply = BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
            .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
            .find(|message| {
                message["type"] == "control_response"
                    && message["response"]["request_id"] == "spark-usage"
            });
        let _ = sender.send(reply);
        drop(stdin);
    });
    let reply = receiver.recv_timeout(STATUS_TIMEOUT);
    terminate_process_group(group, None);
    let _ = child.wait();
    let reply = match reply {
        Ok(Some(reply)) => reply,
        Ok(None) => {
            return Err(ClaudeCodeError::runtime(
                "Claude Code exited without reporting its usage.",
            ))
        }
        Err(_) => {
            return Err(ClaudeCodeError::runtime(format!(
                "Claude Code did not report its usage within {}s.",
                STATUS_TIMEOUT.as_secs()
            )))
        }
    };
    usage_windows(&reply["response"])
}

fn usage_windows(response: &Value) -> Result<Vec<crate::UsageWindow>, ClaudeCodeError> {
    if response["subtype"] != "success" {
        let message = response["error"].as_str().unwrap_or("unknown error");
        return Err(ClaudeCodeError::runtime(format!(
            "Claude Code could not read usage: {message}"
        )));
    }
    let usage = &response["response"];
    // Signed out, or signed in with an API key: no plan, so no limits.
    if usage["rate_limits_available"] == false {
        return Ok(Vec::new());
    }
    let invalid = || ClaudeCodeError::runtime("Claude Code reported usage in an unknown format.");
    let limits = usage["rate_limits"]["limits"]
        .as_array()
        .ok_or_else(invalid)?;
    limits
        .iter()
        .map(|limit| {
            let group = limit["group"].as_str().ok_or_else(invalid)?;
            let mut label = match group {
                "weekly" => "Weekly".to_string(),
                other => {
                    let other = other.replace('_', " ");
                    let mut chars = other.chars();
                    chars.next().map_or(String::new(), |first| {
                        first.to_uppercase().chain(chars).collect()
                    })
                }
            };
            if let Some(model) = limit["scope"]["model"]["display_name"].as_str() {
                label = format!("{label} · {model}");
            }
            Ok(crate::UsageWindow {
                label,
                used_percent: limit["percent"].as_f64().ok_or_else(invalid)?,
                resets_at: limit["resets_at"].as_str().map(str::to_owned),
            })
        })
        .collect()
}
