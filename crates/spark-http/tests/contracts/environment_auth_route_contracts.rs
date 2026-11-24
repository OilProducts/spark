use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::{to_bytes, Body, Bytes};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderValue, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
use spark_http::ProxyTimeouts;
use spark_storage::environment::{EnvironmentLink, EnvironmentRepository};
use tokio::sync::oneshot;
use tower::ServiceExt;

const REMOTE: &str = "10.1.2.3:40000";

/// Requests that are not trusted as this machine: a remote peer, and a
/// loopback peer relaying for someone else.
const UNTRUSTED: &[(&str, &[(&str, &str)])] = &[
    (REMOTE, &[]),
    ("[2001:db8::1]:40000", &[]),
    ("127.0.0.1:40000", &[("x-forwarded-for", "203.0.113.9")]),
    ("127.0.0.1:40000", &[("X-Forwarded-Proto", "https")]),
    ("[::1]:40000", &[("forwarded", "for=203.0.113.9")]),
];

/// One route of each protected kind: both API families, owner routes, the
/// live stream, attachments and run artifacts.
const PROTECTED: &[(&str, &str, Option<&str>)] = &[
    ("GET", "/workspace/api/projects", None),
    ("GET", "/workspace/api/settings", None),
    // Refused before the body is read: a local caller gets a 422 for this.
    ("POST", "/workspace/api/runs/launch", Some("{")),
    (
        "GET",
        "/workspace/api/live/events?include_settings=true",
        None,
    ),
    (
        "POST",
        "/workspace/api/conversations/conversation-1/attachments",
        None,
    ),
    (
        "GET",
        "/workspace/api/conversations/conversation-1/attachments/attachment-1",
        None,
    ),
    (
        "POST",
        "/workspace/api/environment/pairing-codes",
        Some("{}"),
    ),
    ("GET", "/workspace/api/environment/tokens", None),
    ("GET", "/attractor/status", None),
    ("GET", "/attractor/runs", None),
    ("POST", "/attractor/pipelines", Some("{}")),
    ("GET", "/attractor/pipelines/run-1/artifacts", None),
    (
        "GET",
        "/attractor/pipelines/run-1/artifacts/notes.txt",
        None,
    ),
    ("GET", "/workspace", None),
    // Only the listed methods of the public routes are exempt.
    ("DELETE", "/environment", None),
    ("GET", "/environment/pair", None),
];

#[tokio::test]
async fn untrusted_requests_need_a_peer_token_on_every_protected_route() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = crate::store_app_default(settings(temp.path()));

    for &(peer, headers) in UNTRUSTED {
        for &(method, uri, body) in PROTECTED {
            let response = send(&app, method, uri, Some(peer), headers, body).await;
            let status = response.status();
            assert_eq!(
                (status, json(response).await),
                (
                    StatusCode::UNAUTHORIZED,
                    json!({"detail": "A peer token is required."})
                ),
                "{method} {uri} from {peer} {headers:?}"
            );
        }
    }
}

#[tokio::test]
async fn loopback_requests_without_forwarded_headers_are_trusted() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = crate::store_app_default(settings(temp.path()));

    // No connection info means an in-process caller.
    for peer in [
        None,
        Some("127.0.0.1:40000"),
        Some("127.8.9.10:40000"),
        Some("[::1]:40000"),
        Some("[::ffff:127.0.0.1]:40000"),
    ] {
        for (method, uri, expected) in [
            ("GET", "/workspace/api/projects", StatusCode::OK),
            ("GET", "/workspace/api/environment/tokens", StatusCode::OK),
            (
                "GET",
                "/workspace/api/live/events?include_settings=true",
                StatusCode::OK,
            ),
            (
                "GET",
                "/workspace/api/conversations/conversation-1/attachments/attachment-1",
                StatusCode::NOT_FOUND,
            ),
            ("GET", "/attractor/status", StatusCode::OK),
            ("GET", "/attractor/runs", StatusCode::OK),
            (
                "GET",
                "/attractor/pipelines/run-1/artifacts",
                StatusCode::NOT_FOUND,
            ),
        ] {
            // The live stream stays open; only its status is read.
            let status = send(&app, method, uri, peer, &[], None).await.status();
            assert_eq!(status, expected, "{method} {uri} from {peer:?}");
        }
        let launch = send(
            &app,
            "POST",
            "/workspace/api/runs/launch",
            peer,
            &[],
            Some("{"),
        )
        .await;
        assert_eq!(launch.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}

#[tokio::test]
async fn public_routes_serve_untrusted_requests() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let app = crate::store_app_default(settings.clone());
    let environment_id = crate::shared_store(&settings.data_dir)
        .environment_id()
        .to_string();

    for &(peer, headers) in UNTRUSTED {
        let index = send(&app, "GET", "/", Some(peer), headers, None).await;
        assert_eq!(index.status(), StatusCode::OK);
        assert!(text(index).await.contains("environment ui"));
        let asset = send(&app, "GET", "/assets/app.js", Some(peer), headers, None).await;
        assert_eq!(asset.status(), StatusCode::OK);
        let favicon = send(&app, "GET", "/favicon.ico", Some(peer), headers, None).await;
        assert_eq!(favicon.status(), StatusCode::OK);
        let missing = send(&app, "GET", "/assets/missing.js", Some(peer), headers, None).await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(json(missing).await, json!({"detail": "Asset not found"}));

        let descriptor = send(&app, "GET", "/environment", Some(peer), headers, None).await;
        assert_eq!(descriptor.status(), StatusCode::OK);
        assert_eq!(
            descriptor
                .headers()
                .get("access-control-allow-origin")
                .and_then(|value| value.to_str().ok()),
            Some("*")
        );
        let descriptor = json(descriptor).await;
        assert_eq!(descriptor["environment_id"], environment_id.as_str());
        assert_eq!(descriptor["protocol_version"], 1);
        assert_eq!(descriptor["ui_api_version"], 1);
        assert!(!descriptor["spark_version"]
            .as_str()
            .expect("spark version")
            .is_empty());
        assert!(!descriptor["display_name"]
            .as_str()
            .expect("display name")
            .is_empty());
    }
}

