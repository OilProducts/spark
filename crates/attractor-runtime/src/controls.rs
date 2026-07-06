use std::collections::BTreeMap;

use attractor_core::{CheckpointState, ContextMap, DotGraph, RunManifest};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::checkpoints::CheckpointWriteOptions;
use crate::context::INTERNAL_PIPELINE_RETRY_RUN_ID_KEY;
use crate::error::RuntimeStorageError;
use crate::events::{
    cancel_requested_event, log_event, pipeline_paused_event, pipeline_retry_started_event,
    runtime_status_event,
};
use crate::records::{
    mark_record_cancel_requested, mark_record_canceled, mark_record_paused,
    mark_record_retry_started,
};
use crate::store::{CreateRunRequest, RunStore};
use unified_llm_adapter::{
    is_display_model_placeholder, RUNTIME_LAUNCH_MODEL_KEY, RUNTIME_LAUNCH_PROFILE_KEY,
    RUNTIME_LAUNCH_PROVIDER_KEY, RUNTIME_LAUNCH_REASONING_EFFORT_KEY,
};

#[derive(Debug, thiserror::Error)]
pub enum RuntimeControlError {
    #[error("Unknown pipeline")]
    UnknownPipeline,
    #[error("Checkpoint unavailable")]
    CheckpointUnavailable,
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Storage(#[from] RuntimeStorageError),
}

pub type ControlResult<T> = std::result::Result<T, RuntimeControlError>;

#[derive(Debug, Clone)]
pub struct RuntimeControls {
    store: RunStore,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeControlStatus {
    pub status: String,
    pub pipeline_id: String,
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryRunPrepared {
    pub status: String,
    pub pipeline_id: String,
    pub run_id: String,
    pub current_node: String,
    pub completed_nodes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContinueRunRequest {
    pub source_run_id: String,
    pub start_node: String,
    pub flow_source_mode: String,
    #[serde(default)]
    pub flow_name: Option<String>,
    #[serde(default)]
    pub new_run_id: Option<String>,
    pub graph: DotGraph,
    #[serde(default)]
    pub graph_source: Option<String>,
    #[serde(default)]
    pub graph_dot: Option<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub llm_provider: Option<String>,
    #[serde(default)]
    pub llm_profile: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContinueRunStarted {
    pub status: String,
    pub pipeline_id: String,
    pub run_id: String,
    pub working_directory: String,
    pub model: String,
    pub provider: String,
    pub llm_provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

impl RuntimeControls {
    pub fn new(store: RunStore) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &RunStore {
        &self.store
    }

    pub fn get_checkpoint(&self, run_id: &str) -> ControlResult<CheckpointState> {
        let bundle = self
            .store
            .read_run_bundle(run_id)?
            .ok_or(RuntimeControlError::UnknownPipeline)?;
        bundle
            .checkpoint
            .ok_or(RuntimeControlError::CheckpointUnavailable)
    }

    pub fn get_context(&self, run_id: &str) -> ControlResult<ContextMap> {
        Ok(self.get_checkpoint(run_id)?.context)
    }

    pub fn continue_from_snapshot(
        &self,
        request: ContinueRunRequest,
    ) -> ControlResult<ContinueRunStarted> {
        let source = self
            .store
            .read_run_bundle(&request.source_run_id)?
            .ok_or(RuntimeControlError::UnknownPipeline)?;
        let source_record = source.record.ok_or(RuntimeControlError::UnknownPipeline)?;
        let source_checkpoint = source
            .checkpoint
            .ok_or(RuntimeControlError::CheckpointUnavailable)?;
        let start_node = request.start_node.trim();
        if start_node.is_empty() {
            return Err(RuntimeControlError::Validation(
                "start_node is required.".to_string(),
            ));
        }
        if !request.graph.nodes.contains_key(start_node) {
            return Err(RuntimeControlError::Validation(format!(
                "Unknown start node: {start_node}"
            )));
        }
        let mode = request.flow_source_mode.trim().to_lowercase();
        if !matches!(mode.as_str(), "snapshot" | "flow_name") {
            return Err(RuntimeControlError::Validation(
                "flow_source_mode must be either snapshot or flow_name.".to_string(),
            ));
        }

        let mut context = source_checkpoint.context.clone();
        let new_run_id = request
            .new_run_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(generated_run_id);
        let working_directory = request
            .working_directory
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| non_empty_string(&source_record.working_directory))
            .or_else(|| non_empty_string(&source_record.project_path))
            .ok_or_else(|| {
                RuntimeControlError::Conflict("Continue requires a working directory".to_string())
            })?;
        let model = request
            .model
            .as_deref()
            .and_then(trimmed_real_model)
            .or_else(|| {
                context_string(&context, RUNTIME_LAUNCH_MODEL_KEY)
                    .and_then(|value| trimmed_real_model(&value))
            })
            .or_else(|| trimmed_real_model(&source_record.model))
            .unwrap_or_default();
        let provider = request
            .llm_provider
            .as_deref()
            .and_then(normalized_lowercase)
            .or_else(|| {
                context_string(&context, RUNTIME_LAUNCH_PROVIDER_KEY)
                    .and_then(|value| normalized_lowercase(&value))
            })
            .or_else(|| normalized_lowercase(&source_record.llm_provider))
            .or_else(|| normalized_lowercase(&source_record.provider))
            .unwrap_or_else(|| "codex".to_string());
        let llm_profile = request
            .llm_profile
            .as_deref()
            .and_then(non_empty_string)
            .or_else(|| context_string(&context, RUNTIME_LAUNCH_PROFILE_KEY))
            .or_else(|| {
                source_record
                    .llm_profile
                    .as_deref()
                    .and_then(non_empty_string)
            });
        let reasoning_effort = request
            .reasoning_effort
            .as_deref()
            .and_then(normalized_lowercase)
            .or_else(|| {
                context_string(&context, RUNTIME_LAUNCH_REASONING_EFFORT_KEY)
                    .and_then(|value| normalized_lowercase(&value))
            })
            .or_else(|| {
                source_record
                    .reasoning_effort
                    .as_deref()
                    .and_then(normalized_lowercase)
            });
        set_launch_context_value(
            &mut context,
            RUNTIME_LAUNCH_MODEL_KEY,
            non_empty_string(&model),
        );
        set_launch_context_value(
            &mut context,
            RUNTIME_LAUNCH_PROVIDER_KEY,
            Some(provider.clone()),
        );
        set_launch_context_value(
            &mut context,
            RUNTIME_LAUNCH_PROFILE_KEY,
            llm_profile.clone(),
        );
        set_launch_context_value(
            &mut context,
            RUNTIME_LAUNCH_REASONING_EFFORT_KEY,
            reasoning_effort.clone(),
        );

        let flow_name = request
            .flow_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| source_record.flow_name.clone());
        let mut record = source_record.clone();
        record.run_id = new_run_id.clone();
        record.flow_name = flow_name.clone();
        record.status = "running".to_string();
        record.outcome = None;
        record.outcome_reason_code = None;
        record.outcome_reason_message = None;
        record.working_directory = working_directory.clone();
        if record.project_path.trim().is_empty() {
            record.project_path = working_directory.clone();
        }
        record.model = model.clone();
        record.provider = provider.clone();
        record.llm_provider = provider.clone();
        record.llm_profile = llm_profile.clone();
        record.reasoning_effort = reasoning_effort.clone();
        record.started_at = crate::events::utc_timestamp();
        record.ended_at = None;
        record.continued_from_run_id = Some(source_record.run_id.clone());
        record.continued_from_node = Some(start_node.to_string());
        record.continued_from_flow_mode = Some(mode.clone());
        record.continued_from_flow_name = if mode == "flow_name" {
            request
                .flow_name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        } else {
            None
        };
        record.parent_run_id = None;
        record.parent_node_id = None;
        record.root_run_id = Some(new_run_id.clone());
        record.last_error.clear();

        let checkpoint = CheckpointState {
            timestamp: crate::events::utc_timestamp(),
            current_node: start_node.to_string(),
            completed_nodes: Vec::new(),
            context,
            retry_counts: BTreeMap::new(),
            logs: source_checkpoint.logs.clone(),
        };
        let paths = self.store.create_run(CreateRunRequest {
            record,
            checkpoint: Some(checkpoint),
            manifest: Some(RunManifest {
                goal: request.graph.goal(),
                graph_id: request.graph.graph_id.clone(),
                start_node: start_node.to_string(),
                started_at: crate::events::utc_timestamp(),
                extra: BTreeMap::new(),
            }),
            graph_source: request.graph_source,
            graph_dot: request.graph_dot,
        })?;
        self.store.append_transcript_event(
            &paths,
            crate::events::pipeline_started_event(
                &new_run_id,
                &request.graph.graph_id,
                start_node,
                false,
            ),
        )?;

        Ok(ContinueRunStarted {
            status: "started".to_string(),
            pipeline_id: new_run_id.clone(),
            run_id: new_run_id,
            working_directory,
            model,
            provider: provider.clone(),
            llm_provider: provider,
            llm_profile,
            reasoning_effort,
        })
    }

    pub fn prepare_retry(&self, run_id: &str) -> ControlResult<RetryRunPrepared> {
        let bundle = self
            .store
            .read_run_bundle(run_id)?
            .ok_or(RuntimeControlError::UnknownPipeline)?;
        let mut record = bundle.record.ok_or(RuntimeControlError::UnknownPipeline)?;
        if crate::records::normalize_run_status(&record.status) != "failed" {
            return Err(RuntimeControlError::Conflict(
                "Retry requires a failed pipeline".to_string(),
            ));
        }
        let mut checkpoint = bundle.checkpoint.ok_or(RuntimeControlError::Conflict(
            "Retry requires an available checkpoint".to_string(),
        ))?;
        checkpoint.context.insert(
            INTERNAL_PIPELINE_RETRY_RUN_ID_KEY.to_string(),
            json!(run_id),
        );
        let current_outcome = checkpoint
            .context
            .get("_attractor.node_outcomes")
            .and_then(Value::as_object)
            .and_then(|outcomes| outcomes.get(&checkpoint.current_node))
            .and_then(Value::as_str);
        if current_outcome == Some("fail") {
            checkpoint
                .completed_nodes
                .retain(|node_id| node_id != &checkpoint.current_node);
        }

        mark_record_retry_started(&mut record);
        self.store.write_run_record(&bundle.paths, &record)?;
        self.store.save_checkpoint(
            &bundle.paths,
            &checkpoint,
            CheckpointWriteOptions::default(),
        )?;
        self.store.append_event(
            &bundle.paths,
            pipeline_retry_started_event(
                run_id,
                &checkpoint.current_node,
                checkpoint.completed_nodes.clone(),
            ),
        )?;
        self.store.append_event(
            &bundle.paths,
            runtime_status_event(run_id, "running", None, None, None, None),
        )?;

        Ok(RetryRunPrepared {
            status: "started".to_string(),
            pipeline_id: run_id.to_string(),
            run_id: run_id.to_string(),
            current_node: checkpoint.current_node,
            completed_nodes: checkpoint.completed_nodes,
        })
    }

    pub fn request_cancel(&self, run_id: &str) -> ControlResult<RuntimeControlStatus> {
        let bundle = self
            .store
            .read_run_bundle(run_id)?
            .ok_or(RuntimeControlError::UnknownPipeline)?;
        let mut record = bundle.record.ok_or(RuntimeControlError::UnknownPipeline)?;
        if !matches!(
            crate::records::normalize_run_status(&record.status).as_str(),
            "queued" | "running" | "pause_requested" | "cancel_requested"
        ) {
            return Ok(RuntimeControlStatus {
                status: "ignored".to_string(),
                pipeline_id: run_id.to_string(),
                run_id: run_id.to_string(),
            });
        }
        mark_record_cancel_requested(&mut record);
        self.store.write_run_record(&bundle.paths, &record)?;
        self.store
            .append_event(&bundle.paths, cancel_requested_event(run_id))?;
        self.store.append_event(
            &bundle.paths,
            runtime_status_event(
                run_id,
                "cancel_requested",
                None,
                None,
                None,
                Some("cancel_requested_by_user".to_string()),
            ),
        )?;
        self.store.append_event(
            &bundle.paths,
            log_event(
                run_id,
                "[System] Cancel requested. Stopping after current node.",
            ),
        )?;
        Ok(RuntimeControlStatus {
            status: "cancel_requested".to_string(),
            pipeline_id: run_id.to_string(),
            run_id: run_id.to_string(),
        })
    }

    pub fn request_pause(&self, run_id: &str) -> ControlResult<RuntimeControlStatus> {
        let bundle = self
            .store
            .read_run_bundle(run_id)?
            .ok_or(RuntimeControlError::UnknownPipeline)?;
        let mut record = bundle.record.ok_or(RuntimeControlError::UnknownPipeline)?;
        let current_node = bundle
            .checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.current_node.clone())
            .unwrap_or_default();
        mark_record_paused(&mut record);
        self.store.write_run_record(&bundle.paths, &record)?;
        self.store
            .append_event(&bundle.paths, pipeline_paused_event(run_id, current_node))?;
        self.store.append_event(
            &bundle.paths,
            runtime_status_event(run_id, "paused", None, None, None, None),
        )?;
        Ok(RuntimeControlStatus {
            status: "paused".to_string(),
            pipeline_id: run_id.to_string(),
            run_id: run_id.to_string(),
        })
    }

    pub fn mark_canceled(
        &self,
        run_id: &str,
        last_error: &str,
    ) -> ControlResult<RuntimeControlStatus> {
        let bundle = self
            .store
            .read_run_bundle(run_id)?
            .ok_or(RuntimeControlError::UnknownPipeline)?;
        let mut record = bundle.record.ok_or(RuntimeControlError::UnknownPipeline)?;
        mark_record_canceled(&mut record, last_error);
        self.store.write_run_record(&bundle.paths, &record)?;
        self.store.append_event(
            &bundle.paths,
            runtime_status_event(
                run_id,
                "canceled",
                None,
                None,
                None,
                Some(last_error.to_string()),
            ),
        )?;
        Ok(RuntimeControlStatus {
            status: "canceled".to_string(),
            pipeline_id: run_id.to_string(),
            run_id: run_id.to_string(),
        })
    }
}

fn generated_run_id() -> String {
    format!(
        "run-{}",
        crate::events::utc_timestamp().replace([':', '.'], "-")
    )
}

fn non_empty_string(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn normalized_lowercase(value: &str) -> Option<String> {
    non_empty_string(value).map(|value| value.to_ascii_lowercase())
}

fn trimmed_real_model(value: &str) -> Option<String> {
    non_empty_string(value).filter(|value| !is_display_model_placeholder(value))
}

fn set_launch_context_value(context: &mut ContextMap, key: &str, value: Option<String>) {
    match value.and_then(|value| non_empty_string(&value)) {
        Some(value) => {
            context.insert(key.to_string(), json!(value));
        }
        None => {
            context.remove(key);
        }
    }
}

fn context_string(context: &ContextMap, key: &str) -> Option<String> {
    context
        .get(key)
        .and_then(value_to_string)
        .filter(|value| !value.is_empty())
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(value) => Some(value.trim().to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

impl From<RuntimeControlError> for RuntimeStorageError {
    fn from(error: RuntimeControlError) -> Self {
        match error {
            RuntimeControlError::Storage(error) => error,
            other => RuntimeStorageError::InvalidRuntimeGraph {
                reason: other.to_string(),
            },
        }
    }
}
