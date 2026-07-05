use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

fn spark_bin() -> &'static str {
    env!("CARGO_BIN_EXE_spark")
}

#[test]
fn process_help_matches_top_level_contract() {
    let output = Command::new(spark_bin())
        .arg("--help")
        .output()
        .expect("run spark");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout utf8"),
        concat!(
            "usage: spark [-h] {convo,run,flow,trigger} ...\n",
            "\n",
            "Spark agent CLI\n",
            "\n",
            "positional arguments:\n",
            "  {convo,run,flow,trigger}\n",
            "    convo               Conversation-scoped artifact commands\n",
            "    run                 Direct execution commands\n",
            "    flow                Flow discovery and validation\n",
            "    trigger             Workspace trigger management\n",
            "\n",
            "options:\n",
            "  -h, --help            show this help message and exit\n",
        )
    );
    assert_eq!(String::from_utf8(output.stderr).expect("stderr utf8"), "");
}

#[test]
fn process_flow_validate_file_observes_stdout_json() {
    let temp_dir = temp_dir("process-validate");
    let flow_path = temp_dir.join("valid-flow.dot");
    fs::write(&flow_path, valid_flow_source()).expect("write flow");

    let output = Command::new(spark_bin())
        .args(["flow", "validate", "--file"])
        .arg(&flow_path)
        .env_clear()
        .env("HOME", temp_dir.to_string_lossy().to_string())
        .output()
        .expect("run spark");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stderr).expect("stderr utf8"), "");
    let payload: Value = serde_json::from_slice(&output.stdout).expect("stdout is validation JSON");
    assert_eq!(payload["name"], "valid-flow.dot");
    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["diagnostics"], Value::Array(Vec::new()));
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn process_source_checkout_default_target_guard_remains_exit_one() {
    let output = Command::new(spark_bin())
        .args(["flow", "list"])
        .env_clear()
        .output()
        .expect("run spark");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8(output.stdout).expect("stdout utf8"), "");
    let stderr = String::from_utf8(output.stderr).expect("stderr utf8");
    assert!(stderr.starts_with(
        "{\"ok\": false, \"error\": \"Refusing to use default API target http://127.0.0.1:8000 from a source checkout"
    ));
    assert!(stderr.contains("before `spark flow list`"));
}

#[test]
fn process_flow_list_text_executes_http_and_renders_rows() {
    let (base_url, requests) = serve_once(HttpResponse::json(
        200,
        r#"[{"name":"examples/simple.dot","title":"Simple Flow","description":"Small starter."}]"#,
    ));

    let output = Command::new(spark_bin())
        .args(["flow", "list", "--text", "--base-url", base_url.as_str()])
        .env_clear()
        .output()
        .expect("run spark");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout utf8"),
        "examples/simple.dot: Simple Flow\n  Small starter.\n"
    );
    assert_eq!(String::from_utf8(output.stderr).expect("stderr utf8"), "");
    assert_eq!(
        requests
            .recv_timeout(Duration::from_secs(2))
            .expect("request")
            .path,
        "/workspace/api/flows?surface=agent"
    );
}

#[test]
fn process_flow_get_wraps_raw_response_as_json() {
    let (base_url, requests) = serve_once(HttpResponse::text(200, "digraph G {}\n"));

    let output = Command::new(spark_bin())
        .args([
            "flow",
            "get",
            "--flow",
            "software-development/implement-change-request.dot",
            "--base-url",
            base_url.as_str(),
        ])
        .env_clear()
        .output()
        .expect("run spark");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stderr).expect("stderr utf8"), "");
    let payload: Value = serde_json::from_slice(&output.stdout).expect("stdout json");
    assert_eq!(
        payload["name"],
        "software-development/implement-change-request.dot"
    );
    assert_eq!(payload["content"], "digraph G {}\n");
    assert_eq!(
        requests
            .recv_timeout(Duration::from_secs(2))
            .expect("request")
            .path,
        "/workspace/api/flows/software-development%2Fimplement-change-request.dot/raw?surface=agent"
    );
}

#[test]
fn process_http_404_maps_to_stderr_json_and_exit_three() {
    let (base_url, _requests) = serve_once(HttpResponse::json(
        404,
        r#"{"detail":"Unknown flow: missing.dot"}"#,
    ));

    let output = Command::new(spark_bin())
        .args([
            "flow",
            "describe",
            "--flow",
            "missing.dot",
            "--base-url",
            base_url.as_str(),
        ])
        .env_clear()
        .output()
        .expect("run spark");

    assert_eq!(output.status.code(), Some(3));
    assert_eq!(String::from_utf8(output.stdout).expect("stdout utf8"), "");
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr utf8"),
        "{\"ok\": false, \"status_code\": 404, \"error\": \"Unknown flow: missing.dot\"}\n"
    );
}

#[test]
fn process_http_nested_detail_error_maps_to_stderr_json() {
    let (base_url, _requests) = serve_once(HttpResponse::json(
        400,
        r#"{"detail":{"error":"Nested detail process error"}}"#,
    ));

    let output = Command::new(spark_bin())
        .args([
            "flow",
            "describe",
            "--flow",
            "invalid.dot",
            "--base-url",
            base_url.as_str(),
        ])
        .env_clear()
        .output()
        .expect("run spark");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8(output.stdout).expect("stdout utf8"), "");
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr utf8"),
        "{\"ok\": false, \"status_code\": 400, \"error\": \"Nested detail process error\"}\n"
    );
}

