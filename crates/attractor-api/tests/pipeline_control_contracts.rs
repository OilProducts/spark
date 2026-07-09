use std::fs;
use std::path::Path;

use attractor_api::{handle_attractor_request, AttractorApiService, PipelineStartRequest};
use attractor_core::{CheckpointState, RunRecord};
use attractor_runtime::{CreateRunRequest, RunStore};
use serde_json::json;
use spark_common::settings::SparkSettings;

#[test]
fn control_routes_preserve_retry_cancel_continue_and_metadata_shapes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("Project Controls");
    fs::create_dir_all(&project_path).expect("project dir");
    let store = RunStore::for_settings(&settings);
    store
        .create_run(CreateRunRequest {
            record: failed_record("run-control", &project_path),
            checkpoint: Some(checkpoint("task")),
            flow_source: Some(simple_flow()),
            flow_definition_json: Some(simple_flow()),
            ..CreateRunRequest::default()
        })
        .expect("failed run");
    let service = AttractorApiService::new(settings.clone());

    let continue_guard = handle_attractor_request(
        "POST",
        "/attractor/pipelines/run-control/continue",
        &json!({"start_node": "", "flow_source_mode": "snapshot"}).to_string(),
        settings.clone(),
    );
    assert_eq!(continue_guard.status_code, 200);
    assert_eq!(
        continue_guard.body,
        json!({"status": "validation_error", "error": "start_node is required."})
    );

    let retry = service.retry_pipeline_route("run-control");
    assert_eq!(retry.status_code, 200);
    assert_eq!(retry.body["status"], json!("started"));
    assert_eq!(retry.body["pipeline_id"], json!("run-control"));

    // The retry executes detached; wait for it to finish so the metadata
    // patch below cannot race the executor's record writes.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let status = store
            .read_run_bundle("run-control")
            .expect("read")
            .and_then(|bundle| bundle.record)
            .map(|record| record.status);
        if status.as_deref() == Some("completed") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "retried run never completed (last: {status:?})",
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Cancel-request shape, proven against a run that is active on disk but
    // has no executor attached, so the response is deterministic.
    store
        .create_run(CreateRunRequest {
            record: running_record("run-control-cancel", &project_path),
            checkpoint: Some(checkpoint("task")),
            flow_source: Some(simple_flow()),
            flow_definition_json: Some(simple_flow()),
            ..CreateRunRequest::default()
        })
        .expect("running run");
    let cancel = service.cancel_pipeline_route("run-control-cancel");
    assert_eq!(cancel.status_code, 200);
    assert_eq!(
        cancel.body,
        json!({"status": "cancel_requested", "pipeline_id": "run-control-cancel"})
    );

    let patch = service.patch_pipeline_metadata(
        "run-control",
        attractor_api::PipelineMetadataUpdateRequest {
            spec_id: Some("spec-updated".to_string()),
            plan_id: Some("plan-updated".to_string()),
        },
    );
    assert_eq!(patch.status_code, 200);
    assert_eq!(patch.body["spec_id"], json!("spec-updated"));
    assert_eq!(patch.body["plan_id"], json!("plan-updated"));
    let patched = store
        .read_run_bundle("run-control")
        .expect("read")
        .expect("run")
        .record
        .expect("record");
    assert_eq!(patched.spec_id.as_deref(), Some("spec-updated"));
    assert_eq!(patched.plan_id.as_deref(), Some("plan-updated"));

    let retry_missing = service.retry_pipeline_route("missing");
    assert_eq!(retry_missing.status_code, 404);
    assert_eq!(retry_missing.body, json!({"detail": "Unknown pipeline"}));
}

#[test]
fn continue_inherits_source_launch_context() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("Project Continue Context");
    fs::create_dir_all(&project_path).expect("project dir");
    let store = RunStore::for_settings(&settings);
    let mut source_record = failed_record("run-continue-ctx", &project_path);
    source_record.launch_context = Some(
        [("context.topic".to_string(), json!("api"))]
            .into_iter()
            .collect(),
    );
    store
        .create_run(CreateRunRequest {
            record: source_record,
            checkpoint: Some(checkpoint("task")),
            flow_source: Some(simple_flow()),
            flow_definition_json: Some(simple_flow()),
            ..CreateRunRequest::default()
        })
        .expect("failed run");

    let continued = handle_attractor_request(
        "POST",
        "/attractor/pipelines/run-continue-ctx/continue",
        &json!({"start_node": "task", "flow_source_mode": "snapshot"}).to_string(),
        settings.clone(),
    );
    assert_eq!(continued.status_code, 200);
    assert_eq!(continued.body["status"], json!("started"));
    let new_run_id = continued.body["pipeline_id"]
        .as_str()
        .expect("pipeline id")
        .to_string();

    let derived = store
        .read_run_bundle(&new_run_id)
        .expect("read derived run")
        .expect("derived run exists")
        .record
        .expect("derived record");
    assert_eq!(
        derived.launch_context,
        Some(
            [("context.topic".to_string(), json!("api"))]
                .into_iter()
                .collect()
        )
    );
}

