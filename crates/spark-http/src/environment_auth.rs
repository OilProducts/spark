//! Who may call this server, and how another environment gets a token.
//!
//! Any request may present `Authorization: Bearer <token>` with a token this
//! home issued and has not revoked. Without one, only this machine's owner
//! is trusted: a direct loopback connection to a loopback or wildcard
//! listener, naming that listener in `Host`, with no `Forwarded` or
//! `X-Forwarded-*` header and no cross-site browser metadata. A reverse
//! proxy on this host adds a forwarded header to what it relays, so its
//! requests need a token.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Path, Request, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use spark_common::paths::{Environment, ProcessEnvironment};
use spark_common::settings::SparkSettings;
use spark_storage::environment::{EnvironmentRepository, PROTOCOL_VERSION, UI_API_VERSION};

use crate::{HttpAppState, WorkspaceApiError};

/// Largest pairing request body read.
const PAIR_BODY_LIMIT: usize = 8 * 1024;
/// Pairing attempts one peer address may make per window.
const PAIR_ATTEMPTS: u32 = 10;
const PAIR_WINDOW: Duration = Duration::from_secs(60);

/// The address of the listener a request arrived on, which each serve site
/// attaches beside the connection's peer address.
#[derive(Clone, Copy, Debug)]
pub struct ListenerAddr(pub SocketAddr);

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

/// The one value of `name`; `Err` when it is repeated or not text.
fn single(headers: &HeaderMap, name: HeaderName) -> Result<Option<&str>, ()> {
    let mut values = headers.get_all(name).iter();
    match (values.next(), values.next()) {
        (None, _) => Ok(None),
        (Some(value), None) => value.to_str().map(Some).map_err(|_| ()),
        _ => Err(()),
    }
}

/// A host as an origin compares it: a DNS name, an IPv4 literal and an IPv6
/// literal stay distinct, even an IPv6-mapped IPv4 one.
#[derive(PartialEq)]
enum Host {
    Name(String),
    Ip(IpAddr),
}

/// `host[:port]` with a bracketed IPv6 literal, and the port, 80 when
/// omitted. Anything else, such as user information, a path or a list,
/// is malformed.
fn authority(value: &str) -> Option<(Host, u16)> {
    let (host, port) = match value.strip_prefix('[') {
        Some(rest) => {
            let (ip, port) = rest.split_once(']')?;
            (Host::Ip(IpAddr::V6(ip.parse().ok()?)), port)
        }
        None => {
            let (name, port) = value.split_at(value.find(':').unwrap_or(value.len()));
            let host = match name.parse::<Ipv4Addr>() {
                Ok(ip) => Host::Ip(IpAddr::V4(ip)),
                Err(_)
                    if !name.is_empty()
                        && name
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || b"-.".contains(&byte)) =>
                {
                    Host::Name(name.to_ascii_lowercase())
                }
                Err(_) => return None,
            };
            (host, port)
        }
    };
    let port = match port.strip_prefix(':') {
        None if port.is_empty() => 80,
        Some(digits) if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) => {
            digits.parse().ok()?
        }
        _ => return None,
    };
    Some((host, port))
}

/// Whether `host` names the loopback or wildcard `listener` at its port. The
/// name `localhost` stands for the standard loopback addresses only.
fn names_listener((host, port): &(Host, u16), listener: SocketAddr) -> bool {
    let bound = listener.ip().to_canonical();
    *port == listener.port()
        && match host {
            Host::Ip(ip) if bound.is_unspecified() => ip.to_canonical().is_loopback(),
            Host::Ip(ip) => ip.to_canonical() == bound,
            Host::Name(name) => {
                name == "localhost"
                    && (bound.is_unspecified()
                        || bound == IpAddr::V4(Ipv4Addr::LOCALHOST)
                        || bound == IpAddr::V6(Ipv6Addr::LOCALHOST))
            }
        }
}