#[tokio::test]
async fn webhook_route_keeps_its_own_credentials_for_untrusted_requests() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = crate::store_app_default(settings(temp.path()));

    for &(peer, headers) in UNTRUSTED {
        let missing = send(
            &app,
            "POST",
            "/workspace/api/webhooks",
            Some(peer),
            headers,
            Some("{}"),
        )
        .await;
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            json(missing).await,
            json!({"detail": "Webhook key and secret headers are required."})
        );

        let credentials = [
            headers,
            &[
                ("X-Spark-Webhook-Key", "missing-key"),
                ("X-Spark-Webhook-Secret", "secret"),
            ][..],
        ]
        .concat();
        let unknown = send(
            &app,
            "POST",
            "/workspace/api/webhooks",
            Some(peer),
            &credentials,
            Some("{}"),
        )
        .await;
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            json(unknown).await,
            json!({"detail": "Unknown webhook key."})
        );
    }
}

#[tokio::test]
async fn paired_token_is_accepted_until_revoked() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let app = crate::store_app_default(settings.clone());
    let peer = "10.0.0.20:40000";

    let code = send(
        &app,
        "POST",
        "/workspace/api/environment/pairing-codes",
        None,
        &[],
        Some(r#"{"label": "laptop"}"#),
    )
    .await;
    assert_eq!(code.status(), StatusCode::OK);
    let code_payload = json(code).await;
    assert_eq!(
        code_payload["environment_id"],
        crate::shared_store(&settings.data_dir).environment_id()
    );
    let code = code_payload["code"].as_str().expect("code").to_string();
    let paired = send(
        &app,
        "POST",
        "/environment/pair",
        Some(peer),
        &[],
        Some(&json!({"code": code, "label": "ignored"}).to_string()),
    )
    .await;
    assert_eq!(paired.status(), StatusCode::OK);
    let paired = json(paired).await;
    assert_eq!(
        paired["environment_id"],
        crate::shared_store(&settings.data_dir).environment_id()
    );
    assert_eq!(paired["label"], "laptop");
    let token_id = paired["token_id"].as_str().expect("token id").to_string();
    let secret = paired["token"].as_str().expect("token").to_string();
    let bearer = format!("Bearer {secret}");

    for &(peer, headers) in UNTRUSTED {
        let headers = [headers, &[("authorization", bearer.as_str())][..]].concat();
        for uri in ["/workspace/api/projects", "/attractor/status"] {
            let status = send(&app, "GET", uri, Some(peer), &headers, None)
                .await
                .status();
            assert_eq!(status, StatusCode::OK, "{uri} from {peer} {headers:?}");
        }
    }
    for authorization in [
        "Bearer not-a-token".to_string(),
        format!("Basic {secret}"),
        "Bearer ".to_string(),
        secret.clone(),
    ] {
        let status = send(
            &app,
            "GET",
            "/workspace/api/projects",
            Some(peer),
            &[("authorization", authorization.as_str())],
            None,
        )
        .await
        .status();
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{authorization}");
    }

    let listed = send(
        &app,
        "GET",
        "/workspace/api/environment/tokens",
        None,
        &[],
        None,
    )
    .await;
    let listed = text(listed).await;
    assert!(listed.contains(&token_id));
    assert!(!listed.contains(&secret));

    let revoke_uri = format!("/workspace/api/environment/tokens/{token_id}/revoke");
    let revoked = send(&app, "POST", &revoke_uri, None, &[], None).await;
    assert_eq!(revoked.status(), StatusCode::OK);
    assert_eq!(
        json(revoked).await,
        json!({"token_id": token_id, "revoked": true})
    );
    let refused = send(
        &app,
        "GET",
        "/workspace/api/projects",
        Some(peer),
        &[("authorization", bearer.as_str())],
        None,
    )
    .await;
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        json(refused).await,
        json!({"detail": "A peer token is required."})
    );
    let unknown = send(
        &app,
        "POST",
        "/workspace/api/environment/tokens/unknown/revoke",
        None,
        &[],
        None,
    )
    .await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_pairs_consume_a_code_exactly_once() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = crate::store_app_default(settings(temp.path()));
    let code = send(
        &app,
        "POST",
        "/workspace/api/environment/pairing-codes",
        None,
        &[],
        None,
    )
    .await;
    let code = json(code).await["code"].as_str().expect("code").to_string();
    let body = json!({"code": code, "label": "racer"}).to_string();

    // Under the per-peer limit of ten attempts.
    let attempts = (0..8)
        .map(|_| {
            let request = request(
                "POST",
                "/environment/pair",
                Some("10.0.0.30:40000"),
                &[],
                Some(&body),
            );
            tokio::spawn(app.clone().oneshot(request))
        })
        .collect::<Vec<_>>();
    let mut statuses = Vec::new();
    for attempt in attempts {
        statuses.push(attempt.await.expect("task").expect("response").status());
    }
    statuses.sort();
    let mut expected = vec![StatusCode::FORBIDDEN; 7];
    expected.insert(0, StatusCode::OK);
    assert_eq!(statuses, expected);

    let tokens = send(
        &app,
        "GET",
        "/workspace/api/environment/tokens",
        None,
        &[],
        None,
    )
    .await;
    assert_eq!(
        json(tokens).await["tokens"].as_array().map(Vec::len),
        Some(1)
    );
}

