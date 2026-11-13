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
fn process_flow_validate_dir_exit_code_follows_the_report() {
    let temp = tempfile::tempdir().expect("tempdir");
    let valid_dir = temp.path().join("valid");
    crate::write_flow_json(&valid_dir, valid_flow());
    let invalid_dir = temp.path().join("invalid");
    let mut invalid = valid_flow();
    invalid["nodes"]
        .as_object_mut()
        .expect("nodes")
        .remove("start");
    crate::write_flow_json(&invalid_dir, invalid);

    let validate = |dir: &PathBuf, extra: &[&str]| {
        Command::new(spark_bin())
            .args(["flow", "validate", "--dir"])
            .arg(dir)
            .args(extra)
            .env_clear()
            .env("HOME", temp.path())
            .output()
            .expect("run spark")
    };

    let output = validate(&invalid_dir, &[]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let payload: Value = serde_json::from_slice(&output.stdout).expect("validation JSON");
    assert_eq!(payload["name"], "invalid");
    assert_eq!(payload["status"], "validation_error");
    assert_eq!(payload["diagnostics"][0]["severity"], "error");

    let output = validate(&invalid_dir, &["--text"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let text = String::from_utf8(output.stdout).expect("stdout utf8");
    assert!(text.contains("Status: validation_error\n"), "{text}");
    assert!(text.contains("- ERROR start_node: "), "{text}");

    let output = validate(&valid_dir, &[]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let payload: Value = serde_json::from_slice(&output.stdout).expect("validation JSON");
    assert_eq!(payload["status"], "ok");

    let output = validate(&valid_dir, &["--text"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let text = String::from_utf8(output.stdout).expect("stdout utf8");
    assert!(text.contains("Status: ok\n"), "{text}");

    let output = validate(&temp.path().join("missing"), &[]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("Flow directory not found"));
}

/// `--dir` cannot report a warning: its payload marks every diagnostic an error
/// and the loader keeps only error rules. A flow that trips only a warning rule
/// therefore validates clean; the stub test below covers a reported warning.
#[test]
fn process_flow_validate_dir_warning_only_flow_exits_zero() {
    let temp = tempfile::tempdir().expect("tempdir");
    let flow_dir = temp.path().join("warning-only");
    let mut flow = valid_flow();
    let task = &mut flow["nodes"]["task"];
    task.as_object_mut().expect("task").remove("label");
    task["config"]["prompt"] = Value::from("");
    crate::write_flow_json(&flow_dir, flow);

    let graph = attractor_dsl::read_flow_dir(&flow_dir)
        .expect("flow loads")
        .to_runtime_dot_graph();
    let rules = attractor_dsl::validate_graph(&graph)
        .into_iter()
        .map(|d| (d.rule_id, d.severity))
        .collect::<Vec<_>>();
    assert_eq!(
        rules,
        [(
            "prompt_on_llm_nodes".to_string(),
            attractor_dsl::DiagnosticSeverity::Warning
        )]
    );

    for (extra, status) in [
        (&[][..], "\"status\": \"ok\""),
        (&["--text"][..], "Status: ok\n"),
    ] {
        let output = Command::new(spark_bin())
            .args(["flow", "validate", "--dir"])
            .arg(&flow_dir)
            .args(extra)
            .env_clear()
            .env("HOME", temp.path())
            .output()
            .expect("run spark");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains(status),
            "{output:?}"
        );
    }
}

#[test]
fn process_flow_validate_through_the_api_exit_code_follows_the_report() {
    let error = r#"{"severity":"error","rule_id":"start_node","message":"No start.","line":0}"#;
    let warning = r#"{"severity":"warning","rule_id":"style","message":"Odd style.","line":0}"#;
    let cases = [
        (
            format!(
                r#"{{"status":"validation_error","diagnostics":[{error}],"errors":[{error}]}}"#
            ),
            1,
            "- ERROR start_node: No start.",
        ),
        (
            r#"{"status":"validation_error","diagnostics":[],"errors":[]}"#.to_string(),
            1,
            "Status: validation_error",
        ),
        (
            format!(r#"{{"status":"ok","diagnostics":[{warning},{error}],"errors":[]}}"#),
            1,
            "- ERROR start_node: No start.",
        ),
        (
            format!(r#"{{"status":"ok","diagnostics":[{warning}],"errors":[]}}"#),
            0,
            "- WARNING style: Odd style.",
        ),
        (
            r#"{"status":"ok","diagnostics":[],"errors":[]}"#.to_string(),
            0,
            "Status: ok",
        ),
    ];
    let validate = |body: &str, text: bool| {
        let (base_url, _requests) = serve_once(HttpResponse::json(200, body));
        let mut command = Command::new(spark_bin());
        command.args(["flow", "validate", "team/work", "--base-url"]);
        command.arg(base_url).env_clear();
        if text {
            command.arg("--text");
        }
        command.output().expect("run spark")
    };
    for (body, exit_code, line) in &cases {
        let output = validate(body, false);
        assert_eq!(output.status.code(), Some(*exit_code), "{body}: {output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).expect("validation JSON"),
            serde_json::from_str::<Value>(body).expect("body JSON"),
        );

        let output = validate(body, true);
        assert_eq!(output.status.code(), Some(*exit_code), "{body}: {output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let text = String::from_utf8(output.stdout).expect("stdout utf8");
        assert!(text.contains(line), "{text}");
    }

    let (base_url, _requests) =
        serve_once(HttpResponse::json(404, r#"{"detail":"Unknown flow."}"#));
    let output = Command::new(spark_bin())
        .args(["flow", "validate", "team/work", "--base-url"])
        .arg(base_url)
        .env_clear()
        .output()
        .expect("run spark");
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");

    // A flow named `validate` is a name, not a route segment.
    let (base_url, requests) = serve_once(HttpResponse::json(
        200,
        r#"{"status":"ok","diagnostics":[],"errors":[]}"#,
    ));
    let output = Command::new(spark_bin())
        .args(["flow", "validate", "validate", "--base-url"])
        .arg(base_url)
        .env_clear()
        .output()
        .expect("run spark");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        requests
            .recv_timeout(Duration::from_secs(2))
            .expect("validate request")
            .path,
        "/workspace/api/flows/validate/validate"
    );
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
        let payload: Value =
            serde_json::from_slice(&output.stdout).expect("stdout is validation JSON");
        let expected = if payload["status"] == "ok" { 0 } else { 1 };
        assert_eq!(output.status.code(), Some(expected));
        payload
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
            "software-development/implement-change-request",
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
        r#"{"detail":"Unknown pipeline: missing"}"#,
    ));

    let output = Command::new(spark_bin())
        .args(["run", "get", "missing", "--base-url", base_url.as_str()])
        .env_clear()
        .output()
        .expect("run spark");

    assert_eq!(output.status.code(), Some(3));
    assert_eq!(String::from_utf8(output.stdout).expect("stdout utf8"), "");
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr utf8"),
        format!(
            "{{\"ok\": false, \"error\": \"Unknown pipeline: missing\", \"exit_code\": 3, \"status_code\": 404, \"target\": \"{base_url}\", \"target_source\": \"flag\"}}\n"
        )
    );
}

#[test]
fn process_http_nested_detail_error_maps_to_stderr_json() {
    let (base_url, _requests) = serve_once(HttpResponse::json(
        400,
        r#"{"detail":{"error":"Nested detail process error"}}"#,
    ));

    let output = Command::new(spark_bin())
        .args(["run", "get", "invalid", "--base-url", base_url.as_str()])
        .env_clear()
        .output()
        .expect("run spark");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8(output.stdout).expect("stdout utf8"), "");
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr utf8"),
        format!(
            "{{\"ok\": false, \"error\": \"Nested detail process error\", \"exit_code\": 1, \"status_code\": 400, \"target\": \"{base_url}\", \"target_source\": \"flag\"}}\n"
        )
    );
}

#[test]
fn process_run_start_with_conversation_posts_json_payload() {
    let (base_url, requests) = serve_once(HttpResponse::json(
        200,
        r#"{"ok":true,"flow_run_request_id":"flow-run-request-process"}"#,
    ));

    let output = Command::new(spark_bin())
        .args([
            "run",
            "start",
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
            "--project",
            "/repo",
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
        "{\"ok\": false, \"error\": \"--json must be a JSON object.\", \"exit_code\": 1}\n"
    );
}

#[test]
fn process_trigger_list_text_executes_http_and_renders_rows() {
    let (base_url, requests) = serve_once(HttpResponse::json(
        200,
        r#"[{"id":"trigger-123","project_path":"/repo","name":"Nightly","enabled":true,"protected":false,"source_type":"webhook","action":{"flow_name":"ops/run"},"state":{}}]"#,
    ));

    let output = Command::new(spark_bin())
        .args(["trigger", "list", "--text", "--base-url", base_url.as_str()])
        .env_clear()
        .output()
        .expect("run spark");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout utf8"),
        "trigger-123: Nightly [webhook] -> ops/run\n  project=/repo enabled=true protected=false\n"
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
            "protected",
            "--project",
            "/repo",
            "--expected-revision",
            "revision-1",
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
        format!(
            "{{\"ok\": false, \"error\": \"Protected triggers cannot be deleted.\", \"exit_code\": 1, \"status_code\": 400, \"target\": \"{base_url}\", \"target_source\": \"flag\"}}\n"
        )
    );
}

#[test]
fn process_api_token_is_sent_as_a_bearer_only_when_set() {
    let sse = HttpResponse {
        status: 200,
        content_type: "text/event-stream",
        body: "data: {\"payload\":{\"sequence\":1,\"type\":\"log\"}}\n\n".to_string(),
    };
    for token in [Some("secret"), Some("  "), None] {
        for (args, response) in [
            (
                &["delegation", "get", "d-1"][..],
                HttpResponse::json(200, r#"{"id":"d-1"}"#),
            ),
            (&["run", "events", "run-1"][..], sse.clone()),
        ] {
            let (base_url, requests) = serve_once(response);
            let mut command = Command::new(spark_bin());
            command
                .args(args)
                .args(["--base-url", base_url.as_str()])
                .env_clear();
            if let Some(token) = token {
                command.env("SPARK_API_TOKEN", token);
            }
            let output = command.output().expect("run spark");
            assert_eq!(output.status.code(), Some(0), "{args:?}: {output:?}");
            let request = requests
                .recv_timeout(Duration::from_secs(2))
                .expect("request");
            let expected = (token == Some("secret")).then(|| "Bearer secret".to_string());
            assert_eq!(request.authorization, expected, "{args:?} {token:?}");
        }
    }
}

#[test]
fn process_delegation_wait_polls_until_sealed_or_its_timeout() {
    let (base_url, requests) = serve_sequence(vec![
        HttpResponse::json(
            200,
            r#"{"phase":"accepted","timed_out":true,"result":null}"#,
        ),
        HttpResponse::json(
            200,
            r#"{"phase":"sealed","timed_out":false,"result":{"status":"completed"}}"#,
        ),
    ]);
    let output = Command::new(spark_bin())
        .args(["delegation", "wait", "d-1", "--base-url", base_url.as_str()])
        .env_clear()
        .output()
        .expect("run spark");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).expect("record JSON"),
        serde_json::json!({"phase":"sealed","timed_out":false,"result":{"status":"completed"}})
    );
    for _ in 0..2 {
        let request = requests.recv_timeout(Duration::from_secs(2)).expect("poll");
        assert_eq!(request.method, "GET");
        assert_eq!(
            request.path,
            "/workspace/api/delegations/d-1/wait?timeout_ms=25000"
        );
    }

    // Its own timeout ends the wait with the still-open record, not an error.
    let (base_url, requests) = serve_once(HttpResponse::json(
        200,
        r#"{"phase":"accepted","timed_out":true,"result":null}"#,
    ));
    let output = Command::new(spark_bin())
        .args(["delegation", "wait", "d-1", "--timeout", "0"])
        .args(["--base-url", base_url.as_str()])
        .env_clear()
        .output()
        .expect("run spark");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let record: Value = serde_json::from_slice(&output.stdout).expect("record JSON");
    assert_eq!(record["timed_out"], true);
    assert_eq!(record["phase"], "accepted");
    assert_eq!(
        requests.recv_timeout(Duration::from_secs(2)).unwrap().path,
        "/workspace/api/delegations/d-1/wait?timeout_ms=0"
    );
}

#[test]
fn process_delegation_answer_with_a_lost_response_is_reported_unknown_and_not_resent() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
    // Reads the answer, then closes without responding.
    let server = thread::spawn(move || {
        let (mut stream, _addr) = listener.accept().expect("accept");
        let request = read_http_request(&mut stream);
        drop(stream);
        (listener, request)
    });

    let output = Command::new(spark_bin())
        .args(["delegation", "answer", "d-1", "--question", "q1"])
        .args(["--option", "yes", "--base-url", base_url.as_str()])
        .env_clear()
        .output()
        .expect("run spark");

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).expect("stderr utf8");
    assert!(stderr.contains("outcome is unknown"), "{stderr}");
    assert!(stderr.contains("it was not resent"), "{stderr}");
    let (listener, request) = server.join().expect("server");
    assert_eq!(request.method, "POST");
    assert_eq!(
        request.path,
        "/workspace/api/delegations/d-1/questions/q1/answer"
    );
    assert_eq!(
        serde_json::from_str::<Value>(&request.body).expect("request JSON"),
        serde_json::json!({"selected_value": "yes"})
    );
    assert_no_pending_connection(&listener);
}

#[test]
fn process_delegation_answer_sends_several_answers_by_question_id() {
    let (base_url, requests) = serve_once(HttpResponse::json(200, r#"{"answered":true}"#));
    let output = Command::new(spark_bin())
        .args(["delegation", "answer", "d-1", "--question", "q1"])
        .args(["--answers", r#"{"choice":"A","notes":"None"}"#])
        .args(["--base-url", base_url.as_str()])
        .env_clear()
        .output()
        .expect("run spark");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let request = requests
        .recv_timeout(Duration::from_secs(2))
        .expect("answer");
    assert_eq!(
        serde_json::from_str::<Value>(&request.body).expect("request JSON"),
        serde_json::json!({"answers": {"choice": "A", "notes": "None"}})
    );
}

/// `GET /workspace/api/delegations/{id}` as the server serializes it: the
/// target's latest view, with its open requests, sits in `last_state`.
fn delegation_record(open_requests: &[&str]) -> String {
    let questions: Vec<Value> = open_requests
        .iter()
        .map(|id| {
            serde_json::json!({
                "question_id": id,
                "questions": [{"id": "choice", "question": "Which one?"}],
            })
        })
        .collect();
    serde_json::to_string(&spark_storage::environment::DelegationOut {
        delegation_id: "d-1".to_string(),
        environment_id: "env-b".to_string(),
        project_path: "/repo".to_string(),
        kind: "conversation".to_string(),
        request: serde_json::json!({"message": "Do it."}),
        request_sha256: String::new(),
        caller: "cli".to_string(),
        request_key: "key-1".to_string(),
        owner_conversation_id: None,
        phase: "accepted".to_string(),
        target_id: Some("conversation-b".to_string()),
        last_state: Some(serde_json::json!({
            "delegation_id": "d-1",
            "phase": "launched",
            "target_kind": "conversation",
            "target_id": "conversation-b",
            "status": "waiting",
            "questions": questions,
        })),
        result: None,
        paused: None,
        cancel_requested_at: None,
        cancel_acknowledged_at: None,
        created_at: "2026-11-13T00:00:00Z".to_string(),
        updated_at: "2026-11-13T00:00:00Z".to_string(),
    })
    .expect("record JSON")
}

#[test]
fn process_delegation_answer_without_a_question_answers_the_one_open_request() {
    let (base_url, requests) = serve_sequence(vec![
        HttpResponse::json(200, delegation_record(&["request-1"])),
        HttpResponse::json(200, r#"{"answered":true}"#),
    ]);
    let output = Command::new(spark_bin())
        .args([
            "delegation",
            "answer",
            "d-1",
            "--answers",
            r#"{"choice":"A"}"#,
        ])
        .args(["--base-url", base_url.as_str()])
        .env_clear()
        .output()
        .expect("run spark");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).expect("stdout JSON"),
        serde_json::json!({"answered": true})
    );
    let read = requests.recv_timeout(Duration::from_secs(2)).expect("read");
    assert_eq!(
        (read.method.as_str(), read.path.as_str()),
        ("GET", "/workspace/api/delegations/d-1")
    );
    let answer = requests
        .recv_timeout(Duration::from_secs(2))
        .expect("answer");
    assert_eq!(
        (answer.method.as_str(), answer.path.as_str()),
        (
            "POST",
            "/workspace/api/delegations/d-1/questions/request-1/answer"
        )
    );
    assert_eq!(
        serde_json::from_str::<Value>(&answer.body).expect("request JSON"),
        serde_json::json!({"answers": {"choice": "A"}})
    );
    assert!(requests.try_recv().is_err(), "the answer was sent once");

    // Two open requests are ambiguous: nothing is sent.
    let (base_url, requests) = serve_once(HttpResponse::json(
        200,
        delegation_record(&["request-1", "request-2"]),
    ));
    let output = Command::new(spark_bin())
        .args([
            "delegation",
            "answer",
            "d-1",
            "--answers",
            r#"{"choice":"A"}"#,
        ])
        .args(["--base-url", base_url.as_str()])
        .env_clear()
        .output()
        .expect("run spark");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(
        stderr_error(&output),
        "The delegation has 2 open questions (request-1, request-2); pass --question with one of them, from spark delegation get."
    );
    assert_eq!(
        requests
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .method,
        "GET"
    );
    assert!(requests.try_recv().is_err(), "no answer was sent");
}

#[test]
fn process_delegation_start_requires_a_request_key_before_any_request() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let base_url = format!("http://{}", listener.local_addr().expect("local addr"));

    let output = Command::new(spark_bin())
        .args(["delegation", "start", "--environment", "env-1"])
        .args(["--target-project", "/repo", "--kind", "conversation"])
        .args(["--message", "Do it.", "--base-url", base_url.as_str()])
        .env_clear()
        .output()
        .expect("run spark");

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let error: Value = serde_json::from_slice(&output.stderr).expect("stderr JSON");
    assert_eq!(error["exit_code"], 2);
    let message = error["error"].as_str().expect("error");
    assert!(
        message.contains("the following required arguments were not provided"),
        "{message}"
    );
    assert!(message.contains("--request-key <KEY>"), "{message}");
    assert_no_pending_connection(&listener);
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
    authorization: Option<String>,
}

fn serve_once(response: HttpResponse) -> (String, mpsc::Receiver<CapturedRequest>) {
    serve_sequence(vec![response])
}

/// Answers one connection per response, in order.
fn serve_sequence(responses: Vec<HttpResponse>) -> (String, mpsc::Receiver<CapturedRequest>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for response in responses {
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
        }
    });
    (base_url, receiver)
}

/// Fails if a client connected to `listener` and was never accepted.
fn assert_no_pending_connection(listener: &TcpListener) {
    listener.set_nonblocking(true).expect("nonblocking");
    assert_eq!(
        listener.accept().map(|_| ()).map_err(|error| error.kind()),
        Err(std::io::ErrorKind::WouldBlock),
        "the CLI sent another request"
    );
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
    let authorization = header_value(&headers, "authorization").map(str::to_string);
    CapturedRequest {
        method,
        path,
        body,
        authorization,
    }
}

fn header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn header_value<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

fn content_length(headers: &str) -> usize {
    header_value(headers, "content-length")
        .and_then(|value| value.parse().ok())
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
        "schema_version": "2",
        "id": "workflow",
        "title": "Workflow",
        "nodes": {
            "start": {
                "kind": "start",
                "config": {"kind": "start"},
                "routes": {"next": {"to": "task", "description": "Enter the first workflow step."}},
            },
            "task": {
                "kind": "agent_task",
                "label": "Task",
                "config": {"kind": "agent_task", "prompt": "Do work"},
                "contracts": {"response": "decision_envelope"},
                "routes": {"done": {"to": "done", "description": "The task is complete."}},
            },
            "done": {
                "kind": "exit",
                "config": {"kind": "exit", "result": {"name": "completed", "outcome": "success"}},
            },
        },
    })
}

