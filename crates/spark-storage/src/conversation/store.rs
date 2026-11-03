//! The app-owned conversation store: one SQLite database and home lock per
//! Spark home, the loaded conversations' in-memory authority, and the
//! per-project lifecycle gates.
//!
//! Lock order: project lifecycle gate, then one conversation's lock, then the
//! database connection. Reads that only query the database take neither of
//! the first two.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

use fs2::FileExt;

use crate::error::{Result, StorageError};
use crate::{DeletedProjectRecord, ProjectRegistry};

use super::attachments::ConversationAttachment;
use super::database::{ConversationDatabase, ProjectDeletionMarker, StoredConversation};
use super::provider_trace::read_trace;
use super::records::{ConversationArtifacts, ConversationMeta, ConversationRecord, Transcript};

/// Held exclusively for the app's lifetime; never unlinked.
pub const SERVER_LOCK_FILE_NAME: &str = "server.lock";
/// Workflow events are artifact rows in this collection, in insertion order.
pub(crate) const EVENT_LOG_COLLECTION: &str = "event_log";
pub(crate) const ALL_COLLECTIONS: [super::ArtifactCollection; 4] = [
    super::ArtifactCollection::FlowRunRequests,
    super::ArtifactCollection::FlowLaunches,
    super::ArtifactCollection::RunRecoveries,
    super::ArtifactCollection::ProposedPlans,
];
const LEGACY_HANDLE_INDEX: &str = "workspace/conversation-handles.json";
/// How often a passive checkpoint runs when writes occurred.
const PASSIVE_CHECKPOINT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// The one conversation store of a running Spark app.
pub struct ConversationStore {
    home_dir: PathBuf,
    registry: ProjectRegistry,
    database: ConversationDatabase,
    conversations: Mutex<HashMap<String, Arc<Mutex<Option<LoadedConversation>>>>>,
    project_gates: Mutex<HashMap<String, Arc<RwLock<()>>>>,
    pending_deletions: Mutex<HashSet<String>>,
    /// Handles promised to conversations before their first commit.
    reserved_handles: Mutex<HashMap<String, String>>,
    // Declared last so the database closes before the lock is released.
    _home_lock: fs::File,
}

/// One conversation's committed state, published only after its SQL commit.
#[derive(Debug, Clone)]
pub(crate) struct LoadedConversation {
    pub record: ConversationRecord,
    pub positions: HashMap<String, i64>,
    pub next_position: i64,
    pub applied_trace: HashMap<String, u64>,
    pub next_trace_sequence: u64,
    /// Newest durable trace sequence of each turn.
    pub trace_last_by_turn: HashMap<String, u64>,
    pub attachments: BTreeMap<String, StoredAttachment>,
}

