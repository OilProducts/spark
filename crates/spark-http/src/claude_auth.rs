use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use spark_workspace::WorkspaceError;

use crate::{HttpAppState, WorkspaceApiError};

pub(crate) fn router() -> Router<HttpAppState> {
    Router::new()
        .route("/connection", get(status))
        .route("/login", post(login).delete(cancel))
        .route("/login/code", post(code))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CodeRequest {
    code: String,
}

async fn status(State(state): State<HttpAppState>) -> Result<Response, WorkspaceApiError> {
    connection_action(state, Action::Status).await
}

async fn login(State(state): State<HttpAppState>) -> Result<Response, WorkspaceApiError> {
    connection_action(state, Action::Login).await
}

async fn code(
    State(state): State<HttpAppState>,
    Json(request): Json<CodeRequest>,
) -> Result<Response, WorkspaceApiError> {
    connection_action(state, Action::Code(request.code)).await
}

async fn cancel(State(state): State<HttpAppState>) -> Result<Response, WorkspaceApiError> {
    connection_action(state, Action::Cancel).await
}

enum Action {
    Status,
    Login,
    Code(String),
    Cancel,
}

async fn connection_action(
    state: HttpAppState,
    action: Action,
) -> Result<Response, WorkspaceApiError> {
    let status = tokio::task::spawn_blocking(move || {
        let mut connection = state.claude_connection.lock().map_err(|_| {
            WorkspaceError::Internal("Claude Code connection state is unavailable.".into())
        })?;
        let hub = state.live_hub.clone();
        let result = match action {
            Action::Cancel => return Ok(connection.cancel_login()),
            Action::Code(code) => connection
                .submit_code(&code)
                .map_err(|error| WorkspaceError::Validation(error.message))?,
            Action::Status => {
                let native = spark_workspace::models::native_configuration(&state.settings)?;
                connection
                    .status(&native)
                    .map_err(|error| WorkspaceError::ServiceUnavailable(error.message))?
            }
            Action::Login => {
                let native = spark_workspace::models::native_configuration(&state.settings)?;
                connection
                    .start_login(&native, move |status| {
                        // The model catalog depends on the signed-in account.
                        if status.status == "connected" {
                            hub.publish_settings_change(
                                "workspace",
                                "claude_code",
                                None,
                                serde_json::Value::Null,
                            );
                        }
                    })
                    .map_err(|error| WorkspaceError::ServiceUnavailable(error.message))?
            }
        };
        Ok::<_, WorkspaceError>(result)
    })
    .await
    .map_err(|_| WorkspaceError::Internal("Claude Code connection task failed.".into()))??;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(status)).into_response())
}
