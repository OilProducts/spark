use attractor_core::JournalEntry;
use attractor_runtime::RunStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use spark_common::project::normalize_project_path;
use spark_common::settings::SparkSettings;
use spark_storage::conversation::{CONVERSATION_SNAPSHOT_REF_TYPE, TRANSIENT_STREAM_EVENT_TYPE};

use crate::conversations::WorkspaceConversationService;
use crate::triggers::WorkspaceTriggerService;
use crate::{WorkspaceError, WorkspaceResult};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RawLiveQuery {
    pub project_path: Option<String>,
    pub conversation_id: Option<String>,
    pub conversation_project_path: Option<String>,
    pub conversation_revision: Option<String>,
    pub run_id: Option<String>,
    pub run_sequence: Option<String>,
    pub include_runs_overview: Option<String>,
    pub runs_project_path: Option<String>,
    pub include_triggers: Option<String>,
    pub triggers_project_path: Option<String>,
    pub include_workflow_log: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveQuery {
    pub project_path: Option<String>,
    pub conversation_id: Option<String>,
    pub conversation_project_path: Option<String>,
    pub conversation_revision: Option<i64>,
    pub run_id: Option<String>,
    pub run_sequence: Option<u64>,
    pub include_runs_overview: bool,
    pub runs_project_path: Option<String>,
    pub include_triggers: bool,
    pub triggers_project_path: Option<String>,
    pub include_workflow_log: bool,
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
    let conversation_revision = parse_non_negative_i64(
        raw.conversation_revision.as_deref(),
        "conversation_revision",
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
        conversation_revision,
        run_id,
        run_sequence,
        include_runs_overview,
        runs_project_path,
        include_triggers,
        triggers_project_path,
        include_workflow_log,
    })
}

