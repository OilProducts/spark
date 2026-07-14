use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use attractor_core::{
    FlowDefinition, FlowEdge, FlowNode, LaunchContext, NodeConfig, NodeKind, Outcome,
    OutcomeStatus, RunRecord,
};
use attractor_runtime::{
    prepare_fresh_run, ExecuteRunRequest, ExecutionStart, PipelineExecutor, RunStore,
    RuntimeHandlerRunner, HANDLER_CODERGEN,
};

const CANNED_SUMMARY: &str = "## Result\n\nBuilt the thing; no deviations.";

#[derive(Debug, Clone, Default)]
struct ObservedSummaryCall {
    prompt: String,
    workdir: PathBuf,
    node_id: String,
}

fn summary_exit(result_summary: bool, prompt: Option<&str>) -> FlowNode {
    FlowNode {
        kind: NodeKind::Exit,
        config: Some(NodeConfig::Exit {
            result_summary,
            result_summary_prompt: prompt.map(str::to_string),
        }),
        ..FlowNode::default()
    }
}

fn tool_node(command: &str) -> FlowNode {
    FlowNode {
        kind: NodeKind::Tool,
        config: Some(NodeConfig::Tool {
            command: command.to_string(),
            env_map: BTreeMap::new(),
            output_map: BTreeMap::new(),
        }),
        ..FlowNode::default()
    }
}

fn flow_with_exit(command: &str, exit: FlowNode) -> FlowDefinition {
    FlowDefinition {
        schema_version: "1".to_string(),
        id: "summary-contract".to_string(),
        title: "Summary Contract".to_string(),
        nodes: [
            (
                "start".to_string(),
                FlowNode {
                    kind: NodeKind::Start,
                    config: Some(NodeConfig::Start {}),
                    ..FlowNode::default()
                },
            ),
            ("work".to_string(), tool_node(command)),
            ("done".to_string(), exit),
        ]
        .into_iter()
        .collect(),
        edges: vec![
            FlowEdge {
                from: "start".to_string(),
                to: "work".to_string(),
                ..FlowEdge::default()
            },
            FlowEdge {
                from: "work".to_string(),
                to: "done".to_string(),
                condition: "outcome=success".to_string(),
                ..FlowEdge::default()
            },
        ],
        ..FlowDefinition::default()
    }
}

fn execute_flow(
    runs_dir: &std::path::Path,
    workdir: &std::path::Path,
    run_id: &str,
    flow: FlowDefinition,
) -> (String, Arc<Mutex<Vec<ObservedSummaryCall>>>, RunStore) {
    let observed = Arc::new(Mutex::new(Vec::<ObservedSummaryCall>::new()));
    let recorder = Arc::clone(&observed);
    let mut runner = RuntimeHandlerRunner::new();
    runner.register_thread_safe_handler_fn(HANDLER_CODERGEN, move |runtime| {
        recorder
            .lock()
            .expect("record call")
            .push(ObservedSummaryCall {
                prompt: runtime.prompt.clone(),
                workdir: runtime.run_workdir.clone(),
                node_id: runtime.node_id.clone(),
            });
        Ok(Outcome {
            status: OutcomeStatus::Success,
            raw_response_text: CANNED_SUMMARY.to_string(),
            ..Outcome::new(OutcomeStatus::Success)
        })
    });
    let store = RunStore::for_runs_dir(runs_dir.to_path_buf());
    let mut record = RunRecord::new(run_id, workdir.to_string_lossy());
    record.flow_name = flow.title.clone();
    let launch_context = LaunchContext::empty();
    let runtime_context = attractor_core::ContextMap::from([(
        "internal.run_workdir".to_string(),
        serde_json::json!(workdir.to_string_lossy().to_string()),
    )]);
    let paths = prepare_fresh_run(
        &store,
        &record,
        &flow,
        None,
        None,
        &launch_context,
        &runtime_context,
    )
    .expect("prepare run");
    let mut executor = PipelineExecutor::new(runner);
    let result = executor
        .execute(ExecuteRunRequest {
            store: store.clone(),
            record,
            flow,
            flow_source: None,
            flow_definition_json: None,
            launch_context,
            runtime_context,
            max_steps: None,
            start: ExecutionStart::Prepared {
                paths: paths.clone(),
            },
        })
        .expect("execute run");
    (result.status, observed, store)
}

