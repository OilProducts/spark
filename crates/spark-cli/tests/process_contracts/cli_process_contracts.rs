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
            "usage: spark [-h] {convo,run,flow,trigger,mission,role,playbook,settings} ...\n",
            "\n",
            "Spark agent CLI\n",
            "\n",
            "positional arguments:\n",
            "  {convo,run,flow,trigger,mission,role,playbook,settings}\n",
            "    convo               Conversation-scoped artifact commands\n",
            "    run                 Direct execution commands\n",
            "    flow                Flow discovery and validation\n",
            "    trigger             Workspace trigger management\n",
            "    mission             Project mission management\n",
            "    role                Standing role management\n",
            "    playbook            Mission playbook discovery\n",
            "    settings            Read, validate, and save workspace settings\n",
            "\n",
            "options:\n",
            "  -h, --help            show this help message and exit\n",
        )
    );
    assert_eq!(String::from_utf8(output.stderr).expect("stderr utf8"), "");
}

#[test]
fn process_flow_validate_dir_observes_stdout_json() {
    let temp_dir = temp_dir("process-validate");
    let flow_path = temp_dir.join("valid-flow");
    crate::write_flow_json(&flow_path, valid_flow());

    let output = Command::new(spark_bin())
        .args(["flow", "validate", "--dir"])
        .arg(&flow_path)
        .env_clear()
        .env("HOME", temp_dir.to_string_lossy().to_string())
        .output()
        .expect("run spark");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stderr).expect("stderr utf8"), "");
    let payload: Value = serde_json::from_slice(&output.stdout).expect("stdout is validation JSON");
    assert_eq!(payload["name"], "valid-flow");
    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["diagnostics"], Value::Array(Vec::new()));
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn process_flow_validate_dir_resolves_referenced_roles() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    let flow_dir = temp.path().join("reviewed");
    let mut flow = valid_flow();
    flow["nodes"]["task"]["role"] = "persona".into();
    crate::write_flow_json(&flow_dir, flow);
    let validate = || -> Value {
        let output = Command::new(spark_bin())
            .args(["flow", "validate", "--dir"])
            .arg(&flow_dir)
            .env_clear()
            .env("HOME", temp.path())
            .env("SPARK_HOME", &home)
            .output()
            .expect("run spark");
        assert_eq!(output.status.code(), Some(0));
        serde_json::from_slice(&output.stdout).expect("stdout is validation JSON")
    };

    let missing = validate();
    assert_eq!(missing["status"], "validation_error");
    assert_eq!(missing["diagnostics"][0]["rule_id"], "role");
    assert_eq!(missing["diagnostics"][0]["node_id"], "task");

    // Outside a project, roles resolve through the bundled library.
    fs::create_dir_all(home.join("bundled/roles")).expect("roles");
    fs::write(home.join("bundled/roles/persona.md"), "no frontmatter\n").expect("role");
    let malformed = validate();
    assert_eq!(malformed["status"], "validation_error");
    assert_eq!(malformed["diagnostics"][0]["rule_id"], "role");

    fs::write(
        home.join("bundled/roles/persona.md"),
        "---\ntype: Role\n---\nPersona.\n",
    )
    .expect("role");
    assert_eq!(validate()["status"], "ok");
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
        r#"[{"name":"examples/simple","title":"Simple Flow","description":"Small starter."}]"#,
    ));

    let output = Command::new(spark_bin())
        .args(["flow", "list", "--text", "--base-url", base_url.as_str()])
        .env_clear()
        .output()
        .expect("run spark");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout utf8"),
        "examples/simple: Simple Flow\n  Small starter.\n"
    );
    assert_eq!(String::from_utf8(output.stderr).expect("stderr utf8"), "");
    assert_eq!(
        requests
            .recv_timeout(Duration::from_secs(2))
            .expect("request")
            .path,
        "/workspace/api/flows"
    );
}

#[test]
fn process_flow_get_json_prints_files_response() {
    let files = r#"{"name":"software-development/implement-change-request","layer":"bundled","path":"/spark/bundled/flows/software-development/implement-change-request","files":[{"name":"flow.md","content":"---\nid: implement_change_request\n---\n"}]}"#;
    let (base_url, requests) = serve_once(HttpResponse::json(200, files));

    let output = Command::new(spark_bin())
        .args([
            "flow",
            "get",
            "--flow",
            "software-development/implement-change-request",
            "--json",
            "--base-url",
            base_url.as_str(),
        ])
        .env_clear()
        .output()
        .expect("run spark");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stderr).expect("stderr utf8"), "");
    let payload: Value = serde_json::from_slice(&output.stdout).expect("stdout json");
    assert_eq!(payload, serde_json::from_str::<Value>(files).unwrap());
    assert_eq!(
        requests
            .recv_timeout(Duration::from_secs(2))
            .expect("request")
            .path,
        "/workspace/api/flows/software-development%2Fimplement-change-request/files"
    );
}

