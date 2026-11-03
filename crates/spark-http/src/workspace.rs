use std::collections::BTreeSet;
use std::convert::Infallible;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::rejection::{BytesRejection, JsonRejection, QueryRejection};
use axum::extract::{DefaultBodyLimit, Path as AxumPath, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use spark_agent_adapter::AgentTurnBackend;
use spark_common::settings::SparkSettings;
use spark_storage::ConversationStore;
use spark_storage::{DeletedProjectRecord, ProjectRecord};
use spark_workspace::conversations::{
    ConversationDeleteResponse, ConversationRequestUserInputAnswerRequest,
    ConversationSettingsUpdate, ConversationTurnRequest, FlowRunRequestCreateByHandleRequest,
    FlowRunRequestReviewRequest, PreparedConversationTurn, ProposedPlanReviewRequest,
    RunContinueRequest, RunLaunchRequest, RunRetryRequest, TurnAcceptance,
    WorkspaceConversationService,
};
use spark_workspace::live::{
    acceptance_wire, conversation_changes_envelope, conversation_event_envelope,
    conversation_window_envelope, conversations_changes_envelope, conversations_snapshot_envelope,
    initial_live_envelopes, lagged_resync_envelopes, latest_run_sequence, mutation_ack_wire,
    read_conversation_changes, read_conversation_page, read_conversation_summaries,
    trigger_delete_envelope, trigger_upsert_envelope, validate_live_query, ConversationReadOutcome,
    LiveEnvelope, RawLiveQuery,
};
use spark_workspace::missions::{MissionMutation, MissionRecord};
use spark_workspace::projects::{ProjectRegistrationRequest, ProjectStateUpdate};
use spark_workspace::WorkspaceError;
use spark_workspace::{
    workspace_settings, BrowseResponse, ProjectMetadata, SerializedTrigger, TriggerCreateRequest,
    TriggerDeleteResponse, TriggerUpdateRequest, WebhookHandleRequest, WorkspaceFlowService,
    WorkspaceFlowSummary, WorkspaceProjectService, WorkspaceTriggerService,
};
use tokio::time::{self, Duration};

use crate::{
    publish_live_run_after, publish_trigger_activation_outcomes, HttpAppState, Received,
    RunEventObserverHandle, WorkspaceApiError, WorkspaceLiveHub,
};

type ApiResult<T> = std::result::Result<Json<T>, WorkspaceApiError>;

fn conversation_service(
    settings: &SparkSettings,
    store: &Arc<ConversationStore>,
    runtime_handler_runner_factory: &attractor_api::RuntimeHandlerRunnerFactory,
    agent_turn_backend: &Arc<dyn AgentTurnBackend>,
    run_event_observer: &RunEventObserverHandle,
) -> WorkspaceConversationService {
    let service =
        WorkspaceConversationService::new_with_runtime_handler_runner_factory_and_agent_turn_backend(
            settings.clone(),
            store.clone(),
            runtime_handler_runner_factory.clone(),
            agent_turn_backend.clone(),
        );
    match &run_event_observer.0 {
        Some(observer) => service.with_run_event_observer(observer.clone()),
        None => service,
    }
}

fn live_publisher(
    live_hub: &Arc<WorkspaceLiveHub>,
) -> Arc<dyn Fn(spark_workspace::LiveEnvelope) + Send + Sync> {
    let live_hub = live_hub.clone();
    Arc::new(move |envelope| live_hub.publish(envelope))
}

pub fn router() -> Router<HttpAppState> {
    Router::new()
        .nest("/claude", crate::claude_auth::router())
        .nest("/codex", crate::codex_auth::router())
        .route("/missions", get(list_missions).post(create_mission))
        .route(
            "/missions/{mission_id}",
            get(get_mission).patch(update_mission),
        )
        .route("/missions/{mission_id}/events", post(post_mission_event))
        .route("/missions/{mission_id}/{control}", post(control_mission))
        .route("/roles", get(list_roles))
        .route("/roles/{name}/{control}", post(control_role))
        .route("/org", get(list_org))
        .route("/org/item", get(get_org_item))
        .route("/org/history", get(get_org_history))
        .route("/org/memory", get(get_org_memory))
        .route("/playbooks", get(list_playbooks))
        .route("/playbooks/{name}", get(get_playbook))
        .route("/projects", get(list_projects).delete(delete_project))
        .route("/projects/register", post(register_project))
        .route("/projects/state", patch(update_project_state))
        .route("/projects/conversations", get(list_project_conversations))
        .route("/projects/metadata", get(project_metadata))
        .route("/projects/browse", get(browse_project_directories))
        .route("/projects/chat-models", get(project_chat_models))
        .route("/triggers", get(list_triggers).post(create_trigger))
        .route("/webhooks", post(post_trigger_webhook))
        .route("/live/events", get(workspace_live_events))
        .route(
            "/triggers/{trigger_id}",
            get(get_trigger)
                .patch(update_trigger)
                .delete(delete_trigger),
        )
        .route("/attention", get(pending_attention))
        .route("/flows", get(list_workspace_flows))
        .route("/flows/{*flow_path}", get(get_workspace_flow_dispatch))
        .route(
            "/conversations/{conversation_id}/tool-output/{segment_id}",
            get(get_conversation_segment_tool_output),
        )
        .route(
            "/conversations/{conversation_id}/attachments",
            post(upload_conversation_attachment).layer(DefaultBodyLimit::max(
                spark_workspace::attachments::MAX_ATTACHMENT_BYTES + 1,
            )),
        )
        .route(
            "/conversations/{conversation_id}/attachments/{attachment_id}",
            get(download_conversation_attachment).delete(delete_conversation_attachment),
        )
        .route(
            "/conversations/{conversation_id}/interrupt",
            post(interrupt_conversation_turn),
        )
        .route(
            "/conversations/{conversation_id}/turns",
            post(send_conversation_turn),
        )
        .route(
            "/conversations/{conversation_id}/turns/{turn_id}/retry",
            post(retry_conversation_turn),
        )
        .route(
            "/conversations/{conversation_id}/request-user-input/{request_id}/answer",
            post(answer_conversation_request_user_input),
        )
        .route(
            "/conversations/by-handle/{conversation_handle}/flow-run-requests",
            post(create_flow_run_request_by_handle),
        )
        .route(
            "/conversations/{conversation_id}/flow-run-requests/{request_id}/review",
            post(review_flow_run_request),
        )
        .route(
            "/conversations/{conversation_id}/proposed-plans/{plan_id}/review",
            post(review_proposed_plan),
        )
        .route("/runs/launch", post(launch_workspace_run))
        .route("/runs/{run_id}/retry", post(retry_workspace_run))
        .route("/runs/{run_id}/continue", post(continue_workspace_run))
        .route(
            "/conversations/{conversation_id}/settings",
            put(update_conversation_settings),
        )
        .route(
            "/conversations/{conversation_id}",
            get(get_conversation).delete(delete_conversation),
        )
        .route(
            "/conversations/{conversation_id}/changes",
            get(get_conversation_changes),
        )
        .route("/settings", get(settings).patch(patch_settings))
        .route("/settings/validate", post(validate_settings_update))
}

#[derive(Debug, Deserialize)]
struct DeleteProjectQuery {
    project_path: String,
}

#[derive(Debug, Deserialize)]
struct ProjectConversationsQuery {
    project_path: String,
}

#[derive(Debug, Default, Deserialize)]
struct ConversationQuery {
    project_path: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ConversationPageQuery {
    project_path: Option<String>,
    limit: Option<String>,
    before: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ConversationChangesQuery {
    project_path: Option<String>,
    generation: Option<String>,
    since: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AttachmentUploadQuery {
    #[serde(default)]
    project_path: String,
    #[serde(default)]
    filename: String,
}

#[derive(Debug, Deserialize)]
struct DeleteConversationQuery {
    project_path: String,
}

#[derive(Debug, Deserialize)]
struct ProjectMetadataQuery {
    directory: String,
}

#[derive(Debug, Deserialize)]
struct BrowseQuery {
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatModelsQuery {
    project_path: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct FlowSurfaceQuery {
    project_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProjectStateUpdateBody {
    expected_revision: Option<String>,
    project_path: String,
    #[serde(default)]
    is_favorite: Option<Option<bool>>,
    #[serde(default)]
    last_accessed_at: Option<Option<String>>,
    #[serde(default)]
    active_conversation_id: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "spark_common::settings::deserialize_nullable_patch"
    )]
    execution_profile_id: Option<Option<String>>,
}

async fn list_projects(
    State(settings): State<Arc<SparkSettings>>,
) -> ApiResult<Vec<ProjectRecord>> {
    WorkspaceProjectService::new((*settings).clone())
        .list_projects()
        .map(Json)
        .map_err(Into::into)
}

async fn register_project(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    payload: Result<Json<ProjectRegistrationRequest>, JsonRejection>,
) -> ApiResult<ProjectRecord> {
    let request = json_payload(payload)?;
    let changes_settings = request.execution_profile_id.is_some();
    let record =
        WorkspaceProjectService::new((*settings).clone()).register_project(&store, request)?;
    if changes_settings {
        live_hub.publish_settings_change(
            "project",
            "execution",
            Some(record.project_path.clone()),
            Value::Null,
        );
    }
    Ok(Json(record))
}

async fn update_project_state(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    payload: Result<Json<ProjectStateUpdateBody>, JsonRejection>,
) -> ApiResult<ProjectRecord> {
    let request = json_payload(payload)?;
    let is_favorite = request.is_favorite.map(|value| value.unwrap_or(false));
    let changes_settings = request.execution_profile_id.is_some();
    let record = WorkspaceProjectService::new((*settings).clone()).update_project_state(
        &store,
        ProjectStateUpdate {
            expected_revision: request.expected_revision,
            project_path: request.project_path,
            last_accessed_at: request.last_accessed_at,
            is_favorite,
            active_conversation_id: request.active_conversation_id,
            execution_profile_id: request.execution_profile_id,
        },
    )?;
    if changes_settings {
        live_hub.publish_settings_change(
            "project",
            "execution",
            Some(record.project_path.clone()),
            Value::Null,
        );
    }
    Ok(Json(record))
}

async fn delete_project(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    State(agent_turn_backend): State<Arc<dyn AgentTurnBackend>>,
    payload: Result<Query<DeleteProjectQuery>, QueryRejection>,
) -> ApiResult<DeletedProjectRecordWithStatus> {
    let query = query_payload(payload)?;
    let conversations = WorkspaceConversationService::new_with_agent_turn_backend(
        (*settings).clone(),
        store,
        agent_turn_backend,
    );
    let deleted = tokio::task::spawn_blocking(move || {
        WorkspaceProjectService::new((*settings).clone())
            .delete_project(&conversations, &query.project_path)
    })
    .await
    .map_err(|error| WorkspaceError::Internal(format!("project deletion failed: {error}")))??;
    Ok(Json(DeletedProjectRecordWithStatus::from(deleted)))
}

async fn list_project_conversations(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    payload: Result<Query<ConversationQuery>, QueryRejection>,
) -> ApiResult<Value> {
    let query = optional_query_payload(payload)?;
    tokio::task::spawn_blocking(move || {
        read_conversation_summaries(&settings, &store, query.project_path.as_deref())
    })
    .await
    .map_err(|error| WorkspaceError::Internal(format!("conversation list task failed: {error}")))?
    .map(Json)
    .map_err(Into::into)
}

/// A conversation read's rows, or the 409 `Reset` that invalidates the
/// browser's window or cursor.
fn conversation_read_response(outcome: ConversationReadOutcome) -> Response {
    match outcome {
        ConversationReadOutcome::Rows(rows) => Json(rows).into_response(),
        ConversationReadOutcome::Reset(reset) => {
            (StatusCode::CONFLICT, Json(reset)).into_response()
        }
    }
}

fn required_project_path(project_path: Option<String>) -> Result<String, WorkspaceError> {
    project_path
        .filter(|path| !path.trim().is_empty())
        .ok_or_else(|| WorkspaceError::Validation("project_path is required.".to_string()))
}

/// A decimal revision query parameter.
fn decimal_param(value: Option<&str>, field: &str) -> Result<i64, WorkspaceError> {
    value
        .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|value| value.parse::<i64>().ok())
        .ok_or_else(|| WorkspaceError::Validation(format!("{field} must be a decimal revision.")))
}

async fn get_conversation(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    AxumPath(conversation_id): AxumPath<String>,
    payload: Result<Query<ConversationPageQuery>, QueryRejection>,
) -> Result<Response, WorkspaceApiError> {
    let query = optional_query_payload(payload)?;
    let project_path = required_project_path(query.project_path)?;
    let limit = match query.limit.as_deref() {
        None => spark_workspace::live::DEFAULT_PAGE_LIMIT,
        Some(limit) => limit
            .parse::<usize>()
            .ok()
            .filter(|limit| (1..=spark_workspace::live::MAX_PAGE_LIMIT).contains(limit))
            .ok_or_else(|| {
                WorkspaceError::Validation("limit must be an integer from 1 to 100.".to_string())
            })?,
    };
    let outcome = tokio::task::spawn_blocking(move || {
        read_conversation_page(
            &settings,
            &store,
            &conversation_id,
            &project_path,
            limit,
            query.before.as_deref(),
        )
    })
    .await
    .map_err(|error| {
        WorkspaceError::Internal(format!("conversation read task failed: {error}"))
    })??;
    Ok(conversation_read_response(outcome))
}

async fn get_conversation_changes(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    AxumPath(conversation_id): AxumPath<String>,
    payload: Result<Query<ConversationChangesQuery>, QueryRejection>,
) -> Result<Response, WorkspaceApiError> {
    let query = optional_query_payload(payload)?;
    let project_path = required_project_path(query.project_path)?;
    let generation = query
        .generation
        .filter(|generation| !generation.is_empty())
        .ok_or_else(|| WorkspaceError::Validation("generation is required.".to_string()))?;
    let since = decimal_param(query.since.as_deref(), "since")?;
    let outcome = tokio::task::spawn_blocking(move || {
        read_conversation_changes(
            &settings,
            &store,
            &conversation_id,
            &project_path,
            &generation,
            since,
        )
    })
    .await
    .map_err(|error| {
        WorkspaceError::Internal(format!("conversation read task failed: {error}"))
    })??;
    Ok(conversation_read_response(outcome))
}

async fn update_conversation_settings(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    AxumPath(conversation_id): AxumPath<String>,
    payload: Result<Json<ConversationSettingsUpdate>, JsonRejection>,
) -> ApiResult<Value> {
    let request = json_payload(payload)?;
    let changes_models = request.model_settings.is_some()
        || request.provider.is_some()
        || request.llm_profile.is_some()
        || request.model.is_some()
        || request.reasoning_effort.is_some();
    let service = WorkspaceConversationService::new((*settings).clone(), store.clone());
    let ack = tokio::task::spawn_blocking(move || {
        let ack = service.update_conversation_settings(&conversation_id, request)?;
        mutation_ack_wire(&service, ack)
    })
    .await
    .map_err(|error| WorkspaceError::Internal(format!("settings update task failed: {error}")))??;
    if changes_models {
        if let Some(conversation) = ack["changes"]["rows"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|row| row["kind"] == "conversation" && !row["payload"].is_null())
        {
            live_hub.publish_settings_change(
                "conversation",
                "models",
                conversation["payload"]["project_path"]
                    .as_str()
                    .map(str::to_string),
                conversation["payload"]["settings"]["models"]["revision"].clone(),
            );
        }
    }
    Ok(Json(ack))
}

async fn send_conversation_turn(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    State(runtime_handler_runner_factory): State<attractor_api::RuntimeHandlerRunnerFactory>,
    State(agent_turn_backend): State<Arc<dyn AgentTurnBackend>>,
    State(run_event_observer): State<RunEventObserverHandle>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    AxumPath(conversation_id): AxumPath<String>,
    payload: Result<Json<ConversationTurnRequest>, JsonRejection>,
) -> ApiResult<Value> {
    let request = json_payload(payload)?;
    let project_path = request.project_path.clone();
    let service = conversation_service(
        &settings,
        &store,
        &runtime_handler_runner_factory,
        &agent_turn_backend,
        &run_event_observer,
    );
    run_conversation_turn(
        settings,
        live_hub,
        service,
        conversation_id,
        project_path,
        move |service, conversation_id| service.send_turn(conversation_id, request),
    )
    .await
}

async fn retry_conversation_turn(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    State(runtime_handler_runner_factory): State<attractor_api::RuntimeHandlerRunnerFactory>,
    State(agent_turn_backend): State<Arc<dyn AgentTurnBackend>>,
    State(run_event_observer): State<RunEventObserverHandle>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    AxumPath((conversation_id, turn_id)): AxumPath<(String, String)>,
    payload: Result<Json<ConversationProjectPathRequest>, JsonRejection>,
) -> ApiResult<Value> {
    let project_path = json_payload(payload)?.project_path;
    let service = conversation_service(
        &settings,
        &store,
        &runtime_handler_runner_factory,
        &agent_turn_backend,
        &run_event_observer,
    );
    let project_path_for_start = project_path.clone();
    run_conversation_turn(
        settings,
        live_hub,
        service,
        conversation_id,
        project_path,
        move |service, conversation_id| {
            service
                .retry_turn(conversation_id, &project_path_for_start, &turn_id)
                .map(|(prepared, acceptance)| (Some(prepared), acceptance))
        },
    )
    .await
}

/// Accepts a message, answers with its acceptance once committed, and
/// completes a started turn in the background. History, frame and provider
/// startup follow the acceptance. Committed progress reaches live clients as
/// revisions through the store's commit notices; only transient stream deltas
/// are published here. A message delivered into or queued behind a running
/// turn starts nothing.
async fn run_conversation_turn(
    settings: Arc<SparkSettings>,
    live_hub: Arc<WorkspaceLiveHub>,
    service: WorkspaceConversationService,
    conversation_id: String,
    project_path: String,
    start: impl FnOnce(
            &WorkspaceConversationService,
            &str,
        )
            -> Result<(Option<PreparedConversationTurn>, TurnAcceptance), WorkspaceError>
        + Send
        + 'static,
) -> ApiResult<Value> {
    let _ = project_path;
    let service = service.with_live_publisher(live_publisher(&live_hub));
    let service_for_start = service.clone();
    let (prepared, acceptance) = tokio::task::spawn_blocking(move || {
        let (prepared, acceptance) = start(&service_for_start, &conversation_id)?;
        Ok::<_, WorkspaceError>((prepared, acceptance_wire(&service_for_start, acceptance)?))
    })
    .await
    .map_err(|error| {
        WorkspaceApiError(WorkspaceError::Internal(format!(
            "conversation turn start task failed: {error}"
        )))
    })??;
    let Some(prepared) = prepared else {
        return Ok(Json(acceptance));
    };
    let completion_conversation_id = prepared.conversation_id.clone();
    let completion_project_path = prepared.project_path.clone();
    tokio::spawn(async move {
        // The running turn keeps the app's store open through its final
        // commit, even if the app itself is dropped meanwhile.
        let store = service.store().clone();
        let missions = mission_service(&settings, &store);
        let roles =
            spark_workspace::roles::WorkspaceRoleService::new((*settings).clone(), store.clone());
        let (progress_conversation, progress_project) = (
            completion_conversation_id.clone(),
            completion_project_path.clone(),
        );
        let _ = tokio::task::spawn_blocking(move || {
            let completed =
                service.complete_started_turn_with_progress_payloads(prepared, move |payload| {
                    live_hub.publish(conversation_event_envelope(
                        &progress_conversation,
                        &progress_project,
                        payload,
                    ));
                });
            // A mission's inbox may have filled while this turn ran.
            if missions
                .conversation_mission(&completion_project_path, &completion_conversation_id)
                .is_some()
            {
                let _ = missions.process(&completion_project_path, &completion_conversation_id);
            }
            // So is a standing role's.
            if let Some(role) =
                roles.conversation_role(&completion_project_path, &completion_conversation_id)
            {
                let _ = roles.process(&completion_project_path, &role.name);
            }
            completed
        })
        .await;
        drop(store);
    });
    Ok(Json(acceptance))
}

#[derive(Deserialize)]
struct ConversationProjectPathRequest {
    project_path: String,
}

async fn interrupt_conversation_turn(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    State(runtime_handler_runner_factory): State<attractor_api::RuntimeHandlerRunnerFactory>,
    State(agent_turn_backend): State<Arc<dyn AgentTurnBackend>>,
    State(run_event_observer): State<RunEventObserverHandle>,
    AxumPath(conversation_id): AxumPath<String>,
    payload: Result<Json<ConversationProjectPathRequest>, JsonRejection>,
) -> ApiResult<Value> {
    let request = json_payload(payload)?;
    let service = conversation_service(
        &settings,
        &store,
        &runtime_handler_runner_factory,
        &agent_turn_backend,
        &run_event_observer,
    );
    let interrupted = tokio::task::spawn_blocking(move || {
        service.interrupt_turn(&conversation_id, &request.project_path)
    })
    .await
    .map_err(|error| {
        WorkspaceApiError(WorkspaceError::Internal(format!(
            "interrupt task failed: {error}"
        )))
    })??;
    Ok(Json(json!({"interrupted": interrupted})))
}

async fn answer_conversation_request_user_input(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    State(runtime_handler_runner_factory): State<attractor_api::RuntimeHandlerRunnerFactory>,
    State(agent_turn_backend): State<Arc<dyn AgentTurnBackend>>,
    State(run_event_observer): State<RunEventObserverHandle>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    AxumPath((conversation_id, request_id)): AxumPath<(String, String)>,
    payload: Result<Json<ConversationRequestUserInputAnswerRequest>, JsonRejection>,
) -> ApiResult<Value> {
    let request = json_payload(payload)?;
    let service = conversation_service(
        &settings,
        &store,
        &runtime_handler_runner_factory,
        &agent_turn_backend,
        &run_event_observer,
    )
    .with_live_publisher(live_publisher(&live_hub));
    let ack = tokio::task::spawn_blocking(move || {
        let ack =
            service.submit_request_user_input_answer(&conversation_id, &request_id, request)?;
        mutation_ack_wire(&service, ack)
    })
    .await
    .map_err(|error| {
        WorkspaceApiError(WorkspaceError::Internal(format!(
            "request-user-input answer task failed: {error}"
        )))
    })??;
    Ok(Json(ack))
}

async fn create_flow_run_request_by_handle(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    State(runtime_handler_runner_factory): State<attractor_api::RuntimeHandlerRunnerFactory>,
    State(agent_turn_backend): State<Arc<dyn AgentTurnBackend>>,
    State(run_event_observer): State<RunEventObserverHandle>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    AxumPath(conversation_handle): AxumPath<String>,
    payload: Result<Json<FlowRunRequestCreateByHandleRequest>, JsonRejection>,
) -> ApiResult<Value> {
    let request = json_payload(payload)?;
    let payload = conversation_service(
        &settings,
        &store,
        &runtime_handler_runner_factory,
        &agent_turn_backend,
        &run_event_observer,
    )
    .create_flow_run_request_by_handle(&conversation_handle, request)?;
    publish_run_ids_from_value(&settings, &live_hub, &payload);
    Ok(Json(payload))
}

async fn review_flow_run_request(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    State(runtime_handler_runner_factory): State<attractor_api::RuntimeHandlerRunnerFactory>,
    State(agent_turn_backend): State<Arc<dyn AgentTurnBackend>>,
    State(run_event_observer): State<RunEventObserverHandle>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    AxumPath((conversation_id, request_id)): AxumPath<(String, String)>,
    payload: Result<Json<FlowRunRequestReviewRequest>, JsonRejection>,
) -> ApiResult<Value> {
    let request = json_payload(payload)?;
    let service = conversation_service(
        &settings,
        &store,
        &runtime_handler_runner_factory,
        &agent_turn_backend,
        &run_event_observer,
    );
    let ack = service.review_flow_run_request(&conversation_id, &request_id, request)?;
    let ack = mutation_ack_wire(&service, ack)?;
    // Runs the review launched follow from its changed artifact rows.
    publish_run_ids_from_value(&settings, &live_hub, &ack["changes"]);
    Ok(Json(ack))
}

async fn review_proposed_plan(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    State(runtime_handler_runner_factory): State<attractor_api::RuntimeHandlerRunnerFactory>,
    State(agent_turn_backend): State<Arc<dyn AgentTurnBackend>>,
    State(run_event_observer): State<RunEventObserverHandle>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    AxumPath((conversation_id, plan_id)): AxumPath<(String, String)>,
    payload: Result<Json<ProposedPlanReviewRequest>, JsonRejection>,
) -> ApiResult<Value> {
    let request = json_payload(payload)?;
    let service = conversation_service(
        &settings,
        &store,
        &runtime_handler_runner_factory,
        &agent_turn_backend,
        &run_event_observer,
    );
    let ack = service.review_proposed_plan(&conversation_id, &plan_id, request)?;
    let ack = mutation_ack_wire(&service, ack)?;
    publish_run_ids_from_value(&settings, &live_hub, &ack["changes"]);
    Ok(Json(ack))
}

async fn launch_workspace_run(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    State(runtime_handler_runner_factory): State<attractor_api::RuntimeHandlerRunnerFactory>,
    State(agent_turn_backend): State<Arc<dyn AgentTurnBackend>>,
    State(run_event_observer): State<RunEventObserverHandle>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    body: Bytes,
) -> Result<Response, WorkspaceApiError> {
    let request = match run_launch_payload(body) {
        Ok(request) => request,
        Err(response) => return Ok(response),
    };
    let payload = conversation_service(
        &settings,
        &store,
        &runtime_handler_runner_factory,
        &agent_turn_backend,
        &run_event_observer,
    )
    .launch_workspace_run(request)?;
    publish_run_ids_from_value(&settings, &live_hub, &payload);
    Ok(Json(payload).into_response())
}

async fn retry_workspace_run(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    State(runtime_handler_runner_factory): State<attractor_api::RuntimeHandlerRunnerFactory>,
    State(agent_turn_backend): State<Arc<dyn AgentTurnBackend>>,
    State(run_event_observer): State<RunEventObserverHandle>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    AxumPath(run_id): AxumPath<String>,
    body: Bytes,
) -> Result<Response, WorkspaceApiError> {
    let request = match run_retry_payload(body) {
        Ok(request) => request,
        Err(response) => return Ok(response),
    };
    let before_sequence = latest_run_sequence(&settings, &run_id).ok().flatten();
    let payload = conversation_service(
        &settings,
        &store,
        &runtime_handler_runner_factory,
        &agent_turn_backend,
        &run_event_observer,
    )
    .retry_workspace_run(&run_id, request)?;
    publish_live_run_after(&settings, &live_hub, &run_id, before_sequence);
    publish_run_ids_from_value_except(&settings, &live_hub, &payload, Some(&run_id));
    Ok(Json(payload).into_response())
}

async fn continue_workspace_run(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    State(runtime_handler_runner_factory): State<attractor_api::RuntimeHandlerRunnerFactory>,
    State(agent_turn_backend): State<Arc<dyn AgentTurnBackend>>,
    State(run_event_observer): State<RunEventObserverHandle>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    AxumPath(run_id): AxumPath<String>,
    body: Bytes,
) -> Result<Response, WorkspaceApiError> {
    let request = match run_continue_payload(body) {
        Ok(request) => request,
        Err(response) => return Ok(response),
    };
    let before_sequence = latest_run_sequence(&settings, &run_id).ok().flatten();
    let payload = conversation_service(
        &settings,
        &store,
        &runtime_handler_runner_factory,
        &agent_turn_backend,
        &run_event_observer,
    )
    .continue_workspace_run(&run_id, request)?;
    publish_live_run_after(&settings, &live_hub, &run_id, before_sequence);
    publish_run_ids_from_value_except(&settings, &live_hub, &payload, Some(&run_id));
    Ok(Json(payload).into_response())
}

async fn delete_conversation(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    AxumPath(conversation_id): AxumPath<String>,
    payload: Result<Query<DeleteConversationQuery>, QueryRejection>,
) -> ApiResult<ConversationDeleteResponse> {
    let query = query_payload(payload)?;
    WorkspaceConversationService::new((*settings).clone(), store.clone())
        .delete_conversation(&conversation_id, &query.project_path)
        .map(Json)
        .map_err(Into::into)
}

/// Raw request body upload; the filename travels as display metadata only.
async fn upload_conversation_attachment(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    AxumPath(conversation_id): AxumPath<String>,
    query: Result<Query<AttachmentUploadQuery>, QueryRejection>,
    body: Result<Bytes, BytesRejection>,
) -> ApiResult<Value> {
    let query = query_payload(query)?;
    let body = body.map_err(|_| {
        WorkspaceError::Validation("Attachments are limited to 20 MiB per file.".to_string())
    })?;
    tokio::task::spawn_blocking(move || {
        WorkspaceConversationService::new((*settings).clone(), store.clone()).upload_attachment(
            &conversation_id,
            &query.project_path,
            &query.filename,
            &body,
        )
    })
    .await
    .map_err(|error| WorkspaceError::Internal(format!("attachment upload task failed: {error}")))?
    .map(|attachment| Json(json!(attachment)))
    .map_err(Into::into)
}

/// Native images display inline; every other type downloads, so uploaded
/// HTML or SVG never renders in the app's origin.
async fn download_conversation_attachment(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    AxumPath((conversation_id, attachment_id)): AxumPath<(String, String)>,
    payload: Result<Query<ConversationQuery>, QueryRejection>,
) -> Result<Response, WorkspaceApiError> {
    let query = optional_query_payload(payload)?;
    let (attachment, bytes) = WorkspaceConversationService::new((*settings).clone(), store.clone())
        .read_attachment(
            &conversation_id,
            query.project_path.as_deref().unwrap_or_default(),
            &attachment_id,
        )?;
    let inline = spark_agent_adapter::attachments::NATIVE_IMAGE_MEDIA_TYPES
        .contains(&attachment.media_type.as_str());
    let content_type = if inline {
        attachment.media_type.as_str()
    } else {
        "application/octet-stream"
    };
    let encoded_name = attachment
        .filename
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect::<String>();
    let disposition = format!(
        "{}; filename*=UTF-8''{encoded_name}",
        if inline { "inline" } else { "attachment" }
    );
    let mut response = (StatusCode::OK, bytes).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    if let Ok(value) = HeaderValue::from_str(&disposition) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

async fn delete_conversation_attachment(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    AxumPath((conversation_id, attachment_id)): AxumPath<(String, String)>,
    payload: Result<Query<ConversationQuery>, QueryRejection>,
) -> ApiResult<Value> {
    let query = optional_query_payload(payload)?;
    WorkspaceConversationService::new((*settings).clone(), store.clone())
        .delete_draft_attachment(
            &conversation_id,
            query.project_path.as_deref().unwrap_or_default(),
            &attachment_id,
        )
        .map(Json)
        .map_err(Into::into)
}

async fn get_conversation_segment_tool_output(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    AxumPath((conversation_id, segment_id)): AxumPath<(String, String)>,
    payload: Result<Query<ConversationQuery>, QueryRejection>,
) -> ApiResult<Value> {
    let query = optional_query_payload(payload)?;
    WorkspaceConversationService::new((*settings).clone(), store.clone())
        .get_segment_tool_output(&conversation_id, &segment_id, query.project_path.as_deref())
        .map(Json)
        .map_err(Into::into)
}

/// A subscriber's conversation scopes and their catch-up cursors. A scope
/// whose cursor is `None` was reset and waits for the browser to reconnect.
struct LiveScopes {
    conversation: Option<(String, String, Option<i64>)>,
    summaries: Option<(Option<String>, Option<i64>)>,
}

/// Initialize a subscriber's conversation scopes after it subscribed: a
/// newest window or summary snapshot without a cursor, else the rows changed
/// after the browser's cursor.
fn initialize_live_scopes(
    service: &WorkspaceConversationService,
    query: &spark_workspace::live::LiveQuery,
) -> Result<(Vec<LiveEnvelope>, LiveScopes), WorkspaceError> {
    let mut envelopes = Vec::new();
    let mut scopes = LiveScopes {
        conversation: None,
        summaries: None,
    };
    if let Some(conversation_id) = query.conversation_id.clone() {
        let project_path = query.conversation_project_path.clone().ok_or_else(|| {
            WorkspaceError::Validation(
                "conversation_project_path is required when conversation_id is provided."
                    .to_string(),
            )
        })?;
        let (envelope, cursor) = match &query.conversation_cursor {
            None => {
                let (envelope, watermark) =
                    conversation_window_envelope(service, &conversation_id, &project_path)?;
                (envelope, Some(watermark))
            }
            Some((generation, since)) => conversation_changes_envelope(
                service,
                &conversation_id,
                &project_path,
                generation,
                *since,
            )?,
        };
        envelopes.push(envelope);
        scopes.conversation = Some((conversation_id, project_path, cursor));
    }
    if query.include_conversations {
        let project_path = query.conversations_project_path.clone();
        let (envelope, cursor) = match &query.conversations_cursor {
            None => {
                let (envelope, watermark) =
                    conversations_snapshot_envelope(service, project_path.as_deref())?;
                (envelope, Some(watermark))
            }
            Some((generation, since)) => conversations_changes_envelope(
                service,
                project_path.as_deref(),
                generation,
                *since,
            )?,
        };
        envelopes.push(envelope);
        scopes.summaries = Some((project_path, cursor));
    }
    Ok((envelopes, scopes))
}

/// The rows each live scope's subscriber has not seen yet, from its last
/// emitted watermark: after a wakeup, an overflow, or a heartbeat that finds
/// the database ahead of the cursor. Notifications are wakeups; this read is
/// the truth. An empty batch still advances a filtered scope's watermark.
fn catch_up_live_scopes(
    service: &WorkspaceConversationService,
    scopes: &mut LiveScopes,
) -> Result<Vec<LiveEnvelope>, WorkspaceError> {
    let generation = service.store().database().generation().to_string();
    let mut envelopes = Vec::new();
    if let Some((conversation_id, project_path, cursor)) = scopes.conversation.as_mut() {
        if let Some(since) = *cursor {
            let (envelope, next) = conversation_changes_envelope(
                service,
                conversation_id,
                project_path,
                &generation,
                since,
            )?;
            if next != Some(since) {
                envelopes.push(envelope);
            }
            *cursor = next;
        }
    }
    if let Some((project_path, cursor)) = scopes.summaries.as_mut() {
        if let Some(since) = *cursor {
            let (envelope, next) = conversations_changes_envelope(
                service,
                project_path.as_deref(),
                &generation,
                since,
            )?;
            if next != Some(since) {
                envelopes.push(envelope);
            }
            *cursor = next;
        }
    }
    Ok(envelopes)
}

async fn workspace_live_events(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    payload: Result<Query<RawLiveQuery>, QueryRejection>,
) -> Result<Response, WorkspaceApiError> {
    let raw_query = optional_query_payload(payload)?;
    let query = validate_live_query(raw_query)?;
    // Subscribe before reading initial state: a commit in between wakes this
    // subscriber, and its catch-up selects only rows after the initial
    // watermark, so nothing is missed or applied twice.
    let subscriber =
        live_query_subscribes(&query).then(|| live_hub.subscribe_with(Some(query.clone())));
    let service = WorkspaceConversationService::new((*settings).clone(), store.clone());
    let (initial, scopes) = {
        let (service, query) = (service.clone(), query.clone());
        tokio::task::spawn_blocking(move || {
            let mut envelopes = initial_live_envelopes(&settings, service.store(), &query)?;
            let (conversation, scopes) = initialize_live_scopes(&service, &query)?;
            envelopes.splice(0..0, conversation);
            Ok::<_, WorkspaceError>((envelopes, scopes))
        })
        .await
        .map_err(|error| {
            WorkspaceError::Internal(format!("live initialization failed: {error}"))
        })??
    };
    let has_initial = !initial.is_empty();
    let stream = async_stream::stream! {
        let mut scopes = scopes;
        for envelope in initial {
            yield Ok::<Bytes, Infallible>(sse_data_frame(&envelope));
        }

        let mut interval = time::interval(Duration::from_secs(15));
        interval.tick().await;
        if !has_initial {
            yield Ok::<Bytes, Infallible>(Bytes::from_static(b": keepalive\n\n"));
        }

        loop {
            let Some(subscriber) = subscriber.as_ref() else {
                interval.tick().await;
                yield Ok::<Bytes, Infallible>(Bytes::from_static(b": keepalive\n\n"));
                continue;
            };
            let catch_up = tokio::select! {
                received = subscriber.recv() => match received {
                    Received::Envelope(envelope) => {
                        yield Ok::<Bytes, Infallible>(sse_data_frame(&envelope));
                        false
                    }
                    Received::Lagged(_) => {
                        // This subscriber's queue overflowed: other resources
                        // refetch, conversation scopes catch up from rows.
                        for envelope in lagged_resync_envelopes(&query) {
                            yield Ok::<Bytes, Infallible>(sse_data_frame(&envelope));
                        }
                        true
                    }
                    Received::Changed => true,
                },
                _ = interval.tick() => {
                    yield Ok::<Bytes, Infallible>(Bytes::from_static(b": keepalive\n\n"));
                    // Closes a commit-to-notification gap.
                    true
                }
            };
            if !catch_up || (scopes.conversation.is_none() && scopes.summaries.is_none()) {
                continue;
            }
            let service = service.clone();
            let caught_up = tokio::task::spawn_blocking(move || {
                let envelopes = catch_up_live_scopes(&service, &mut scopes);
                (scopes, envelopes)
            })
            .await;
            let Ok((returned, envelopes)) = caught_up else {
                break;
            };
            scopes = returned;
            match envelopes {
                Ok(envelopes) => {
                    for envelope in envelopes {
                        yield Ok::<Bytes, Infallible>(sse_data_frame(&envelope));
                    }
                }
                // A failed read retries at the next wakeup or heartbeat.
                Err(error) => eprintln!("Live conversation catch-up failed: {error}"),
            }
        }
    };
    let mut response = Response::new(Body::from_stream(stream));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
        .headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("keep-alive"));
    Ok(response)
}

async fn project_metadata(
    State(settings): State<Arc<SparkSettings>>,
    payload: Result<Query<ProjectMetadataQuery>, QueryRejection>,
) -> ApiResult<ProjectMetadata> {
    let query = query_payload(payload)?;
    WorkspaceProjectService::new((*settings).clone())
        .project_metadata(&query.directory)
        .map(Json)
        .map_err(Into::into)
}

async fn browse_project_directories(
    State(settings): State<Arc<SparkSettings>>,
    payload: Result<Query<BrowseQuery>, QueryRejection>,
) -> ApiResult<BrowseResponse> {
    let query = query_payload(payload)?;
    WorkspaceProjectService::new((*settings).clone())
        .browse_project_directories(query.path.as_deref())
        .map(Json)
        .map_err(Into::into)
}

async fn project_chat_models(
    State(settings): State<Arc<SparkSettings>>,
    payload: Result<Query<ChatModelsQuery>, QueryRejection>,
) -> ApiResult<Value> {
    let query = query_payload(payload)?;
    let models = tokio::task::spawn_blocking(move || {
        WorkspaceProjectService::new((*settings).clone()).chat_models(query.project_path.as_deref())
    })
    .await
    .map_err(|error| {
        WorkspaceApiError(WorkspaceError::Internal(format!(
            "model discovery task failed: {error}"
        )))
    })??;
    Ok(Json(models))
}

#[derive(Deserialize)]
struct TriggerListQuery {
    project_path: Option<String>,
}

async fn list_triggers(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    payload: Result<Query<TriggerListQuery>, QueryRejection>,
) -> ApiResult<Vec<SerializedTrigger>> {
    let query = query_payload(payload)?;
    WorkspaceTriggerService::new((*settings).clone(), store.clone())
        .list_triggers(
            query
                .project_path
                .as_deref()
                .filter(|path| !path.trim().is_empty()),
        )
        .map(Json)
        .map_err(Into::into)
}

async fn create_trigger(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    payload: Result<Json<TriggerCreateRequest>, JsonRejection>,
) -> ApiResult<SerializedTrigger> {
    let request = json_payload(payload)?;
    let trigger =
        WorkspaceTriggerService::new((*settings).clone(), store.clone()).create_trigger(request)?;
    if let Ok(value) = serde_json::to_value(&trigger) {
        live_hub.publish(trigger_upsert_envelope(&value));
    }
    live_hub.publish_settings_change("workspace", "triggers", None, Value::Null);
    Ok(Json(trigger))
}

/// A trigger is identified by its project plus its id.
#[derive(Deserialize)]
struct TriggerQuery {
    project_path: String,
}

async fn get_trigger(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    AxumPath(trigger_id): AxumPath<String>,
    payload: Result<Query<TriggerQuery>, QueryRejection>,
) -> ApiResult<SerializedTrigger> {
    let query = query_payload(payload)?;
    WorkspaceTriggerService::new((*settings).clone(), store.clone())
        .get_trigger(&query.project_path, &trigger_id)
        .map(Json)
        .map_err(Into::into)
}

async fn update_trigger(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    AxumPath(trigger_id): AxumPath<String>,
    query: Result<Query<TriggerQuery>, QueryRejection>,
    payload: Result<Json<TriggerUpdateRequest>, JsonRejection>,
) -> ApiResult<SerializedTrigger> {
    let query = query_payload(query)?;
    let request = json_payload(payload)?;
    let trigger = WorkspaceTriggerService::new((*settings).clone(), store.clone()).update_trigger(
        &query.project_path,
        &trigger_id,
        request,
    )?;
    if let Ok(value) = serde_json::to_value(&trigger) {
        live_hub.publish(trigger_upsert_envelope(&value));
    }
    live_hub.publish_settings_change("workspace", "triggers", None, Value::Null);
    Ok(Json(trigger))
}

#[derive(Deserialize)]
struct TriggerDeleteQuery {
    project_path: String,
    expected_revision: String,
    message: Option<String>,
}

async fn delete_trigger(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    AxumPath(trigger_id): AxumPath<String>,
    payload: Result<Query<TriggerDeleteQuery>, QueryRejection>,
) -> ApiResult<TriggerDeleteResponse> {
    let query = query_payload(payload)?;
    let deleted = WorkspaceTriggerService::new((*settings).clone(), store.clone()).delete_trigger(
        &query.project_path,
        &trigger_id,
        &query.expected_revision,
        query.message.as_deref(),
    )?;
    if let Ok(value) = serde_json::to_value(&deleted) {
        live_hub.publish(trigger_delete_envelope(
            &value,
            Some(deleted.project_path.clone()),
        ));
    }
    live_hub.publish_settings_change("workspace", "triggers", None, Value::Null);
    Ok(Json(deleted))
}

async fn post_trigger_webhook(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    State(run_event_observer): State<RunEventObserverHandle>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, WorkspaceApiError> {
    let webhook_key = header_value(&headers, "X-Spark-Webhook-Key");
    let webhook_secret = header_value(&headers, "X-Spark-Webhook-Secret");
    let request_id = header_value(&headers, "X-Spark-Webhook-Request-Id");
    if webhook_key.is_empty() || webhook_secret.is_empty() {
        return Ok(webhook_error_response(
            StatusCode::UNAUTHORIZED,
            "Webhook key and secret headers are required.",
        ));
    }
    let value = serde_json::from_slice::<Value>(&body).map_err(|_| {
        WorkspaceError::Validation("Webhook payload must be valid JSON.".to_string())
    })?;
    let payload = value.as_object().cloned().ok_or_else(|| {
        WorkspaceError::Validation("Webhook payload must be a JSON object.".to_string())
    })?;
    let mut service = WorkspaceTriggerService::new((*settings).clone(), store.clone());
    if let Some(observer) = run_event_observer.0 {
        service = service.with_run_event_observer(observer);
    }
    let dispatch = service.dispatch_webhook(WebhookHandleRequest {
        webhook_key,
        webhook_secret,
        request_id: (!request_id.is_empty()).then_some(request_id),
        payload,
    })?;
    publish_trigger_activation_outcomes(&settings, &live_hub, vec![dispatch.activation]);
    Ok(Json(dispatch.response).into_response())
}

async fn pending_attention(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    State(runtime_handler_runner_factory): State<attractor_api::RuntimeHandlerRunnerFactory>,
    State(agent_turn_backend): State<Arc<dyn AgentTurnBackend>>,
    State(run_event_observer): State<RunEventObserverHandle>,
) -> ApiResult<Value> {
    conversation_service(
        &settings,
        &store,
        &runtime_handler_runner_factory,
        &agent_turn_backend,
        &run_event_observer,
    )
    .pending_attention()
    .map(|items| Json(serde_json::json!({ "items": items })))
    .map_err(Into::into)
}

async fn list_workspace_flows(
    State(settings): State<Arc<SparkSettings>>,
    payload: Result<Query<FlowSurfaceQuery>, QueryRejection>,
) -> ApiResult<Vec<WorkspaceFlowSummary>> {
    let query = optional_query_payload(payload)?;
    WorkspaceFlowService::new((*settings).clone())
        .list_flows(query.project_path.as_deref())
        .map(Json)
        .map_err(Into::into)
}

async fn get_workspace_flow_dispatch(
    State(settings): State<Arc<SparkSettings>>,
    AxumPath(flow_path): AxumPath<String>,
    payload: Result<Query<FlowSurfaceQuery>, QueryRejection>,
) -> Result<Response, WorkspaceApiError> {
    let query = optional_query_payload(payload)?;
    let flow_path = flow_path.trim_start_matches('/');
    let service = WorkspaceFlowService::new((*settings).clone());
    if let Some(flow_name) = flow_path.strip_suffix("/files") {
        let files = service.flow_files(flow_name, query.project_path.as_deref())?;
        return Ok(Json(files).into_response());
    }
    if let Some(flow_name) = flow_path.strip_suffix("/validate") {
        let payload = service.validate_flow(flow_name, query.project_path.as_deref())?;
        return Ok(Json(payload).into_response());
    }
    if flow_path.is_empty() {
        return Err(WorkspaceError::NotFound("Not Found".to_string()).into());
    }
    let flow = service.describe_flow(flow_path, query.project_path.as_deref())?;
    Ok(Json(flow).into_response())
}

#[derive(Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsQuery {
    client_id: Option<String>,
    project_path: Option<String>,
    conversation_id: Option<String>,
}

async fn settings(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    Query(query): Query<SettingsQuery>,
) -> ApiResult<Value> {
    if let Some(id) = query.client_id {
        if query.project_path.is_some() || query.conversation_id.is_some() {
            return Err(WorkspaceError::Validation(
                "Client scope cannot be combined with project or conversation scope.".into(),
            )
            .into());
        }
        return spark_workspace::settings::client_settings(&settings, &id)
            .map(Json)
            .map_err(Into::into);
    }
    match (query.project_path, query.conversation_id) {
        (project_path, Some(id)) => {
            WorkspaceConversationService::new((*settings).clone(), store.clone())
                .conversation_settings_view(&id, project_path.as_deref().unwrap_or_default())
        }
        (Some(path), None) => {
            spark_workspace::settings::project_model_settings_view(&settings, &path)
        }
        (None, None) => workspace_settings(&settings),
    }
    .map(Json)
    .map_err(Into::into)
}

async fn patch_settings(
    State(settings): State<Arc<SparkSettings>>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    payload: Result<Json<spark_workspace::settings::WorkspaceSettingsUpdate>, JsonRejection>,
) -> ApiResult<Value> {
    let request = payload
        .map_err(|error| {
            let detail = error.body_text();
            let field = [
                "thinking_budget_tokens",
                "thinking",
                "reasoning_mode",
                "reasoning_summary",
            ]
            .into_iter()
            .find(|field| detail.contains(field));
            WorkspaceError::Validation(match field {
                Some(field) => format!("Invalid {field}; check its value and type."),
                None => "Invalid settings update; check section, revision, and field types.".into(),
            })
        })?
        .0;
    let (scope, section, project_path) = match &request.section {
        spark_workspace::settings::WorkspaceSettingsSection::Providers(_) => {
            ("workspace", "providers", None)
        }
        spark_workspace::settings::WorkspaceSettingsSection::Agents(_) => {
            ("workspace", "agents", None)
        }
        spark_workspace::settings::WorkspaceSettingsSection::Connections(_) => {
            ("workspace", "connections", None)
        }
        spark_workspace::settings::WorkspaceSettingsSection::LlmProfiles(_) => {
            ("workspace", "llm_profiles", None)
        }
        spark_workspace::settings::WorkspaceSettingsSection::ExecutionProfiles(_) => {
            ("workspace", "execution_profiles", None)
        }
        spark_workspace::settings::WorkspaceSettingsSection::ClientPreferences { .. } => {
            ("client", "preferences", None)
        }
        spark_workspace::settings::WorkspaceSettingsSection::ProjectExecution {
            project_path,
            ..
        } => ("project", "execution", Some(project_path.clone())),
        spark_workspace::settings::WorkspaceSettingsSection::Runtime(_) => {
            ("workspace", "runtime", None)
        }
        spark_workspace::settings::WorkspaceSettingsSection::Models(_) => {
            ("workspace", "models", None)
        }
        spark_workspace::settings::WorkspaceSettingsSection::UtilityModels(_) => {
            ("workspace", "utility_models", None)
        }
        spark_workspace::settings::WorkspaceSettingsSection::ProjectModels {
            project_path, ..
        } => ("project", "models", Some(project_path.clone())),
        spark_workspace::settings::WorkspaceSettingsSection::ConversationModels {
            project_path,
            ..
        } => ("conversation", "models", Some(project_path.clone())),
    };
    let store = live_hub
        .conversation_store()
        .ok_or_else(|| WorkspaceError::ServiceUnavailable("Spark is shutting down.".to_string()))?;
    // A conversation's settings commit reaches its live clients as a revision.
    let response =
        spark_workspace::settings::update_workspace_settings(&settings, &store, request)?;
    live_hub.publish_settings_change(
        scope,
        section,
        project_path,
        response[section]["revision"].clone(),
    );
    Ok(Json(response))
}

async fn validate_settings_update(
    State(settings): State<Arc<SparkSettings>>,
    payload: Result<Json<spark_workspace::settings::WorkspaceSettingsUpdate>, JsonRejection>,
) -> ApiResult<Value> {
    let request = payload
        .map_err(|error| {
            let detail = error.body_text();
            let field = [
                "thinking_budget_tokens",
                "thinking",
                "reasoning_mode",
                "reasoning_summary",
            ]
            .into_iter()
            .find(|field| detail.contains(field));
            WorkspaceError::Validation(match field {
                Some(field) => format!("Invalid {field}; check its value and type."),
                None => "Invalid settings update; check section, revision, and field types.".into(),
            })
        })?
        .0;
    spark_workspace::settings::validate_workspace_settings_update(&settings, &request)?;
    Ok(Json(json!({"valid": true})))
}

#[derive(Debug, serde::Serialize)]
struct DeletedProjectRecordWithStatus {
    status: &'static str,
    project_id: String,
    project_path: String,
    display_name: String,
}

impl From<DeletedProjectRecord> for DeletedProjectRecordWithStatus {
    fn from(value: DeletedProjectRecord) -> Self {
        Self {
            status: "deleted",
            project_id: value.project_id,
            project_path: value.project_path,
            display_name: value.display_name,
        }
    }
}

fn json_payload<T>(payload: Result<Json<T>, JsonRejection>) -> Result<T, WorkspaceApiError> {
    payload
        .map(|Json(value)| value)
        .map_err(|rejection| WorkspaceError::Validation(rejection.to_string()).into())
}

fn run_launch_payload(body: Bytes) -> Result<RunLaunchRequest, Response> {
    let value = run_body_value(body)?;
    let object = run_body_object(&value)?;
    let errors = required_field_errors(object, &["flow_name", "summary"], &value);
    if !errors.is_empty() {
        return Err(run_validation_response(errors));
    }
    deserialize_run_payload(value)
}

fn run_retry_payload(body: Bytes) -> Result<RunRetryRequest, Response> {
    let value = run_body_value(body)?;
    let object = run_body_object(&value)?;
    let errors = extra_field_errors(object, &["conversation_handle"]);
    if !errors.is_empty() {
        return Err(run_validation_response(errors));
    }
    deserialize_run_payload(value)
}

fn run_continue_payload(body: Bytes) -> Result<RunContinueRequest, Response> {
    let value = run_body_value(body)?;
    let object = run_body_object(&value)?;
    let mut errors = required_field_errors(object, &["start_node", "flow_source_mode"], &value);
    errors.extend(extra_field_errors(
        object,
        &[
            "start_node",
            "flow_source_mode",
            "flow_name",
            "project_path",
            "conversation_handle",
            "model",
            "llm_provider",
            "llm_profile",
            "reasoning_effort",
        ],
    ));
    if !errors.is_empty() {
        return Err(run_validation_response(errors));
    }
    deserialize_run_payload(value)
}

fn run_body_value(body: Bytes) -> Result<Value, Response> {
    serde_json::from_slice::<Value>(&body).map_err(|error| {
        run_validation_response(vec![json!({
            "type": "json_invalid",
            "loc": ["body", error.column()],
            "msg": "JSON decode error",
            "input": {},
            "ctx": {
                "error": error.to_string(),
            },
        })])
    })
}

fn run_body_object(value: &Value) -> Result<&serde_json::Map<String, Value>, Response> {
    value.as_object().ok_or_else(|| {
        run_validation_response(vec![json!({
            "type": "model_attributes_type",
            "loc": ["body"],
            "msg": "Input should be a valid dictionary or object to extract fields from",
            "input": value,
        })])
    })
}

fn required_field_errors(
    object: &serde_json::Map<String, Value>,
    fields: &[&str],
    input: &Value,
) -> Vec<Value> {
    fields
        .iter()
        .filter(|field| !object.contains_key(**field))
        .map(|field| {
            json!({
                "type": "missing",
                "loc": ["body", *field],
                "msg": "Field required",
                "input": input,
            })
        })
        .collect()
}

fn extra_field_errors(
    object: &serde_json::Map<String, Value>,
    allowed_fields: &[&str],
) -> Vec<Value> {
    object
        .iter()
        .filter(|(field, _)| !allowed_fields.contains(&field.as_str()))
        .map(|(field, value)| {
            json!({
                "type": "extra_forbidden",
                "loc": ["body", field],
                "msg": "Extra inputs are not permitted",
                "input": value,
            })
        })
        .collect()
}

fn deserialize_run_payload<T: DeserializeOwned>(value: Value) -> Result<T, Response> {
    serde_json::from_value(value.clone()).map_err(|error| {
        run_validation_response(vec![json!({
            "type": "value_error",
            "loc": ["body"],
            "msg": error.to_string(),
            "input": value,
        })])
    })
}

fn run_validation_response(errors: Vec<Value>) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "detail": errors })),
    )
        .into_response()
}

