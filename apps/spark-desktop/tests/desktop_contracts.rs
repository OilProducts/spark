use std::fs;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use spark_desktop::desktop_core::{
    bootstrap_desktop_runtime, default_spark_data_dir, desktop_config_file, frontend_url_for_addr,
    is_app_owned_data_dir, load_desktop_settings, server_host_for_settings,
    set_remote_access_enabled, settings_view, start_desktop_server, DesktopPaths,
    DesktopServerSettings, LOCAL_BIND_HOST, REMOTE_BIND_HOST,
};
use spark_storage::ConversationRepository;

#[test]
fn bootstrap_uses_app_owned_spark_data_directory_by_default() {
    let temp = tempfile::tempdir().expect("tempdir");
    // Canonicalize: TempDir may be symlinked (macOS /var -> /private/var) while
    // resolved settings paths are physical.
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let paths = DesktopPaths::new(root.join("data"), root.join("config"));

    let bootstrap =
        bootstrap_desktop_runtime(&paths, &DesktopServerSettings::default()).expect("bootstrap");

    assert_eq!(bootstrap.settings.data_dir, default_spark_data_dir(&paths));
    assert!(is_app_owned_data_dir(&paths, &bootstrap.settings.data_dir));
    assert_eq!(
        bootstrap.settings.flows_dir,
        bootstrap.settings.data_dir.join("flows")
    );
    assert_eq!(bootstrap.bind_host, LOCAL_BIND_HOST);
}

#[test]
fn first_launch_seeds_packaged_flows_into_desktop_runtime() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = DesktopPaths::new(temp.path().join("data"), temp.path().join("config"));

    let bootstrap =
        bootstrap_desktop_runtime(&paths, &DesktopServerSettings::default()).expect("bootstrap");

    assert_eq!(bootstrap.seeded_flows.created.len(), 28);
    assert!(bootstrap
        .settings
        .flows_dir
        .join("software-development/implement-change-request.yaml")
        .is_file());
    assert!(bootstrap
        .settings
        .config_dir
        .join("flow-catalog.toml")
        .is_file());
}

#[test]
fn remote_toggle_maps_to_local_or_remote_bind_host_and_requires_confirmation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = DesktopPaths::new(temp.path().join("data"), temp.path().join("config"));

    let default_settings = load_desktop_settings(&paths).expect("default settings");
    assert_eq!(server_host_for_settings(&default_settings), LOCAL_BIND_HOST);
    assert!(set_remote_access_enabled(&paths, true, false).is_err());

    let enabled = set_remote_access_enabled(&paths, true, true).expect("enable remote");
    assert_eq!(server_host_for_settings(&enabled), REMOTE_BIND_HOST);
    assert_eq!(
        fs::read_to_string(desktop_config_file(&paths)).expect("desktop config"),
        "{\n  \"remote_access_enabled\": true\n}\n"
    );

    let disabled = set_remote_access_enabled(&paths, false, false).expect("disable remote");
    assert_eq!(server_host_for_settings(&disabled), LOCAL_BIND_HOST);
}

#[test]
fn settings_view_marks_remote_change_as_restart_required_for_running_server() {
    let view = settings_view(
        &DesktopServerSettings {
            remote_access_enabled: true,
        },
        LOCAL_BIND_HOST,
        "http://127.0.0.1:42001/",
    );

    assert_eq!(view.bind_host, REMOTE_BIND_HOST);
    assert!(view.requires_restart);
}

#[test]
fn frontend_url_uses_in_process_server_port_and_loopback_for_unspecified_bind() {
    let local = frontend_url_for_addr(SocketAddr::from(([127, 0, 0, 1], 49152)));
    let remote_bound = frontend_url_for_addr(SocketAddr::from(([0, 0, 0, 0], 49153)));

    assert_eq!(local, "http://127.0.0.1:49152/");
    assert_eq!(remote_bound, "http://127.0.0.1:49153/");
}

