use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use attractor_api::{
    ContinuePipelineRequest, PipelineStartRequest, RuntimeHandlerRunnerFactory,
    RuntimeRouteResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use spark_agent_adapter::{
    AgentError, AgentRequestUserInputAnswerRequest, AgentThreadResumeFailure, AgentTurnBackend,
    AgentTurnEventSink, AgentTurnOutput, AgentTurnRequest, AssistantTurn, HistoryTurn,
    RustLlmAgentTurnBackend, UserTurn,
};
use spark_common::debug::{codex_jsonrpc_trace_enabled, CODEX_JSONRPC_TRACE_PATH_METADATA_KEY};
use spark_common::events::{
    TurnStreamChannel, TurnStreamEvent, TurnStreamEventKind, TurnStreamSource,
};
use spark_common::project::normalize_project_path;
use spark_common::settings::SparkSettings;
use spark_storage::{
    ConversationRepository, ProjectRegistry, StorageError, CONVERSATION_STATE_SCHEMA_VERSION,
};
use spark_triggers::TriggerActivationRequest;
use time::OffsetDateTime;

use crate::errors::{WorkspaceError, WorkspaceResult};

const UI_TOOL_OUTPUT_PREVIEW_BYTES: usize = 8 * 1024;
const DEPRECATED_EVENTS_MESSAGE: &str =
    "Deprecated. Use /workspace/api/live/events with conversation_id and conversation_revision.";
const ACTIVE_ASSISTANT_TURN_MESSAGE: &str = "An assistant turn is still in progress for this conversation. Wait for it to finish before sending another message.";
const REQUEST_USER_INPUT_EXPIRED_ERROR: &str =
    "The requested input expired before the answer could be used.";
const MISSING_FINAL_ANSWER_ERROR: &str =
    "codex app-server completed the turn without a final answer item.";
const IMPLEMENT_CHANGE_REQUEST_FLOW: &str = "software-development/implement-change-request.yaml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationSummary {
    pub conversation_id: String,
    pub conversation_handle: String,
    pub project_path: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub revision: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_message_preview: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ConversationSettingsUpdate {
    pub project_path: String,
    #[serde(default)]
    pub chat_mode: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub llm_profile: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ConversationTurnRequest {
    pub project_path: String,
    pub message: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub llm_profile: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub chat_mode: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ConversationRequestUserInputAnswerRequest {
    pub project_path: String,
    #[serde(default)]
    pub answers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlowRunRequestCreateByHandleRequest {
    pub flow_name: String,
    pub summary: String,
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default)]
    pub launch_context: Option<Value>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub llm_provider: Option<String>,
    #[serde(default)]
    pub llm_profile: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub execution_profile_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct FlowRunRequestReviewRequest {
    pub project_path: String,
    pub disposition: String,
    pub message: String,
    #[serde(default)]
    pub flow_name: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub llm_provider: Option<String>,
    #[serde(default)]
    pub llm_profile: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub execution_profile_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ProposedPlanReviewRequest {
    pub project_path: String,
    pub disposition: String,
    #[serde(default)]
    pub review_note: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct RunLaunchRequest {
    pub flow_name: String,
    pub summary: String,
    #[serde(default)]
    pub conversation_handle: Option<String>,
    #[serde(default)]
    pub project_path: Option<String>,
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default)]
    pub launch_context: Option<Value>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub llm_provider: Option<String>,
    #[serde(default)]
    pub llm_profile: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub execution_profile_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRetryRequest {
    #[serde(default)]
    pub conversation_handle: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunContinueRequest {
    pub start_node: String,
    pub flow_source_mode: String,
    #[serde(default)]
    pub flow_name: Option<String>,
    #[serde(default)]
    pub project_path: Option<String>,
    #[serde(default)]
    pub conversation_handle: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub llm_provider: Option<String>,
    #[serde(default)]
    pub llm_profile: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PreparedConversationTurn {
    pub conversation_id: String,
    pub project_path: String,
    pub chat_mode: String,
    pub prompt: String,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    pub user_turn_id: String,
    pub assistant_turn_id: String,
    pub agent_turn_request: AgentTurnRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConversationDeleteResponse {
    pub status: &'static str,
    pub conversation_id: String,
    pub project_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FlowRunRequestCreateResponse {
    pub ok: bool,
    pub conversation_handle: String,
    pub conversation_id: String,
    pub project_path: String,
    pub turn_id: String,
    pub flow_run_request_id: String,
    pub segment_id: String,
}

#[derive(Clone)]
pub struct WorkspaceConversationService {
    settings: SparkSettings,
    runtime_handler_runner_factory: RuntimeHandlerRunnerFactory,
    agent_turn_backend: Arc<dyn AgentTurnBackend>,
}

#[derive(Clone)]
struct EnvironmentAgentTurnBackend {
    config_dir: PathBuf,
}

impl EnvironmentAgentTurnBackend {
    fn new(config_dir: PathBuf) -> Self {
        Self { config_dir }
    }
}

impl AgentTurnBackend for EnvironmentAgentTurnBackend {
    fn run_turn(&self, request: AgentTurnRequest) -> Result<AgentTurnOutput, AgentError> {
        self.run_turn_with_event_sink(request, None)
    }

    fn run_turn_with_event_sink(
        &self,
        request: AgentTurnRequest,
        event_sink: Option<AgentTurnEventSink>,
    ) -> Result<AgentTurnOutput, AgentError> {
        let client = unified_llm_adapter::Client::from_env_and_profiles(&self.config_dir, None)
            .map_err(adapter_error_to_agent_error)?;
        RustLlmAgentTurnBackend::new(client).run_turn_with_event_sink(request, event_sink)
    }

    fn answer_request_user_input(
        &self,
        request: AgentRequestUserInputAnswerRequest,
    ) -> Result<AgentTurnOutput, AgentError> {
        let client = unified_llm_adapter::Client::from_env_and_profiles(&self.config_dir, None)
            .map_err(adapter_error_to_agent_error)?;
        RustLlmAgentTurnBackend::new(client).answer_request_user_input(request)
    }
}

fn default_agent_turn_backend(settings: &SparkSettings) -> Arc<dyn AgentTurnBackend> {
    Arc::new(EnvironmentAgentTurnBackend::new(
        settings.config_dir.clone(),
    ))
}

fn adapter_error_to_agent_error(error: unified_llm_adapter::AdapterError) -> AgentError {
    AgentError {
        message: error.to_string(),
        retryable: error.retryable,
        raw: serde_json::to_value(error).ok(),
    }
}

fn agent_turn_backend_error(error: AgentError) -> WorkspaceError {
    let message =
        non_empty_string(&error.message).unwrap_or_else(|| "Agent turn failed.".to_string());
    if error.retryable {
        WorkspaceError::ServiceUnavailable(message)
    } else {
        WorkspaceError::Internal(message)
    }
}

impl WorkspaceConversationService {
    pub fn new(settings: SparkSettings) -> Self {
        Self::new_with_runtime_handler_runner_factory(
            settings,
            attractor_api::default_runtime_handler_runner_factory(),
        )
    }

    pub fn default_agent_turn_backend(settings: &SparkSettings) -> Arc<dyn AgentTurnBackend> {
        default_agent_turn_backend(settings)
    }

    pub fn new_with_runtime_handler_runner_factory(
        settings: SparkSettings,
        runtime_handler_runner_factory: RuntimeHandlerRunnerFactory,
    ) -> Self {
        let agent_turn_backend = default_agent_turn_backend(&settings);
        Self::new_with_runtime_handler_runner_factory_and_agent_turn_backend(
            settings,
            runtime_handler_runner_factory,
            agent_turn_backend,
        )
    }

    pub fn new_with_agent_turn_backend(
        settings: SparkSettings,
        agent_turn_backend: Arc<dyn AgentTurnBackend>,
    ) -> Self {
        Self::new_with_runtime_handler_runner_factory_and_agent_turn_backend(
            settings,
            attractor_api::default_runtime_handler_runner_factory(),
            agent_turn_backend,
        )
    }

    pub fn new_with_runtime_handler_runner_factory_and_agent_turn_backend(
        settings: SparkSettings,
        runtime_handler_runner_factory: RuntimeHandlerRunnerFactory,
        agent_turn_backend: Arc<dyn AgentTurnBackend>,
    ) -> Self {
        Self {
            settings,
            runtime_handler_runner_factory,
            agent_turn_backend,
        }
    }

    pub fn list_project_conversations(
        &self,
        project_path: &str,
    ) -> WorkspaceResult<Vec<ConversationSummary>> {
        let project_path = normalize_project_path_or_400(project_path)?;
        let repository = self.repository();
        let project_paths = repository.project_paths(&project_path)?;
        let mut summaries = Vec::new();
        for conversation_id in repository.list_conversation_ids_for_project(&project_path)? {
            match repository.read_snapshot(&conversation_id, Some(&project_path)) {
                Ok(Some(snapshot)) => {
                    if let Some(mut summary) = conversation_summary_from_snapshot(
                        &snapshot,
                        &conversation_id,
                        &project_path,
                    ) {
                        let preferred_handle =
                            snapshot.get("conversation_handle").and_then(Value::as_str);
                        summary.conversation_handle =
                            repository.handle_repository().ensure_conversation_handle(
                                &summary.conversation_id,
                                &project_paths.project_id,
                                &project_path,
                                &summary.created_at,
                                preferred_handle,
                            )?;
                        summaries.push(summary);
                    }
                }
                Ok(None) => {}
                Err(StorageError::InvalidConversationState { .. }) => {}
                Err(error) => return Err(error.into()),
            }
        }
        summaries.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.conversation_id.cmp(&right.conversation_id))
        });
        Ok(summaries)
    }

    pub fn get_snapshot(
        &self,
        conversation_id: &str,
        project_path: Option<&str>,
    ) -> WorkspaceResult<Value> {
        let requested_project_path = normalize_optional_project_path(project_path)?;
        let repository = self.repository();
        let mut snapshot =
            match repository.read_snapshot(conversation_id, requested_project_path.as_deref())? {
                Some(snapshot) => snapshot,
                None => {
                    let Some(ref project_path) = requested_project_path else {
                        return Err(WorkspaceError::NotFound(format!(
                            "Unknown conversation: {conversation_id}"
                        )));
                    };
                    if let Some(existing) = repository.read_snapshot(conversation_id, None)? {
                        let actual_project_path = existing
                            .get("project_path")
                            .and_then(Value::as_str)
                            .and_then(normalize_project_path_string)
                            .unwrap_or_default();
                        if !actual_project_path.is_empty()
                            && actual_project_path != project_path.as_str()
                        {
                            return Err(WorkspaceError::Validation(
                                "Conversation is already bound to a different project path."
                                    .to_string(),
                            ));
                        }
                    }
                    shell_snapshot(conversation_id, project_path)
                }
            };

        if let Some(expected_project_path) = requested_project_path.as_deref() {
            let actual_project_path = snapshot
                .get("project_path")
                .and_then(Value::as_str)
                .and_then(normalize_project_path_string)
                .unwrap_or_default();
            if !actual_project_path.is_empty() && actual_project_path != expected_project_path {
                return Err(WorkspaceError::Validation(
                    "Conversation is already bound to a different project path.".to_string(),
                ));
            }
        }
        prepare_snapshot_for_ui(&mut snapshot, conversation_id);
        Ok(snapshot)
    }

    pub fn update_conversation_settings(
        &self,
        conversation_id: &str,
        request: ConversationSettingsUpdate,
    ) -> WorkspaceResult<Value> {
        let project_path = normalize_project_path_or_400(&request.project_path)?;
        let chat_mode = request
            .chat_mode
            .as_deref()
            .map(validate_chat_mode)
            .transpose()?;
        let provider = request
            .provider
            .as_deref()
            .map(validate_provider)
            .transpose()?;
        let reasoning_effort = request
            .reasoning_effort
            .as_deref()
            .map(validate_reasoning_effort)
            .transpose()?;
        let normalized_model = request.model.as_deref().and_then(non_empty_string);
        let normalized_profile = request.llm_profile.as_deref().and_then(non_empty_string);

        let repository = self.repository();
        let mut snapshot = match repository.read_snapshot(conversation_id, None)? {
            Some(snapshot) => snapshot,
            None => shell_snapshot(conversation_id, &project_path),
        };
        let actual_project_path = snapshot
            .get("project_path")
            .and_then(Value::as_str)
            .and_then(normalize_project_path_string)
            .unwrap_or_else(|| project_path.clone());
        if actual_project_path != project_path {
            return Err(WorkspaceError::Validation(
                "Conversation is already bound to a different project path.".to_string(),
            ));
        }

        prepare_snapshot_core_defaults(&mut snapshot, conversation_id, &project_path);
        let current_chat_mode = normalize_chat_mode(
            snapshot
                .get("chat_mode")
                .and_then(Value::as_str)
                .unwrap_or("chat"),
        );
        if let Some(chat_mode) = chat_mode {
            if chat_mode != current_chat_mode {
                append_mode_change_turn(&mut snapshot, &chat_mode);
                set_string(&mut snapshot, "chat_mode", &chat_mode);
            } else {
                set_string(&mut snapshot, "chat_mode", &current_chat_mode);
            }
        } else {
            set_string(&mut snapshot, "chat_mode", &current_chat_mode);
        }
        if request.provider.is_some() {
            set_string(
                &mut snapshot,
                "provider",
                provider.as_deref().unwrap_or("codex"),
            );
        }
        if request.model.is_some() {
            set_optional_string(&mut snapshot, "model", normalized_model.as_deref());
        }
        if request.llm_profile.is_some() {
            set_optional_string(&mut snapshot, "llm_profile", normalized_profile.as_deref());
        }
        if request.reasoning_effort.is_some() {
            set_optional_string(
                &mut snapshot,
                "reasoning_effort",
                reasoning_effort.as_deref(),
            );
        }

        touch_snapshot(&repository, &mut snapshot, conversation_id, &project_path)?;
        repository.write_snapshot(&snapshot)?;
        prepare_snapshot_for_ui(&mut snapshot, conversation_id);
        Ok(snapshot)
    }

    pub fn start_turn(
        &self,
        conversation_id: &str,
        request: ConversationTurnRequest,
    ) -> WorkspaceResult<(PreparedConversationTurn, Value)> {
        let project_path = normalize_project_path_or_400(&request.project_path)?;
        let message = non_empty_string(&request.message)
            .ok_or_else(|| WorkspaceError::Validation("Message is required.".to_string()))?;
        let chat_mode = request
            .chat_mode
            .as_deref()
            .map(validate_chat_mode)
            .transpose()?;
        let provider = request
            .provider
            .as_deref()
            .map(validate_provider)
            .transpose()?;
        let reasoning_effort = request
            .reasoning_effort
            .as_deref()
            .map(validate_reasoning_effort)
            .transpose()?;
        let normalized_model = request.model.as_deref().and_then(non_empty_string);
        let normalized_profile = request.llm_profile.as_deref().and_then(non_empty_string);

        let repository = self.repository();
        let mut snapshot = match repository.read_snapshot(conversation_id, None)? {
            Some(snapshot) => snapshot,
            None => shell_snapshot(conversation_id, &project_path),
        };
        let actual_project_path =
            snapshot_project_path(&snapshot).unwrap_or_else(|| project_path.clone());
        if actual_project_path != project_path {
            return Err(WorkspaceError::Validation(
                "Conversation is already bound to a different project path.".to_string(),
            ));
        }

        prepare_snapshot_core_defaults(&mut snapshot, conversation_id, &project_path);
        let current_chat_mode = normalize_chat_mode(
            snapshot
                .get("chat_mode")
                .and_then(Value::as_str)
                .unwrap_or("chat"),
        );
        let effective_chat_mode = chat_mode.unwrap_or(current_chat_mode.clone());
        let mut emitted_payloads = Vec::new();
        if effective_chat_mode != current_chat_mode {
            let mode_change = append_mode_change_turn(&mut snapshot, &effective_chat_mode);
            emitted_payloads.push(build_turn_upsert_payload(&snapshot, &mode_change));
        }
        set_string(&mut snapshot, "chat_mode", &effective_chat_mode);
        if request.provider.is_some() {
            set_string(
                &mut snapshot,
                "provider",
                provider.as_deref().unwrap_or("codex"),
            );
        }
        if request.model.is_some() {
            set_optional_string(&mut snapshot, "model", normalized_model.as_deref());
        }
        if request.llm_profile.is_some() {
            set_optional_string(&mut snapshot, "llm_profile", normalized_profile.as_deref());
        }
        if request.reasoning_effort.is_some() {
            set_optional_string(
                &mut snapshot,
                "reasoning_effort",
                reasoning_effort.as_deref(),
            );
        }

        let effective_provider = snapshot
            .get("provider")
            .and_then(Value::as_str)
            .map(validate_provider)
            .transpose()?
            .unwrap_or_else(|| "codex".to_string());
        let effective_model = snapshot
            .get("model")
            .and_then(Value::as_str)
            .and_then(non_empty_string);
        let effective_profile = snapshot
            .get("llm_profile")
            .and_then(Value::as_str)
            .and_then(non_empty_string);
        let effective_reasoning_effort = snapshot
            .get("reasoning_effort")
            .and_then(Value::as_str)
            .and_then(non_empty_string);
        if ((effective_profile.is_some() && effective_provider != "codex")
            || matches!(
                effective_provider.as_str(),
                "openrouter" | "litellm" | "openai_compatible"
            ))
            && effective_model.is_none()
        {
            return Err(WorkspaceError::Validation(format!(
                "Provider {effective_provider} requires an explicit model."
            )));
        }
        if active_assistant_turn_id(&snapshot).is_some() {
            return Err(WorkspaceError::Conflict(
                ACTIVE_ASSISTANT_TURN_MESSAGE.to_string(),
            ));
        }
        let previous_app_thread_id = latest_codex_app_thread_id(&snapshot);

        let user_turn = json!({
            "id": format!("turn-{}", uuid::Uuid::new_v4().simple()),
            "role": "user",
            "content": message,
            "timestamp": iso_now(),
            "status": "complete",
            "kind": "message",
        });
        let assistant_turn = json!({
            "id": format!("turn-{}", uuid::Uuid::new_v4().simple()),
            "role": "assistant",
            "content": "",
            "timestamp": iso_now(),
            "status": "pending",
            "kind": "message",
            "parent_turn_id": user_turn.get("id").and_then(Value::as_str).unwrap_or(""),
        });
        let user_turn_id = user_turn
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let assistant_turn_id = assistant_turn
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        push_turn(&mut snapshot, user_turn.clone());
        push_turn(&mut snapshot, assistant_turn.clone());
        maybe_set_title_from_message(&mut snapshot, &message);
        touch_snapshot(&repository, &mut snapshot, conversation_id, &project_path)?;
        emitted_payloads.push(build_turn_upsert_payload(&snapshot, &user_turn));
        emitted_payloads.push(build_turn_upsert_payload(&snapshot, &assistant_turn));
        stamp_progress_payloads_with_state_revision(&mut snapshot, &mut emitted_payloads);
        repository.write_snapshot(&snapshot)?;
        append_events(
            &repository,
            conversation_id,
            &project_path,
            &emitted_payloads,
        )?;

        let mut metadata = BTreeMap::from([
            (
                "spark.workspace.user_turn_id".to_string(),
                json!(user_turn_id.clone()),
            ),
            (
                "spark.workspace.assistant_turn_id".to_string(),
                json!(assistant_turn_id.clone()),
            ),
        ]);
        if let Some(thread_id) = previous_app_thread_id {
            metadata.insert(
                "spark.runtime.codex_app_server.thread_id".to_string(),
                json!(thread_id),
            );
        }
        if codex_jsonrpc_trace_enabled() {
            if let Some(trace_path) = repository
                .conversation_codex_jsonrpc_trace_path(conversation_id, Some(&project_path))?
            {
                metadata.insert(
                    CODEX_JSONRPC_TRACE_PATH_METADATA_KEY.to_string(),
                    json!(trace_path.to_string_lossy().to_string()),
                );
            }
        }
        let agent_turn_request = AgentTurnRequest {
            conversation_id: conversation_id.to_string(),
            project_path: project_path.clone(),
            prompt: message.clone(),
            history: agent_history_from_snapshot(&snapshot, &[&user_turn_id, &assistant_turn_id]),
            provider: Some(effective_provider.clone()),
            model: effective_model.clone(),
            llm_profile: effective_profile.clone(),
            reasoning_effort: effective_reasoning_effort.clone(),
            chat_mode: Some(effective_chat_mode.clone()),
            metadata,
        };

        let prepared = PreparedConversationTurn {
            conversation_id: conversation_id.to_string(),
            project_path: project_path.clone(),
            chat_mode: effective_chat_mode,
            prompt: message,
            provider: effective_provider,
            model: effective_model,
            llm_profile: effective_profile,
            reasoning_effort: effective_reasoning_effort,
            user_turn_id,
            assistant_turn_id,
            agent_turn_request,
        };
        prepare_snapshot_for_ui(&mut snapshot, conversation_id);
        Ok((prepared, snapshot))
    }

    pub fn ingest_agent_turn_output(
        &self,
        conversation_id: &str,
        project_path: &str,
        assistant_turn_id: &str,
        chat_mode: &str,
        output: AgentTurnOutput,
    ) -> WorkspaceResult<Value> {
        let project_path = normalize_project_path_or_400(project_path)?;
        let chat_mode = normalize_chat_mode(chat_mode);
        let repository = self.repository();

        let mut snapshot = repository
            .read_snapshot(conversation_id, Some(&project_path))?
            .ok_or_else(|| {
                WorkspaceError::NotFound(format!("Unknown conversation: {conversation_id}"))
            })?;
        prepare_snapshot_core_defaults(&mut snapshot, conversation_id, &project_path);
        let mut emitted_payloads = Vec::new();
        if apply_assistant_turn_app_server_ids(
            &mut snapshot,
            assistant_turn_id,
            output.app_thread_id.as_deref(),
            output.app_turn_id.as_deref(),
        ) {
            emit_assistant_turn_upsert(&snapshot, assistant_turn_id, &mut emitted_payloads);
        }
        let mut buffered_plan_assistant_event: Option<TurnStreamEvent> = None;

        for event in output.events {
            ensure_assistant_streaming(&mut snapshot, assistant_turn_id, &mut emitted_payloads);
            match &event.kind {
                TurnStreamEventKind::TokenUsageUpdated => {
                    if let Some(token_usage) = event.token_usage.clone() {
                        if let Some(turn) = find_turn_mut(&mut snapshot, assistant_turn_id) {
                            set_value(turn, "token_usage", token_usage);
                            let turn = turn.clone();
                            emitted_payloads.push(build_turn_upsert_payload(&snapshot, &turn));
                        }
                    }
                }
                TurnStreamEventKind::ContentCompleted
                    if event.channel == Some(TurnStreamChannel::Assistant) =>
                {
                    if chat_mode == "plan" && is_final_answer_phase(event.phase.as_deref()) {
                        buffered_plan_assistant_event = Some(event.clone());
                    } else {
                        if chat_mode != "plan"
                            && is_final_answer_phase(event.phase.as_deref())
                            && event_text(&event).is_some()
                        {
                            if let Some(turn) = find_turn_mut(&mut snapshot, assistant_turn_id) {
                                set_string_value(
                                    turn,
                                    "content",
                                    event_text(&event).as_deref().unwrap_or(""),
                                );
                                set_string_value(turn, "status", "streaming");
                                remove_key(turn, "error");
                                let turn = turn.clone();
                                emitted_payloads.push(build_turn_upsert_payload(&snapshot, &turn));
                            }
                        }
                        if let Some(segment) =
                            materialize_segment_for_event(&mut snapshot, assistant_turn_id, &event)
                                .filter(|_| should_emit_segment_upsert_for_event(&event))
                        {
                            emitted_payloads
                                .push(build_segment_upsert_payload(&snapshot, &segment));
                        }
                    }
                }
                TurnStreamEventKind::Error => {
                    let message = event
                        .error
                        .clone()
                        .or_else(|| event.message.clone())
                        .unwrap_or_else(|| "Conversation turn failed.".to_string());
                    if let Some(turn) = find_turn_mut(&mut snapshot, assistant_turn_id) {
                        set_string_value(turn, "status", "failed");
                        set_string_value(turn, "error", &message);
                        if let Some(error_code) =
                            event.error_code.as_deref().and_then(non_empty_string)
                        {
                            set_string_value(turn, "error_code", &error_code);
                        }
                        if let Some(details) = event.details.as_ref() {
                            set_value(turn, "details", details.clone());
                        }
                        let turn = turn.clone();
                        emitted_payloads.push(build_turn_upsert_payload(&snapshot, &turn));
                    }
                    if let Some(segment) =
                        materialize_segment_for_event(&mut snapshot, assistant_turn_id, &event)
                            .filter(|_| should_emit_segment_upsert_for_event(&event))
                    {
                        emitted_payloads.push(build_segment_upsert_payload(&snapshot, &segment));
                    }
                }
                TurnStreamEventKind::TurnCompleted => {
                    if let Some(segment) =
                        materialize_segment_for_event(&mut snapshot, assistant_turn_id, &event)
                            .filter(|_| should_emit_segment_upsert_for_event(&event))
                    {
                        emitted_payloads.push(build_segment_upsert_payload(&snapshot, &segment));
                    }
                }
                _ => {
                    if let Some(segment) =
                        materialize_segment_for_event(&mut snapshot, assistant_turn_id, &event)
                            .filter(|_| should_emit_segment_upsert_for_event(&event))
                    {
                        emitted_payloads.push(build_segment_upsert_payload(&snapshot, &segment));
                    }
                }
            }
        }

        finalize_agent_turn_output(
            &mut snapshot,
            assistant_turn_id,
            &chat_mode,
            output.final_assistant_text.as_deref(),
            output.token_usage,
            output.token_usage_breakdown,
            output.thread_resume_failure.as_ref(),
            buffered_plan_assistant_event.as_ref(),
            &mut emitted_payloads,
        );
        touch_snapshot(&repository, &mut snapshot, conversation_id, &project_path)?;
        emitted_payloads.push(build_conversation_snapshot_payload(&snapshot));
        stamp_progress_payloads_with_state_revision(&mut snapshot, &mut emitted_payloads);
        repository.write_snapshot(&snapshot)?;
        append_events(
            &repository,
            conversation_id,
            &project_path,
            &emitted_payloads,
        )?;
        prepare_snapshot_for_ui(&mut snapshot, conversation_id);
        Ok(snapshot)
    }

    fn ingest_agent_turn_backend_failure(
        &self,
        prepared: &PreparedConversationTurn,
        error: AgentError,
    ) -> WorkspaceResult<Value> {
        let repository = self.repository();
        let mut snapshot = repository
            .read_snapshot(&prepared.conversation_id, Some(&prepared.project_path))?
            .ok_or_else(|| {
                WorkspaceError::NotFound(format!(
                    "Unknown conversation: {}",
                    prepared.conversation_id
                ))
            })?;
        prepare_snapshot_core_defaults(
            &mut snapshot,
            &prepared.conversation_id,
            &prepared.project_path,
        );
        let message =
            non_empty_string(&error.message).unwrap_or_else(|| "Agent turn failed.".to_string());
        let mut emitted_payloads = Vec::new();
        fail_assistant_turn_and_segments(
            &mut snapshot,
            &prepared.assistant_turn_id,
            &message,
            None,
            error.raw.as_ref(),
            true,
            &mut emitted_payloads,
        );
        touch_snapshot(
            &repository,
            &mut snapshot,
            &prepared.conversation_id,
            &prepared.project_path,
        )?;
        emitted_payloads.push(build_conversation_snapshot_payload(&snapshot));
        stamp_progress_payloads_with_state_revision(&mut snapshot, &mut emitted_payloads);
        repository.write_snapshot(&snapshot)?;
        append_events(
            &repository,
            &prepared.conversation_id,
            &prepared.project_path,
            &emitted_payloads,
        )?;
        prepare_snapshot_for_ui(&mut snapshot, &prepared.conversation_id);
        Ok(snapshot)
    }

    pub fn execute_turn(
        &self,
        conversation_id: &str,
        request: ConversationTurnRequest,
    ) -> WorkspaceResult<Value> {
        self.execute_turn_with_progress_payloads(conversation_id, request, |_| {})
    }

    pub fn execute_turn_with_progress_payloads<F>(
        &self,
        conversation_id: &str,
        request: ConversationTurnRequest,
        progress: F,
    ) -> WorkspaceResult<Value>
    where
        F: Fn(Value) + Send + Sync + 'static,
    {
        let requested_project_path = normalize_project_path_or_400(&request.project_path)?;
        let before_revision = self
            .repository()
            .read_snapshot(conversation_id, Some(&requested_project_path))?
            .and_then(|snapshot| snapshot.get("revision").and_then(Value::as_i64))
            .unwrap_or(0);
        let (prepared, started_snapshot) = self.start_turn(conversation_id, request)?;
        for payload in
            self.read_events_after(conversation_id, &prepared.project_path, before_revision)?
        {
            progress(payload);
        }
        self.complete_started_turn_with_progress_payloads(prepared, started_snapshot, progress)
    }

    pub fn complete_started_turn_with_progress_payloads<F>(
        &self,
        prepared: PreparedConversationTurn,
        started_snapshot: Value,
        progress: F,
    ) -> WorkspaceResult<Value>
    where
        F: Fn(Value) + Send + Sync + 'static,
    {
        let progress = Arc::new(progress);
        let event_sink = live_conversation_turn_event_sink(
            started_snapshot,
            prepared.assistant_turn_id.clone(),
            prepared.chat_mode.clone(),
            self.repository(),
            prepared.conversation_id.clone(),
            prepared.project_path.clone(),
            progress,
        );
        match self
            .agent_turn_backend
            .run_turn_with_event_sink(prepared.agent_turn_request.clone(), Some(event_sink))
        {
            Ok(output) => self.ingest_agent_turn_output(
                &prepared.conversation_id,
                &prepared.project_path,
                &prepared.assistant_turn_id,
                &prepared.chat_mode,
                output,
            ),
            Err(error) => self.ingest_agent_turn_backend_failure(&prepared, error),
        }
    }

    pub fn submit_request_user_input_answer(
        &self,
        conversation_id: &str,
        request_id: &str,
        request: ConversationRequestUserInputAnswerRequest,
    ) -> WorkspaceResult<Value> {
        let project_path = normalize_project_path_or_400(&request.project_path)?;
        let lookup_id = non_empty_string(request_id)
            .ok_or_else(|| WorkspaceError::Validation("Request id is required.".to_string()))?;
        if request.answers.is_empty() {
            return Err(WorkspaceError::Validation(
                "Answers are required.".to_string(),
            ));
        }

        let repository = self.repository();
        let mut snapshot = repository
            .read_snapshot(conversation_id, Some(&project_path))?
            .ok_or_else(|| {
                WorkspaceError::NotFound(format!("Unknown conversation input request: {lookup_id}"))
            })?;
        prepare_snapshot_core_defaults(&mut snapshot, conversation_id, &project_path);
        let Some(segment_id) = find_request_user_input_segment_id(&snapshot, &lookup_id) else {
            return Err(WorkspaceError::NotFound(format!(
                "Unknown conversation input request: {lookup_id}"
            )));
        };
        let segment = find_segment(&snapshot, &segment_id)
            .cloned()
            .ok_or_else(|| {
                WorkspaceError::NotFound(format!("Unknown conversation input request: {lookup_id}"))
            })?;
        let mut request_record = segment
            .get("request_user_input")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let normalized_answers =
            normalize_request_user_input_answers(&request_record, &request.answers)?;
        let existing_answers = request_record
            .get("answers")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let status = request_record
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("pending");
        if matches!(status, "answered" | "expired") {
            if answers_equal(&existing_answers, &normalized_answers) {
                prepare_snapshot_for_ui(&mut snapshot, conversation_id);
                return Ok(snapshot);
            }
            let message = if status == "answered" {
                "That conversation request is already answered."
            } else {
                REQUEST_USER_INPUT_EXPIRED_ERROR
            };
            return Err(WorkspaceError::Conflict(message.to_string()));
        }

        let actual_request_id = request_record
            .get("request_id")
            .and_then(Value::as_str)
            .and_then(non_empty_string)
            .unwrap_or_else(|| lookup_id.clone());
        let assistant_turn_id = segment
            .get("turn_id")
            .and_then(Value::as_str)
            .and_then(non_empty_string)
            .ok_or_else(|| {
                WorkspaceError::Internal(
                    "Conversation input request is missing its assistant turn.".to_string(),
                )
            })?;
        let effective_chat_mode = normalize_chat_mode(
            snapshot
                .get("chat_mode")
                .and_then(Value::as_str)
                .unwrap_or("chat"),
        );
        let effective_provider = snapshot
            .get("provider")
            .and_then(Value::as_str)
            .map(validate_provider)
            .transpose()?
            .unwrap_or_else(|| "codex".to_string());
        let effective_model = snapshot
            .get("model")
            .and_then(Value::as_str)
            .and_then(non_empty_string);
        let effective_profile = snapshot
            .get("llm_profile")
            .and_then(Value::as_str)
            .and_then(non_empty_string);
        let effective_reasoning_effort = snapshot
            .get("reasoning_effort")
            .and_then(Value::as_str)
            .and_then(non_empty_string);
        let history = agent_history_from_snapshot(&snapshot, &[&assistant_turn_id]);
        let normalized_answer_strings = request_user_input_answer_strings(&normalized_answers);
        let submitted_at = iso_now();
        request_record.insert(
            "answers".to_string(),
            Value::Object(normalized_answers.clone()),
        );
        request_record.insert("submitted_at".to_string(), json!(submitted_at));
        request_record.insert("status".to_string(), json!("answered"));

        let updated_segment =
            answered_request_user_input_segment(&segment, &request_record, &submitted_at);
        upsert_segment(&mut snapshot, updated_segment.clone());

        let mut emitted_payloads = vec![build_segment_upsert_payload(&snapshot, &updated_segment)];
        persist_snapshot_with_payloads(
            &repository,
            conversation_id,
            &project_path,
            &mut snapshot,
            &mut emitted_payloads,
        )?;

        let answer_trace_path = if codex_jsonrpc_trace_enabled() {
            repository
                .conversation_codex_jsonrpc_trace_path(conversation_id, Some(&project_path))?
        } else {
            None
        };
        let answer_metadata = request_user_input_answer_metadata(
            &segment,
            &request_record,
            &assistant_turn_id,
            &segment_id,
            &lookup_id,
            &submitted_at,
            answer_trace_path.as_deref(),
        );

        let answer_request = AgentRequestUserInputAnswerRequest {
            conversation_id: conversation_id.to_string(),
            project_path: project_path.clone(),
            request_id: actual_request_id.clone(),
            assistant_turn_id: assistant_turn_id.clone(),
            answers: normalized_answer_strings,
            request_user_input: Some(Value::Object(request_record.clone())),
            history,
            provider: Some(effective_provider),
            model: effective_model,
            llm_profile: effective_profile,
            reasoning_effort: effective_reasoning_effort,
            chat_mode: Some(effective_chat_mode.clone()),
            metadata: answer_metadata,
        };

        let output = match self
            .agent_turn_backend
            .answer_request_user_input(answer_request)
        {
            Ok(output) if !request_user_input_answer_cannot_resume(&output) => output,
            Ok(_) => {
                let mut emitted_payloads = Vec::new();
                let expired_at = iso_now();
                expire_request_user_input_answer_in_snapshot(
                    &mut snapshot,
                    &segment_id,
                    request_record,
                    &expired_at,
                    &mut emitted_payloads,
                );
                persist_snapshot_with_payloads(
                    &repository,
                    conversation_id,
                    &project_path,
                    &mut snapshot,
                    &mut emitted_payloads,
                )?;
                prepare_snapshot_for_ui(&mut snapshot, conversation_id);
                return Ok(snapshot);
            }
            Err(error) => return Err(agent_turn_backend_error(error)),
        };

        if request_user_input_answer_delivered_to_live_request(&output) {
            prepare_snapshot_for_ui(&mut snapshot, conversation_id);
            return Ok(snapshot);
        }

        self.ingest_agent_turn_output(
            conversation_id,
            &project_path,
            &assistant_turn_id,
            &effective_chat_mode,
            output,
        )
    }

    pub fn create_flow_run_request_by_handle(
        &self,
        conversation_handle: &str,
        request: FlowRunRequestCreateByHandleRequest,
    ) -> WorkspaceResult<FlowRunRequestCreateResponse> {
        let repository = self.repository();
        let handle_match = repository
            .handle_repository()
            .find_conversation_by_handle(conversation_handle)?
            .ok_or_else(|| {
                WorkspaceError::NotFound(format!(
                    "Unknown conversation handle: {conversation_handle}. Verify the handle shown in the thread UI and try again."
                ))
            })?;
        let payload = normalize_flow_run_request_payload(request, "spark convo run-request")?;
        ensure_flow_exists(&self.settings, &payload.flow_name)?;

        let mut snapshot = repository
            .read_snapshot(
                &handle_match.conversation_id,
                Some(&handle_match.project_path),
            )?
            .ok_or_else(|| {
                WorkspaceError::NotFound(format!(
                    "Unknown conversation handle: {conversation_handle}. Verify the handle shown in the thread UI and try again."
                ))
            })?;
        prepare_snapshot_core_defaults(
            &mut snapshot,
            &handle_match.conversation_id,
            &handle_match.project_path,
        );
        let actual_project_path =
            snapshot_project_path(&snapshot).unwrap_or_else(|| handle_match.project_path.clone());
        if actual_project_path != handle_match.project_path {
            return Err(WorkspaceError::Validation(
                "Conversation is already bound to a different project path.".to_string(),
            ));
        }
        let parent_turn_id = latest_assistant_message_turn_id(&snapshot).ok_or_else(|| {
            WorkspaceError::Validation(
                "Conversation has no assistant turn that can own a flow run request.".to_string(),
            )
        })?;
        if duplicate_flow_run_request_exists(&snapshot, &parent_turn_id, &payload) {
            return Err(WorkspaceError::Conflict(
                "Flow run request was not created because an identical request already exists on the latest assistant turn."
                    .to_string(),
            ));
        }

        let now = iso_now();
        let request_id = random_artifact_id("flow-run-request");
        let segment_id = format!("segment-artifact-{request_id}");
        let mut request_record = json!({
            "id": request_id.clone(),
            "created_at": now.clone(),
            "updated_at": now.clone(),
            "flow_name": payload.flow_name.clone(),
            "summary": payload.summary.clone(),
            "project_path": handle_match.project_path.clone(),
            "conversation_id": handle_match.conversation_id.clone(),
            "source_turn_id": parent_turn_id.clone(),
            "status": "pending",
            "source_segment_id": segment_id,
        });
        set_optional_artifact_string(&mut request_record, "goal", payload.goal.as_deref());
        if let Some(launch_context) = payload.launch_context.as_ref() {
            set_value(
                &mut request_record,
                "launch_context",
                serde_json::to_value(launch_context).unwrap_or_else(|_| json!({})),
            );
        }
        set_optional_artifact_string(&mut request_record, "model", payload.model.as_deref());
        set_optional_artifact_string(
            &mut request_record,
            "llm_provider",
            payload.llm_provider.as_deref(),
        );
        set_optional_artifact_string(
            &mut request_record,
            "llm_profile",
            payload.llm_profile.as_deref(),
        );
        set_optional_artifact_string(
            &mut request_record,
            "reasoning_effort",
            payload.reasoning_effort.as_deref(),
        );
        set_optional_artifact_string(
            &mut request_record,
            "execution_profile_id",
            payload.execution_profile_id.as_deref(),
        );

        let request_segment = json!({
            "id": segment_id.clone(),
            "turn_id": parent_turn_id.clone(),
            "order": next_turn_segment_order(&snapshot, &parent_turn_id),
            "kind": "flow_run_request",
            "role": "system",
            "status": "complete",
            "timestamp": now.clone(),
            "updated_at": now.clone(),
            "content": "",
            "artifact_id": request_id.clone(),
            "source": {},
        });
        push_array_value(&mut snapshot, "flow_run_requests", request_record);
        upsert_segment(&mut snapshot, request_segment);
        append_workflow_event(
            &mut snapshot,
            json!({
                "message": format!("Created flow run request {request_id} for {}.", payload.flow_name),
                "timestamp": now,
            }),
        );
        touch_snapshot(
            &repository,
            &mut snapshot,
            &handle_match.conversation_id,
            &handle_match.project_path,
        )?;
        repository.write_snapshot(&snapshot)?;
        append_events(
            &repository,
            &handle_match.conversation_id,
            &handle_match.project_path,
            &[build_conversation_snapshot_payload(&snapshot)],
        )?;

        Ok(FlowRunRequestCreateResponse {
            ok: true,
            conversation_handle: handle_match.conversation_handle,
            conversation_id: handle_match.conversation_id,
            project_path: handle_match.project_path,
            turn_id: parent_turn_id,
            flow_run_request_id: request_id,
            segment_id,
        })
    }

    pub fn review_flow_run_request(
        &self,
        conversation_id: &str,
        request_id: &str,
        request: FlowRunRequestReviewRequest,
    ) -> WorkspaceResult<Value> {
        let project_path = normalize_project_path_or_400(&request.project_path)?;
        let disposition = normalize_review_disposition(
            &request.disposition,
            "Flow run request disposition must be approved or rejected.",
        )?;
        let review_message = non_empty_string(&request.message)
            .ok_or_else(|| WorkspaceError::Validation("Review message is required.".to_string()))?;
        let repository = self.repository();
        let mut snapshot = repository
            .read_snapshot(conversation_id, Some(&project_path))?
            .ok_or_else(|| {
                WorkspaceError::Validation("Conversation not found for project.".to_string())
            })?;
        prepare_snapshot_core_defaults(&mut snapshot, conversation_id, &project_path);
        let actual_project_path = snapshot_project_path(&snapshot).unwrap_or_default();
        if actual_project_path != project_path {
            return Err(WorkspaceError::Validation(
                "Conversation not found for project.".to_string(),
            ));
        }

        let request_index = artifact_index(&snapshot, "flow_run_requests", request_id)
            .ok_or_else(|| WorkspaceError::Validation("Unknown flow run request.".to_string()))?;
        let status = artifact_status(&snapshot, "flow_run_requests", request_index, "pending");
        if !matches!(status.as_str(), "pending" | "approved" | "launch_failed") {
            return Err(WorkspaceError::Validation(format!(
                "Flow run request is not reviewable in status '{status}'."
            )));
        }

        let now = iso_now();
        if disposition == "rejected" {
            update_artifact_at(
                &mut snapshot,
                "flow_run_requests",
                request_index,
                |artifact| {
                    set_string_value(artifact, "status", "rejected");
                    set_string_value(artifact, "review_message", &review_message);
                    set_string_value(artifact, "updated_at", &now);
                },
            );
            append_workflow_event(
                &mut snapshot,
                json!({
                    "message": format!("Rejected flow run request {request_id}."),
                    "timestamp": now,
                }),
            );
            touch_snapshot(&repository, &mut snapshot, conversation_id, &project_path)?;
            repository.write_snapshot(&snapshot)?;
            append_events(
                &repository,
                conversation_id,
                &project_path,
                &[build_conversation_snapshot_payload(&snapshot)],
            )?;
            prepare_snapshot_for_ui(&mut snapshot, conversation_id);
            return Ok(snapshot);
        }

        let approved_flow_name = request
            .flow_name
            .as_deref()
            .and_then(non_empty_string)
            .unwrap_or_else(|| {
                artifact_string(&snapshot, "flow_run_requests", request_index, "flow_name")
                    .unwrap_or_default()
            });
        ensure_flow_exists(&self.settings, &approved_flow_name)?;

        update_artifact_at(
            &mut snapshot,
            "flow_run_requests",
            request_index,
            |artifact| {
                set_string_value(artifact, "status", "approved");
                set_string_value(artifact, "review_message", &review_message);
                set_string_value(artifact, "updated_at", &now);
                set_string_value(artifact, "flow_name", &approved_flow_name);
                if let Some(model) = request.model.as_deref().and_then(non_empty_string) {
                    set_string_value(artifact, "model", &model);
                }
                if request.llm_provider.is_some() {
                    set_optional_artifact_string(
                        artifact,
                        "llm_provider",
                        request
                            .llm_provider
                            .as_deref()
                            .and_then(non_empty_string)
                            .map(|value| value.to_lowercase())
                            .as_deref(),
                    );
                }
                if request.llm_profile.is_some() {
                    set_optional_artifact_string(
                        artifact,
                        "llm_profile",
                        request.llm_profile.as_deref(),
                    );
                }
                if request.reasoning_effort.is_some() {
                    set_optional_artifact_string(
                        artifact,
                        "reasoning_effort",
                        request
                            .reasoning_effort
                            .as_deref()
                            .and_then(non_empty_string)
                            .map(|value| value.to_lowercase())
                            .as_deref(),
                    );
                }
                if request.execution_profile_id.is_some() {
                    set_optional_artifact_string(
                        artifact,
                        "execution_profile_id",
                        request.execution_profile_id.as_deref(),
                    );
                }
                remove_key(artifact, "launch_error");
            },
        );
        append_workflow_event(
            &mut snapshot,
            json!({
                "message": format!("Approved flow run request {request_id}."),
                "timestamp": now,
            }),
        );
        touch_snapshot(&repository, &mut snapshot, conversation_id, &project_path)?;
        repository.write_snapshot(&snapshot)?;
        append_events(
            &repository,
            conversation_id,
            &project_path,
            &[build_conversation_snapshot_payload(&snapshot)],
        )?;

        let launch_artifact =
            artifact_at(&snapshot, "flow_run_requests", request_index).unwrap_or_default();
        let launch_result =
            self.launch_workspace_flow(&project_path, &approved_flow_name, &launch_artifact);
        let now = iso_now();
        update_artifact_at(
            &mut snapshot,
            "flow_run_requests",
            request_index,
            |artifact| {
                set_string_value(artifact, "updated_at", &now);
                set_string_value(artifact, "flow_name", &approved_flow_name);
                match &launch_result {
                    Ok(run_id) => {
                        set_string_value(artifact, "status", "launched");
                        set_string_value(artifact, "run_id", run_id);
                        remove_key(artifact, "launch_error");
                    }
                    Err(error) => {
                        set_string_value(artifact, "status", "launch_failed");
                        set_string_value(artifact, "launch_error", error);
                        remove_key(artifact, "run_id");
                    }
                }
            },
        );
        append_workflow_event(
            &mut snapshot,
            match &launch_result {
                Ok(run_id) => json!({
                    "message": format!("Launched flow run request {request_id} as run {run_id} using {approved_flow_name}."),
                    "timestamp": now,
                }),
                Err(error) => json!({
                    "message": format!("Flow run request {request_id} failed to launch: {error}"),
                    "timestamp": now,
                }),
            },
        );
        touch_snapshot(&repository, &mut snapshot, conversation_id, &project_path)?;
        repository.write_snapshot(&snapshot)?;
        append_events(
            &repository,
            conversation_id,
            &project_path,
            &[build_conversation_snapshot_payload(&snapshot)],
        )?;
        prepare_snapshot_for_ui(&mut snapshot, conversation_id);
        Ok(snapshot)
    }

    pub fn review_proposed_plan(
        &self,
        conversation_id: &str,
        plan_id: &str,
        request: ProposedPlanReviewRequest,
    ) -> WorkspaceResult<Value> {
        let project_path = normalize_project_path_or_400(&request.project_path)?;
        let disposition = normalize_review_disposition(
            &request.disposition,
            "Proposed plan disposition must be approved or rejected.",
        )?;
        let repository = self.repository();
        let mut snapshot = repository
            .read_snapshot(conversation_id, Some(&project_path))?
            .ok_or_else(|| {
                WorkspaceError::Validation("Conversation not found for project.".to_string())
            })?;
        prepare_snapshot_core_defaults(&mut snapshot, conversation_id, &project_path);
        let actual_project_path = snapshot_project_path(&snapshot).unwrap_or_default();
        if actual_project_path != project_path {
            return Err(WorkspaceError::Validation(
                "Conversation not found for project.".to_string(),
            ));
        }
        let plan_index = artifact_index(&snapshot, "proposed_plans", plan_id).ok_or_else(|| {
            WorkspaceError::Validation("Unknown proposed plan artifact.".to_string())
        })?;
        let status = artifact_status(&snapshot, "proposed_plans", plan_index, "pending_review");
        if status != "pending_review" {
            return Err(WorkspaceError::Validation(format!(
                "Proposed plan is not reviewable in status '{status}'."
            )));
        }
        let review_note = request.review_note.as_deref().and_then(non_empty_string);
        let now = iso_now();

        if disposition == "rejected" {
            update_artifact_at(&mut snapshot, "proposed_plans", plan_index, |artifact| {
                set_string_value(artifact, "status", "rejected");
                set_optional_artifact_string(artifact, "review_note", review_note.as_deref());
                set_string_value(artifact, "updated_at", &now);
            });
            append_workflow_event(
                &mut snapshot,
                json!({
                    "message": format!("Rejected proposed plan {plan_id}."),
                    "timestamp": now,
                }),
            );
            touch_snapshot(&repository, &mut snapshot, conversation_id, &project_path)?;
            repository.write_snapshot(&snapshot)?;
            append_events(
                &repository,
                conversation_id,
                &project_path,
                &[build_conversation_snapshot_payload(&snapshot)],
            )?;
            prepare_snapshot_for_ui(&mut snapshot, conversation_id);
            return Ok(snapshot);
        }

        ensure_flow_exists(&self.settings, IMPLEMENT_CHANGE_REQUEST_FLOW)?;
        let project_root = PathBuf::from(&project_path);
        if !project_root.is_dir() {
            return Err(WorkspaceError::Validation(
                "Project path is not available for writing the approved plan.".to_string(),
            ));
        }
        let source_turn_id =
            artifact_string(&snapshot, "proposed_plans", plan_index, "source_turn_id").ok_or_else(
                || {
                    WorkspaceError::Validation(
                        "Proposed plan is missing its source turn.".to_string(),
                    )
                },
            )?;
        if find_turn(&snapshot, &source_turn_id).is_none() {
            return Err(WorkspaceError::Validation(
                "Proposed plan is missing its source turn.".to_string(),
            ));
        }
        let title = artifact_string(&snapshot, "proposed_plans", plan_index, "title")
            .unwrap_or_else(|| "Proposed Plan".to_string());
        let content = artifact_string(&snapshot, "proposed_plans", plan_index, "content")
            .ok_or_else(|| {
                WorkspaceError::Validation("Proposed plan content is required.".to_string())
            })?;
        let (change_request_id, request_path) =
            write_change_request(&project_root, &title, &content, &now)?;
        let relative_request_path = relative_project_path(&project_root, &request_path);
        let flow_launch_id = random_artifact_id("flow-launch");
        let flow_launch_segment_id = format!("segment-artifact-{flow_launch_id}");
        let launch_context = json!({
            "context.request.change_request_id": change_request_id.clone(),
            "context.request.change_request_path": relative_request_path.clone(),
        });
        let flow_launch = json!({
            "id": flow_launch_id.clone(),
            "created_at": now.clone(),
            "updated_at": now.clone(),
            "flow_name": IMPLEMENT_CHANGE_REQUEST_FLOW,
            "summary": format!("Implement approved change request: {title}"),
            "project_path": project_path.clone(),
            "conversation_id": conversation_id,
            "source_turn_id": source_turn_id.clone(),
            "status": "pending",
            "source_segment_id": flow_launch_segment_id.clone(),
            "goal": format!("Implement the approved change request written to {relative_request_path}."),
            "launch_context": launch_context.clone(),
        });
        let flow_launch_segment = json!({
            "id": flow_launch_segment_id,
            "turn_id": source_turn_id.clone(),
            "order": next_turn_segment_order(&snapshot, &source_turn_id),
            "kind": "flow_launch",
            "role": "system",
            "status": "complete",
            "timestamp": now.clone(),
            "updated_at": now.clone(),
            "content": "",
            "artifact_id": flow_launch_id.clone(),
            "source": {},
        });
        push_array_value(&mut snapshot, "flow_launches", flow_launch);
        upsert_segment(&mut snapshot, flow_launch_segment);
        update_artifact_at(&mut snapshot, "proposed_plans", plan_index, |artifact| {
            set_string_value(artifact, "status", "approved");
            set_optional_artifact_string(artifact, "review_note", review_note.as_deref());
            set_string_value(
                artifact,
                "written_change_request_path",
                &absolute_path_string(&request_path),
            );
            set_string_value(artifact, "flow_launch_id", &flow_launch_id);
            set_string_value(artifact, "updated_at", &iso_now());
            remove_key(artifact, "run_id");
            remove_key(artifact, "launch_error");
        });
        append_workflow_event(
            &mut snapshot,
            json!({
                "message": format!("Approved proposed plan {plan_id} and wrote {relative_request_path}."),
                "timestamp": now,
            }),
        );
        touch_snapshot(&repository, &mut snapshot, conversation_id, &project_path)?;
        repository.write_snapshot(&snapshot)?;
        append_events(
            &repository,
            conversation_id,
            &project_path,
            &[build_conversation_snapshot_payload(&snapshot)],
        )?;

        let launch_index =
            artifact_index(&snapshot, "flow_launches", &flow_launch_id).ok_or_else(|| {
                WorkspaceError::Internal("Flow launch artifact was not persisted.".to_string())
            })?;
        let launch_artifact =
            artifact_at(&snapshot, "flow_launches", launch_index).unwrap_or_default();
        let launch_result = self.launch_workspace_flow(
            &project_path,
            IMPLEMENT_CHANGE_REQUEST_FLOW,
            &launch_artifact,
        );
        let now = iso_now();
        update_artifact_at(&mut snapshot, "proposed_plans", plan_index, |artifact| {
            set_string_value(artifact, "updated_at", &now);
            match &launch_result {
                Ok(run_id) => {
                    set_string_value(artifact, "status", "approved");
                    set_string_value(artifact, "run_id", run_id);
                    remove_key(artifact, "launch_error");
                }
                Err(error) => {
                    set_string_value(artifact, "status", "launch_failed");
                    set_string_value(artifact, "launch_error", error);
                    remove_key(artifact, "run_id");
                }
            }
        });
        update_artifact_at(&mut snapshot, "flow_launches", launch_index, |artifact| {
            set_string_value(artifact, "updated_at", &now);
            set_string_value(artifact, "flow_name", IMPLEMENT_CHANGE_REQUEST_FLOW);
            match &launch_result {
                Ok(run_id) => {
                    set_string_value(artifact, "status", "launched");
                    set_string_value(artifact, "run_id", run_id);
                    remove_key(artifact, "launch_error");
                }
                Err(error) => {
                    set_string_value(artifact, "status", "launch_failed");
                    set_string_value(artifact, "launch_error", error);
                    remove_key(artifact, "run_id");
                }
            }
        });
        append_workflow_event(
            &mut snapshot,
            match &launch_result {
                Ok(run_id) => json!({
                    "message": format!("Launched proposed plan {plan_id} as run {run_id} using {IMPLEMENT_CHANGE_REQUEST_FLOW}."),
                    "timestamp": now,
                }),
                Err(error) => json!({
                    "message": format!("Approved proposed plan {plan_id} failed to launch {IMPLEMENT_CHANGE_REQUEST_FLOW}: {error}"),
                    "timestamp": now,
                }),
            },
        );
        touch_snapshot(&repository, &mut snapshot, conversation_id, &project_path)?;
        repository.write_snapshot(&snapshot)?;
        append_events(
            &repository,
            conversation_id,
            &project_path,
            &[build_conversation_snapshot_payload(&snapshot)],
        )?;
        prepare_snapshot_for_ui(&mut snapshot, conversation_id);
        Ok(snapshot)
    }

    pub fn launch_workspace_run(&self, request: RunLaunchRequest) -> WorkspaceResult<Value> {
        let conversation_handle = request
            .conversation_handle
            .as_deref()
            .and_then(non_empty_string);
        let explicit_project_path = request
            .project_path
            .as_deref()
            .and_then(non_empty_string)
            .map(|value| normalize_project_path_or_400(&value))
            .transpose()?;
        let payload = normalize_run_launch_request_payload(request)?;
        ensure_flow_exists(&self.settings, &payload.flow_name)?;

        let repository = self.repository();
        let mut launch_artifact: Option<FlowLaunchArtifactCreated> = None;
        let project_path = if let Some(handle) = conversation_handle.as_deref() {
            let handle_match = repository
                .handle_repository()
                .find_conversation_by_handle(handle)?
                .ok_or_else(|| {
                    WorkspaceError::NotFound(format!(
                        "Unknown conversation handle: {handle}. Verify the handle shown in the thread UI and try again."
                    ))
                })?;
            if let Some(explicit_project_path) = explicit_project_path.as_deref() {
                if explicit_project_path != handle_match.project_path {
                    return Err(WorkspaceError::Validation(
                        "Explicit --project path does not match the project bound to the conversation handle."
                            .to_string(),
                    ));
                }
            }
            launch_artifact = Some(self.create_flow_launch_artifact(
                &handle_match.conversation_id,
                &handle_match.project_path,
                &handle_match.conversation_handle,
                &payload,
            )?);
            handle_match.project_path
        } else {
            explicit_project_path.ok_or_else(|| {
                WorkspaceError::Validation(
                    "Project path is required when conversation_handle is omitted.".to_string(),
                )
            })?
        };

        let artifact_value = flow_launch_artifact_payload(&payload, &project_path);
        let launch_response = self.start_workspace_flow_route_response(
            &project_path,
            &payload.flow_name,
            &artifact_value,
        );
        let outcome = flow_start_outcome_from_response(&launch_response);
        if let Some(artifact) = launch_artifact.as_ref() {
            self.note_flow_launch_result(artifact, &payload.flow_name, outcome.as_ref())?;
        }
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                return Err(if launch_response.status_code >= 400 {
                    runtime_response_workspace_error(launch_response, "Flow launch failed")
                } else {
                    WorkspaceError::Internal(error.detail)
                });
            }
        };

        let mut response = serde_json::Map::from_iter([
            ("ok".to_string(), json!(true)),
            ("status".to_string(), json!(outcome.status.as_str())),
            ("flow_name".to_string(), json!(payload.flow_name)),
            ("project_path".to_string(), json!(project_path)),
        ]);
        response.insert("run_id".to_string(), json!(outcome.run_id));
        if let Some(handle) = conversation_handle {
            response.insert("conversation_handle".to_string(), json!(handle));
        }
        if let Some(artifact) = launch_artifact {
            response.insert(
                "conversation_id".to_string(),
                json!(artifact.conversation_id),
            );
            response.insert("flow_launch_id".to_string(), json!(artifact.flow_launch_id));
            response.insert("segment_id".to_string(), json!(artifact.segment_id));
            response.insert("turn_id".to_string(), json!(artifact.turn_id));
        }
        Ok(Value::Object(response))
    }

    pub fn launch_trigger_flow(
        &self,
        request: TriggerActivationRequest,
    ) -> WorkspaceResult<String> {
        let trigger_id = request.trigger_id.clone();
        let flow_name = request.action.flow_name.clone();
        ensure_flow_exists(&self.settings, &flow_name)?;
        let working_directory = request
            .action
            .project_path
            .clone()
            .unwrap_or_else(|| self.settings.data_dir.to_string_lossy().into_owned());
        let launch_context = json!({
            "context.trigger_static": Value::Object(request.action.static_context),
            "context.trigger_payload": request.source_payload,
            "context.spark_trigger": {
                "trigger_id": request.trigger_id,
                "trigger_name": request.trigger_name,
                "source_type": request.source_type,
            },
        });
        let artifact = json!({
            "flow_name": flow_name.clone(),
            "summary": format!("Trigger {trigger_id} fired {flow_name}."),
            "project_path": working_directory.clone(),
            "launch_context": launch_context,
        });
        self.launch_workspace_flow(&working_directory, &flow_name, &artifact)
            .map_err(WorkspaceError::Internal)
    }

    pub fn retry_workspace_run(
        &self,
        run_id: &str,
        request: RunRetryRequest,
    ) -> WorkspaceResult<Value> {
        let source_run_id = non_empty_string(run_id)
            .ok_or_else(|| WorkspaceError::Validation("Run id is required.".to_string()))?;
        let (selection, _project_override) =
            self.resolve_recovery_conversation(request.conversation_handle.as_deref(), None)?;
        let recovery = if let Some(selection) = selection.as_ref() {
            Some(self.create_run_recovery_artifact(
                selection,
                json!({
                    "operation": "retry",
                    "source_run_id": source_run_id,
                    "result_run_id": source_run_id,
                    "status": "pending",
                }),
            )?)
        } else {
            None
        };

        let route_response = self
            .runtime_api_service()
            .retry_pipeline_route(&source_run_id);
        if route_response.status_code >= 400 {
            if let (Some(selection), Some(recovery)) = (selection.as_ref(), recovery.as_ref()) {
                self.note_run_recovery_result(
                    selection,
                    recovery,
                    &source_run_id,
                    "failed",
                    runtime_response_detail(&route_response, "Retry failed").as_deref(),
                )?;
            }
            return Err(runtime_response_workspace_error(
                route_response,
                "Retry failed",
            ));
        }

        let status = route_response
            .body
            .get("status")
            .and_then(Value::as_str)
            .and_then(non_empty_string)
            .unwrap_or_else(|| "started".to_string());
        let result_run_id = route_response
            .body
            .get("run_id")
            .and_then(Value::as_str)
            .and_then(non_empty_string)
            .unwrap_or_else(|| source_run_id.clone());
        let recovery_error = route_response
            .body
            .get("error")
            .and_then(Value::as_str)
            .and_then(non_empty_string);
        let recovery_status = if status == "validation_error" {
            "failed"
        } else {
            status.as_str()
        };
        if let (Some(selection), Some(recovery)) = (selection.as_ref(), recovery.as_ref()) {
            self.note_run_recovery_result(
                selection,
                recovery,
                &result_run_id,
                recovery_status,
                recovery_error.as_deref(),
            )?;
        }

        let mut response = recovery_response_base(
            status != "validation_error" && recovery_error.is_none(),
            "retry",
            &source_run_id,
            &result_run_id,
            &status,
        );
        if let Some(selection) = selection.as_ref() {
            response.insert(
                "conversation_handle".to_string(),
                json!(selection.conversation_handle),
            );
            response.insert(
                "conversation_id".to_string(),
                json!(selection.conversation_id),
            );
        }
        if let Some(recovery) = recovery.as_ref() {
            response.insert(
                "run_recovery_id".to_string(),
                json!(recovery.run_recovery_id),
            );
            response.insert("segment_id".to_string(), json!(recovery.segment_id));
            response.insert("turn_id".to_string(), json!(recovery.turn_id));
        }
        if let Some(error) = recovery_error {
            response.insert("error".to_string(), json!(error));
        }
        Ok(Value::Object(response))
    }

    pub fn continue_workspace_run(
        &self,
        run_id: &str,
        request: RunContinueRequest,
    ) -> WorkspaceResult<Value> {
        let source_run_id = non_empty_string(run_id)
            .ok_or_else(|| WorkspaceError::Validation("Run id is required.".to_string()))?;
        let mode = request.flow_source_mode.trim().to_lowercase();
        if !matches!(mode.as_str(), "snapshot" | "flow_name") {
            return Err(WorkspaceError::Validation(
                "flow_source_mode must be either snapshot or flow_name.".to_string(),
            ));
        }
        let start_node = non_empty_string(&request.start_node)
            .ok_or_else(|| WorkspaceError::Validation("start_node is required.".to_string()))?;
        let flow_name = request.flow_name.as_deref().and_then(non_empty_string);
        let flow_name = if mode == "flow_name" {
            Some(flow_name.ok_or_else(|| {
                WorkspaceError::Validation(
                    "flow_name is required when flow_source_mode is flow_name.".to_string(),
                )
            })?)
        } else {
            None
        };
        let model = request.model.as_deref().and_then(non_empty_string);
        let llm_provider = request.llm_provider.as_deref().and_then(non_empty_string);
        let llm_profile = request.llm_profile.as_deref().and_then(non_empty_string);
        let reasoning_effort = request
            .reasoning_effort
            .as_deref()
            .and_then(non_empty_string);
        let (selection, project_override) = self.resolve_recovery_conversation(
            request.conversation_handle.as_deref(),
            request.project_path.as_deref(),
        )?;
        let recovery = if let Some(selection) = selection.as_ref() {
            Some(self.create_run_recovery_artifact(
                selection,
                json!({
                    "operation": "continue",
                    "source_run_id": source_run_id,
                    "result_run_id": "",
                    "status": "pending",
                    "start_node": start_node.clone(),
                    "flow_source_mode": mode.clone(),
                    "flow_name": flow_name.clone(),
                    "model": model.clone(),
                    "llm_provider": llm_provider.clone(),
                    "llm_profile": llm_profile.clone(),
                    "reasoning_effort": reasoning_effort.clone(),
                }),
            )?)
        } else {
            None
        };

        let route_response = self.runtime_api_service().continue_pipeline_route(
            &source_run_id,
            ContinuePipelineRequest {
                start_node: start_node.clone(),
                flow_source_mode: mode.clone(),
                flow_name: flow_name.clone(),
                working_directory: project_override.clone(),
                model,
                llm_provider,
                llm_profile,
                reasoning_effort,
            },
        );
        if route_response.status_code >= 400 {
            if let (Some(selection), Some(recovery)) = (selection.as_ref(), recovery.as_ref()) {
                self.note_run_recovery_result(
                    selection,
                    recovery,
                    "",
                    "failed",
                    runtime_response_detail(&route_response, "Continue failed").as_deref(),
                )?;
            }
            return Err(runtime_response_workspace_error(
                route_response,
                "Continue failed",
            ));
        }

        let status = route_response
            .body
            .get("status")
            .and_then(Value::as_str)
            .and_then(non_empty_string)
            .unwrap_or_else(|| "started".to_string());
        let result_run_id = route_response
            .body
            .get("run_id")
            .and_then(Value::as_str)
            .and_then(non_empty_string)
            .unwrap_or_default();
        let recovery_error = route_response
            .body
            .get("error")
            .and_then(Value::as_str)
            .and_then(non_empty_string);
        let recovery_status = if status == "validation_error" || recovery_error.is_some() {
            "failed"
        } else {
            status.as_str()
        };
        if let (Some(selection), Some(recovery)) = (selection.as_ref(), recovery.as_ref()) {
            self.note_run_recovery_result(
                selection,
                recovery,
                &result_run_id,
                recovery_status,
                recovery_error.as_deref(),
            )?;
        }

        let mut response = recovery_response_base(
            status != "validation_error" && recovery_error.is_none(),
            "continue",
            &source_run_id,
            &result_run_id,
            &status,
        );
        response.insert("start_node".to_string(), json!(start_node));
        response.insert("flow_source_mode".to_string(), json!(mode));
        response.insert("continued_from_run_id".to_string(), json!(source_run_id));
        if let Some(flow_name) = flow_name {
            response.insert("flow_name".to_string(), json!(flow_name));
        }
        if let Some(selection) = selection.as_ref() {
            response.insert(
                "conversation_handle".to_string(),
                json!(selection.conversation_handle),
            );
            response.insert(
                "conversation_id".to_string(),
                json!(selection.conversation_id),
            );
        }
        if let Some(recovery) = recovery.as_ref() {
            response.insert(
                "run_recovery_id".to_string(),
                json!(recovery.run_recovery_id),
            );
            response.insert("segment_id".to_string(), json!(recovery.segment_id));
            response.insert("turn_id".to_string(), json!(recovery.turn_id));
        }
        if let Some(error) = recovery_error {
            response.insert("error".to_string(), json!(error));
        }
        Ok(Value::Object(response))
    }

    pub fn read_events_after(
        &self,
        conversation_id: &str,
        project_path: &str,
        revision: i64,
    ) -> WorkspaceResult<Vec<Value>> {
        let project_path = normalize_project_path_or_400(project_path)?;
        self.repository()
            .read_conversation_events_after(conversation_id, &project_path, revision)
            .map_err(Into::into)
    }

    pub fn delete_conversation(
        &self,
        conversation_id: &str,
        project_path: &str,
    ) -> WorkspaceResult<ConversationDeleteResponse> {
        let project_path = normalize_project_path_or_400(project_path)?;
        let repository = self.repository();
        let snapshot = repository
            .read_snapshot(conversation_id, Some(&project_path))?
            .ok_or_else(|| {
                WorkspaceError::NotFound(format!("Unknown conversation: {conversation_id}"))
            })?;
        let actual_project_path = snapshot
            .get("project_path")
            .and_then(Value::as_str)
            .and_then(normalize_project_path_string)
            .unwrap_or_default();
        if actual_project_path != project_path {
            return Err(WorkspaceError::NotFound(format!(
                "Unknown conversation: {conversation_id}"
            )));
        }
        repository.delete_conversation(conversation_id, &project_path)?;
        Ok(ConversationDeleteResponse {
            status: "deleted",
            conversation_id: conversation_id.to_string(),
            project_path,
        })
    }

    pub fn get_segment_tool_output(
        &self,
        conversation_id: &str,
        segment_id: &str,
        project_path: Option<&str>,
    ) -> WorkspaceResult<Value> {
        let snapshot = self.get_snapshot_without_ui_truncation(conversation_id, project_path)?;
        let Some(segments) = snapshot.get("segments").and_then(Value::as_array) else {
            return Err(WorkspaceError::NotFound(
                "Unknown conversation segment tool output.".to_string(),
            ));
        };
        for segment in segments {
            let Some(segment_object) = segment.as_object() else {
                continue;
            };
            if segment_object.get("id").and_then(Value::as_str) != Some(segment_id) {
                continue;
            }
            let Some(output) = segment_object
                .get("tool_call")
                .and_then(Value::as_object)
                .and_then(|tool_call| tool_call.get("output"))
                .and_then(Value::as_str)
            else {
                break;
            };
            return Ok(json!({
                "output": output,
                "output_size": output.len(),
            }));
        }
        Err(WorkspaceError::NotFound(
            "Unknown conversation segment tool output.".to_string(),
        ))
    }

    pub fn deprecated_events_response(&self) -> &'static str {
        DEPRECATED_EVENTS_MESSAGE
    }

    fn get_snapshot_without_ui_truncation(
        &self,
        conversation_id: &str,
        project_path: Option<&str>,
    ) -> WorkspaceResult<Value> {
        let requested_project_path = normalize_optional_project_path(project_path)?;
        let repository = self.repository();
        let Some(mut snapshot) =
            repository.read_snapshot(conversation_id, requested_project_path.as_deref())?
        else {
            return Err(WorkspaceError::NotFound(format!(
                "Unknown conversation: {conversation_id}"
            )));
        };
        prepare_snapshot_core_defaults(
            &mut snapshot,
            conversation_id,
            requested_project_path.as_deref().unwrap_or(""),
        );
        Ok(snapshot)
    }

    fn create_flow_launch_artifact(
        &self,
        conversation_id: &str,
        project_path: &str,
        conversation_handle: &str,
        payload: &NormalizedFlowRunRequestPayload,
    ) -> WorkspaceResult<FlowLaunchArtifactCreated> {
        let repository = self.repository();
        let mut snapshot = repository
            .read_snapshot(conversation_id, Some(project_path))?
            .ok_or_else(|| {
                WorkspaceError::NotFound(format!("Unknown conversation: {conversation_id}"))
            })?;
        prepare_snapshot_core_defaults(&mut snapshot, conversation_id, project_path);
        let actual_project_path = snapshot_project_path(&snapshot).unwrap_or_default();
        if actual_project_path != project_path {
            return Err(WorkspaceError::Validation(
                "Conversation not found for project.".to_string(),
            ));
        }
        let parent_turn_id = latest_assistant_message_turn_id(&snapshot).ok_or_else(|| {
            WorkspaceError::Validation(
                "Conversation has no assistant turn that can own a flow launch.".to_string(),
            )
        })?;

        let now = iso_now();
        let flow_launch_id = random_artifact_id("flow-launch");
        let segment_id = format!("segment-artifact-{flow_launch_id}");
        let mut launch = json!({
            "id": flow_launch_id.clone(),
            "created_at": now.clone(),
            "updated_at": now.clone(),
            "flow_name": payload.flow_name.clone(),
            "summary": payload.summary.clone(),
            "project_path": project_path,
            "conversation_id": conversation_id,
            "source_turn_id": parent_turn_id.clone(),
            "status": "pending",
            "source_segment_id": segment_id.clone(),
        });
        copy_payload_options_to_artifact(&mut launch, payload);

        let segment = json!({
            "id": segment_id.clone(),
            "turn_id": parent_turn_id.clone(),
            "order": next_turn_segment_order(&snapshot, &parent_turn_id),
            "kind": "flow_launch",
            "role": "system",
            "status": "complete",
            "timestamp": now.clone(),
            "updated_at": now.clone(),
            "content": "",
            "artifact_id": flow_launch_id.clone(),
            "source": {},
        });
        push_array_value(&mut snapshot, "flow_launches", launch);
        upsert_segment(&mut snapshot, segment);
        append_workflow_event(
            &mut snapshot,
            json!({
                "message": format!("Created flow launch {flow_launch_id} for {}.", payload.flow_name),
                "timestamp": now,
            }),
        );
        touch_snapshot(&repository, &mut snapshot, conversation_id, project_path)?;
        repository.write_snapshot(&snapshot)?;
        append_events(
            &repository,
            conversation_id,
            project_path,
            &[build_conversation_snapshot_payload(&snapshot)],
        )?;
        Ok(FlowLaunchArtifactCreated {
            conversation_id: conversation_id.to_string(),
            project_path: project_path.to_string(),
            conversation_handle: conversation_handle.to_string(),
            flow_launch_id,
            segment_id,
            turn_id: parent_turn_id,
        })
    }

    fn note_flow_launch_result(
        &self,
        artifact: &FlowLaunchArtifactCreated,
        flow_name: &str,
        outcome: Result<&WorkspaceFlowStartOutcome, &WorkspaceFlowStartFailure>,
    ) -> WorkspaceResult<()> {
        let repository = self.repository();
        let mut snapshot = repository
            .read_snapshot(&artifact.conversation_id, Some(&artifact.project_path))?
            .ok_or_else(|| {
                WorkspaceError::NotFound(format!(
                    "Unknown conversation: {}",
                    artifact.conversation_id
                ))
            })?;
        prepare_snapshot_core_defaults(
            &mut snapshot,
            &artifact.conversation_id,
            &artifact.project_path,
        );
        let launch_index = artifact_index(&snapshot, "flow_launches", &artifact.flow_launch_id)
            .ok_or_else(|| WorkspaceError::Validation("Unknown flow launch.".to_string()))?;
        let now = iso_now();
        update_artifact_at(&mut snapshot, "flow_launches", launch_index, |entry| {
            set_string_value(entry, "updated_at", &now);
            set_string_value(entry, "flow_name", flow_name);
            match outcome {
                Ok(outcome) => {
                    set_string_value(entry, "status", "launched");
                    set_string_value(entry, "run_id", &outcome.run_id);
                    remove_key(entry, "launch_error");
                }
                Err(error) => {
                    set_string_value(entry, "status", "launch_failed");
                    set_string_value(entry, "launch_error", &error.detail);
                    remove_key(entry, "run_id");
                }
            }
        });
        append_workflow_event(
            &mut snapshot,
            match outcome {
                Ok(outcome) => json!({
                    "message": format!("Launched flow launch {} as run {} using {flow_name}.", artifact.flow_launch_id, outcome.run_id),
                    "timestamp": now,
                }),
                Err(error) => json!({
                    "message": format!("Flow launch {} failed to launch: {}", artifact.flow_launch_id, error.detail),
                    "timestamp": now,
                }),
            },
        );
        touch_snapshot(
            &repository,
            &mut snapshot,
            &artifact.conversation_id,
            &artifact.project_path,
        )?;
        repository.write_snapshot(&snapshot)?;
        append_events(
            &repository,
            &artifact.conversation_id,
            &artifact.project_path,
            &[build_conversation_snapshot_payload(&snapshot)],
        )?;
        Ok(())
    }

    fn resolve_recovery_conversation(
        &self,
        conversation_handle: Option<&str>,
        explicit_project_path: Option<&str>,
    ) -> WorkspaceResult<(Option<RecoveryConversationSelection>, Option<String>)> {
        let explicit_project_path = explicit_project_path
            .and_then(non_empty_string)
            .map(|value| normalize_project_path_or_400(&value))
            .transpose()?;
        let Some(handle) = conversation_handle.and_then(non_empty_string) else {
            return Ok((None, explicit_project_path));
        };
        let handle_match = self
            .repository()
            .handle_repository()
            .find_conversation_by_handle(&handle)?
            .ok_or_else(|| {
                WorkspaceError::NotFound(format!(
                    "Unknown conversation handle: {handle}. Verify the handle shown in the thread UI and try again."
                ))
            })?;
        if let Some(explicit_project_path) = explicit_project_path.as_deref() {
            if explicit_project_path != handle_match.project_path {
                return Err(WorkspaceError::Validation(
                    "Explicit --project path does not match the project bound to the conversation handle."
                        .to_string(),
                ));
            }
        }
        Ok((
            Some(RecoveryConversationSelection {
                conversation_id: handle_match.conversation_id,
                project_path: handle_match.project_path,
                conversation_handle: handle_match.conversation_handle,
            }),
            explicit_project_path,
        ))
    }

    fn create_run_recovery_artifact(
        &self,
        selection: &RecoveryConversationSelection,
        payload: Value,
    ) -> WorkspaceResult<RunRecoveryArtifactCreated> {
        let repository = self.repository();
        let mut snapshot = repository
            .read_snapshot(&selection.conversation_id, Some(&selection.project_path))?
            .ok_or_else(|| {
                WorkspaceError::NotFound(format!(
                    "Unknown conversation: {}",
                    selection.conversation_id
                ))
            })?;
        prepare_snapshot_core_defaults(
            &mut snapshot,
            &selection.conversation_id,
            &selection.project_path,
        );
        let actual_project_path = snapshot_project_path(&snapshot).unwrap_or_default();
        if actual_project_path != selection.project_path {
            return Err(WorkspaceError::Validation(
                "Conversation not found for project.".to_string(),
            ));
        }
        let parent_turn_id = latest_assistant_message_turn_id(&snapshot).ok_or_else(|| {
            WorkspaceError::Validation(
                "Conversation has no assistant turn that can own a run recovery.".to_string(),
            )
        })?;
        let now = iso_now();
        let run_recovery_id = random_artifact_id("run-recovery");
        let segment_id = format!("segment-artifact-{run_recovery_id}");
        let mut record = payload.as_object().cloned().unwrap_or_default();
        record.insert("id".to_string(), json!(run_recovery_id.clone()));
        record.insert("created_at".to_string(), json!(now.clone()));
        record.insert("updated_at".to_string(), json!(now.clone()));
        record.insert("project_path".to_string(), json!(selection.project_path));
        record.insert(
            "conversation_id".to_string(),
            json!(selection.conversation_id),
        );
        record.insert("source_turn_id".to_string(), json!(parent_turn_id.clone()));
        record.insert("source_segment_id".to_string(), json!(segment_id.clone()));
        let record = Value::Object(record);
        let segment = json!({
            "id": segment_id.clone(),
            "turn_id": parent_turn_id.clone(),
            "order": next_turn_segment_order(&snapshot, &parent_turn_id),
            "kind": "run_recovery",
            "role": "system",
            "status": "complete",
            "timestamp": now.clone(),
            "updated_at": now.clone(),
            "content": "",
            "artifact_id": run_recovery_id.clone(),
            "source": {},
        });
        push_array_value(&mut snapshot, "run_recoveries", record);
        upsert_segment(&mut snapshot, segment);
        append_workflow_event(
            &mut snapshot,
            json!({
                "message": format!("Created run recovery {run_recovery_id}."),
                "timestamp": now,
            }),
        );
        touch_snapshot(
            &repository,
            &mut snapshot,
            &selection.conversation_id,
            &selection.project_path,
        )?;
        repository.write_snapshot(&snapshot)?;
        append_events(
            &repository,
            &selection.conversation_id,
            &selection.project_path,
            &[build_conversation_snapshot_payload(&snapshot)],
        )?;
        Ok(RunRecoveryArtifactCreated {
            run_recovery_id,
            segment_id,
            turn_id: parent_turn_id,
        })
    }

    fn note_run_recovery_result(
        &self,
        selection: &RecoveryConversationSelection,
        recovery: &RunRecoveryArtifactCreated,
        result_run_id: &str,
        status: &str,
        recovery_error: Option<&str>,
    ) -> WorkspaceResult<()> {
        let repository = self.repository();
        let mut snapshot = repository
            .read_snapshot(&selection.conversation_id, Some(&selection.project_path))?
            .ok_or_else(|| {
                WorkspaceError::NotFound(format!(
                    "Unknown conversation: {}",
                    selection.conversation_id
                ))
            })?;
        prepare_snapshot_core_defaults(
            &mut snapshot,
            &selection.conversation_id,
            &selection.project_path,
        );
        let index = artifact_index(&snapshot, "run_recoveries", &recovery.run_recovery_id)
            .ok_or_else(|| WorkspaceError::Validation("Unknown run recovery.".to_string()))?;
        let now = iso_now();
        update_artifact_at(&mut snapshot, "run_recoveries", index, |entry| {
            set_string_value(entry, "updated_at", &now);
            set_string_value(entry, "status", status);
            set_string_value(entry, "result_run_id", result_run_id);
            set_optional_artifact_string(entry, "recovery_error", recovery_error);
        });
        append_workflow_event(
            &mut snapshot,
            json!({
                "message": format!("Updated run recovery {} to {status}.", recovery.run_recovery_id),
                "timestamp": now,
            }),
        );
        touch_snapshot(
            &repository,
            &mut snapshot,
            &selection.conversation_id,
            &selection.project_path,
        )?;
        repository.write_snapshot(&snapshot)?;
        append_events(
            &repository,
            &selection.conversation_id,
            &selection.project_path,
            &[build_conversation_snapshot_payload(&snapshot)],
        )?;
        Ok(())
    }

    fn start_workspace_flow_route_response(
        &self,
        project_path: &str,
        flow_name: &str,
        artifact: &Value,
    ) -> RuntimeRouteResponse {
        let execution_profile_id = artifact
            .get("execution_profile_id")
            .and_then(Value::as_str)
            .and_then(non_empty_string);
        let project_default_execution_profile_id = if execution_profile_id.is_none() {
            ProjectRegistry::new(self.settings.data_dir.clone())
                .read_project_record(project_path)
                .ok()
                .flatten()
                .and_then(|record| record.execution_profile_id)
        } else {
            None
        };
        self.runtime_api_service()
            .start_pipeline(PipelineStartRequest {
                run_id: None,
                flow_name: Some(flow_name.to_string()),
                working_directory: project_path.to_string(),
                model: artifact
                    .get("model")
                    .and_then(Value::as_str)
                    .and_then(non_empty_string),
                llm_provider: artifact
                    .get("llm_provider")
                    .and_then(Value::as_str)
                    .and_then(non_empty_string),
                llm_profile: artifact
                    .get("llm_profile")
                    .and_then(Value::as_str)
                    .and_then(non_empty_string),
                reasoning_effort: artifact
                    .get("reasoning_effort")
                    .and_then(Value::as_str)
                    .and_then(non_empty_string),
                execution_profile_id,
                project_default_execution_profile_id,
                goal: artifact
                    .get("goal")
                    .and_then(Value::as_str)
                    .and_then(non_empty_string),
                launch_context: artifact
                    .get("launch_context")
                    .and_then(Value::as_object)
                    .map(|object| {
                        object
                            .iter()
                            .map(|(key, value)| (key.clone(), value.clone()))
                            .collect()
                    }),
                ..PipelineStartRequest::default()
            })
    }

    fn launch_workspace_flow(
        &self,
        project_path: &str,
        flow_name: &str,
        artifact: &Value,
    ) -> Result<String, String> {
        let response = self.start_workspace_flow_route_response(project_path, flow_name, artifact);
        flow_start_outcome_from_response(&response)
            .map(|outcome| outcome.run_id)
            .map_err(|failure| failure.detail)
    }

    fn repository(&self) -> ConversationRepository {
        ConversationRepository::new(self.settings.data_dir.clone())
    }

    fn runtime_api_service(&self) -> attractor_api::AttractorApiService {
        attractor_api::AttractorApiService::new_with_runtime_handler_runner_factory(
            self.settings.clone(),
            self.runtime_handler_runner_factory.clone(),
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
struct NormalizedFlowRunRequestPayload {
    flow_name: String,
    summary: String,
    goal: Option<String>,
    launch_context: Option<BTreeMap<String, Value>>,
    model: Option<String>,
    llm_provider: Option<String>,
    llm_profile: Option<String>,
    reasoning_effort: Option<String>,
    execution_profile_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FlowLaunchArtifactCreated {
    conversation_id: String,
    project_path: String,
    conversation_handle: String,
    flow_launch_id: String,
    segment_id: String,
    turn_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecoveryConversationSelection {
    conversation_id: String,
    project_path: String,
    conversation_handle: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RunRecoveryArtifactCreated {
    run_recovery_id: String,
    segment_id: String,
    turn_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceFlowStartOutcome {
    run_id: String,
    status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceFlowStartFailure {
    status: String,
    detail: String,
}

fn normalize_flow_run_request_payload(
    request: FlowRunRequestCreateByHandleRequest,
    source_name: &str,
) -> WorkspaceResult<NormalizedFlowRunRequestPayload> {
    normalize_flow_run_payload_fields(
        &request.flow_name,
        &request.summary,
        request.goal.as_deref(),
        request.launch_context.as_ref(),
        request.model.as_deref(),
        request.llm_provider.as_deref(),
        request.llm_profile.as_deref(),
        request.reasoning_effort.as_deref(),
        request.execution_profile_id.as_deref(),
        source_name,
    )
}

fn normalize_run_launch_request_payload(
    request: RunLaunchRequest,
) -> WorkspaceResult<NormalizedFlowRunRequestPayload> {
    normalize_flow_run_payload_fields(
        &request.flow_name,
        &request.summary,
        request.goal.as_deref(),
        request.launch_context.as_ref(),
        request.model.as_deref(),
        request.llm_provider.as_deref(),
        request.llm_profile.as_deref(),
        request.reasoning_effort.as_deref(),
        request.execution_profile_id.as_deref(),
        "spark run launch",
    )
}

fn normalize_flow_run_payload_fields(
    flow_name: &str,
    summary: &str,
    goal: Option<&str>,
    launch_context: Option<&Value>,
    model: Option<&str>,
    llm_provider: Option<&str>,
    llm_profile: Option<&str>,
    reasoning_effort: Option<&str>,
    execution_profile_id: Option<&str>,
    source_name: &str,
) -> WorkspaceResult<NormalizedFlowRunRequestPayload> {
    let flow_name = non_empty_string(flow_name).ok_or_else(|| {
        WorkspaceError::Validation(format!("{source_name} requires a non-empty flow_name."))
    })?;
    let summary = non_empty_string(summary).ok_or_else(|| {
        WorkspaceError::Validation(format!("{source_name} requires a non-empty summary."))
    })?;
    Ok(NormalizedFlowRunRequestPayload {
        flow_name,
        summary,
        goal: goal.and_then(non_empty_string),
        launch_context: normalize_launch_context_value(launch_context, source_name)?,
        model: model.and_then(non_empty_string),
        llm_provider: llm_provider
            .and_then(non_empty_string)
            .map(|value| value.to_lowercase()),
        llm_profile: llm_profile.and_then(non_empty_string),
        reasoning_effort: reasoning_effort
            .and_then(non_empty_string)
            .map(|value| value.to_lowercase()),
        execution_profile_id: execution_profile_id.and_then(non_empty_string),
    })
}

fn shell_snapshot(conversation_id: &str, project_path: &str) -> Value {
    let now = iso_now();
    json!({
        "schema_version": CONVERSATION_STATE_SCHEMA_VERSION,
        "revision": 0,
        "conversation_id": conversation_id,
        "conversation_handle": "",
        "project_path": project_path,
        "chat_mode": "chat",
        "provider": "codex",
        "model": Value::Null,
        "llm_profile": Value::Null,
        "reasoning_effort": Value::Null,
        "title": "New thread",
        "created_at": now,
        "updated_at": now,
        "turns": [],
        "segments": [],
        "event_log": [],
        "flow_run_requests": [],
        "flow_launches": [],
        "run_recoveries": [],
        "proposed_plans": [],
    })
}

fn touch_snapshot(
    repository: &ConversationRepository,
    snapshot: &mut Value,
    conversation_id: &str,
    project_path: &str,
) -> WorkspaceResult<()> {
    prepare_snapshot_core_defaults(snapshot, conversation_id, project_path);
    let now = iso_now();
    let created_at = snapshot
        .get("created_at")
        .and_then(Value::as_str)
        .and_then(non_empty_string)
        .unwrap_or_else(|| now.clone());
    set_string(snapshot, "created_at", &created_at);
    if snapshot
        .get("title")
        .and_then(Value::as_str)
        .and_then(non_empty_string)
        .is_none()
    {
        let title = derive_conversation_title(snapshot.get("turns").and_then(Value::as_array));
        set_string(snapshot, "title", &title);
    }
    let project_paths = repository.project_paths(project_path)?;
    let preferred_handle = snapshot.get("conversation_handle").and_then(Value::as_str);
    let handle = repository.handle_repository().ensure_conversation_handle(
        conversation_id,
        &project_paths.project_id,
        project_path,
        &created_at,
        preferred_handle,
    )?;
    set_string(snapshot, "conversation_handle", &handle);
    set_string(snapshot, "project_path", project_path);
    set_string(snapshot, "updated_at", &now);
    let revision = snapshot
        .get("revision")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        + 1;
    if let Some(object) = snapshot.as_object_mut() {
        object.insert("revision".to_string(), json!(revision));
    }
    Ok(())
}

fn prepare_snapshot_for_ui(snapshot: &mut Value, conversation_id: &str) {
    let project_path = snapshot
        .get("project_path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    prepare_snapshot_core_defaults(snapshot, conversation_id, &project_path);
    truncate_tool_call_outputs(snapshot);
}

fn prepare_snapshot_core_defaults(snapshot: &mut Value, conversation_id: &str, project_path: &str) {
    let Some(object) = snapshot.as_object_mut() else {
        return;
    };
    object
        .entry("schema_version".to_string())
        .or_insert_with(|| json!(CONVERSATION_STATE_SCHEMA_VERSION));
    object
        .entry("revision".to_string())
        .or_insert_with(|| json!(0));
    object
        .entry("conversation_id".to_string())
        .or_insert_with(|| json!(conversation_id));
    object
        .entry("conversation_handle".to_string())
        .or_insert_with(|| json!(""));
    if !project_path.is_empty() {
        object
            .entry("project_path".to_string())
            .or_insert_with(|| json!(project_path));
    }
    object
        .entry("chat_mode".to_string())
        .or_insert_with(|| json!("chat"));
    object
        .entry("provider".to_string())
        .or_insert_with(|| json!("codex"));
    object.entry("model".to_string()).or_insert(Value::Null);
    object
        .entry("llm_profile".to_string())
        .or_insert(Value::Null);
    object
        .entry("reasoning_effort".to_string())
        .or_insert(Value::Null);
    object
        .entry("title".to_string())
        .or_insert_with(|| json!("New thread"));
    let created_at = object
        .get("created_at")
        .and_then(Value::as_str)
        .and_then(non_empty_string);
    if created_at.is_none() {
        object.insert("created_at".to_string(), json!(iso_now()));
    }
    let updated_at = object
        .get("updated_at")
        .and_then(Value::as_str)
        .and_then(non_empty_string);
    if updated_at.is_none() {
        let fallback = object
            .get("created_at")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        object.insert("updated_at".to_string(), json!(fallback));
    }
    for key in [
        "turns",
        "segments",
        "event_log",
        "flow_run_requests",
        "flow_launches",
        "run_recoveries",
        "proposed_plans",
    ] {
        object.entry(key.to_string()).or_insert_with(|| json!([]));
    }
    if object
        .get("title")
        .and_then(Value::as_str)
        .and_then(non_empty_string)
        .is_none()
    {
        let title = derive_conversation_title(object.get("turns").and_then(Value::as_array));
        object.insert("title".to_string(), json!(title));
    }
}

fn snapshot_project_path(snapshot: &Value) -> Option<String> {
    snapshot
        .get("project_path")
        .and_then(Value::as_str)
        .and_then(normalize_project_path_string)
}

fn push_turn(snapshot: &mut Value, turn: Value) {
    if let Some(turns) = snapshot
        .as_object_mut()
        .and_then(|object| object.get_mut("turns"))
        .and_then(Value::as_array_mut)
    {
        turns.push(turn);
    } else if let Some(object) = snapshot.as_object_mut() {
        object.insert("turns".to_string(), json!([turn]));
    }
}

fn push_array_value(snapshot: &mut Value, key: &str, value: Value) {
    if let Some(values) = snapshot
        .as_object_mut()
        .and_then(|object| object.get_mut(key))
        .and_then(Value::as_array_mut)
    {
        values.push(value);
    } else if let Some(object) = snapshot.as_object_mut() {
        object.insert(key.to_string(), json!([value]));
    }
}

fn maybe_set_title_from_message(snapshot: &mut Value, message: &str) {
    let current = snapshot
        .get("title")
        .and_then(Value::as_str)
        .and_then(non_empty_string);
    if current.as_deref().is_none() || current.as_deref() == Some("New thread") {
        set_string(snapshot, "title", &truncate_text(message, 64));
    }
}

fn active_assistant_turn_id(snapshot: &Value) -> Option<String> {
    snapshot
        .get("turns")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|turn| {
            let object = turn.as_object()?;
            if object.get("role").and_then(Value::as_str) != Some("assistant") {
                return None;
            }
            match object.get("status").and_then(Value::as_str) {
                Some("pending" | "streaming") => {
                    object.get("id").and_then(Value::as_str).map(str::to_string)
                }
                _ => None,
            }
        })
}

fn find_turn<'a>(snapshot: &'a Value, turn_id: &str) -> Option<&'a Value> {
    snapshot
        .get("turns")
        .and_then(Value::as_array)?
        .iter()
        .find(|turn| turn.get("id").and_then(Value::as_str) == Some(turn_id))
}

fn latest_assistant_message_turn_id(snapshot: &Value) -> Option<String> {
    snapshot
        .get("turns")
        .and_then(Value::as_array)?
        .iter()
        .rev()
        .find(|turn| {
            turn.get("role").and_then(Value::as_str) == Some("assistant")
                && turn
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("message")
                    == "message"
                && turn
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("complete")
                    == "complete"
        })
        .and_then(|turn| turn.get("id").and_then(Value::as_str))
        .map(str::to_string)
}

fn find_turn_mut<'a>(snapshot: &'a mut Value, turn_id: &str) -> Option<&'a mut Value> {
    snapshot
        .as_object_mut()
        .and_then(|object| object.get_mut("turns"))
        .and_then(Value::as_array_mut)?
        .iter_mut()
        .find(|turn| turn.get("id").and_then(Value::as_str) == Some(turn_id))
}

fn find_segment<'a>(snapshot: &'a Value, segment_id: &str) -> Option<&'a Value> {
    snapshot
        .get("segments")
        .and_then(Value::as_array)?
        .iter()
        .find(|segment| segment.get("id").and_then(Value::as_str) == Some(segment_id))
}

fn upsert_segment(snapshot: &mut Value, segment: Value) {
    let segment_id = segment.get("id").and_then(Value::as_str).unwrap_or("");
    if segment_id.is_empty() {
        return;
    }
    if let Some(segments) = snapshot
        .as_object_mut()
        .and_then(|object| object.get_mut("segments"))
        .and_then(Value::as_array_mut)
    {
        if let Some(existing) = segments
            .iter_mut()
            .find(|candidate| candidate.get("id").and_then(Value::as_str) == Some(segment_id))
        {
            *existing = segment;
        } else {
            segments.push(segment);
        }
    } else if let Some(object) = snapshot.as_object_mut() {
        object.insert("segments".to_string(), json!([segment]));
    }
}

fn next_turn_segment_order(snapshot: &Value, turn_id: &str) -> i64 {
    snapshot
        .get("segments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|segment| segment.get("turn_id").and_then(Value::as_str) == Some(turn_id))
        .filter_map(|segment| segment.get("order").and_then(Value::as_i64))
        .max()
        .unwrap_or(0)
        + 1
}

fn artifact_index(snapshot: &Value, key: &str, artifact_id: &str) -> Option<usize> {
    snapshot
        .get(key)
        .and_then(Value::as_array)?
        .iter()
        .position(|artifact| artifact.get("id").and_then(Value::as_str) == Some(artifact_id))
}

fn artifact_at(snapshot: &Value, key: &str, index: usize) -> Option<Value> {
    snapshot
        .get(key)
        .and_then(Value::as_array)
        .and_then(|artifacts| artifacts.get(index))
        .cloned()
}

fn artifact_string(snapshot: &Value, key: &str, index: usize, field: &str) -> Option<String> {
    snapshot
        .get(key)
        .and_then(Value::as_array)
        .and_then(|artifacts| artifacts.get(index))
        .and_then(|artifact| artifact.get(field))
        .and_then(Value::as_str)
        .and_then(non_empty_string)
}

fn artifact_status(snapshot: &Value, key: &str, index: usize, default: &str) -> String {
    artifact_string(snapshot, key, index, "status").unwrap_or_else(|| default.to_string())
}

fn update_artifact_at(
    snapshot: &mut Value,
    key: &str,
    index: usize,
    update: impl FnOnce(&mut Value),
) {
    if let Some(artifact) = snapshot
        .as_object_mut()
        .and_then(|object| object.get_mut(key))
        .and_then(Value::as_array_mut)
        .and_then(|artifacts| artifacts.get_mut(index))
    {
        update(artifact);
    }
}

fn duplicate_flow_run_request_exists(
    snapshot: &Value,
    turn_id: &str,
    payload: &NormalizedFlowRunRequestPayload,
) -> bool {
    let Some(segments) = snapshot.get("segments").and_then(Value::as_array) else {
        return false;
    };
    let Some(requests) = snapshot.get("flow_run_requests").and_then(Value::as_array) else {
        return false;
    };
    segments
        .iter()
        .filter(|segment| {
            segment.get("turn_id").and_then(Value::as_str) == Some(turn_id)
                && segment.get("kind").and_then(Value::as_str) == Some("flow_run_request")
        })
        .filter_map(|segment| segment.get("artifact_id").and_then(Value::as_str))
        .any(|artifact_id| {
            requests
                .iter()
                .find(|request| request.get("id").and_then(Value::as_str) == Some(artifact_id))
                .map(|request| flow_run_request_matches_payload(request, payload))
                .unwrap_or(false)
        })
}

fn flow_run_request_matches_payload(
    request: &Value,
    payload: &NormalizedFlowRunRequestPayload,
) -> bool {
    request.get("flow_name").and_then(Value::as_str) == Some(payload.flow_name.as_str())
        && request.get("summary").and_then(Value::as_str) == Some(payload.summary.as_str())
        && optional_artifact_string(request, "goal") == payload.goal
        && optional_artifact_string(request, "model") == payload.model
        && optional_artifact_string(request, "llm_provider") == payload.llm_provider
        && optional_artifact_string(request, "llm_profile") == payload.llm_profile
        && optional_artifact_string(request, "reasoning_effort") == payload.reasoning_effort
        && optional_artifact_string(request, "execution_profile_id") == payload.execution_profile_id
        && normalized_context_object(request.get("launch_context")) == payload.launch_context
}

fn optional_artifact_string(request: &Value, key: &str) -> Option<String> {
    request
        .get(key)
        .and_then(Value::as_str)
        .and_then(non_empty_string)
}

fn copy_payload_options_to_artifact(target: &mut Value, payload: &NormalizedFlowRunRequestPayload) {
    set_optional_artifact_string(target, "goal", payload.goal.as_deref());
    if let Some(launch_context) = payload.launch_context.as_ref() {
        set_value(
            target,
            "launch_context",
            Value::Object(Map::from_iter(
                launch_context
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            )),
        );
    } else {
        remove_key(target, "launch_context");
    }
    set_optional_artifact_string(target, "model", payload.model.as_deref());
    set_optional_artifact_string(target, "llm_provider", payload.llm_provider.as_deref());
    set_optional_artifact_string(target, "llm_profile", payload.llm_profile.as_deref());
    set_optional_artifact_string(
        target,
        "reasoning_effort",
        payload.reasoning_effort.as_deref(),
    );
    set_optional_artifact_string(
        target,
        "execution_profile_id",
        payload.execution_profile_id.as_deref(),
    );
}

fn flow_launch_artifact_payload(
    payload: &NormalizedFlowRunRequestPayload,
    project_path: &str,
) -> Value {
    let mut artifact = json!({
        "flow_name": payload.flow_name,
        "summary": payload.summary,
        "project_path": project_path,
    });
    copy_payload_options_to_artifact(&mut artifact, payload);
    artifact
}

fn flow_start_outcome_from_response(
    response: &RuntimeRouteResponse,
) -> Result<WorkspaceFlowStartOutcome, WorkspaceFlowStartFailure> {
    if response.status_code >= 400 {
        return Err(WorkspaceFlowStartFailure {
            status: "failed".to_string(),
            detail: runtime_response_detail(response, "Flow launch failed")
                .unwrap_or_else(|| "Flow launch failed.".to_string()),
        });
    }
    let status = response
        .body
        .get("status")
        .and_then(Value::as_str)
        .and_then(non_empty_string)
        .unwrap_or_else(|| "failed".to_string());
    if !matches!(status.as_str(), "started" | "queued") {
        return Err(WorkspaceFlowStartFailure {
            status,
            detail: runtime_response_detail(response, "Flow run could not be started")
                .unwrap_or_else(|| "Flow run could not be started.".to_string()),
        });
    }
    let Some(run_id) = response
        .body
        .get("run_id")
        .and_then(Value::as_str)
        .and_then(non_empty_string)
    else {
        return Err(WorkspaceFlowStartFailure {
            status,
            detail: "Flow run did not return a run id.".to_string(),
        });
    };
    Ok(WorkspaceFlowStartOutcome { run_id, status })
}

fn runtime_response_detail(response: &RuntimeRouteResponse, fallback: &str) -> Option<String> {
    response
        .body
        .get("detail")
        .and_then(|detail| {
            detail.as_str().map(str::to_string).or_else(|| {
                detail
                    .get("error")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
        })
        .or_else(|| {
            response
                .body
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            if fallback.is_empty() {
                None
            } else {
                Some(fallback.to_string())
            }
        })
}

fn runtime_response_workspace_error(
    response: RuntimeRouteResponse,
    fallback: &str,
) -> WorkspaceError {
    let detail =
        runtime_response_detail(&response, fallback).unwrap_or_else(|| fallback.to_string());
    match response.status_code {
        400 | 422 => WorkspaceError::Validation(detail),
        403 => WorkspaceError::Forbidden(detail),
        404 => WorkspaceError::NotFound(detail),
        409 => WorkspaceError::Conflict(detail),
        503 => WorkspaceError::ServiceUnavailable(detail),
        _ => WorkspaceError::Internal(detail),
    }
}

fn recovery_response_base(
    ok: bool,
    operation: &str,
    source_run_id: &str,
    result_run_id: &str,
    status: &str,
) -> Map<String, Value> {
    Map::from_iter([
        ("ok".to_string(), json!(ok)),
        ("operation".to_string(), json!(operation)),
        ("source_run_id".to_string(), json!(source_run_id)),
        ("run_id".to_string(), json!(result_run_id)),
        ("status".to_string(), json!(status)),
    ])
}

fn normalized_context_object(value: Option<&Value>) -> Option<BTreeMap<String, Value>> {
    let object = value.and_then(Value::as_object)?;
    if object.is_empty() {
        return None;
    }
    Some(
        object
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    )
}

fn set_value(target: &mut Value, key: &str, value: Value) {
    if let Some(object) = target.as_object_mut() {
        object.insert(key.to_string(), value);
    }
}

fn set_string_value(target: &mut Value, key: &str, value: &str) {
    set_value(target, key, json!(value));
}

fn set_optional_artifact_string(target: &mut Value, key: &str, value: Option<&str>) {
    if let Some(value) = value.and_then(non_empty_string) {
        set_string_value(target, key, &value);
    } else {
        remove_key(target, key);
    }
}

fn remove_key(target: &mut Value, key: &str) {
    if let Some(object) = target.as_object_mut() {
        object.remove(key);
    }
}

fn ensure_assistant_streaming(
    snapshot: &mut Value,
    assistant_turn_id: &str,
    emitted_payloads: &mut Vec<Value>,
) {
    let Some(turn) = find_turn_mut(snapshot, assistant_turn_id) else {
        return;
    };
    if turn.get("status").and_then(Value::as_str) != Some("pending") {
        return;
    }
    set_string_value(turn, "status", "streaming");
    let turn = turn.clone();
    emitted_payloads.push(build_turn_upsert_payload(snapshot, &turn));
}

fn live_conversation_turn_event_sink(
    snapshot: Value,
    assistant_turn_id: String,
    chat_mode: String,
    repository: ConversationRepository,
    conversation_id: String,
    project_path: String,
    progress: Arc<dyn Fn(Value) + Send + Sync + 'static>,
) -> AgentTurnEventSink {
    let state = Arc::new(Mutex::new(LiveConversationTurnState {
        snapshot,
        assistant_turn_id,
        chat_mode,
        repository,
        conversation_id,
        project_path,
        progress,
    }));
    Arc::new(move |event| {
        if let Ok(mut state) = state.lock() {
            state.ingest_event(event);
        }
    })
}

struct LiveConversationTurnState {
    snapshot: Value,
    assistant_turn_id: String,
    chat_mode: String,
    repository: ConversationRepository,
    conversation_id: String,
    project_path: String,
    progress: Arc<dyn Fn(Value) + Send + Sync + 'static>,
}

impl LiveConversationTurnState {
    fn ingest_event(&mut self, event: TurnStreamEvent) {
        let mut emitted_payloads = Vec::new();
        if apply_assistant_turn_app_server_ids(
            &mut self.snapshot,
            &self.assistant_turn_id,
            event.source.app_thread_id.as_deref(),
            event.source.app_turn_id.as_deref(),
        ) {
            emit_assistant_turn_upsert(
                &self.snapshot,
                &self.assistant_turn_id,
                &mut emitted_payloads,
            );
        }
        ensure_assistant_streaming(
            &mut self.snapshot,
            &self.assistant_turn_id,
            &mut emitted_payloads,
        );
        match &event.kind {
            TurnStreamEventKind::TokenUsageUpdated => {
                if let Some(token_usage) = event.token_usage.clone() {
                    if let Some(turn) = find_turn_mut(&mut self.snapshot, &self.assistant_turn_id) {
                        set_value(turn, "token_usage", token_usage);
                        let turn = turn.clone();
                        emitted_payloads.push(build_turn_upsert_payload(&self.snapshot, &turn));
                    }
                }
            }
            TurnStreamEventKind::ContentDelta
                if event.channel == Some(TurnStreamChannel::Assistant)
                    && self.chat_mode != "plan"
                    && is_final_answer_phase(event.phase.as_deref()) =>
            {
                if let Some(delta) = event.content_delta.as_deref() {
                    if let Some(turn) = find_turn_mut(&mut self.snapshot, &self.assistant_turn_id) {
                        append_turn_content(turn, delta);
                        set_string_value(turn, "status", "streaming");
                        remove_key(turn, "error");
                        let turn = turn.clone();
                        emitted_payloads.push(build_turn_upsert_payload(&self.snapshot, &turn));
                    }
                }
                self.emit_materialized_segment(&event, &mut emitted_payloads);
            }
            TurnStreamEventKind::ContentCompleted
                if event.channel == Some(TurnStreamChannel::Assistant) =>
            {
                if self.chat_mode == "plan" && is_final_answer_phase(event.phase.as_deref()) {
                    // Plan-mode final answer is applied by the durable finalizer so the plan
                    // artifact remains the primary live surface during streaming.
                } else {
                    if self.chat_mode != "plan"
                        && is_final_answer_phase(event.phase.as_deref())
                        && event_text(&event).is_some()
                    {
                        if let Some(turn) =
                            find_turn_mut(&mut self.snapshot, &self.assistant_turn_id)
                        {
                            set_string_value(
                                turn,
                                "content",
                                event_text(&event).as_deref().unwrap_or(""),
                            );
                            set_string_value(turn, "status", "streaming");
                            remove_key(turn, "error");
                            let turn = turn.clone();
                            emitted_payloads.push(build_turn_upsert_payload(&self.snapshot, &turn));
                        }
                    }
                    self.emit_materialized_segment(&event, &mut emitted_payloads);
                }
            }
            TurnStreamEventKind::Error => {
                let message = event
                    .error
                    .clone()
                    .or_else(|| event.message.clone())
                    .unwrap_or_else(|| "Conversation turn failed.".to_string());
                if let Some(turn) = find_turn_mut(&mut self.snapshot, &self.assistant_turn_id) {
                    set_string_value(turn, "status", "failed");
                    set_string_value(turn, "error", &message);
                    if let Some(error_code) = event.error_code.as_deref().and_then(non_empty_string)
                    {
                        set_string_value(turn, "error_code", &error_code);
                    }
                    if let Some(details) = event.details.as_ref() {
                        set_value(turn, "details", details.clone());
                    }
                    let turn = turn.clone();
                    emitted_payloads.push(build_turn_upsert_payload(&self.snapshot, &turn));
                }
                self.emit_materialized_segment(&event, &mut emitted_payloads);
            }
            _ => self.emit_materialized_segment(&event, &mut emitted_payloads),
        }
        if event.kind == TurnStreamEventKind::RequestUserInputRequested
            && persist_snapshot_with_payloads(
                &self.repository,
                &self.conversation_id,
                &self.project_path,
                &mut self.snapshot,
                &mut emitted_payloads,
            )
            .is_err()
        {
            emitted_payloads.clear();
        }
        for payload in emitted_payloads {
            (self.progress)(payload);
        }
    }

    fn emit_materialized_segment(
        &mut self,
        event: &TurnStreamEvent,
        emitted_payloads: &mut Vec<Value>,
    ) {
        if let Some(segment) =
            materialize_segment_for_event(&mut self.snapshot, &self.assistant_turn_id, event)
        {
            emitted_payloads.push(build_segment_upsert_payload(&self.snapshot, &segment));
        }
    }
}

fn append_turn_content(turn: &mut Value, delta: &str) {
    let mut content = turn
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    content.push_str(delta);
    set_string_value(turn, "content", &content);
}

fn event_text(event: &TurnStreamEvent) -> Option<String> {
    event
        .content_delta
        .as_deref()
        .and_then(non_empty_string)
        .or_else(|| event.message.as_deref().and_then(non_empty_string))
}

fn is_final_answer_phase(phase: Option<&str>) -> bool {
    match phase
        .and_then(non_empty_string)
        .map(|value| value.to_lowercase())
    {
        None => true,
        Some(value) => value == "final_answer",
    }
}

fn materialize_segment_for_event(
    snapshot: &mut Value,
    assistant_turn_id: &str,
    event: &TurnStreamEvent,
) -> Option<Value> {
    let now = iso_now();
    match &event.kind {
        TurnStreamEventKind::ContentDelta
            if event.channel == Some(TurnStreamChannel::Reasoning) =>
        {
            let segment_id = reasoning_segment_id(assistant_turn_id, event);
            let mut segment = find_segment(snapshot, &segment_id)
                .cloned()
                .unwrap_or_else(|| {
                    segment_shell(
                        &segment_id,
                        assistant_turn_id,
                        next_turn_segment_order(snapshot, assistant_turn_id),
                        "reasoning",
                        "assistant",
                        "streaming",
                        &now,
                        build_segment_source(event, None),
                    )
                });
            append_segment_content(&mut segment, event.content_delta.as_deref().unwrap_or(""));
            set_string_value(&mut segment, "status", "streaming");
            set_string_value(&mut segment, "updated_at", &now);
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        TurnStreamEventKind::ContentDelta
            if event.channel == Some(TurnStreamChannel::Assistant) =>
        {
            let segment_id = assistant_segment_id(assistant_turn_id, event);
            let mut segment = find_segment(snapshot, &segment_id)
                .cloned()
                .unwrap_or_else(|| {
                    let mut segment = segment_shell(
                        &segment_id,
                        assistant_turn_id,
                        next_turn_segment_order(snapshot, assistant_turn_id),
                        "assistant_message",
                        "assistant",
                        "streaming",
                        &now,
                        build_segment_source(event, None),
                    );
                    if let Some(phase) = normalized_assistant_phase(event.phase.as_deref()) {
                        set_string_value(&mut segment, "phase", &phase);
                    }
                    segment
                });
            append_segment_content(&mut segment, event.content_delta.as_deref().unwrap_or(""));
            set_string_value(&mut segment, "status", "streaming");
            set_string_value(&mut segment, "updated_at", &now);
            remove_key(&mut segment, "error");
            remove_key(&mut segment, "error_code");
            remove_key(&mut segment, "details");
            if let Some(phase) = normalized_assistant_phase(event.phase.as_deref()) {
                set_string_value(&mut segment, "phase", &phase);
            }
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        TurnStreamEventKind::ContentDelta if event.channel == Some(TurnStreamChannel::Plan) => {
            let segment_id = plan_segment_id(assistant_turn_id, event);
            let mut segment = find_segment(snapshot, &segment_id)
                .cloned()
                .unwrap_or_else(|| {
                    segment_shell(
                        &segment_id,
                        assistant_turn_id,
                        next_turn_segment_order(snapshot, assistant_turn_id),
                        "plan",
                        "assistant",
                        "streaming",
                        &now,
                        build_segment_source(event, None),
                    )
                });
            append_segment_content(&mut segment, event.content_delta.as_deref().unwrap_or(""));
            set_string_value(&mut segment, "status", "streaming");
            set_string_value(&mut segment, "updated_at", &now);
            remove_key(&mut segment, "error");
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        TurnStreamEventKind::ContentCompleted
            if event.channel == Some(TurnStreamChannel::Assistant) =>
        {
            let segment_id = assistant_segment_id(assistant_turn_id, event);
            let mut segment = find_segment(snapshot, &segment_id)
                .cloned()
                .unwrap_or_else(|| {
                    segment_shell(
                        &segment_id,
                        assistant_turn_id,
                        next_turn_segment_order(snapshot, assistant_turn_id),
                        "assistant_message",
                        "assistant",
                        "complete",
                        &now,
                        build_segment_source(event, None),
                    )
                });
            if let Some(text) = event_text(event) {
                set_string_value(&mut segment, "content", &text);
            } else if let Some(turn_content) = find_turn(snapshot, assistant_turn_id)
                .and_then(|turn| turn.get("content"))
                .and_then(Value::as_str)
                .and_then(non_empty_string)
            {
                set_string_value(&mut segment, "content", &turn_content);
            }
            set_string_value(&mut segment, "status", "complete");
            set_string_value(&mut segment, "updated_at", &now);
            set_string_value(&mut segment, "completed_at", &now);
            remove_key(&mut segment, "error");
            remove_key(&mut segment, "error_code");
            remove_key(&mut segment, "details");
            if let Some(phase) = normalized_assistant_phase(event.phase.as_deref()) {
                set_string_value(&mut segment, "phase", &phase);
            }
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        TurnStreamEventKind::ContentCompleted
            if event.channel == Some(TurnStreamChannel::Reasoning) =>
        {
            let segment_id = reasoning_segment_id(assistant_turn_id, event);
            let mut segment = find_segment(snapshot, &segment_id)
                .cloned()
                .unwrap_or_else(|| {
                    segment_shell(
                        &segment_id,
                        assistant_turn_id,
                        next_turn_segment_order(snapshot, assistant_turn_id),
                        "reasoning",
                        "assistant",
                        "complete",
                        &now,
                        build_segment_source(event, None),
                    )
                });
            if let Some(text) = event_text(event) {
                set_string_value(&mut segment, "content", &text);
            }
            set_string_value(&mut segment, "status", "complete");
            set_string_value(&mut segment, "updated_at", &now);
            set_string_value(&mut segment, "completed_at", &now);
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        TurnStreamEventKind::ContentCompleted if event.channel == Some(TurnStreamChannel::Plan) => {
            let segment_id = plan_segment_id(assistant_turn_id, event);
            let mut segment = find_segment(snapshot, &segment_id)
                .cloned()
                .unwrap_or_else(|| {
                    segment_shell(
                        &segment_id,
                        assistant_turn_id,
                        next_turn_segment_order(snapshot, assistant_turn_id),
                        "plan",
                        "assistant",
                        "complete",
                        &now,
                        build_segment_source(event, None),
                    )
                });
            if let Some(text) = event_text(event) {
                set_string_value(&mut segment, "content", &text);
            }
            set_string_value(&mut segment, "status", "complete");
            set_string_value(&mut segment, "updated_at", &now);
            set_string_value(&mut segment, "completed_at", &now);
            remove_key(&mut segment, "error");
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        TurnStreamEventKind::ContextCompactionStarted
        | TurnStreamEventKind::ContextCompactionCompleted => {
            let complete = event.kind == TurnStreamEventKind::ContextCompactionCompleted;
            let segment_id = context_compaction_segment_id(assistant_turn_id, event);
            let mut segment = find_segment(snapshot, &segment_id)
                .cloned()
                .unwrap_or_else(|| {
                    segment_shell(
                        &segment_id,
                        assistant_turn_id,
                        next_turn_segment_order(snapshot, assistant_turn_id),
                        "context_compaction",
                        "system",
                        if complete { "complete" } else { "running" },
                        &now,
                        build_segment_source(event, None),
                    )
                });
            set_string_value(
                &mut segment,
                "content",
                if complete {
                    "Context compacted to continue the turn."
                } else {
                    "Compacting conversation context..."
                },
            );
            set_string_value(
                &mut segment,
                "status",
                if complete { "complete" } else { "running" },
            );
            set_string_value(&mut segment, "updated_at", &now);
            if complete {
                set_string_value(&mut segment, "completed_at", &now);
            }
            remove_key(&mut segment, "error");
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        TurnStreamEventKind::RequestUserInputRequested => {
            let request = event
                .request_user_input
                .as_ref()
                .and_then(normalize_request_user_input_payload)?;
            let segment_id = request_user_input_segment_id(assistant_turn_id, event, &request);
            let mut segment = find_segment(snapshot, &segment_id)
                .cloned()
                .unwrap_or_else(|| {
                    segment_shell(
                        &segment_id,
                        assistant_turn_id,
                        next_turn_segment_order(snapshot, assistant_turn_id),
                        "request_user_input",
                        "system",
                        "pending",
                        &now,
                        build_segment_source(event, None),
                    )
                });
            if segment.get("status").and_then(Value::as_str) == Some("complete") {
                return None;
            }
            set_string_value(&mut segment, "status", "pending");
            set_string_value(&mut segment, "updated_at", &now);
            set_string_value(
                &mut segment,
                "content",
                &request_user_input_segment_content(&request),
            );
            remove_key(&mut segment, "completed_at");
            remove_key(&mut segment, "error");
            set_value(&mut segment, "request_user_input", Value::Object(request));
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        TurnStreamEventKind::Other(kind) if is_model_tool_call_kind(kind) => {
            let tool_call = event.tool_call.clone()?;
            let segment_id = model_tool_segment_id(assistant_turn_id, event, &tool_call);
            let status = model_tool_call_status(kind, &tool_call);
            let mut segment = find_segment(snapshot, &segment_id)
                .cloned()
                .unwrap_or_else(|| {
                    segment_shell(
                        &segment_id,
                        assistant_turn_id,
                        next_turn_segment_order(snapshot, assistant_turn_id),
                        "model_tool_call",
                        "assistant",
                        &status,
                        &now,
                        build_segment_source(event, tool_call_id(&tool_call).as_deref()),
                    )
                });
            set_string_value(&mut segment, "status", &status);
            set_string_value(&mut segment, "updated_at", &now);
            set_value(
                &mut segment,
                "source",
                build_segment_source(event, tool_call_id(&tool_call).as_deref()),
            );
            set_value(&mut segment, "tool_call", tool_call);
            if status == "complete" || status == "failed" {
                set_string_value(&mut segment, "completed_at", &now);
            }
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        TurnStreamEventKind::ToolCallStarted
        | TurnStreamEventKind::ToolCallUpdated
        | TurnStreamEventKind::ToolCallCompleted
        | TurnStreamEventKind::ToolCallFailed => {
            let tool_call = event.tool_call.clone()?;
            let segment_id = tool_segment_id(assistant_turn_id, event, &tool_call);
            let status = tool_call_status(event, &tool_call);
            let mut segment = find_segment(snapshot, &segment_id)
                .cloned()
                .unwrap_or_else(|| {
                    segment_shell(
                        &segment_id,
                        assistant_turn_id,
                        next_turn_segment_order(snapshot, assistant_turn_id),
                        "tool_call",
                        "system",
                        &status,
                        &now,
                        build_segment_source(event, tool_call_id(&tool_call).as_deref()),
                    )
                });
            set_string_value(&mut segment, "status", &status);
            set_string_value(&mut segment, "updated_at", &now);
            set_value(&mut segment, "tool_call", tool_call);
            if status != "running" {
                set_string_value(&mut segment, "completed_at", &now);
            }
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        TurnStreamEventKind::TurnCompleted => {
            let segment_id = agent_event_segment_id(snapshot, assistant_turn_id, event);
            let mut segment = agent_event_segment_shell(
                &segment_id,
                assistant_turn_id,
                next_turn_segment_order(snapshot, assistant_turn_id),
                "processing",
                "complete",
                &now,
                event,
            );
            if let Some(status) = event.status.as_deref().and_then(non_empty_string) {
                set_string_value(&mut segment, "event_status", &status);
            }
            if let Some(phase) = event.phase.as_deref().and_then(non_empty_string) {
                set_string_value(&mut segment, "phase", &phase);
            }
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        TurnStreamEventKind::Other(kind) if is_agent_event_kind(kind) => {
            let segment_id = agent_event_segment_id(snapshot, assistant_turn_id, event);
            let (category, status) = match kind.as_str() {
                "session_start" => ("lifecycle", "running"),
                "session_end" => ("lifecycle", "complete"),
                "warning" => ("warning", "complete"),
                _ => ("session", "complete"),
            };
            let mut segment = agent_event_segment_shell(
                &segment_id,
                assistant_turn_id,
                next_turn_segment_order(snapshot, assistant_turn_id),
                category,
                status,
                &now,
                event,
            );
            if let Some(event_status) = event.status.as_deref().and_then(non_empty_string) {
                set_string_value(&mut segment, "event_status", &event_status);
            }
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        TurnStreamEventKind::Error => {
            let segment_id = assistant_segment_id(assistant_turn_id, event);
            let mut segment = find_segment(snapshot, &segment_id)
                .cloned()
                .unwrap_or_else(|| {
                    segment_shell(
                        &segment_id,
                        assistant_turn_id,
                        next_turn_segment_order(snapshot, assistant_turn_id),
                        "assistant_message",
                        "assistant",
                        "failed",
                        &now,
                        build_segment_source(event, None),
                    )
                });
            let message = event
                .error
                .clone()
                .or_else(|| event.message.clone())
                .unwrap_or_else(|| "Conversation turn failed.".to_string());
            if let Some(text) = event_text(event) {
                set_string_value(&mut segment, "content", &text);
            }
            set_string_value(&mut segment, "status", "failed");
            set_string_value(&mut segment, "error", &message);
            if let Some(error_code) = event.error_code.as_deref().and_then(non_empty_string) {
                set_string_value(&mut segment, "error_code", &error_code);
            }
            if let Some(details) = event.details.as_ref() {
                set_value(&mut segment, "details", details.clone());
            }
            set_string_value(&mut segment, "updated_at", &now);
            set_string_value(&mut segment, "completed_at", &now);
            if let Some(phase) = normalized_assistant_phase(event.phase.as_deref()) {
                set_string_value(&mut segment, "phase", &phase);
            }
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        _ => None,
    }
}

fn should_emit_segment_upsert_for_event(event: &TurnStreamEvent) -> bool {
    match &event.kind {
        TurnStreamEventKind::ContentDelta | TurnStreamEventKind::ToolCallUpdated => false,
        TurnStreamEventKind::Other(kind) if kind == "model_tool_call_delta" => false,
        _ => true,
    }
}

fn is_agent_event_kind(kind: &str) -> bool {
    matches!(kind, "session_start" | "session_end" | "warning")
}

fn agent_event_segment_shell(
    segment_id: &str,
    assistant_turn_id: &str,
    order: i64,
    category: &str,
    status: &str,
    timestamp: &str,
    event: &TurnStreamEvent,
) -> Value {
    let mut segment = segment_shell(
        segment_id,
        assistant_turn_id,
        order,
        "agent_event",
        "system",
        status,
        timestamp,
        build_segment_source(event, None),
    );
    let event_kind = event
        .source
        .raw_kind
        .as_deref()
        .and_then(non_empty_string)
        .unwrap_or_else(|| event.kind.as_str().to_string());
    set_string_value(&mut segment, "event_kind", &event_kind);
    set_string_value(&mut segment, "category", category);
    if let Some(message) = event.message.as_deref().and_then(non_empty_string) {
        set_string_value(&mut segment, "content", &message);
        set_string_value(&mut segment, "message", &message);
    } else {
        set_string_value(&mut segment, "content", &event_kind);
    }
    if let Some(details) = event.details.as_ref() {
        set_value(&mut segment, "details", details.clone());
    }
    set_string_value(&mut segment, "updated_at", timestamp);
    set_string_value(&mut segment, "completed_at", timestamp);
    segment
}

fn segment_shell(
    segment_id: &str,
    turn_id: &str,
    order: i64,
    kind: &str,
    role: &str,
    status: &str,
    timestamp: &str,
    source: Value,
) -> Value {
    json!({
        "id": segment_id,
        "turn_id": turn_id,
        "order": order,
        "kind": kind,
        "role": role,
        "status": status,
        "timestamp": timestamp,
        "updated_at": timestamp,
        "content": "",
        "source": source,
    })
}

fn append_segment_content(segment: &mut Value, delta: &str) {
    if delta.is_empty() {
        return;
    }
    let mut content = segment
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    content.push_str(delta);
    set_string_value(segment, "content", &content);
}

fn build_segment_source(event: &TurnStreamEvent, call_id: Option<&str>) -> Value {
    let mut source = Map::new();
    if let Some(value) = event
        .source
        .backend
        .as_ref()
        .and_then(|value| non_empty_string(value))
    {
        source.insert("backend".to_string(), json!(value));
    }
    if let Some(value) = event
        .source
        .session_id
        .as_ref()
        .and_then(|value| non_empty_string(value))
    {
        source.insert("session_id".to_string(), json!(value));
    }
    if let Some(value) = event
        .source
        .app_thread_id
        .as_ref()
        .and_then(|value| non_empty_string(value))
    {
        source.insert("app_thread_id".to_string(), json!(value));
    }
    if let Some(value) = event
        .source
        .app_turn_id
        .as_ref()
        .and_then(|value| non_empty_string(value))
    {
        source.insert("app_turn_id".to_string(), json!(value));
    }
    if let Some(value) = event
        .source
        .item_id
        .as_ref()
        .and_then(|value| non_empty_string(value))
    {
        source.insert("item_id".to_string(), json!(value));
    }
    if let Some(value) = event.source.summary_index {
        source.insert("summary_index".to_string(), json!(value));
    }
    if let Some(value) = event
        .source
        .response_id
        .as_ref()
        .and_then(|value| non_empty_string(value))
    {
        source.insert("response_id".to_string(), json!(value));
    }
    if let Some(value) = event
        .source
        .raw_kind
        .as_ref()
        .and_then(|value| non_empty_string(value))
    {
        source.insert("raw_kind".to_string(), json!(value));
    }
    if let Some(value) = call_id.and_then(non_empty_string) {
        source.insert("call_id".to_string(), json!(value));
    }
    Value::Object(source)
}

fn reasoning_segment_id(turn_id: &str, event: &TurnStreamEvent) -> String {
    let app_turn_id = event
        .source
        .app_turn_id
        .as_deref()
        .and_then(non_empty_string)
        .unwrap_or_else(|| turn_id.to_string());
    let item_id = event
        .source
        .item_id
        .as_deref()
        .and_then(non_empty_string)
        .unwrap_or_else(|| "reasoning".to_string());
    let summary_index = event.source.summary_index.unwrap_or(0);
    format!("segment-reasoning-{app_turn_id}-{item_id}-{summary_index}")
}

fn assistant_segment_id(turn_id: &str, event: &TurnStreamEvent) -> String {
    match (
        event
            .source
            .app_turn_id
            .as_deref()
            .and_then(non_empty_string),
        event.source.item_id.as_deref().and_then(non_empty_string),
    ) {
        (Some(app_turn_id), Some(item_id)) => format!("segment-assistant-{app_turn_id}-{item_id}"),
        _ => format!("segment-assistant-{turn_id}"),
    }
}

fn plan_segment_id(turn_id: &str, event: &TurnStreamEvent) -> String {
    match (
        event
            .source
            .app_turn_id
            .as_deref()
            .and_then(non_empty_string),
        event.source.item_id.as_deref().and_then(non_empty_string),
    ) {
        (Some(app_turn_id), Some(item_id)) => format!("segment-plan-{app_turn_id}-{item_id}"),
        _ => format!("segment-plan-{turn_id}"),
    }
}

fn agent_event_segment_id(snapshot: &Value, turn_id: &str, event: &TurnStreamEvent) -> String {
    let kind = event
        .source
        .raw_kind
        .as_deref()
        .and_then(non_empty_string)
        .unwrap_or_else(|| event.kind.as_str().to_string());
    let sequence = next_turn_segment_order(snapshot, turn_id);
    match (
        event
            .source
            .app_turn_id
            .as_deref()
            .and_then(non_empty_string),
        event.source.item_id.as_deref().and_then(non_empty_string),
    ) {
        (Some(app_turn_id), Some(item_id)) => {
            format!("segment-agent-event-{app_turn_id}-{kind}-{item_id}")
        }
        _ => format!("segment-agent-event-{turn_id}-{kind}-{sequence}"),
    }
}

fn context_compaction_segment_id(turn_id: &str, event: &TurnStreamEvent) -> String {
    let app_turn_id = event
        .source
        .app_turn_id
        .as_deref()
        .and_then(non_empty_string)
        .unwrap_or_else(|| turn_id.to_string());
    format!("segment-context-compaction-{app_turn_id}")
}

fn tool_segment_id(turn_id: &str, event: &TurnStreamEvent, tool_call: &Value) -> String {
    let app_turn_id = event
        .source
        .app_turn_id
        .as_deref()
        .and_then(non_empty_string)
        .unwrap_or_else(|| turn_id.to_string());
    let call_id = tool_call_id(tool_call)
        .or_else(|| event.source.item_id.as_deref().and_then(non_empty_string))
        .unwrap_or_else(|| "tool".to_string());
    format!("segment-tool-{app_turn_id}-{call_id}")
}

fn model_tool_segment_id(turn_id: &str, event: &TurnStreamEvent, tool_call: &Value) -> String {
    let app_turn_id = event
        .source
        .app_turn_id
        .as_deref()
        .and_then(non_empty_string)
        .unwrap_or_else(|| turn_id.to_string());
    let call_id = tool_call_id(tool_call)
        .or_else(|| event.source.item_id.as_deref().and_then(non_empty_string))
        .unwrap_or_else(|| "model-tool".to_string());
    format!("segment-model-tool-{app_turn_id}-{call_id}")
}

fn request_user_input_segment_id(
    turn_id: &str,
    event: &TurnStreamEvent,
    request: &Map<String, Value>,
) -> String {
    let app_turn_id = event
        .source
        .app_turn_id
        .as_deref()
        .and_then(non_empty_string)
        .unwrap_or_else(|| turn_id.to_string());
    let request_id = request
        .get("request_id")
        .and_then(Value::as_str)
        .and_then(non_empty_string)
        .or_else(|| event.source.item_id.as_deref().and_then(non_empty_string))
        .unwrap_or_else(|| "request".to_string());
    format!("segment-request-user-input-{app_turn_id}-{request_id}")
}

fn tool_call_id(tool_call: &Value) -> Option<String> {
    tool_call
        .get("id")
        .and_then(Value::as_str)
        .and_then(non_empty_string)
}

fn tool_call_status(event: &TurnStreamEvent, tool_call: &Value) -> String {
    let raw_status = tool_call
        .get("status")
        .and_then(Value::as_str)
        .and_then(non_empty_string)
        .or_else(|| event.status.as_deref().and_then(non_empty_string));
    if let Some(raw_status) = raw_status {
        return match raw_status.as_str() {
            "started" | "updated" | "pending" | "streaming" => "running".to_string(),
            "completed" => "complete".to_string(),
            other => other.to_string(),
        };
    }
    match &event.kind {
        TurnStreamEventKind::ToolCallStarted | TurnStreamEventKind::ToolCallUpdated => {
            "running".to_string()
        }
        TurnStreamEventKind::ToolCallFailed => "failed".to_string(),
        _ => "complete".to_string(),
    }
}

fn is_model_tool_call_kind(kind: &str) -> bool {
    matches!(
        kind,
        "model_tool_call_start" | "model_tool_call_delta" | "model_tool_call_end"
    )
}

fn model_tool_call_status(kind: &str, tool_call: &Value) -> String {
    let raw_status = tool_call
        .get("status")
        .and_then(Value::as_str)
        .and_then(non_empty_string);
    if let Some(raw_status) = raw_status {
        return match raw_status.as_str() {
            "completed" | "complete" => "complete".to_string(),
            "failed" => "failed".to_string(),
            _ => "running".to_string(),
        };
    }
    match kind {
        "model_tool_call_end" => "complete".to_string(),
        _ => "running".to_string(),
    }
}

fn normalized_assistant_phase(phase: Option<&str>) -> Option<String> {
    phase.and_then(non_empty_string).map(|value| {
        value
            .trim()
            .to_lowercase()
            .replace('-', "_")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("_")
    })
}

fn normalize_request_user_input_payload(payload: &Value) -> Option<Map<String, Value>> {
    let object = payload.as_object()?;
    let request_id = object
        .get("request_id")
        .or_else(|| object.get("itemId"))
        .and_then(Value::as_str)
        .and_then(non_empty_string)?;
    let raw_questions = object.get("questions").and_then(Value::as_array)?;
    let mut questions = Vec::new();
    for (index, raw_question) in raw_questions.iter().enumerate() {
        let Some(question) = raw_question.as_object() else {
            continue;
        };
        let prompt = question
            .get("question")
            .and_then(Value::as_str)
            .and_then(non_empty_string);
        let Some(prompt) = prompt else {
            continue;
        };
        let options = question
            .get("options")
            .and_then(Value::as_array)
            .map(|options| {
                options
                    .iter()
                    .filter_map(|option| {
                        let option = option.as_object()?;
                        let label = option
                            .get("label")
                            .and_then(Value::as_str)
                            .and_then(non_empty_string)?;
                        let mut output = Map::new();
                        output.insert("label".to_string(), json!(label));
                        if let Some(description) = option
                            .get("description")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                        {
                            output.insert("description".to_string(), json!(description));
                        }
                        Some(Value::Object(output))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let question_type = question
            .get("question_type")
            .and_then(Value::as_str)
            .and_then(non_empty_string)
            .unwrap_or_else(|| {
                if options.is_empty() {
                    "FREEFORM".to_string()
                } else {
                    "MULTIPLE_CHOICE".to_string()
                }
            });
        questions.push(json!({
            "id": question
                .get("id")
                .and_then(Value::as_str)
                .and_then(non_empty_string)
                .unwrap_or_else(|| format!("question-{}", index + 1)),
            "header": question
                .get("header")
                .and_then(Value::as_str)
                .and_then(non_empty_string)
                .unwrap_or_else(|| format!("Question {}", index + 1)),
            "question": prompt,
            "question_type": question_type,
            "options": options,
            "allow_other": question
                .get("allow_other")
                .or_else(|| question.get("isOther"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            "is_secret": question
                .get("is_secret")
                .or_else(|| question.get("isSecret"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }));
    }
    if questions.is_empty() {
        return None;
    }
    let answers = object
        .get("answers")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut output = Map::new();
    output.insert("request_id".to_string(), json!(request_id));
    output.insert(
        "status".to_string(),
        json!(normalize_request_user_input_status(
            object.get("status").and_then(Value::as_str)
        )),
    );
    output.insert("questions".to_string(), Value::Array(questions));
    output.insert("answers".to_string(), Value::Object(answers));
    for key in ["app_thread_id", "app_turn_id"] {
        if let Some(value) = object
            .get(key)
            .and_then(Value::as_str)
            .and_then(non_empty_string)
        {
            output.insert(key.to_string(), json!(value));
        }
    }
    if let Some(submitted_at) = object.get("submitted_at").and_then(Value::as_str) {
        output.insert("submitted_at".to_string(), json!(submitted_at));
    }
    Some(output)
}

fn request_user_input_answer_metadata(
    segment: &Value,
    request_record: &Map<String, Value>,
    assistant_turn_id: &str,
    segment_id: &str,
    lookup_id: &str,
    submitted_at: &str,
    trace_path: Option<&Path>,
) -> BTreeMap<String, Value> {
    let mut metadata = BTreeMap::from([
        (
            "spark.workspace.assistant_turn_id".to_string(),
            json!(assistant_turn_id),
        ),
        (
            "spark.workspace.request_user_input.segment_id".to_string(),
            json!(segment_id),
        ),
        (
            "spark.workspace.request_user_input.lookup_id".to_string(),
            json!(lookup_id),
        ),
        (
            "spark.workspace.request_user_input.submitted_at".to_string(),
            json!(submitted_at),
        ),
    ]);
    for (source_key, metadata_key) in [
        ("app_thread_id", "spark.runtime.codex_app_server.thread_id"),
        ("app_turn_id", "spark.runtime.codex_app_server.turn_id"),
    ] {
        if let Some(value) = segment
            .get("source")
            .and_then(Value::as_object)
            .and_then(|source| source.get(source_key))
            .or_else(|| request_record.get(source_key))
            .and_then(Value::as_str)
            .and_then(non_empty_string)
        {
            metadata.insert(metadata_key.to_string(), json!(value));
        }
    }
    if let Some(trace_path) = trace_path {
        metadata.insert(
            CODEX_JSONRPC_TRACE_PATH_METADATA_KEY.to_string(),
            json!(trace_path.to_string_lossy().to_string()),
        );
    }
    metadata
}

fn normalize_request_user_input_status(status: Option<&str>) -> &'static str {
    match status {
        Some("answered") => "answered",
        Some("expired") => "expired",
        _ => "pending",
    }
}

fn request_user_input_segment_content(request: &Map<String, Value>) -> String {
    match request.get("status").and_then(Value::as_str) {
        Some("answered" | "expired") => request_user_input_answer_summary(request),
        _ => request_user_input_prompt_summary(request),
    }
}

fn request_user_input_prompt_summary(request: &Map<String, Value>) -> String {
    let prompts = request
        .get("questions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|question| {
            question
                .get("question")
                .and_then(Value::as_str)
                .map(normalize_assistant_text)
                .filter(|value| !value.is_empty())
        })
        .collect::<Vec<_>>();
    match prompts.len() {
        0 => "User input requested.".to_string(),
        1 => prompts[0].clone(),
        count => format!("{count} questions need user input."),
    }
}

fn request_user_input_answer_summary(request: &Map<String, Value>) -> String {
    let answers = request
        .get("answers")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let lines = request
        .get("questions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|question| {
            let prompt = question
                .get("question")
                .and_then(Value::as_str)
                .map(normalize_assistant_text)
                .unwrap_or_default();
            let question_id = question.get("id").and_then(Value::as_str)?;
            let answer = answers
                .get(question_id)
                .and_then(Value::as_str)
                .map(normalize_assistant_text)
                .unwrap_or_default();
            (!prompt.is_empty() && !answer.is_empty())
                .then(|| format!("{prompt}\nAnswer: {answer}"))
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        request_user_input_prompt_summary(request)
    } else {
        lines.join("\n\n")
    }
}

fn request_user_input_answer_strings(answers: &Map<String, Value>) -> BTreeMap<String, String> {
    answers
        .iter()
        .filter_map(|(key, value)| {
            value
                .as_str()
                .and_then(non_empty_string)
                .map(|answer| (key.clone(), answer))
        })
        .collect()
}

fn answered_request_user_input_segment(
    segment: &Value,
    request_record: &Map<String, Value>,
    submitted_at: &str,
) -> Value {
    let mut updated_segment = segment.clone();
    if let Some(object) = updated_segment.as_object_mut() {
        object.insert("status".to_string(), json!("complete"));
        object.insert("updated_at".to_string(), json!(submitted_at));
        object.insert("completed_at".to_string(), json!(submitted_at));
        object.insert(
            "content".to_string(),
            json!(request_user_input_answer_summary(request_record)),
        );
        object.insert(
            "request_user_input".to_string(),
            Value::Object(request_record.clone()),
        );
    }
    remove_key(&mut updated_segment, "error");
    updated_segment
}

fn expire_request_user_input_answer_in_snapshot(
    snapshot: &mut Value,
    segment_id: &str,
    mut request_record: Map<String, Value>,
    expired_at: &str,
    emitted_payloads: &mut Vec<Value>,
) {
    request_record.insert("status".to_string(), json!("expired"));
    request_record
        .entry("submitted_at".to_string())
        .or_insert_with(|| json!(expired_at));
    let Some(segment) = find_segment(snapshot, segment_id).cloned() else {
        return;
    };
    let mut updated_segment = segment.clone();
    if let Some(object) = updated_segment.as_object_mut() {
        object.insert("status".to_string(), json!("failed"));
        object.insert("updated_at".to_string(), json!(expired_at));
        object.insert("completed_at".to_string(), json!(expired_at));
        object.insert("error".to_string(), json!(REQUEST_USER_INPUT_EXPIRED_ERROR));
        object.insert(
            "content".to_string(),
            json!(request_user_input_answer_summary(&request_record)),
        );
        object.insert(
            "request_user_input".to_string(),
            Value::Object(request_record),
        );
    }
    upsert_segment(snapshot, updated_segment.clone());
    if let Some(assistant_turn_id) = updated_segment.get("turn_id").and_then(Value::as_str) {
        if let Some(turn) = find_turn_mut(snapshot, assistant_turn_id) {
            if turn.get("role").and_then(Value::as_str) == Some("assistant")
                && matches!(
                    turn.get("status").and_then(Value::as_str),
                    Some("pending" | "streaming" | "failed")
                )
            {
                set_string_value(turn, "status", "failed");
                set_string_value(turn, "error", REQUEST_USER_INPUT_EXPIRED_ERROR);
                let turn = turn.clone();
                emitted_payloads.push(build_turn_upsert_payload(snapshot, &turn));
            }
        }
    }
    emitted_payloads.push(build_segment_upsert_payload(snapshot, &updated_segment));
}

fn request_user_input_answer_cannot_resume(output: &AgentTurnOutput) -> bool {
    output
        .thread_resume_failure
        .as_ref()
        .and_then(|failure| failure.error_code.as_deref())
        .is_some_and(|error_code| error_code.starts_with("request_user_input_"))
        || output.events.iter().any(|event| {
            event.source.raw_kind.as_deref() == Some("request_user_input_resume_failure")
        })
}

fn request_user_input_answer_delivered_to_live_request(output: &AgentTurnOutput) -> bool {
    output.events.iter().any(|event| {
        event.source.raw_kind.as_deref() == Some("request_user_input_answer_delivered")
            || matches!(
                &event.kind,
                TurnStreamEventKind::Other(kind) if kind == "request_user_input_answer_delivered"
            )
    })
}

fn normalize_request_user_input_answers(
    request: &Map<String, Value>,
    answers: &BTreeMap<String, String>,
) -> WorkspaceResult<Map<String, Value>> {
    let mut output = Map::new();
    let questions = request
        .get("questions")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            WorkspaceError::Validation("At least one answer is required.".to_string())
        })?;
    for question in questions {
        let Some(question_id) = question.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(answer) = answers
            .get(question_id)
            .and_then(|value| non_empty_string(value))
        else {
            return Err(WorkspaceError::Validation(format!(
                "Missing answer for question '{question_id}'."
            )));
        };
        if question.get("question_type").and_then(Value::as_str) == Some("MULTIPLE_CHOICE") {
            let option_labels = question
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|option| {
                    option
                        .get("label")
                        .and_then(Value::as_str)
                        .map(normalize_assistant_text)
                        .filter(|value| !value.is_empty())
                })
                .collect::<Vec<_>>();
            let allow_other = question
                .get("allow_other")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !option_labels.iter().any(|label| label == &answer) && !allow_other {
                return Err(WorkspaceError::Validation(format!(
                    "Unsupported answer option for question '{question_id}'."
                )));
            }
        }
        output.insert(question_id.to_string(), json!(answer));
    }
    if output.is_empty() {
        return Err(WorkspaceError::Validation(
            "At least one answer is required.".to_string(),
        ));
    }
    Ok(output)
}

fn answers_equal(left: &Map<String, Value>, right: &Map<String, Value>) -> bool {
    left.len() == right.len()
        && right
            .iter()
            .all(|(key, value)| left.get(key).and_then(Value::as_str) == value.as_str())
}

fn find_request_user_input_segment_id(snapshot: &Value, lookup_id: &str) -> Option<String> {
    snapshot
        .get("segments")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|segment| {
            if segment.get("kind").and_then(Value::as_str) != Some("request_user_input") {
                return None;
            }
            let request = segment.get("request_user_input")?.as_object()?;
            if request.get("request_id").and_then(Value::as_str) == Some(lookup_id)
                || request
                    .get("questions")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .any(|question| question.get("id").and_then(Value::as_str) == Some(lookup_id))
            {
                segment
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            } else {
                None
            }
        })
}

fn normalize_assistant_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn finalized_plan_segment(snapshot: &Value, assistant_turn_id: &str) -> Option<Value> {
    snapshot
        .get("segments")
        .and_then(Value::as_array)?
        .iter()
        .find(|segment| {
            segment.get("turn_id").and_then(Value::as_str) == Some(assistant_turn_id)
                && segment.get("kind").and_then(Value::as_str) == Some("plan")
                && segment.get("status").and_then(Value::as_str) == Some("complete")
                && segment
                    .get("content")
                    .and_then(Value::as_str)
                    .and_then(non_empty_string)
                    .is_some()
        })
        .cloned()
}

fn finalized_assistant_segment(snapshot: &Value, assistant_turn_id: &str) -> Option<Value> {
    snapshot
        .get("segments")
        .and_then(Value::as_array)?
        .iter()
        .find(|segment| {
            segment.get("turn_id").and_then(Value::as_str) == Some(assistant_turn_id)
                && segment.get("kind").and_then(Value::as_str) == Some("assistant_message")
                && segment.get("status").and_then(Value::as_str) == Some("complete")
                && is_final_answer_phase(segment.get("phase").and_then(Value::as_str))
        })
        .cloned()
}

fn persist_proposed_plan_artifact(
    snapshot: &mut Value,
    assistant_turn_id: &str,
    plan_segment: &Value,
) -> Option<Value> {
    if plan_segment.get("kind").and_then(Value::as_str) != Some("plan") {
        return None;
    }
    let content = plan_segment
        .get("content")
        .and_then(Value::as_str)
        .and_then(non_empty_string)?;
    let segment_id = plan_segment.get("id").and_then(Value::as_str)?;
    let title = proposed_plan_title(&content);
    let now = iso_now();
    let existing_index = snapshot
        .get("proposed_plans")
        .and_then(Value::as_array)
        .and_then(|plans| {
            plans.iter().position(|plan| {
                let artifact_match = plan_segment
                    .get("artifact_id")
                    .and_then(Value::as_str)
                    .and_then(non_empty_string)
                    .map(|artifact_id| plan.get("id").and_then(Value::as_str) == Some(&artifact_id))
                    .unwrap_or(false);
                artifact_match
                    || plan.get("source_segment_id").and_then(Value::as_str) == Some(segment_id)
            })
        });
    let artifact_id = if let Some(index) = existing_index {
        let existing_id = artifact_string(snapshot, "proposed_plans", index, "id")?;
        update_artifact_at(snapshot, "proposed_plans", index, |artifact| {
            set_string_value(artifact, "title", &title);
            set_string_value(artifact, "content", &content);
            set_string_value(artifact, "updated_at", &now);
        });
        existing_id
    } else {
        let artifact_id = random_artifact_id("proposed-plan");
        push_array_value(
            snapshot,
            "proposed_plans",
            json!({
                "id": artifact_id.clone(),
                "created_at": now.clone(),
                "updated_at": now.clone(),
                "title": title.clone(),
                "content": content.clone(),
                "project_path": snapshot.get("project_path").and_then(Value::as_str).unwrap_or(""),
                "conversation_id": snapshot.get("conversation_id").and_then(Value::as_str).unwrap_or(""),
                "source_turn_id": assistant_turn_id,
                "status": "pending_review",
                "source_segment_id": segment_id,
            }),
        );
        append_workflow_event(
            snapshot,
            json!({
                "message": format!("Created proposed plan artifact {artifact_id}."),
                "timestamp": now,
            }),
        );
        artifact_id
    };
    set_segment_artifact_id(snapshot, segment_id, &artifact_id)
}

fn set_segment_artifact_id(
    snapshot: &mut Value,
    segment_id: &str,
    artifact_id: &str,
) -> Option<Value> {
    let segment = snapshot
        .as_object_mut()
        .and_then(|object| object.get_mut("segments"))
        .and_then(Value::as_array_mut)?
        .iter_mut()
        .find(|segment| segment.get("id").and_then(Value::as_str) == Some(segment_id))?;
    set_string_value(segment, "artifact_id", artifact_id);
    Some(segment.clone())
}

fn fallback_assistant_segment_id(snapshot: &Value, assistant_turn_id: &str) -> Option<String> {
    let segments = snapshot.get("segments").and_then(Value::as_array)?;
    let assistant_segments = segments
        .iter()
        .filter(|segment| {
            segment.get("turn_id").and_then(Value::as_str) == Some(assistant_turn_id)
                && segment.get("kind").and_then(Value::as_str) == Some("assistant_message")
        })
        .collect::<Vec<_>>();

    if let Some(segment) = assistant_segments
        .iter()
        .find(|segment| is_final_answer_phase(segment.get("phase").and_then(Value::as_str)))
    {
        return segment
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string);
    }

    let source_keyed_segments = assistant_segments
        .iter()
        .filter(|segment| {
            segment
                .get("source")
                .and_then(Value::as_object)
                .map(|source| source.contains_key("app_turn_id") || source.contains_key("item_id"))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    if source_keyed_segments.len() == 1 {
        return source_keyed_segments[0]
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string);
    }

    if assistant_segments.len() == 1 {
        return assistant_segments[0]
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string);
    }

    None
}

fn complete_existing_assistant_segment_with_text(
    snapshot: &mut Value,
    segment_id: &str,
    text: &str,
) -> Option<Value> {
    let now = iso_now();
    let segment = snapshot
        .as_object_mut()
        .and_then(|object| object.get_mut("segments"))
        .and_then(Value::as_array_mut)?
        .iter_mut()
        .find(|segment| segment.get("id").and_then(Value::as_str) == Some(segment_id))?;
    set_string_value(segment, "content", text);
    set_string_value(segment, "status", "complete");
    set_string_value(segment, "updated_at", &now);
    set_string_value(segment, "completed_at", &now);
    set_string_value(segment, "phase", "final_answer");
    remove_key(segment, "error");
    remove_key(segment, "error_code");
    remove_key(segment, "details");
    Some(segment.clone())
}

fn failed_assistant_state(
    snapshot: &Value,
    assistant_turn_id: &str,
) -> Option<(String, Option<String>, Option<Value>)> {
    if let Some(turn) = find_turn(snapshot, assistant_turn_id) {
        if turn.get("status").and_then(Value::as_str) == Some("failed") {
            let message = turn
                .get("error")
                .and_then(Value::as_str)
                .and_then(non_empty_string)
                .unwrap_or_else(|| "Conversation turn failed.".to_string());
            let error_code = turn
                .get("error_code")
                .and_then(Value::as_str)
                .and_then(non_empty_string);
            let details = turn.get("details").cloned();
            return Some((message, error_code, details));
        }
    }

    snapshot
        .get("segments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find_map(|segment| {
            if segment.get("turn_id").and_then(Value::as_str) != Some(assistant_turn_id)
                || !matches!(
                    segment.get("kind").and_then(Value::as_str),
                    Some("assistant_message" | "plan")
                )
                || segment.get("status").and_then(Value::as_str) != Some("failed")
            {
                return None;
            }
            let message = segment
                .get("error")
                .and_then(Value::as_str)
                .and_then(non_empty_string)
                .unwrap_or_else(|| "Conversation turn failed.".to_string());
            let error_code = segment
                .get("error_code")
                .and_then(Value::as_str)
                .and_then(non_empty_string);
            let details = segment.get("details").cloned();
            Some((message, error_code, details))
        })
}

fn fail_assistant_turn_and_segments(
    snapshot: &mut Value,
    assistant_turn_id: &str,
    message: &str,
    error_code: Option<&str>,
    details: Option<&Value>,
    force_turn_upsert: bool,
    emitted_payloads: &mut Vec<Value>,
) {
    let now = iso_now();
    if let Some(segments) = snapshot
        .as_object_mut()
        .and_then(|object| object.get_mut("segments"))
        .and_then(Value::as_array_mut)
    {
        let mut updated_segments = Vec::new();
        for segment in segments.iter_mut() {
            if segment.get("turn_id").and_then(Value::as_str) != Some(assistant_turn_id)
                || !matches!(
                    segment.get("kind").and_then(Value::as_str),
                    Some("assistant_message" | "plan")
                )
            {
                continue;
            }
            let changed = segment.get("status").and_then(Value::as_str) != Some("failed")
                || segment.get("error").and_then(Value::as_str) != Some(message)
                || error_code.is_some()
                || details.is_some_and(|details| segment.get("details") != Some(details));
            set_string_value(segment, "status", "failed");
            set_string_value(segment, "error", message);
            if let Some(error_code) = error_code {
                set_string_value(segment, "error_code", error_code);
            }
            if let Some(details) = details {
                set_value(segment, "details", details.clone());
            }
            set_string_value(segment, "updated_at", &now);
            set_string_value(segment, "completed_at", &now);
            if changed {
                updated_segments.push(segment.clone());
            }
        }
        for segment in updated_segments {
            emitted_payloads.push(build_segment_upsert_payload(snapshot, &segment));
        }
    }

    if let Some(turn) = find_turn_mut(snapshot, assistant_turn_id) {
        let changed = turn.get("status").and_then(Value::as_str) != Some("failed")
            || turn.get("error").and_then(Value::as_str) != Some(message)
            || error_code.is_some()
            || details.is_some_and(|details| turn.get("details") != Some(details))
            || force_turn_upsert;
        set_string_value(turn, "status", "failed");
        set_string_value(turn, "error", message);
        if let Some(error_code) = error_code {
            set_string_value(turn, "error_code", error_code);
        }
        if let Some(details) = details {
            set_value(turn, "details", details.clone());
        }
        if changed {
            let turn = turn.clone();
            emitted_payloads.push(build_turn_upsert_payload(snapshot, &turn));
        }
    }
}

fn apply_assistant_turn_token_usage(
    snapshot: &mut Value,
    assistant_turn_id: &str,
    token_usage: &Value,
) -> bool {
    let Some(turn) = find_turn_mut(snapshot, assistant_turn_id) else {
        return false;
    };
    let changed = turn.get("token_usage") != Some(token_usage);
    set_value(turn, "token_usage", token_usage.clone());
    changed
}

fn apply_assistant_turn_token_usage_breakdown(
    snapshot: &mut Value,
    assistant_turn_id: &str,
    token_usage_breakdown: &Value,
) -> bool {
    let Some(turn) = find_turn_mut(snapshot, assistant_turn_id) else {
        return false;
    };
    let changed = turn.get("token_usage_breakdown") != Some(token_usage_breakdown);
    set_value(turn, "token_usage_breakdown", token_usage_breakdown.clone());
    changed
}

fn apply_assistant_turn_app_server_ids(
    snapshot: &mut Value,
    assistant_turn_id: &str,
    app_thread_id: Option<&str>,
    app_turn_id: Option<&str>,
) -> bool {
    let Some(turn) = find_turn_mut(snapshot, assistant_turn_id) else {
        return false;
    };
    let mut changed = false;
    if let Some(app_thread_id) = app_thread_id.and_then(non_empty_string) {
        changed |=
            turn.get("app_thread_id").and_then(Value::as_str) != Some(app_thread_id.as_str());
        set_string_value(turn, "app_thread_id", &app_thread_id);
    }
    if let Some(app_turn_id) = app_turn_id.and_then(non_empty_string) {
        changed |= turn.get("app_turn_id").and_then(Value::as_str) != Some(app_turn_id.as_str());
        set_string_value(turn, "app_turn_id", &app_turn_id);
    }
    changed
}

fn latest_codex_app_thread_id(snapshot: &Value) -> Option<String> {
    snapshot
        .get("turns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .rev()
        .filter(|turn| turn.get("role").and_then(Value::as_str) == Some("assistant"))
        .find_map(|turn| {
            if turn_is_codex_app_thread_resume_failure(turn) {
                return Some(None);
            }
            turn.get("app_thread_id")
                .and_then(Value::as_str)
                .and_then(non_empty_string)
                .or_else(|| {
                    turn.get("details")
                        .and_then(Value::as_object)
                        .and_then(|details| details.get("thread_id"))
                        .and_then(Value::as_str)
                        .and_then(non_empty_string)
                })
                .map(Some)
        })
        .flatten()
}

fn turn_is_codex_app_thread_resume_failure(turn: &Value) -> bool {
    turn.get("status").and_then(Value::as_str) == Some("failed")
        && turn
            .get("error_code")
            .and_then(Value::as_str)
            .is_some_and(|error_code| {
                matches!(
                    error_code,
                    "codex_app_server_resume_failed" | "thread_resume_failed"
                )
            })
}

fn emit_assistant_turn_upsert(
    snapshot: &Value,
    assistant_turn_id: &str,
    emitted_payloads: &mut Vec<Value>,
) {
    if let Some(turn) = find_turn(snapshot, assistant_turn_id) {
        emitted_payloads.push(build_turn_upsert_payload(snapshot, turn));
    }
}

fn finalize_agent_turn_output(
    snapshot: &mut Value,
    assistant_turn_id: &str,
    chat_mode: &str,
    final_assistant_text: Option<&str>,
    token_usage: Option<Value>,
    token_usage_breakdown: Option<Value>,
    thread_resume_failure: Option<&AgentThreadResumeFailure>,
    buffered_plan_assistant_event: Option<&TurnStreamEvent>,
    emitted_payloads: &mut Vec<Value>,
) {
    let token_usage_changed = token_usage
        .as_ref()
        .map(|token_usage| {
            apply_assistant_turn_token_usage(snapshot, assistant_turn_id, token_usage)
        })
        .unwrap_or(false);
    let token_usage_breakdown_changed = token_usage_breakdown
        .as_ref()
        .map(|token_usage_breakdown| {
            apply_assistant_turn_token_usage_breakdown(
                snapshot,
                assistant_turn_id,
                token_usage_breakdown,
            )
        })
        .unwrap_or(false);
    let usage_changed = token_usage_changed || token_usage_breakdown_changed;

    if let Some(failure) = thread_resume_failure {
        let now = iso_now();
        fail_assistant_turn_and_segments(
            snapshot,
            assistant_turn_id,
            &failure.message,
            failure.error_code.as_deref(),
            failure.details.as_ref(),
            usage_changed,
            emitted_payloads,
        );
        append_workflow_event(
            snapshot,
            json!({
                "message": failure.message,
                "timestamp": now,
                "kind": "continuity_reset",
                "error_code": failure.error_code,
                "details": failure.details,
            }),
        );
        return;
    }

    if let Some((message, error_code, details)) =
        failed_assistant_state(snapshot, assistant_turn_id)
    {
        fail_assistant_turn_and_segments(
            snapshot,
            assistant_turn_id,
            &message,
            error_code.as_deref(),
            details.as_ref(),
            usage_changed,
            emitted_payloads,
        );
        return;
    }

    let mut plan_segment = finalized_plan_segment(snapshot, assistant_turn_id);
    if let Some(segment) = plan_segment.as_ref() {
        if let Some(updated_segment) =
            persist_proposed_plan_artifact(snapshot, assistant_turn_id, segment)
        {
            emitted_payloads.push(build_segment_upsert_payload(snapshot, &updated_segment));
            plan_segment = Some(updated_segment);
        }
    }
    let mut final_answer_segment = finalized_assistant_segment(snapshot, assistant_turn_id);
    let final_text = final_assistant_text.and_then(non_empty_string);
    if final_answer_segment.is_none()
        && plan_segment.is_none()
        && final_text.is_none()
        && has_pending_request_user_input_segment(snapshot, assistant_turn_id)
    {
        if usage_changed {
            emit_assistant_turn_upsert(snapshot, assistant_turn_id, emitted_payloads);
        }
        return;
    }
    if final_answer_segment.is_none() {
        let fallback_text = if chat_mode == "plan" {
            final_text
        } else {
            final_text.clone()
        };
        if let Some(text) = fallback_text {
            final_answer_segment = fallback_assistant_segment_id(snapshot, assistant_turn_id)
                .and_then(|segment_id| {
                    complete_existing_assistant_segment_with_text(snapshot, &segment_id, &text)
                });
            if final_answer_segment.is_none() {
                let mut event =
                    buffered_plan_assistant_event
                        .cloned()
                        .unwrap_or_else(|| TurnStreamEvent {
                            kind: TurnStreamEventKind::ContentCompleted,
                            channel: Some(TurnStreamChannel::Assistant),
                            source: TurnStreamSource::default(),
                            content_delta: Some(text.clone()),
                            message: Some(text.clone()),
                            tool_call: None,
                            request_user_input: None,
                            token_usage: None,
                            error: None,
                            error_code: None,
                            details: None,
                            phase: Some("final_answer".to_string()),
                            status: None,
                        });
                event.content_delta = Some(text);
                final_answer_segment =
                    materialize_segment_for_event(snapshot, assistant_turn_id, &event);
            }
            if let Some(segment) = final_answer_segment.as_ref() {
                emitted_payloads.push(build_segment_upsert_payload(snapshot, segment));
            }
        }
    }

    if final_answer_segment.is_none() && plan_segment.is_none() {
        let (message, error_code, details) = failed_assistant_state(snapshot, assistant_turn_id)
            .unwrap_or_else(|| (MISSING_FINAL_ANSWER_ERROR.to_string(), None, None));
        fail_assistant_turn_and_segments(
            snapshot,
            assistant_turn_id,
            &message,
            error_code.as_deref(),
            details.as_ref(),
            false,
            emitted_payloads,
        );
        return;
    }

    let resolved_content = final_answer_segment
        .as_ref()
        .and_then(|segment| segment.get("content"))
        .and_then(Value::as_str)
        .and_then(non_empty_string)
        .or_else(|| {
            plan_segment
                .as_ref()
                .and_then(|segment| segment.get("content"))
                .and_then(Value::as_str)
                .and_then(non_empty_string)
        })
        .unwrap_or_default();

    if let Some(turn) = find_turn_mut(snapshot, assistant_turn_id) {
        let turn_changed = turn.get("content").and_then(Value::as_str)
            != Some(resolved_content.as_str())
            || turn.get("status").and_then(Value::as_str) != Some("complete")
            || turn.get("error").is_some()
            || turn.get("error_code").is_some()
            || turn.get("details").is_some()
            || usage_changed;
        set_string_value(turn, "content", &resolved_content);
        set_string_value(turn, "status", "complete");
        remove_key(turn, "error");
        remove_key(turn, "error_code");
        remove_key(turn, "details");
        if turn_changed {
            let turn = turn.clone();
            emitted_payloads.push(build_turn_upsert_payload(snapshot, &turn));
        }
    }
}

fn has_pending_request_user_input_segment(snapshot: &Value, assistant_turn_id: &str) -> bool {
    snapshot
        .get("segments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|segment| {
            segment.get("turn_id").and_then(Value::as_str) == Some(assistant_turn_id)
                && segment.get("kind").and_then(Value::as_str) == Some("request_user_input")
                && segment.get("status").and_then(Value::as_str) == Some("pending")
        })
}

fn append_workflow_event(snapshot: &mut Value, event: Value) {
    if let Some(events) = snapshot
        .as_object_mut()
        .and_then(|object| object.get_mut("event_log"))
        .and_then(Value::as_array_mut)
    {
        events.push(event);
    } else if let Some(object) = snapshot.as_object_mut() {
        object.insert("event_log".to_string(), json!([event]));
    }
}

fn agent_history_from_snapshot(snapshot: &Value, excluded_turn_ids: &[&str]) -> Vec<HistoryTurn> {
    snapshot
        .get("turns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|turn| agent_history_turn_from_persisted_turn(turn, excluded_turn_ids))
        .collect()
}

fn agent_history_turn_from_persisted_turn(
    turn: &Value,
    excluded_turn_ids: &[&str],
) -> Option<HistoryTurn> {
    if turn
        .get("id")
        .and_then(Value::as_str)
        .map(|turn_id| excluded_turn_ids.contains(&turn_id))
        .unwrap_or(false)
    {
        return None;
    }
    let kind = turn
        .get("kind")
        .and_then(Value::as_str)
        .and_then(non_empty_string)
        .unwrap_or_else(|| "message".to_string());
    if !kind.eq_ignore_ascii_case("message") {
        return None;
    }
    let status = turn
        .get("status")
        .and_then(Value::as_str)
        .and_then(non_empty_string)
        .unwrap_or_else(|| "complete".to_string());
    if !status.eq_ignore_ascii_case("complete") {
        return None;
    }
    let content = turn
        .get("content")
        .and_then(Value::as_str)
        .and_then(non_empty_string)?;
    let timestamp = history_turn_timestamp(turn);
    match turn.get("role").and_then(Value::as_str) {
        Some(role) if role.eq_ignore_ascii_case("user") => {
            let mut user_turn = UserTurn::new(content);
            user_turn.timestamp = timestamp;
            Some(HistoryTurn::User(user_turn))
        }
        Some(role) if role.eq_ignore_ascii_case("assistant") => {
            let mut assistant_turn = AssistantTurn::new(content);
            assistant_turn.timestamp = timestamp;
            Some(HistoryTurn::Assistant(assistant_turn))
        }
        _ => None,
    }
}

fn history_turn_timestamp(turn: &Value) -> OffsetDateTime {
    turn.get("timestamp")
        .and_then(Value::as_str)
        .and_then(|timestamp| {
            OffsetDateTime::parse(timestamp, &time::format_description::well_known::Rfc3339).ok()
        })
        .unwrap_or_else(OffsetDateTime::now_utc)
}

fn build_turn_upsert_payload(snapshot: &Value, turn: &Value) -> Value {
    json!({
        "type": "turn_upsert",
        "revision": snapshot.get("revision").and_then(Value::as_i64).unwrap_or(0),
        "conversation_id": snapshot.get("conversation_id").and_then(Value::as_str).unwrap_or(""),
        "project_path": snapshot.get("project_path").and_then(Value::as_str).unwrap_or(""),
        "title": snapshot.get("title").and_then(Value::as_str).unwrap_or("New thread"),
        "updated_at": snapshot.get("updated_at").and_then(Value::as_str).unwrap_or(""),
        "turn": turn,
    })
}

fn build_segment_upsert_payload(snapshot: &Value, segment: &Value) -> Value {
    json!({
        "type": "segment_upsert",
        "revision": snapshot.get("revision").and_then(Value::as_i64).unwrap_or(0),
        "conversation_id": snapshot.get("conversation_id").and_then(Value::as_str).unwrap_or(""),
        "project_path": snapshot.get("project_path").and_then(Value::as_str).unwrap_or(""),
        "title": snapshot.get("title").and_then(Value::as_str).unwrap_or("New thread"),
        "updated_at": snapshot.get("updated_at").and_then(Value::as_str).unwrap_or(""),
        "segment": segment,
    })
}

fn build_conversation_snapshot_payload(snapshot: &Value) -> Value {
    json!({
        "type": "conversation_snapshot",
        "revision": snapshot.get("revision").and_then(Value::as_i64).unwrap_or(0),
        "state": snapshot,
    })
}

fn stamp_progress_payloads_with_state_revision(snapshot: &mut Value, payloads: &mut [Value]) {
    if payloads.is_empty() {
        return;
    }
    let base_revision = snapshot
        .get("revision")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let updated_at = snapshot
        .get("updated_at")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let final_revision = base_revision + payloads.len() as i64 - 1;
    for (index, payload) in payloads.iter_mut().enumerate() {
        if let Some(object) = payload.as_object_mut() {
            object.insert("revision".to_string(), json!(base_revision + index as i64));
            object.insert("updated_at".to_string(), json!(updated_at));
            if object.get("type").and_then(Value::as_str) == Some("conversation_snapshot") {
                if let Some(state_object) = object.get_mut("state").and_then(Value::as_object_mut) {
                    state_object.insert("revision".to_string(), json!(final_revision));
                    state_object.insert("updated_at".to_string(), json!(updated_at));
                }
            }
        }
    }
    if let Some(object) = snapshot.as_object_mut() {
        object.insert("revision".to_string(), json!(final_revision));
    }
}

fn append_events(
    repository: &ConversationRepository,
    conversation_id: &str,
    project_path: &str,
    events: &[Value],
) -> WorkspaceResult<()> {
    for event in events {
        repository.append_conversation_event(conversation_id, project_path, event)?;
    }
    Ok(())
}

fn persist_snapshot_with_payloads(
    repository: &ConversationRepository,
    conversation_id: &str,
    project_path: &str,
    snapshot: &mut Value,
    emitted_payloads: &mut Vec<Value>,
) -> WorkspaceResult<()> {
    touch_snapshot(repository, snapshot, conversation_id, project_path)?;
    emitted_payloads.push(build_conversation_snapshot_payload(snapshot));
    stamp_progress_payloads_with_state_revision(snapshot, emitted_payloads);
    repository.write_snapshot(snapshot)?;
    append_events(repository, conversation_id, project_path, emitted_payloads)
}

fn append_mode_change_turn(snapshot: &mut Value, chat_mode: &str) -> Value {
    let now = iso_now();
    let turn = json!({
        "id": format!("turn-{}", uuid::Uuid::new_v4().simple()),
        "role": "system",
        "content": chat_mode,
        "timestamp": now,
        "status": "complete",
        "kind": "mode_change",
    });
    if let Some(turns) = snapshot
        .as_object_mut()
        .and_then(|object| object.get_mut("turns"))
        .and_then(Value::as_array_mut)
    {
        turns.push(turn.clone());
    } else if let Some(object) = snapshot.as_object_mut() {
        object.insert("turns".to_string(), json!([turn.clone()]));
    }
    turn
}

fn conversation_summary_from_snapshot(
    snapshot: &Value,
    fallback_conversation_id: &str,
    expected_project_path: &str,
) -> Option<ConversationSummary> {
    let payload = snapshot.as_object()?;
    if payload.get("schema_version").and_then(Value::as_i64)? != CONVERSATION_STATE_SCHEMA_VERSION {
        return None;
    }
    let revision = payload.get("revision").and_then(Value::as_i64)?;
    payload.get("segments").and_then(Value::as_array)?;
    let project_path = payload
        .get("project_path")
        .and_then(Value::as_str)
        .and_then(normalize_project_path_string)?;
    if project_path != expected_project_path {
        return None;
    }
    let turns = payload.get("turns").and_then(Value::as_array);
    let conversation_id = payload
        .get("conversation_id")
        .and_then(Value::as_str)
        .and_then(non_empty_string)
        .unwrap_or_else(|| fallback_conversation_id.to_string());
    let conversation_handle = payload
        .get("conversation_handle")
        .and_then(Value::as_str)
        .and_then(non_empty_string)
        .unwrap_or_default();
    let created_at = payload
        .get("created_at")
        .and_then(Value::as_str)
        .and_then(non_empty_string)
        .or_else(|| first_turn_timestamp(turns))
        .unwrap_or_else(iso_now);
    let updated_at = payload
        .get("updated_at")
        .and_then(Value::as_str)
        .and_then(non_empty_string)
        .or_else(|| last_turn_timestamp(turns))
        .unwrap_or_else(|| created_at.clone());
    let title = payload
        .get("title")
        .and_then(Value::as_str)
        .and_then(non_empty_string)
        .unwrap_or_else(|| derive_conversation_title(turns));
    let last_message_preview = build_conversation_preview(turns);
    Some(ConversationSummary {
        conversation_id,
        conversation_handle,
        project_path,
        title,
        created_at,
        updated_at,
        revision,
        last_message_preview,
    })
}

fn truncate_tool_call_outputs(snapshot: &mut Value) {
    let Some(segments) = snapshot
        .as_object_mut()
        .and_then(|object| object.get_mut("segments"))
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for segment in segments {
        let Some(tool_call) = segment
            .as_object_mut()
            .and_then(|segment| segment.get_mut("tool_call"))
            .and_then(Value::as_object_mut)
        else {
            continue;
        };
        let Some(output) = tool_call
            .get("output")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        let output_size = output.len();
        let (preview, truncated) = truncate_utf8(&output, UI_TOOL_OUTPUT_PREVIEW_BYTES);
        tool_call.insert("output".to_string(), json!(preview));
        tool_call.insert("output_size".to_string(), json!(output_size));
        tool_call.insert("output_truncated".to_string(), json!(truncated));
    }
}

fn truncate_utf8(value: &str, byte_limit: usize) -> (String, bool) {
    if value.len() <= byte_limit {
        return (value.to_string(), false);
    }
    let mut end = 0;
    for (index, ch) in value.char_indices() {
        let next = index + ch.len_utf8();
        if next > byte_limit {
            break;
        }
        end = next;
    }
    (value[..end].to_string(), true)
}

fn derive_conversation_title(turns: Option<&Vec<Value>>) -> String {
    turns
        .into_iter()
        .flatten()
        .find_map(|turn| {
            let object = turn.as_object()?;
            let kind = turn_kind(object);
            let role = object.get("role").and_then(Value::as_str);
            if kind == "message" && role == Some("user") {
                object
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|value| truncate_text(value, 64))
            } else {
                None
            }
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "New thread".to_string())
}

fn build_conversation_preview(turns: Option<&Vec<Value>>) -> Option<String> {
    turns.into_iter().flatten().rev().find_map(|turn| {
        let object = turn.as_object()?;
        if turn_kind(object) != "message" {
            return None;
        }
        object
            .get("content")
            .and_then(Value::as_str)
            .map(|value| truncate_text(value, 120))
            .filter(|value| !value.is_empty())
    })
}

fn first_turn_timestamp(turns: Option<&Vec<Value>>) -> Option<String> {
    turns.into_iter().flatten().find_map(|turn| {
        turn.get("timestamp")
            .and_then(Value::as_str)
            .and_then(non_empty_string)
    })
}

fn last_turn_timestamp(turns: Option<&Vec<Value>>) -> Option<String> {
    turns.into_iter().flatten().rev().find_map(|turn| {
        turn.get("timestamp")
            .and_then(Value::as_str)
            .and_then(non_empty_string)
    })
}

fn turn_kind(turn: &Map<String, Value>) -> String {
    turn.get("kind")
        .and_then(Value::as_str)
        .and_then(non_empty_string)
        .unwrap_or_else(|| "message".to_string())
}

fn truncate_text(value: &str, limit: usize) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= limit {
        return collapsed;
    }
    let mut truncated = collapsed
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>();
    truncated = truncated.trim_end().to_string();
    truncated.push('\u{2026}');
    truncated
}

fn proposed_plan_title(content: &str) -> String {
    content
        .lines()
        .find_map(|line| {
            let trimmed = line.trim_start();
            let title = trimmed.strip_prefix("# ")?;
            let title = strip_markdown_heading(title);
            (!title.is_empty()).then_some(title)
        })
        .unwrap_or_else(|| "Proposed Plan".to_string())
}

fn strip_markdown_heading(value: &str) -> String {
    let mut output = String::new();
    let mut in_link_label = false;
    let mut skip_link_target = false;
    for ch in value.chars() {
        if skip_link_target {
            if ch == ')' {
                skip_link_target = false;
            }
            continue;
        }
        match ch {
            '[' => in_link_label = true,
            ']' if in_link_label => in_link_label = false,
            '(' if !in_link_label => skip_link_target = true,
            '*' | '_' | '~' | '`' | '#' | '>' | '!' => {}
            other => output.push(other),
        }
    }
    output.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn ensure_flow_exists(settings: &SparkSettings, flow_name: &str) -> WorkspaceResult<()> {
    let flow_name = non_empty_string(flow_name)
        .ok_or_else(|| WorkspaceError::Validation("Flow name is required.".to_string()))?;
    match attractor_api::read_named_flow_source(&settings.flows_dir, &flow_name) {
        Ok(_) => Ok(()),
        Err(error) if error.status_code() == 404 => Err(WorkspaceError::NotFound(format!(
            "Unknown flow: {flow_name}"
        ))),
        Err(error) if error.status_code() == 400 => {
            Err(WorkspaceError::Validation(error.detail().to_string()))
        }
        Err(error) => Err(WorkspaceError::Internal(error.detail().to_string())),
    }
}

fn normalize_launch_context_value(
    value: Option<&Value>,
    source_name: &str,
) -> WorkspaceResult<Option<BTreeMap<String, Value>>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(object) = value.as_object() else {
        return Err(WorkspaceError::Validation(format!(
            "{source_name} launch_context must be an object."
        )));
    };
    if object.is_empty() {
        return Ok(None);
    }
    let mut normalized = BTreeMap::new();
    for (key, entry) in object {
        if !key.starts_with("context.") {
            return Err(WorkspaceError::Validation(format!(
                "{source_name} launch_context key must use the context.* namespace: {key}"
            )));
        }
        normalized.insert(key.clone(), entry.clone());
    }
    Ok(Some(normalized))
}

fn normalize_review_disposition(value: &str, message: &str) -> WorkspaceResult<String> {
    let normalized = value.trim().to_lowercase();
    if matches!(normalized.as_str(), "approved" | "rejected") {
        Ok(normalized)
    } else {
        Err(WorkspaceError::Validation(message.to_string()))
    }
}

fn random_artifact_id(prefix: &str) -> String {
    let hex = uuid::Uuid::new_v4().simple().to_string();
    format!("{prefix}-{}", &hex[..12])
}

fn write_change_request(
    project_root: &Path,
    title: &str,
    content: &str,
    created_at: &str,
) -> WorkspaceResult<(String, PathBuf)> {
    let changes_dir = project_root.join("changes");
    fs::create_dir_all(&changes_dir).map_err(|error| {
        WorkspaceError::Internal(format!(
            "Unable to create change request directory {}: {error}",
            changes_dir.display()
        ))
    })?;
    let year = if created_at.len() >= 4 && created_at[..4].chars().all(|ch| ch.is_ascii_digit()) {
        &created_at[..4]
    } else {
        "0000"
    };
    let mut max_sequence = 0_i64;
    if let Ok(entries) = fs::read_dir(&changes_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if let Some(sequence) = change_request_sequence(name, year) {
                max_sequence = max_sequence.max(sequence);
            }
        }
    }
    let slug = slugify_change_request_title(title);
    let mut sequence = max_sequence + 1;
    loop {
        let change_request_id = format!("CR-{year}-{sequence:04}-{slug}");
        let candidate = changes_dir.join(&change_request_id);
        if !candidate.exists() {
            fs::create_dir_all(&candidate).map_err(|error| {
                WorkspaceError::Internal(format!(
                    "Unable to create change request directory {}: {error}",
                    candidate.display()
                ))
            })?;
            let request_path = candidate.join("request.md");
            fs::write(&request_path, format!("{}\n", content.trim_end())).map_err(|error| {
                WorkspaceError::Internal(format!(
                    "Unable to write change request {}: {error}",
                    request_path.display()
                ))
            })?;
            return Ok((change_request_id, request_path));
        }
        sequence += 1;
    }
}

fn change_request_sequence(name: &str, year: &str) -> Option<i64> {
    let rest = name.strip_prefix("CR-")?;
    let (entry_year, rest) = rest.split_once('-')?;
    if entry_year != year {
        return None;
    }
    let sequence = rest.get(..4)?;
    if !sequence.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    sequence.parse::<i64>().ok()
}

fn slugify_change_request_title(value: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in strip_markdown_heading(value)
        .chars()
        .flat_map(char::to_lowercase)
    {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_dash = false;
        } else if !last_dash && !slug.is_empty() {
            slug.push('-');
            last_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "change-request".to_string()
    } else {
        slug
    }
}

fn relative_project_path(project_root: &Path, path: &Path) -> String {
    path.strip_prefix(project_root)
        .unwrap_or(path)
        .components()
        .collect::<PathBuf>()
        .to_string_lossy()
        .replace('\\', "/")
}

fn absolute_path_string(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn validate_chat_mode(value: &str) -> WorkspaceResult<String> {
    let normalized = value.trim().to_lowercase();
    if normalized == "chat" || normalized == "plan" {
        Ok(normalized)
    } else {
        Err(WorkspaceError::Validation(
            "Chat mode must be 'chat' or 'plan'.".to_string(),
        ))
    }
}

fn normalize_chat_mode(value: &str) -> String {
    let normalized = value.trim().to_lowercase();
    if normalized == "plan" {
        "plan".to_string()
    } else {
        "chat".to_string()
    }
}

fn validate_provider(value: &str) -> WorkspaceResult<String> {
    let normalized = value.trim().to_lowercase();
    let normalized = if normalized.is_empty() {
        "codex".to_string()
    } else {
        normalized
    };
    match normalized.as_str() {
        "codex" | "openai" | "anthropic" | "gemini" | "openrouter" | "litellm"
        | "openai_compatible" => Ok(normalized),
        _ => Err(WorkspaceError::Validation(
            "Provider must be blank or one of: codex, openai, anthropic, gemini, openrouter, litellm, openai_compatible."
                .to_string(),
        )),
    }
}

fn validate_reasoning_effort(value: &str) -> WorkspaceResult<String> {
    let normalized = value.trim().to_lowercase();
    if normalized.is_empty() || matches!(normalized.as_str(), "low" | "medium" | "high" | "xhigh") {
        Ok(normalized)
    } else {
        Err(WorkspaceError::Validation(
            "Reasoning effort must be blank or one of: low, medium, high, xhigh.".to_string(),
        ))
    }
}

fn normalize_project_path_or_400(project_path: &str) -> WorkspaceResult<String> {
    normalize_project_path(project_path)
        .map_err(|error| WorkspaceError::Validation(error.to_string()))?
        .map(|path| path.to_string_lossy().into_owned())
        .ok_or_else(|| WorkspaceError::Validation("Project path is required.".to_string()))
}

fn normalize_optional_project_path(value: Option<&str>) -> WorkspaceResult<Option<String>> {
    match value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    }) {
        Some(value) => normalize_project_path(value)
            .map_err(|error| WorkspaceError::Validation(error.to_string()))
            .map(|value| value.map(|path| path.to_string_lossy().into_owned())),
        None => Ok(None),
    }
}

fn normalize_project_path_string(value: &str) -> Option<String> {
    normalize_project_path(value)
        .ok()
        .flatten()
        .map(|path| path.to_string_lossy().into_owned())
}

fn set_string(snapshot: &mut Value, key: &str, value: &str) {
    if let Some(object) = snapshot.as_object_mut() {
        object.insert(key.to_string(), json!(value));
    }
}

fn set_optional_string(snapshot: &mut Value, key: &str, value: Option<&str>) {
    if let Some(object) = snapshot.as_object_mut() {
        object.insert(
            key.to_string(),
            value.map(|value| json!(value)).unwrap_or(Value::Null),
        );
    }
}

fn non_empty_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
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
