//! Routes under `/workspace/api` for links to other environments, the
//! delegations this environment sends, and the ones it admits.
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, HeaderName, HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::{any, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use spark_storage::environment::EnvironmentRepository;
use spark_workspace::conversations::WorkspaceConversationService;
use spark_workspace::delegations::{
    self, DelegationAnswerRequest, DelegationListFilter, DelegationStartRequest, IncomingCancel,
    IncomingDelegation, LinkAddRequest, MAX_WAIT,
};
use spark_workspace::{WorkspaceError, WorkspaceResult};

use crate::workspace::{conversation_service, json_payload, optional_query_payload};
use crate::{HttpAppState, WorkspaceApiError};

type ApiResult<T> = Result<Json<T>, WorkspaceApiError>;

pub(crate) fn router() -> Router<HttpAppState> {
    Router::new()
        .route("/environments/links", get(list_links).post(add_link))
        .route(
            "/environments/links/{environment_id}",
            axum::routing::delete(remove_link),
        )
        .route(
            "/environments/links/{environment_id}/projects",
            get(link_projects),
        )
        .route(
            "/environments/links/{environment_id}/flows",
            get(link_flows),
        )
        .route(
            "/environments/links/{environment_id}/flows/{*flow_path}",
            get(link_flow_files),
        )
        .route(
            "/environments/links/{environment_id}/proxy/{*rest}",
            any(proxy_linked_environment),
        )
        .route("/delegations", get(list_delegations).post(start))
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

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkFlowQuery {
    #[serde(default)]
    project_path: Option<String>,
}

async fn link_projects(
    State(state): State<HttpAppState>,
    Path(environment_id): Path<String>,
) -> ApiResult<Value> {
    blocking(move || delegations::link_projects(&state.settings, &environment_id)).await
}

async fn link_flows(
    State(state): State<HttpAppState>,
    Path(environment_id): Path<String>,
    query: Result<Query<LinkFlowQuery>, axum::extract::rejection::QueryRejection>,
) -> ApiResult<Value> {
    let query = optional_query_payload(query)?;
    blocking(move || {
        delegations::link_flows(
            &state.settings,
            &environment_id,
            query.project_path.as_deref(),
        )
    })
    .await
}

async fn link_flow_files(
    State(state): State<HttpAppState>,
    Path((environment_id, flow_path)): Path<(String, String)>,
    query: Result<Query<LinkFlowQuery>, axum::extract::rejection::QueryRejection>,
) -> ApiResult<Value> {
    let query = optional_query_payload(query)?;
    let flow_name = flow_path
        .trim_start_matches('/')
        .strip_suffix("/files")
        .ok_or_else(|| WorkspaceApiError(WorkspaceError::NotFound("Not Found".to_string())))?
        .to_string();
    blocking(move || {
        delegations::link_flow_files(
            &state.settings,
            &environment_id,
            &flow_name,
            query.project_path.as_deref(),
        )
    })
    .await
}

async fn list_delegations(
    State(state): State<HttpAppState>,
    query: Result<Query<DelegationListFilter>, axum::extract::rejection::QueryRejection>,
) -> ApiResult<Value> {
    let filter = optional_query_payload(query)?;
    blocking(move || {
        Ok(json!({
            "delegations": delegations::list(&state.settings, filter)?
        }))
    })
    .await
}

fn proxy_client() -> Result<&'static reqwest::Client, WorkspaceError> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| WorkspaceError::Internal(error.clone()))
}

/// Only Spark's own API surfaces may be proxied through a link, with no
/// traversal segments.
fn allowed_proxy_path(rest: &str) -> Option<&str> {
    let trimmed = rest.trim_start_matches('/');
    if trimmed.is_empty()
        || trimmed.contains('\\')
        || trimmed.bytes().any(|byte| byte < 0x20 || byte == 0x7f)
        || trimmed.split('/').any(|seg| matches!(seg, "." | ".."))
    {
        return None;
    }
    let allowed = trimmed == "environment"
        || trimmed == "workspace/api"
        || trimmed.starts_with("workspace/api/")
        || trimmed == "attractor"
        || trimmed.starts_with("attractor/");
    allowed.then_some(trimmed)
}

/// Streams HTTP, binary, and SSE requests to a linked environment, injecting
/// the stored bearer token on A's server so the token never reaches a client.
async fn proxy_linked_environment(
    State(state): State<HttpAppState>,
    Path((environment_id, rest)): Path<(String, String)>,
    request: Request,
) -> Result<Response, WorkspaceApiError> {
    let Some(path) = allowed_proxy_path(&rest) else {
        return Err(WorkspaceApiError(WorkspaceError::NotFound(
            "Not Found".to_string(),
        )));
    };
    let data_dir = state.settings.data_dir.clone();
    let env_id = environment_id.clone();
    let link_and_token = tokio::task::spawn_blocking(move || {
        EnvironmentRepository::new(&data_dir)
            .link(&env_id)
            .map_err(WorkspaceError::from)
    })
    .await
    .map_err(|error| WorkspaceApiError(WorkspaceError::Internal(error.to_string())))??;
    let Some((link, token)) = link_and_token else {
        return Err(WorkspaceApiError(WorkspaceError::NotFound(format!(
            "No link to environment {environment_id:?}."
        ))));
    };
    let method =
        reqwest::Method::from_bytes(request.method().as_str().as_bytes()).map_err(|_| {
            WorkspaceApiError(WorkspaceError::Validation(
                "Unsupported HTTP method.".into(),
            ))
        })?;
    let mut url = format!("{}/{path}", link.origin.trim_end_matches('/'));
    if let Some(query) = request.uri().query() {
        url.push('?');
        url.push_str(query);
    }
    let client = proxy_client().map_err(WorkspaceApiError)?;
    let mut upstream = client.request(method, &url).bearer_auth(&token);
    for name in [
        header::CONTENT_TYPE,
        header::ACCEPT,
        header::IF_NONE_MATCH,
        header::IF_MODIFIED_SINCE,
        header::RANGE,
        HeaderName::from_static("last-event-id"),
    ] {
        if let Some(value) = request.headers().get(&name) {
            upstream = upstream.header(name, value.clone());
        }
    }
    let body_stream = request.into_body().into_data_stream();
    upstream = upstream.body(reqwest::Body::wrap_stream(body_stream));
    let response = upstream.send().await.map_err(|error| {
        WorkspaceApiError(WorkspaceError::ServiceUnavailable(format!(
            "{}: {error}",
            link.origin
        )))
    })?;
    let status =
        StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = Response::builder().status(status);
    for name in [
        header::CONTENT_TYPE,
        header::CACHE_CONTROL,
        header::CONTENT_DISPOSITION,
        header::ETAG,
        header::CONTENT_LENGTH,
        HeaderName::from_static("x-accel-buffering"),
    ] {
        if let Some(value) = response.headers().get(&name) {
            if let Ok(header_value) = HeaderValue::from_bytes(value.as_bytes()) {
                builder = builder.header(name, header_value);
            }
        }
    }
    let stream = Body::from_stream(response.bytes_stream());
    builder
        .body(stream)
        .map_err(|error| WorkspaceApiError(WorkspaceError::Internal(error.to_string())))
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
