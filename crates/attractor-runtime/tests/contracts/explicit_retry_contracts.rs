use attractor_core::{
    CheckpointState, ContextMap, FlowDefinition, LaunchContext, Outcome, OutcomeStatus, RunRecord,
};
use attractor_runtime::retry::{execution_attempt, saved_retry_request, EXECUTION_BASES_KEY};
use attractor_runtime::{
    CreateRunRequest, ExecuteRunRequest, ExecutionStart, NodeArtifacts, NodeExecutionRequest,
    PipelineExecutor, RunStore, RuntimeControls,
};
use serde_json::json;

const FLOW: &str = r#"schema_version: "1"
id: retry
title: Retry
nodes:
  start: {kind: start}
  task:
    kind: agent_task
    config: {kind: agent_task, prompt: test}
    retry: {max_retries: 1}
  done: {kind: exit}
edges:
- {from: start, to: task}
- {from: task, to: done}
"#;

fn seed(store: &RunStore) {
    let mut record = RunRecord::new("retry", "/tmp");
    record.status = "failed".into();
    let paths = store
        .create_run(CreateRunRequest {
            record,
            checkpoint: Some(CheckpointState {
                current_node: "task".into(),
                completed_nodes: vec!["start".into(), "task".into()],
                context: ContextMap::from([(
                    "_attractor.node_outcomes".into(),
                    json!({"task":"fail"}),
                )]),
                retry_counts: [("task".into(), 1), ("start".into(), 7)].into(),
                logs: vec!["preserve log".into()],
                timestamp: "2026-01-01T00:00:00Z".into(),
            }),
            manifest: None,
            flow_source: Some(FLOW.into()),
            flow_definition_json: None,
        })
        .unwrap();
    store.write_node_artifacts(&paths, "task", 1, 1, &NodeArtifacts {
        response: Some("old failure".into()),
        status: Some(json!({"outcome":"fail", "preferred_label":"", "suggested_next_ids":[], "context_updates":{}, "notes":"", "failure_reason":"old failure"})),
        under_logs: true,
        ..Default::default()
    }).unwrap();
}

fn resume(store: &RunStore, mut execute: impl FnMut(NodeExecutionRequest) -> Outcome) {
    let bundle = store.read_run_bundle("retry").unwrap().unwrap();
    PipelineExecutor::new(move |request| Ok(execute(request)))
        .execute(ExecuteRunRequest {
            store: store.clone(),
            record: bundle.record.unwrap(),
            flow: FlowDefinition::from_yaml_str(
                &store.read_graph_source(&bundle.paths).unwrap().unwrap(),
            )
            .unwrap()
            .normalize(),
            flow_source: None,
            flow_definition_json: None,
            launch_context: LaunchContext::empty(),
            runtime_context: Default::default(),
            max_steps: None,
            start: ExecutionStart::Resume {
                paths: bundle.paths,
                checkpoint: bundle.checkpoint.unwrap(),
            },
        })
        .unwrap();
}

#[test]
fn repeated_explicit_retries_restore_allowance_and_preserve_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let store = RunStore::for_runs_dir(temp.path());
    seed(&store);
    let controls = RuntimeControls::new(store.clone());
    let mut ids = Vec::new();
    for (round, expected) in [(0, vec![2, 3]), (1, vec![4, 5]), (2, vec![6])] {
        controls.prepare_retry("retry").unwrap();
        let checkpoint = controls.get_checkpoint("retry").unwrap();
        assert_eq!(checkpoint.retry_counts.get("start"), Some(&7));
        assert!(!checkpoint.retry_counts.contains_key("task"));
        assert_eq!(checkpoint.logs, ["preserve log"]);
        ids.push(
            saved_retry_request(&checkpoint.context)
                .unwrap()
                .unwrap()
                .id,
        );
        let mut attempts = Vec::new();
        resume(&store, |request| {
            if request.node_id == "task" {
                attempts.push(request.attempt);
                return Outcome {
                    raw_response_text: format!("attempt {}", request.attempt),
                    ..Outcome::new(if round < 2 {
                        OutcomeStatus::Fail
                    } else {
                        OutcomeStatus::Success
                    })
                };
            }
            Outcome::new(OutcomeStatus::Success)
        });
        assert_eq!(attempts, expected);
    }
    assert!(ids.windows(2).all(|pair| pair[0] != pair[1]));
    let bundle = store.read_run_bundle("retry").unwrap().unwrap();
    assert_eq!(bundle.record.unwrap().status, "completed");
    for attempt in 1..=6 {
        let path = store
            .node_execution_root(&bundle.paths, "task", 1, attempt)
            .unwrap();
        assert!(
            path.join("response.md").is_file(),
            "missing attempt {attempt}"
        );
    }
}