fn read_result(store: &RunStore, run_id: &str) -> attractor_core::RunResult {
    let bundle = store
        .read_run_bundle(run_id)
        .expect("read bundle")
        .expect("bundle exists");
    store
        .read_result(&bundle.paths)
        .expect("read result")
        .expect("result exists")
}

#[test]
fn opted_in_exit_summarizes_completed_run_from_run_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workdir = temp.path().join("project");
    std::fs::create_dir_all(&workdir).expect("workdir");
    let (status, observed, store) = execute_flow(
        &temp.path().join("runs"),
        &workdir,
        "run-summary-ok",
        flow_with_exit("printf worked", summary_exit(true, None)),
    );
    assert_eq!(status, "completed");

    let calls = observed.lock().expect("calls").clone();
    assert_eq!(calls.len(), 1, "exactly one summarizer execution");
    assert_eq!(calls[0].node_id, "result_summary");
    assert!(
        calls[0].prompt.contains("recorded artifacts"),
        "{}",
        calls[0].prompt
    );
    let run_root = calls[0].workdir.clone();
    assert!(
        run_root.ends_with("run-summary-ok"),
        "summarizer must run inside the run directory: {}",
        run_root.display()
    );
    assert!(
        run_root.join("events.jsonl").exists(),
        "run directory holds the transcripts the summarizer reads"
    );

    let result = read_result(&store, "run-summary-ok");
    assert_eq!(result.state, "ready");
    assert_eq!(result.display_mode.as_deref(), Some("summary"));
    assert_eq!(result.body_markdown.trim(), CANNED_SUMMARY);
    assert!(result.summary_enabled);
}

#[test]
fn opted_in_exit_summarizes_failed_run_and_preserves_failure_reason() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workdir = temp.path().join("project");
    std::fs::create_dir_all(&workdir).expect("workdir");
    let (status, observed, store) = execute_flow(
        &temp.path().join("runs"),
        &workdir,
        "run-summary-fail",
        flow_with_exit(
            "echo boom-reason >&2; exit 1",
            summary_exit(true, Some("Explain what blocked this run.")),
        ),
    );
    assert_eq!(status, "failed");

    let calls = observed.lock().expect("calls").clone();
    assert_eq!(
        calls.len(),
        1,
        "failed runs summarize through the opted-in exit"
    );
    assert_eq!(calls[0].prompt, "Explain what blocked this run.");

    let result = read_result(&store, "run-summary-fail");
    assert_eq!(result.status, "failed");
    assert_eq!(result.state, "ready");
    assert_eq!(result.display_mode.as_deref(), Some("summary"));
    assert_eq!(result.body_markdown.trim(), CANNED_SUMMARY);
    assert_eq!(result.error.as_deref(), Some("boom-reason"));
}

#[test]
fn exit_without_opt_in_keeps_source_artifact_result() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workdir = temp.path().join("project");
    std::fs::create_dir_all(&workdir).expect("workdir");
    let (status, observed, store) = execute_flow(
        &temp.path().join("runs"),
        &workdir,
        "run-summary-off",
        flow_with_exit("printf plain-output", summary_exit(false, None)),
    );
    assert_eq!(status, "completed");
    assert!(
        observed.lock().expect("calls").is_empty(),
        "no summarizer execution without opt-in"
    );

    let result = read_result(&store, "run-summary-off");
    assert_eq!(result.display_mode.as_deref(), Some("raw"));
    assert_eq!(result.source_node_id.as_deref(), Some("work"));
    assert!(!result.summary_enabled);
}

