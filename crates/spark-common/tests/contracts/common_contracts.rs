use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;
use spark_common::events::{
    TurnStreamChannel, TurnStreamEvent, TurnStreamEventKind, TurnStreamSource,
};
use spark_common::logging::spark_target;
use spark_common::paths::{
    ensure_writable_directory, resolve_runtime_workspace_path_with_env, WRITABLE_DIRECTORY_PROBE,
};
use spark_common::process::ProcessLineReader;
use spark_common::project::{build_project_id, normalize_project_path};
use spark_common::settings::{
    parse_project_roots, resolve_settings_with_env, validate_settings, SettingsOverrides,
};
use spark_common::source_checkout::{
    default_api_target_refusal_message, default_runtime_home_refusal_message,
    installed_package_root_from_executable, is_source_checkout,
    require_explicit_agent_base_url_with_env, require_explicit_dev_home_with_env,
};

#[cfg(unix)]
fn symlink_dir(original: &Path, link: &Path) {
    std::os::unix::fs::symlink(original, link).expect("symlink dir");
}

// TempDir may hand back a symlinked path (macOS /var -> /private/var); the code
// under test resolves symlinks, so expectations must start from the physical path.
fn canonical_tempdir() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    (temp, root)
}

fn env(entries: &[(&str, &Path)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.to_string_lossy().into_owned()))
        .collect()
}

#[test]
fn settings_resolution_preserves_precedence_and_derived_paths() {
    let (_temp, root) = canonical_tempdir();
    let env_home = root.join("env-home");
    let cli_home = root.join("cli-home");
    let flows = root.join("flows");
    let ui = root.join("ui");
    let env = env(&[
        ("SPARK_HOME", &env_home),
        ("SPARK_FLOWS_DIR", &flows),
        ("SPARK_UI_DIR", &ui),
    ]);

    let settings = resolve_settings_with_env(
        &SettingsOverrides {
            data_dir: Some(cli_home.clone()),
            ..SettingsOverrides::default()
        },
        &env,
    )
    .expect("settings");

    assert_eq!(settings.data_dir, cli_home);
    assert_eq!(settings.config_dir, settings.data_dir.join("config"));
    assert_eq!(settings.runtime_dir, settings.data_dir.join("runtime"));
    assert_eq!(settings.logs_dir, settings.data_dir.join("logs"));
    assert_eq!(settings.workspace_dir, settings.data_dir.join("workspace"));
    assert_eq!(
        settings.projects_dir,
        settings.data_dir.join("workspace/projects")
    );
    assert_eq!(settings.attractor_dir, settings.data_dir.join("attractor"));
    assert_eq!(settings.runs_dir, settings.data_dir.join("attractor/runs"));
    assert_eq!(settings.flows_dir, flows);
    assert_eq!(settings.ui_dir, Some(ui));
}

#[test]
fn project_roots_parse_absolute_entries_and_skip_empty_or_relative_entries() {
    let (_temp, root) = canonical_tempdir();
    let primary = root.join("primary");
    let secondary = root.join("secondary");
    let roots = std::env::join_paths([
        PathBuf::from("relative"),
        primary.clone(),
        PathBuf::new(),
        secondary.clone(),
    ])
    .expect("join paths");
    let roots = parse_project_roots(roots.to_str()).expect("project roots");

    assert_eq!(roots, vec![primary, secondary]);
}

#[cfg(unix)]
#[test]
fn project_roots_resolve_existing_symlink_prefixes_without_requiring_missing_tails() {
    let (_temp, root) = canonical_tempdir();
    let real_root = root.join("real-root");
    let linked_root = root.join("linked-root");
    std::fs::create_dir_all(&real_root).expect("real root");
    symlink_dir(&real_root, &linked_root);

    let requested_project_root = linked_root.join("future-project");
    let expected_project_root = real_root.join("future-project");
    let roots = std::env::join_paths([requested_project_root, PathBuf::from("relative")])
        .expect("join paths");

    let parsed = parse_project_roots(roots.to_str()).expect("project roots");

    assert_eq!(parsed, vec![expected_project_root]);
}

#[test]
fn validate_settings_creates_runtime_directories_and_checks_ui_index() {
    let temp = tempfile::tempdir().expect("tempdir");
    let ui = temp.path().join("ui");
    std::fs::create_dir_all(&ui).expect("ui dir");
    std::fs::write(ui.join("index.html"), "<main></main>").expect("ui index");
    let env = env(&[
        ("SPARK_HOME", &temp.path().join("spark-home")),
        ("SPARK_UI_DIR", &ui),
    ]);
    let settings =
        resolve_settings_with_env(&SettingsOverrides::default(), &env).expect("settings");

    validate_settings(&settings).expect("valid settings");

    for path in [
        &settings.config_dir,
        &settings.runtime_dir,
        &settings.logs_dir,
        &settings.workspace_dir,
        &settings.projects_dir,
        &settings.attractor_dir,
        &settings.runs_dir,
        &settings.flows_dir,
    ] {
        assert!(path.is_dir(), "{} should be created", path.display());
        assert!(!path.join(WRITABLE_DIRECTORY_PROBE).exists());
    }
}

