use std::fs;
use std::path::{Path, PathBuf};

use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::error::{Result, StorageError};
use crate::{write_atomic, ConversationRepository};

use super::database::AttachmentRow;
use super::store::{remove_dir_if_exists, LoadedConversation, StoredAttachment};
use super::tool_output::is_safe_segment_file_id;

pub const ATTACHMENTS_DIR_NAME: &str = "attachments";

/// One uploaded conversation file. Bytes live in an immutable blob under a
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

impl StoredAttachment {
    pub(crate) fn path(&self, home_dir: &Path) -> PathBuf {
        home_dir.join(&self.blob_path)
    }

    /// Whether the referenced bytes exist with the recorded size.
    pub(crate) fn readable(&self, home_dir: &Path) -> bool {
        fs::File::open(self.path(home_dir))
            .and_then(|file| file.metadata())
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() == self.record.size_bytes)
    }
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
            .store()
            .conversation_root(project_path, conversation_id)?
            .join(ATTACHMENTS_DIR_NAME))
    }

    /// Store `bytes` as a new draft attachment: the blob is written and
    /// synced before the transaction that records it. The media type is
    /// detected from the content; the client filename never becomes part of
    /// a path. A conversation that does not exist yet is created bound to
    /// `project_path`, with no transcript.
    pub fn create_attachment(
        &self,
        conversation_id: &str,
        project_path: &str,
        filename: &str,
        bytes: &[u8],
    ) -> Result<ConversationAttachment> {
        let store = self.store().clone();
        // Normalized without registering: nothing touches the registry or the
        // filesystem until the project and conversation are admitted below.
        let project_path = store.registry().project_paths(project_path)?.project_path;
        let id = format!("att-{:032x}", rand::thread_rng().gen::<u128>());
        let dir = self
            .attachments_dir(conversation_id, &project_path)?
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
        let blob = dir.join(&record.stored_name);
        let blob_path = store.home_relative(&blob);
        let gate = store.project_gate(&project_path);
        let _gate = gate.read().unwrap_or_else(|poison| poison.into_inner());
        store.admit_project(&project_path)?;
        store.with_conversation(conversation_id, |slot| {
            let mut loaded = match slot.as_ref() {
                Some(loaded) if loaded.record.meta.project_path != project_path => {
                    return Err(StorageError::ConversationCommitRejected {
                        conversation_id: conversation_id.to_string(),
                        reason: "Conversation is already bound to a different project path."
                            .to_string(),
                    })
                }
                Some(loaded) => loaded.clone(),
                None => {
                    if store
                        .database()
                        .read(|read| read.conversation_location(conversation_id))?
                        .is_some()
                    {
                        return Err(StorageError::ConversationCommitRejected {
                            conversation_id: conversation_id.to_string(),
                            reason: "conversation does not exist or was deleted".to_string(),
                        });
                    }
                    let mut loaded = LoadedConversation::new(conversation_id, &project_path);
                    let now = super::commit::iso_now();
                    loaded.record.meta.created_at = now.clone();
                    loaded.record.meta.updated_at = now;
                    loaded.record.meta.conversation_handle =
                        crate::workspace_conversations::free_handle(
                            &store,
                            conversation_id,
                            store.reserved_handle(conversation_id).as_deref(),
                        )?;
                    loaded
                }
            };
            let is_new = slot.is_none();
            // Admitted: register a new conversation's project, then publish the
            // blob before the transaction that references it, all under the
            // project gate and conversation lock.
            if is_new {
                store.registry().ensure_project_paths(&project_path)?;
            }
            write_atomic(&blob, bytes)?;
            let meta = &loaded.record.meta;
            let payload =
                serde_json::to_value(&record).map_err(|source| StorageError::JsonWrite {
                    path: blob.clone(),
                    source,
                })?;
            let conversation_row = is_new.then(|| super::database::ConversationRow {
                id: conversation_id.to_string(),
                project_path: project_path.clone(),
                handle: meta.conversation_handle.clone(),
                created_at: meta.created_at.clone(),
                updated_at: meta.updated_at.clone(),
                title: meta.title.clone(),
                preview: String::new(),
                running: false,
                needs_attention: false,
                launched_run_ids: Vec::new(),
                metadata: serde_json::to_value(meta).unwrap_or_default(),
            });
            store.database().write(|tx| {
                if let Some(row) = &conversation_row {
                    tx.put_conversation(row)?;
                }
                tx.put_attachment(&AttachmentRow {
                    conversation_id: conversation_id.to_string(),
                    id: record.id.clone(),
                    blob_path: blob_path.clone(),
                    status: "draft".to_string(),
                    updated_at: record.created_at.clone(),
                    payload: payload.clone(),
                })?;
                tx.put_blob_reference(conversation_id, "attachment", &record.id, &blob_path)
            })?;
            loaded.attachments.insert(
                record.id.clone(),
                StoredAttachment {
                    record: record.clone(),
                    blob_path: blob_path.clone(),
                    attached: false,
                },
            );
            if is_new {
                store.release_handle(conversation_id);
            }
            *slot = Some(loaded);
            Ok(())
        })?;
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
        let project_path = self
            .store()
            .registry()
            .project_paths(project_path)?
            .project_path;
        let home_dir = self.store().home_dir().to_path_buf();
        self.store().with_conversation(conversation_id, |slot| {
            Ok(slot
                .as_ref()
                .filter(|loaded| loaded.record.meta.project_path == project_path)
                .and_then(|loaded| loaded.attachments.get(attachment_id))
                .filter(|attachment| attachment.readable(&home_dir))
                .map(|attachment| (attachment.record.clone(), attachment.path(&home_dir))))
        })
    }

    /// An attachment's record and bytes, read under the project gate and the
    /// conversation lock, so cleanup cannot remove the bytes mid-download.
    /// `None` when the attachment is unknown or its bytes are unreadable.
    pub fn read_attachment_bytes(
        &self,
        conversation_id: &str,
        project_path: &str,
        attachment_id: &str,
    ) -> Result<Option<(ConversationAttachment, Vec<u8>)>> {
        let store = self.store();
        let project_path = store.registry().project_paths(project_path)?.project_path;
        let home_dir = store.home_dir().to_path_buf();
        let gate = store.project_gate(&project_path);
        let _gate = gate.read().unwrap_or_else(|poison| poison.into_inner());
        store.with_conversation(conversation_id, |slot| {
            let Some(attachment) = slot
                .as_ref()
                .filter(|loaded| loaded.record.meta.project_path == project_path)
                .and_then(|loaded| loaded.attachments.get(attachment_id))
                .filter(|attachment| attachment.readable(&home_dir))
            else {
                return Ok(None);
            };
            let path = attachment.path(&home_dir);
            let bytes = fs::read(&path)
                .map_err(|source| StorageError::io("read attachment", &path, source))?;
            Ok(Some((attachment.record.clone(), bytes)))
        })
    }

    pub fn list_attachments(
        &self,
        conversation_id: &str,
        project_path: &str,
    ) -> Result<Vec<ConversationAttachment>> {
        let project_path = self
            .store()
            .registry()
            .project_paths(project_path)?
            .project_path;
        self.store().with_conversation(conversation_id, |slot| {
            Ok(slot
                .as_ref()
                .filter(|loaded| loaded.record.meta.project_path == project_path)
                .map(|loaded| {
                    loaded
                        .attachments
                        .values()
                        .map(|attachment| attachment.record.clone())
                        .collect()
                })
                .unwrap_or_default())
        })
    }

    /// Delete one attachment no turn references. Returns whether it existed;
    /// a referenced attachment is a `ConversationCommitRejected` error.
    pub fn delete_unreferenced_attachment(
        &self,
        conversation_id: &str,
        project_path: &str,
        attachment_id: &str,
    ) -> Result<bool> {
        let removed = self.remove_draft_attachments(conversation_id, project_path, |id, attachment| {
            if id != attachment_id {
                return Ok(false);
            }
            if attachment.attached {
                return Err(StorageError::ConversationCommitRejected {
                    conversation_id: conversation_id.to_string(),
                    reason: "This attachment belongs to a sent message and stays until the conversation is deleted.".to_string(),
                });
            }
            Ok(true)
        }, None)?;
        Ok(!removed.is_empty())
    }

    /// Delete unsent uploads older than `cutoff`, and leftover blobs of
    /// interrupted or earlier-deleted uploads no row references. Returns the
    /// deleted ids.
    pub fn prune_attachments(
        &self,
        conversation_id: &str,
        project_path: &str,
        cutoff: OffsetDateTime,
    ) -> Result<Vec<String>> {
        self.remove_draft_attachments(
            conversation_id,
            project_path,
            |_, attachment| {
                Ok(!attachment.attached
                    && OffsetDateTime::parse(&attachment.record.created_at, &Rfc3339)
                        .is_ok_and(|created_at| created_at < cutoff))
            },
            Some(cutoff),
        )
    }

    /// Tombstone the matching draft rows and their blob references, then,
    /// behind the cleanup barrier and a recheck of live references, remove
    /// their bytes; with `orphans_before`, also unreferenced attachment
    /// directories older than it. Runs under the project gate and
    /// conversation lock so no commit can start referencing them meanwhile.
    fn remove_draft_attachments(
        &self,
        conversation_id: &str,
        project_path: &str,
        mut select: impl FnMut(&str, &StoredAttachment) -> Result<bool>,
        orphans_before: Option<OffsetDateTime>,
    ) -> Result<Vec<String>> {
        let store = self.store().clone();
        let project_path = store.registry().project_paths(project_path)?.project_path;
        let dir = self.attachments_dir(conversation_id, &project_path)?;
        let gate = store.project_gate(&project_path);
        let _gate = gate.read().unwrap_or_else(|poison| poison.into_inner());
        store.with_conversation(conversation_id, |slot| {
            let Some(loaded) = slot
                .as_mut()
                .filter(|loaded| loaded.record.meta.project_path == project_path)
            else {
                return Ok(Vec::new());
            };
            let mut selected = Vec::new();
            for (id, attachment) in &loaded.attachments {
                if select(id, attachment)? {
                    selected.push(id.clone());
                }
            }
            if !selected.is_empty() {
                store.database().write(|tx| {
                    for id in &selected {
                        let attachment = &loaded.attachments[id];
                        tx.delete_attachment(conversation_id, id)?;
                        tx.delete_blob_reference(
                            conversation_id,
                            "attachment",
                            id,
                            &attachment.blob_path,
                        )?;
                    }
                    Ok(())
                })?;
                for id in &selected {
                    loaded.attachments.remove(id);
                }
            }
            let mut candidates: Vec<PathBuf> = selected.iter().map(|id| dir.join(id)).collect();
            if let Some(cutoff) = orphans_before {
                for entry in read_dir_or_empty(&dir)? {
                    let name = entry
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("");
                    let old = fs::metadata(&entry)
                        .and_then(|metadata| metadata.modified())
                        .is_ok_and(|modified| OffsetDateTime::from(modified) < cutoff);
                    if old && !loaded.attachments.contains_key(name) && !candidates.contains(&entry)
                    {
                        candidates.push(entry);
                    }
                }
            }
            if candidates.is_empty() {
                return Ok(selected);
            }
            let cleanup = store.database().after_checkpoint(|view| {
                for candidate in &candidates {
                    if !view.has_live_reference_under(&store.home_relative(candidate))? {
                        remove_dir_if_exists(candidate, "delete attachment")?;
                    }
                }
                Ok(())
            });
            // The rows are gone either way; unremoved bytes are retried garbage.
            if let Err(error) = cleanup {
                eprintln!("Deferred attachment cleanup in {}: {error}", dir.display());
            }
            Ok(selected
                .into_iter()
                .chain(candidates.iter().filter_map(|candidate| {
                    candidate
                        .file_name()
                        .and_then(|name| name.to_str())
                        .map(str::to_string)
                }))
                .fold(Vec::new(), |mut ids, id| {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                    ids
                }))
        })
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
    } else if let Some(brand) = iso_media_image_brand(bytes) {
        brand
    } else if bytes.starts_with(b"%PDF-") {
        ("application/pdf", "pdf")
    } else if std::str::from_utf8(bytes).is_ok_and(is_svg) {
        ("image/svg+xml", "svg")
    } else if std::str::from_utf8(bytes).is_ok() {
        ("text/plain", "txt")
    } else {
        ("application/octet-stream", "bin")
    }
}