fn query_payload<T>(payload: Result<Query<T>, QueryRejection>) -> Result<T, WorkspaceApiError> {
    payload
        .map(|Query(value)| value)
        .map_err(|rejection| WorkspaceError::Validation(rejection.to_string()).into())
}

fn optional_query_payload<T: Default>(
    payload: Result<Query<T>, QueryRejection>,
) -> Result<T, WorkspaceApiError> {
    match payload {
        Ok(Query(value)) => Ok(value),
        Err(rejection) if rejection.to_string().contains("missing field") => Ok(T::default()),
        Err(rejection) => Err(WorkspaceError::Validation(rejection.to_string()).into()),
    }
}

fn header_value(headers: &HeaderMap, name: &'static str) -> String {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn webhook_error_response(status: StatusCode, detail: &str) -> Response {
    (status, Json(json!({ "detail": detail }))).into_response()
}

fn sse_data_frame(envelope: &spark_workspace::live::LiveEnvelope) -> Bytes {
    let payload = serde_json::to_string(envelope)
        .expect("serializing Workspace live envelope for SSE cannot fail");
    Bytes::from(format!("data: {payload}\n\n"))
}

fn live_query_subscribes(query: &spark_workspace::live::LiveQuery) -> bool {
    query.include_settings
        || query.conversation_id.is_some()
        || query.run_id.is_some()
        || query.include_runs_overview
        || query.include_triggers
        || query.include_workflow_log
        || query.include_missions
        || query.include_conversations
}

