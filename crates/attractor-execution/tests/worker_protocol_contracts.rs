use attractor_core::{ContextMap, Outcome, OutcomeStatus};
use attractor_dsl::parse_dot;
use attractor_execution::{run_worker_node_from_reader_writer, WorkerNodeRequest};
use attractor_runtime::RuntimeHandlerRunner;
use serde_json::{json, Value};

#[test]
fn worker_run_node_executes_request_and_returns_result_frame() {
    let graph = parse_dot(
        r#"
        digraph G {
          work [shape=box, type="custom.success"]
        }
        "#,
    )
    .expect("dot parses");
    let mut runner = RuntimeHandlerRunner::new();
    runner.register_static_handler(
        "custom.success",
        Outcome {
            status: OutcomeStatus::Success,
            context_updates: ContextMap::from([("context.worker".to_string(), json!("ok"))]),
            notes: "done".to_string(),
            ..Outcome::new(OutcomeStatus::Success)
        },
    );
    let request = WorkerNodeRequest {
        run_id: "run-worker".to_string(),
        graph,
        node_id: "work".to_string(),
        prompt: "Do work".to_string(),
        context: ContextMap::from([("context.input".to_string(), json!("value"))]),
        context_logs: Vec::new(),
        logs_root: None,
        working_dir: ".".into(),
        backend_name: None,
        model: None,
        config_dir: None,
    };
    let input = serde_json::to_string(&request).expect("request json") + "\n";
    let mut output = Vec::new();

    let exit_code = run_worker_node_from_reader_writer(input.as_bytes(), &mut output, runner);

    assert_eq!(exit_code, 0);
    let frame: Value = serde_json::from_slice(&output).expect("frame json");
    assert_eq!(frame["type"], json!("result"));
    assert_eq!(frame["outcome"]["status"], json!("success"));
    assert_eq!(frame["outcome"]["notes"], json!("done"));
    assert_eq!(frame["context"]["context.input"], json!("value"));
    assert_eq!(frame["context"]["context.worker"], json!("ok"));
}

#[test]
fn worker_protocol_errors_return_nonretryable_runtime_failure() {
    let mut output = Vec::new();

    let exit_code = run_worker_node_from_reader_writer(
        b"{not valid json}\n".as_slice(),
        &mut output,
        RuntimeHandlerRunner::new(),
    );

    assert_eq!(exit_code, 1);
    let frame: Value = serde_json::from_slice(&output).expect("frame json");
    assert_eq!(frame["type"], json!("result"));
    assert_eq!(frame["outcome"]["status"], json!("fail"));
    assert_eq!(frame["outcome"]["retryable"], json!(false));
    assert_eq!(frame["outcome"]["failure_kind"], json!("runtime"));
    assert!(frame["outcome"]["failure_reason"]
        .as_str()
        .expect("failure reason")
        .contains("invalid worker request JSON"));
}
