#![forbid(unsafe_code)]

//! Trigger definition contracts and CRUD services.
//!
//! A trigger's definition lives in its project's `.spark/definitions`; its
//! activation, private bindings and runtime state live in the state database.
//! A definition alone is available but inactive. This crate owns protected edit
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
use spark_storage::{TriggerIssue, TriggerRepositories};

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
    /// Reads write nothing: a definition without activation shows inactive.
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
            .map(|definition| self.serialize_current(definition))
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
        self.serialize_current(definition).map(Some)
    }

    /// Saves the definition, then its private state (mission binding and
    /// webhook credentials), inactive. `enabled` then activates it; a
    /// failed activation leaves the definition available but inactive, with
    /// the reason in `state.activation_error`.
    pub fn create_trigger(
        &self,
        request: TriggerCreateRequest,
    ) -> TriggerResult<SerializedTrigger> {
        let repository = self.repositories.definitions(&request.project_path)?;
        let enabled = request.enabled;
        let message = request.message.clone();
        let mut definition = normalize_trigger_create(request)?;
        definition.project_path = repository.project_path().to_string();
        let mission_id = definition.action.mission_id.clone();
        definition.revision = repository.put(&definition, message.as_deref())?;
        let mut webhook_secret = None;
        self.repositories
            .runtime_state(&definition.project_path)?
            .update(&definition.id, |state| {
                // Whatever a removed namesake left is never restored.
                *state = TriggerState {
                    mission_id,
                    ..TriggerState::default()
                };
                if definition.source_type == SOURCE_WEBHOOK {
                    webhook_secret = Some(set_webhook_credentials(state));
                }
            })?;
        let mut trigger = if enabled {
            self.activate_trigger(&definition.project_path, &definition.id)?
        } else {
            self.serialize_current(definition)?
        };
        trigger.webhook_secret = webhook_secret.or(trigger.webhook_secret);
        Ok(trigger)
    }

    /// Commits the definition only when its file content changed: a mission
    /// binding, switching `enabled` or regenerating a webhook secret writes
    /// private state alone. `enabled: true` revalidates and activates;
    /// `false` deactivates; a changed binding deactivates.
    pub fn update_trigger(
        &self,
        project_path: &str,
        trigger_id: &str,
        request: TriggerUpdateRequest,
    ) -> TriggerResult<SerializedTrigger> {
        let repository = self.repositories.definitions(project_path)?;
        let states = self.repositories.runtime_state(project_path)?;
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
        let current = states.get(trigger_id)?;
        let existing_binding = current.as_ref().and_then(|state| state.mission_id.clone());
        let enabled = request.enabled;
        let regenerate = request.regenerate_webhook_secret;
        let message = request.message.clone();
        let mut definition =
            normalize_trigger_update(current.unwrap_or_default().bind(existing.clone()), request)?;
        let mission_id = definition.action.mission_id.take();
        let unchanged = TriggerDefinition {
            updated_at: existing.updated_at.clone(),
            ..definition.clone()
        } == existing;
        if unchanged {
            definition = existing;
        } else {
            definition.revision = repository.put(&definition, message.as_deref())?;
        }
        let rebound = mission_id != existing_binding;
        let mut webhook_secret = None;
        if rebound || regenerate || enabled == Some(false) {
            states.update(trigger_id, |state| {
                if rebound {
                    state.mission_id = mission_id;
                    state.enabled = false;
                }
                if enabled == Some(false) {
                    state.enabled = false;
                    state.activation_error = None;
                }
                if regenerate {
                    webhook_secret = Some(set_webhook_credentials(state));
                }
            })?;
        }
        let mut trigger = if enabled == Some(true) {
            self.activate_trigger(project_path, trigger_id)?
        } else {
            self.serialize_current(definition)?
        };
        trigger.webhook_secret = webhook_secret.or(trigger.webhook_secret);
        Ok(trigger)
    }

    /// Activates the definition as it reads now, after checking its private
    /// bindings; see [`Self::activate_trigger_with`].
    pub fn activate_trigger(
        &self,
        project_path: &str,
        trigger_id: &str,
    ) -> TriggerResult<SerializedTrigger> {
        self.activate_trigger_with(project_path, trigger_id, |_| Ok(()))
    }

    /// The explicit installation action that arms a trigger. Reconciles the
    /// project first, then validates the readable definition with its
    /// private bindings, and `validate` (effective profiles and targets).
    /// Success records the definition's fingerprint; failure leaves the
    /// trigger inactive with the reason in `state.activation_error`. A
    /// webhook trigger without credentials gets new ones, returned once.
    pub fn activate_trigger_with(
        &self,
        project_path: &str,
        trigger_id: &str,
        validate: impl FnOnce(&TriggerDefinition) -> TriggerResult<()>,
    ) -> TriggerResult<SerializedTrigger> {
        self.repositories.reconcile_project(project_path)?;
        let definition = self
            .repositories
            .definitions(project_path)?
            .get(trigger_id)?
            .ok_or(TriggerError::UnknownTrigger)?;
        let states = self.repositories.runtime_state(project_path)?;
        let bound = states.load(trigger_id)?.bind(definition.clone());
        let validation = if bound.action.mode == "mission" && bound.action.mission_id.is_none() {
            Err(TriggerError::Validation(
                "Choose the mission this trigger delivers to before activating it.".into(),
            ))
        } else {
            validate(&bound)
        };
        let fingerprint = spark_storage::definition_fingerprint(&definition);
        let mut webhook_secret = None;
        let state = states.update(trigger_id, |state| match &validation {
            Ok(()) => {
                state.enabled = true;
                state.fingerprint = Some(fingerprint);
                state.activation_error = None;
                if definition.source_type == SOURCE_WEBHOOK && state.secret_hash.is_none() {
                    webhook_secret = Some(set_webhook_credentials(state));
                }
                state::refresh_next_run_at(&definition, state);
            }
            Err(error) => {
                state.enabled = false;
                state.activation_error = Some(error.to_string());
            }
        })?;
        Ok(serialize_trigger(definition, state, webhook_secret))
    }

    /// Clears the activation first, so no new dispatch starts even if
    /// removing the file then fails; a definition left on disk stays
    /// inactive.
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
        self.repositories
            .runtime_state(project_path)?
            .delete(trigger_id)?;
        repository.delete(trigger_id, expected_revision, message)?;
        Ok(TriggerDeleteResponse {
            status: "deleted".to_string(),
            id: trigger_id.to_string(),
            project_path: definition.project_path,
        })
    }

    /// Reconciles every registered project, returning the definitions that
    /// cannot run and the projects that could not be read.
    pub fn reconcile(&self) -> TriggerResult<Vec<TriggerIssue>> {
        Ok(self.repositories.reconcile()?.issues)
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

    /// The trigger with its stored state; a read writes nothing.
    fn serialize_current(&self, definition: TriggerDefinition) -> TriggerResult<SerializedTrigger> {
        let mut state = self
            .repositories
            .runtime_state(&definition.project_path)?
            .load(&definition.id)?;
        if state.is_active_for(&definition) {
            state::refresh_next_run_at(&definition, &mut state);
        } else {
            state.next_run_at = None;
        }
        Ok(serialize_trigger(definition, state, None))
    }
}

