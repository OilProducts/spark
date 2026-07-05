#![forbid(unsafe_code)]

//! Workspace-global trigger definition contracts and CRUD services.
//!
//! This crate owns workspace-global trigger repositories, protected edit
//! guards, credential mutation, and production source activation. Webhook
//! dispatch is routed through the source runtime so Workspace-owned launch
//! services can create real trigger-launched runs.

pub mod credentials;
mod error;
pub mod models;
pub mod sources;
pub mod state;
pub mod validation;

use serde_json::{Map, Value};
use spark_common::settings::SparkSettings;
use spark_storage::TriggerRepositories;

pub use credentials::{generate_webhook_credentials, verify_webhook_secret};
pub use error::{TriggerError, TriggerResult};
pub use models::{
    SerializedTrigger, TriggerAction, TriggerActivationOutcome, TriggerActivationRequest,
    TriggerActivationSinkOutcome, TriggerCreateRequest, TriggerDefinition, TriggerDeleteResponse,
    TriggerState, TriggerUpdateRequest, WebhookDispatchOutcome, WebhookHandleRequest,
    WebhookHandleResponse, SOURCE_FLOW_EVENT, SOURCE_POLL, SOURCE_SCHEDULE, SOURCE_WEBHOOK,
    TERMINAL_PIPELINE_STATUSES,
};
pub use sources::{AcceptAllTriggerActivationSink, TriggerActivationSink, TriggerSourceRuntime};
pub use validation::{normalize_trigger_create, normalize_trigger_update};

#[derive(Debug, Clone)]
pub struct TriggerService {
    repositories: TriggerRepositories,
}

impl TriggerService {
    pub fn new(settings: SparkSettings) -> Self {
        Self::with_repositories(TriggerRepositories::from_settings(&settings))
    }

    pub fn with_repositories(repositories: TriggerRepositories) -> Self {
        Self { repositories }
    }

    pub fn list_triggers(&self) -> TriggerResult<Vec<SerializedTrigger>> {
        let mut definitions = self.repositories.definitions.list()?;
        definitions.sort_by(|left, right| {
            (
                left.protected == false,
                left.name.to_ascii_lowercase(),
                &left.id,
            )
                .cmp(&(
                    right.protected == false,
                    right.name.to_ascii_lowercase(),
                    &right.id,
                ))
        });
        definitions
            .into_iter()
            .map(|definition| self.serialize_with_refreshed_state(definition, None))
            .collect()
    }

    pub fn get_trigger(&self, trigger_id: &str) -> TriggerResult<Option<SerializedTrigger>> {
        let Some(definition) = self.repositories.definitions.get(trigger_id)? else {
            return Ok(None);
        };
        self.serialize_with_refreshed_state(definition, None)
            .map(Some)
    }

    pub fn create_trigger(
        &self,
        request: TriggerCreateRequest,
    ) -> TriggerResult<SerializedTrigger> {
        let (definition, webhook_secret) = normalize_trigger_create(request)?;
        self.repositories.definitions.put(&definition)?;
        self.serialize_with_refreshed_state(definition, webhook_secret)
    }

    pub fn update_trigger(
        &self,
        trigger_id: &str,
        request: TriggerUpdateRequest,
    ) -> TriggerResult<SerializedTrigger> {
        let existing = self
            .repositories
            .definitions
            .get(trigger_id)?
            .ok_or(TriggerError::UnknownTrigger)?;
        let (definition, webhook_secret) = normalize_trigger_update(existing, request)?;
        self.repositories.definitions.put(&definition)?;
        self.serialize_with_refreshed_state(definition, webhook_secret)
    }

    pub fn delete_trigger(&self, trigger_id: &str) -> TriggerResult<TriggerDeleteResponse> {
        let definition = self
            .repositories
            .definitions
            .get(trigger_id)?
            .ok_or(TriggerError::UnknownTrigger)?;
        if definition.protected {
            return Err(TriggerError::ProtectedDelete);
        }
        self.repositories.definitions.delete(trigger_id)?;
        self.repositories.runtime_state.delete(trigger_id)?;
        Ok(TriggerDeleteResponse {
            status: "deleted".to_string(),
            id: trigger_id.to_string(),
        })
    }

    pub fn handle_webhook(
        &self,
        request: WebhookHandleRequest,
    ) -> TriggerResult<WebhookHandleResponse> {
        let definition = authenticate_webhook_request(&self.repositories, &request)?;
        Ok(WebhookHandleResponse {
            ok: true,
            trigger_id: definition.id,
        })
    }

    fn serialize_with_refreshed_state(
        &self,
        definition: TriggerDefinition,
        webhook_secret: Option<String>,
    ) -> TriggerResult<SerializedTrigger> {
        let state = self
            .repositories
            .runtime_state
            .update(&definition.id, |state| {
                state::refresh_next_run_at(&definition, state);
            })?;
        Ok(serialize_trigger(definition, state, webhook_secret))
    }
}

pub(crate) fn authenticate_webhook_request(
    repositories: &TriggerRepositories,
    request: &WebhookHandleRequest,
) -> TriggerResult<TriggerDefinition> {
    let webhook_key = request.webhook_key.trim();
    let definition = definition_for_webhook_key(repositories, webhook_key)?
        .ok_or(TriggerError::UnknownWebhookKey)?;
    if definition.source_type != SOURCE_WEBHOOK {
        return Err(TriggerError::Validation(
            "Webhook key does not resolve to a webhook trigger.".to_string(),
        ));
    }
    if !definition.enabled {
        return Err(TriggerError::Validation(
            "Webhook trigger is disabled.".to_string(),
        ));
    }
    let secret_hash = string_field(&definition.source, "secret_hash");
    if !verify_webhook_secret(&secret_hash, request.webhook_secret.trim()) {
        return Err(TriggerError::InvalidWebhookSecret);
    }
    Ok(definition)
}

fn definition_for_webhook_key(
    repositories: &TriggerRepositories,
    webhook_key: &str,
) -> TriggerResult<Option<TriggerDefinition>> {
    for definition in repositories.definitions.list()? {
        if definition.source_type != SOURCE_WEBHOOK {
            continue;
        }
        if string_field(&definition.source, "webhook_key").trim() == webhook_key {
            return Ok(Some(definition));
        }
    }
    Ok(None)
}

pub fn serialize_trigger(
    definition: TriggerDefinition,
    state: TriggerState,
    webhook_secret: Option<String>,
) -> SerializedTrigger {
    let mut source = definition.source.clone();
    if definition.source_type == SOURCE_WEBHOOK {
        source.remove("secret_hash");
    }
    SerializedTrigger {
        id: definition.id,
        name: definition.name,
        enabled: definition.enabled,
        protected: definition.protected,
        source_type: definition.source_type,
        created_at: definition.created_at,
        updated_at: definition.updated_at,
        action: definition.action,
        source: Value::Object(source),
        state,
        webhook_secret,
    }
}

fn string_field(source: &Map<String, Value>, key: &str) -> String {
    source
        .get(key)
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string()
}
