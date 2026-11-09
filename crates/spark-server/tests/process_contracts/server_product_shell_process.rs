use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

#[test]
fn serve_binary_process_exposes_product_shell_and_mount_boundaries() {
    let temp = tempfile::tempdir().expect("tempdir");
    let ui_dir = temp.path().join("ui");
    let assets_dir = ui_dir.join("assets");
    fs::create_dir_all(&assets_dir).expect("assets");
    fs::write(
        ui_dir.join("index.html"),
        "<!doctype html><div id=\"root\">process ui</div>\n",
    )
    .expect("index");
    fs::write(
        assets_dir.join("compat-app.js"),
        "window.__sparkProcessAsset = true;\n",
    )
    .expect("script");
    fs::write(
        assets_dir.join("spark-app-icon.png"),
        b"spark-process-favicon\n",
    )
    .expect("favicon");

    let server = RunningServer::start(temp.path(), &ui_dir);

    let index = http_get(server.port, "/");
    assert_eq!(index.status, 200);
    assert_eq!(
        index.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(
        index.body_text(),
        "<!doctype html><div id=\"root\">process ui</div>\n"
    );

    let script = http_get(server.port, "/assets/compat-app.js");
    assert_eq!(script.status, 200);
    assert_eq!(
        script.header("content-type"),
        Some("text/javascript; charset=utf-8")
    );
    assert_eq!(script.body_text(), "window.__sparkProcessAsset = true;\n");

    let favicon = http_get(server.port, "/favicon.ico");
    assert_eq!(favicon.status, 200);
    assert_eq!(favicon.header("content-type"), Some("image/png"));
    assert_eq!(favicon.body, b"spark-process-favicon\n");

    let workspace_base = http_get(server.port, "/workspace");
    let expected_workspace_location = format!("http://127.0.0.1:{}/workspace/", server.port);
    assert_eq!(workspace_base.status, 307);
    assert_eq!(
        workspace_base.header("location"),
        Some(expected_workspace_location.as_str())
    );
    assert!(workspace_base.header("content-type").is_none());
    assert!(workspace_base.body.is_empty());

    let workspace_slash = http_get(server.port, "/workspace/");
    assert_eq!(workspace_slash.status, 404);
    assert_eq!(
        workspace_slash.header("content-type"),
        Some("application/json")
    );
    assert_eq!(workspace_slash.body_text(), r#"{"detail":"Not Found"}"#);

    let attractor_base = http_get(server.port, "/attractor");
    let expected_attractor_location = format!("http://127.0.0.1:{}/attractor/", server.port);
    assert_eq!(attractor_base.status, 307);
    assert_eq!(
        attractor_base.header("location"),
        Some(expected_attractor_location.as_str())
    );
    assert!(attractor_base.header("content-type").is_none());
    assert!(attractor_base.body.is_empty());

    let attractor_slash = http_get(server.port, "/attractor/");
    assert_eq!(attractor_slash.status, 404);
    assert_eq!(
        attractor_slash.header("content-type"),
        Some("application/json")
    );
    assert_eq!(attractor_slash.body_text(), r#"{"detail":"Not Found"}"#);

    let api_missing = http_get(server.port, "/workspace/api/missing");
    assert_eq!(api_missing.status, 404);
    assert_eq!(api_missing.header("content-type"), Some("application/json"));
    assert_eq!(api_missing.body_text(), r#"{"detail":"Not Found"}"#);
    assert!(!api_missing.body_text().contains("process ui"));
}

/// Help prints and exits: it never writes to the Spark home it inherited,
/// and never reaches a command's own side effects.
#[test]
fn subcommand_help_writes_nothing() {
    for (args, usage) in [
        (vec!["serve", "--help"], "usage: spark-server serve"),
        (vec!["serve", "-h"], "usage: spark-server serve"),
        (vec!["init", "--help"], "usage: spark-server init"),
        (
            vec!["service", "install", "--help"],
            "usage: spark-server service",
        ),
    ] {
        let temp = tempfile::tempdir().expect("tempdir");
        let spark_home = temp.path().join("spark-home");
        let output = Command::new(env!("CARGO_BIN_EXE_spark-server"))
            .args(&args)
            .env("HOME", temp.path())
            .env("SPARK_HOME", &spark_home)
            .current_dir(temp.path())
            .output()
            .expect("run spark-server");
        assert_eq!(output.status.code(), Some(0), "{args:?}");
        let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
        assert!(stdout.starts_with(usage), "{args:?}: {stdout}");
        let written = fs::read_dir(temp.path())
            .expect("list temp")
            .map(|entry| entry.expect("entry").path())
            .collect::<Vec<_>>();
        assert_eq!(written, Vec::<PathBuf>::new(), "{args:?} wrote files");
    }
}

/// One server per Spark home: a second process on the same home fails
/// before serving, an independent home is unaffected, and the home opens
/// again once its server is gone.
#[test]
fn a_second_serve_process_on_one_home_exits_before_serving() {
    let temp = tempfile::tempdir().expect("tempdir");
    let other = tempfile::tempdir().expect("other home");
    let ui_dir = temp.path().join("ui");
    fs::create_dir_all(&ui_dir).expect("ui dir");
    fs::write(ui_dir.join("index.html"), "<!doctype html>\n").expect("ui index");
    let first = RunningServer::start(temp.path(), &ui_dir);

    let mut second = Command::new(env!("CARGO_BIN_EXE_spark-server"))
        .env("HOME", temp.path())
        .args([
            "serve",
            "--host",
            "127.0.0.1",
            "--port",
            "0",
            "--data-dir",
            &temp.path().join("spark-home").to_string_lossy(),
            "--ui-dir",
            &ui_dir.to_string_lossy(),
        ])
        .current_dir(repo_root())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn second server");
    let status = (0..3600)
        .find_map(|_| {
            let status = second.try_wait().expect("poll second server");
            if status.is_none() {
                thread::sleep(Duration::from_millis(50));
            }
            status
        })
        .unwrap_or_else(|| {
            let _ = second.kill();
            panic!("the second server kept running");
        });
    let mut stderr = String::new();
    second
        .stderr
        .take()
        .expect("stderr")
        .read_to_string(&mut stderr)
        .expect("read stderr");
    assert!(!status.success());
    assert!(
        stderr.contains("Unable to initialize Spark server")
            && stderr.contains("Another Spark server"),
        "{stderr}"
    );
    // It exited without ever announcing an address to serve on.
    let mut stdout = String::new();
    second
        .stdout
        .take()
        .expect("stdout")
        .read_to_string(&mut stdout)
        .expect("read stdout");
    assert_eq!(stdout, "");
    assert_eq!(http_get(first.port, "/").status, 200);

    let independent = RunningServer::start(other.path(), &ui_dir);
    assert_eq!(http_get(independent.port, "/").status, 200);

    drop(first);
    let restarted = RunningServer::start(temp.path(), &ui_dir);
    assert_eq!(http_get(restarted.port, "/").status, 200);
}

#[test]
fn serve_binary_recovers_a_recursive_run_tree_after_process_kill() {
    let temp = tempfile::tempdir().expect("tempdir");
    let ui_dir = temp.path().join("ui");
    let project_dir = temp.path().join("project");
    // Serve rewrites the bundled library, so the flows live in the project's org.
    let flows_dir = spark_common::project::definitions_dir(&project_dir).join("flows");
    fs::create_dir_all(&ui_dir).expect("ui dir");
    fs::create_dir_all(&project_dir).expect("project dir");
    fs::write(ui_dir.join("index.html"), "<!doctype html>\n").expect("ui index");
    write_flow(&flows_dir, "root", nested_flow("root", "branch", "child"));
    write_flow(&flows_dir, "child", nested_flow("child", "branch", "leaf"));
    write_flow(&flows_dir, "leaf", gate_flow());

    let mut server = RunningServer::start(temp.path(), &ui_dir);
    let launch = http_json(
        server.port,
        "POST",
        "/attractor/pipelines",
        &json!({
            "run_id": "process-tree-root",
            "flow_name": "root",
            "working_directory": project_dir,
            "project_path": project_dir,
        }),
    );
    assert_eq!(launch.status, 200, "{}", launch.body_text());

    let data_dir = temp.path().join("spark-home");
    let before = wait_for_tree(&data_dir, |records| {
        records.len() == 3 && records.iter().any(|record| record["status"] == "waiting")
    });
    let before_ids = lineage_ids(&before);
    assert_eq!(before_ids.len(), 3, "expected root, child, and grandchild");

    server.stop();
    server = RunningServer::start(temp.path(), &ui_dir);

    let recovered = wait_for_tree(&data_dir, |records| {
        records.len() == 3 && records.iter().any(|record| record["status"] == "waiting")
    });
    assert_eq!(
        lineage_ids(&recovered),
        before_ids,
        "recovery must be in place"
    );
    let leaf_id = recovered
        .iter()
        .find(|record| {
            record["parent_run_id"].as_str().is_some_and(|parent| {
                recovered.iter().any(|candidate| {
                    candidate["run_id"] == parent && candidate["parent_run_id"].is_string()
                })
            })
        })
        .and_then(|record| record["run_id"].as_str())
        .expect("grandchild run id");
    let questions = http_get(
        server.port,
        &format!("/attractor/pipelines/{leaf_id}/questions"),
    );
    let questions: Value = serde_json::from_slice(&questions.body).expect("questions json");
    let question_id = questions["questions"][0]["question_id"]
        .as_str()
        .expect("pending question");
    let answer = http_json(
        server.port,
        "POST",
        &format!("/attractor/pipelines/{leaf_id}/questions/{question_id}/answer"),
        &json!({"selected_value": "finish"}),
    );
    assert_eq!(answer.status, 200, "{}", answer.body_text());

    let completed = wait_for_tree(&data_dir, |records| {
        records.len() == 3 && records.iter().all(|record| record["status"] == "completed")
    });
    assert_eq!(lineage_ids(&completed), before_ids);
    assert!(
        completed
            .iter()
            .all(|record| record["outcome"] == "success"
                && record["outcome_reason_code"] == "completed"),
        "{completed:?}"
    );
}

#[test]
fn serve_gives_codex_and_claude_code_agents_homes_under_its_data_directory() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let ui_dir = root.join("ui");
    let home = root.join("home");
    let dumps = root.join("agent-env");
    for dir in [&ui_dir, &home, &dumps] {
        fs::create_dir_all(dir).expect("dir");
    }
    fs::write(ui_dir.join("index.html"), "<!doctype html>\n").expect("ui index");
    // Each fake agent records the environment Spark launched it with, then exits.
    let fake = |name: &str| {
        let path = root.join(name);
        fs::write(
            &path,
            format!("#!/bin/sh\nenv > '{}'\n", dumps.join(name).display()),
        )
        .expect("fake agent");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    };
    let (codex, claude) = (fake("codex"), fake("claude"));
    let server = RunningServer::start_with(&root, &ui_dir, |command| {
        command
            .env("HOME", &home)
            .env_remove("SPARK_HOME")
            .env_remove("SPARK_CLAUDE_CODE_CONFIG_DIR")
            .env_remove("ATTRACTOR_CODEX_RUNTIME_ROOT")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_DATA_HOME")
            .env("SPARK_CODEX_APP_SERVER_BIN", &codex)
            .env("SPARK_CLAUDE_CODE_BIN", &claude);
    });

    // Model discovery launches both agents.
    http_get(server.port, "/workspace/api/projects/chat-models");

    let data_dir = root.join("spark-home");
    let agent_env = |name: &str| -> BTreeMap<String, String> {
        fs::read_to_string(dumps.join(name))
            .unwrap_or_else(|error| panic!("{name} was not launched: {error}"))
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    };
    let codex_env = agent_env("codex");
    let data_dir_text = data_dir.to_string_lossy().into_owned();
    assert_eq!(codex_env.get("SPARK_HOME"), Some(&data_dir_text));
    assert_eq!(
        codex_env.get("HOME").map(PathBuf::from),
        Some(data_dir.join("runtime/codex"))
    );
    let claude_env = agent_env("claude");
    assert_eq!(claude_env.get("SPARK_HOME"), Some(&data_dir_text));
    assert_eq!(
        claude_env.get("CLAUDE_CONFIG_DIR").map(PathBuf::from),
        Some(data_dir.join("runtime/claude"))
    );
}

fn write_flow(flows_dir: &Path, name: &str, flow: Value) {
    attractor_dsl::write_flow_dir(
        &flows_dir.join(name),
        &attractor_dsl::validated_flow(serde_json::from_value(flow).expect("flow JSON"))
            .expect("valid flow"),
    )
    .expect("write flow");
}

fn nested_flow(id: &str, node: &str, child: &str) -> Value {
    json!({
        "schema_version": "2",
        "id": id,
        "nodes": {
            "start": {
                "kind": "start",
                "config": {"kind": "start"},
                "routes": {"next": {"to": node, "description": "Enter the first workflow step."}}
            },
            node: {
                "kind": "subflow",
                "config": {"kind": "subflow", "flow_ref": child, "results": {"completed": "child_completed"}},
                "manager": {"poll_interval": "10ms", "max_cycles": 10000},
                "routes": {"child_completed": {"to": "done", "description": "The child flow completed."}}
            },
            "done": {
                "kind": "exit",
                "config": {"kind": "exit", "result": {"name": "completed", "outcome": "success"}}
            }
        }
    })
}

fn gate_flow() -> Value {
    json!({
        "schema_version": "2",
        "id": "leaf",
        "nodes": {
            "start": {
                "kind": "start",
                "config": {"kind": "start"},
                "routes": {"next": {"to": "hold", "description": "Enter the first workflow step."}}
            },
            "hold": {
                "kind": "human_gate",
                "config": {"kind": "human_gate", "prompt": "Continue?"},
                "routes": {"finish": {"to": "done", "label": "Finish", "description": "The operator chose to finish."}}
            },
            "done": {
                "kind": "exit",
                "config": {"kind": "exit", "result": {"name": "completed", "outcome": "success"}}
            }
        }
    })
}

fn wait_for_tree(data_dir: &Path, ready: impl Fn(&[Value]) -> bool) -> Vec<Value> {
    for _ in 0..3600 {
        let records = run_records(data_dir);
        if ready(&records) {
            return records;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!(
        "run tree did not reach expected state: {:?}",
        run_records(data_dir)
    );
}

fn run_records(data_dir: &Path) -> Vec<Value> {
    let runs = data_dir.join("attractor/runs");
    let Ok(projects) = fs::read_dir(runs) else {
        return Vec::new();
    };
    projects
        .flatten()
        .flat_map(|project| fs::read_dir(project.path()).into_iter().flatten().flatten())
        .filter_map(|run| fs::read(run.path().join("run.json")).ok())
        .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
        .collect()
}

fn lineage_ids(records: &[Value]) -> BTreeMap<String, Option<String>> {
    records
        .iter()
        .map(|record| {
            (
                record["run_id"].as_str().expect("run id").to_string(),
                record["parent_run_id"].as_str().map(str::to_string),
            )
        })
        .collect()
}

struct RunningServer {
    child: Child,
    port: u16,
}

impl RunningServer {
    fn start(root: &Path, ui_dir: &Path) -> Self {
        Self::start_with(root, ui_dir, |_| {})
    }

    fn start_with(root: &Path, ui_dir: &Path, configure: impl FnOnce(&mut Command)) -> Self {
        let data_dir = root.join("spark-home");
        let data_dir_text = data_dir.to_string_lossy().into_owned();
        let ui_dir_text = ui_dir.to_string_lossy().into_owned();
        let mut command = Command::new(env!("CARGO_BIN_EXE_spark-server"));
        // The Home project is HOME; keep it in the test root.
        command.env("HOME", root);
        configure(&mut command);
        let mut child = command
            .args([
                "serve",
                "--host",
                "127.0.0.1",
                "--port",
                "0",
                "--data-dir",
                data_dir_text.as_str(),
                "--ui-dir",
                ui_dir_text.as_str(),
            ])
            .current_dir(repo_root())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn spark-server serve");

        // The server binds a free port and prints it once it can serve.
        let stdout = BufReader::new(child.stdout.take().expect("server stdout"));
        let (address, printed) = mpsc::channel();
        thread::spawn(move || {
            for line in stdout.lines().map_while(Result::ok) {
                if let Some(port) = line.rsplit(':').next().and_then(|port| port.parse().ok()) {
                    let _ = address.send(port);
                }
            }
        });
        match printed.recv_timeout(Duration::from_secs(180)) {
            Ok(port) => Self { child, port },
            Err(error) => {
                let _ = child.kill();
                let status = child.wait().expect("reap spark-server");
                let mut stderr = String::new();
                if let Some(mut stream) = child.stderr.take() {
                    let _ = stream.read_to_string(&mut stderr);
                }
                panic!("spark-server did not start ({error}, {status}): {stderr}");
            }
        }
    }

    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Debug)]
struct HttpResponse {
    status: u16,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

impl HttpResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    fn body_text(&self) -> String {
        String::from_utf8(self.body.clone()).expect("utf-8 response body")
    }
}

fn http_get(port: u16, path: &str) -> HttpResponse {
    try_http_get(port, path).expect("http response")
}

fn http_json(port: u16, method: &str, path: &str, body: &Value) -> HttpResponse {
    let body = serde_json::to_vec(body).expect("json body");
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .expect("read timeout");
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .expect("request headers");
    stream.write_all(&body).expect("request body");
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).expect("response");
    parse_http_response(&bytes).expect("http response")
}

fn try_http_get(port: u16, path: &str) -> std::io::Result<HttpResponse> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(60)))?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )?;

    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes)?;
    parse_http_response(&bytes)
}

