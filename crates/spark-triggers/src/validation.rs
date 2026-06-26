use rand::RngCore;
use serde_json::{Map, Value};
use spark_storage::{
    normalize_trigger_action_payload, normalize_trigger_source_payload, TriggerAction,
    TriggerDefinition,
};
use time::OffsetDateTime;

use crate::credentials::generate_webhook_credentials;
use crate::error::{TriggerError, TriggerResult};
use crate::models::{
    TriggerCreateRequest, TriggerUpdateRequest, SOURCE_FLOW_EVENT, SOURCE_POLL, SOURCE_SCHEDULE,
    SOURCE_WEBHOOK,
};

pub fn normalize_trigger_create(
    request: TriggerCreateRequest,
) -> TriggerResult<(TriggerDefinition, Option<String>)> {
    let mut source = request.source;
    let mut webhook_secret = None;
    if request.source_type.trim() == SOURCE_WEBHOOK {
        let (webhook_key, secret, secret_hash) = generate_webhook_credentials(None);
        source.insert("webhook_key".to_string(), Value::String(webhook_key));
        source.insert("secret_hash".to_string(), Value::String(secret_hash));
        webhook_secret = Some(secret);
    }
    let definition = validate_trigger_definition_payload(
        None,
        request.name,
        request.enabled,
        false,
        request.source_type,
        request.action,
        source,
        None,
        None,
    )?;
    Ok((definition, webhook_secret))
}

pub fn normalize_trigger_update(
    existing: TriggerDefinition,
    request: TriggerUpdateRequest,
) -> TriggerResult<(TriggerDefinition, Option<String>)> {
    if existing.protected {
        if request.source.is_some() {
            return Err(validation(
                "Protected triggers do not allow source changes.",
            ));
        }
        if let Some(action_update) = request.action.as_ref() {
            let next_action = normalize_action(merge_action(&existing.action, action_update)?)?;
            if next_action.project_path != existing.action.project_path {
                return Err(validation(
                    "Protected triggers do not allow project target changes.",
                ));
            }
            if next_action.static_context != existing.action.static_context {
                return Err(validation(
                    "Protected triggers do not allow static context changes.",
                ));
            }
        }
        if request.regenerate_webhook_secret {
            return Err(validation(
                "Protected triggers do not support webhook secret regeneration.",
            ));
        }
    }

    let mut next_source = existing.source.clone();
    if let Some(source) = request.source {
        next_source = normalize_source(
            &existing.source_type,
            &source,
            existing.source.get("secret_hash"),
        )?;
    }

    let mut webhook_secret = None;
    if request.regenerate_webhook_secret {
        if existing.source_type != SOURCE_WEBHOOK {
            return Err(validation(
                "Only webhook triggers can regenerate webhook secrets.",
            ));
        }
        let webhook_key = next_source
            .get("webhook_key")
            .or_else(|| existing.source.get("webhook_key"))
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        if webhook_key.is_empty() {
            return Err(validation("Webhook trigger is missing a routing key."));
        }
        let (_, secret, secret_hash) = generate_webhook_credentials(Some(&webhook_key));
        next_source.insert("secret_hash".to_string(), Value::String(secret_hash));
        webhook_secret = Some(secret);
    }

    let next_action = if let Some(action_update) = request.action {
        merge_action(&existing.action, &action_update)?
    } else {
        action_to_map(&existing.action)
    };
    let definition = validate_trigger_definition_payload(
        Some(existing.id),
        request.name.unwrap_or(existing.name),
        request.enabled.unwrap_or(existing.enabled),
        existing.protected,
        existing.source_type,
        next_action,
        next_source,
        Some(existing.created_at),
        Some(iso_now()),
    )?;
    Ok((definition, webhook_secret))
}

fn validate_trigger_definition_payload(
    trigger_id: Option<String>,
    name: String,
    enabled: bool,
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
    let source = normalize_source(&source_type, &source, None)?;
    let now = iso_now();
    Ok(TriggerDefinition {
        id,
        name,
        enabled,
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
    preserve_secret_hash: Option<&Value>,
) -> TriggerResult<Map<String, Value>> {
    normalize_trigger_source_payload(source_type, payload, preserve_secret_hash).map_err(Into::into)
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
    payload.insert(
        "flow_name".to_string(),
        Value::String(action.flow_name.clone()),
    );
    payload.insert(
        "project_path".to_string(),
        action
            .project_path
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
    );
    payload.insert(
        "static_context".to_string(),
        Value::Object(action.static_context.clone()),
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
