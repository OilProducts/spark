use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{json, Value};
use spark_assets::ResourceSource;
use spark_common::debug::ENV_SPARK_DEBUG_CODEX_JSONRPC;
use spark_common::settings::SettingsOverrides;
use spark_server::run_with_args_and_env;
use spark_server::{
    build_serve_configuration_from_args, resolve_server_settings_with_executable_path,
};

const STARTER_FLOW_NAMES: &[&str] = &[
    "examples/human-review-loop.yaml",
    "examples/implement-review-loop.yaml",
    "examples/parallel-review.yaml",
    "examples/simple-linear.yaml",
    "examples/supervision/implementation-worker.yaml",
    "examples/supervision/supervised-implementation.yaml",
    "software-development/audit-codebase.yaml",
    "software-development/design-change.yaml",
    "software-development/implement-change.yaml",
    "software-development/integrate-ready-branches.yaml",
    "software-development/investigate-bug.yaml",
    "software-development/merge-change.yaml",
    "software-development/review-change.yaml",
    "software-development/run-retrospective.yaml",
    "software-development/spec-implementation/implement-milestone.yaml",
    "software-development/spec-implementation/implement-spec.yaml",
    "software-development/workers/implement-task.yaml",
    "software-development/workers/resolve-merge-conflicts.yaml",
];

const TOP_LEVEL_HELP: &str = concat!(
    "usage: spark-server [-h] {serve,init,service} ...\n",
    "\n",
    "Spark operator CLI\n",
    "\n",
    "positional arguments:\n",
    "  {serve,init,service}\n",
    "    serve               Start the Spark API server\n",
    "    init                Initialize Spark runtime directories and seed packaged\n",
    "                        flows\n",
    "    service             Manage the installed Spark user service\n",
    "\n",
    "options:\n",
    "  -h, --help            show this help message and exit\n",
);

const WORKER_RUN_NODE_HELP: &str = concat!(
    "usage: spark-server worker run-node [-h]\n",
    "\n",
    "options:\n",
    "  -h, --help  show this help message and exit\n",
);

#[test]
fn top_level_help_hides_worker_command() {
    let env = BTreeMap::new();
    let output = run_with_args_and_env(["spark-server", "--help"], &env);

    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stdout, TOP_LEVEL_HELP);
    assert_eq!(output.stderr, "");
    assert!(!output.stdout.contains("worker"));
}

#[test]
fn worker_run_node_help_is_directly_callable() {
    let env = BTreeMap::new();
    let output = run_with_args_and_env(["spark-server", "worker", "run-node", "--help"], &env);

    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stdout, WORKER_RUN_NODE_HELP);
    assert_eq!(output.stderr, "");
}

#[test]
fn init_source_checkout_guard_matches_runtime_home_contract() {
    let env = BTreeMap::new();
    let output = run_with_args_and_env(["spark-server", "init"], &env);

    assert_eq!(output.exit_code, 1);
    assert_eq!(output.stdout, "");
    assert!(output
        .stderr
        .starts_with("Refusing to use default runtime home ~/.spark from a source checkout"));
    assert!(output.stderr.contains("before `spark-server init`"));
}

#[cfg(unix)]
#[test]
fn cargo_installed_server_process_init_uses_default_home_without_source_checkout_guard() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().expect("tempdir");
    // Canonicalize: init output prints canonical paths (macOS /var -> /private/var).
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let install_root = root.join("cargo-root");
    let installed_bin = install_root.join("bin/spark-server");
    let home = root.join("home");
    fs::create_dir_all(installed_bin.parent().expect("bin parent")).expect("bin dir");
    fs::create_dir_all(&home).expect("home dir");
    fs::copy(env!("CARGO_BIN_EXE_spark-server"), &installed_bin).expect("copy binary");
    let mut permissions = fs::metadata(&installed_bin)
        .expect("installed metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&installed_bin, permissions).expect("installed executable");

    let output = Command::new(&installed_bin)
        .arg("init")
        .env_clear()
        .env("HOME", &home)
        .output()
        .expect("run installed spark-server");

    let data_dir = home.join(".spark");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stderr).expect("stderr utf8"), "");
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout utf8"),
        format!(
            "Initialized Spark at {}\nSeeded flows: {}\ncreated=18 updated=0 skipped=0\n",
            data_dir.display(),
            data_dir.join("flows").display()
        )
    );
    assert!(data_dir.join("config/flow-catalog.toml").is_file());
    assert!(data_dir.join("flows/examples/simple-linear.yaml").is_file());
}