#[tokio::test]
async fn pairing_is_rate_limited_per_peer_and_caps_its_body() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = crate::store_app_default(settings(temp.path()));
    let wrong = r#"{"code": "wrong", "label": ""}"#;

    for _ in 0..10 {
        let response = send(
            &app,
            "POST",
            "/environment/pair",
            Some("10.0.0.40:40000"),
            &[],
            Some(wrong),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            json(response).await,
            json!({"detail": "The pairing code is unknown, used or expired."})
        );
    }
    let limited = send(
        &app,
        "POST",
        "/environment/pair",
        Some("10.0.0.40:40001"),
        &[],
        Some(wrong),
    )
    .await;
    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
    let other_peer = send(
        &app,
        "POST",
        "/environment/pair",
        Some("10.0.0.41:40000"),
        &[],
        Some(wrong),
    )
    .await;
    assert_eq!(other_peer.status(), StatusCode::FORBIDDEN);

    let oversized = format!(r#"{{"code": "{}", "label": ""}}"#, "x".repeat(8 * 1024));
    let too_large = send(
        &app,
        "POST",
        "/environment/pair",
        Some("10.0.0.42:40000"),
        &[],
        Some(&oversized),
    )
    .await;
    assert_eq!(too_large.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let malformed = send(
        &app,
        "POST",
        "/environment/pair",
        Some("10.0.0.42:40000"),
        &[],
        Some(r#"{"code": "x", "extra": true}"#),
    )
    .await;
    assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn specific_bound_ip_trusts_only_direct_local_self_connections() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut bound = settings(temp.path());
    bound.connections.server_host = Some("10.1.2.3".to_string());
    let app = crate::store_app_default(bound);

    // Direct connection from the server's own bound IP (e.g. a local agent) passes.
    let self_local = send(
        &app,
        "GET",
        "/workspace/api/projects",
        Some(REMOTE),
        &[],
        None,
    )
    .await;
    assert_eq!(self_local.status(), StatusCode::OK);

    // A forwarded header on that IP still requires a bearer token.
    let proxied = send(
        &app,
        "GET",
        "/workspace/api/projects",
        Some(REMOTE),
        &[("x-forwarded-for", "203.0.113.9")],
        None,
    )
    .await;
    assert_eq!(proxied.status(), StatusCode::UNAUTHORIZED);

    // Any other remote peer still requires a bearer token.
    let other_remote = send(
        &app,
        "GET",
        "/workspace/api/projects",
        Some("10.1.2.4:40000"),
        &[],
        None,
    )
    .await;
    assert_eq!(other_remote.status(), StatusCode::UNAUTHORIZED);

    // An unspecified bind (`0.0.0.0`) never trusts non-loopback peers.
    let temp_any = tempfile::tempdir().expect("tempdir");
    let mut any = settings(temp_any.path());
    any.connections.server_host = Some("0.0.0.0".to_string());
    let app_any = crate::store_app_default(any);
    let remote_on_any = send(
        &app_any,
        "GET",
        "/workspace/api/projects",
        Some(REMOTE),
        &[],
        None,
    )
    .await;
    assert_eq!(remote_on_any.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn delegation_list_and_discovery_routes_are_mounted_and_protected() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = crate::store_app_default(settings(temp.path()));

    let local_list = send(
        &app,
        "GET",
        "/workspace/api/delegations?open=true",
        None,
        &[],
        None,
    )
    .await;
    assert_eq!(local_list.status(), StatusCode::OK);
    assert_eq!(json(local_list).await, json!({"delegations": []}));

    let missing_link = send(
        &app,
        "GET",
        "/workspace/api/environments/links/env-missing/projects",
        None,
        &[],
        None,
    )
    .await;
    assert_eq!(missing_link.status(), StatusCode::NOT_FOUND);

    let bad_flow_suffix = send(
        &app,
        "GET",
        "/workspace/api/environments/links/env-missing/flows/ops/run",
        None,
        &[],
        None,
    )
    .await;
    assert_eq!(bad_flow_suffix.status(), StatusCode::NOT_FOUND);

    for uri in [
        "/workspace/api/delegations",
        "/workspace/api/environments/links/env-b/projects",
        "/workspace/api/environments/links/env-b/flows",
        "/workspace/api/environments/links/env-b/flows/ops/run/files",
        "/workspace/api/environments/links/env-b/proxy/environment",
        "/workspace/api/environments/links/env-b/proxy/workspace/api/projects",
    ] {
        let remote = send(&app, "GET", uri, Some(REMOTE), &[], None).await;
        assert_eq!(remote.status(), StatusCode::UNAUTHORIZED, "{uri}");
    }
}

const PROXY: &str = "/workspace/api/environments/links/env-b/proxy/";

/// What the fake peer saw of one request.
#[derive(Clone, Debug)]
struct Seen {
    method: String,
    target: String,
    authorization: Option<String>,
    forwarded: Option<String>,
    body: Vec<u8>,
}

#[derive(Clone)]
struct FakePeer {
    seen: Arc<Mutex<Vec<Seen>>>,
    /// Dropped with the live stream's body, when the proxy hangs up.
    stream_closed: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

impl FakePeer {
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().expect("seen").clone()
    }
}

/// A real HTTP server answering as `env-b`, linked as `env-b` from
/// `settings`' home. The receiver resolves when its live stream is dropped.
async fn fake_peer(settings: &SparkSettings) -> (FakePeer, oneshot::Receiver<()>) {
    let (closed, stream_closed) = oneshot::channel();
    let peer = FakePeer {
        seen: Default::default(),
        stream_closed: Arc::new(Mutex::new(Some(closed))),
    };
    let app = Router::new()
        .fallback(fake_peer_route)
        .with_state(peer.clone());
    let origin = serve(app).await;
    link(settings, "env-b", &origin, "stored-peer-token");
    (peer, stream_closed)
}

async fn fake_peer_route(State(peer): State<FakePeer>, request: Request<Body>) -> Response {
    use axum::http::header;
    let (parts, body) = request.into_parts();
    let header = |name: &str| {
        parts
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    };
    let (method, target) = (parts.method.to_string(), parts.uri.to_string());
    let (authorization, forwarded) = (header("authorization"), header("forwarded"));
    let path = parts.uri.path().to_string();
    let body = to_bytes(body, usize::MAX)
        .await
        .expect("peer body")
        .to_vec();
    peer.seen.lock().expect("seen").push(Seen {
        method,
        target,
        authorization,
        forwarded,
        body: body.clone(),
    });
    let page = |content_type: &'static str| {
        [
            (header::CONTENT_TYPE, content_type),
            (header::CONTENT_DISPOSITION, "inline"),
            (header::CONTENT_SECURITY_POLICY, "default-src *"),
            (header::X_CONTENT_TYPE_OPTIONS, "sniff"),
            (header::SET_COOKIE, "session=peer"),
        ]
    };
    let chunk = |text: String| Ok::<_, std::io::Error>(Bytes::from(text));
    let mut response = match path.as_str() {
        "/environment" | "/workspace/api/anonymous" => {
            return Json(json!({"environment_id": "env-b"})).into_response()
        }
        "/workspace/api/denied" => {
            return (StatusCode::UNAUTHORIZED, Json(json!({"detail": "denied"}))).into_response()
        }
        "/workspace/api/page.html" => {
            (page("text/html; charset=utf-8"), "<p>hi</p>").into_response()
        }
        "/workspace/api/badge.svg" => (page("image/svg+xml"), "<svg/>").into_response(),
        "/workspace/api/pixel.png" => (
            [(header::CONTENT_TYPE, "image/png")],
            b"\x89PNG\r\n\x1a\n".to_vec(),
        )
            .into_response(),
        "/workspace/api/echo" => (
            [
                (header::CONTENT_TYPE, "application/octet-stream"),
                (
                    header::CONTENT_DISPOSITION,
                    r#"attachment; filename="echo.bin""#,
                ),
            ],
            body,
        )
            .into_response(),
        "/workspace/api/moved" => (
            StatusCode::FOUND,
            [(header::LOCATION, "/workspace/api/projects")],
        )
            .into_response(),
        "/workspace/api/broken" => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"detail": "peer broke"})),
        )
            .into_response(),
        "/workspace/api/silent" => std::future::pending().await,
        "/workspace/api/stalled" => Body::from_stream(async_stream::stream! {
            yield chunk("partial".into());
            std::future::pending::<()>().await;
        })
        .into_response(),
        "/workspace/api/trickle" => Body::from_stream(async_stream::stream! {
            for index in 0..15 {
                tokio::time::sleep(Duration::from_millis(50)).await;
                yield chunk(format!("{index};"));
            }
        })
        .into_response(),
        "/workspace/api/live/events" => {
            let closed = peer.stream_closed.lock().expect("closed").take();
            let events = async_stream::stream! {
                let _closed = closed;
                yield chunk("data: first\n\n".into());
                // Silent from here: only a hang-up can end the stream soon.
                std::future::pending::<()>().await;
            };
            (
                [(header::CONTENT_TYPE, "text/event-stream")],
                Body::from_stream(events),
            )
                .into_response()
        }
        _ => Json(json!({"ok": true})).into_response(),
    };
    response
        .headers_mut()
        .insert("x-spark-environment-id", HeaderValue::from_static("env-b"));
    response
}

