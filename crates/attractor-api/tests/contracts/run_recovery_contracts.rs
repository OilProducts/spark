use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use attractor_api::AttractorApiService;
use attractor_core::CheckpointState;
use attractor_runtime::{
    human_gate_answered_event, prepare_fresh_run, RunStore, RuntimeHandlerRunner,
};
use serde_json::json;
use spark_common::settings::SparkSettings;

fn settings(root: &Path) -> SparkSettings {
    SparkSettings {
        project_root: root.join("project"),
        data_dir: root.join("spark-home"),
        config_dir: root.join("spark-home/config"),
        runtime_dir: root.join("spark-home/runtime"),
        logs_dir: root.join("spark-home/logs"),
        workspace_dir: root.join("spark-home/workspace"),
        projects_dir: root.join("spark-home/workspace/projects"),
        attractor_dir: root.join("spark-home/attractor"),
        runs_dir: root.join("spark-home/attractor/runs"),
        flows_dir: root.join("spark-home/flows"),
        ui_dir: None,
        project_roots: Vec::new(),
    }
}

const GATE_FLOW: &str = r#"schema_version: "1"
id: recovery_gate
title: Recovery Gate
nodes:
  start:
    kind: start
  review:
    kind: human_gate
    config:
      kind: human_gate
      prompt: Ship the report?
  done:
    kind: exit
edges:
- from: start
  to: review
- from: review
  to: done
  label: Finish
"#;

/// Creates a run whose durable state says "parked at the review gate" with no
/// executor attached — exactly what a server restart leaves behind.
fn manufacture_orphaned_waiting_run(
    settings: &SparkSettings,
    workdir: &Path,
    run_id: &str,
) -> RunStore {
    let store = RunStore::for_settings(settings);
    let flow = attractor_dsl::parse_flow_definition(GATE_FLOW).expect("gate flow parses");
    let mut record = attractor_core::RunRecord::new(run_id, workdir.to_string_lossy());
    record.flow_name = "recovery-gate".to_string();
    let launch_context = attractor_core::LaunchContext::empty();
    let runtime_context = attractor_core::ContextMap::from([(
        "internal.run_workdir".to_string(),
        json!(workdir.to_string_lossy().to_string()),
    )]);
    let paths = prepare_fresh_run(
        &store,
        &record,
        &flow,
        Some(GATE_FLOW.to_string()),
        None,
        &launch_context,
        &runtime_context,
    )
    .expect("prepare run");
    let checkpoint = CheckpointState {
        timestamp: "2026-07-14T12:00:00Z".to_string(),
        current_node: "review".to_string(),
        completed_nodes: vec!["start".to_string()],
        context: runtime_context,
        retry_counts: Default::default(),
        logs: Vec::new(),
    };
    store
        .save_checkpoint(&paths, &checkpoint, Default::default())
        .expect("save checkpoint");
    store
        .update_run_record(run_id, |record| {
            record.status = "waiting".to_string();
        })
        .expect("mark waiting");
    store
}

fn blocking_gate_service(settings: &SparkSettings) -> AttractorApiService {
    AttractorApiService::new_with_runtime_handler_runner_factory(
        settings.clone(),
        Arc::new(|| RuntimeHandlerRunner::new().with_blocking_human_gates()),
    )
}

fn wait_for_status(store: &RunStore, run_id: &str, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let status = store
            .read_run_bundle(run_id)
            .expect("read bundle")
            .and_then(|bundle| bundle.record)
            .map(|record| attractor_runtime::normalize_run_status(&record.status))
            .unwrap_or_default();
        if status == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "run never reached {expected}; last status {status}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn startup_recovery_resumes_orphaned_waiting_run_and_consumes_journaled_answer() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.config_dir).expect("config dir");
    let workdir = temp.path().join("project");
    std::fs::create_dir_all(&workdir).expect("workdir");
    let store = manufacture_orphaned_waiting_run(&settings, &workdir, "run-orphan-answered");

    // The user answered after the executor died: the answer is journaled,
    // nothing is consuming it.
    let bundle = store
        .read_run_bundle("run-orphan-answered")
        .expect("bundle")
        .expect("bundle exists");
    store
        .append_event(
            &bundle.paths,
            human_gate_answered_event(
                "run-orphan-answered",
                "review-1",
                Some("review".to_string()),
                Some("recovery-gate".to_string()),
                Some("Ship the report?".to_string()),
                "Finish",
                None,
            ),
        )
        .expect("journal answer");

    let recovery = blocking_gate_service(&settings).recover_interrupted_runs();
    assert_eq!(
        recovery["resumed"],
        json!(["run-orphan-answered"]),
        "{recovery:?}"
    );

    wait_for_status(&store, "run-orphan-answered", "completed");
}

