use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use spark_assets::ResourceSource;
use spark_common::debug::ENV_SPARK_DEBUG_CODEX_JSONRPC;
use spark_common::settings::SettingsOverrides;
use spark_server::run_with_args_and_env;
use spark_server::{
    build_serve_configuration_from_args, resolve_server_settings_with_executable_path,
};

/// Every packaged flow file (`<flow>/<file>.md`); seeding counts files.
fn starter_flow_files() -> Vec<String> {
    spark_assets::flows::starter_flow_assets()
        .expect("packaged starter flow assets")
        .into_iter()
        .map(|asset| asset.name)
        .collect()
}

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
    let flow_count = starter_flow_files().len();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stderr).expect("stderr utf8"), "");
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout utf8"),
        format!(
            "Initialized Spark at {}\nBundled library: {}\n",
            data_dir.display(),
            data_dir.join("bundled").display()
        )
    );
    assert!(!data_dir.join("config/flow-catalog.toml").exists());
    assert_eq!(
        list_flow_files(&data_dir.join("bundled/flows")).len(),
        flow_count
    );
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
fn init_creates_runtime_layout_and_bundled_library() {
    let env = BTreeMap::new();
    let temp = tempfile::tempdir().expect("tempdir");
    // Canonicalize: init output prints canonical paths (macOS /var -> /private/var).
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let data_dir = root.join("spark-home");
    let bundled = data_dir.join("bundled");

    let output = run_with_args_and_env(
        [
            "spark-server",
            "init",
            "--data-dir",
            data_dir.to_str().expect("utf-8 data dir"),
        ],
        &env,
    );

    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stderr, "");
    assert_eq!(
        output.stdout,
        format!(
            "Initialized Spark at {}\nBundled library: {}\n",
            data_dir.display(),
            bundled.display(),
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
        list_flow_files(&bundled.join("flows")),
        starter_flow_files()
    );
    assert!(bundled.join("roles/adversarial-reviewer.md").is_file());
    assert!(!data_dir.join("flows").exists());
    assert!(!data_dir.join("config/flow-catalog.toml").exists());
    // Execution locks travel in the bundled flows' frontmatter.
    let lock = |name: &str| {
        attractor_dsl::read_flow_dir(&bundled.join("flows").join(name))
            .expect("bundled flow loads")
            .execution_lock
            .map(|lock| (lock.scope, lock.key, lock.conflict_policy))
    };
    for name in [
        "math-research/explore-conjecture",
        "math-research/formalize-result",
        "math-research/prove-refute",
    ] {
        assert_eq!(
            lock(name),
            Some(("project".into(), "math-research".into(), "queue".into()))
        );
    }
    assert_eq!(
        lock("software-development/merge-change"),
        Some((
            "project".into(),
            "software-development-integration".into(),
            "queue".into()
        ))
    );
    assert_eq!(lock("software-development/implement-change"), None);
}

#[test]
fn init_seeds_playbooks_updating_unmodified_copies_and_keeping_edits() {
    let env = BTreeMap::new();
    let temp = tempfile::tempdir().expect("tempdir");
    let data_dir = temp.path().join("spark-home");
    let init = || {
        let output = run_with_args_and_env(
            [
                "spark-server",
                "init",
                "--data-dir",
                data_dir.to_str().expect("utf-8 data dir"),
            ],
            &env,
        );
        assert_eq!(output.exit_code, 0, "{}", output.stderr);
    };
    init();
    let playbooks = data_dir.join("playbooks");
    let bug_report = playbooks.join("bug-report.md");
    let packaged = fs::read_to_string(&bug_report).expect("seeded bug report playbook");
    assert!(packaged.starts_with("---\ntitle: Bug report\n"));

    // An unmodified copy of an older packaged version is updated.
    let old_packaged = "---\ntitle: Old\n---\nOld.\n";
    fs::write(&bug_report, old_packaged).expect("old packaged playbook");
    let manifest_path = playbooks.join(".packaged-playbook-hashes.json");
    let mut manifest: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(&manifest_path).expect("playbook manifest"))
            .expect("manifest json");
    manifest.insert(
        "bug-report.md".into(),
        format!("{:x}", Sha256::digest(old_packaged)),
    );
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).expect("manifest");
    init();
    assert_eq!(fs::read_to_string(&bug_report).unwrap(), packaged);

    // An edited copy is kept.
    fs::write(&bug_report, "edited\n").expect("user edit");
    init();
    assert_eq!(fs::read_to_string(&bug_report).unwrap(), "edited\n");

    // A packaged copy whose manifest entry fell behind converges instead of
    // reading as an edit that would block every later update.
    fs::write(&bug_report, &packaged).expect("packaged copy");
    let mut manifest: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest.insert("bug-report.md".into(), "stale".into());
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).expect("manifest");
    init();
    let manifest: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(
        manifest["bug-report.md"],
        format!("{:x}", Sha256::digest(packaged.as_bytes()))
    );
}

