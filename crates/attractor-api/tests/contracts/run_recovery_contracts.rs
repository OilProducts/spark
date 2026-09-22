use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use attractor_api::AttractorApiService;
use attractor_core::CheckpointState;
use attractor_runtime::{
    human_gate_answered_event, prepare_fresh_run, NodeArtifacts, RunStore, RuntimeHandlerRunner,
};
use serde_json::json;
use spark_common::settings::SparkSettings;

fn settings(root: &Path) -> SparkSettings {
    SparkSettings {
        startup_sources: Default::default(),
        connections: Default::default(),
        providers: Default::default(),
        agents: Default::default(),
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

const PAUSED_RECOVERY_FLOW: &str = r#"schema_version: "1"
id: paused_recovery
title: Paused Recovery
nodes:
  start: { kind: start }
  review:
    kind: human_gate
    runtime: { recovery_policy: pause }
    config: { kind: human_gate, prompt: Retry me? }
    contracts:
      writes_context: [context.accepted]
  done: { kind: exit }
edges:
- { from: start, to: review }
- { from: review, to: done, label: Finish }
"#;

const TREE_ROOT_FLOW: &str = r#"schema_version: "1"
id: tree_root
title: Tree Root
nodes:
  start: { kind: start }
  branch:
    kind: subflow
    runtime: { recovery_policy: pause }
    config: { kind: subflow, flow_ref: child.yaml }
    manager: { poll_interval: 0s, max_cycles: 1 }
  done: { kind: exit }
edges:
- { from: start, to: branch }
- { from: branch, to: done }
"#;

const TREE_CHILD_FLOW: &str = r#"schema_version: "1"
id: tree_child
title: Tree Child
nodes:
  start:
    kind: start
    runtime: { recovery_policy: pause }
  done: { kind: exit }
edges:
- { from: start, to: done }
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
    record.execution_profile_id = Some("native".to_string());
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
fn startup_recovery_resumes_a_linked_orphaned_child_in_place() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.config_dir).expect("config dir");
    let workdir = temp.path().join("project");
    std::fs::create_dir_all(&workdir).expect("workdir");
    let store = RunStore::for_settings(&settings);
    let root_flow = attractor_dsl::parse_flow_definition(TREE_ROOT_FLOW).expect("root flow");
    let child_flow = attractor_dsl::parse_flow_definition(TREE_CHILD_FLOW).expect("child flow");

    let mut root = attractor_core::RunRecord::new("tree-root", workdir.to_string_lossy());
    root.flow_name = "tree-root".to_string();
    root.root_run_id = Some("tree-root".to_string());
    root.execution_profile_id = Some("native".to_string());
    root.execution_profile_capabilities = Some(json!({"network": false}));
    root.execution_lock = Some(attractor_core::RunExecutionLock {
        scope: "project".to_string(),
        key: "tree".to_string(),
        conflict_policy: "queue".to_string(),
        identity: "tree-lock".to_string(),
        state: "acquired".to_string(),
        queue_position: None,
    });
    let root_context = attractor_core::ContextMap::from([
        ("internal.run_id".to_string(), json!("tree-root")),
        ("internal.root_run_id".to_string(), json!("tree-root")),
        (
            "internal.run_workdir".to_string(),
            json!(workdir.to_string_lossy().to_string()),
        ),
        (
            "context.stack.child.run_id".to_string(),
            json!("tree-child"),
        ),
        ("context.stack.child.status".to_string(), json!("running")),
    ]);
    let root_paths = prepare_fresh_run(
        &store,
        &root,
        &root_flow,
        Some(TREE_ROOT_FLOW.to_string()),
        None,
        &attractor_core::LaunchContext::empty(),
        &root_context,
    )
    .expect("prepare root");
    store
        .save_checkpoint(
            &root_paths,
            &CheckpointState {
                timestamp: "2026-08-14T00:00:00Z".to_string(),
                current_node: "branch".to_string(),
                completed_nodes: vec!["start".to_string()],
                context: root_context,
                retry_counts: Default::default(),
                logs: Vec::new(),
            },
            Default::default(),
        )
        .expect("checkpoint root");

    let mut child = attractor_core::RunRecord::new("tree-child", workdir.to_string_lossy());
    child.flow_name = "tree-child".to_string();
    child.parent_run_id = Some("tree-root".to_string());
    child.parent_node_id = Some("branch".to_string());
    child.root_run_id = Some("tree-root".to_string());
    child.child_invocation_index = Some(1);
    child.execution_profile_id = root.execution_profile_id.clone();
    child.execution_profile_capabilities = root.execution_profile_capabilities.clone();
    child.execution_lock = root.execution_lock.clone().map(|mut lock| {
        lock.state = "inherited".to_string();
        lock
    });
    let child_context = attractor_core::ContextMap::from([
        ("internal.run_id".to_string(), json!("tree-child")),
        ("internal.parent_run_id".to_string(), json!("tree-root")),
        ("internal.parent_node_id".to_string(), json!("branch")),
        ("internal.root_run_id".to_string(), json!("tree-root")),
        (
            "internal.run_workdir".to_string(),
            json!(workdir.to_string_lossy().to_string()),
        ),
    ]);
    let child_paths = prepare_fresh_run(
        &store,
        &child,
        &child_flow,
        Some(TREE_CHILD_FLOW.to_string()),
        None,
        &attractor_core::LaunchContext::empty(),
        &child_context,
    )
    .expect("prepare child");
    store
        .save_checkpoint(
            &child_paths,
            &CheckpointState {
                timestamp: "2026-08-14T00:00:01Z".to_string(),
                current_node: "start".to_string(),
                completed_nodes: Vec::new(),
                context: child_context,
                retry_counts: Default::default(),
                logs: Vec::new(),
            },
            Default::default(),
        )
        .expect("checkpoint child");

    store
        .write_node_artifacts(
            &child_paths,
            "start",
            0,
            0,
            &NodeArtifacts {
                response: Some("accepted before the crash\n".to_string()),
                status: Some(json!({
                    "outcome": "success",
                    "preferred_label": "",
                    "suggested_next_ids": [],
                    "context_updates": {},
                    "notes": ""
                })),
                under_logs: true,
                ..NodeArtifacts::default()
            },
        )
        .expect("durable child response");

    for run_id in ["tree-root", "tree-child"] {
        store
            .update_run_record(run_id, |record| {
                record.status = "failed".to_string();
                record.outcome = None;
                record.ended_at = None;
                record.last_error = "interrupted by restart".to_string();
            })
            .expect("mark restart interruption");
    }

    let recovery = blocking_gate_service(&settings).recover_interrupted_runs();
    assert_eq!(recovery["resumed"], json!(["tree-root"]), "{recovery:?}");
    let root_record = store
        .read_run_bundle("tree-root")
        .expect("root bundle")
        .and_then(|bundle| bundle.record)
        .expect("root record");
    assert_eq!(root_record.status, "waiting");
    assert_eq!(
        root_record.outcome_reason_code.as_deref(),
        Some("recovery_decision_required")
    );

    let retry = blocking_gate_service(&settings).retry_pipeline_route("tree-root");
    assert_eq!(retry.status_code, 200, "{:?}", retry.body);
    assert_eq!(retry.body["run_id"], json!("tree-root"));
    wait_for_status(&store, "tree-root", "completed");
    wait_for_status(&store, "tree-child", "completed");
    let children = store.list_child_run_bundles("tree-root").expect("children");
    assert_eq!(children.len(), 1, "recovery must not launch a sibling");
    assert_eq!(children[0].paths.run_id, "tree-child");
    let child = children[0].record.as_ref().expect("child record");
    assert_eq!(child.parent_run_id.as_deref(), Some("tree-root"));
    assert_eq!(child.parent_node_id.as_deref(), Some("branch"));
    assert_eq!(child.root_run_id.as_deref(), Some("tree-root"));
    assert_eq!(child.child_invocation_index, Some(1));
    assert_eq!(child.execution_profile_id.as_deref(), Some("native"));
    assert_eq!(
        child.execution_profile_capabilities,
        Some(json!({"network": false}))
    );
    let lock = child.execution_lock.as_ref().expect("child lock metadata");
    assert_eq!(
        lock.identity,
        root.execution_lock.as_ref().unwrap().identity
    );
    assert_eq!(lock.state, "released");
    assert_eq!(lock.queue_position, None);
    let child_checkpoint = children[0].checkpoint.as_ref().expect("child checkpoint");
    assert!(child_checkpoint
        .completed_nodes
        .contains(&"start".to_string()));
}