pub fn initial_live_envelopes(
    settings: &SparkSettings,
    query: &LiveQuery,
) -> WorkspaceResult<Vec<LiveEnvelope>> {
    let mut envelopes = Vec::new();
    if let Some(conversation_id) = query.conversation_id.as_deref() {
        let project_path = query.conversation_project_path.as_deref().ok_or_else(|| {
            WorkspaceError::Validation(
                "conversation_project_path is required when conversation_id is provided."
                    .to_string(),
            )
        })?;
        envelopes.extend(conversation_envelopes(
            settings,
            conversation_id,
            project_path,
            query.conversation_revision,
        )?);
    }
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

pub fn conversation_envelopes_after(
    settings: &SparkSettings,
    conversation_id: &str,
    project_path: &str,
    revision: i64,
) -> WorkspaceResult<Vec<LiveEnvelope>> {
    conversation_envelopes(settings, conversation_id, project_path, Some(revision))
}

pub fn conversation_snapshot_envelope(
    settings: &SparkSettings,
    conversation_id: &str,
    project_path: &str,
) -> WorkspaceResult<LiveEnvelope> {
    let service = WorkspaceConversationService::new(settings.clone());
    let snapshot = service.get_snapshot(conversation_id, Some(project_path))?;
    Ok(conversation_snapshot_envelope_from_state(
        conversation_id,
        project_path,
        snapshot,
    ))
}

pub fn run_envelopes_after(
    settings: &SparkSettings,
    run_id: &str,
    run_sequence: u64,
) -> WorkspaceResult<Vec<LiveEnvelope>> {
    run_envelopes(settings, run_id, Some(run_sequence))
}

pub fn latest_run_sequence(settings: &SparkSettings, run_id: &str) -> WorkspaceResult<Option<u64>> {
    let store = RunStore::for_settings(settings);
    let Some(entries) = run_journal_entries(&store, run_id)? else {
        return Ok(None);
    };
    Ok(entries.into_iter().map(|entry| entry.sequence).max())
}

/// Segment upserts for every projected transcript segment touched after the
/// given combined-journal sequence. Full-segment snapshots on the run
/// resource with the shared run_sequence cursor, so existing query gating
/// and replay semantics apply unchanged.
pub fn run_segment_envelopes_after(
    settings: &SparkSettings,
    run_id: &str,
    after_sequence: u64,
) -> WorkspaceResult<Vec<LiveEnvelope>> {
    let store = RunStore::for_settings(settings);
    let Some(entries) = run_journal_entries(&store, run_id)? else {
        return Ok(Vec::new());
    };
    let projection = attractor_runtime::project_run_segments(&entries);
    Ok(projection
        .segments
        .into_iter()
        .filter(|segment| {
            segment
                .get("latest_sequence")
                .and_then(Value::as_u64)
                .is_some_and(|sequence| sequence > after_sequence)
        })
        .map(|segment| run_segment_upsert_envelope(run_id, segment))
        .collect())
}

fn run_segment_upsert_envelope(run_id: &str, segment: Value) -> LiveEnvelope {
    let sequence = segment
        .get("latest_sequence")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    LiveEnvelope {
        event_type: "run.segment_upsert".to_string(),
        project_path: None,
        resource: LiveResource {
            kind: "run".to_string(),
            id: Some(run_id.to_string()),
        },
        cursor: Some(LiveCursor {
            kind: "run_sequence".to_string(),
            value: sequence as i64,
        }),
        payload: json!({"run_id": run_id, "segment": segment}),
        reason: None,
    }
}

pub fn run_upsert_envelope(
    settings: &SparkSettings,
    run_id: &str,
) -> WorkspaceResult<Option<LiveEnvelope>> {
    let store = RunStore::for_settings(settings);
    let Some(bundle) = store
        .read_run_bundle(run_id)
        .map_err(|error| WorkspaceError::Internal(error.to_string()))?
    else {
        return Ok(None);
    };
    let Some(record) = bundle.record else {
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
        // The workflow log is a global, all-project feed by design.
        "workflow_log" => query.include_workflow_log,
        _ => false,
    }
}

fn conversation_envelopes(
    settings: &SparkSettings,
    conversation_id: &str,
    project_path: &str,
    revision: Option<i64>,
) -> WorkspaceResult<Vec<LiveEnvelope>> {
    let service = WorkspaceConversationService::new(settings.clone());
    let snapshot = service.get_snapshot(conversation_id, Some(project_path))?;
    let snapshot_revision = value_revision(&snapshot).unwrap_or(0);
    let Some(revision) = revision else {
        return Ok(vec![conversation_snapshot_envelope_from_state(
            conversation_id,
            project_path,
            snapshot,
        )]);
    };
    if snapshot_revision <= revision {
        return Ok(Vec::new());
    }
    let events = service.read_events_after(conversation_id, project_path, revision)?;
    if events.is_empty() {
        return Ok(vec![resync_required(
            "conversation",
            Some(conversation_id.to_string()),
            Some(project_path.to_string()),
            "conversation journal cannot replay from the requested revision",
        )]);
    }
    let mut envelopes = Vec::new();
    let mut expected_revision = revision + 1;
    let mut saw_snapshot_ref = false;
    for event in events {
        let Some(event_revision) = value_revision(&event) else {
            continue;
        };
        if event_revision != expected_revision {
            envelopes.push(resync_required(
                "conversation",
                Some(conversation_id.to_string()),
                Some(project_path.to_string()),
                "conversation journal has a revision gap from the requested cursor",
            ));
            return Ok(envelopes);
        }
        if event.get("type").and_then(Value::as_str) == Some(CONVERSATION_SNAPSHOT_REF_TYPE) {
            // Snapshot-level commits journal slim ref lines that never embed
            // state; the current committed snapshot supersedes the ref and
            // everything after it.
            saw_snapshot_ref = true;
            break;
        }
        envelopes.push(conversation_event_envelope(
            conversation_id,
            project_path,
            event,
            event_revision,
        ));
        expected_revision += 1;
    }
    if saw_snapshot_ref {
        envelopes.push(conversation_snapshot_envelope_from_state(
            conversation_id,
            project_path,
            snapshot,
        ));
        return Ok(envelopes);
    }
    if expected_revision <= snapshot_revision {
        envelopes.push(resync_required(
            "conversation",
            Some(conversation_id.to_string()),
            Some(project_path.to_string()),
            "conversation journal cannot replay from the requested revision",
        ));
    }
    Ok(envelopes)
}

fn conversation_snapshot_envelope_from_state(
    conversation_id: &str,
    project_path: &str,
    snapshot: Value,
) -> LiveEnvelope {
    let snapshot_revision = value_revision(&snapshot).unwrap_or(0);
    LiveEnvelope {
        event_type: "conversation.snapshot".to_string(),
        project_path: Some(project_path.to_string()),
        resource: LiveResource {
            kind: "conversation".to_string(),
            id: Some(conversation_id.to_string()),
        },
        cursor: Some(LiveCursor {
            kind: "conversation_revision".to_string(),
            value: snapshot_revision,
        }),
        payload: json!({ "state": snapshot }),
        reason: None,
    }
}

pub fn conversation_event_envelope(
    conversation_id: &str,
    project_path: &str,
    event: Value,
    revision: i64,
) -> LiveEnvelope {
    let raw_type = event.get("type").and_then(Value::as_str).unwrap_or("");
    if raw_type == TRANSIENT_STREAM_EVENT_TYPE {
        // Transient deltas are stream-local: they carry a stream-sequence
        // cursor instead of a committed revision and are never journaled.
        let stream_sequence = event
            .get("stream_sequence")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        return LiveEnvelope {
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
        };
    }
    let event_type = match raw_type {
        "turn_upsert" => "conversation.turn_upsert".to_string(),
        "segment_upsert" => "conversation.segment_upsert".to_string(),
        "conversation_snapshot" => "conversation.snapshot".to_string(),
        "" => "conversation.event".to_string(),
        other => format!("conversation.{other}"),
    };
    let payload = if raw_type == "conversation_snapshot" {
        event
            .get("state")
            .cloned()
            .map_or_else(|| event.clone(), |state| json!({ "state": state }))
    } else {
        event
    };
    LiveEnvelope {
        event_type,
        project_path: Some(project_path.to_string()),
        resource: LiveResource {
            kind: "conversation".to_string(),
            id: Some(conversation_id.to_string()),
        },
        cursor: Some(LiveCursor {
            kind: "conversation_revision".to_string(),
            value: revision,
        }),
        payload,
        reason: None,
    }
}

fn run_envelopes(
    settings: &SparkSettings,
    run_id: &str,
    run_sequence: Option<u64>,
) -> WorkspaceResult<Vec<LiveEnvelope>> {
    let store = RunStore::for_settings(settings);
    let Some(entries) = run_journal_entries(&store, run_id)? else {
        return Err(WorkspaceError::NotFound("Unknown pipeline".to_string()));
    };

    let after_sequence = run_sequence.unwrap_or(0);
    let max_sequence = entries
        .iter()
        .map(|entry| entry.sequence)
        .max()
        .unwrap_or(0);
    let mut expected_sequence = after_sequence.saturating_add(1);
    let mut envelopes = Vec::new();
    for entry in entries
        .into_iter()
        .filter(|entry| entry.sequence > after_sequence)
    {
        if entry.sequence != expected_sequence {
            envelopes.push(resync_required(
                "run",
                Some(run_id.to_string()),
                None,
                "run journal no longer contains a contiguous replay from the requested cursor",
            ));
            return Ok(envelopes);
        }
        envelopes.push(run_journal_envelope(run_id, entry));
        expected_sequence = expected_sequence.saturating_add(1);
    }
    if envelopes.is_empty() && max_sequence > after_sequence {
        envelopes.push(resync_required(
            "run",
            Some(run_id.to_string()),
            None,
            "run journal no longer contains a contiguous replay from the requested cursor",
        ));
        return Ok(envelopes);
    }
    // Segment upserts are otherwise only published live: replay the projected
    // segments touched after the cursor so a late subscriber converges to the
    // same transcript without a resync.
    envelopes.extend(run_segment_envelopes_after(
        settings,
        run_id,
        after_sequence,
    )?);
    Ok(envelopes)
}

fn run_journal_entries(
    store: &RunStore,
    run_id: &str,
) -> WorkspaceResult<Option<Vec<JournalEntry>>> {
    attractor_runtime::combined_run_journal_entries(store, run_id)
        .map_err(|error| WorkspaceError::Internal(error.to_string()))
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
    let mut records = store
        .list_run_records()
        .map_err(|error| WorkspaceError::Internal(error.to_string()))?;
    if let Some(project_path) = project_path {
        records.retain(|record| {
            normalized_record_path(&record.project_path).as_deref() == Some(project_path)
                || normalized_record_path(&record.working_directory).as_deref()
                    == Some(project_path)
        });
    }
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
    project_path: Option<&str>,
) -> WorkspaceResult<LiveEnvelope> {
    let mut triggers = WorkspaceTriggerService::new(settings.clone()).list_triggers()?;
    if let Some(project_path) = project_path {
        triggers.retain(|trigger| {
            let value = serde_json::to_value(trigger).unwrap_or_else(|_| json!({}));
            trigger_project_path(&value).as_deref() == Some(project_path)
        });
    }
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

fn value_revision(value: &Value) -> Option<i64> {
    value
        .get("revision")
        .and_then(Value::as_i64)
        .or_else(|| value.get("state")?.get("revision")?.as_i64())
}

fn public_run_record(record: attractor_core::RunRecord) -> Value {
    json!({
        "run_id": record.run_id,
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
        .get("action")
        .and_then(Value::as_object)
        .and_then(|action| action.get("project_path"))
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
