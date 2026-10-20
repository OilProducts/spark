//! Conversation attachments: uploads stored with the conversation, resolved
//! on the server when a turn references them.

use std::collections::HashSet;
use std::path::PathBuf;

use serde_json::{json, Value};
use spark_agent_adapter::AgentAttachment;
use spark_storage::conversation::ConversationAttachment;
use spark_storage::ConversationRepository;
use time::OffsetDateTime;

use crate::conversations::WorkspaceConversationService;
use crate::errors::{WorkspaceError, WorkspaceResult};

pub const MAX_ATTACHMENTS_PER_MESSAGE: usize = 10;
pub const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_MESSAGE_ATTACHMENT_BYTES: u64 = 50 * 1024 * 1024;
/// Unsent uploads survive this long so a failed send can be retried.
const ABANDONED_UPLOAD_RETENTION: time::Duration = time::Duration::hours(24);

impl WorkspaceConversationService {
    pub fn upload_attachment(
        &self,
        conversation_id: &str,
        project_path: &str,
        filename: &str,
        bytes: &[u8],
    ) -> WorkspaceResult<ConversationAttachment> {
        if bytes.len() > MAX_ATTACHMENT_BYTES {
            return Err(WorkspaceError::Validation(
                "Attachments are limited to 20 MiB per file.".to_string(),
            ));
        }
        let project_path = self.bound_chat_project_path(conversation_id, project_path)?;
        Ok(self
            .repository()
            .create_attachment(conversation_id, &project_path, filename, bytes)?)
    }

    pub fn read_attachment(
        &self,
        conversation_id: &str,
        project_path: &str,
        attachment_id: &str,
    ) -> WorkspaceResult<(ConversationAttachment, Vec<u8>)> {
        let project_path = self.bound_chat_project_path(conversation_id, project_path)?;
        let (record, path) = self
            .repository()
            .read_attachment(conversation_id, &project_path, attachment_id)?
            .ok_or_else(|| unknown_attachment(attachment_id))?;
        let bytes = std::fs::read(&path).map_err(|error| {
            WorkspaceError::Internal(format!(
                "Unable to read attachment {attachment_id}: {error}"
            ))
        })?;
        Ok((record, bytes))
    }

    /// Delete an upload that no sent message references.
    pub fn delete_draft_attachment(
        &self,
        conversation_id: &str,
        project_path: &str,
        attachment_id: &str,
    ) -> WorkspaceResult<Value> {
        let project_path = self.bound_chat_project_path(conversation_id, project_path)?;
        // Held through the delete, so a send cannot start referencing it.
        let turns = crate::conversations::active_turns();
        if turns
            .get(&self.active_turn_key(&project_path, conversation_id))
            .is_some_and(|active| active.attachment_ids.contains(attachment_id))
        {
            return Err(WorkspaceError::Conflict(
                "This attachment belongs to a sent message and stays until the conversation is deleted.".to_string(),
            ));
        }
        if !self.repository().delete_unreferenced_attachment(
            conversation_id,
            &project_path,
            attachment_id,
        )? {
            return Err(unknown_attachment(attachment_id));
        }
        Ok(json!({"status": "deleted", "attachment_id": attachment_id}))
    }

    /// Resolve a turn's attachment ids against this conversation's uploads
    /// and enforce the per-message limits.
    pub(crate) fn resolve_turn_attachments(
        &self,
        conversation_id: &str,
        project_path: &str,
        attachment_ids: &[String],
    ) -> WorkspaceResult<Vec<AgentAttachment>> {
        if attachment_ids.len() > MAX_ATTACHMENTS_PER_MESSAGE {
            return Err(WorkspaceError::Validation(format!(
                "A message can carry at most {MAX_ATTACHMENTS_PER_MESSAGE} attachments."
            )));
        }
        let repository = self.repository();
        let mut seen = HashSet::new();
        let mut attachments = Vec::new();
        for id in attachment_ids {
            if !seen.insert(id.as_str()) {
                return Err(WorkspaceError::Validation(format!(
                    "Attachment {id} is listed more than once."
                )));
            }
            let attachment = resolve_attachment(&repository, conversation_id, project_path, id)?
                .ok_or_else(|| {
                    WorkspaceError::Validation(format!(
                        "Attachment {id} is not available in this conversation. Remove it and attach the file again."
                    ))
                })?;
            attachments.push(attachment);
        }
        if attachments
            .iter()
            .map(|attachment| attachment.size_bytes)
            .sum::<u64>()
            > MAX_MESSAGE_ATTACHMENT_BYTES
        {
            return Err(WorkspaceError::Validation(
                "Attachments are limited to 50 MiB per message.".to_string(),
            ));
        }
        Ok(attachments)
    }

    /// Remove unreferenced uploads older than the retry window.
    pub(crate) fn prune_abandoned_attachments(
        &self,
        conversation_id: &str,
        project_path: &str,
    ) -> WorkspaceResult<Vec<String>> {
        // ponytail: skips the whole prune while steered messages hold uploads.
        let turns = crate::conversations::active_turns();
        if turns
            .get(&self.active_turn_key(project_path, conversation_id))
            .is_some_and(|active| !active.attachment_ids.is_empty())
        {
            return Ok(Vec::new());
        }
        Ok(self.repository().prune_attachments(
            conversation_id,
            project_path,
            OffsetDateTime::now_utc() - ABANDONED_UPLOAD_RETENTION,
        )?)
    }
}

/// An attachment's agent-facing record, or `None` if this conversation has
/// no such upload.
pub(crate) fn resolve_attachment(
    repository: &ConversationRepository,
    conversation_id: &str,
    project_path: &str,
    attachment_id: &str,
) -> WorkspaceResult<Option<AgentAttachment>> {
    Ok(repository
        .read_attachment(conversation_id, project_path, attachment_id)?
        .map(|(record, path)| agent_attachment(record, path)))
}

fn agent_attachment(record: ConversationAttachment, path: PathBuf) -> AgentAttachment {
    AgentAttachment {
        id: record.id,
        filename: record.filename,
        media_type: record.media_type,
        size_bytes: record.size_bytes,
        path: path.to_string_lossy().to_string(),
    }
}

/// The attachment reference persisted on a user turn.
pub(crate) fn turn_attachment_reference(attachment: &AgentAttachment) -> Value {
    json!({
        "id": attachment.id,
        "filename": attachment.filename,
        "media_type": attachment.media_type,
        "size_bytes": attachment.size_bytes,
    })
}

pub(crate) fn turn_attachment_ids(turn: &Value) -> Vec<String> {
    spark_storage::conversation::turn_attachment_ids(turn.get("attachments"))
}

fn unknown_attachment(attachment_id: &str) -> WorkspaceError {
    WorkspaceError::NotFound(format!("Unknown attachment: {attachment_id}"))
}
