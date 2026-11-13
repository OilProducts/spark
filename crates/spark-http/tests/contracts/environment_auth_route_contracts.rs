use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use axum::Router;
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
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
    let code = json(code).await["code"].as_str().expect("code").to_string();
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
