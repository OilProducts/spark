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
const LOCAL: &str = "127.0.0.1:40000";

/// Requests that are not trusted as this machine's owner without a token: a
/// remote peer, a loopback peer relaying for someone else, a request without
/// a peer, and a loopback peer whose Host, Origin or fetch metadata do not
/// name this listener from the same origin.
const UNTRUSTED: &[(Option<&str>, &[(&str, &str)])] = &[
    (Some(REMOTE), &[]),
    (Some("[2001:db8::1]:40000"), &[]),
    (Some(LOCAL), &[("x-forwarded-for", "203.0.113.9")]),
    (Some(LOCAL), &[("X-Forwarded-Proto", "https")]),
    (Some("[::1]:40000"), &[("forwarded", "for=203.0.113.9")]),
    (None, &[]),
    (Some(LOCAL), &[("host", "testserver")]),
    (Some(LOCAL), &[("host", "127.0.0.1:8001")]),
    (Some(LOCAL), &[("host", "127.0.0.1")]),
    (Some(LOCAL), &[("host", "127.0.0.2:8000")]),
    (Some(LOCAL), &[("host", "[::1]:8000")]),
    (Some(LOCAL), &[("host", "user@127.0.0.1:8000")]),
    (Some(LOCAL), &[("host", "localhost.:8000")]),
    (Some(LOCAL), &[("host", "127.0.0.1:8000, 127.0.0.1:8000")]),
    (
        Some(LOCAL),
        &[("host", "127.0.0.1:8000"), ("host", "127.0.0.1:8000")],
    ),
    (Some(LOCAL), &[("origin", "http://evil.example")]),
    (Some(LOCAL), &[("origin", "null")]),
    (Some(LOCAL), &[("origin", "https://127.0.0.1:8000")]),
    (Some(LOCAL), &[("origin", "http://127.0.0.1:8001")]),
    (Some(LOCAL), &[("origin", "http://127.0.0.1:8000/")]),
    (Some(LOCAL), &[("origin", "http://localhost:8000")]),
    (
        Some(LOCAL),
        &[
            ("origin", "http://127.0.0.1:8000"),
            ("origin", "http://127.0.0.1:8000"),
        ],
    ),
    (Some(LOCAL), &[("sec-fetch-site", "cross-site")]),
    (Some(LOCAL), &[("sec-fetch-site", "same-origin, none")]),
    (
        Some(LOCAL),
        &[("sec-fetch-site", "none"), ("sec-fetch-site", "none")],
    ),
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
            let response = send(&app, method, uri, peer, headers, body).await;
            let status = response.status();
            assert_eq!(
                (status, json(response).await),
                (
                    StatusCode::UNAUTHORIZED,
                    json!({"detail": "A peer token is required."})
                ),
                "{method} {uri} from {peer:?} {headers:?}"
            );
        }
    }
    // Without the listener it arrived on, a request cannot name it.
    let mut unlisted = request("GET", "/workspace/api/projects", Some(LOCAL), &[], None);
    unlisted
        .extensions_mut()
        .remove::<spark_http::ListenerAddr>();
    let unlisted = app.clone().oneshot(unlisted).await.expect("response");
    assert_eq!(unlisted.status(), StatusCode::UNAUTHORIZED);
    // Nor can a request whose target names another authority or scheme.
    for uri in [
        "http://127.0.0.1:9000/workspace/api/projects",
        "https://127.0.0.1:8000/workspace/api/projects",
    ] {
        let status = send(&app, "GET", uri, Some(LOCAL), &[], None)
            .await
            .status();
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
    }
}

#[tokio::test]
async fn direct_loopback_and_same_origin_requests_are_trusted() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = crate::store_app_default(settings(temp.path()));

    let same_origin: &[&[(&str, &str)]] = &[
        &[],
        &[
            ("origin", "http://127.0.0.1:8000"),
            ("sec-fetch-site", "same-origin"),
        ],
        &[("sec-fetch-site", "none")],
        &[("sec-fetch-site", "same-site")],
        &[
            ("host", "LocalHost:8000"),
            ("origin", "http://localhost:8000"),
        ],
        // An invalid token still leaves the owner's own access.
        &[("authorization", "Bearer not-a-token")],
    ];
    for peer in [
        "127.0.0.1:40000",
        "127.8.9.10:40000",
        "[::1]:40000",
        "[::ffff:127.0.0.1]:40000",
    ] {
        for headers in same_origin {
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
                let status = send(&app, method, uri, Some(peer), headers, None)
                    .await
                    .status();
                assert_eq!(status, expected, "{method} {uri} from {peer} {headers:?}");
            }
        }
        // A request target naming the listener agrees with its Host.
        let absolute = send(
            &app,
            "GET",
            "http://127.0.0.1:8000/attractor/status",
            Some(peer),
            &[],
            None,
        )
        .await;
        assert_eq!(absolute.status(), StatusCode::OK);
        let launch = send(
            &app,
            "POST",
            "/workspace/api/runs/launch",
            Some(peer),
            &[],
            Some("{"),
        )
        .await;
        assert_eq!(launch.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let upload = send(
            &app,
            "POST",
            "/workspace/api/conversations/conversation-1/attachments?project_path=/missing",
            Some(peer),
            &[("content-type", "application/octet-stream")],
            Some("bytes"),
        )
        .await;
        assert_ne!(upload.status(), StatusCode::UNAUTHORIZED);
        assert_ne!(upload.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }
}

