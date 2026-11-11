use serde_json::{Map, Value};
use spark_common::settings::SparkSettings;
use spark_storage::workspace_records::RecordRepository;
use spark_storage::{ConversationStore, TriggerRepositories};
use spark_triggers::{
    SerializedTrigger, TriggerActivationOutcome, TriggerActivationRequest, TriggerActivationSink,
    TriggerActivationSinkOutcome, TriggerCreateRequest, TriggerDefinition, TriggerDeleteResponse,
    TriggerError, TriggerService, TriggerSourceRuntime, TriggerUpdateRequest,
    WebhookDispatchOutcome, WebhookHandleRequest, WebhookHandleResponse,
    TERMINAL_PIPELINE_STATUSES,
};
use std::sync::Arc;
use time::OffsetDateTime;

use crate::conversations::WorkspaceConversationService;
use crate::errors::{WorkspaceError, WorkspaceResult};
use crate::flows::WorkspaceFlowService;

#[derive(Clone)]
pub struct WorkspaceTriggerService {
    settings: SparkSettings,
    store: Arc<ConversationStore>,
    run_event_observer: Option<attractor_runtime::RunEventObserver>,
}

impl WorkspaceTriggerService {
    pub fn new(settings: SparkSettings, store: Arc<ConversationStore>) -> Self {
        Self {
            settings,
            store,
            run_event_observer: None,
        }
    }

    pub fn with_run_event_observer(
        mut self,
        observer: attractor_runtime::RunEventObserver,
    ) -> Self {
        self.run_event_observer = Some(observer);
        self
    }

    /// The triggers of `project_path`, or of every registered project.
    pub fn list_triggers(
        &self,
        project_path: Option<&str>,
    ) -> WorkspaceResult<Vec<SerializedTrigger>> {
        TriggerService::new(self.settings.clone())
            .list_triggers(project_path)
            .map_err(Into::into)
    }

    pub fn get_trigger(
        &self,
        project_path: &str,
        trigger_id: &str,
    ) -> WorkspaceResult<SerializedTrigger> {
        TriggerService::new(self.settings.clone())
            .get_trigger(project_path, trigger_id)?
            .ok_or_else(|| WorkspaceError::NotFound("Unknown trigger.".to_string()))
    }

    /// Creates the trigger in a registered project's definitions. The
    /// definition is saved first; `enabled` then activates it, and a failed
    /// activation leaves it inactive with `state.activation_error`.
    pub fn create_trigger(
        &self,
        mut request: TriggerCreateRequest,
    ) -> WorkspaceResult<SerializedTrigger> {
        if spark_storage::ProjectRegistry::new(self.settings.data_dir.clone())
            .read_project_record(&request.project_path)?
            .is_none()
        {
            return Err(WorkspaceError::Validation(
                "Triggers are created in a registered project.".into(),
            ));
        }
        let enabled = std::mem::replace(&mut request.enabled, false);
        let created = TriggerService::new(self.settings.clone()).create_trigger(request)?;
        if !enabled {
            return Ok(created);
        }
        let mut trigger = self.activate_trigger(&created.project_path, &created.id)?;
        trigger.webhook_secret = created.webhook_secret.or(trigger.webhook_secret);
        Ok(trigger)
    }

    /// `enabled: true` revalidates and activates the trigger as updated.
    pub fn update_trigger(
        &self,
        project_path: &str,
        trigger_id: &str,
        mut request: TriggerUpdateRequest,
    ) -> WorkspaceResult<SerializedTrigger> {
        let activate = request.enabled == Some(true);
        if activate {
            request.enabled = None;
        }
        let updated = TriggerService::new(self.settings.clone()).update_trigger(
            project_path,
            trigger_id,
            request,
        )?;
        if !activate {
            return Ok(updated);
        }
        let mut trigger = self.activate_trigger(&updated.project_path, trigger_id)?;
        trigger.webhook_secret = updated.webhook_secret.or(trigger.webhook_secret);
        Ok(trigger)
    }

