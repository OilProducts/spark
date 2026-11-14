use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use spark_cli::{
    http_status_exit_code, request_plan_with_args_env_and_stdin, run_with_args_and_env,
    run_with_args_env_and_stdin, HttpMethod,
};

/// The one JSON object a failure writes to stderr.
fn stderr_json(output: &spark_cli::CommandOutput) -> Value {
    assert_eq!(output.stderr.lines().count(), 1, "{}", output.stderr);
    serde_json::from_str(&output.stderr).expect("stderr json")
}

#[test]
fn start_unknown_image_argument_is_a_json_usage_error() {
    let env = BTreeMap::new();
    let output = run_with_args_and_env(
        [
            "spark",
            "run",
            "start",
            "--flow",
            "test-dispatch",
            "--summary",
            "Launch directly",
            "--project",
            "/tmp/project",
            "--image",
            "direct-selection",
        ],
        &env,
    );

    assert_eq!(output.exit_code, 2);
    assert_eq!(output.stdout, "");
    let error = stderr_json(&output);
    assert_eq!(error["ok"], false);
    assert_eq!(error["exit_code"], 2);
    let message = error["error"].as_str().unwrap();
    assert!(
        message.contains("unexpected argument '--image'"),
        "{message}"
    );
    assert!(!message.contains("as a value"), "{message}");
    assert!(message.contains("Usage: spark run start"), "{message}");
}

#[test]
fn flow_list_source_checkout_guard_returns_json_error() {
    let env = BTreeMap::new();
    let output = run_with_args_and_env(["spark", "flow", "list"], &env);

    assert_eq!(output.exit_code, 1);
    assert_eq!(output.stdout, "");
    assert!(output.stderr.starts_with("{\"ok\": false, \"error\": \"Refusing to use default API target http://127.0.0.1:8000 from a source checkout"));
    assert!(output.stderr.contains("before `spark flow list`"));
}

#[test]
fn flow_list_text_executes_http_request_and_renders_rows() {
    let env = BTreeMap::new();
    let (base_url, requests) = serve_once(HttpResponse::json(
        200,
        r#"[
          {"name":"examples/simple","title":"Simple Flow","description":"Small starter."},
          {"name":"ops/review","title":"ops/review","description":""}
        ]"#,
    ));
    let output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "list",
            "--text",
            "--base-url",
            base_url.as_str(),
        ],
        &env,
    );

    assert_eq!(output.exit_code, 0);
    assert_eq!(
        output.stdout,
        "examples/simple: Simple Flow\n  Small starter.\nops/review\n"
    );
    assert_eq!(output.stderr, "");
    let request = requests
        .recv_timeout(Duration::from_secs(2))
        .expect("request");
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, "/workspace/api/flows");
    assert_eq!(request.body, "");
}

#[test]
fn run_start_with_conversation_posts_by_handle_and_prints_response_json() {
    let env = BTreeMap::new();
    let (base_url, requests) = serve_once(HttpResponse::json(
        200,
        r#"{"ok":true,"flow_run_request_id":"flow-run-request-123"}"#,
    ));

    let output = run_with_args_and_env(
        [
            "spark",
            "run",
            "start",
            "--conversation",
            "amber-otter",
            "--flow",
            "software-development/implement-change-request",
            "--summary",
            "Run the approved scope",
            "--goal",
            "Implement it.",
            "--context",
            r#"{"context.request.summary":"Run the approved scope"}"#,
            "--model",
            "gpt-5",
            "--provider",
            "OpenAI",
            "--profile",
            "implementation",
            "--reasoning-effort",
            "FutureEffort",
            "--node-overrides",
            r#"{"verify":{"llm_model":"gpt-5.5","reasoning_effort":"high"}}"#,
            "--execution-profile",
            "local-dev",
            "--base-url",
            base_url.as_str(),
        ],
        &env,
    );

    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stderr, "");
    assert_eq!(
        serde_json::from_str::<Value>(&output.stdout).expect("stdout json")["flow_run_request_id"],
        "flow-run-request-123"
    );
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
            "execution_profile_id": "local-dev",
            "flow_name": "software-development/implement-change-request",
            "goal": "Implement it.",
            "launch_context": {"context.request.summary": "Run the approved scope"},
            "llm_profile": "implementation",
            "llm_provider": "openai",
            "model": "gpt-5",
            "node_overrides": {"verify": {"llm_model": "gpt-5.5", "reasoning_effort": "high"}},
            "reasoning_effort": "FutureEffort",
            "summary": "Run the approved scope"
        })
    );
}

