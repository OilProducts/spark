//! Who may call this server, and how another environment gets a token.
//!
//! A request from this machine (a loopback peer) with no `Forwarded` or
//! `X-Forwarded-*` header is trusted, as every request was before. Any other
//! request needs `Authorization: Bearer <token>` with a token this home
//! issued and has not revoked. A reverse proxy on this host adds a forwarded
//! header to what it relays, so its requests count as remote.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Path, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
use spark_storage::environment::{EnvironmentRepository, PROTOCOL_VERSION, UI_API_VERSION};

use crate::{HttpAppState, WorkspaceApiError};

/// Largest pairing request body read.
const PAIR_BODY_LIMIT: usize = 8 * 1024;
/// Pairing attempts one peer address may make per window.
const PAIR_ATTEMPTS: u32 = 10;
const PAIR_WINDOW: Duration = Duration::from_secs(60);

/// Routes anyone may call: the page shell, the environment descriptor,
/// pairing, and the webhook, which checks its own key and secret.
fn is_exempt(method: &Method, path: &str) -> bool {
    path == "/"
        || path == "/favicon.ico"
        || path.starts_with("/assets/")
        || (method == Method::GET && path == "/environment")
        || (method == Method::POST
            && matches!(path, "/environment/pair" | "/workspace/api/webhooks"))
}

fn forwarded(headers: &HeaderMap) -> bool {
    headers.keys().any(|name| {
        let name = name.as_str();
        name == "forwarded" || name.starts_with("x-forwarded-")
    })
}

/// The specific interface IP this server bound to, if any. Unspecified
/// addresses (`0.0.0.0`, `::`) return `None` because agents reach them via
/// loopback.
fn bound_ip(settings: &SparkSettings) -> Option<IpAddr> {
    settings
        .connections
        .server_host
        .as_deref()
        .and_then(|host| host.trim_matches(['[', ']']).parse::<IpAddr>().ok())
        .map(|ip| ip.to_canonical())
        .filter(|ip| !ip.is_unspecified())
}

/// A request served without connection info has no network peer: it came
/// from this process, such as a test driving the router directly. Both
/// serve sites attach connection info. A local process connecting to a
/// specific non-loopback `--host` IP arrives from that same bound IP.
fn is_local(request: &Request, settings: &SparkSettings) -> bool {
    let local_bind = bound_ip(settings);
    !forwarded(request.headers())
        && request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .is_none_or(|ConnectInfo(peer)| {
                let peer = peer.ip().to_canonical();
                peer.is_loopback() || local_bind == Some(peer)
            })
}

/// The token of an `Authorization: Bearer <token>` header; the scheme name
/// is case-insensitive.
fn bearer(headers: &HeaderMap) -> Option<&str> {
    let (scheme, token) = headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

/// The ID of the peer token a bearer-authenticated request used. Never the
/// secret.
#[derive(Clone)]
struct AuthenticatedPeerToken(String);

/// The trust rule, applied before any handler reads the request body.
pub(crate) async fn require_trust(
    State(settings): State<Arc<SparkSettings>>,
    mut request: Request,
    next: Next,
) -> Response {
    if is_exempt(request.method(), request.uri().path()) {
        return next.run(request).await;
    }
    // Resolved before the local check: a peer paired over loopback or the
    // bound host still presents its token and must not revoke it.
    let token_id = bearer(request.headers()).and_then(|secret| {
        EnvironmentRepository::new(&settings.data_dir)
            .token_for_secret(secret)
            .ok()
            .flatten()
    });
    if let Some(token_id) = token_id {
        request
            .extensions_mut()
            .insert(AuthenticatedPeerToken(token_id));
        return next.run(request).await;
    }
    if is_local(&request, &settings) {
        return next.run(request).await;
    }
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({"detail": "A peer token is required."})),
    )
        .into_response()
}

/// The machine's name, read once.
// ponytail: checks COMPUTERNAME (Windows), then `hostname`; falls back to "Spark".
fn display_name() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        std::env::var("COMPUTERNAME")
            .ok()
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .or_else(|| {
                std::process::Command::new("hostname")
                    .output()
                    .ok()
                    .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
                    .filter(|name| !name.is_empty())
            })
            .unwrap_or_else(|| "Spark".to_string())
    })
}

