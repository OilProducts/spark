use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{json, Value};
use spark_assets::ResourceSource;
use spark_common::settings::SettingsOverrides;
use spark_server::run_with_args_and_env;
use spark_server::{
    build_serve_configuration_from_args, resolve_server_settings_with_executable_path,
};

const STARTER_FLOW_NAMES: &[&str] = &[
    "examples/human-review-loop.dot",
    "examples/implement-review-loop.dot",
    "examples/parallel-review.dot",
    "examples/simple-linear.dot",
    "examples/supervision/implementation-worker.dot",
    "examples/supervision/supervised-implementation.dot",
    "software-development/implement-change-request.dot",
    "software-development/spec-implementation/implement-milestone.dot",
    "software-development/spec-implementation/implement-spec.dot",
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
    let data_dir = temp.path().join("spark-home");
    let flows_dir = temp.path().join("flows");

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
            "Initialized Spark at {}\nSeeded flows: {}\ncreated=9 updated=0 skipped=0\n",
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
        list_dot_files(&flows_dir),
        STARTER_FLOW_NAMES
            .iter()
            .map(|name| (*name).to_string())
            .collect::<Vec<_>>()
    );
    let catalog = fs::read_to_string(data_dir.join("config/flow-catalog.toml")).expect("catalog");
    assert_eq!(
        catalog,
        "[flows.\"software-development/implement-change-request.dot\"]\n\
launch_policy = \"agent_requestable\"\n\
\n\
[flows.\"software-development/spec-implementation/implement-spec.dot\"]\n\
launch_policy = \"agent_requestable\"\n"
    );
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
    let edited_flow = flows_dir.join("examples/simple-linear.dot");
    fs::write(&edited_flow, "digraph UserEdited {}\n").expect("user edit");

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
    assert!(second.stdout.ends_with("created=0 updated=0 skipped=9\n"));
    assert_eq!(
        fs::read_to_string(&edited_flow).expect("edited flow"),
        "digraph UserEdited {}\n"
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
    assert!(forced.stdout.ends_with("created=0 updated=9 skipped=0\n"));
    assert_ne!(
        fs::read_to_string(&edited_flow).expect("forced flow"),
        "digraph UserEdited {}\n"
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
fn serve_configuration_preserves_ui_dir_and_rejects_missing_index() {
    let env = BTreeMap::new();
    let temp = tempfile::tempdir().expect("tempdir");
    let data_dir = temp.path().join("spark-home");
    let flows_dir = temp.path().join("flows");
    let ui_dir = temp.path().join("ui");
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
        "graph": {
            "graph_id": "G",
            "graph_attrs": {},
            "nodes": {
                "start": {
                    "node_id": "start",
                    "attrs": {
                        "shape": {"key": "shape", "value": "Mdiamond", "value_type": "string", "line": 0}
                    },
                    "line": 0,
                    "explicit_attr_keys": ["shape"]
                }
            },
            "edges": [],
            "defaults": {"node": {}, "edge": {}},
            "subgraphs": []
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
        "graph": {
            "graph_id": "G",
            "graph_attrs": {},
            "nodes": {
                "task": {
                    "node_id": "task",
                    "attrs": {
                        "shape": {"key": "shape", "value": "box", "value_type": "string", "line": 1},
                        "prompt": {"key": "prompt", "value": "Write a worker note", "value_type": "string", "line": 1},
                        "llm_provider": {"key": "llm_provider", "value": "OpenAI", "value_type": "string", "line": 1},
                        "llm_model": {"key": "llm_model", "value": "gpt-boundary", "value_type": "string", "line": 1}
                    },
                    "line": 1,
                    "explicit_attr_keys": ["shape", "prompt", "llm_provider", "llm_model"]
                }
            },
            "edges": [],
            "defaults": {"node": {}, "edge": {}},
            "subgraphs": []
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

fn list_dot_files(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    collect_dot_files(root, root, &mut files);
    files.sort();
    files
}

fn collect_dot_files(root: &Path, current: &Path, files: &mut Vec<String>) {
    let entries = fs::read_dir(current).expect("read flow dir");
    for entry in entries {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            collect_dot_files(root, &path, files);
        } else if path.extension().and_then(|value| value.to_str()) == Some("dot") {
            files.push(
                path.strip_prefix(root)
                    .expect("relative flow")
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
}