#[test]
fn recovery_pause_with_checkpoint_lag_consumes_the_durable_response() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.config_dir).expect("config dir");
    let workdir = temp.path().join("project");
    std::fs::create_dir_all(&workdir).expect("workdir");
    let store = RunStore::for_settings(&settings);
    let flow = attractor_dsl::parse_flow_definition(PAUSED_RECOVERY_FLOW).expect("flow");
    let mut record = attractor_core::RunRecord::new("pause-answered", workdir.to_string_lossy());
    record.execution_profile_id = Some("native".to_string());
    let paths = prepare_fresh_run(
        &store,
        &record,
        &flow,
        Some(PAUSED_RECOVERY_FLOW.to_string()),
        None,
        &attractor_core::LaunchContext::empty(),
        &attractor_core::ContextMap::default(),
    )
    .expect("prepare");
    store
        .save_checkpoint(
            &paths,
            &CheckpointState {
                timestamp: "2026-08-14T00:00:00Z".to_string(),
                current_node: "review".to_string(),
                completed_nodes: vec!["start".to_string()],
                context: Default::default(),
                retry_counts: Default::default(),
                logs: Vec::new(),
            },
            Default::default(),
        )
        .expect("checkpoint");
    store
        .write_node_artifacts(
            &paths,
            "review",
            1,
            0,
            &NodeArtifacts {
                response: Some("accepted before checkpoint\n".to_string()),
                status: Some(json!({
                    "outcome": "success",
                    "preferred_label": "Finish",
                    "suggested_next_ids": [],
                    "context_updates": {},
                    "notes": ""
                })),
                under_logs: true,
                ..NodeArtifacts::default()
            },
        )
        .expect("durable response");

    let recovery = blocking_gate_service(&settings).recover_interrupted_runs();
    assert_eq!(recovery["resumed"], json!(["pause-answered"]));
    wait_for_status(&store, "pause-answered", "completed");
    let record = store
        .read_run_bundle("pause-answered")
        .expect("bundle")
        .and_then(|bundle| bundle.record)
        .expect("record");
    assert_ne!(
        record.outcome_reason_code.as_deref(),
        Some("recovery_decision_required")
    );
}

