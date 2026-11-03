use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use spark_common::debug::CODEX_JSONRPC_TRACE_FILE_NAME;
use spark_common::events::TurnStreamEvent;
use time::OffsetDateTime;

use crate::conversation::{ConversationStore, EntityRow, ProviderTraceRecord};
use crate::error::{Result, StorageError};
use crate::{append_jsonl_record, ProjectPaths};

pub const CONVERSATION_STATE_SCHEMA_VERSION: i64 = 5;
pub const UNSUPPORTED_CONVERSATION_STATE_SCHEMA: &str =
    "Unsupported conversation state schema. Delete the local conversation and recreate it.";
pub const UNSUPPORTED_CONVERSATION_STATE_SEGMENTS: &str =
    "Unsupported conversation state payload: missing canonical segments. Delete the local conversation and recreate it.";

const HANDLE_ADJECTIVES: &[&str] = &[
    "amber", "ancient", "autumn", "bold", "brisk", "calm", "cedar", "clear", "cloudy", "cobalt",
    "crisp", "curious", "daily", "daring", "deep", "delicate", "eager", "early", "electric",
    "ember", "faint", "fancy", "fast", "fern", "fierce", "final", "forest", "fresh", "gentle",
    "glossy", "golden", "grand", "graphic", "green", "hidden", "hollow", "honest", "icy", "jagged",
    "juniper", "keen", "kind", "lattice", "light", "lively", "lunar", "mellow", "midnight",
    "misty", "modern", "mossy", "navy", "nimble", "noble", "north", "odd", "olive", "open",
    "orange", "patient", "pearl", "pine", "plain", "polished", "prairie", "proud", "quick",
    "quiet", "rapid", "rare", "red", "remote", "river", "robust", "rocky", "royal", "rustic",
    "sage", "scarlet", "shadow", "sharp", "silver", "simple", "sky", "small", "smoky", "solar",
    "solid", "spring", "steady", "stone", "stormy", "summer", "sunny", "swift", "tidy", "timber",
    "tiny", "topaz", "tranquil", "true", "urban", "vivid", "warm", "western", "white", "wild",
    "winter", "wise", "wooden",
];

