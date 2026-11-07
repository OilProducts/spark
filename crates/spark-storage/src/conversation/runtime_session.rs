use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::ConversationRepository;

pub const RUNTIME_SESSION_SCHEMA_VERSION: i64 = 1;
/// Best-effort model runtime continuity for one conversation.
///
/// Recovery bookkeeping in the conversation database, keyed by provider:
/// never part of snapshots or browser history, and losing it only costs
/// thread resume (the next turn starts a fresh provider thread). A resume
/// failure marks the record (`resume_failed`) while keeping the failed
/// thread id for debugging.
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
    /// The conversation's latest runtime session. Unreadable payloads resolve
    /// to `None` (continuity is best-effort; the next write replaces them).
    pub fn read_runtime_session(
        &self,
        conversation_id: &str,
        _project_path: Option<&str>,
    ) -> Result<Option<RuntimeSession>> {
        Ok(self
            .store()
            .database()
            .read(|read| read.runtime_session(conversation_id))?
            .and_then(|payload| serde_json::from_value::<RuntimeSession>(payload).ok())
            .filter(|session| session.schema_version == RUNTIME_SESSION_SCHEMA_VERSION))
    }
}
