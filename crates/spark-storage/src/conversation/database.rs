//! SQLite conversation store foundation: schema, configuration, revision
//! allocation and revision-indexed reads.
//!
//! One database per Spark home holds every conversation entity as a current
//! row plus tombstones. Each successful write transaction allocates exactly one
//! home-global revision and stamps every row it changes, so `R < revision <= H`
//! selects the replacement rows a client at cursor `R` needs to reach watermark
//! `H`. [`super::ConversationStore`] owns the one connection per Spark home.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use rusqlite::{params, Connection, TransactionBehavior};
use serde::Serialize;
use serde_json::Value;

use crate::error::{Result, StorageError};

/// File name of the conversation database inside the Spark home.
pub const CONVERSATION_DATABASE_FILE_NAME: &str = "conversations.sqlite3";
/// Schema version written to `store_meta`; any other stored version fails open.
pub const CONVERSATION_DATABASE_SCHEMA_VERSION: i64 = 2;
/// Maximum rows returned by one catch-up before a resync is required.
pub const CATCH_UP_MAX_ROWS: usize = 1000;
/// Maximum payload bytes returned by one catch-up before a resync is required.
pub const CATCH_UP_MAX_BYTES: usize = 2 * 1024 * 1024;

const SCHEMA: &str = r#"
CREATE TABLE store_meta (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    schema_version INTEGER NOT NULL,
    generation TEXT NOT NULL CHECK (length(generation) > 0),
    next_revision INTEGER NOT NULL CHECK (next_revision >= 1),
    reset_complete INTEGER NOT NULL DEFAULT 0 CHECK (reset_complete IN (0, 1))
);

CREATE TABLE project_deletions (
    project_path TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    phase TEXT NOT NULL CHECK (phase = 'cleanup_pending'),
    deletion_revision INTEGER NOT NULL CHECK (deletion_revision >= 1),
    response TEXT NOT NULL CHECK (json_valid(response))
);

CREATE TABLE conversations (
    id TEXT PRIMARY KEY CHECK (length(id) > 0),
    project_path TEXT NOT NULL,
    handle TEXT NOT NULL CHECK (length(handle) > 0),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    deleted INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    title TEXT NOT NULL,
    preview TEXT NOT NULL,
    running INTEGER NOT NULL CHECK (running IN (0, 1)),
    needs_attention INTEGER NOT NULL CHECK (needs_attention IN (0, 1)),
    launched_run_ids TEXT NOT NULL CHECK (json_type(launched_run_ids) = 'array'),
    metadata TEXT NOT NULL CHECK (json_type(metadata) = 'object')
);
CREATE UNIQUE INDEX conversations_live_handle ON conversations(handle) WHERE deleted = 0;
CREATE INDEX conversations_project_listing ON conversations(project_path, updated_at, id) WHERE deleted = 0;
CREATE INDEX conversations_attention_listing ON conversations(needs_attention, updated_at, id) WHERE deleted = 0;
CREATE INDEX conversations_revision ON conversations(revision, id);

CREATE TABLE turns (
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    turn_id TEXT NOT NULL CHECK (length(turn_id) > 0),
    role TEXT NOT NULL,
    kind TEXT NOT NULL,
    position INTEGER NOT NULL CHECK (position >= 0),
    order_changed_revision INTEGER NOT NULL DEFAULT 0 CHECK (order_changed_revision >= 0),
    status TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    deleted INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    -- Set once when the execution this assistant turn owns is settled.
    settled INTEGER NOT NULL DEFAULT 0 CHECK (settled IN (0, 1)),
    payload TEXT NOT NULL CHECK (json_type(payload) = 'object'),
    PRIMARY KEY (conversation_id, turn_id)
) WITHOUT ROWID;
CREATE INDEX turns_page ON turns(conversation_id, position, turn_id) WHERE deleted = 0;
CREATE INDEX turns_order_changed ON turns(conversation_id, order_changed_revision);
CREATE INDEX turns_revision ON turns(conversation_id, revision);
CREATE INDEX turns_status ON turns(status, conversation_id) WHERE deleted = 0;
CREATE INDEX turns_kind ON turns(conversation_id, kind, role, position) WHERE deleted = 0;
CREATE INDEX turns_unsettled ON turns(conversation_id) WHERE deleted = 0 AND settled = 0 AND role = 'assistant';

CREATE TABLE segments (
    conversation_id TEXT NOT NULL,
    segment_id TEXT NOT NULL CHECK (length(segment_id) > 0),
    turn_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    sort_order INTEGER NOT NULL,
    status TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    deleted INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    payload TEXT NOT NULL CHECK (json_type(payload) = 'object'),
    PRIMARY KEY (conversation_id, segment_id),
    FOREIGN KEY (conversation_id, turn_id) REFERENCES turns(conversation_id, turn_id)
) WITHOUT ROWID;
CREATE INDEX segments_page ON segments(conversation_id, turn_id, sort_order, segment_id) WHERE deleted = 0;
CREATE INDEX segments_revision ON segments(conversation_id, revision);
CREATE INDEX segments_status ON segments(status, conversation_id) WHERE deleted = 0;
CREATE INDEX segments_kind ON segments(conversation_id, turn_id, kind, sort_order) WHERE deleted = 0;

CREATE TABLE artifacts (
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    collection TEXT NOT NULL CHECK (length(collection) > 0),
    artifact_id TEXT NOT NULL CHECK (length(artifact_id) > 0),
    turn_id TEXT,
    -- Insertion order within the collection, fixed at the first insert.
    position INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    deleted INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    payload TEXT NOT NULL CHECK (json_type(payload) = 'object'),
    PRIMARY KEY (conversation_id, collection, artifact_id),
    FOREIGN KEY (conversation_id, turn_id) REFERENCES turns(conversation_id, turn_id)
) WITHOUT ROWID;
CREATE INDEX artifacts_owner ON artifacts(conversation_id, turn_id, collection, artifact_id) WHERE deleted = 0;
CREATE INDEX artifacts_revision ON artifacts(conversation_id, revision);
CREATE INDEX artifacts_pending ON artifacts(conversation_id, status) WHERE deleted = 0;

CREATE TABLE attachments (
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    attachment_id TEXT NOT NULL CHECK (length(attachment_id) > 0),
    blob_path TEXT NOT NULL CHECK (length(blob_path) > 0),
    status TEXT NOT NULL CHECK (status IN ('draft', 'attached')),
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    deleted INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    payload TEXT NOT NULL CHECK (json_type(payload) = 'object'),
    PRIMARY KEY (conversation_id, attachment_id)
) WITHOUT ROWID;
CREATE INDEX attachments_revision ON attachments(conversation_id, revision);

CREATE TABLE blob_references (
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    owner_kind TEXT NOT NULL CHECK (owner_kind IN ('attachment', 'tool_output')),
    owner_id TEXT NOT NULL CHECK (length(owner_id) > 0),
    blob_path TEXT NOT NULL CHECK (length(blob_path) > 0),
    revision INTEGER NOT NULL CHECK (revision >= 1),
    deleted INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    PRIMARY KEY (conversation_id, owner_kind, owner_id, blob_path)
) WITHOUT ROWID;
CREATE INDEX blob_references_path ON blob_references(blob_path, deleted);

