use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::Duration;

use attractor_core::{
    ContextMap, DotGraph, FailureKind, LaunchContext, Outcome, OutcomeStatus, RawRuntimeEvent,
    RunRecord,
};
use attractor_dsl::parse_dot;
use attractor_runtime::{
    ensure_run_layout, read_raw_events, resolve_handler_type_for_attrs, ChildInterventionRequest,
    ChildInterventionResult, ChildRunResult, ExecuteRunRequest, HumanAnswer, NodeArtifacts,
    NodeExecutionRequest, NodeExecutor, PipelineExecutor, QueueInterviewer, RunRootPaths, RunStore,
    RuntimeHandlerRunner, HANDLER_CODERGEN, HANDLER_CONDITIONAL, HANDLER_FAN_IN,
    HANDLER_MANAGER_LOOP, HANDLER_START, HANDLER_TOOL, HANDLER_WAIT_HUMAN,
};
use serde_json::{json, Value};
use unified_llm_adapter::{
    RUNTIME_LAUNCH_MODEL_KEY, RUNTIME_LAUNCH_PROFILE_KEY, RUNTIME_LAUNCH_PROVIDER_KEY,
    RUNTIME_LAUNCH_REASONING_EFFORT_KEY,
};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repo root")
        .to_path_buf()
}