const HANDLE_NOUNS: &[&str] = &[
    "anchor",
    "antler",
    "arch",
    "arrow",
    "ash",
    "badger",
    "bank",
    "barley",
    "bay",
    "beacon",
    "berry",
    "bird",
    "blossom",
    "bridge",
    "brook",
    "brush",
    "cabin",
    "canyon",
    "cardinal",
    "cedar",
    "circle",
    "cliff",
    "cloud",
    "coast",
    "comet",
    "creek",
    "crest",
    "crow",
    "delta",
    "dove",
    "drift",
    "dune",
    "echo",
    "falcon",
    "field",
    "finch",
    "firefly",
    "fjord",
    "flower",
    "forest",
    "forge",
    "fox",
    "garden",
    "glade",
    "grain",
    "grove",
    "harbor",
    "hawk",
    "hazel",
    "hill",
    "hollow",
    "island",
    "jet",
    "juniper",
    "kingfisher",
    "lake",
    "lantern",
    "leaf",
    "line",
    "lily",
    "meadow",
    "mesa",
    "moon",
    "mountain",
    "otter",
    "owl",
    "peak",
    "pebble",
    "pine",
    "planet",
    "pond",
    "prairie",
    "quartz",
    "raven",
    "reef",
    "ridge",
    "river",
    "robin",
    "sail",
    "sandpiper",
    "shadow",
    "shore",
    "signal",
    "sky",
    "snowflake",
    "sparrow",
    "spring",
    "spruce",
    "star",
    "stone",
    "stream",
    "summit",
    "sunrise",
    "swallow",
    "thicket",
    "thistle",
    "timber",
    "trail",
    "valley",
    "wave",
    "willow",
    "wind",
    "wren",
    "yard",
    "zephyr",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationHandleMatch {
    pub conversation_id: String,
    pub conversation_handle: String,
    pub project_id: String,
    pub project_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawConversationLogLine {
    pub timestamp: String,
    pub direction: String,
    pub line: String,
}

/// Conversation persistence through the app's one [`ConversationStore`].
#[derive(Clone)]
pub struct ConversationRepository {
    store: Arc<ConversationStore>,
}

impl ConversationRepository {
    pub fn new(store: Arc<ConversationStore>) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &Arc<ConversationStore> {
        &self.store
    }

    /// The project's storage paths. Never registers the project or creates
    /// anything: reads must not resurrect a deleted project.
    pub fn project_paths(&self, project_path: &str) -> Result<ProjectPaths> {
        self.store.registry().project_paths(project_path)
    }

    /// The project a live conversation belongs to.
    pub fn conversation_project_path(&self, conversation_id: &str) -> Result<Option<String>> {
        Ok(self
            .store
            .database()
            .read(|read| read.conversation_location(conversation_id))?
            .and_then(|(project_path, deleted)| (!deleted).then_some(project_path)))
    }

    pub fn list_conversation_ids_for_project(&self, project_path: &str) -> Result<Vec<String>> {
        let project_path = self
            .store
            .registry()
            .project_paths(project_path)?
            .project_path;
        self.store
            .database()
            .read(|read| read.conversation_ids(&project_path))
    }

    /// Live conversation summary rows of one project, or of every project,
    /// newest first, from one indexed query.
    pub fn conversation_summaries(&self, project_path: Option<&str>) -> Result<Vec<EntityRow>> {
        let project_path = project_path
            .map(|path| self.store.registry().project_paths(path))
            .transpose()?
            .map(|paths| paths.project_path);
        self.store
            .database()
            .read(|read| read.summaries(project_path.as_deref()))
    }

    /// Live conversations with pending review artifacts, newest first.
    pub fn attention_summaries(&self) -> Result<Vec<EntityRow>> {
        self.store
            .database()
            .read(|read| read.attention_summaries())
    }

    /// Live conversations a previous process may have left in flight:
    /// (conversation id, project path).
    pub fn unsettled_conversations(&self) -> Result<Vec<(String, String)>> {
        self.store
            .database()
            .read(|read| read.unsettled_conversations())
    }

    pub fn conversation_root(
        &self,
        conversation_id: &str,
        project_path: Option<&str>,
    ) -> Result<Option<PathBuf>> {
        let project_path = match project_path.and_then(non_empty_str) {
            Some(project_path) => project_path.to_string(),
            None => match self.conversation_project_path(conversation_id)? {
                Some(project_path) => project_path,
                None => return Ok(None),
            },
        };
        self.store
            .conversation_root(&project_path, conversation_id)
            .map(Some)
    }

    pub fn conversation_codex_jsonrpc_trace_path(
        &self,
        conversation_id: &str,
        project_path: Option<&str>,
    ) -> Result<Option<PathBuf>> {
        Ok(self
            .conversation_root(conversation_id, project_path)?
            .map(|root| root.join(CODEX_JSONRPC_TRACE_FILE_NAME)))
    }

    /// The legacy full snapshot of a live conversation, from its loaded state.
    /// `None` when it is unknown, deleted, or bound to another project.
    pub fn read_snapshot(
        &self,
        conversation_id: &str,
        project_path: Option<&str>,
    ) -> Result<Option<Value>> {
        let expected = project_path
            .and_then(non_empty_str)
            .map(|path| self.store.registry().project_paths(path))
            .transpose()?
            .map(|paths| paths.project_path);
        self.store.with_conversation(conversation_id, |loaded| {
            Ok(loaded
                .as_ref()
                .filter(|loaded| {
                    expected
                        .as_deref()
                        .is_none_or(|expected| loaded.record.meta.project_path == expected)
                })
                .map(|loaded| crate::conversation::snapshot_from_record(&loaded.record)))
        })
    }

    /// `read_snapshot` for a known project.
    pub fn read_existing_snapshot(
        &self,
        conversation_id: &str,
        project_path: &str,
    ) -> Result<Option<Value>> {
        self.read_snapshot(conversation_id, Some(project_path))
    }

    /// Read one segment's externalized tool output. Returns `None` when the
    /// segment's output is stored inline or the conversation is unknown. A
    /// segment that holds only a preview whose full output is missing is an
    /// error (`NotFound` I/O), never its preview passed off as complete.
    pub fn read_segment_tool_output(
        &self,
        conversation_id: &str,
        project_path: Option<&str>,
        segment_id: &str,
    ) -> Result<Option<String>> {
        let Some(snapshot) = self.read_snapshot(conversation_id, project_path)? else {
            return Ok(None);
        };
        let Some(tool_call) = snapshot["segments"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|segment| segment["id"] == segment_id)
            .map(|segment| &segment["tool_call"])
        else {
            return Ok(None);
        };
        let missing = |path: PathBuf| {
            StorageError::io(
                "read segment tool output",
                path,
                std::io::Error::from(std::io::ErrorKind::NotFound),
            )
        };
        let Some(artifact) = tool_call["output_artifact"]
            .as_str()
            .and_then(crate::conversation::safe_tool_output_artifact)
        else {
            return if tool_call["output_truncated"] == true {
                Err(missing(PathBuf::from(segment_id)))
            } else {
                Ok(None)
            };
        };
        let Some(root) =
            self.conversation_root(conversation_id, snapshot["project_path"].as_str())?
        else {
            return Ok(None);
        };
        let path = root.join(artifact);
        match fs::read_to_string(&path) {
            Ok(output) => Ok(Some(output)),
            Err(source) => Err(StorageError::io("read segment tool output", &path, source)),
        }
    }

    pub fn append_codex_jsonrpc_trace(
        &self,
        conversation_id: &str,
        project_path: &str,
        direction: &str,
        line: &str,
    ) -> Result<()> {
        let path = self
            .store
            .conversation_root(project_path, conversation_id)?
            .join(CODEX_JSONRPC_TRACE_FILE_NAME);
        append_jsonl_record(
            path,
            &RawConversationLogLine {
                timestamp: iso_now(),
                direction: direction.to_string(),
                line: line.to_string(),
            },
        )
    }

    pub fn read_codex_jsonrpc_trace(
        &self,
        conversation_id: &str,
        project_path: &str,
    ) -> Result<Vec<RawConversationLogLine>> {
        let path = self
            .store
            .conversation_root(project_path, conversation_id)?
            .join(CODEX_JSONRPC_TRACE_FILE_NAME);
        match crate::read_jsonl(path, crate::JsonLinesOptions::allow_blank_lines()) {
            Ok(records) => Ok(records),
            Err(StorageError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                Ok(Vec::new())
            }
            Err(error) => Err(error),
        }
    }

    /// Append and sync one normalized provider event to the conversation's
    /// provider trace before it is projected. Oversized tool output moves to
    /// an immutable blob first. Fails for a conversation that is not live, so
    /// a late provider callback cannot recreate deleted state.
    pub fn append_provider_event(
        &self,
        conversation_id: &str,
        project_path: &str,
        turn_id: &str,
        event: &mut TurnStreamEvent,
    ) -> Result<ProviderTraceRecord> {
        let project_path = self
            .store
            .registry()
            .project_paths(project_path)?
            .project_path;
        let gate = self.store.project_gate(&project_path);
        let _gate = gate.read().unwrap_or_else(|poison| poison.into_inner());
        self.store.admit_project(&project_path)?;
        let root = self
            .store
            .conversation_root(&project_path, conversation_id)?;
        self.store.with_conversation(conversation_id, |loaded| {
            let Some(loaded) = loaded
                .as_mut()
                .filter(|loaded| loaded.record.meta.project_path == project_path)
            else {
                return Err(StorageError::ConversationCommitRejected {
                    conversation_id: conversation_id.to_string(),
                    reason: "conversation does not exist or was deleted".to_string(),
                });
            };
            if matches!(
                event.kind,
                spark_common::events::TurnStreamEventKind::ToolCallCompleted
                    | spark_common::events::TurnStreamEventKind::ToolCallFailed
            ) {
                if let Some(artifact_id) = event.tool_call.as_ref().map(|tool_call| {
                    spark_common::segments::tool_segment_id(turn_id, event, tool_call)
                }) {
                    if let Some(tool_call) = event.tool_call.as_mut().and_then(Value::as_object_mut)
                    {
                        crate::conversation::externalize_conversation_tool_output(
                            &root,
                            &artifact_id,
                            tool_call,
                        )?;
                    }
                }
            }
            let record = ProviderTraceRecord {
                sequence: loaded.next_trace_sequence,
                turn_id: turn_id.to_string(),
                recorded_at: iso_now(),
                event: serde_json::to_value(&*event).map_err(|source| StorageError::JsonWrite {
                    path: root.clone(),
                    source,
                })?,
            };
            if let Err(error) = crate::conversation::append_trace(&root, &record) {
                // The append may or may not have landed: rescan before
                // allocating again so no sequence is reused.
                if let Ok(records) = crate::conversation::read_trace(&root) {
                    let max = records
                        .iter()
                        .map(|record| record.sequence)
                        .max()
                        .unwrap_or(0);
                    loaded.next_trace_sequence = loaded.next_trace_sequence.max(max + 1);
                }
                return Err(error);
            }
            loaded.next_trace_sequence = record.sequence + 1;
            loaded
                .trace_last_by_turn
                .insert(turn_id.to_string(), record.sequence);
            Ok(record)
        })
    }

    /// The retained provider events of `turn_id` its committed state has not
    /// projected yet, in sequence order, for crash recovery.
    pub fn unapplied_provider_events(
        &self,
        conversation_id: &str,
        project_path: &str,
        turn_id: &str,
    ) -> Result<Vec<ProviderTraceRecord>> {
        let root = self
            .store
            .conversation_root(project_path, conversation_id)?;
        let applied = self.store.with_conversation(conversation_id, |loaded| {
            Ok(loaded
                .as_ref()
                .and_then(|loaded| loaded.applied_trace.get(turn_id).copied())
                .unwrap_or(0))
        })?;
        let mut records: Vec<_> = crate::conversation::read_trace(&root)?
            .into_iter()
            .filter(|record| record.turn_id == turn_id && record.sequence > applied)
            .collect();
        records.sort_by_key(|record| record.sequence);
        Ok(records)
    }

    /// Tombstone one conversation and everything it owns, then remove its
    /// directory once the cleanup barrier allows. A deferred or failed
    /// cleanup leaves unreferenced garbage, never a live reference.
    pub fn delete_conversation(&self, conversation_id: &str, project_path: &str) -> Result<()> {
        let project_paths = self.store.registry().project_paths(project_path)?;
        let gate = self.store.project_gate(&project_paths.project_path);
        let _gate = gate.read().unwrap_or_else(|poison| poison.into_inner());
        let root = project_paths.conversations_dir.join(conversation_id);
        self.store.with_conversation(conversation_id, |loaded| {
            if loaded.is_some() {
                self.store
                    .database()
                    .write(|tx| tx.delete_conversation(conversation_id))?;
                *loaded = None;
            }
            Ok(())
        })?;
        self.store.evict(&[conversation_id.to_string()]);
        let cleanup = self.store.database().after_checkpoint(|view| {
            if view.conversation_is_live(conversation_id)? {
                return Ok(());
            }
            crate::conversation::remove_dir_if_exists(&root, "delete conversation directory")
        });
        if let Err(error) = cleanup {
            eprintln!("Unable to remove deleted conversation {conversation_id}: {error}");
        }
        for path in [
            project_paths
                .flow_run_requests_dir
                .join(format!("{conversation_id}.json")),
            project_paths
                .flow_launches_dir
                .join(format!("{conversation_id}.json")),
            project_paths
                .proposed_plans_dir
                .join(format!("{conversation_id}.json")),
        ] {
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(StorageError::io(
                        "delete conversation sidecar",
                        path,
                        source,
                    ))
                }
            }
        }
        Ok(())
    }

    pub fn find_conversation_by_handle(
        &self,
        handle: &str,
    ) -> Result<Option<ConversationHandleMatch>> {
        let normalized = normalize_conversation_handle(handle);
        if normalized.is_empty() {
            return Ok(None);
        }
        let Some((conversation_id, project_path)) = self
            .store
            .database()
            .read(|read| read.conversation_by_handle(&normalized))?
        else {
            return Ok(None);
        };
        let project_id = self
            .store
            .registry()
            .project_paths(&project_path)?
            .project_id;
        Ok(Some(ConversationHandleMatch {
            conversation_id,
            conversation_handle: normalized,
            project_id,
            project_path,
        }))
    }

    /// A live conversation's handle, or for a new one the handle reserved for
    /// its first commit: `preferred` when free, else a fresh free one.
    pub fn conversation_handle(
        &self,
        conversation_id: &str,
        preferred: Option<&str>,
    ) -> Result<String> {
        if let Some(handle) = self.store.with_conversation(conversation_id, |loaded| {
            Ok(loaded
                .as_ref()
                .map(|loaded| loaded.record.meta.conversation_handle.clone()))
        })? {
            return Ok(handle);
        }
        let preferred = self
            .store
            .reserved_handle(conversation_id)
            .or(preferred.map(str::to_string));
        let handle = free_handle(&self.store, conversation_id, preferred.as_deref())?;
        self.store.reserve_handle(conversation_id, &handle);
        Ok(handle)
    }
}

