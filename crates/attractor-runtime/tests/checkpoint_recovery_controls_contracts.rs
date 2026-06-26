use std::collections::BTreeMap;
use std::path::Path;

use attractor_core::{CheckpointState, DotGraph, LaunchContext, Outcome, OutcomeStatus, RunRecord};
use attractor_dsl::parse_dot;
use attractor_runtime::{
    normalize_checkpoint_for_write, read_checkpoint, read_run_record, CheckpointWriteOptions,
    ContinueRunRequest, CreateRunRequest, ExecuteRunRequest, ExecutionControlAction,
    ExecutionStart, NodeExecutionRequest, PipelineExecutor, RunStore, RuntimeControls,
    RUNTIME_FIDELITY_KEY,
};
use serde_json::{json, Value};
use spark_storage::read_json;

fn parse_graph(dot: &str) -> DotGraph {
    parse_dot(dot).expect("dot parses")
}

fn temp_store(temp: &tempfile::TempDir) -> RunStore {
    RunStore::for_runs_dir(temp.path().join("spark-home/attractor/runs"))
}

fn record(run_id: &str, project_path: &Path) -> RunRecord {
    let mut record = RunRecord::new(run_id, project_path.to_string_lossy());
    record.flow_name = "checkpoint-contract".to_string();
    record.model = "compat-model".to_string();
    record.started_at = "2026-06-23T10:00:00Z".to_string();
    record
}

fn checkpoint(current_node: &str, completed_nodes: &[&str]) -> CheckpointState {
    CheckpointState {
        timestamp: "2026-06-23T10:00:00Z".to_string(),
        current_node: current_node.to_string(),
        completed_nodes: completed_nodes
            .iter()
            .map(|node| node.to_string())
            .collect(),
        context: BTreeMap::from([
            ("outcome".to_string(), json!("success")),
            ("preferred_label".to_string(), json!("")),
            (
                "_attractor.node_outcomes".to_string(),
                json!({"start": "success", "plan": "success", "work": "fail"}),
            ),
        ]),
        retry_counts: BTreeMap::new(),
        logs: Vec::new(),
    }
}

fn create_source_run(
    store: &RunStore,
    temp: &tempfile::TempDir,
    run_id: &str,
    status: &str,
    checkpoint: CheckpointState,
    graph_source: Option<&str>,
) -> (std::path::PathBuf, attractor_runtime::RunRootPaths) {
    let project_path = temp.path().join(format!("Project {run_id}"));
    std::fs::create_dir_all(&project_path).expect("project dir");
    let mut record = record(run_id, &project_path);
    record.status = status.to_string();
    record.outcome = (status == "completed").then(|| "success".to_string());
    record.last_error = if status == "failed" {
        "previous failure".to_string()
    } else {
        String::new()
    };
    let paths = store
        .create_run(CreateRunRequest {
            record,
            checkpoint: Some(checkpoint),
            graph_source: graph_source.map(str::to_string),
            graph_dot: graph_source.map(str::to_string),
            ..CreateRunRequest::default()
        })
        .expect("create source run");
    (project_path, paths)
}

fn success() -> Outcome {
    Outcome::new(OutcomeStatus::Success)
}

#[test]
fn checkpoint_writes_fill_timestamp_and_mirror_all_compatible_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = temp_store(&temp);
    let project_path = temp.path().join("Project Checkpoint");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let paths = store
        .run_root(&project_path.to_string_lossy(), "run-checkpoint")
        .expect("paths");
    std::fs::create_dir_all(paths.logs_dir()).expect("logs dir");
    let checkpoint = CheckpointState::new("start");

    store
        .save_checkpoint(&paths, &checkpoint, CheckpointWriteOptions::default())
        .expect("save checkpoint");

    for path in [
        paths.state_json(),
        paths.checkpoint_json(),
        paths.logs_checkpoint_json(),
    ] {
        let raw: Value = read_json(&path).expect("checkpoint json");
        assert_eq!(raw["current_node"], json!("start"));
        assert!(raw["timestamp"]
            .as_str()
            .is_some_and(|value| !value.is_empty()));
    }
    assert!(normalize_checkpoint_for_write(&checkpoint)
        .timestamp
        .starts_with("20"));
}