CREATE TABLE runtime_sessions (
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    provider TEXT NOT NULL CHECK (length(provider) > 0),
    revision INTEGER NOT NULL CHECK (revision >= 1),
    deleted INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    payload TEXT NOT NULL CHECK (json_type(payload) = 'object'),
    PRIMARY KEY (conversation_id, provider)
) WITHOUT ROWID;
"#;

/// Conversation rows carry metadata and summary, never transcript arrays.
macro_rules! conversation_row_select {
    ($filter:literal) => {
        concat!(
            "SELECT 'conversation', id, id, NULL, NULL, NULL, NULL, revision, deleted, \
             CASE WHEN deleted THEN NULL ELSE json_object(\
             'handle', handle, 'project_path', project_path, \
             'created_at', created_at, 'updated_at', updated_at, \
             'title', title, 'preview', preview, \
             'running', json(iif(running, 'true', 'false')), \
             'needs_attention', json(iif(needs_attention, 'true', 'false')), \
             'launched_run_ids', json(launched_run_ids), 'metadata', json(metadata)) END \
             FROM conversations ",
            $filter
        )
    };
}

const SUMMARIES_FOR_PROJECT_SQL: &str = conversation_row_select!(
    "WHERE project_path = ?1 AND deleted = 0 ORDER BY updated_at DESC, id DESC"
);
// ponytail: all-project listing sorts every live conversation; its size is the
// conversation count, never history. Index (updated_at, id) if that grows.
const SUMMARIES_ALL_SQL: &str =
    conversation_row_select!("WHERE deleted = 0 ORDER BY updated_at DESC, id DESC");
const ATTENTION_SUMMARIES_SQL: &str = conversation_row_select!(
    "WHERE needs_attention = 1 AND deleted = 0 ORDER BY updated_at DESC, id DESC"
);
const LIVE_CONVERSATION_SQL: &str = conversation_row_select!("WHERE id = ?1 AND deleted = 0");
const SUMMARY_CHANGES_SQL: &str = conversation_row_select!(
    "WHERE revision > ?2 AND revision <= ?3 AND (?1 IS NULL OR project_path = ?1) LIMIT ?4"
);
const CONVERSATION_CHANGES_SQL: &str = concat!(
    conversation_row_select!("WHERE id = ?1 AND revision > ?2 AND revision <= ?3"),
    " UNION ALL SELECT 'turn', conversation_id, turn_id, NULL, NULL, position, NULL, revision, \
     deleted, CASE WHEN deleted THEN NULL ELSE payload END FROM turns \
     WHERE conversation_id = ?1 AND revision > ?2 AND revision <= ?3",
    " UNION ALL SELECT 'segment', conversation_id, segment_id, NULL, turn_id, NULL, sort_order, \
     revision, deleted, CASE WHEN deleted THEN NULL ELSE payload END FROM segments \
     WHERE conversation_id = ?1 AND revision > ?2 AND revision <= ?3",
    " UNION ALL SELECT 'artifact', conversation_id, artifact_id, collection, turn_id, position, \
     NULL, revision, deleted, CASE WHEN deleted THEN NULL ELSE payload END FROM artifacts \
     WHERE conversation_id = ?1 AND revision > ?2 AND revision <= ?3",
    " UNION ALL SELECT 'attachment', conversation_id, attachment_id, NULL, NULL, NULL, NULL, \
     revision, deleted, CASE WHEN deleted THEN NULL ELSE payload END FROM attachments \
     WHERE conversation_id = ?1 AND revision > ?2 AND revision <= ?3",
    " LIMIT ?4"
);
const ORDER_CHANGED_SQL: &str = "SELECT EXISTS(SELECT 1 FROM turns \
     WHERE conversation_id = ?1 AND order_changed_revision > ?2)";
const PAGE_TURNS_SQL: &str = "SELECT 'turn', conversation_id, turn_id, NULL, NULL, position, \
     NULL, revision, deleted, payload FROM turns \
     WHERE conversation_id = ?1 AND deleted = 0 AND (position, turn_id) < (?2, ?3) \
     ORDER BY position DESC, turn_id DESC LIMIT ?4";
const PAGE_SEGMENTS_SQL: &str = "SELECT 'segment', conversation_id, segment_id, NULL, turn_id, \
     NULL, sort_order, revision, deleted, payload FROM segments \
     WHERE conversation_id = ?1 AND turn_id = ?2 AND deleted = 0 \
     ORDER BY sort_order, segment_id";
// Without statistics the planner prefers the primary key prefix for the
// nullable owner column, which scans every artifact of the conversation.
const PAGE_ARTIFACTS_SQL: &str = "SELECT 'artifact', conversation_id, artifact_id, collection, \
     turn_id, position, NULL, revision, deleted, payload FROM artifacts INDEXED BY artifacts_owner \
     WHERE conversation_id = ?1 AND turn_id = ?2 AND deleted = 0 \
     ORDER BY collection, artifact_id";
/// Active and queued turns, which a window pins even outside its page.
const PINNED_TURNS_SQL: &str = "SELECT 'turn', conversation_id, turn_id, NULL, NULL, position, \
     NULL, revision, deleted, payload FROM turns INDEXED BY turns_status \
     WHERE status IN ('pending', 'streaming', 'queued') AND conversation_id = ?1 AND deleted = 0";
/// Artifacts awaiting review, which a window pins even outside its page.
const PINNED_ARTIFACTS_SQL: &str = "SELECT 'artifact', conversation_id, artifact_id, collection, \
     turn_id, position, NULL, revision, deleted, payload FROM artifacts INDEXED BY artifacts_pending \
     WHERE conversation_id = ?1 AND status IN ('pending', 'pending_review') AND deleted = 0";
/// Named read statements whose query plans contract checks inspect.
const READ_STATEMENTS: &[(&str, &str)] = &[
    ("summaries_for_project", SUMMARIES_FOR_PROJECT_SQL),
    ("attention_summaries", ATTENTION_SUMMARIES_SQL),
    ("summary_changes", SUMMARY_CHANGES_SQL),
    ("conversation_changes", CONVERSATION_CHANGES_SQL),
    ("order_changed", ORDER_CHANGED_SQL),
    ("page_turns", PAGE_TURNS_SQL),
    ("page_segments", PAGE_SEGMENTS_SQL),
    ("page_artifacts", PAGE_ARTIFACTS_SQL),
    ("pinned_turns", PINNED_TURNS_SQL),
    ("pinned_artifacts", PINNED_ARTIFACTS_SQL),
];

/// Executions, turns and deliveries a previous process may have left
/// unsettled.
const UNSETTLED_CONVERSATIONS_SQL: &str = "SELECT c.id, c.project_path FROM conversations c \
     WHERE c.deleted = 0 AND (EXISTS(SELECT 1 FROM turns t INDEXED BY turns_unsettled \
     WHERE t.conversation_id = c.id AND t.deleted = 0 AND t.settled = 0 \
     AND t.role = 'assistant') OR EXISTS(SELECT 1 FROM turns t INDEXED BY turns_status \
     WHERE t.status IN ('pending', 'streaming', 'queued') AND t.conversation_id = c.id \
     AND t.deleted = 0) OR EXISTS(SELECT 1 FROM segments s INDEXED BY segments_status \
     WHERE s.status = 'delivery_pending' AND s.conversation_id = c.id AND s.deleted = 0)) \
     ORDER BY c.id";