fn parse_http_response(bytes: &[u8]) -> std::io::Result<HttpResponse> {
    let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "HTTP response did not include a header terminator",
        ));
    };
    let header_text = String::from_utf8_lossy(&bytes[..header_end]);
    let mut lines = header_text.lines();
    let status = lines
        .next()
        .expect("status line")
        .split_whitespace()
        .nth(1)
        .expect("status code")
        .parse::<u16>()
        .expect("numeric status");
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect::<BTreeMap<_, _>>();
    let raw_body = &bytes[header_end + 4..];
    let body = if headers
        .get("transfer-encoding")
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"))
    {
        decode_chunked_body(raw_body)
    } else {
        raw_body.to_vec()
    };

    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

fn decode_chunked_body(raw_body: &[u8]) -> Vec<u8> {
    let mut decoded = Vec::new();
    let mut cursor = 0;
    while cursor < raw_body.len() {
        let Some(line_end) = raw_body[cursor..]
            .windows(2)
            .position(|window| window == b"\r\n")
            .map(|offset| cursor + offset)
        else {
            break;
        };
        let size_text = String::from_utf8_lossy(&raw_body[cursor..line_end]);
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
            .expect("chunk size");
        cursor = line_end + 2;
        if size == 0 {
            break;
        }
        decoded.extend_from_slice(&raw_body[cursor..cursor + size]);
        cursor += size + 2;
    }
    decoded
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("repo root")
        .to_path_buf()
}