#[test]
fn cancel_finalizes_an_orphaned_run_immediately() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.config_dir).expect("config dir");
    let workdir = temp.path().join("project");
    std::fs::create_dir_all(&workdir).expect("workdir");
    let store = manufacture_orphaned_waiting_run(&settings, &workdir, "run-orphan-cancel");

    let response = blocking_gate_service(&settings).cancel_pipeline_route("run-orphan-cancel");
    assert_eq!(response.status_code, 200, "{:?}", response.body);
    assert_eq!(
        response.body["status"],
        json!("canceled"),
        "{:?}",
        response.body
    );

    let status = store
        .read_run_bundle("run-orphan-cancel")
        .expect("bundle")
        .and_then(|bundle| bundle.record)
        .map(|record| attractor_runtime::normalize_run_status(&record.status))
        .unwrap_or_default();
    assert_eq!(status, "canceled", "no executor needed to finalize");
}

#[test]
fn startup_recovery_resumes_unanswered_gate_back_into_waiting() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.config_dir).expect("config dir");
    let workdir = temp.path().join("project");
    std::fs::create_dir_all(&workdir).expect("workdir");
    let store = manufacture_orphaned_waiting_run(&settings, &workdir, "run-orphan-pending");

    let recovery = blocking_gate_service(&settings).recover_interrupted_runs();
    assert_eq!(
        recovery["resumed"],
        json!(["run-orphan-pending"]),
        "{recovery:?}"
    );

    // The resumed run re-enters the gate wait and republishes its pending
    // question; answering it then completes the run.
    wait_for_status(&store, "run-orphan-pending", "waiting");
    let bundle = store
        .read_run_bundle("run-orphan-pending")
        .expect("bundle")
        .expect("bundle exists");
    store
        .append_event(
            &bundle.paths,
            human_gate_answered_event(
                "run-orphan-pending",
                "review-1",
                Some("review".to_string()),
                Some("recovery-gate".to_string()),
                Some("Ship the report?".to_string()),
                "Finish",
                None,
            ),
        )
        .expect("journal answer");
    wait_for_status(&store, "run-orphan-pending", "completed");
}

#[test]
fn startup_recovery_marks_orphaned_running_runs_failed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.config_dir).expect("config dir");
    let workdir = temp.path().join("project");
    std::fs::create_dir_all(&workdir).expect("workdir");
    let store = RunStore::for_settings(&settings);
    let flow = attractor_dsl::parse_flow_definition(GATE_FLOW).expect("gate flow parses");
    let launch_context = attractor_core::LaunchContext::empty();
    let runtime_context = attractor_core::ContextMap::default();

    // A root and its child, both left in `running` by a dead process.
    for (run_id, parent) in [
        ("run-orphan-running-root", None),
        ("run-orphan-running-child", Some("run-orphan-running-root")),
    ] {
        let mut record = attractor_core::RunRecord::new(run_id, workdir.to_string_lossy());
        record.flow_name = "recovery-gate".to_string();
        record.parent_run_id = parent.map(str::to_string);
        prepare_fresh_run(
            &store,
            &record,
            &flow,
            Some(GATE_FLOW.to_string()),
            None,
            &launch_context,
            &runtime_context,
        )
        .expect("prepare run");
        store
            .update_run_record(run_id, |record| {
                record.status = "running".to_string();
            })
            .expect("mark running");
    }

    let recovery = blocking_gate_service(&settings).recover_interrupted_runs();
    let mut interrupted: Vec<String> = recovery["interrupted"]
        .as_array()
        .expect("interrupted list")
        .iter()
        .map(|value| value.as_str().expect("run id").to_string())
        .collect();
    interrupted.sort();
    assert_eq!(
        interrupted,
        vec![
            "run-orphan-running-child".to_string(),
            "run-orphan-running-root".to_string(),
        ],
        "{recovery:?}"
    );

    for run_id in ["run-orphan-running-root", "run-orphan-running-child"] {
        let record = store
            .read_run_bundle(run_id)
            .expect("read bundle")
            .expect("bundle exists")
            .record
            .expect("record");
        assert_eq!(record.status, "failed");
        assert!(
            record
                .last_error
                .contains("interrupted by an earlier restart"),
            "unexpected last_error: {}",
            record.last_error
        );
    }
}