impl LoadedConversation {
    pub fn new(conversation_id: &str, project_path: &str) -> Self {
        Self {
            record: ConversationRecord::new(conversation_id, project_path),
            positions: HashMap::new(),
            next_position: 0,
            applied_trace: HashMap::new(),
            next_trace_sequence: 1,
            trace_last_by_turn: HashMap::new(),
            attachments: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StoredAttachment {
    pub record: ConversationAttachment,
    /// Relative to the Spark home.
    pub blob_path: String,
    pub attached: bool,
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
            conversations: Mutex::new(HashMap::new()),
            project_gates: Mutex::new(HashMap::new()),
            pending_deletions: Mutex::new(
                markers
                    .iter()
                    .map(|marker| marker.project_path.clone())
                    .collect(),
            ),
            reserved_handles: Mutex::new(HashMap::new()),
            _home_lock: lock,
        };
        if !store.database.read(|read| read.reset_complete())? {
            store.reset_legacy_conversations()?;
        }
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
    /// segment's output, or an abandoned publication), and no retained trace
    /// record does, so replay can still project them. Holds every
    /// conversation's lock, so no commit or trace append can start
    /// referencing a candidate, and removes candidates only behind the
    /// cleanup barrier after rechecking their references. Returns the number
    /// of removed files.
    #[doc(hidden)]
    pub fn collect_tool_output_garbage(&self) -> Result<usize> {
        // ponytail: scans every conversation directory once per startup;
        // record candidates in a table if that grows slow.
        let mut roots = Vec::new();
        for project in self.registry.list_project_records()? {
            let dir = self
                .registry
                .project_paths(&project.project_path)?
                .conversations_dir;
            for entry in read_dir_or_empty(&dir)? {
                if let Some(id) = entry.file_name().and_then(|name| name.to_str()) {
                    roots.push((id.to_string(), entry.clone()));
                }
            }
        }
        roots.sort();
        let slots: Vec<_> = {
            let mut conversations = self
                .conversations
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            roots
                .iter()
                .map(|(id, _)| conversations.entry(id.clone()).or_default().clone())
                .collect()
        };
        // Sorted by conversation id, per the lock order.
        let _locks: Vec<_> = slots
            .iter()
            .map(|slot| slot.lock().unwrap_or_else(|poison| poison.into_inner()))
            .collect();
        let mut candidates = Vec::new();
        for (_, root) in &roots {
            // An unreadable trace may still need any of its blobs.
            let Ok(trace) = read_trace(root) else {
                continue;
            };
            let replay_needed: HashSet<String> = trace
                .iter()
                .filter_map(|record| record.event.pointer("/tool_call/output_artifact"))
                .filter_map(|artifact| artifact.as_str())
                .map(|artifact| self.home_relative(&root.join(artifact)))
                .collect();
            for file in read_dir_or_empty(&root.join(super::tool_output::TOOL_OUTPUT_DIR_NAME))? {
                let path = self.home_relative(&file);
                if !replay_needed.contains(&path)
                    && !self.database.read(|read| read.has_live_reference(&path))?
                {
                    candidates.push((file, path));
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

    pub fn database(&self) -> &ConversationDatabase {
        &self.database
    }

    pub(crate) fn registry(&self) -> &ProjectRegistry {
        &self.registry
    }

    /// Delete a registered project: fence its work, tombstone everything it
    /// owns with a durable cleanup marker in one transaction, then remove its
    /// directory once the cleanup barrier allows. `cancel` then interrupts
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
            let (_, tombstoned) = self.database.write(|tx| tx.delete_project(&marker))?;
            self.pending_deletions()
                .insert(deleted.project_path.clone());
            self.evict(&tombstoned);
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
        let root = self.registry.projects_root().join(&marker.project_id);
        self.database.after_checkpoint(|view| {
            if view.project_has_live_conversations(project_path)? {
                return Err(StorageError::CleanupDeferred {
                    reason: format!("project {project_path} has live conversations again"),
                });
            }
            remove_dir_if_exists(&root, "delete workspace project directory")
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

    /// Run `apply` on one conversation's loaded state under its lock,
    /// loading it from the database first. The slot stays `None` for an
    /// unknown or deleted conversation.
    pub(crate) fn with_conversation<T>(
        &self,
        conversation_id: &str,
        apply: impl FnOnce(&mut Option<LoadedConversation>) -> Result<T>,
    ) -> Result<T> {
        let slot = self
            .conversations
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .entry(conversation_id.to_string())
            .or_default()
            .clone();
        let mut loaded = slot.lock().unwrap_or_else(|poison| poison.into_inner());
        if loaded.is_none() {
            *loaded = self.load(conversation_id)?;
        }
        apply(&mut loaded)
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

    pub(crate) fn evict(&self, conversation_ids: &[String]) {
        let mut conversations = self
            .conversations
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        for conversation_id in conversation_ids {
            conversations.remove(conversation_id);
        }
    }

    /// The directory holding a conversation's blobs and provider trace.
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

    fn load(&self, conversation_id: &str) -> Result<Option<LoadedConversation>> {
        let Some(stored) = self
            .database
            .read(|read| read.conversation(conversation_id))?
        else {
            return Ok(None);
        };
        let root = self.conversation_root(&stored.project_path, conversation_id)?;
        let trace = read_trace(&root)?;
        let mut loaded = loaded_from_rows(conversation_id, stored)?;
        for record in trace {
            loaded.next_trace_sequence = loaded.next_trace_sequence.max(record.sequence + 1);
            let last = loaded.trace_last_by_turn.entry(record.turn_id).or_default();
            *last = (*last).max(record.sequence);
        }
        Ok(Some(loaded))
    }

    fn reset_legacy_conversations(&self) -> Result<()> {
        for project in self.registry.list_project_records()? {
            let paths = self.registry.project_paths(&project.project_path)?;
            remove_dir_if_exists(&paths.conversations_dir, "delete legacy conversations")?;
        }
        let index = self.home_dir.join(LEGACY_HANDLE_INDEX);
        match fs::remove_file(&index) {
            Ok(()) => {}
            Err(source) if source.kind() == ErrorKind::NotFound => {}
            Err(source) => {
                return Err(StorageError::io(
                    "delete legacy conversation handles",
                    index,
                    source,
                ))
            }
        }
        self.database.mark_reset_complete()
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

fn loaded_from_rows(
    conversation_id: &str,
    stored: StoredConversation,
) -> Result<LoadedConversation> {
    let invalid = |reason: String| StorageError::InvalidConversationState {
        path: PathBuf::from(conversation_id),
        reason,
    };
    let mut meta: ConversationMeta = serde_json::from_value(stored.metadata)
        .map_err(|error| invalid(format!("Invalid conversation metadata: {error}")))?;
    meta.conversation_id = conversation_id.to_string();
    meta.project_path = stored.project_path;
    meta.conversation_handle = stored.handle;
    let mut positions = HashMap::new();
    let mut applied_trace = HashMap::new();
    let mut turns = Vec::with_capacity(stored.turns.len());
    for (turn_id, position, applied, payload) in stored.turns {
        turns.push(
            serde_json::from_value(payload)
                .map_err(|error| invalid(format!("Invalid turn {turn_id}: {error}")))?,
        );
        positions.insert(turn_id.clone(), position);
        applied_trace.insert(turn_id, applied);
    }
    let segments: Vec<super::records::TranscriptSegment> = stored
        .segments
        .into_iter()
        .map(|payload| {
            serde_json::from_value(payload)
                .map_err(|error| invalid(format!("Invalid segment: {error}")))
        })
        .collect::<Result<_>>()?;
    let mut artifacts = ConversationArtifacts::default();
    for (collection, _, payload) in stored.artifacts {
        let target = match collection.as_str() {
            EVENT_LOG_COLLECTION => &mut artifacts.event_log,
            "flow_run_requests" => &mut artifacts.flow_run_requests,
            "flow_launches" => &mut artifacts.flow_launches,
            "run_recoveries" => &mut artifacts.run_recoveries,
            "proposed_plans" => &mut artifacts.proposed_plans,
            other => return Err(invalid(format!("Unknown artifact collection {other}"))),
        };
        target.push(payload);
    }
    let mut attachments = BTreeMap::new();
    for (id, blob_path, status, payload) in stored.attachments {
        let record: ConversationAttachment = serde_json::from_value(payload)
            .map_err(|error| invalid(format!("Invalid attachment {id}: {error}")))?;
        attachments.insert(
            id,
            StoredAttachment {
                record,
                blob_path,
                attached: status == "attached",
            },
        );
    }
    let next_position = positions.values().max().map_or(0, |max| max + 1);
    Ok(LoadedConversation {
        record: ConversationRecord {
            meta,
            transcript: Transcript { turns, segments },
            artifacts,
        },
        positions,
        next_position,
        applied_trace,
        next_trace_sequence: stored.trace_sequence_high_water + 1,
        trace_last_by_turn: HashMap::new(),
        attachments,
    })
}
