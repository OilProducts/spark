//! Durable questions use a process-held lock as their live-session boundary.
//! Locks vanish on executor death; journal history deliberately does not.
use std::fs::{File, OpenOptions};

use serde_json::Value;

use crate::{RunRootPaths, RunStore};

pub struct ActiveClarification {
    pub id: String,
    lease: File,
    paths: RunRootPaths,
}

fn open_lock(paths: &RunRootPaths, id: &str, suffix: &str) -> Result<File, String> {
    if id.len() != 32 || !id.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Invalid clarification identity".into());
    }
    let directory = paths.root.join("clarifications");
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join(format!("{id}.{suffix}")))
        .map_err(|e| e.to_string())
}

impl ActiveClarification {
    pub fn new(paths: &RunRootPaths) -> Result<Self, String> {
        let id = format!("{:032x}", rand::random::<u128>());
        let lease = open_lock(paths, &id, "active")?;
        lease.lock().map_err(|e| e.to_string())?;
        Ok(Self {
            id,
            lease,
            paths: paths.clone(),
        })
    }
}

impl Drop for ActiveClarification {
    fn drop(&mut self) {
        // Finish any accepted submission before invalidating the request.
        if let Ok(lock) = open_lock(&self.paths, &self.id, "answers") {
            if lock.lock().is_ok() {
                let _ = self.lease.unlock();
            }
        }
    }
}

pub fn is_active(paths: &RunRootPaths, question: &Value) -> bool {
    let Some(id) = question.get("clarification_id").and_then(Value::as_str) else {
        return false;
    };
    let Ok(file) = open_lock(paths, id, "active") else {
        return false;
    };
    matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock))
}

/// Serialize submissions, then recheck the journal under the lock. An accepted
/// answer is immutable even across concurrent HTTP requests and processes.
pub fn accept_answer(
    store: &RunStore,
    paths: &RunRootPaths,
    question: &Value,
    event: attractor_core::RawRuntimeEvent,
) -> Result<(), String> {
    let id = question
        .get("clarification_id")
        .and_then(Value::as_str)
        .ok_or("Missing clarification identity")?;
    let lock = open_lock(paths, id, "answers")?;
    lock.lock().map_err(|e| e.to_string())?;
    if !is_active(paths, question) {
        return Err("Clarification is no longer active".into());
    }
    let status = crate::read_run_record(paths)
        .map_err(|e| e.to_string())?
        .map(|r| r.status)
        .unwrap_or_default();
    if !matches!(status.as_str(), "running" | "waiting") {
        return Err("Run is not accepting answers".into());
    }
    let events = crate::read_raw_events(paths).map_err(|e| e.to_string())?;
    if events.iter().any(|e| {
        e.event_type == "InterviewCompleted"
            && e.payload.get("question_id") == question.get("question_id")
    }) {
        return Err("Question already answered".into());
    }
    store
        .append_event(paths, event)
        .map(|_| ())
        .map_err(|e| e.to_string())
}
