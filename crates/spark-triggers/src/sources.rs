use std::path::Path;
use std::sync::Arc;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{json, Map, Value};
use spark_common::project::normalize_project_path;
use spark_storage::TriggerRepositories;
use time::{Duration, OffsetDateTime};

use crate::error::{TriggerError, TriggerResult};
use crate::models::{
    SerializedTrigger, TriggerActivationOutcome, TriggerActivationRequest,
    TriggerActivationSinkOutcome, TriggerDefinition, WebhookDispatchOutcome, WebhookHandleRequest,
    WebhookHandleResponse, SOURCE_FLOW_EVENT, SOURCE_POLL, SOURCE_SCHEDULE, SOURCE_WEBHOOK,
    TERMINAL_PIPELINE_STATUSES,
};
use crate::{authenticate_webhook_request, serialize_trigger, state};

pub trait TriggerActivationSink: Send + Sync {
    fn activate(
        &self,
        request: TriggerActivationRequest,
    ) -> TriggerResult<TriggerActivationSinkOutcome>;
}

#[derive(Debug, Clone, Default)]
pub struct AcceptAllTriggerActivationSink;

impl TriggerActivationSink for AcceptAllTriggerActivationSink {
    fn activate(
        &self,
        _request: TriggerActivationRequest,
    ) -> TriggerResult<TriggerActivationSinkOutcome> {
        Ok(TriggerActivationSinkOutcome {
            run_id: None,
            message: Some("Trigger fired successfully.".to_string()),
            no_op: false,
        })
    }
}

#[derive(Clone)]
pub struct TriggerSourceRuntime {
    repositories: TriggerRepositories,
    sink: Arc<dyn TriggerActivationSink>,
    poll_client: reqwest::Client,
}

impl TriggerSourceRuntime {
    pub fn new(repositories: TriggerRepositories) -> Self {
        Self::with_sink(repositories, AcceptAllTriggerActivationSink)
    }

    pub fn with_sink(
        repositories: TriggerRepositories,
        sink: impl TriggerActivationSink + 'static,
    ) -> Self {
        Self {
            repositories,
            sink: Arc::new(sink),
            poll_client: reqwest::Client::new(),
        }
    }

    pub fn with_sink_arc(
        repositories: TriggerRepositories,
        sink: Arc<dyn TriggerActivationSink>,
    ) -> Self {
        Self {
            repositories,
            sink,
            poll_client: reqwest::Client::new(),
        }
    }

    pub fn reload_refresh_state(&self) -> TriggerResult<Vec<SerializedTrigger>> {
        self.repositories
            .list_all()?
            .into_iter()
            .map(|definition| {
                let state = self
                    .repositories
                    .runtime_state(&definition.project_path)?
                    .update(&definition.id, |state| {
                        state::refresh_next_run_at(&definition, state);
                    })?;
                Ok(serialize_trigger(definition, state, None))
            })
            .collect()
    }

    pub async fn process_due_sources(
        &self,
        now: OffsetDateTime,
    ) -> TriggerResult<Vec<TriggerActivationOutcome>> {
        let mut outcomes = Vec::new();
        // ponytail: reads every registered project's org triggers each tick,
        // so hand edits apply without a restart; cache by directory mtime if
        // projects or triggers grow large.
        for definition in self.repositories.list_all()? {
            match definition.source_type.as_str() {
                SOURCE_SCHEDULE => {
                    outcomes.extend(self.process_schedule_source(&definition, now)?);
                }
                SOURCE_POLL => {
                    outcomes.extend(self.process_poll_source(&definition, now).await?);
                }
                _ => {}
            }
        }
        Ok(outcomes)
    }

    pub fn process_schedule_source(
        &self,
        definition: &TriggerDefinition,
        now: OffsetDateTime,
    ) -> TriggerResult<Vec<TriggerActivationOutcome>> {
        if definition.source_type != SOURCE_SCHEDULE {
            return Ok(Vec::new());
        }
        let states = self.repositories.runtime_state(&definition.project_path)?;
        let mut due_at = None;
        states.update(&definition.id, |trigger_state| {
            if trigger_state.enabled {
                due_at = state::schedule_due_at(definition, trigger_state, now);
                trigger_state.next_run_at =
                    state::compute_next_run_at_at(definition, trigger_state, now);
            }
        })?;
        let Some(due_at) = due_at else {
            return Ok(Vec::new());
        };
        self.execute_activation(
            definition,
            json!({ "scheduled_at": state::datetime_to_iso(due_at) }),
            now,
            None,
        )
        .map(|outcome| vec![outcome])
    }