#[test]
fn installed_cli_reads_persisted_target_and_preserves_explicit_target_precedence() {
    // Hard-link beside the build so no writable copy exists for a concurrent
    // spawn to inherit (exec would hit ETXTBSY), even with TMPDIR on another mount.
    let temp = tempfile::tempdir_in(std::path::Path::new(spark_bin()).parent().unwrap()).unwrap();
    let bin = temp.path().join("package/bin/spark");
    fs::create_dir_all(bin.parent().unwrap()).unwrap();
    fs::hard_link(spark_bin(), &bin).unwrap();
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
    let org = project.join(".spark/definitions");
    let flow_dir = org.join("flows/team/work");
    crate::write_flow_json(
        &flow_dir,
        serde_json::json!({
            "schema_version": "2",
            "id": "work",
            "nodes": {
                "start": {
                    "kind": "start",
                    "config": {"kind": "start"},
                    "routes": {"next": {"to": "task", "description": "Enter the first workflow step."}},
                },
                "task": {
                    "kind": "agent_task",
                    "role": "reviewer",
                    "config": {"kind": "agent_task", "prompt": "Review."},
                    "contracts": {"response": "decision_envelope"},
                    "routes": {"done": {"to": "done", "description": "The review is complete."}},
                },
                "done": {
                    "kind": "exit",
                    "config": {"kind": "exit", "result": {"name": "completed", "outcome": "success"}},
                },
            },
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
        serde_json::from_slice::<Value>(&output.stdout).expect("validation JSON")["status"].clone()
    };
    let missing = spark(&["flow", "validate", "--dir", &flow_dir]);
    assert_eq!(missing.status.code(), Some(1), "{missing:?}");
    assert_eq!(status(&missing), "validation_error");
    assert!(String::from_utf8_lossy(&missing.stdout).contains("unknown role 'reviewer'"));

    fs::create_dir_all(org.join("roles")).expect("roles");
    fs::write(
        org.join("roles/reviewer.md"),
        "---\ntype: Role\n---\nYou review.\n",
    )
    .expect("role");
    let valid = spark(&["flow", "validate", "--dir", &flow_dir]);
    assert_eq!(valid.status.code(), Some(0), "{valid:?}");
    assert_eq!(status(&valid), "ok");

    for removed in ["accept", "promote"] {
        let output = spark(&["flow", removed, "flows/team/work/flow.md"]);
        assert_ne!(output.status.code(), Some(0), "{output:?}");
    }
}

#[test]
fn process_flow_validate_dir_reports_why_a_flow_failed_to_load() {
    let temp = tempfile::tempdir().expect("tempdir");
    let validate = |dir: &std::path::Path, extra: &[&str]| {
        let output = Command::new(spark_bin())
            .args(["flow", "validate", "--dir"])
            .arg(dir)
            .args(extra)
            .env_clear()
            .env("HOME", temp.path())
            .output()
            .expect("run spark");
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        String::from_utf8(output.stdout).expect("stdout utf8")
    };
    // Schema 1 nodes list their routes, which do not parse as schema 2.
    let write = |name: &str, header: &str| {
        let dir = temp.path().join(name);
        fs::create_dir_all(&dir).expect("flow dir");
        fs::write(dir.join("flow.md"), header).expect("flow.md");
        fs::write(
            dir.join("start.md"),
            "---\ntype: start\nroutes:\n- to: done\n  condition: outcome=success\n---\n",
        )
        .expect("start.md");
        dir
    };

    for (name, header, found) in [
        (
            "quoted",
            "---\ntype: Flow\nid: old\nschema_version: '1'\n---\n",
            "found '1'",
        ),
        (
            "integer",
            "---\ntype: Flow\nid: old\nschema_version: 1\n---\n",
            "found '1'",
        ),
        ("missing", "---\ntype: Flow\nid: old\n---\n", "found ''"),
        (
            "unknown-header-field",
            "---\ntype: Flow\nid: old\nschema_version: '1'\nretired: true\n---\n",
            "found '1'",
        ),
    ] {
        let dir = write(name, header);
        let dir_text = dir.display().to_string();

        let payload: Value = serde_json::from_str(&validate(&dir, &[])).expect("validation JSON");
        assert_eq!(payload["status"], "validation_error", "{payload}");
        let error = payload["error"].as_str().expect("error");
        assert!(error.contains(&dir_text), "{payload}");
        assert!(error.contains(found), "{payload}");
        assert!(error.contains("convert this flow's routes"), "{payload}");
        assert_eq!(payload["diagnostics"][0]["rule_id"], "schema_version");
        assert_eq!(payload["errors"], payload["diagnostics"]);

        let text = validate(&dir, &["--text"]);
        let path = dir.canonicalize().expect("canonical").display().to_string();
        assert!(text.contains(&format!("Path: {path}")), "{text}");
        assert!(text.contains("convert this flow's routes"), "{text}");
        assert!(text.contains("Diagnostics: 1"), "{text}");
    }

    // A failure carrying only detail still surfaces it, as a diagnostic too.
    let broken = write(
        "broken",
        "---\ntype: Flow\nid: new\nschema_version: '2'\n---\n",
    );
    let broken_text = broken.display().to_string();
    let payload: Value = serde_json::from_str(&validate(&broken, &[])).expect("validation JSON");
    let message = payload["diagnostics"][0]["message"]
        .as_str()
        .expect("message");
    assert!(message.contains(&broken_text), "{payload}");
    assert!(message.contains("start.md"), "{payload}");
    assert_eq!(payload["error"], payload["diagnostics"][0]["message"]);
    let text = validate(&broken, &["--text"]);
    assert!(text.contains("Diagnostics: 1"), "{text}");
    assert!(text.contains("start.md"), "{text}");
}

/// Runs spark with `stdin` on its standard input.
fn spark_with_stdin(args: &[&str], stdin: &str) -> std::process::Output {
    let mut child = Command::new(spark_bin())
        .args(args)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn spark");
    // A command that never reads stdin may exit before this write lands.
    let _ = child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin.as_bytes());
    child.wait_with_output().expect("wait spark")
}

fn stderr_error(output: &std::process::Output) -> String {
    let error: Value = serde_json::from_slice(&output.stderr).expect("stderr json");
    error["error"].as_str().expect("error text").to_string()
}

#[test]
fn process_run_events_and_run_retry_take_the_same_run_id() {
    let (base_url, requests) = serve_sequence(vec![
        HttpResponse {
            status: 200,
            content_type: "text/event-stream",
            body: String::new(),
        },
        HttpResponse::json(200, r#"{"run_id":"run-2"}"#),
    ]);
    for leaf in ["events", "retry"] {
        let output = spark_with_stdin(&["run", leaf, "run-1", "--base-url", &base_url], "");
        assert_eq!(output.status.code(), Some(0), "{leaf}: {output:?}");
    }
    let paths = (0..2)
        .map(|_| {
            requests
                .recv_timeout(Duration::from_secs(2))
                .expect("request")
                .path
        })
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        [
            "/workspace/api/live/events?run_id=run-1",
            "/workspace/api/runs/run-1/retry"
        ]
    );
}

#[test]
fn process_run_list_get_and_cancel_call_their_routes() {
    let (base_url, requests) = serve_sequence(vec![
        HttpResponse::json(200, r#"{"runs":[]}"#),
        HttpResponse::json(200, r#"{"run_id":"run-1","status":"running"}"#),
        HttpResponse::json(200, r#"{"status":"cancel_requested"}"#),
    ]);
    for (command, method, path) in [
        (
            &["run", "list", "--project", "/repo"][..],
            "GET",
            "/attractor/runs?project_path=%2Frepo",
        ),
        (
            &["run", "get", "run-1"],
            "GET",
            "/attractor/pipelines/run-1",
        ),
        (
            &["run", "cancel", "run-1"],
            "POST",
            "/attractor/pipelines/run-1/cancel",
        ),
    ] {
        let output = spark_with_stdin(&[command, &["--base-url", &base_url]].concat(), "");
        assert_eq!(output.status.code(), Some(0), "{command:?}: {output:?}");
        let request = requests
            .recv_timeout(Duration::from_secs(2))
            .expect("request");
        assert_eq!(
            (request.method.as_str(), request.path.as_str()),
            (method, path)
        );
    }
}

#[test]
fn process_unknown_short_flow_name_suggests_the_namespaced_one_from_the_conversations_project() {
    let (base_url, requests) = serve_sequence(vec![
        HttpResponse::json(404, r#"{"detail":"Unknown flow: design-change"}"#),
        HttpResponse::json(
            200,
            r#"{"rows":[{"kind":"conversation","payload":{"handle":"other-otter","project_path":"/other"}},{"kind":"conversation","payload":{"handle":"amber-otter","project_path":"/repo"}}]}"#,
        ),
        HttpResponse::json(
            200,
            r#"[{"name":"team/design-change"},{"name":"examples/simple-linear"}]"#,
        ),
    ]);
    let output = spark_with_stdin(
        &[
            "run",
            "start",
            "--conversation",
            "amber-otter",
            "--flow",
            "design-change",
            "--summary",
            "Design it",
            "--base-url",
            &base_url,
        ],
        "",
    );

    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert_eq!(
        stderr_error(&output),
        "Unknown flow: design-change; did you mean team/design-change?"
    );
    let paths = (0..3)
        .map(|_| {
            requests
                .recv_timeout(Duration::from_secs(2))
                .expect("request")
                .path
        })
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        [
            "/workspace/api/conversations/by-handle/amber-otter/flow-run-requests",
            "/workspace/api/projects/conversations",
            "/workspace/api/flows?project_path=%2Frepo",
        ]
    );
}

#[test]
fn process_retry_conflict_names_run_continue() {
    let (base_url, _requests) = serve_once(HttpResponse::json(
        409,
        r#"{"detail":"Retry requires a failed pipeline"}"#,
    ));
    let output = spark_with_stdin(&["run", "retry", "run-1", "--base-url", &base_url], "");

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(
        stderr_error(&output),
        "Retry requires a failed pipeline; use spark run continue run-1 --from <node> to resume a run that did not fail"
    );
}

#[test]
fn process_run_start_from_a_conversation_and_a_project_explains_the_conflict() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
    let output = spark_with_stdin(
        &[
            "run",
            "start",
            "--conversation",
            "amber-otter",
            "--project",
            "/repo",
            "--flow",
            "examples/simple-linear",
            "--summary",
            "Inspect",
            "--base-url",
            &base_url,
        ],
        "",
    );

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(
        stderr_error(&output),
        "spark run start takes --conversation or --project, not both: a run started from a conversation runs in that conversation's project, under its mission or role budget. Drop --project."
    );
    assert_no_pending_connection(&listener);
}

#[test]
fn process_flow_get_inputs_prints_only_the_declared_inputs() {
    let flow_md = "---\ntype: Flow\nid: design\ntitle: Design\nschema_version: '2'\ninputs:\n- key: context.request.problem\n  label: Problem\n  description: What to design.\n  required: true\n---\n\nDesign it.\n";
    let (base_url, requests) = serve_once(HttpResponse::json(
        200,
        serde_json::json!({
            "name": "team/design",
            "layer": "bundled",
            "path": "/flows/team/design",
            "files": [
                {"name": "flow.md", "content": flow_md},
                {"name": "nodes/start.md", "content": "---\nkind: start\n---\n"},
            ],
        })
        .to_string(),
    ));
    let output = spark_with_stdin(
        &[
            "flow",
            "get",
            "team/design",
            "--inputs",
            "--base-url",
            &base_url,
        ],
        "",
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).expect("stdout json"),
        serde_json::json!([{
            "key": "context.request.problem",
            "label": "Problem",
            "type": "string",
            "description": "What to design.",
            "required": true,
        }])
    );
    let request = requests
        .recv_timeout(Duration::from_secs(2))
        .expect("request");
    assert_eq!(request.path, "/workspace/api/flows/team%2Fdesign/files");
}

#[test]
fn process_json_flags_take_inline_json_a_file_or_stdin() {
    let input = r#"{"k":{"v":1}}"#;
    let file = temp_dir("json-input").join("input.json");
    fs::write(&file, input).expect("write input");
    let file = file.to_str().expect("utf8 path");
    let launch = [
        "run",
        "start",
        "--project",
        "/repo",
        "--flow",
        "f",
        "--summary",
        "s",
    ];
    let cases: [(&[&str], &str, &str); 4] = [
        (&launch, "--context", "/launch_context/k"),
        (&launch, "--node-overrides", "/node_overrides/k"),
        (
            &["delegation", "answer", "d-1", "--question", "q1"],
            "--answers",
            "/answers/k",
        ),
        (&["trigger", "create", "--project", "/repo"], "--json", "/k"),
    ];
    for (command, flag, pointer) in cases {
        for value in [input, file, "-"] {
            let (base_url, requests) = serve_once(HttpResponse::json(200, "{}"));
            let output = spark_with_stdin(
                &[command, &[flag, value, "--base-url", &base_url]].concat(),
                input,
            );
            assert_eq!(output.status.code(), Some(0), "{flag} {value}: {output:?}");
            let request = requests
                .recv_timeout(Duration::from_secs(2))
                .expect("request");
            let body: Value = serde_json::from_str(&request.body).expect("request JSON");
            assert_eq!(
                body.pointer(pointer),
                Some(&serde_json::json!({"v": 1})),
                "{flag} {value}: {body}"
            );
        }
    }
}

/// A run's server for `run events --until-done`. The run reads as ended
/// after `live_reads` reads; its journal holds `entries` (sequence, raw
/// type, or `resync_required` for a lag notice); its event stream replays the
/// entries after the requested cursor, then stays open when `hold_open`. Sends each request's path.
fn serve_run(
    entries: &'static [(u64, &'static str)],
    live_reads: usize,
    hold_open: bool,
) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
    let (sender, paths) = mpsc::channel();
    thread::spawn(move || {
        let mut run_reads = 0;
        let mut open_streams = Vec::new();
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let path = read_http_request(&mut stream).path;
            let _ = sender.send(path.clone());
            if path.starts_with("/workspace/api/live/events") {
                let after = path
                    .split("run_sequence=")
                    .nth(1)
                    .map_or(0, |after| after.parse::<u64>().expect("cursor"));
                let _ =
                    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n");
                for (sequence, raw_type) in entries.iter().filter(|(sequence, _)| *sequence > after)
                {
                    let event = if *raw_type == "resync_required" {
                        serde_json::json!({"type": "resync_required", "resource": {"kind": "run", "id": "run-1"}})
                    } else {
                        serde_json::json!({
                            "type": "run.journal_entry",
                            "cursor": {"kind": "run_sequence", "value": sequence},
                            "payload": {"sequence": sequence, "raw_type": raw_type},
                        })
                    };
                    let _ = write!(stream, "data: {event}\n\n");
                }
                if hold_open {
                    open_streams.push(stream);
                }
                continue;
            }
            let body = if path.ends_with("/journal?limit=1") {
                serde_json::json!({"newest_sequence": entries.last().map(|(sequence, _)| sequence)})
            } else {
                run_reads += 1;
                let ended = (run_reads > live_reads).then_some("2026-11-13T00:00:00Z");
                serde_json::json!({"run_id": "run-1", "ended_at": ended})
            }
            .to_string();
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (base_url, paths)
}

/// `spark run events run-1 --until-done`, failing if it still runs after
/// ten seconds.
fn run_events_until_done(base_url: &str, extra: &[&str]) -> std::process::Output {
    let mut child = Command::new(spark_bin())
        .args([
            "run",
            "events",
            "run-1",
            "--until-done",
            "--base-url",
            base_url,
        ])
        .args(extra)
        .env_clear()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn spark");
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while child.try_wait().expect("poll spark").is_none() {
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            panic!("spark run events --until-done kept reading after the run ended");
        }
        thread::sleep(Duration::from_millis(20));
    }
    child.wait_with_output().expect("wait spark")
}

