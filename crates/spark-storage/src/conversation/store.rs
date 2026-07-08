//! Split-file persistence for conversation records.
//!
//! One conversation directory holds all of its durable authorities:
//! `conversation.json` (metadata + revision cursor), `transcript.json`
//! (turns/segments), `artifacts/<kind>.json` and `event-log.json` (artifact
//! records and workflow events), and `journal.jsonl` (committed mutations).
//! `conversation.json` is written last in a commit so an observed revision
//! always implies durable transcript and artifact state.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::{Result, StorageError};
use crate::{
    read_json_optional, write_json_atomic, JsonWriteOptions, CONVERSATION_STATE_SCHEMA_VERSION,
    UNSUPPORTED_CONVERSATION_STATE_SCHEMA,
};

use super::records::{
    ArtifactCollection, ConversationArtifacts, ConversationMeta, ConversationRecord, Transcript,
};

pub(crate) const CONVERSATION_META_FILE_NAME: &str = "conversation.json";
pub(crate) const TRANSCRIPT_FILE_NAME: &str = "transcript.json";
pub(crate) const EVENT_LOG_FILE_NAME: &str = "event-log.json";
pub(crate) const JOURNAL_FILE_NAME: &str = "journal.jsonl";
pub(crate) const TOOL_OUTPUT_DIR_NAME: &str = "tool-output";
pub(crate) const LEGACY_STATE_FILE_NAME: &str = "state.json";
pub(crate) const LEGACY_EVENTS_FILE_NAME: &str = "events.jsonl";

/// File locations for one conversation's record set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConversationRecordPaths {
    root: PathBuf,
}

impl ConversationRecordPaths {
    pub fn new(conversation_root: impl Into<PathBuf>) -> Self {
        Self {
            root: conversation_root.into(),
        }
    }

    pub fn conversation_json(&self) -> PathBuf {
        self.root.join(CONVERSATION_META_FILE_NAME)
    }

    pub fn transcript_json(&self) -> PathBuf {
        self.root.join(TRANSCRIPT_FILE_NAME)
    }

    pub fn artifact_file(&self, collection: ArtifactCollection) -> PathBuf {
        self.root
            .join("artifacts")
            .join(artifact_file_name(collection))
    }

    pub fn event_log_json(&self) -> PathBuf {
        self.root.join(EVENT_LOG_FILE_NAME)
    }

    pub fn journal_jsonl(&self) -> PathBuf {
        self.root.join(JOURNAL_FILE_NAME)
    }

    pub fn tool_output_file(&self, segment_id: &str) -> PathBuf {
        self.root
            .join(TOOL_OUTPUT_DIR_NAME)
            .join(format!("{segment_id}.txt"))
    }

    pub fn legacy_state_json(&self) -> PathBuf {
        self.root.join(LEGACY_STATE_FILE_NAME)
    }

    pub fn legacy_events_jsonl(&self) -> PathBuf {
        self.root.join(LEGACY_EVENTS_FILE_NAME)
    }
}

fn artifact_file_name(collection: ArtifactCollection) -> &'static str {
    match collection {
        ArtifactCollection::FlowRunRequests => "flow-run-requests.json",
        ArtifactCollection::FlowLaunches => "flow-launches.json",
        ArtifactCollection::RunRecoveries => "run-recoveries.json",
        ArtifactCollection::ProposedPlans => "proposed-plans.json",
    }
}

pub(crate) const ALL_ARTIFACT_COLLECTIONS: [ArtifactCollection; 4] = [
    ArtifactCollection::FlowRunRequests,
    ArtifactCollection::FlowLaunches,
    ArtifactCollection::RunRecoveries,
    ArtifactCollection::ProposedPlans,
];

/// Which record files a commit must rewrite. `everything` is set for newly
/// created (or freshly migrated) conversations.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct RecordWritePlan {
    pub everything: bool,
    pub transcript: bool,
    pub event_log: bool,
    pub collections: Vec<ArtifactCollection>,
}

impl RecordWritePlan {
    pub fn touch_collection(&mut self, collection: ArtifactCollection) {
        if !self.collections.contains(&collection) {
            self.collections.push(collection);
        }
    }

    fn includes_collection(&self, collection: ArtifactCollection) -> bool {
        self.everything || self.collections.contains(&collection)
    }
}

/// Read one conversation's typed record from its split files. Returns `None`
/// when `conversation.json` does not exist. Missing transcript/artifact files
/// default to empty (a crash between record writes must not brick the read).
pub(crate) fn read_record(paths: &ConversationRecordPaths) -> Result<Option<ConversationRecord>> {
    let Some(meta) = read_json_optional::<ConversationMeta>(paths.conversation_json())? else {
        return Ok(None);
    };
    if meta.schema_version != CONVERSATION_STATE_SCHEMA_VERSION {
        return Err(StorageError::InvalidConversationState {
            path: paths.conversation_json(),
            reason: UNSUPPORTED_CONVERSATION_STATE_SCHEMA.to_string(),
        });
    }
    let transcript = read_json_optional::<Transcript>(paths.transcript_json())?.unwrap_or_default();
    let artifacts = ConversationArtifacts {
        event_log: read_value_array(paths.event_log_json())?,
        flow_run_requests: read_value_array(
            paths.artifact_file(ArtifactCollection::FlowRunRequests),
        )?,
        flow_launches: read_value_array(paths.artifact_file(ArtifactCollection::FlowLaunches))?,
        run_recoveries: read_value_array(paths.artifact_file(ArtifactCollection::RunRecoveries))?,
        proposed_plans: read_value_array(paths.artifact_file(ArtifactCollection::ProposedPlans))?,
    };
    Ok(Some(ConversationRecord {
        meta,
        transcript,
        artifacts,
    }))
}

/// Write the record files named by `plan`, ending with `conversation.json` so
/// the revision cursor never claims undurable state.
pub(crate) fn write_record(
    paths: &ConversationRecordPaths,
    record: &ConversationRecord,
    plan: &RecordWritePlan,
) -> Result<()> {
    for collection in ALL_ARTIFACT_COLLECTIONS {
        if plan.includes_collection(collection) {
            write_json_atomic(
                paths.artifact_file(collection),
                record.artifacts.collection(collection),
                JsonWriteOptions::default(),
            )?;
        }
    }
    if plan.everything || plan.event_log {
        write_json_atomic(
            paths.event_log_json(),
            &record.artifacts.event_log,
            JsonWriteOptions::default(),
        )?;
    }
    if plan.everything || plan.transcript {
        write_json_atomic(
            paths.transcript_json(),
            &record.transcript,
            JsonWriteOptions::default(),
        )?;
    }
    write_json_atomic(
        paths.conversation_json(),
        &record.meta,
        JsonWriteOptions::default(),
    )
}

fn read_value_array(path: impl AsRef<Path>) -> Result<Vec<Value>> {
    Ok(read_json_optional::<Vec<Value>>(path)?.unwrap_or_default())
}
