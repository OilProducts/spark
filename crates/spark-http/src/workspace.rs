use std::collections::BTreeSet;
use std::convert::Infallible;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use spark_agent_adapter::AgentTurnBackend;
use spark_common::settings::SparkSettings;
use spark_storage::{read_trigger_definition, DeletedProjectRecord, ProjectRecord};
use spark_workspace::conversations::{
    ConversationDeleteResponse, ConversationRequestUserInputAnswerRequest,
    ConversationSettingsUpdate, ConversationTurnRequest, FlowRunRequestCreateByHandleRequest,
    FlowRunRequestCreateResponse, FlowRunRequestReviewRequest, ProposedPlanReviewRequest,
    RunContinueRequest, RunLaunchRequest, RunRetryRequest, WorkspaceConversationService,
};
use spark_workspace::live::{
    conversation_envelopes_after, conversation_event_envelope, conversation_snapshot_envelope,
    envelope_matches_query, initial_live_envelopes, latest_run_sequence, trigger_delete_envelope,
    trigger_upsert_envelope, validate_live_query, RawLiveQuery,
};
use spark_workspace::projects::{ProjectRegistrationRequest, ProjectStateUpdate};
use spark_workspace::WorkspaceError;
use spark_workspace::{
    workspace_settings, BrowseResponse, ConversationSummary, ProjectMetadata, SerializedTrigger,
    TriggerCreateRequest, TriggerDeleteResponse, TriggerUpdateRequest, WebhookHandleRequest,
    WorkspaceFlowLaunchPolicyUpdate, WorkspaceFlowService, WorkspaceFlowSummary,
    WorkspaceProjectService, WorkspaceTriggerService,
};
use tokio::sync::broadcast;
use tokio::time::{self, Duration};

use crate::{
    publish_live_run_after, publish_trigger_activation_outcomes, HttpAppState, WorkspaceApiError,
    WorkspaceLiveHub,
};

type ApiResult<T> = std::result::Result<Json<T>, WorkspaceApiError>;

fn conversation_service(
    settings: &SparkSettings,
    runtime_handler_runner_factory: &attractor_api::RuntimeHandlerRunnerFactory,
    agent_turn_backend: &Arc<dyn AgentTurnBackend>,
) -> WorkspaceConversationService {
    WorkspaceConversationService::new_with_runtime_handler_runner_factory_and_agent_turn_backend(
        settings.clone(),
        runtime_handler_runner_factory.clone(),
        agent_turn_backend.clone(),
    )
}

