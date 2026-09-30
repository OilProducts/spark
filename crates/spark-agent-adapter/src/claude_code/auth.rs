//! Claude Code sign-in through the CLI's own `claude auth` commands. Spark
//! relays the sign-in link and a pasted code; it never reads or stores tokens.

use super::{claude_command, launch_error, ClaudeCodeError};
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
    pub fn status(
        &mut self,
        native: &NativeAgentSettings,
    ) -> Result<ClaudeCodeConnectionStatus, ClaudeCodeError> {
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
        if let Some(pending) = &self.pending {
            let status = lock(&pending.status)?.clone();
            if status.status == "pending" {
                return Ok(status);
            }
        }
        self.pending = None;
        let (executable, mut command) = claude_command(Some(native));
        let mut child = command
            .args(["auth", "login"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| launch_error(&executable, error))?;
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
            status,
            stdin,
            cancelled,
            worker: Some(worker),
        });
        Ok(snapshot)
    }

    pub fn submit_code(
        &mut self,
        code: &str,
    ) -> Result<ClaudeCodeConnectionStatus, ClaudeCodeError> {
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
    let output = command
        .args(["auth", "status", "--json"])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| launch_error(&executable, error))?;
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
