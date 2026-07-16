use std::path::Path;
use std::time::{Duration, Instant};

use attractor_core::{
    FlowDefinition, FlowEdge, FlowNode, LaunchContext, NodeConfig, NodeContracts, NodeKind,
    Outcome, OutcomeStatus, RunRecord,
};
use attractor_runtime::{
    disk_execution_control, human_gate_answered_event, prepare_fresh_run, ExecuteRunRequest,
    ExecutionStart, HumanAnswer, PipelineExecutor, QueueInterviewer, RunStore, RuntimeControls,
    RuntimeHandlerRunner, HANDLER_CODERGEN,
};
use serde_json::Value;

fn agent_node(prompt: &str) -> FlowNode {
    FlowNode {
        kind: NodeKind::AgentTask,
        config: Some(NodeConfig::AgentTask {
            prompt: prompt.to_string(),
        }),
        ..FlowNode::default()
    }
}

fn edge(from: &str, to: &str, label: &str) -> FlowEdge {
    FlowEdge {
        from: from.to_string(),
        to: to.to_string(),
        label: label.to_string(),
        ..FlowEdge::default()
    }
}

fn gate_flow() -> FlowDefinition {
    FlowDefinition {
        schema_version: "1".to_string(),
        id: "gate-contract".to_string(),
        title: "Gate Contract".to_string(),
        nodes: [
            (
                "start".to_string(),
                FlowNode {
                    kind: NodeKind::Start,
                    config: Some(NodeConfig::Start {}),
                    ..FlowNode::default()
                },
            ),
            (
                "review".to_string(),
                FlowNode {
                    kind: NodeKind::HumanGate,
                    config: Some(NodeConfig::HumanGate {
                        prompt: "Approve the change?".to_string(),
                        decisions: Vec::new(),
                    }),
                    ..FlowNode::default()
                },
            ),
            (
                "prep".to_string(),
                FlowNode {
                    contracts: Some(NodeContracts {
                        writes_context: vec!["context.review.summary".to_string()],
                        ..NodeContracts::default()
                    }),
                    ..agent_node("prepare summary")
                },
            ),
            ("approved".to_string(), agent_node("ship")),
            ("rejected".to_string(), agent_node("revise")),
            (
                "done".to_string(),
                FlowNode {
                    kind: NodeKind::Exit,
                    config: Some(NodeConfig::Exit {
                        result_summary: false,
                        result_summary_prompt: None,
                    }),
                    ..FlowNode::default()
                },
            ),
        ]
        .into_iter()
        .collect(),
        edges: vec![
            edge("start", "prep", ""),
            edge("prep", "review", ""),
            edge("review", "approved", "Approve"),
            edge("review", "rejected", "Reject"),
            edge("approved", "done", ""),
            edge("rejected", "done", ""),
        ],
        ..FlowDefinition::default()
    }
}

fn temp_store(temp: &tempfile::TempDir) -> RunStore {
    RunStore::for_runs_dir(temp.path().join("spark-home/attractor/runs"))
}

fn record(run_id: &str, project_path: &Path) -> RunRecord {
    let mut record = RunRecord::new(run_id, project_path.to_string_lossy());
    record.flow_name = "gate-contract".to_string();
    record.model = "compat-model".to_string();
    record.started_at = "2026-07-08T10:00:00Z".to_string();
    record
}

struct GateHarness {
    store: RunStore,
    paths: attractor_runtime::paths::RunRootPaths,
    handle: std::thread::JoinHandle<
        attractor_runtime::Result<attractor_runtime::PipelineExecutionResult>,
    >,
}

fn spawn_gate_run(
    temp: &tempfile::TempDir,
    run_id: &str,
    runner: RuntimeHandlerRunner,
) -> GateHarness {
    let store = temp_store(temp);
    let project_path = temp.path().join("project");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let flow = gate_flow();
    let run_record = record(run_id, &project_path);
    let paths = prepare_fresh_run(
        &store,
        &run_record,
        &flow,
        None,
        None,
        &LaunchContext::empty(),
        &Default::default(),
    )
    .expect("prepare run");

    let executor_store = store.clone();
    let control_paths = paths.clone();
    let start_paths = paths.clone();
    let handle = std::thread::spawn(move || {
        let mut executor =
            PipelineExecutor::with_control(runner, disk_execution_control(control_paths));
        executor.execute(ExecuteRunRequest {
            store: executor_store,
            record: run_record,
            flow,
            flow_source: None,
            flow_definition_json: None,
            launch_context: LaunchContext::empty(),
            runtime_context: Default::default(),
            max_steps: None,
            start: ExecutionStart::Prepared { paths: start_paths },
        })
    });
    GateHarness {
        store,
        paths,
        handle,
    }
}

