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

const TOP_LEVEL_HELP: &str = concat!(
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
);

#[test]
fn top_level_help_matches_argparse_contract() {
    let env = BTreeMap::new();
    let output = run_with_args_and_env(["spark", "--help"], &env);

    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stdout, TOP_LEVEL_HELP);
    assert_eq!(output.stderr, "");
}

#[test]
fn missing_domain_prints_top_level_help() {
    let env = BTreeMap::new();
    let output = run_with_args_and_env(["spark"], &env);

    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stdout, TOP_LEVEL_HELP);
    assert_eq!(output.stderr, "");
}

#[test]
fn launch_unknown_image_argument_keeps_usage_error_category() {
    let env = BTreeMap::new();
    let output = run_with_args_and_env(
        [
            "spark",
            "run",
            "launch",
            "--flow",
            "test-dispatch.dot",
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
    assert_eq!(
        output.stderr,
        "usage: spark [-h] {convo,run,flow,trigger} ...\n\
spark: error: unrecognized arguments: --image direct-selection\n"
    );
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
          {"name":"examples/simple.dot","title":"Simple Flow","description":"Small starter."},
          {"name":"ops/review.dot","title":"ops/review.dot","description":""}
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
        "examples/simple.dot: Simple Flow\n  Small starter.\nops/review.dot\n"
    );
    assert_eq!(output.stderr, "");
    let request = requests
        .recv_timeout(Duration::from_secs(2))
        .expect("request");
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, "/workspace/api/flows?surface=agent");
    assert_eq!(request.body, "");
}

#[test]
fn convo_run_request_posts_payload_and_prints_response_json() {
    let env = BTreeMap::new();
    let (base_url, requests) = serve_once(HttpResponse::json(
        200,
        r#"{"ok":true,"flow_run_request_id":"flow-run-request-123"}"#,
    ));

    let output = run_with_args_and_env(
        [
            "spark",
            "convo",
            "run-request",
            "--conversation",
            "amber-otter",
            "--flow",
            "software-development/implement-change-request.dot",
            "--summary",
            "Run the approved scope",
            "--goal",
            "Implement it.",
            "--launch-context-json",
            r#"{"context.request.summary":"Run the approved scope"}"#,
            "--model",
            "gpt-5",
            "--llm-provider",
            "OpenAI",
            "--llm-profile",
            "implementation",
            "--reasoning-effort",
            "HIGH",
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
            "flow_name": "software-development/implement-change-request.dot",
            "goal": "Implement it.",
            "launch_context": {"context.request.summary": "Run the approved scope"},
            "llm_profile": "implementation",
            "llm_provider": "openai",
            "model": "gpt-5",
            "reasoning_effort": "high",
            "summary": "Run the approved scope"
        })
    );
}

#[test]
fn run_launch_requires_project_when_conversation_is_omitted() {
    let env = BTreeMap::new();

    let output = run_with_args_and_env(
        [
            "spark",
            "run",
            "launch",
            "--flow",
            "test.dot",
            "--summary",
            "Launch directly",
            "--base-url",
            "http://127.0.0.1:8010",
        ],
        &env,
    );

    assert_eq!(output.exit_code, 1);
    assert_eq!(output.stdout, "");
    assert_eq!(
        output.stderr,
        "{\"ok\": false, \"error\": \"spark run launch requires --project when --conversation is omitted.\"}\n"
    );
}

