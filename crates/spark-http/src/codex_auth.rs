use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use spark_agent_adapter::codex_app_server::auth::CodexLoginMethod;
use spark_workspace::WorkspaceError;

use crate::{HttpAppState, WorkspaceApiError};

pub(crate) fn router() -> Router<HttpAppState> {
    Router::new()
        .route("/connection", get(status))
        .route("/login", post(login).delete(cancel))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginRequest {
    method: CodexLoginMethod,
}

async fn status(State(state): State<HttpAppState>) -> Result<Response, WorkspaceApiError> {
    connection_action(state, Action::Status).await
}

async fn login(
    State(state): State<HttpAppState>,
    Json(request): Json<LoginRequest>,
) -> Result<Response, WorkspaceApiError> {
    connection_action(state, Action::Login(request.method)).await
}

async fn cancel(State(state): State<HttpAppState>) -> Result<Response, WorkspaceApiError> {
    connection_action(state, Action::Cancel).await
}

enum Action {
    Status,
    Login(CodexLoginMethod),
    Cancel,
}

async fn connection_action(
    state: HttpAppState,
    action: Action,
) -> Result<Response, WorkspaceApiError> {
    let status = tokio::task::spawn_blocking(move || {
        let mut connection = state.codex_connection.lock().map_err(|_| {
            WorkspaceError::Internal("Codex connection state is unavailable.".into())
        })?;
        if matches!(action, Action::Cancel) {
            return Ok(connection.cancel_login());
        }
        let native = spark_workspace::models::native_configuration(&state.settings)?;
        let working_dir = &state.settings.data_dir;
        match action {
            Action::Status => connection.status(working_dir, &native),
            Action::Login(method) => connection.start_login(working_dir, &native, method),
            Action::Cancel => unreachable!(),
        }
        .map_err(|error| WorkspaceError::ServiceUnavailable(error.message))
    })
    .await
    .map_err(|_| WorkspaceError::Internal("Codex connection task failed.".into()))??;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(status)).into_response())
}