/// Host names the listener by its canonical address: IPv6 literals in any
/// spelling, mapped IPv4, and on a wildcard bind any loopback literal or
/// `localhost`. Origin keeps names and literal families apart.
#[tokio::test]
async fn host_and_origin_name_the_listener_by_its_canonical_address() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = crate::store_app_default(settings(temp.path()));
    let cases: &[(&str, &str, &[(&str, &str)], StatusCode)] = &[
        (
            "[::1]:8000",
            "[::1]:40000",
            &[
                ("host", "[0:0:0:0:0:0:0:1]:8000"),
                ("origin", "http://[::1]:8000"),
            ],
            StatusCode::OK,
        ),
        (
            "[::1]:8000",
            "[::1]:40000",
            &[("host", "localhost:8000")],
            StatusCode::OK,
        ),
        (
            "[::1]:8000",
            "[::1]:40000",
            &[("host", "127.0.0.1:8000")],
            StatusCode::UNAUTHORIZED,
        ),
        (
            "[::ffff:127.0.0.1]:8000",
            "[::ffff:127.0.0.1]:40000",
            &[("host", "[::ffff:127.0.0.1]:8000")],
            StatusCode::OK,
        ),
        (
            "[::ffff:127.0.0.1]:8000",
            "[::ffff:127.0.0.1]:40000",
            &[
                ("host", "[::ffff:127.0.0.1]:8000"),
                ("origin", "http://127.0.0.1:8000"),
            ],
            StatusCode::UNAUTHORIZED,
        ),
        (
            "127.0.0.1:8000",
            LOCAL,
            &[("host", "127.0.0.2:8000")],
            StatusCode::UNAUTHORIZED,
        ),
        (
            "127.0.0.2:8000",
            LOCAL,
            &[("host", "127.0.0.2:8000")],
            StatusCode::OK,
        ),
        (
            "127.0.0.2:8000",
            LOCAL,
            &[("host", "localhost:8000")],
            StatusCode::UNAUTHORIZED,
        ),
        (
            "0.0.0.0:8000",
            LOCAL,
            &[("host", "127.0.0.2:8000")],
            StatusCode::OK,
        ),
        (
            "0.0.0.0:8000",
            LOCAL,
            &[("host", "localhost:8000")],
            StatusCode::OK,
        ),
        (
            "0.0.0.0:8000",
            LOCAL,
            &[("host", "10.1.2.3:8000")],
            StatusCode::UNAUTHORIZED,
        ),
        (
            "0.0.0.0:8000",
            REMOTE,
            &[("host", "127.0.0.1:8000")],
            StatusCode::UNAUTHORIZED,
        ),
        (
            "127.0.0.1:80",
            LOCAL,
            &[("host", "127.0.0.1"), ("origin", "http://127.0.0.1")],
            StatusCode::OK,
        ),
        (
            "127.0.0.1:80",
            LOCAL,
            &[("host", "127.0.0.1:80"), ("origin", "http://127.0.0.1")],
            StatusCode::OK,
        ),
    ];
    for &(listener, peer, headers, expected) in cases {
        let response = send_to(
            &app,
            Some(listener),
            "GET",
            "/workspace/api/projects",
            Some(peer),
            headers,
            None,
        )
        .await;
        assert_eq!(response.status(), expected, "{listener} {peer} {headers:?}");
    }
}