fn publish_run_ids_from_value(
    settings: &SparkSettings,
    live_hub: &WorkspaceLiveHub,
    value: &Value,
) {
    publish_run_ids_from_value_except(settings, live_hub, value, None);
}

fn publish_run_ids_from_value_except(
    settings: &SparkSettings,
    live_hub: &WorkspaceLiveHub,
    value: &Value,
    skip_run_id: Option<&str>,
) {
    let mut run_ids = BTreeSet::new();
    collect_run_ids_from_value(value, &mut run_ids);
    for run_id in run_ids {
        if Some(run_id.as_str()) == skip_run_id {
            continue;
        }
        publish_live_run_after(settings, live_hub, &run_id, None);
    }
}

fn collect_run_ids_from_value(value: &Value, run_ids: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if matches!(
                    key.as_str(),
                    "run_id" | "pipeline_id" | "source_run_id" | "result_run_id" | "target_run_id"
                ) {
                    if let Some(run_id) = value.as_str().filter(|value| !value.trim().is_empty()) {
                        run_ids.insert(run_id.to_string());
                    }
                }
                collect_run_ids_from_value(value, run_ids);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_run_ids_from_value(value, run_ids);
            }
        }
        _ => {}
    }
}

fn mission_service(
    settings: &SparkSettings,
    store: &Arc<ConversationStore>,
) -> spark_workspace::missions::WorkspaceMissionService {
    spark_workspace::missions::WorkspaceMissionService::new(settings.clone(), store.clone())
}

