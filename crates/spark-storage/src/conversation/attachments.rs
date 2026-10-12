use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::error::{Result, StorageError};
use crate::{
    read_json_optional, write_atomic, write_json_atomic, ConversationRepository, JsonWriteOptions,
};

use super::tool_output::is_safe_segment_file_id;

pub const ATTACHMENTS_DIR_NAME: &str = "attachments";
const ATTACHMENT_RECORD_FILE_NAME: &str = "attachment.json";

/// One uploaded conversation file. Bytes live beside the record under a
/// server-generated id; `filename` is the client's name, kept for display only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationAttachment {
    pub id: String,
    pub filename: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub created_at: String,
    /// Content file name inside the attachment directory.
    pub stored_name: String,
}

impl ConversationRepository {
    fn attachments_dir(&self, conversation_id: &str, project_path: &str) -> Result<PathBuf> {
        if !is_safe_segment_file_id(conversation_id) {
            return Err(StorageError::InvalidRepositoryPath {
                path: PathBuf::from(conversation_id),
                reason: "Invalid conversation id.".to_string(),
            });
        }
        Ok(self
            .project_paths(project_path)?
            .conversations_dir
            .join(conversation_id)
            .join(ATTACHMENTS_DIR_NAME))
    }

    /// Store `bytes` as a new attachment. The media type is detected from
    /// the content; the client filename never becomes part of a path.
    pub fn create_attachment(
        &self,
        conversation_id: &str,
        project_path: &str,
        filename: &str,
        bytes: &[u8],
    ) -> Result<ConversationAttachment> {
        let id = format!("att-{:032x}", rand::thread_rng().gen::<u128>());
        let dir = self
            .attachments_dir(conversation_id, project_path)?
            .join(&id);
        let (media_type, extension) = detect_media_type(bytes);
        let record = ConversationAttachment {
            id,
            filename: display_filename(filename),
            media_type: media_type.to_string(),
            size_bytes: bytes.len() as u64,
            created_at: OffsetDateTime::now_utc()
                .replace_nanosecond(0)
                .ok()
                .and_then(|now| now.format(&Rfc3339).ok())
                .unwrap_or_default(),
            stored_name: format!("content.{extension}"),
        };
        write_atomic(dir.join(&record.stored_name), bytes)?;
        // The record is written last: an attachment without one was never published.
        write_json_atomic(
            dir.join(ATTACHMENT_RECORD_FILE_NAME),
            &record,
            JsonWriteOptions::default(),
        )?;
        Ok(record)
    }

