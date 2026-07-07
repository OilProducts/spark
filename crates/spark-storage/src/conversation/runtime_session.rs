use std::fs;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::{write_json_atomic, ConversationRepository, JsonWriteOptions};

pub const RUNTIME_SESSION_SCHEMA_VERSION: i64 = 1;
pub const RUNTIME_SESSION_FILE_NAME: &str = "runtime-session.json";

/// Best-effort model runtime continuity for one conversation.
///
/// This is a separate authority from the transcript: it is never written by
/// the conversation commit boundary, never parsed by snapshot or journal
/// reads, and losing it only costs thread resume (the next turn starts a
/// fresh provider thread). A resume failure tombstones the record
/// (`resume_failed`) while keeping the failed thread id for debugging.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeSession {
    pub schema_version: i64,
    pub provider: String,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub established_at: String,
    /// The workspace assistant turn id that last confirmed the thread.
    #[serde(default)]
    pub last_turn_id: Option<String>,
    #[serde(default)]
    pub resume_failed: bool,
    #[serde(default)]
    pub updated_at: String,
}

impl ConversationRepository {
    /// Read the runtime session sidecar. Missing or unreadable files resolve
    /// to `None` (continuity is best-effort; the record self-heals on the
    /// next write).
    pub fn read_runtime_session(
        &self,
        conversation_id: &str,
        project_path: Option<&str>,
    ) -> Result<Option<RuntimeSession>> {
        let Some(path) = self.conversation_session_path(conversation_id, project_path)? else {
            return Ok(None);
        };
        let Ok(text) = fs::read_to_string(&path) else {
            return Ok(None);
        };
        Ok(serde_json::from_str::<RuntimeSession>(&text)
            .ok()
            .filter(|session| session.schema_version == RUNTIME_SESSION_SCHEMA_VERSION))
    }

    pub fn write_runtime_session(
        &self,
        conversation_id: &str,
        project_path: &str,
        session: &RuntimeSession,
    ) -> Result<()> {
        let project_paths = self.project_paths(project_path)?;
        let path = project_paths
            .conversations_dir
            .join(conversation_id)
            .join(RUNTIME_SESSION_FILE_NAME);
        write_json_atomic(path, session, JsonWriteOptions::default())
    }
}
