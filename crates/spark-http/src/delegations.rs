//! Routes under `/workspace/api` for links to other environments, the
//! delegations this environment sends, and the ones it admits.
use std::time::{Duration, Instant};

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use spark_workspace::conversations::WorkspaceConversationService;
use spark_workspace::delegations::{
    self, DelegationAnswerRequest, DelegationStartRequest, IncomingCancel, IncomingDelegation,
    LinkAddRequest, MAX_WAIT,
};
use spark_workspace::{WorkspaceError, WorkspaceResult};

use crate::workspace::{conversation_service, json_payload};
use crate::{HttpAppState, WorkspaceApiError};

type ApiResult<T> = Result<Json<T>, WorkspaceApiError>;

pub(crate) fn router() -> Router<HttpAppState> {
    Router::new()
        .route("/environments/links", get(list_links).post(add_link))
        .route(
            "/environments/links/{environment_id}",
            axum::routing::delete(remove_link),
        )
        .route("/delegations", post(start))
        .route("/delegations/{delegation_id}", get(get_delegation))
        .route("/delegations/{delegation_id}/wait", get(wait))
        .route("/delegations/{delegation_id}/cancel", post(cancel))
        .route(
            "/delegations/{delegation_id}/questions/{question_id}/answer",
            post(answer),
        )
        .route("/delegations/incoming", post(admit))
        .route("/delegations/incoming/{delegation_id}", get(incoming_state))
        .route(
            "/delegations/incoming/{delegation_id}/cancel",
            post(cancel_incoming),
        )
        .route(
            "/delegations/incoming/{delegation_id}/questions/{question_id}/answer",
            post(answer_incoming),
        )
}

/// Runs storage and network work off the async runtime.
async fn blocking<T: Serialize + Send + 'static>(
    work: impl FnOnce() -> WorkspaceResult<T> + Send + 'static,
) -> ApiResult<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| WorkspaceApiError(WorkspaceError::Internal(error.to_string())))?
        .map(Json)
        .map_err(WorkspaceApiError)
}

fn conversations(state: &HttpAppState) -> WorkspaceConversationService {
    conversation_service(
        &state.settings,
        &state.conversation_store,
        &state.runtime_handler_runner_factory,
        &state.agent_turn_backend,
        &state.run_event_observer,
    )
}

async fn list_links(State(state): State<HttpAppState>) -> ApiResult<Value> {
    blocking(move || Ok(json!({"links": delegations::links(&state.settings)?}))).await
}

async fn add_link(
    State(state): State<HttpAppState>,
    payload: Result<Json<LinkAddRequest>, JsonRejection>,
) -> ApiResult<Value> {
    let request = json_payload(payload)?;
    blocking(move || {
        let link = delegations::add_link(&state.settings, &state.conversation_store, request)?;
        Ok(json!(link))
    })
    .await
}

async fn remove_link(
    State(state): State<HttpAppState>,
    Path(environment_id): Path<String>,
) -> ApiResult<Value> {
    blocking(move || {
        delegations::remove_link(&state.settings, &environment_id)?;
        Ok(json!({"environment_id": environment_id, "removed": true}))
    })
    .await
}

async fn start(
    State(state): State<HttpAppState>,
    payload: Result<Json<DelegationStartRequest>, JsonRejection>,
) -> ApiResult<Value> {
    let request = json_payload(payload)?;
    blocking(move || {
        let record = delegations::start(&state.settings, &state.conversation_store, request)?;
        Ok(json!(record))
    })
    .await
}

async fn get_delegation(
    State(state): State<HttpAppState>,
    Path(delegation_id): Path<String>,
) -> ApiResult<Value> {
    blocking(move || Ok(json!(delegations::get(&state.settings, &delegation_id)?))).await
}

#[derive(Deserialize)]
struct WaitQuery {
    timeout_ms: Option<u64>,
}

/// Holds until the delegation seals or the timeout (at most 30 s) passes;
/// returns the record with `timed_out`.
async fn wait(
    State(state): State<HttpAppState>,
    Path(delegation_id): Path<String>,
    Query(query): Query<WaitQuery>,
) -> ApiResult<Value> {
    let timeout = query
        .timeout_ms
        .map_or(MAX_WAIT, Duration::from_millis)
        .min(MAX_WAIT);
    let deadline = Instant::now() + timeout;
    loop {
        let (settings, id) = (state.settings.clone(), delegation_id.clone());
        let Json(record) = blocking(move || delegations::get(&settings, &id)).await?;
        let sealed = record.phase == "sealed";
        let now = Instant::now();
        if sealed || now >= deadline {
            let mut value = json!(record);
            value["timed_out"] = json!(!sealed);
            return Ok(Json(value));
        }
        tokio::time::sleep((deadline - now).min(Duration::from_millis(250))).await;
    }
}

async fn cancel(
    State(state): State<HttpAppState>,
    Path(delegation_id): Path<String>,
) -> ApiResult<Value> {
    blocking(move || {
        Ok(json!(delegations::cancel(
            &state.settings,
            &state.conversation_store,
            &delegation_id,
        )?))
    })
    .await
}

async fn answer(
    State(state): State<HttpAppState>,
    Path((delegation_id, question_id)): Path<(String, String)>,
    payload: Result<Json<DelegationAnswerRequest>, JsonRejection>,
) -> ApiResult<Value> {
    let request = json_payload(payload)?;
    blocking(move || delegations::answer(&state.settings, &delegation_id, &question_id, request))
        .await
}

async fn admit(
    State(state): State<HttpAppState>,
    payload: Result<Json<IncomingDelegation>, JsonRejection>,
) -> ApiResult<Value> {
    let request = json_payload(payload)?;
    blocking(move || delegations::admit(&conversations(&state), request)).await
}

async fn incoming_state(
    State(state): State<HttpAppState>,
    Path(delegation_id): Path<String>,
) -> ApiResult<Value> {
    blocking(move || delegations::incoming_state(&conversations(&state), &delegation_id)).await
}

async fn cancel_incoming(
    State(state): State<HttpAppState>,
    Path(delegation_id): Path<String>,
    payload: Result<Json<IncomingCancel>, JsonRejection>,
) -> ApiResult<Value> {
    let request = json_payload(payload)?;
    blocking(move || delegations::cancel_incoming(&conversations(&state), &delegation_id, request))
        .await
}

async fn answer_incoming(
    State(state): State<HttpAppState>,
    Path((delegation_id, question_id)): Path<(String, String)>,
    payload: Result<Json<DelegationAnswerRequest>, JsonRejection>,
) -> ApiResult<Value> {
    let request = json_payload(payload)?;
    blocking(move || {
        delegations::answer_incoming(
            &conversations(&state),
            &delegation_id,
            &question_id,
            request,
        )
    })
    .await
}