fn wait_for_pending_gate(harness: &GateHarness) -> (String, String) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(
            Instant::now() < deadline,
            "gate never published a pending question",
        );
        let events = harness
            .store
            .read_raw_events(&harness.paths)
            .unwrap_or_default();
        if let Some(event) = events.iter().find(|event| event.event_type == "human_gate") {
            let question_id = event
                .payload
                .get("question_id")
                .and_then(Value::as_str)
                .expect("question id")
                .to_string();
            let status = harness
                .store
                .read_run_record(&harness.paths)
                .expect("record")
                .expect("record")
                .status;
            if status == "waiting" {
                return (question_id, status);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn blocking_gate_waits_for_journaled_answer_and_routes_it() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut runner = RuntimeHandlerRunner::new().with_blocking_human_gates();
    runner.register_thread_safe_handler_fn(HANDLER_CODERGEN, |runtime| {
        let mut outcome = Outcome::new(OutcomeStatus::Success);
        if runtime.node_id == "prep" {
            outcome.context_updates.insert(
                "context.review.summary".to_string(),
                serde_json::json!("The fix removes the stale lock file."),
            );
        }
        Ok(outcome)
    });
    let harness = spawn_gate_run(&temp, "run-gate-answer", runner);

    let (question_id, status) = wait_for_pending_gate(&harness);
    assert_eq!(status, "waiting");
    assert_eq!(question_id, format!("review-{}", 2));

    // The pending question carries the edge-derived options.
    let events = harness
        .store
        .read_raw_events(&harness.paths)
        .expect("events");
    let pending = events
        .iter()
        .find(|event| event.event_type == "human_gate")
        .expect("pending event");
    assert_eq!(pending.payload["prompt"], "Approve the change?");
    assert_eq!(pending.payload["options"][0]["label"], "Approve");
    assert_eq!(pending.payload["flow_name"], "gate-contract");
    let details = pending.payload["details"].as_str().expect("gate details");
    assert!(
        details.contains("The fix removes the stale lock file."),
        "details must present the preceding node's context output: {details}"
    );
    assert!(
        details.contains("### review.summary"),
        "details use the context key as a heading: {details}"
    );

    // Answer exactly like the API route does: journal an InterviewCompleted.
    harness
        .store
        .append_event(
            &harness.paths,
            human_gate_answered_event(
                "run-gate-answer",
                &question_id,
                Some("review".to_string()),
                Some("gate-contract".to_string()),
                Some("Approve the change?".to_string()),
                "Approve",
                Some("Looks right, but double-check the lock cleanup.".to_string()),
            ),
        )
        .expect("append answer");

    let result = harness
        .handle
        .join()
        .expect("executor thread")
        .expect("execution result");
    assert_eq!(result.status, "completed");
    assert!(
        result.completed_nodes.contains(&"approved".to_string()),
        "answer must route down the Approve edge: {:?}",
        result.completed_nodes,
    );
    assert!(!result.completed_nodes.contains(&"rejected".to_string()));
    let final_record = harness
        .store
        .read_run_record(&harness.paths)
        .expect("record")
        .expect("record");
    assert_eq!(final_record.status, "completed");

    // The note travels with the selection into the gate's context updates so
    // downstream nodes can read it as `human.gate.note`.
    let gate_status_path = harness.paths.logs_dir().join("review").join("status.json");
    let gate_status: Value = serde_json::from_str(
        &std::fs::read_to_string(&gate_status_path).expect("gate status artifact"),
    )
    .expect("gate status json");
    assert_eq!(
        gate_status["context_updates"]["human.gate.label"],
        "Approve"
    );
    assert_eq!(
        gate_status["context_updates"]["human.gate.note"],
        "Looks right, but double-check the lock cleanup."
    );
}

#[test]
fn cancel_while_waiting_finalizes_the_run_canceled() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runner = RuntimeHandlerRunner::new().with_blocking_human_gates();
    let harness = spawn_gate_run(&temp, "run-gate-cancel", runner);

    wait_for_pending_gate(&harness);
    RuntimeControls::new(harness.store.clone())
        .request_cancel("run-gate-cancel")
        .expect("cancel");

    let result = harness
        .handle
        .join()
        .expect("executor thread")
        .expect("execution result");
    assert_eq!(result.status, "canceled");
    let final_record = harness
        .store
        .read_run_record(&harness.paths)
        .expect("record")
        .expect("record");
    assert_eq!(final_record.status, "canceled");
}

#[test]
fn queued_interviewer_answers_bypass_the_wait() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runner =
        RuntimeHandlerRunner::with_interviewer(QueueInterviewer::new([HumanAnswer::selected(
            "Reject",
        )]))
        .with_blocking_human_gates();
    let harness = spawn_gate_run(&temp, "run-gate-queued", runner);

    let result = harness
        .handle
        .join()
        .expect("executor thread")
        .expect("execution result");
    assert_eq!(result.status, "completed");
    assert!(result.completed_nodes.contains(&"rejected".to_string()));
    // No pending question was ever published.
    let events = harness
        .store
        .read_raw_events(&harness.paths)
        .expect("events");
    assert!(!events.iter().any(|event| event.event_type == "human_gate"));
}

#[test]
fn without_the_flag_gates_keep_skip_semantics() {
    let temp = tempfile::tempdir().expect("tempdir");
    let harness = spawn_gate_run(&temp, "run-gate-skip", RuntimeHandlerRunner::new());

    let result = harness
        .handle
        .join()
        .expect("executor thread")
        .expect("execution result");
    // The empty default interviewer skips, which fails the gate immediately.
    assert_eq!(result.status, "failed");
    let events = harness
        .store
        .read_raw_events(&harness.paths)
        .expect("events");
    assert!(!events.iter().any(|event| event.event_type == "human_gate"));
}
