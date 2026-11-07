use attractor_core::JournalEntry;
pub use attractor_runtime::usage::RunUsageAccumulator;
use attractor_runtime::RunStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use spark_common::project::{build_project_id, normalize_project_path};
use spark_common::settings::SparkSettings;
use spark_storage::ConversationStore;
use std::sync::Arc;

use spark_storage::conversation::{
    CatchUp, ChangeBatch, ChangeScope, EntityKind, EntityRow, PageCursor, PageRead, ResetReason,
    TurnPage,
};

use crate::conversations::{MutationAck, TurnAcceptance, WorkspaceConversationService};
use crate::triggers::WorkspaceTriggerService;
use crate::{WorkspaceError, WorkspaceResult};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RawLiveQuery {
    pub project_path: Option<String>,
    pub conversation_id: Option<String>,
    pub conversation_project_path: Option<String>,
    pub conversation_revision: Option<String>,
    pub conversation_generation: Option<String>,
    pub run_id: Option<String>,
    pub run_sequence: Option<String>,
    pub include_runs_overview: Option<String>,
    pub runs_project_path: Option<String>,
    pub include_triggers: Option<String>,
    pub triggers_project_path: Option<String>,
    pub include_workflow_log: Option<String>,
    pub include_settings: Option<String>,
    pub include_missions: Option<String>,
    pub missions_project_path: Option<String>,
    pub include_conversations: Option<String>,
    pub conversations_project_path: Option<String>,
    pub conversations_generation: Option<String>,
    pub conversations_revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveQuery {
    pub project_path: Option<String>,
    pub conversation_id: Option<String>,
    pub conversation_project_path: Option<String>,
    /// Catch-up cursor (generation, revision R) for the conversation scope.
    pub conversation_cursor: Option<(String, i64)>,
    pub run_id: Option<String>,
    pub run_sequence: Option<u64>,
    pub include_runs_overview: bool,
    pub runs_project_path: Option<String>,
    pub include_triggers: bool,
    pub triggers_project_path: Option<String>,
    pub include_workflow_log: bool,
    pub include_settings: bool,
    pub include_missions: bool,
    pub missions_project_path: Option<String>,
    /// Thread-list summaries, of one project or of every project.
    pub include_conversations: bool,
    pub conversations_project_path: Option<String>,
    /// Catch-up cursor (generation, revision R) for the summary scope.
    pub conversations_cursor: Option<(String, i64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LiveResource {
    pub kind: String,
    pub id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LiveCursor {
    pub kind: String,
    pub value: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LiveEnvelope {
    #[serde(rename = "type")]
    pub event_type: String,
    pub project_path: Option<String>,
    pub resource: LiveResource,
    pub cursor: Option<LiveCursor>,
    pub payload: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

pub fn validate_live_query(raw: RawLiveQuery) -> WorkspaceResult<LiveQuery> {
    let project_path = normalize_project_path_opt(raw.project_path.as_deref())?;
    let conversation_id = trimmed(raw.conversation_id.as_deref());
    let conversation_project_path =
        normalize_project_path_opt(raw.conversation_project_path.as_deref())?;
    let conversation_cursor = live_cursor(
        raw.conversation_generation.as_deref(),
        raw.conversation_revision.as_deref(),
        "conversation",
    )?;
    let conversations_cursor = live_cursor(
        raw.conversations_generation.as_deref(),
        raw.conversations_revision.as_deref(),
        "conversations",
    )?;
    let run_id = trimmed(raw.run_id.as_deref());
    let run_sequence = parse_non_negative_i64(raw.run_sequence.as_deref(), "run_sequence")?
        .map(|value| value as u64);
    let include_runs_overview = parse_bool(raw.include_runs_overview.as_deref());
    let runs_project_path = normalize_project_path_opt(raw.runs_project_path.as_deref())?;
    let include_triggers = parse_bool(raw.include_triggers.as_deref());
    let triggers_project_path = normalize_project_path_opt(raw.triggers_project_path.as_deref())?;
    let include_workflow_log = parse_bool(raw.include_workflow_log.as_deref());

    let conversation_project_path = if conversation_id.is_some() {
        match conversation_project_path.or_else(|| project_path.clone()) {
            Some(project_path) => Some(project_path),
            None => {
                return Err(WorkspaceError::Validation(
                    "conversation_project_path is required when conversation_id is provided."
                        .to_string(),
                ))
            }
        }
    } else {
        conversation_project_path
    };

    let runs_project_path = if include_runs_overview && runs_project_path.is_none() {
        if conversation_id.is_none() {
            project_path.clone()
        } else {
            None
        }
    } else {
        runs_project_path
    };
    let triggers_project_path = if include_triggers && triggers_project_path.is_none() {
        if conversation_id.is_none() {
            project_path.clone()
        } else {
            None
        }
    } else {
        triggers_project_path
    };

    Ok(LiveQuery {
        project_path,
        conversation_id,
        conversation_project_path,
        conversation_cursor,
        run_id,
        run_sequence,
        include_runs_overview,
        runs_project_path,
        include_triggers,
        triggers_project_path,
        include_workflow_log,
        include_settings: parse_bool(raw.include_settings.as_deref()),
        include_missions: parse_bool(raw.include_missions.as_deref()),
        missions_project_path: normalize_project_path_opt(raw.missions_project_path.as_deref())?,
        include_conversations: parse_bool(raw.include_conversations.as_deref()),
        conversations_project_path: normalize_project_path_opt(
            raw.conversations_project_path.as_deref(),
        )?,
        conversations_cursor,
    })
}

/// A scope's catch-up cursor: its generation and revision come together.
fn live_cursor(
    generation: Option<&str>,
    revision: Option<&str>,
    scope: &str,
) -> WorkspaceResult<Option<(String, i64)>> {
    let revision = parse_non_negative_i64(revision, &format!("{scope}_revision"))?;
    match (trimmed(generation), revision) {
        (Some(generation), Some(revision)) => Ok(Some((generation, revision))),
        (None, None) => Ok(None),
        _ => Err(WorkspaceError::Validation(format!(
            "{scope}_generation and {scope}_revision must be sent together."
        ))),
    }
}

pub fn initial_live_envelopes(
    settings: &SparkSettings,
    store: &Arc<ConversationStore>,
    query: &LiveQuery,
) -> WorkspaceResult<Vec<LiveEnvelope>> {
    // Conversation scopes initialize in the stream itself, after it subscribed.
    let mut envelopes = Vec::new();
    if let Some(run_id) = query.run_id.as_deref() {
        envelopes.extend(run_envelopes(settings, run_id, query.run_sequence)?);
    }
    if query.include_runs_overview {
        envelopes.extend(runs_overview_envelopes(
            settings,
            query.runs_project_path.as_deref(),
        )?);
    }
    if query.include_triggers {
        envelopes.push(trigger_snapshot_envelope(
            settings,
            store,
            query.triggers_project_path.as_deref(),
        )?);
    }
    if query.include_workflow_log {
        envelopes.extend(crate::workflow_log::workflow_log_tail_envelopes(
            settings,
            crate::workflow_log::WORKFLOW_LOG_TAIL_LIMIT,
        )?);
    }
    Ok(envelopes)
}

pub fn run_envelopes_after(
    settings: &SparkSettings,
    run_id: &str,
    run_sequence: u64,
) -> WorkspaceResult<Vec<LiveEnvelope>> {
    run_envelopes(settings, run_id, Some(run_sequence))
}

/// Evicts the run's incremental journal cache once it reaches a terminal
/// status, so finished runs stop holding ring and projection memory.
pub fn evict_run_live_cache_if_terminal(settings: &SparkSettings, run_id: &str) -> bool {
    let store = RunStore::for_settings(settings);
    let Ok(Some(meta)) = store.read_run_meta(run_id) else {
        return false;
    };
    let Some(record) = meta.record else {
        return false;
    };
    let status = attractor_runtime::normalize_run_status(&record.status);
    if matches!(
        status.as_str(),
        "completed" | "failed" | "canceled" | "cancelled" | "aborted"
    ) {
        return true;
    }
    false
}

pub fn latest_run_sequence(settings: &SparkSettings, run_id: &str) -> WorkspaceResult<Option<u64>> {
    let store = RunStore::for_settings(settings);
    let Some(entries) = run_journal_entries(&store, run_id)? else {
        return Ok(None);
    };
    Ok(entries.into_iter().map(|entry| entry.sequence).max())
}

/// A transient run envelope: droppable and never stored.
pub const NODE_THINKING_EVENT_TYPE: &str = "run.thinking";

/// The run's unpublished thinking updates as transient live envelopes, shaped
/// like its transcript envelopes.
pub fn node_thinking_envelopes(settings: &SparkSettings, run_id: &str) -> Vec<LiveEnvelope> {
    let updates = attractor_runtime::events::take_node_thinking(run_id);
    if updates.is_empty() {
        return Vec::new();
    }
    let parent_run_id = RunStore::for_settings(settings)
        .read_run_meta(run_id)
        .ok()
        .flatten()
        .and_then(|meta| meta.record)
        .and_then(|record| record.parent_run_id);
    updates
        .into_iter()
        .map(|thinking| LiveEnvelope {
            event_type: NODE_THINKING_EVENT_TYPE.to_string(),
            project_path: None,
            resource: LiveResource {
                kind: "node_execution".to_string(),
                id: Some(format!(
                    "{}:{}:{}:{}",
                    run_id, thinking.node_id, thinking.stage_index, thinking.attempt
                )),
            },
            cursor: None,
            reason: None,
            payload: json!({
                "run_id": run_id,
                "presentation_run_id": parent_run_id.as_deref().unwrap_or(run_id),
                "node_id": thinking.node_id,
                "stage_index": thinking.stage_index,
                "attempt": thinking.attempt,
                "active": thinking.active,
                "estimated_tokens": thinking.estimated_tokens,
            }),
        })
        .collect()
}

pub fn execution_transcript_envelopes(
    settings: &SparkSettings,
    run_id: &str,
    cursors: &mut std::collections::HashMap<String, ExecutionTranscriptCursor>,
) -> WorkspaceResult<Vec<LiveEnvelope>> {
    let store = RunStore::for_settings(settings);
    let Some(meta) = store
        .read_run_meta(run_id)
        .map_err(|error| WorkspaceError::Internal(error.to_string()))?
    else {
        return Ok(Vec::new());
    };
    let parent_run_id = meta
        .record
        .as_ref()
        .and_then(|record| record.parent_run_id.clone());
    let presentation_run_id = parent_run_id.as_deref().unwrap_or(run_id);
    let source_scope = if parent_run_id.is_some() {
        "child"
    } else {
        "root"
    };
    let mut envelopes = Vec::new();
    for execution in store
        .list_node_executions(&meta.paths)
        .map_err(|error| WorkspaceError::Internal(error.to_string()))?
    {
        let identity = format!(
            "{}:{}:{}:{}",
            run_id, execution.node_id, execution.stage_index, execution.attempt
        );
        let cursor = cursors.get(&identity).copied().unwrap_or_default();
        let root = store
            .node_execution_root(
                &meta.paths,
                &execution.node_id,
                execution.stage_index,
                execution.attempt,
            )
            .map_err(|error| WorkspaceError::Internal(error.to_string()))?;
        let (records, byte_offset) = spark_storage::ActivityRepository::new(root)
            .read_transcript_records_from(cursor.byte_offset)
            .map_err(|error| WorkspaceError::Internal(error.to_string()))?;
        for record in records {
            if record.revision() <= cursor.revision {
                continue;
            }
            let event_type = match &record {
                spark_storage::TranscriptRecord::TurnUpsert { .. } => "conversation.turn_upsert",
                spark_storage::TranscriptRecord::SegmentUpsert { .. } => {
                    "conversation.segment_upsert"
                }
                spark_storage::TranscriptRecord::SegmentTombstone { .. } => {
                    "conversation.segment_tombstone"
                }
            };
            envelopes.push(LiveEnvelope {
                event_type: event_type.to_string(),
                project_path: None,
                resource: LiveResource {
                    kind: "node_execution".to_string(),
                    id: Some(identity.clone()),
                },
                cursor: Some(LiveCursor {
                    kind: "transcript_revision".to_string(),
                    value: record.revision() as i64,
                }),
                payload: json!({
                    "run_id": run_id,
                    "presentation_run_id": presentation_run_id,
                    "source_scope": source_scope,
                    "node_id": execution.node_id,
                    "stage_index": execution.stage_index,
                    "attempt": execution.attempt,
                    "record": record,
                }),
                reason: None,
            });
            cursors.insert(
                identity.clone(),
                ExecutionTranscriptCursor {
                    revision: record.revision(),
                    byte_offset,
                },
            );
        }
    }
    Ok(envelopes)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ExecutionTranscriptCursor {
    pub revision: u64,
    pub byte_offset: u64,
}

pub fn run_upsert_envelope(
    settings: &SparkSettings,
    run_id: &str,
) -> WorkspaceResult<Option<LiveEnvelope>> {
    let store = RunStore::for_settings(settings);
    // Record-only read: the upsert never uses events or checkpoint, and a
    // full bundle read reparses the entire event log.
    let Some(paths) = store
        .find_run_root(run_id)
        .map_err(|error| WorkspaceError::Internal(error.to_string()))?
    else {
        return Ok(None);
    };
    let Some(record) = store
        .read_run_record(&paths)
        .map_err(|error| WorkspaceError::Internal(error.to_string()))?
    else {
        return Ok(None);
    };
    let project_path = normalized_record_path(&record.project_path)
        .or_else(|| normalized_record_path(&record.working_directory));
    Ok(Some(run_upsert_envelope_from_record(record, project_path)))
}

pub fn trigger_upsert_envelope(trigger: &Value) -> LiveEnvelope {
    let trigger_id = trigger
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let project_path = trigger_project_path(trigger);
    LiveEnvelope {
        event_type: "trigger.upsert".to_string(),
        project_path,
        resource: LiveResource {
            kind: "trigger".to_string(),
            id: Some(trigger_id),
        },
        cursor: None,
        payload: json!({
            "type": "trigger_upsert",
            "trigger": trigger,
        }),
        reason: None,
    }
}

pub fn trigger_delete_envelope(deleted: &Value, project_path: Option<String>) -> LiveEnvelope {
    let trigger_id = deleted
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    LiveEnvelope {
        event_type: "trigger.delete".to_string(),
        project_path,
        resource: LiveResource {
            kind: "trigger".to_string(),
            id: Some(trigger_id),
        },
        cursor: None,
        payload: json!({
            "type": "trigger_deleted",
            "trigger": deleted,
        }),
        reason: None,
    }
}

pub fn envelope_matches_query(envelope: &LiveEnvelope, query: &LiveQuery) -> bool {
    match envelope.resource.kind.as_str() {
        "settings" => query.include_settings,
        "conversation" => {
            let Some(expected_id) = query.conversation_id.as_deref() else {
                return false;
            };
            if envelope.resource.id.as_deref() != Some(expected_id) {
                return false;
            }
            match query.conversation_project_path.as_deref() {
                Some(expected) => envelope.project_path.as_deref() == Some(expected),
                None => true,
            }
        }
        "run" => query
            .run_id
            .as_deref()
            .is_some_and(|expected| envelope.resource.id.as_deref() == Some(expected)),
        "node_execution" => query.run_id.as_deref().is_some_and(|expected| {
            envelope.payload.get("run_id").and_then(Value::as_str) == Some(expected)
        }),
        "runs_overview" => {
            if !query.include_runs_overview {
                return false;
            }
            match query.runs_project_path.as_deref() {
                Some(expected) => envelope.project_path.as_deref() == Some(expected),
                None => true,
            }
        }
        "trigger" => {
            if !query.include_triggers {
                return false;
            }
            match query.triggers_project_path.as_deref() {
                Some(expected) => envelope.project_path.as_deref() == Some(expected),
                None => true,
            }
        }
        "mission" => {
            query.include_missions
                && query
                    .missions_project_path
                    .as_deref()
                    .is_none_or(|expected| envelope.project_path.as_deref() == Some(expected))
        }
        // The workflow log is a global, all-project feed by design.
        "workflow_log" => query.include_workflow_log,
        _ => false,
    }
}

/// A transient stream delta as a live envelope: stream-local, carrying a
/// stream-sequence cursor instead of a committed revision, never journaled.
pub fn conversation_event_envelope(
    conversation_id: &str,
    project_path: &str,
    event: Value,
) -> LiveEnvelope {
    let stream_sequence = event
        .get("stream_sequence")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    LiveEnvelope {
        event_type: "conversation.stream_delta".to_string(),
        project_path: Some(project_path.to_string()),
        resource: LiveResource {
            kind: "conversation".to_string(),
            id: Some(conversation_id.to_string()),
        },
        cursor: Some(LiveCursor {
            kind: "conversation_stream_sequence".to_string(),
            value: stream_sequence,
        }),
        payload: event,
        reason: None,
    }
}

fn run_envelopes(
    settings: &SparkSettings,
    run_id: &str,
    run_sequence: Option<u64>,
) -> WorkspaceResult<Vec<LiveEnvelope>> {
    let store = RunStore::for_settings(settings);
    let after = run_sequence.unwrap_or(0);
    let Some(entries) = run_journal_entries(&store, run_id)? else {
        return Err(WorkspaceError::NotFound("Unknown pipeline".to_string()));
    };
    Ok(run_envelopes_from_entries(run_id, &entries, after))
}

fn run_envelopes_from_entries(
    run_id: &str,
    entries: &[JournalEntry],
    after_sequence: u64,
) -> Vec<LiveEnvelope> {
    let (envelopes, _) = journal_envelopes_from_entries(run_id, entries, after_sequence);
    envelopes
}

/// Journal envelopes strictly after the cursor. Returns false when the
/// replay hit a gap and was terminated with a resync envelope.
fn journal_envelopes_from_entries(
    run_id: &str,
    entries: &[JournalEntry],
    after_sequence: u64,
) -> (Vec<LiveEnvelope>, bool) {
    let max_sequence = entries
        .iter()
        .map(|entry| entry.sequence)
        .max()
        .unwrap_or(0);
    let mut expected_sequence = after_sequence.saturating_add(1);
    let mut envelopes = Vec::new();
    for entry in entries
        .iter()
        .filter(|entry| entry.sequence > after_sequence)
    {
        if entry.sequence != expected_sequence {
            envelopes.push(resync_required(
                "run",
                Some(run_id.to_string()),
                None,
                "run journal no longer contains a contiguous replay from the requested cursor",
            ));
            return (envelopes, false);
        }
        envelopes.push(run_journal_envelope(run_id, entry.clone()));
        expected_sequence = expected_sequence.saturating_add(1);
    }
    if envelopes.is_empty() && max_sequence > after_sequence {
        envelopes.push(resync_required(
            "run",
            Some(run_id.to_string()),
            None,
            "run journal no longer contains a contiguous replay from the requested cursor",
        ));
        return (envelopes, false);
    }
    (envelopes, true)
}

/// One publish cycle over the run's source-local orchestration journal.
pub struct RunLivePublication {
    pub envelopes: Vec<LiveEnvelope>,
    pub latest_sequence: Option<u64>,
    pub newly_read_entries: Vec<JournalEntry>,
    pub starts_at_sequence_zero: bool,
    pub fallback_model: String,
}

pub fn run_live_publication(
    settings: &SparkSettings,
    run_id: &str,
    after_sequence: Option<u64>,
) -> WorkspaceResult<Option<RunLivePublication>> {
    let store = RunStore::for_settings(settings);
    let fallback_model = store
        .read_run_meta(run_id)
        .map_err(|error| WorkspaceError::Internal(error.to_string()))?
        .and_then(|meta| meta.record)
        .map(|record| record.model)
        .unwrap_or_default();
    let after = after_sequence.unwrap_or(0);
    let Some(entries) = run_journal_entries(&store, run_id)? else {
        return Ok(None);
    };
    let latest_sequence = entries.iter().map(|entry| entry.sequence).max();
    let newly_read_entries = entries
        .into_iter()
        .filter(|entry| entry.sequence > after)
        .collect::<Vec<_>>();
    let (envelopes, _) = journal_envelopes_from_replay(run_id, newly_read_entries.clone(), after);
    Ok(Some(RunLivePublication {
        envelopes,
        latest_sequence,
        newly_read_entries,
        starts_at_sequence_zero: after == 0,
        fallback_model,
    }))
}

pub fn full_run_usage_accumulator(
    settings: &SparkSettings,
    run_id: &str,
) -> WorkspaceResult<RunUsageAccumulator> {
    let store = RunStore::for_settings(settings);
    let fallback_model = store
        .read_run_meta(run_id)
        .map_err(|error| WorkspaceError::Internal(error.to_string()))?
        .and_then(|meta| meta.record)
        .map(|record| record.model)
        .unwrap_or_default();
    let Some(meta) = store
        .read_run_meta(run_id)
        .map_err(|error| WorkspaceError::Internal(error.to_string()))?
    else {
        return Ok(RunUsageAccumulator::new(fallback_model));
    };
    let mut usage = RunUsageAccumulator::new(fallback_model);
    for execution in store
        .list_node_executions(&meta.paths)
        .map_err(|error| WorkspaceError::Internal(error.to_string()))?
    {
        if let Some(events) = store
            .read_node_execution_events(
                &meta.paths,
                &execution.node_id,
                execution.stage_index,
                execution.attempt,
            )
            .map_err(|error| WorkspaceError::Internal(error.to_string()))?
        {
            usage.apply_activity_events(&events);
        }
    }
    Ok(usage)
}

/// Journal envelopes from an already-windowed replay (entries strictly after
/// the cursor). Same contiguity contract as `journal_envelopes_from_entries`
/// without re-filtering the full journal.
fn journal_envelopes_from_replay(
    run_id: &str,
    entries_after: Vec<JournalEntry>,
    after_sequence: u64,
) -> (Vec<LiveEnvelope>, bool) {
    let mut expected_sequence = after_sequence.saturating_add(1);
    let mut envelopes = Vec::new();
    for entry in entries_after {
        if entry.sequence != expected_sequence {
            envelopes.push(resync_required(
                "run",
                Some(run_id.to_string()),
                None,
                "run journal no longer contains a contiguous replay from the requested cursor",
            ));
            return (envelopes, false);
        }
        envelopes.push(run_journal_envelope(run_id, entry));
        expected_sequence = expected_sequence.saturating_add(1);
    }
    (envelopes, true)
}

fn run_journal_entries(
    store: &RunStore,
    run_id: &str,
) -> WorkspaceResult<Option<Vec<JournalEntry>>> {
    let mut entries = store
        .read_run_bundle(run_id)
        .map(|bundle| bundle.map(|bundle| bundle.journal))
        .map_err(|error| WorkspaceError::Internal(error.to_string()))?;
    if let Some(entries) = entries.as_mut() {
        entries.sort_by_key(|entry| entry.sequence);
    }
    Ok(entries)
}

fn run_journal_envelope(run_id: &str, entry: JournalEntry) -> LiveEnvelope {
    let event_type = match entry.raw_type.as_str() {
        "human_gate" | "HumanInterventionRequested" | "ChildInterventionRequested" => {
            "run.question_pending"
        }
        "InterviewCompleted" => "run.question_answered",
        _ => "run.journal_entry",
    };
    LiveEnvelope {
        event_type: event_type.to_string(),
        project_path: None,
        resource: LiveResource {
            kind: "run".to_string(),
            id: Some(run_id.to_string()),
        },
        cursor: Some(LiveCursor {
            kind: "run_sequence".to_string(),
            value: entry.sequence as i64,
        }),
        payload: serde_json::to_value(entry).unwrap_or_else(|_| json!({})),
        reason: None,
    }
}

fn runs_overview_envelopes(
    settings: &SparkSettings,
    project_path: Option<&str>,
) -> WorkspaceResult<Vec<LiveEnvelope>> {
    let store = RunStore::for_settings(settings);
    let project_id = project_path.and_then(|path| build_project_id(path).ok());
    let mut records = store
        .list_run_metas()
        .map_err(|error| WorkspaceError::Internal(error.to_string()))?
        .into_iter()
        .filter_map(|meta| {
            let record = meta.record?;
            let in_project = project_path.is_none_or(|project_path| {
                normalized_record_path(&record.project_path).as_deref() == Some(project_path)
                    || normalized_record_path(&record.working_directory).as_deref()
                        == Some(project_path)
                    // No record names its project, so its directory does.
                    || (record.status == attractor_core::RUN_STATUS_UNREADABLE
                        && project_id.as_deref() == Some(meta.paths.project_id.as_str()))
            });
            in_project.then_some(record)
        })
        .collect::<Vec<_>>();
    records.sort_by(|left, right| {
        run_sort_key(right)
            .cmp(&run_sort_key(left))
            .then_with(|| left.run_id.cmp(&right.run_id))
    });
    Ok(records
        .into_iter()
        .map(|record| run_upsert_envelope_from_record(record, project_path.map(str::to_string)))
        .collect())
}

fn run_upsert_envelope_from_record(
    record: attractor_core::RunRecord,
    project_path: Option<String>,
) -> LiveEnvelope {
    run_upsert_envelope_from_record_with_usage(record, project_path, None)
}

pub fn run_upsert_envelope_with_usage(
    settings: &SparkSettings,
    run_id: &str,
    usage: Option<&attractor_runtime::usage::TokenUsageBreakdown>,
) -> WorkspaceResult<Option<LiveEnvelope>> {
    let store = RunStore::for_settings(settings);
    let Some(paths) = store
        .find_run_root(run_id)
        .map_err(|error| WorkspaceError::Internal(error.to_string()))?
    else {
        return Ok(None);
    };
    let Some(record) = store
        .read_run_record(&paths)
        .map_err(|error| WorkspaceError::Internal(error.to_string()))?
    else {
        return Ok(None);
    };
    let project_path = normalized_record_path(&record.project_path)
        .or_else(|| normalized_record_path(&record.working_directory));
    Ok(Some(run_upsert_envelope_from_record_with_usage(
        record,
        project_path,
        usage,
    )))
}

fn run_upsert_envelope_from_record_with_usage(
    mut record: attractor_core::RunRecord,
    project_path: Option<String>,
    usage: Option<&attractor_runtime::usage::TokenUsageBreakdown>,
) -> LiveEnvelope {
    if let Some(usage) = usage {
        record.token_usage = Some(usage.totals.total_tokens);
        record.token_usage_breakdown = Some(usage.to_value());
        record.estimated_model_cost = attractor_runtime::usage::estimate_model_cost(usage);
    }
    LiveEnvelope {
        event_type: "run.upsert".to_string(),
        project_path,
        resource: LiveResource {
            kind: "runs_overview".to_string(),
            id: None,
        },
        cursor: None,
        payload: json!({ "run": public_run_record(record) }),
        reason: None,
    }
}

fn trigger_snapshot_envelope(
    settings: &SparkSettings,
    store: &Arc<ConversationStore>,
    project_path: Option<&str>,
) -> WorkspaceResult<LiveEnvelope> {
    let triggers = WorkspaceTriggerService::new(settings.clone(), store.clone())
        .list_triggers(project_path)?;
    Ok(LiveEnvelope {
        event_type: "trigger.snapshot".to_string(),
        project_path: project_path.map(str::to_string),
        resource: LiveResource {
            kind: "trigger".to_string(),
            id: None,
        },
        cursor: None,
        payload: json!({ "triggers": triggers }),
        reason: None,
    })
}

/// Envelopes telling a subscriber that the broadcast ring overwrote frames it
/// had not read yet: one `resync_required` per resource the query follows, so
/// the client refetches durable state instead of rendering a gapped stream.
pub fn lagged_resync_envelopes(query: &LiveQuery) -> Vec<LiveEnvelope> {
    const REASON: &str = "live event stream lagged behind the publisher and dropped frames";
    let mut envelopes = Vec::new();
    if query.include_settings {
        envelopes.push(resync_required(
            "settings",
            Some("workspace".into()),
            None,
            REASON,
        ));
    }
    if let Some(run_id) = query.run_id.as_deref() {
        envelopes.push(resync_required(
            "run",
            Some(run_id.to_string()),
            None,
            REASON,
        ));
    }
    if query.include_runs_overview {
        envelopes.push(resync_required(
            "runs_overview",
            None,
            query.runs_project_path.clone(),
            REASON,
        ));
    }
    if query.include_triggers {
        envelopes.push(resync_required(
            "trigger",
            None,
            query.triggers_project_path.clone(),
            REASON,
        ));
    }
    if query.include_missions {
        envelopes.push(resync_required(
            "mission",
            None,
            query.missions_project_path.clone(),
            REASON,
        ));
    }
    envelopes
}

pub fn mission_upsert_envelope(mission: &crate::missions::MissionRecord) -> LiveEnvelope {
    LiveEnvelope {
        event_type: "mission.upsert".to_string(),
        project_path: Some(mission.project_path.clone()),
        resource: LiveResource {
            kind: "mission".to_string(),
            id: Some(mission.id.clone()),
        },
        cursor: None,
        payload: json!({ "mission": mission }),
        reason: None,
    }
}

fn resync_required(
    kind: &str,
    id: Option<String>,
    project_path: Option<String>,
    reason: &str,
) -> LiveEnvelope {
    LiveEnvelope {
        event_type: "resync_required".to_string(),
        project_path,
        resource: LiveResource {
            kind: kind.to_string(),
            id,
        },
        cursor: None,
        payload: json!({ "reason": reason }),
        reason: Some(reason.to_string()),
    }
}

fn public_run_record(record: attractor_core::RunRecord) -> Value {
    let first_launch_input = record
        .launch_context
        .as_ref()
        .and_then(|context| attractor_api::first_launch_input(&json!(context)));
    json!({
        "run_id": record.run_id,
        "title": record.title,
        "flow_name": record.flow_name,
        "status": record.status,
        "outcome": record.outcome,
        "outcome_reason_code": record.outcome_reason_code,
        "outcome_reason_message": record.outcome_reason_message,
        "working_directory": record.working_directory,
        "model": record.model,
        "provider": record.provider,
        "llm_provider": record.llm_provider,
        "llm_profile": record.llm_profile,
        "reasoning_effort": record.reasoning_effort,
        "started_at": record.started_at,
        "ended_at": record.ended_at,
        "project_path": record.project_path,
        "git_branch": record.git_branch,
        "git_commit": record.git_commit,
        "spec_id": record.spec_id,
        "plan_id": record.plan_id,
        "continued_from_run_id": record.continued_from_run_id,
        "continued_from_node": record.continued_from_node,
        "continued_from_flow_mode": record.continued_from_flow_mode,
        "continued_from_flow_name": record.continued_from_flow_name,
        "parent_run_id": record.parent_run_id,
        "parent_node_id": record.parent_node_id,
        "root_run_id": record.root_run_id,
        "child_invocation_index": record.child_invocation_index,
        "last_error": record.last_error,
        "token_usage": record.token_usage,
        "token_usage_breakdown": record.token_usage_breakdown,
        "estimated_model_cost": record.estimated_model_cost,
        "execution_mode": record.execution_mode,
        "execution_profile_id": record.execution_profile_id,
        "execution_profile_capabilities": record.execution_profile_capabilities.unwrap_or_else(|| json!({})),
        "first_launch_input": first_launch_input,
    })
}

fn run_sort_key(record: &attractor_core::RunRecord) -> String {
    record
        .started_at
        .trim()
        .to_string()
        .if_empty_then(record.ended_at.clone().unwrap_or_default())
}

fn trigger_project_path(trigger: &Value) -> Option<String> {
    trigger
        .get("project_path")
        .and_then(Value::as_str)
        .and_then(|value| normalize_project_path_opt(Some(value)).ok().flatten())
}

fn normalized_record_path(value: &str) -> Option<String> {
    normalize_project_path_opt(Some(value)).ok().flatten()
}

fn normalize_project_path_opt(value: Option<&str>) -> WorkspaceResult<Option<String>> {
    let Some(value) = trimmed(value) else {
        return Ok(None);
    };
    normalize_project_path(value)
        .map_err(|error| WorkspaceError::Validation(error.to_string()))
        .map(|path| path.map(|path| path.to_string_lossy().into_owned()))
}

fn parse_non_negative_i64(value: Option<&str>, field: &str) -> WorkspaceResult<Option<i64>> {
    let Some(value) = trimmed(value) else {
        return Ok(None);
    };
    let parsed = value.parse::<i64>().map_err(|_| {
        WorkspaceError::Validation(format!("{field} must be a non-negative integer."))
    })?;
    if parsed < 0 {
        return Err(WorkspaceError::Validation(format!(
            "{field} must be a non-negative integer."
        )));
    }
    Ok(Some(parsed))
}

fn parse_bool(value: Option<&str>) -> bool {
    matches!(
        trimmed(value).as_deref(),
        Some("true" | "True" | "1" | "yes" | "on")
    )
}

fn trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

trait EmptyStringFallback {
    fn if_empty_then(self, fallback: String) -> String;
}

impl EmptyStringFallback for String {
    fn if_empty_then(self, fallback: String) -> String {
        if self.trim().is_empty() {
            fallback
        } else {
            self
        }
    }
}

// Conversation rows on the wire: revisions, positions and orders are
// canonical decimal strings, since JSON numbers cannot hold every SQLite
// integer. Envelopes for conversation scopes carry `cursor: null`; their
// payload watermark is authoritative.

/// Turns in a window when the browser does not ask for a size.
pub const DEFAULT_PAGE_LIMIT: usize = 50;
/// The largest window a browser may ask for.
pub const MAX_PAGE_LIMIT: usize = 100;
const MAX_PAGE_CURSOR_BYTES: usize = 2048;

pub fn row_wire(row: &EntityRow) -> Value {
    let mut wire = json!({
        "kind": row.kind,
        "conversation_id": row.conversation_id,
        "id": row.id,
        "revision": row.revision.to_string(),
        "deleted": row.deleted,
        "payload": row.payload,
    });
    if let Some(collection) = &row.collection {
        wire["collection"] = json!(collection);
    }
    if let Some(turn_id) = &row.turn_id {
        wire["turn_id"] = json!(turn_id);
    }
    if let Some(position) = row.position {
        wire["position"] = json!(position.to_string());
    }
    if let Some(order) = row.order {
        wire["order"] = json!(order.to_string());
    }
    wire
}

pub fn batch_wire(generation: &str, batch: &ChangeBatch) -> Value {
    json!({
        "generation": generation,
        "from_revision": batch.from_revision.to_string(),
        "watermark": batch.watermark.to_string(),
        "rows": batch.rows.iter().map(row_wire).collect::<Vec<_>>(),
    })
}

pub fn reset_wire(code: &str, reason: ResetReason, generation: &str, watermark: i64) -> Value {
    json!({
        "code": code,
        "reason": reason,
        "generation": generation,
        "watermark": watermark.to_string(),
    })
}

/// A settings, answer or review result for the browser, its conversation
/// row decorated with the model-settings view.
pub fn mutation_ack_wire(
    service: &WorkspaceConversationService,
    ack: MutationAck,
) -> WorkspaceResult<Value> {
    let generation = service.store().database().generation().to_string();
    let changes = decorated_batch(service, ack.changes)?;
    Ok(json!({
        "generation": generation,
        "revision": ack.revision.to_string(),
        "changes": batch_wire(&generation, &changes),
    }))
}

/// A send's acceptance for the browser. `client_turn_id` repeats the
/// browser's id under the name it sent.
pub fn acceptance_wire(
    service: &WorkspaceConversationService,
    acceptance: TurnAcceptance,
) -> WorkspaceResult<Value> {
    let generation = service.store().database().generation().to_string();
    let changes = decorated_batch(service, acceptance.changes)?;
    Ok(json!({
        "generation": generation,
        "revision": acceptance.revision.to_string(),
        "user_turn_id": acceptance.user_turn_id,
        "client_turn_id": acceptance.user_turn_id,
        "assistant_turn_id": acceptance.assistant_turn_id,
        "status": acceptance.status,
        "changes": batch_wire(&generation, &changes),
    }))
}

fn decorated_batch(
    service: &WorkspaceConversationService,
    mut batch: ChangeBatch,
) -> WorkspaceResult<ChangeBatch> {
    for row in &mut batch.rows {
        if row.kind == EntityKind::Conversation {
            service.decorate_conversation_row(row)?;
        }
    }
    Ok(batch)
}

/// Why a conversation read did not return its rows.
pub enum ConversationReadOutcome {
    /// The rows, as wire JSON.
    Rows(Value),
    /// A `Reset` the browser answers by reloading the scope.
    Reset(Value),
}

/// `GET /conversations/{id}`: the newest window, or the window older than
/// `before`. Unknown and deleted conversations are not found.
pub fn read_conversation_page(
    settings: &SparkSettings,
    store: &Arc<ConversationStore>,
    conversation_id: &str,
    project_path: &str,
    limit: usize,
    before: Option<&str>,
) -> WorkspaceResult<ConversationReadOutcome> {
    let service = WorkspaceConversationService::new(settings.clone(), store.clone());
    let project_path = conversation_scope_project(store, conversation_id, project_path)?;
    let database = store.database();
    let generation = database.generation();
    let cursor = match before {
        Some(before) => {
            let (cursor_generation, cursor) = decode_page_cursor(before, conversation_id)?;
            if cursor_generation != generation {
                let watermark = database.read(|read| Ok(read.watermark()))?;
                return Ok(ConversationReadOutcome::Reset(reset_wire(
                    "window_reset",
                    ResetReason::GenerationChanged,
                    generation,
                    watermark,
                )));
            }
            Some(cursor)
        }
        None => None,
    };
    let (watermark, page) = database.read(|read| {
        Ok((
            read.watermark(),
            read.page(conversation_id, limit, cursor.as_ref())?,
        ))
    })?;
    match page {
        PageRead::Page(page) => {
            if page
                .conversation
                .payload
                .as_ref()
                .and_then(|payload| payload["project_path"].as_str())
                != Some(project_path.as_str())
            {
                return Err(WorkspaceError::NotFound(format!(
                    "Unknown conversation: {conversation_id}"
                )));
            }
            Ok(ConversationReadOutcome::Rows(page_wire(
                &service,
                generation,
                conversation_id,
                page,
            )?))
        }
        PageRead::Missing => Err(WorkspaceError::NotFound(format!(
            "Unknown conversation: {conversation_id}"
        ))),
        PageRead::Reset(reason) => Ok(ConversationReadOutcome::Reset(reset_wire(
            "window_reset",
            reason,
            generation,
            watermark,
        ))),
    }
}

/// `GET /conversations/{id}/changes`: the rows changed after `since`. A
/// deleted conversation still answers with its tombstones.
pub fn read_conversation_changes(
    settings: &SparkSettings,
    store: &Arc<ConversationStore>,
    conversation_id: &str,
    project_path: &str,
    generation: &str,
    since: i64,
) -> WorkspaceResult<ConversationReadOutcome> {
    conversation_scope_project(store, conversation_id, project_path)?;
    let service = WorkspaceConversationService::new(settings.clone(), store.clone());
    let (envelope, _) =
        conversation_changes_envelope(&service, conversation_id, project_path, generation, since)?;
    Ok(if envelope.event_type == "conversation.reset" {
        ConversationReadOutcome::Reset(envelope.payload)
    } else {
        ConversationReadOutcome::Rows(envelope.payload)
    })
}

/// `GET /projects/conversations`: every live summary row of one project, or
/// of every project, at one watermark.
pub fn read_conversation_summaries(
    settings: &SparkSettings,
    store: &Arc<ConversationStore>,
    project_path: Option<&str>,
) -> WorkspaceResult<Value> {
    let service = WorkspaceConversationService::new(settings.clone(), store.clone());
    Ok(conversations_snapshot_envelope(&service, project_path)?
        .0
        .payload)
}

/// The project path of a conversation scope, checked against the
/// conversation's own binding (also for tombstones), never registering it.
fn conversation_scope_project(
    store: &Arc<ConversationStore>,
    conversation_id: &str,
    project_path: &str,
) -> WorkspaceResult<String> {
    let project_path = normalize_project_path_opt(Some(project_path))?
        .ok_or_else(|| WorkspaceError::Validation("project_path is required.".to_string()))?;
    let location = store
        .database()
        .read(|read| read.conversation_location(conversation_id))?;
    if let Some((actual, _)) = location {
        if actual != project_path {
            return Err(WorkspaceError::Validation(
                "Conversation is already bound to a different project path.".to_string(),
            ));
        }
    }
    Ok(project_path)
}

fn page_wire(
    service: &WorkspaceConversationService,
    generation: &str,
    conversation_id: &str,
    page: TurnPage,
) -> WorkspaceResult<Value> {
    let mut conversation = page.conversation;
    service.decorate_conversation_row(&mut conversation)?;
    let mut rows = vec![row_wire(&conversation)];
    rows.extend(page.rows.iter().map(row_wire));
    Ok(json!({
        "generation": generation,
        "watermark": page.watermark.to_string(),
        "conversation_id": conversation_id,
        "rows": rows,
        "turn_ids": page.turn_ids,
        "pinned_turn_ids": page.pinned_turn_ids,
        "pinned_artifact_ids": page
            .pinned_artifact_ids
            .iter()
            .map(|(collection, id)| json!({"collection": collection, "id": id}))
            .collect::<Vec<_>>(),
        "older_cursor": page
            .older_cursor
            .as_ref()
            .map(|cursor| encode_page_cursor(generation, conversation_id, cursor)),
    }))
}

/// An empty window: the conversation has no rows yet (or no longer).
fn empty_page_wire(generation: &str, conversation_id: &str, watermark: i64) -> Value {
    json!({
        "generation": generation,
        "watermark": watermark.to_string(),
        "conversation_id": conversation_id,
        "rows": [],
        "turn_ids": [],
        "pinned_turn_ids": [],
        "pinned_artifact_ids": [],
        "older_cursor": null,
    })
}

/// Opaque but not secret: unpadded base64url of the keyset position JSON.
pub fn encode_page_cursor(generation: &str, conversation_id: &str, cursor: &PageCursor) -> String {
    let json = json!({
        "v": 1,
        "generation": generation,
        "conversation_id": conversation_id,
        "opening_revision": cursor.opening_revision.to_string(),
        "before_position": cursor.before_position.to_string(),
        "before_turn_id": cursor.before_turn_id,
    });
    base64url_encode(json.to_string().as_bytes())
}

/// The generation and keyset position of an older-page cursor issued for
/// `conversation_id`; anything else is a validation error.
pub fn decode_page_cursor(
    value: &str,
    conversation_id: &str,
) -> WorkspaceResult<(String, PageCursor)> {
    let invalid = || WorkspaceError::Validation("Invalid conversation page cursor.".to_string());
    if value.len() > MAX_PAGE_CURSOR_BYTES {
        return Err(invalid());
    }
    let bytes = base64url_decode(value).ok_or_else(invalid)?;
    let json: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    let decimal = |key: &str| -> WorkspaceResult<i64> {
        let text = json[key].as_str().ok_or_else(invalid)?;
        if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(invalid());
        }
        text.parse().map_err(|_| invalid())
    };
    if json["v"] != json!(1) || json["conversation_id"] != json!(conversation_id) {
        return Err(invalid());
    }
    let generation = json["generation"].as_str().ok_or_else(invalid)?.to_string();
    let before_turn_id = json["before_turn_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(invalid)?
        .to_string();
    Ok((
        generation,
        PageCursor {
            opening_revision: decimal("opening_revision")?,
            before_position: decimal("before_position")?,
            before_turn_id,
        },
    ))
}

const BASE64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64url_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (index, byte)| {
            n | (u32::from(*byte) << (16 - 8 * index))
        });
        for index in 0..=chunk.len() {
            out.push(BASE64URL[(n >> (18 - 6 * index) & 63) as usize] as char);
        }
    }
    out
}

fn base64url_decode(text: &str) -> Option<Vec<u8>> {
    if text.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    for chunk in text.as_bytes().chunks(4) {
        let mut n = 0u32;
        for (index, byte) in chunk.iter().enumerate() {
            let value = BASE64URL.iter().position(|candidate| candidate == byte)? as u32;
            n |= value << (18 - 6 * index);
        }
        for index in 0..chunk.len() - 1 {
            out.push((n >> (16 - 8 * index)) as u8);
        }
    }
    Some(out)
}

fn conversation_envelope(
    event_type: &str,
    conversation_id: &str,
    project_path: &str,
    payload: Value,
) -> LiveEnvelope {
    LiveEnvelope {
        event_type: event_type.to_string(),
        project_path: Some(project_path.to_string()),
        resource: LiveResource {
            kind: "conversation".to_string(),
            id: Some(conversation_id.to_string()),
        },
        cursor: None,
        payload,
        reason: None,
    }
}

fn conversations_envelope(
    event_type: &str,
    project_path: Option<&str>,
    payload: Value,
) -> LiveEnvelope {
    LiveEnvelope {
        event_type: event_type.to_string(),
        project_path: project_path.map(str::to_string),
        resource: LiveResource {
            kind: "conversations".to_string(),
            id: None,
        },
        cursor: None,
        payload,
        reason: None,
    }
}

/// A single-conversation scope's newest window and its watermark. A
/// conversation without rows gets an empty window.
pub fn conversation_window_envelope(
    service: &WorkspaceConversationService,
    conversation_id: &str,
    project_path: &str,
) -> WorkspaceResult<(LiveEnvelope, i64)> {
    let project_path = conversation_scope_project(service.store(), conversation_id, project_path)?;
    let database = service.store().database();
    let generation = database.generation();
    let (watermark, page) = database.read(|read| {
        Ok((
            read.watermark(),
            read.page(conversation_id, DEFAULT_PAGE_LIMIT, None)?,
        ))
    })?;
    let payload = match page {
        PageRead::Page(page) => page_wire(service, generation, conversation_id, page)?,
        PageRead::Missing | PageRead::Reset(_) => {
            empty_page_wire(generation, conversation_id, watermark)
        }
    };
    Ok((
        conversation_envelope(
            "conversation.window",
            conversation_id,
            &project_path,
            payload,
        ),
        watermark,
    ))
}

/// A single-conversation scope's changed rows after `since`, and the new
/// cursor; a reset ends the scope's cursor (`None`).
pub fn conversation_changes_envelope(
    service: &WorkspaceConversationService,
    conversation_id: &str,
    project_path: &str,
    generation: &str,
    since: i64,
) -> WorkspaceResult<(LiveEnvelope, Option<i64>)> {
    let database = service.store().database();
    let (watermark, catch_up) = database.read(|read| {
        Ok((
            read.watermark(),
            read.changes(
                ChangeScope::Conversation(conversation_id),
                generation,
                since,
            )?,
        ))
    })?;
    Ok(match catch_up {
        CatchUp::Batch(batch) => {
            let watermark = batch.watermark;
            let batch = decorated_batch(service, batch)?;
            (
                conversation_envelope(
                    "conversation.changes",
                    conversation_id,
                    project_path,
                    batch_wire(database.generation(), &batch),
                ),
                Some(watermark),
            )
        }
        CatchUp::Reset(reason) => (
            conversation_envelope(
                "conversation.reset",
                conversation_id,
                project_path,
                reset_wire("resync_required", reason, database.generation(), watermark),
            ),
            None,
        ),
    })
}

/// A summary scope's complete membership at one watermark: every live,
/// listed conversation of the project (or of every project), decorated with
/// standing roles, plus placeholder rows (revision 0) for roles without a
/// conversation yet. Mission conversations and draft-only conversations are
/// not listed.
pub fn conversations_snapshot_envelope(
    service: &WorkspaceConversationService,
    project_path: Option<&str>,
) -> WorkspaceResult<(LiveEnvelope, i64)> {
    let project_path = normalize_project_path_opt(project_path)?;
    let database = service.store().database();
    let (watermark, rows) =
        database.read(|read| Ok((read.watermark(), read.summaries(project_path.as_deref())?)))?;
    let mut rows = listed_summary_rows(service, rows);
    let projects = match project_path.as_deref() {
        Some(project_path) => vec![project_path.to_string()],
        None => spark_storage::ProjectRegistry::new(service.settings().data_dir.clone())
            .list_project_records()?
            .into_iter()
            .map(|project| project.project_path)
            .collect(),
    };
    let roles = crate::roles::WorkspaceRoleService::new(
        service.settings().clone(),
        service.store().clone(),
    );
    for project in projects {
        for role in roles.list(&project).unwrap_or_default() {
            // A touched role has a conversation row; one never touched has
            // only this placeholder.
            if rows.iter().any(|row| row.id == role.conversation_id) {
                continue;
            }
            rows.push(placeholder_row(&role, &project));
        }
    }
    let generation = database.generation();
    let payload = json!({
        "generation": generation,
        "watermark": watermark.to_string(),
        "project_path": project_path,
        "rows": rows.iter().map(row_wire).collect::<Vec<_>>(),
    });
    Ok((
        conversations_envelope("conversations.snapshot", project_path.as_deref(), payload),
        watermark,
    ))
}

/// A standing role's summary row before its conversation exists.
fn placeholder_row(role: &crate::roles::RoleRecord, project_path: &str) -> EntityRow {
    EntityRow {
        kind: EntityKind::Conversation,
        conversation_id: role.conversation_id.clone(),
        id: role.conversation_id.clone(),
        collection: None,
        turn_id: None,
        position: None,
        order: None,
        revision: 0,
        deleted: false,
        payload: Some(json!({
            "placeholder": true,
            "handle": "",
            "project_path": project_path,
            "created_at": "",
            "updated_at": "",
            "title": role.title,
            "preview": "",
            "running": false,
            "needs_attention": false,
            "launched_run_ids": [],
            "standing_role": {"name": role.name, "title": role.title, "state": role.state},
        })),
    }
}

/// A summary scope's changed conversation rows after `since`, and the new
/// cursor; a reset ends the scope's cursor (`None`).
pub fn conversations_changes_envelope(
    service: &WorkspaceConversationService,
    project_path: Option<&str>,
    generation: &str,
    since: i64,
) -> WorkspaceResult<(LiveEnvelope, Option<i64>)> {
    let database = service.store().database();
    let (watermark, catch_up) = database.read(|read| {
        Ok((
            read.watermark(),
            read.changes(ChangeScope::Summaries { project_path }, generation, since)?,
        ))
    })?;
    Ok(match catch_up {
        CatchUp::Batch(mut batch) => {
            batch.rows = listed_summary_rows(service, batch.rows);
            (
                conversations_envelope(
                    "conversations.changes",
                    project_path,
                    batch_wire(database.generation(), &batch),
                ),
                Some(batch.watermark),
            )
        }
        CatchUp::Reset(reason) => (
            conversations_envelope(
                "conversations.reset",
                project_path,
                reset_wire("resync_required", reason, database.generation(), watermark),
            ),
            None,
        ),
    })
}

/// Summary rows as thread lists show them: tombstones pass through, mission
/// and draft-only conversations are left out, standing roles are marked and
/// titled after the role.
fn listed_summary_rows(
    service: &WorkspaceConversationService,
    rows: Vec<EntityRow>,
) -> Vec<EntityRow> {
    let roles = crate::roles::WorkspaceRoleService::new(
        service.settings().clone(),
        service.store().clone(),
    );
    rows.into_iter()
        .filter_map(|mut row| {
            let Some(payload) = row.payload.as_mut() else {
                return Some(row);
            };
            // A conversation with only draft uploads has never been committed.
            if payload["metadata"]["revision"].as_i64().unwrap_or(0) == 0 {
                return None;
            }
            let project_path = payload["project_path"].as_str()?.to_string();
            let missions = spark_storage::ProjectRegistry::new(service.settings().data_dir.clone())
                .project_paths(&project_path)
                .ok()
                .map(|paths| {
                    spark_storage::workspace_records::RecordRepository::missions(&paths.root)
                });
            if missions.is_some_and(|missions| matches!(missions.read(&row.id), Ok(Some(_)))) {
                return None;
            }
            if let Some(role) = roles.conversation_role(&project_path, &row.id) {
                payload["title"] = json!(role.title);
                let state = role.state_beside(payload["running"] == true);
                payload["standing_role"] =
                    json!({"name": role.name, "title": role.title, "state": state});
            }
            Some(row)
        })
        .collect()
}
