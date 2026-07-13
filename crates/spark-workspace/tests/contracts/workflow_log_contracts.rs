use std::path::{Path, PathBuf};

use attractor_core::RunRecord;
use attractor_runtime::{CreateRunRequest, RunStore};
use serde_json::json;
use spark_common::settings::SparkSettings;
use spark_workspace::{
    project_run_milestones, read_workflow_log_tail, LiveCursor, LiveEnvelope, LiveResource,
};

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
        flows_dir: root.join("flows"),
        ui_dir: None,
        project_roots: Vec::<PathBuf>::new(),
    }
}

fn canonical_tempdir() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    (temp, root)
}

fn seed_run(settings: &SparkSettings, run_id: &str, configure: impl FnOnce(&mut RunRecord)) {
    let store = RunStore::for_settings(settings);
    let mut record = RunRecord::new(run_id, "/spark-contract-fixture/project");
    record.flow_name = "ops/run.dot".to_string();
    record.started_at = "2026-01-01T00:00:00Z".to_string();
    configure(&mut record);
    store
        .create_run(CreateRunRequest {
            record,
            ..CreateRunRequest::default()
        })
        .expect("create run");
}

fn set_run_status(settings: &SparkSettings, run_id: &str, status: &str) {
    let store = RunStore::for_settings(settings);
    store
        .update_run_record(run_id, |record| {
            record.status = status.to_string();
        })
        .expect("update record");
}

fn question_pending_envelope(run_id: &str, question_id: &str, node_id: &str) -> LiveEnvelope {
    LiveEnvelope {
        event_type: "run.question_pending".to_string(),
        project_path: None,
        resource: LiveResource {
            kind: "run".to_string(),
            id: Some(run_id.to_string()),
        },
        cursor: Some(LiveCursor {
            kind: "run_sequence".to_string(),
            value: 7,
        }),
        payload: json!({
            "question_id": question_id,
            "node_id": node_id,
            "source_scope": "root",
        }),
        reason: None,
    }
}

#[test]
fn lifecycle_milestones_project_idempotently() {
    let (_temp, root) = canonical_tempdir();
    let settings = settings(&root);
    seed_run(&settings, "run-lifecycle", |_| {});

    let started = project_run_milestones(&settings, "run-lifecycle", &[]).expect("project started");
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].kind, "run_started");
    assert_eq!(started[0].message, "Started ops/run.dot");
    assert_eq!(started[0].project_path, "/spark-contract-fixture/project");
    assert_eq!(started[0].run_id, "run-lifecycle");
    assert_eq!(started[0].seq, 0);

    let repeated = project_run_milestones(&settings, "run-lifecycle", &[]).expect("repeat");
    assert!(repeated.is_empty(), "same state must project to nothing");

    set_run_status(&settings, "run-lifecycle", "completed");
    let completed =
        project_run_milestones(&settings, "run-lifecycle", &[]).expect("project completed");
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].kind, "run_completed");
    assert_eq!(completed[0].seq, 1);

    let after_terminal = project_run_milestones(&settings, "run-lifecycle", &[]).expect("repeat");
    assert!(after_terminal.is_empty());

    let tail = read_workflow_log_tail(&settings, 100).expect("tail");
    assert_eq!(
        tail.iter()
            .map(|entry| entry.kind.as_str())
            .collect::<Vec<_>>(),
        ["run_started", "run_completed"],
    );
}

#[test]
fn failure_milestones_carry_a_one_line_reason() {
    let (_temp, root) = canonical_tempdir();
    let settings = settings(&root);
    seed_run(&settings, "run-failed", |_| {});
    project_run_milestones(&settings, "run-failed", &[]).expect("started");

    let store = RunStore::for_settings(&settings);
    store
        .update_run_record("run-failed", |record| {
            record.status = "failed".to_string();
            record.outcome_reason_message =
                Some("node deploy exhausted retries\nstack trace line".to_string());
        })
        .expect("update record");

    let failed = project_run_milestones(&settings, "run-failed", &[]).expect("failed milestone");
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].kind, "run_failed");
    assert_eq!(
        failed[0].message,
        "ops/run.dot failed: node deploy exhausted retries",
    );
}

#[test]
fn waiting_milestones_come_from_gate_envelopes_and_dedupe_by_question() {
    let (_temp, root) = canonical_tempdir();
    let settings = settings(&root);
    seed_run(&settings, "run-gated", |_| {});
    project_run_milestones(&settings, "run-gated", &[]).expect("started");

    let gate = question_pending_envelope("run-gated", "q-1", "approve-node");
    let waiting = project_run_milestones(&settings, "run-gated", &[gate.clone()]).expect("waiting");
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].kind, "run_waiting_on_input");
    assert_eq!(waiting[0].node_id.as_deref(), Some("approve-node"));
    assert_eq!(
        waiting[0].message,
        "ops/run.dot is waiting for input at approve-node",
    );

    let replay = project_run_milestones(&settings, "run-gated", &[gate]).expect("replay");
    assert!(replay.is_empty(), "same gate must not log twice");

    let mut child_gate = question_pending_envelope("run-gated", "q-child", "child-node");
    child_gate.payload["source_scope"] = json!("child");
    let child = project_run_milestones(&settings, "run-gated", &[child_gate]).expect("child gate");
    assert!(child.is_empty(), "child-scope gates stay out of the log");
}

#[test]
fn child_runs_are_excluded_and_continuations_are_labeled() {
    let (_temp, root) = canonical_tempdir();
    let settings = settings(&root);

    seed_run(&settings, "run-child", |record| {
        record.parent_run_id = Some("run-parent".to_string());
    });
    let child = project_run_milestones(&settings, "run-child", &[]).expect("child");
    assert!(child.is_empty(), "child runs never reach the workflow log");

    seed_run(&settings, "run-continued", |record| {
        record.continued_from_run_id = Some("run-original-1234".to_string());
    });
    let continued = project_run_milestones(&settings, "run-continued", &[]).expect("continued");
    assert_eq!(continued.len(), 1);
    assert_eq!(continued[0].kind, "run_continued");
    assert_eq!(continued[0].message, "Continued ops/run.dot from run-orig");
}

#[test]
fn tail_read_dedupes_crash_window_duplicates_and_applies_limit() {
    let (_temp, root) = canonical_tempdir();
    let settings = settings(&root);
    seed_run(&settings, "run-tail", |_| {});
    let entries = project_run_milestones(&settings, "run-tail", &[]).expect("started");
    assert_eq!(entries.len(), 1);

    // Simulate a crash between the log append and the state write: the same
    // entry line appears twice in the jsonl file.
    let log_path = settings.workspace_dir.join("event-log.jsonl");
    let existing = std::fs::read_to_string(&log_path).expect("log text");
    std::fs::write(&log_path, format!("{existing}{existing}")).expect("duplicate lines");

    let tail = read_workflow_log_tail(&settings, 100).expect("tail");
    assert_eq!(tail.len(), 1, "duplicates share an id and dedupe on read");

    set_run_status(&settings, "run-tail", "completed");
    project_run_milestones(&settings, "run-tail", &[]).expect("completed");
    let limited = read_workflow_log_tail(&settings, 1).expect("limited tail");
    assert_eq!(limited.len(), 1);
    assert_eq!(
        limited[0].kind, "run_completed",
        "tail keeps the newest entries"
    );
}