#[test]
fn recovery_rejects_incomplete_malformed_and_contract_invalid_durable_responses() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    let workdir = temp.path().join("project");
    std::fs::create_dir_all(&workdir).expect("workdir");
    let store = RunStore::for_settings(&settings);
    let flow = attractor_dsl::parse_flow_definition(PAUSED_RECOVERY_FLOW).expect("flow");
    let node = flow.nodes.get("review").expect("review node");
    let record = attractor_core::RunRecord::new("invalid-durable", workdir.to_string_lossy());
    let paths = prepare_fresh_run(
        &store,
        &record,
        &flow,
        Some(PAUSED_RECOVERY_FLOW.to_string()),
        None,
        &attractor_core::LaunchContext::empty(),
        &attractor_core::ContextMap::default(),
    )
    .expect("prepare");

    let valid_status = json!({
        "outcome": "success",
        "preferred_label": "Finish",
        "suggested_next_ids": [],
        "context_updates": {},
        "notes": ""
    });
    store
        .write_node_artifacts(
            &paths,
            "review",
            1,
            0,
            &NodeArtifacts {
                status: Some(valid_status.clone()),
                under_logs: true,
                ..NodeArtifacts::default()
            },
        )
        .expect("status without response");
    assert!(attractor_runtime::durable_outcome(&store, &paths, "review", node, 1, 0).is_none());

    let mut contract_invalid_status = valid_status;
    contract_invalid_status["context_updates"] = json!({"context.forbidden": true});
    for (attempt, status) in [
        (1, json!({"outcome": "success"})),
        (2, contract_invalid_status),
    ] {
        store
            .write_node_artifacts(
                &paths,
                "review",
                1,
                attempt,
                &NodeArtifacts {
                    response: Some("unaccepted\n".to_string()),
                    status: Some(status),
                    under_logs: true,
                    ..NodeArtifacts::default()
                },
            )
            .expect("invalid durable artifacts");
        assert!(
            attractor_runtime::durable_outcome(&store, &paths, "review", node, 1, attempt)
                .is_none()
        );
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
fn recovery_pause_survives_two_startups_without_retry_authorization() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.config_dir).expect("config dir");
    let workdir = temp.path().join("project");
    std::fs::create_dir_all(&workdir).expect("workdir");
    let store = RunStore::for_settings(&settings);
    let flow = attractor_dsl::parse_flow_definition(PAUSED_RECOVERY_FLOW).expect("flow");
    let mut record = attractor_core::RunRecord::new("paused-twice", workdir.to_string_lossy());
    record.execution_profile_id = Some("native".to_string());
    let paths = prepare_fresh_run(
        &store,
        &record,
        &flow,
        Some(PAUSED_RECOVERY_FLOW.to_string()),
        None,
        &attractor_core::LaunchContext::empty(),
        &attractor_core::ContextMap::default(),
    )
    .expect("prepare");
    store
        .save_checkpoint(
            &paths,
            &CheckpointState {
                timestamp: "2026-08-14T00:00:00Z".to_string(),
                current_node: "review".to_string(),
                completed_nodes: vec!["start".to_string()],
                context: Default::default(),
                retry_counts: Default::default(),
                logs: Vec::new(),
            },
            Default::default(),
        )
        .expect("checkpoint");

    for _ in 0..2 {
        let result = blocking_gate_service(&settings).recover_interrupted_runs();
        assert_eq!(result["resumed"], json!(["paused-twice"]), "{result:?}");
        let record = store
            .read_run_bundle("paused-twice")
            .expect("bundle")
            .and_then(|bundle| bundle.record)
            .expect("record");
        assert_eq!(record.status, "waiting");
        assert_eq!(
            record.outcome_reason_code.as_deref(),
            Some("recovery_decision_required")
        );
    }
}

#[test]
fn parent_node_without_parent_run_is_stably_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.config_dir).expect("config dir");
    let workdir = temp.path().join("project");
    std::fs::create_dir_all(&workdir).expect("workdir");
    let store = manufacture_orphaned_waiting_run(&settings, &workdir, "broken-lineage");
    store
        .update_run_record("broken-lineage", |record| {
            record.parent_node_id = Some("review".to_string());
        })
        .expect("corrupt lineage");

    for _ in 0..2 {
        let result = blocking_gate_service(&settings).recover_interrupted_runs();
        assert_eq!(result["resumed"], json!([]), "{result:?}");
        let record = store
            .read_run_bundle("broken-lineage")
            .expect("bundle")
            .and_then(|bundle| bundle.record)
            .expect("record");
        assert_eq!(record.status, "failed");
        assert_eq!(
            record.outcome_reason_code.as_deref(),
            Some("recovery_missing_lineage")
        );
    }
}