#[test]
fn prepared_attempt_survives_restart_and_reuses_only_its_durable_response() {
    let temp = tempfile::tempdir().unwrap();
    let store = RunStore::for_runs_dir(temp.path());
    seed(&store);
    let controls = RuntimeControls::new(store.clone());
    controls.prepare_retry("retry").unwrap();
    let checkpoint = controls.get_checkpoint("retry").unwrap();
    let request = saved_retry_request(&checkpoint.context).unwrap().unwrap();
    controls.apply_retry_request(&request).unwrap();
    assert_eq!(controls.get_checkpoint("retry").unwrap(), checkpoint);
    let paths = store.read_run_bundle("retry").unwrap().unwrap().paths;
    store.write_node_artifacts(&paths, "task", 1, 2, &NodeArtifacts {
        response: Some("new durable success".into()),
        status: Some(json!({"outcome":"success", "preferred_label":"", "suggested_next_ids":[], "context_updates":{}, "notes":""})),
        under_logs: true, ..Default::default()
    }).unwrap();
    resume(&store, |request| {
        assert_ne!(
            request.node_id, "task",
            "durable response must prevent duplicate execution"
        );
        Outcome::new(OutcomeStatus::Success)
    });
    assert_eq!(
        store
            .read_run_bundle("retry")
            .unwrap()
            .unwrap()
            .record
            .unwrap()
            .status,
        "completed"
    );
}

#[test]
fn old_checkpoints_default_to_zero_and_invalid_attempt_metadata_is_rejected() {
    assert_eq!(execution_attempt(&ContextMap::new(), "task", 3).unwrap(), 3);
    for child_id in ["", "run"] {
        let value = json!({"id":"request", "preparing":false, "target":{"run_id":"run", "node_id":"task", "stage_index":1, "child":{"run_id":child_id, "node_id":"task", "stage_index":1}}});
        assert!(saved_retry_request(&ContextMap::from([(
            attractor_runtime::retry::RETRY_REQUEST_KEY.into(),
            value
        )]))
        .is_err());
    }

    for value in [
        json!(null),
        json!({"task": -1}),
        json!({"task": "bad"}),
        json!({"task": u64::MAX}),
    ] {
        assert!(execution_attempt(
            &ContextMap::from([(EXECUTION_BASES_KEY.into(), value)]),
            "task",
            1
        )
        .is_err());
    }
}

#[test]
fn continue_starts_a_new_run_without_retry_authority_or_execution_bases() {
    let temp = tempfile::tempdir().unwrap();
    let store = RunStore::for_runs_dir(temp.path());
    seed(&store);
    let controls = RuntimeControls::new(store.clone());
    controls.prepare_retry("retry").unwrap();
    let source = controls.get_checkpoint("retry").unwrap();
    assert!(attractor_runtime::retry::retry_authorizes_checkpoint(
        &source.context,
        "retry",
        "task",
        1
    ));
    assert!(!attractor_runtime::retry::retry_authorizes_checkpoint(
        &source.context,
        "retry",
        "done",
        1
    ));
    assert!(!attractor_runtime::retry::retry_authorizes_checkpoint(
        &source.context,
        "retry",
        "task",
        2
    ));
    let legacy = ContextMap::from([(
        attractor_runtime::INTERNAL_PIPELINE_RETRY_RUN_ID_KEY.into(),
        json!("retry"),
    )]);
    assert!(!attractor_runtime::retry::retry_authorizes_checkpoint(
        &legacy, "retry", "task", 1
    ));
    let started = controls
        .continue_from_snapshot(attractor_runtime::ContinueRunRequest {
            source_run_id: "retry".into(),
            start_node: "task".into(),
            flow_source_mode: "snapshot".into(),
            new_run_id: Some("continued".into()),
            flow: FlowDefinition::from_yaml_str(FLOW).unwrap().normalize(),
            flow_source: Some(FLOW.into()),
            flow_definition_json: None,
            flow_name: None,
            working_directory: None,
            model: None,
            llm_provider: None,
            llm_profile: None,
            reasoning_effort: None,
        })
        .unwrap();
    assert_eq!(started.run_id, "continued");
    let checkpoint = controls.get_checkpoint("continued").unwrap();
    assert!(!checkpoint.context.contains_key(EXECUTION_BASES_KEY));
    assert!(saved_retry_request(&checkpoint.context).unwrap().is_none());
    assert!(checkpoint.retry_counts.is_empty());
    assert_eq!(controls.get_checkpoint("retry").unwrap(), source);
}

