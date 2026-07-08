use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use attractor_core::{DotGraph, LaunchContext, Outcome, OutcomeStatus, RunRecord};
use attractor_dsl::parse_dot;
use attractor_runtime::{
    disk_execution_control, prepare_fresh_run, ExecuteRunRequest, ExecutionStart,
    NodeExecutionRequest, PipelineExecutor, RunStore, RuntimeControls,
};

fn parse_graph(dot: &str) -> DotGraph {
    parse_dot(dot).expect("dot parses")
}

fn simple_graph() -> DotGraph {
    parse_graph(
        r#"
        digraph G {
          start [shape=Mdiamond]
          plan [shape=box]
          build [shape=box]
          done [shape=Msquare]
          start -> plan -> build -> done
        }
        "#,
    )
}

fn temp_store(temp: &tempfile::TempDir) -> RunStore {
    RunStore::for_runs_dir(temp.path().join("spark-home/attractor/runs"))
}

fn record(run_id: &str, project_path: &Path) -> RunRecord {
    let mut record = RunRecord::new(run_id, project_path.to_string_lossy());
    record.flow_name = "detached-contract".to_string();
    record.model = "compat-model".to_string();
    record.started_at = "2026-07-08T10:00:00Z".to_string();
    record
}

fn success() -> Outcome {
    Outcome::new(OutcomeStatus::Success)
}

fn journal_shape(
    store: &RunStore,
    project_path: &Path,
    run_id: &str,
) -> Vec<(String, Option<String>)> {
    let paths = store
        .run_root(&project_path.to_string_lossy(), run_id)
        .expect("run root");
    store
        .read_journal(&paths)
        .expect("journal")
        .into_iter()
        .map(|entry| (entry.raw_type, entry.node_id))
        .collect()
}

#[test]
fn prepared_start_produces_the_same_journal_as_fresh() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = temp_store(&temp);
    let project_path = temp.path().join("Project Parity");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let graph = simple_graph();

    // Baseline: today's Fresh execution.
    let mut executor = PipelineExecutor::new(|_request: NodeExecutionRequest| Ok(success()));
    let fresh_result = executor
        .execute(ExecuteRunRequest {
            store: store.clone(),
            record: record("run-fresh", &project_path),
            graph: graph.clone(),
            graph_source: None,
            graph_dot: None,
            launch_context: LaunchContext::empty(),
            runtime_context: Default::default(),
            max_steps: None,
            start: ExecutionStart::Fresh,
        })
        .expect("fresh execute");
    assert_eq!(fresh_result.status, "completed");

    // Detached shape: prepare synchronously, then execute from Prepared.
    let prepared_record = record("run-prepared", &project_path);
    let paths = prepare_fresh_run(
        &store,
        &prepared_record,
        &graph,
        None,
        None,
        &LaunchContext::empty(),
        &Default::default(),
    )
    .expect("prepare run");
    let prepared_run_record = store
        .read_run_record(&paths)
        .expect("read prepared record")
        .expect("prepared record");
    assert_eq!(prepared_run_record.status, "running");

    let mut executor = PipelineExecutor::new(|_request: NodeExecutionRequest| Ok(success()));
    let prepared_result = executor
        .execute(ExecuteRunRequest {
            store: store.clone(),
            record: prepared_record,
            graph,
            graph_source: None,
            graph_dot: None,
            launch_context: LaunchContext::empty(),
            runtime_context: Default::default(),
            max_steps: None,
            start: ExecutionStart::Prepared { paths },
        })
        .expect("prepared execute");
    assert_eq!(prepared_result.status, "completed");
    assert_eq!(
        prepared_result.completed_nodes,
        fresh_result.completed_nodes
    );

    assert_eq!(
        journal_shape(&store, &project_path, "run-prepared"),
        journal_shape(&store, &project_path, "run-fresh"),
        "Prepared runs must write the same journal a Fresh run does",
    );
}

#[test]
fn run_event_observer_fires_for_store_writes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project_path = temp.path().join("Project Observer");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let observed = Arc::new(AtomicUsize::new(0));
    let observer_hits = Arc::clone(&observed);
    let store = temp_store(&temp).with_run_event_observer(Arc::new(move |run_id: &str| {
        assert_eq!(run_id, "run-observed");
        observer_hits.fetch_add(1, Ordering::SeqCst);
    }));

    let mut executor = PipelineExecutor::new(|_request: NodeExecutionRequest| Ok(success()));
    executor
        .execute(ExecuteRunRequest {
            store,
            record: record("run-observed", &project_path),
            graph: simple_graph(),
            graph_source: None,
            graph_dot: None,
            launch_context: LaunchContext::empty(),
            runtime_context: Default::default(),
            max_steps: None,
            start: ExecutionStart::Fresh,
        })
        .expect("observed execute");

    assert!(
        observed.load(Ordering::SeqCst) >= 10,
        "expected a notification per durable run mutation, got {}",
        observed.load(Ordering::SeqCst),
    );
}

#[test]
fn disk_control_cancels_a_run_mid_flight() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = temp_store(&temp);
    let project_path = temp.path().join("Project Cancel");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let graph = simple_graph();

    let cancel_record = record("run-disk-cancel", &project_path);
    let paths = prepare_fresh_run(
        &store,
        &cancel_record,
        &graph,
        None,
        None,
        &LaunchContext::empty(),
        &Default::default(),
    )
    .expect("prepare run");

    // Slow node executor: each node takes 50ms, giving the control thread
    // time to observe a persisted cancel request.
    let executor_store = store.clone();
    let control_paths = paths.clone();
    let start_paths = paths.clone();
    let handle = std::thread::spawn(move || {
        let mut executor = PipelineExecutor::with_control(
            |_request: NodeExecutionRequest| {
                std::thread::sleep(Duration::from_millis(50));
                Ok(success())
            },
            disk_execution_control(control_paths),
        );
        executor.execute(ExecuteRunRequest {
            store: executor_store,
            record: cancel_record,
            graph,
            graph_source: None,
            graph_dot: None,
            launch_context: LaunchContext::empty(),
            runtime_context: Default::default(),
            max_steps: None,
            start: ExecutionStart::Prepared { paths: start_paths },
        })
    });

    let controls = RuntimeControls::new(store.clone());
    // Wait for the run to actually be executing, then request cancellation.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = store
            .read_run_record(&paths)
            .ok()
            .flatten()
            .map(|record| record.status);
        if status.as_deref() == Some("running") {
            break;
        }
        assert!(Instant::now() < deadline, "run never reached running state");
        std::thread::sleep(Duration::from_millis(5));
    }
    controls
        .request_cancel("run-disk-cancel")
        .expect("cancel request");

    let result = handle
        .join()
        .expect("executor thread")
        .expect("execution result");
    assert_eq!(result.status, "canceled");
    let final_record = store
        .read_run_record(&paths)
        .expect("final record")
        .expect("record");
    assert_eq!(final_record.status, "canceled");
}