#[test]
fn writable_directory_errors_use_compatible_message_shape() {
    let temp = tempfile::tempdir().expect("tempdir");
    let blocked = temp.path().join("blocked");
    std::fs::write(&blocked, "not a dir").expect("blocked file");

    let error = ensure_writable_directory(&blocked, "runtime").expect_err("directory creation");

    assert!(error
        .to_string()
        .starts_with("Unable to create runtime directory: "));
}

#[test]
fn project_identity_matches_slug_and_sha1_contract() {
    // Use a prefix that exists on no platform so resolve(strict=false) semantics
    // keep the path verbatim; /tmp is a symlink on macOS and would change the hash.
    let project_id = build_project_id("/spark-contract-fixture/Spark Demo!").expect("project id");

    assert_eq!(project_id, "spark-demo-23a3ac05822d");
    assert_eq!(normalize_project_path("  ").expect("empty normalize"), None);
    assert_eq!(
        build_project_id("  ")
            .expect_err("empty project")
            .to_string(),
        "Project path is required."
    );
}

#[cfg(unix)]
#[test]
fn project_identity_resolves_symlinked_prefixes_like_python_resolve_strict_false() {
    let (_temp, root) = canonical_tempdir();
    let real_root = root.join("real-root");
    let linked_root = root.join("linked-root");
    let real_project = real_root.join("Spark Demo!");
    std::fs::create_dir_all(&real_project).expect("real project");
    symlink_dir(&real_root, &linked_root);
    let requested_project = linked_root.join("Spark Demo!");

    assert_eq!(
        normalize_project_path(requested_project.to_string_lossy())
            .expect("normalize project")
            .as_deref(),
        Some(real_project.as_path())
    );
    assert_eq!(
        build_project_id(requested_project.to_string_lossy()).expect("linked project id"),
        build_project_id(real_project.to_string_lossy()).expect("real project id")
    );

    let requested_missing_tail = requested_project.join("future");
    let real_missing_tail = real_project.join("future");
    assert_eq!(
        normalize_project_path(requested_missing_tail.to_string_lossy())
            .expect("normalize missing tail")
            .as_deref(),
        Some(real_missing_tail.as_path())
    );
}

#[test]
fn runtime_workspace_path_remaps_host_root_to_existing_runtime_root() {
    let (_temp, root) = canonical_tempdir();
    let host_root = root.join("host/repo");
    let runtime_root = root.join("runtime/repo");
    let runtime_file = runtime_root.join("project/src/main.rs");
    std::fs::create_dir_all(runtime_file.parent().expect("parent")).expect("runtime dirs");
    std::fs::write(&runtime_file, "fn main() {}\n").expect("runtime file");
    let requested = host_root.join("project/src/main.rs");
    let env = env(&[
        ("ATTRACTOR_HOST_REPO_ROOT", &host_root),
        ("ATTRACTOR_RUNTIME_REPO_ROOT", &runtime_root),
    ]);

    let resolved = resolve_runtime_workspace_path_with_env(requested.to_string_lossy(), &env)
        .expect("runtime path");

    assert_eq!(resolved, runtime_file.to_string_lossy());
}

#[test]
fn runtime_workspace_path_keeps_existing_requested_path() {
    let (_temp, root) = canonical_tempdir();
    let host_root = root.join("host/repo");
    let runtime_root = root.join("runtime/repo");
    let requested = host_root.join("project/src/main.rs");
    let runtime_file = runtime_root.join("project/src/main.rs");
    std::fs::create_dir_all(requested.parent().expect("parent")).expect("host dirs");
    std::fs::create_dir_all(runtime_file.parent().expect("parent")).expect("runtime dirs");
    std::fs::write(&requested, "host").expect("host file");
    std::fs::write(&runtime_file, "runtime").expect("runtime file");
    let env = env(&[
        ("ATTRACTOR_HOST_REPO_ROOT", &host_root),
        ("ATTRACTOR_RUNTIME_REPO_ROOT", &runtime_root),
    ]);

    let resolved = resolve_runtime_workspace_path_with_env(requested.to_string_lossy(), &env)
        .expect("runtime path");

    assert_eq!(resolved, requested.to_string_lossy());
}