/// Runs a blocking mission operation; the service publishes the result.
async fn mission_call(
    work: impl FnOnce() -> spark_workspace::WorkspaceResult<MissionRecord> + Send + 'static,
) -> ApiResult<MissionRecord> {
    let mission = tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| WorkspaceError::Internal(format!("mission task failed: {error}")))??;
    Ok(Json(mission))
}

async fn list_playbooks(
    State(settings): State<Arc<SparkSettings>>,
    payload: Result<Query<FlowSurfaceQuery>, QueryRejection>,
) -> ApiResult<Vec<spark_workspace::playbooks::Playbook>> {
    let query = optional_query_payload(payload)?;
    Ok(Json(spark_workspace::playbooks::list(
        &settings,
        query.project_path.as_deref(),
    )?))
}
async fn get_playbook(
    State(settings): State<Arc<SparkSettings>>,
    AxumPath(name): AxumPath<String>,
    payload: Result<Query<FlowSurfaceQuery>, QueryRejection>,
) -> ApiResult<spark_workspace::playbooks::Playbook> {
    let query = optional_query_payload(payload)?;
    Ok(Json(spark_workspace::playbooks::get(
        &settings,
        query.project_path.as_deref(),
        &name,
    )?))
}

async fn list_missions(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    payload: Result<Query<ConversationQuery>, QueryRejection>,
) -> ApiResult<Value> {
    let query = optional_query_payload(payload)?;
    Ok(Json(
        mission_service(&settings, &store).board(query.project_path.as_deref())?,
    ))
}
async fn get_mission(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    AxumPath(id): AxumPath<String>,
    payload: Result<Query<ProjectConversationsQuery>, QueryRejection>,
) -> ApiResult<MissionRecord> {
    let query = query_payload(payload)?;
    Ok(Json(
        mission_service(&settings, &store).get(&query.project_path, &id)?,
    ))
}
#[derive(Debug, Default, Deserialize)]
struct MissionCreateQuery {
    project_path: Option<String>,
    conversation_handle: Option<String>,
}