/// A peer at the IP the server bound, such as a local agent calling the
/// advertised non-loopback URL, needs a token like any other remote peer.
#[tokio::test]
async fn a_non_loopback_listener_requires_a_bearer_even_from_its_own_ip() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let app = crate::store_app_default(settings.clone());
    let listener = Some("10.1.2.3:8000");
    let host = [("host", "10.1.2.3:8000")];

    for peer in [REMOTE, LOCAL] {
        let refused = send_to(
            &app,
            listener,
            "GET",
            "/workspace/api/projects",
            Some(peer),
            &host,
            None,
        )
        .await;
        assert_eq!(refused.status(), StatusCode::UNAUTHORIZED, "{peer}");
    }
    let shell = send_to(&app, listener, "GET", "/", Some(REMOTE), &host, None).await;
    assert_eq!(shell.status(), StatusCode::OK);

    // The reserved agent token works as a bearer, but is not a peer token
    // the owner lists or revokes.
    let secret = spark_storage::environment::EnvironmentRepository::new(&settings.data_dir)
        .provision_agent_token()
        .expect("agent token");
    let bearer = format!("Bearer {secret}");
    let authorized = [host[0], ("authorization", bearer.as_str())];
    let status = send_to(
        &app,
        listener,
        "GET",
        "/workspace/api/projects",
        Some(REMOTE),
        &authorized,
        None,
    )
    .await
    .status();
    assert_eq!(status, StatusCode::OK);
    let tokens = send(
        &app,
        "GET",
        "/workspace/api/environment/tokens",
        Some(LOCAL),
        &[],
        None,
    )
    .await;
    assert_eq!(json(tokens).await, json!({"tokens": []}));
    let tokens = text(
        send_to(
            &app,
            listener,
            "GET",
            "/workspace/api/environment/tokens",
            Some(REMOTE),
            &authorized,
            None,
        )
        .await,
    )
    .await;
    assert!(!tokens.contains(&secret));
    let revoke = send(
        &app,
        "POST",
        "/workspace/api/environment/tokens/spark-agent/revoke",
        Some(LOCAL),
        &[],
        None,
    )
    .await;
    assert_eq!(revoke.status(), StatusCode::NOT_FOUND);
    let still = send_to(
        &app,
        listener,
        "GET",
        "/attractor/status",
        Some(REMOTE),
        &authorized,
        None,
    )
    .await;
    assert_eq!(still.status(), StatusCode::OK);
}

