use std::fs;
use std::path::{Path, PathBuf};

use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use spark_common::debug::CODEX_JSONRPC_TRACE_FILE_NAME;
use spark_common::project::normalize_project_path;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationRepository {
    home_dir: PathBuf,
    registry: ProjectRegistry,
}

impl ConversationRepository {
    pub fn new(home_dir: impl Into<PathBuf>) -> Self {
        let home_dir = home_dir.into();
        Self {
            registry: ProjectRegistry::new(home_dir.clone()),
            home_dir,
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

    pub fn conversation_state_path(
        &self,
        conversation_id: &str,
        project_path: Option<&str>,
    ) -> Result<Option<PathBuf>> {
        Ok(self
            .conversation_root(conversation_id, project_path)?
            .map(|root| root.join("state.json")))
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

    pub fn conversation_events_path(
        &self,
        conversation_id: &str,
        project_path: Option<&str>,
    ) -> Result<Option<PathBuf>> {
        Ok(self
            .conversation_root(conversation_id, project_path)?
            .map(|root| root.join("events.jsonl")))
    }

    pub fn conversation_session_path(
        &self,
        conversation_id: &str,
        project_path: Option<&str>,
    ) -> Result<Option<PathBuf>> {
        Ok(self
            .conversation_root(conversation_id, project_path)?
            .map(|root| root.join("session.json")))
    }

    pub fn conversation_keyed_session_path(
        &self,
        conversation_id: &str,
        project_path: Option<&str>,
        provider: &str,
        model: Option<&str>,
    ) -> Result<Option<PathBuf>> {
        let Some(root) = self.conversation_root(conversation_id, project_path)? else {
            return Ok(None);
        };
        let provider = non_empty_str(provider).unwrap_or("codex").to_lowercase();
        let model = model.and_then(non_empty_str).unwrap_or("");
        let digest = sha256_digest_24(&format!("{provider}\0{model}"));
        Ok(Some(
            root.join("sessions")
                .join(format!("{provider}-{digest}.json")),
        ))
    }

    pub fn flow_run_requests_state_path(
        &self,
        conversation_id: &str,
        project_path: Option<&str>,
    ) -> Result<Option<PathBuf>> {
        Ok(self
            .project_paths_for_conversation(conversation_id, project_path)?
            .map(|paths| {
                paths
                    .flow_run_requests_dir
                    .join(format!("{conversation_id}.json"))
            }))
    }

    pub fn flow_launches_state_path(
        &self,
        conversation_id: &str,
        project_path: Option<&str>,
    ) -> Result<Option<PathBuf>> {
        Ok(self
            .project_paths_for_conversation(conversation_id, project_path)?
            .map(|paths| {
                paths
                    .flow_launches_dir
                    .join(format!("{conversation_id}.json"))
            }))
    }

    pub fn proposed_plans_state_path(
        &self,
        conversation_id: &str,
        project_path: Option<&str>,
    ) -> Result<Option<PathBuf>> {
        Ok(self
            .project_paths_for_conversation(conversation_id, project_path)?
            .map(|paths| {
                paths
                    .proposed_plans_dir
                    .join(format!("{conversation_id}.json"))
            }))
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
            if !record_paths.legacy_state_json().exists() {
                return Ok(None);
            }
            crate::conversation::migrate_legacy_conversation(&project_paths, conversation_id)?;
        }
        let Some(record) = crate::conversation::read_record(&record_paths)? else {
            return Ok(None);
        };
        Ok(Some(crate::conversation::snapshot_from_record(&record)))
    }

    pub fn write_snapshot(&self, snapshot: &Value) -> Result<()> {
        let object = snapshot
            .as_object()
            .ok_or_else(|| StorageError::InvalidDocumentShape {
                path: PathBuf::from("conversation snapshot"),
                format: "JSON",
                expected: "object",
            })?;
        let conversation_id = object
            .get("conversation_id")
            .and_then(Value::as_str)
            .and_then(non_empty_str)
            .map(str::to_string)
            .ok_or_else(|| StorageError::InvalidRepositoryPath {
                path: PathBuf::from("conversation snapshot"),
                reason: "Conversation id is required.".to_string(),
            })?;
        let project_path = object
            .get("project_path")
            .and_then(Value::as_str)
            .and_then(normalize_project_path_string)
            .ok_or_else(|| StorageError::InvalidRepositoryPath {
                path: PathBuf::from("conversation snapshot"),
                reason: "Project path is required.".to_string(),
            })?;
        let project_paths = self.registry.ensure_project_paths(&project_path)?;

        let mut core = Map::new();
        for key in [
            "schema_version",
            "revision",
            "conversation_id",
            "conversation_handle",
            "project_path",
            "chat_mode",
            "provider",
            "model",
            "llm_profile",
            "reasoning_effort",
            "title",
            "created_at",
            "updated_at",
            "turns",
            "segments",
        ] {
            if let Some(value) = object.get(key) {
                core.insert(key.to_string(), value.clone());
            }
        }
        core.entry("schema_version".to_string())
            .or_insert_with(|| json!(CONVERSATION_STATE_SCHEMA_VERSION));
        core.entry("revision".to_string())
            .or_insert_with(|| json!(0));
        core.entry("conversation_id".to_string())
            .or_insert_with(|| json!(&conversation_id));
        core.entry("project_path".to_string())
            .or_insert_with(|| json!(&project_path));
        core.entry("chat_mode".to_string())
            .or_insert_with(|| json!("chat"));
        core.entry("provider".to_string())
            .or_insert_with(|| json!("codex"));
        core.entry("model".to_string()).or_insert(Value::Null);
        core.entry("llm_profile".to_string()).or_insert(Value::Null);
        core.entry("reasoning_effort".to_string())
            .or_insert(Value::Null);
        core.entry("title".to_string())
            .or_insert_with(|| json!("New thread"));
        core.entry("created_at".to_string())
            .or_insert_with(|| json!(""));
        core.entry("updated_at".to_string())
            .or_insert_with(|| json!(""));
        core.entry("turns".to_string()).or_insert_with(|| json!([]));
        core.entry("segments".to_string())
            .or_insert_with(|| json!([]));

        let state_path = project_paths
            .conversations_dir
            .join(&conversation_id)
            .join("state.json");
        write_json_atomic(
            &state_path,
            &Value::Object(core),
            JsonWriteOptions::default(),
        )?;

        write_json_atomic(
            project_paths
                .flow_run_requests_dir
                .join(format!("{conversation_id}.json")),
            &json!({
                "conversation_id": &conversation_id,
                "project_id": project_paths.project_id,
                "project_path": &project_path,
                "event_log": array_or_empty(object.get("event_log")),
                "flow_run_requests": array_or_empty(object.get("flow_run_requests")),
            }),
            JsonWriteOptions::default(),
        )?;
        write_json_atomic(
            project_paths
                .flow_launches_dir
                .join(format!("{conversation_id}.json")),
            &json!({
                "conversation_id": &conversation_id,
                "project_id": project_paths.project_id,
                "project_path": &project_path,
                "flow_launches": array_or_empty(object.get("flow_launches")),
                "run_recoveries": array_or_empty(object.get("run_recoveries")),
            }),
            JsonWriteOptions::default(),
        )?;
        write_json_atomic(
            project_paths
                .proposed_plans_dir
                .join(format!("{conversation_id}.json")),
            &json!({
                "conversation_id": &conversation_id,
                "project_id": project_paths.project_id,
                "project_path": &project_path,
                "proposed_plans": array_or_empty(object.get("proposed_plans")),
            }),
            JsonWriteOptions::default(),
        )
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
        let path = crate::conversation::ConversationRecordPaths::new(
            project_paths.conversations_dir.join(conversation_id),
        )
        .journal_jsonl();
        append_jsonl_record(path, payload)
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
        // Committed journal, with a legacy fallback for conversations that
        // have not been read (and therefore migrated) yet.
        let journal_path = record_paths.journal_jsonl();
        let path = if journal_path.exists() {
            journal_path
        } else {
            record_paths.legacy_events_jsonl()
        };
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(StorageError::io("read conversation events", &path, source)),
        };
        let mut events = Vec::new();
        for line in text.lines() {
            let Ok(payload) = serde_json::from_str::<Value>(line) else {
                continue;
            };
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

pub(crate) fn validate_supported_state(path: &Path, payload: &Value) -> Result<()> {
    let Some(object) = payload.as_object() else {
        return Err(invalid_conversation_state(
            path,
            UNSUPPORTED_CONVERSATION_STATE_SCHEMA,
        ));
    };
    if object.get("schema_version").and_then(Value::as_i64)
        != Some(CONVERSATION_STATE_SCHEMA_VERSION)
    {
        return Err(invalid_conversation_state(
            path,
            UNSUPPORTED_CONVERSATION_STATE_SCHEMA,
        ));
    }
    if !matches!(object.get("revision"), Some(Value::Number(number)) if number.as_i64().is_some()) {
        return Err(invalid_conversation_state(
            path,
            UNSUPPORTED_CONVERSATION_STATE_SCHEMA,
        ));
    }
    if !matches!(object.get("segments"), Some(Value::Array(_))) {
        return Err(invalid_conversation_state(
            path,
            UNSUPPORTED_CONVERSATION_STATE_SEGMENTS,
        ));
    }
    Ok(())
}

fn invalid_conversation_state(path: &Path, reason: &str) -> StorageError {
    StorageError::InvalidConversationState {
        path: path.to_path_buf(),
        reason: reason.to_string(),
    }
}

pub(crate) fn merge_sidecars(
    project_paths: &ProjectPaths,
    conversation_id: &str,
    project_path: &str,
    payload: &mut Value,
) {
    let Some(object) = payload.as_object_mut() else {
        return;
    };
    let run_requests = read_json_object_lossy(
        project_paths
            .flow_run_requests_dir
            .join(format!("{conversation_id}.json")),
    );
    if let Some(sidecar) = run_requests {
        let event_log = sidecar
            .get("event_log")
            .cloned()
            .or_else(|| object.get("event_log").cloned())
            .unwrap_or_else(|| json!([]));
        object.insert("event_log".to_string(), event_log);
        object.insert(
            "flow_run_requests".to_string(),
            sidecar
                .get("flow_run_requests")
                .cloned()
                .unwrap_or_else(|| json!([])),
        );
    }
    let launches = read_json_object_lossy(
        project_paths
            .flow_launches_dir
            .join(format!("{conversation_id}.json")),
    );
    if let Some(sidecar) = launches {
        object.insert(
            "flow_launches".to_string(),
            sidecar
                .get("flow_launches")
                .cloned()
                .unwrap_or_else(|| json!([])),
        );
        object.insert(
            "run_recoveries".to_string(),
            sidecar
                .get("run_recoveries")
                .cloned()
                .unwrap_or_else(|| json!([])),
        );
    }
    let proposed = read_json_object_lossy(
        project_paths
            .proposed_plans_dir
            .join(format!("{conversation_id}.json")),
    );
    if let Some(sidecar) = proposed {
        object.insert(
            "proposed_plans".to_string(),
            sidecar
                .get("proposed_plans")
                .cloned()
                .unwrap_or_else(|| json!([])),
        );
    }
    ensure_snapshot_defaults(payload, conversation_id, project_path);
}

fn ensure_snapshot_defaults(payload: &mut Value, conversation_id: &str, project_path: &str) {
    let Some(object) = payload.as_object_mut() else {
        return;
    };
    object
        .entry("schema_version".to_string())
        .or_insert_with(|| json!(CONVERSATION_STATE_SCHEMA_VERSION));
    object
        .entry("revision".to_string())
        .or_insert_with(|| json!(0));
    object
        .entry("conversation_id".to_string())
        .or_insert_with(|| json!(conversation_id));
    object
        .entry("conversation_handle".to_string())
        .or_insert_with(|| json!(""));
    object
        .entry("project_path".to_string())
        .or_insert_with(|| json!(project_path));
    object
        .entry("chat_mode".to_string())
        .or_insert_with(|| json!("chat"));
    object
        .entry("provider".to_string())
        .or_insert_with(|| json!("codex"));
    object.entry("model".to_string()).or_insert(Value::Null);
    object
        .entry("llm_profile".to_string())
        .or_insert(Value::Null);
    object
        .entry("reasoning_effort".to_string())
        .or_insert(Value::Null);
    object
        .entry("title".to_string())
        .or_insert_with(|| json!("New thread"));
    object
        .entry("created_at".to_string())
        .or_insert_with(|| json!(""));
    object
        .entry("updated_at".to_string())
        .or_insert_with(|| json!(""));
    for key in [
        "turns",
        "segments",
        "event_log",
        "flow_run_requests",
        "flow_launches",
        "run_recoveries",
        "proposed_plans",
    ] {
        object.entry(key.to_string()).or_insert_with(|| json!([]));
    }
}

fn read_json_object_lossy(path: impl AsRef<Path>) -> Option<Map<String, Value>> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|value| value.as_object().cloned())
}

fn array_or_empty(value: Option<&Value>) -> Value {
    value
        .filter(|value| value.is_array())
        .cloned()
        .unwrap_or_else(|| json!([]))
}

fn event_revision(payload: &Value) -> Option<i64> {
    match payload.get("revision") {
        Some(Value::Number(number)) => number.as_i64(),
        _ => payload
            .get("state")
            .and_then(Value::as_object)
            .and_then(|state| state.get("revision"))
            .and_then(|value| match value {
                Value::Number(number) => number.as_i64(),
                _ => None,
            }),
    }
}

fn normalize_project_path_string(value: &str) -> Option<String> {
    normalize_project_path(value)
        .ok()
        .flatten()
        .map(|path| path.to_string_lossy().into_owned())
}

fn non_empty_str(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn sha256_digest_24(value: &str) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(value.as_bytes());
    digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
        .chars()
        .take(24)
        .collect()
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
