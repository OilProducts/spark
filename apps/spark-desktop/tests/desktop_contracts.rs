use std::fs;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use spark_desktop::desktop_core::{
    bootstrap_desktop_runtime, core_config_file, default_spark_data_dir, desktop_settings_revision,
    frontend_url_for_addr, is_app_owned_data_dir, load_desktop_settings, server_host_for_settings,
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

    assert_eq!(
        bootstrap.seeded_flows.created,
        spark_assets::flows::starter_flow_names().expect("packaged starter flow names")
    );
    assert!(bootstrap
        .settings
        .flows_dir
        .join("software-development/implement-change.yaml")
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
    assert!(set_remote_access_enabled(
        &paths,
        true,
        false,
        &desktop_settings_revision(&paths).unwrap()
    )
    .is_err());

    let enabled = set_remote_access_enabled(
        &paths,
        true,
        true,
        &desktop_settings_revision(&paths).unwrap(),
    )
    .expect("enable remote");
    assert_eq!(server_host_for_settings(&enabled), REMOTE_BIND_HOST);
    assert!(fs::read_to_string(core_config_file(&paths))
        .unwrap()
        .contains("remote_access_enabled = true"));

    let disabled = set_remote_access_enabled(
        &paths,
        false,
        false,
        &desktop_settings_revision(&paths).unwrap(),
    )
    .expect("disable remote");
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
        "revision".into(),
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

#[test]
fn desktop_permissions_are_restricted_to_expected_commands_and_origins() {
    let capability: serde_json::Value =
        serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
    let permissions = capability["permissions"].as_array().unwrap();
    assert_eq!(capability["windows"], serde_json::json!(["main"]));
    assert_eq!(
        capability["remote"],
        serde_json::json!({"urls": ["http://127.0.0.1:*"]})
    );
    let desktop: serde_json::Value =
        serde_json::from_str(include_str!("../permissions/desktop.json")).unwrap();
    assert_eq!(desktop["permission"].as_array().unwrap().len(), 1);
    assert_eq!(
        desktop["permission"][0]["identifier"],
        "allow-desktop-settings"
    );
    assert_eq!(
        desktop["permission"][0]["commands"],
        serde_json::json!({
            "allow": ["desktop_client_identity", "desktop_server_settings", "set_desktop_remote_access_enabled"]
        })
    );
    assert_eq!(
        permissions,
        &vec![
            serde_json::json!("core:default"),
            serde_json::json!("allow-desktop-settings"),
            serde_json::json!({
                "identifier": "opener:allow-open-url",
                "allow": [{"url": "http://*"}, {"url": "https://*"}]
            })
        ]
    );
}

#[test]
fn desktop_runtime_and_model_settings_survive_restarts_on_different_ports() {
    let temp = tempfile::tempdir().unwrap();
    let paths = DesktopPaths::new(temp.path().join("data"), temp.path().join("config"));
    let client_id = spark_desktop::desktop_core::desktop_client_identity(&paths).unwrap();
    let desktop = load_desktop_settings(&paths).unwrap();
    let bootstrap = bootstrap_desktop_runtime(&paths, &desktop).unwrap();
    let mut first = start_desktop_server(bootstrap.settings, LOCAL_BIND_HOST).unwrap();
    let first_url = first.url().to_string();
    let client = http_client();
    let document: serde_json::Value = client
        .get(format!("{first_url}workspace/api/settings"))
        .send()
        .unwrap()
        .json()
        .unwrap();
    let flows = temp.path().canonicalize().unwrap().join("new-flows");
    let response = client.patch(format!("{first_url}workspace/api/settings")).json(&serde_json::json!({
        "expected_revision": document["runtime"]["revision"], "section": "runtime", "value": {"flows_dir": flows, "project_roots": []}
    })).send().unwrap();
    assert!(
        response.status().is_success(),
        "{}",
        response.text().unwrap()
    );
    let saved: serde_json::Value = response.json().unwrap();
    let model_response = client.patch(format!("{first_url}workspace/api/settings")).json(&serde_json::json!({
        "expected_revision": saved["models"]["revision"], "section": "models", "value": {"provider": "codex", "model": "restart-model"}
    })).send().unwrap();
    assert!(model_response.status().is_success());
    let preference_response = client.patch(format!("{first_url}workspace/api/settings")).json(&serde_json::json!({
        "expected_revision": "absent", "section": "client_preferences",
        "value": {"client_id": client_id, "preferences": {"editor_mode": "raw", "editor_sidebar_width": 400}}
    })).send().unwrap();
    assert!(preference_response.status().is_success());
    // Keep the old listener open so the replacement necessarily gets a different port.
    let restarted =
        bootstrap_desktop_runtime(&paths, &load_desktop_settings(&paths).unwrap()).unwrap();
    assert_eq!(restarted.settings.flows_dir, flows);
    let mut second = start_desktop_server(restarted.settings, LOCAL_BIND_HOST).unwrap();
    assert_ne!(second.url(), first_url);
    assert_eq!(
        spark_desktop::desktop_core::desktop_client_identity(&paths).unwrap(),
        client_id
    );
    let preferences: serde_json::Value = client
        .get(format!(
            "{}workspace/api/settings?client_id={client_id}",
            second.url()
        ))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(
        preferences["preferences"]["effective"]["editor_sidebar_width"],
        400
    );
    assert_eq!(
        preferences["preferences"]["effective"]["editor_mode"],
        "raw"
    );
    let response: serde_json::Value = client
        .get(format!("{}workspace/api/settings", second.url()))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(
        response["runtime"]["effective"]["flows_dir"],
        flows.to_string_lossy().as_ref()
    );
    assert_eq!(response["models"]["stored"]["model"], "restart-model");
    assert_eq!(response["models"]["effective"]["model"], "restart-model");
    first.shutdown();
    second.shutdown();
}

#[test]
fn native_settings_save_publishes_through_existing_http_live_transport() {
    use std::io::{BufRead, BufReader};
    let temp = tempfile::tempdir().unwrap();
    let paths = DesktopPaths::new(temp.path().join("data"), temp.path().join("config"));
    let bootstrap = bootstrap_desktop_runtime(&paths, &DesktopServerSettings::default()).unwrap();
    let mut server = start_desktop_server(bootstrap.settings, &bootstrap.bind_host).unwrap();
    let response = http_client()
        .get(format!(
            "{}workspace/api/live/events?include_settings=true",
            server.url()
        ))
        .send()
        .unwrap();
    assert!(response.status().is_success());
    let original = desktop_settings_revision(&paths).unwrap();
    assert!(server
        .save_remote_access(&paths, true, false, &original)
        .is_err());
    assert_eq!(desktop_settings_revision(&paths).unwrap(), original);
    server
        .save_remote_access(&paths, true, true, &original)
        .unwrap();
    let revision = desktop_settings_revision(&paths).unwrap();
    assert!(server
        .save_remote_access(&paths, false, false, &original)
        .is_err());
    let mut stream = BufReader::new(response);
    loop {
        let mut line = String::new();
        assert!(stream.read_line(&mut line).unwrap() > 0);
        if let Some(payload) = line.strip_prefix("data: ") {
            let event: serde_json::Value = serde_json::from_str(payload).unwrap();
            if event["type"] == "settings.changed" {
                assert_eq!(event["payload"]["section"], "desktop");
                assert_eq!(event["payload"]["revision"], revision);
                break;
            }
        }
    }
    drop(stream);
    server.shutdown();
}

#[test]
fn desktop_retains_environment_selected_startup_paths() {
    const CHILD: &str = "SPARK_DESKTOP_PRECEDENCE_TEST_ROOT";
    let Ok(root) = std::env::var(CHILD) else {
        let temp = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "desktop_retains_environment_selected_startup_paths",
                "--nocapture",
            ])
            .env(CHILD, temp.path())
            .env(
                "SPARK_CLAUDE_CODE_CONFIG_DIR",
                temp.path().join("environment-claude"),
            )
            .env("SPARK_HOME", temp.path().join("ignored-home"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    };
    let root = std::path::PathBuf::from(root);
    let paths = DesktopPaths::new(root.join("data"), root.join("config"));
    let boot = bootstrap_desktop_runtime(&paths, &DesktopServerSettings::default()).unwrap();
    assert_eq!(
        boot.settings.data_dir,
        default_spark_data_dir(&paths).canonicalize().unwrap()
    );
    let selected = root
        .join("environment-claude")
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        boot.settings.agents.native.claude_config_dir,
        Some(selected.clone())
    );
    let core = boot.settings.config_dir.join("spark.toml");
    let source = fs::read_to_string(&core).unwrap();
    fs::write(
        &core,
        format!("{source}\n[agents.native]\nclaude_config_dir='/after-restart'\n"),
    )
    .unwrap();
    let mut captured = spark_storage::settings::read_execution_configuration(
        &boot.settings.config_dir,
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(
        captured.agents.native.claude_config_dir.as_deref(),
        Some("/after-restart")
    );
    captured.retain_startup_settings(&boot.settings);
    assert_eq!(captured.agents.native.claude_config_dir, Some(selected));
}
