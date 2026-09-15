//! Managed Codex login. Spark never reads or writes OAuth tokens.

use super::{CodexAppServerClient, CodexAppServerError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use spark_common::agent_settings::NativeAgentSettings;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const AUTH_REQUIRED_MESSAGE: &str =
    "Your Codex connection needs sign-in. Reconnect Codex, then retry your message or run.";

pub fn requires_login(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    if message.trim() == "unauthorized" || message.trim_end().ends_with(": unauthorized") {
        return true;
    }
    [
        "codex connection needs sign-in",
        "refresh token was already used",
        "refresh token has expired",
        "refresh token has been invalidated",
        "refresh token is invalid",
        "refresh_token_reused",
        "refresh_token_expired",
        "refresh_token_invalidated",
        "access token has expired",
        "access token could not be refreshed",
        "not logged in",
        "authentication required",
        "please log in",
        "please login",
        "please sign in",
    ]
    .iter()
    .any(|pattern| message.contains(pattern))
}

pub(super) fn normalize_auth_error(message: String) -> String {
    if requires_login(&message) {
        AUTH_REQUIRED_MESSAGE.to_string()
    } else {
        message
    }
}

pub(super) fn error_message(error: &Value) -> Option<String> {
    if error
        .get("codexErrorInfo")
        .or_else(|| error.pointer("/data/codexErrorInfo"))
        .and_then(Value::as_str)
        == Some("unauthorized")
    {
        return Some(AUTH_REQUIRED_MESSAGE.into());
    }
    error
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexLoginMethod {
    Browser,
    Device,
}

#[derive(Clone, Debug, Serialize)]
pub struct CodexAccount {
    pub kind: String,
    pub email: Option<String>,
    pub plan: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CodexConnectionStatus {
    pub status: &'static str,
    pub account: Option<CodexAccount>,
    pub login_url: Option<String>,
    pub user_code: Option<String>,
    pub message: Option<String>,
}

impl CodexConnectionStatus {
    fn disconnected(message: Option<String>) -> Self {
        Self {
            status: "disconnected",
            account: None,
            login_url: None,
            user_code: None,
            message,
        }
    }
}

/// One pending sign-in per Spark server. Codex persists the result in the same
/// runtime home used by turns; the temporary login process has no thread state.
#[derive(Default)]
pub struct CodexConnection {
    pending: Option<PendingLogin>,
}

struct PendingLogin {
    status: Arc<Mutex<CodexConnectionStatus>>,
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

impl CodexConnection {
    pub fn status(
        &mut self,
        working_dir: &Path,
        native: &NativeAgentSettings,
    ) -> Result<CodexConnectionStatus, CodexAppServerError> {
        if let Some(pending) = &self.pending {
            let status = pending.status.lock().map_err(|_| state_error())?.clone();
            if status.status != "pending" {
                self.pending = None;
            }
            return Ok(status);
        }
        let mut client = CodexAppServerClient::connect_with_settings(
            working_dir.to_path_buf(),
            None,
            Some(native),
        )?;
        match client.read_account(true) {
            Err(error) if requires_login(&error.message) => Ok(
                CodexConnectionStatus::disconnected(Some(AUTH_REQUIRED_MESSAGE.into())),
            ),
            result => result,
        }
    }

    pub fn start_login(
        &mut self,
        working_dir: &Path,
        native: &NativeAgentSettings,
        method: CodexLoginMethod,
    ) -> Result<CodexConnectionStatus, CodexAppServerError> {
        if let Some(pending) = &self.pending {
            let status = pending.status.lock().map_err(|_| state_error())?.clone();
            if status.status == "pending" {
                return Ok(status);
            }
        }
        self.pending = None;
        // No trace sink: login URLs and account details never enter run logs.
        let mut client = CodexAppServerClient::connect_with_settings(
            working_dir.to_path_buf(),
            None,
            Some(native),
        )?;
        let result = client.account_request(
            "account/login/start",
            json!({
                "type": match method {
                    CodexLoginMethod::Browser => "chatgpt",
                    CodexLoginMethod::Device => "chatgptDeviceCode",
                },
            }),
        )?;
        let login_id = required_string(&result, "loginId")?;
        let (login_url, user_code) = match method {
            CodexLoginMethod::Browser => (required_string(&result, "authUrl")?, None),
            CodexLoginMethod::Device => (
                required_string(&result, "verificationUrl")?,
                Some(required_string(&result, "userCode")?),
            ),
        };
        let status = CodexConnectionStatus {
            status: "pending",
            account: None,
            login_url: Some(login_url),
            user_code,
            message: None,
        };
        let shared_status = Arc::new(Mutex::new(status.clone()));
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_status = Arc::clone(&shared_status);
        let worker_cancelled = Arc::clone(&cancelled);
        let worker = thread::spawn(move || {
            let result =
                client.wait_for_login(&login_id, &worker_cancelled, Duration::from_secs(600));
            let status = result
                .unwrap_or_else(|error| CodexConnectionStatus::disconnected(Some(error.message)));
            if let Ok(mut current) = worker_status.lock() {
                *current = status;
            }
        });
        self.pending = Some(PendingLogin {
            status: shared_status,
            cancelled,
            worker: Some(worker),
        });
        Ok(status)
    }

    pub fn cancel_login(&mut self) -> CodexConnectionStatus {
        self.pending = None;
        CodexConnectionStatus::disconnected(Some(
            "Sign-in cancelled. Existing credentials were kept.".into(),
        ))
    }
}

impl CodexAppServerClient {
    fn account_request(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<Value, CodexAppServerError> {
        let response = self.send_request(method, Some(params))?;
        if response.get("error").is_some() {
            return Err(super::rpc_error("Codex connection failed", &response));
        }
        response
            .get("result")
            .filter(|value| value.is_object())
            .cloned()
            .ok_or_else(|| {
                CodexAppServerError::runtime("Codex returned an invalid account response.")
            })
    }

    fn read_account(
        &mut self,
        refresh: bool,
    ) -> Result<CodexConnectionStatus, CodexAppServerError> {
        let result = self.account_request("account/read", json!({"refreshToken": refresh}))?;
        let requires_auth = result
            .get("requiresOpenaiAuth")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                CodexAppServerError::runtime("Codex did not report authentication requirements.")
            })?;
        let account = match result.get("account") {
            Some(Value::Null) => None,
            Some(account) if account.is_object() => Some(CodexAccount {
                kind: required_string(account, "type")?,
                email: account
                    .get("email")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                plan: account
                    .get("planType")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            }),
            _ => {
                return Err(CodexAppServerError::runtime(
                    "Codex did not report an account.",
                ))
            }
        };
        Ok(CodexConnectionStatus {
            status: if account.is_some() || !requires_auth {
                "connected"
            } else {
                "disconnected"
            },
            account,
            login_url: None,
            user_code: None,
            message: None,
        })
    }

    fn wait_for_login(
        &mut self,
        login_id: &str,
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> Result<CodexConnectionStatus, CodexAppServerError> {
        let started = Instant::now();
        loop {
            if cancelled.load(Ordering::Relaxed) || started.elapsed() >= timeout {
                let _ = self.account_request("account/login/cancel", json!({"loginId": login_id}));
                return Ok(CodexConnectionStatus::disconnected(Some(
                    if cancelled.load(Ordering::Relaxed) {
                        "Sign-in cancelled."
                    } else {
                        "Sign-in timed out. Please connect again."
                    }
                    .into(),
                )));
            }
            if let Some(message) = self.next_message(Duration::from_millis(100))? {
                if message.get("method").and_then(Value::as_str) != Some("account/login/completed")
                    || message.pointer("/params/loginId").and_then(Value::as_str) != Some(login_id)
                {
                    continue;
                }
                if message.pointer("/params/success").and_then(Value::as_bool) == Some(true) {
                    return self.read_account(false);
                }
                return Ok(CodexConnectionStatus::disconnected(Some(
                    message
                        .pointer("/params/error")
                        .and_then(Value::as_str)
                        .unwrap_or("Codex sign-in failed. Please try again.")
                        .to_string(),
                )));
            }
            if self
                .child
                .try_wait()
                .map_err(|error| CodexAppServerError::runtime(error.to_string()))?
                .is_some()
            {
                return Err(CodexAppServerError::runtime(
                    "Codex exited before sign-in completed. Please connect again.",
                ));
            }
        }
    }
}

fn required_string(value: &Value, key: &str) -> Result<String, CodexAppServerError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            CodexAppServerError::runtime(format!("Codex account response is missing {key}."))
        })
}

fn state_error() -> CodexAppServerError {
    CodexAppServerError::runtime(
        "Codex connection state is unavailable. Restart Spark and try again.",
    )
}