async fn create_mission(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    query: Result<Query<MissionCreateQuery>, QueryRejection>,
    payload: Result<Json<MissionMutation>, JsonRejection>,
) -> ApiResult<MissionRecord> {
    let query = optional_query_payload(query)?;
    let mutation = json_payload(payload)?;
    mission_call(move || {
        let service = mission_service(&settings, &store);
        match query.conversation_handle.as_deref() {
            Some(handle) => {
                service.create_from_conversation(handle, query.project_path.as_deref(), mutation)
            }
            None => service.create(query.project_path.as_deref().unwrap_or_default(), mutation),
        }
    })
    .await
}
async fn update_mission(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    AxumPath(id): AxumPath<String>,
    query: Result<Query<ProjectConversationsQuery>, QueryRejection>,
    payload: Result<Json<MissionMutation>, JsonRejection>,
) -> ApiResult<MissionRecord> {
    let query = query_payload(query)?;
    let mutation = json_payload(payload)?;
    mission_call(move || {
        mission_service(&settings, &store).update(&query.project_path, &id, mutation)
    })
    .await
}
async fn post_mission_event(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    AxumPath(id): AxumPath<String>,
    query: Result<Query<ProjectConversationsQuery>, QueryRejection>,
    payload: Result<Json<spark_workspace::inbox::InboxPost>, JsonRejection>,
) -> ApiResult<MissionRecord> {
    let query = query_payload(query)?;
    let post = json_payload(payload)?;
    mission_call(move || {
        mission_service(&settings, &store).post_event(&query.project_path, &id, post)
    })
    .await
}
async fn control_mission(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    AxumPath((id, control)): AxumPath<(String, String)>,
    query: Result<Query<ProjectConversationsQuery>, QueryRejection>,
    body: Bytes,
) -> ApiResult<MissionRecord> {
    let query = query_payload(query)?;
    let body: &[u8] = if body.is_empty() { b"{}" } else { &body };
    let close: spark_workspace::missions::MissionCloseRequest = serde_json::from_slice(body)
        .map_err(|error| WorkspaceError::Validation(format!("Invalid close request: {error}")))?;
    mission_call(move || {
        let service = mission_service(&settings, &store);
        let project = query.project_path.as_str();
        match control.as_str() {
            "start" => service.start(project, &id),
            "cancel" => service.cancel(project, &id),
            "close" => service.close(project, &id, close),
            "wait" => service.wait(project, &id, &close.reason),
            _ => Err(WorkspaceError::NotFound("Unknown mission control".into())),
        }
    })
    .await
}