#[test]
fn desktop_server_serves_web_ui_from_in_process_loopback_url() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = DesktopPaths::new(temp.path().join("data"), temp.path().join("config"));
    let bootstrap =
        bootstrap_desktop_runtime(&paths, &DesktopServerSettings::default()).expect("bootstrap");

    let mut server =
        start_desktop_server(bootstrap.settings, &bootstrap.bind_host).expect("desktop server");
    let server_url = server.url().to_string();

    assert!(server_url.starts_with("http://127.0.0.1:"), "{server_url}");
    let response = http_client()
        .get(&server_url)
        .send()
        .expect("desktop index response");

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/html; charset=utf-8")
    );
    assert!(response
        .text()
        .expect("index html")
        .contains("<div id=\"root\"></div>"));

    server.shutdown();
}

#[test]
fn desktop_server_shutdown_request_does_not_wait_for_live_event_stream_to_drain() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = DesktopPaths::new(temp.path().join("data"), temp.path().join("config"));
    let bootstrap =
        bootstrap_desktop_runtime(&paths, &DesktopServerSettings::default()).expect("bootstrap");
    let mut server =
        start_desktop_server(bootstrap.settings, &bootstrap.bind_host).expect("desktop server");

    let live_events_response = http_client()
        .get(format!("{}workspace/api/live/events", server.url()))
        .send()
        .expect("live events response");
    assert_eq!(live_events_response.status(), reqwest::StatusCode::OK);

    let started_at = Instant::now();
    server.request_shutdown();

    assert!(
        started_at.elapsed() < Duration::from_millis(250),
        "shutdown request should not block on the live events connection"
    );
    drop(live_events_response);
}

#[test]
#[ignore = "requires SPARK_CODEX_APP_SERVER_BIN or a real codex app-server binary"]
fn desktop_server_codex_smoke_persists_codex_app_server_backend_marker() {
    let codex_bin = std::env::var("SPARK_CODEX_APP_SERVER_BIN")
        .expect("set SPARK_CODEX_APP_SERVER_BIN to codex app-server or the repo fake binary");
    assert!(!codex_bin.trim().is_empty());

    let temp = tempfile::tempdir().expect("tempdir");
    let paths = DesktopPaths::new(temp.path().join("data"), temp.path().join("config"));
    let log_path = temp.path().join("fake-codex-rpc.jsonl");
    let _mode_guard = EnvVarGuard::set("SPARK_FAKE_CODEX_APP_SERVER_MODE", "default");
    let _log_guard = EnvVarGuard::set("SPARK_FAKE_CODEX_APP_SERVER_LOG", &log_path);
    let _runtime_guard = EnvVarGuard::set(
        "ATTRACTOR_CODEX_RUNTIME_ROOT",
        temp.path().join("codex-runtime"),
    );
    let bootstrap =
        bootstrap_desktop_runtime(&paths, &DesktopServerSettings::default()).expect("bootstrap");
    let settings = bootstrap.settings.clone();
    let mut server =
        start_desktop_server(bootstrap.settings, &bootstrap.bind_host).expect("desktop server");

    let conversation_id = "desktop-codex-smoke";
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).expect("project dir");
    let response = http_client()
        .post(format!(
            "{}workspace/api/conversations/{conversation_id}/turns",
            server.url()
        ))
        .json(&serde_json::json!({
            "project_path": project_path,
            "message": "Run the desktop Codex smoke.",
            "provider": "codex",
            "model": "gpt-codex-test",
            "reasoning_effort": "HIGH",
            "chat_mode": "chat"
        }))
        .send()
        .expect("conversation turn response");

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body = response
        .json::<serde_json::Value>()
        .expect("conversation turn body");
    assert_eq!(body["conversation_id"], conversation_id);
    assert!(body["turns"].as_array().is_some_and(|turns| {
        turns.iter().any(|turn| {
            turn["role"] == "assistant" && turn["status"] == "complete" && turn["content"] == "Ack"
        })
    }));

    let events = ConversationRepository::new(&settings.data_dir)
        .read_conversation_events_after(conversation_id, project_path.to_string_lossy().as_ref(), 0)
        .expect("conversation events");
    assert!(events.iter().any(|event| {
        event["type"] == "segment_upsert"
            && event["segment"]["source"]["backend"] == "codex_app_server"
    }));

    server.shutdown();
}

fn http_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .expect("http client")
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}
