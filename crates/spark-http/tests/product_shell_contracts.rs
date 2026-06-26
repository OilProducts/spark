use std::fs;
use std::path::{Path, PathBuf};

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use spark_common::settings::SparkSettings;
use spark_http::build_app;
use tower::ServiceExt;

#[tokio::test]
async fn source_frontend_dist_serves_index_icon_favicon_and_json_asset_errors() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = build_app(settings_with_project_root(temp.path(), repo_root()));

    let index = request_bytes(app.clone(), "/").await;
    assert_eq!(index.0, StatusCode::OK);
    assert_eq!(index.1, "text/html; charset=utf-8");
    let index_text = String::from_utf8(index.2).expect("index utf-8");
    assert!(index_text.contains("<div id=\"root\"></div>"));

    let icon = request_bytes(app.clone(), "/assets/spark-app-icon.png").await;
    assert_eq!(icon.0, StatusCode::OK);
    assert_eq!(icon.1, "image/png");
    assert_eq!(icon.2.len(), 969_369);

    let favicon = request_bytes(app.clone(), "/favicon.ico").await;
    assert_eq!(favicon.0, StatusCode::OK);
    assert_eq!(favicon.1, "image/png");
    assert_eq!(favicon.2, icon.2);

    let missing = request_json(app.clone(), "/assets/missing.png").await;
    assert_eq!(missing.0, StatusCode::NOT_FOUND);
    assert_eq!(missing.1, "application/json");
    assert_eq!(missing.2, json!({"detail": "Asset not found"}));

    let traversal = request_json(app.clone(), "/assets/../Cargo.toml").await;
    assert_eq!(traversal.0, StatusCode::NOT_FOUND);
    assert_eq!(traversal.1, "application/json");
    assert_eq!(traversal.2, json!({"detail": "Asset not found"}));

    let encoded_traversal = request_json(app.clone(), "/assets/%2e%2e/Cargo.toml").await;
    assert_eq!(encoded_traversal.0, StatusCode::NOT_FOUND);
    assert_eq!(encoded_traversal.1, "application/json");
    assert_eq!(encoded_traversal.2, json!({"detail": "Asset not found"}));

    let absolute = request_json(app, "/assets//tmp/spark-secret.txt").await;
    assert_eq!(absolute.0, StatusCode::NOT_FOUND);
    assert_eq!(absolute.1, "application/json");
}

#[tokio::test]
async fn explicit_ui_dir_serves_development_assets_and_blocks_symlink_escape() {
    let temp = tempfile::tempdir().expect("tempdir");
    let ui_root = temp.path().join("ui");
    let assets_dir = ui_root.join("assets");
    fs::create_dir_all(&assets_dir).expect("assets");
    fs::write(
        ui_root.join("index.html"),
        "<!doctype html><main id=\"root\">Explicit UI</main>\n",
    )
    .expect("index");
    fs::write(
        assets_dir.join("compat-app.js"),
        "window.__sparkCompatAsset = { status: 'ok' };\n",
    )
    .expect("js");
    fs::write(
        assets_dir.join("spark-app-icon.png"),
        b"spark-compat-favicon\n",
    )
    .expect("icon");
    fs::write(temp.path().join("secret.txt"), "outside").expect("secret");
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        temp.path().join("secret.txt"),
        assets_dir.join("escape.txt"),
    )
    .expect("symlink");

    let mut settings = settings(temp.path());
    settings.ui_dir = Some(ui_root);
    let app = build_app(settings);

    let index = request_bytes(app.clone(), "/").await;
    assert_eq!(index.0, StatusCode::OK);
    assert_eq!(index.1, "text/html; charset=utf-8");
    assert_eq!(
        String::from_utf8(index.2).expect("index utf-8"),
        "<!doctype html><main id=\"root\">Explicit UI</main>\n"
    );

    let script = request_bytes(app.clone(), "/assets/compat-app.js").await;
    assert_eq!(script.0, StatusCode::OK);
    assert_eq!(script.1, "text/javascript; charset=utf-8");
    assert_eq!(
        String::from_utf8(script.2).expect("script utf-8"),
        "window.__sparkCompatAsset = { status: 'ok' };\n"
    );

    let favicon = request_bytes(app.clone(), "/favicon.ico").await;
    assert_eq!(favicon.0, StatusCode::OK);
    assert_eq!(favicon.1, "image/png");
    assert_eq!(favicon.2, b"spark-compat-favicon\n");

    #[cfg(unix)]
    {
        let escaped = request_json(app.clone(), "/assets/escape.txt").await;
        assert_eq!(escaped.0, StatusCode::NOT_FOUND);
        assert_eq!(escaped.1, "application/json");
        assert_eq!(escaped.2, json!({"detail": "Asset not found"}));
    }
}

