use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use spark_agent_adapter::codex_app_server::auth::CodexLoginMethod;
use spark_workspace::WorkspaceError;

use crate::{HttpAppState, WorkspaceApiError};

#[derive(Default)]
pub(crate) struct ConnectionState {
    connection: spark_agent_adapter::codex_app_server::auth::CodexConnection,
    observed: std::sync::Arc<std::sync::Mutex<Option<serde_json::Value>>>,
}

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
        let mut tracked = state.codex_connection.lock().map_err(|_| {
            WorkspaceError::Internal("Codex connection state is unavailable.".into())
        })?;
        if matches!(action, Action::Cancel) {
            return Ok(tracked.connection.cancel_login());
        }
        let native = spark_workspace::models::native_configuration(&state.settings)?;
        let working_dir = &state.settings.data_dir;
        let observed = tracked.observed.clone();
        let hub = state.live_hub.clone();
        let status = match action {
            Action::Status => tracked.connection.status(working_dir, &native),
            Action::Login(method) => tracked.connection.start_login_with_completion(
                working_dir,
                &native,
                method,
                move |status| {
                    if status.status == "connected" {
                        let mut observed =
                            observed.lock().unwrap_or_else(|error| error.into_inner());
                        *observed = Some(serde_json::json!([status.status, status.account]));
                        hub.publish_settings_change(
                            "workspace",
                            "codex",
                            None,
                            serde_json::Value::Null,
                        );
                    }
                },
            ),
            Action::Cancel => unreachable!(),
        }
        .map_err(|error| WorkspaceError::ServiceUnavailable(error.message))?;
        if status.status != "pending" {
            let current = serde_json::json!([status.status, status.account]);
            let mut observed = tracked
                .observed
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if observed
                .as_ref()
                .is_some_and(|previous| previous != &current)
            {
                state.live_hub.publish_settings_change(
                    "workspace",
                    "codex",
                    None,
                    serde_json::Value::Null,
                );
            }
            *observed = Some(current);
        }
        Ok::<_, WorkspaceError>(status)
    })
    .await
    .map_err(|_| WorkspaceError::Internal("Codex connection task failed.".into()))??;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(status)).into_response())
}