    pub async fn process_poll_source(
        &self,
        definition: &TriggerDefinition,
        now: OffsetDateTime,
    ) -> TriggerResult<Vec<TriggerActivationOutcome>> {
        if definition.source_type != SOURCE_POLL {
            return Ok(Vec::new());
        }
        let interval_seconds = definition
            .source
            .get("interval_seconds")
            .and_then(Value::as_i64)
            .ok_or_else(|| TriggerError::Validation("Poll interval is missing.".to_string()))?;
        let states = self.repositories.runtime_state(&definition.project_path)?;
        let mut due = false;
        states.update(&definition.id, |trigger_state| {
            due = trigger_state.enabled
                && !state::parse_iso_datetime(trigger_state.next_run_at.as_deref())
                    .is_some_and(|next_run_at| now < next_run_at);
            if due {
                trigger_state.next_run_at = Some(state::datetime_to_iso(
                    now + Duration::seconds(interval_seconds),
                ));
            }
        })?;
        if !due {
            return Ok(Vec::new());
        }

        let payload = match self.fetch_poll_payload(definition).await {
            Ok(payload) => payload,
            Err(message) => {
                return self
                    .record_source_failure(
                        definition,
                        now,
                        format!("Polling failed: {message}"),
                        Value::Null,
                    )
                    .map(|outcome| vec![outcome])
            }
        };
        let items_path = string_source_field(definition, "items_path");
        let item_id_path = string_source_field(definition, "item_id_path");
        let Some(Value::Array(items)) = extract_json_path(&payload, &items_path) else {
            return self
                .record_source_failure(
                    definition,
                    now,
                    "Polling failed: Poll source items_path did not resolve to a JSON array.",
                    Value::Null,
                )
                .map(|outcome| vec![outcome]);
        };

        let mut outcomes = Vec::new();
        for item in items {
            let Some(item_id) = extract_json_path(item, &item_id_path) else {
                continue;
            };
            if item_id.is_null() {
                continue;
            }
            outcomes.push(self.execute_activation(
                definition,
                json!({ "poll_item": item.clone() }),
                now,
                None,
            )?);
        }
        Ok(outcomes)
    }

    /// Fires the enabled `flow_event` triggers that match a terminal run, each
    /// only for runs in its own project.
    pub fn emit_flow_event(
        &self,
        payload: Map<String, Value>,
    ) -> TriggerResult<Vec<TriggerActivationOutcome>> {
        let flow_name = payload
            .get("flow_name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let status = payload
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if flow_name.is_empty() || !TERMINAL_PIPELINE_STATUSES.contains(&status.as_str()) {
            return Ok(Vec::new());
        }
        let run_project = payload
            .get("project_path")
            .and_then(Value::as_str)
            .and_then(|path| normalize_project_path(path).ok().flatten());

        let now = OffsetDateTime::now_utc();
        let mut outcomes = Vec::new();
        for definition in self.repositories.list_all()? {
            if definition.source_type != SOURCE_FLOW_EVENT
                || run_project.as_deref() != Some(Path::new(&definition.project_path))
            {
                continue;
            }
            if !self
                .repositories
                .runtime_state(&definition.project_path)?
                .load(&definition.id)?
                .enabled
            {
                continue;
            }
            let configured_flow_name = definition
                .source
                .get("flow_name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            if !configured_flow_name.is_empty() && configured_flow_name != flow_name {
                continue;
            }
            let statuses = definition
                .source
                .get("statuses")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            if !statuses.is_empty()
                && !statuses.iter().any(|value| {
                    value
                        .as_str()
                        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(&status))
                })
            {
                continue;
            }
            outcomes.push(self.execute_activation(
                &definition,
                Value::Object(payload.clone()),
                now,
                None,
            )?);
        }
        Ok(outcomes)
    }

