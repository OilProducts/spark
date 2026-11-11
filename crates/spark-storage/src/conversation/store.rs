//! The app-owned conversation store: one SQLite database and home lock per
//! Spark home, which is the only authority for conversation state, and the
//! per-project lifecycle gates.
//!
//! Lock order: project lifecycle gate, then the database connection. Reads
//! take only the connection.

use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

use fs2::FileExt;

use crate::error::{Result, StorageError};
use crate::{DeletedProjectRecord, ProjectRegistry};

use super::database::{ConversationDatabase, ProjectDeletionMarker};

/// Held exclusively for the app's lifetime; never unlinked.
pub const SERVER_LOCK_FILE_NAME: &str = "server.lock";
/// Workflow events, stored as artifacts with no owner.
pub(crate) const EVENT_LOG_COLLECTION: &str = "event_log";
pub(crate) const ALL_COLLECTIONS: [super::ArtifactCollection; 4] = [
    super::ArtifactCollection::FlowRunRequests,
    super::ArtifactCollection::FlowLaunches,
    super::ArtifactCollection::RunRecoveries,
    super::ArtifactCollection::ProposedPlans,
];
/// How often a passive checkpoint runs when writes occurred.
const PASSIVE_CHECKPOINT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Told about every committed write after it commits: a wakeup for live
/// subscribers, who read the changed rows themselves. It must not block.
pub type CommitListener = Arc<dyn Fn(&CommitNotice) + Send + Sync>;

/// One committed write: the conversations it changed and its revision, and
/// the open content it ended, whose live display copies are now obsolete.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommitNotice {
    pub revision: i64,
    /// (conversation id, project path).
    pub conversations: Vec<(String, String)>,
    /// Segments this commit closed or removed: (conversation, turn, segment).
    pub closed_segments: Vec<(String, String, String)>,
    /// Executions this commit settled or removed: (conversation, owning turn).
    pub settled_executions: Vec<(String, String)>,
}

/// The one conversation store of a running Spark app.
pub struct ConversationStore {
    home_dir: PathBuf,
    registry: ProjectRegistry,
    database: ConversationDatabase,
    project_gates: Mutex<HashMap<String, Arc<RwLock<()>>>>,
    pending_deletions: Mutex<HashSet<String>>,
    /// Handles promised to conversations before their first commit.
    reserved_handles: Mutex<HashMap<String, String>>,
    commit_listener: RwLock<Option<CommitListener>>,
    // Declared last so the database closes before the lock is released.
    _home_lock: fs::File,
}