#[tokio::test]
async fn packaged_frontend_dist_serves_when_source_tree_ui_is_unavailable() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = build_app(settings(temp.path()));

    let index = request_bytes(app.clone(), "/").await;
    assert_eq!(index.0, StatusCode::OK);
    assert_eq!(index.1, "text/html; charset=utf-8");
    let index_text = String::from_utf8(index.2).expect("index utf-8");
    assert!(index_text.contains("<div id=\"root\"></div>"));

    let icon = request_bytes(app.clone(), "/assets/spark-app-icon.png").await;
    assert_eq!(icon.0, StatusCode::OK);
    assert_eq!(icon.1, "image/png");
    assert!(icon.2.len() > 1000);

    let favicon = request_bytes(app.clone(), "/favicon.ico").await;
    assert_eq!(favicon.0, StatusCode::OK);
    assert_eq!(favicon.1, "image/png");
    assert_eq!(favicon.2, icon.2);

    let missing = request_json(app, "/assets/missing.png").await;
    assert_eq!(missing.0, StatusCode::NOT_FOUND);
    assert_eq!(missing.1, "application/json");
    assert_eq!(missing.2, json!({"detail": "Asset not found"}));
}

#[tokio::test]
async fn api_and_legacy_root_paths_do_not_fall_back_to_html() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = build_app(settings_with_project_root(temp.path(), repo_root()));

    for path in ["/workspace/api/missing", "/status", "/docs"] {
        let response = request_json(app.clone(), path).await;
        assert_eq!(response.0, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(response.1, "application/json", "{path}");
        assert_eq!(response.2, json!({"detail": "Not Found"}), "{path}");
    }

    let attractor = request_json(app, "/attractor/status").await;
    assert_eq!(attractor.0, StatusCode::OK);
    assert_eq!(attractor.1, "application/json");
    assert_eq!(attractor.2["status"], "idle");
}

#[tokio::test]
async fn product_mount_base_paths_match_mounted_app_boundaries() {
    let temp = tempfile::tempdir().expect("tempdir");
    let app = build_app(settings_with_project_root(temp.path(), repo_root()));

    let workspace_base = request_bytes_with_host(app.clone(), "/workspace", "testserver").await;
    assert_eq!(workspace_base.0, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        workspace_base.2.as_deref(),
        Some("http://testserver/workspace/")
    );
    assert_eq!(workspace_base.1, "");
    assert!(workspace_base.3.is_empty());

    let workspace_slash = request_json(app.clone(), "/workspace/").await;
    assert_eq!(workspace_slash.0, StatusCode::NOT_FOUND);
    assert_eq!(workspace_slash.1, "application/json");
    assert_eq!(workspace_slash.2, json!({"detail": "Not Found"}));

    let attractor_base = request_bytes_with_host(app.clone(), "/attractor", "testserver").await;
    assert_eq!(attractor_base.0, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        attractor_base.2.as_deref(),
        Some("http://testserver/attractor/")
    );
    assert_eq!(attractor_base.1, "");
    assert!(attractor_base.3.is_empty());

    let attractor_slash = request_json(app, "/attractor/").await;
    assert_eq!(attractor_slash.0, StatusCode::NOT_FOUND);
    assert_eq!(attractor_slash.1, "application/json");
    assert_eq!(attractor_slash.2, json!({"detail": "Not Found"}));
}

async fn request_bytes(app: axum::Router, uri: &str) -> (StatusCode, String, Vec<u8>) {
    let response = request_bytes_with_optional_host(app, uri, None).await;
    (response.0, response.1, response.3)
}

async fn request_bytes_with_host(
    app: axum::Router,
    uri: &str,
    host: &str,
) -> (StatusCode, String, Option<String>, Vec<u8>) {
    request_bytes_with_optional_host(app, uri, Some(host)).await
}

async fn request_bytes_with_optional_host(
    app: axum::Router,
    uri: &str,
    host: Option<&str>,
) -> (StatusCode, String, Option<String>, Vec<u8>) {
    let mut request = Request::builder().uri(uri);
    if let Some(host) = host {
        request = request.header("host", host);
    }
    let response = app
        .oneshot(request.body(Body::empty()).expect("request"))
        .await
        .expect("response");
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let location = response
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, content_type, location, bytes.to_vec())
}

async fn request_json(app: axum::Router, uri: &str) -> (StatusCode, String, Value) {
    let response = request_bytes(app, uri).await;
    (
        response.0,
        response.1,
        serde_json::from_slice(&response.2).expect("json response"),
    )
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("repo root")
        .to_path_buf()
}

fn settings_with_project_root(root: &Path, project_root: PathBuf) -> SparkSettings {
    SparkSettings {
        project_root,
        ..settings(root)
    }
}

fn settings(root: &Path) -> SparkSettings {
    SparkSettings {
        project_root: root.join("source"),
        data_dir: root.join("spark-home"),
        config_dir: root.join("spark-home/config"),
        runtime_dir: root.join("spark-home/runtime"),
        logs_dir: root.join("spark-home/logs"),
        workspace_dir: root.join("spark-home/workspace"),
        projects_dir: root.join("spark-home/workspace/projects"),
        attractor_dir: root.join("spark-home/attractor"),
        runs_dir: root.join("spark-home/attractor/runs"),
        flows_dir: root.join("flows"),
        ui_dir: None,
        project_roots: Vec::new(),
    }
}
