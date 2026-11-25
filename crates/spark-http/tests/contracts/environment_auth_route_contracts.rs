use std::collections::BTreeMap;
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
        assert_eq!(descriptor["ui_api_version"], 2);
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

#[tokio::test]
async fn bearer_peer_sees_its_current_token_and_cannot_revoke_it() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = crate::store_app_default(settings(temp.path()));
    let (self_id, self_secret) = pair_token(&app, "laptop").await;
    let (other_id, other_secret) = pair_token(&app, "ci").await;
    let bearer = format!("Bearer {self_secret}");
    let auth = [("authorization", bearer.as_str())];
    let tokens_uri = "/workspace/api/environment/tokens";
    let self_revoke = format!("/workspace/api/environment/tokens/{self_id}/revoke");
    let loopback = "127.0.0.1:40000";
    // Token ID to the listed field, whatever order tokens come in.
    let field = |listed: &Value, name: &str| {
        listed["tokens"]
            .as_array()
            .expect("tokens")
            .iter()
            .map(|token| (token["token_id"].to_string(), token[name].clone()))
            .collect::<BTreeMap<_, _>>()
    };
    let ids = [json!(self_id).to_string(), json!(other_id).to_string()];

    // A peer paired over loopback or the bound host arrives from a local
    // address, still presenting its token.
    for peer in [REMOTE, loopback] {
        let listed = send(&app, "GET", tokens_uri, Some(peer), &auth, None).await;
        assert_eq!(listed.status(), StatusCode::OK, "{peer}");
        let listed = text(listed).await;
        assert!(!listed.contains(&self_secret) && !listed.contains(&other_secret));
        assert_eq!(
            field(&serde_json::from_str(&listed).expect("json"), "is_current"),
            BTreeMap::from([
                (ids[0].clone(), json!(true)),
                (ids[1].clone(), json!(false))
            ]),
            "{peer}"
        );

        let refused = send(&app, "POST", &self_revoke, Some(peer), &auth, None).await;
        assert_eq!(refused.status(), StatusCode::CONFLICT, "{peer}");
        assert_eq!(
            json(refused).await,
            json!({"detail": "Cannot revoke the peer token authenticating this connection."})
        );
        let still = send(&app, "GET", tokens_uri, Some(peer), &auth, None).await;
        assert_eq!(still.status(), StatusCode::OK, "{peer}");
        assert_eq!(
            field(&json(still).await, "revoked_at")[&ids[0]],
            Value::Null,
            "{peer}"
        );
    }

    // The owner's own requests use no token, so the owner may revoke it.
    let local = json(send(&app, "GET", tokens_uri, Some(loopback), &[], None).await).await;
    assert_eq!(
        field(&local, "is_current"),
        BTreeMap::from([
            (ids[0].clone(), json!(false)),
            (ids[1].clone(), json!(false))
        ])
    );
    let revoked = send(&app, "POST", &self_revoke, Some(loopback), &[], None).await;
    assert_eq!(revoked.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_bearer_peer_revokes_other_tokens_and_the_owner_revokes_its_token() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = crate::store_app_default(settings(temp.path()));
    let (self_id, self_secret) = pair_token(&app, "laptop").await;
    let (other_id, other_secret) = pair_token(&app, "ci").await;
    let bearer = format!("Bearer {self_secret}");
    let other_bearer = format!("Bearer {other_secret}");
    let tokens_uri = "/workspace/api/environment/tokens";

    let other_revoke = format!("/workspace/api/environment/tokens/{other_id}/revoke");
    let revoked = send(
        &app,
        "POST",
        &other_revoke,
        Some(REMOTE),
        &[("authorization", bearer.as_str())],
        None,
    )
    .await;
    assert_eq!(revoked.status(), StatusCode::OK);
    assert_eq!(
        json(revoked).await,
        json!({"token_id": other_id, "revoked": true})
    );
    let refused = send(
        &app,
        "GET",
        tokens_uri,
        Some(REMOTE),
        &[("authorization", other_bearer.as_str())],
        None,
    )
    .await;
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);

    let self_revoke = format!("/workspace/api/environment/tokens/{self_id}/revoke");
    let revoked = send(&app, "POST", &self_revoke, None, &[], None).await;
    assert_eq!(revoked.status(), StatusCode::OK);
    let refused = send(
        &app,
        "GET",
        tokens_uri,
        Some(REMOTE),
        &[("authorization", bearer.as_str())],
        None,
    )
    .await;
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_token_lookup_failure_is_an_error_not_local_owner_trust() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let app = crate::store_app_default(settings.clone());
    let (_, secret) = pair_token(&app, "laptop").await;
    let bearer = format!("Bearer {secret}");
    // The token lookup now fails. Dropped over SQL, not by deleting the file,
    // which Windows refuses while the live store holds it open.
    rusqlite::Connection::open(settings.data_dir.join("state.sqlite3"))
        .and_then(|connection| connection.execute_batch("DROP TABLE peer_tokens"))
        .expect("drop peer tokens");

    let response = send(
        &app,
        "GET",
        "/attractor/status",
        Some("127.0.0.1:40000"),
        &[("authorization", bearer.as_str())],
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

/// Pairs a token labelled `label` and returns its ID and secret.
async fn pair_token(app: &Router, label: &str) -> (String, String) {
    let code = send(
        app,
        "POST",
        "/workspace/api/environment/pairing-codes",
        None,
        &[],
        Some(&json!({"label": label}).to_string()),
    )
    .await;
    let code = json(code).await["code"].as_str().expect("code").to_string();
    // A peer address no other test pairs from, under the per-peer limit.
    let paired = send(
        app,
        "POST",
        "/environment/pair",
        Some("10.0.0.50:40000"),
        &[],
        Some(&json!({"code": code}).to_string()),
    )
    .await;
    assert_eq!(paired.status(), StatusCode::OK);
    let paired = json(paired).await;
    (
        paired["token_id"].as_str().expect("token id").to_string(),
        paired["token"].as_str().expect("token").to_string(),
    )
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
    let typed = |content_types: &[&'static [u8]], disposition: &'static [u8]| {
        let mut response = "typed".into_response();
        let headers = response.headers_mut();
        headers.remove(header::CONTENT_TYPE);
        for value in content_types {
            headers.append(
                header::CONTENT_TYPE,
                HeaderValue::from_bytes(value).expect("type"),
            );
        }
        if !disposition.is_empty() {
            headers.insert(
                header::CONTENT_DISPOSITION,
                HeaderValue::from_bytes(disposition).expect("disposition"),
            );
        }
        response
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
        "/workspace/api/latin1.html" => {
            let mut response = (page("text/html"), "<p>hi</p>").into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_bytes(b"text/plain, TEXT/HTML ; charset=\xe9").expect("obs-text"),
            );
            response
        }
        "/workspace/api/nbsp.html" => typed(&[b"text/html"], b"\xc2\xa0attachment"),
        "/workspace/api/named.html" => {
            typed(&[b"text/html"], br#"Attachment; filename="page.html""#)
        }
        "/workspace/api/spaced.html" => {
            typed(&[b"text/html"], br#"attachment ; filename="page.html""#)
        }
        "/workspace/api/frames" => typed(&[b"multipart/x-mixed-replace; boundary=frame"], b""),
        "/workspace/api/quoted.png" => typed(&[br#"image/png; note="a, text/html""#], b""),
        "/workspace/api/escaped.png" => {
            typed(&[br#"image/png; note="a\", text/html; b=\"c""#], b"")
        }
        "/workspace/api/listed.png" => typed(&[b"image/png, text/html"], b""),
        "/workspace/api/doubled.png" => typed(&[b"image/png", b"text/html"], b""),
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

/// The proxy's own headers, on everything it answers.
#[track_caller]
fn assert_sandboxed(response: &Response) {
    assert_eq!(
        (
            header(response, "content-security-policy"),
            header(response, "x-content-type-options")
        ),
        (Some("sandbox; default-src 'none'"), Some("nosniff"))
    );
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
        // An empty path never reaches the proxy route.
        if !bad.is_empty() {
            assert_sandboxed(&refused);
        }
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
    // The client would send a CONNECT to the peer's authority, not the path.
    let connect = proxy_with(
        &app,
        "CONNECT",
        "workspace/api/projects",
        Vec::new(),
        ProxyTimeouts::default(),
    )
    .await;
    assert_eq!(connect.status(), StatusCode::BAD_REQUEST);
    assert_sandboxed(&connect);
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

    // The HTTP parser drops `#…` from the request line before routing (see
    // `allowed_proxy_path`), so the proxy cannot see it to refuse it. The
    // fragment never reaches the peer; encoded `%23` and `%3F` pass unchanged.
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
            &b"text/html; charset=utf-8"[..],
            Some("attachment"),
            &b"<p>hi</p>"[..],
        ),
        (
            "workspace/api/latin1.html",
            b"text/plain, TEXT/HTML ; charset=\xe9",
            Some("attachment"),
            b"<p>hi</p>",
        ),
        (
            "workspace/api/badge.svg",
            b"image/svg+xml",
            Some("attachment"),
            b"<svg/>",
        ),
        // Only a plain ASCII `attachment` followed at once by `;` or the end
        // survives on markup; anything else is replaced by the proxy's own.
        (
            "workspace/api/nbsp.html",
            b"text/html",
            Some("attachment"),
            b"typed",
        ),
        (
            "workspace/api/spaced.html",
            b"text/html",
            Some("attachment"),
            b"typed",
        ),
        (
            "workspace/api/named.html",
            b"text/html",
            Some(r#"Attachment; filename="page.html""#),
            b"typed",
        ),
        // A browser renders each part of a multipart stream by its own type.
        (
            "workspace/api/frames",
            b"multipart/x-mixed-replace; boundary=frame",
            Some("attachment"),
            b"typed",
        ),
        // A comma in a quoted parameter does not start another type; a
        // listed or repeated markup type counts.
        (
            "workspace/api/quoted.png",
            br#"image/png; note="a, text/html""#,
            None,
            b"typed",
        ),
        (
            "workspace/api/escaped.png",
            br#"image/png; note="a\", text/html; b=\"c""#,
            None,
            b"typed",
        ),
        (
            "workspace/api/listed.png",
            b"image/png, text/html",
            Some("attachment"),
            b"typed",
        ),
        (
            "workspace/api/doubled.png",
            b"image/png",
            Some("attachment"),
            b"typed",
        ),
        (
            "workspace/api/pixel.png",
            b"image/png",
            None,
            b"\x89PNG\r\n\x1a\n",
        ),
        (
            "workspace/api/projects",
            b"application/json",
            None,
            br#"{"ok":true}"#,
        ),
    ] {
        let response = proxy(&app, target).await;
        assert_eq!(response.status(), StatusCode::OK, "{target}");
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .map(HeaderValue::as_bytes),
            Some(content_type),
            "{target}"
        );
        assert_eq!(
            header(&response, "content-disposition"),
            disposition,
            "{target}"
        );
        assert_sandboxed(&response);
        assert_eq!(header(&response, "set-cookie"), None);
        assert_eq!(bytes(response).await, body, "{target}");
    }
}

/// A forward proxy set in the environment never sees a linked peer's token:
/// a child test process runs the proxy with every proxy variable pointing at
/// a listener that must stay undialed.
#[tokio::test]
async fn linked_environment_proxy_ignores_ambient_forward_proxies() {
    const CHILD: &str = "SPARK_AMBIENT_PROXY_TEST";
    if std::env::var_os(CHILD).is_none() {
        let forward = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        forward.set_nonblocking(true).expect("nonblocking");
        let url = format!("http://{}", forward.local_addr().expect("address"));
        let names = [
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
        ];
        let output = std::process::Command::new(std::env::current_exe().expect("executable"))
            .args([
                "--exact",
                "environment_auth_route_contracts::linked_environment_proxy_ignores_ambient_forward_proxies",
                "--nocapture",
            ])
            .env_clear()
            .env(CHILD, "1")
            .envs(names.map(|name| (name, &url)))
            .output()
            .expect("child test");
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            forward.accept().err().map(|error| error.kind()),
            Some(std::io::ErrorKind::WouldBlock),
            "the forward proxy was dialed"
        );
        return;
    }
    let temp = tempfile::tempdir().expect("tempdir");
    let cfg = settings(temp.path());
    let app = crate::store_app_default(cfg.clone());
    let (peer, _) = fake_peer(&cfg).await;
    let quick = ProxyTimeouts {
        headers: Duration::from_secs(5),
        idle: Duration::from_secs(5),
    };
    let response = proxy_with(&app, "GET", "workspace/api/projects", Vec::new(), quick).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        peer.seen()
            .iter()
            .map(|seen| seen.authorization.as_deref())
            .collect::<Vec<_>>(),
        [Some("Bearer stored-peer-token")]
    );
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
    assert_sandboxed(&anonymous);
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
        assert_sandboxed(&swapped);
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
    assert_sandboxed(&silent);
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
    // UI API v2 is this wire contract: protected routes must answer with
    // `X-Spark-Environment-Id` (the proxied 200 above needs it), and the
    // authenticated `GET /workspace/api/environment` exposes `build_id`
    // (checked below). A v1 peer fails the version check rather than a
    // protected route.
    assert_eq!(spark_storage::environment::UI_API_VERSION, 2);
    assert_eq!(identity["ui_api_version"], 2);
    for field in ["display_name", "spark_version"] {
        assert!(
            !identity[field].as_str().expect(field).is_empty(),
            "{field}"
        );
    }
    // The build id is the version plus a hash of the running executable's
    // bytes, so two builds that differ in any byte announce different ids.
    let mut executable = std::hash::DefaultHasher::new();
    std::hash::Hasher::write(
        &mut executable,
        &std::fs::read(std::env::current_exe().expect("executable")).expect("read executable"),
    );
    assert_eq!(
        identity["build_id"],
        format!(
            "{}+{:016x}",
            env!("CARGO_PKG_VERSION"),
            std::hash::Hasher::finish(&executable)
        )
    );
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
            submitted_at: None,
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
    // Named by the file storage resolved, however the request spelled it.
    for spelled in ["result.json/", "./result.json/.", "result.json%20"] {
        let download = send(
            &app,
            "GET",
            &format!("{}?download=1", uri(spelled)),
            None,
            &[],
            None,
        )
        .await;
        assert_eq!(download.status(), StatusCode::OK, "{spelled}");
        assert_eq!(
            header(&download, "content-disposition"),
            Some("attachment; filename=\"result.json\""),
            "{spelled}"
        );
        assert_eq!(&bytes(download).await, &json_bytes, "{spelled}");
    }

    // Previews are unchanged: text as before, binary refused.
    let preview = send(&app, "GET", &uri("result.json"), None, &[], None).await;
    assert_eq!(preview.status(), StatusCode::OK);
    assert_eq!(header(&preview, "content-disposition"), None);
    assert_eq!(json(preview).await, json!("{\"ok\": true}\n"));
    let binary_preview = send(&app, "GET", &uri("capture.bin"), None, &[], None).await;
    assert_eq!(binary_preview.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn delegation_routes_refuse_dot_identifiers_before_contacting_the_peer() {
    use spark_storage::environment::{DelegationOut, EnvironmentLink, EnvironmentRepository};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::sync::{Arc, Mutex};

    let temp = tempfile::tempdir().expect("tempdir");
    let cfg = settings(temp.path());
    let app = crate::store_app_default(cfg.clone());
    // The linked environment: it records each request line and answers.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind upstream");
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let recorded = seen.clone();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut length = 0;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                if header.trim().is_empty() {
                    break;
                }
                if let Some((name, value)) = header.split_once(':') {
                    if name.eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse().unwrap();
                    }
                }
            }
            reader.read_exact(&mut vec![0; length]).unwrap();
            recorded.lock().unwrap().push(line.trim_end().to_string());
            let body = r#"{"ok":true}"#;
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    let environments = EnvironmentRepository::new(&cfg.data_dir);
    environments
        .save_link(
            &EnvironmentLink {
                environment_id: "env-b".into(),
                origin,
                label: "B".into(),
                created_at: spark_storage::environment::now(),
            },
            "stored-peer-token",
        )
        .unwrap();
    let id = "00000000-0000-4000-8000-000000000001";
    environments
        .create_out(&DelegationOut {
            delegation_id: id.into(),
            environment_id: "env-b".into(),
            project_path: "/b/project".into(),
            kind: "flow".into(),
            request: json!({}),
            request_sha256: "digest".into(),
            caller: "owner".into(),
            request_key: "ask".into(),
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
            submitted_at: None,
        })
        .unwrap();
    assert!(environments.accept_out(id, "run-1").unwrap());
    // Paused, so the server's follower leaves the peer alone.
    assert!(environments.pause_out(id, "Held by the test.").unwrap());

    let answer = Some(r#"{"selected_value":"yes"}"#);
    for (uri, body) in [
        (
            "/workspace/api/environments/links/env-b/flows/./files",
            None,
        ),
        (
            "/workspace/api/environments/links/env-b/flows/../files",
            None,
        ),
        (
            "/workspace/api/environments/links/env-b/flows/a/../b/files",
            None,
        ),
        (
            "/workspace/api/environments/links/env-b/flows/%2e%2e/files",
            None,
        ),
        (
            "/workspace/api/environments/links/env-b/flows/ops/%2E/run/files",
            None,
        ),
        (
            &*format!("/workspace/api/delegations/{id}/questions/./answer"),
            answer,
        ),
        (
            &*format!("/workspace/api/delegations/{id}/questions/../answer"),
            answer,
        ),
        (
            &*format!("/workspace/api/delegations/{id}/questions/%2e%2e/answer"),
            answer,
        ),
        (
            &*format!("/workspace/api/delegations/{id}/questions/a%2F..%2Fb/answer"),
            answer,
        ),
        (
            "/workspace/api/delegations/%2e%2e/questions/q-1/answer",
            answer,
        ),
    ] {
        let method = if body.is_some() { "POST" } else { "GET" };
        let refused = send(&app, method, uri, None, &[], body).await;
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST, "{uri}");
    }
    assert_eq!(*seen.lock().unwrap(), Vec::<String>::new());

    let files = send(
        &app,
        "GET",
        "/workspace/api/environments/links/env-b/flows/software-development/implement-change/files",
        None,
        &[],
        None,
    )
    .await;
    assert_eq!(files.status(), StatusCode::OK);
    let answered = send(
        &app,
        "POST",
        &format!("/workspace/api/delegations/{id}/questions/q-1.2/answer"),
        None,
        &[],
        answer,
    )
    .await;
    assert_eq!(answered.status(), StatusCode::OK);
    assert_eq!(
        *seen.lock().unwrap(),
        [
            "GET /workspace/api/flows/software-development%2Fimplement-change/files HTTP/1.1"
                .to_string(),
            format!(
                "POST /workspace/api/delegations/incoming/{id}/questions/q-1.2/answer HTTP/1.1"
            ),
        ]
    );

    // An incoming cancel names a delegation by UUID and leaves nothing for
    // another ID.
    let cancel = send(
        &app,
        "POST",
        "/workspace/api/delegations/incoming/not-a-uuid/cancel",
        None,
        &[],
        Some("{}"),
    )
    .await;
    assert_eq!(cancel.status(), StatusCode::BAD_REQUEST);
    assert!(environments.in_("not-a-uuid").unwrap().is_none());
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