/// Serves `app` on a loopback port with connection info; returns its origin.
async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let origin = format!("http://{}", listener.local_addr().expect("address"));
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
    });
    origin
}

fn link(settings: &SparkSettings, environment_id: &str, origin: &str, token: &str) {
    EnvironmentRepository::new(&settings.data_dir)
        .save_link(
            &EnvironmentLink {
                environment_id: environment_id.into(),
                origin: origin.into(),
                label: environment_id.into(),
                created_at: spark_storage::environment::now(),
            },
            token,
        )
        .expect("save link");
}

async fn proxy_with(
    app: &Router,
    method: &str,
    target: &str,
    body: Vec<u8>,
    timeouts: ProxyTimeouts,
) -> Response {
    let request = Request::builder()
        .method(method)
        .uri(format!("{PROXY}{target}"))
        .header("authorization", "Bearer caller-should-not-leak")
        .extension(timeouts)
        .body(Body::from(body))
        .expect("request");
    app.clone().oneshot(request).await.expect("response")
}

async fn proxy(app: &Router, target: &str) -> Response {
    proxy_with(app, "GET", target, Vec::new(), ProxyTimeouts::default()).await
}

fn header<'a>(response: &'a Response, name: &str) -> Option<&'a str> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
}

async fn bytes(response: Response) -> Vec<u8> {
    to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body")
        .to_vec()
}

