use std::path::Path;
use std::time::{Duration, Instant};

use attractor_core::{FlowDefinition, LaunchContext, Outcome, RunRecord};
use attractor_runtime::{
    disk_execution_control, human_gate_answered_event, prepare_fresh_run, ExecuteRunRequest,
    ExecutionStart, HumanAnswer, PipelineExecutor, QueueInterviewer, RunStore, RuntimeControls,
    RuntimeHandlerRunner, HANDLER_CODERGEN,
};
use serde_json::{json, Value};

fn agent(prompt: &str, to: &str) -> Value {
    json!({"kind": "agent_task", "config": {"kind": "agent_task", "prompt": prompt},
        "contracts": {"response": "decision_envelope"},
        "routes": {"done": {"to": to, "description": "The step is complete."}}})
}

fn gate_flow() -> FlowDefinition {
    let mut prep = agent("prepare summary", "review");
    prep["contracts"]["writes_context"] = json!(["context.review.summary"]);
    serde_json::from_value(json!({
        "schema_version": "2",
        "id": "gate-contract",
        "title": "Gate Contract",
        "nodes": {
            "start": {"kind": "start", "config": {"kind": "start"},
                "routes": {"next": {"to": "prep", "description": "Prepare the summary."}}},
            "prep": prep,
            "review": {
                "kind": "human_gate",
                "config": {"kind": "human_gate", "prompt": "Approve the change?"},
                "routes": {
                    "approve": {"to": "approved", "description": "Ship the change.", "label": "Approve"},
                    "reject": {"to": "rejected", "description": "Revise the change.", "label": "Reject"}
                }
            },
            "approved": agent("ship", "shipped"),
            "rejected": agent("revise", "revised"),
            "shipped": {"kind": "exit", "config": {"kind": "exit",
                "result": {"name": "shipped", "outcome": "success"}}},
            "revised": {"kind": "exit", "config": {"kind": "exit",
                "result": {"name": "rejected", "outcome": "failure"}}}
        }
    }))
    .expect("flow json")
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
    spawn_flow_run(temp, run_id, runner, gate_flow())
}