#[test]
fn bundled_library_is_rewritten_on_every_start_including_serve() {
    let env = BTreeMap::new();
    let temp = tempfile::tempdir().expect("tempdir");
    let data_dir = temp.path().join("spark-home");
    let data_dir_text = data_dir.to_str().expect("utf-8 data dir");
    let bundled = data_dir.join("bundled");
    let init = run_with_args_and_env(["spark-server", "init", "--data-dir", data_dir_text], &env);
    assert_eq!(init.exit_code, 0, "{}", init.stderr);

    // Local edits, deletions and stray files in the bundled library do not
    // survive a start.
    let edited = bundled.join("flows/examples/simple-linear/plan.md");
    let packaged = fs::read_to_string(&edited).expect("bundled node");
    fs::write(&edited, "edited\n").expect("edit");
    fs::remove_dir_all(bundled.join("flows/examples/parallel-review")).expect("delete flow");
    fs::write(bundled.join("flows/examples/simple-linear/extra.md"), "x").expect("stray");
    let role = bundled.join("roles/adversarial-reviewer.md");
    fs::write(&role, "edited role\n").expect("edit role");
    fs::write(bundled.join("roles/stray.md"), "x").expect("stray role");

    let serve = run_with_args_and_env(
        [
            "spark-server",
            "serve",
            "--data-dir",
            data_dir_text,
            "--host",
            "127.0.0.1",
            "--port",
            "9101",
        ],
        &env,
    );
    assert_eq!(serve.exit_code, 0, "{}", serve.stderr);
    assert_eq!(fs::read_to_string(&edited).expect("rewritten"), packaged);
    assert_eq!(
        list_flow_files(&bundled.join("flows")),
        starter_flow_files()
    );
    assert_ne!(fs::read_to_string(&role).expect("role"), "edited role\n");
    assert!(!bundled.join("roles/stray.md").exists());
}