#[test]
fn process_convo_run_request_posts_json_payload() {
    let (base_url, requests) = serve_once(HttpResponse::json(
        200,
        r#"{"ok":true,"flow_run_request_id":"flow-run-request-process"}"#,
    ));

    let output = Command::new(spark_bin())
        .args([
            "convo",
            "run-request",
            "--conversation",
            "amber-otter",
            "--flow",
            "software-development/implement-change-request.dot",
            "--summary",
            "Process request",
            "--goal",
            "Ship it.",
            "--base-url",
            base_url.as_str(),
        ])
        .env_clear()
        .output()
        .expect("run spark");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stderr).expect("stderr utf8"), "");
    let payload: Value = serde_json::from_slice(&output.stdout).expect("stdout json");
    assert_eq!(payload["flow_run_request_id"], "flow-run-request-process");
    let request = requests
        .recv_timeout(Duration::from_secs(2))
        .expect("request");
    assert_eq!(request.method, "POST");
    assert_eq!(
        request.path,
        "/workspace/api/conversations/by-handle/amber-otter/flow-run-requests"
    );
    assert_eq!(
        serde_json::from_str::<Value>(&request.body).expect("request json"),
        serde_json::json!({
            "flow_name": "software-development/implement-change-request.dot",
            "goal": "Ship it.",
            "summary": "Process request"
        })
    );
}

#[test]
fn process_trigger_stdin_payload_errors_before_server_dispatch() {
    let mut child = Command::new(spark_bin())
        .args([
            "trigger",
            "create",
            "--json",
            "-",
            "--base-url",
            "http://127.0.0.1:8010",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn spark");

    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"[]")
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait spark");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8(output.stdout).expect("stdout utf8"), "");
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr utf8"),
        "{\"ok\": false, \"error\": \"Trigger payload must be a JSON object.\"}\n"
    );
}

#[test]
fn process_trigger_list_text_executes_http_and_renders_rows() {
    let (base_url, requests) = serve_once(HttpResponse::json(
        200,
        r#"[{"id":"trigger-123","name":"Nightly","enabled":true,"protected":false,"source_type":"webhook","action":{"flow_name":"ops/run.dot"},"state":{}}]"#,
    ));

    let output = Command::new(spark_bin())
        .args(["trigger", "list", "--text", "--base-url", base_url.as_str()])
        .env_clear()
        .output()
        .expect("run spark");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout utf8"),
        "trigger-123: Nightly [webhook] -> ops/run.dot\n  enabled=True protected=False\n"
    );
    assert_eq!(String::from_utf8(output.stderr).expect("stderr utf8"), "");
    let request = requests
        .recv_timeout(Duration::from_secs(2))
        .expect("request");
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, "/workspace/api/triggers");
}

#[test]
fn process_trigger_http_error_maps_to_stderr_json() {
    let (base_url, _requests) = serve_once(HttpResponse::json(
        400,
        r#"{"detail":"Protected triggers cannot be deleted."}"#,
    ));

    let output = Command::new(spark_bin())
        .args([
            "trigger",
            "delete",
            "--id",
            "protected",
            "--base-url",
            base_url.as_str(),
        ])
        .env_clear()
        .output()
        .expect("run spark");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8(output.stdout).expect("stdout utf8"), "");
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr utf8"),
        "{\"ok\": false, \"status_code\": 400, \"error\": \"Protected triggers cannot be deleted.\"}\n"
    );
}

#[derive(Debug, Clone)]
struct HttpResponse {
    status: u16,
    content_type: &'static str,
    body: String,
}

impl HttpResponse {
    fn json(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: body.into(),
        }
    }

    fn text(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8",
            body: body.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CapturedRequest {
    method: String,
    path: String,
    body: String,
}

fn serve_once(response: HttpResponse) -> (String, mpsc::Receiver<CapturedRequest>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let Ok((mut stream, _addr)) = listener.accept() else {
            return;
        };
        let request = read_http_request(&mut stream);
        let _ = sender.send(request);
        let reason = match response.status {
            200 => "OK",
            404 => "Not Found",
            _ => "OK",
        };
        let wire_response = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.status,
            reason,
            response.content_type,
            response.body.as_bytes().len(),
            response.body
        );
        let _ = stream.write_all(wire_response.as_bytes());
    });
    (base_url, receiver)
}

fn read_http_request(stream: &mut TcpStream) -> CapturedRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set read timeout");
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let count = match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(_) => break,
        };
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(header_end) = header_end(&bytes) {
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let content_length = content_length(&headers);
            if bytes.len() >= header_end + 4 + content_length {
                break;
            }
        }
    }
    let header_end = header_end(&bytes).unwrap_or(bytes.len());
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let mut request_line = headers
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace();
    let method = request_line.next().unwrap_or_default().to_string();
    let path = request_line.next().unwrap_or_default().to_string();
    let body_start = (header_end + 4).min(bytes.len());
    let body = String::from_utf8_lossy(&bytes[body_start..]).into_owned();
    CapturedRequest { method, path, body }
}

fn header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length(headers: &str) -> usize {
    headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0)
}

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("spark-cli-{label}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&path).expect("create temp dir");
    path
}

fn valid_flow_source() -> &'static str {
    "digraph Workflow {\n  start [shape=Mdiamond];\n  task [shape=box, prompt=\"Do work\"];\n  done [shape=Msquare];\n  start -> task;\n  task -> done;\n}\n"
}