pub fn router() -> Router<HttpAppState> {
    Router::new()
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
        .route("/flows", get(list_workspace_flows))
        .route(
            "/flows/{*flow_path}",
            get(get_workspace_flow_dispatch).put(put_workspace_flow_dispatch),
        )
        .route(
            "/conversations/{conversation_id}/segments/{segment_id}/tool-output",
            get(get_conversation_segment_tool_output),
        )
        .route(
            "/conversations/{conversation_id}/events",
            get(deprecated_conversation_events),
        )
        .route(
            "/conversations/{conversation_id}/turns",
            post(send_conversation_turn),
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
        .route("/settings", get(settings))
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
    project_path: String,
}

#[derive(Debug, Default, Deserialize)]
struct FlowSurfaceQuery {
    surface: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProjectStateUpdateBody {
    project_path: String,
    #[serde(default)]
    is_favorite: Option<Option<bool>>,
    #[serde(default)]
    last_accessed_at: Option<Option<String>>,
    #[serde(default)]
    active_conversation_id: Option<Option<String>>,
    #[serde(default)]
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
    payload: Result<Json<ProjectRegistrationRequest>, JsonRejection>,
) -> ApiResult<ProjectRecord> {
    let request = json_payload(payload)?;
    WorkspaceProjectService::new((*settings).clone())
        .register_project(request)
        .map(Json)
        .map_err(Into::into)
}

async fn update_project_state(
    State(settings): State<Arc<SparkSettings>>,
    payload: Result<Json<ProjectStateUpdateBody>, JsonRejection>,
) -> ApiResult<ProjectRecord> {
    let request = json_payload(payload)?;
    let is_favorite = request.is_favorite.map(|value| value.unwrap_or(false));
    WorkspaceProjectService::new((*settings).clone())
        .update_project_state(ProjectStateUpdate {
            project_path: request.project_path,
            last_accessed_at: request.last_accessed_at,
            is_favorite,
            active_conversation_id: request.active_conversation_id,
            execution_profile_id: request.execution_profile_id,
        })
        .map(Json)
        .map_err(Into::into)
}

async fn delete_project(
    State(settings): State<Arc<SparkSettings>>,
    payload: Result<Query<DeleteProjectQuery>, QueryRejection>,
) -> ApiResult<DeletedProjectRecordWithStatus> {
    let query = query_payload(payload)?;
    let deleted =
        WorkspaceProjectService::new((*settings).clone()).delete_project(&query.project_path)?;
    Ok(Json(DeletedProjectRecordWithStatus::from(deleted)))
}

async fn list_project_conversations(
    State(settings): State<Arc<SparkSettings>>,
    payload: Result<Query<ProjectConversationsQuery>, QueryRejection>,
) -> ApiResult<Vec<ConversationSummary>> {
    let query = query_payload(payload)?;
    WorkspaceProjectService::new((*settings).clone())
        .list_project_conversations(&query.project_path)
        .map(Json)
        .map_err(Into::into)
}

async fn get_conversation(
    State(settings): State<Arc<SparkSettings>>,
    AxumPath(conversation_id): AxumPath<String>,
    payload: Result<Query<ConversationQuery>, QueryRejection>,
) -> ApiResult<Value> {
    let query = optional_query_payload(payload)?;
    WorkspaceConversationService::new((*settings).clone())
        .get_snapshot(&conversation_id, query.project_path.as_deref())
        .map(Json)
        .map_err(Into::into)
}

async fn update_conversation_settings(
    State(settings): State<Arc<SparkSettings>>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    AxumPath(conversation_id): AxumPath<String>,
    payload: Result<Json<ConversationSettingsUpdate>, JsonRejection>,
) -> ApiResult<Value> {
    let request = json_payload(payload)?;
    let project_path = request.project_path.clone();
    let service = WorkspaceConversationService::new((*settings).clone());
    let updated = service.update_conversation_settings(&conversation_id, request)?;
    publish_conversation_snapshot(&settings, &live_hub, &conversation_id, &project_path);
    Ok(Json(updated))
}

async fn send_conversation_turn(
    State(settings): State<Arc<SparkSettings>>,
    State(runtime_handler_runner_factory): State<attractor_api::RuntimeHandlerRunnerFactory>,
    State(agent_turn_backend): State<Arc<dyn AgentTurnBackend>>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    AxumPath(conversation_id): AxumPath<String>,
    payload: Result<Json<ConversationTurnRequest>, JsonRejection>,
) -> ApiResult<Value> {
    let request = json_payload(payload)?;
    let project_path = request.project_path.clone();
    let service = conversation_service(
        &settings,
        &runtime_handler_runner_factory,
        &agent_turn_backend,
    );
    let before_revision = current_conversation_revision(&service, &conversation_id, &project_path);
    let conversation_id_for_start = conversation_id.clone();
    let service_for_start = service.clone();
    let (prepared, started_snapshot) = tokio::task::spawn_blocking(move || {
        service_for_start.start_turn(&conversation_id_for_start, request)
    })
    .await
    .map_err(|error| {
        WorkspaceApiError(WorkspaceError::Internal(format!(
            "conversation turn start task failed: {error}"
        )))
    })??;
    publish_conversation_after(
        &settings,
        &live_hub,
        &conversation_id,
        &prepared.project_path,
        before_revision,
    );

    let started_revision = started_snapshot
        .get("revision")
        .and_then(Value::as_i64)
        .unwrap_or(before_revision);
    let published_revision = Arc::new(AtomicI64::new(started_revision));
    let conversation_id_for_progress = conversation_id.clone();
    let project_path_for_progress = prepared.project_path.clone();
    let live_hub_for_progress = live_hub.clone();
    let published_revision_for_progress = Arc::clone(&published_revision);
    let settings_for_completion = settings.clone();
    let live_hub_for_completion = live_hub.clone();
    let service_for_completion = service.clone();
    let prepared_for_completion = prepared.clone();
    let started_snapshot_for_completion = started_snapshot.clone();
    tokio::spawn(async move {
        let completion_conversation_id = prepared_for_completion.conversation_id.clone();
        let completion_project_path = prepared_for_completion.project_path.clone();
        let completion_result = tokio::task::spawn_blocking(move || {
            service_for_completion.complete_started_turn_with_progress_payloads(
                prepared_for_completion,
                started_snapshot_for_completion,
                move |payload| {
                    let revision = payload
                        .get("revision")
                        .and_then(Value::as_i64)
                        .unwrap_or(started_revision);
                    published_revision_for_progress.fetch_max(revision, Ordering::Relaxed);
                    live_hub_for_progress.publish(conversation_event_envelope(
                        &conversation_id_for_progress,
                        &project_path_for_progress,
                        payload,
                        revision,
                    ));
                },
            )
        })
        .await;

        let after_revision = published_revision.load(Ordering::Relaxed);
        match completion_result {
            Ok(Ok(_)) => publish_conversation_after(
                &settings_for_completion,
                &live_hub_for_completion,
                &completion_conversation_id,
                &completion_project_path,
                after_revision,
            ),
            Ok(Err(_error)) => {
                publish_conversation_after(
                    &settings_for_completion,
                    &live_hub_for_completion,
                    &completion_conversation_id,
                    &completion_project_path,
                    after_revision,
                );
            }
            Err(_error) => {
                publish_conversation_after(
                    &settings_for_completion,
                    &live_hub_for_completion,
                    &completion_conversation_id,
                    &completion_project_path,
                    after_revision,
                );
            }
        }
    });
    Ok(Json(started_snapshot))
}

async fn answer_conversation_request_user_input(
    State(settings): State<Arc<SparkSettings>>,
    State(runtime_handler_runner_factory): State<attractor_api::RuntimeHandlerRunnerFactory>,
    State(agent_turn_backend): State<Arc<dyn AgentTurnBackend>>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    AxumPath((conversation_id, request_id)): AxumPath<(String, String)>,
    payload: Result<Json<ConversationRequestUserInputAnswerRequest>, JsonRejection>,
) -> ApiResult<Value> {
    let request = json_payload(payload)?;
    let project_path = request.project_path.clone();
    let service = conversation_service(
        &settings,
        &runtime_handler_runner_factory,
        &agent_turn_backend,
    );
    let before_revision = current_conversation_revision(&service, &conversation_id, &project_path);
    let conversation_id_for_answer = conversation_id.clone();
    let request_id_for_answer = request_id.clone();
    let updated = tokio::task::spawn_blocking(move || {
        service.submit_request_user_input_answer(
            &conversation_id_for_answer,
            &request_id_for_answer,
            request,
        )
    })
    .await
    .map_err(|error| {
        WorkspaceApiError(WorkspaceError::Internal(format!(
            "request-user-input answer task failed: {error}"
        )))
    })??;
    publish_conversation_after(
        &settings,
        &live_hub,
        &conversation_id,
        &project_path,
        before_revision,
    );
    Ok(Json(updated))
}

async fn create_flow_run_request_by_handle(
    State(settings): State<Arc<SparkSettings>>,
    State(runtime_handler_runner_factory): State<attractor_api::RuntimeHandlerRunnerFactory>,
    State(agent_turn_backend): State<Arc<dyn AgentTurnBackend>>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    AxumPath(conversation_handle): AxumPath<String>,
    payload: Result<Json<FlowRunRequestCreateByHandleRequest>, JsonRejection>,
) -> ApiResult<FlowRunRequestCreateResponse> {
    let request = json_payload(payload)?;
    let response = conversation_service(
        &settings,
        &runtime_handler_runner_factory,
        &agent_turn_backend,
    )
    .create_flow_run_request_by_handle(&conversation_handle, request)?;
    publish_conversation_snapshot(
        &settings,
        &live_hub,
        &response.conversation_id,
        &response.project_path,
    );
    Ok(Json(response))
}

async fn review_flow_run_request(
    State(settings): State<Arc<SparkSettings>>,
    State(runtime_handler_runner_factory): State<attractor_api::RuntimeHandlerRunnerFactory>,
    State(agent_turn_backend): State<Arc<dyn AgentTurnBackend>>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    AxumPath((conversation_id, request_id)): AxumPath<(String, String)>,
    payload: Result<Json<FlowRunRequestReviewRequest>, JsonRejection>,
) -> ApiResult<Value> {
    let request = json_payload(payload)?;
    let project_path = request.project_path.clone();
    let service = conversation_service(
        &settings,
        &runtime_handler_runner_factory,
        &agent_turn_backend,
    );
    let before_revision = current_conversation_revision(&service, &conversation_id, &project_path);
    let before_run_ids = current_conversation_run_ids(&service, &conversation_id, &project_path);
    let updated = service.review_flow_run_request(&conversation_id, &request_id, request)?;
    publish_conversation_after(
        &settings,
        &live_hub,
        &conversation_id,
        &project_path,
        before_revision,
    );
    publish_new_run_ids_from_value(&settings, &live_hub, &updated, &before_run_ids);
    Ok(Json(updated))
}

async fn review_proposed_plan(
    State(settings): State<Arc<SparkSettings>>,
    State(runtime_handler_runner_factory): State<attractor_api::RuntimeHandlerRunnerFactory>,
    State(agent_turn_backend): State<Arc<dyn AgentTurnBackend>>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    AxumPath((conversation_id, plan_id)): AxumPath<(String, String)>,
    payload: Result<Json<ProposedPlanReviewRequest>, JsonRejection>,
) -> ApiResult<Value> {
    let request = json_payload(payload)?;
    let project_path = request.project_path.clone();
    let service = conversation_service(
        &settings,
        &runtime_handler_runner_factory,
        &agent_turn_backend,
    );
    let before_revision = current_conversation_revision(&service, &conversation_id, &project_path);
    let before_run_ids = current_conversation_run_ids(&service, &conversation_id, &project_path);
    let updated = service.review_proposed_plan(&conversation_id, &plan_id, request)?;
    publish_conversation_after(
        &settings,
        &live_hub,
        &conversation_id,
        &project_path,
        before_revision,
    );
    publish_new_run_ids_from_value(&settings, &live_hub, &updated, &before_run_ids);
    Ok(Json(updated))
}

async fn launch_workspace_run(
    State(settings): State<Arc<SparkSettings>>,
    State(runtime_handler_runner_factory): State<attractor_api::RuntimeHandlerRunnerFactory>,
    State(agent_turn_backend): State<Arc<dyn AgentTurnBackend>>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    body: Bytes,
) -> Result<Response, WorkspaceApiError> {
    let request = match run_launch_payload(body) {
        Ok(request) => request,
        Err(response) => return Ok(response),
    };
    let payload = conversation_service(
        &settings,
        &runtime_handler_runner_factory,
        &agent_turn_backend,
    )
    .launch_workspace_run(request)?;
    publish_optional_conversation_snapshot_from_value(&settings, &live_hub, &payload);
    publish_run_ids_from_value(&settings, &live_hub, &payload);
    Ok(Json(payload).into_response())
}

async fn retry_workspace_run(
    State(settings): State<Arc<SparkSettings>>,
    State(runtime_handler_runner_factory): State<attractor_api::RuntimeHandlerRunnerFactory>,
    State(agent_turn_backend): State<Arc<dyn AgentTurnBackend>>,
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
        &runtime_handler_runner_factory,
        &agent_turn_backend,
    )
    .retry_workspace_run(&run_id, request)?;
    publish_optional_conversation_snapshot_from_value(&settings, &live_hub, &payload);
    publish_live_run_after(&settings, &live_hub, &run_id, before_sequence);
    publish_run_ids_from_value_except(&settings, &live_hub, &payload, Some(&run_id));
    Ok(Json(payload).into_response())
}

async fn continue_workspace_run(
    State(settings): State<Arc<SparkSettings>>,
    State(runtime_handler_runner_factory): State<attractor_api::RuntimeHandlerRunnerFactory>,
    State(agent_turn_backend): State<Arc<dyn AgentTurnBackend>>,
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
        &runtime_handler_runner_factory,
        &agent_turn_backend,
    )
    .continue_workspace_run(&run_id, request)?;
    publish_optional_conversation_snapshot_from_value(&settings, &live_hub, &payload);
    publish_live_run_after(&settings, &live_hub, &run_id, before_sequence);
    publish_run_ids_from_value_except(&settings, &live_hub, &payload, Some(&run_id));
    Ok(Json(payload).into_response())
}