fn spawn_flow_run(
    temp: &tempfile::TempDir,
    run_id: &str,
    runner: RuntimeHandlerRunner,
    flow: FlowDefinition,
) -> GateHarness {
    let store = temp_store(temp);
    let project_path = temp.path().join("project");
    std::fs::create_dir_all(&project_path).expect("project dir");
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

fn join_run(harness: GateHarness) -> attractor_runtime::PipelineExecutionResult {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !harness.handle.is_finished() {
        assert!(Instant::now() < deadline, "run never finished");
        std::thread::sleep(Duration::from_millis(10));
    }
    harness
        .handle
        .join()
        .expect("executor thread")
        .expect("execution result")
}

fn final_record(store: &RunStore, paths: &attractor_runtime::paths::RunRootPaths) -> RunRecord {
    store
        .read_run_record(paths)
        .expect("record")
        .expect("record")
}

/// The durable `status.json` of `node_id`'s only execution.
fn node_status(
    store: &RunStore,
    paths: &attractor_runtime::paths::RunRootPaths,
    node_id: &str,
) -> Value {
    let execution = store
        .list_node_executions(paths)
        .expect("execution inventory")
        .into_iter()
        .find(|execution| execution.node_id == node_id)
        .unwrap_or_else(|| panic!("{node_id} execution"));
    let root = store
        .node_execution_root(
            paths,
            &execution.node_id,
            execution.stage_index,
            execution.attempt,
        )
        .expect("execution root");
    serde_json::from_str(
        &std::fs::read_to_string(root.join("status.json")).expect("status artifact"),
    )
    .expect("status json")
}

fn answer(harness: &GateHarness, run_id: &str, question_id: &str, value: &str, note: Option<&str>) {
    harness
        .store
        .append_event(
            &harness.paths,
            human_gate_answered_event(
                run_id,
                question_id,
                Some("review".to_string()),
                Some("gate-contract".to_string()),
                Some("Approve the change?".to_string()),
                value,
                note.map(str::to_string),
            ),
        )
        .expect("append answer");
}

fn stage_completed(harness: &GateHarness, node_id: &str) -> bool {
    harness
        .store
        .read_raw_events(&harness.paths)
        .expect("events")
        .iter()
        .any(|event| event.event_type == "StageCompleted" && event.payload["node_id"] == node_id)
}

#[test]
fn blocking_gate_waits_for_journaled_answer_and_selects_its_decision() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut runner = RuntimeHandlerRunner::new().with_blocking_human_gates();
    runner.register_thread_safe_handler_fn(HANDLER_CODERGEN, |runtime| {
        let mut outcome = Outcome::completed("done");
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

    // The pending question offers the gate's declared decisions: the
    // decision id is the value, the route label the button text.
    let events = harness
        .store
        .read_raw_events(&harness.paths)
        .expect("events");
    let pending = events
        .iter()
        .find(|event| event.event_type == "human_gate")
        .expect("pending event");
    assert_eq!(pending.payload["prompt"], "Approve the change?");
    let options = pending.payload["options"].as_array().expect("options");
    let offered = options
        .iter()
        .map(|option| (option["value"].clone(), option["label"].clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        offered,
        vec![
            (json!("approve"), json!("Approve")),
            (json!("reject"), json!("Reject"))
        ]
    );
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

    // Answer exactly like the API route does, with the decision id.
    answer(
        &harness,
        "run-gate-answer",
        &question_id,
        "approve",
        Some("Looks right, but double-check the lock cleanup."),
    );

    let (store, paths) = (harness.store.clone(), harness.paths.clone());
    let result = join_run(harness);
    assert_eq!(result.status, "completed");
    assert!(
        result.completed_nodes.contains(&"approved".to_string()),
        "answer must select the approve route: {:?}",
        result.completed_nodes,
    );
    assert!(!result.completed_nodes.contains(&"rejected".to_string()));
    let record = final_record(&store, &paths);
    assert_eq!(record.status, "completed");
    assert_eq!(record.outcome.as_deref(), Some("success"));
    assert_eq!(record.outcome_reason_code.as_deref(), Some("shipped"));

    // The journal records the selected decision and its target.
    let transition = store
        .read_raw_events(&paths)
        .expect("events")
        .into_iter()
        .find(|event| event.event_type == "StageTransition" && event.payload["node_id"] == "review")
        .expect("gate transition");
    assert_eq!(transition.payload["decision"], "approve");
    assert_eq!(transition.payload["target"], "approved");

    // The note travels with the selection into the gate's context updates so
    // downstream nodes can read it as `human.gate.note`.
    let gate_status = node_status(&store, &paths, "review");
    assert_eq!(gate_status["status"], "completed");
    assert_eq!(gate_status["decision"], "approve");
    assert_eq!(
        gate_status["context_updates"]["human.gate.selected"],
        "approve"
    );
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
fn skipped_and_undeclared_answers_keep_the_gate_waiting() {
    let temp = tempfile::tempdir().expect("tempdir");
    // The interviewer skips; a skip is not a decision, so the gate waits.
    let runner =
        RuntimeHandlerRunner::with_interviewer(QueueInterviewer::new([HumanAnswer::skipped()]))
            .with_blocking_human_gates();
    let harness = spawn_gate_run(&temp, "run-gate-invalid", runner);

    let (question_id, _) = wait_for_pending_gate(&harness);
    // An answer naming no declared decision is rejected; the gate keeps waiting.
    answer(&harness, "run-gate-invalid", &question_id, "maybe", None);
    std::thread::sleep(Duration::from_millis(1000));
    assert!(!harness.handle.is_finished());
    assert_eq!(
        final_record(&harness.store, &harness.paths).status,
        "waiting"
    );
    assert!(!stage_completed(&harness, "review"));

    // Labels are display: only the decision ID is an answer.
    answer(&harness, "run-gate-invalid", &question_id, "Reject", None);
    std::thread::sleep(Duration::from_millis(1000));
    assert!(!harness.handle.is_finished());
    answer(&harness, "run-gate-invalid", &question_id, "reject", None);
    let (store, paths) = (harness.store.clone(), harness.paths.clone());
    let result = join_run(harness);
    assert_eq!(result.status, "completed");
    assert!(result.completed_nodes.contains(&"rejected".to_string()));
    assert!(!result.completed_nodes.contains(&"approved".to_string()));
    assert_eq!(node_status(&store, &paths, "review")["decision"], "reject");
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

    let (store, paths) = (harness.store.clone(), harness.paths.clone());
    let result = join_run(harness);
    assert_eq!(result.status, "canceled");
    assert_eq!(final_record(&store, &paths).status, "canceled");
    assert!(!result.completed_nodes.contains(&"approved".to_string()));
    assert!(!result.completed_nodes.contains(&"rejected".to_string()));
}

#[test]
fn queued_interviewer_answers_bypass_the_wait() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runner =
        RuntimeHandlerRunner::with_interviewer(QueueInterviewer::new([HumanAnswer::selected(
            "reject",
        )]))
        .with_blocking_human_gates();
    let harness = spawn_gate_run(&temp, "run-gate-queued", runner);

    let (store, paths) = (harness.store.clone(), harness.paths.clone());
    let result = join_run(harness);
    assert_eq!(result.status, "completed");
    assert!(result.completed_nodes.contains(&"rejected".to_string()));
    // The rejected path ends at a failure exit: completed, but not a success.
    let record = final_record(&store, &paths);
    assert_eq!(record.status, "completed");
    assert_eq!(record.outcome.as_deref(), Some("failure"));
    assert_eq!(record.outcome_reason_code.as_deref(), Some("rejected"));
    // No pending question was ever published.
    let events = store.read_raw_events(&paths).expect("events");
    assert!(!events.iter().any(|event| event.event_type == "human_gate"));
}

#[test]
fn a_gate_that_cannot_wait_fails_the_run_instead_of_choosing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let harness = spawn_gate_run(&temp, "run-gate-skip", RuntimeHandlerRunner::new());

    let (store, paths) = (harness.store.clone(), harness.paths.clone());
    let result = join_run(harness);
    // The default interviewer skips and nothing can deliver an answer: an
    // execution error, never a route.
    assert_eq!(result.status, "failed");
    assert!(
        result.failure_reason.contains("no valid answer"),
        "{}",
        result.failure_reason
    );
    assert_eq!(final_record(&store, &paths).status, "failed");
    let gate_status = node_status(&store, &paths, "review");
    assert_eq!(gate_status["status"], "error");
    assert!(!result.completed_nodes.contains(&"approved".to_string()));
    assert!(!result.completed_nodes.contains(&"rejected".to_string()));
    let events = store.read_raw_events(&paths).expect("events");
    assert!(!events.iter().any(|event| event.event_type == "human_gate"));
}

/// `approve`'s label and accelerator collide with `stop`'s decision ID.
fn colliding_gate_flow() -> FlowDefinition {
    let mut flow = gate_flow();
    flow.nodes.get_mut("review").unwrap().routes = serde_json::from_value(json!({
        "approve": {"to": "approved", "description": "Ship the change.", "label": "stop"},
        "stop": {"to": "rejected", "description": "Stop without shipping.", "label": "Stop Run"}
    }))
    .unwrap();
    flow.validate().expect("valid flow");
    flow
}

#[test]
fn only_decision_ids_answer_a_gate_even_when_labels_collide() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runner = RuntimeHandlerRunner::new().with_blocking_human_gates();
    let harness = spawn_flow_run(&temp, "run-gate-collision", runner, colliding_gate_flow());
    let (question_id, _) = wait_for_pending_gate(&harness);
    // The other choice's label and accelerator are not answers.
    for display in ["Stop Run", "S"] {
        answer(&harness, "run-gate-collision", &question_id, display, None);
        std::thread::sleep(Duration::from_millis(600));
        assert!(!harness.handle.is_finished(), "{display}");
        assert!(!stage_completed(&harness, "review"), "{display}");
    }
    // `stop` is the stop decision, never `approve` whose label reads "stop".
    answer(&harness, "run-gate-collision", &question_id, "stop", None);
    let (store, paths) = (harness.store.clone(), harness.paths.clone());
    let result = join_run(harness);
    assert_eq!(result.status, "completed");
    assert_eq!(node_status(&store, &paths, "review")["decision"], "stop");
    assert!(!result.completed_nodes.contains(&"approved".to_string()));
    let record = final_record(&store, &paths);
    assert_eq!(record.outcome.as_deref(), Some("failure"));
    assert_eq!(record.outcome_reason_code.as_deref(), Some("rejected"));
}