#[test]
fn run_start_rejects_supplied_node_overrides_that_are_not_a_json_object() {
    let env = BTreeMap::new();
    let command = |base_url: &str, overrides: &str| -> Vec<String> {
        [
            "--flow",
            "test",
            "--summary",
            "Launch",
            "--node-overrides",
            overrides,
            "--base-url",
            base_url,
        ]
        .map(str::to_string)
        .to_vec()
    };
    for prefix in [
        ["spark", "run", "start", "--conversation", "amber-otter"],
        ["spark", "run", "start", "--project", "/tmp/project"],
    ] {
        // An empty `"$OVERRIDES"` must not launch without the overrides.
        for (overrides, exit_code, error) in [
            ("", 2, "must not be empty"),
            ("  ", 2, "must not be empty"),
            (
                r#"["verify"]"#,
                1,
                "--node-overrides must be a JSON object keyed by node id.",
            ),
            ("{", 1, "--node-overrides inline value is not valid JSON"),
        ] {
            let (base_url, requests) = serve_once(HttpResponse::json(200, r#"{"ok":true}"#));
            let args = prefix
                .map(str::to_string)
                .into_iter()
                .chain(command(&base_url, overrides));
            let output = run_with_args_and_env(args, &env);
            assert_eq!(
                output.exit_code, exit_code,
                "{prefix:?} {overrides:?}: {output:?}"
            );
            assert_eq!(output.stdout, "");
            let stderr = stderr_json(&output);
            assert_eq!(stderr["exit_code"], exit_code);
            assert!(
                stderr["error"].as_str().unwrap().contains(error),
                "{prefix:?} {overrides:?}: {}",
                output.stderr
            );
            assert!(
                requests.recv_timeout(Duration::from_millis(200)).is_err(),
                "{prefix:?} {overrides:?} dispatched a request"
            );
        }

        // An empty object is still a valid value and is sent as given.
        let (base_url, requests) = serve_once(HttpResponse::json(200, r#"{"ok":true}"#));
        let args = prefix
            .map(str::to_string)
            .into_iter()
            .chain(command(&base_url, "{}"));
        let output = run_with_args_and_env(args, &env);
        assert_eq!(output.exit_code, 0, "{prefix:?}: {output:?}");
        let request = requests
            .recv_timeout(Duration::from_secs(2))
            .expect("request");
        assert_eq!(
            serde_json::from_str::<Value>(&request.body).expect("request json")["node_overrides"],
            serde_json::json!({})
        );
    }
}

#[test]
fn run_start_requires_a_conversation_or_a_project() {
    let env = BTreeMap::new();

    let output = run_with_args_and_env(
        [
            "spark",
            "run",
            "start",
            "--flow",
            "test",
            "--summary",
            "Launch directly",
            "--base-url",
            "http://127.0.0.1:8010",
        ],
        &env,
    );

    assert_eq!(output.exit_code, 2);
    assert_eq!(output.stdout, "");
    let error = stderr_json(&output);
    assert_eq!(error["exit_code"], 2);
    let message = error["error"].as_str().unwrap();
    assert!(
        message.contains("<--conversation <HANDLE>|--project <PATH>>"),
        "{message}"
    );
}

#[test]
fn run_start_retry_and_continue_send_expected_json_bodies() {
    let env = BTreeMap::new();
    let (launch_base_url, launch_requests) = serve_once(HttpResponse::json(
        200,
        r#"{"ok":true,"operation":"launch"}"#,
    ));
    let launch_output = run_with_args_and_env(
        [
            "spark",
            "run",
            "start",
            "--flow",
            "test",
            "--summary",
            "Launch directly",
            "--project",
            "/tmp/project",
            "--execution-profile",
            "local-review",
            "--model",
            "gpt-5.3",
            "--provider",
            "Anthropic",
            "--profile",
            "launch-profile",
            "--reasoning-effort",
            "LOW",
            "--base-url",
            launch_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(launch_output.exit_code, 0);
    let launch_request = launch_requests
        .recv_timeout(Duration::from_secs(2))
        .expect("launch request");
    assert_eq!(launch_request.path, "/workspace/api/runs/launch");
    assert_eq!(
        serde_json::from_str::<Value>(&launch_request.body).expect("launch body"),
        serde_json::json!({
            "execution_profile_id": "local-review",
            "flow_name": "test",
            "llm_profile": "launch-profile",
            "llm_provider": "anthropic",
            "model": "gpt-5.3",
            "project_path": "/tmp/project",
            "reasoning_effort": "LOW",
            "summary": "Launch directly"
        })
    );

    let (retry_base_url, retry_requests) = serve_once(HttpResponse::json(
        200,
        r#"{"ok":true,"operation":"retry"}"#,
    ));
    let retry_output = run_with_args_and_env(
        [
            "spark",
            "run",
            "retry",
            "run-1",
            "--conversation",
            "amber-otter",
            "--base-url",
            retry_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(retry_output.exit_code, 0);
    let retry_request = retry_requests
        .recv_timeout(Duration::from_secs(2))
        .expect("retry request");
    assert_eq!(retry_request.path, "/workspace/api/runs/run-1/retry");
    assert_eq!(
        serde_json::from_str::<Value>(&retry_request.body).expect("retry body"),
        serde_json::json!({"conversation_handle": "amber-otter"})
    );

    let (continue_base_url, continue_requests) = serve_once(HttpResponse::json(
        200,
        r#"{"ok":true,"operation":"continue"}"#,
    ));
    let continue_output = run_with_args_and_env(
        [
            "spark",
            "run",
            "continue",
            "run-1",
            "--from",
            "next",
            "--project",
            "/repo",
            "--conversation",
            "amber-otter",
            "--model",
            "gpt-5.4",
            "--provider",
            "openai",
            "--profile",
            "default",
            "--reasoning-effort",
            "high",
            "--base-url",
            continue_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(continue_output.exit_code, 0);
    let continue_request = continue_requests
        .recv_timeout(Duration::from_secs(2))
        .expect("continue request");
    assert_eq!(continue_request.path, "/workspace/api/runs/run-1/continue");
    assert_eq!(
        serde_json::from_str::<Value>(&continue_request.body).expect("continue body"),
        serde_json::json!({
            "conversation_handle": "amber-otter",
            "flow_source_mode": "snapshot",
            "llm_profile": "default",
            "llm_provider": "openai",
            "model": "gpt-5.4",
            "project_path": "/repo",
            "reasoning_effort": "high",
            "start_node": "next"
        })
    );

    // `--flow` continues from the flow's current definition instead.
    let plan = request_plan_with_args_env_and_stdin(
        [
            "spark",
            "run",
            "continue",
            "run-1",
            "--from",
            "next",
            "--flow",
            "team/work",
            "--base-url",
            "http://127.0.0.1:8010",
        ],
        &env,
        "",
    )
    .expect("continue plan");
    assert_eq!(plan.path, "/workspace/api/runs/run-1/continue");
    assert_eq!(
        plan.body.expect("continue body"),
        serde_json::json!({
            "flow_name": "team/work",
            "flow_source_mode": "flow_name",
            "start_node": "next"
        })
    );
}

#[test]
fn flow_validate_by_name_renders_text_and_exits_one_on_errors() {
    let env = BTreeMap::new();
    let (validate_base_url, validate_requests) = serve_once(HttpResponse::json(
        200,
        r#"{
          "name":"software-development/implement-change-request",
          "path":"/flows/software-development/implement-change-request",
          "status":"invalid",
          "diagnostics":[{"severity":"error","rule_id":"missing-edge","message":"Missing edge.","line":7}],
          "errors":["Missing edge."]
        }"#,
    ));
    let validate_output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "validate",
            "software-development/implement-change-request",
            "--text",
            "--base-url",
            validate_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(validate_output.exit_code, 1);
    assert!(validate_output.stdout.contains("Status: invalid"));
    assert!(validate_output
        .stdout
        .contains("- ERROR missing-edge line 7: Missing edge."));
    assert_eq!(
        validate_requests
            .recv_timeout(Duration::from_secs(2))
            .expect("validate request")
            .path,
        "/workspace/api/flows/software-development%2Fimplement-change-request/validate"
    );
}

#[test]
fn flow_get_prints_the_files_response_and_text_prints_each_file_exactly() {
    let env = BTreeMap::new();
    let files = serde_json::json!({
        "name": "software-development/implement-change-request",
        "layer": "project",
        "path": "/project/.spark/definitions/flows/software-development/implement-change-request",
        "files": [
            {"name": "flow.md", "content": "---\nid: implement_change_request\n---\nGoal.\n"},
            {"name": "start.md", "content": "---\nkind: start\n---"},
        ],
    });
    let (text_base_url, text_requests) = serve_once(HttpResponse::json(200, files.to_string()));
    let text_output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "get",
            "software-development/implement-change-request",
            "--project",
            "/project",
            "--text",
            "--base-url",
            text_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(text_output.exit_code, 0, "{}", text_output.stderr);
    assert_eq!(
        text_output.stdout,
        "Layer: project\n\
Path: /project/.spark/definitions/flows/software-development/implement-change-request\n\
\n==> flow.md <==\n---\nid: implement_change_request\n---\nGoal.\n\
\n==> start.md <==\n---\nkind: start\n---\n"
    );
    let request = text_requests
        .recv_timeout(Duration::from_secs(2))
        .expect("text request");
    assert_eq!(
        request.path,
        "/workspace/api/flows/software-development%2Fimplement-change-request/files?project_path=%2Fproject"
    );

    let (json_base_url, _json_requests) = serve_once(HttpResponse::json(200, files.to_string()));
    let json_output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "get",
            "software-development/implement-change-request",
            "--base-url",
            json_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(json_output.exit_code, 0, "{}", json_output.stderr);
    assert_eq!(
        serde_json::from_str::<Value>(&json_output.stdout).expect("stdout json"),
        files
    );
}

#[test]
fn http_errors_map_detail_payloads_and_validation_arrays() {
    let env = BTreeMap::new();
    for (status, body, exit_code, message) in [
        (
            404,
            r#"{"detail":"Unknown pipeline"}"#,
            3,
            "Unknown pipeline",
        ),
        (
            422,
            r#"{"detail":[{"loc":["body","flow_name"],"msg":"Field required"},{"loc":["query","surface"],"msg":"Invalid surface"}]}"#,
            1,
            "flow_name: Field required; query.surface: Invalid surface",
        ),
        (
            409,
            r#"{"error":"Conflict from top level","detail":"Lower priority detail"}"#,
            1,
            "Conflict from top level",
        ),
        (
            400,
            r#"{"detail":{"error":"Nested detail error"}}"#,
            1,
            "Nested detail error",
        ),
    ] {
        let (base_url, _requests) = serve_once(HttpResponse::json(status, body));
        let output = run_with_args_and_env(
            [
                "spark",
                "run",
                "get",
                "run-1",
                "--base-url",
                base_url.as_str(),
            ],
            &env,
        );
        assert_eq!(output.exit_code, exit_code, "{status}");
        assert_eq!(output.stdout, "");
        assert_eq!(
            output.stderr,
            format!(
                "{{\"ok\": false, \"error\": \"{message}\", \"exit_code\": {exit_code}, \"status_code\": {status}, \"target\": \"{base_url}\", \"target_source\": \"flag\"}}\n"
            )
        );
    }

    let (text_error_base_url, _text_error_requests) =
        serve_once(HttpResponse::text(500, "Plain text failure\n"));
    let text_error_output = run_with_args_and_env(
        [
            "spark",
            "run",
            "get",
            "run-1",
            "--base-url",
            text_error_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(text_error_output.exit_code, 1);
    let error = stderr_json(&text_error_output);
    assert_eq!(error["error"], "Plain text failure");
    assert_eq!(error["status_code"], 500);
}

#[test]
fn http_transport_failures_return_request_failed_json() {
    let env = BTreeMap::new();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind unused port");
    let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
    drop(listener);

    let output = run_with_args_and_env(
        ["spark", "flow", "list", "--base-url", base_url.as_str()],
        &env,
    );

    assert_eq!(output.exit_code, 1);
    assert_eq!(output.stdout, "");
    assert!(output
        .stderr
        .starts_with("{\"ok\": false, \"error\": \"Request failed: "));
    let error = stderr_json(&output);
    assert_eq!(error["exit_code"], 1);
    assert_eq!(error["target"], base_url);
    assert_eq!(error["target_source"], "flag");
    assert!(error.get("status_code").is_none());
}

#[test]
fn run_events_streams_sse_json_lines_and_text() {
    let env = BTreeMap::new();
    let (json_base_url, json_requests) = serve_once(HttpResponse::sse(
        200,
        ": keepalive\n\
data: not-json\n\
\n\
data: {\n\
data: \"type\":\"run.journal_entry\",\"payload\":{\"sequence\":8,\"type\":\"log\",\"msg\":\"hello\"}}\n\
\n",
    ));
    let json_output = run_with_args_and_env(
        [
            "spark",
            "run",
            "events",
            "run-123",
            "--after",
            "7",
            "--base-url",
            json_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(json_output.exit_code, 0);
    assert_eq!(
        json_output.stdout,
        "{\"payload\":{\"msg\":\"hello\",\"sequence\":8,\"type\":\"log\"},\"type\":\"run.journal_entry\"}\n"
    );
    assert_eq!(
        json_requests
            .recv_timeout(Duration::from_secs(2))
            .expect("json sse request")
            .path,
        "/workspace/api/live/events?run_id=run-123&run_sequence=7"
    );

    let (text_base_url, _text_requests) = serve_once(HttpResponse::sse(
        200,
        "data: {\"payload\":{\"sequence\":9,\"type\":\"progress\",\"summary\":\"done\"}}\n\n",
    ));
    let text_output = run_with_args_and_env(
        [
            "spark",
            "run",
            "events",
            "run-123",
            "--text",
            "--base-url",
            text_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(text_output.exit_code, 0);
    assert_eq!(text_output.stdout, "9 progress: done\n");
}

#[test]
fn send_messages_a_conversation_by_handle_and_replaces_the_role_and_mission_sends() {
    let env = BTreeMap::new();
    let plan = spark_cli::request_plan_with_args_env_and_stdin(
        [
            "spark",
            "send",
            "--to",
            "brisk-otter",
            "--from",
            "amber-anchor",
            "--message",
            "Prefer small diffs",
            "--base-url",
            "http://localhost:8000",
        ],
        &env,
        "",
    )
    .unwrap();
    assert_eq!(plan.method, HttpMethod::Post);
    assert_eq!(
        plan.path,
        "/workspace/api/conversations/by-handle/brisk-otter/messages"
    );
    assert_eq!(
        plan.body.unwrap(),
        serde_json::json!({"conversation_handle": "amber-anchor", "message": "Prefer small diffs"})
    );
    for missing in ["--to", "--from", "--message"] {
        let mut args = vec!["spark", "send", "--base-url", "http://localhost:8000"];
        for (flag, value) in [
            ("--to", "brisk-otter"),
            ("--from", "amber-anchor"),
            ("--message", "Hi"),
        ] {
            if flag != missing {
                args.extend([flag, value]);
            }
        }
        assert!(
            spark_cli::request_plan_with_args_env_and_stdin(args, &env, "").is_err(),
            "{missing}"
        );
    }
    for removed in [
        &["spark", "role", "send", "coordinator", "--project", "/p"][..],
        &["spark", "mission", "send", "mission-1", "--project", "/p"][..],
    ] {
        let output = run_with_args_and_env(removed.iter().copied(), &env);
        assert_eq!(output.exit_code, 2, "{output:?}");
        assert!(
            stderr_json(&output)["error"]
                .as_str()
                .unwrap()
                .contains("unrecognized subcommand 'send'"),
            "{}",
            output.stderr
        );
    }
}

#[test]
fn flow_format_is_not_a_command() {
    let output = run_with_args_and_env(["spark", "flow", "format", "--dir", "x"], &BTreeMap::new());

    assert_eq!(output.exit_code, 2, "{output:?}");
    assert_eq!(output.stdout, "");
    let message = stderr_json(&output)["error"].as_str().unwrap().to_string();
    assert!(
        message.contains("unrecognized subcommand 'format'"),
        "{message}"
    );
    assert!(
        message.contains("valid subcommands: list, get, validate"),
        "{message}"
    );
}

#[test]
fn flow_validate_dir_json_uses_local_preview_without_source_checkout_guard() {
    let temp_dir = temp_dir("flow-validate-success");
    let flow_dir = temp_dir.join("valid-flow");
    crate::write_flow_json(&flow_dir, valid_flow());
    let env = BTreeMap::new();

    let output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "validate",
            "--dir",
            flow_dir.to_str().expect("path"),
        ],
        &env,
    );

    assert_eq!(output.exit_code, 0, "{}", output.stderr);
    assert_eq!(output.stderr, "");
    let payload: Value = serde_json::from_str(&output.stdout).expect("valid json");
    assert_eq!(payload["name"], "valid-flow");
    assert_eq!(
        payload["path"],
        fs::canonicalize(&flow_dir)
            .expect("canonical path")
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["diagnostics"], Value::Array(Vec::new()));
    assert_eq!(payload["errors"], Value::Array(Vec::new()));
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn flow_validate_dir_rejects_single_flow_files() {
    let temp_dir = temp_dir("flow-validate-yaml");
    let flow_path = temp_dir.join("flow.yaml");
    // A whole-flow YAML file is no longer a flow source.
    fs::write(&flow_path, "id: workflow\n").expect("write flow");
    let flow_path = flow_path.to_str().expect("path");
    let env = BTreeMap::new();

    let yaml_file = run_with_args_and_env(["spark", "flow", "validate", "--dir", flow_path], &env);
    assert_eq!(yaml_file.exit_code, 1, "{yaml_file:?}");
    assert_eq!(yaml_file.stdout, "");
    assert_eq!(
        yaml_file.stderr,
        format!(
            "{{\"ok\": false, \"error\": \"Flow directory not found: {flow_path}\", \"exit_code\": 1}}\n"
        )
    );
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn flow_validate_dir_checks_reasoning_controls_and_inherited_budgets() {
    let temp_dir = temp_dir("flow-validate-reasoning");
    let flow_path = temp_dir.join("flow");
    let budget = serde_json::json!({"thinking":"budget", "thinking_budget_tokens":2048});
    for (defaults, execution, valid) in [
        (
            serde_json::json!({"reasoning_mode":"fast"}),
            serde_json::json!({}),
            false,
        ),
        (
            serde_json::json!({"thinking":"invalid"}),
            serde_json::json!({}),
            false,
        ),
        (
            serde_json::json!({"reasoning_summary":"none"}),
            serde_json::json!({}),
            false,
        ),
        (
            serde_json::json!({"thinking":"budget"}),
            serde_json::json!({}),
            false,
        ),
        (
            budget.clone(),
            serde_json::json!({"thinking_budget_tokens":4096}),
            true,
        ),
        // An explicit non-budget thinking drops the inherited budget.
        (budget.clone(), serde_json::json!({"thinking":"off"}), true),
        (
            budget.clone(),
            serde_json::json!({"thinking":"off", "thinking_budget_tokens":4096}),
            false,
        ),
    ] {
        let source = serde_json::json!({"schema_version":"2", "id":"flow",
            "defaults":defaults,
            "nodes":{
                "start":{"kind":"start", "config":{"kind":"start"}, "routes":{"next":{"to":"work", "description":"Enter the first workflow step."}}},
                "work":{"kind":"agent_task", "config":{"kind":"agent_task", "prompt":"hello"}, "contracts":{"response":"decision_envelope"}, "execution":execution, "routes":{"done":{"to":"end", "description":"The work is complete."}}},
                "end":{"kind":"exit", "config":{"kind":"exit", "result":{"name":"completed", "outcome":"success"}}}
            }
        });
        crate::write_flow_json(&flow_path, source);
        let output = run_with_args_and_env(
            [
                "spark",
                "flow",
                "validate",
                "--dir",
                flow_path.to_str().unwrap(),
            ],
            &BTreeMap::new(),
        );
        assert_eq!(output.exit_code, if valid { 0 } else { 1 });
        let payload: Value = serde_json::from_str(&output.stdout).unwrap();
        assert_eq!(
            payload["status"],
            if valid { "ok" } else { "validation_error" },
            "{payload}"
        );
        assert_eq!(
            payload["diagnostics"].as_array().unwrap().is_empty(),
            valid,
            "{payload}"
        );
        if !valid {
            assert!(
                payload["diagnostics"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["rule_id"] == "reasoning_controls"),
                "{payload}"
            );
        }
    }
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn flow_validate_dir_text_renders_diagnostics() {
    let temp_dir = temp_dir("flow-validate-text");
    let flow_path = temp_dir.join("validation-error-flow");
    crate::write_flow_json(&flow_path, validation_error_flow());
    let env = BTreeMap::new();

    let output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "validate",
            "--dir",
            flow_path.to_str().expect("path"),
            "--text",
        ],
        &env,
    );

    assert_eq!(output.exit_code, 1);
    assert_eq!(output.stderr, "");
    assert_eq!(
        output.stdout,
        format!(
            "Name: validation-error-flow\n\
Path: {}\n\
Status: validation_error\n\
Diagnostics: 3\n\
Errors: 3\n\
- ERROR start_node: flow must have exactly one start node, found 0\n\
- ERROR exit_node: flow must have at least one exit node\n\
- ERROR decision_domain: task.md: agent_task steps need at least one route\n",
            fs::canonicalize(&flow_path)
                .expect("canonical path")
                .display()
        )
    );
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn flow_validate_dir_and_flow_are_mutually_exclusive() {
    let env = BTreeMap::new();

    let output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "validate",
            "examples/simple-linear",
            "--dir",
            "local",
        ],
        &env,
    );

    assert_eq!(output.exit_code, 2);
    assert_eq!(output.stdout, "");
    let message = stderr_json(&output)["error"].as_str().unwrap().to_string();
    assert!(message.contains("cannot be used with"), "{message}");
}

#[test]
fn request_plans_resolve_target_order_and_escape_path_segments() {
    let mut env = BTreeMap::new();
    env.insert(
        "SPARK_API_BASE_URL".to_string(),
        "http://127.0.0.1:8010".to_string(),
    );

    let env_plan = request_plan_with_args_env_and_stdin(["spark", "flow", "list"], &env, "")
        .expect("flow list plan");
    assert_eq!(env_plan.method, HttpMethod::Get);
    assert_eq!(env_plan.base_url, "http://127.0.0.1:8010");
    assert_eq!(env_plan.target_source, "env");
    assert_eq!(env_plan.path, "/workspace/api/flows");

    let explicit_plan = request_plan_with_args_env_and_stdin(
        [
            "spark",
            "flow",
            "get",
            "software-development/implement-change-request",
            "--base-url=http://127.0.0.1:8020",
        ],
        &env,
        "",
    )
    .expect("flow get plan");
    assert_eq!(explicit_plan.base_url, "http://127.0.0.1:8020");
    assert_eq!(explicit_plan.target_source, "flag");
    assert_eq!(
        explicit_plan.path,
        "/workspace/api/flows/software-development%2Fimplement-change-request/files"
    );
}

#[test]
fn trigger_json_payload_can_come_from_injected_stdin() {
    let env = BTreeMap::new();

    let plan = request_plan_with_args_env_and_stdin(
        [
            "spark",
            "trigger",
            "create",
            "--project",
            "/repo",
            "--json",
            "-",
            "--base-url",
            "http://127.0.0.1:8010",
        ],
        &env,
        r#"{"id":"nightly","enabled":true}"#,
    )
    .expect("trigger create plan");

    assert_eq!(plan.method, HttpMethod::Post);
    assert_eq!(plan.path, "/workspace/api/triggers");
    assert_eq!(
        plan.body.expect("body"),
        serde_json::json!({"enabled": true, "id": "nightly", "project_path": "/repo"})
    );
}

#[test]
fn trigger_json_payload_rejects_non_object_stdin_before_dispatch() {
    let env = BTreeMap::new();

    let output = run_with_args_env_and_stdin(
        [
            "spark",
            "trigger",
            "create",
            "--project",
            "/repo",
            "--json",
            "-",
            "--base-url",
            "http://127.0.0.1:8010",
        ],
        &env,
        "[]",
    );

    assert_eq!(output.exit_code, 1);
    assert_eq!(output.stdout, "");
    assert_eq!(
        output.stderr,
        "{\"ok\": false, \"error\": \"--json must be a JSON object.\", \"exit_code\": 1}\n"
    );
}

#[test]
fn trigger_commands_execute_http_requests_and_render_text() {
    let env = BTreeMap::new();
    let (list_base_url, list_requests) = serve_once(HttpResponse::json(
        200,
        r#"[{"id":"trigger-123","project_path":"/repo","name":"Nightly","enabled":true,"protected":false,"source_type":"webhook","action":{"flow_name":"ops/run"},"state":{}}]"#,
    ));
    let list_output = run_with_args_and_env(
        [
            "spark",
            "trigger",
            "list",
            "--project",
            "/repo",
            "--base-url",
            list_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(list_output.exit_code, 0);
    assert_eq!(list_output.stderr, "");
    let list_payload = serde_json::from_str::<Value>(&list_output.stdout).expect("list json");
    assert_eq!(list_payload[0]["id"], "trigger-123");
    let list_request = list_requests
        .recv_timeout(Duration::from_secs(2))
        .expect("list request");
    assert_eq!(list_request.method, "GET");
    assert_eq!(
        list_request.path,
        "/workspace/api/triggers?project_path=%2Frepo"
    );
    assert_eq!(list_request.body, "");

    let (list_text_base_url, _list_text_requests) = serve_once(HttpResponse::json(
        200,
        r#"[{"id":"trigger-123","project_path":"/repo","name":"Nightly","enabled":true,"protected":false,"source_type":"webhook","action":{"flow_name":"ops/run"},"state":{}}]"#,
    ));
    let list_text_output = run_with_args_and_env(
        [
            "spark",
            "trigger",
            "list",
            "--text",
            "--base-url",
            list_text_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(list_text_output.exit_code, 0);
    assert_eq!(
        list_text_output.stdout,
        "trigger-123: Nightly [webhook] -> ops/run\n  project=/repo enabled=true protected=false\n"
    );

    let (describe_base_url, describe_requests) = serve_once(HttpResponse::json(
        200,
        r#"{"id":"trigger-123","project_path":"/repo","name":"Nightly","enabled":true,"protected":false,"source_type":"webhook","action":{"flow_name":"ops/run"},"state":{"last_fired_at":null,"last_result":null,"next_run_at":null},"webhook_secret":"secret-123"}"#,
    ));
    let describe_output = run_with_args_and_env(
        [
            "spark",
            "trigger",
            "get",
            "trigger-123",
            "--project",
            "/repo",
            "--text",
            "--base-url",
            describe_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(describe_output.exit_code, 0);
    assert_eq!(
        describe_output.stdout,
        "ID: trigger-123\n\
Name: Nightly\n\
Project: /repo\n\
Source Type: webhook\n\
Enabled: true\n\
Protected: false\n\
Flow Target: ops/run\n\
Last Fired: (never)\n\
Last Result: (none)\n\
Next Run: (n/a)\n\
Webhook Secret: secret-123\n"
    );
    assert_eq!(
        describe_requests
            .recv_timeout(Duration::from_secs(2))
            .expect("describe request")
            .path,
        "/workspace/api/triggers/trigger-123?project_path=%2Frepo"
    );
}

#[test]
fn trigger_create_update_delete_use_payloads_and_percent_encoded_ids() {
    let env = BTreeMap::new();
    let temp_dir = temp_dir("trigger-command-payloads");
    let payload_path = temp_dir.join("trigger.json");
    fs::write(
        &payload_path,
        r#"{"name":"Nightly","source_type":"webhook","action":{"flow_name":"ops/run"},"source":{}}"#,
    )
    .expect("write trigger payload");

    let (create_base_url, create_requests) = serve_once(HttpResponse::json(
        200,
        r#"{"id":"trigger-123","name":"Nightly"}"#,
    ));
    let create_output = run_with_args_and_env(
        [
            "spark",
            "trigger",
            "create",
            "--project",
            "/repo",
            "--json",
            payload_path.to_str().expect("payload path"),
            "--message",
            "Nightly digest",
            "--base-url",
            create_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(create_output.exit_code, 0);
    assert_eq!(create_output.stderr, "");
    let create_request = create_requests
        .recv_timeout(Duration::from_secs(2))
        .expect("create request");
    assert_eq!(create_request.method, "POST");
    assert_eq!(create_request.path, "/workspace/api/triggers");
    assert_eq!(
        serde_json::from_str::<Value>(&create_request.body).expect("create body"),
        serde_json::json!({
            "project_path": "/repo",
            "message": "Nightly digest",
            "name": "Nightly",
            "source_type": "webhook",
            "action": {"flow_name": "ops/run"},
            "source": {}
        })
    );

    let update_path = temp_dir.join("trigger-update.json");
    fs::write(
        &update_path,
        r#"{"name":"Nightly updated","regenerate_webhook_secret":true}"#,
    )
    .expect("write trigger update");
    let (update_base_url, update_requests) = serve_once(HttpResponse::json(
        200,
        r#"{"id":"trigger/custom","name":"Nightly updated","webhook_secret":"new-secret"}"#,
    ));
    let update_output = run_with_args_and_env(
        [
            "spark",
            "trigger",
            "update",
            "trigger/custom",
            "--project",
            "/repo",
            "--json",
            update_path.to_str().expect("update path"),
            "--base-url",
            update_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(update_output.exit_code, 0);
    let update_request = update_requests
        .recv_timeout(Duration::from_secs(2))
        .expect("update request");
    assert_eq!(update_request.method, "PATCH");
    assert_eq!(
        update_request.path,
        "/workspace/api/triggers/trigger%2Fcustom?project_path=%2Frepo"
    );
    assert_eq!(
        serde_json::from_str::<Value>(&update_request.body).expect("update body"),
        serde_json::json!({"name": "Nightly updated", "regenerate_webhook_secret": true})
    );

    let (delete_base_url, delete_requests) = serve_once(HttpResponse::json(
        200,
        r#"{"id":"trigger/custom","status":"deleted"}"#,
    ));
    let delete_output = run_with_args_and_env(
        [
            "spark",
            "trigger",
            "delete",
            "trigger/custom",
            "--project",
            "/repo",
            "--expected-revision",
            "revision-1",
            "--message",
            "Retire it",
            "--base-url",
            delete_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(delete_output.exit_code, 0);
    let delete_request = delete_requests
        .recv_timeout(Duration::from_secs(2))
        .expect("delete request");
    assert_eq!(delete_request.method, "DELETE");
    assert_eq!(
        delete_request.path,
        "/workspace/api/triggers/trigger%2Fcustom?project_path=%2Frepo&expected_revision=revision-1&message=Retire%20it"
    );

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn trigger_http_errors_preserve_cli_stderr_envelope() {
    let env = BTreeMap::new();
    let (base_url, _requests) = serve_once(HttpResponse::json(
        400,
        r#"{"detail":"Protected triggers cannot be deleted."}"#,
    ));
    let output = run_with_args_and_env(
        [
            "spark",
            "trigger",
            "delete",
            "protected",
            "--project",
            "/repo",
            "--expected-revision",
            "revision-1",
            "--base-url",
            base_url.as_str(),
        ],
        &env,
    );

    assert_eq!(output.exit_code, 1);
    assert_eq!(output.stdout, "");
    assert_eq!(
        output.stderr,
        format!(
            "{{\"ok\": false, \"error\": \"Protected triggers cannot be deleted.\", \"exit_code\": 1, \"status_code\": 400, \"target\": \"{base_url}\", \"target_source\": \"flag\"}}\n"
        )
    );
}

#[test]
fn run_start_goal_reads_stdin_and_only_one_flag_may_read_it() {
    let env = BTreeMap::new();
    let args = [
        "spark",
        "run",
        "start",
        "--project",
        "/tmp/project",
        "--flow",
        "test",
        "--summary",
        "Summary",
        "--goal",
        "-",
        "--base-url",
        "http://localhost:8000",
    ];

    let plan = request_plan_with_args_env_and_stdin(args, &env, "Goal text\n").unwrap();
    assert_eq!(plan.path, "/workspace/api/runs/launch");
    assert_eq!(plan.body.unwrap()["goal"], "Goal text");

    let output = run_with_args_env_and_stdin(
        args.into_iter().chain(["--context", "-"]),
        &env,
        "Goal text\n",
    );
    assert_eq!(output.exit_code, 2, "{output:?}");
    assert_eq!(output.stdout, "");
    assert_eq!(
        stderr_json(&output)["error"],
        "--goal and --context both read stdin (-); pass one of them inline or as a file."
    );
}

#[test]
fn http_not_found_maps_to_exit_three() {
    assert_eq!(http_status_exit_code(404), 3);
    assert_eq!(http_status_exit_code(409), 1);
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

    fn sse(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "text/event-stream",
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
            422 => "Unprocessable Entity",
            500 => "Internal Server Error",
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

fn validation_error_flow() -> Value {
    serde_json::json!({
        "schema_version": "2",
        "id": "workflow",
        "title": "Workflow",
        "nodes": {
            "task": {
                "kind": "agent_task",
                "label": "Task",
                "config": {"kind": "agent_task", "prompt": "No start or done"},
                "contracts": {"response": "decision_envelope"},
            },
        },
    })
}

#[test]
fn mission_stdin_uses_structured_json_and_run_request_rejects_task_flags() {
    let env = BTreeMap::new();
    let plan = request_plan_with_args_env_and_stdin(
        [
            "spark",
            "mission",
            "update",
            "mission-123",
            "--project",
            "/project",
            "--json",
            "-",
            "--base-url",
            "http://localhost:8000",
        ],
        &env,
        r#"{"revision":4,"fields":{"description":"One\nTwo"},"note":"Evidence"}"#,
    )
    .unwrap();
    assert_eq!(plan.method, HttpMethod::Patch);
    assert_eq!(plan.body.unwrap()["fields"]["description"], "One\nTwo");
    for flag in ["--task-id", "--task-stage"] {
        assert!(request_plan_with_args_env_and_stdin(
            [
                "spark",
                "run",
                "start",
                "--conversation",
                "amber-anchor",
                "--flow",
                "ops/test",
                "--summary",
                "Work",
                flag,
                "removed",
                "--base-url",
                "http://localhost:8000"
            ],
            &env,
            ""
        )
        .is_err());
    }
}

#[test]
fn mission_commands_use_project_scope_and_minimal_payloads() {
    let env = BTreeMap::new();
    for command in ["list", "get", "create", "update"] {
        let mut args = vec![
            "spark",
            "mission",
            command,
            "--project",
            "/my project",
            "--base-url",
            "http://localhost:8000",
        ];
        if matches!(command, "get" | "update") {
            args.insert(3, "mission-123");
        }
        if matches!(command, "create" | "update") {
            args.extend(["--json", "-"]);
        }
        let payload = if command == "update" {
            r#"{"revision":2,"fields":{"archived":true}}"#
        } else {
            r#"{"fields":{"title":"Title only"}}"#
        };
        let plan = request_plan_with_args_env_and_stdin(args, &env, payload).unwrap();
        assert!(plan.path.ends_with("?project_path=%2Fmy%20project"));
        if matches!(command, "get" | "list") {
            assert_eq!(plan.method, HttpMethod::Get);
            assert!(plan.body.is_none());
        } else {
            let body = plan.body.unwrap();
            assert_eq!(body["actor"], "assistant");
            assert!(body.get("conversation_id").is_none());
            assert!(body.get("note").is_none());
            if command == "create" {
                assert_eq!(body["fields"], serde_json::json!({"title":"Title only"}));
            }
        }
    }
    let scoped = |command: &str, extra: &[&'static str]| {
        let mut args = vec![
            "spark",
            "mission",
            command,
            "mission-1",
            "--project",
            "/p",
            "--base-url",
            "http://localhost:8000",
        ];
        args.extend_from_slice(extra);
        request_plan_with_args_env_and_stdin(args, &env, "").unwrap()
    };
    let start = scoped("start", &[]);
    assert_eq!(start.method, HttpMethod::Post);
    assert_eq!(
        start.path,
        "/workspace/api/missions/mission-1/start?project_path=%2Fp"
    );
    let waiting = scoped("wait", &["--reason", "Review pending"]);
    assert_eq!(waiting.method, HttpMethod::Post);
    assert_eq!(
        waiting.path,
        "/workspace/api/missions/mission-1/wait?project_path=%2Fp"
    );
    assert_eq!(waiting.body.unwrap()["reason"], "Review pending");
    let close = scoped("close", &["--status", "done", "--reason", "Shipped"]);
    assert_eq!(close.method, HttpMethod::Post);
    assert_eq!(
        close.path,
        "/workspace/api/missions/mission-1/close?project_path=%2Fp"
    );
    assert_eq!(
        close.body.unwrap(),
        serde_json::json!({"status":"done","reason":"Shipped","actor":"assistant"})
    );
    for invalid in [
        &["--status", "finished", "--reason", "x"][..],
        &["--status", "done"][..],
    ] {
        let mut args = vec![
            "spark",
            "mission",
            "close",
            "mission-1",
            "--project",
            "/p",
            "--base-url",
            "http://localhost:8000",
        ];
        args.extend_from_slice(invalid);
        assert!(request_plan_with_args_env_and_stdin(args, &env, "").is_err());
    }
    for removed in ["events", "pause", "resume", "send"] {
        assert!(request_plan_with_args_env_and_stdin(
            ["spark", "mission", removed, "mission-1", "--project", "/p"],
            &env,
            ""
        )
        .is_err());
    }
    assert!(request_plan_with_args_env_and_stdin(
        ["spark", "task", "list", "--project", "/p"],
        &env,
        ""
    )
    .is_err());
}

#[test]
fn settings_commands_use_scoped_http_and_preserve_explicit_target_safeguards() {
    let env = std::collections::BTreeMap::<String, String>::new();
    let get = spark_cli::request_plan_with_args_env_and_stdin(
        [
            "spark",
            "settings",
            "get",
            "--base-url",
            "http://127.0.0.1:9876",
        ],
        &env,
        "",
    )
    .unwrap();
    assert_eq!(get.method, spark_cli::HttpMethod::Get);
    assert_eq!(get.path, "/workspace/api/settings");
    for (command, method, path) in [
        (
            "set",
            spark_cli::HttpMethod::Patch,
            "/workspace/api/settings",
        ),
        (
            "validate",
            spark_cli::HttpMethod::Post,
            "/workspace/api/settings/validate",
        ),
    ] {
        let payload =
            r#"{"expected_revision":"rev-1","section":"runtime","value":{"runs_dir":"/chosen"}}"#;
        let plan = spark_cli::request_plan_with_args_env_and_stdin(
            [
                "spark",
                "settings",
                command,
                "--base-url",
                "http://127.0.0.1:9876",
                "--json",
                "-",
            ],
            &env,
            payload,
        )
        .unwrap();
        assert_eq!(plan.method, method);
        assert_eq!(plan.path, path);
        assert_eq!(plan.body.unwrap()["expected_revision"], "rev-1");
    }
    assert!(spark_cli::request_plan_with_args_env_and_stdin(
        ["spark", "settings", "get"],
        &env,
        ""
    )
    .is_err());
}

#[test]
fn settings_get_encodes_project_and_conversation_scopes() {
    let env = std::collections::BTreeMap::<String, String>::new();
    let plan = spark_cli::request_plan_with_args_env_and_stdin(
        [
            "spark",
            "settings",
            "get",
            "--base-url",
            "http://127.0.0.1:9876",
            "--project",
            "/tmp/a & b",
            "--conversation-id",
            "chat/1",
        ],
        &env,
        "",
    )
    .unwrap();
    assert_eq!(
        plan.path,
        "/workspace/api/settings?project_path=%2Ftmp%2Fa%20%26%20b&conversation_id=chat%2F1"
    );
    assert_eq!(plan.method, spark_cli::HttpMethod::Get);
}

#[test]
fn run_questions_and_answers_use_existing_routes_and_owning_run() {
    let env = BTreeMap::new();
    let (base, requests) = serve_once(HttpResponse::json(
        200,
        r#"{"questions":[{"run_id":"child","question_id":"q1"}]}"#,
    ));
    let output = run_with_args_and_env(
        ["spark", "run", "questions", "root", "--base-url", &base],
        &env,
    );
    assert_eq!(output.exit_code, 0);
    assert!(output.stdout.contains("child"));
    assert_eq!(
        requests.recv_timeout(Duration::from_secs(2)).unwrap().path,
        "/attractor/pipelines/root/questions"
    );
    for (flag, value, note) in [
        ("--option", "approve", None),
        (
            "--text",
            "Use the existing design",
            Some("Checked the spec"),
        ),
    ] {
        let (base, requests) = serve_once(HttpResponse::json(200, r#"{"ok":true}"#));
        let mut args = vec![
            "spark",
            "run",
            "answer",
            "child",
            "--question",
            "q1",
            flag,
            value,
            "--base-url",
            &base,
        ];
        if let Some(note) = note {
            args.extend(["--note", note]);
        }
        let output = run_with_args_and_env(args, &env);
        assert_eq!(output.exit_code, 0, "{}", output.stderr);
        let request = requests.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            request.path,
            "/attractor/pipelines/child/questions/q1/answer"
        );
        let mut expected = serde_json::json!({"selected_value": value});
        if let Some(note) = note {
            expected["note"] = Value::from(note);
        }
        assert_eq!(
            serde_json::from_str::<Value>(&request.body).unwrap(),
            expected
        );
    }
    for args in [vec![], vec!["--option", "a", "--text", "b"]] {
        let mut command = vec!["spark", "run", "answer", "child", "--question", "q1"];
        command.extend(args);
        assert_ne!(run_with_args_and_env(command, &env).exit_code, 0);
    }
}

#[test]
fn a_leading_base_url_keeps_each_commands_error_guidance() {
    let env = BTreeMap::new();
    let error = |args: &[&str]| {
        let output = run_with_args_and_env(
            [&["spark", "--base-url", "http://127.0.0.1:9"][..], args].concat(),
            &env,
        );
        assert_eq!(output.exit_code, 2, "{args:?}: {output:?}");
        stderr_json(&output)["error"].as_str().unwrap().to_string()
    };

    let unknown = error(&["run", "bogus"]);
    assert!(
        unknown.contains(
            "valid subcommands: start, list, get, cancel, retry, continue, events, questions, answer"
        ),
        "{unknown}"
    );
    let missing = error(&["run", "get"]);
    assert!(
        missing.contains("<RUN_ID>: Run id, from spark run list"),
        "{missing}"
    );
}

#[test]
fn delegation_wait_rejects_an_unrepresentable_timeout_as_a_usage_error() {
    let env = BTreeMap::new();
    for timeout in ["1e300", "-1", "inf", "nan"] {
        let output = run_with_args_and_env(
            [
                "spark",
                "delegation",
                "wait",
                "d-1",
                &format!("--timeout={timeout}"),
                "--base-url",
                "http://127.0.0.1:9",
            ],
            &env,
        );
        assert_eq!(output.exit_code, 2, "{timeout}: {output:?}");
        let message = stderr_json(&output)["error"].as_str().unwrap().to_string();
        assert!(
            message.contains(&format!("invalid value '{timeout}' for '--timeout")),
            "{message}"
        );
    }
}

#[test]
fn mission_create_takes_a_project_or_a_conversation_not_both() {
    // No target is configured: a usage error proves the check runs before
    // the target is resolved or any request is sent.
    let env = BTreeMap::new();
    let output = run_with_args_and_env(
        [
            "spark",
            "mission",
            "create",
            "--project",
            "/repo",
            "--conversation",
            "amber-otter",
            "--json",
            "{}",
        ],
        &env,
    );

    assert_eq!(output.exit_code, 2, "{output:?}");
    let message = stderr_json(&output)["error"].as_str().unwrap().to_string();
    assert!(
        message.contains("'--project <PATH>' cannot be used with '--conversation <HANDLE>'"),
        "{message}"
    );
}