    pub fn process_webhook(
        &self,
        request: WebhookHandleRequest,
    ) -> TriggerResult<WebhookDispatchOutcome> {
        let definition = authenticate_webhook_request(&self.repositories, &request)?;
        if definition.source_type != SOURCE_WEBHOOK {
            return Err(TriggerError::Validation(
                "Webhook key does not resolve to a webhook trigger.".to_string(),
            ));
        }
        let response = WebhookHandleResponse {
            ok: true,
            trigger_id: definition.id.clone(),
        };
        let activation = self.execute_activation(
            &definition,
            Value::Object(request.payload),
            OffsetDateTime::now_utc(),
            request.request_id.filter(|id| !id.trim().is_empty()),
        )?;
        Ok(WebhookDispatchOutcome {
            response,
            activation,
        })
    }

    fn execute_activation(
        &self,
        definition: &TriggerDefinition,
        source_payload: Value,
        timestamp: OffsetDateTime,
        delivery_id: Option<String>,
    ) -> TriggerResult<TriggerActivationOutcome> {
        let sink_result = self.sink.activate(TriggerActivationRequest {
            trigger_id: definition.id.clone(),
            project_path: definition.project_path.clone(),
            trigger_name: definition.name.clone(),
            source_type: definition.source_type.clone(),
            action: definition.action.clone(),
            source_payload: source_payload.clone(),
            delivery_id,
        });
        let states = self.repositories.runtime_state(&definition.project_path)?;
        match sink_result {
            Ok(sink_outcome) => {
                let message = sink_outcome
                    .message
                    .clone()
                    .unwrap_or_else(|| "Trigger fired successfully.".to_string());
                // Recorded on the latest state, under its lock: a switch-off or
                // secret rotation during the activation survives.
                let trigger_state = states.update(&definition.id, |trigger_state| {
                    if sink_outcome.no_op {
                        state::record_activation_no_op(
                            definition,
                            trigger_state,
                            timestamp,
                            message.clone(),
                        );
                    } else {
                        state::record_activation_success(
                            definition,
                            trigger_state,
                            timestamp,
                            message.clone(),
                            sink_outcome.run_id.clone(),
                        );
                    }
                })?;
                Ok(TriggerActivationOutcome {
                    trigger_id: definition.id.clone(),
                    source_type: definition.source_type.clone(),
                    status: "success".to_string(),
                    message,
                    run_id: sink_outcome.run_id,
                    source_payload,
                    trigger: serialize_trigger(definition.clone(), trigger_state, None),
                })
            }
            Err(error) => {
                self.record_source_failure(definition, timestamp, error.to_string(), source_payload)
            }
        }
    }

    fn record_source_failure(
        &self,
        definition: &TriggerDefinition,
        timestamp: OffsetDateTime,
        message: impl Into<String>,
        source_payload: Value,
    ) -> TriggerResult<TriggerActivationOutcome> {
        let message = message.into();
        let trigger_state = self
            .repositories
            .runtime_state(&definition.project_path)?
            .update(&definition.id, |trigger_state| {
                state::record_activation_failure(definition, trigger_state, timestamp, &message);
            })?;
        Ok(TriggerActivationOutcome {
            trigger_id: definition.id.clone(),
            source_type: definition.source_type.clone(),
            status: "failed".to_string(),
            message,
            run_id: None,
            source_payload,
            trigger: serialize_trigger(definition.clone(), trigger_state, None),
        })
    }

    async fn fetch_poll_payload(&self, definition: &TriggerDefinition) -> Result<Value, String> {
        let url = string_source_field(definition, "url");
        let mut request = self.poll_client.get(url);
        let headers = poll_headers(definition)?;
        if !headers.is_empty() {
            request = request.headers(headers);
        }
        let response = request.send().await.map_err(|error| error.to_string())?;
        let response = response
            .error_for_status()
            .map_err(|error| error.to_string())?;
        response
            .json::<Value>()
            .await
            .map_err(|error| error.to_string())
    }
}

fn poll_headers(definition: &TriggerDefinition) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    let Some(source_headers) = definition.source.get("headers").and_then(Value::as_object) else {
        return Ok(headers);
    };
    for (key, value) in source_headers {
        let name = HeaderName::from_bytes(key.as_bytes()).map_err(|error| error.to_string())?;
        let value = HeaderValue::from_str(value.as_str().unwrap_or_default())
            .map_err(|error| error.to_string())?;
        headers.insert(name, value);
    }
    Ok(headers)
}

fn extract_json_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for part in path
        .split('.')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let object = current.as_object()?;
        current = object.get(part)?;
    }
    Some(current)
}

fn string_source_field(definition: &TriggerDefinition, key: &str) -> String {
    definition
        .source
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}