#[test]
fn run_launch_retry_and_continue_send_expected_json_bodies() {
    let env = BTreeMap::new();
    let (launch_base_url, launch_requests) = serve_once(HttpResponse::json(
        200,
        r#"{"ok":true,"operation":"launch"}"#,
    ));
    let launch_output = run_with_args_and_env(
        [
            "spark",
            "run",
            "launch",
            "--flow",
            "test.dot",
            "--summary",
            "Launch directly",
            "--project",
            "/tmp/project",
            "--execution-profile",
            "local-review",
            "--model",
            "gpt-5.3",
            "--llm-provider",
            "Anthropic",
            "--llm-profile",
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
            "flow_name": "test.dot",
            "llm_profile": "launch-profile",
            "llm_provider": "anthropic",
            "model": "gpt-5.3",
            "project_path": "/tmp/project",
            "reasoning_effort": "low",
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
            "--run",
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
            "--run",
            "run-1",
            "--start-node",
            "next",
            "--flow-source-mode",
            "snapshot",
            "--flow",
            "ignored.dot",
            "--project",
            "/repo",
            "--conversation",
            "amber-otter",
            "--model",
            "gpt-5.4",
            "--llm-provider",
            "openai",
            "--llm-profile",
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
}

#[test]
fn flow_describe_and_validate_text_match_python_labels() {
    let env = BTreeMap::new();
    let (describe_base_url, describe_requests) = serve_once(HttpResponse::json(
        200,
        r#"{
          "name":"software-development/implement-change-request.dot",
          "title":"Implement Change Request",
          "description":"Execute approved work items.",
          "effective_launch_policy":"agent_requestable",
          "graph_label":"Implement Change Request",
          "graph_goal":"Implement approved changes",
          "node_count":6,
          "edge_count":6,
          "features":{"has_human_gate":false,"has_manager_loop":true}
        }"#,
    ));
    let describe_output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "describe",
            "--flow",
            "software-development/implement-change-request.dot",
            "--text",
            "--base-url",
            describe_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(describe_output.exit_code, 0);
    assert!(describe_output
        .stdout
        .contains("Launch Policy: agent_requestable"));
    assert!(describe_output.stdout.contains("Has Human Gate: False"));
    assert!(describe_output.stdout.contains("Has Manager Loop: True"));
    assert_eq!(
        describe_requests
            .recv_timeout(Duration::from_secs(2))
            .expect("describe request")
            .path,
        "/workspace/api/flows/software-development%2Fimplement-change-request.dot?surface=agent"
    );

    let (validate_base_url, validate_requests) = serve_once(HttpResponse::json(
        200,
        r#"{
          "name":"software-development/implement-change-request.dot",
          "path":"/flows/software-development/implement-change-request.dot",
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
            "--flow",
            "software-development/implement-change-request.dot",
            "--text",
            "--base-url",
            validate_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(validate_output.exit_code, 0);
    assert!(validate_output.stdout.contains("Status: invalid"));
    assert!(validate_output
        .stdout
        .contains("- ERROR missing-edge line 7: Missing edge."));
    assert_eq!(
        validate_requests
            .recv_timeout(Duration::from_secs(2))
            .expect("validate request")
            .path,
        "/workspace/api/flows/software-development%2Fimplement-change-request.dot/validate"
    );
}

#[test]
fn flow_get_wraps_json_and_raw_text_preserves_trailing_newline_rule() {
    let env = BTreeMap::new();
    let (json_base_url, json_requests) =
        serve_once(HttpResponse::text(200, "digraph G {\n  a -> b;\n}\n"));
    let json_output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "get",
            "--flow",
            "software-development/implement-change-request.dot",
            "--base-url",
            json_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(json_output.exit_code, 0);
    let payload = serde_json::from_str::<Value>(&json_output.stdout).expect("stdout json");
    assert_eq!(
        payload["name"],
        "software-development/implement-change-request.dot"
    );
    assert!(payload["content"]
        .as_str()
        .expect("content")
        .contains("a -> b"));
    assert_eq!(
        json_requests
            .recv_timeout(Duration::from_secs(2))
            .expect("json request")
            .path,
        "/workspace/api/flows/software-development%2Fimplement-change-request.dot/raw?surface=agent"
    );

    let (text_base_url, _text_requests) = serve_once(HttpResponse::text(200, "digraph G {}"));
    let text_output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "get",
            "--flow",
            "plain.dot",
            "--text",
            "--base-url",
            text_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(text_output.exit_code, 0);
    assert_eq!(text_output.stdout, "digraph G {}\n");
}

#[test]
fn http_errors_map_detail_payloads_and_validation_arrays() {
    let env = BTreeMap::new();
    let (missing_base_url, _missing_requests) = serve_once(HttpResponse::json(
        404,
        r#"{"detail":"Unknown flow: missing.dot"}"#,
    ));
    let missing_output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "describe",
            "--flow",
            "missing.dot",
            "--base-url",
            missing_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(missing_output.exit_code, 3);
    assert_eq!(missing_output.stdout, "");
    assert_eq!(
        missing_output.stderr,
        "{\"ok\": false, \"status_code\": 404, \"error\": \"Unknown flow: missing.dot\"}\n"
    );

    let (validation_base_url, _validation_requests) = serve_once(HttpResponse::json(
        422,
        r#"{"detail":[{"loc":["body","flow_name"],"msg":"Field required"},{"loc":["query","surface"],"msg":"Invalid surface"}]}"#,
    ));
    let validation_output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "list",
            "--base-url",
            validation_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(validation_output.exit_code, 1);
    assert_eq!(
        validation_output.stderr,
        "{\"ok\": false, \"status_code\": 422, \"error\": \"flow_name: Field required; query.surface: Invalid surface\"}\n"
    );

    let (top_level_error_base_url, _top_level_error_requests) = serve_once(HttpResponse::json(
        409,
        r#"{"error":"Conflict from top level","detail":"Lower priority detail"}"#,
    ));
    let top_level_error_output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "describe",
            "--flow",
            "conflict.dot",
            "--base-url",
            top_level_error_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(top_level_error_output.exit_code, 1);
    assert_eq!(
        top_level_error_output.stderr,
        "{\"ok\": false, \"status_code\": 409, \"error\": \"Conflict from top level\"}\n"
    );

    let (nested_error_base_url, _nested_error_requests) = serve_once(HttpResponse::json(
        400,
        r#"{"detail":{"error":"Nested detail error"}}"#,
    ));
    let nested_error_output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "describe",
            "--flow",
            "invalid.dot",
            "--base-url",
            nested_error_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(nested_error_output.exit_code, 1);
    assert_eq!(
        nested_error_output.stderr,
        "{\"ok\": false, \"status_code\": 400, \"error\": \"Nested detail error\"}\n"
    );

    let (text_error_base_url, _text_error_requests) =
        serve_once(HttpResponse::text(500, "Plain text failure\n"));
    let text_error_output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "describe",
            "--flow",
            "server-error.dot",
            "--base-url",
            text_error_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(text_error_output.exit_code, 1);
    assert_eq!(
        text_error_output.stderr,
        "{\"ok\": false, \"status_code\": 500, \"error\": \"Plain text failure\"}\n"
    );
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
            "--json",
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
            "--base-url",
            text_base_url.as_str(),
        ],
        &env,
    );
    assert_eq!(text_output.exit_code, 0);
    assert_eq!(text_output.stdout, "9 progress: done\n");
}

#[test]
fn flow_format_file_prints_readable_dot_without_source_checkout_guard() {
    let fixture = fixture_json("cli/flow-format-stdout.json");
    let expected_stdout = fixture["process"]["stdout"].as_str().expect("stdout");
    let temp_dir = temp_dir("flow-format-stdout");
    let flow_path = temp_dir.join("messy-flow.dot");
    fs::write(&flow_path, messy_flow_source()).expect("write flow");
    let env = BTreeMap::new();

    let output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "format",
            "--file",
            flow_path.to_str().expect("path"),
        ],
        &env,
    );

    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stdout, expected_stdout);
    assert_eq!(output.stderr, "");
    assert_eq!(
        fs::read_to_string(&flow_path).expect("read flow"),
        messy_flow_source()
    );
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn flow_format_file_accepts_argparse_equals_value_syntax() {
    let fixture = fixture_json("cli/flow-format-stdout.json");
    let expected_stdout = fixture["process"]["stdout"].as_str().expect("stdout");
    let temp_dir = temp_dir("flow-format-equals");
    let flow_path = temp_dir.join("messy-flow.dot");
    fs::write(&flow_path, messy_flow_source()).expect("write flow");
    let env = BTreeMap::new();
    let file_arg = format!("--file={}", flow_path.to_str().expect("path"));

    let output = run_with_args_and_env(["spark", "flow", "format", file_arg.as_str()], &env);

    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stdout, expected_stdout);
    assert_eq!(output.stderr, "");
    assert_eq!(
        fs::read_to_string(&flow_path).expect("read flow"),
        messy_flow_source()
    );
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn flow_format_file_write_replaces_only_target_file_contents() {
    let fixture = fixture_json("cli/flow-format-stdout.json");
    let expected_content = fixture["process"]["stdout"].as_str().expect("stdout");
    let temp_dir = temp_dir("flow-format-write");
    let flow_path = temp_dir.join("messy-write.dot");
    let sibling_path = temp_dir.join("untouched.txt");
    fs::write(&flow_path, messy_flow_source()).expect("write flow");
    fs::write(&sibling_path, "leave me alone\n").expect("write sibling");
    let env = BTreeMap::new();

    let output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "format",
            "--file",
            flow_path.to_str().expect("path"),
            "--write",
            "--text",
        ],
        &env,
    );

    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stdout, "");
    assert_eq!(output.stderr, "");
    assert_eq!(
        fs::read_to_string(&flow_path).expect("read flow"),
        expected_content
    );
    assert_eq!(
        fs::read_to_string(&sibling_path).expect("read sibling"),
        "leave me alone\n"
    );
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn flow_format_file_prints_python_repr_float_values() {
    let temp_dir = temp_dir("flow-format-floats");
    let flow_path = temp_dir.join("floats.dot");
    fs::write(
        &flow_path,
        "digraph Workflow { start [shape=Mdiamond, score=0.000001]; done [shape=Msquare]; start -> done [weight=10000000000000000.0]; }\n",
    )
    .expect("write flow");
    let env = BTreeMap::new();

    let output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "format",
            "--file",
            flow_path.to_str().expect("path"),
        ],
        &env,
    );

    assert_eq!(output.exit_code, 0);
    assert_eq!(
        output.stdout,
        "digraph Workflow {\n\n  start [score=1e-06, shape=\"Mdiamond\"];\n  start -> done [weight=1e+16];\n\n  done [shape=\"Msquare\"];\n}\n"
    );
    assert_eq!(output.stderr, "");
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn flow_format_file_write_is_idempotent_for_python_repr_float_values() {
    let expected_content = "digraph Workflow {\n\n  start [score=1e-06, shape=\"Mdiamond\"];\n  start -> done [weight=1e+16];\n\n  done [shape=\"Msquare\"];\n}\n";
    let temp_dir = temp_dir("flow-format-float-write");
    let flow_path = temp_dir.join("floats-write.dot");
    fs::write(
        &flow_path,
        "digraph Workflow { start [shape=Mdiamond, score=0.000001]; done [shape=Msquare]; start -> done [weight=10000000000000000.0]; }\n",
    )
    .expect("write flow");
    let env = BTreeMap::new();

    let write_output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "format",
            "--file",
            flow_path.to_str().expect("path"),
            "--write",
        ],
        &env,
    );

    assert_eq!(write_output.exit_code, 0);
    assert_eq!(write_output.stdout, "");
    assert_eq!(write_output.stderr, "");
    assert_eq!(
        fs::read_to_string(&flow_path).expect("read flow"),
        expected_content
    );

    let second_output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "format",
            "--file",
            flow_path.to_str().expect("path"),
        ],
        &env,
    );

    assert_eq!(second_output.exit_code, 0);
    assert_eq!(second_output.stdout, expected_content);
    assert_eq!(second_output.stderr, "");
    assert_eq!(
        fs::read_to_string(&flow_path).expect("read flow"),
        expected_content
    );
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn flow_format_file_preserves_same_line_node_declaration_order() {
    let temp_dir = temp_dir("flow-format-same-line-order");
    let flow_path = temp_dir.join("same-line.dot");
    fs::write(
        &flow_path,
        "digraph Workflow { b [shape=box]; a [shape=box]; }\n",
    )
    .expect("write flow");
    let env = BTreeMap::new();

    let output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "format",
            "--file",
            flow_path.to_str().expect("path"),
        ],
        &env,
    );

    assert_eq!(output.exit_code, 0);
    assert_eq!(
        output.stdout,
        "digraph Workflow {\n\n  b [shape=\"box\"];\n\n  a [shape=\"box\"];\n}\n"
    );
    assert_eq!(output.stderr, "");
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn flow_format_file_accepts_empty_equals_value_as_file_path() {
    let env = BTreeMap::new();

    let output = run_with_args_and_env(["spark", "flow", "format", "--file="], &env);

    assert_eq!(output.exit_code, 1);
    assert_eq!(output.stdout, "");
    assert!(output
        .stderr
        .starts_with("{\"ok\": false, \"error\": \"Unable to read flow file : "));
    assert!(output.stderr.contains("Is a directory"));
    assert!(!output
        .stderr
        .contains("the following arguments are required"));
}