#[test]
fn service_install_source_checkout_guard_matches_runtime_home_contract() {
    let env = BTreeMap::new();
    let output = run_with_args_and_env(["spark-server", "service", "install"], &env);

    assert_eq!(output.exit_code, 1);
    assert_eq!(output.stdout, "");
    assert!(output
        .stderr
        .starts_with("Refusing to use default runtime home ~/.spark from a source checkout"));
    assert!(output
        .stderr
        .contains("before `spark-server service install`"));
}

#[test]
fn service_install_rejects_invalid_port_as_usage_error() {
    let env = BTreeMap::new();
    let output = run_with_args_and_env(
        ["spark-server", "service", "install", "--port", "not-a-port"],
        &env,
    );

    assert_eq!(output.exit_code, 2);
    assert_eq!(output.stdout, "");
    assert_eq!(
        output.stderr,
        "usage: spark-server [-h] {serve,init,service} ...\n\
spark-server: error: argument --port: invalid int value: 'not-a-port'\n"
    );
}

#[test]
fn init_creates_runtime_layout_flows_and_catalog() {
    let env = BTreeMap::new();
    let temp = tempfile::tempdir().expect("tempdir");
    // Canonicalize: init output prints canonical paths (macOS /var -> /private/var).
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let data_dir = root.join("spark-home");
    let flows_dir = root.join("flows");

    let output = run_with_args_and_env(
        [
            "spark-server",
            "init",
            "--data-dir",
            data_dir.to_str().expect("utf-8 data dir"),
            "--flows-dir",
            flows_dir.to_str().expect("utf-8 flows dir"),
        ],
        &env,
    );

    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stderr, "");
    assert_eq!(
        output.stdout,
        format!(
            "Initialized Spark at {}\nSeeded flows: {}\ncreated=18 updated=0 skipped=0\n",
            data_dir.display(),
            flows_dir.display()
        )
    );
    for relative in [
        "config",
        "runtime",
        "logs",
        "workspace",
        "workspace/projects",
        "attractor",
        "attractor/runs",
    ] {
        assert!(data_dir.join(relative).is_dir(), "missing {relative}");
    }
    assert_eq!(
        list_flow_files(&flows_dir),
        STARTER_FLOW_NAMES
            .iter()
            .map(|name| (*name).to_string())
            .collect::<Vec<_>>()
    );
    let catalog = fs::read_to_string(data_dir.join("config/flow-catalog.toml")).expect("catalog");
    assert_eq!(
        catalog
            .matches("launch_policy = \"agent_requestable\"")
            .count(),
        9
    );
    assert!(catalog.contains("[flows.\"software-development/merge-change.yaml\".execution_lock]"));
    assert!(catalog.contains("key = \"software-development-integration\""));
    assert!(!catalog.contains("software-development/workers/"));
}

#[test]
fn init_respects_skip_and_force_counts() {
    let env = BTreeMap::new();
    let temp = tempfile::tempdir().expect("tempdir");
    let data_dir = temp.path().join("spark-home");
    let flows_dir = temp.path().join("flows");
    let data_dir_text = data_dir.to_str().expect("utf-8 data dir");
    let flows_dir_text = flows_dir.to_str().expect("utf-8 flows dir");

    let first = run_with_args_and_env(
        [
            "spark-server",
            "init",
            "--data-dir",
            data_dir_text,
            "--flows-dir",
            flows_dir_text,
        ],
        &env,
    );
    assert_eq!(first.exit_code, 0);
    let edited_flow = flows_dir.join("examples/simple-linear.yaml");
    fs::write(&edited_flow, "edited: true\n").expect("user edit");

    let second = run_with_args_and_env(
        [
            "spark-server",
            "init",
            "--data-dir",
            data_dir_text,
            "--flows-dir",
            flows_dir_text,
        ],
        &env,
    );
    assert_eq!(second.exit_code, 0);
    assert!(second.stdout.ends_with("created=0 updated=0 skipped=18\n"));
    assert_eq!(
        fs::read_to_string(&edited_flow).expect("edited flow"),
        "edited: true\n"
    );

    let forced = run_with_args_and_env(
        [
            "spark-server",
            "init",
            "--data-dir",
            data_dir_text,
            "--flows-dir",
            flows_dir_text,
            "--force",
        ],
        &env,
    );
    assert_eq!(forced.exit_code, 0);
    assert!(forced.stdout.ends_with("created=0 updated=18 skipped=0\n"));
    assert_ne!(
        fs::read_to_string(&edited_flow).expect("forced flow"),
        "edited: true\n"
    );
}

