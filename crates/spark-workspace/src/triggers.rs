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

#[derive(Debug, Clone)]
pub struct WorkspaceTriggerService {
    settings: SparkSettings,
}

impl WorkspaceTriggerService {
    pub fn new(settings: SparkSettings) -> Self {
        Self { settings }
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
        let flow_name = required_flow_name(&request.action)?;
        self.ensure_flow_exists(flow_name)?;
        TriggerService::new(self.settings.clone())
            .create_trigger(request)
            .map_err(Into::into)
    }

    pub fn update_trigger(
        &self,
        trigger_id: &str,
        request: TriggerUpdateRequest,
    ) -> WorkspaceResult<SerializedTrigger> {
        if let Some(flow_name) = optional_flow_name(request.action.as_ref()) {
            self.ensure_flow_exists(flow_name)?;
        }
        TriggerService::new(self.settings.clone())
            .update_trigger(trigger_id, request)
            .map_err(Into::into)
    }

    pub fn delete_trigger(&self, trigger_id: &str) -> WorkspaceResult<TriggerDeleteResponse> {
        TriggerService::new(self.settings.clone())
            .delete_trigger(trigger_id)
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
        let Some(bundle) = attractor_runtime::RunStore::for_settings(&self.settings)
            .read_run_bundle(run_id)
            .map_err(|error| WorkspaceError::Internal(error.to_string()))?
        else {
            return Ok(Vec::new());
        };
        let Some(record) = bundle.record else {
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

    fn ensure_flow_exists(&self, flow_name: &str) -> WorkspaceResult<()> {
        WorkspaceFlowService::new(self.settings.clone()).ensure_flow_exists(flow_name)
    }

    fn source_runtime(&self) -> TriggerSourceRuntime {
        TriggerSourceRuntime::with_sink(
            TriggerRepositories::from_settings(&self.settings),
            WorkspaceTriggerActivationSink {
                settings: self.settings.clone(),
            },
        )
    }
}

#[derive(Debug, Clone)]
struct WorkspaceTriggerActivationSink {
    settings: SparkSettings,
}

impl TriggerActivationSink for WorkspaceTriggerActivationSink {
    fn activate(
        &self,
        request: TriggerActivationRequest,
    ) -> spark_triggers::TriggerResult<TriggerActivationSinkOutcome> {
        let run_id = WorkspaceConversationService::new(self.settings.clone())
            .launch_trigger_flow(request)
            .map_err(|error| spark_triggers::TriggerError::Validation(error.detail()))?;
        Ok(TriggerActivationSinkOutcome {
            run_id: Some(run_id),
            message: Some("Trigger fired successfully.".to_string()),
        })
    }
}

fn required_flow_name(action: &Map<String, Value>) -> WorkspaceResult<&str> {
    action
        .get("flow_name")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            WorkspaceError::Validation("Trigger action requires a flow_name.".to_string())
        })
}

fn optional_flow_name(action: Option<&Map<String, Value>>) -> Option<&str> {
    action?
        .get("flow_name")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
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
