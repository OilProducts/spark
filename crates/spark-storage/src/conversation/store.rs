//! Split-file persistence for conversation records.
//!
//! One conversation directory holds all of its durable authorities:
//! `conversation.json` (metadata + revision cursor), shared append-only
//! `events.jsonl` / `transcript.jsonl`, and artifact records.
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
    ArtifactCollection, ConversationArtifacts, ConversationMeta, ConversationRecord,
};

pub(crate) const CONVERSATION_META_FILE_NAME: &str = "conversation.json";
pub(crate) const EVENT_LOG_FILE_NAME: &str = "event-log.json";
pub(crate) const TOOL_OUTPUT_DIR_NAME: &str = "tool-output";

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

    pub fn artifact_file(&self, collection: ArtifactCollection) -> PathBuf {
        self.root
            .join("artifacts")
            .join(artifact_file_name(collection))
    }

    pub fn event_log_json(&self) -> PathBuf {
        self.root.join(EVENT_LOG_FILE_NAME)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn tool_output_file(&self, segment_id: &str) -> PathBuf {
        self.root
            .join(TOOL_OUTPUT_DIR_NAME)
            .join(format!("{segment_id}.txt"))
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
    let Some(meta) =
        read_json_optional::<ConversationMeta>(paths.conversation_json()).map_err(|error| {
            match error {
                StorageError::JsonRead { .. } => StorageError::SettingsValidation {
                    path: paths.conversation_json(),
                    reason: "Invalid conversation metadata; check field names and types.".into(),
                },
                error => error,
            }
        })?
    else {
        return Ok(None);
    };
    if meta.schema_version != CONVERSATION_STATE_SCHEMA_VERSION
        || !(0..=1).contains(&meta.settings_schema_version)
    {
        return Err(StorageError::InvalidConversationState {
            path: paths.conversation_json(),
            reason: UNSUPPORTED_CONVERSATION_STATE_SCHEMA.to_string(),
        });
    }
    let activity = crate::ActivityRepository::new(paths.root());
    // conversation.json is the commit publication cursor. Ignore transcript
    // records from an in-flight batch until that batch's final revision is
    // visible, so readers never observe a completed/failed turn without the
    // rest of its committed state.
    let published_event_sequence = activity
        .read_events()?
        .into_iter()
        .filter(|event| {
            event
                .event
                .get("revision")
                .and_then(Value::as_i64)
                .is_some_and(|revision| revision <= meta.revision)
        })
        .map(|event| event.sequence)
        .max()
        .unwrap_or(0);
    let mut transcript =
        activity.hydrate_transcript_through_event_sequence(published_event_sequence)?;
    let artifacts = ConversationArtifacts {
        event_log: read_value_array(paths.event_log_json())?,
        flow_run_requests: read_value_array(
            paths.artifact_file(ArtifactCollection::FlowRunRequests),
        )?,
        flow_launches: read_value_array(paths.artifact_file(ArtifactCollection::FlowLaunches))?,
        run_recoveries: read_value_array(paths.artifact_file(ArtifactCollection::RunRecoveries))?,
        proposed_plans: read_value_array(paths.artifact_file(ArtifactCollection::ProposedPlans))?,
    };
    // Derive the plan review link at read time from the durable plan record,
    // so a transcript rewrite that drops a segment's `artifact_id` cannot
    // orphan a plan from its Approve/Disapprove controls.
    for plan in &artifacts.proposed_plans {
        let (Some(plan_id), Some(segment_id)) = (
            plan.get("id").and_then(Value::as_str),
            plan.get("source_segment_id").and_then(Value::as_str),
        ) else {
            continue;
        };
        if let Some(segment) = transcript
            .segments
            .iter_mut()
            .find(|segment| segment.id == segment_id && segment.artifact_id.is_none())
        {
            segment.artifact_id = Some(plan_id.to_string());
        }
    }
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
    write_json_atomic(
        paths.conversation_json(),
        &record.meta,
        JsonWriteOptions::default(),
    )
}

fn read_value_array(path: impl AsRef<Path>) -> Result<Vec<Value>> {
    Ok(read_json_optional::<Vec<Value>>(path)?.unwrap_or_default())
}