#[test]
fn serve_validates_settings_and_parses_host_port_without_binding_socket() {
    let env = BTreeMap::new();
    let temp = tempfile::tempdir().expect("tempdir");
    let data_dir = temp.path().join("spark-home");
    let flows_dir = temp.path().join("flows");

    let serve = run_with_args_and_env(
        [
            "spark-server",
            "serve",
            "--data-dir",
            data_dir.to_str().expect("utf-8 data dir"),
            "--flows-dir",
            flows_dir.to_str().expect("utf-8 flows dir"),
            "--host",
            "127.0.0.1",
            "--port",
            "9100",
            "--reload",
        ],
        &env,
    );
    assert_eq!(serve.exit_code, 0);
    assert_eq!(serve.stderr, "");
    assert_eq!(
        serve.stdout,
        "spark-server serve configured for 127.0.0.1:9100\n"
    );
}

#[test]
fn serve_debug_codex_jsonrpc_flag_and_env_enable_process_debug_configuration() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data_dir = temp.path().join("spark-home");
    let flows_dir = temp.path().join("flows");

    let flag_config = build_serve_configuration_from_args(
        &[
            "serve".to_string(),
            "--data-dir".to_string(),
            data_dir.to_string_lossy().into_owned(),
            "--flows-dir".to_string(),
            flows_dir.to_string_lossy().into_owned(),
            "--debug-codex-jsonrpc".to_string(),
        ],
        &BTreeMap::new(),
    )
    .expect("serve config");
    assert!(flag_config.debug_codex_jsonrpc);

    let env_config = build_serve_configuration_from_args(
        &[
            "serve".to_string(),
            "--data-dir".to_string(),
            data_dir.to_string_lossy().into_owned(),
            "--flows-dir".to_string(),
            flows_dir.to_string_lossy().into_owned(),
        ],
        &BTreeMap::from([(ENV_SPARK_DEBUG_CODEX_JSONRPC.to_string(), "YES".to_string())]),
    )
    .expect("serve env config");
    assert!(env_config.debug_codex_jsonrpc);

    let default_config = build_serve_configuration_from_args(
        &[
            "serve".to_string(),
            "--data-dir".to_string(),
            data_dir.to_string_lossy().into_owned(),
            "--flows-dir".to_string(),
            flows_dir.to_string_lossy().into_owned(),
        ],
        &BTreeMap::new(),
    )
    .expect("serve default config");
    assert!(!default_config.debug_codex_jsonrpc);
}

#[test]
fn serve_configuration_preserves_ui_dir_and_rejects_missing_index() {
    let env = BTreeMap::new();
    let temp = tempfile::tempdir().expect("tempdir");
    // Canonicalize: resolved settings hold canonical paths (macOS /var -> /private/var).
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let data_dir = root.join("spark-home");
    let flows_dir = root.join("flows");
    let ui_dir = root.join("ui");
    fs::create_dir_all(&ui_dir).expect("ui dir");
    fs::write(ui_dir.join("index.html"), "<!doctype html>\n").expect("index");

    let config = build_serve_configuration_from_args(
        &[
            "serve".to_string(),
            "--data-dir".to_string(),
            data_dir.to_string_lossy().into_owned(),
            "--flows-dir".to_string(),
            flows_dir.to_string_lossy().into_owned(),
            "--ui-dir".to_string(),
            ui_dir.to_string_lossy().into_owned(),
        ],
        &env,
    )
    .expect("serve config");

    assert_eq!(config.settings.ui_dir.as_deref(), Some(ui_dir.as_path()));

    let missing_ui = temp.path().join("missing-ui");
    fs::create_dir_all(&missing_ui).expect("missing ui dir");
    let error = build_serve_configuration_from_args(
        &[
            "serve".to_string(),
            "--data-dir".to_string(),
            data_dir.to_string_lossy().into_owned(),
            "--flows-dir".to_string(),
            flows_dir.to_string_lossy().into_owned(),
            "--ui-dir".to_string(),
            missing_ui.to_string_lossy().into_owned(),
        ],
        &env,
    )
    .expect_err("missing index rejects ui dir");

    assert_eq!(error.exit_code, 1);
    assert!(error.stderr.contains("UI directory"));
    assert!(error.stderr.contains("index.html"));
}

