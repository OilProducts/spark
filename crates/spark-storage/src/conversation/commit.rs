//! The private commit machinery behind the typed conversation operations.
//!
//! An operation runs under its project's lifecycle gate in one immediate
//! SQLite transaction: it reads the current rows it needs through the
//! transaction, validates its facts against them, and writes changed
//! entities with their references and the conversation's summary. A failure
//! rolls everything back; an operation that changes nothing allocates no
//! revision. The commit notice is published before the database is released,
//! so notices follow commit order. Nothing is kept between transactions.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};
use spark_common::segments::SegmentStore;
use time::OffsetDateTime;

use crate::error::{Result, StorageError};
use crate::ConversationRepository;

use super::attachments::{turn_attachment_ids, ConversationAttachment};
use super::database::{
    ArtifactRow, AttachmentRow, ChangeBatch, ConversationRow, SegmentRow, TurnRow, WriteTransaction,
};
use super::records::{ConversationMeta, TranscriptSegment, TranscriptTurn};
use super::store::{CommitNotice, ConversationStore, ALL_COLLECTIONS, EVENT_LOG_COLLECTION};
use super::tool_output::{externalize_conversation_segment_tool_output, safe_tool_output_artifact};

/// The result of one committed conversation operation: its revision and the
/// rows it changed, never the conversation's full state.
#[derive(Debug, Clone, PartialEq)]
pub struct ConversationCommit {
    /// The transaction's home-global revision, now the conversation's.
    pub revision: i64,
    /// Every row this commit changed, at `revision`.
    pub changes: ChangeBatch,
}

/// Whether an operation may create the conversation it targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Open {
    Existing,
    Create,
}

/// What a finished operation committed, for its notice.
pub(crate) struct Finished {
    rows: Vec<super::EntityRow>,
    closed: Vec<(String, String)>,
    settled: Vec<String>,
    created: bool,
}

impl ConversationRepository {
    /// Run one operation on one conversation. `project_path` names the
    /// project the conversation must belong to; without it the
    /// conversation's own project is used and it must exist.
    pub(crate) fn operate<T>(
        &self,
        conversation_id: &str,
        project_path: Option<&str>,
        open: Open,
        apply: impl FnOnce(&mut Op<'_>) -> Result<T>,
    ) -> Result<(Option<ConversationCommit>, T)> {
        let store = self.store().clone();
        let project_path = match project_path.map(str::trim).filter(|path| !path.is_empty()) {
            Some(project_path) => store.registry().project_paths(project_path)?.project_path,
            None => match self.conversation_project_path(conversation_id)? {
                Some(project_path) => project_path,
                None => return Err(missing(conversation_id)),
            },
        };
        let gate = store.project_gate(&project_path);
        let _gate = gate.read().unwrap_or_else(|poison| poison.into_inner());
        store.admit_project(&project_path)?;
        let root = store.conversation_root(&project_path, conversation_id)?;
        let (revision, (value, finished)) = store.database().write_if_changed(
            |tx| {
                let mut op = Op::open(tx, &store, conversation_id, &project_path, root, open)?;
                let value = apply(&mut op)?;
                let finished = op.finish()?;
                Ok((value, finished))
            },
            |revision, (_, finished)| {
                let Some(finished) = finished else {
                    return;
                };
                if finished.created {
                    store.release_handle(conversation_id);
                }
                store.notify_commit(CommitNotice {
                    revision,
                    conversations: vec![(conversation_id.to_string(), project_path.clone())],
                    closed_segments: finished
                        .closed
                        .iter()
                        .map(|(turn_id, segment_id)| {
                            (
                                conversation_id.to_string(),
                                turn_id.clone(),
                                segment_id.clone(),
                            )
                        })
                        .collect(),
                    settled_executions: finished
                        .settled
                        .iter()
                        .map(|turn_id| (conversation_id.to_string(), turn_id.clone()))
                        .collect(),
                });
            },
        )?;
        let commit = match (revision, finished) {
            (Some(revision), Some(finished)) => Some(ConversationCommit {
                revision,
                changes: ChangeBatch {
                    from_revision: revision - 1,
                    watermark: revision,
                    rows: finished.rows,
                },
            }),
            _ => None,
        };
        Ok((commit, value))
    }
}

impl ConversationRepository {
    /// Run `apply` on a live conversation's inbox rows under its project's
    /// lifecycle gate, in one transaction that allocates no revision.
    pub(crate) fn write_inbox<T>(
        &self,
        conversation_id: &str,
        apply: impl FnOnce(&Connection) -> Result<T>,
    ) -> Result<T> {
        let store = self.store().clone();
        let Some(project_path) = self.conversation_project_path(conversation_id)? else {
            return Err(missing(conversation_id));
        };
        let gate = store.project_gate(&project_path);
        let _gate = gate.read().unwrap_or_else(|poison| poison.into_inner());
        store.admit_project(&project_path)?;
        store.database().write_inbox(conversation_id, apply)
    }
}

/// One operation's view of a conversation inside its transaction. Reads see
/// every earlier write of the same transaction.
pub(crate) struct Op<'a> {
    tx: &'a WriteTransaction<'a>,
    store: &'a ConversationStore,
    pub(crate) conversation_id: String,
    pub(crate) root: PathBuf,
    /// The conversation's metadata; changes are written when it finishes.
    pub(crate) meta: ConversationMeta,
    stored_meta: ConversationMeta,
    /// The operation's one timestamp.
    pub(crate) now: String,
    settings_changed: bool,
    touched: bool,
    created: bool,
    changes_before: u64,
    closed: Vec<(String, String)>,
    settled: Vec<String>,
}

