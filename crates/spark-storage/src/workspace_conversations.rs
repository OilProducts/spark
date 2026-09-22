use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use spark_common::debug::CODEX_JSONRPC_TRACE_FILE_NAME;
use spark_common::events::{TurnStreamEvent, TurnStreamEventKind};
use time::OffsetDateTime;

use crate::error::{Result, StorageError};
use crate::{
    append_jsonl_record, write_json_atomic, JsonWriteOptions, ProjectPaths, ProjectRegistry,
};

pub const CONVERSATION_STATE_SCHEMA_VERSION: i64 = 5;
pub const CONVERSATION_HANDLE_SCHEMA_VERSION: i64 = 1;
pub const CONVERSATION_HANDLE_PATTERN: &str = "adjective-noun";
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationHandleRecord {
    pub conversation_id: String,
    pub project_id: String,
    pub project_path: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationHandleMatch {
    pub conversation_id: String,
    pub conversation_handle: String,
    pub project_id: String,
    pub project_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationHandleRepository {
    home_dir: PathBuf,
}

impl ConversationHandleRepository {
    pub fn new(home_dir: impl Into<PathBuf>) -> Self {
        Self {
            home_dir: home_dir.into(),
        }
    }

    pub fn conversation_handles_path(&self) -> PathBuf {
        self.home_dir.join("workspace/conversation-handles.json")
    }

    pub fn load(&self) -> Result<Value> {
        let path = self.conversation_handles_path();
        let payload = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str::<Value>(&text).unwrap_or_else(|_| default_index()),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => default_index(),
            Err(source) => {
                return Err(StorageError::io(
                    "read conversation handle index",
                    &path,
                    source,
                ))
            }
        };
        Ok(normalize_index_payload(payload))
    }

    pub fn write(&self, payload: &Value) -> Result<()> {
        write_json_atomic(
            self.conversation_handles_path(),
            &normalize_index_payload(payload.clone()),
            JsonWriteOptions::default(),
        )
    }

    pub fn ensure_conversation_handle(
        &self,
        conversation_id: &str,
        project_id: &str,
        project_path: &str,
        created_at: &str,
        preferred_handle: Option<&str>,
    ) -> Result<String> {
        let mut payload = self.load()?;
        let Some(object) = payload.as_object_mut() else {
            return Err(StorageError::InvalidDocumentShape {
                path: self.conversation_handles_path(),
                format: "JSON",
                expected: "object",
            });
        };

        let existing_handle = object
            .get("conversation_ids")
            .and_then(Value::as_object)
            .and_then(|conversation_ids| conversation_ids.get(conversation_id))
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(existing_handle) = existing_handle {
            if object
                .get("handles")
                .and_then(Value::as_object)
                .and_then(|handles| handles.get(&existing_handle))
                .and_then(Value::as_object)
                .is_some()
            {
                return Ok(existing_handle);
            }
        }

        let normalized_preferred = normalize_conversation_handle(preferred_handle.unwrap_or(""));
        if !normalized_preferred.is_empty()
            && !object
                .get("handles")
                .and_then(Value::as_object)
                .map(|handles| handles.contains_key(&normalized_preferred))
                .unwrap_or(false)
        {
            insert_handle_record(
                object,
                &normalized_preferred,
                conversation_id,
                project_id,
                project_path,
                created_at,
            );
            self.write(&payload)?;
            return Ok(normalized_preferred);
        }

        for _ in 0..2048 {
            let candidate = generate_conversation_handle();
            if object
                .get("handles")
                .and_then(Value::as_object)
                .map(|handles| handles.contains_key(&candidate))
                .unwrap_or(false)
            {
                continue;
            }
            insert_handle_record(
                object,
                &candidate,
                conversation_id,
                project_id,
                project_path,
                created_at,
            );
            self.write(&payload)?;
            return Ok(candidate);
        }

        Err(StorageError::InvalidRepositoryPath {
            path: self.conversation_handles_path(),
            reason: "Could not allocate a unique conversation handle.".to_string(),
        })
    }

    pub fn find_conversation_by_handle(
        &self,
        handle: &str,
    ) -> Result<Option<ConversationHandleMatch>> {
        let normalized = normalize_conversation_handle(handle);
        if normalized.is_empty() {
            return Ok(None);
        }
        let payload = self.load()?;
        let Some(entry) = payload
            .get("handles")
            .and_then(Value::as_object)
            .and_then(|handles| handles.get(&normalized))
            .and_then(Value::as_object)
        else {
            return Ok(None);
        };
        let Some(conversation_id) = entry.get("conversation_id").and_then(Value::as_str) else {
            return Ok(None);
        };
        let Some(project_id) = entry.get("project_id").and_then(Value::as_str) else {
            return Ok(None);
        };
        let Some(project_path) = entry.get("project_path").and_then(Value::as_str) else {
            return Ok(None);
        };
        Ok(Some(ConversationHandleMatch {
            conversation_id: conversation_id.to_string(),
            conversation_handle: normalized,
            project_id: project_id.to_string(),
            project_path: project_path.to_string(),
        }))
    }

    pub fn remove_conversation_handle(&self, conversation_id: &str) -> Result<()> {
        let mut payload = self.load()?;
        let Some(object) = payload.as_object_mut() else {
            return Ok(());
        };
        let existing_handle = object
            .get_mut("conversation_ids")
            .and_then(Value::as_object_mut)
            .and_then(|conversation_ids| conversation_ids.remove(conversation_id))
            .and_then(|value| value.as_str().map(str::to_string));
        if let Some(existing_handle) = existing_handle {
            if let Some(handles) = object.get_mut("handles").and_then(Value::as_object_mut) {
                handles.remove(&existing_handle);
            }
            self.write(&payload)?;
        }
        Ok(())
    }

    pub fn remove_project_conversation_handles(&self, project_id: &str) -> Result<()> {
        let mut payload = self.load()?;
        let Some(object) = payload.as_object_mut() else {
            return Ok(());
        };
        let Some(handles) = object.get_mut("handles").and_then(Value::as_object_mut) else {
            return Ok(());
        };

        let mut removed_handles = Vec::new();
        let mut removed_conversation_ids = Vec::new();
        for (handle, record) in handles.iter() {
            let matches_project = record
                .as_object()
                .and_then(|entry| entry.get("project_id"))
                .and_then(Value::as_str)
                .map(|value| value == project_id)
                .unwrap_or(false);
            if !matches_project {
                continue;
            }
            if let Some(conversation_id) = record
                .as_object()
                .and_then(|entry| entry.get("conversation_id"))
                .and_then(Value::as_str)
            {
                removed_conversation_ids.push(conversation_id.to_string());
            }
            removed_handles.push(handle.clone());
        }
        if removed_handles.is_empty() {
            return Ok(());
        }
        for handle in removed_handles {
            handles.remove(&handle);
        }
        if let Some(conversation_ids) = object
            .get_mut("conversation_ids")
            .and_then(Value::as_object_mut)
        {
            for conversation_id in removed_conversation_ids {
                conversation_ids.remove(&conversation_id);
            }
        }
        self.write(&payload)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawConversationLogLine {
    pub timestamp: String,
    pub direction: String,
    pub line: String,
}

#[derive(Debug, Clone)]
pub struct ConversationRepository {
    home_dir: PathBuf,
    registry: ProjectRegistry,
    activities: Arc<Mutex<HashMap<PathBuf, crate::ActivityRepository>>>,
}

impl ConversationRepository {
    pub fn new(home_dir: impl Into<PathBuf>) -> Self {
        let home_dir = home_dir.into();
        Self {
            registry: ProjectRegistry::new(home_dir.clone()),
            home_dir,
            activities: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn handle_repository(&self) -> ConversationHandleRepository {
        ConversationHandleRepository::new(self.home_dir.clone())
    }

    pub fn project_paths(&self, project_path: &str) -> Result<ProjectPaths> {
        self.registry.ensure_project_paths(project_path)
    }

    pub fn project_paths_for_conversation(
        &self,
        conversation_id: &str,
        project_path: Option<&str>,
    ) -> Result<Option<ProjectPaths>> {
        if let Some(project_path) = project_path.and_then(non_empty_str) {
            return self.registry.ensure_project_paths(project_path).map(Some);
        }

        let projects_root = self.registry.projects_root();
        let entries = match fs::read_dir(&projects_root) {
            Ok(entries) => entries,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(StorageError::io(
                    "read workspace projects directory",
                    &projects_root,
                    source,
                ))
            }
        };

        let mut candidates = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| {
                StorageError::io("read workspace projects directory", &projects_root, source)
            })?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(project_id) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let Some(project_paths) = self.registry.read_project_paths_by_id(project_id)? else {
                continue;
            };
            if project_paths
                .conversations_dir
                .join(conversation_id)
                .exists()
            {
                candidates.push(project_paths);
            }
        }

        match candidates.len() {
            0 => Ok(None),
            1 => Ok(candidates.pop()),
            _ => Err(StorageError::InvalidRepositoryPath {
                path: PathBuf::from(conversation_id),
                reason: format!("Conversation id is ambiguous across projects: {conversation_id}"),
            }),
        }
    }

    pub fn list_conversation_ids_for_project(&self, project_path: &str) -> Result<Vec<String>> {
        let project_paths = self.registry.ensure_project_paths(project_path)?;
        let entries = match fs::read_dir(&project_paths.conversations_dir) {
            Ok(entries) => entries,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(StorageError::io(
                    "read conversations directory",
                    &project_paths.conversations_dir,
                    source,
                ))
            }
        };
        let mut conversation_ids = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| {
                StorageError::io(
                    "read conversations directory",
                    &project_paths.conversations_dir,
                    source,
                )
            })?;
            if !entry.path().is_dir() {
                continue;
            }
            if let Some(conversation_id) = entry.file_name().to_str().and_then(non_empty_str) {
                conversation_ids.push(conversation_id.to_string());
            }
        }
        conversation_ids.sort();
        Ok(conversation_ids)
    }

    pub fn conversation_root(
        &self,
        conversation_id: &str,
        project_path: Option<&str>,
    ) -> Result<Option<PathBuf>> {
        Ok(self
            .project_paths_for_conversation(conversation_id, project_path)?
            .map(|paths| paths.conversations_dir.join(conversation_id)))
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

    pub fn conversation_session_path(
        &self,
        conversation_id: &str,
        project_path: Option<&str>,
    ) -> Result<Option<PathBuf>> {
        Ok(self
            .conversation_root(conversation_id, project_path)?
            .map(|root| root.join(crate::conversation::RUNTIME_SESSION_FILE_NAME)))
    }

    pub fn read_snapshot(
        &self,
        conversation_id: &str,
        project_path: Option<&str>,
    ) -> Result<Option<Value>> {
        let Some(project_paths) =
            self.project_paths_for_conversation(conversation_id, project_path)?
        else {
            return Ok(None);
        };
        let record_paths = crate::conversation::ConversationRecordPaths::new(
            project_paths.conversations_dir.join(conversation_id),
        );
        if !record_paths.conversation_json().exists() {
            return Ok(None);
        }
        let Some(mut record) = crate::conversation::read_record(&record_paths)? else {
            return Ok(None);
        };
        let activity = crate::ActivityRepository::new(record_paths.root());
        let mut snapshot = crate::conversation::snapshot_from_record(&record);
        // Replay the immutable provider authority on every reopen. Stable ids
        // and equality checks make current projections mutation-free while
        // allowing older malformed terminal projections to converge.
        for provider_record in activity.read_events()? {
            let Some(turn_id) = provider_record.event.get("turn_id").and_then(Value::as_str) else {
                continue;
            };
            let Some(event) = provider_record.event.get("event") else {
                continue;
            };
            let Ok(event) = serde_json::from_value::<TurnStreamEvent>(event.clone()) else {
                continue;
            };
            if !matches!(
                event.kind,
                TurnStreamEventKind::ContentCompleted
                    | TurnStreamEventKind::ToolCallCompleted
                    | TurnStreamEventKind::ToolCallFailed
                    | TurnStreamEventKind::ContextCompactionCompleted
                    | TurnStreamEventKind::Error
                    | TurnStreamEventKind::TurnCompleted
            ) {
                continue;
            }
            if event.kind == TurnStreamEventKind::TurnCompleted {
                let successful = !matches!(
                    event.status.as_deref(),
                    Some("failed" | "canceled" | "cancelled")
                ) && snapshot
                    .get("turns")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .find(|turn| turn.get("id").and_then(Value::as_str) == Some(turn_id))
                    .and_then(|turn| turn.get("status"))
                    .and_then(Value::as_str)
                    != Some("failed");
                let corrected = spark_common::segments::finalize_turn_segments(
                    &mut snapshot,
                    turn_id,
                    successful,
                    &provider_record.committed_at,
                );
                if !corrected.is_empty() {
                    let mutations = corrected
                        .into_iter()
                        .map(|segment| {
                            serde_json::from_value(segment)
                                .map(|segment| crate::conversation::ConversationMutation::RecoveredSegmentUpserted {
                                    segment,
                                    source_event_sequence: provider_record.sequence,
                                })
                        })
                        .collect::<std::result::Result<Vec<_>, _>>()
                        .map_err(|source| StorageError::JsonRead {
                            path: activity.transcript_path(),
                            source,
                        })?;
                    let commit = self.commit_conversation(
                        conversation_id,
                        &record.meta.project_path,
                        record.meta.revision,
                        mutations,
                    )?;
                    record = commit.record;
                    snapshot = commit.snapshot;
                }
            }
            let Some(segment) = spark_common::segments::materialize_segment_for_event(
                &mut snapshot,
                turn_id,
                &event,
                &provider_record.committed_at,
                Some(provider_record.sequence),
            ) else {
                continue;
            };
            let segment: crate::conversation::TranscriptSegment =
                serde_json::from_value(segment).map_err(|source| StorageError::JsonRead {
                    path: activity.transcript_path(),
                    source,
                })?;
            if record.transcript.find_segment(&segment.id).is_some() {
                continue;
            }
            let commit = self.commit_conversation(
                conversation_id,
                &record.meta.project_path,
                record.meta.revision,
                vec![
                    crate::conversation::ConversationMutation::RecoveredSegmentUpserted {
                        segment,
                        source_event_sequence: provider_record.sequence,
                    },
                ],
            )?;
            record = commit.record;
            snapshot = commit.snapshot;
        }
        Ok(Some(snapshot))
    }

    pub(crate) fn read_snapshot_without_recovery(
        &self,
        conversation_id: &str,
        project_path: Option<&str>,
    ) -> Result<Option<Value>> {
        let Some(project_paths) =
            self.project_paths_for_conversation(conversation_id, project_path)?
        else {
            return Ok(None);
        };
        let paths = crate::conversation::ConversationRecordPaths::new(
            project_paths.conversations_dir.join(conversation_id),
        );
        Ok(crate::conversation::read_record(&paths)?
            .map(|record| crate::conversation::snapshot_from_record(&record)))
    }

    /// Read one segment's externalized tool output. Returns `None` when the
    /// segment has no sidecar file (its output is stored inline) or the id is
    /// not filesystem-safe.
    pub fn read_segment_tool_output(
        &self,
        conversation_id: &str,
        project_path: Option<&str>,
        segment_id: &str,
    ) -> Result<Option<String>> {
        if !crate::conversation::is_safe_segment_file_id(segment_id) {
            return Ok(None);
        }
        let Some(root) = self.conversation_root(conversation_id, project_path)? else {
            return Ok(None);
        };
        let path =
            crate::conversation::ConversationRecordPaths::new(root).tool_output_file(segment_id);
        match fs::read_to_string(&path) {
            Ok(output) => Ok(Some(output)),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
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
        let project_paths = self.registry.ensure_project_paths(project_path)?;
        let path = project_paths
            .conversations_dir
            .join(conversation_id)
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
        let project_paths = self.registry.ensure_project_paths(project_path)?;
        let path = project_paths
            .conversations_dir
            .join(conversation_id)
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

    pub fn append_conversation_event(
        &self,
        conversation_id: &str,
        project_path: &str,
        payload: &Value,
    ) -> Result<()> {
        let payload_type = payload
            .get("type")
            .and_then(Value::as_str)
            .and_then(non_empty_str);
        if event_revision(payload).is_none()
            || payload_type.is_none()
            || payload_type == Some(crate::conversation::TRANSIENT_STREAM_EVENT_TYPE)
        {
            return Ok(());
        }
        let project_paths = self.registry.ensure_project_paths(project_path)?;
        let root = crate::conversation::ConversationRecordPaths::new(
            project_paths.conversations_dir.join(conversation_id),
        );
        crate::ActivityRepository::new(root.root())
            .append_event(payload.clone(), iso_now())
            .map(|_| ())
    }

    /// Persist one normalized provider event before it is projected or published.
    pub fn append_provider_event(
        &self,
        conversation_id: &str,
        project_path: &str,
        turn_id: &str,
        event: &mut TurnStreamEvent,
    ) -> Result<crate::ActivityEvent> {
        let project_paths = self.registry.ensure_project_paths(project_path)?;
        let root = project_paths.conversations_dir.join(conversation_id);
        let activity = self
            .activities
            .lock()
            .map_err(|_| StorageError::InvalidRepositoryPath {
                path: root.clone(),
                reason: "conversation activity cache lock poisoned".to_string(),
            })?
            .entry(root.clone())
            .or_insert_with(|| crate::ActivityRepository::new(root.clone()))
            .clone();
        if matches!(
            event.kind,
            spark_common::events::TurnStreamEventKind::ToolCallCompleted
                | spark_common::events::TurnStreamEventKind::ToolCallFailed
        ) {
            if let Some(artifact_id) = event
                .tool_call
                .as_ref()
                .map(|tool_call| spark_common::segments::tool_segment_id(turn_id, event, tool_call))
            {
                if let Some(tool_call) = event.tool_call.as_mut() {
                    activity.externalize_tool_output(&artifact_id, tool_call)?;
                }
            }
        }
        activity.append_event(
            json!({"type": "provider_event", "turn_id": turn_id, "event": event}),
            iso_now(),
        )
    }

    pub fn read_conversation_events_after(
        &self,
        conversation_id: &str,
        project_path: &str,
        revision: i64,
    ) -> Result<Vec<Value>> {
        let project_paths = self.registry.ensure_project_paths(project_path)?;
        let record_paths = crate::conversation::ConversationRecordPaths::new(
            project_paths.conversations_dir.join(conversation_id),
        );
        let records = crate::ActivityRepository::new(record_paths.root()).read_events()?;
        let mut events = Vec::new();
        for record in records {
            let payload = record.event;
            let Some(event_revision) = event_revision(&payload) else {
                continue;
            };
            if event_revision > revision {
                events.push(payload);
            }
        }
        events.sort_by_key(|event| event_revision(event).unwrap_or(0));
        Ok(events)
    }

    pub fn delete_conversation(&self, conversation_id: &str, project_path: &str) -> Result<()> {
        let project_paths = self.registry.ensure_project_paths(project_path)?;
        let conversation_root = project_paths.conversations_dir.join(conversation_id);
        match fs::remove_dir_all(&conversation_root) {
            Ok(()) => {}
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(StorageError::io(
                    "delete conversation directory",
                    conversation_root,
                    source,
                ))
            }
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
        self.handle_repository()
            .remove_conversation_handle(conversation_id)
    }
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

fn normalize_index_payload(payload: Value) -> Value {
    let mut output = Map::new();
    let object = payload.as_object();
    let handles = object
        .and_then(|payload| payload.get("handles"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let conversation_ids = object
        .and_then(|payload| payload.get("conversation_ids"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    output.insert(
        "schema_version".to_string(),
        json!(CONVERSATION_HANDLE_SCHEMA_VERSION),
    );
    output.insert("pattern".to_string(), json!(CONVERSATION_HANDLE_PATTERN));
    output.insert("handles".to_string(), Value::Object(handles));
    output.insert(
        "conversation_ids".to_string(),
        Value::Object(conversation_ids),
    );
    Value::Object(output)
}

fn default_index() -> Value {
    json!({
        "schema_version": CONVERSATION_HANDLE_SCHEMA_VERSION,
        "pattern": CONVERSATION_HANDLE_PATTERN,
        "handles": {},
        "conversation_ids": {},
    })
}

fn insert_handle_record(
    object: &mut Map<String, Value>,
    handle: &str,
    conversation_id: &str,
    project_id: &str,
    project_path: &str,
    created_at: &str,
) {
    if !object.get("handles").map(Value::is_object).unwrap_or(false) {
        object.insert("handles".to_string(), json!({}));
    }
    if !object
        .get("conversation_ids")
        .map(Value::is_object)
        .unwrap_or(false)
    {
        object.insert("conversation_ids".to_string(), json!({}));
    }
    if let Some(handles) = object.get_mut("handles").and_then(Value::as_object_mut) {
        handles.insert(
            handle.to_string(),
            json!({
                "conversation_id": conversation_id,
                "project_id": project_id,
                "project_path": project_path,
                "created_at": created_at,
            }),
        );
    }
    if let Some(conversation_ids) = object
        .get_mut("conversation_ids")
        .and_then(Value::as_object_mut)
    {
        conversation_ids.insert(conversation_id.to_string(), json!(handle));
    }
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

fn event_revision(payload: &Value) -> Option<i64> {
    match payload.get("revision") {
        Some(Value::Number(number)) => number.as_i64(),
        _ => None,
    }
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