#[test]
fn child_without_invocation_index_is_reported_by_run_id() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.config_dir).expect("config dir");
    let workdir = temp.path().join("project");
    std::fs::create_dir_all(&workdir).expect("workdir");
    manufacture_orphaned_waiting_run(&settings, &workdir, "indexed-parent");
    let store = manufacture_orphaned_waiting_run(&settings, &workdir, "unindexed-child");
    store
        .update_run_record("unindexed-child", |record| {
            record.parent_run_id = Some("indexed-parent".to_string());
            record.parent_node_id = Some("review".to_string());
            record.root_run_id = Some("indexed-parent".to_string());
            record.child_invocation_index = None;
        })
        .expect("drop invocation index");

    let result = blocking_gate_service(&settings).recover_interrupted_runs();
    assert!(
        result["failed"]
            .as_array()
            .expect("failed list")
            .contains(&json!({"run_id": "unindexed-child", "code": "recovery_missing_lineage"})),
        "{result:?}"
    );
    let record = store
        .read_run_bundle("unindexed-child")
        .expect("bundle")
        .and_then(|bundle| bundle.record)
        .expect("record");
    assert_eq!(record.status, "failed");
    assert_eq!(
        record.child_invocation_index, None,
        "index is never invented"
    );
    assert!(
        record
            .last_error
            .contains("child invocation index is missing"),
        "{}",
        record.last_error
    );
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

#[test]
fn ambiguous_child_invocations_leave_parent_terminal_and_never_resume_it() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings = settings(temp.path());
    std::fs::create_dir_all(&settings.config_dir).expect("config dir");
    let workdir = temp.path().join("project");
    std::fs::create_dir_all(&workdir).expect("workdir");
    let store = RunStore::for_settings(&settings);
    let flow = attractor_dsl::parse_flow_definition(GATE_FLOW).expect("flow parses");

    for (run_id, parent) in [
        ("ambiguous-parent", None),
        ("ambiguous-child-a", Some("ambiguous-parent")),
        ("ambiguous-child-b", Some("ambiguous-parent")),
    ] {
        let mut record = attractor_core::RunRecord::new(run_id, workdir.to_string_lossy());
        record.flow_name = "recovery-gate".to_string();
        record.execution_profile_id = Some("native".to_string());
        record.root_run_id = Some("ambiguous-parent".to_string());
        if let Some(parent) = parent {
            record.parent_run_id = Some(parent.to_string());
            record.parent_node_id = Some("review".to_string());
            record.child_invocation_index = Some(1);
        }
        prepare_fresh_run(
            &store,
            &record,
            &flow,
            Some(GATE_FLOW.to_string()),
            None,
            &attractor_core::LaunchContext::empty(),
            &attractor_core::ContextMap::default(),
        )
        .expect("prepare run");
    }

    let service = blocking_gate_service(&settings);
    let first = service.recover_interrupted_runs();
    assert_eq!(first["resumed"], json!([]), "{first:?}");
    let record = store
        .read_run_bundle("ambiguous-parent")
        .expect("bundle")
        .and_then(|bundle| bundle.record)
        .expect("record");
    assert_eq!(record.status, "failed");
    assert_eq!(
        record.outcome_reason_code.as_deref(),
        Some("recovery_ambiguous_child_invocation")
    );
    let second = service.recover_interrupted_runs();
    assert_eq!(second["resumed"], json!([]), "{second:?}");
    let record = store
        .read_run_bundle("ambiguous-parent")
        .expect("bundle")
        .and_then(|bundle| bundle.record)
        .expect("record");
    assert_eq!(record.status, "failed");
    assert_eq!(
        record.outcome_reason_code.as_deref(),
        Some("recovery_ambiguous_child_invocation"),
        "terminal failure code must be stable"
    );
    for child_id in ["ambiguous-child-a", "ambiguous-child-b"] {
        let child = store
            .read_run_bundle(child_id)
            .expect("child bundle")
            .and_then(|bundle| bundle.record)
            .expect("child record");
        assert_eq!(child.status, "failed", "ambiguous child must be terminal");
        assert_eq!(
            child.outcome_reason_code.as_deref(),
            Some("recovery_ambiguous_child_invocation"),
            "child failure code must be stable"
        );
    }
}