fn runtime_fixture(name: &str) -> Value {
    let path = repo_root()
        .join(".spark/rust-rewrite/current/compat-fixtures/runtime")
        .join(format!("{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(&path).expect("fixture")).expect("fixture json")
}

fn fixture_graph(name: &str, input_key: &str) -> DotGraph {
    let fixture = runtime_fixture(name);
    parse_dot(
        fixture["input"][input_key]
            .as_str()
            .expect("fixture dot input"),
    )
    .expect("dot parses")
}

fn parse_graph(dot: &str) -> DotGraph {
    parse_dot(dot).expect("dot parses")
}

fn context(entries: impl IntoIterator<Item = (&'static str, Value)>) -> ContextMap {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn run_paths(temp: &tempfile::TempDir, run_id: &str) -> RunRootPaths {
    let project_dir = temp.path().join("Project");
    std::fs::create_dir_all(&project_dir).expect("project dir");
    let paths = RunRootPaths::new(
        temp.path().join("spark-home/attractor/runs"),
        &project_dir.to_string_lossy(),
        run_id,
    )
    .expect("run paths");
    ensure_run_layout(&paths).expect("run layout");
    paths
}

fn run_pipeline(
    graph: DotGraph,
    runner: RuntimeHandlerRunner,
    temp: &tempfile::TempDir,
    run_id: &str,
    launch_context: LaunchContext,
) -> (
    attractor_runtime::PipelineExecutionResult,
    RunStore,
    PathBuf,
) {
    let project_path = temp.path().join("Project");
    std::fs::create_dir_all(&project_path).expect("project");
    let store = RunStore::for_runs_dir(temp.path().join("spark-home/attractor/runs"));
    let mut record = RunRecord::new(run_id, project_path.to_string_lossy());
    record.started_at = "2026-06-23T10:00:00Z".to_string();
    let mut executor = PipelineExecutor::new(runner);
    let result = executor
        .execute(ExecuteRunRequest {
            store: store.clone(),
            record,
            graph,
            graph_source: None,
            graph_dot: None,
            launch_context,
            runtime_context: Default::default(),
            max_steps: None,
            start: Default::default(),
        })
        .expect("pipeline result");
    (result, store, project_path)
}

fn handler_request(
    graph: &DotGraph,
    node_id: &str,
    context: ContextMap,
    paths: &RunRootPaths,
    run_workdir: &Path,
) -> NodeExecutionRequest {
    let node = graph.nodes[node_id].clone();
    let prompt = attractor_core::attr_text(&node.attrs, "prompt").unwrap_or_default();
    NodeExecutionRequest {
        node_id: node_id.to_string(),
        stage_index: 0,
        context,
        prompt,
        node,
        graph: graph.clone(),
        outgoing_edges: attractor_runtime::outgoing_routing_edges(graph, node_id)
            .expect("outgoing edges"),
        run_paths: Some(paths.clone()),
        run_workdir: run_workdir.to_path_buf(),
        run_id: paths.run_id.clone(),
    }
}

fn execute_handler(
    runner: &mut RuntimeHandlerRunner,
    graph: &DotGraph,
    node_id: &str,
    context: ContextMap,
    paths: &RunRootPaths,
    run_workdir: &Path,
) -> Outcome {
    runner
        .execute(handler_request(graph, node_id, context, paths, run_workdir))
        .expect("handler executes")
}

fn parallel_result_by_id<'a>(results: &'a [Value], id: &str) -> &'a Value {
    results
        .iter()
        .find(|result| result.get("id") == Some(&json!(id)))
        .unwrap_or_else(|| panic!("missing parallel result for {id}"))
}

#[derive(Default)]
struct ParallelProbeState {
    in_flight: AtomicUsize,
    peak: AtomicUsize,
}

fn run_parallel_probe(state: &ParallelProbeState) {
    let current = state.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
    let mut observed = state.peak.load(Ordering::SeqCst);
    while current > observed {
        match state
            .peak
            .compare_exchange(observed, current, Ordering::SeqCst, Ordering::SeqCst)
        {
            Ok(_) => break,
            Err(actual) => observed = actual,
        }
    }
    thread::sleep(Duration::from_millis(50));
    state.in_flight.fetch_sub(1, Ordering::SeqCst);
}

fn parallel_probe_graph(max_parallel: usize) -> DotGraph {
    parse_graph(&format!(
        r#"
        digraph G {{
          fan [shape=component, max_parallel={max_parallel}]
          a [shape=box, type="custom.probe"]
          b [shape=box, type="custom.probe"]
          c [shape=box, type="custom.probe"]
          d [shape=box, type="custom.probe"]
          a_stop [shape=tripleoctagon]
          b_stop [shape=tripleoctagon]
          c_stop [shape=tripleoctagon]
          d_stop [shape=tripleoctagon]

          fan -> a
          fan -> b
          fan -> c
          fan -> d
          a -> a_stop [condition="outcome=success"]
          b -> b_stop [condition="outcome=success"]
          c -> c_stop [condition="outcome=success"]
          d -> d_stop [condition="outcome=success"]
        }}
        "#
    ))
}

#[test]
fn registry_resolution_preserves_type_shape_fallback_and_manager_placeholder() {
    let graph = parse_graph(
        r#"
        digraph G {
          start [shape=Mdiamond]
          gate [shape=hexagon]
          conditional [shape=diamond]
          tool [shape=parallelogram]
          fan_in [shape=tripleoctagon]
          manager [shape=house]
          fallback [shape=octagon]
          custom [shape=box, type="custom.success"]
        }
        "#,
    );
    let mut runner = RuntimeHandlerRunner::new();
    runner.register_static_handler("custom.success", Outcome::new(OutcomeStatus::Success));

    assert_eq!(
        runner.resolve_handler_type(&graph.nodes["start"].attrs),
        HANDLER_START
    );
    assert_eq!(
        runner.resolve_handler_type(&graph.nodes["gate"].attrs),
        HANDLER_WAIT_HUMAN
    );
    assert_eq!(
        runner.resolve_handler_type(&graph.nodes["conditional"].attrs),
        HANDLER_CONDITIONAL
    );
    assert_eq!(
        runner.resolve_handler_type(&graph.nodes["tool"].attrs),
        HANDLER_TOOL
    );
    assert_eq!(
        runner.resolve_handler_type(&graph.nodes["fan_in"].attrs),
        HANDLER_FAN_IN
    );
    assert_eq!(
        runner.resolve_handler_type(&graph.nodes["manager"].attrs),
        HANDLER_MANAGER_LOOP
    );
    assert_eq!(
        runner.resolve_handler_type(&graph.nodes["fallback"].attrs),
        HANDLER_CODERGEN
    );
    assert_eq!(
        runner.resolve_handler_type(&graph.nodes["custom"].attrs),
        "custom.success"
    );
    assert_eq!(
        resolve_handler_type_for_attrs(&graph.nodes["custom"].attrs, |_| false),
        HANDLER_CODERGEN
    );
}

#[test]
fn manager_loop_house_handler_no_longer_returns_placeholder_failure() {
    let graph = parse_graph(
        r#"
        digraph G {
          manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=1, manager.actions=""]
        }
        "#,
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = run_paths(&temp, "run-manager-no-placeholder");
    let mut runner = RuntimeHandlerRunner::new();

    let outcome = execute_handler(
        &mut runner,
        &graph,
        "manager",
        ContextMap::new(),
        &paths,
        temp.path(),
    );

    assert_eq!(outcome.status, OutcomeStatus::Fail);
    assert_eq!(outcome.failure_reason, "Max cycles exceeded");
    assert!(!outcome.failure_reason.contains("not implemented"));
}

#[test]
fn manager_loop_autostarts_first_class_child_run_and_records_lineage() {
    let temp = tempfile::tempdir().expect("tempdir");
    let child_dot_path = temp.path().join("child.dot");
    std::fs::write(
        &child_dot_path,
        r#"
        digraph Child {
          start [shape=Mdiamond]
          task [shape=box, prompt="Child task"]
          done [shape=Msquare]

          start -> task -> done
        }
        "#,
    )
    .expect("child dot");
    let graph = parse_graph(&format!(
        r#"
        digraph Parent {{
          graph [stack.child_dotfile="{}"]
          start [shape=Mdiamond]
          manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=1, manager.actions=""]
          done [shape=Msquare]

          start -> manager -> done
        }}
        "#,
        child_dot_path.display()
    ));

    let (result, store, project_path) = run_pipeline(
        graph,
        RuntimeHandlerRunner::new(),
        &temp,
        "run-manager-child-parent",
        LaunchContext::empty(),
    );

    assert_eq!(result.status, "completed");
    assert_eq!(result.current_node, "done");
    let child_run_id = result.context["context.stack.child.run_id"]
        .as_str()
        .expect("child run id");
    assert!(!child_run_id.is_empty());
    assert_eq!(
        result.context["context.stack.child.status"],
        json!("completed")
    );
    assert_eq!(
        result.context["context.stack.child.outcome"],
        json!("success")
    );
    assert_eq!(
        result.context["context.stack.child.active_stage"],
        json!("done")
    );

    let child_bundle = store
        .read_run_bundle(child_run_id)
        .expect("read child")
        .expect("child run");
    let child_record = child_bundle.record.expect("child record");
    assert_eq!(
        child_record.parent_run_id.as_deref(),
        Some("run-manager-child-parent")
    );
    assert_eq!(child_record.parent_node_id.as_deref(), Some("manager"));
    assert_eq!(
        child_record.root_run_id.as_deref(),
        Some("run-manager-child-parent")
    );
    assert_eq!(child_record.child_invocation_index, Some(1));
    assert_eq!(child_record.flow_name, "child.dot");
    assert_eq!(child_record.status, "completed");
    assert_eq!(child_record.outcome.as_deref(), Some("success"));

    let parent_paths = store
        .run_root(&project_path.to_string_lossy(), "run-manager-child-parent")
        .expect("parent paths");
    let parent_events = read_raw_events(&parent_paths).expect("parent events");
    assert!(parent_events
        .iter()
        .any(|event| event.event_type == "ChildRunStarted"
            && event.payload["child_run_id"] == json!(child_run_id)));
    assert!(parent_events
        .iter()
        .any(|event| event.event_type == "ChildRunCompleted"
            && event.payload["status"] == json!("completed")));
    assert!(child_bundle
        .journal
        .iter()
        .any(|entry| entry.raw_type == "PipelineStarted"));
}

