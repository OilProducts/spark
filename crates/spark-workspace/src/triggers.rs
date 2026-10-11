use serde_json::{Map, Value};
use spark_common::settings::SparkSettings;
use spark_storage::TriggerRepositories;
use spark_triggers::{
    SerializedTrigger, TriggerActivationOutcome, TriggerActivationRequest, TriggerActivationSink,
    TriggerActivationSinkOutcome, TriggerCreateRequest, TriggerDeleteResponse, TriggerService,
    TriggerSourceRuntime, TriggerUpdateRequest, WebhookDispatchOutcome, WebhookHandleRequest,
    WebhookHandleResponse, TERMINAL_PIPELINE_STATUSES,
};
use time::OffsetDateTime;

use crate::conversations::WorkspaceConversationService;
use crate::errors::{WorkspaceError, WorkspaceResult};
use crate::flows::WorkspaceFlowService;

#[derive(Clone)]
pub struct WorkspaceTriggerService {
    settings: SparkSettings,
    run_event_observer: Option<attractor_runtime::RunEventObserver>,
}

impl WorkspaceTriggerService {
    pub fn new(settings: SparkSettings) -> Self {
        Self {
            settings,
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

    pub fn list_triggers(&self) -> WorkspaceResult<Vec<SerializedTrigger>> {
        TriggerService::new(self.settings.clone())
            .list_triggers()
            .map_err(Into::into)
    }

    pub fn get_trigger(&self, trigger_id: &str) -> WorkspaceResult<SerializedTrigger> {
        TriggerService::new(self.settings.clone())
            .get_trigger(trigger_id)?
            .ok_or_else(|| WorkspaceError::NotFound("Unknown trigger.".to_string()))
    }

    pub fn create_trigger(
        &self,
        request: TriggerCreateRequest,
    ) -> WorkspaceResult<SerializedTrigger> {
        let action = spark_storage::normalize_trigger_action_payload(&request.action)?;
        self.with_validated_action(&action, || {
            TriggerService::new(self.settings.clone())
                .create_trigger(request)
                .map_err(Into::into)
        })
    }

    pub fn update_trigger(
        &self,
        trigger_id: &str,
        request: TriggerUpdateRequest,
    ) -> WorkspaceResult<SerializedTrigger> {
        let existing = TriggerRepositories::from_settings(&self.settings)
            .definitions
            .get(trigger_id)?
            .ok_or_else(|| WorkspaceError::NotFound("Unknown trigger.".into()))?;
        let mut action = serde_json::to_value(&existing.action)
            .unwrap()
            .as_object()
            .unwrap()
            .clone();
        if let Some(update) = &request.action {
            action.extend(update.clone());
        }
        let action = spark_storage::normalize_trigger_action_payload(&action)?;
        if action.mode == "static"
            && request
                .action
                .as_ref()
                .is_none_or(|a| !a.contains_key("flow_name"))
        {
            return TriggerService::new(self.settings.clone())
                .update_trigger(trigger_id, request)
                .map_err(Into::into);
        }
        self.with_validated_action(&action, || {
            TriggerService::new(self.settings.clone())
                .update_trigger(trigger_id, request)
                .map_err(Into::into)
        })
    }

    pub fn delete_trigger(
        &self,
        trigger_id: &str,
        expected_revision: &str,
    ) -> WorkspaceResult<TriggerDeleteResponse> {
        TriggerService::new(self.settings.clone())
            .delete_trigger(trigger_id, expected_revision)
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

    pub async fn process_due_trigger_sources(
        &self,
    ) -> WorkspaceResult<Vec<TriggerActivationOutcome>> {
        self.process_due_trigger_sources_at(OffsetDateTime::now_utc())
            .await
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

    fn with_validated_action<T>(
        &self,
        action: &spark_storage::TriggerAction,
        work: impl FnOnce() -> WorkspaceResult<T>,
    ) -> WorkspaceResult<T> {
        if action.mode == "mission" {
            // Serialize target validation and persistence with mission closure.
            let scope = spark_storage::ProjectRegistry::new(self.settings.data_dir.clone())
                .ensure_project_paths(action.project_path.as_deref().unwrap())?;
            spark_storage::workspace_missions::MissionRepository::new(&scope.root)
                .locked::<_, WorkspaceError>(|_| {
                    self.validate_action(action)?;
                    work()
                })
        } else {
            self.validate_action(action)?;
            work()
        }
    }

    fn validate_action(&self, action: &spark_storage::TriggerAction) -> WorkspaceResult<()> {
        if action.mode == "mission" {
            let mission = crate::missions::WorkspaceMissionService::new(self.settings.clone())
                .get(
                    action.project_path.as_deref().unwrap(),
                    action.mission_id.as_deref().unwrap(),
                )?;
            if mission.closed.is_some() {
                return Err(WorkspaceError::Validation("Mission is closed".into()));
            }
        } else if action.mode == "static" {
            self.ensure_flow_exists(&action.flow_name)?;
        }
        Ok(())
    }

    fn ensure_flow_exists(&self, flow_name: &str) -> WorkspaceResult<()> {
        WorkspaceFlowService::new(self.settings.clone()).ensure_flow_exists(flow_name)
    }

    fn source_runtime(&self) -> TriggerSourceRuntime {
        TriggerSourceRuntime::with_sink(
            TriggerRepositories::from_settings(&self.settings),
            WorkspaceTriggerActivationSink {
                settings: self.settings.clone(),
                run_event_observer: self.run_event_observer.clone(),
            },
        )
    }
}

#[derive(Clone)]
struct WorkspaceTriggerActivationSink {
    settings: SparkSettings,
    run_event_observer: Option<attractor_runtime::RunEventObserver>,
}

impl TriggerActivationSink for WorkspaceTriggerActivationSink {
    fn activate(
        &self,
        request: TriggerActivationRequest,
    ) -> spark_triggers::TriggerResult<TriggerActivationSinkOutcome> {
        if request.action.mode == "mission" {
            return crate::missions::WorkspaceMissionService::new(self.settings.clone())
                .deliver_trigger(&request)
                .map_err(|error| spark_triggers::TriggerError::Validation(error.detail()));
        }
        let mut service = WorkspaceConversationService::new(self.settings.clone());
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