#[tokio::test]
async fn linked_environment_proxy_forwards_raw_targets_and_refuses_unsafe_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cfg = settings(temp.path());
    let app = crate::store_app_default(cfg.clone());
    let (peer, _) = fake_peer(&cfg).await;

    // Refused before any upstream request: other surfaces, prefix
    // look-alikes, traversal in any encoding, and bad escapes.
    for bad in [
        "",
        "assets/app.js",
        "etc/passwd",
        "attractor",
        "attractor/",
        "attractorx/status",
        "environmentx",
        "environment/pair",
        "workspace/apix",
        "workspace/apis/projects",
        "workspace",
        "workspace/api/../environment",
        "workspace/api/./projects",
        "workspace/api/%2e%2e/environment",
        "workspace/api/%2E%2E/environment",
        "workspace/api/%2e/projects",
        "workspace/api/..%2Fenvironment",
        "workspace/api/runs/x%2F..%2F..%2Fenvironment",
        "attractor/pipelines/%2e%2e%2f%2e%2e%2fenvironment",
        "workspace/api/a%5Cb",
        "workspace/api/a%00b",
        "workspace/api/a%0Ab",
        "workspace/api/a%7Fb",
        "workspace/api/%zz",
        "workspace/api/%4",
        "workspace/api/%+1",
    ] {
        let refused = proxy(&app, bad).await;
        assert_eq!(refused.status(), StatusCode::NOT_FOUND, "{bad:?}");
    }
    let missing = send(
        &app,
        "GET",
        "/workspace/api/environments/links/env-missing/proxy/environment",
        None,
        &[],
        None,
    )
    .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    // A target the URL would re-encode is refused rather than altered.
    for drifting in ["workspace/api/a{b}", "workspace/api/projects?q=it's"] {
        let refused = proxy(&app, drifting).await;
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST, "{drifting}");
    }
    assert!(peer.seen().is_empty(), "{:?}", peer.seen());

    // The peer gets exactly the raw target: encoded slashes, `#` and `?`
    // in names, and encoded query values, unchanged.
    let targets = [
        "attractor/pipelines/run-1/artifacts/notes%23draft%3F.txt",
        "workspace/api/flows/team%2Fops/files",
        "workspace/api/flows/team%2fops/files?project_path=%2Fsrv%2Fmy%20repo&q=a%26b%3Dc%23d",
        "workspace/api/projects?",
        "workspace/api",
        "environment",
    ];
    for target in targets {
        let response = proxy(&app, target).await;
        assert_eq!(response.status(), StatusCode::OK, "{target}");
    }
    let seen = peer.seen();
    assert_eq!(
        seen.iter()
            .map(|seen| seen.target.as_str())
            .collect::<Vec<_>>(),
        targets.map(|target| format!("/{target}"))
    );
    for seen in seen {
        assert_eq!(
            seen.authorization.as_deref(),
            Some("Bearer stored-peer-token")
        );
        assert_eq!(seen.forwarded.as_deref(), Some("by=spark-proxy"));
    }
}