#[test]
fn summarizer_failure_falls_back_and_records_the_error() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workdir = temp.path().join("project");
    std::fs::create_dir_all(&workdir).expect("workdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let mut runner = RuntimeHandlerRunner::new();
    runner.register_thread_safe_handler_fn(HANDLER_CODERGEN, |_runtime| {
        Ok(Outcome {
            status: OutcomeStatus::Fail,
            failure_reason: "summarizer backend unavailable".to_string(),
            ..Outcome::new(OutcomeStatus::Fail)
        })
    });
    let flow = flow_with_exit("printf still-here", summary_exit(true, None));
    let mut record = RunRecord::new("run-summary-broken", workdir.to_string_lossy());
    record.flow_name = flow.title.clone();
    let launch_context = LaunchContext::empty();
    let runtime_context = attractor_core::ContextMap::from([(
        "internal.run_workdir".to_string(),
        serde_json::json!(workdir.to_string_lossy().to_string()),
    )]);
    let paths = prepare_fresh_run(
        &store,
        &record,
        &flow,
        None,
        None,
        &launch_context,
        &runtime_context,
    )
    .expect("prepare run");
    let status = PipelineExecutor::new(runner)
        .execute(ExecuteRunRequest {
            store: store.clone(),
            record,
            flow,
            flow_source: None,
            flow_definition_json: None,
            launch_context,
            runtime_context,
            max_steps: None,
            start: ExecutionStart::Prepared { paths },
        })
        .expect("execute run")
        .status;
    assert_eq!(status, "completed", "summary failure must not fail the run");

    let result = read_result(&store, "run-summary-broken");
    assert_eq!(result.display_mode.as_deref(), Some("raw"));
    assert_eq!(result.body_markdown.trim(), "still-here");
    assert!(result.summary_enabled);
    assert_eq!(
        result.summary_error.as_deref(),
        Some("summarizer backend unavailable")
    );
}

#[test]
fn summarizer_resolves_through_the_real_codergen_graph_lookup() {
    // The stubbed-handler tests bypass RuntimeCodergen's node lookup, which
    // rejected the synthetic summary node in production; the default runner's
    // simulation backend exercises that lookup for real.
    let temp = tempfile::tempdir().expect("tempdir");
    let workdir = temp.path().join("project");
    std::fs::create_dir_all(&workdir).expect("workdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let flow = flow_with_exit("printf worked", summary_exit(true, None));
    let mut record = RunRecord::new("run-summary-lookup", workdir.to_string_lossy());
    record.flow_name = flow.title.clone();
    let launch_context = LaunchContext::empty();
    let runtime_context = attractor_core::ContextMap::from([(
        "internal.run_workdir".to_string(),
        serde_json::json!(workdir.to_string_lossy().to_string()),
    )]);
    let paths = prepare_fresh_run(
        &store,
        &record,
        &flow,
        None,
        None,
        &launch_context,
        &runtime_context,
    )
    .expect("prepare run");
    let status = PipelineExecutor::new(RuntimeHandlerRunner::new())
        .execute(ExecuteRunRequest {
            store: store.clone(),
            record,
            flow,
            flow_source: None,
            flow_definition_json: None,
            launch_context,
            runtime_context,
            max_steps: None,
            start: ExecutionStart::Prepared { paths },
        })
        .expect("execute run")
        .status;
    assert_eq!(status, "completed");

    let result = read_result(&store, "run-summary-lookup");
    assert_eq!(
        result.summary_error, None,
        "the synthetic node must resolve through the flow graph"
    );
    assert_eq!(result.display_mode.as_deref(), Some("summary"));
    assert_eq!(
        result.body_markdown.trim(),
        "Stage completed: result_summary",
        "the simulation backend's summary output becomes the result body"
    );
}