/// Gives a webhook trigger a new secret, keeping its key or making one if it
/// has none. Returns the secret.
fn set_webhook_credentials(state: &mut TriggerState) -> String {
    let (key, secret, secret_hash) = generate_webhook_credentials(state.webhook_key.as_deref());
    state.webhook_key = Some(key);
    state.secret_hash = Some(secret_hash);
    secret
}

/// The active webhook trigger a request's key and secret authenticate,
/// with its private bindings, rechecked against its definition as it
/// reads now.
pub(crate) fn authenticate_webhook_request(
    repositories: &TriggerRepositories,
    request: &WebhookHandleRequest,
) -> TriggerResult<TriggerDefinition> {
    let webhook_key = request.webhook_key.trim();
    if webhook_key.is_empty() {
        return Err(TriggerError::UnknownWebhookKey);
    }
    let (project_path, trigger_id) = repositories
        .find_webhook_key(webhook_key)?
        .ok_or(TriggerError::UnknownWebhookKey)?;
    repositories.reconcile_project(&project_path)?;
    let definition = repositories
        .definitions(&project_path)?
        .get(&trigger_id)?
        .ok_or(TriggerError::UnknownWebhookKey)?;
    let state = repositories
        .runtime_state(&project_path)?
        .get(&trigger_id)?
        .ok_or(TriggerError::UnknownWebhookKey)?;
    if !state.is_active_for(&definition) {
        return Err(TriggerError::Validation(
            "Webhook trigger is disabled.".to_string(),
        ));
    }
    let secret_hash = state.secret_hash.clone().unwrap_or_default();
    if !verify_webhook_secret(&secret_hash, request.webhook_secret.trim()) {
        return Err(TriggerError::InvalidWebhookSecret);
    }
    Ok(state.bind(definition))
}

/// The API's trigger: `enabled` is whether the activation matches the
/// definition, the mission binding and webhook key come from private state;
/// the secret hash never leaves it.
pub fn serialize_trigger(
    definition: TriggerDefinition,
    mut state: TriggerState,
    webhook_secret: Option<String>,
) -> SerializedTrigger {
    let enabled = state.is_active_for(&definition);
    let definition = state.bind(definition);
    let mut source = definition.source.clone();
    state.secret_hash = None;
    state.fingerprint = None;
    state.flow_event_deliveries.clear();
    if let Some(key) = state.webhook_key.take() {
        source.insert("webhook_key".to_string(), Value::String(key));
    }
    SerializedTrigger {
        revision: definition.revision,
        id: definition.id,
        project_path: definition.project_path,
        name: definition.name,
        enabled,
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