impl ConversationStore {
    /// Lock the home, open its database, and finish startup work before the
    /// app accepts requests: the one-time wipe of pre-SQLite conversations
    /// and any interrupted project deletion cleanup.
    pub fn open(home_dir: impl Into<PathBuf>) -> Result<Arc<Self>> {
        let home_dir = home_dir.into();
        fs::create_dir_all(&home_dir)
            .map_err(|source| StorageError::io("create directory", &home_dir, source))?;
        let lock_path = home_dir.join(SERVER_LOCK_FILE_NAME);
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|source| StorageError::io("open server lock", &lock_path, source))?;
        // A lock description stays held while any process shares it, such as
        // a child forked from this process before it execs, or an app in this
        // process that is still closing. Retry briefly before refusing.
        // ponytail: fixed one-second window; a competing server still fails.
        let mut attempts = 0;
        while lock.try_lock_exclusive().is_err() {
            attempts += 1;
            if attempts >= 50 {
                return Err(StorageError::HomeLocked { path: home_dir });
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let database = ConversationDatabase::open(&home_dir)?;
        let markers = database.read(|read| read.project_deletions())?;
        let store = Self {
            registry: ProjectRegistry::new(home_dir.clone()),
            home_dir,
            database,
            project_gates: Mutex::new(HashMap::new()),
            pending_deletions: Mutex::new(
                markers
                    .iter()
                    .map(|marker| marker.project_path.clone())
                    .collect(),
            ),
            reserved_handles: Mutex::new(HashMap::new()),
            commit_listener: RwLock::new(None),
            _home_lock: lock,
        };
        for marker in markers {
            // A failure keeps the marker; deletion or registration retries it.
            if let Err(error) = store.finish_project_deletion(&marker.project_path) {
                eprintln!(
                    "Unable to finish deleting project {}: {error}",
                    marker.project_path
                );
            }
        }
        // A deferred collection keeps every file; the next startup retries it.
        if let Err(error) = store.collect_tool_output_garbage() {
            eprintln!("Deferred tool output cleanup: {error}");
        }
        let store = Arc::new(store);
        let weak = Arc::downgrade(&store);
        std::thread::spawn(move || loop {
            std::thread::sleep(PASSIVE_CHECKPOINT_INTERVAL);
            let Some(store) = weak.upgrade() else {
                return;
            };
            // Still pending after a failure, so the next tick retries it.
            if let Err(error) = store.database.checkpoint_if_written() {
                eprintln!("Conversation database checkpoint failed: {error}");
            }
        });
        Ok(store)
    }

    /// Recovery collection of tool-output blobs nothing needs: no live row
    /// references them (an old version a commit replaced, a tombstoned
    /// segment's output, or an abandoned publication). Candidates are removed
    /// only behind the cleanup barrier after rechecking their references, so
    /// a commit that references one first keeps it. Returns the number of
    /// removed files.
    #[doc(hidden)]
    pub fn collect_tool_output_garbage(&self) -> Result<usize> {
        // ponytail: scans every conversation directory once per startup;
        // record candidates in a table if that grows slow.
        let mut candidates = Vec::new();
        for project in self.registry.list_project_records()? {
            let dir = self
                .registry
                .project_paths(&project.project_path)?
                .conversations_dir;
            for root in read_dir_or_empty(&dir)? {
                for file in read_dir_or_empty(&root.join(super::tool_output::TOOL_OUTPUT_DIR_NAME))?
                {
                    let path = self.home_relative(&file);
                    if !self.database.read(|read| read.has_live_reference(&path))? {
                        candidates.push((file, path));
                    }
                }
            }
        }
        if candidates.is_empty() {
            return Ok(0);
        }
        self.database.after_checkpoint(|view| {
            let mut removed = 0;
            for (file, path) in &candidates {
                if view.has_live_reference_under(path)? {
                    continue;
                }
                match fs::remove_file(file) {
                    Ok(()) => removed += 1,
                    Err(source) if source.kind() == ErrorKind::NotFound => {}
                    Err(source) => {
                        return Err(StorageError::io("delete tool output", file, source))
                    }
                }
            }
            Ok(removed)
        })
    }

    pub fn home_dir(&self) -> &Path {
        &self.home_dir
    }

    /// Install the app's commit listener, replacing any earlier one. It runs
    /// while the database is held, so it must not reenter the store.
    pub fn set_commit_listener(&self, listener: CommitListener) {
        *self
            .commit_listener
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(listener);
    }

    /// Wake live subscribers about a committed write. Called while the
    /// database is still serialized, so notices follow commit order; the
    /// listener must not block or reenter the store. A lost notice is closed
    /// by the subscriber's heartbeat catch-up.
    pub(crate) fn notify_commit(&self, notice: CommitNotice) {
        let listener = self
            .commit_listener
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        if let Some(listener) = listener {
            listener(&notice);
        }
    }

    pub fn database(&self) -> &ConversationDatabase {
        &self.database
    }

    pub(crate) fn registry(&self) -> &ProjectRegistry {
        &self.registry
    }

    /// Delete a registered project: fence its work, tombstone everything it
    /// owns with a durable cleanup marker in one transaction, then remove its
    /// conversation files once the cleanup barrier allows. `cancel` then interrupts
    /// each tombstoned conversation's running work. A cleanup failure keeps the
    /// marker and is returned; repeating the deletion retries it.
    pub fn delete_project(
        &self,
        deleted: &DeletedProjectRecord,
        cancel: &dyn Fn(&str),
    ) -> Result<()> {
        // Fence first: under the exclusive gate, tombstone everything and
        // record the marker, after which nothing new is admitted.
        let gate = self.project_gate(&deleted.project_path);
        let tombstoned = {
            let _gate = gate.write().unwrap_or_else(|poison| poison.into_inner());
            let marker = ProjectDeletionMarker {
                project_path: deleted.project_path.clone(),
                project_id: deleted.project_id.clone(),
                response: serde_json::to_value(deleted).map_err(|source| {
                    StorageError::JsonWrite {
                        path: PathBuf::from(&deleted.project_path),
                        source,
                    }
                })?,
            };
            let (_, (tombstoned, _)) = self.database.write_if_changed(
                |tx| {
                    let open = open_executions(tx.connection(), &deleted.project_path, None)?;
                    Ok((tx.delete_project(&marker)?, open))
                },
                |revision, (tombstoned, open)| {
                    self.notify_commit(CommitNotice {
                        revision,
                        conversations: tombstoned
                            .iter()
                            .map(|id| (id.clone(), deleted.project_path.clone()))
                            .collect(),
                        closed_segments: Vec::new(),
                        settled_executions: open.clone(),
                    })
                },
            )?;
            self.pending_deletions()
                .insert(deleted.project_path.clone());
            tombstoned
        };
        // Then stop the work that was running. Outside the gate, so a provider
        // blocked on a callback cannot hold up its own interruption; its
        // callbacks fail against the tombstones.
        for conversation_id in &tombstoned {
            cancel(conversation_id);
        }
        let _gate = gate.write().unwrap_or_else(|poison| poison.into_inner());
        self.finish_project_deletion_locked(&deleted.project_path)
    }

    /// The stored response of a project deletion still awaiting cleanup.
    pub fn pending_project_deletion(
        &self,
        project_path: &str,
    ) -> Result<Option<DeletedProjectRecord>> {
        if !self.pending_deletions().contains(project_path) {
            return Ok(None);
        }
        let markers = self.database.read(|read| read.project_deletions())?;
        Ok(markers
            .into_iter()
            .find(|marker| marker.project_path == project_path)
            .and_then(|marker| serde_json::from_value(marker.response).ok()))
    }

    /// Retry a pending deletion's cleanup; `Ok(false)` when none is pending.
    pub fn finish_project_deletion(&self, project_path: &str) -> Result<bool> {
        if !self.pending_deletions().contains(project_path) {
            return Ok(false);
        }
        let gate = self.project_gate(project_path);
        let _gate = gate.write().unwrap_or_else(|poison| poison.into_inner());
        self.finish_project_deletion_locked(project_path)?;
        Ok(true)
    }

    /// Run a project's registration under its lifecycle gate, after
    /// finishing any deletion of the same path still awaiting cleanup, so a
    /// concurrent deletion and the registration never interleave.
    pub fn register_project<T, E: From<StorageError>>(
        &self,
        project_path: &str,
        register: impl FnOnce() -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E> {
        let gate = self.project_gate(project_path);
        let _gate = gate.write().unwrap_or_else(|poison| poison.into_inner());
        if self.pending_deletions().contains(project_path) {
            self.finish_project_deletion_locked(project_path)?;
        }
        register()
    }

    /// Run a change to an existing project's record under its lifecycle
    /// gate. A project whose deletion still awaits cleanup is refused.
    pub fn update_project<T, E: From<StorageError>>(
        &self,
        project_path: &str,
        update: impl FnOnce() -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E> {
        let gate = self.project_gate(project_path);
        let _gate = gate.write().unwrap_or_else(|poison| poison.into_inner());
        self.admit_project(project_path)?;
        update()
    }

    fn finish_project_deletion_locked(&self, project_path: &str) -> Result<()> {
        let markers = self.database.read(|read| read.project_deletions())?;
        let Some(marker) = markers
            .into_iter()
            .find(|marker| marker.project_path == project_path)
        else {
            self.pending_deletions().remove(project_path);
            return Ok(());
        };
        let root = spark_common::project::project_storage_dir(
            self.registry.home_dir(),
            &marker.project_id,
        );
        self.database.after_checkpoint(|view| {
            if view.project_has_live_conversations(project_path)? {
                return Err(StorageError::CleanupDeferred {
                    reason: format!("project {project_path} has live conversations again"),
                });
            }
            // Conversation files are the only project storage removal deletes;
            // memory, work, runs and unknown entries stay, and keep `root`.
            remove_dir_if_exists(
                &root.join("conversations"),
                "delete project conversations directory",
            )?;
            match fs::remove_dir(&root) {
                Err(source)
                    if !matches!(
                        source.kind(),
                        ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty
                    ) =>
                {
                    Err(StorageError::io("delete project directory", &root, source))
                }
                _ => Ok(()),
            }
        })?;
        self.database.remove_project_deletion(project_path)?;
        self.pending_deletions().remove(project_path);
        Ok(())
    }

    /// Reject work for a project whose deletion has not finished cleanup.
    /// Callers hold the project's gate.
    pub(crate) fn admit_project(&self, project_path: &str) -> Result<()> {
        if self.pending_deletions().contains(project_path) {
            return Err(StorageError::ConversationCommitRejected {
                conversation_id: String::new(),
                reason: format!(
                    "Project {project_path} is being deleted; delete it again to finish."
                ),
            });
        }
        Ok(())
    }

    pub(crate) fn project_gate(&self, project_path: &str) -> Arc<RwLock<()>> {
        self.project_gates
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .entry(project_path.to_string())
            .or_default()
            .clone()
    }

    /// The handle reserved for a conversation's first commit, if any.
    pub(crate) fn reserved_handle(&self, conversation_id: &str) -> Option<String> {
        self.reserved_handles
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(conversation_id)
            .cloned()
    }

    pub(crate) fn reserve_handle(&self, conversation_id: &str, handle: &str) {
        self.reserved_handles
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(conversation_id.to_string(), handle.to_string());
    }

    pub(crate) fn release_handle(&self, conversation_id: &str) {
        self.reserved_handles
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(conversation_id);
    }

    /// The directory holding a conversation's blobs.
    pub(crate) fn conversation_root(
        &self,
        project_path: &str,
        conversation_id: &str,
    ) -> Result<PathBuf> {
        Ok(self
            .registry
            .project_paths(project_path)?
            .conversations_dir
            .join(conversation_id))
    }

    /// `path` relative to the Spark home, as stored in blob references.
    pub(crate) fn home_relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.home_dir)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn pending_deletions(&self) -> MutexGuard<'_, HashSet<String>> {
        self.pending_deletions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

impl Drop for ConversationStore {
    fn drop(&mut self) {
        if let Err(error) = self.database.checkpoint_on_shutdown() {
            eprintln!("Conversation database shutdown checkpoint failed: {error}");
        }
    }
}

pub(crate) fn remove_dir_if_exists(path: &Path, action: &'static str) -> Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(()),
        Err(source) => Err(StorageError::io(action, path, source)),
    }
}

/// The entries of `dir`, or none when it does not exist.
pub(crate) fn read_dir_or_empty(dir: &Path) -> Result<Vec<PathBuf>> {
    match fs::read_dir(dir) {
        Ok(entries) => entries
            .map(|entry| {
                entry
                    .map(|entry| entry.path())
                    .map_err(|source| StorageError::io("read directory", dir, source))
            })
            .collect(),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(Vec::new()),
        Err(source) => Err(StorageError::io("read directory", dir, source)),
    }
}

/// The unsettled assistant turns of a project's live conversations, or of
/// one conversation: the executions a deletion ends. (conversation, turn).
pub(crate) fn open_executions(
    connection: &rusqlite::Connection,
    project_path: &str,
    conversation_id: Option<&str>,
) -> Result<Vec<(String, String)>> {
    let mut statement = connection.prepare_cached(
        "SELECT t.conversation_id, t.turn_id FROM conversations c JOIN turns t \
         ON t.conversation_id = c.id WHERE c.project_path = ?1 AND (?2 IS NULL OR c.id = ?2) \
         AND c.deleted = 0 AND t.deleted = 0 AND t.role = 'assistant' AND t.settled = 0",
    )?;
    let rows = statement
        .query_map(rusqlite::params![project_path, conversation_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