static RETRY_TREE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn seed_failed_retry_tree(settings: &SparkSettings) -> RunStore {
    let store = RunStore::for_settings(settings);
    std::fs::create_dir_all(&settings.project_root).unwrap();
    for (id, parent, child, source, node) in [
        (
            "retry-root",
            None,
            Some("retry-child"),
            TREE_ROOT_FLOW,
            "branch",
        ),
        (
            "retry-child",
            Some("retry-root"),
            Some("retry-leaf"),
            TREE_ROOT_FLOW,
            "branch",
        ),
        (
            "retry-leaf",
            Some("retry-child"),
            None,
            TREE_CHILD_FLOW,
            "start",
        ),
    ] {
        let mut record =
            attractor_core::RunRecord::new(id, settings.project_root.to_string_lossy());
        record.status = "failed".into();
        record.parent_run_id = parent.map(str::to_string);
        record.parent_node_id = parent.map(|_| "branch".into());
        record.root_run_id = Some("retry-root".into());
        record.child_invocation_index = parent.map(|_| 1);
        record.execution_profile_id = Some("native".into());
        let mut context = attractor_core::ContextMap::from([
            ("internal.run_id".into(), json!(id)),
            ("internal.root_run_id".into(), json!("retry-root")),
            ("_attractor.node_outcomes".into(), json!({node: "fail"})),
        ]);
        if let Some(child) = child {
            context.insert("context.stack.child.run_id".into(), json!(child));
            context.insert("context.stack.child.status".into(), json!("failed"));
        }
        let completed = if node == "branch" {
            vec!["start".into(), node.into()]
        } else {
            vec![node.into()]
        };
        let paths = store
            .create_run(attractor_runtime::CreateRunRequest {
                record,
                checkpoint: Some(CheckpointState {
                    timestamp: "2026-09-01T00:00:00Z".into(),
                    current_node: node.into(),
                    completed_nodes: completed,
                    context,
                    retry_counts: Default::default(),
                    logs: vec!["prior log".into()],
                }),
                manifest: None,
                flow_source: Some(source.into()),
                flow_definition_json: None,
            })
            .unwrap();
        store.write_node_artifacts(&paths, node, u64::from(node == "branch"), 0, &NodeArtifacts {
            response: Some("prior failed attempt".into()),
            status: Some(json!({"outcome":"fail", "preferred_label":"", "suggested_next_ids":[], "context_updates":{}, "notes":"", "retryable":false})),
            under_logs: true, ..Default::default()
        }).unwrap();
        if let Some(child) = child {
            let mut event = attractor_runtime::child_run_completed_event(
                id,
                child,
                node,
                "retry-root",
                "child",
                "failed",
                Some("fail".into()),
                None,
                None,
                Some("prior failure".into()),
            );
            event.emitted_at = "2026-08-31T00:00:00Z".into();
            store.append_event(&paths, event).unwrap();
        }
    }
    store
}

#[test]
fn explicit_parent_retry_recovers_nested_failed_children_without_siblings() {
    let _serial = RETRY_TREE_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let store = seed_failed_retry_tree(&settings);
    let response = blocking_gate_service(&settings).retry_pipeline_route("retry-root");
    assert_eq!(response.status_code, 200, "{:?}", response.body);
    wait_for_status(&store, "retry-root", "completed");
    for (id, node, stage) in [
        ("retry-root", "branch", 1),
        ("retry-child", "branch", 1),
        ("retry-leaf", "start", 0),
    ] {
        let bundle = store.read_run_bundle(id).unwrap().unwrap();
        assert_eq!(bundle.record.unwrap().status, "completed");
        assert!(store
            .node_execution_root(&bundle.paths, node, stage, 0)
            .unwrap()
            .join("response.md")
            .is_file());
        assert!(store
            .node_execution_root(&bundle.paths, node, stage, 1)
            .unwrap()
            .join("response.md")
            .is_file());
    }
    assert_eq!(store.list_run_records().unwrap().len(), 3);
}

#[test]
fn restart_finishes_partial_tree_preparation_once_and_consumes_successful_children() {
    let _serial = RETRY_TREE_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let store = seed_failed_retry_tree(&settings);
    let child_before = store.read_run_bundle("retry-child").unwrap().unwrap();
    let leaf_before = store.read_run_bundle("retry-leaf").unwrap().unwrap();
    let controls = attractor_runtime::RuntimeControls::new(store.clone());
    controls.prepare_retry("retry-root").unwrap();
    // Reconstruct a crash immediately after the parent's write-ahead checkpoint.
    for bundle in [child_before, leaf_before] {
        store
            .write_run_record(&bundle.paths, &bundle.record.unwrap())
            .unwrap();
        store
            .save_checkpoint(
                &bundle.paths,
                &bundle.checkpoint.unwrap(),
                Default::default(),
            )
            .unwrap();
    }
    let root = store.read_run_bundle("retry-root").unwrap().unwrap();
    let mut checkpoint = root.checkpoint.unwrap();
    checkpoint
        .context
        .get_mut(attractor_runtime::retry::RETRY_REQUEST_KEY)
        .unwrap()["preparing"] = json!(true);
    store
        .save_checkpoint(&root.paths, &checkpoint, Default::default())
        .unwrap();
    store
        .update_run_record("retry-root", |r| r.status = "failed".into())
        .unwrap();
    let recovery = blocking_gate_service(&settings).recover_interrupted_runs();
    assert_eq!(recovery["resumed"], json!(["retry-root"]), "{recovery}");
    wait_for_status(&store, "retry-root", "completed");
    for id in ["retry-root", "retry-child", "retry-leaf"] {
        let checkpoint = controls.get_checkpoint(id).unwrap();
        let request = attractor_runtime::retry::saved_retry_request(&checkpoint.context)
            .unwrap()
            .unwrap();
        controls.apply_retry_request(&request).unwrap();
        assert_eq!(
            checkpoint.context[attractor_runtime::retry::EXECUTION_BASES_KEY]
                [&request.target.node_id],
            json!(1)
        );
    }
    // Another explicit parent retry must consume its now-successful child.
    store
        .update_run_record("retry-root", |r| r.status = "failed".into())
        .unwrap();
    let mut checkpoint = controls.get_checkpoint("retry-root").unwrap();
    checkpoint.current_node = "branch".into();
    checkpoint.completed_nodes = vec!["start".into(), "branch".into()];
    checkpoint
        .context
        .insert("_attractor.node_outcomes".into(), json!({"branch":"fail"}));
    store
        .save_checkpoint(&root.paths, &checkpoint, Default::default())
        .unwrap();
    let child_checkpoint = controls.get_checkpoint("retry-child").unwrap();
    let response = blocking_gate_service(&settings).retry_pipeline_route("retry-root");
    assert_eq!(response.status_code, 200, "{:?}", response.body);
    wait_for_status(&store, "retry-root", "completed");
    assert_eq!(
        controls.get_checkpoint("retry-child").unwrap(),
        child_checkpoint
    );
    assert_eq!(store.list_run_records().unwrap().len(), 3);
}