    /// Explicitly activates a trigger after validating its effective
    /// execution profile and its target in the trigger's project. Held under
    /// the project's mission lock, so a mission closing meanwhile either
    /// sees the activation or rejects it.
    pub fn activate_trigger(
        &self,
        project_path: &str,
        trigger_id: &str,
    ) -> WorkspaceResult<SerializedTrigger> {
        let project = spark_storage::ProjectRegistry::new(self.settings.data_dir.clone())
            .project_paths(project_path)?;
        RecordRepository::missions(&self.settings.data_dir, &project.project_id).locked(|_| {
            TriggerService::new(self.settings.clone())
                .activate_trigger_with(&project.project_path, trigger_id, |definition| {
                    self.validate_activation(definition)
                        .map_err(|error| TriggerError::Validation(error.detail()))
                })
                .map_err(Into::into)
        })
    }

    pub fn delete_trigger(
        &self,
        project_path: &str,
        trigger_id: &str,
        expected_revision: &str,
        message: Option<&str>,
    ) -> WorkspaceResult<TriggerDeleteResponse> {
        TriggerService::new(self.settings.clone())
            .delete_trigger(project_path, trigger_id, expected_revision, message)
            .map_err(Into::into)
    }

    pub fn handle_webhook(
        &self,
        request: WebhookHandleRequest,
    ) -> WorkspaceResult<WebhookHandleResponse> {
        self.dispatch_webhook(request)
            .map(|outcome| outcome.response)
    }

    pub fn dispatch_webhook(
        &self,
        request: WebhookHandleRequest,
    ) -> WorkspaceResult<WebhookDispatchOutcome> {
        self.source_runtime()
            .process_webhook(request)
            .map_err(Into::into)
    }

    pub fn refresh_trigger_runtime_state(&self) -> WorkspaceResult<Vec<SerializedTrigger>> {
        self.source_runtime()
            .reload_refresh_state()
            .map_err(Into::into)
    }

    /// One background tick: the activations it ran and what its
    /// reconciliation could not read.
    pub async fn process_due_trigger_sources(
        &self,
    ) -> WorkspaceResult<(
        Vec<TriggerActivationOutcome>,
        Vec<spark_storage::TriggerIssue>,
    )> {
        self.source_runtime()
            .process_due_sources_reporting(OffsetDateTime::now_utc())
            .await
            .map_err(Into::into)
    }

    pub async fn process_due_trigger_sources_at(
        &self,
        now: OffsetDateTime,
    ) -> WorkspaceResult<Vec<TriggerActivationOutcome>> {
        self.source_runtime()
            .process_due_sources(now)
            .await
            .map_err(Into::into)
    }

    pub fn emit_flow_event(
        &self,
        payload: Map<String, Value>,
    ) -> WorkspaceResult<Vec<TriggerActivationOutcome>> {
        self.source_runtime()
            .emit_flow_event(payload)
            .map_err(Into::into)
    }

    pub fn emit_terminal_flow_event_for_run(
        &self,
        run_id: &str,
    ) -> WorkspaceResult<Vec<TriggerActivationOutcome>> {
        // Record-only read: runs on every publish cycle, and a full bundle
        // read reparses the entire event log.
        let store = attractor_runtime::RunStore::for_settings(&self.settings);
        let Some(paths) = store
            .find_run_root(run_id)
            .map_err(|error| WorkspaceError::Internal(error.to_string()))?
        else {
            return Ok(Vec::new());
        };
        let Some(record) = store
            .read_run_record(&paths)
            .map_err(|error| WorkspaceError::Internal(error.to_string()))?
        else {
            return Ok(Vec::new());
        };
        let status = record.status.trim().to_ascii_lowercase();
        if !TERMINAL_PIPELINE_STATUSES.contains(&status.as_str()) {
            return Ok(Vec::new());
        }
        let project_path = record
            .project_path
            .trim()
            .to_string()
            .if_empty_then(record.working_directory.trim().to_string());
        self.emit_flow_event(Map::from_iter([
            ("run_id".to_string(), Value::String(record.run_id)),
            ("flow_name".to_string(), Value::String(record.flow_name)),
            (
                "project_path".to_string(),
                if project_path.is_empty() {
                    Value::Null
                } else {
                    Value::String(project_path)
                },
            ),
            ("status".to_string(), Value::String(status)),
        ]))
    }