#[test]
fn looping_retry_resumes_the_prepared_visit_and_reuses_its_durable_response() {
    const LOOP_FLOW: &str = r#"schema_version: "1"
id: looping_retry
title: Looping retry
nodes:
  start: {kind: start}
  task:
    kind: agent_task
    config: {kind: agent_task, prompt: test}
  again:
    kind: agent_task
    config: {kind: agent_task, prompt: loop}
  done: {kind: exit}
edges:
- {from: start, to: task}
- {from: task, to: again, condition: "preferred_label=again"}
- {from: task, to: done, condition: "preferred_label=done"}
- {from: again, to: task}
"#;
    for crash in ["ready", "preparing", "durable"] {
        let temp = tempfile::tempdir().unwrap();
        let store = RunStore::for_runs_dir(temp.path());
        let mut visits = Vec::new();
        PipelineExecutor::new(|request: NodeExecutionRequest| {
            let mut outcome = Outcome::new(OutcomeStatus::Success);
            if request.node_id == "task" {
                visits.push((request.stage_index, request.attempt));
                outcome.raw_response_text = format!("visit {}", visits.len());
                if visits.len() == 1 {
                    outcome.preferred_label = "again".into();
                } else {
                    outcome.status = OutcomeStatus::Fail;
                    outcome.retryable = Some(false);
                }
            }
            Ok(outcome)
        })
        .execute(ExecuteRunRequest {
            store: store.clone(),
            record: RunRecord::new("retry", temp.path().to_string_lossy()),
            flow: FlowDefinition::from_yaml_str(LOOP_FLOW)
                .unwrap()
                .normalize(),
            flow_source: Some(LOOP_FLOW.into()),
            flow_definition_json: None,
            launch_context: LaunchContext::empty(),
            runtime_context: Default::default(),
            max_steps: Some(10),
            start: ExecutionStart::Fresh,
        })
        .unwrap();
        assert_eq!(visits, [(1, 0), (3, 0)]);
        let controls = RuntimeControls::new(store.clone());
        let failed = store.read_run_bundle("retry").unwrap().unwrap();
        assert_eq!(failed.record.unwrap().status, "failed");
        assert_eq!(
            failed.checkpoint.unwrap().completed_nodes,
            ["start", "task", "again", "task"]
        );
        controls.prepare_retry("retry").unwrap();
        let prepared = controls.get_checkpoint("retry").unwrap();
        assert_eq!(prepared.completed_nodes, ["start", "task", "again"]);
        let mut request = saved_retry_request(&prepared.context).unwrap().unwrap();
        assert_eq!(request.target.stage_index, 3);
        if crash == "preparing" {
            request.preparing = true;
            let mut checkpoint = prepared.clone();
            checkpoint.context.insert(
                attractor_runtime::retry::RETRY_REQUEST_KEY.into(),
                json!(request),
            );
            store
                .save_checkpoint(&failed.paths, &checkpoint, Default::default())
                .unwrap();
            store
                .update_run_record("retry", |record| record.status = "failed".into())
                .unwrap();
        }
        if crash == "durable" {
            store.write_node_artifacts(&failed.paths, "task", 3, 1, &NodeArtifacts {
                response: Some("retried success".into()),
                status: Some(json!({"outcome":"success", "suggested_next_ids":[], "context_updates":{}, "preferred_label":"done", "notes":""})),
                under_logs: true,
                ..Default::default()
            }).unwrap();
        }
        // Reopen storage as a restarted executor would, using only persisted state.
        let restarted = RunStore::for_runs_dir(temp.path());
        let mut retried = Vec::new();
        resume(&restarted, |request| {
            let mut outcome = Outcome::new(OutcomeStatus::Success);
            assert_ne!(
                request.node_id, "again",
                "retry must finish the prepared visit"
            );
            if request.node_id == "task" {
                retried.push((request.stage_index, request.attempt));
                outcome.preferred_label = "done".into();
                outcome.raw_response_text = "retried success".into();
            }
            outcome
        });
        assert_eq!(
            retried,
            if crash == "durable" {
                vec![]
            } else {
                vec![(3, 1)]
            },
            "{crash}"
        );
        let completed = restarted.read_run_bundle("retry").unwrap().unwrap();
        assert_eq!(
            completed.record.as_ref().unwrap().status,
            "completed",
            "{crash}: {:?}",
            completed.record
        );
        let checkpoint = completed.checkpoint.unwrap();
        assert_eq!(
            checkpoint.completed_nodes,
            ["start", "task", "again", "task"]
        );
        assert_eq!(
            execution_attempt(&checkpoint.context, "task", 0).unwrap(),
            1
        );
        assert_eq!(
            saved_retry_request(&checkpoint.context)
                .unwrap()
                .unwrap()
                .id,
            request.id
        );
        for (stage, attempt, response) in [
            (1, 0, "visit 1"),
            (3, 0, "visit 2"),
            (3, 1, "retried success"),
        ] {
            let root = restarted
                .node_execution_root(&completed.paths, "task", stage, attempt)
                .unwrap();
            assert_eq!(
                std::fs::read_to_string(root.join("response.md")).unwrap(),
                format!("{response}\n")
            );
        }
    }
}