#[test]
fn explicit_retry_rejects_invalid_lineage_and_cancellation_before_preparation() {
    let _serial = RETRY_TREE_TEST.lock().unwrap_or_else(|e| e.into_inner());
    for case in ["parent", "node", "root", "canceled", "checkpoint", "flow"] {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let store = seed_failed_retry_tree(&settings);
        let before = attractor_runtime::RuntimeControls::new(store.clone())
            .get_checkpoint("retry-root")
            .unwrap();
        store
            .update_run_record("retry-leaf", |r| match case {
                "parent" => r.parent_run_id = Some("wrong".into()),
                "node" => r.parent_node_id = Some("wrong".into()),
                "root" => r.root_run_id = Some("wrong".into()),
                "canceled" => r.status = "canceled".into(),
                _ => (),
            })
            .unwrap();
        if case == "checkpoint" {
            let bundle = store.read_run_bundle("retry-leaf").unwrap().unwrap();
            let mut checkpoint = bundle.checkpoint.unwrap();
            checkpoint.current_node = "missing".into();
            store
                .save_checkpoint(&bundle.paths, &checkpoint, Default::default())
                .unwrap();
        }
        if case == "flow" {
            let bundle = store.read_run_bundle("retry-leaf").unwrap().unwrap();
            std::fs::write(
                bundle.paths.root.join("artifacts/flow/flow-source.yaml"),
                "invalid: [",
            )
            .unwrap();
        }
        let response = blocking_gate_service(&settings).retry_pipeline_route("retry-root");
        assert_eq!(response.status_code, 409, "{case}: {:?}", response.body);
        assert_eq!(
            attractor_runtime::RuntimeControls::new(store.clone())
                .get_checkpoint("retry-root")
                .unwrap(),
            before
        );
    }
}

fn replace_retry_leaf(store: &RunStore, source: &str, node: &str) {
    let bundle = store.read_run_bundle("retry-leaf").unwrap().unwrap();
    std::fs::write(
        bundle.paths.root.join("artifacts/flow/flow-source.yaml"),
        source,
    )
    .unwrap();
    let mut checkpoint = bundle.checkpoint.unwrap();
    checkpoint.current_node = node.into();
    checkpoint.completed_nodes = vec!["start".into(), node.into()];
    checkpoint
        .context
        .insert("_attractor.node_outcomes".into(), json!({node:"fail"}));
    store
        .save_checkpoint(&bundle.paths, &checkpoint, Default::default())
        .unwrap();
}

#[test]
fn retry_preserves_human_wait_and_rejects_execution_overlap_across_ancestors() {
    let _serial = RETRY_TREE_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let store = seed_failed_retry_tree(&settings);
    replace_retry_leaf(&store, GATE_FLOW, "review");
    let service = blocking_gate_service(&settings);
    assert_eq!(service.retry_pipeline_route("retry-root").status_code, 200);
    wait_for_status(&store, "retry-leaf", "waiting");
    let paths = store.read_run_bundle("retry-leaf").unwrap().unwrap().paths;
    assert!(!store
        .read_raw_events(&paths)
        .unwrap()
        .iter()
        .any(|e| e.event_type == "InterviewCompleted"));
    for id in ["retry-root", "retry-child", "retry-leaf"] {
        let response = service.retry_pipeline_route(id);
        assert_eq!(response.status_code, 409, "{id}: {:?}", response.body);
        assert!(response.body["detail"]
            .as_str()
            .unwrap()
            .contains("active executor"));
    }
    store
        .append_event(
            &paths,
            human_gate_answered_event(
                "retry-leaf",
                "review-1",
                Some("review".into()),
                None,
                None,
                "Finish",
                None,
            ),
        )
        .unwrap();
    wait_for_status(&store, "retry-root", "completed");
    assert_eq!(store.list_run_records().unwrap().len(), 3);
}

#[test]
fn new_child_failure_propagates_once_and_direct_child_retry_gets_a_new_request() {
    let _serial = RETRY_TREE_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let store = seed_failed_retry_tree(&settings);
    let source = GATE_FLOW
        .replace("kind: human_gate", "kind: tool")
        .replace("prompt: Ship the report?", "command: exit 1");
    replace_retry_leaf(&store, &source, "review");
    let service = blocking_gate_service(&settings);
    assert_eq!(service.retry_pipeline_route("retry-root").status_code, 200);
    wait_for_status(&store, "retry-root", "failed");
    let controls = attractor_runtime::RuntimeControls::new(store.clone());
    let leaf = controls.get_checkpoint("retry-leaf").unwrap();
    let request = attractor_runtime::retry::saved_retry_request(&leaf.context)
        .unwrap()
        .unwrap();
    controls.apply_retry_request(&request).unwrap();
    assert_eq!(controls.get_checkpoint("retry-leaf").unwrap(), leaf);
    assert_eq!(
        leaf.context[attractor_runtime::retry::EXECUTION_BASES_KEY]["review"],
        json!(1)
    );
    // Direct child preparation is synchronous here, after the parent has failed.
    controls.prepare_retry("retry-leaf").unwrap();
    let fresh = controls.get_checkpoint("retry-leaf").unwrap();
    assert_ne!(
        attractor_runtime::retry::saved_retry_request(&fresh.context)
            .unwrap()
            .unwrap()
            .id,
        request.id
    );
    assert_eq!(
        fresh.context[attractor_runtime::retry::EXECUTION_BASES_KEY]["review"],
        json!(2)
    );
    assert_eq!(store.list_run_records().unwrap().len(), 3);
}

