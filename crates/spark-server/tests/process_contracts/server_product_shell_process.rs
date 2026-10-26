use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
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

#[test]
fn serve_binary_recovers_a_recursive_run_tree_after_process_kill() {
    let temp = tempfile::tempdir().expect("tempdir");
    let ui_dir = temp.path().join("ui");
    let project_dir = temp.path().join("project");
    // Serve rewrites the bundled library, so the flows live in the project's org.
    let flows_dir = spark_common::project::org_dir(&project_dir).join("flows");
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
        &json!({"selected_value": "Finish"}),
    );
    assert_eq!(answer.status, 200, "{}", answer.body_text());

    let completed = wait_for_tree(&data_dir, |records| {
        records.len() == 3 && records.iter().all(|record| record["status"] == "completed")
    });
    assert_eq!(lineage_ids(&completed), before_ids);
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
        "schema_version": "1",
        "id": id,
        "nodes": {
            "start": {"kind": "start"},
            node: {
                "kind": "subflow",
                "config": {"kind": "subflow", "flow_ref": child},
                "manager": {"poll_interval": "10ms", "max_cycles": 10000}
            },
            "done": {"kind": "exit"}
        },
        "edges": [{"from": "start", "to": node}, {"from": node, "to": "done"}]
    })
}

fn gate_flow() -> Value {
    json!({
        "schema_version": "1",
        "id": "leaf",
        "nodes": {
            "start": {"kind": "start"},
            "hold": {"kind": "human_gate", "config": {"kind": "human_gate", "prompt": "Continue?"}},
            "done": {"kind": "exit"}
        },
        "edges": [
            {"from": "start", "to": "hold"},
            {"from": "hold", "to": "done", "label": "Finish"}
        ]
    })
}

fn wait_for_tree(data_dir: &Path, ready: impl Fn(&[Value]) -> bool) -> Vec<Value> {
    for _ in 0..200 {
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
        let port = free_tcp_port();
        let data_dir = root.join("spark-home");
        let port_text = port.to_string();
        let data_dir_text = data_dir.to_string_lossy().into_owned();
        let ui_dir_text = ui_dir.to_string_lossy().into_owned();
        let mut child = Command::new(env!("CARGO_BIN_EXE_spark-server"))
            .args([
                "serve",
                "--host",
                "127.0.0.1",
                "--port",
                port_text.as_str(),
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

        for _ in 0..100 {
            if let Some(status) = child.try_wait().expect("poll spark-server") {
                let mut stderr = String::new();
                if let Some(mut stream) = child.stderr.take() {
                    let _ = stream.read_to_string(&mut stderr);
                }
                panic!("spark-server exited early with {status}: {stderr}");
            }
            match try_http_get(port, "/") {
                Ok(response) if response.status == 200 => return Self { child, port },
                _ => thread::sleep(Duration::from_millis(50)),
            }
        }

        let _ = child.kill();
        let _ = child.wait();
        panic!("spark-server did not become ready on port {port}");
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
        .set_read_timeout(Some(Duration::from_secs(2)))
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
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
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

fn free_tcp_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("bind free port")
        .local_addr()
        .expect("local address")
        .port()
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("repo root")
        .to_path_buf()
}