#[test]
fn flow_format_file_rejects_missing_value_before_option() {
    let env = BTreeMap::new();

    let output = run_with_args_and_env(["spark", "flow", "format", "--file", "--write"], &env);

    assert_eq!(output.exit_code, 2);
    assert_eq!(output.stdout, "");
    assert_eq!(
        output.stderr,
        "usage: spark [-h] {convo,run,flow,trigger} ...\n\
spark: error: argument --file: expected one argument\n"
    );
}

#[test]
fn flow_format_invalid_dot_returns_compat_json_error() {
    let fixture = fixture_json("cli/flow-format-invalid.json");
    let expected_stderr = fixture["process"]["stderr"].as_str().expect("stderr");
    let temp_dir = temp_dir("flow-format-invalid");
    let flow_path = temp_dir.join("invalid-flow.dot");
    fs::write(&flow_path, "digraph Workflow { start -> }\n").expect("write flow");
    let env = BTreeMap::new();

    let output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "format",
            "--file",
            flow_path.to_str().expect("path"),
        ],
        &env,
    );

    assert_eq!(output.exit_code, 1);
    assert_eq!(output.stdout, "");
    assert_eq!(output.stderr, expected_stderr);
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn flow_format_missing_file_returns_json_error() {
    let temp_dir = temp_dir("flow-format-missing");
    let flow_path = temp_dir.join("missing.dot");
    let env = BTreeMap::new();

    let output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "format",
            "--file",
            flow_path.to_str().expect("path"),
        ],
        &env,
    );

    assert_eq!(output.exit_code, 1);
    assert_eq!(output.stdout, "");
    assert_eq!(
        output.stderr,
        format!(
            "{{\"ok\": false, \"error\": \"Flow file not found: {}\"}}\n",
            flow_path.display()
        )
    );
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn flow_validate_file_json_uses_local_preview_without_source_checkout_guard() {
    let temp_dir = temp_dir("flow-validate-success");
    let flow_path = temp_dir.join("valid-flow.dot");
    fs::write(&flow_path, valid_flow_source()).expect("write flow");
    let env = BTreeMap::new();

    let output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "validate",
            "--file",
            flow_path.to_str().expect("path"),
        ],
        &env,
    );

    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stderr, "");
    let payload: Value = serde_json::from_str(&output.stdout).expect("valid json");
    assert_eq!(payload["name"], "valid-flow.dot");
    assert_eq!(
        payload["path"],
        fs::canonicalize(&flow_path)
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
fn flow_validate_file_text_renders_diagnostics() {
    let temp_dir = temp_dir("flow-validate-text");
    let flow_path = temp_dir.join("validation-error-flow.dot");
    fs::write(&flow_path, validation_error_flow_source()).expect("write flow");
    let env = BTreeMap::new();

    let output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "validate",
            "--file",
            flow_path.to_str().expect("path"),
            "--text",
        ],
        &env,
    );

    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stderr, "");
    assert_eq!(
        output.stdout,
        format!(
            "Name: validation-error-flow.dot\n\
Path: {}\n\
Status: validation_error\n\
Diagnostics: 3\n\
Errors: 3\n\
- ERROR start_node: pipeline must have exactly one start node, found 0\n\
- ERROR terminal_node: pipeline must have exactly one exit node, found 0\n\
- ERROR node_has_outgoing_edge line 2: node 'task' must declare at least one outgoing edge\n",
            fs::canonicalize(&flow_path)
                .expect("canonical path")
                .display()
        )
    );
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn flow_validate_file_and_flow_are_mutually_exclusive() {
    let env = BTreeMap::new();

    let output = run_with_args_and_env(
        [
            "spark",
            "flow",
            "validate",
            "--flow",
            "examples/simple-linear.dot",
            "--file",
            "local.dot",
        ],
        &env,
    );

    assert_eq!(output.exit_code, 2);
    assert_eq!(output.stdout, "");
    assert_eq!(
        output.stderr,
        "usage: spark [-h] {convo,run,flow,trigger} ...\n\
spark: error: argument --file: not allowed with argument --flow\n"
    );
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
    assert_eq!(env_plan.path, "/workspace/api/flows?surface=agent");

    let explicit_plan = request_plan_with_args_env_and_stdin(
        [
            "spark",
            "flow",
            "describe",
            "--flow=software-development/implement-change-request.dot",
            "--base-url=http://127.0.0.1:8020",
        ],
        &env,
        "",
    )
    .expect("flow describe plan");
    assert_eq!(explicit_plan.base_url, "http://127.0.0.1:8020");
    assert_eq!(
        explicit_plan.path,
        "/workspace/api/flows/software-development%2Fimplement-change-request.dot?surface=agent"
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
        serde_json::json!({"enabled": true, "id": "nightly"})
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
        "{\"ok\": false, \"error\": \"Trigger payload must be a JSON object.\"}\n"
    );
}