impl<'a> Op<'a> {
    fn open(
        tx: &'a WriteTransaction<'a>,
        store: &'a ConversationStore,
        conversation_id: &str,
        project_path: &str,
        root: PathBuf,
        open: Open,
    ) -> Result<Self> {
        let connection = tx.connection();
        let changes_before = connection.total_changes();
        let now = iso_now();
        let row: Option<(String, String, String, bool)> = connection
            .query_row(
                "SELECT project_path, handle, metadata, deleted FROM conversations WHERE id = ?1",
                [conversation_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let (meta, created) = match row {
            Some((_, _, _, true)) => return Err(missing(conversation_id)),
            Some((stored_project, _, _, false)) if stored_project != project_path => {
                return Err(commit_rejected(
                    conversation_id,
                    "Conversation is already bound to a different project path.",
                ))
            }
            Some((_, handle, metadata, false)) => {
                let mut meta = parse_meta(conversation_id, &metadata)?;
                meta.conversation_id = conversation_id.to_string();
                meta.project_path = project_path.to_string();
                meta.conversation_handle = handle;
                (meta, false)
            }
            None if open == Open::Existing => return Err(missing(conversation_id)),
            None => {
                store.registry().ensure_project_paths(project_path)?;
                let mut meta = ConversationMeta::new(conversation_id, project_path);
                meta.created_at = now.clone();
                meta.updated_at = now.clone();
                meta.conversation_handle = crate::workspace_conversations::free_handle(
                    connection,
                    store.home_dir(),
                    conversation_id,
                    store.reserved_handle(conversation_id).as_deref(),
                )?;
                meta.revision = tx.revision();
                // Children reference the row; its summary is written on finish.
                tx.put_conversation(&ConversationRow {
                    id: conversation_id.to_string(),
                    project_path: project_path.to_string(),
                    handle: meta.conversation_handle.clone(),
                    created_at: now.clone(),
                    updated_at: now.clone(),
                    title: meta.title.clone(),
                    preview: String::new(),
                    running: false,
                    needs_attention: false,
                    launched_run_ids: Vec::new(),
                    metadata: to_object(&meta)?,
                })?;
                (meta, true)
            }
        };
        Ok(Self {
            tx,
            store,
            conversation_id: conversation_id.to_string(),
            root,
            stored_meta: meta.clone(),
            meta,
            now,
            settings_changed: false,
            touched: false,
            created,
            changes_before,
            closed: Vec::new(),
            settled: Vec::new(),
        })
    }

    pub(crate) fn connection(&self) -> &Connection {
        self.tx.connection()
    }

    pub(crate) fn home_dir(&self) -> &Path {
        self.store.home_dir()
    }

    /// Settings changes advance the settings revision when they commit.
    pub(crate) fn settings_changed(&mut self) {
        self.settings_changed = true;
    }

    /// Record the conversation's runtime session for a provider.
    pub(crate) fn put_runtime_session(&mut self, provider: &str, payload: &Value) -> Result<()> {
        self.tx
            .put_runtime_session(&self.conversation_id, provider, payload)
    }

    /// Commit the conversation even when nothing else changes.
    pub(crate) fn touch(&mut self) {
        self.touched = true;
    }

    fn finish(mut self) -> Result<Option<Finished>> {
        let rows_changed = self.connection().total_changes() != self.changes_before;
        if !rows_changed && !self.touched && self.meta == self.stored_meta {
            return Ok(None);
        }
        let revision = self.tx.revision();
        if self.meta.created_at.trim().is_empty() {
            self.meta.created_at = self.now.clone();
        }
        self.meta.updated_at = self.now.clone();
        let summary = summary(self.connection(), &self.conversation_id)?;
        let title = self.meta.title.trim();
        if title.is_empty() || title == "New thread" {
            self.meta.title = derived_title(self.connection(), &self.conversation_id)?;
        }
        if self.settings_changed {
            self.meta.settings_revision = revision;
        }
        self.meta.revision = revision;
        let meta = &self.meta;
        self.tx.put_conversation(&ConversationRow {
            id: self.conversation_id.clone(),
            project_path: meta.project_path.clone(),
            handle: meta.conversation_handle.clone(),
            created_at: meta.created_at.clone(),
            updated_at: meta.updated_at.clone(),
            title: meta.title.clone(),
            preview: summary.preview,
            running: summary.running,
            needs_attention: summary.needs_attention,
            launched_run_ids: summary.launched_run_ids,
            metadata: to_object(meta)?,
        })?;
        Ok(Some(Finished {
            rows: self.tx.changed_rows(&self.conversation_id)?,
            closed: self.closed,
            settled: self.settled,
            created: self.created,
        }))
    }

    // ---- Reads -------------------------------------------------------------

    pub(crate) fn turn(&self, turn_id: &str) -> Result<Option<Value>> {
        payload(
            self.connection(),
            "SELECT payload FROM turns WHERE conversation_id = ?1 AND turn_id = ?2 AND deleted = 0",
            params![self.conversation_id, turn_id],
        )
    }

    /// Whether the execution an assistant turn owns is settled; `None` for
    /// an unknown turn.
    pub(crate) fn turn_settled(&self, turn_id: &str) -> Result<Option<bool>> {
        Ok(self
            .connection()
            .query_row(
                "SELECT settled FROM turns WHERE conversation_id = ?1 AND turn_id = ?2 \
                 AND deleted = 0",
                params![self.conversation_id, turn_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub(crate) fn segment(&self, segment_id: &str) -> Result<Option<Value>> {
        payload(
            self.connection(),
            "SELECT payload FROM segments WHERE conversation_id = ?1 AND segment_id = ?2 \
             AND deleted = 0",
            params![self.conversation_id, segment_id],
        )
    }

    /// A turn's segments in order, optionally of one kind.
    pub(crate) fn turn_segments(&self, turn_id: &str, kind: Option<&str>) -> Result<Vec<Value>> {
        payloads(
            self.connection(),
            "SELECT payload FROM segments WHERE conversation_id = ?1 AND turn_id = ?2 \
             AND deleted = 0 AND (?3 IS NULL OR kind = ?3) ORDER BY sort_order, segment_id",
            params![self.conversation_id, turn_id, kind],
        )
    }

    /// Turns in one of `statuses`, in transcript order.
    pub(crate) fn turns_with_status(&self, statuses: &[&str]) -> Result<Vec<Value>> {
        let mut turns = Vec::new();
        for status in statuses {
            turns.extend(
                payloads(
                    self.connection(),
                    "SELECT payload FROM turns WHERE conversation_id = ?1 AND status = ?2 \
                     AND deleted = 0 ORDER BY position, turn_id",
                    params![self.conversation_id, status],
                )?
                .into_iter(),
            );
        }
        Ok(turns)
    }

    /// The newest pending or streaming assistant turn.
    pub(crate) fn active_assistant_turn(&self) -> Result<Option<Value>> {
        payload(
            self.connection(),
            "SELECT payload FROM turns WHERE conversation_id = ?1 AND role = 'assistant' \
             AND status IN ('pending', 'streaming') AND deleted = 0 \
             ORDER BY position DESC LIMIT 1",
            params![self.conversation_id],
        )
    }

    pub(crate) fn artifact(&self, collection: &str, artifact_id: &str) -> Result<Option<Value>> {
        payload(
            self.connection(),
            "SELECT payload FROM artifacts WHERE conversation_id = ?1 AND collection = ?2 \
             AND artifact_id = ?3 AND deleted = 0",
            params![self.conversation_id, collection, artifact_id],
        )
    }

    pub(crate) fn artifacts(&self, collection: &str) -> Result<Vec<Value>> {
        payloads(
            self.connection(),
            "SELECT payload FROM artifacts WHERE conversation_id = ?1 AND collection = ?2 \
             AND deleted = 0 ORDER BY position",
            params![self.conversation_id, collection],
        )
    }

    /// One past the highest order among a turn's segments, starting at 1.
    pub(crate) fn next_segment_order(&self, turn_id: &str) -> Result<i64> {
        Ok(self.connection().query_row(
            "SELECT coalesce(max(sort_order), 0) + 1 FROM segments \
             WHERE conversation_id = ?1 AND turn_id = ?2 AND deleted = 0",
            params![self.conversation_id, turn_id],
            |row| row.get(0),
        )?)
    }

    // ---- Writes ------------------------------------------------------------

    /// Write a turn. A new turn's attachments must be stored and readable and
    /// become attached; an accepted message's acceptance never changes; a
    /// queued turn moves behind every other turn whenever it is written.
    pub(crate) fn put_turn(&mut self, turn: Value) -> Result<()> {
        let mut turn: TranscriptTurn = parse(&self.conversation_id, "turn", turn)?;
        let existing: Option<(i64, bool, Value)> = self
            .connection()
            .query_row(
                "SELECT position, settled, payload FROM turns WHERE conversation_id = ?1 \
                 AND turn_id = ?2 AND deleted = 0",
                params![self.conversation_id, turn.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get::<_, String>(2)?)),
            )
            .optional()?
            .map(|(position, settled, payload)| {
                Ok::<_, StorageError>((position, settled, parse_json(&payload)?))
            })
            .transpose()?;
        keep_acceptance(
            &mut turn.extra,
            existing.as_ref().and_then(|(_, _, old)| old.as_object()),
        );
        if existing.is_none() {
            self.attach(turn.extra.get("attachments"))?;
        }
        let payload = to_object(&turn)?;
        let position = match &existing {
            Some((position, _, old)) if old["status"] != "queued" => *position,
            Some((position, _, old)) if *old == payload => {
                // Rewritten unchanged: a queued turn still moves to the end.
                let last: i64 = self.connection().query_row(
                    "SELECT coalesce(max(position), -1) FROM turns WHERE conversation_id = ?1",
                    [&self.conversation_id],
                    |row| row.get(0),
                )?;
                if *position == last {
                    return Ok(());
                }
                last + 1
            }
            _ => self.connection().query_row(
                "SELECT coalesce(max(position), -1) + 1 FROM turns WHERE conversation_id = ?1",
                [&self.conversation_id],
                |row| row.get(0),
            )?,
        };
        if existing
            .as_ref()
            .is_some_and(|(at, _, old)| *at == position && *old == payload)
        {
            return Ok(());
        }
        self.tx.put_turn(&TurnRow {
            conversation_id: self.conversation_id.clone(),
            id: turn.id.clone(),
            role: turn.role.clone(),
            kind: turn.turn_kind().to_string(),
            position,
            settled: existing.is_some_and(|(_, settled, _)| settled),
            status: turn.status.clone(),
            updated_at: self.now.clone(),
            payload,
        })
    }

    /// Mark the execution an assistant turn owns settled, once.
    pub(crate) fn settle(&mut self, turn_id: &str) -> Result<()> {
        let changed = self.connection().execute(
            "UPDATE turns SET settled = 1, revision = ?1 WHERE conversation_id = ?2 \
             AND turn_id = ?3 AND deleted = 0 AND settled = 0",
            params![self.tx.revision(), self.conversation_id, turn_id],
        )?;
        if changed > 0 {
            self.tx.touch(&self.conversation_id)?;
            self.settled.push(turn_id.to_string());
        }
        Ok(())
    }

    /// Write a segment and its tool-output reference. An unassigned order is
    /// allocated at its first insert and kept after; a full tool output
    /// moves to a new immutable blob. Returns the stored payload.
    pub(crate) fn put_segment(&mut self, segment: Value) -> Result<Value> {
        let mut segment: TranscriptSegment = parse(&self.conversation_id, "segment", segment)?;
        if self.turn(&segment.turn_id)?.is_none() {
            return Err(commit_rejected(
                &self.conversation_id,
                &format!(
                    "Segment {} targets unknown turn {}.",
                    segment.id, segment.turn_id
                ),
            ));
        }
        let existing: Option<TranscriptSegment> = self
            .segment(&segment.id)?
            .map(|old| parse(&self.conversation_id, "segment", old))
            .transpose()?;
        if segment.has_unassigned_order() {
            segment.order = match &existing {
                Some(existing) => existing.order,
                None => self.next_segment_order(&segment.turn_id)?,
            };
        }
        if existing.is_none() {
            // A message accepted into a running turn owns its uploads.
            self.attach(segment.extra.get("attachments"))?;
        }
        keep_acceptance(
            &mut segment.extra,
            existing.as_ref().map(|existing| &existing.extra),
        );
        let previous_output = existing.as_ref().and_then(tool_output_artifact);
        // An echoed preview keeps referencing the stored output.
        if let (Some(previous), Some(tool_call)) = (
            previous_output.as_ref(),
            segment.tool_call.as_mut().and_then(Value::as_object_mut),
        ) {
            if tool_call.get("output_truncated") == Some(&json!(true))
                && !tool_call.contains_key("output_artifact")
            {
                tool_call.insert("output_artifact".into(), json!(previous));
            }
        }
        externalize_conversation_segment_tool_output(&self.root, &mut segment)?;
        let payload = to_object(&segment)?;
        let existing_payload = existing.as_ref().map(to_object).transpose()?;
        if existing_payload.as_ref() == Some(&payload) {
            return Ok(payload);
        }
        self.tx.put_segment(&SegmentRow {
            conversation_id: self.conversation_id.clone(),
            id: segment.id.clone(),
            turn_id: segment.turn_id.clone(),
            kind: segment.kind.clone(),
            order: segment.order,
            status: segment.status.clone(),
            updated_at: self.now.clone(),
            payload: payload.clone(),
        })?;
        let current = tool_output_artifact(&segment);
        if let Some(previous) = previous_output
            .as_ref()
            .filter(|previous| Some(*previous) != current.as_ref())
        {
            self.tx.delete_blob_reference(
                &self.conversation_id,
                "tool_output",
                &segment.id,
                &self.store.home_relative(&self.root.join(previous)),
            )?;
        }
        if let Some(current) = current {
            self.tx.put_blob_reference(
                &self.conversation_id,
                "tool_output",
                &segment.id,
                &self.store.home_relative(&self.root.join(current)),
            )?;
        }
        // The completion timestamp closes a segment's content for good.
        if segment.completed_at.is_some()
            && existing
                .as_ref()
                .is_none_or(|existing| existing.completed_at.is_none())
        {
            self.closed
                .push((segment.turn_id.clone(), segment.id.clone()));
        }
        // The artifact this segment anchors may now have an owner.
        if let Some(artifact_id) = segment.artifact_id.clone() {
            for collection in ALL_COLLECTIONS {
                let unowned: bool = self
                    .connection()
                    .query_row(
                        "SELECT turn_id IS NULL FROM artifacts WHERE conversation_id = ?1 \
                         AND collection = ?2 AND artifact_id = ?3 AND deleted = 0",
                        params![self.conversation_id, collection.key(), artifact_id],
                        |row| row.get(0),
                    )
                    .optional()?
                    .unwrap_or(false);
                if unowned {
                    let artifact = self
                        .artifact(collection.key(), &artifact_id)?
                        .expect("artifact read above");
                    self.write_artifact(collection.key(), &artifact_id, artifact)?;
                }
            }
        }
        Ok(payload)
    }

    /// Tombstone a segment and the tool-output reference it owns.
    pub(crate) fn delete_segment(&mut self, segment_id: &str) -> Result<()> {
        if let Some(segment) = self.segment(segment_id)? {
            if segment.get("completed_at").is_none() {
                if let Some(turn_id) = segment["turn_id"].as_str() {
                    self.closed
                        .push((turn_id.to_string(), segment_id.to_string()));
                }
            }
        }
        self.tx.delete_segment(&self.conversation_id, segment_id)?;
        Ok(())
    }

    /// Write an artifact of a known collection; a missing id is assigned.
    /// Returns its id.
    pub(crate) fn put_artifact(&mut self, collection: &str, mut artifact: Value) -> Result<String> {
        if !artifact.is_object() {
            return Err(commit_rejected(
                &self.conversation_id,
                "Artifact records must be JSON objects.",
            ));
        }
        if !ALL_COLLECTIONS
            .iter()
            .any(|known| known.key() == collection)
        {
            return Err(commit_rejected(
                &self.conversation_id,
                &format!("Unknown artifact collection {collection}"),
            ));
        }
        let id = match artifact.get("id").and_then(Value::as_str) {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => {
                let id = format!("artifact-{}", uuid::Uuid::new_v4().simple());
                artifact["id"] = json!(id);
                id
            }
        };
        if self.artifact(collection, &id)?.as_ref() == Some(&artifact) {
            return Ok(id);
        }
        self.write_artifact(collection, &id, artifact)?;
        Ok(id)
    }

    /// Append a workflow event to the conversation's event log.
    pub(crate) fn append_event(&mut self, event: Value) -> Result<()> {
        self.tx.put_artifact(&ArtifactRow {
            conversation_id: self.conversation_id.clone(),
            collection: EVENT_LOG_COLLECTION.to_string(),
            id: format!("event-{}", uuid::Uuid::new_v4().simple()),
            turn_id: None,
            status: event["status"].as_str().unwrap_or_default().to_string(),
            updated_at: self.now.clone(),
            payload: event,
        })
    }

    /// Write an artifact row, resolving its owning turn: its source turn, or
    /// the turn of the segment that anchors it.
    fn write_artifact(&mut self, collection: &str, id: &str, artifact: Value) -> Result<()> {
        let owner: Option<String> = match artifact.get("source_turn_id").and_then(Value::as_str) {
            Some(turn_id) => Some(turn_id.to_string()),
            None => self
                .connection()
                .query_row(
                    "SELECT turn_id FROM segments WHERE conversation_id = ?1 AND deleted = 0 \
                     AND json_extract(payload, '$.artifact_id') = ?2 LIMIT 1",
                    params![self.conversation_id, id],
                    |row| row.get(0),
                )
                .optional()?,
        };
        let owner = match owner {
            Some(turn_id) if self.turn(&turn_id)?.is_some() => Some(turn_id),
            _ => None,
        };
        self.tx.put_artifact(&ArtifactRow {
            conversation_id: self.conversation_id.clone(),
            collection: collection.to_string(),
            id: id.to_string(),
            turn_id: owner,
            status: artifact["status"].as_str().unwrap_or_default().to_string(),
            updated_at: self.now.clone(),
            payload: artifact,
        })
    }

    /// Mark the attachments a new message references attached; each must
    /// be stored in this conversation and readable now.
    fn attach(&mut self, references: Option<&Value>) -> Result<()> {
        for id in turn_attachment_ids(references) {
            let row: Option<(String, String, String)> = self
                .connection()
                .query_row(
                    "SELECT blob_path, status, payload FROM attachments \
                     WHERE conversation_id = ?1 AND attachment_id = ?2 AND deleted = 0",
                    params![self.conversation_id, id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            let readable = row.as_ref().and_then(|(blob_path, _, payload)| {
                let record: ConversationAttachment = serde_json::from_str(payload).ok()?;
                readable(self.home_dir(), blob_path, record.size_bytes).then_some(record)
            });
            let (Some((blob_path, status, _)), Some(record)) = (row, readable) else {
                return Err(commit_rejected(
                    &self.conversation_id,
                    &format!("Attachment {id} is not available in this conversation. Remove it and attach the file again."),
                ));
            };
            if status != "attached" {
                self.tx.put_attachment(&AttachmentRow {
                    conversation_id: self.conversation_id.clone(),
                    id: id.clone(),
                    blob_path,
                    status: "attached".to_string(),
                    updated_at: self.now.clone(),
                    payload: to_object(&record)?,
                })?;
            }
        }
        Ok(())
    }

    /// The materialization rules' view of this transaction's segments.
    pub(crate) fn segments_view(&self) -> SegmentView<'_, 'a> {
        SegmentView {
            op: self,
            error: RefCell::new(None),
            written: Vec::new(),
        }
    }
}

/// Segment reads for the shared materialization rules, over current rows.
/// What the rules write is collected for the operation to store.
pub(crate) struct SegmentView<'o, 'a> {
    op: &'o Op<'a>,
    error: RefCell<Option<StorageError>>,
    pub(crate) written: Vec<Value>,
}

impl SegmentView<'_, '_> {
    fn read<T: Default>(&self, read: impl FnOnce(&Op<'_>) -> Result<T>) -> T {
        match read(self.op) {
            Ok(value) => value,
            Err(error) => {
                self.error.borrow_mut().get_or_insert(error);
                T::default()
            }
        }
    }

    /// The first read error, which fails the operation.
    pub(crate) fn finish(self) -> Result<Vec<Value>> {
        match self.error.into_inner() {
            Some(error) => Err(error),
            None => Ok(self.written),
        }
    }
}

impl SegmentStore for SegmentView<'_, '_> {
    fn segment(&self, segment_id: &str) -> Option<Value> {
        self.written
            .iter()
            .rev()
            .find(|segment| segment["id"] == segment_id)
            .cloned()
            .or_else(|| self.read(|op| op.segment(segment_id)))
    }

    fn next_segment_order(&self, turn_id: &str) -> i64 {
        self.read(|op| op.next_segment_order(turn_id)).max(1)
    }

    fn latest_user_message(&self, turn_id: &str) -> Option<Value> {
        self.read(|op| {
            payload(
                op.connection(),
                "SELECT payload FROM segments WHERE conversation_id = ?1 AND turn_id = ?2 \
                 AND kind = 'user_message' AND deleted = 0 ORDER BY sort_order DESC LIMIT 1",
                params![op.conversation_id, turn_id],
            )
        })
    }

    fn item_parts(&self, item_segment_id: &str) -> Vec<Value> {
        let prefix = format!("{item_segment_id}-after-");
        self.read(|op| {
            payloads(
                op.connection(),
                "SELECT payload FROM segments WHERE conversation_id = ?1 AND deleted = 0 \
                 AND (segment_id = ?2 OR substr(segment_id, 1, length(?3)) = ?3)",
                params![op.conversation_id, item_segment_id, prefix],
            )
        })
    }

    fn turn_content(&self, turn_id: &str) -> Option<String> {
        self.read(|op| op.turn(turn_id))
            .and_then(|turn| turn["content"].as_str().map(str::to_string))
    }

    fn put_segment(&mut self, segment: Value) {
        self.written.push(segment);
    }
}

/// A conversation's summary columns, from indexed SQL over its current rows.
pub(crate) struct Summary {
    pub preview: String,
    pub running: bool,
    pub needs_attention: bool,
    pub launched_run_ids: Vec<String>,
}

pub(crate) fn summary(connection: &Connection, conversation_id: &str) -> Result<Summary> {
    let mut preview = String::new();
    {
        // Newest first; the first message with visible text ends the scan.
        let mut statement = connection.prepare_cached(
            "SELECT json_extract(payload, '$.content') FROM turns INDEXED BY turns_page \
             WHERE conversation_id = ?1 AND deleted = 0 AND kind = 'message' \
             ORDER BY position DESC, turn_id DESC",
        )?;
        let mut rows = statement.query([conversation_id])?;
        while let Some(row) = rows.next()? {
            let content: Option<String> = row.get(0)?;
            let text = truncate_text(content.as_deref().unwrap_or(""), 120);
            if !text.is_empty() {
                preview = text;
                break;
            }
        }
    }
    let running = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM turns INDEXED BY turns_status \
         WHERE status IN ('pending', 'streaming') AND conversation_id = ?1 \
         AND role = 'assistant' AND deleted = 0)",
        [conversation_id],
        |row| row.get(0),
    )?;
    let needs_attention = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM artifacts INDEXED BY artifacts_pending \
         WHERE conversation_id = ?1 AND deleted = 0 AND \
         ((status = 'pending' AND collection = 'flow_run_requests') OR \
         (status = 'pending_review' AND collection = 'proposed_plans')))",
        [conversation_id],
        |row| row.get(0),
    )?;
    let ids = connection
        .prepare_cached(
            "SELECT trim(json_extract(payload, '$.run_id')) FROM artifacts \
             WHERE conversation_id = ?1 AND collection = 'flow_launches' AND deleted = 0 \
             ORDER BY position",
        )?
        .query_map([conversation_id], |row| row.get::<_, Option<String>>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Summary {
        preview,
        running,
        needs_attention,
        launched_run_ids: ids
            .into_iter()
            .flatten()
            .filter(|id| !id.is_empty())
            .collect(),
    })
}

/// The title a conversation without a stored one shows: its first user
/// message, shortened.
pub(crate) fn derived_title(connection: &Connection, conversation_id: &str) -> Result<String> {
    let first: Option<Option<String>> = connection
        .query_row(
            "SELECT json_extract(payload, '$.content') FROM turns INDEXED BY turns_kind \
             WHERE conversation_id = ?1 AND kind = 'message' AND role = 'user' AND deleted = 0 \
             ORDER BY position LIMIT 1",
            [conversation_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(first
        .flatten()
        .map(|content| truncate_text(&content, 64))
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| "New thread".to_string()))
}

/// Completed assistant replies, counted up to 2: title generation only asks
/// whether there is exactly one.
pub(crate) fn completed_replies(connection: &Connection, conversation_id: &str) -> Result<i64> {
    Ok(connection.query_row(
        "SELECT count(*) FROM (SELECT 1 FROM turns INDEXED BY turns_kind \
         WHERE conversation_id = ?1 AND kind = 'message' AND role = 'assistant' \
         AND deleted = 0 AND status = 'complete' LIMIT 2)",
        [conversation_id],
        |row| row.get(0),
    )?)
}

/// One row's JSON payload.
pub(crate) fn payload(
    connection: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Option<Value>> {
    let payload: Option<String> = connection
        .prepare_cached(sql)?
        .query_row(params, |row| row.get(0))
        .optional()?;
    payload.map(|payload| parse_json(&payload)).transpose()
}

/// Several rows' JSON payloads, in query order.
pub(crate) fn payloads(
    connection: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<Value>> {
    let mut statement = connection.prepare_cached(sql)?;
    let rows = statement
        .query_map(params, |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.iter().map(|payload| parse_json(payload)).collect()
}

/// Whether an attachment's bytes exist with the recorded size.
pub(crate) fn readable(home_dir: &Path, blob_path: &str, size_bytes: u64) -> bool {
    std::fs::File::open(home_dir.join(blob_path))
        .and_then(|file| file.metadata())
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() == size_bytes)
}

fn parse<T: serde::de::DeserializeOwned>(
    conversation_id: &str,
    entity: &str,
    value: Value,
) -> Result<T> {
    serde_json::from_value(value).map_err(|error| {
        commit_rejected(
            conversation_id,
            &format!("Invalid {entity} record: {error}"),
        )
    })
}

pub(crate) fn parse_json(payload: &str) -> Result<Value> {
    serde_json::from_str(payload).map_err(|source| StorageError::JsonRead {
        path: PathBuf::from(super::CONVERSATION_DATABASE_FILE_NAME),
        source,
    })
}

pub(crate) fn to_object(value: &impl serde::Serialize) -> Result<Value> {
    serde_json::to_value(value).map_err(|source| StorageError::JsonWrite {
        path: PathBuf::from(super::CONVERSATION_DATABASE_FILE_NAME),
        source,
    })
}

/// What an accepted message recorded: the fingerprint of the request its
/// client turn id was accepted for, and the uploads it owns.
const ACCEPTANCE_KEYS: [&str; 2] = ["client_request_sha256", "attachments"];

/// An accepted message's record never changes, even when a later write
/// rebuilds the entity without it.
fn keep_acceptance(extra: &mut Map<String, Value>, existing: Option<&Map<String, Value>>) {
    for key in ACCEPTANCE_KEYS {
        if let Some(accepted) = existing.and_then(|existing| existing.get(key)) {
            extra.insert(key.to_string(), accepted.clone());
        }
    }
}

fn tool_output_artifact(segment: &TranscriptSegment) -> Option<String> {
    segment
        .tool_call
        .as_ref()?
        .get("output_artifact")?
        .as_str()
        .and_then(safe_tool_output_artifact)
        .map(str::to_string)
}

pub(crate) fn truncate_text(value: &str, limit: usize) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= limit {
        return collapsed;
    }
    let mut truncated = collapsed
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>();
    truncated = truncated.trim_end().to_string();
    truncated.push('\u{2026}');
    truncated
}

/// A stored conversation's metadata, refused in a settings version Spark
/// does not read.
pub(crate) fn parse_meta(conversation_id: &str, metadata: &str) -> Result<ConversationMeta> {
    let invalid = |reason: String| StorageError::InvalidConversationState {
        path: PathBuf::from(conversation_id),
        reason,
    };
    let meta: ConversationMeta = serde_json::from_str(metadata)
        .map_err(|error| invalid(format!("Invalid conversation metadata: {error}")))?;
    if meta.settings_schema_version != 1 {
        return Err(invalid(format!(
            "Conversation {conversation_id} has unsupported settings_schema_version {}",
            meta.settings_schema_version
        )));
    }
    Ok(meta)
}

pub(crate) fn commit_rejected(conversation_id: &str, reason: &str) -> StorageError {
    StorageError::ConversationCommitRejected {
        conversation_id: conversation_id.to_string(),
        reason: reason.to_string(),
    }
}

pub(crate) fn missing(conversation_id: &str) -> StorageError {
    commit_rejected(
        conversation_id,
        "conversation does not exist or was deleted",
    )
}

pub(crate) fn iso_now() -> String {
    let now = OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}