#[test]
fn parent_retry_keeps_custom_child_execution_with_its_launcher() {
    let _serial = RETRY_TREE_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let store = seed_failed_retry_tree(&settings);
    let service = AttractorApiService::new_with_runtime_handler_runner_factory(
        settings,
        Arc::new(|| {
            RuntimeHandlerRunner::new()
                .with_child_run_launcher(|_| panic!("must not replace a custom child"))
        }),
    );
    let before = attractor_runtime::RuntimeControls::new(store.clone())
        .get_checkpoint("retry-root")
        .unwrap();
    let response = service.retry_pipeline_route("retry-root");
    assert_eq!(response.status_code, 409);
    assert!(response.body["detail"]
        .as_str()
        .unwrap()
        .contains("retry_custom_child_launcher"));
    assert_eq!(
        attractor_runtime::RuntimeControls::new(store)
            .get_checkpoint("retry-root")
            .unwrap(),
        before
    );
}

#[test]
fn accepted_child_retry_owns_the_tree_before_executor_spawn() {
    let _serial = RETRY_TREE_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let store = seed_failed_retry_tree(&settings);
    let constructing = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::new(std::sync::Barrier::new(2));
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let service = Arc::new(
        AttractorApiService::new_with_runtime_handler_runner_factory(settings, {
            let constructing = constructing.clone();
            let release = release.clone();
            Arc::new(move || {
                if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
                    // Acceptance finished; executor construction has not completed.
                    constructing.wait();
                    release.wait();
                }
                RuntimeHandlerRunner::new()
            })
        }),
    );
    let child = {
        let service = service.clone();
        std::thread::spawn(move || service.retry_pipeline_route("retry-leaf"))
    };
    constructing.wait();
    let parent = service.retry_pipeline_route("retry-root");
    release.wait();
    assert_eq!(child.join().unwrap().status_code, 200);
    assert_eq!(parent.status_code, 409, "{:?}", parent.body);
    wait_for_status(&store, "retry-leaf", "completed");
}

#[test]
fn direct_child_retry_recovers_all_crash_windows_with_failed_parent() {
    let _serial = RETRY_TREE_TEST.lock().unwrap_or_else(|e| e.into_inner());
    for window in ["preparing", "prepared", "response"] {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let store = seed_failed_retry_tree(&settings);
        replace_retry_leaf(&store, GATE_FLOW, "review");
        let controls = attractor_runtime::RuntimeControls::new(store.clone());
        controls.prepare_retry("retry-leaf").unwrap();
        let bundle = store.read_run_bundle("retry-leaf").unwrap().unwrap();
        let mut checkpoint = bundle.checkpoint.unwrap();
        let request = attractor_runtime::retry::saved_retry_request(&checkpoint.context)
            .unwrap()
            .unwrap();
        if window == "preparing" {
            checkpoint
                .context
                .get_mut(attractor_runtime::retry::RETRY_REQUEST_KEY)
                .unwrap()["preparing"] = json!(true);
            store
                .save_checkpoint(&bundle.paths, &checkpoint, Default::default())
                .unwrap();
            store
                .update_run_record("retry-leaf", |r| r.status = "failed".into())
                .unwrap();
        }
        if window == "response" {
            store.write_node_artifacts(&bundle.paths, "review", 1, 1, &NodeArtifacts {
                response: Some("durable new answer".into()),
                status: Some(json!({"outcome":"success", "preferred_label":"Finish", "suggested_next_ids":[], "context_updates":{}, "notes":""})),
                under_logs: true, ..Default::default()
            }).unwrap();
        }
        let service = blocking_gate_service(&settings);
        let recovery = service.recover_interrupted_runs();
        assert_eq!(
            recovery["resumed"],
            json!(["retry-leaf"]),
            "{window}: {recovery}"
        );
        if window != "response" {
            wait_for_status(&store, "retry-leaf", "waiting");
            assert_eq!(service.recover_interrupted_runs()["resumed"], json!([]));
            for id in ["retry-root", "retry-child", "retry-leaf"] {
                assert_eq!(service.retry_pipeline_route(id).status_code, 409);
            }
            store
                .append_event(
                    &bundle.paths,
                    human_gate_answered_event(
                        "retry-leaf",
                        "review-1",
                        Some("review".into()),
                        None,
                        None,
                        "Finish",
                        None,
                    ),
                )
                .unwrap();
        }
        wait_for_status(&store, "retry-leaf", "completed");
        let saved = controls.get_checkpoint("retry-leaf").unwrap();
        assert_eq!(
            saved.context[attractor_runtime::retry::EXECUTION_BASES_KEY]["review"],
            json!(1)
        );
        assert_eq!(
            attractor_runtime::retry::saved_retry_request(&saved.context)
                .unwrap()
                .unwrap()
                .id,
            request.id
        );
        assert_eq!(store.list_run_records().unwrap().len(), 3);
        for id in ["retry-root", "retry-child"] {
            assert_eq!(
                store
                    .read_run_bundle(id)
                    .unwrap()
                    .unwrap()
                    .record
                    .unwrap()
                    .status,
                "failed"
            );
        }
        let interviews = store
            .read_raw_events(&bundle.paths)
            .unwrap()
            .into_iter()
            .filter(|e| e.event_type == "InterviewStarted")
            .count();
        assert_eq!(interviews, usize::from(window != "response"));
    }
}