/// A served listener attaches itself and the peer to each request.
#[tokio::test]
async fn a_served_listener_trusts_only_requests_naming_it() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let temp = tempfile::tempdir().expect("tempdir");
    let app = crate::store_app_default(settings(temp.path()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("address");
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(spark_http::serve(listener, app, async {
        let _ = stopped.await;
    }));

    let own = format!("Host: {addr}\r\n");
    let other_port = format!("Host: 127.0.0.1:{}\r\n", addr.port() + 1);
    for (headers, expected) in [
        (own.clone(), "200"),
        ("Host: testserver\r\n".to_string(), "401"),
        (other_port, "401"),
        (format!("{own}Origin: http://evil.example\r\n"), "401"),
        (format!("{own}X-Forwarded-For: 203.0.113.9\r\n"), "401"),
    ] {
        let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        let head =
            format!("GET /workspace/api/projects HTTP/1.1\r\n{headers}Connection: close\r\n\r\n");
        stream.write_all(head.as_bytes()).await.expect("write");
        let mut response = String::new();
        stream.read_to_string(&mut response).await.expect("read");
        assert!(
            response.starts_with(&format!("HTTP/1.1 {expected} ")),
            "{headers:?}: {response}"
        );
    }
    stop.send(()).expect("server running");
    server.await.expect("server task").expect("served");
}

/// A JSON body needs one JSON media type, checked before the body is read.
#[tokio::test]
async fn json_bodies_need_a_json_media_type() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = crate::store_app_default(settings(temp.path()));
    let codes = "/workspace/api/environment/pairing-codes";
    let body = Some(r#"{"label": "laptop"}"#);

    for content_type in [
        "application/json",
        "Application/JSON",
        "application/json; charset=utf-8",
        "application/json ;\tcharset=utf-8",
        r#"application/json;charset=utf-8; profile="a, b; c""#,
        "application/vnd.spark+json",
    ] {
        let status = send(
            &app,
            "POST",
            codes,
            Some(LOCAL),
            &[("content-type", content_type)],
            body,
        )
        .await
        .status();
        assert_eq!(status, StatusCode::OK, "{content_type}");
    }
    let refused: &[&[(&str, &str)]] = &[
        &[("content-type", "text/plain")],
        &[("content-type", "application/x-www-form-urlencoded")],
        &[("content-type", "text/vnd.spark+json")],
        &[("content-type", "application/json+xml")],
        &[("content-type", "application/+json")],
        &[("content-type", "application/json; charset=")],
        &[("content-type", r#"application/json; profile="a\""#)],
        &[("content-type", "application/json, text/plain")],
        &[
            ("content-type", "application/json"),
            ("content-type", "application/json"),
        ],
    ];
    for headers in refused {
        let response = send(&app, "POST", codes, Some(LOCAL), headers, body).await;
        let status = response.status();
        assert_eq!(
            (status, json(response).await),
            (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                json!({"detail": "Expected Content-Type: application/json."})
            ),
            "{headers:?}"
        );
    }
    let mut untyped = request("POST", codes, Some(LOCAL), &[], body);
    untyped.headers_mut().remove("content-type");
    let untyped = app.clone().oneshot(untyped).await.expect("response");
    assert_eq!(untyped.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    // A JSON media type still needs a valid JSON body.
    let malformed = send(&app, "POST", codes, Some(LOCAL), &[], Some("{")).await;
    assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
    // An empty body still asks for a code with the defaults.
    let empty = send(&app, "POST", codes, Some(LOCAL), &[], None).await;
    assert_eq!(empty.status(), StatusCode::OK);

    let pair = send(
        &app,
        "POST",
        "/environment/pair",
        Some(REMOTE),
        &[("content-type", "text/plain")],
        Some(r#"{"code": "x"}"#),
    )
    .await;
    assert_eq!(pair.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
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
        let index = send(&app, "GET", "/", peer, headers, None).await;
        assert_eq!(index.status(), StatusCode::OK);
        assert!(text(index).await.contains("environment ui"));
        let asset = send(&app, "GET", "/assets/app.js", peer, headers, None).await;
        assert_eq!(asset.status(), StatusCode::OK);
        let favicon = send(&app, "GET", "/favicon.ico", peer, headers, None).await;
        assert_eq!(favicon.status(), StatusCode::OK);
        let missing = send(&app, "GET", "/assets/missing.js", peer, headers, None).await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(json(missing).await, json!({"detail": "Asset not found"}));

        let descriptor = send(&app, "GET", "/environment", peer, headers, None).await;
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
            peer,
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
            peer,
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
        Some(LOCAL),
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
            let status = send(&app, "GET", uri, peer, &headers, None).await.status();
            assert_eq!(status, StatusCode::OK, "{uri} from {peer:?} {headers:?}");
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
        Some(LOCAL),
        &[],
        None,
    )
    .await;
    let listed = text(listed).await;
    assert!(listed.contains(&token_id));
    assert!(!listed.contains(&secret));

    let revoke_uri = format!("/workspace/api/environment/tokens/{token_id}/revoke");
    let revoked = send(&app, "POST", &revoke_uri, Some(LOCAL), &[], None).await;
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
        Some(LOCAL),
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
        Some(LOCAL),
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
        Some(LOCAL),
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
async fn delegation_list_and_discovery_routes_are_mounted_and_protected() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = crate::store_app_default(settings(temp.path()));

    let local_list = send(
        &app,
        "GET",
        "/workspace/api/delegations?open=true",
        Some(LOCAL),
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
        Some(LOCAL),
        &[],
        None,
    )
    .await;
    assert_eq!(missing_link.status(), StatusCode::NOT_FOUND);

    let bad_flow_suffix = send(
        &app,
        "GET",
        "/workspace/api/environments/links/env-missing/flows/ops/run",
        Some(LOCAL),
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

#[tokio::test]
async fn linked_environment_proxy_injects_bearer_streams_sse_and_blocks_disallowed_paths() {
    use futures_util::StreamExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let temp = tempfile::tempdir().expect("tempdir");
    let cfg = settings(temp.path());
    let app = crate::store_app_default(cfg.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind upstream");
    let port = listener.local_addr().unwrap().port();
    let (release_sse_tx, mut release_sse_rx) = tokio::sync::oneshot::channel::<()>();

    let upstream = tokio::spawn(async move {
        // Request 1: POST JSON to /workspace/api/projects/register?foo=1
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = sock.read(&mut buf).await.unwrap();
        let first_req = String::from_utf8_lossy(&buf[..n]).to_string();
        let body = r#"{"ok":true}"#;
        let resp = format!(
            "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        sock.write_all(resp.as_bytes()).await.unwrap();
        drop(sock);

        // Request 2: Streaming SSE on /workspace/api/live/events
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = sock.read(&mut buf).await.unwrap();
        let second_req = String::from_utf8_lossy(&buf[..n]).to_string();
        sock.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\n\r\ndata: first\n\n",
        )
        .await
        .unwrap();
        sock.flush().await.unwrap();
        let _ = (&mut release_sse_rx).await;
        sock.write_all(b"data: second\n\n").await.unwrap();
        sock.flush().await.unwrap();
        drop(sock);
        (first_req, second_req)
    });

    spark_storage::environment::EnvironmentRepository::new(&cfg.data_dir)
        .save_link(
            &spark_storage::environment::EnvironmentLink {
                environment_id: "env-b".into(),
                origin: format!("http://127.0.0.1:{port}"),
                label: "B".into(),
                created_at: spark_storage::environment::now(),
            },
            "stored-peer-token",
        )
        .unwrap();

    // Disallowed paths and traversal are refused before contacting upstream.
    for bad in [
        "/workspace/api/environments/links/env-b/proxy/assets/app.js",
        "/workspace/api/environments/links/env-b/proxy/workspace/api/../environment",
        "/workspace/api/environments/links/env-b/proxy/etc/passwd",
        "/workspace/api/environments/links/env-missing/proxy/environment",
    ] {
        let refused = send(&app, "GET", bad, Some(LOCAL), &[], None).await;
        assert_eq!(refused.status(), StatusCode::NOT_FOUND, "{bad}");
    }

    // Request 1: forwards method, query, JSON body, and replaces caller Authorization with link bearer.
    let posted = send(
        &app,
        "POST",
        "/workspace/api/environments/links/env-b/proxy/workspace/api/projects/register?foo=1",
        Some(LOCAL),
        &[("authorization", "Bearer caller-should-not-leak")],
        Some(r#"{"project_path":"/srv/repo"}"#),
    )
    .await;
    assert_eq!(posted.status(), StatusCode::CREATED);
    assert_eq!(json(posted).await, json!({"ok": true}));

    // Request 2: SSE chunk 1 arrives while upstream is still open!
    let sse = send(
        &app,
        "GET",
        "/workspace/api/environments/links/env-b/proxy/workspace/api/live/events",
        Some(LOCAL),
        &[("accept", "text/event-stream")],
        None,
    )
    .await;
    assert_eq!(sse.status(), StatusCode::OK);
    assert_eq!(
        sse.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream")
    );
    let mut stream = sse.into_body().into_data_stream();
    let first_chunk = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
        .await
        .expect("first SSE chunk arrived before upstream closed")
        .expect("some chunk")
        .expect("ok bytes");
    assert_eq!(String::from_utf8_lossy(&first_chunk), "data: first\n\n");
    let _ = release_sse_tx.send(());
    let second_chunk = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
        .await
        .expect("second SSE chunk")
        .expect("some chunk")
        .expect("ok bytes");
    assert_eq!(String::from_utf8_lossy(&second_chunk), "data: second\n\n");

    let (first_req, second_req) = upstream.await.unwrap();
    assert!(first_req.starts_with("POST /workspace/api/projects/register?foo=1 HTTP/"));
    assert!(first_req
        .to_ascii_lowercase()
        .contains("authorization: bearer stored-peer-token"));
    assert!(!first_req.contains("caller-should-not-leak"));
    assert!(first_req.contains(r#"{"project_path":"/srv/repo"}"#));
    assert!(second_req.starts_with("GET /workspace/api/live/events HTTP/"));
    assert!(second_req
        .to_ascii_lowercase()
        .contains("authorization: bearer stored-peer-token"));
}

fn request(
    method: &str,
    uri: &str,
    peer: Option<&str>,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> Request<Body> {
    request_to(Some(crate::LISTENER), method, uri, peer, headers, body)
}

/// A request arriving on `listener` from `peer`, as a served app sees it.
/// It names the listener in `Host` unless `headers` give one.
fn request_to(
    listener: Option<&str>,
    method: &str,
    uri: &str,
    peer: Option<&str>,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(listener) = listener {
        builder = builder.extension(spark_http::ListenerAddr(
            listener.parse().expect("listener"),
        ));
        if !headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("host"))
        {
            builder = builder.header("host", listener);
        }
    }
    if let Some(peer) = peer {
        builder = builder.extension(ConnectInfo(peer.parse::<SocketAddr>().expect("peer")));
    }
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    if body.is_some()
        && !headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
    {
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
    send_to(app, Some(crate::LISTENER), method, uri, peer, headers, body).await
}

async fn send_to(
    app: &Router,
    listener: Option<&str>,
    method: &str,
    uri: &str,
    peer: Option<&str>,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> Response {
    app.clone()
        .oneshot(request_to(listener, method, uri, peer, headers, body))
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
