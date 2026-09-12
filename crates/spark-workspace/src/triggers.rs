use serde::Deserialize;
use serde_json::{Map, Value};
use spark_common::project::normalize_project_path;
use spark_common::settings::SparkSettings;
use spark_storage::TriggerRepositories;
use spark_triggers::{
    SerializedTrigger, TriggerActivationOutcome, TriggerActivationRequest, TriggerActivationSink,
    TriggerActivationSinkOutcome, TriggerCreateRequest, TriggerDeleteResponse, TriggerService,
    TriggerSourceRuntime, TriggerUpdateRequest, WebhookDispatchOutcome, WebhookHandleRequest,
    WebhookHandleResponse, TERMINAL_PIPELINE_STATUSES,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
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
        if action_mode(&request.action) != "workspace_draft" {
            let flow_name = required_flow_name(&request.action)?;
            self.ensure_flow_exists(flow_name)?;
        }
        TriggerService::new(self.settings.clone())
            .create_trigger(request)
            .map_err(Into::into)
    }

    pub fn update_trigger(
        &self,
        trigger_id: &str,
        request: TriggerUpdateRequest,
    ) -> WorkspaceResult<SerializedTrigger> {
        if request
            .action
            .as_ref()
            .is_none_or(|action| action_mode(action) != "workspace_draft")
        {
            if let Some(flow_name) = optional_flow_name(request.action.as_ref()) {
                self.ensure_flow_exists(flow_name)?;
            }
        }
        TriggerService::new(self.settings.clone())
            .update_trigger(trigger_id, request)
            .map_err(Into::into)
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
        if request.action.mode == "workspace_draft" {
            return self.activate_workspace_draft(request);
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

pub(crate) fn is_math_chain_trigger(request: &TriggerActivationRequest) -> bool {
    request.action.mode == "workspace_draft"
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NextSessionDraft {
    #[serde(default)]
    status: Option<String>,
    #[serde(alias = "flow")]
    flow_name: String,
    inputs: BTreeMap<String, Value>,
    #[serde(default, rename = "rationale")]
    _rationale: Option<String>,
}

impl WorkspaceTriggerActivationSink {
    fn activate_workspace_draft(
        &self,
        mut request: TriggerActivationRequest,
    ) -> spark_triggers::TriggerResult<TriggerActivationSinkOutcome> {
        let configured_project =
            match request
                .action
                .project_path
                .as_deref()
                .map(normalize_project_path)
                .transpose()
            {
                Ok(Some(Some(path))) => path,
                Ok(_) => return Ok(chain_noop(
                    "Next-session launch skipped: chain trigger has no configured project path.",
                )),
                Err(error) => {
                    return Ok(chain_noop(format!(
                    "Next-session launch skipped: chain trigger project path is invalid: {error}"
                )))
                }
            };
        // Only flow events carry a source project; the cross-project guard is
        // meaningless for schedule/poll activations, which are scoped by the
        // action's own configured project path.
        if request.source_type == "flow_event" {
            let Some(project_path) = request
                .source_payload
                .get("project_path")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                return Ok(chain_noop(
                    "Next-session launch skipped: terminal event has no project path.",
                ));
            };
            let event_project = match normalize_project_path(project_path) {
                Ok(Some(path)) => path,
                Ok(None) => unreachable!("the event project path was checked above"),
                Err(error) => {
                    return Ok(chain_noop(format!(
                    "Next-session launch skipped: terminal event project path is invalid: {error}"
                )))
                }
            };
            if configured_project != event_project {
                return Ok(chain_noop(
                    "Next-session launch skipped: terminal event belongs to another project.",
                ));
            }
        }
        let project_path = configured_project.to_string_lossy().into_owned();
        let active = Path::new(&project_path).join(".mathlab/next-session.json");
        let launching = Path::new(&project_path).join(".mathlab/next-session.launching.json");
        let launched = Path::new(&project_path).join(".mathlab/next-session.launched.json");
        let draft = match read_and_validate_next_session(
            &self.settings,
            &active,
            &request.action.flow_allowlist,
        ) {
            Ok(Some(draft)) => draft,
            Ok(None) => {
                return Ok(chain_noop(
                    "Next-session launch skipped: no next-session.json was available.",
                ))
            }
            Err(message) => return Ok(chain_noop(message)),
        };
        match fs::rename(&active, &launching) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(chain_noop(
                    "Next-session launch skipped: another consumer claimed next-session.json.",
                ))
            }
            Err(error) => {
                return Ok(chain_noop(format!(
                    "Next-session launch skipped: unable to claim next-session.json: {error}"
                )))
            }
        }

        request.action.flow_name = draft.flow_name;
        request.action.project_path = Some(project_path);
        request.action.static_context.extend(draft.inputs);
        let mut service = WorkspaceConversationService::new(self.settings.clone());
        if let Some(observer) = &self.run_event_observer {
            service = service.with_run_event_observer(observer.clone());
        }
        match service.launch_trigger_flow(request) {
            Ok(run_id) => Ok(TriggerActivationSinkOutcome {
                run_id: Some(run_id),
                message: Some("Next session launched successfully.".to_string()),
                no_op: false,
            }),
            Err(error) => {
                let launch_error = error.detail();
                let claim = if launching.exists() {
                    &launching
                } else {
                    &launched
                };
                match fs::rename(claim, &active) {
                    Ok(()) => Err(spark_triggers::TriggerError::Validation(launch_error)),
                    Err(restore_error) => Err(spark_triggers::TriggerError::Validation(format!(
                        "{launch_error}; next-session claim remains at {} because restoration failed: {restore_error}",
                        claim.display()
                    ))),
                }
            }
        }
    }
}