#[test]
fn manager_loop_observes_seeded_child_run_without_failing_missing_progress() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("spark-home/attractor/runs"));
    let child_project = temp.path().join("Child Project");
    std::fs::create_dir_all(&child_project).expect("child project");
    let mut child_record = RunRecord::new("child-observed", child_project.to_string_lossy());
    child_record.status = "completed".to_string();
    child_record.outcome = Some("success".to_string());
    child_record.parent_run_id = Some("parent-observe".to_string());
    child_record.parent_node_id = Some("manager".to_string());
    child_record.root_run_id = Some("parent-observe".to_string());
    child_record.child_invocation_index = Some(1);
    let mut retry_counts = BTreeMap::new();
    retry_counts.insert("done".to_string(), 2);
    let child_paths = store
        .create_run(attractor_runtime::CreateRunRequest {
            record: child_record,
            checkpoint: Some(attractor_core::CheckpointState {
                timestamp: "2026-06-23T10:00:00Z".to_string(),
                current_node: "done".to_string(),
                completed_nodes: vec!["start".to_string(), "done".to_string()],
                context: ContextMap::new(),
                retry_counts,
                logs: Vec::new(),
            }),
            ..Default::default()
        })
        .expect("child run");
    store
        .write_node_artifacts(
            &child_paths,
            "done",
            &NodeArtifacts {
                status: Some(json!({"phase": "observed"})),
                ..Default::default()
            },
        )
        .expect("child artifact");
    let mut progress_event = RawRuntimeEvent::new("ChildProgress", "child-observed");
    progress_event.emitted_at = "2026-06-23T10:00:05Z".to_string();
    store
        .append_event(&child_paths, progress_event)
        .expect("child progress event");
    let graph = parse_graph(
        r#"
        digraph G {
          manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=1, manager.actions="observe"]
        }
        "#,
    );
    let paths = run_paths(&temp, "parent-observe");
    let mut runner = RuntimeHandlerRunner::new();

    let outcome = execute_handler(
        &mut runner,
        &graph,
        "manager",
        context([("context.stack.child.run_id", json!("child-observed"))]),
        &paths,
        temp.path(),
    );

    assert_eq!(outcome.status, OutcomeStatus::Success);
    assert_eq!(outcome.notes, "Child completed");
    assert_eq!(
        outcome.context_updates["context.stack.child.status"],
        json!("completed")
    );
    assert_eq!(
        outcome.context_updates["context.stack.child.active_stage"],
        json!("done")
    );
    assert_eq!(
        outcome.context_updates["context.stack.child.retry_count"],
        json!(2)
    );
    assert_eq!(
        outcome.context_updates["context.stack.child.retry_counts"],
        json!({"done": 2})
    );
    assert!(
        outcome.context_updates["context.stack.child.artifact_count"]
            .as_u64()
            .expect("artifact count")
            >= 1
    );
    assert_eq!(
        outcome.context_updates["context.stack.child.event_count"],
        json!(4)
    );
    assert_eq!(
        outcome.context_updates["context.stack.child.checkpoint_timestamp"],
        json!("2026-06-23T10:00:00Z")
    );
    assert_eq!(
        outcome.context_updates["context.stack.child.latest_event_at"],
        json!("2026-06-23T10:00:05Z")
    );
    assert!(paths
        .logs_dir()
        .join("manager/manager_telemetry.jsonl")
        .exists());

    let missing_paths = run_paths(&temp, "parent-observe-missing");
    let missing = execute_handler(
        &mut runner,
        &graph,
        "manager",
        context([("context.stack.child.run_id", json!("missing-child"))]),
        &missing_paths,
        temp.path(),
    );
    assert_eq!(missing.status, OutcomeStatus::Fail);
    assert_eq!(missing.failure_reason, "Max cycles exceeded");
}

