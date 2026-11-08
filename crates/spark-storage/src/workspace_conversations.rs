use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use rand::seq::SliceRandom;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use spark_common::debug::CODEX_JSONRPC_TRACE_FILE_NAME;
use time::OffsetDateTime;

use crate::conversation::{ConversationRecord, ConversationStore, EntityRow};
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

    /// The full snapshot of a live conversation, from its current rows: only
    /// for explicit history and export consumers. `None` when it is unknown,
    /// deleted, or bound to another project.
    pub fn read_snapshot(
        &self,
        conversation_id: &str,
        project_path: Option<&str>,
    ) -> Result<Option<Value>> {
        let expected = self.expected_project(project_path)?;
        self.store.database().read(|read| {
            Ok(read_record(
                read.connection(),
                conversation_id,
                expected.as_deref(),
                false,
            )?
            .map(|record| crate::conversation::snapshot_from_record(&record)))
        })
    }

    /// The snapshot-shaped head of a live conversation: its metadata, its
    /// pending, streaming and queued turns with their segments and artifacts,
    /// and artifacts awaiting review. Accepting a message or projecting a
    /// running turn reads this, never history. Adds `derived_title` and
    /// `completed_replies` (counted up to 2) for title generation, and
    /// `settings_revision`.
    pub fn read_head(
        &self,
        conversation_id: &str,
        project_path: Option<&str>,
    ) -> Result<Option<Value>> {
        let expected = self.expected_project(project_path)?;
        self.store.database().read(|read| {
            let connection = read.connection();
            let Some(record) = read_record(connection, conversation_id, expected.as_deref(), true)?
            else {
                return Ok(None);
            };
            let mut head = crate::conversation::snapshot_from_record(&record);
            head["derived_title"] = Value::from(crate::conversation::derived_title(
                connection,
                conversation_id,
            )?);
            head["settings_revision"] = Value::from(record.meta.settings_revision);
            head["completed_replies"] = Value::from(crate::conversation::completed_replies(
                connection,
                conversation_id,
            )?);
            Ok(Some(head))
        })
    }

    /// One turn of a live conversation, by id.
    pub fn read_turn(&self, conversation_id: &str, turn_id: &str) -> Result<Option<Value>> {
        self.store.database().read(|read| {
            crate::conversation::payload(
                read.connection(),
                "SELECT payload FROM turns WHERE conversation_id = ?1 AND turn_id = ?2 \
                 AND deleted = 0",
                [conversation_id, turn_id],
            )
        })
    }

    /// One segment of a live conversation, by id.
    pub fn read_segment(&self, conversation_id: &str, segment_id: &str) -> Result<Option<Value>> {
        self.store.database().read(|read| {
            crate::conversation::payload(
                read.connection(),
                "SELECT payload FROM segments WHERE conversation_id = ?1 AND segment_id = ?2 \
                 AND deleted = 0",
                [conversation_id, segment_id],
            )
        })
    }

    /// One artifact of a live conversation, by collection and id.
    pub fn read_artifact(
        &self,
        conversation_id: &str,
        collection: crate::conversation::ArtifactCollection,
        artifact_id: &str,
    ) -> Result<Option<Value>> {
        self.store.database().read(|read| {
            crate::conversation::payload(
                read.connection(),
                "SELECT payload FROM artifacts WHERE conversation_id = ?1 AND collection = ?2 \
                 AND artifact_id = ?3 AND deleted = 0",
                [conversation_id, collection.key(), artifact_id],
            )
        })
    }

    /// Re-deliver a conversation's summary row to summary subscribers, when
    /// something outside the database decorates it (a standing role's state).
    /// A conversation with no row yet is created, so its summary is a SQL row.
    pub fn touch_conversation(&self, conversation_id: &str, project_path: &str) -> Result<()> {
        self.operate(
            conversation_id,
            Some(project_path),
            crate::conversation::Open::Create,
            |op| {
                op.touch();
                Ok(())
            },
        )?;
        Ok(())
    }

    fn expected_project(&self, project_path: Option<&str>) -> Result<Option<String>> {
        Ok(project_path
            .and_then(non_empty_str)
            .map(|path| self.store.registry().project_paths(path))
            .transpose()?
            .map(|paths| paths.project_path))
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
        let expected = self.expected_project(project_path)?;
        let Some(project_path) = self.conversation_project_path(conversation_id)? else {
            return Ok(None);
        };
        if expected.is_some_and(|expected| expected != project_path) {
            return Ok(None);
        }
        let Some(segment) = self.read_segment(conversation_id, segment_id)? else {
            return Ok(None);
        };
        let tool_call = &segment["tool_call"];
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
        let path = self
            .store
            .conversation_root(&project_path, conversation_id)?
            .join(artifact);
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

    /// Tombstone one conversation and everything it owns, then remove its
    /// directory once the cleanup barrier allows. A deferred or failed
    /// cleanup leaves unreferenced garbage, never a live reference.
    pub fn delete_conversation(&self, conversation_id: &str, project_path: &str) -> Result<()> {
        let project_paths = self.store.registry().project_paths(project_path)?;
        let gate = self.store.project_gate(&project_paths.project_path);
        let _gate = gate.read().unwrap_or_else(|poison| poison.into_inner());
        let root = project_paths.conversations_dir.join(conversation_id);
        self.store.database().write_if_changed(
            |tx| {
                let live = tx.connection().query_row(
                    "SELECT EXISTS(SELECT 1 FROM conversations WHERE id = ?1 \
                         AND project_path = ?2 AND deleted = 0)",
                    [conversation_id, project_paths.project_path.as_str()],
                    |row| row.get::<_, bool>(0),
                )?;
                if !live {
                    return Ok(Vec::new());
                }
                let open = crate::conversation::open_executions(
                    tx.connection(),
                    &project_paths.project_path,
                    Some(conversation_id),
                )?;
                tx.delete_conversation(conversation_id)?;
                Ok(open)
            },
            |revision, open| {
                self.store.notify_commit(crate::conversation::CommitNotice {
                    revision,
                    conversations: vec![(
                        conversation_id.to_string(),
                        project_paths.project_path.clone(),
                    )],
                    closed_segments: Vec::new(),
                    settled_executions: open.clone(),
                })
            },
        )?;
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

    /// A live conversation's handle.
    pub fn find_conversation_handle(&self, conversation_id: &str) -> Result<Option<String>> {
        self.store.database().read(|read| {
            Ok(read
                .connection()
                .query_row(
                    "SELECT handle FROM conversations WHERE id = ?1 AND deleted = 0",
                    [conversation_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?)
        })
    }

    /// A live conversation's handle, or for a new one the handle reserved for
    /// its first commit: `preferred` when free, else a fresh free one.
    pub fn conversation_handle(
        &self,
        conversation_id: &str,
        preferred: Option<&str>,
    ) -> Result<String> {
        self.store.database().read(|read| {
            let connection = read.connection();
            if let Some(handle) = connection
                .query_row(
                    "SELECT handle FROM conversations WHERE id = ?1 AND deleted = 0",
                    [conversation_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
            {
                return Ok(handle);
            }
            let preferred = self
                .store
                .reserved_handle(conversation_id)
                .or(preferred.map(str::to_string));
            let handle = free_handle(
                connection,
                self.store.home_dir(),
                conversation_id,
                preferred.as_deref(),
            )?;
            self.store.reserve_handle(conversation_id, &handle);
            Ok(handle)
        })
    }
}

/// A live conversation's typed record from its current rows: all of them, or
/// only its head. `None` when it is unknown, deleted or in another project.
fn read_record(
    connection: &Connection,
    conversation_id: &str,
    expected_project: Option<&str>,
    head: bool,
) -> Result<Option<ConversationRecord>> {
    use crate::conversation::{parse_json, payloads};
    let Some((project_path, handle, metadata, revision)) = connection
        .query_row(
            "SELECT project_path, handle, metadata, revision FROM conversations \
             WHERE id = ?1 AND deleted = 0",
            [conversation_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok(None);
    };
    if expected_project.is_some_and(|expected| expected != project_path) {
        return Ok(None);
    }
    let mut meta = crate::conversation::parse_meta(conversation_id, &metadata)?;
    let invalid = |reason: String| StorageError::InvalidConversationState {
        path: PathBuf::from(conversation_id),
        reason,
    };
    meta.conversation_id = conversation_id.to_string();
    meta.project_path = project_path;
    meta.conversation_handle = handle;
    meta.revision = revision;
    let (turns, segments, artifacts) = if head {
        let turns = payloads(
            connection,
            "SELECT payload FROM turns INDEXED BY turns_status \
             WHERE status IN ('pending', 'streaming', 'queued') AND conversation_id = ?1 \
             AND deleted = 0 ORDER BY position, turn_id",
            [conversation_id],
        )?;
        let mut segments = Vec::new();
        let mut artifacts: Vec<(String, String, i64, Value)> = Vec::new();
        let mut artifact_rows = |sql: &str, params: &[&dyn rusqlite::ToSql]| -> Result<()> {
            let mut statement = connection.prepare_cached(sql)?;
            let rows = statement
                .query_map(params, |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<(String, String, i64, String)>>>()?;
            for (collection, id, position, payload) in rows {
                if !artifacts
                    .iter()
                    .any(|(c, i, _, _)| *c == collection && *i == id)
                {
                    artifacts.push((collection, id, position, parse_json(&payload)?));
                }
            }
            Ok(())
        };
        for turn in &turns {
            let turn_id = turn["id"].as_str().unwrap_or_default();
            segments.extend(payloads(
                connection,
                "SELECT payload FROM segments WHERE conversation_id = ?1 AND turn_id = ?2 \
                 AND deleted = 0 ORDER BY sort_order, segment_id",
                [conversation_id, turn_id],
            )?);
            artifact_rows(
                "SELECT collection, artifact_id, position, payload FROM artifacts \
                 INDEXED BY artifacts_owner WHERE conversation_id = ?1 AND turn_id = ?2 \
                 AND deleted = 0",
                &[&conversation_id, &turn_id],
            )?;
        }
        artifact_rows(
            "SELECT collection, artifact_id, position, payload FROM artifacts \
             INDEXED BY artifacts_pending WHERE conversation_id = ?1 \
             AND status IN ('pending', 'pending_review') AND deleted = 0",
            &[&conversation_id],
        )?;
        artifacts.sort_by(|a, b| (&a.0, a.2).cmp(&(&b.0, b.2)));
        (turns, segments, artifacts)
    } else {
        let turns = payloads(
            connection,
            "SELECT payload FROM turns WHERE conversation_id = ?1 AND deleted = 0 \
             ORDER BY position, turn_id",
            [conversation_id],
        )?;
        let segments = payloads(
            connection,
            "SELECT s.payload FROM segments s JOIN turns t \
             ON t.conversation_id = s.conversation_id AND t.turn_id = s.turn_id \
             WHERE s.conversation_id = ?1 AND s.deleted = 0 \
             ORDER BY t.position, s.sort_order, s.segment_id",
            [conversation_id],
        )?;
        let mut statement = connection.prepare_cached(
            "SELECT collection, artifact_id, position, payload FROM artifacts \
             WHERE conversation_id = ?1 AND deleted = 0 ORDER BY collection, position",
        )?;
        let artifacts = statement
            .query_map([conversation_id], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<(String, String, i64, String)>>>()?
            .into_iter()
            .map(|(collection, id, position, payload)| {
                Ok((collection, id, position, parse_json(&payload)?))
            })
            .collect::<Result<Vec<_>>>()?;
        (turns, segments, artifacts)
    };
    let mut record = ConversationRecord::new(conversation_id, &meta.project_path);
    record.meta = meta;
    record.transcript.turns = turns
        .into_iter()
        .map(|turn| {
            serde_json::from_value(turn).map_err(|error| invalid(format!("Invalid turn: {error}")))
        })
        .collect::<Result<_>>()?;
    record.transcript.segments = segments
        .into_iter()
        .map(|segment| {
            serde_json::from_value(segment)
                .map_err(|error| invalid(format!("Invalid segment: {error}")))
        })
        .collect::<Result<_>>()?;
    for (collection, _, _, payload) in artifacts {
        let target = match collection.as_str() {
            crate::conversation::EVENT_LOG_COLLECTION => &mut record.artifacts.event_log,
            "flow_run_requests" => &mut record.artifacts.flow_run_requests,
            "flow_launches" => &mut record.artifacts.flow_launches,
            "run_recoveries" => &mut record.artifacts.run_recoveries,
            "proposed_plans" => &mut record.artifacts.proposed_plans,
            _ => continue,
        };
        target.push(payload);
    }
    Ok(Some(record))
}

/// `preferred` when no other live conversation holds it, else a random free
/// handle.
pub(crate) fn free_handle(
    connection: &Connection,
    home_dir: &std::path::Path,
    conversation_id: &str,
    preferred: Option<&str>,
) -> Result<String> {
    let preferred = normalize_conversation_handle(preferred.unwrap_or(""));
    let free = |candidate: &str| -> Result<bool> {
        let owner: Option<String> = connection
            .query_row(
                "SELECT id FROM conversations WHERE handle = ?1 AND deleted = 0",
                [candidate],
                |row| row.get(0),
            )
            .optional()?;
        Ok(owner.is_none_or(|owner| owner == conversation_id))
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
        path: home_dir.to_path_buf(),
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
