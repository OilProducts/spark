use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use attractor_api::AttractorApiService;
use attractor_core::CheckpointState;
use attractor_core::FlowDefinition;
use attractor_runtime::{
    human_gate_answered_event, prepare_fresh_run, NodeArtifacts, RunStore, RuntimeHandlerRunner,
};
use serde_json::{json, Value};
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
        ui_dir: None,
        project_roots: Vec::new(),
    }
}

fn gate_flow_json() -> Value {
    json!({
        "schema_version": "2",
        "id": "recovery_gate",
        "title": "Recovery Gate",
        "nodes": {
            "start": {
                "kind": "start",
                "config": {"kind": "start"},
                "routes": {
                    "next": {"to": "review", "description": "Enter the first workflow step."}
                }
            },
            "review": {
                "kind": "human_gate",
                "config": {"kind": "human_gate", "prompt": "Ship the report?"},
                "routes": {
                    "finish": {
                        "to": "done",
                        "label": "Finish",
                        "description": "The reviewer chose Finish."
                    }
                }
            },
            "done": {
                "kind": "exit",
                "config": {
                    "kind": "exit",
                    "result": {"name": "completed", "outcome": "success"}
                }
            }
        }
    })
}

fn paused_recovery_flow_json() -> Value {
    json!({
        "schema_version": "2",
        "id": "paused_recovery",
        "title": "Paused Recovery",
        "nodes": {
            "start": {
                "kind": "start",
                "config": {"kind": "start"},
                "routes": {
                    "next": {"to": "review", "description": "Enter the first workflow step."}
                }
            },
            "review": {
                "kind": "human_gate",
                "runtime": {"recovery_policy": "pause"},
                "config": {"kind": "human_gate", "prompt": "Retry me?"},
                "contracts": {"writes_context": ["context.accepted"]},
                "routes": {
                    "finish": {
                        "to": "done",
                        "label": "Finish",
                        "description": "The reviewer chose Finish."
                    }
                }
            },
            "done": {
                "kind": "exit",
                "config": {
                    "kind": "exit",
                    "result": {"name": "completed", "outcome": "success"}
                }
            }
        }
    })
}

fn tree_root_flow_json() -> Value {
    json!({
        "schema_version": "2",
        "id": "tree_root",
        "title": "Tree Root",
        "nodes": {
            "start": {
                "kind": "start",
                "config": {"kind": "start"},
                "routes": {
                    "next": {"to": "branch", "description": "Enter the first workflow step."}
                }
            },
            "branch": {
                "kind": "subflow",
                "runtime": {"recovery_policy": "pause"},
                "config": {
                    "kind": "subflow",
                    "flow_ref": "child",
                    "results": {"completed": "done"}
                },
                "manager": {"poll_interval": "0s", "max_cycles": 1},
                "routes": {"done": {"to": "done", "description": "The step is complete."}}
            },
            "done": {
                "kind": "exit",
                "config": {
                    "kind": "exit",
                    "result": {"name": "completed", "outcome": "success"}
                }
            }
        }
    })
}

fn tree_child_flow_json() -> Value {
    json!({
        "schema_version": "2",
        "id": "tree_child",
        "title": "Tree Child",
        "nodes": {
            "start": {
                "kind": "start",
                "runtime": {"recovery_policy": "pause"},
                "config": {"kind": "start"},
                "routes": {
                    "next": {"to": "done", "description": "Enter the first workflow step."}
                }
            },
            "done": {
                "kind": "exit",
                "config": {
                    "kind": "exit",
                    "result": {"name": "completed", "outcome": "success"}
                }
            }
        }
    })
}

fn flow(value: Value) -> FlowDefinition {
    attractor_dsl::validated_flow(serde_json::from_value(value).expect("flow definition JSON"))
        .expect("valid flow")
}

/// A run's `artifacts/flow/flow-definition.json` text for `value`.
fn definition_json(value: Value) -> String {
    flow(value).to_canonical_json_string()
}