/// The trusted owner without a token: a direct loopback connection to a
/// loopback or wildcard listener, with one `Host` naming that listener at
/// its actual port, an `Origin`, if any, of that same `http` authority, and
/// fetch metadata, if any, that is not cross-site. Missing connection or
/// listener information never qualifies.
fn is_local_owner(request: &Request) -> bool {
    let extensions = request.extensions();
    let (Some(ListenerAddr(listener)), Some(ConnectInfo(peer))) = (
        extensions.get::<ListenerAddr>(),
        extensions.get::<ConnectInfo<SocketAddr>>(),
    ) else {
        return false;
    };
    let bound = listener.ip().to_canonical();
    let headers = request.headers();
    if !(bound.is_loopback() || bound.is_unspecified())
        || !peer.ip().to_canonical().is_loopback()
        || forwarded(headers)
    {
        return false;
    }
    let Ok(Some(host)) = single(headers, header::HOST) else {
        return false;
    };
    let Some(host) = authority(host).filter(|host| names_listener(host, *listener)) else {
        return false;
    };
    let uri = request.uri();
    if uri.scheme().is_some_and(|scheme| scheme.as_str() != "http")
        || uri
            .authority()
            .is_some_and(|target| authority(target.as_str()).as_ref() != Some(&host))
    {
        return false;
    }
    let origin_matches = match single(headers, header::ORIGIN) {
        Ok(None) => true,
        Ok(Some(origin)) => origin
            .strip_prefix("http://")
            .and_then(authority)
            .is_some_and(|origin| origin == host),
        Err(()) => false,
    };
    origin_matches
        && matches!(
            single(headers, HeaderName::from_static("sec-fetch-site")),
            Ok(None | Some("same-origin" | "same-site" | "none"))
        )
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

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({"detail": "A peer token is required."})),
    )
        .into_response()
}

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
    // bound host still presents its token and must not revoke it, and a
    // revoked peer token must never fall back to local-owner access.
    if let Some(secret) = bearer(request.headers()) {
        match EnvironmentRepository::new(&settings.data_dir).peer_token_status(secret) {
            Ok(spark_storage::environment::PeerTokenStatus::Active(token_id)) => {
                request
                    .extensions_mut()
                    .insert(AuthenticatedPeerToken(token_id));
                return next.run(request).await;
            }
            Ok(spark_storage::environment::PeerTokenStatus::Revoked) => return unauthorized(),
            Ok(spark_storage::environment::PeerTokenStatus::Unknown) => {}
            Err(error) => return WorkspaceApiError(error.into()).into_response(),
        }
    }
    if is_local_owner(&request) {
        return next.run(request).await;
    }
    unauthorized()
}

/// The machine's name, read once.
// ponytail: COMPUTERNAME (Windows only), then `hostname`; falls back to "Spark".
fn display_name() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        resolve_display_name(computer_name(&ProcessEnvironment), || {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        })
    })
}

/// `COMPUTERNAME` names the machine only on Windows; elsewhere a stray value
/// must never override `hostname`.
#[cfg(windows)]
fn computer_name(env: &impl Environment) -> Option<String> {
    env.get_var("COMPUTERNAME")
}

#[cfg(not(windows))]
fn computer_name(_env: &impl Environment) -> Option<String> {
    None
}

/// The first trimmed, nonempty name of `computer_name`, then `hostname`;
/// otherwise "Spark".
fn resolve_display_name(
    computer_name: Option<String>,
    hostname: impl FnOnce() -> Option<String>,
) -> String {
    let nonempty = |name: String| Some(name.trim().to_string()).filter(|name| !name.is_empty());
    computer_name
        .and_then(nonempty)
        .or_else(|| hostname().and_then(nonempty))
        .unwrap_or_else(|| "Spark".to_string())
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
    if !crate::is_json_content_type(request.headers()) {
        return crate::unsupported_media_type();
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
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response, WorkspaceApiError> {
    let request = if body.is_empty() {
        PairingCodeRequest::default()
    } else if !crate::is_json_content_type(&headers) {
        return Ok(crate::unsupported_media_type());
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
    }))
    .into_response())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_prefers_a_nonempty_computer_name_then_hostname_then_spark() {
        let host = || Some(" host-box\n".to_string());
        assert_eq!(
            resolve_display_name(Some(" win-box ".into()), host),
            "win-box"
        );
        assert_eq!(resolve_display_name(Some(" \t".into()), host), "host-box");
        assert_eq!(resolve_display_name(None, host), "host-box");
        assert_eq!(resolve_display_name(Some(String::new()), || None), "Spark");
        assert_eq!(resolve_display_name(None, || Some("\n".into())), "Spark");
    }

    #[test]
    fn only_windows_reads_computer_name() {
        let env = [("COMPUTERNAME".to_string(), "other-box".to_string())];
        let name = resolve_display_name(computer_name(&env), || Some("host-box".into()));
        let expected = if cfg!(windows) {
            "other-box"
        } else {
            "host-box"
        };
        assert_eq!(name, expected);
    }
}