    /// The checks an activation runs: the action's target in the trigger's
    /// project, and the execution profile a flow launch would use.
    fn validate_activation(&self, definition: &TriggerDefinition) -> WorkspaceResult<()> {
        let project_path = definition.project_path.as_str();
        let action = &definition.action;
        if action.mode == "mission" {
            let mission = crate::missions::WorkspaceMissionService::new(
                self.settings.clone(),
                self.store.clone(),
            )
            .get(
                project_path,
                action.mission_id.as_deref().unwrap_or_default(),
            )?;
            if mission.closed.is_some() {
                return Err(WorkspaceError::Validation("Mission is closed".into()));
            }
        } else if action.mode == "role" {
            crate::roles::role_file(
                &self.settings,
                project_path,
                action.role.as_deref().unwrap_or_default(),
            )?;
        } else {
            WorkspaceFlowService::new(self.settings.clone())
                .ensure_flow_exists(&action.flow_name, Some(project_path))?;
            let project_default =
                spark_storage::ProjectRegistry::new(self.settings.data_dir.clone())
                    .project_settings(project_path)?
                    .execution_profile_id;
            attractor_execution::resolve_execution_profile_by_id(
                &self.settings,
                action.execution_profile_id.as_deref(),
                project_default.as_deref(),
                None,
            )
            .map_err(|error| WorkspaceError::Validation(error.to_string()))?;
        }
        Ok(())
    }

    /// Reconciles every registered project's triggers with their activation
    /// rows, returning what could not be read.
    pub fn reconcile_triggers(&self) -> WorkspaceResult<Vec<spark_storage::TriggerIssue>> {
        TriggerService::new(self.settings.clone())
            .reconcile()
            .map_err(Into::into)
    }

    /// The definitions that cannot run; reads only.
    pub fn trigger_issues(
        &self,
        project_path: Option<&str>,
    ) -> WorkspaceResult<Vec<spark_storage::TriggerIssue>> {
        TriggerService::new(self.settings.clone())
            .trigger_issues(project_path)
            .map_err(Into::into)
    }

    fn source_runtime(&self) -> TriggerSourceRuntime {
        TriggerSourceRuntime::with_sink(
            TriggerRepositories::from_settings(&self.settings),
            WorkspaceTriggerActivationSink {
                settings: self.settings.clone(),
                store: self.store.clone(),
                run_event_observer: self.run_event_observer.clone(),
            },
        )
    }
}

#[derive(Clone)]
struct WorkspaceTriggerActivationSink {
    settings: SparkSettings,
    store: Arc<ConversationStore>,
    run_event_observer: Option<attractor_runtime::RunEventObserver>,
}

impl TriggerActivationSink for WorkspaceTriggerActivationSink {
    fn activate(
        &self,
        request: TriggerActivationRequest,
    ) -> spark_triggers::TriggerResult<TriggerActivationSinkOutcome> {
        if request.action.mode == "mission" {
            return crate::missions::WorkspaceMissionService::new(
                self.settings.clone(),
                self.store.clone(),
            )
            .deliver_trigger(&request)
            .map_err(|error| spark_triggers::TriggerError::Validation(error.detail()));
        }
        if request.action.mode == "role" {
            return crate::roles::WorkspaceRoleService::new(
                self.settings.clone(),
                self.store.clone(),
            )
            .deliver_trigger(&request)
            .map_err(|error| spark_triggers::TriggerError::Validation(error.detail()));
        }
        let mut service =
            WorkspaceConversationService::new(self.settings.clone(), self.store.clone());
        if let Some(observer) = &self.run_event_observer {
            service = service.with_run_event_observer(observer.clone());
        }
        let run_id = service
            .launch_trigger_flow(request)
            .map_err(|error| spark_triggers::TriggerError::Validation(error.detail()))?;
        Ok(TriggerActivationSinkOutcome {
            run_id: Some(run_id),
            message: Some("Trigger fired successfully.".to_string()),
            no_op: false,
        })
    }
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
