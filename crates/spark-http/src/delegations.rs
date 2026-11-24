//! Routes under `/workspace/api` for links to other environments, the
//! delegations this environment sends, and the ones it admits.
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
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
    // Read at startup, before an upgrade can replace the binary.
    build_id();
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
        .route("/environment", get(environment))
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

/// How long the linked-environment proxy waits on its upstream. Tests insert
/// one as a request extension to shorten them; a network client cannot.
#[derive(Clone, Copy, Debug)]
pub struct ProxyTimeouts {
    /// Until the response headers arrive.
    pub headers: Duration,
    /// Between body chunks; above the live stream's 15 s keepalive.
    pub idle: Duration,
}

impl Default for ProxyTimeouts {
    fn default() -> Self {
        Self {
            headers: Duration::from_secs(30),
            idle: Duration::from_secs(45),
        }
    }
}

/// The proxied path as the client sent it, still percent-encoded, so `%2F`,
/// `%23` and `%3F` reach the peer unchanged.
fn raw_proxy_path(uri: &Uri) -> Option<&str> {
    uri.path()
        .strip_prefix("/environments/links/")?
        .split_once('/')?
        .1
        .strip_prefix("proxy/")
}

fn percent_decoded(segment: &str) -> Option<Vec<u8>> {
    let mut bytes = segment.bytes();
    let mut decoded = Vec::with_capacity(segment.len());
    let hex = |byte: Option<u8>| char::from(byte?).to_digit(16);
    while let Some(byte) = bytes.next() {
        decoded.push(if byte == b'%' {
            (hex(bytes.next())? * 16 + hex(bytes.next())?) as u8
        } else {
            byte
        });
    }
    Some(decoded)
}

/// Only Spark's own API surfaces may be proxied through a link. The raw path
/// and each decoded segment are checked, so no encoding smuggles in a
/// traversal, backslash or control byte.
fn allowed_proxy_path(raw: &str) -> bool {
    let surface = raw == "environment"
        || raw == "workspace/api"
        || raw.starts_with("workspace/api/")
        || raw
            .strip_prefix("attractor/")
            .is_some_and(|rest| !rest.is_empty());
    surface
        && !raw
            .bytes()
            .any(|byte| matches!(byte, b'?' | b'#' | b'\\') || byte.is_ascii_control())
        && raw.split('/').all(|segment| {
            percent_decoded(segment).is_some_and(|decoded| {
                !decoded
                    .iter()
                    .any(|byte| *byte == b'\\' || byte.is_ascii_control())
                    && !decoded
                        .split(|byte| *byte == b'/')
                        .any(|part| part == b"." || part == b"..")
            })
        })
}

/// Types a browser renders as a document that can run script: HTML, XML,
/// SVG and every `+html` or `+xml` type.
fn renders_as_markup(content_type: Option<&HeaderValue>) -> bool {
    let essence = content_type
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    matches!(
        essence.as_str(),
        "text/html" | "text/xml" | "application/xml"
    ) || essence.ends_with("+html")
        || essence.ends_with("+xml")
}

fn is_attachment(disposition: Option<&HeaderValue>) -> bool {
    disposition
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("attachment"))
}

fn detail(status: StatusCode, detail: String) -> Response {
    (status, Json(json!({ "detail": detail }))).into_response()
}