#[test]
fn resume_from_checkpoint_executes_current_or_routed_next_node_and_skips_terminal_checkpoint() {
    let graph = parse_graph(
        r#"
        digraph G {
          start [shape=Mdiamond]
          plan [shape=box]
          review [shape=box]
          done [shape=Msquare]
          start -> plan -> review -> done
        }
        "#,
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let store = temp_store(&temp);
    let (project_path, paths) = create_source_run(
        &store,
        &temp,
        "run-resume-current",
        "paused",
        checkpoint("plan", &["start"]),
        None,
    );
    let mut calls = Vec::<String>::new();
    let mut executor = PipelineExecutor::new(|request: NodeExecutionRequest| {
        calls.push(request.node_id);
        Ok(success())
    });
    let result = executor
        .execute(ExecuteRunRequest {
            store: store.clone(),
            record: record("run-resume-current", &project_path),
            graph: graph.clone(),
            graph_source: None,
            graph_dot: None,
            launch_context: LaunchContext::empty(),
            runtime_context: Default::default(),
            max_steps: None,
            start: ExecutionStart::Resume {
                paths,
                checkpoint: checkpoint("plan", &["start"]),
            },
        })
        .expect("resume current");
    assert_eq!(result.status, "completed");
    assert_eq!(calls, ["plan", "review"]);

    let (project_path, paths) = create_source_run(
        &store,
        &temp,
        "run-resume-next",
        "paused",
        checkpoint("plan", &["start", "plan"]),
        None,
    );
    let mut calls = Vec::<String>::new();
    let mut executor = PipelineExecutor::new(|request: NodeExecutionRequest| {
        calls.push(request.node_id);
        Ok(success())
    });
    let result = executor
        .execute(ExecuteRunRequest {
            store: store.clone(),
            record: record("run-resume-next", &project_path),
            graph: graph.clone(),
            graph_source: None,
            graph_dot: None,
            launch_context: LaunchContext::empty(),
            runtime_context: Default::default(),
            max_steps: None,
            start: ExecutionStart::Resume {
                paths,
                checkpoint: checkpoint("plan", &["start", "plan"]),
            },
        })
        .expect("resume next");
    assert_eq!(result.status, "completed");
    assert_eq!(result.completed_nodes, ["start", "plan", "review"]);
    assert_eq!(calls, ["review"]);

    let (_project_path, paths) = create_source_run(
        &store,
        &temp,
        "run-resume-terminal",
        "completed",
        checkpoint("done", &["start", "plan", "review"]),
        None,
    );
    let mut executor = PipelineExecutor::new(|_request: NodeExecutionRequest| {
        panic!("terminal checkpoint should not execute nodes")
    });
    let result = executor
        .execute(ExecuteRunRequest {
            store,
            record: record("run-resume-terminal", temp.path()),
            graph,
            graph_source: None,
            graph_dot: None,
            launch_context: LaunchContext::empty(),
            runtime_context: Default::default(),
            max_steps: None,
            start: ExecutionStart::Resume {
                paths,
                checkpoint: checkpoint("done", &["start", "plan", "review"]),
            },
        })
        .expect("terminal checkpoint");
    assert_eq!(result.status, "completed");
    assert_eq!(result.current_node, "done");
}

#[test]
fn resumed_full_fidelity_degrades_first_hop_once_then_restores_graph_fidelity() {
    let graph = parse_graph(
        r#"
        digraph G {
          graph [default_fidelity="full"]
          start [shape=Mdiamond]
          plan [shape=box]
          review [shape=box]
          done [shape=Msquare]
          start -> plan -> review -> done
        }
        "#,
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let store = temp_store(&temp);
    let mut resume_checkpoint = checkpoint("plan", &["start"]);
    resume_checkpoint
        .context
        .insert(RUNTIME_FIDELITY_KEY.to_string(), json!("full"));
    let (project_path, paths) = create_source_run(
        &store,
        &temp,
        "run-resume-fidelity",
        "paused",
        resume_checkpoint.clone(),
        None,
    );
    let mut seen = Vec::<String>::new();
    let mut executor = PipelineExecutor::new(|request: NodeExecutionRequest| {
        seen.push(
            request
                .context
                .get(RUNTIME_FIDELITY_KEY)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        );
        Ok(success())
    });

    let result = executor
        .execute(ExecuteRunRequest {
            store,
            record: record("run-resume-fidelity", &project_path),
            graph,
            graph_source: None,
            graph_dot: None,
            launch_context: LaunchContext::empty(),
            runtime_context: Default::default(),
            max_steps: None,
            start: ExecutionStart::Resume {
                paths,
                checkpoint: resume_checkpoint,
            },
        })
        .expect("resume");

    assert_eq!(result.status, "completed");
    assert_eq!(seen, ["summary:high", "full"]);
}

#[test]
fn continue_retry_pause_and_cancel_controls_update_durable_run_state() {
    let graph_source = r#"
    digraph G {
      start [shape=Mdiamond]
      work [shape=box]
      done [shape=Msquare]
      start -> work -> done
    }
    "#;
    let graph = parse_graph(graph_source);
    let temp = tempfile::tempdir().expect("tempdir");
    let store = temp_store(&temp);
    let controls = RuntimeControls::new(store.clone());
    let mut source_checkpoint = checkpoint("work", &["start", "work"]);
    source_checkpoint
        .context
        .insert("context.seed".to_string(), json!("from-source"));
    let (_project_path, source_paths) = create_source_run(
        &store,
        &temp,
        "run-source",
        "failed",
        source_checkpoint.clone(),
        Some(graph_source),
    );
    let source_run_before = std::fs::read_to_string(source_paths.run_json()).expect("source run");
    let source_checkpoint_before =
        std::fs::read_to_string(source_paths.state_json()).expect("source checkpoint");

    let continued = controls
        .continue_from_snapshot(ContinueRunRequest {
            source_run_id: "run-source".to_string(),
            start_node: "work".to_string(),
            flow_source_mode: "snapshot".to_string(),
            flow_name: Some("continued.dot".to_string()),
            new_run_id: Some("run-continued".to_string()),
            graph: graph.clone(),
            graph_source: Some(graph_source.to_string()),
            graph_dot: Some(graph_source.to_string()),
            working_directory: None,
            model: None,
            llm_provider: None,
            llm_profile: None,
            reasoning_effort: None,
        })
        .expect("continue run");
    assert_eq!(continued.status, "started");
    let continued_bundle = store
        .read_run_bundle("run-continued")
        .expect("bundle")
        .expect("continued bundle");
    let continued_record = continued_bundle.record.expect("record");
    assert_eq!(
        continued_record.continued_from_run_id.as_deref(),
        Some("run-source")
    );
    assert_eq!(
        continued_record.continued_from_node.as_deref(),
        Some("work")
    );
    assert_eq!(
        continued_record.continued_from_flow_mode.as_deref(),
        Some("snapshot")
    );
    let continued_checkpoint = continued_bundle.checkpoint.expect("checkpoint");
    assert_eq!(continued_checkpoint.current_node, "work");
    assert_eq!(
        continued_checkpoint.context["context.seed"],
        json!("from-source")
    );
    assert_eq!(
        std::fs::read_to_string(source_paths.run_json()).expect("source run after"),
        source_run_before
    );
    assert_eq!(
        std::fs::read_to_string(source_paths.state_json()).expect("source checkpoint after"),
        source_checkpoint_before
    );

    let retry = controls.prepare_retry("run-source").expect("prepare retry");
    assert_eq!(retry.status, "started");
    assert_eq!(retry.run_id, "run-source");
    assert_eq!(retry.completed_nodes, ["start"]);
    let retry_record = read_run_record(&source_paths)
        .expect("read retry record")
        .expect("retry record");
    assert_eq!(retry_record.status, "running");
    assert!(retry_record.last_error.is_empty());
    let retry_checkpoint = read_checkpoint(&source_paths)
        .expect("read retry checkpoint")
        .expect("retry checkpoint");
    assert_eq!(
        retry_checkpoint.context["internal.pipeline_retry_run_id"],
        json!("run-source")
    );
    assert_eq!(retry_checkpoint.completed_nodes, ["start"]);
    let retry_journal = store.read_journal(&source_paths).expect("journal");
    assert!(retry_journal
        .iter()
        .any(|entry| entry.summary == "Retry started from work"));

    let cancel = controls.request_cancel("run-source").expect("cancel");
    assert_eq!(cancel.status, "cancel_requested");
    let canceled_record = read_run_record(&source_paths)
        .expect("read canceled record")
        .expect("canceled record");
    assert_eq!(canceled_record.status, "cancel_requested");
    assert_eq!(canceled_record.last_error, "cancel_requested_by_user");
    let cancel_events = store.read_raw_events(&source_paths).expect("events");
    assert!(cancel_events.iter().any(|event| {
        event.event_type == "log"
            && event.payload.get("msg")
                == Some(&json!(
                    "[System] Cancel requested. Stopping after current node."
                ))
    }));

    let pause = controls.request_pause("run-source").expect("pause");
    assert_eq!(pause.status, "paused");
    let paused_record = read_run_record(&source_paths)
        .expect("read paused record")
        .expect("paused record");
    assert_eq!(paused_record.status, "paused");
}

#[test]
fn executor_control_polling_persists_pause_and_cancel_results() {
    let graph = parse_graph(
        r#"
        digraph G {
          start [shape=Mdiamond]
          plan [shape=box]
          done [shape=Msquare]
          start -> plan -> done
        }
        "#,
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let project_path = temp.path().join("Project Controls");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let store = temp_store(&temp);
    let mut polls = 0u64;
    let mut executor = PipelineExecutor::with_control(
        |_request: NodeExecutionRequest| Ok(success()),
        move || {
            polls += 1;
            (polls == 2).then_some(ExecutionControlAction::Pause)
        },
    );
    let result = executor
        .execute(ExecuteRunRequest {
            store: store.clone(),
            record: record("run-pause", &project_path),
            graph: graph.clone(),
            graph_source: None,
            graph_dot: None,
            launch_context: LaunchContext::empty(),
            runtime_context: Default::default(),
            max_steps: None,
            start: ExecutionStart::Fresh,
        })
        .expect("pause execute");
    assert_eq!(result.status, "paused");
    assert_eq!(result.completed_nodes, ["start"]);
    let pause_paths = store
        .run_root(&project_path.to_string_lossy(), "run-pause")
        .expect("pause paths");
    let pause_record = read_run_record(&pause_paths)
        .expect("pause record")
        .expect("pause record");
    assert_eq!(pause_record.status, "paused");

    let mut executor = PipelineExecutor::with_control(
        |_request: NodeExecutionRequest| Ok(success()),
        || Some(ExecutionControlAction::Cancel),
    );
    let result = executor
        .execute(ExecuteRunRequest {
            store: store.clone(),
            record: record("run-cancel", &project_path),
            graph,
            graph_source: None,
            graph_dot: None,
            launch_context: LaunchContext::empty(),
            runtime_context: Default::default(),
            max_steps: None,
            start: ExecutionStart::Fresh,
        })
        .expect("cancel execute");
    assert_eq!(result.status, "canceled");
    let cancel_paths = store
        .run_root(&project_path.to_string_lossy(), "run-cancel")
        .expect("cancel paths");
    let cancel_record = read_run_record(&cancel_paths)
        .expect("cancel record")
        .expect("cancel record");
    assert_eq!(cancel_record.status, "canceled");
    assert_eq!(cancel_record.last_error, "aborted_by_user");
}