#[test]
fn cancel_terminal_run_is_ignored_and_steer_records_rejected_intervention() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let project_path = temp.path().join("Project Steer");
    let service = AttractorApiService::new(settings.clone());
    let started = service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("run-steer".to_string()),
        flow_content: Some(simple_flow()),
        working_directory: project_path.to_string_lossy().to_string(),
        ..PipelineStartRequest::default()
    });
    assert_eq!(started.body["status"], json!("started"));

    let cancel_terminal = service.cancel_pipeline_route("run-steer");
    assert_eq!(
        cancel_terminal.body,
        json!({"status": "ignored", "pipeline_id": "run-steer"})
    );

    let empty_steer = handle_attractor_request(
        "POST",
        "/attractor/pipelines/run-steer/steer",
        &json!({"message": ""}).to_string(),
        settings.clone(),
    );
    assert_eq!(
        empty_steer.body,
        json!({"status": "validation_error", "error": "message is required."})
    );

    let steer = handle_attractor_request(
        "POST",
        "/attractor/pipelines/run-steer/steer",
        &json!({"message": "please inspect", "target_run_id": "child-run"}).to_string(),
        settings.clone(),
    );
    assert_eq!(steer.status_code, 200);
    assert_eq!(steer.body["pipeline_id"], json!("run-steer"));
    assert_eq!(steer.body["target_run_id"], json!("child-run"));
    assert_eq!(steer.body["status"], json!("rejected"));
    assert_eq!(steer.body["reason"], json!("no_active_child_run"));

    let bundle = RunStore::for_settings(&settings)
        .read_run_bundle("run-steer")
        .expect("read")
        .expect("run");
    assert!(bundle
        .raw_events
        .iter()
        .any(|event| event.event_type == "HumanInterventionRequested"));
    assert!(bundle
        .journal
        .iter()
        .any(|entry| entry.raw_type == "HumanInterventionRequested"));
}

#[test]
fn reset_clears_only_runs_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let flow_dir = settings.flows_dir.clone();
    fs::create_dir_all(&flow_dir).expect("flows");
    fs::write(flow_dir.join("kept.yaml"), simple_flow()).expect("flow");
    let project_path = temp.path().join("Project Reset");
    let service = AttractorApiService::new(settings.clone());
    service.start_pipeline(PipelineStartRequest {
        wait: Some(true),
        run_id: Some("run-reset".to_string()),
        flow_content: Some(simple_flow()),
        working_directory: project_path.to_string_lossy().to_string(),
        ..PipelineStartRequest::default()
    });
    assert!(RunStore::for_settings(&settings)
        .read_run_bundle("run-reset")
        .expect("read")
        .is_some());

    let reset = service.reset();

    assert_eq!(reset.body, json!({"status": "reset"}));
    assert!(RunStore::for_settings(&settings)
        .read_run_bundle("run-reset")
        .expect("read")
        .is_none());
    assert!(flow_dir.join("kept.yaml").is_file());
}

fn failed_record(run_id: &str, project_path: &Path) -> RunRecord {
    let mut record = RunRecord::new(run_id, project_path.to_string_lossy());
    record.flow_name = "control.yaml".to_string();
    record.status = "failed".to_string();
    record.last_error = "previous failure".to_string();
    record.started_at = "2026-06-23T10:00:00Z".to_string();
    record
}

fn running_record(run_id: &str, project_path: &Path) -> RunRecord {
    let mut record = RunRecord::new(run_id, project_path.to_string_lossy());
    record.flow_name = "control.yaml".to_string();
    record.status = "running".to_string();
    record.started_at = "2026-06-23T10:00:00Z".to_string();
    record
}

fn checkpoint(current_node: &str) -> CheckpointState {
    CheckpointState {
        timestamp: "2026-06-23T10:00:00Z".to_string(),
        current_node: current_node.to_string(),
        completed_nodes: vec!["start".to_string(), current_node.to_string()],
        context: [
            ("context.seed".to_string(), json!("control")),
            (
                "_attractor.node_outcomes".to_string(),
                json!({current_node: "fail"}),
            ),
        ]
        .into_iter()
        .collect(),
        retry_counts: Default::default(),
        logs: Vec::new(),
    }
}

fn simple_flow() -> String {
    r#"schema_version: "1"
id: api_controls
title: API Controls
nodes:
  start:
    kind: start
  task:
    kind: agent_task
    label: Task
    config:
      kind: agent_task
      prompt: Write a control note
  done:
    kind: exit
edges:
  - from: start
    to: task
  - from: task
    to: done
"#
    .to_string()
}

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
