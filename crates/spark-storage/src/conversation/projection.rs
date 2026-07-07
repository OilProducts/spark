use std::path::PathBuf;

use serde_json::{json, Map, Value};

use crate::error::{Result, StorageError};

use super::records::{ConversationArtifacts, ConversationMeta, ConversationRecord, Transcript};

/// Project a legacy merged snapshot (`read_snapshot` output) into typed
/// records. Unknown turn/segment fields are preserved; the snapshot must have
/// already passed stored-schema validation.
pub fn record_from_snapshot(snapshot: &Value) -> Result<ConversationRecord> {
    let object = snapshot
        .as_object()
        .ok_or_else(|| StorageError::InvalidDocumentShape {
            path: PathBuf::from("conversation snapshot"),
            format: "JSON",
            expected: "object",
        })?;
    let meta: ConversationMeta =
        serde_json::from_value(snapshot.clone()).map_err(|source| StorageError::JsonRead {
            path: PathBuf::from("conversation snapshot"),
            source,
        })?;
    let transcript = Transcript {
        turns: typed_array(object, "turns")?,
        segments: typed_array(object, "segments")?,
    };
    let artifacts = ConversationArtifacts {
        event_log: value_array(object, "event_log"),
        flow_run_requests: value_array(object, "flow_run_requests"),
        flow_launches: value_array(object, "flow_launches"),
        run_recoveries: value_array(object, "run_recoveries"),
        proposed_plans: value_array(object, "proposed_plans"),
    };
    Ok(ConversationRecord {
        meta,
        transcript,
        artifacts,
    })
}

/// Project typed records into the legacy full snapshot shape consumed by
/// `write_snapshot`, hydration responses, and `conversation_snapshot`
/// payloads.
pub fn snapshot_from_record(record: &ConversationRecord) -> Value {
    let meta = &record.meta;
    let mut object = Map::new();
    object.insert("schema_version".to_string(), json!(meta.schema_version));
    object.insert("revision".to_string(), json!(meta.revision));
    object.insert(
        "conversation_id".to_string(),
        json!(meta.conversation_id.clone()),
    );
    object.insert(
        "conversation_handle".to_string(),
        json!(meta.conversation_handle.clone()),
    );
    object.insert("project_path".to_string(), json!(meta.project_path.clone()));
    object.insert("chat_mode".to_string(), json!(meta.chat_mode.clone()));
    object.insert("provider".to_string(), json!(meta.provider.clone()));
    object.insert("model".to_string(), optional_string(&meta.model));
    object.insert(
        "llm_profile".to_string(),
        optional_string(&meta.llm_profile),
    );
    object.insert(
        "reasoning_effort".to_string(),
        optional_string(&meta.reasoning_effort),
    );
    object.insert("title".to_string(), json!(meta.title.clone()));
    object.insert("created_at".to_string(), json!(meta.created_at.clone()));
    object.insert("updated_at".to_string(), json!(meta.updated_at.clone()));
    object.insert(
        "turns".to_string(),
        serde_json::to_value(&record.transcript.turns).unwrap_or_else(|_| json!([])),
    );
    object.insert(
        "segments".to_string(),
        serde_json::to_value(&record.transcript.segments).unwrap_or_else(|_| json!([])),
    );
    object.insert(
        "event_log".to_string(),
        Value::Array(record.artifacts.event_log.clone()),
    );
    object.insert(
        "flow_run_requests".to_string(),
        Value::Array(record.artifacts.flow_run_requests.clone()),
    );
    object.insert(
        "flow_launches".to_string(),
        Value::Array(record.artifacts.flow_launches.clone()),
    );
    object.insert(
        "run_recoveries".to_string(),
        Value::Array(record.artifacts.run_recoveries.clone()),
    );
    object.insert(
        "proposed_plans".to_string(),
        Value::Array(record.artifacts.proposed_plans.clone()),
    );
    Value::Object(object)
}

fn typed_array<T: serde::de::DeserializeOwned>(
    object: &Map<String, Value>,
    key: &'static str,
) -> Result<Vec<T>> {
    let Some(value) = object.get(key) else {
        return Ok(Vec::new());
    };
    serde_json::from_value(value.clone()).map_err(|source| StorageError::JsonRead {
        path: PathBuf::from(format!("conversation snapshot {key}")),
        source,
    })
}

fn value_array(object: &Map<String, Value>, key: &str) -> Vec<Value> {
    object
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn optional_string(value: &Option<String>) -> Value {
    match value {
        Some(value) => json!(value.clone()),
        None => Value::Null,
    }
}
