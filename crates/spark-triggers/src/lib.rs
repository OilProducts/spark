#![forbid(unsafe_code)]

//! Trigger definition contracts and CRUD services.
//!
//! A trigger's definition lives in its project's org repo and its runtime
//! state under Spark's data directory. This crate owns protected edit
//! guards, credential mutation, and production source activation. Webhook
//! dispatch is routed through the source runtime so Workspace-owned launch
//! services can create real trigger-launched runs.

pub mod credentials;
mod error;
pub mod models;
pub mod sources;
pub mod state;
pub mod validation;

use serde_json::Value;
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

    /// The triggers of `project_path`, or of every registered project.
    pub fn list_triggers(
        &self,
        project_path: Option<&str>,
    ) -> TriggerResult<Vec<SerializedTrigger>> {
        let mut definitions = match project_path {
            Some(project_path) => self.repositories.definitions(project_path)?.list()?,
            None => self.repositories.list_all()?,
        };
        definitions.sort_by(|left, right| {
            (
                !left.protected,
                left.name.to_ascii_lowercase(),
                &left.project_path,
                &left.id,
            )
                .cmp(&(
                    !right.protected,
                    right.name.to_ascii_lowercase(),
                    &right.project_path,
                    &right.id,
                ))
        });
        definitions
            .into_iter()
            .map(|definition| self.serialize_with_refreshed_state(definition, |_| {}))
            .collect()
    }

    pub fn get_trigger(
        &self,
        project_path: &str,
        trigger_id: &str,
    ) -> TriggerResult<Option<SerializedTrigger>> {
        let Some(definition) = self
            .repositories
            .definitions(project_path)?
            .get(trigger_id)?
        else {
            return Ok(None);
        };
        self.serialize_with_refreshed_state(definition, |_| {})
            .map(Some)
    }

    pub fn create_trigger(
        &self,
        request: TriggerCreateRequest,
    ) -> TriggerResult<SerializedTrigger> {
        let repository = self.repositories.definitions(&request.project_path)?;
        let enabled = request.enabled;
        let message = request.message.clone();
        let mut definition = normalize_trigger_create(request)?;
        definition.project_path = repository.project_path().to_string();
        // State first, so the source loop never sees the new definition with
        // default state (enabled, no credentials); dropped again if the
        // definition's commit fails.
        let states = self.repositories.runtime_state(&definition.project_path)?;
        let mut webhook_secret = None;
        let mut initial = TriggerState {
            enabled,
            ..TriggerState::default()
        };
        if definition.source_type == SOURCE_WEBHOOK {
            webhook_secret = Some(set_webhook_credentials(&mut initial));
        }
        states.save(&definition.id, &initial)?;
        definition.revision = repository
            .put(&definition, message.as_deref())
            .inspect_err(|_| {
                let _ = states.delete(&definition.id);
            })?;
        let mut trigger = self.serialize_with_refreshed_state(definition, |_| {})?;
        trigger.webhook_secret = webhook_secret;
        Ok(trigger)
    }

    /// Commits the definition only when it changed: switching `enabled` or
    /// regenerating a webhook secret writes runtime state alone.
    pub fn update_trigger(
        &self,
        project_path: &str,
        trigger_id: &str,
        request: TriggerUpdateRequest,
    ) -> TriggerResult<SerializedTrigger> {
        let repository = self.repositories.definitions(project_path)?;
        let existing = repository
            .get(trigger_id)?
            .ok_or(TriggerError::UnknownTrigger)?;
        if request.expected_revision.is_empty() {
            return Err(TriggerError::Validation(
                "expected_revision is required.".into(),
            ));
        }
        if existing.revision != request.expected_revision {
            return Err(spark_storage::StorageError::SettingsConflict {
                path: repository.definition_path(trigger_id)?,
            }
            .into());
        }
        let enabled = request.enabled;
        let regenerate = request.regenerate_webhook_secret;
        let message = request.message.clone();
        let mut definition = normalize_trigger_update(existing.clone(), request)?;
        let unchanged = TriggerDefinition {
            updated_at: existing.updated_at.clone(),
            ..definition.clone()
        } == existing;
        if unchanged {
            definition = existing;
        } else {
            definition.revision = repository.put(&definition, message.as_deref())?;
        }
        let mut webhook_secret = None;
        let mut trigger = self.serialize_with_refreshed_state(definition, |state| {
            if let Some(enabled) = enabled {
                state.enabled = enabled;
            }
            if regenerate {
                webhook_secret = Some(set_webhook_credentials(state));
            }
        })?;
        trigger.webhook_secret = webhook_secret;
        Ok(trigger)
    }

    pub fn delete_trigger(
        &self,
        project_path: &str,
        trigger_id: &str,
        expected_revision: &str,
        message: Option<&str>,
    ) -> TriggerResult<TriggerDeleteResponse> {
        let repository = self.repositories.definitions(project_path)?;
        let definition = repository
            .get(trigger_id)?
            .ok_or(TriggerError::UnknownTrigger)?;
        if definition.protected {
            return Err(TriggerError::ProtectedDelete);
        }
        if definition.revision != expected_revision {
            return Err(spark_storage::StorageError::SettingsConflict {
                path: repository.definition_path(trigger_id)?,
            }
            .into());
        }
        repository.delete(trigger_id, expected_revision, message)?;
        self.repositories
            .runtime_state(project_path)?
            .delete(trigger_id)?;
        Ok(TriggerDeleteResponse {
            status: "deleted".to_string(),
            id: trigger_id.to_string(),
            project_path: definition.project_path,
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
        update: impl FnOnce(&mut TriggerState),
    ) -> TriggerResult<SerializedTrigger> {
        let state = self
            .repositories
            .runtime_state(&definition.project_path)?
            .update(&definition.id, |state| {
                update(state);
                state::refresh_next_run_at(&definition, state);
            })?;
        Ok(serialize_trigger(definition, state, None))
    }
}

/// Gives a webhook trigger a new secret, keeping its key or making one if it
/// has none (a definition copied from another project). Returns the secret.
fn set_webhook_credentials(state: &mut TriggerState) -> String {
    let (key, secret, secret_hash) = generate_webhook_credentials(state.webhook_key.as_deref());
    state.webhook_key = Some(key);
    state.secret_hash = Some(secret_hash);
    secret
}

pub(crate) fn authenticate_webhook_request(
    repositories: &TriggerRepositories,
    request: &WebhookHandleRequest,
) -> TriggerResult<TriggerDefinition> {
    let webhook_key = request.webhook_key.trim();
    let (definition, state) = definition_for_webhook_key(repositories, webhook_key)?
        .ok_or(TriggerError::UnknownWebhookKey)?;
    if !state.enabled {
        return Err(TriggerError::Validation(
            "Webhook trigger is disabled.".to_string(),
        ));
    }
    let secret_hash = state.secret_hash.unwrap_or_default();
    if !verify_webhook_secret(&secret_hash, request.webhook_secret.trim()) {
        return Err(TriggerError::InvalidWebhookSecret);
    }
    Ok(definition)
}

/// The webhook trigger whose key, kept in runtime state, is `webhook_key`.
fn definition_for_webhook_key(
    repositories: &TriggerRepositories,
    webhook_key: &str,
) -> TriggerResult<Option<(TriggerDefinition, TriggerState)>> {
    if webhook_key.is_empty() {
        return Ok(None);
    }
    for definition in repositories.list_all()? {
        if definition.source_type != SOURCE_WEBHOOK {
            continue;
        }
        let state = repositories
            .runtime_state(&definition.project_path)?
            .load(&definition.id)?;
        if state.webhook_key.as_deref() == Some(webhook_key) {
            return Ok(Some((definition, state)));
        }
    }
    Ok(None)
}

/// The API's trigger: `enabled` and the webhook key come from runtime state;
/// the secret hash never leaves it.
pub fn serialize_trigger(
    definition: TriggerDefinition,
    mut state: TriggerState,
    webhook_secret: Option<String>,
) -> SerializedTrigger {
    let mut source = definition.source.clone();
    state.secret_hash = None;
    if let Some(key) = state.webhook_key.take() {
        source.insert("webhook_key".to_string(), Value::String(key));
    }
    SerializedTrigger {
        revision: definition.revision,
        id: definition.id,
        project_path: definition.project_path,
        name: definition.name,
        enabled: state.enabled,
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