#[test]
fn manager_loop_progress_telemetry_can_stop_without_triggering_steering() {
    let graph = parse_graph(
        r#"
        digraph G {
          manager [
            shape=house,
            manager.poll_interval=0ms,
            manager.max_cycles=1,
            manager.actions="observe,steer",
            manager.stop_condition="context.stack.child.event_count=5"
          ]
        }
        "#,
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = run_paths(&temp, "parent-progress-stop");
    let requests = Arc::new(Mutex::new(Vec::<ChildInterventionRequest>::new()));
    let captured = Arc::clone(&requests);
    let mut runner = RuntimeHandlerRunner::new()
        .with_child_status_resolver(|run_id| {
            Some(ChildRunResult {
                run_id: run_id.to_string(),
                status: "running".to_string(),
                current_node: "work".to_string(),
                retry_count: Some(3),
                artifact_count: Some(0),
                event_count: Some(5),
                checkpoint_timestamp: "2026-06-23T10:00:00Z".to_string(),
                latest_event_at: "2026-06-23T10:00:05Z".to_string(),
                ..Default::default()
            })
        })
        .with_child_intervention_requester(move |request| {
            captured.lock().expect("requests").push(request.clone());
            ChildInterventionResult {
                run_id: request.child_run_id,
                status: "delivered".to_string(),
                delivery_mode: "test".to_string(),
                reason: request.reason,
                message: "unexpected".to_string(),
                target_node_id: request.target_node_id,
            }
        });

    let outcome = execute_handler(
        &mut runner,
        &graph,
        "manager",
        context([("context.stack.child.run_id", json!("child-progress"))]),
        &paths,
        temp.path(),
    );

    assert_eq!(outcome.status, OutcomeStatus::Success);
    assert_eq!(outcome.notes, "Stop condition satisfied");
    assert!(requests.lock().expect("requests").is_empty());
    assert_eq!(
        outcome.context_updates["context.stack.child.status"],
        json!("running")
    );
    assert_eq!(
        outcome.context_updates["context.stack.child.active_stage"],
        json!("work")
    );
    assert_eq!(
        outcome.context_updates["context.stack.child.retry_count"],
        json!(3)
    );
    assert_eq!(
        outcome.context_updates["context.stack.child.artifact_count"],
        json!(0)
    );
    assert_eq!(
        outcome.context_updates["context.stack.child.event_count"],
        json!(5)
    );
    assert!(paths
        .logs_dir()
        .join("manager/manager_telemetry.jsonl")
        .exists());
    assert!(!paths
        .logs_dir()
        .join("manager/manager_interventions.jsonl")
        .exists());
}

#[test]
fn manager_loop_observe_ingests_terminal_failure_outcome_reason() {
    let graph = parse_graph(
        r#"
        digraph G {
          manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=1, manager.actions="observe"]
        }
        "#,
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = run_paths(&temp, "parent-terminal-failure");
    let mut runner = RuntimeHandlerRunner::new().with_child_status_resolver(|run_id| {
        Some(ChildRunResult {
            run_id: run_id.to_string(),
            status: "completed".to_string(),
            outcome: Some("failure".to_string()),
            outcome_reason_code: Some("tests_failed".to_string()),
            outcome_reason_message: Some("child tests failed".to_string()),
            current_node: "verify".to_string(),
            completed_nodes: vec!["start".to_string(), "verify".to_string()],
            route_trace: vec!["start".to_string(), "verify".to_string()],
            ..Default::default()
        })
    });

    let outcome = execute_handler(
        &mut runner,
        &graph,
        "manager",
        context([(
            "context.stack.child.run_id",
            json!("child-terminal-failure"),
        )]),
        &paths,
        temp.path(),
    );

    assert_eq!(outcome.status, OutcomeStatus::Fail);
    assert_eq!(outcome.failure_reason, "child tests failed");
    assert_eq!(
        outcome.context_updates["context.stack.child.status"],
        json!("completed")
    );
    assert_eq!(
        outcome.context_updates["context.stack.child.outcome"],
        json!("failure")
    );
    assert_eq!(
        outcome.context_updates["context.stack.child.outcome_reason_code"],
        json!("tests_failed")
    );
    assert_eq!(
        outcome.context_updates["context.stack.child.outcome_reason_message"],
        json!("child tests failed")
    );
}

#[test]
fn manager_loop_steering_records_rejected_and_delivered_interventions() {
    let graph = parse_graph(
        r#"
        digraph G {
          manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=2, manager.actions="steer"]
        }
        "#,
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = run_paths(&temp, "run-manager-steer");
    let mut unsupported_runner = RuntimeHandlerRunner::new();

    let unsupported = execute_handler(
        &mut unsupported_runner,
        &graph,
        "manager",
        context([
            ("internal.run_id", json!("run-manager-steer")),
            ("context.stack.child.run_id", json!("child-steer")),
            ("context.stack.child.status", json!("running")),
            ("context.stack.child.active_stage", json!("task")),
            (
                "context.stack.child.failure_reason",
                json!("unit tests failed"),
            ),
        ]),
        &paths,
        temp.path(),
    );

    assert_eq!(unsupported.status, OutcomeStatus::Fail);
    assert_eq!(
        unsupported.context_updates["context.stack.child.intervention_delivery_mode"],
        json!("none")
    );
    assert_eq!(
        unsupported.context_updates["context.stack.child.intervention_reason"],
        json!("auto_steer_limit_reached")
    );
    let events = read_raw_events(&paths).expect("events");
    let intervention_events = events
        .iter()
        .filter(|event| event.event_type == "ChildInterventionRequested")
        .collect::<Vec<_>>();
    assert_eq!(intervention_events.len(), 2);
    assert_eq!(
        intervention_events[0].payload["delivery_mode"],
        json!("unsupported")
    );
    assert_eq!(intervention_events[1].payload["status"], json!("skipped"));

    let delivered_paths = run_paths(&temp, "run-manager-steer-delivered");
    let requests = Arc::new(Mutex::new(Vec::<ChildInterventionRequest>::new()));
    let captured = Arc::clone(&requests);
    let mut delivered_runner =
        RuntimeHandlerRunner::new().with_child_intervention_requester(move |request| {
            captured.lock().expect("requests").push(request.clone());
            ChildInterventionResult {
                run_id: request.child_run_id,
                status: "delivered".to_string(),
                delivery_mode: "test".to_string(),
                reason: request.reason,
                message: "queued".to_string(),
                target_node_id: request.target_node_id,
            }
        });
    let delivered = execute_handler(
        &mut delivered_runner,
        &graph,
        "manager",
        context([
            ("internal.run_id", json!("run-manager-steer-delivered")),
            ("internal.root_run_id", json!("root-run")),
            ("context.stack.child.run_id", json!("child-steer-delivered")),
            ("context.stack.child.status", json!("running")),
            ("context.stack.child.active_stage", json!("task")),
            (
                "context.stack.child.failure_reason",
                json!("integration failed"),
            ),
        ]),
        &delivered_paths,
        temp.path(),
    );

    assert_eq!(delivered.status, OutcomeStatus::Fail);
    assert_eq!(requests.lock().expect("requests").len(), 1);
    assert_eq!(
        delivered.context_updates["context.stack.child.intervention_status"],
        json!("skipped")
    );
    let delivered_events = read_raw_events(&delivered_paths).expect("events");
    assert!(delivered_events.iter().any(|event| {
        event.event_type == "ChildInterventionRequested"
            && event.payload["delivery_mode"] == json!("test")
            && event.payload["reason"] == json!("integration failed")
    }));
}

#[test]
fn noop_start_exit_and_conditional_handlers_match_fixture_outcomes() {
    let graph = fixture_graph("handler-start-exit-conditional", "dot");
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = run_paths(&temp, "run-noop");
    let mut runner = RuntimeHandlerRunner::new();

    for node_id in ["start", "gate", "done"] {
        let outcome = execute_handler(
            &mut runner,
            &graph,
            node_id,
            ContextMap::new(),
            &paths,
            temp.path(),
        );
        assert_eq!(outcome.status, OutcomeStatus::Success);
        assert!(outcome.context_updates.is_empty());
        assert!(outcome.failure_reason.is_empty());
        assert!(outcome.notes.is_empty());
        assert!(outcome.preferred_label.is_empty());
        assert!(outcome.suggested_next_ids.is_empty());
    }
}

#[test]
fn tool_handler_writes_output_context_logs_and_declared_artifacts() {
    let graph = fixture_graph("handler-tool-success-artifacts", "dot");
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = run_paths(&temp, "run-tool-success");
    let mut runner = RuntimeHandlerRunner::new();
    let outcome = execute_handler(
        &mut runner,
        &graph,
        "tool_node",
        context([("internal.run_workdir", json!(temp.path()))]),
        &paths,
        temp.path(),
    );

    assert_eq!(outcome.status, OutcomeStatus::Success);
    assert_eq!(outcome.notes, "compat-tool");
    assert_eq!(
        outcome.context_updates["context.tool.output"],
        json!("compat-tool")
    );
    assert_eq!(outcome.context_updates["context.tool.exit_code"], json!(0));
    assert_eq!(
        std::fs::read_to_string(paths.logs_dir().join("tool_node/tool_output.txt"))
            .expect("tool log"),
        "compat-tool"
    );
    assert_eq!(
        std::fs::read_to_string(paths.artifacts_dir().join("tool_node/tool/stdout.txt"))
            .expect("stdout artifact"),
        "compat-tool"
    );
}

#[test]
fn tool_handler_blocks_on_pre_hook_and_records_hook_failure() {
    let graph = fixture_graph("handler-tool-prehook-failure", "dot");
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = run_paths(&temp, "run-tool-prehook");
    let mut runner = RuntimeHandlerRunner::new();
    let outcome = execute_handler(
        &mut runner,
        &graph,
        "tool_node",
        context([("internal.run_workdir", json!(temp.path()))]),
        &paths,
        temp.path(),
    );

    assert_eq!(outcome.status, OutcomeStatus::Fail);
    assert_eq!(
        outcome.failure_reason,
        "tool pre-hook blocked execution (exit code 1): false"
    );
    assert_eq!(outcome.context_updates["context.tool.exit_code"], json!(-1));
    assert_eq!(
        std::fs::read_to_string(paths.logs_dir().join("tool_node/tool_output.txt"))
            .expect("tool output"),
        ""
    );
    let hook_record: Value = serde_json::from_str(
        std::fs::read_to_string(paths.logs_dir().join("tool_node/tool_hook_failures.jsonl"))
            .expect("hook failures")
            .trim(),
    )
    .expect("hook record");
    assert_eq!(hook_record["hook_phase"], json!("pre"));
    assert_eq!(hook_record["command"], json!("false"));
    assert_eq!(hook_record["exit_code"], json!(1));
}

#[test]
fn tool_artifact_capture_rejects_unsafe_destination_observably() {
    let graph = parse_graph(
        r#"
        digraph G {
          tool_node [
            shape=parallelogram,
            tool.command="printf blocked",
            tool.artifacts.stdout="../escape.txt"
          ]
        }
        "#,
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = run_paths(&temp, "run-tool-unsafe-artifact");
    let mut runner = RuntimeHandlerRunner::new();
    let outcome = execute_handler(
        &mut runner,
        &graph,
        "tool_node",
        context([("internal.run_workdir", json!(temp.path()))]),
        &paths,
        temp.path(),
    );

    assert_eq!(outcome.status, OutcomeStatus::Fail);
    assert!(outcome
        .failure_reason
        .starts_with("artifact capture failed:"));
    assert_eq!(
        outcome.context_updates["context.tool.output"],
        json!("blocked")
    );
}

#[test]
fn human_gate_selects_outgoing_edge_and_records_interview_events() {
    let graph = fixture_graph("handler-wait-human-answer", "dot");
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = run_paths(&temp, "run-human");
    let mut runner =
        RuntimeHandlerRunner::with_interviewer(QueueInterviewer::new([HumanAnswer::selected(
            "ship",
        )]));
    let selected = execute_handler(
        &mut runner,
        &graph,
        "gate",
        ContextMap::new(),
        &paths,
        temp.path(),
    );

    assert_eq!(selected.status, OutcomeStatus::Success);
    assert_eq!(selected.preferred_label, "[S] Ship");
    assert_eq!(selected.suggested_next_ids, ["ship"]);
    assert_eq!(selected.context_updates["human.gate.selected"], json!("S"));
    assert_eq!(
        selected.context_updates["human.gate.label"],
        json!("[S] Ship")
    );

    let events = read_raw_events(&paths).expect("events");
    assert!(events
        .iter()
        .any(|event| event.event_type == "InterviewStarted"
            && event.payload.get("node_id") == Some(&json!("gate"))));
    assert!(events
        .iter()
        .any(|event| event.event_type == "InterviewCompleted"
            && event.payload.get("answer") == Some(&json!("[S] Ship"))));

    let skipped_paths = run_paths(&temp, "run-human-skipped");
    let mut skipped_runner = RuntimeHandlerRunner::with_interviewer(QueueInterviewer::new([]));
    let skipped = execute_handler(
        &mut skipped_runner,
        &graph,
        "gate",
        ContextMap::new(),
        &skipped_paths,
        temp.path(),
    );
    assert_eq!(skipped.status, OutcomeStatus::Fail);
    assert_eq!(skipped.failure_reason, "human skipped interaction");
}

#[test]
fn parallel_fanout_and_fan_in_preserve_branch_payload_and_selection() {
    let graph = fixture_graph("handler-parallel-fanout-join", "parallel_dot");
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = run_paths(&temp, "run-parallel");
    let mut runner = RuntimeHandlerRunner::new();
    runner.register_static_handler(
        "custom.success",
        Outcome {
            status: OutcomeStatus::Success,
            notes: "custom-success".to_string(),
            ..Outcome::new(OutcomeStatus::Success)
        },
    );
    runner.register_static_handler(
        "custom.fail",
        Outcome {
            status: OutcomeStatus::Fail,
            failure_reason: "custom-fail".to_string(),
            retryable: Some(false),
            ..Outcome::new(OutcomeStatus::Fail)
        },
    );

    let parallel = execute_handler(
        &mut runner,
        &graph,
        "fan",
        ContextMap::new(),
        &paths,
        temp.path(),
    );
    assert_eq!(parallel.status, OutcomeStatus::Success);
    assert_eq!(parallel.notes, "parallel fan-out completed");
    let results = parallel.context_updates["parallel.results"]
        .as_array()
        .expect("parallel results");
    assert_eq!(results.len(), 2);
    let result_a = parallel_result_by_id(results, "a");
    assert_eq!(result_a["node_outcomes"]["a"], json!("success"));
    let result_b = parallel_result_by_id(results, "b");
    assert_eq!(result_b["current_node"], json!("b_stop"));
    assert_eq!(result_b["context"]["outcome"], json!("fail"));

    let fan_in_graph = fixture_graph("handler-parallel-fanout-join", "fan_in_dot");
    let fan_in_paths = run_paths(&temp, "run-fan-in");
    let mut fan_in_runner = RuntimeHandlerRunner::new();
    let fan_in = execute_handler(
        &mut fan_in_runner,
        &fan_in_graph,
        "fan_in",
        context([(
            "parallel.results",
            parallel.context_updates["parallel.results"].clone(),
        )]),
        &fan_in_paths,
        temp.path(),
    );
    assert_eq!(fan_in.status, OutcomeStatus::Success);
    assert_eq!(
        fan_in.context_updates["parallel.fan_in.best_id"],
        json!("a")
    );
    assert_eq!(
        fan_in.context_updates["parallel.fan_in.best_outcome"],
        json!("success")
    );
}

#[test]
fn parallel_respects_max_parallel_bound() {
    let graph = parallel_probe_graph(2);
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = run_paths(&temp, "run-parallel-max-two");
    let state = Arc::new(ParallelProbeState::default());
    let captured = Arc::clone(&state);
    let mut runner = RuntimeHandlerRunner::new();
    runner.register_thread_safe_handler_fn("custom.probe", move |_runtime| {
        run_parallel_probe(&captured);
        Ok(Outcome::new(OutcomeStatus::Success))
    });

    let outcome = execute_handler(
        &mut runner,
        &graph,
        "fan",
        ContextMap::new(),
        &paths,
        temp.path(),
    );

    assert_eq!(outcome.status, OutcomeStatus::Success);
    let peak = state.peak.load(Ordering::SeqCst);
    assert!(
        peak > 1,
        "max_parallel=2 should allow more than one in-flight branch"
    );
    assert!(
        peak <= 2,
        "max_parallel=2 should cap in-flight branches at two, saw {peak}"
    );
    let results = outcome.context_updates["parallel.results"]
        .as_array()
        .expect("parallel results");
    assert_eq!(results.len(), 4);
    assert!(["a", "b", "c", "d"]
        .iter()
        .all(|branch| parallel_result_by_id(results, branch)["outcome"] == json!("success")));
}

#[test]
fn parallel_max_parallel_one_remains_serialized() {
    let graph = parallel_probe_graph(1);
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = run_paths(&temp, "run-parallel-max-one");
    let state = Arc::new(ParallelProbeState::default());
    let captured = Arc::clone(&state);
    let mut runner = RuntimeHandlerRunner::new();
    runner.register_thread_safe_handler_fn("custom.probe", move |_runtime| {
        run_parallel_probe(&captured);
        Ok(Outcome::new(OutcomeStatus::Success))
    });

    let outcome = execute_handler(
        &mut runner,
        &graph,
        "fan",
        ContextMap::new(),
        &paths,
        temp.path(),
    );

    assert_eq!(outcome.status, OutcomeStatus::Success);
    assert_eq!(state.peak.load(Ordering::SeqCst), 1);
    let results = outcome.context_updates["parallel.results"]
        .as_array()
        .expect("parallel results");
    assert_eq!(results.len(), 4);
}

#[test]
fn parallel_default_custom_handlers_remain_serialized() {
    let graph = parallel_probe_graph(4);
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = run_paths(&temp, "run-parallel-serialized-custom");
    let state = Arc::new(ParallelProbeState::default());
    let captured = Arc::clone(&state);
    let mut runner = RuntimeHandlerRunner::new();
    runner.register_handler_fn("custom.probe", move |_runtime| {
        run_parallel_probe(&captured);
        Ok(Outcome::new(OutcomeStatus::Success))
    });

    let outcome = execute_handler(
        &mut runner,
        &graph,
        "fan",
        ContextMap::new(),
        &paths,
        temp.path(),
    );

    assert_eq!(outcome.status, OutcomeStatus::Success);
    assert_eq!(
        state.peak.load(Ordering::SeqCst),
        1,
        "default custom handlers must stay serialized under parallel fanout"
    );
    let results = outcome.context_updates["parallel.results"]
        .as_array()
        .expect("parallel results");
    assert_eq!(results.len(), 4);
}

#[test]
fn pipeline_allows_tool_and_human_builtin_context_updates_without_write_declarations() {
    let graph = parse_graph(
        r#"
        digraph G {
          start [shape=Mdiamond]
          tool_node [
            shape=parallelogram,
            tool.command="printf pipeline-tool",
            tool.artifacts.stdout="tool/stdout.txt"
          ]
          gate [shape=hexagon, prompt="Ship?"]
          ship [shape=Msquare]
          hold [shape=Msquare]

          start -> tool_node
          tool_node -> gate [condition="outcome=success"]
          gate -> ship [label="[S] Ship"]
          gate -> hold [label="[H] Hold"]
        }
        "#,
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let runner =
        RuntimeHandlerRunner::with_interviewer(QueueInterviewer::new([HumanAnswer::selected(
            "ship",
        )]));

    let (result, store, project_path) = run_pipeline(
        graph,
        runner,
        &temp,
        "run-tool-human-pipeline",
        LaunchContext::empty(),
    );

    assert_eq!(result.status, "completed");
    assert_eq!(result.current_node, "ship");
    assert_eq!(
        result.context["context.tool.output"],
        json!("pipeline-tool")
    );
    assert_eq!(result.context["context.tool.exit_code"], json!(0));
    assert_eq!(result.context["human.gate.selected"], json!("S"));
    assert_eq!(result.context["human.gate.label"], json!("[S] Ship"));
    assert_eq!(
        result.node_outcomes["tool_node"].status,
        OutcomeStatus::Success
    );
    assert_eq!(result.node_outcomes["gate"].status, OutcomeStatus::Success);

    let paths = store
        .run_root(&project_path.to_string_lossy(), "run-tool-human-pipeline")
        .expect("run paths");
    assert_eq!(
        std::fs::read_to_string(paths.logs_dir().join("tool_node/tool_output.txt"))
            .expect("tool log"),
        "pipeline-tool"
    );
    assert_eq!(
        std::fs::read_to_string(paths.artifacts_dir().join("tool_node/tool/stdout.txt"))
            .expect("tool artifact"),
        "pipeline-tool"
    );
    let events = read_raw_events(&paths).expect("events");
    assert!(events
        .iter()
        .any(|event| event.event_type == "InterviewStarted"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "InterviewCompleted"));
}

#[test]
fn pipeline_allows_parallel_and_fan_in_builtin_context_updates_without_write_declarations() {
    let graph = parse_graph(
        r#"
        digraph G {
          start [shape=Mdiamond]
          fan [shape=component, max_parallel=2]
          a [shape=box, type="custom.success"]
          b [shape=box, type="custom.fail"]
          join [shape=tripleoctagon]
          done [shape=Msquare]

          start -> fan
          fan -> a
          fan -> b
          a -> join [condition="outcome=success"]
          b -> join [condition="outcome=fail"]
          join -> done
        }
        "#,
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let mut runner = RuntimeHandlerRunner::new();
    runner.register_static_handler(
        "custom.success",
        Outcome {
            status: OutcomeStatus::Success,
            ..Outcome::new(OutcomeStatus::Success)
        },
    );
    runner.register_static_handler(
        "custom.fail",
        Outcome {
            status: OutcomeStatus::Fail,
            failure_reason: "custom-fail".to_string(),
            retryable: Some(false),
            ..Outcome::new(OutcomeStatus::Fail)
        },
    );

    let (result, store, project_path) = run_pipeline(
        graph,
        runner,
        &temp,
        "run-parallel-fan-in-pipeline",
        LaunchContext::empty(),
    );

    assert_eq!(result.status, "completed");
    assert_eq!(result.current_node, "done");
    assert_eq!(result.node_outcomes["fan"].status, OutcomeStatus::Success);
    assert_eq!(result.node_outcomes["join"].status, OutcomeStatus::Success);
    assert_eq!(result.context["parallel.fan_in.best_id"], json!("a"));
    assert_eq!(
        result.context["parallel.fan_in.best_outcome"],
        json!("success")
    );
    let results = result.context["parallel.results"]
        .as_array()
        .expect("parallel results");
    assert_eq!(results.len(), 2);
    assert_eq!(
        parallel_result_by_id(results, "a")["outcome"],
        json!("success")
    );
    assert_eq!(
        parallel_result_by_id(results, "b")["node_outcomes"]["b"],
        json!("fail")
    );

    let paths = store
        .run_root(
            &project_path.to_string_lossy(),
            "run-parallel-fan-in-pipeline",
        )
        .expect("run paths");
    let events = read_raw_events(&paths).expect("events");
    assert!(events
        .iter()
        .any(|event| event.event_type == "ParallelStarted"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "ParallelCompleted"));
}

#[test]
fn parallel_error_policies_preserve_python_visible_results() {
    for (error_policy, expected_status, expected_result_count, expected_failures) in [
        ("continue", OutcomeStatus::PartialSuccess, 3_usize, 1_usize),
        ("ignore", OutcomeStatus::Success, 2, 0),
        ("fail_fast", OutcomeStatus::PartialSuccess, 1, 1),
    ] {
        let graph = parse_graph(&format!(
            r#"
            digraph G {{
              fan [shape=component, join_policy=wait_all, error_policy={error_policy}, max_parallel=1]
              bad [shape=box, type="custom.fail"]
              good_a [shape=box, type="custom.success"]
              good_b [shape=box, type="custom.success"]
              stop_a [shape=tripleoctagon]
              stop_b [shape=tripleoctagon]

              fan -> bad
              fan -> good_a
              fan -> good_b
              good_a -> stop_a [condition="outcome=success"]
              good_b -> stop_b [condition="outcome=success"]
            }}
            "#
        ));
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = run_paths(&temp, &format!("run-parallel-{error_policy}"));
        let mut runner = RuntimeHandlerRunner::new();
        runner.register_static_handler(
            "custom.success",
            Outcome {
                status: OutcomeStatus::Success,
                ..Outcome::new(OutcomeStatus::Success)
            },
        );
        runner.register_static_handler(
            "custom.fail",
            Outcome {
                status: OutcomeStatus::Fail,
                failure_reason: "custom-fail".to_string(),
                retryable: Some(false),
                ..Outcome::new(OutcomeStatus::Fail)
            },
        );

        let outcome = execute_handler(
            &mut runner,
            &graph,
            "fan",
            ContextMap::new(),
            &paths,
            temp.path(),
        );

        assert_eq!(outcome.status, expected_status, "{error_policy}");
        let results = outcome.context_updates["parallel.results"]
            .as_array()
            .expect("parallel results");
        assert_eq!(results.len(), expected_result_count, "{error_policy}");
        let fail_count = results
            .iter()
            .filter(|result| result.get("status") == Some(&json!("failed")))
            .count();
        assert_eq!(fail_count, expected_failures, "{error_policy}");
    }
}

#[test]
fn fan_in_accepts_ranker_best_id_before_deterministic_fallback() {
    let graph = parse_graph(
        r#"
        digraph G {
          fan_in [shape=tripleoctagon, prompt="Pick best"]
        }
        "#,
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = run_paths(&temp, "run-ranked-fan-in");
    let results = json!([
        {"id": "a", "status": "completed", "outcome": "success", "score": 10},
        {"id": "b", "status": "completed", "outcome": "success", "score": 1}
    ]);
    let mut runner = RuntimeHandlerRunner::new().with_fan_in_ranker(|request| {
        assert_eq!(request.model.as_deref(), Some("fast"));
        Some("{\"best_id\":\"b\"}".to_string())
    });
    let ranked_graph = parse_graph(
        r#"
        digraph G {
          fan_in [shape=tripleoctagon, prompt="Pick best", llm_model="fast"]
        }
        "#,
    );
    let outcome = execute_handler(
        &mut runner,
        &ranked_graph,
        "fan_in",
        context([("parallel.results", results)]),
        &paths,
        temp.path(),
    );
    assert_eq!(outcome.status, OutcomeStatus::Success);
    assert_eq!(
        outcome.context_updates["parallel.fan_in.best_id"],
        json!("b")
    );

    let fallback_paths = run_paths(&temp, "run-ranked-fallback");
    let mut fallback_runner = RuntimeHandlerRunner::new();
    let fallback = execute_handler(
        &mut fallback_runner,
        &graph,
        "fan_in",
        context([(
            "parallel.results",
            json!([
                {"id": "z", "status": "completed", "outcome": "partial_success", "score": 99},
                {"id": "a", "status": "completed", "outcome": "success", "score": 0}
            ]),
        )]),
        &fallback_paths,
        temp.path(),
    );
    assert_eq!(
        fallback.context_updates["parallel.fan_in.best_id"],
        json!("a")
    );
}

#[test]
fn fan_in_ranking_uses_llm_resolution_contract_for_node_and_launch_metadata() {
    let results = json!([
        {"id": "branch_a", "status": "completed", "outcome": "success"},
        {"id": "branch_b", "status": "completed", "outcome": "success"}
    ]);
    let temp = tempfile::tempdir().expect("tempdir");

    let node_graph = parse_graph(
        r#"
        digraph G {
          fan_in [
            shape=tripleoctagon,
            prompt="Rank",
            llm_model="gpt-fan-in",
            llm_provider="Gemini",
            llm_profile="balanced",
            reasoning_effort="HIGH"
          ]
        }
        "#,
    );
    let node_calls = Arc::new(Mutex::new(Vec::new()));
    let captured_node_calls = Arc::clone(&node_calls);
    let mut node_runner = RuntimeHandlerRunner::new().with_fan_in_ranker(move |request| {
        captured_node_calls
            .lock()
            .expect("node calls lock")
            .push(request.clone());
        Some("{\"best_id\":\"branch_b\"}".to_string())
    });
    let node_paths = run_paths(&temp, "run-fan-in-node-llm");
    let node_outcome = execute_handler(
        &mut node_runner,
        &node_graph,
        "fan_in",
        context([
            (RUNTIME_LAUNCH_MODEL_KEY, json!("gpt-launch")),
            (RUNTIME_LAUNCH_PROVIDER_KEY, json!("openai")),
            (RUNTIME_LAUNCH_PROFILE_KEY, json!("launch-profile")),
            (RUNTIME_LAUNCH_REASONING_EFFORT_KEY, json!("low")),
            ("parallel.results", results.clone()),
        ]),
        &node_paths,
        temp.path(),
    );
    assert_eq!(node_outcome.status, OutcomeStatus::Success);
    let node_call = node_calls.lock().expect("node calls lock");
    assert_eq!(node_call[0].model.as_deref(), Some("gpt-fan-in"));
    assert_eq!(node_call[0].provider, "gemini");
    assert_eq!(node_call[0].llm_profile, "balanced");
    assert_eq!(node_call[0].reasoning_effort, "high");

    let launch_graph = parse_graph(
        r#"
        digraph G {
          fan_in [shape=tripleoctagon, prompt="Rank"]
        }
        "#,
    );
    let launch_calls = Arc::new(Mutex::new(Vec::new()));
    let captured_launch_calls = Arc::clone(&launch_calls);
    let mut launch_runner = RuntimeHandlerRunner::new().with_fan_in_ranker(move |request| {
        captured_launch_calls
            .lock()
            .expect("launch calls lock")
            .push(request.clone());
        Some("{\"best_id\":\"branch_a\"}".to_string())
    });
    let launch_paths = run_paths(&temp, "run-fan-in-launch-llm");
    let launch_outcome = execute_handler(
        &mut launch_runner,
        &launch_graph,
        "fan_in",
        context([
            (RUNTIME_LAUNCH_MODEL_KEY, json!("gpt-launch")),
            (RUNTIME_LAUNCH_PROVIDER_KEY, json!("OpenAI")),
            (RUNTIME_LAUNCH_PROFILE_KEY, json!("launch-profile")),
            (RUNTIME_LAUNCH_REASONING_EFFORT_KEY, json!("medium")),
            ("parallel.results", results),
        ]),
        &launch_paths,
        temp.path(),
    );
    assert_eq!(launch_outcome.status, OutcomeStatus::Success);
    let launch_call = launch_calls.lock().expect("launch calls lock");
    assert_eq!(launch_call[0].model.as_deref(), Some("gpt-launch"));
    assert_eq!(launch_call[0].provider, "openai");
    assert_eq!(launch_call[0].llm_profile, "launch-profile");
    assert_eq!(launch_call[0].reasoning_effort, "medium");
}

#[test]
fn handler_panics_normalize_to_durable_pipeline_failure() {
    let graph = parse_graph(
        r#"
        digraph G {
          start [shape=Mdiamond]
          boom [shape=box, type="custom.panic"]
          done [shape=Msquare]
          start -> boom -> done
        }
        "#,
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let project_path = temp.path().join("Project");
    std::fs::create_dir_all(&project_path).expect("project");
    let store = RunStore::for_runs_dir(temp.path().join("spark-home/attractor/runs"));
    let mut record = RunRecord::new("run-handler-panic", project_path.to_string_lossy());
    record.started_at = "2026-06-23T10:00:00Z".to_string();
    let mut runner = RuntimeHandlerRunner::new();
    runner.register_handler_fn("custom.panic", |_runtime| panic!("handler exploded"));
    let mut executor = PipelineExecutor::new(runner);

    let old_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = executor
        .execute(ExecuteRunRequest {
            store: store.clone(),
            record,
            graph,
            graph_source: None,
            graph_dot: None,
            launch_context: LaunchContext::empty(),
            runtime_context: Default::default(),
            max_steps: None,
            start: Default::default(),
        })
        .expect("pipeline result");
    std::panic::set_hook(old_hook);
    assert_eq!(result.status, "failed");
    assert_eq!(result.current_node, "boom");
    assert!(result
        .failure_reason
        .contains("handler panic: handler exploded"));
    assert_eq!(
        result.node_outcomes["boom"].failure_kind,
        Some(FailureKind::Runtime)
    );

    let paths = store
        .run_root(&project_path.to_string_lossy(), "run-handler-panic")
        .expect("run paths");
    let status: Value = serde_json::from_str(
        &std::fs::read_to_string(paths.logs_dir().join("boom/status.json"))
            .expect("status artifact"),
    )
    .expect("status json");
    assert_eq!(status["outcome"], json!("fail"));
    assert_eq!(status["failure_kind"], json!("runtime"));
}