    /// The attachment record and its content path, or `None` when this
    /// conversation has no such attachment or its stored content is missing,
    /// unreadable or not the recorded size.
    pub fn read_attachment(
        &self,
        conversation_id: &str,
        project_path: &str,
        attachment_id: &str,
    ) -> Result<Option<(ConversationAttachment, PathBuf)>> {
        if !is_safe_segment_file_id(attachment_id) {
            return Ok(None);
        }
        let dir = self
            .attachments_dir(conversation_id, project_path)?
            .join(attachment_id);
        let Some(record) = read_attachment_record(&dir)?.filter(|record| {
            record.id == attachment_id && is_safe_segment_file_id(&record.stored_name)
        }) else {
            return Ok(None);
        };
        let path = dir.join(&record.stored_name);
        let readable = fs::File::open(&path)
            .and_then(|file| file.metadata())
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() == record.size_bytes);
        Ok(readable.then_some((record, path)))
    }

    pub fn list_attachments(
        &self,
        conversation_id: &str,
        project_path: &str,
    ) -> Result<Vec<ConversationAttachment>> {
        let dir = self.attachments_dir(conversation_id, project_path)?;
        let mut records = Vec::new();
        for entry in read_dir_or_empty(&dir)? {
            if let Some(record) = read_attachment_record(&entry)? {
                records.push(record);
            }
        }
        records.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(records)
    }

    /// Delete one attachment no turn references. Returns whether it existed;
    /// a referenced attachment is a `ConversationCommitRejected` error. Runs
    /// under the conversation lock, so it cannot interleave with a commit
    /// that starts referencing the attachment.
    pub fn delete_unreferenced_attachment(
        &self,
        conversation_id: &str,
        project_path: &str,
        attachment_id: &str,
    ) -> Result<bool> {
        if !is_safe_segment_file_id(attachment_id) {
            return Ok(false);
        }
        let dir = self
            .attachments_dir(conversation_id, project_path)?
            .join(attachment_id);
        if !dir.exists() {
            return Ok(false);
        }
        let _lock = self.lock_conversation(conversation_id, project_path)?;
        if self
            .referenced_attachment_ids(conversation_id, project_path)?
            .contains(attachment_id)
        {
            return Err(StorageError::ConversationCommitRejected {
                conversation_id: conversation_id.to_string(),
                reason: "This attachment belongs to a sent message and stays until the conversation is deleted.".to_string(),
            });
        }
        match fs::remove_dir_all(&dir) {
            Ok(()) => Ok(true),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(StorageError::io("delete attachment", dir, source)),
        }
    }

    /// Delete attachments no turn references that are older than `cutoff`,
    /// plus unpublished leftovers of interrupted uploads. Returns the deleted
    /// ids. Runs under the conversation lock, like
    /// `delete_unreferenced_attachment`.
    pub fn prune_attachments(
        &self,
        conversation_id: &str,
        project_path: &str,
        cutoff: OffsetDateTime,
    ) -> Result<Vec<String>> {
        let dir = self.attachments_dir(conversation_id, project_path)?;
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let _lock = self.lock_conversation(conversation_id, project_path)?;
        let referenced = self.referenced_attachment_ids(conversation_id, project_path)?;
        let mut removed = Vec::new();
        for entry in read_dir_or_empty(&dir)? {
            let created_at = match read_attachment_record(&entry)? {
                Some(record) if referenced.contains(&record.id) => continue,
                Some(record) => OffsetDateTime::parse(&record.created_at, &Rfc3339).ok(),
                None => fs::metadata(&entry)
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .map(OffsetDateTime::from),
            };
            if created_at.is_some_and(|created_at| created_at >= cutoff) {
                continue;
            }
            match fs::remove_dir_all(&entry) {
                Ok(()) => {}
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => return Err(StorageError::io("prune attachment", entry, source)),
            }
            if let Some(name) = entry.file_name().and_then(|name| name.to_str()) {
                removed.push(name.to_string());
            }
        }
        Ok(removed)
    }
}

impl ConversationRepository {
    /// Ids referenced by the committed turns. Callers hold the conversation lock.
    fn referenced_attachment_ids(
        &self,
        conversation_id: &str,
        project_path: &str,
    ) -> Result<HashSet<String>> {
        Ok(self
            .read_snapshot(conversation_id, Some(project_path))?
            .as_ref()
            .and_then(|snapshot| snapshot.get("turns"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|turn| turn_attachment_ids(turn.get("attachments")))
            .collect())
    }
}

/// The ids in a turn's `attachments` references.
pub fn turn_attachment_ids(attachments: Option<&Value>) -> Vec<String> {
    attachments
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|attachment| attachment.get("id").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

fn read_attachment_record(dir: &Path) -> Result<Option<ConversationAttachment>> {
    // A malformed record reads as unpublished, so pruning can still remove it.
    Ok(
        read_json_optional::<ConversationAttachment>(dir.join(ATTACHMENT_RECORD_FILE_NAME))
            .ok()
            .flatten(),
    )
}

fn read_dir_or_empty(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(StorageError::io("read attachments directory", dir, source)),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|source| StorageError::io("read attachments directory", dir, source))?;
        if entry.path().is_dir() {
            paths.push(entry.path());
        }
    }
    Ok(paths)
}

/// The client's base name with path components and control characters
/// removed; display metadata only.
fn display_filename(filename: &str) -> String {
    let base = filename
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .chars()
        .filter(|ch| !ch.is_control())
        .take(255)
        .collect::<String>();
    let base = base.trim();
    if base.is_empty() || base == "." || base == ".." {
        "attachment".to_string()
    } else {
        base.to_string()
    }
}

/// Media type and stored extension, from the content alone.
fn detect_media_type(bytes: &[u8]) -> (&'static str, &'static str) {
    let bmp_size = bytes
        .get(2..6)
        .map(|size| u32::from_le_bytes([size[0], size[1], size[2], size[3]]) as usize);
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        ("image/png", "png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        ("image/jpeg", "jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        ("image/gif", "gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        ("image/webp", "webp")
    } else if bytes.starts_with(b"BM") && bmp_size == Some(bytes.len()) {
        ("image/bmp", "bmp")
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        ("image/tiff", "tif")
    } else if bytes.starts_with(b"%PDF-") {
        ("application/pdf", "pdf")
    } else if std::str::from_utf8(bytes).is_ok() {
        ("text/plain", "txt")
    } else {
        ("application/octet-stream", "bin")
    }
}