#[test]
fn source_checkout_guard_messages_match_cargo_contract() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    std::fs::write(root.join("Cargo.toml"), "[workspace]\n").expect("cargo manifest");
    std::fs::create_dir_all(root.join("crates/spark-cli")).expect("cli crate");
    std::fs::create_dir_all(root.join("crates/spark-server")).expect("server crate");
    std::fs::create_dir_all(root.join("crates/spark-assets/assets/flows")).expect("flows");
    std::fs::create_dir_all(root.join("frontend")).expect("frontend");
    let empty_env = BTreeMap::<String, String>::new();

    assert!(is_source_checkout(root));
    let home_error =
        require_explicit_dev_home_with_env("spark-server init", None, root, &empty_env)
            .expect_err("home guard");
    assert_eq!(
        home_error.to_string(),
        default_runtime_home_refusal_message("spark-server init", root)
    );
    let api_error =
        require_explicit_agent_base_url_with_env("spark flow list", None, root, &empty_env)
            .expect_err("api guard");
    assert_eq!(
        api_error.to_string(),
        default_api_target_refusal_message("spark flow list", root)
    );

    let mut configured_env = BTreeMap::new();
    configured_env.insert(
        "SPARK_HOME".to_string(),
        root.join(".spark-dev").display().to_string(),
    );
    require_explicit_dev_home_with_env("spark-server init", None, root, &configured_env)
        .expect("configured home");
    configured_env.insert("SPARK_HOME".to_string(), String::new());
    require_explicit_dev_home_with_env("spark-server init", None, root, &configured_env)
        .expect_err("empty home does not satisfy guard");
}

#[test]
fn installed_package_root_detection_accepts_cargo_install_roots_without_source_guarding() {
    let temp = tempfile::tempdir().expect("tempdir");
    let install_root = temp.path().join("cargo-root");
    let executable_path = install_root.join("bin/spark-server");
    std::fs::create_dir_all(executable_path.parent().expect("bin parent")).expect("bin dir");

    assert_eq!(
        installed_package_root_from_executable(&executable_path, "spark-server").as_deref(),
        Some(install_root.as_path())
    );
    assert!(!is_source_checkout(&install_root));
    require_explicit_dev_home_with_env(
        "spark-server init",
        None,
        &install_root,
        &BTreeMap::<String, String>::new(),
    )
    .expect("installed cargo root may use default home");
}

#[test]
fn process_line_reader_drains_lines_and_repeats_eof() {
    let stream = Cursor::new("one\ntwo\npartial".as_bytes().to_vec());
    let mut reader = ProcessLineReader::new(stream);

    assert_eq!(
        reader.read_line(Duration::from_millis(100)).as_deref(),
        Some("one")
    );
    assert_eq!(
        reader.read_line(Duration::from_millis(100)).as_deref(),
        Some("two")
    );
    assert_eq!(
        reader.read_line(Duration::from_millis(100)).as_deref(),
        Some("partial")
    );
    assert_eq!(reader.read_line(Duration::from_millis(100)), None);
    assert_eq!(reader.read_line(Duration::from_millis(100)), None);
    assert!(reader.join(Some(Duration::from_secs(1))).expect("join"));
}

#[test]
fn logging_target_names_follow_spark_logger_rules() {
    assert_eq!(spark_target(""), "spark");
    assert_eq!(spark_target("workspace"), "spark.workspace");
    assert_eq!(spark_target("spark.workspace"), "spark.workspace");
}

#[test]
fn turn_stream_events_round_trip_and_validate_content_channel() {
    let mut event = TurnStreamEvent::content_delta(TurnStreamChannel::Assistant, "Ack");
    event.source = TurnStreamSource {
        backend: Some("codex_app_server".to_string()),
        item_id: Some("msg-1".to_string()),
        raw_kind: Some("response/output_text/delta".to_string()),
        ..TurnStreamSource::default()
    };
    let payload = serde_json::to_value(&event).expect("serialize event");

    assert_eq!(
        payload,
        json!({
            "kind": "content_delta",
            "channel": "assistant",
            "source": {
                "backend": "codex_app_server",
                "item_id": "msg-1",
                "raw_kind": "response/output_text/delta"
            },
            "content_delta": "Ack",
            "message": "Ack"
        })
    );
    assert_eq!(
        serde_json::from_value::<TurnStreamEvent>(payload).expect("deserialize event"),
        event
    );
    assert!(
        serde_json::from_value::<TurnStreamEvent>(json!({"kind": "content_delta"}))
            .expect_err("content channel validation")
            .to_string()
            .contains("content TurnStreamEvent values must set channel")
    );

    let unknown = TurnStreamEvent::new(TurnStreamEventKind::Other("custom".to_string()))
        .expect("unknown kind event");
    assert_eq!(
        serde_json::to_value(unknown).expect("unknown json")["kind"],
        "custom"
    );
}