async fn list_roles(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    payload: Result<Query<ProjectConversationsQuery>, QueryRejection>,
) -> ApiResult<Value> {
    let query = query_payload(payload)?;
    let roles =
        spark_workspace::roles::WorkspaceRoleService::new((*settings).clone(), store.clone())
            .list(&query.project_path)?;
    Ok(Json(json!({ "roles": roles })))
}

#[derive(Debug, Deserialize)]
struct OrgQuery {
    project_path: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    role: String,
    file: Option<String>,
}

fn org_service(
    settings: &SparkSettings,
    store: &Arc<ConversationStore>,
) -> spark_workspace::org::WorkspaceOrgService {
    spark_workspace::org::WorkspaceOrgService::new(settings.clone(), store.clone())
}

async fn list_org(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    payload: Result<Query<OrgQuery>, QueryRejection>,
) -> ApiResult<Value> {
    let query = query_payload(payload)?;
    Ok(Json(
        org_service(&settings, &store).list(&query.project_path)?,
    ))
}

async fn get_org_item(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    payload: Result<Query<OrgQuery>, QueryRejection>,
) -> ApiResult<spark_workspace::org::OrgItemDetail> {
    let query = query_payload(payload)?;
    Ok(Json(org_service(&settings, &store).item(
        &query.project_path,
        &query.kind,
        &query.name,
    )?))
}