#[test]
fn direct_child_retry_recovery_preserves_cancellation_validation_and_custom_launcher() {
    let _serial = RETRY_TREE_TEST.lock().unwrap_or_else(|e| e.into_inner());
    for case in [
        "canceled",
        "cancel_requested",
        "ancestor_canceled",
        "source",
        "lineage",
        "custom",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let store = seed_failed_retry_tree(&settings);
        let controls = attractor_runtime::RuntimeControls::new(store.clone());
        controls.prepare_retry("retry-leaf").unwrap();
        let before = controls.get_checkpoint("retry-leaf").unwrap();
        match case {
            "canceled" | "cancel_requested" => {
                store
                    .update_run_record("retry-leaf", |r| r.status = case.into())
                    .unwrap();
            }
            "ancestor_canceled" => {
                store
                    .update_run_record("retry-root", |r| r.status = "canceled".into())
                    .unwrap();
            }
            "source" => {
                let bundle = store.read_run_bundle("retry-leaf").unwrap().unwrap();
                std::fs::write(
                    bundle.paths.root.join("artifacts/flow/flow-source.yaml"),
                    "invalid",
                )
                .unwrap();
            }
            "lineage" => {
                store
                    .update_run_record("retry-leaf", |r| r.root_run_id = Some("wrong-root".into()))
                    .unwrap();
            }
            _ => {}
        }
        let service = AttractorApiService::new_with_runtime_handler_runner_factory(
            settings,
            Arc::new(move || {
                if case == "custom" {
                    RuntimeHandlerRunner::new()
                        .with_child_run_launcher(|_| panic!("custom launcher owns child"))
                } else {
                    RuntimeHandlerRunner::new()
                }
            }),
        );
        let recovery = service.recover_interrupted_runs();
        assert_eq!(recovery["resumed"], json!([]), "{case}: {recovery}");
        assert_eq!(controls.get_checkpoint("retry-leaf").unwrap(), before);
        if matches!(case, "canceled" | "cancel_requested") {
            assert_eq!(
                store
                    .read_run_bundle("retry-leaf")
                    .unwrap()
                    .unwrap()
                    .record
                    .unwrap()
                    .status,
                "canceled"
            );
        }
        assert_eq!(store.list_run_records().unwrap().len(), 3);
    }
}

#[test]
fn direct_child_retry_rejects_ambiguous_ancestor_invocations_without_mutation() {
    let _serial = RETRY_TREE_TEST.lock().unwrap_or_else(|e| e.into_inner());
    for duplicate in ["retry-leaf", "retry-child"] {
        for invocation in [Some(1), None] {
            let temp = tempfile::tempdir().unwrap();
            let settings = settings(temp.path());
            let store = seed_failed_retry_tree(&settings);
            store
                .update_run_record(duplicate, |r| r.child_invocation_index = invocation)
                .unwrap();
            // A unique legacy child without an invocation index remains supported.
            assert!(attractor_runtime::RuntimeControls::new(store.clone())
                .retry_target("retry-leaf")
                .is_ok());
            let source = store.read_run_bundle(duplicate).unwrap().unwrap();
            let mut record = source.record.unwrap();
            record.run_id = "duplicate".into();
            store
                .create_run(attractor_runtime::CreateRunRequest {
                    record,
                    checkpoint: source.checkpoint,
                    manifest: None,
                    flow_source: store.read_graph_source(&source.paths).unwrap(),
                    flow_definition_json: None,
                })
                .unwrap();
            let ids = ["retry-root", "retry-child", "retry-leaf", "duplicate"];
            let before = ids.map(|id| {
                let bundle = store.read_run_bundle(id).unwrap().unwrap();
                (bundle.record, bundle.checkpoint, bundle.raw_events)
            });
            let response = blocking_gate_service(&settings).retry_pipeline_route("retry-leaf");
            assert_eq!(
                response.status_code, 409,
                "{duplicate}/{invocation:?}: {:?}",
                response.body
            );
            assert_eq!(response.body["detail"], "retry_ambiguous_child_invocation");
            for (id, expected) in ids.into_iter().zip(before) {
                let bundle = store.read_run_bundle(id).unwrap().unwrap();
                assert_eq!(
                    (bundle.record, bundle.checkpoint, bundle.raw_events),
                    expected,
                    "{id}"
                );
            }
        }
    }
}