#[test]
fn installed_server_settings_use_package_root_for_resource_fallback() {
    let env = BTreeMap::new();
    let temp = tempfile::tempdir().expect("tempdir");
    let data_dir = temp.path().join("spark-home");
    let flows_dir = temp.path().join("flows");
    let package_root = temp.path().join("site-packages/spark");
    let executable_path = package_root.join("bin/spark-server");
    let ui_dist = package_root.join("ui_dist");
    let installed_index = ui_dist.join("index.html");
    let installed_icon = ui_dist.join("assets/spark-app-icon.png");
    fs::create_dir_all(executable_path.parent().expect("bin parent")).expect("bin dir");
    fs::create_dir_all(ui_dist.join("assets")).expect("ui dist");
    fs::write(&installed_index, "<!doctype html><div id=\"root\"></div>").expect("index");
    fs::write(&installed_icon, b"installed-package-icon").expect("icon");

    let settings = resolve_server_settings_with_executable_path(
        &SettingsOverrides {
            data_dir: Some(data_dir),
            flows_dir: Some(flows_dir),
            runs_dir: None,
            ui_dir: None,
        },
        &env,
        Some(&executable_path),
    )
    .expect("settings");

    assert_eq!(settings.project_root, package_root);
    assert_eq!(
        spark_assets::frontend::resolve_index_path(&settings),
        Some(installed_index.clone())
    );
    let index = spark_assets::frontend::load_index(&settings).expect("packaged index");
    assert_eq!(index.source(), ResourceSource::Packaged);
    assert_eq!(index.filesystem_path(), Some(installed_index.as_path()));
    assert_eq!(
        spark_assets::frontend::load_asset(&settings, "assets/spark-app-icon.png")
            .expect("asset lookup")
            .expect("installed icon")
            .bytes(),
        b"installed-package-icon"
    );
}

#[test]
fn visible_later_scope_commands_fail_explicitly() {
    let env = BTreeMap::new();

    let worker = run_with_args_and_env(["spark-server", "worker", "run-node"], &env);
    assert_eq!(worker.exit_code, 1);
    assert_eq!(worker.stderr, "");
    assert!(worker.stdout.contains("\"type\":\"result\""));
    assert!(!worker.stdout.contains("not implemented"));
}

#[test]
fn worker_run_node_process_accepts_json_line_request() {
    let binary = env!("CARGO_BIN_EXE_spark-server");
    let request = json!({
        "run_id": "run-worker-process",
        "flow": {
            "schema_version": "1",
            "id": "G",
            "nodes": {
                "start": {
                    "kind": "start",
                    "config": {"kind": "start"}
                }
            },
            "edges": []
        },
        "node_id": "start",
        "prompt": "",
        "context": {},
        "context_logs": [],
        "logs_root": null,
        "working_dir": ".",
        "backend_name": null,
        "model": null,
        "config_dir": null
    });
    let mut child = Command::new(binary)
        .args(["worker", "run-node"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn worker");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all((request.to_string() + "\n").as_bytes())
        .expect("write request");

    let output = child.wait_with_output().expect("worker output");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let frame: Value = serde_json::from_slice(&output.stdout).expect("worker frame");
    assert_eq!(frame["type"], json!("result"));
    assert_eq!(frame["outcome"]["status"], json!("success"));
}

#[test]
fn worker_run_node_process_routes_llm_nodes_to_rust_adapter_boundary() {
    let binary = env!("CARGO_BIN_EXE_spark-server");
    let request = json!({
        "run_id": "run-worker-llm-process",
        "flow": {
            "schema_version": "1",
            "id": "G",
            "nodes": {
                "task": {
                    "kind": "agent_task",
                    "config": {
                        "kind": "agent_task",
                        "prompt": "Write a worker note"
                    },
                    "execution": {
                        "llm_provider": "OpenAI",
                        "llm_model": "gpt-boundary"
                    }
                }
            },
            "edges": []
        },
        "node_id": "task",
        "prompt": "Write a worker note",
        "context": {},
        "context_logs": [],
        "logs_root": null,
        "working_dir": ".",
        "backend_name": null,
        "model": null,
        "config_dir": null
    });
    let listener = TcpListener::bind("127.0.0.1:0").expect("unused local listener");
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let mut child = Command::new(binary)
        .args(["worker", "run-node"])
        .env("OPENAI_API_KEY", "test-key")
        .env("OPENAI_BASE_URL", base_url)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn worker");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all((request.to_string() + "\n").as_bytes())
        .expect("write request");

    let output = child.wait_with_output().expect("worker output");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let frame: Value = serde_json::from_slice(&output.stdout).expect("worker frame");
    assert_eq!(frame["type"], json!("result"));
    assert_eq!(frame["outcome"]["status"], json!("fail"));
    let reason = frame["outcome"]["failure_reason"]
        .as_str()
        .expect("failure reason");
    assert!(reason.contains("NetworkError"));
    assert!(reason.contains("Provider 'openai' HTTP network failed"));
    assert!(!reason.contains("no HTTP transport is configured"));
}

fn list_flow_files(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    collect_flow_files(root, root, &mut files);
    files.sort();
    files
}

fn collect_flow_files(root: &Path, current: &Path, files: &mut Vec<String>) {
    let entries = fs::read_dir(current).expect("read flow dir");
    for entry in entries {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            collect_flow_files(root, &path, files);
        } else if matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("yaml" | "yml")
        ) {
            files.push(
                path.strip_prefix(root)
                    .expect("relative flow")
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
}