async fn get_org_history(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    payload: Result<Query<OrgQuery>, QueryRejection>,
) -> ApiResult<Value> {
    let query = query_payload(payload)?;
    let commits = org_service(&settings, &store).history(&query.project_path, &query.path)?;
    Ok(Json(json!({ "commits": commits })))
}

async fn get_org_memory(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    payload: Result<Query<OrgQuery>, QueryRejection>,
) -> ApiResult<Value> {
    let query = query_payload(payload)?;
    Ok(Json(org_service(&settings, &store).memory(
        &query.project_path,
        &query.role,
        query.file.as_deref(),
    )?))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoleSendRequest {
    conversation_handle: String,
    message: String,
}

async fn control_role(
    State(settings): State<Arc<SparkSettings>>,
    State(store): State<Arc<ConversationStore>>,
    AxumPath((name, control)): AxumPath<(String, String)>,
    query: Result<Query<ProjectConversationsQuery>, QueryRejection>,
    body: Bytes,
) -> ApiResult<spark_workspace::roles::RoleRecord> {
    let query = query_payload(query)?;
    let send: Option<RoleSendRequest> = (control == "send")
        .then(|| serde_json::from_slice(&body))
        .transpose()
        .map_err(|error| WorkspaceError::Validation(format!("Invalid send request: {error}")))?;
    let role = tokio::task::spawn_blocking(move || {
        let service =
            spark_workspace::roles::WorkspaceRoleService::new((*settings).clone(), store.clone());
        let project = query.project_path.as_str();
        match (control.as_str(), send) {
            ("start", _) => service.start(project, &name),
            ("stop", _) => service.stop(project, &name),
            ("send", Some(send)) => {
                service.send(project, &name, &send.conversation_handle, &send.message)
            }
            _ => Err(WorkspaceError::NotFound("Unknown role control".into())),
        }
    })
    .await
    .map_err(|error| WorkspaceError::Internal(format!("role task failed: {error}")))??;
    Ok(Json(role))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileResourceUpdate {
    expected_revision: String,
    value: Value,
}

pub(crate) async fn patch_llm_profiles(
    settings: State<Arc<SparkSettings>>,
    live_hub: State<Arc<WorkspaceLiveHub>>,
    payload: Result<Json<ProfileResourceUpdate>, JsonRejection>,
) -> ApiResult<Value> {
    patch_profile_resource(settings, live_hub, payload, "llm_profiles").await
}

pub(crate) async fn patch_execution_profiles(
    settings: State<Arc<SparkSettings>>,
    live_hub: State<Arc<WorkspaceLiveHub>>,
    payload: Result<Json<ProfileResourceUpdate>, JsonRejection>,
) -> ApiResult<Value> {
    patch_profile_resource(settings, live_hub, payload, "execution_profiles").await
}

async fn patch_profile_resource(
    settings: State<Arc<SparkSettings>>,
    live_hub: State<Arc<WorkspaceLiveHub>>,
    payload: Result<Json<ProfileResourceUpdate>, JsonRejection>,
    section: &str,
) -> ApiResult<Value> {
    let payload = payload
        .map_err(|_| {
            WorkspaceError::Validation(
                "Invalid profile update; supply expected_revision and a typed value.".into(),
            )
        })?
        .0;
    let request = serde_json::from_value(json!({"expected_revision": payload.expected_revision, "section": section, "value": payload.value}))
        .map_err(|_| WorkspaceError::Validation("Invalid profile fields.".into()))?;
    patch_settings(settings, live_hub, Ok(Json(request))).await
}