/// The app-owned conversation database.
pub struct ConversationDatabase {
    path: PathBuf,
    generation: String,
    connection: Mutex<Connection>,
    /// Fault injection: results the next cleanup checkpoints report instead
    /// of running.
    injected_checkpoints: Mutex<VecDeque<CheckpointFault>>,
    /// Set by each committed write; cleared by a complete passive checkpoint.
    written_since_checkpoint: AtomicBool,
}

/// A cleanup checkpoint result to inject in place of the real one.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointFault {
    /// Another connection blocked it (`busy=1`).
    Busy,
    /// It returned without copying every WAL frame.
    Incomplete,
    /// It reported no WAL (`log=-1`).
    NoWal,
    /// The checkpoint statement itself failed.
    Error,
}

/// Every live row of one conversation, for loading it into memory.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredConversation {
    pub project_path: String,
    pub handle: String,
    /// The conversation row's last-change revision.
    pub revision: i64,
    pub metadata: Value,
    /// Ascending position: (turn id, position, payload).
    pub turns: Vec<(String, i64, Value)>,
    pub segments: Vec<Value>,
    /// (collection, artifact id, position, owner turn, payload), in insertion
    /// order per collection.
    pub artifacts: Vec<(String, String, i64, Option<String>, Value)>,
    /// (attachment id, blob path, status, payload).
    pub attachments: Vec<(String, String, String, Value)>,
}

/// A durable project deletion whose filesystem cleanup has not finished.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectDeletionMarker {
    pub project_path: String,
    pub project_id: String,
    pub response: Value,
}

/// Conversation metadata and summary columns.
#[derive(Debug, Clone, PartialEq)]
pub struct ConversationRow {
    pub id: String,
    pub project_path: String,
    pub handle: String,
    pub created_at: String,
    pub updated_at: String,
    pub title: String,
    pub preview: String,
    pub running: bool,
    pub needs_attention: bool,
    pub launched_run_ids: Vec<String>,
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TurnRow {
    pub conversation_id: String,
    pub id: String,
    pub role: String,
    pub kind: String,
    pub position: i64,
    pub settled: bool,
    pub status: String,
    pub updated_at: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SegmentRow {
    pub conversation_id: String,
    pub id: String,
    pub turn_id: String,
    pub kind: String,
    pub order: i64,
    pub status: String,
    pub updated_at: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArtifactRow {
    pub conversation_id: String,
    pub collection: String,
    pub id: String,
    pub turn_id: Option<String>,
    pub status: String,
    pub updated_at: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttachmentRow {
    pub conversation_id: String,
    pub id: String,
    pub blob_path: String,
    pub status: String,
    pub updated_at: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Conversation,
    Turn,
    Segment,
    Artifact,
    Attachment,
}

/// One current entity or tombstone as delivered to readers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EntityRow {
    pub kind: EntityKind,
    pub conversation_id: String,
    pub id: String,
    pub collection: Option<String>,
    pub turn_id: Option<String>,
    pub position: Option<i64>,
    pub order: Option<i64>,
    pub revision: i64,
    pub deleted: bool,
    /// `None` for tombstones.
    pub payload: Option<Value>,
}

/// Which rows a catch-up covers.
#[derive(Debug, Clone, Copy)]
pub enum ChangeScope<'a> {
    /// The conversation row and all of its children.
    Conversation(&'a str),
    /// Conversation rows only, optionally limited to one project.
    Summaries { project_path: Option<&'a str> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResetReason {
    OrderChanged,
    GenerationChanged,
    CursorAhead,
    CatchupLimit,
}

/// Every row with `from_revision < revision <= watermark` in the scope.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChangeBatch {
    pub from_revision: i64,
    pub watermark: i64,
    pub rows: Vec<EntityRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CatchUp {
    Batch(ChangeBatch),
    Reset(ResetReason),
}

/// Keyset position of the oldest turn already returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageCursor {
    pub opening_revision: i64,
    pub before_position: i64,
    pub before_turn_id: String,
}

/// Newest-first window of live turns, returned in ascending position.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnPage {
    pub watermark: i64,
    pub conversation: EntityRow,
    /// Turns followed by their live segments and owned artifacts, then the
    /// pinned turns (with their segments) and pinned artifacts.
    pub rows: Vec<EntityRow>,
    pub turn_ids: Vec<String>,
    /// Active or queued turns outside the page.
    pub pinned_turn_ids: Vec<String>,
    /// Artifacts awaiting review that no page turn owns: (collection, id).
    pub pinned_artifact_ids: Vec<(String, String)>,
    pub older_cursor: Option<PageCursor>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PageRead {
    Page(TurnPage),
    /// Unknown or deleted conversation.
    Missing,
    Reset(ResetReason),
}

/// A write transaction holding its allocated revision. Dropping it without
/// commit rolls back every change and the revision allocation.
pub struct WriteTransaction<'a> {
    tx: rusqlite::Transaction<'a>,
    revision: i64,
}

/// A read transaction pinned to one database snapshot.
pub struct ReadTransaction<'a> {
    tx: rusqlite::Transaction<'a>,
    generation: &'a str,
    watermark: i64,
}

impl ConversationDatabase {
    /// Open or create `$SPARK_HOME/conversations.sqlite3`.
    pub fn open(spark_home: impl AsRef<Path>) -> Result<Self> {
        let spark_home = spark_home.as_ref();
        std::fs::create_dir_all(spark_home)
            .map_err(|source| StorageError::io("create directory", spark_home, source))?;
        let path = spark_home.join(CONVERSATION_DATABASE_FILE_NAME);
        let mut connection = Connection::open(&path)?;
        configure(&connection, &path)?;

        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let initialized: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'store_meta')",
            [],
            |row| row.get(0),
        )?;
        if !initialized {
            tx.execute_batch(SCHEMA)?;
            tx.execute(
                "INSERT INTO store_meta (singleton, schema_version, generation, next_revision) \
                 VALUES (1, ?1, ?2, 1)",
                params![
                    CONVERSATION_DATABASE_SCHEMA_VERSION,
                    uuid::Uuid::new_v4().to_string()
                ],
            )?;
        }
        let (schema_version, generation): (i64, String) = tx.query_row(
            "SELECT schema_version, generation FROM store_meta WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if schema_version != CONVERSATION_DATABASE_SCHEMA_VERSION {
            return Err(StorageError::UnsupportedConversationDatabase {
                path,
                found: schema_version,
                expected: CONVERSATION_DATABASE_SCHEMA_VERSION,
            });
        }
        tx.commit()?;