const RUN: &str = "/attractor/pipelines/run-1";
const JOURNAL: &str = "/attractor/pipelines/run-1/journal?limit=1";
const EVENTS: &str = "/workspace/api/live/events?run_id=run-1";
const RETRIED: &[(u64, &str)] = &[
    (1, "StageCompleted"),
    (2, "PipelineFailed"),
    (3, "StageCompleted"),
    (4, "PipelineCompleted"),
];

#[test]
fn process_run_events_until_done_stops_at_a_live_runs_end_on_an_open_stream() {
    let (base_url, paths) = serve_run(&[(1, "StageCompleted"), (2, "PipelineCompleted")], 1, true);
    let output = run_events_until_done(&base_url, &[]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    assert_eq!(stdout.lines().count(), 2, "{stdout}");
    assert_eq!(
        paths.try_iter().collect::<Vec<_>>(),
        [RUN, EVENTS, RUN, JOURNAL]
    );
}

#[test]
fn process_run_events_until_done_replays_a_retried_run_past_its_earlier_failure() {
    let (base_url, paths) = serve_run(RETRIED, 0, true);
    let output = run_events_until_done(&base_url, &[]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    let raw_types = stdout
        .lines()
        .map(|line| {
            let event: Value = serde_json::from_str(line).expect("event JSON");
            event["payload"]["raw_type"].as_str().unwrap().to_string()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        raw_types,
        RETRIED
            .iter()
            .map(|(_, raw_type)| *raw_type)
            .collect::<Vec<_>>()
    );
    assert_eq!(paths.try_iter().collect::<Vec<_>>(), [RUN, JOURNAL, EVENTS]);
}

#[test]
fn process_run_events_until_done_after_an_ended_runs_last_cursor_exits_at_once() {
    let (base_url, paths) = serve_run(RETRIED, 0, true);
    let output = run_events_until_done(&base_url, &["--after", "4"]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_eq!(paths.try_iter().collect::<Vec<_>>(), [RUN, JOURNAL]);
}

#[test]
fn process_run_events_until_done_reports_a_stream_lost_before_the_run_ended() {
    let (base_url, _paths) = serve_run(&[(1, "StageCompleted")], usize::MAX, false);
    let output = run_events_until_done(&base_url, &[]);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout).lines().count(), 1);
    assert_eq!(
        output.stderr.iter().filter(|byte| **byte == b'\n').count(),
        1
    );
    let error: Value = serde_json::from_slice(&output.stderr).expect("stderr json");
    assert_eq!(
        error,
        serde_json::json!({
            "ok": false,
            "error": "The event stream for run run-1 dropped events or ended before the run did; continue with spark run events run-1 --after 1 --until-done.",
            "exit_code": 1,
            "target": base_url,
            "target_source": "flag",
        })
    );
}

#[test]
fn process_run_events_until_done_reports_a_lagged_stream_with_its_cursor() {
    // The lag notice stands in for frames that held the run's end.
    let (base_url, _paths) = serve_run(
        &[(1, "StageCompleted"), (2, "resync_required")],
        usize::MAX,
        true,
    );
    let output = run_events_until_done(&base_url, &[]);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let error: Value = serde_json::from_slice(&output.stderr).expect("stderr json");
    assert_eq!(
        error["error"],
        "The event stream for run run-1 dropped events or ended before the run did; continue with spark run events run-1 --after 1 --until-done."
    );
}