#[tokio::test]
async fn linked_environment_proxy_never_sends_a_literal_fragment_over_the_wire() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let temp = tempfile::tempdir().expect("tempdir");
    let cfg = settings(temp.path());
    let origin = serve(crate::store_app_default(cfg.clone())).await;
    let (peer, _) = fake_peer(&cfg).await;
    let address = origin.trim_start_matches("http://");

    // The HTTP parser drops `#…` from the request line before routing; the
    // peer gets the path without it, and encoded `%23` and `%3F` unchanged.
    for target in [
        "workspace/api/projects#../../environment",
        "attractor/pipelines/run-1/artifacts/notes%23draft%3F.txt#x",
    ] {
        let mut socket = tokio::net::TcpStream::connect(address)
            .await
            .expect("connect");
        socket
            .write_all(
                format!(
                    "GET {PROXY}{target} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .expect("request");
        let mut response = Vec::new();
        socket.read_to_end(&mut response).await.expect("response");
        assert!(
            response.starts_with(b"HTTP/1.1 200 "),
            "{}",
            String::from_utf8_lossy(&response)
        );
    }
    assert_eq!(
        peer.seen()
            .iter()
            .map(|seen| seen.target.as_str())
            .collect::<Vec<_>>(),
        [
            "/workspace/api/projects",
            "/attractor/pipelines/run-1/artifacts/notes%23draft%3F.txt"
        ]
    );
}

#[tokio::test]
async fn linked_environment_proxy_keeps_methods_bodies_redirects_and_errors() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cfg = settings(temp.path());
    let app = crate::store_app_default(cfg.clone());
    let (peer, _) = fake_peer(&cfg).await;
    let binary = (0..=255u8).cycle().take(70_000).collect::<Vec<_>>();

    for method in ["GET", "POST", "PATCH", "PUT", "DELETE"] {
        let body = if method == "GET" {
            Vec::new()
        } else {
            binary.clone()
        };
        let response = proxy_with(
            &app,
            method,
            "workspace/api/echo",
            body.clone(),
            ProxyTimeouts::default(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK, "{method}");
        assert_eq!(
            header(&response, "content-disposition"),
            Some(r#"attachment; filename="echo.bin""#)
        );
        assert_eq!(bytes(response).await, body, "{method}");
        let seen = peer.seen().pop().expect("seen");
        assert_eq!((seen.method.as_str(), seen.body == body), (method, true));
    }

    // A redirect comes back as is; the proxy does not follow it.
    let before = peer.seen().len();
    let moved = proxy(&app, "workspace/api/moved").await;
    assert_eq!(moved.status(), StatusCode::FOUND);
    assert_eq!(peer.seen().len(), before + 1);

    let broken = proxy(&app, "workspace/api/broken").await;
    assert_eq!(broken.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(json(broken).await, json!({"detail": "peer broke"}));
}

#[tokio::test]
async fn linked_environment_proxy_sandboxes_what_the_peer_serves() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cfg = settings(temp.path());
    let app = crate::store_app_default(cfg.clone());
    let _peer = fake_peer(&cfg).await;

    for (target, content_type, disposition, body) in [
        (
            "workspace/api/page.html",
            "text/html; charset=utf-8",
            Some("attachment"),
            &b"<p>hi</p>"[..],
        ),
        (
            "workspace/api/badge.svg",
            "image/svg+xml",
            Some("attachment"),
            b"<svg/>",
        ),
        (
            "workspace/api/pixel.png",
            "image/png",
            None,
            b"\x89PNG\r\n\x1a\n",
        ),
        (
            "workspace/api/projects",
            "application/json",
            None,
            br#"{"ok":true}"#,
        ),
    ] {
        let response = proxy(&app, target).await;
        assert_eq!(response.status(), StatusCode::OK, "{target}");
        assert_eq!(header(&response, "content-type"), Some(content_type));
        assert_eq!(header(&response, "content-disposition"), disposition);
        assert_eq!(
            header(&response, "content-security-policy"),
            Some("sandbox; default-src 'none'"),
            "{target}"
        );
        assert_eq!(header(&response, "x-content-type-options"), Some("nosniff"));
        assert_eq!(header(&response, "set-cookie"), None);
        assert_eq!(bytes(response).await, body, "{target}");
    }
}

#[tokio::test]
async fn linked_environment_proxy_requires_the_peer_to_answer_as_the_link() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cfg = settings(temp.path());
    let app = crate::store_app_default(cfg.clone());
    let _peer = fake_peer(&cfg).await;

    // The public descriptor may omit the header; a protected route may not,
    // except to refuse the token.
    let descriptor = proxy(&app, "environment").await;
    assert_eq!(descriptor.status(), StatusCode::OK);
    let anonymous = proxy(&app, "workspace/api/anonymous").await;
    assert_eq!(anonymous.status(), StatusCode::BAD_GATEWAY);
    assert!(json(anonymous).await["detail"]
        .as_str()
        .expect("detail")
        .contains("not the linked \"env-b\""));
    let denied = proxy(&app, "workspace/api/denied").await;
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

    // The same peer under another environment's link is refused.
    let origin = EnvironmentRepository::new(&cfg.data_dir)
        .link("env-b")
        .expect("link")
        .expect("env-b")
        .0
        .origin;
    link(&cfg, "env-c", &origin, "stored-peer-token");
    for target in ["workspace/api/projects", "attractor/status"] {
        let swapped = send(
            &app,
            "GET",
            &format!("/workspace/api/environments/links/env-c/proxy/{target}"),
            None,
            &[],
            None,
        )
        .await;
        assert_eq!(swapped.status(), StatusCode::BAD_GATEWAY, "{target}");
    }
}

#[tokio::test]
async fn linked_environment_proxy_times_out_silent_and_stalled_peers() {
    use futures_util::StreamExt;

    let temp = tempfile::tempdir().expect("tempdir");
    let cfg = settings(temp.path());
    let app = crate::store_app_default(cfg.clone());
    let _peer = fake_peer(&cfg).await;
    // A delegation `wait` holds up to MAX_WAIT before answering; the default
    // deadline must outlast it, so a pending poll gets its 200, not a 504.
    assert!(ProxyTimeouts::default().headers > spark_workspace::delegations::MAX_WAIT);
    let short = ProxyTimeouts {
        headers: Duration::from_millis(200),
        idle: Duration::from_millis(200),
    };

    let started = Instant::now();
    let silent = proxy_with(&app, "GET", "workspace/api/silent", Vec::new(), short).await;
    assert_eq!(silent.status(), StatusCode::GATEWAY_TIMEOUT);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(json(silent).await["detail"]
        .as_str()
        .expect("detail")
        .contains("sent no response within 200ms"));

    let stalled = proxy_with(&app, "GET", "workspace/api/stalled", Vec::new(), short).await;
    assert_eq!(stalled.status(), StatusCode::OK);
    let mut stream = stalled.into_body().into_data_stream();
    let first = stream.next().await.expect("chunk").expect("bytes");
    assert_eq!(&first[..], b"partial");
    let stalled_at = Instant::now();
    let ended = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("the stalled stream ends");
    assert!(matches!(ended, Some(Err(_)) | None), "{ended:?}");
    assert!(stalled_at.elapsed() >= short.idle);

    // Chunks closer together than the idle deadline keep the stream open
    // well past it.
    let steady = ProxyTimeouts {
        idle: Duration::from_millis(300),
        ..short
    };
    let started = Instant::now();
    let trickle = proxy_with(&app, "GET", "workspace/api/trickle", Vec::new(), steady).await;
    assert_eq!(trickle.status(), StatusCode::OK);
    let body = String::from_utf8(bytes(trickle).await).expect("utf-8");
    assert_eq!(
        body,
        (0..15).map(|index| format!("{index};")).collect::<String>()
    );
    assert!(started.elapsed() > steady.idle * 2);
}

#[tokio::test]
async fn linked_environment_proxy_streams_sse_and_hangs_up_with_the_client() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cfg = settings(temp.path());
    let origin = serve(crate::store_app_default(cfg.clone())).await;
    let (peer, stream_closed) = fake_peer(&cfg).await;

    // A real HTTP client on a served Spark router: the event arrives while
    // the peer's stream is still open.
    let client = reqwest::Client::new();
    let mut sse = client
        .get(format!("{origin}{PROXY}workspace/api/live/events"))
        .header("accept", "text/event-stream")
        .send()
        .await
        .expect("connect");
    assert_eq!(sse.status(), StatusCode::OK);
    assert_eq!(
        sse.headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream")
    );
    let first = tokio::time::timeout(Duration::from_secs(5), sse.chunk())
        .await
        .expect("first event")
        .expect("read")
        .expect("chunk");
    assert_eq!(&first[..], b"data: first\n\n");

    // Closing the client's socket drops the upstream connection right away,
    // though the peer has nothing more to send.
    drop((sse, client));
    tokio::time::timeout(Duration::from_secs(2), stream_closed)
        .await
        .expect("the peer's stream is dropped promptly")
        .expect_err("dropped, not sent");
    let seen = peer.seen().pop().expect("seen");
    assert_eq!(seen.target, "/workspace/api/live/events");
}