/// Streams HTTP, binary, and SSE requests to a linked environment, injecting
/// the stored bearer token on A's server so the token never reaches a client.
/// The peer must answer as the linked environment, and its responses are
/// sandboxed so nothing it serves runs as this origin.
async fn proxy_linked_environment(
    State(state): State<HttpAppState>,
    Path((environment_id, _)): Path<(String, String)>,
    request: Request,
) -> Result<Response, WorkspaceApiError> {
    let not_found = || WorkspaceApiError(WorkspaceError::NotFound("Not Found".to_string()));
    let path = raw_proxy_path(request.uri())
        .filter(|path| allowed_proxy_path(path))
        .ok_or_else(not_found)?
        .to_string();
    let query = request.uri().query().map(str::to_string);
    let timeouts = request
        .extensions()
        .get::<ProxyTimeouts>()
        .copied()
        .unwrap_or_default();
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
    let origin = reqwest::Url::parse(&link.origin).map_err(|error| {
        WorkspaceApiError(WorkspaceError::Internal(format!(
            "Stored origin {:?} is invalid: {error}",
            link.origin
        )))
    })?;
    let mut url = origin.clone();
    url.set_path(&format!("/{path}"));
    url.set_query(query.as_deref());
    // The token goes only to the linked origin, for exactly the path sent.
    if url.scheme() != origin.scheme()
        || url.host_str() != origin.host_str()
        || url.port_or_known_default() != origin.port_or_known_default()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.path().strip_prefix('/') != Some(path.as_str())
        || url.query() != query.as_deref()
    {
        return Err(WorkspaceApiError(WorkspaceError::Validation(
            "The proxied path cannot be forwarded exactly as sent.".to_string(),
        )));
    }
    let client = proxy_client().map_err(WorkspaceApiError)?;
    let mut upstream = client
        .request(request.method().clone(), url)
        .bearer_auth(&token)
        // Marks the call as relayed, so even a loopback peer checks the token.
        .header(header::FORWARDED, "by=spark-proxy");
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
    let response = match tokio::time::timeout(timeouts.headers, upstream.send()).await {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            return Err(WorkspaceApiError(WorkspaceError::ServiceUnavailable(
                format!("{}: {error}", link.origin),
            )))
        }
        Err(_) => {
            let waited = timeouts.headers;
            return Ok(detail(
                StatusCode::GATEWAY_TIMEOUT,
                format!(
                    "{} sent no response within {waited:?}; check that it is running and reachable.",
                    link.origin
                ),
            ));
        }
    };
    let answered_as = response.headers().get(&crate::ENVIRONMENT_ID_HEADER);
    let public = path == "environment";
    let verified = answered_as.map_or(public, |id| id.as_bytes() == link.environment_id.as_bytes())
        || (!public && response.status() == StatusCode::UNAUTHORIZED);
    if !verified {
        return Ok(detail(
            StatusCode::BAD_GATEWAY,
            format!(
                "{} answered as environment {:?}, not the linked {:?}; link it again if it moved.",
                link.origin,
                answered_as.map(|id| String::from_utf8_lossy(id.as_bytes()).into_owned()),
                link.environment_id
            ),
        ));
    }
    let status = response.status();
    let mut headers = HeaderMap::new();
    for name in [
        header::CONTENT_TYPE,
        header::CACHE_CONTROL,
        header::CONTENT_DISPOSITION,
        header::ETAG,
        header::CONTENT_LENGTH,
        HeaderName::from_static("x-accel-buffering"),
    ] {
        if let Some(value) = response.headers().get(&name) {
            headers.insert(name, value.clone());
        }
    }
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("sandbox; default-src 'none'"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if renders_as_markup(headers.get(header::CONTENT_TYPE))
        && !is_attachment(headers.get(header::CONTENT_DISPOSITION))
    {
        headers.insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment"),
        );
    }
    // Dropping the body (a client disconnect) drops `response`, which closes
    // the upstream connection.
    let idle = timeouts.idle;
    let body = async_stream::stream! {
        let mut response = response;
        loop {
            match tokio::time::timeout(idle, response.chunk()).await {
                Ok(Ok(Some(chunk))) => yield Ok::<_, std::io::Error>(chunk),
                Ok(Ok(None)) => break,
                Ok(Err(error)) => {
                    yield Err(std::io::Error::other(error));
                    break;
                }
                Err(_) => {
                    yield Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("{} sent nothing for {idle:?}.", link.origin),
                    ));
                    break;
                }
            }
        }
    };
    Ok((status, headers, Body::from_stream(body)).into_response())
}

/// `GET /workspace/api/environment`: the public descriptor plus this build's
/// id, for a caller holding a token.
async fn environment(State(state): State<HttpAppState>) -> ApiResult<Value> {
    let internal = |error: String| WorkspaceApiError(WorkspaceError::Internal(error));
    let descriptor = crate::environment_auth::environment_descriptor(State(state)).await;
    let bytes = axum::body::to_bytes(descriptor.into_body(), usize::MAX)
        .await
        .map_err(|error| internal(error.to_string()))?;
    let mut descriptor: Value =
        serde_json::from_slice(&bytes).map_err(|error| internal(error.to_string()))?;
    descriptor["build_id"] = json!(build_id());
    Ok(Json(descriptor))
}

/// Tells apart two builds that share `CARGO_PKG_VERSION`: the running
/// binary's size and modification time, read once when the routes are built.
// ponytail: a copy that resets the file time reads as a new build; embed a
// build stamp if ids must match across installs of one build.
fn build_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        let version = env!("CARGO_PKG_VERSION");
        let nanos = |time: SystemTime| {
            time.duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_nanos())
        };
        match std::env::current_exe()
            .and_then(std::fs::metadata)
            .and_then(|metadata| Ok((metadata.len(), metadata.modified()?)))
        {
            Ok((len, modified)) => format!("{version}+{len:x}.{:x}", nanos(modified)),
            // Still unique to this build, since a new build needs a new process.
            Err(error) => {
                eprintln!("Unable to stamp the build id from the executable: {error}");
                format!("{version}+process.{:x}", nanos(SystemTime::now()))
            }
        }
    })
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