/// `preferred` when no other live conversation holds it, else a random free
/// handle.
pub(crate) fn free_handle(
    store: &ConversationStore,
    conversation_id: &str,
    preferred: Option<&str>,
) -> Result<String> {
    let preferred = normalize_conversation_handle(preferred.unwrap_or(""));
    let free = |candidate: &str| -> Result<bool> {
        Ok(store
            .database()
            .read(|read| read.conversation_by_handle(candidate))?
            .is_none_or(|(owner, _)| owner == conversation_id))
    };
    if !preferred.is_empty() && free(&preferred)? {
        return Ok(preferred);
    }
    for _ in 0..2048 {
        let candidate = generate_conversation_handle();
        if free(&candidate)? {
            return Ok(candidate);
        }
    }
    Err(StorageError::InvalidRepositoryPath {
        path: store.home_dir().to_path_buf(),
        reason: "Could not allocate a unique conversation handle.".to_string(),
    })
}

pub fn normalize_conversation_handle(value: &str) -> String {
    let trimmed = value.trim().to_lowercase();
    if trimmed.is_empty() {
        return String::new();
    }
    let Some((left, right)) = trimmed.split_once('-') else {
        return String::new();
    };
    if left.is_empty()
        || right.is_empty()
        || right.contains('-')
        || !left.chars().all(char::is_alphabetic)
        || !right.chars().all(char::is_alphabetic)
    {
        return String::new();
    }
    format!("{left}-{right}")
}

fn generate_conversation_handle() -> String {
    let mut rng = rand::thread_rng();
    let adjective = HANDLE_ADJECTIVES
        .choose(&mut rng)
        .copied()
        .unwrap_or("amber");
    let noun = HANDLE_NOUNS.choose(&mut rng).copied().unwrap_or("anchor");
    format!("{adjective}-{noun}")
}

fn non_empty_str(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn iso_now() -> String {
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