        Ok(Self {
            path,
            generation,
            connection: Mutex::new(connection),
            injected_checkpoints: Mutex::new(VecDeque::new()),
            written_since_checkpoint: AtomicBool::new(false),
        })
    }

    /// The destructive-cleanup barrier. Holding the connection, so no write
    /// can install a reference meanwhile, run a FULL checkpoint and require it
    /// to be complete before `cleanup` rechecks rows and removes files. A NORMAL
    /// commit alone never authorizes deleting previously referenced bytes;
    /// any busy, error or incomplete result defers cleanup with files intact.
    pub fn after_checkpoint<T>(
        &self,
        cleanup: impl FnOnce(&CheckpointedView<'_>) -> Result<T>,
    ) -> Result<T> {
        let connection = self.lock();
        let injected = self
            .injected_checkpoints
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .pop_front();
        let result = match injected {
            None => query_checkpoint(&connection, "FULL"),
            Some(CheckpointFault::Busy) => Ok((1, 4, 2)),
            Some(CheckpointFault::Incomplete) => Ok((0, 4, 2)),
            Some(CheckpointFault::NoWal) => Ok((0, -1, -1)),
            Some(CheckpointFault::Error) => Err(StorageError::ConversationDatabase(
                rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_IOERR),
                    Some("injected checkpoint failure".to_string()),
                ),
            )),
        };
        result
            .and_then(|result| require_complete("FULL", result))
            .map_err(|error| StorageError::CleanupDeferred {
                reason: error.to_string(),
            })?;
        cleanup(&CheckpointedView {
            connection: &connection,
        })
    }

    /// Fault injection: the next cleanup checkpoints report `faults`, in
    /// order, instead of running.
    #[doc(hidden)]
    pub fn fail_next_checkpoints(&self, faults: impl IntoIterator<Item = CheckpointFault>) {
        self.injected_checkpoints
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .extend(faults);
    }

    /// Measurement only: switch this connection's `synchronous` setting
    /// (`NORMAL` or `FULL`) to compare durability costs on one workload.
    #[doc(hidden)]
    pub fn set_synchronous(&self, mode: &str) -> Result<()> {
        if !matches!(mode, "NORMAL" | "FULL") {
            return Err(StorageError::InvalidConversationState {
                path: self.path.clone(),
                reason: format!("unsupported synchronous mode {mode}"),
            });
        }
        self.lock().pragma_update(None, "synchronous", mode)?;
        Ok(())
    }

    /// Measurement only: run a passive checkpoint and return its raw
    /// `(busy, wal frames, checkpointed frames)` result.
    #[doc(hidden)]
    pub fn passive_checkpoint_frames(&self) -> Result<(i64, i64, i64)> {
        query_checkpoint(&self.lock(), "PASSIVE")
    }

    /// Statements that change only cleanup bookkeeping, outside revisions.
    pub(crate) fn mark_reset_complete(&self) -> Result<()> {
        self.lock()
            .execute("UPDATE store_meta SET reset_complete = 1", [])?;
        Ok(())
    }

    pub(crate) fn remove_project_deletion(&self, project_path: &str) -> Result<()> {
        self.lock().execute(
            "DELETE FROM project_deletions WHERE project_path = ?1",
            [project_path],
        )?;
        Ok(())
    }

    /// The periodic passive checkpoint: `Ok(false)` when nothing was written
    /// since the last complete one. A failed or incomplete checkpoint keeps
    /// the pending flag, so the next call retries it.
    pub fn checkpoint_if_written(&self) -> Result<bool> {
        let connection = self.lock();
        if !self.written_since_checkpoint.load(Ordering::SeqCst) {
            return Ok(false);
        }
        checkpoint(&connection, "PASSIVE")?;
        self.written_since_checkpoint.store(false, Ordering::SeqCst);
        Ok(true)
    }

    /// Whether a write awaits the periodic passive checkpoint.
    #[doc(hidden)]
    pub fn checkpoint_pending(&self) -> bool {
        self.written_since_checkpoint.load(Ordering::SeqCst)
    }

    /// Truncating checkpoint once writers have stopped.
    pub fn checkpoint_on_shutdown(&self) -> Result<()> {
        checkpoint(&self.lock(), "TRUNCATE")
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Random UUID assigned when this database was created.
    pub fn generation(&self) -> &str {
        &self.generation
    }

    /// Run `apply` in one immediate transaction with one newly allocated
    /// revision. An error from `apply` or the commit rolls everything back,
    /// including the revision allocation. Low-level contract tests only:
    /// conversation state changes go through the typed operations.
    #[doc(hidden)]
    pub fn write<T>(
        &self,
        apply: impl FnOnce(&WriteTransaction<'_>) -> Result<T>,
    ) -> Result<(i64, T)> {
        let mut connection = self.lock();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let revision = tx.query_row(
            "UPDATE store_meta SET next_revision = next_revision + 1 \
             WHERE singleton = 1 RETURNING next_revision - 1",
            [],
            |row| row.get(0),
        )?;
        let write = WriteTransaction { tx, revision };
        let value = apply(&write)?;
        write.tx.commit()?;
        self.written_since_checkpoint.store(true, Ordering::SeqCst);
        Ok((revision, value))
    }

    /// `write` for an operation that may change nothing: when `apply`
    /// changed no row the transaction rolls back, allocating no revision.
    /// `publish` runs after the commit while the connection is still held,
    /// so notices of successive commits are published in commit order.
    pub(crate) fn write_if_changed<T>(
        &self,
        apply: impl FnOnce(&WriteTransaction<'_>) -> Result<T>,
        publish: impl FnOnce(i64, &T),
    ) -> Result<(Option<i64>, T)> {
        let mut connection = self.lock();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let revision = tx.query_row(
            "UPDATE store_meta SET next_revision = next_revision + 1 \
             WHERE singleton = 1 RETURNING next_revision - 1",
            [],
            |row| row.get(0),
        )?;
        let changes_before = tx.total_changes();
        let write = WriteTransaction { tx, revision };
        let value = apply(&write)?;
        if write.tx.total_changes() == changes_before {
            return Ok((None, value));
        }
        write.tx.commit()?;
        self.written_since_checkpoint.store(true, Ordering::SeqCst);
        publish(revision, &value);
        Ok((Some(revision), value))
    }

    /// Run `read` against one coherent snapshot and its watermark.
    pub fn read<T>(&self, read: impl FnOnce(&ReadTransaction<'_>) -> Result<T>) -> Result<T> {
        let mut connection = self.lock();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        // The first read starts the snapshot every later statement sees.
        let watermark = tx.query_row(
            "SELECT next_revision - 1 FROM store_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        let snapshot = ReadTransaction {
            tx,
            generation: &self.generation,
            watermark,
        };
        read(&snapshot)
    }

    /// Current value of a connection pragma, for configuration checks.
    #[doc(hidden)]
    pub fn pragma(&self, name: &str) -> Result<String> {
        let value: rusqlite::types::Value = self
            .lock()
            .pragma_query_value(None, name, |row| row.get(0))?;
        Ok(match value {
            rusqlite::types::Value::Integer(value) => value.to_string(),
            rusqlite::types::Value::Text(value) => value,
            other => format!("{other:?}"),
        })
    }

    /// `EXPLAIN QUERY PLAN` details for each named read statement.
    #[doc(hidden)]
    pub fn read_query_plans(&self) -> Result<Vec<(&'static str, Vec<String>)>> {
        let connection = self.lock();
        READ_STATEMENTS
            .iter()
            .map(|(name, sql)| {
                let mut statement = connection.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
                let mut rows = statement.raw_query();
                let mut details = Vec::new();
                while let Some(row) = rows.next()? {
                    details.push(row.get(3)?);
                }
                Ok((*name, details))
            })
            .collect()
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        // A panic inside a transaction drops it, which rolls it back, so the
        // connection behind a poisoned mutex is still consistent.
        self.connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Run a checkpoint and require it to be complete: not busy, with every WAL
/// frame checkpointed.
fn checkpoint(connection: &Connection, mode: &str) -> Result<()> {
    require_complete(mode, query_checkpoint(connection, mode)?)
}

/// The raw `(busy, log, checkpointed)` row of one checkpoint.
fn query_checkpoint(connection: &Connection, mode: &str) -> Result<(i64, i64, i64)> {
    Ok(
        connection.query_row(&format!("PRAGMA main.wal_checkpoint({mode})"), [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?,
    )
}

fn require_complete(mode: &str, (busy, log, checkpointed): (i64, i64, i64)) -> Result<()> {
    if busy != 0 || log < 0 || log != checkpointed {
        return Err(StorageError::CheckpointIncomplete {
            reason: format!(
                "conversation database {mode} checkpoint incomplete (busy={busy}, log={log}, \
                 checkpointed={checkpointed})"
            ),
        });
    }
    Ok(())
}

fn configure(connection: &Connection, path: &Path) -> Result<()> {
    connection.pragma_update(None, "foreign_keys", "ON")?;
    let journal_mode: String =
        connection.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(StorageError::InvalidConversationState {
            path: path.to_path_buf(),
            reason: format!(
                "Conversation database {} cannot use WAL journaling (got {journal_mode}); \
                 it must live on a local filesystem.",
                path.display()
            ),
        });
    }
    // Approved policy: NORMAL keeps committed state across process crashes but
    // can lose recent acknowledged transactions on power loss.
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    connection.pragma_update(None, "checkpoint_fullfsync", "ON")?;
    Ok(())
}

impl WriteTransaction<'_> {
    /// Revision stamped on every row this transaction changes.
    pub fn revision(&self) -> i64 {
        self.revision
    }

    /// The connection, for row reads shared with read transactions.
    pub(crate) fn connection(&self) -> &Connection {
        &self.tx
    }

    pub fn put_conversation(&self, row: &ConversationRow) -> Result<()> {
        self.tx.execute(
            "INSERT INTO conversations (id, project_path, handle, created_at, updated_at, \
             revision, title, preview, running, needs_attention, launched_run_ids, metadata) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
             ON CONFLICT (id) DO UPDATE SET project_path = excluded.project_path, \
             handle = excluded.handle, updated_at = excluded.updated_at, \
             revision = excluded.revision, deleted = 0, title = excluded.title, \
             preview = excluded.preview, running = excluded.running, \
             needs_attention = excluded.needs_attention, \
             launched_run_ids = excluded.launched_run_ids, metadata = excluded.metadata",
            params![
                row.id,
                row.project_path,
                row.handle,
                row.created_at,
                row.updated_at,
                self.revision,
                row.title,
                row.preview,
                row.running,
                row.needs_attention,
                serde_json::to_string(&row.launched_run_ids).expect("string array serializes"),
                row.metadata.to_string(),
            ],
        )?;
        Ok(())
    }

    /// Tombstone a conversation and every live child row, releasing its handle.
    pub fn delete_conversation(&self, conversation_id: &str) -> Result<bool> {
        for table in [
            "turns",
            "segments",
            "artifacts",
            "attachments",
            "blob_references",
            "runtime_sessions",
        ] {
            self.tx.execute(
                &format!(
                    "UPDATE {table} SET deleted = 1, revision = ?1 \
                     WHERE conversation_id = ?2 AND deleted = 0"
                ),
                params![self.revision, conversation_id],
            )?;
        }
        let changed = self.tx.execute(
            "UPDATE conversations SET deleted = 1, revision = ?1 WHERE id = ?2 AND deleted = 0",
            params![self.revision, conversation_id],
        )?;
        Ok(changed > 0)
    }

    pub fn put_turn(&self, row: &TurnRow) -> Result<()> {
        self.touch(&row.conversation_id)?;
        // Moving an existing turn invalidates older page cursors; appends do not.
        self.tx.execute(
            "INSERT INTO turns (conversation_id, turn_id, role, kind, position, settled, status, \
             updated_at, revision, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
             ON CONFLICT (conversation_id, turn_id) DO UPDATE SET \
             order_changed_revision = iif(turns.position != excluded.position, \
             excluded.revision, turns.order_changed_revision), \
             role = excluded.role, kind = excluded.kind, position = excluded.position, \
             settled = excluded.settled, status = excluded.status, \
             updated_at = excluded.updated_at, revision = excluded.revision, deleted = 0, \
             payload = excluded.payload",
            params![
                row.conversation_id,
                row.id,
                row.role,
                row.kind,
                row.position,
                row.settled,
                row.status,
                row.updated_at,
                self.revision,
                row.payload.to_string(),
            ],
        )?;
        Ok(())
    }

    pub fn put_segment(&self, row: &SegmentRow) -> Result<()> {
        self.touch(&row.conversation_id)?;
        self.tx.execute(
            "INSERT INTO segments (conversation_id, segment_id, turn_id, kind, sort_order, status, \
             updated_at, revision, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
             ON CONFLICT (conversation_id, segment_id) DO UPDATE SET \
             turn_id = excluded.turn_id, kind = excluded.kind, sort_order = excluded.sort_order, \
             status = excluded.status, updated_at = excluded.updated_at, \
             revision = excluded.revision, deleted = 0, payload = excluded.payload",
            params![
                row.conversation_id,
                row.id,
                row.turn_id,
                row.kind,
                row.order,
                row.status,
                row.updated_at,
                self.revision,
                row.payload.to_string(),
            ],
        )?;
        Ok(())
    }

    pub fn put_artifact(&self, row: &ArtifactRow) -> Result<()> {
        self.touch(&row.conversation_id)?;
        self.tx.execute(
            "INSERT INTO artifacts (conversation_id, collection, artifact_id, turn_id, status, \
             updated_at, revision, payload, position) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, \
             (SELECT coalesce(max(position), 0) + 1 FROM artifacts \
             WHERE conversation_id = ?1 AND collection = ?2)) \
             ON CONFLICT (conversation_id, collection, artifact_id) DO UPDATE SET \
             turn_id = excluded.turn_id, status = excluded.status, \
             updated_at = excluded.updated_at, revision = excluded.revision, deleted = 0, \
             payload = excluded.payload",
            params![
                row.conversation_id,
                row.collection,
                row.id,
                row.turn_id,
                row.status,
                row.updated_at,
                self.revision,
                row.payload.to_string(),
            ],
        )?;
        Ok(())
    }

    pub fn put_attachment(&self, row: &AttachmentRow) -> Result<()> {
        self.touch(&row.conversation_id)?;
        self.tx.execute(
            "INSERT INTO attachments (conversation_id, attachment_id, blob_path, status, \
             updated_at, revision, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT (conversation_id, attachment_id) DO UPDATE SET \
             blob_path = excluded.blob_path, status = excluded.status, \
             updated_at = excluded.updated_at, revision = excluded.revision, deleted = 0, \
             payload = excluded.payload",
            params![
                row.conversation_id,
                row.id,
                row.blob_path,
                row.status,
                row.updated_at,
                self.revision,
                row.payload.to_string(),
            ],
        )?;
        Ok(())
    }

    pub fn put_blob_reference(
        &self,
        conversation_id: &str,
        owner_kind: &str,
        owner_id: &str,
        blob_path: &str,
    ) -> Result<()> {
        self.touch(conversation_id)?;
        self.tx.execute(
            "INSERT INTO blob_references (conversation_id, owner_kind, owner_id, blob_path, \
             revision) VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT (conversation_id, owner_kind, owner_id, blob_path) DO UPDATE SET \
             revision = excluded.revision, deleted = 0",
            params![
                conversation_id,
                owner_kind,
                owner_id,
                blob_path,
                self.revision
            ],
        )?;
        Ok(())
    }

    pub fn put_runtime_session(
        &self,
        conversation_id: &str,
        provider: &str,
        payload: &Value,
    ) -> Result<()> {
        // Internal recovery bookkeeping: never a browser-visible change.
        self.require_live(conversation_id)?;
        self.tx.execute(
            "INSERT INTO runtime_sessions (conversation_id, provider, revision, payload) \
             VALUES (?1, ?2, ?3, ?4) ON CONFLICT (conversation_id, provider) DO UPDATE SET \
             revision = excluded.revision, deleted = 0, payload = excluded.payload",
            params![
                conversation_id,
                provider,
                self.revision,
                payload.to_string()
            ],
        )?;
        Ok(())
    }

    pub fn delete_turn(&self, conversation_id: &str, turn_id: &str) -> Result<bool> {
        self.tombstone("turns", "turn_id = ?3", conversation_id, &[turn_id])
    }

    /// Tombstone a segment and the tool-output references it owns.
    pub fn delete_segment(&self, conversation_id: &str, segment_id: &str) -> Result<bool> {
        self.tombstone(
            "blob_references",
            "owner_kind = 'tool_output' AND owner_id = ?3",
            conversation_id,
            &[segment_id],
        )?;
        self.tombstone(
            "segments",
            "segment_id = ?3",
            conversation_id,
            &[segment_id],
        )
    }

    pub fn delete_artifact(
        &self,
        conversation_id: &str,
        collection: &str,
        artifact_id: &str,
    ) -> Result<bool> {
        self.tombstone(
            "artifacts",
            "collection = ?3 AND artifact_id = ?4",
            conversation_id,
            &[collection, artifact_id],
        )
    }

    pub fn delete_attachment(&self, conversation_id: &str, attachment_id: &str) -> Result<bool> {
        self.tombstone(
            "attachments",
            "attachment_id = ?3",
            conversation_id,
            &[attachment_id],
        )
    }

    pub fn delete_blob_reference(
        &self,
        conversation_id: &str,
        owner_kind: &str,
        owner_id: &str,
        blob_path: &str,
    ) -> Result<bool> {
        self.tombstone(
            "blob_references",
            "owner_kind = ?3 AND owner_id = ?4 AND blob_path = ?5",
            conversation_id,
            &[owner_kind, owner_id, blob_path],
        )
    }

    pub fn delete_runtime_session(&self, conversation_id: &str, provider: &str) -> Result<bool> {
        self.tombstone(
            "runtime_sessions",
            "provider = ?3",
            conversation_id,
            &[provider],
        )
    }

    /// Tombstone every live conversation of a project and record the
    /// pending filesystem cleanup. Returns the tombstoned conversation ids.
    pub fn delete_project(&self, marker: &ProjectDeletionMarker) -> Result<Vec<String>> {
        let ids = {
            let mut statement = self.tx.prepare(
                "SELECT id FROM conversations WHERE project_path = ?1 AND deleted = 0 ORDER BY id",
            )?;
            let ids = statement
                .query_map([&marker.project_path], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            ids
        };
        for id in &ids {
            self.delete_conversation(id)?;
        }
        self.tx.execute(
            "INSERT INTO project_deletions (project_path, project_id, phase, deletion_revision, \
             response) VALUES (?1, ?2, 'cleanup_pending', ?3, ?4) \
             ON CONFLICT (project_path) DO UPDATE SET project_id = excluded.project_id, \
             deletion_revision = excluded.deletion_revision, response = excluded.response",
            params![
                marker.project_path,
                marker.project_id,
                self.revision,
                marker.response.to_string()
            ],
        )?;
        Ok(ids)
    }

    fn tombstone(
        &self,
        table: &str,
        identity: &str,
        conversation_id: &str,
        keys: &[&str],
    ) -> Result<bool> {
        self.touch(conversation_id)?;
        let mut values: Vec<&dyn rusqlite::ToSql> = vec![&self.revision, &conversation_id];
        values.extend(keys.iter().map(|key| key as &dyn rusqlite::ToSql));
        let changed = self.tx.execute(
            &format!(
                "UPDATE {table} SET deleted = 1, revision = ?1 \
                 WHERE conversation_id = ?2 AND {identity} AND deleted = 0"
            ),
            values.as_slice(),
        )?;
        Ok(changed > 0)
    }

    /// Every row this transaction changed in one conversation, tombstones
    /// included: the entity result of a mutation.
    pub fn changed_rows(&self, conversation_id: &str) -> Result<Vec<EntityRow>> {
        entity_rows(
            &self.tx,
            CONVERSATION_CHANGES_SQL,
            params![conversation_id, self.revision - 1, self.revision, i64::MAX],
        )
    }

    fn require_live(&self, conversation_id: &str) -> Result<()> {
        let live: bool = self.tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id = ?1 AND deleted = 0)",
            [conversation_id],
            |row| row.get(0),
        )?;
        if !live {
            return Err(StorageError::ConversationCommitRejected {
                conversation_id: conversation_id.to_string(),
                reason: "conversation does not exist or was deleted".to_string(),
            });
        }
        Ok(())
    }

    /// Stamp the owning conversation with this revision; children of a
    /// missing or deleted conversation are rejected. Alone, it re-delivers the
    /// conversation row, such as when a summary decoration changed.
    pub fn touch(&self, conversation_id: &str) -> Result<()> {
        let changed = self.tx.execute(
            "UPDATE conversations SET revision = ?1 WHERE id = ?2 AND deleted = 0",
            params![self.revision, conversation_id],
        )?;
        if changed == 0 {
            return Err(StorageError::ConversationCommitRejected {
                conversation_id: conversation_id.to_string(),
                reason: "conversation does not exist or was deleted".to_string(),
            });
        }
        Ok(())
    }
}

impl ReadTransaction<'_> {
    /// The connection, for row reads shared with write transactions.
    pub(crate) fn connection(&self) -> &Connection {
        &self.tx
    }

    /// Whether any live row references exactly `blob_path`.
    pub fn has_live_reference(&self, blob_path: &str) -> Result<bool> {
        Ok(self.tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM blob_references WHERE blob_path = ?1 AND deleted = 0)",
            [blob_path],
            |row| row.get(0),
        )?)
    }

    /// Highest committed revision visible to this snapshot.
    pub fn watermark(&self) -> i64 {
        self.watermark
    }

    /// Live conversation rows for one project, or all projects, newest first.
    pub fn summaries(&self, project_path: Option<&str>) -> Result<Vec<EntityRow>> {
        match project_path {
            Some(project_path) => self.entity_rows(SUMMARIES_FOR_PROJECT_SQL, [project_path]),
            None => self.entity_rows(SUMMARIES_ALL_SQL, []),
        }
    }

    /// A live conversation's metadata and summary row.
    pub fn conversation_row(&self, conversation_id: &str) -> Result<Option<EntityRow>> {
        Ok(self
            .entity_rows(LIVE_CONVERSATION_SQL, [conversation_id])?
            .pop())
    }

    /// One turn's current row or tombstone.
    pub fn turn_row(&self, conversation_id: &str, turn_id: &str) -> Result<Option<EntityRow>> {
        Ok(self
            .entity_rows(
                "SELECT 'turn', conversation_id, turn_id, NULL, NULL, position, NULL, revision, \
                 deleted, CASE WHEN deleted THEN NULL ELSE payload END FROM turns \
                 WHERE conversation_id = ?1 AND turn_id = ?2",
                [conversation_id, turn_id],
            )?
            .pop())
    }

    /// One segment's current row or tombstone.
    pub fn segment_row(
        &self,
        conversation_id: &str,
        segment_id: &str,
    ) -> Result<Option<EntityRow>> {
        Ok(self
            .entity_rows(
                "SELECT 'segment', conversation_id, segment_id, NULL, turn_id, NULL, sort_order, \
                 revision, deleted, CASE WHEN deleted THEN NULL ELSE payload END FROM segments \
                 WHERE conversation_id = ?1 AND segment_id = ?2",
                [conversation_id, segment_id],
            )?
            .pop())
    }

    /// Live conversations needing attention, newest first.
    pub fn attention_summaries(&self) -> Result<Vec<EntityRow>> {
        self.entity_rows(ATTENTION_SUMMARIES_SQL, [])
    }

    /// Replacement rows and tombstones changed after `since`, up to this
    /// snapshot's watermark. Never partial: over the limits it requests resync.
    pub fn changes(&self, scope: ChangeScope<'_>, generation: &str, since: i64) -> Result<CatchUp> {
        if generation != self.generation {
            return Ok(CatchUp::Reset(ResetReason::GenerationChanged));
        }
        if since > self.watermark {
            return Ok(CatchUp::Reset(ResetReason::CursorAhead));
        }
        let limit = CATCH_UP_MAX_ROWS as i64 + 1;
        let rows = match scope {
            ChangeScope::Conversation(conversation_id) => self.entity_rows(
                CONVERSATION_CHANGES_SQL,
                params![conversation_id, since, self.watermark, limit],
            )?,
            ChangeScope::Summaries { project_path } => self.entity_rows(
                SUMMARY_CHANGES_SQL,
                params![project_path, since, self.watermark, limit],
            )?,
        };
        if rows.len() > CATCH_UP_MAX_ROWS {
            return Ok(CatchUp::Reset(ResetReason::CatchupLimit));
        }
        let batch = ChangeBatch {
            from_revision: since,
            watermark: self.watermark,
            rows,
        };
        // The byte cap covers the whole serialized batch: identities,
        // tombstones and envelope fields, not just payloads.
        let bytes = serde_json::to_vec(&batch)
            .map_err(|source| StorageError::JsonWrite {
                path: PathBuf::from(CONVERSATION_DATABASE_FILE_NAME),
                source,
            })?
            .len();
        if bytes > CATCH_UP_MAX_BYTES {
            return Ok(CatchUp::Reset(ResetReason::CatchupLimit));
        }
        Ok(CatchUp::Batch(batch))
    }

    /// The newest `limit` live turns, or the `limit` turns older than
    /// `before`, with their segments and owned artifacts.
    pub fn page(
        &self,
        conversation_id: &str,
        limit: usize,
        before: Option<&PageCursor>,
    ) -> Result<PageRead> {
        let Some(conversation) = self
            .entity_rows(LIVE_CONVERSATION_SQL, [conversation_id])?
            .pop()
        else {
            return Ok(PageRead::Missing);
        };
        let opening_revision = match before {
            Some(cursor) => {
                let moved: bool = self.tx.query_row(
                    ORDER_CHANGED_SQL,
                    params![conversation_id, cursor.opening_revision],
                    |row| row.get(0),
                )?;
                if moved {
                    return Ok(PageRead::Reset(ResetReason::OrderChanged));
                }
                cursor.opening_revision
            }
            None => self.watermark,
        };
        let (before_position, before_turn_id) = before
            .map(|cursor| (cursor.before_position, cursor.before_turn_id.as_str()))
            .unwrap_or((i64::MAX, ""));
        let mut turns = self.entity_rows(
            PAGE_TURNS_SQL,
            params![
                conversation_id,
                before_position,
                before_turn_id,
                limit as i64 + 1
            ],
        )?;
        let older_cursor = if turns.len() > limit {
            turns.truncate(limit);
            turns.last().map(|oldest| PageCursor {
                opening_revision,
                before_position: oldest.position.unwrap_or_default(),
                before_turn_id: oldest.id.clone(),
            })
        } else {
            None
        };
        turns.reverse();

        let turn_ids: Vec<String> = turns.iter().map(|turn| turn.id.clone()).collect();
        let mut rows = turns;
        for turn_id in &turn_ids {
            rows.extend(self.entity_rows(PAGE_SEGMENTS_SQL, [conversation_id, turn_id])?);
            rows.extend(self.entity_rows(PAGE_ARTIFACTS_SQL, [conversation_id, turn_id])?);
        }
        // Few rows, sorted here rather than by a second index.
        let mut pinned_turns = self.entity_rows(PINNED_TURNS_SQL, [conversation_id])?;
        pinned_turns
            .sort_by(|left, right| (left.position, &left.id).cmp(&(right.position, &right.id)));
        let mut pinned_artifacts = self.entity_rows(PINNED_ARTIFACTS_SQL, [conversation_id])?;
        pinned_artifacts.sort_by(|left, right| {
            (&left.collection, left.position).cmp(&(&right.collection, right.position))
        });
        let mut pinned_turn_ids = Vec::new();
        for turn in pinned_turns {
            if turn_ids.contains(&turn.id) {
                continue;
            }
            pinned_turn_ids.push(turn.id.clone());
            let segments = self.entity_rows(PAGE_SEGMENTS_SQL, [conversation_id, &turn.id])?;
            rows.push(turn);
            rows.extend(segments);
        }
        let mut pinned_artifact_ids = Vec::new();
        for artifact in pinned_artifacts {
            if artifact
                .turn_id
                .as_ref()
                .is_some_and(|owner| turn_ids.contains(owner))
            {
                continue;
            }
            pinned_artifact_ids.push((
                artifact.collection.clone().unwrap_or_default(),
                artifact.id.clone(),
            ));
            rows.push(artifact);
        }
        Ok(PageRead::Page(TurnPage {
            watermark: self.watermark,
            conversation,
            rows,
            turn_ids,
            pinned_turn_ids,
            pinned_artifact_ids,
            older_cursor,
        }))
    }

    /// Every live row of a conversation; `None` when it is unknown or deleted.
    pub fn conversation(&self, conversation_id: &str) -> Result<Option<StoredConversation>> {
        let head = self.tx.query_row(
            "SELECT project_path, handle, metadata, revision \
             FROM conversations WHERE id = ?1 AND deleted = 0",
            [conversation_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        );
        let (project_path, handle, metadata, revision) = match head {
            Ok(head) => head,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let turns = self
            .json_rows(
                "SELECT turn_id, position, payload FROM turns \
             WHERE conversation_id = ?1 AND deleted = 0 ORDER BY position, turn_id",
                conversation_id,
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )?
            .into_iter()
            .map(|(id, position, payload)| Ok((id, position, parse(&payload)?)))
            .collect::<Result<Vec<_>>>()?;
        let segments = self
            .json_rows(
                "SELECT s.payload FROM segments s JOIN turns t \
                 ON t.conversation_id = s.conversation_id AND t.turn_id = s.turn_id \
                 WHERE s.conversation_id = ?1 AND s.deleted = 0 \
                 ORDER BY t.position, s.sort_order, s.segment_id",
                conversation_id,
                |row| row.get::<_, String>(0),
            )?
            .iter()
            .map(|payload| parse(payload))
            .collect::<Result<Vec<_>>>()?;
        let artifacts = self
            .json_rows(
                "SELECT collection, artifact_id, position, turn_id, payload FROM artifacts \
                 WHERE conversation_id = ?1 AND deleted = 0 ORDER BY collection, position",
                conversation_id,
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )?
            .into_iter()
            .map(|(collection, id, position, owner, payload)| {
                Ok((collection, id, position, owner, parse(&payload)?))
            })
            .collect::<Result<Vec<_>>>()?;
        let attachments = self
            .json_rows(
                "SELECT attachment_id, blob_path, status, payload FROM attachments \
                 WHERE conversation_id = ?1 AND deleted = 0 ORDER BY attachment_id",
                conversation_id,
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )?
            .into_iter()
            .map(
                |(id, path, status, payload): (String, String, String, String)| {
                    Ok((id, path, status, parse(&payload)?))
                },
            )
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(StoredConversation {
            project_path,
            handle,
            revision,
            metadata: parse(&metadata)?,
            turns,
            segments,
            artifacts,
            attachments,
        }))
    }

    /// The project a conversation id belongs to and whether it was deleted.
    pub fn conversation_location(&self, conversation_id: &str) -> Result<Option<(String, bool)>> {
        match self.tx.query_row(
            "SELECT project_path, deleted FROM conversations WHERE id = ?1",
            [conversation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ) {
            Ok(location) => Ok(Some(location)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// The live conversation holding `handle`: (conversation id, project path).
    pub fn conversation_by_handle(&self, handle: &str) -> Result<Option<(String, String)>> {
        match self.tx.query_row(
            "SELECT id, project_path FROM conversations WHERE handle = ?1 AND deleted = 0",
            [handle],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ) {
            Ok(found) => Ok(Some(found)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Live conversation ids of one project, in id order.
    pub fn conversation_ids(&self, project_path: &str) -> Result<Vec<String>> {
        self.json_rows(
            "SELECT id FROM conversations WHERE project_path = ?1 AND deleted = 0 ORDER BY id",
            project_path,
            |row| row.get(0),
        )
    }

    /// Live conversations with an active or queued turn or an undecided
    /// live delivery: (conversation id, project path).
    pub fn unsettled_conversations(&self) -> Result<Vec<(String, String)>> {
        let mut statement = self.tx.prepare_cached(UNSETTLED_CONVERSATIONS_SQL)?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The most recently written live runtime session payload.
    pub fn runtime_session(&self, conversation_id: &str) -> Result<Option<Value>> {
        let payload = self
            .json_rows(
                "SELECT payload FROM runtime_sessions WHERE conversation_id = ?1 AND deleted = 0 \
                 ORDER BY revision DESC LIMIT 1",
                conversation_id,
                |row| row.get::<_, String>(0),
            )?
            .pop();
        payload.map(|payload| parse(&payload)).transpose()
    }

    pub fn project_deletions(&self) -> Result<Vec<ProjectDeletionMarker>> {
        let mut statement = self.tx.prepare(
            "SELECT project_path, project_id, response FROM project_deletions ORDER BY project_path",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get::<_, String>(2)?))
            })?
            .collect::<rusqlite::Result<Vec<(String, String, String)>>>()?;
        rows.into_iter()
            .map(|(project_path, project_id, response)| {
                Ok(ProjectDeletionMarker {
                    project_path,
                    project_id,
                    response: parse(&response)?,
                })
            })
            .collect()
    }

    pub fn reset_complete(&self) -> Result<bool> {
        Ok(self.tx.query_row(
            "SELECT reset_complete FROM store_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?)
    }

    fn json_rows<T>(
        &self,
        sql: &str,
        key: &str,
        map: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Vec<T>> {
        let mut statement = self.tx.prepare_cached(sql)?;
        let rows = statement.query_map([key], map)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn entity_rows(&self, sql: &str, params: impl rusqlite::Params) -> Result<Vec<EntityRow>> {
        entity_rows(&self.tx, sql, params)
    }
}

fn entity_rows(
    connection: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<EntityRow>> {
    let mut statement = connection.prepare_cached(sql)?;
    let rows = statement.query_map(params, |row| {
        let kind = match row.get_ref(0)?.as_str()? {
            "conversation" => EntityKind::Conversation,
            "turn" => EntityKind::Turn,
            "segment" => EntityKind::Segment,
            "artifact" => EntityKind::Artifact,
            _ => EntityKind::Attachment,
        };
        let payload: Option<String> = row.get(9)?;
        Ok(EntityRow {
            kind,
            conversation_id: row.get(1)?,
            id: row.get(2)?,
            collection: row.get(3)?,
            turn_id: row.get(4)?,
            position: row.get(5)?,
            order: row.get(6)?,
            revision: row.get(7)?,
            deleted: row.get(8)?,
            payload: payload
                .map(|payload| serde_json::from_str(&payload))
                .transpose()
                .map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        9,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The connection after a complete FULL checkpoint, for rechecking cleanup
/// candidates against current rows before removing files.
pub struct CheckpointedView<'a> {
    connection: &'a Connection,
}

impl CheckpointedView<'_> {
    /// Whether any live reference names `blob_path` or a path below it.
    pub fn has_live_reference_under(&self, blob_path: &str) -> Result<bool> {
        let prefix = format!("{}/", blob_path.trim_end_matches('/'));
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM blob_references WHERE deleted = 0 \
             AND (blob_path = ?1 OR substr(blob_path, 1, length(?2)) = ?2))",
            params![blob_path, prefix],
            |row| row.get(0),
        )?)
    }

    pub fn conversation_is_live(&self, conversation_id: &str) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id = ?1 AND deleted = 0)",
            [conversation_id],
            |row| row.get(0),
        )?)
    }

    pub fn project_has_live_conversations(&self, project_path: &str) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE project_path = ?1 AND deleted = 0)",
            [project_path],
            |row| row.get(0),
        )?)
    }
}

fn parse(payload: &str) -> Result<Value> {
    serde_json::from_str(payload).map_err(|source| StorageError::JsonRead {
        path: PathBuf::from(CONVERSATION_DATABASE_FILE_NAME),
        source,
    })
}