#[test]
fn serve_validates_settings_and_parses_host_port_without_binding_socket() {
    let env = BTreeMap::new();
    let temp = tempfile::tempdir().expect("tempdir");
    let data_dir = temp.path().join("spark-home");

    let serve = run_with_args_and_env(
        [
            "spark-server",
            "serve",
            "--data-dir",
            data_dir.to_str().expect("utf-8 data dir"),
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

    let flag_config = build_serve_configuration_from_args(
        &[
            "serve".to_string(),
            "--data-dir".to_string(),
            data_dir.to_string_lossy().into_owned(),
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
    let ui_dir = root.join("ui");
    fs::create_dir_all(&ui_dir).expect("ui dir");
    fs::write(ui_dir.join("index.html"), "<!doctype html>\n").expect("index");

    let config = build_serve_configuration_from_args(
        &[
            "serve".to_string(),
            "--data-dir".to_string(),
            data_dir.to_string_lossy().into_owned(),
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
        "stage_index": 0,
        "attempt": 0,
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
    let frames: Vec<Value> = String::from_utf8(output.stdout)
        .expect("utf8 worker frames")
        .lines()
        .map(|line| serde_json::from_str(line).expect("worker frame"))
        .collect();
    assert!(frames[..frames.len() - 1]
        .iter()
        .all(|frame| frame["type"] == json!("event")));
    assert_eq!(
        frames
            .iter()
            .filter(|frame| frame["type"] == json!("result"))
            .count(),
        1
    );
    let frame = frames.last().expect("terminal result");
    assert_eq!(frame["type"], json!("result"));
    assert_eq!(frame["outcome"]["status"], json!("success"));
}

#[test]
fn worker_run_node_process_routes_llm_nodes_to_rust_adapter_boundary() {
    let binary = env!("CARGO_BIN_EXE_spark-server");
    let request = json!({
        "run_id": "run-worker-llm-process",
        "stage_index": 0,
        "attempt": 0,
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
    let frames: Vec<Value> = String::from_utf8(output.stdout)
        .expect("utf8 worker frames")
        .lines()
        .map(|line| serde_json::from_str(line).expect("worker frame"))
        .collect();
    assert!(frames[..frames.len() - 1]
        .iter()
        .all(|frame| frame["type"] == json!("event")));
    assert_eq!(
        frames
            .iter()
            .filter(|frame| frame["type"] == json!("result"))
            .count(),
        1
    );
    let frame = frames.last().expect("terminal result");
    assert_eq!(frame["type"], json!("result"));
    assert_eq!(frame["outcome"]["status"], json!("fail"));
    let reason = frame["outcome"]["failure_reason"]
        .as_str()
        .expect("failure reason");
    assert!(reason.contains("NetworkError"));
    assert!(reason.contains("Provider 'openai' HTTP network failed"));
    assert!(!reason.contains("no HTTP transport is configured"));
}

/// A Claude CLI that never reads stdin, so a prompt larger than the pipe
/// blocks its write. It records its pid in `started`.
#[cfg(unix)]
const STALLED_CLAUDE: &str = r#"#!/usr/bin/env python3
import json, os, time
print(json.dumps({'type':'system','subtype':'init','session_id':'s1','model':'m','tools':[],'capabilities':['interrupt_receipt_v1']}), flush=True)
with open('started','w') as f: f.write(str(os.getpid()))
time.sleep(600)
"#;

/// A Codex app-server that stops reading stdin once its thread starts, so a
/// prompt larger than the pipe blocks the `turn/start` write.
#[cfg(unix)]
const STALLED_CODEX: &str = r#"#!/usr/bin/env python3
import json, os, sys, time
def send(x):
    print(json.dumps(x), flush=True)
while True:
    m = json.loads(sys.stdin.readline())
    method = m.get('method')
    if method == 'initialize': send({'id':m['id'],'result':{}})
    elif method == 'thread/start':
        send({'id':m['id'],'result':{'thread':{'id':'thread'}}})
        break
with open('started','w') as f: f.write(str(os.getpid()))
time.sleep(600)
"#;

/// A worker node whose agent stopped reading stdin mid-prompt still ends on
/// its timeout or run cancel, with the agent process killed.
#[cfg(unix)]
fn assert_worker_stops_stalled_agent(provider: &str, cancel: bool) {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().expect("tempdir");
    let bin = temp.path().join("agent");
    let script = if provider == "codex" {
        STALLED_CODEX
    } else {
        STALLED_CLAUDE
    };
    fs::write(&bin, script).expect("fake agent");
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).expect("executable");
    let run_id = format!("run-worker-{provider}-stalled");
    let store = attractor_runtime::RunStore::for_runs_dir(temp.path().join("runs"));
    let mut run = attractor_runtime::CreateRunRequest::default();
    run.record.run_id = run_id.clone();
    run.record.working_directory = temp.path().to_string_lossy().into();
    run.record.status = "running".into();
    let paths = store.create_run(run).expect("run");
    // Far beyond any pipe buffer.
    let prompt = format!("Work {}", "x".repeat(1 << 20));
    let runtime = if cancel {
        json!({})
    } else {
        json!({"timeout": "1s"})
    };
    let request = json!({
        "run_id": run_id,
        "stage_index": 0,
        "attempt": 0,
        "flow": {
            "schema_version": "1",
            "id": "G",
            "nodes": {
                "task": {
                    "kind": "agent_task",
                    "runtime": runtime,
                    "config": {"kind": "agent_task", "prompt": prompt},
                    "execution": {"llm_provider": provider, "llm_model": "test-model"}
                }
            },
            "edges": []
        },
        "node_id": "task",
        "prompt": prompt,
        "context": {},
        "context_logs": [],
        "logs_root": paths.logs_dir(),
        "working_dir": temp.path(),
        "backend_name": null,
        "model": null,
        "config_dir": null,
        "run_root": {"runs_dir": paths.runs_dir, "project_id": paths.project_id, "root": paths.root}
    });
    let started = std::time::Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_spark-server"))
        .args(["worker", "run-node"])
        .env("SPARK_CLAUDE_CODE_BIN", &bin)
        .env("SPARK_CODEX_APP_SERVER_BIN", &bin)
        .env(
            "ATTRACTOR_CODEX_RUNTIME_ROOT",
            temp.path().join("codex-runtime"),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn worker");
    let mut stdin = child.stdin.take().expect("stdin");
    stdin
        .write_all((request.to_string() + "\n").as_bytes())
        .expect("write request");
    drop(stdin);
    let mut stdout = child.stdout.take().expect("stdout");
    let reader = std::thread::spawn(move || {
        let mut output = String::new();
        stdout.read_to_string(&mut output).map(|_| output)
    });
    let agent_pid = temp.path().join("started");
    let deadline = started + std::time::Duration::from_secs(30);
    let mut canceled = false;
    // 1s timeout plus the 10s interrupt grace, with slack; a hang fails here.
    while child.try_wait().expect("worker status").is_none() {
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            panic!("{provider} worker still running after its stop");
        }
        if cancel && !canceled && agent_pid.exists() {
            let mut record = store.read_run_record(&paths).unwrap().unwrap();
            record.status = "cancel_requested".into();
            store.write_run_record(&paths, &record).unwrap();
            canceled = true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let stdout = reader.join().unwrap().expect("worker stdout");
    let frame: Value =
        serde_json::from_str(stdout.lines().last().expect("result frame")).expect("worker frame");
    assert_eq!(frame["type"], json!("result"), "{stdout}");
    assert_eq!(frame["outcome"]["status"], json!("fail"), "{stdout}");
    let reason = frame["outcome"]["failure_reason"]
        .as_str()
        .unwrap_or_default();
    let expected = if cancel {
        "Run canceled"
    } else {
        "timed out after 1 seconds"
    };
    assert!(reason.contains(expected), "{reason}");
    let pid = fs::read_to_string(&agent_pid).expect("agent started");
    let alive = Command::new("kill")
        .args(["-0", &pid])
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success();
    assert!(!alive, "{provider} agent {pid} still running");
}

#[cfg(unix)]
#[test]
fn worker_run_node_stops_claude_that_stops_reading_stdin_at_timeout() {
    assert_worker_stops_stalled_agent("claude-code", false);
}

#[cfg(unix)]
#[test]
fn worker_run_node_stops_claude_that_stops_reading_stdin_on_cancel() {
    assert_worker_stops_stalled_agent("claude-code", true);
}

#[cfg(unix)]
#[test]
fn worker_run_node_stops_codex_that_stops_reading_stdin_at_timeout() {
    assert_worker_stops_stalled_agent("codex", false);
}

#[cfg(unix)]
#[test]
fn worker_run_node_stops_codex_that_stops_reading_stdin_on_cancel() {
    assert_worker_stops_stalled_agent("codex", true);
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
            Some("md")
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

#[test]
fn core_runtime_paths_are_loaded_again_on_restart_and_keep_override_precedence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().unwrap();
    let home = root.join("home");
    fs::create_dir_all(home.join("config")).unwrap();
    let path = home.join("config/spark.toml");
    let first = root.join("first-runs");
    let second = root.join("second-runs");
    let overrides = SettingsOverrides {
        data_dir: Some(home.clone()),
        ..Default::default()
    };
    let env = BTreeMap::new();
    fs::write(
        &path,
        format!("[runtime]\nruns_dir = '{}'\n", first.display()),
    )
    .unwrap();
    let captured = resolve_server_settings_with_executable_path(&overrides, &env, None).unwrap();
    assert_eq!(captured.runs_dir, first);
    fs::write(
        &path,
        format!(
            "schema_version = 1\n[runtime]\nruns_dir = '{}'\n",
            second.display()
        ),
    )
    .unwrap();
    let restarted = resolve_server_settings_with_executable_path(&overrides, &env, None).unwrap();
    assert_eq!(restarted.runs_dir, second);
    assert_eq!(captured.runs_dir, first);
    assert_eq!(restarted.data_dir, home);
    let overrides = SettingsOverrides {
        runs_dir: Some(first.clone()),
        ..overrides
    };
    assert_eq!(
        resolve_server_settings_with_executable_path(&overrides, &env, None)
            .unwrap()
            .runs_dir,
        first
    );
    fs::write(&path, "[runtime]\nruns_dir = 123\n").unwrap();
    assert!(resolve_server_settings_with_executable_path(&overrides, &env, None).is_err());
    // The retired flows_dir setting is rejected, not silently ignored.
    fs::write(&path, "[runtime]\nflows_dir = '/flows'\n").unwrap();
    assert!(resolve_server_settings_with_executable_path(&overrides, &env, None).is_err());
}

#[test]
fn persisted_connections_apply_on_restart_with_cli_and_environment_precedence() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    fs::create_dir_all(&config).unwrap();
    fs::write(config.join("spark.toml"), "[connections]\nserver_host = '127.0.0.2'\nserver_port = 8123\nclient_api_base_url = 'https://spark.example'\n").unwrap();
    let args: Vec<String> = ["serve", "--data-dir", temp.path().to_str().unwrap()]
        .into_iter()
        .map(str::to_owned)
        .collect();
    let persisted = build_serve_configuration_from_args(&args, &BTreeMap::new()).unwrap();
    assert_eq!(
        (persisted.host.as_str(), persisted.port),
        ("127.0.0.2", 8123)
    );
    let env = BTreeMap::from([
        ("SPARK_HOST".into(), "127.0.0.3".into()),
        ("SPARK_PORT".into(), "9123".into()),
    ]);
    let overridden = build_serve_configuration_from_args(&args, &env).unwrap();
    assert_eq!(
        (overridden.host.as_str(), overridden.port),
        ("127.0.0.3", 9123)
    );
    let mut flags = args.clone();
    flags.extend(
        ["--host", "127.0.0.4", "--port", "0"]
            .into_iter()
            .map(str::to_owned),
    );
    let explicit = build_serve_configuration_from_args(&flags, &env).unwrap();
    assert_eq!((explicit.host.as_str(), explicit.port), ("127.0.0.4", 0));
    assert_eq!(explicit.settings.connections.server_port, Some(0));
    let document =
        spark_storage::settings::read_settings_document(&config.join("spark.toml")).unwrap();
    assert_eq!(
        document.values["connections"]["server_port"].as_integer(),
        Some(8123)
    );
}