async fn delete_conversation(
    State(settings): State<Arc<SparkSettings>>,
    AxumPath(conversation_id): AxumPath<String>,
    payload: Result<Query<DeleteConversationQuery>, QueryRejection>,
) -> ApiResult<ConversationDeleteResponse> {
    let query = query_payload(payload)?;
    WorkspaceConversationService::new((*settings).clone())
        .delete_conversation(&conversation_id, &query.project_path)
        .map(Json)
        .map_err(Into::into)
}

async fn get_conversation_segment_tool_output(
    State(settings): State<Arc<SparkSettings>>,
    AxumPath((conversation_id, segment_id)): AxumPath<(String, String)>,
    payload: Result<Query<ConversationQuery>, QueryRejection>,
) -> ApiResult<Value> {
    let query = optional_query_payload(payload)?;
    WorkspaceConversationService::new((*settings).clone())
        .get_segment_tool_output(&conversation_id, &segment_id, query.project_path.as_deref())
        .map(Json)
        .map_err(Into::into)
}

async fn deprecated_conversation_events(
    State(settings): State<Arc<SparkSettings>>,
    AxumPath(_conversation_id): AxumPath<String>,
) -> impl IntoResponse {
    (
        StatusCode::GONE,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        WorkspaceConversationService::new((*settings).clone())
            .deprecated_events_response()
            .to_string(),
    )
}