fn chain_noop(message: impl Into<String>) -> TriggerActivationSinkOutcome {
    TriggerActivationSinkOutcome {
        run_id: None,
        message: Some(message.into()),
        no_op: true,
    }
}

fn read_and_validate_next_session(
    settings: &SparkSettings,
    path: &Path,
    allowlist: &[String],
) -> Result<Option<NextSessionDraft>, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "Next-session launch skipped: unable to read draft: {error}"
            ))
        }
    };
    let draft: NextSessionDraft = serde_json::from_str(&text)
        .map_err(|error| format!("Next-session launch skipped: invalid draft JSON: {error}"))?;
    if draft
        .status
        .as_deref()
        .is_some_and(|status| status != "continue")
    {
        return Err("Next-session launch skipped: draft did not request continuation.".to_string());
    }
    if !flow_allowed(&draft.flow_name, allowlist) {
        return Err(format!(
            "Next-session launch skipped: flow {:?} is not allowlisted.",
            draft.flow_name
        ));
    }
    let source = attractor_dsl::read_named_flow_source(&settings.flows_dir, &draft.flow_name)
        .map_err(|error| format!("Next-session launch skipped: {}", error.detail()))?;
    validate_next_session_inputs(&draft.inputs, &source.flow.inputs)?;
    Ok(Some(draft))
}

fn validate_next_session_inputs(
    supplied: &BTreeMap<String, Value>,
    declared: &[attractor_core::FlowInput],
) -> Result<(), String> {
    for input in declared {
        if !matches!(
            input.r#type.as_str(),
            "string" | "string[]" | "boolean" | "number" | "json"
        ) {
            return Err(format!(
                "Next-session launch skipped: input {:?} declares unsupported type {:?}.",
                input.key, input.r#type
            ));
        }
    }
    let declared_keys = declared
        .iter()
        .map(|input| input.key.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(key) = supplied
        .keys()
        .find(|key| !declared_keys.contains(key.as_str()))
    {
        return Err(format!(
            "Next-session launch skipped: input {key:?} is not declared by the selected flow."
        ));
    }
    for input in declared {
        let Some(value) = supplied.get(&input.key) else {
            if input.required && input.default.is_none() {
                return Err(format!(
                    "Next-session launch skipped: required input {:?} is missing.",
                    input.key
                ));
            }
            continue;
        };
        if !json_value_matches_type(value, &input.r#type) {
            return Err(format!(
                "Next-session launch skipped: input {:?} must be {}, not {}.",
                input.key,
                input.r#type,
                json_type_name(value)
            ));
        }
    }
    Ok(())
}

fn json_value_matches_type(value: &Value, expected: &str) -> bool {
    match expected {
        "string" => value.is_string(),
        "string[]" => value
            .as_array()
            .is_some_and(|values| values.iter().all(Value::is_string)),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "json" => true,
        _ => false,
    }
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod next_session_tests {
    use super::*;
    use attractor_core::FlowInput;
    use serde_json::json;

    fn input(key: &str, kind: &str, required: bool, default: Option<Value>) -> FlowInput {
        FlowInput {
            key: key.to_string(),
            r#type: kind.to_string(),
            required,
            default,
            ..FlowInput::default()
        }
    }

    #[test]
    fn next_session_inputs_require_declared_typed_values_without_requiring_optionals() {
        let declared = vec![
            input("context.request.problem", "string", true, None),
            input("context.request.tags", "string[]", false, None),
            input("context.request.rounds", "number", false, None),
            input("context.request.enabled", "boolean", false, None),
            input("context.request.payload", "json", false, None),
            input("context.request.focus", "string", true, Some(json!("all"))),
        ];
        assert!(validate_next_session_inputs(
            &BTreeMap::from([("context.request.problem".to_string(), json!("P"))]),
            &declared,
        )
        .is_ok());
        assert!(validate_next_session_inputs(&BTreeMap::new(), &declared)
            .unwrap_err()
            .contains("required input"));
        assert!(validate_next_session_inputs(
            &BTreeMap::from([
                ("context.request.problem".to_string(), json!("P")),
                ("context.request.tags".to_string(), json!(["one", 2])),
            ]),
            &declared,
        )
        .unwrap_err()
        .contains("must be string[]"));
        assert!(validate_next_session_inputs(
            &BTreeMap::from([
                ("context.request.problem".to_string(), json!("P")),
                ("context.request.unknown".to_string(), json!(true)),
            ]),
            &declared,
        )
        .unwrap_err()
        .contains("not declared"));
        assert!(validate_next_session_inputs(
            &BTreeMap::from([
                ("context.request.problem".to_string(), json!("P")),
                ("context.request.tags".to_string(), json!(["one", "two"])),
                ("context.request.rounds".to_string(), json!(1.5)),
                ("context.request.enabled".to_string(), json!(true)),
                ("context.request.payload".to_string(), json!(null)),
            ]),
            &declared,
        )
        .is_ok());
        assert!(validate_next_session_inputs(
            &BTreeMap::new(),
            &[input("context.request.legacy", "integer", false, None)],
        )
        .unwrap_err()
        .contains("unsupported type"));
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

fn action_mode(action: &Map<String, Value>) -> &str {
    action
        .get("mode")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("static")
}

fn flow_allowed(flow_name: &str, allowlist: &[String]) -> bool {
    allowlist.iter().any(|pattern| {
        let pattern = pattern.trim();
        pattern
            .strip_suffix('*')
            .map_or(flow_name == pattern, |prefix| flow_name.starts_with(prefix))
    })
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