/// AVIF or HEIF images, from the brands in a leading ISO media `ftyp` box.
fn iso_media_image_brand(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.get(4..8) != Some(b"ftyp".as_slice()) {
        return None;
    }
    let box_size = u32::from_be_bytes(bytes.get(..4)?.try_into().ok()?) as usize;
    let brands = bytes.get(8..box_size.min(bytes.len()))?;
    // Major brand, minor version, then compatible brands.
    let brands = brands
        .chunks_exact(4)
        .enumerate()
        .filter(|(index, _)| *index != 1)
        .map(|(_, brand)| brand)
        .collect::<Vec<_>>();
    if brands
        .iter()
        .any(|brand| matches!(*brand, b"avif" | b"avis"))
    {
        Some(("image/avif", "avif"))
    } else if brands.iter().any(|brand| {
        matches!(
            *brand,
            b"heic" | b"heix" | b"heim" | b"heis" | b"mif1" | b"msf1"
        )
    }) {
        Some(("image/heic", "heic"))
    } else {
        None
    }
}

/// Whether text is an SVG document: its first element, after any XML
/// declaration, comments or doctype, is `<svg`.
fn is_svg(text: &str) -> bool {
    let mut rest = text.trim_start_matches('\u{feff}');
    loop {
        rest = rest.trim_start();
        let close = if rest.starts_with("<?") {
            "?>"
        } else if rest.starts_with("<!--") {
            "-->"
        } else if rest.starts_with("<!") {
            ">"
        } else {
            break;
        };
        match rest.find(close) {
            Some(end) => rest = &rest[end + close.len()..],
            None => return false,
        }
    }
    rest.get(..4)
        .is_some_and(|tag| tag.eq_ignore_ascii_case("<svg"))
        && rest[4..].starts_with(|ch: char| ch.is_whitespace() || ch == '>' || ch == '/')
}