#[test]
fn process_http_404_maps_to_stderr_json_and_exit_three() {
    let (base_url, _requests) = serve_once(HttpResponse::json(
        404,
        r#"{"detail":"Unknown flow: missing"}"#,
    ));

    let output = Command::new(spark_bin())
        .args([
            "flow",
            "describe",
            "--flow",
            "missing",
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
        "{\"ok\": false, \"status_code\": 404, \"error\": \"Unknown flow: missing\"}\n"
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
            "invalid",
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
            "software-development/implement-change-request",
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
            "flow_name": "software-development/implement-change-request",
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
        r#"[{"id":"trigger-123","name":"Nightly","enabled":true,"protected":false,"source_type":"webhook","action":{"flow_name":"ops/run"},"state":{}}]"#,
    ));

    let output = Command::new(spark_bin())
        .args(["trigger", "list", "--text", "--base-url", base_url.as_str()])
        .env_clear()
        .output()
        .expect("run spark");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout utf8"),
        "trigger-123: Nightly [webhook] -> ops/run\n  enabled=True protected=False\n"
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
            "--expected-revision",
            "revision-1",
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

fn valid_flow() -> Value {
    serde_json::json!({
        "schema_version": "1",
        "id": "workflow",
        "title": "Workflow",
        "nodes": {
            "start": {"kind": "start"},
            "task": {
                "kind": "agent_task",
                "label": "Task",
                "config": {"kind": "agent_task", "prompt": "Do work"},
            },
            "done": {"kind": "exit"},
        },
        "edges": [{"from": "start", "to": "task"}, {"from": "task", "to": "done"}],
    })
}

#[test]
fn installed_cli_reads_persisted_target_and_preserves_explicit_target_precedence() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("package/bin/spark");
    fs::create_dir_all(bin.parent().unwrap()).unwrap();
    fs::copy(spark_bin(), &bin).unwrap();
    let home = temp.path().join("home");
    fs::create_dir_all(home.join("config")).unwrap();
    for source in ["persisted", "environment", "flag"] {
        let (target, requests) = serve_once(HttpResponse::json(200, r#"{"preferences":{}}"#));
        let persisted = if source == "persisted" {
            &target
        } else {
            "http://127.0.0.1:1"
        };
        fs::write(
            home.join("config/spark.toml"),
            format!("[connections]\nclient_api_base_url = '{persisted}'\n"),
        )
        .unwrap();
        let mut command = Command::new(&bin);
        command
            .args(["settings", "get"])
            .env_clear()
            .env("SPARK_HOME", &home);
        if source == "environment" {
            command.env("SPARK_API_BASE_URL", format!(" {target} "));
        }
        if source == "flag" {
            command.env("SPARK_API_BASE_URL", "http://127.0.0.1:1");
            command.args(["--base-url", target.as_str()]);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            requests.recv_timeout(Duration::from_secs(2)).unwrap().path,
            "/workspace/api/settings"
        );
    }
    // The source binary must still require an explicit target even with a saved one.
    let rejected = Command::new(spark_bin())
        .args(["settings", "get"])
        .env_clear()
        .env("SPARK_HOME", &home)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("Refusing to use default API target")
    );
}

#[test]
fn process_flow_validate_dir_resolves_roles_through_the_flows_org() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    let project = temp.path().join("project");
    let org = project.join(".spark/agent-org");
    let flow_dir = org.join("flows/team/work");
    crate::write_flow_json(
        &flow_dir,
        serde_json::json!({
            "id": "work",
            "nodes": {
                "start": {"kind": "start"},
                "task": {"kind": "agent_task", "role": "reviewer", "prompt": "Review."},
                "done": {"kind": "exit"},
            },
            "edges": [{"from": "start", "to": "task"}, {"from": "task", "to": "done"}],
        }),
    );
    let spark = |args: &[&str]| {
        Command::new(spark_bin())
            .args(args)
            .env_clear()
            .env("HOME", temp.path())
            .env("SPARK_HOME", &home)
            .output()
            .expect("run spark")
    };
    let flow_dir = flow_dir.to_string_lossy().to_string();

    let status = |output: &std::process::Output| {
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        serde_json::from_slice::<Value>(&output.stdout).expect("validation JSON")["status"].clone()
    };
    let missing = spark(&["flow", "validate", "--dir", &flow_dir]);
    assert_eq!(status(&missing), "validation_error");
    assert!(String::from_utf8_lossy(&missing.stdout).contains("unknown role 'reviewer'"));

    fs::create_dir_all(org.join("roles")).expect("roles");
    fs::write(
        org.join("roles/reviewer.md"),
        "---\ntype: Role\n---\nYou review.\n",
    )
    .expect("role");
    let valid = spark(&["flow", "validate", "--dir", &flow_dir]);
    assert_eq!(status(&valid), "ok");

    for removed in ["accept", "promote"] {
        let output = spark(&["flow", removed, "flows/team/work/flow.md"]);
        assert_ne!(output.status.code(), Some(0), "{output:?}");
    }
}