async fn workspace_live_events(
    State(settings): State<Arc<SparkSettings>>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    payload: Result<Query<RawLiveQuery>, QueryRejection>,
) -> Result<Response, WorkspaceApiError> {
    let raw_query = optional_query_payload(payload)?;
    let query = validate_live_query(raw_query)?;
    let initial = initial_live_envelopes(&settings, &query)?;
    let has_initial = !initial.is_empty();
    let mut live_receiver = live_query_subscribes(&query).then(|| live_hub.subscribe());
    let stream = async_stream::stream! {
        for envelope in initial {
            yield Ok::<Bytes, Infallible>(sse_data_frame(&envelope));
        }

        let mut interval = time::interval(Duration::from_secs(15));
        interval.tick().await;
        if !has_initial {
            yield Ok::<Bytes, Infallible>(Bytes::from_static(b": keepalive\n\n"));
        }

        loop {
            if let Some(receiver) = live_receiver.as_mut() {
                tokio::select! {
                    received = receiver.recv() => {
                        match received {
                            Ok(envelope) if envelope_matches_query(&envelope, &query) => {
                                yield Ok::<Bytes, Infallible>(sse_data_frame(&envelope));
                            }
                            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                    _ = interval.tick() => {
                        yield Ok::<Bytes, Infallible>(Bytes::from_static(b": keepalive\n\n"));
                    }
                }
            } else {
                interval.tick().await;
                yield Ok::<Bytes, Infallible>(Bytes::from_static(b": keepalive\n\n"));
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
    WorkspaceProjectService::new((*settings).clone())
        .chat_models(&query.project_path)
        .map(Json)
        .map_err(Into::into)
}

async fn list_triggers(
    State(settings): State<Arc<SparkSettings>>,
) -> ApiResult<Vec<SerializedTrigger>> {
    WorkspaceTriggerService::new((*settings).clone())
        .list_triggers()
        .map(Json)
        .map_err(Into::into)
}

async fn create_trigger(
    State(settings): State<Arc<SparkSettings>>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    payload: Result<Json<TriggerCreateRequest>, JsonRejection>,
) -> ApiResult<SerializedTrigger> {
    let request = json_payload(payload)?;
    let trigger = WorkspaceTriggerService::new((*settings).clone()).create_trigger(request)?;
    if let Ok(value) = serde_json::to_value(&trigger) {
        live_hub.publish(trigger_upsert_envelope(&value));
    }
    Ok(Json(trigger))
}

async fn get_trigger(
    State(settings): State<Arc<SparkSettings>>,
    AxumPath(trigger_id): AxumPath<String>,
) -> ApiResult<SerializedTrigger> {
    WorkspaceTriggerService::new((*settings).clone())
        .get_trigger(&trigger_id)
        .map(Json)
        .map_err(Into::into)
}

async fn update_trigger(
    State(settings): State<Arc<SparkSettings>>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    AxumPath(trigger_id): AxumPath<String>,
    payload: Result<Json<TriggerUpdateRequest>, JsonRejection>,
) -> ApiResult<SerializedTrigger> {
    let request = json_payload(payload)?;
    let trigger =
        WorkspaceTriggerService::new((*settings).clone()).update_trigger(&trigger_id, request)?;
    if let Ok(value) = serde_json::to_value(&trigger) {
        live_hub.publish(trigger_upsert_envelope(&value));
    }
    Ok(Json(trigger))
}

async fn delete_trigger(
    State(settings): State<Arc<SparkSettings>>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
    AxumPath(trigger_id): AxumPath<String>,
) -> ApiResult<TriggerDeleteResponse> {
    let service = WorkspaceTriggerService::new((*settings).clone());
    let project_path = read_trigger_definition(&settings.config_dir, &trigger_id)
        .map_err(WorkspaceError::from)?
        .and_then(|definition| definition.action.project_path);
    let deleted = service.delete_trigger(&trigger_id)?;
    if let Ok(value) = serde_json::to_value(&deleted) {
        live_hub.publish(trigger_delete_envelope(&value, project_path));
    }
    Ok(Json(deleted))
}

async fn post_trigger_webhook(
    State(settings): State<Arc<SparkSettings>>,
    State(live_hub): State<Arc<WorkspaceLiveHub>>,
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
    let dispatch = WorkspaceTriggerService::new((*settings).clone()).dispatch_webhook(
        WebhookHandleRequest {
            webhook_key,
            webhook_secret,
            request_id: (!request_id.is_empty()).then_some(request_id),
            payload,
        },
    )?;
    publish_trigger_activation_outcomes(&settings, &live_hub, vec![dispatch.activation]);
    Ok(Json(dispatch.response).into_response())
}

async fn list_workspace_flows(
    State(settings): State<Arc<SparkSettings>>,
    payload: Result<Query<FlowSurfaceQuery>, QueryRejection>,
) -> ApiResult<Vec<WorkspaceFlowSummary>> {
    let query = optional_query_payload(payload)?;
    WorkspaceFlowService::new((*settings).clone())
        .list_flows(query.surface.as_deref())
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
    if let Some(flow_name) = flow_path.strip_suffix("/raw") {
        let raw = service.raw_flow(flow_name, query.surface.as_deref())?;
        return raw_flow_response(raw);
    }
    if let Some(flow_name) = flow_path.strip_suffix("/validate") {
        let payload = service.validate_flow(flow_name)?;
        return Ok(Json(payload).into_response());
    }
    if flow_path.is_empty() || flow_path.ends_with("/launch-policy") {
        return Err(WorkspaceError::NotFound("Not Found".to_string()).into());
    }
    let flow = service.describe_flow(flow_path, query.surface.as_deref())?;
    Ok(Json(flow).into_response())
}

async fn put_workspace_flow_dispatch(
    State(settings): State<Arc<SparkSettings>>,
    AxumPath(flow_path): AxumPath<String>,
    payload: Result<Json<WorkspaceFlowLaunchPolicyUpdate>, JsonRejection>,
) -> ApiResult<spark_workspace::WorkspaceFlowLaunchPolicyResponse> {
    let flow_path = flow_path.trim_start_matches('/');
    let Some(flow_name) = flow_path.strip_suffix("/launch-policy") else {
        return Err(WorkspaceError::NotFound("Not Found".to_string()).into());
    };
    let request = json_payload(payload)?;
    WorkspaceFlowService::new((*settings).clone())
        .update_launch_policy(flow_name, request)
        .map(Json)
        .map_err(Into::into)
}

async fn settings(State(settings): State<Arc<SparkSettings>>) -> Json<Value> {
    Json(workspace_settings(&settings))
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
    query.conversation_id.is_some()
        || query.run_id.is_some()
        || query.include_runs_overview
        || query.include_triggers
        || query.include_workflow_log
}

fn current_conversation_revision(
    service: &WorkspaceConversationService,
    conversation_id: &str,
    project_path: &str,
) -> i64 {
    service
        .get_snapshot(conversation_id, Some(project_path))
        .ok()
        .and_then(|snapshot| snapshot.get("revision").and_then(Value::as_i64))
        .unwrap_or(0)
}

fn current_conversation_run_ids(
    service: &WorkspaceConversationService,
    conversation_id: &str,
    project_path: &str,
) -> BTreeSet<String> {
    let mut run_ids = BTreeSet::new();
    if let Ok(snapshot) = service.get_snapshot(conversation_id, Some(project_path)) {
        collect_run_ids_from_value(&snapshot, &mut run_ids);
    }
    run_ids
}

fn publish_conversation_after(
    settings: &SparkSettings,
    live_hub: &WorkspaceLiveHub,
    conversation_id: &str,
    project_path: &str,
    before_revision: i64,
) {
    match conversation_envelopes_after(settings, conversation_id, project_path, before_revision) {
        Ok(envelopes) if !envelopes.is_empty() => {
            for envelope in envelopes {
                live_hub.publish(envelope);
            }
        }
        _ => publish_conversation_snapshot(settings, live_hub, conversation_id, project_path),
    }
}

fn publish_conversation_snapshot(
    settings: &SparkSettings,
    live_hub: &WorkspaceLiveHub,
    conversation_id: &str,
    project_path: &str,
) {
    if let Ok(envelope) = conversation_snapshot_envelope(settings, conversation_id, project_path) {
        live_hub.publish(envelope);
    }
}

fn publish_optional_conversation_snapshot_from_value(
    settings: &SparkSettings,
    live_hub: &WorkspaceLiveHub,
    value: &Value,
) {
    let Some(conversation_id) = value
        .get("conversation_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    else {
        return;
    };
    let Some(project_path) = value
        .get("project_path")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    else {
        return;
    };
    publish_conversation_snapshot(settings, live_hub, conversation_id, project_path);
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

fn publish_new_run_ids_from_value(
    settings: &SparkSettings,
    live_hub: &WorkspaceLiveHub,
    value: &Value,
    before_run_ids: &BTreeSet<String>,
) {
    let mut after_run_ids = BTreeSet::new();
    collect_run_ids_from_value(value, &mut after_run_ids);
    for run_id in after_run_ids.difference(before_run_ids) {
        publish_live_run_after(settings, live_hub, run_id, None);
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

fn raw_flow_response(
    flow: spark_workspace::WorkspaceFlowRaw,
) -> Result<Response, WorkspaceApiError> {
    static X_SPARK_FLOW_NAME: HeaderName = HeaderName::from_static("x-spark-flow-name");
    let flow_name = HeaderValue::from_str(&flow.name).map_err(|error| {
        WorkspaceError::Internal(format!("Invalid flow name response header: {error}"))
    })?;
    let mut response = (StatusCode::OK, flow.content).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/vnd.graphviz; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(X_SPARK_FLOW_NAME.clone(), flow_name);
    Ok(response)
}