/// Pairs with `app` the way another environment would; returns the token
/// id and secret.
async fn pair(app: &Router) -> (String, String) {
    let code = send(
        app,
        "POST",
        "/workspace/api/environment/pairing-codes",
        None,
        &[],
        None,
    )
    .await;
    let code = json(code).await["code"].as_str().expect("code").to_string();
    let paired = send(
        app,
        "POST",
        "/environment/pair",
        None,
        &[],
        Some(&json!({"code": code}).to_string()),
    )
    .await;
    let paired = json(paired).await;
    (
        paired["token_id"].as_str().expect("token id").to_string(),
        paired["token"].as_str().expect("token").to_string(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn linked_spark_environments_check_identity_and_tokens_over_loopback() {
    let homes = [(); 3].map(|()| tempfile::tempdir().expect("tempdir"));
    let [a, b, c] = homes.each_ref().map(|home| settings(home.path()));
    let [app_a, app_b, app_c] = [&a, &b, &c].map(|cfg| crate::store_app_default(cfg.clone()));
    let b_id = crate::shared_store(&b.data_dir)
        .environment_id()
        .to_string();
    let c_id = crate::shared_store(&c.data_dir)
        .environment_id()
        .to_string();
    let (b_origin, c_origin) = (serve(app_b.clone()).await, serve(app_c.clone()).await);
    let (b_token_id, b_token) = pair(&app_b).await;
    let (_, c_token) = pair(&app_c).await;
    link(&a, &b_id, &b_origin, &b_token);
    let via = |target: &str| format!("/workspace/api/environments/links/{b_id}/proxy/{target}");

    // Every response names its environment; the identity route needs a token.
    let direct = send(&app_b, "GET", "/workspace/api/projects", None, &[], None).await;
    assert_eq!(
        header(&direct, "x-spark-environment-id"),
        Some(b_id.as_str())
    );
    let remote = send(
        &app_b,
        "GET",
        "/workspace/api/environment",
        Some(REMOTE),
        &[],
        None,
    )
    .await;
    assert_eq!(remote.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        header(&remote, "x-spark-environment-id"),
        Some(b_id.as_str())
    );

    let identity = send(
        &app_a,
        "GET",
        &via("workspace/api/environment"),
        None,
        &[],
        None,
    )
    .await;
    assert_eq!(identity.status(), StatusCode::OK);
    let identity = json(identity).await;
    assert_eq!(identity["environment_id"], b_id.as_str());
    assert_eq!(identity["protocol_version"], 1);
    assert_eq!(identity["ui_api_version"], 1);
    for field in ["display_name", "spark_version", "build_id"] {
        assert!(
            !identity[field].as_str().expect(field).is_empty(),
            "{field}"
        );
    }
    let projects = send(
        &app_a,
        "GET",
        &via("workspace/api/projects"),
        None,
        &[],
        None,
    )
    .await;
    assert_eq!(projects.status(), StatusCode::OK);

    // Polling a pending delegation through the link answers normally.
    let pending = EnvironmentRepository::new(&b.data_dir)
        .create_out(&spark_storage::environment::DelegationOut {
            delegation_id: "delegation-pending".into(),
            environment_id: c_id.clone(),
            project_path: "/srv/repo".into(),
            kind: "flow".into(),
            request: json!({}),
            request_sha256: "sha".into(),
            caller: "test".into(),
            request_key: "pending".into(),
            owner_conversation_id: None,
            phase: "created".into(),
            target_id: None,
            last_state: None,
            result: None,
            paused: None,
            cancel_requested_at: None,
            cancel_acknowledged_at: None,
            created_at: String::new(),
            updated_at: String::new(),
        })
        .expect("pending delegation");
    let polled = send(
        &app_a,
        "GET",
        &via(&format!(
            "workspace/api/delegations/{}/wait?timeout_ms=50",
            pending.delegation_id
        )),
        None,
        &[],
        None,
    )
    .await;
    assert_eq!(polled.status(), StatusCode::OK);
    let polled = json(polled).await;
    assert_eq!(
        (&polled["delegation_id"], &polled["timed_out"]),
        (&json!("delegation-pending"), &json!(true))
    );

    // Another environment now at B's link cannot answer for B.
    link(&a, &b_id, &c_origin, &c_token);
    for target in [
        "workspace/api/projects",
        "workspace/api/environment",
        "environment",
    ] {
        let swapped = send(&app_a, "GET", &via(target), None, &[], None).await;
        assert_eq!(swapped.status(), StatusCode::BAD_GATEWAY, "{target}");
        let detail = json(swapped).await["detail"]
            .as_str()
            .expect("detail")
            .to_string();
        assert!(detail.contains(&c_id), "{detail}");
    }

    // A loopback peer still checks the token the proxy relays.
    link(&a, &b_id, &b_origin, &b_token);
    let revoke = format!("/workspace/api/environment/tokens/{b_token_id}/revoke");
    let revoked = send(&app_b, "POST", &revoke, None, &[], None).await;
    assert_eq!(revoked.status(), StatusCode::OK);
    let refused = send(
        &app_a,
        "GET",
        &via("workspace/api/projects"),
        None,
        &[],
        None,
    )
    .await;
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        json(refused).await,
        json!({"detail": "A peer token is required."})
    );
}

#[tokio::test]
async fn artifact_downloads_are_exact_attachments_the_browser_does_not_sniff() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cfg = settings(temp.path());
    let app = crate::store_app_default(cfg.clone());
    let paths = attractor_runtime::RunStore::for_settings(&cfg)
        .create_run(attractor_runtime::CreateRunRequest {
            record: attractor_core::RunRecord::new(
                "run-download",
                cfg.project_root.to_string_lossy(),
            ),
            ..Default::default()
        })
        .expect("run");
    fs::create_dir_all(paths.artifacts_dir()).expect("artifacts");
    let json_bytes = b"{\"ok\": true}\n".to_vec();
    let binary = vec![0x89, b'P', b'N', b'G', 0x00, 0xff, 0xfe, b'\n'];
    fs::write(paths.artifacts_dir().join("result.json"), &json_bytes).expect("json");
    fs::write(paths.artifacts_dir().join("capture.bin"), &binary).expect("binary");
    let uri = |name: &str| format!("/attractor/pipelines/run-download/artifacts/artifacts/{name}");

    for (name, content_type, content) in [
        ("result.json", "application/json", &json_bytes),
        ("capture.bin", "application/octet-stream", &binary),
    ] {
        for query in ["?download=1", "?download=true"] {
            let download = send(
                &app,
                "GET",
                &format!("{}{query}", uri(name)),
                None,
                &[],
                None,
            )
            .await;
            assert_eq!(download.status(), StatusCode::OK, "{name}{query}");
            assert_eq!(header(&download, "content-type"), Some(content_type));
            assert_eq!(
                header(&download, "content-disposition"),
                Some(format!("attachment; filename=\"{name}\"").as_str())
            );
            assert_eq!(header(&download, "x-content-type-options"), Some("nosniff"));
            assert_eq!(&bytes(download).await, content, "{name}{query}");
        }
    }

    // Previews are unchanged: text as before, binary refused.
    let preview = send(&app, "GET", &uri("result.json"), None, &[], None).await;
    assert_eq!(preview.status(), StatusCode::OK);
    assert_eq!(header(&preview, "content-disposition"), None);
    assert_eq!(json(preview).await, json!("{\"ok\": true}\n"));
    let binary_preview = send(&app, "GET", &uri("capture.bin"), None, &[], None).await;
    assert_eq!(binary_preview.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

fn request(
    method: &str,
    uri: &str,
    peer: Option<&str>,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(peer) = peer {
        builder = builder.extension(ConnectInfo(peer.parse::<SocketAddr>().expect("peer")));
    }
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    builder
        .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
        .expect("request")
}

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    peer: Option<&str>,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> Response {
    app.clone()
        .oneshot(request(method, uri, peer, headers, body))
        .await
        .expect("response")
}

async fn text(response: Response) -> String {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    String::from_utf8(bytes.to_vec()).expect("utf-8 body")
}

async fn json(response: Response) -> Value {
    serde_json::from_str(&text(response).await).expect("json body")
}

fn settings(root: &Path) -> SparkSettings {
    crate::isolate_home();
    let ui_dir = root.join("ui");
    fs::create_dir_all(ui_dir.join("assets")).expect("ui assets");
    fs::write(ui_dir.join("index.html"), "<main>environment ui</main>\n").expect("index");
    fs::write(ui_dir.join("assets/app.js"), "// app\n").expect("script");
    fs::write(ui_dir.join("assets/spark-app-icon.png"), b"icon").expect("icon");
    SparkSettings {
        startup_sources: Default::default(),
        connections: Default::default(),
        providers: Default::default(),
        agents: Default::default(),
        project_root: root.join("project"),
        data_dir: root.join("spark-home"),
        config_dir: root.join("spark-home/config"),
        runtime_dir: root.join("spark-home/runtime"),
        runs_dir: None,
        ui_dir: Some(ui_dir),
        project_roots: Vec::<PathBuf>::new(),
    }
}