#[test]
fn trigger_commands_execute_http_requests_and_render_text() {
    let env = BTreeMap::new();
    let (list_base_url, list_requests) = serve_once(HttpResponse::json(
        200,
        r#"[{"id":"trigger-123","name":"Nightly","enabled":true,"protected":false,"source_type":"webhook","action":{"flow_name":"ops/run.dot"},"state":{}}]"#,
    ));
    let list_output = run_with_args_and_env(
        [
            "spark",
            "trigger",
            "list",
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
    assert_eq!(list_request.path, "/workspace/api/triggers");
    assert_eq!(list_request.body, "");

    let (list_text_base_url, _list_text_requests) = serve_once(HttpResponse::json(
        200,
        r#"[{"id":"trigger-123","name":"Nightly","enabled":true,"protected":false,"source_type":"webhook","action":{"flow_name":"ops/run.dot"},"state":{}}]"#,
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
        "trigger-123: Nightly [webhook] -> ops/run.dot\n  enabled=True protected=False\n"
    );

    let (describe_base_url, describe_requests) = serve_once(HttpResponse::json(
        200,
        r#"{"id":"trigger-123","name":"Nightly","enabled":true,"protected":false,"source_type":"webhook","action":{"flow_name":"ops/run.dot","project_path":"/repo"},"state":{"last_fired_at":null,"last_result":null,"next_run_at":null},"webhook_secret":"secret-123"}"#,
    ));
    let describe_output = run_with_args_and_env(
        [
            "spark",
            "trigger",
            "describe",
            "--id",
            "trigger-123",
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
Source Type: webhook\n\
Enabled: True\n\
Protected: False\n\
Flow Target: ops/run.dot\n\
Project Target: /repo\n\
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
        "/workspace/api/triggers/trigger-123"
    );
}

#[test]
fn trigger_create_update_delete_use_payloads_and_percent_encoded_ids() {
    let env = BTreeMap::new();
    let temp_dir = temp_dir("trigger-command-payloads");
    let payload_path = temp_dir.join("trigger.json");
    fs::write(
        &payload_path,
        r#"{"name":"Nightly","source_type":"webhook","action":{"flow_name":"ops/run.dot"},"source":{}}"#,
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
            "--json",
            payload_path.to_str().expect("payload path"),
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
            "name": "Nightly",
            "source_type": "webhook",
            "action": {"flow_name": "ops/run.dot"},
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
            "--id",
            "trigger/custom",
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
        "/workspace/api/triggers/trigger%2Fcustom"
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
            "--id",
            "trigger/custom",
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
        "/workspace/api/triggers/trigger%2Fcustom"
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
            "--id",
            "protected",
            "--base-url",
            base_url.as_str(),
        ],
        &env,
    );

    assert_eq!(output.exit_code, 1);
    assert_eq!(output.stdout, "");
    assert_eq!(
        output.stderr,
        "{\"ok\": false, \"status_code\": 400, \"error\": \"Protected triggers cannot be deleted.\"}\n"
    );
}

#[test]
fn launch_goal_sources_are_mutually_exclusive() {
    let env = BTreeMap::new();

    let output = run_with_args_and_env(
        [
            "spark",
            "run",
            "launch",
            "--flow",
            "test.dot",
            "--summary",
            "Summary",
            "--project",
            "/tmp/project",
            "--goal",
            "inline",
            "--goal-file",
            "goal.txt",
        ],
        &env,
    );

    assert_eq!(output.exit_code, 2);
    assert_eq!(output.stdout, "");
    assert_eq!(
        output.stderr,
        "usage: spark [-h] {convo,run,flow,trigger} ...\n\
spark: error: argument --goal-file: not allowed with argument --goal\n"
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

fn fixture_json(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("crates/test-fixtures/compat")
        .join(name);
    serde_json::from_str(&fs::read_to_string(&path).expect("fixture readable"))
        .expect("fixture json")
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

fn messy_flow_source() -> &'static str {
    "\ndigraph Workflow {\n  done [shape=Msquare];\n  task [shape=box, prompt=\"Do work\"];\n  start [shape=Mdiamond];\n  task -> done;\n  start -> task;\n}\n"
}

fn valid_flow_source() -> &'static str {
    "digraph Workflow {\n  start [shape=Mdiamond];\n  task [shape=box, prompt=\"Do work\"];\n  done [shape=Msquare];\n  start -> task;\n  task -> done;\n}\n"
}

fn validation_error_flow_source() -> &'static str {
    "digraph Workflow {\n  task [shape=box, prompt=\"No start or done\"];\n}\n"
}
