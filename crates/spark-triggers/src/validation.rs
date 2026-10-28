use rand::RngCore;
use serde_json::{Map, Value};
use spark_storage::{
    normalize_trigger_action_payload, normalize_trigger_source_payload, TriggerAction,
    TriggerDefinition,
};
use time::OffsetDateTime;

use crate::error::{TriggerError, TriggerResult};
use crate::models::{
    TriggerCreateRequest, TriggerUpdateRequest, SOURCE_FLOW_EVENT, SOURCE_POLL, SOURCE_SCHEDULE,
    SOURCE_WEBHOOK,
};

/// The definition a create request describes; the caller places it in
/// `request.project_path` and writes `enabled` and webhook credentials to
/// runtime state.
pub fn normalize_trigger_create(request: TriggerCreateRequest) -> TriggerResult<TriggerDefinition> {
    validate_trigger_definition_payload(
        None,
        request.name,
        false,
        request.source_type,
        request.action,
        request.source,
        None,
        None,
    )
}

/// The definition an update request makes of `existing`, keeping its
/// project and revision. `enabled` and webhook secrets are runtime state,
/// left to the caller.
pub fn normalize_trigger_update(
    existing: TriggerDefinition,
    request: TriggerUpdateRequest,
) -> TriggerResult<TriggerDefinition> {
    if existing.protected {
        if request.source.is_some() {
            return Err(validation(
                "Protected triggers do not allow source changes.",
            ));
        }
        if let Some(action_update) = request.action.as_ref() {
            let next_action = normalize_action(merge_action(&existing.action, action_update)?)?;
            if next_action.mode != existing.action.mode {
                return Err(validation(
                    "Protected triggers do not allow action mode changes.",
                ));
            }
            if next_action.mission_id != existing.action.mission_id {
                return Err(validation(
                    "Protected triggers do not allow mission target changes.",
                ));
            }
            if next_action.role != existing.action.role {
                return Err(validation(
                    "Protected triggers do not allow role target changes.",
                ));
            }
            if next_action.static_context != existing.action.static_context {
                return Err(validation(
                    "Protected triggers do not allow static context changes.",
                ));
            }
            if next_action.execution_profile_id != existing.action.execution_profile_id {
                return Err(validation(
                    "Protected triggers do not allow execution profile changes.",
                ));
            }
        }
        if request.regenerate_webhook_secret {
            return Err(validation(
                "Protected triggers do not support webhook secret regeneration.",
            ));
        }
    }

    if request.regenerate_webhook_secret && existing.source_type != SOURCE_WEBHOOK {
        return Err(validation(
            "Only webhook triggers can regenerate webhook secrets.",
        ));
    }
    let next_source = match request.source {
        Some(source) => normalize_source(&existing.source_type, &source)?,
        None => existing.source.clone(),
    };

    let next_action = if let Some(action_update) = request.action {
        merge_action(&existing.action, &action_update)?
    } else {
        action_to_map(&existing.action)
    };
    let mut definition = validate_trigger_definition_payload(
        Some(existing.id),
        request.name.unwrap_or(existing.name),
        existing.protected,
        existing.source_type,
        next_action,
        next_source,
        Some(existing.created_at),
        Some(iso_now()),
    )?;
    definition.revision = existing.revision;
    definition.project_path = existing.project_path;
    Ok(definition)
}

fn validate_trigger_definition_payload(
    trigger_id: Option<String>,
    name: String,
    protected: bool,
    source_type: String,
    action: Map<String, Value>,
    source: Map<String, Value>,
    created_at: Option<String>,
    updated_at: Option<String>,
) -> TriggerResult<TriggerDefinition> {
    let id = trigger_id.unwrap_or_else(generate_trigger_id);
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(validation("Trigger name is required."));
    }
    let source_type = source_type.trim().to_string();
    if ![
        SOURCE_SCHEDULE,
        SOURCE_POLL,
        SOURCE_WEBHOOK,
        SOURCE_FLOW_EVENT,
    ]
    .contains(&source_type.as_str())
    {
        return Err(validation(format!(
            "Unsupported trigger source type: {}",
            source_type
        )));
    }
    let action = normalize_action(action)?;
    let source = normalize_source(&source_type, &source)?;
    let now = iso_now();
    Ok(TriggerDefinition {
        revision: String::new(),
        project_path: String::new(),
        id,
        name,
        protected,
        source_type,
        action,
        source,
        created_at: created_at.unwrap_or_else(|| now.clone()),
        updated_at: updated_at.unwrap_or(now),
    })
}

fn normalize_action(payload: Map<String, Value>) -> TriggerResult<TriggerAction> {
    normalize_trigger_action_payload(&payload).map_err(Into::into)
}

fn normalize_source(
    source_type: &str,
    payload: &Map<String, Value>,
) -> TriggerResult<Map<String, Value>> {
    normalize_trigger_source_payload(source_type, payload).map_err(Into::into)
}

fn merge_action(
    existing: &TriggerAction,
    update: &Map<String, Value>,
) -> TriggerResult<Map<String, Value>> {
    let mut merged = action_to_map(existing);
    for (key, value) in update {
        merged.insert(key.clone(), value.clone());
    }
    Ok(merged)
}

fn action_to_map(action: &TriggerAction) -> Map<String, Value> {
    let mut payload = Map::new();
    payload.insert("mission_id".into(), serde_json::json!(action.mission_id));
    payload.insert("role".into(), serde_json::json!(action.role));
    payload.insert("mode".to_string(), Value::String(action.mode.clone()));
    payload.insert(
        "flow_name".to_string(),
        Value::String(action.flow_name.clone()),
    );
    payload.insert(
        "static_context".to_string(),
        Value::Object(action.static_context.clone()),
    );
    payload.insert(
        "execution_profile_id".to_string(),
        action
            .execution_profile_id
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
    );
    payload
}

fn generate_trigger_id() -> String {
    let mut bytes = [0u8; 6];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!(
        "trigger-{}",
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
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

fn validation(message: impl Into<String>) -> TriggerError {
    TriggerError::Validation(message.into())
}