/// Creates a run whose durable state says "parked at the review gate" with no
/// executor attached — exactly what a server restart leaves behind.
fn manufacture_orphaned_waiting_run(
    settings: &SparkSettings,
    workdir: &Path,
    run_id: &str,
) -> RunStore {
    let store = RunStore::for_settings(settings);
    let flow = flow(gate_flow_json());
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
        Some(definition_json(gate_flow_json())),
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
        visit_counts: Default::default(),
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
    let root_flow = flow(tree_root_flow_json());
    let child_flow = flow(tree_child_flow_json());

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
        Some(definition_json(tree_root_flow_json())),
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
                visit_counts: Default::default(),
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
        Some(definition_json(tree_child_flow_json())),
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
                visit_counts: Default::default(),
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
                    "status": "completed",
                    "decision": "next",
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
    let flow = flow(paused_recovery_flow_json());
    let mut record = attractor_core::RunRecord::new("pause-answered", workdir.to_string_lossy());
    record.execution_profile_id = Some("native".to_string());
    let paths = prepare_fresh_run(
        &store,
        &record,
        &flow,
        Some(definition_json(paused_recovery_flow_json())),
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
                visit_counts: Default::default(),
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
                    "status": "completed",
                    "decision": "finish",
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
    let flow = flow(paused_recovery_flow_json());
    let node = flow.nodes.get("review").expect("review node");
    let record = attractor_core::RunRecord::new("invalid-durable", workdir.to_string_lossy());
    let paths = prepare_fresh_run(
        &store,
        &record,
        &flow,
        Some(definition_json(paused_recovery_flow_json())),
        None,
        &attractor_core::LaunchContext::empty(),
        &attractor_core::ContextMap::default(),
    )
    .expect("prepare");

    let valid_status = json!({
        "status": "completed",
        "decision": "finish",
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
        (1, json!({"status": "completed"})),
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
                "finish",
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
    let flow = flow(paused_recovery_flow_json());
    let mut record = attractor_core::RunRecord::new("paused-twice", workdir.to_string_lossy());
    record.execution_profile_id = Some("native".to_string());
    let paths = prepare_fresh_run(
        &store,
        &record,
        &flow,
        Some(definition_json(paused_recovery_flow_json())),
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
                visit_counts: Default::default(),
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
        // A terminal result replaces the pending one so consumers see the end.
        let paths = store.find_run_root("broken-lineage").unwrap().unwrap();
        let result = store.read_result(&paths).unwrap().expect("result");
        assert_eq!(
            (result.status.as_str(), result.state.as_str()),
            ("failed", "error")
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
    let paths = store.find_run_root("run-orphan-cancel").unwrap().unwrap();
    let result = store.read_result(&paths).unwrap().expect("result");
    assert_eq!(
        (result.status.as_str(), result.state.as_str()),
        ("canceled", "error")
    );
}

const ORPHAN_RUN: &str = "run-orphan-processes";
const ORPHAN_LIFE_ENV: &str = "SPARK_RECOVERY_ORPHAN_LIFE";
const ORPHAN_ROOT_ENV: &str = "SPARK_RECOVERY_ORPHAN_ROOT";
const ORPHAN_CHILD_ENV: &str = "SPARK_RECOVERY_ORPHAN_CHILD";
const BOUNDARY_EXIT_ENV: &str = "SPARK_RECOVERY_BOUNDARY_EXIT";
const TOLERATE_ENV: &str = "SPARK_RECOVERY_TOLERATE";
const ORPHAN_CRASH_CODE: i32 = 7;
const ORPHAN_DEADLINE: Duration = Duration::from_secs(60);

fn orphan_wait(mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + ORPHAN_DEADLINE;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

/// A tool that announces itself and holds, surviving TERM so that only a KILL
/// ends it: a cancellation that dies after telling it to stop leaves it running.
fn orphan_tool(root: &Path, signal: &str, next: &str) -> Value {
    let p = |name: &str| root.join(name).display().to_string();
    json!({"kind": "tool", "config": {"kind": "tool", "exit_codes": {"0": "done"},
        "command": format!(
            "exec 2>/dev/null; trap \"touch '{term}'\" TERM; ps -o pgid= -p $$ | tr -d ' ' >> '{pids}'; \
             touch '{signal}'; i=0; while [ $i -lt 2400 ]; do sleep 0.05; i=$((i+1)); done",
            term = p("term-seen"),
            pids = p("pids"),
            signal = p(signal),
        )},
        "routes": {"done": {"to": next, "description": "Continue."}}})
}

fn orphan_step(root: &Path, next: &str) -> Value {
    json!({"kind": "tool", "config": {"kind": "tool", "exit_codes": {"0": "done"},
        "command": format!("echo pre >> '{}'", root.join("calls").display())},
        "routes": {"done": {"to": next, "description": "Continue."}}})
}

fn orphan_flow(root: &Path, in_child: bool) -> FlowDefinition {
    let to = |target: &str| json!({"to": target, "description": "Continue."});
    let exit = json!({"kind": "exit", "config": {"kind": "exit",
        "result": {"name": "completed", "outcome": "success"}}});
    let work = if in_child { "sub" } else { "work" };
    let mut nodes = json!({
        "start": {"kind": "start", "config": {"kind": "start"}, "routes": {"next": to("fork")}},
        "fork": {"kind": "parallel", "config": {"kind": "parallel",
            "branches": ["pre", "sib"], "join": "join"}},
        "pre": orphan_step(root, work),
        "sib": orphan_tool(root, "sib-started", "join"),
        "join": {"kind": "fan_in", "config": {"kind": "fan_in"}, "routes": {"next": to("done")}},
        "done": exit.clone(),
    });
    nodes[work] = if in_child {
        json!({"kind": "subflow", "manager": {"poll_interval": "0s"},
            "config": {"kind": "subflow", "flow_ref": "child", "results": {"completed": "child_completed"}},
            "routes": {"child_completed": to("join")}})
    } else {
        orphan_tool(root, "work-started", "join")
    };
    let flow: FlowDefinition = serde_json::from_value(
        json!({"schema_version": "2", "id": "orphan_processes", "nodes": nodes}),
    )
    .expect("flow JSON");
    attractor_dsl::validated_flow(flow).expect("valid flow")
}

fn write_orphan_child_flow(root: &Path) {
    let to = |target: &str| json!({"to": target, "description": "Continue."});
    let flow: FlowDefinition = serde_json::from_value(json!({
        "schema_version": "2", "id": "child", "nodes": {
            "start": {"kind": "start", "config": {"kind": "start"}, "routes": {"next": to("work")}},
            "work": orphan_tool(root, "work-started", "cdone"),
            "cdone": {"kind": "exit", "config": {"kind": "exit",
                "result": {"name": "completed", "outcome": "success"}}},
        }
    }))
    .expect("child flow JSON");
    attractor_dsl::write_flow_dir(&root.join("flows/child"), &flow).expect("write child flow");
}

/// Child process entry point: the executor's first life, killed once both
/// branches' tools run; or a cancellation through the route, killed once a tool
/// has been told to stop. A no-op in the ordinary test run.
#[test]
fn orphan_processes_life() {
    let Ok(life) = std::env::var(ORPHAN_LIFE_ENV) else {
        return;
    };
    let root = std::path::PathBuf::from(std::env::var(ORPHAN_ROOT_ENV).unwrap());
    let settings = settings(&root);
    let crash_on = |signals: &'static [&'static str]| {
        let root = root.clone();
        std::thread::spawn(move || {
            if orphan_wait(|| signals.iter().all(|signal| root.join(signal).exists())) {
                std::process::exit(ORPHAN_CRASH_CODE);
            }
        });
    };
    if life == "accepted" {
        let conductor = root.clone();
        std::thread::spawn(move || hold_sibling_record_then_crash(&conductor));
        let flow = accepted_flow(&root, std::env::var(TOLERATE_ENV).as_deref() == Ok("true"));
        let _ = attractor_runtime::PipelineExecutor::new(RuntimeHandlerRunner::new()).execute(
            attractor_runtime::ExecuteRunRequest {
                store: RunStore::for_settings(&settings),
                record: attractor_core::RunRecord::new(
                    ORPHAN_RUN,
                    root.join("project").to_string_lossy(),
                ),
                flow_definition_json: Some(flow.to_canonical_json_string()),
                flow,
                launch_context: attractor_core::LaunchContext::empty(),
                runtime_context: attractor_core::ContextMap::new(),
                max_steps: None,
                start: Default::default(),
            },
        );
        panic!("the executor should have crashed");
    }
    if life == "boundary" {
        let conductor = root.clone();
        std::thread::spawn(move || hold_record_publication_then_crash(&conductor));
        let exit_code = std::env::var(BOUNDARY_EXIT_ENV).unwrap();
        let _ = attractor_runtime::PipelineExecutor::new(RuntimeHandlerRunner::new()).execute(
            attractor_runtime::ExecuteRunRequest {
                store: RunStore::for_settings(&settings),
                record: attractor_core::RunRecord::new(
                    ORPHAN_RUN,
                    root.join("project").to_string_lossy(),
                ),
                flow_definition_json: Some(
                    boundary_flow(&root, &exit_code).to_canonical_json_string(),
                ),
                flow: boundary_flow(&root, &exit_code),
                launch_context: attractor_core::LaunchContext::empty(),
                runtime_context: attractor_core::ContextMap::new(),
                max_steps: None,
                start: Default::default(),
            },
        );
        panic!("the executor should have crashed");
    }
    if life == "executor" {
        crash_on(&["work-started", "sib-started"]);
        let in_child = std::env::var(ORPHAN_CHILD_ENV).is_ok();
        let store = RunStore::for_settings(&settings);
        let _ = attractor_runtime::PipelineExecutor::new(RuntimeHandlerRunner::new()).execute(
            attractor_runtime::ExecuteRunRequest {
                store,
                record: attractor_core::RunRecord::new(
                    ORPHAN_RUN,
                    root.join("project").to_string_lossy(),
                ),
                flow_definition_json: Some(orphan_flow(&root, in_child).to_canonical_json_string()),
                flow: orphan_flow(&root, in_child),
                launch_context: attractor_core::LaunchContext::empty(),
                runtime_context: attractor_core::ContextMap::from([(
                    "internal.bundled_root".to_string(),
                    json!(root.to_string_lossy()),
                )]),
                max_steps: None,
                start: Default::default(),
            },
        );
        panic!("the executor should have crashed");
    }
    crash_on(&["term-seen"]);
    let response = AttractorApiService::new(settings).cancel_pipeline_route(ORPHAN_RUN);
    panic!(
        "the cancellation should have died mid-cleanup: {:?}",
        response.body
    );
}

/// `start -> fork[pre -> work, sib] -> join -> done`, where `work` goes on to the
/// join or to a failure exit by its exit code, once released.
fn boundary_flow(root: &Path, exit_code: &str) -> FlowDefinition {
    let p = |name: &str| root.join(name).display().to_string();
    let to = |target: &str| json!({"to": target, "description": "Continue."});
    let exit = |name: &str, outcome: &str| {
        json!({"kind": "exit", "config": {"kind": "exit",
        "result": {"name": name, "outcome": outcome}}})
    };
    let flow: FlowDefinition = serde_json::from_value(json!({
        "schema_version": "2", "id": "boundary", "nodes": {
            "start": {"kind": "start", "config": {"kind": "start"}, "routes": {"next": to("fork")}},
            "fork": {"kind": "parallel", "config": {"kind": "parallel",
                "branches": ["pre", "sib"], "join": "join"}},
            "pre": orphan_step(root, "work"),
            "work": {"kind": "tool", "config": {"kind": "tool",
                "exit_codes": {"0": "done", "1": "blocked"},
                "command": format!(
                    "echo work >> '{calls}'; touch '{started}'; i=0; \
                     while [ ! -f '{release}' ] && [ $i -lt 1200 ]; do sleep 0.05; i=$((i+1)); done; \
                     exit {exit_code}",
                    calls = p("calls"),
                    started = p("work-started"),
                    release = p("release"),
                )},
                "routes": {"done": to("join"), "blocked": to("failed")}},
            "sib": orphan_tool(root, "sib-started", "join"),
            "join": {"kind": "fan_in", "config": {"kind": "fan_in"}, "routes": {"next": to("done")}},
            "done": exit("completed", "success"),
            "failed": exit("blocked", "failure"),
        }
    }))
    .expect("flow JSON");
    attractor_dsl::validated_flow(flow).expect("valid flow")
}

fn boundary_branches(root: &Path) -> Option<std::path::PathBuf> {
    let paths = RunStore::for_settings(&settings(root))
        .find_run_root(ORPHAN_RUN)
        .ok()??;
    Some(paths.logs_dir().join("fork/executions/1-0/branches"))
}

/// Runs in the first life. A FIFO where the fork writes the branch's record holds
/// that write open with no reader, which is exactly after the branch saved its
/// progress at its boundary and before anything published. The process dies there.
fn hold_record_publication_then_crash(root: &Path) {
    let both_started =
        orphan_wait(|| root.join("work-started").exists() && root.join("sib-started").exists());
    assert!(both_started);
    let branches = boundary_branches(root).unwrap();
    let hold = std::process::Command::new("mkfifo")
        .arg(branches.join("0.json.tmp"))
        .status()
        .unwrap();
    assert!(hold.success());
    std::fs::write(root.join("release"), "").unwrap();
    let progress = branches.join("0.progress.json");
    let at_boundary = orphan_wait(|| {
        std::fs::read_to_string(&progress)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .is_some_and(|progress| matches!(progress["current"].as_str(), Some("join" | "failed")))
    });
    assert!(at_boundary);
    std::process::exit(ORPHAN_CRASH_CODE);
}

/// `start -> fork[pre -> work, sib] -> join -> done`. `work` holds; `sib`, released,
/// exits selecting itself, which has no visits left.
fn accepted_flow(root: &Path, tolerate: bool) -> FlowDefinition {
    let p = |name: &str| root.join(name).display().to_string();
    let to = |target: &str| json!({"to": target, "description": "Continue."});
    let flow: FlowDefinition = serde_json::from_value(json!({
        "schema_version": "2", "id": "accepted", "nodes": {
            "start": {"kind": "start", "config": {"kind": "start"}, "routes": {"next": to("fork")}},
            "fork": {"kind": "parallel", "config": {"kind": "parallel",
                "branches": ["pre", "sib"], "join": "join",
                "tolerate_branch_execution_errors": tolerate}},
            "pre": orphan_step(root, "work"),
            "work": orphan_tool(root, "work-started", "join"),
            "sib": {"kind": "tool", "runtime": {"max_visits": 1},
                "config": {"kind": "tool", "exit_codes": {"0": "again", "1": "done"},
                "command": format!(
                    "echo sib >> '{calls}'; i=0; \
                     while [ ! -f '{release}' ] && [ $i -lt 1200 ]; do sleep 0.05; i=$((i+1)); done",
                    calls = p("calls"),
                    release = p("release"),
                )},
                "routes": {"again": to("sib"), "done": to("join")}},
            "join": {"kind": "fan_in", "config": {"kind": "fan_in"}, "routes": {"next": to("done")}},
            "done": {"kind": "exit", "config": {"kind": "exit",
                "result": {"name": "completed", "outcome": "success"}}},
        }
    }))
    .expect("flow JSON");
    attractor_dsl::validated_flow(flow).expect("valid flow")
}

/// Runs in the first life: `sib`'s accepted result becomes durable with its
/// record's publication held open by a FIFO, and the process dies there.
fn hold_sibling_record_then_crash(root: &Path) {
    assert!(orphan_wait(|| root.join("work-started").exists()));
    let branches = boundary_branches(root).unwrap();
    assert!(orphan_wait(|| branches.join("1.progress.json").exists()));
    let hold = std::process::Command::new("mkfifo")
        .arg(branches.join("1.json.tmp"))
        .status()
        .unwrap();
    assert!(hold.success());
    std::fs::write(root.join("release"), "").unwrap();
    let accepted = orphan_wait(|| {
        std::fs::read_to_string(branches.join("1.progress.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .is_some_and(|progress| progress["accepted"]["decision"] == json!("again"))
    });
    assert!(accepted);
    std::process::exit(ORPHAN_CRASH_CODE);
}

/// The client's cancel of a crashed run keeps a branch's routing error that was
/// decided by a result accepted before the crash, while the run itself is canceled.
#[test]
fn canceling_a_crashed_run_through_the_route_keeps_an_accepted_routing_error() {
    for tolerate in [false, true] {
        let name = format!("tolerate={tolerate}");
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().to_path_buf();
        let settings = settings(&root);
        std::fs::create_dir_all(&settings.config_dir).expect("config dir");
        std::fs::create_dir_all(root.join("project")).expect("workdir");
        run_orphan_life_with(&root, "accepted", &[(TOLERATE_ENV, &tolerate.to_string())]);

        let branches = boundary_branches(&root).unwrap();
        assert!(!branches.join("1.json").exists(), "{name}: published");
        std::fs::remove_file(branches.join("1.json.tmp")).unwrap();
        let response = AttractorApiService::new(settings.clone()).cancel_pipeline_route(ORPHAN_RUN);
        assert_eq!(response.status_code, 200, "{name}: {:?}", response.body);
        assert_eq!(
            response.body["status"],
            json!("canceled"),
            "{name}: {:?}",
            response.body
        );

        let branch = |index: usize| -> Value {
            serde_json::from_str(
                &std::fs::read_to_string(branches.join(format!("{index}.json"))).unwrap(),
            )
            .unwrap()
        };
        let sib = branch(1);
        assert_eq!(sib["status"], json!("error"), "{name}: {sib}");
        assert_eq!(sib["error"]["kind"], json!("routing"), "{name}: {sib}");
        assert!(
            sib["error"]["reason"]
                .as_str()
                .unwrap()
                .contains("visit_limit_reached"),
            "{name}: {sib}"
        );
        assert!(sib.get("canceled_at").is_none(), "{name}: {sib}");
        assert_eq!(branch(0)["status"], json!("canceled"), "{name}");

        let store = RunStore::for_settings(&settings);
        let paths = store.find_run_root(ORPHAN_RUN).unwrap().unwrap();
        let result = store.read_result(&paths).unwrap().expect("result");
        assert_eq!(result.status, "canceled", "{name}");
        for group in std::fs::read_to_string(root.join("pids")).unwrap().lines() {
            assert!(
                !orphan_group_running(group),
                "{name}: group {group} still runs"
            );
        }
        let calls = std::fs::read_to_string(root.join("calls")).unwrap();
        assert_eq!(
            calls.lines().filter(|line| *line == "sib").count(),
            1,
            "{name}: the accepted invocation ran again"
        );
        assert!(
            !store.read_raw_events(&paths).unwrap().iter().any(|event| {
                event.event_type == "StageStarted"
                    && event.payload.get("node_id") == Some(&json!("join"))
            }),
            "{name}: the join ran"
        );
    }
}

/// How a crashed run with a branch at its boundary, and no record for it, ends.
#[derive(Debug, Clone, Copy)]
enum BoundaryRecovery {
    /// A client's cancel, finalized here since no executor is attached.
    CancelRoute,
    /// A cancel request on record, picked up by resuming the run.
    ResumeAfterRequest,
}

/// A branch that reached its join or a failure exit has ended, so settling it
/// after a crash keeps that, with its evidence, even though the run as a whole is
/// canceled. Nothing runs again and the join stays unexecuted.
#[test]
fn a_branch_at_its_boundary_keeps_its_result_when_a_crashed_run_is_canceled() {
    for (exit_code, boundary, status) in [
        ("0", "join", "completed"),
        ("1", "failed", "terminal_result"),
    ] {
        for recovery in [
            BoundaryRecovery::CancelRoute,
            BoundaryRecovery::ResumeAfterRequest,
        ] {
            let name = format!("{boundary} {recovery:?}");
            let temp = tempfile::tempdir().expect("tempdir");
            let root = temp.path().to_path_buf();
            let settings = settings(&root);
            std::fs::create_dir_all(&settings.config_dir).expect("config dir");
            std::fs::create_dir_all(root.join("project")).expect("workdir");
            run_orphan_life_with(&root, "boundary", &[(BOUNDARY_EXIT_ENV, exit_code)]);

            // The boundary: progress saved, no record, nothing published.
            let store = RunStore::for_settings(&settings);
            let paths = store.find_run_root(ORPHAN_RUN).unwrap().unwrap();
            let branches = boundary_branches(&root).unwrap();
            assert!(!branches.join("0.json").exists(), "{name}: published");
            let saved: Value = serde_json::from_str(
                &std::fs::read_to_string(branches.join("0.progress.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(saved["current"], json!(boundary), "{name}");
            assert_eq!(
                store.read_run_record(&paths).unwrap().unwrap().status,
                "running",
                "{name}"
            );
            // The hold was the test's; recovery writes the record normally.
            std::fs::remove_file(branches.join("0.json.tmp")).unwrap();

            let observer = {
                let (store, paths, root, branches) =
                    (store.clone(), paths.clone(), root.clone(), branches.clone());
                std::thread::spawn(move || {
                    let saw_terminal = orphan_wait(|| {
                        store.read_run_record(&paths).unwrap().unwrap().status == "canceled"
                    });
                    (
                        saw_terminal,
                        branches.join("0.json").exists(),
                        branches.join("1.json").exists(),
                        std::fs::read_to_string(root.join("pids"))
                            .unwrap()
                            .lines()
                            .all(|group| !orphan_group_running(group)),
                    )
                })
            };
            match recovery {
                BoundaryRecovery::CancelRoute => {
                    let response = AttractorApiService::new(settings.clone())
                        .cancel_pipeline_route(ORPHAN_RUN);
                    assert_eq!(response.status_code, 200, "{name}: {:?}", response.body);
                    assert_eq!(response.body["status"], json!("canceled"), "{name}");
                }
                BoundaryRecovery::ResumeAfterRequest => {
                    attractor_runtime::RuntimeControls::new(store.clone())
                        .request_cancel(ORPHAN_RUN)
                        .unwrap();
                    let bundle = store.read_run_bundle(ORPHAN_RUN).unwrap().unwrap();
                    // As production resumes: the executor polls the record for controls.
                    let control = attractor_runtime::disk_execution_control(bundle.paths.clone());
                    let result = attractor_runtime::PipelineExecutor::with_control(
                        RuntimeHandlerRunner::new(),
                        control,
                    )
                    .execute(attractor_runtime::ExecuteRunRequest {
                        store: store.clone(),
                        record: bundle.record.unwrap(),
                        flow: boundary_flow(&root, exit_code),
                        flow_definition_json: None,
                        launch_context: attractor_core::LaunchContext::empty(),
                        runtime_context: attractor_core::ContextMap::new(),
                        max_steps: None,
                        start: attractor_runtime::ExecutionStart::Resume {
                            paths: bundle.paths,
                            checkpoint: bundle.checkpoint.unwrap(),
                        },
                    })
                    .unwrap();
                    assert_eq!(
                        result.status, "canceled",
                        "{name}: {}",
                        result.failure_reason
                    );
                    assert_eq!(result.failure_reason, "aborted_by_user", "{name}");
                }
            }
            assert_eq!(observer.join().unwrap(), (true, true, true, true), "{name}");

            // The branch keeps how it ended and everything it saved.
            let branch = |index: usize| -> Value {
                serde_json::from_str(
                    &std::fs::read_to_string(branches.join(format!("{index}.json"))).unwrap(),
                )
                .unwrap()
            };
            let affected = branch(0);
            assert_eq!(affected["status"], json!(status), "{name}: {affected}");
            assert_eq!(affected["id"], json!("pre"), "{name}: {affected}");
            assert!(affected.get("canceled_at").is_none(), "{name}: {affected}");
            assert!(affected.get("error").is_none(), "{name}: {affected}");
            assert_eq!(
                affected["route_trace"],
                json!(["pre", "work", boundary]),
                "{name}: {affected}"
            );
            assert_eq!(
                affected["visits"], saved["visits"],
                "{name}: visits changed"
            );
            assert_eq!(affected["visits"]["work"], json!(1), "{name}: {affected}");
            assert_eq!(
                affected["context"], saved["context"],
                "{name}: context changed"
            );
            assert_eq!(affected["decisions"], saved["decisions"], "{name}");
            if boundary == "failed" {
                let terminal = &affected["terminal"];
                assert_eq!(terminal["exit_id"], json!("failed"), "{name}: {affected}");
                assert_eq!(terminal["result"]["name"], json!("blocked"), "{name}");
                assert_eq!(terminal["result"]["outcome"], json!("failure"), "{name}");
                assert_eq!(terminal["branch"], json!("pre"), "{name}");
                assert_eq!(terminal["branch_index"], json!(0), "{name}");
                assert_eq!(terminal["node"], json!("work"), "{name}");
                assert_eq!(terminal["decision"], json!("blocked"), "{name}");
                assert_eq!(terminal["notes"], saved["last"][2], "{name}");
            } else {
                assert!(affected.get("terminal").is_none(), "{name}: {affected}");
            }
            assert_eq!(affected["notes"], saved["last"][2], "{name}: {affected}");

            // External cancellation still decides the run, and everything settled.
            let sibling = branch(1);
            assert_eq!(sibling["status"], json!("canceled"), "{name}: {sibling}");
            let record = store.read_run_record(&paths).unwrap().unwrap();
            assert_eq!(record.status, "canceled", "{name}");
            let result = store.read_result(&paths).unwrap().expect("result");
            assert_eq!(result.status, "canceled", "{name}");
            let events = store.read_raw_events(&paths).unwrap();
            assert_eq!(
                events
                    .iter()
                    .filter(|event| event.event_type == "runtime"
                        && event.payload.get("status") == Some(&json!("canceled")))
                    .count(),
                1,
                "{name}"
            );
            assert!(
                !events.iter().any(|event| event.event_type == "StageStarted"
                    && event.payload.get("node_id") == Some(&json!("join"))),
                "{name}: the join ran"
            );
            let calls = std::fs::read_to_string(root.join("calls")).unwrap();
            assert_eq!(
                calls.lines().filter(|line| *line == "pre").count(),
                1,
                "{name}: pre ran again"
            );
            assert_eq!(
                calls.lines().filter(|line| *line == "work").count(),
                1,
                "{name}: work ran again"
            );
            let tools = paths.root.join("tool-processes");
            let owned = std::fs::read_dir(tools)
                .map(|entries| entries.count())
                .unwrap_or(0);
            assert_eq!(owned, 0, "{name}: ownership left behind");
        }
    }
}

fn run_orphan_life(root: &Path, life: &str, in_child: bool) {
    if in_child {
        run_orphan_life_with(root, life, &[(ORPHAN_CHILD_ENV, "1")]);
    } else {
        run_orphan_life_with(root, life, &[]);
    }
}

fn run_orphan_life_with(root: &Path, life: &str, envs: &[(&str, &str)]) {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "run_recovery_contracts::orphan_processes_life",
            "--nocapture",
        ])
        .env(ORPHAN_LIFE_ENV, life)
        .env(ORPHAN_ROOT_ENV, root)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command.envs(envs.iter().copied());
    let mut process = command.spawn().unwrap();
    let mut status = None;
    orphan_wait(|| {
        status = process.try_wait().unwrap();
        status.is_some()
    });
    if status.is_none() {
        let _ = process.kill();
        let _ = process.wait();
    }
    assert_eq!(
        status.and_then(|status| status.code()),
        Some(ORPHAN_CRASH_CODE),
        "{life} must die at its crash point"
    );
}

fn orphan_group_running(group: &str) -> bool {
    let output = std::process::Command::new("ps")
        .args(["-A", "-o", "pgid=,stat="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).lines().any(|line| {
        let mut fields = line.split_whitespace();
        fields.next() == Some(group) && fields.next().is_some_and(|stat| !stat.starts_with('Z'))
    })
}

/// A run whose executor crashed, canceled through the route as a client would:
/// a cancellation crashing mid-cleanup publishes nothing, the same route finishes
/// it, the first sight of the terminal state finds its processes, branches and
/// child settled, and repeating the route changes nothing.
#[test]
fn canceling_a_crashed_run_through_the_route_settles_before_it_publishes() {
    for in_child in [false, true] {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().to_path_buf();
        let settings = settings(&root);
        std::fs::create_dir_all(&settings.config_dir).expect("config dir");
        std::fs::create_dir_all(root.join("project")).expect("workdir");
        if in_child {
            write_orphan_child_flow(&root);
        }
        run_orphan_life(&root, "executor", in_child);

        let store = RunStore::for_settings(&settings);
        let paths = store.find_run_root(ORPHAN_RUN).unwrap().unwrap();
        let groups = || -> Vec<String> {
            std::fs::read_to_string(root.join("pids"))
                .unwrap()
                .lines()
                .map(str::to_string)
                .collect()
        };
        assert_eq!(groups().len(), 2, "both tools started");
        let status = || store.read_run_record(&paths).unwrap().unwrap().status;
        assert_eq!(status(), "running", "the crash published nothing");

        // Cancellation crashes with a tool told to stop.
        run_orphan_life(&root, "cancel", in_child);
        assert_eq!(status(), "cancel_requested", "durable, not terminal");
        assert_eq!(
            store.read_result(&paths).unwrap().expect("result").state,
            "pending"
        );
        let canceled_events = || {
            store
                .read_raw_events(&paths)
                .unwrap()
                .iter()
                .filter(|event| event.payload.get("status") == Some(&json!("canceled")))
                .count()
        };
        assert_eq!(canceled_events(), 0);
        assert!(
            groups().iter().any(|group| orphan_group_running(group)),
            "the crash left a tool running for the retry to stop"
        );

        let child_records = || {
            store
                .list_run_records()
                .unwrap()
                .into_iter()
                .filter(|record| record.parent_run_id.as_deref() == Some(ORPHAN_RUN))
                .collect::<Vec<_>>()
        };
        let branches = paths.logs_dir().join("fork/executions/1-0/branches");

        // The first sight of the terminal state finds everything settled, with
        // no further wait for any of it.
        let observer = {
            let (store, paths, root, branches) =
                (store.clone(), paths.clone(), root.clone(), branches.clone());
            std::thread::spawn(move || {
                let saw_terminal = orphan_wait(|| {
                    store.read_run_record(&paths).unwrap().unwrap().status == "canceled"
                });
                let tools_stopped = std::fs::read_to_string(root.join("pids"))
                    .unwrap()
                    .lines()
                    .all(|group| !orphan_group_running(group));
                let children_settled = store
                    .list_run_records()
                    .unwrap()
                    .iter()
                    .filter(|record| record.parent_run_id.as_deref() == Some(ORPHAN_RUN))
                    .all(|record| record.status == "canceled");
                (
                    saw_terminal,
                    tools_stopped,
                    children_settled,
                    branches.join("0.json").exists(),
                    branches.join("1.json").exists(),
                )
            })
        };
        let response = AttractorApiService::new(settings.clone()).cancel_pipeline_route(ORPHAN_RUN);
        assert_eq!(response.status_code, 200, "{:?}", response.body);
        assert_eq!(
            response.body["status"],
            json!("canceled"),
            "{:?}",
            response.body
        );
        assert_eq!(observer.join().unwrap(), (true, true, true, true, true));

        // Each branch's evidence, the child and the published result.
        let branch = |index: usize| -> Value {
            serde_json::from_str(
                &std::fs::read_to_string(branches.join(format!("{index}.json"))).unwrap(),
            )
            .unwrap()
        };
        let work = if in_child { "sub" } else { "work" };
        let affected = branch(0);
        assert_eq!(affected["status"], json!("canceled"), "{affected}");
        assert_eq!(affected["canceled_at"], json!(work), "{affected}");
        assert_eq!(affected["route_trace"], json!(["pre", work]), "{affected}");
        assert_eq!(affected["visits"]["pre"], json!(1), "{affected}");
        assert!(
            affected["context"]["context.tool.exit_code"].is_number(),
            "{affected}"
        );
        assert!(affected.get("error").is_none(), "{affected}");
        let sibling = branch(1);
        assert_eq!(sibling["status"], json!("canceled"), "{sibling}");
        assert_eq!(sibling["route_trace"], json!(["sib"]), "{sibling}");
        assert_eq!(child_records().len(), usize::from(in_child));
        for child in child_records() {
            let child_paths = store.find_run_root(&child.run_id).unwrap().unwrap();
            assert_eq!(child.status, "canceled");
            assert_eq!(
                store
                    .read_result(&child_paths)
                    .unwrap()
                    .expect("child result")
                    .status,
                "canceled"
            );
        }
        let record = store.read_run_record(&paths).unwrap().unwrap();
        assert_eq!(record.status, "canceled");
        assert!(
            record.last_error.contains("no executor attached"),
            "{}",
            record.last_error
        );
        let result = store.read_result(&paths).unwrap().expect("result");
        assert_eq!(
            (result.status.as_str(), result.state.as_str()),
            ("canceled", "error")
        );
        assert_eq!(canceled_events(), 1);

        // Repeating the cancellation changes nothing.
        let repeated = AttractorApiService::new(settings.clone()).cancel_pipeline_route(ORPHAN_RUN);
        assert_eq!(repeated.status_code, 200, "{:?}", repeated.body);
        assert_ne!(
            repeated.body["status"],
            json!("cancel_requested"),
            "{:?}",
            repeated.body
        );
        assert_eq!(status(), "canceled");
        assert_eq!(canceled_events(), 1);
        assert_eq!(
            std::fs::read_to_string(root.join("calls"))
                .unwrap()
                .lines()
                .count(),
            1,
            "the finished step ran once"
        );
    }
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
                "finish",
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
    let flow = flow(gate_flow_json());
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
            Some(definition_json(gate_flow_json())),
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
    let flow = flow(gate_flow_json());

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
            Some(definition_json(gate_flow_json())),
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
            tree_root_flow_json(),
            "branch",
        ),
        (
            "retry-child",
            Some("retry-root"),
            Some("retry-leaf"),
            tree_root_flow_json(),
            "branch",
        ),
        (
            "retry-leaf",
            Some("retry-child"),
            None,
            tree_child_flow_json(),
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
            (
                "_attractor.node_outcomes".into(),
                json!({node: {"status": "error", "decision": ""}}),
            ),
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
                    visit_counts: Default::default(),
                    logs: vec!["prior log".into()],
                }),
                manifest: None,
                flow_definition_json: Some(definition_json(source)),
                flow_files: None,
            })
            .unwrap();
        store.write_node_artifacts(&paths, node, u64::from(node == "branch"), 0, &NodeArtifacts {
            response: Some("prior failed attempt".into()),
            status: Some(json!({"status":"error", "decision":"", "failure_kind":"runtime", "failure_reason":"prior failure", "context_updates":{}, "notes":"", "retryable":false})),
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
                None,
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
    checkpoint.context.insert(
        "_attractor.node_outcomes".into(),
        json!({"branch": {"status": "error", "decision": ""}}),
    );
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
                bundle
                    .paths
                    .root
                    .join("artifacts/flow/flow-definition.json"),
                "{\"invalid\": [",
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

fn replace_retry_leaf(store: &RunStore, source: Value, node: &str) {
    let bundle = store.read_run_bundle("retry-leaf").unwrap().unwrap();
    std::fs::write(
        bundle
            .paths
            .root
            .join("artifacts/flow/flow-definition.json"),
        definition_json(source),
    )
    .unwrap();
    let mut checkpoint = bundle.checkpoint.unwrap();
    checkpoint.current_node = node.into();
    checkpoint.completed_nodes = vec!["start".into(), node.into()];
    checkpoint.context.insert(
        "_attractor.node_outcomes".into(),
        json!({node: {"status": "error", "decision": ""}}),
    );
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
    replace_retry_leaf(&store, gate_flow_json(), "review");
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
                "finish",
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
    let mut source = gate_flow_json();
    // Exit code 1 is unmapped, so the tool ends with an execution error.
    source["nodes"]["review"] = json!({
        "kind": "tool",
        "config": {"kind": "tool", "command": "exit 1", "exit_codes": {"0": "finish"}},
        "routes": {"finish": {"to": "done", "description": "The review passed."}}
    });
    replace_retry_leaf(&store, source, "review");
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
        replace_retry_leaf(&store, gate_flow_json(), "review");
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
                status: Some(json!({"status":"completed", "decision":"finish", "context_updates":{}, "notes":""})),
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
                        "finish",
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
                    bundle
                        .paths
                        .root
                        .join("artifacts/flow/flow-definition.json"),
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
                    flow_definition_json: store
                        .read_graph_source(&source.paths)
                        .unwrap()
                        .map(|flow| flow.to_canonical_json_string()),
                    flow_files: None,
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

/// One unreadable record is reported under its project, and neither the runs
/// list nor startup recovery stops for the other runs.
#[test]
fn an_unreadable_run_is_reported_without_stopping_the_list_or_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path());
    let store = RunStore::for_settings(&settings);
    let project = temp.path().join("project").to_string_lossy().into_owned();
    let other = temp.path().join("other").to_string_lossy().into_owned();
    let create = |run_id: &str, status: &str| {
        let mut record = attractor_core::RunRecord::new(run_id, project.as_str());
        record.status = status.to_string();
        store
            .create_run(attractor_runtime::CreateRunRequest {
                record,
                ..Default::default()
            })
            .unwrap()
    };
    let bad = create("run-bad", "running");
    create("run-stopping", "cancel_requested");
    std::fs::write(bad.run_json(), b"").unwrap();

    let service = blocking_gate_service(&settings);
    let listed = |filter: Option<&str>| {
        let response = service.list_runs_for_project(filter);
        assert_eq!(response.status_code, 200, "{}", response.body);
        response.body["runs"].as_array().unwrap().clone()
    };
    for filter in [None, Some(project.as_str())] {
        let runs = listed(filter);
        let row = runs.iter().find(|run| run["run_id"] == "run-bad").unwrap();
        assert_eq!(row["status"], "unreadable");
        assert!(row["last_error"].as_str().unwrap().contains("run.json"));
        assert!(runs.iter().any(|run| run["run_id"] == "run-stopping"));
    }
    assert!(listed(Some(other.as_str())).is_empty());

    let recovery = service.recover_interrupted_runs();
    assert_eq!(recovery["failed"][0]["run_id"], "run-bad", "{recovery}");
    assert_eq!(recovery["canceled"], json!(["run-stopping"]), "{recovery}");
}