/// `GET /environment`: public, readable from any page, so a client can check
/// a saved origin is the environment it expects before opening it.
pub(crate) async fn environment_descriptor(State(state): State<HttpAppState>) -> Response {
    let mut response = Json(json!({
        "environment_id": state.conversation_store.environment_id(),
        "display_name": display_name(),
        "protocol_version": PROTOCOL_VERSION,
        "ui_api_version": UI_API_VERSION,
        "spark_version": env!("CARGO_PKG_VERSION"),
    }))
    .into_response();
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    response
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PairRequest {
    code: String,
    #[serde(default)]
    label: String,
}

/// Whether `peer` may make another pairing attempt now.
fn pairing_allowed(peer: Option<IpAddr>) -> bool {
    static ATTEMPTS: OnceLock<Mutex<HashMap<Option<IpAddr>, (Instant, u32)>>> = OnceLock::new();
    let mut attempts = ATTEMPTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let now = Instant::now();
    attempts.retain(|_, (started, _)| now.duration_since(*started) < PAIR_WINDOW);
    let (_, count) = attempts.entry(peer).or_insert((now, 0));
    *count += 1;
    *count <= PAIR_ATTEMPTS
}

fn detail(status: StatusCode, detail: &str) -> Response {
    (status, Json(json!({"detail": detail}))).into_response()
}

/// `POST /environment/pair {code, label}`: public. Consumes the code and
/// issues a token in one transaction; the token is returned only here.
pub(crate) async fn pair(State(state): State<HttpAppState>, request: Request) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(peer)| peer.ip().to_canonical());
    if !pairing_allowed(peer) {
        return detail(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many pairing attempts; wait a minute.",
        );
    }
    let Ok(body) = axum::body::to_bytes(request.into_body(), PAIR_BODY_LIMIT).await else {
        return detail(StatusCode::PAYLOAD_TOO_LARGE, "The request is too large.");
    };
    let Ok(request) = serde_json::from_slice::<PairRequest>(&body) else {
        return detail(StatusCode::BAD_REQUEST, "Expected {\"code\", \"label\"}.");
    };
    match EnvironmentRepository::new(&state.settings.data_dir)
        .pair(request.code.trim(), &request.label)
    {
        Ok(Some((token, secret))) => Json(json!({
            "environment_id": state.conversation_store.environment_id(),
            "token_id": token.token_id,
            "label": token.label,
            "token": secret,
        }))
        .into_response(),
        Ok(None) => detail(
            StatusCode::FORBIDDEN,
            "The pairing code is unknown, used or expired.",
        ),
        Err(error) => WorkspaceApiError(error.into()).into_response(),
    }
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PairingCodeRequest {
    #[serde(default)]
    label: String,
}

/// Routes under `/workspace/api/environment`, for this machine's owner or a
/// paired environment, whose token grants what the owner can do.
pub(crate) fn workspace_router() -> Router<HttpAppState> {
    Router::new()
        .route("/environment/pairing-codes", post(create_pairing_code))
        .route("/environment/tokens", get(list_tokens))
        .route("/environment/tokens/{token_id}/revoke", post(revoke_token))
}

async fn create_pairing_code(
    State(state): State<HttpAppState>,
    body: axum::body::Bytes,
) -> Result<Json<Value>, WorkspaceApiError> {
    let request = if body.is_empty() {
        PairingCodeRequest::default()
    } else {
        serde_json::from_slice::<PairingCodeRequest>(&body).map_err(|error| {
            spark_workspace::WorkspaceError::Validation(format!("Invalid request: {error}"))
        })?
    };
    let (code, expires_at) = EnvironmentRepository::new(&state.settings.data_dir)
        .create_pairing_code(request.label.trim())
        .map_err(spark_workspace::WorkspaceError::from)?;
    Ok(Json(json!({
        "environment_id": state.conversation_store.environment_id(),
        "code": code,
        "expires_at": expires_at,
    })))
}

/// Each token says whether it is the one authenticating this request.
async fn list_tokens(
    State(settings): State<Arc<SparkSettings>>,
    current: Option<Extension<AuthenticatedPeerToken>>,
) -> Result<Json<Value>, WorkspaceApiError> {
    let current = current.map(|Extension(AuthenticatedPeerToken(id))| id);
    let tokens = EnvironmentRepository::new(&settings.data_dir)
        .tokens()
        .map_err(spark_workspace::WorkspaceError::from)?
        .into_iter()
        .map(|token| {
            let is_current = current.as_deref() == Some(token.token_id.as_str());
            let mut token = json!(token);
            token["is_current"] = json!(is_current);
            token
        })
        .collect::<Vec<_>>();
    Ok(Json(json!({"tokens": tokens})))
}

/// A peer cannot revoke the token its own connection uses; the owner, or
/// another peer, can.
async fn revoke_token(
    State(settings): State<Arc<SparkSettings>>,
    current: Option<Extension<AuthenticatedPeerToken>>,
    Path(token_id): Path<String>,
) -> Result<Json<Value>, WorkspaceApiError> {
    if current.is_some_and(|Extension(AuthenticatedPeerToken(id))| id == token_id) {
        return Err(spark_workspace::WorkspaceError::Conflict(
            "Cannot revoke the peer token authenticating this connection.".into(),
        )
        .into());
    }
    let repository = EnvironmentRepository::new(&settings.data_dir);
    let revoked = repository
        .revoke_token(&token_id)
        .map_err(spark_workspace::WorkspaceError::from)?;
    if !revoked
        && !repository
            .tokens()
            .map_err(spark_workspace::WorkspaceError::from)?
            .iter()
            .any(|token| token.token_id == token_id)
    {
        return Err(spark_workspace::WorkspaceError::NotFound(format!(
            "Unknown token: {token_id}"
        ))
        .into());
    }
    Ok(Json(json!({"token_id": token_id, "revoked": true})))
}
