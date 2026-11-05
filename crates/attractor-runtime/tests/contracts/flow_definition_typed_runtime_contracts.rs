use std::collections::BTreeMap;
use std::fs;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

use attractor_core::{
    attr_bool, attr_text, ContextMap, ExitOutcome, ExitResult, FailureKind, FlowDefinition,
    FlowNode, FlowRoute, LaunchContext, ManagerLoopConfig, NodeConfig, NodeContracts, NodeKind,
    NodeRuntimeConfig, Outcome, OutcomeStatus, RunRecord,
};
use attractor_runtime::{
    ChildRunResult, CreateRunRequest, ExecuteRunRequest, NodeExecutionRequest, NodeExecutor,
    PipelineExecutor, RunRootPaths, RunStore, RuntimeHandlerRunner, HANDLER_CODERGEN,
};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Schema-2 fixture builders
// ---------------------------------------------------------------------------

fn route(to: &str) -> FlowRoute {
    FlowRoute {
        to: to.to_string(),
        description: format!("Continue to {to}."),
        ..FlowRoute::default()
    }
}

fn routes(entries: &[(&str, &str)]) -> BTreeMap<String, FlowRoute> {
    entries
        .iter()
        .map(|(decision, to)| (decision.to_string(), route(to)))
        .collect()
}

fn start_node(to: &str) -> FlowNode {
    FlowNode {
        kind: NodeKind::Start,
        config: Some(NodeConfig::Start {}),
        routes: routes(&[("next", to)]),
        ..FlowNode::default()
    }
}

fn exit_node(name: &str, outcome: ExitOutcome) -> FlowNode {
    FlowNode {
        kind: NodeKind::Exit,
        config: Some(NodeConfig::Exit {
            result: ExitResult {
                name: name.to_string(),
                outcome,
            },
            result_summary: false,
            result_summary_prompt: None,
        }),
        ..FlowNode::default()
    }
}

fn success_exit() -> FlowNode {
    exit_node("completed", ExitOutcome::Success)
}

fn decision_envelope() -> Option<NodeContracts> {
    Some(NodeContracts {
        response: Some("decision_envelope".to_string()),
        ..NodeContracts::default()
    })
}

fn agent(prompt: &str, decisions: &[(&str, &str)]) -> FlowNode {
    FlowNode {
        kind: NodeKind::AgentTask,
        config: Some(NodeConfig::AgentTask {
            prompt: prompt.to_string(),
        }),
        contracts: decision_envelope(),
        routes: routes(decisions),
        ..FlowNode::default()
    }
}

fn fan_in(to: &str) -> FlowNode {
    FlowNode {
        kind: NodeKind::FanIn,
        config: Some(NodeConfig::FanIn {}),
        routes: routes(&[("next", to)]),
        ..FlowNode::default()
    }
}

fn fork(branches: &[&str], join: &str, tolerate: bool) -> FlowNode {
    FlowNode {
        kind: NodeKind::Parallel,
        config: Some(NodeConfig::Parallel {
            branches: branches.iter().map(|branch| branch.to_string()).collect(),
            join: join.to_string(),
            max_parallel: None,
            tolerate_branch_execution_errors: tolerate,
        }),
        ..FlowNode::default()
    }
}

fn flow_of(id: &str, nodes: Vec<(&str, FlowNode)>) -> FlowDefinition {
    FlowDefinition {
        schema_version: "2".to_string(),
        id: id.to_string(),
        title: id.to_string(),
        nodes: nodes
            .into_iter()
            .map(|(node_id, node)| (node_id.to_string(), node))
            .collect(),
        ..FlowDefinition::default()
    }
}

/// `start -> task -> done`; a task without routes declares `done`.
fn linear_flow(mut task: FlowNode) -> FlowDefinition {
    if task.routes.is_empty() {
        task.routes = routes(&[("done", "done")]);
    }
    flow_of(
        "typed-runtime",
        vec![
            ("start", start_node("task")),
            ("task", task),
            ("done", success_exit()),
        ],
    )
}

/// A node's ordinary completion: start's `next`, every other node's `done`.
fn step(node_id: &str) -> Outcome {
    Outcome::completed(if node_id == "start" { "next" } else { "done" })
}

/// One handler-level invocation of `node_id`.
fn node_request(
    flow: &FlowDefinition,
    node_id: &str,
    context: ContextMap,
    run_paths: Option<RunRootPaths>,
    run_workdir: &std::path::Path,
    run_id: &str,
) -> NodeExecutionRequest {
    NodeExecutionRequest {
        node_id: node_id.to_string(),
        stage_index: 0,
        attempt: 0,
        context,
        prompt: String::new(),
        node: flow.nodes[node_id].clone(),
        node_attrs: attractor_runtime::flow_runtime::node_attrs_for_handler(
            node_id,
            &flow.nodes[node_id],
        ),
        flow: flow.clone(),
        outgoing_edges: attractor_runtime::outgoing_routing_edges(flow, node_id).expect("outgoing"),
        run_paths,
        run_workdir: run_workdir.to_path_buf(),
        run_id: run_id.to_string(),
        fallback_model: None,
        fallback_provider: None,
        fallback_profile: None,
        fallback_reasoning_effort: None,
    }
}

fn create_run(store: &RunStore, run_id: &str, workdir: &std::path::Path) -> RunRootPaths {
    store
        .create_run(CreateRunRequest {
            record: RunRecord::new(run_id, workdir.to_string_lossy()),
            checkpoint: None,
            manifest: None,
            flow_definition_json: None,
            flow_files: None,
        })
        .expect("create run")
}

fn execute_request(
    store: &RunStore,
    run_id: &str,
    workdir: &std::path::Path,
    flow: FlowDefinition,
    max_steps: Option<usize>,
    start: attractor_runtime::ExecutionStart,
) -> ExecuteRunRequest {
    ExecuteRunRequest {
        store: store.clone(),
        record: RunRecord::new(run_id, workdir.to_string_lossy()),
        flow,
        flow_definition_json: None,
        launch_context: LaunchContext::empty(),
        runtime_context: ContextMap::new(),
        max_steps,
        start,
    }
}

/// The `status.json` of `node_id`'s latest execution in `run_id`.
fn latest_status(store: &RunStore, run_id: &str, node_id: &str) -> Value {
    let meta = store.read_run_meta(run_id).unwrap().unwrap();
    let execution = store
        .list_node_executions(&meta.paths)
        .unwrap()
        .into_iter()
        .filter(|execution| execution.node_id == node_id)
        .max_by_key(|execution| (execution.stage_index, execution.attempt))
        .unwrap_or_else(|| panic!("{node_id} execution"));
    serde_json::from_str(
        &fs::read_to_string(
            store
                .node_execution_root(
                    &meta.paths,
                    node_id,
                    execution.stage_index,
                    execution.attempt,
                )
                .unwrap()
                .join("status.json"),
        )
        .unwrap(),
    )
    .unwrap()
}

fn events_of(store: &RunStore, run_id: &str) -> Vec<attractor_core::RawRuntimeEvent> {
    let paths = store.find_run_root(run_id).unwrap().unwrap();
    store.read_raw_events(&paths).unwrap()
}

fn visit_limit(node: &str, decision: &str, target: &str, used: u64, limit: u64) -> String {
    format!(
        "visit_limit_reached: node \"{node}\" decision \"{decision}\" selects \"{target}\", which used {used} of its {limit} visits"
    )
}

fn no_matching_route(node: &str, decision: &str, declared: &[(&str, &str)]) -> String {
    let declared = declared
        .iter()
        .map(|(decision, target)| format!("\"{decision}\" -> \"{target}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "no_matching_route: node \"{node}\" returned decision \"{decision}\"; declared decisions [{declared}]"
    )
}

// ---------------------------------------------------------------------------
// Typed configuration drives handlers
// ---------------------------------------------------------------------------

#[test]
fn executor_enforces_typed_write_contract_without_extension_duplicate() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let flow = linear_flow(FlowNode {
        contracts: Some(NodeContracts {
            writes_context: vec!["context.allowed".to_string()],
            response: Some("decision_envelope".to_string()),
            ..NodeContracts::default()
        }),
        ..agent("write allowed context", &[])
    });
    let mut executor = PipelineExecutor::new(|request: NodeExecutionRequest| {
        if request.node_id == "task" {
            return Ok(Outcome {
                context_updates: ContextMap::from([("context.denied".to_string(), json!(true))]),
                ..Outcome::completed("done")
            });
        }
        Ok(step(&request.node_id))
    });

    let result = executor
        .execute(execute_request(
            &store,
            "run-typed-contract",
            temp.path(),
            flow,
            None,
            Default::default(),
        ))
        .expect("execute typed contract flow");

    assert_eq!(result.status, "failed");
    assert!(
        result.failure_reason.contains("context.denied"),
        "failure reason should name the denied typed contract key: {}",
        result.failure_reason
    );
    let status = latest_status(&store, "run-typed-contract", "task");
    assert_eq!(status["status"], "error");
    assert_eq!(status["failure_kind"], "contract");
}

#[test]
fn parallel_handler_uses_typed_parallel_config_without_extension_duplicate() {
    let mut fan = fork(&["bad", "good"], "join", false);
    if let Some(NodeConfig::Parallel { max_parallel, .. }) = fan.config.as_mut() {
        *max_parallel = Some(1);
    }
    let with_type = |kind: &str| FlowNode {
        extensions: [("type".to_string(), json!(kind))].into_iter().collect(),
        ..agent("branch", &[("done", "join")])
    };
    let flow = flow_of(
        "typed-parallel",
        vec![
            ("start", start_node("fan")),
            ("fan", fan),
            ("good", with_type("custom.success")),
            ("bad", with_type("custom.fail")),
            ("join", fan_in("done")),
            ("done", success_exit()),
        ],
    );
    let mut runner = RuntimeHandlerRunner::new();
    runner.register_static_handler(
        "custom.fail",
        Outcome::error(FailureKind::Runtime, "extension type ran", Some(false)),
    );
    runner.register_handler_fn(HANDLER_CODERGEN, |runtime| {
        // A branch invocation's identity is the fork's stage, then its
        // invocation number under the fork's prepared attempt.
        assert_eq!(runtime.stage_index, 0);
        assert_eq!(
            runtime.attempt, 3_000_000,
            "branches number their invocations under the prepared parallel attempt"
        );
        Ok(Outcome::completed("done"))
    });
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = create_run(
        &RunStore::for_runs_dir(temp.path().join("runs")),
        "run-typed-parallel",
        temp.path(),
    );

    let mut request = node_request(
        &flow,
        "fan",
        ContextMap::new(),
        Some(paths),
        temp.path(),
        "run-typed-parallel",
    );
    request.attempt = 3;
    let outcome = runner.execute(request).expect("parallel outcome");

    assert_eq!(outcome.status, OutcomeStatus::Completed, "{outcome:?}");
    assert!(outcome.terminal.is_none());
    let results = outcome.context_updates["parallel.results"]
        .as_array()
        .expect("results")
        .clone();
    assert_eq!(
        results
            .iter()
            .map(|record| (record["id"].clone(), record["status"].clone()))
            .collect::<Vec<_>>(),
        vec![
            (json!("bad"), json!("completed")),
            (json!("good"), json!("completed"))
        ]
    );
}

/// `fan` forks one branch, `work`, which loops on `again`; with
/// `route_to_join` it may also choose `done` to reach the paired join.
fn looping_branch_flow(max_visits: Option<u64>, route_to_join: bool) -> FlowDefinition {
    let mut decisions = vec![("again", "work")];
    if route_to_join {
        decisions.push(("done", "join"));
    }
    flow_of(
        "looping-branch",
        vec![
            ("start", start_node("fan")),
            ("fan", fork(&["work"], "join", false)),
            (
                "work",
                FlowNode {
                    runtime: Some(NodeRuntimeConfig {
                        max_visits,
                        ..NodeRuntimeConfig::default()
                    }),
                    ..agent("work", &decisions)
                },
            ),
            ("join", fan_in("done")),
            ("done", success_exit()),
        ],
    )
}

/// Runs `fan` through the real parallel path and returns its single branch
/// record plus how many times the branch ran `work`.
fn execute_looping_branch(
    runner: &mut RuntimeHandlerRunner,
    calls: &Arc<AtomicUsize>,
    flow: &FlowDefinition,
) -> Value {
    calls.store(0, Ordering::SeqCst);
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = create_run(
        &RunStore::for_runs_dir(temp.path().join("runs")),
        "run-looping-branch",
        temp.path(),
    );
    let outcome = runner
        .execute(node_request(
            flow,
            "fan",
            ContextMap::new(),
            Some(paths),
            temp.path(),
            "run-looping-branch",
        ))
        .expect("parallel outcome");
    let mut result = outcome.context_updates["parallel.results"][0].clone();
    result["work_calls"] = json!(calls.load(Ordering::SeqCst));
    result["fork_status"] = json!(outcome.status.as_str());
    result
}

/// A runner whose agent answers `decide(n)` on its n-th call.
fn counting_runner(
    decide: impl Fn(usize) -> Outcome + Send + 'static,
) -> (RuntimeHandlerRunner, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut runner = RuntimeHandlerRunner::new();
    let counter = calls.clone();
    runner.register_handler_fn(HANDLER_CODERGEN, move |_| {
        Ok(decide(counter.fetch_add(1, Ordering::SeqCst) + 1))
    });
    (runner, calls)
}

#[test]
fn parallel_branch_loop_without_alternative_fails_at_default_visit_limit() {
    let flow = looping_branch_flow(None, false);
    let (mut runner, calls) = counting_runner(|_| Outcome::completed("again"));

    for _ in 0..2 {
        // Each branch execution starts its visit counts afresh.
        let result = execute_looping_branch(&mut runner, &calls, &flow);
        assert_eq!(result["status"], "error", "{result}");
        assert_eq!(result["fork_status"], "error", "{result}");
        assert_eq!(result["work_calls"], 32, "{result}");
        assert_eq!(
            result["error"]["reason"],
            visit_limit("work", "again", "work", 32, 32)
        );
        assert_eq!(result["error"]["kind"], "routing");
        assert_eq!(result["error"]["tolerable"], false);
    }
}

/// A branch decision selecting its exhausted loop target fails the branch
/// even though a join route is declared: exhaustion never picks the other
/// route. Choosing the join route itself completes the branch at the join.
#[test]
fn parallel_branch_choosing_an_exhausted_loop_fails_despite_a_join_route() {
    // Both limits exceed nodes.len() + edges, the old branch step cap.
    for (max_visits, limit) in [(None, 32usize), (Some(12), 12)] {
        let flow = looping_branch_flow(max_visits, true);
        assert!(limit > flow.nodes.len() + flow.edges().len());

        let (mut runner, calls) = counting_runner(|_| Outcome::completed("again"));
        let result = execute_looping_branch(&mut runner, &calls, &flow);
        assert_eq!(result["status"], "error", "{result}");
        assert_eq!(result["work_calls"], limit, "{result}");
        assert_eq!(
            result["error"]["reason"],
            visit_limit("work", "again", "work", limit as u64, limit as u64)
        );
        assert!(
            !result["route_trace"]
                .as_array()
                .unwrap()
                .contains(&json!("join")),
            "{result}"
        );

        let (mut runner, calls) = counting_runner(move |call| {
            Outcome::completed(if call == limit { "done" } else { "again" })
        });
        for _ in 0..2 {
            let result = execute_looping_branch(&mut runner, &calls, &flow);
            assert_eq!(result["status"], "completed", "{result}");
            assert_eq!(
                result["route_trace"].as_array().unwrap().last(),
                Some(&json!("join"))
            );
            assert_eq!(result["work_calls"], limit, "{result}");
        }
    }
}

#[test]
fn handler_dispatch_ignores_legacy_extension_type_override() {
    let flow = linear_flow(FlowNode {
        extensions: [("type".to_string(), json!("custom.fail"))]
            .into_iter()
            .collect(),
        ..agent("task", &[])
    });
    let mut runner = RuntimeHandlerRunner::new();
    runner.register_static_handler(
        "custom.fail",
        Outcome::error(FailureKind::Runtime, "extension type ran", Some(false)),
    );
    runner.register_static_handler(HANDLER_CODERGEN, Outcome::completed("done"));

    let outcome = runner
        .execute(node_request(
            &flow,
            "task",
            ContextMap::new(),
            None,
            &std::env::current_dir().expect("current dir"),
            "run-typed-dispatch",
        ))
        .expect("typed handler outcome");

    assert_eq!(outcome.status, OutcomeStatus::Completed);
    assert_eq!(outcome.decision, "done");
}

/// The removed retry-target, goal-gate and partial-acceptance settings are
/// rejected wherever they appear, and never drive execution: an execution
/// error fails the run without jumping to the node they name.
#[test]
fn legacy_runtime_keys_in_extensions_are_rejected_and_never_drive_execution() {
    let legacy = [
        ("allow_partial", json!(true)),
        ("goal_gate", json!(true)),
        ("retry_target", json!("retry")),
        ("fallback_retry_target", json!("retry")),
    ];
    let node = FlowNode {
        extensions: legacy
            .clone()
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
        ..agent("task", &[("done", "done")])
    };
    let mut flow = linear_flow(node);
    flow.nodes
        .insert("retry".to_string(), agent("retry", &[("done", "done")]));

    let removed = flow
        .diagnostics()
        .into_iter()
        .filter(|diagnostic| {
            diagnostic.rule_id == "removed_field" && diagnostic.node_id.as_deref() == Some("task")
        })
        .count();
    assert_eq!(removed, legacy.len());

    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let calls = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = calls.clone();
    let result = PipelineExecutor::new(move |request: NodeExecutionRequest| {
        seen.lock().unwrap().push(request.node_id.clone());
        if request.node_id == "task" {
            return Ok(Outcome::error(
                FailureKind::Runtime,
                "gate failed",
                Some(false),
            ));
        }
        Ok(step(&request.node_id))
    })
    .execute(execute_request(
        &store,
        "run-legacy-keys",
        temp.path(),
        flow,
        None,
        Default::default(),
    ))
    .expect("execute");
    assert_eq!(result.status, "failed");
    assert_eq!(result.failure_reason, "gate failed");
    assert_eq!(*calls.lock().unwrap(), vec!["start", "task"]);
    assert_durably_failed(&store, "run-legacy-keys", "gate failed");
}

/// The typed forms of the removed settings are not fields at all.
#[test]
fn typed_legacy_runtime_keys_are_rejected_by_name() {
    for key in [
        "allow_partial",
        "goal_gate",
        "retry_target",
        "fallback_retry_target",
        "error_policy",
    ] {
        let error = serde_json::from_value::<NodeRuntimeConfig>(json!({ key: true }))
            .expect_err("removed runtime setting");
        assert!(error.to_string().contains(key), "{key}: {error}");
    }
    for key in ["join_policy", "join_k", "join_quorum"] {
        let error = serde_json::from_value::<NodeConfig>(json!({
            "kind": "parallel",
            "branches": ["a"],
            "join": "join",
            key: 1
        }))
        .expect_err("removed parallel setting");
        assert!(error.to_string().contains(key), "{key}: {error}");
    }
}

#[test]
fn node_context_core_keys_do_not_override_typed_handler_attrs() {
    let node = FlowNode {
        kind: NodeKind::Subflow,
        config: Some(NodeConfig::Subflow {
            flow_ref: "typed-child".to_string(),
            output_map: BTreeMap::new(),
            input_map: BTreeMap::new(),
            results: BTreeMap::from([("completed".to_string(), "done".to_string())]),
        }),
        manager: Some(ManagerLoopConfig {
            child_autostart: Some(false),
            ..ManagerLoopConfig::default()
        }),
        context: [
            ("stack.child_flow_ref".to_string(), json!("legacy-child")),
            ("stack.child_autostart".to_string(), json!(true)),
        ]
        .into_iter()
        .collect(),
        extensions: [
            ("manager.actions".to_string(), json!("observe,steer,wait")),
            ("custom.metadata".to_string(), json!("kept")),
        ]
        .into_iter()
        .collect(),
        ..FlowNode::default()
    };

    let attrs = attractor_runtime::flow_runtime::node_attrs_for_handler("run_child", &node);

    assert_eq!(
        attr_text(&attrs, "stack.child_flow_ref").as_deref(),
        Some("typed-child")
    );
    assert_eq!(attr_bool(&attrs, "stack.child_autostart", true), false);
    assert!(!attrs.contains_key("manager.actions"));
    assert!(attrs.contains_key("custom.metadata"));
}

#[test]
fn extension_only_subflow_child_ref_does_not_start_child_pipeline() {
    let flow = flow_of(
        "extension-child-ref-ignored",
        vec![(
            "run_child",
            FlowNode {
                kind: NodeKind::Subflow,
                manager: Some(ManagerLoopConfig {
                    max_cycles: Some(1),
                    child_autostart: Some(true),
                    ..ManagerLoopConfig::default()
                }),
                extensions: [("stack.child_flow_ref".to_string(), json!("child"))]
                    .into_iter()
                    .collect(),
                routes: routes(&[("done", "run_child")]),
                ..FlowNode::default()
            },
        )],
    );
    let temp = tempfile::tempdir().expect("tempdir");
    write_child_flow(&temp.path().join("child"), child_flow_source());
    let launched = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&launched);
    let mut runner = RuntimeHandlerRunner::new();
    runner.set_child_run_launcher(move |_request| {
        observed.store(true, Ordering::SeqCst);
        completed_child("")
    });

    let outcome = runner
        .execute(node_request(
            &flow,
            "run_child",
            ContextMap::from([(
                "internal.run_workdir".to_string(),
                json!(temp.path().to_string_lossy().to_string()),
            )]),
            None,
            temp.path(),
            "parent-run",
        ))
        .expect("subflow outcome");

    assert_eq!(outcome.status, OutcomeStatus::Error);
    assert!(!launched.load(Ordering::SeqCst));
}

/// A child that reached its `completed` exit.
fn completed_child(run_id: &str) -> ChildRunResult {
    ChildRunResult {
        run_id: run_id.to_string(),
        status: "completed".to_string(),
        outcome: Some("success".to_string()),
        outcome_reason_code: Some("completed".to_string()),
        ..ChildRunResult::default()
    }
}

fn subflow_node(input_map: BTreeMap<String, String>, manager: ManagerLoopConfig) -> FlowNode {
    FlowNode {
        kind: NodeKind::Subflow,
        config: Some(NodeConfig::Subflow {
            flow_ref: "child".to_string(),
            output_map: BTreeMap::new(),
            input_map,
            results: BTreeMap::from([("completed".to_string(), "done".to_string())]),
        }),
        manager: Some(manager),
        routes: routes(&[("done", "run_child")]),
        ..FlowNode::default()
    }
}

#[test]
fn default_subflow_launch_applies_typed_input_map_to_child_context() {
    let temp = tempfile::tempdir().expect("tempdir");
    let child_source = json!({
        "schema_version": "2",
        "id": "child_input_map",
        "title": "Child Input Map",
        "nodes": {
            "start": {
                "kind": "start",
                "routes": { "next": { "to": "check", "description": "Check the mapped context." } }
            },
            "check": {
                "kind": "agent_task",
                "config": {
                    "kind": "agent_task",
                    "prompt": "Check mapped context"
                },
                "contracts": { "response": "decision_envelope" },
                "routes": { "done": { "to": "done", "description": "The context is mapped." } }
            },
            "done": {
                "kind": "exit",
                "config": { "kind": "exit", "result": { "name": "completed", "outcome": "success" } }
            }
        }
    });
    write_child_flow(&temp.path().join("child"), child_source);

    let parent_flow = flow_of(
        "parent_input_map",
        vec![(
            "run_child",
            subflow_node(
                [
                    ("context.child_ticket", "context.parent_ticket"),
                    ("context.child_count", "context.count"),
                    ("context.child_nested", "context.payload.ticket"),
                ]
                .into_iter()
                .map(|(child, parent)| (child.to_string(), parent.to_string()))
                .collect(),
                ManagerLoopConfig {
                    max_cycles: Some(1),
                    child_autostart: Some(true),
                    ..ManagerLoopConfig::default()
                },
            ),
        )],
    );
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let paths = create_run(&store, "parent-run", temp.path());
    let child_handler_observed_map = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&child_handler_observed_map);
    let mut runner = RuntimeHandlerRunner::new();
    runner.register_thread_safe_handler_fn(HANDLER_CODERGEN, move |runtime| {
        let has_map = runtime.context.get("context.child_ticket") == Some(&json!("TICKET-7"))
            && runtime.context.get("context.child_count") == Some(&json!(42))
            && runtime.context.get("context.child_nested") == Some(&json!("nested-7"));
        observed.store(has_map, Ordering::SeqCst);
        if has_map {
            Ok(Outcome::completed("done"))
        } else {
            Ok(Outcome::error(
                FailureKind::Runtime,
                format!(
                    "child context did not include mapped input values: {:?}",
                    runtime.context
                ),
                Some(false),
            ))
        }
    });

    let outcome = runner
        .execute(node_request(
            &parent_flow,
            "run_child",
            ContextMap::from([
                ("context.parent_ticket".to_string(), json!("TICKET-7")),
                ("context.count".to_string(), json!(42)),
                (
                    "context.payload".to_string(),
                    json!({ "ticket": "nested-7" }),
                ),
                (
                    "internal.bundled_root".to_string(),
                    json!(temp.path().to_string_lossy().to_string()),
                ),
                (
                    "internal.run_workdir".to_string(),
                    json!(temp.path().to_string_lossy().to_string()),
                ),
            ]),
            Some(paths),
            temp.path(),
            "parent-run",
        ))
        .expect("subflow outcome");

    assert_eq!(outcome.status, OutcomeStatus::Completed, "{outcome:?}");
    assert_eq!(outcome.decision, "done", "the child's result maps to done");
    assert!(child_handler_observed_map.load(Ordering::SeqCst));
}

/// Writes a child flow as a bundled flow: `dir` is `<root>/<name>`, and the
/// flow lands in `<root>/flows/<name>`, where `internal.bundled_root` =
/// `<root>` finds it.
fn write_child_flow(dir: &std::path::Path, source: Value) {
    let root = dir.parent().expect("bundled root");
    attractor_dsl::write_flow_dir(
        &root.join("flows").join(dir.file_name().expect("flow name")),
        &attractor_dsl::validated_flow(serde_json::from_value(source).expect("flow json"))
            .expect("valid flow"),
    )
    .expect("write flow");
}

fn child_flow_source() -> Value {
    json!({
        "schema_version": "2",
        "id": "child",
        "title": "Child",
        "nodes": {
            "start": {
                "kind": "start",
                "routes": { "next": { "to": "done", "description": "Finish." } }
            },
            "done": {
                "kind": "exit",
                "config": { "kind": "exit", "result": { "name": "completed", "outcome": "success" } }
            }
        }
    })
}

fn git_in(dir: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.email=contracts@example.com",
            "-c",
            "user.name=Contracts",
        ])
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn dynamic_workdir_subflow_node(child_workdir_from: &str) -> FlowNode {
    subflow_node(
        BTreeMap::new(),
        ManagerLoopConfig {
            max_cycles: Some(1),
            child_autostart: Some(true),
            child_workdir_from: Some(child_workdir_from.to_string()),
            ..ManagerLoopConfig::default()
        },
    )
}

fn execute_dynamic_workdir_subflow(
    node: FlowNode,
    run_workdir: &std::path::Path,
    bundled_root: &std::path::Path,
    context_entries: Vec<(String, Value)>,
) -> (Outcome, Option<std::path::PathBuf>) {
    let flow = flow_of("dynamic-workdir", vec![("run_child", node)]);
    let observed_workdir = Arc::new(std::sync::Mutex::new(None::<std::path::PathBuf>));
    let recorded = Arc::clone(&observed_workdir);
    let mut runner = RuntimeHandlerRunner::new();
    runner.set_child_run_launcher(move |request| {
        *recorded.lock().expect("record child workdir") = Some(request.child_workdir.clone());
        completed_child(&request.child_run_id)
    });
    let mut context = ContextMap::from([
        (
            "internal.run_workdir".to_string(),
            json!(run_workdir.to_string_lossy().to_string()),
        ),
        (
            "internal.bundled_root".to_string(),
            json!(bundled_root.to_string_lossy().to_string()),
        ),
    ]);
    context.extend(context_entries);
    let outcome = runner
        .execute(node_request(
            &flow,
            "run_child",
            context,
            None,
            run_workdir,
            "parent-run",
        ))
        .expect("subflow outcome");
    let observed = observed_workdir.lock().expect("read child workdir").clone();
    (outcome, observed)
}

#[test]
fn subflow_child_workdir_from_launches_child_in_context_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_child_flow(&temp.path().join("child"), child_flow_source());
    let workspace = temp.path().join("workspaces/run-123");
    fs::create_dir_all(&workspace).expect("create workspace dir");

    let (outcome, observed) = execute_dynamic_workdir_subflow(
        dynamic_workdir_subflow_node("context.workspace.path"),
        temp.path(),
        temp.path(),
        vec![(
            "context.workspace.path".to_string(),
            json!(workspace.to_string_lossy().to_string()),
        )],
    );

    assert_eq!(outcome.status, OutcomeStatus::Completed, "{outcome:?}");
    assert_eq!(outcome.decision, "done");
    assert_eq!(
        observed.expect("child launched"),
        fs::canonicalize(&workspace).expect("canonical workspace")
    );
}

#[test]
fn default_child_launch_inherits_parent_project_and_keeps_worktree() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = temp.path().join("project");
    let worktree = project.join("worktree");
    fs::create_dir_all(&worktree).expect("create worktree");
    // flow_ref resolves against the parent flow's directory, not child_workdir.
    write_child_flow(&project.join("child"), child_flow_source());
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let mut parent = RunRecord::new("parent-run", project.to_string_lossy());
    parent.flow_name = "Parent".to_string();
    let parent_paths = store
        .create_run(CreateRunRequest {
            record: parent,
            ..CreateRunRequest::default()
        })
        .expect("create parent");
    let flow = flow_of(
        "parent",
        vec![(
            "run_child",
            subflow_node(
                BTreeMap::new(),
                ManagerLoopConfig {
                    max_cycles: Some(1),
                    child_autostart: Some(true),
                    child_workdir: Some("worktree".to_string()),
                    ..ManagerLoopConfig::default()
                },
            ),
        )],
    );
    let outcome = RuntimeHandlerRunner::new()
        .execute(node_request(
            &flow,
            "run_child",
            ContextMap::from([
                ("internal.run_id".to_string(), json!("parent-run")),
                ("internal.root_run_id".to_string(), json!("parent-run")),
                ("internal.run_workdir".to_string(), json!(project)),
                ("internal.bundled_root".to_string(), json!(project)),
            ]),
            Some(parent_paths),
            &project,
            "parent-run",
        ))
        .expect("launch child");
    assert_eq!(outcome.status, OutcomeStatus::Completed, "{outcome:?}");
    assert_eq!(outcome.decision, "done");

    let child = store
        .list_run_records()
        .expect("list runs")
        .into_iter()
        .find(|record| record.parent_run_id.as_deref() == Some("parent-run"))
        .expect("child record");
    assert_eq!(child.project_path, project.to_string_lossy());
    assert_eq!(child.working_directory, worktree.to_string_lossy());
    assert_eq!(child.flow_name, "Child");
    assert_eq!(child.outcome_reason_code.as_deref(), Some("completed"));
    assert!(store
        .list_run_records()
        .expect("list runs")
        .into_iter()
        .filter(|record| record.project_path == project.to_string_lossy())
        .any(|record| record.run_id == child.run_id));
}

#[test]
fn subflow_child_workdir_from_accepts_linked_worktree_of_same_repository() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo dir");
    write_child_flow(&repo.join("child"), child_flow_source());
    git_in(&repo, &["init"]);
    git_in(&repo, &["add", "--all"]);
    git_in(&repo, &["commit", "-m", "initial"]);
    let worktree = temp.path().join("repo-worktrees/run-123");
    fs::create_dir_all(worktree.parent().expect("worktree parent")).expect("create worktree root");
    git_in(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "spark/run-123",
            worktree.to_str().expect("worktree path utf8"),
        ],
    );

    let (outcome, observed) = execute_dynamic_workdir_subflow(
        dynamic_workdir_subflow_node("context.workspace.path"),
        &repo,
        &repo,
        vec![(
            "context.workspace.path".to_string(),
            json!(worktree.to_string_lossy().to_string()),
        )],
    );

    assert_eq!(outcome.status, OutcomeStatus::Completed, "{outcome:?}");
    assert_eq!(
        observed.expect("child launched"),
        fs::canonicalize(&worktree).expect("canonical worktree")
    );
}

#[test]
fn subflow_child_workdir_from_rejects_directory_outside_launch_repository() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo dir");
    write_child_flow(&repo.join("child"), child_flow_source());
    git_in(&repo, &["init"]);
    let elsewhere = temp.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).expect("create outside dir");
    git_in(&elsewhere, &["init"]);

    let (outcome, observed) = execute_dynamic_workdir_subflow(
        dynamic_workdir_subflow_node("context.workspace.path"),
        &repo,
        &repo,
        vec![(
            "context.workspace.path".to_string(),
            json!(elsewhere.to_string_lossy().to_string()),
        )],
    );

    assert_eq!(outcome.status, OutcomeStatus::Error);
    assert!(
        outcome.failure_reason.contains("child_workdir_from"),
        "{}",
        outcome.failure_reason
    );
    assert!(observed.is_none(), "child must not launch");
}

#[test]
fn subflow_child_workdir_from_without_context_value_fails_before_launch() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_child_flow(&temp.path().join("child"), child_flow_source());

    let (outcome, observed) = execute_dynamic_workdir_subflow(
        dynamic_workdir_subflow_node("context.workspace.path"),
        temp.path(),
        temp.path(),
        Vec::new(),
    );

    assert_eq!(outcome.status, OutcomeStatus::Error);
    assert!(
        outcome
            .failure_reason
            .contains("did not resolve to a non-empty string"),
        "{}",
        outcome.failure_reason
    );
    assert!(observed.is_none(), "child must not launch");
}

#[test]
fn subflow_declaring_both_child_workdir_sources_fails() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_child_flow(&temp.path().join("child"), child_flow_source());
    let mut node = dynamic_workdir_subflow_node("context.workspace.path");
    if let Some(manager) = node.manager.as_mut() {
        manager.child_workdir = Some(".".to_string());
    }

    let (outcome, observed) = execute_dynamic_workdir_subflow(
        node,
        temp.path(),
        temp.path(),
        vec![(
            "context.workspace.path".to_string(),
            json!(temp.path().to_string_lossy().to_string()),
        )],
    );

    assert_eq!(outcome.status, OutcomeStatus::Error);
    assert!(
        outcome
            .failure_reason
            .contains("both manager.child_workdir and manager.child_workdir_from"),
        "{}",
        outcome.failure_reason
    );
    assert!(observed.is_none(), "child must not launch");
}

// ---------------------------------------------------------------------------
// Tool adapters and bindings
// ---------------------------------------------------------------------------

fn tool_with_adapter(
    command: &str,
    exit_codes: &[(&str, &str)],
    decision_field: Option<&str>,
    decisions: &[(&str, &str)],
) -> FlowNode {
    FlowNode {
        kind: NodeKind::Tool,
        config: Some(NodeConfig::Tool {
            command: command.to_string(),
            env_map: BTreeMap::new(),
            output_map: BTreeMap::new(),
            exit_codes: exit_codes
                .iter()
                .map(|(code, decision)| (code.to_string(), decision.to_string()))
                .collect(),
            decision_field: decision_field.map(str::to_string),
        }),
        routes: routes(decisions),
        ..FlowNode::default()
    }
}

fn tool_node(
    command: &str,
    env_map: BTreeMap<String, String>,
    output_map: BTreeMap<String, String>,
) -> FlowNode {
    let mut node = tool_with_adapter(command, &[("0", "done")], None, &[("done", "run_tool")]);
    if let Some(NodeConfig::Tool {
        env_map: env,
        output_map: output,
        ..
    }) = node.config.as_mut()
    {
        *env = env_map;
        *output = output_map;
    }
    node
}

fn execute_tool_node(
    node: FlowNode,
    run_workdir: &std::path::Path,
    context_entries: Vec<(String, Value)>,
) -> Outcome {
    let flow = flow_of("tool-bindings", vec![("run_tool", node)]);
    let mut context = ContextMap::from([(
        "internal.run_workdir".to_string(),
        json!(run_workdir.to_string_lossy().to_string()),
    )]);
    context.extend(context_entries);
    RuntimeHandlerRunner::new()
        .execute(node_request(
            &flow,
            "run_tool",
            context,
            None,
            run_workdir,
            "tool-run",
        ))
        .expect("tool outcome")
}

#[test]
fn tool_env_map_binds_context_values_as_environment_variables() {
    let temp = tempfile::tempdir().expect("tempdir");
    let outcome = execute_tool_node(
        tool_node(
            r#"printf '%s %s' "$WORKSPACE_PATH" "$RETRY_COUNT""#,
            [
                (
                    "WORKSPACE_PATH".to_string(),
                    "context.workspace.path".to_string(),
                ),
                ("RETRY_COUNT".to_string(), "context.retry_count".to_string()),
            ]
            .into_iter()
            .collect(),
            BTreeMap::new(),
        ),
        temp.path(),
        vec![
            (
                "context.workspace.path".to_string(),
                json!("/workspaces/run-9"),
            ),
            ("context.retry_count".to_string(), json!(3)),
        ],
    );

    assert_eq!(outcome.status, OutcomeStatus::Completed, "{outcome:?}");
    assert_eq!(outcome.decision, "done");
    assert_eq!(outcome.notes, "/workspaces/run-9 3");
}

#[test]
fn tool_env_map_with_missing_context_value_fails_before_execution() {
    let temp = tempfile::tempdir().expect("tempdir");
    let outcome = execute_tool_node(
        tool_node(
            "touch executed-anyway",
            [(
                "WORKSPACE_PATH".to_string(),
                "context.workspace.path".to_string(),
            )]
            .into_iter()
            .collect(),
            BTreeMap::new(),
        ),
        temp.path(),
        Vec::new(),
    );

    assert_eq!(outcome.status, OutcomeStatus::Error);
    assert!(
        outcome
            .failure_reason
            .contains("context.workspace.path did not resolve"),
        "{}",
        outcome.failure_reason
    );
    assert!(
        !temp.path().join("executed-anyway").exists(),
        "command must not run when env bindings are unresolved"
    );
}

#[test]
fn tool_env_map_rejects_invalid_environment_variable_names() {
    let temp = tempfile::tempdir().expect("tempdir");
    let outcome = execute_tool_node(
        tool_node(
            "touch executed-anyway",
            [("BAD-NAME".to_string(), "context.value".to_string())]
                .into_iter()
                .collect(),
            BTreeMap::new(),
        ),
        temp.path(),
        vec![("context.value".to_string(), json!("x"))],
    );

    assert_eq!(outcome.status, OutcomeStatus::Error);
    assert!(
        outcome.failure_reason.contains("BAD-NAME"),
        "{}",
        outcome.failure_reason
    );
    assert!(!temp.path().join("executed-anyway").exists());
}

#[test]
fn tool_output_map_parses_json_stdout_into_context_updates() {
    let temp = tempfile::tempdir().expect("tempdir");
    let outcome = execute_tool_node(
        tool_node(
            r#"printf '{"worktree_path":"/w/run-9","git":{"branch":"spark/run-9"},"created":true}'"#,
            BTreeMap::new(),
            [
                (
                    "context.workspace.path".to_string(),
                    "worktree_path".to_string(),
                ),
                (
                    "context.workspace.branch".to_string(),
                    "git.branch".to_string(),
                ),
                (
                    "context.workspace.created".to_string(),
                    "created".to_string(),
                ),
            ]
            .into_iter()
            .collect(),
        ),
        temp.path(),
        Vec::new(),
    );

    assert_eq!(outcome.status, OutcomeStatus::Completed, "{outcome:?}");
    assert_eq!(
        outcome.context_updates.get("context.workspace.path"),
        Some(&json!("/w/run-9"))
    );
    assert_eq!(
        outcome.context_updates.get("context.workspace.branch"),
        Some(&json!("spark/run-9"))
    );
    assert_eq!(
        outcome.context_updates.get("context.workspace.created"),
        Some(&json!(true))
    );
    assert_eq!(
        outcome.context_updates.get("context.tool.exit_code"),
        Some(&json!(0))
    );
}

#[test]
fn tool_output_map_with_non_json_stdout_fails() {
    let temp = tempfile::tempdir().expect("tempdir");
    let outcome = execute_tool_node(
        tool_node(
            "printf 'not json'",
            BTreeMap::new(),
            [(
                "context.workspace.path".to_string(),
                "worktree_path".to_string(),
            )]
            .into_iter()
            .collect(),
        ),
        temp.path(),
        Vec::new(),
    );

    assert_eq!(outcome.status, OutcomeStatus::Error);
    assert!(
        outcome.failure_reason.contains("JSON"),
        "{}",
        outcome.failure_reason
    );
    assert_eq!(
        outcome.context_updates.get("context.tool.output"),
        Some(&json!("not json")),
        "raw stdout stays observable on mapping failure"
    );
}

#[test]
fn tool_output_map_with_missing_json_path_fails() {
    let temp = tempfile::tempdir().expect("tempdir");
    let outcome = execute_tool_node(
        tool_node(
            r#"printf '{"other":1}'"#,
            BTreeMap::new(),
            [(
                "context.workspace.path".to_string(),
                "worktree_path".to_string(),
            )]
            .into_iter()
            .collect(),
        ),
        temp.path(),
        Vec::new(),
    );

    assert_eq!(outcome.status, OutcomeStatus::Error);
    assert!(
        outcome.failure_reason.contains("worktree_path"),
        "{}",
        outcome.failure_reason
    );
}

/// A tool's one adapter is its only decision source: mapped exit codes and
/// a declared JSON decision field complete; unmapped codes, nonzero JSON
/// exits and undeclared decisions are execution errors with their evidence.
#[test]
fn tool_adapters_select_declared_decisions_and_reject_everything_else() {
    let temp = tempfile::tempdir().expect("tempdir");
    let exit_routes = [("passed", "run_tool"), ("rejected", "run_tool")];
    let exit_map = [("0", "passed"), ("1", "rejected")];
    for (command, expected) in [
        ("true", Ok("passed")),
        ("exit 1", Ok("rejected")),
        (
            "echo setup broke >&2; exit 2",
            Err("tool exited with unmapped code 2; stderr: setup broke"),
        ),
    ] {
        let outcome = execute_tool_node(
            tool_with_adapter(command, &exit_map, None, &exit_routes),
            temp.path(),
            Vec::new(),
        );
        match expected {
            Ok(decision) => {
                assert_eq!(
                    outcome.status,
                    OutcomeStatus::Completed,
                    "{command}: {outcome:?}"
                );
                assert_eq!(outcome.decision, decision, "{command}");
            }
            Err(reason) => {
                assert_eq!(outcome.status, OutcomeStatus::Error, "{command}");
                assert_eq!(outcome.decision, "", "{command}");
                assert_eq!(outcome.failure_reason, reason, "{command}");
                assert_eq!(outcome.failure_kind, Some(FailureKind::Runtime));
            }
        }
    }
    for (command, expected, kind) in [
        (r#"printf '{"decision":"passed"}'"#, Ok("passed"), None),
        (
            r#"printf '{"decision":"maybe"}'"#,
            Err("tool returned undeclared decision \"maybe\""),
            Some(FailureKind::Contract),
        ),
        (
            r#"printf '{"decision":"passed"}'; exit 3"#,
            Err("tool exited with code 3 instead of 0 with a JSON decision decision"),
            Some(FailureKind::Runtime),
        ),
        (
            "printf 'passed'",
            Err("tool stdout is not one JSON object with a decision decision"),
            Some(FailureKind::Runtime),
        ),
    ] {
        let outcome = execute_tool_node(
            tool_with_adapter(command, &[], Some("decision"), &exit_routes),
            temp.path(),
            Vec::new(),
        );
        match expected {
            Ok(decision) => {
                assert_eq!(
                    outcome.status,
                    OutcomeStatus::Completed,
                    "{command}: {outcome:?}"
                );
                assert_eq!(outcome.decision, decision, "{command}");
            }
            Err(reason) => {
                assert_eq!(outcome.status, OutcomeStatus::Error, "{command}");
                assert_eq!(outcome.failure_reason, reason, "{command}");
                assert_eq!(outcome.failure_kind, kind, "{command}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Switch
// ---------------------------------------------------------------------------

fn switch_node(on: &str, cases: &[(&str, &str)], otherwise: &str) -> FlowNode {
    let mut decisions = cases
        .iter()
        .map(|(_, decision)| *decision)
        .collect::<Vec<_>>();
    decisions.push(otherwise);
    decisions.sort();
    decisions.dedup();
    FlowNode {
        kind: NodeKind::Switch,
        config: Some(NodeConfig::Switch {
            on: on.to_string(),
            cases: cases
                .iter()
                .map(|(key, decision)| (key.to_string(), decision.to_string()))
                .collect(),
            otherwise: otherwise.to_string(),
        }),
        contracts: Some(NodeContracts {
            reads_context: vec![on.to_string()],
            ..NodeContracts::default()
        }),
        routes: decisions
            .into_iter()
            .map(|decision| (decision.to_string(), route("check")))
            .collect(),
        ..FlowNode::default()
    }
}

/// A switch compares one context value with its JSON-scalar case keys by
/// exact equality, without coercion; only a supported scalar matching no
/// case takes `otherwise`, and missing or unsupported values are errors.
#[test]
fn switch_compares_exact_scalars_without_coercion() {
    let flow = flow_of(
        "switch-scalars",
        vec![(
            "check",
            switch_node(
                "context.value",
                &[("0", "clean"), ("true", "flagged"), ("\"ready\"", "ready")],
                "other",
            ),
        )],
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let mut runner = RuntimeHandlerRunner::new();
    for (value, expected) in [
        (Some(json!(0)), Ok("clean")),
        (Some(json!(0.0)), Ok("clean")),
        (Some(json!("0")), Ok("other")),
        (Some(json!(1)), Ok("other")),
        (Some(json!(true)), Ok("flagged")),
        (Some(json!("true")), Ok("other")),
        (Some(json!(false)), Ok("other")),
        (Some(json!("ready")), Ok("ready")),
        (
            Some(json!(null)),
            Err("null is not a supported switch value"),
        ),
        (
            Some(json!([0])),
            Err("arrays are not supported switch values"),
        ),
        (
            None,
            Err("switch key context.value is missing from the context"),
        ),
    ] {
        let context = value
            .clone()
            .map(|value| ContextMap::from([("context.value".to_string(), value)]))
            .unwrap_or_default();
        let outcome = runner
            .execute(node_request(
                &flow,
                "check",
                context,
                None,
                temp.path(),
                "switch-run",
            ))
            .expect("switch outcome");
        match expected {
            Ok(decision) => {
                assert_eq!(outcome.status, OutcomeStatus::Completed, "{value:?}");
                assert_eq!(outcome.decision, decision, "{value:?}");
            }
            Err(reason) => {
                assert_eq!(outcome.status, OutcomeStatus::Error, "{value:?}");
                assert!(
                    outcome.failure_reason.contains(reason),
                    "{value:?}: {}",
                    outcome.failure_reason
                );
                assert_eq!(outcome.failure_kind, Some(FailureKind::Contract));
            }
        }
    }
}

/// `start -> implement -> check`; the switch reads implement's verdict:
/// `"fix"` loops back to implement (three visits), anything else ships.
fn switch_loop_flow() -> FlowDefinition {
    let mut check = switch_node("context.review.verdict", &[("\"fix\"", "again")], "ship");
    check.routes = routes(&[("again", "implement"), ("ship", "done")]);
    flow_of(
        "switch-loop",
        vec![
            ("start", start_node("implement")),
            (
                "implement",
                FlowNode {
                    contracts: Some(NodeContracts {
                        writes_context: vec!["context.review.verdict".to_string()],
                        response: Some("decision_envelope".to_string()),
                        ..NodeContracts::default()
                    }),
                    runtime: Some(NodeRuntimeConfig {
                        max_visits: Some(3),
                        ..NodeRuntimeConfig::default()
                    }),
                    ..agent("implement", &[("done", "check")])
                },
            ),
            ("check", check),
            ("done", success_exit()),
        ],
    )
}

/// A runner whose agent completes `done`, writing `verdict` when given.
fn verdict_runner(
    verdict: Option<&'static str>,
) -> (RuntimeHandlerRunner, Arc<std::sync::Mutex<Vec<String>>>) {
    let calls = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = calls.clone();
    let mut runner = RuntimeHandlerRunner::new();
    runner.register_handler_fn(HANDLER_CODERGEN, move |runtime| {
        seen.lock().unwrap().push(runtime.node_id.clone());
        Ok(Outcome {
            context_updates: verdict
                .map(|verdict| {
                    ContextMap::from([("context.review.verdict".to_string(), json!(verdict))])
                })
                .unwrap_or_default(),
            ..Outcome::completed("done")
        })
    });
    (runner, calls)
}

/// A switch dispatches like any node: its selected decision's exhausted
/// target fails the run (never `otherwise`), `otherwise` reaches the exit,
/// and a missing key fails the run instead of selecting anything.
#[test]
fn switch_dispatch_uses_exact_lookup_and_visit_limits() {
    for (verdict, status, reason, implement_runs) in [
        (
            Some("fix"),
            "failed",
            visit_limit("check", "again", "implement", 3, 3),
            3,
        ),
        (Some("ship"), "completed", String::new(), 1),
        (
            None,
            "failed",
            "switch key context.review.verdict is missing from the context".to_string(),
            1,
        ),
    ] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = RunStore::for_runs_dir(temp.path().join("runs"));
        let (runner, calls) = verdict_runner(verdict);
        let result = PipelineExecutor::new(runner)
            .execute(execute_request(
                &store,
                "run-switch",
                temp.path(),
                switch_loop_flow(),
                None,
                Default::default(),
            ))
            .expect("execute");
        assert_eq!(
            result.status, status,
            "{verdict:?}: {}",
            result.failure_reason
        );
        assert_eq!(calls.lock().unwrap().len(), implement_runs, "{verdict:?}");
        if status == "failed" {
            assert_eq!(result.failure_reason, reason, "{verdict:?}");
            assert!(!result.route_trace.contains(&"done".to_string()));
            assert_durably_failed(&store, "run-switch", &reason);
        } else {
            assert_eq!(result.current_node, "done");
            assert_eq!(result.node_outcomes["check"].decision, "ship");
        }
    }
}

/// Recovery of a completed switch visit takes its stored decision; it does
/// not evaluate the switch again against the restored context.
#[test]
fn resume_of_a_completed_switch_visit_takes_its_stored_decision() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let (runner, _) = verdict_runner(Some("ship"));
    // Polls: before/after start, before/after implement, before/after check.
    let mut polls = 0;
    let paused = PipelineExecutor::with_control(runner, move || {
        polls += 1;
        (polls == 6).then_some(attractor_runtime::ExecutionControlAction::Pause)
    })
    .execute(execute_request(
        &store,
        "run-switch-resume",
        temp.path(),
        switch_loop_flow(),
        None,
        Default::default(),
    ))
    .expect("execute");
    assert_eq!(paused.status, "paused");
    let meta = store.read_run_meta("run-switch-resume").unwrap().unwrap();
    let mut checkpoint = meta.checkpoint.unwrap();
    assert_eq!(checkpoint.current_node, "check");
    checkpoint
        .context
        .insert("context.review.verdict".to_string(), json!("fix"));

    let (runner, calls) = verdict_runner(Some("fix"));
    let resumed = PipelineExecutor::new(runner)
        .execute(execute_request(
            &store,
            "run-switch-resume",
            temp.path(),
            switch_loop_flow(),
            None,
            attractor_runtime::ExecutionStart::Resume {
                paths: meta.paths,
                checkpoint,
            },
        ))
        .expect("resume");
    assert_eq!(resumed.status, "completed", "{}", resumed.failure_reason);
    assert_eq!(resumed.current_node, "done");
    assert!(calls.lock().unwrap().is_empty());
    let events = events_of(&store, "run-switch-resume");
    assert_eq!(
        events
            .iter()
            .filter(
                |event| event.event_type == "StageStarted" && event.payload["node_id"] == "check"
            )
            .count(),
        1,
        "the switch is not evaluated again"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "StageTransition"
                && event.payload["node_id"] == "check")
            .map(|event| event.payload["decision"].clone())
            .collect::<Vec<_>>(),
        vec![json!("ship")],
        "the stored transition is taken exactly once"
    );
}

// ---------------------------------------------------------------------------
// Manager-loop child invocation reset (CR-2026-0102)
// ---------------------------------------------------------------------------

fn reentry_parent_flow() -> FlowDefinition {
    flow_of(
        "parent_reentry",
        vec![(
            "run_child",
            subflow_node(
                Default::default(),
                ManagerLoopConfig {
                    max_cycles: Some(1),
                    child_autostart: Some(true),
                    ..ManagerLoopConfig::default()
                },
            ),
        )],
    )
}

fn reentry_context(temp: &std::path::Path, run_id: &str) -> ContextMap {
    ContextMap::from([
        ("internal.run_id".to_string(), json!(run_id)),
        ("internal.root_run_id".to_string(), json!(run_id)),
        (
            "internal.bundled_root".to_string(),
            json!(temp.to_string_lossy().to_string()),
        ),
        (
            "internal.run_workdir".to_string(),
            json!(temp.to_string_lossy().to_string()),
        ),
    ])
}

fn with_outcome_updates(mut base: ContextMap, outcome: &Outcome) -> ContextMap {
    for (key, value) in outcome.context_updates.iter() {
        base.insert(key.clone(), value.clone());
    }
    base
}

fn execute_reentry_node(
    runner: &mut RuntimeHandlerRunner,
    flow: &FlowDefinition,
    run_id: &str,
    paths: &RunRootPaths,
    temp: &std::path::Path,
    context: ContextMap,
) -> Outcome {
    runner
        .execute(node_request(
            flow,
            "run_child",
            context,
            Some(paths.clone()),
            temp,
            run_id,
        ))
        .expect("subflow outcome")
}

fn linked_child_id(outcome: &Outcome) -> String {
    outcome
        .context_updates
        .get("context.stack.child.run_id")
        .and_then(|value| value.as_str())
        .expect("linked child id")
        .to_string()
}

/// Re-entry after the node already completed on its child (the executor
/// checkpointed past it) is a new attempt: the stale completed child is
/// history and a fresh invocation launches with an advanced index.
#[test]
fn manager_reentry_after_acknowledged_completion_launches_fresh_child() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_child_flow(&temp.path().join("child"), child_flow_source());
    let flow = reentry_parent_flow();
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let paths = create_run(&store, "parent-run", temp.path());
    let mut runner = RuntimeHandlerRunner::new();

    let first = execute_reentry_node(
        &mut runner,
        &flow,
        "parent-run",
        &paths,
        temp.path(),
        reentry_context(temp.path(), "parent-run"),
    );
    assert_eq!(first.status, OutcomeStatus::Completed, "{first:?}");
    assert_eq!(first.decision, "done");
    let first_child = linked_child_id(&first);

    // The executor completed this node and checkpointed past it, so the
    // child's completion is acknowledged.
    store
        .save_checkpoint(
            &paths,
            &attractor_core::CheckpointState {
                timestamp: attractor_runtime::events::utc_timestamp(),
                current_node: "evaluate".to_string(),
                completed_nodes: vec!["run_child".to_string()],
                context: Default::default(),
                retry_counts: Default::default(),
                visit_counts: Default::default(),
                logs: Vec::new(),
            },
            Default::default(),
        )
        .expect("checkpoint past the node");

    let second = execute_reentry_node(
        &mut runner,
        &flow,
        "parent-run",
        &paths,
        temp.path(),
        with_outcome_updates(reentry_context(temp.path(), "parent-run"), &first),
    );
    assert_eq!(second.status, OutcomeStatus::Completed, "{second:?}");
    let second_child = linked_child_id(&second);
    assert_ne!(
        first_child, second_child,
        "re-entry must launch a fresh child"
    );

    let children = store
        .list_child_run_bundles("parent-run")
        .expect("child bundles");
    let mut indices: Vec<u64> = children
        .iter()
        .filter_map(|bundle| bundle.record.as_ref()?.child_invocation_index)
        .collect();
    indices.sort_unstable();
    assert_eq!(indices, vec![1, 2]);

    let events = store.read_raw_events(&paths).expect("events");
    assert!(events.iter().any(|event| {
        event.event_type == "ChildInvocationReset"
            && event
                .payload
                .get("prior_child_run_id")
                .and_then(|value| value.as_str())
                == Some(first_child.as_str())
    }));
}

/// A completed child the executor never checkpointed past is the one durable,
/// unacknowledged invocation a dead executor left behind: crash recovery
/// adopts it and must not manufacture a sibling.
#[test]
fn manager_reentry_with_unacknowledged_completed_child_adopts_it() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_child_flow(&temp.path().join("child"), child_flow_source());
    let flow = reentry_parent_flow();
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let paths = create_run(&store, "parent-run", temp.path());
    let mut runner = RuntimeHandlerRunner::new();

    let first = execute_reentry_node(
        &mut runner,
        &flow,
        "parent-run",
        &paths,
        temp.path(),
        reentry_context(temp.path(), "parent-run"),
    );
    let first_child = linked_child_id(&first);

    let second = execute_reentry_node(
        &mut runner,
        &flow,
        "parent-run",
        &paths,
        temp.path(),
        with_outcome_updates(reentry_context(temp.path(), "parent-run"), &first),
    );
    assert_eq!(second.status, OutcomeStatus::Completed, "{second:?}");
    assert_eq!(second.decision, "done");
    // The adopted child is unchanged, so it is absent from the context diff;
    // it must never be replaced by a different child.
    let relinked = second
        .context_updates
        .get("context.stack.child.run_id")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    assert!(
        relinked.as_deref().is_none_or(|id| id == first_child),
        "adopted child was replaced: {relinked:?}"
    );
    assert_eq!(
        store
            .list_child_run_bundles("parent-run")
            .expect("child bundles")
            .len(),
        1
    );
    let events = store.read_raw_events(&paths).expect("events");
    assert!(!events
        .iter()
        .any(|event| event.event_type == "ChildInvocationReset"));
}

/// A continuation restores another run's context, including its completed
/// child link. That child belongs to the source run; the continuation must
/// invoke afresh rather than adopt it.
#[test]
fn manager_reentry_with_foreign_completed_child_launches_fresh_child() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_child_flow(&temp.path().join("child"), child_flow_source());
    let flow = reentry_parent_flow();
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let source_paths = create_run(&store, "source-run", temp.path());
    let mut runner = RuntimeHandlerRunner::new();
    let source = execute_reentry_node(
        &mut runner,
        &flow,
        "source-run",
        &source_paths,
        temp.path(),
        reentry_context(temp.path(), "source-run"),
    );
    let source_child = linked_child_id(&source);

    let continuation_paths = create_run(&store, "continuation-run", temp.path());
    let continued = execute_reentry_node(
        &mut runner,
        &flow,
        "continuation-run",
        &continuation_paths,
        temp.path(),
        with_outcome_updates(reentry_context(temp.path(), "continuation-run"), &source),
    );
    assert_eq!(continued.status, OutcomeStatus::Completed, "{continued:?}");
    assert_ne!(linked_child_id(&continued), source_child);
    assert_eq!(
        store
            .list_child_run_bundles("continuation-run")
            .expect("continuation children")
            .len(),
        1
    );
}

// ---------------------------------------------------------------------------
// Dispatch: exact decision lookup, then the selected target's visit limit
// ---------------------------------------------------------------------------

/// `start -> implement -> evaluate`, with `implement` limited to three visits
/// and `evaluate`'s routes given by the caller.
fn visit_loop_flow(evaluate_routes: &[(&str, &str)]) -> FlowDefinition {
    flow_of(
        "visit-loop",
        vec![
            ("start", start_node("implement")),
            (
                "implement",
                FlowNode {
                    runtime: Some(NodeRuntimeConfig {
                        max_visits: Some(3),
                        ..NodeRuntimeConfig::default()
                    }),
                    ..agent("implement", &[("done", "evaluate")])
                },
            ),
            ("evaluate", agent("evaluate", evaluate_routes)),
            ("done", success_exit()),
        ],
    )
}

/// Runs `flow` with `evaluate` answering `evaluate(n)` on its n-th call and
/// every other node completing `step`; returns the result and the node ids
/// executed, in order.
fn run_visit_loop(
    store: &RunStore,
    run_id: &str,
    workdir: &std::path::Path,
    flow: FlowDefinition,
    evaluate: impl Fn(usize) -> Outcome + 'static,
    max_steps: Option<usize>,
    start: attractor_runtime::ExecutionStart,
) -> (attractor_runtime::PipelineExecutionResult, Vec<String>) {
    let calls = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = calls.clone();
    let mut executor = PipelineExecutor::new(move |request: NodeExecutionRequest| {
        let mut seen = seen.lock().unwrap();
        seen.push(request.node_id.clone());
        if request.node_id == "evaluate" {
            let evaluations = seen.iter().filter(|node| *node == "evaluate").count();
            return Ok(evaluate(evaluations));
        }
        Ok(step(&request.node_id))
    });
    let result = executor
        .execute(execute_request(
            store, run_id, workdir, flow, max_steps, start,
        ))
        .expect("execute visit loop");
    let calls = calls.lock().unwrap().clone();
    (result, calls)
}

fn again(_: usize) -> Outcome {
    Outcome::completed("again")
}

fn implement_runs(calls: &[String]) -> usize {
    calls
        .iter()
        .filter(|node| node.as_str() == "implement")
        .count()
}

/// A loop fails at its target's visit limit; the reason names the selecting
/// node, its decision, the target and the count.
#[test]
fn loop_without_another_route_fails_when_its_target_used_all_visits() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let (result, calls) = run_visit_loop(
        &store,
        "run-visit-loop",
        temp.path(),
        visit_loop_flow(&[("again", "implement")]),
        again,
        None,
        Default::default(),
    );
    let reason = visit_limit("evaluate", "again", "implement", 3, 3);
    assert_eq!(result.status, "failed");
    assert_eq!(result.failure_reason, reason);
    assert_eq!(implement_runs(&calls), 3);
    assert_eq!(calls.iter().filter(|node| *node == "evaluate").count(), 3);
    assert_durably_failed(&store, "run-visit-loop", &reason);
}

/// A decision selecting an exhausted target fails the run with
/// visit_limit_reached even though another route is declared: exhaustion
/// never changes the decision. The request still offers the exhausted
/// decision, marked as such, and the decision survives in the node's
/// durable status.
#[test]
fn exhausted_loop_decision_fails_instead_of_taking_another_route() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let flow = visit_loop_flow(&[("again", "implement"), ("done", "done")]);
    let unavailable = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = unavailable.clone();
    let mut executor = PipelineExecutor::new(move |request: NodeExecutionRequest| {
        if request.node_id == "evaluate" {
            seen.lock().unwrap().push(
                request
                    .context
                    .get(attractor_core::RUNTIME_UNAVAILABLE_ROUTES_KEY)
                    .cloned(),
            );
            return Ok(Outcome {
                notes: "still not good enough".to_string(),
                ..Outcome::completed("again")
            });
        }
        Ok(step(&request.node_id))
    });
    let result = executor
        .execute(execute_request(
            &store,
            "run-visit-done",
            temp.path(),
            flow,
            None,
            Default::default(),
        ))
        .expect("execute");
    let reason = visit_limit("evaluate", "again", "implement", 3, 3);
    assert_eq!(result.status, "failed");
    assert_eq!(result.failure_reason, reason);
    assert_eq!(result.current_node, "evaluate");
    assert!(!result.route_trace.contains(&"done".to_string()));
    assert_eq!(
        *unavailable.lock().unwrap(),
        vec![None, None, Some(json!(["implement"]))]
    );
    assert_durably_failed(&store, "run-visit-done", &reason);

    let status = latest_status(&store, "run-visit-done", "evaluate");
    assert_eq!(status["status"], "completed");
    assert_eq!(status["decision"], "again");
    assert_eq!(status["notes"], "still not good enough");
    let events = events_of(&store, "run-visit-done");
    assert!(!events
        .iter()
        .any(|event| event.event_type == "StageTransition" && event.payload["target"] == "done"));
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "StageCompleted"
                && event.payload["node_id"] == "evaluate")
            .last()
            .unwrap()
            .payload["decision"],
        "again"
    );
}

/// An execution error never selects a route, whatever the node declares:
/// it fails the run with its own reason instead of looping or finishing.
#[test]
fn execution_error_fails_without_jumping_to_any_route() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let (result, calls) = run_visit_loop(
        &store,
        "run-error-no-jump",
        temp.path(),
        visit_loop_flow(&[("again", "implement"), ("done", "done")]),
        |_| Outcome::error(FailureKind::Runtime, "not good enough", Some(false)),
        None,
        Default::default(),
    );
    assert_eq!(result.status, "failed");
    assert_eq!(result.failure_reason, "not good enough");
    assert_eq!(result.current_node, "evaluate");
    assert_eq!(calls, vec!["start", "implement", "evaluate"]);
    assert_durably_failed(&store, "run-error-no-jump", "not good enough");
    let status = latest_status(&store, "run-error-no-jump", "evaluate");
    assert_eq!(status["status"], "error");
    assert_eq!(status["decision"], "");
    assert!(!events_of(&store, "run-error-no-jump")
        .iter()
        .any(
            |event| event.event_type == "StageTransition" && event.payload["node_id"] == "evaluate"
        ));
}

/// The reached exit's declared result decides the run outcome: a blocked
/// decision to a failure exit completes the run with outcome failure and the
/// exit's result name, carrying the deciding node's evidence.
#[test]
fn reached_exit_declares_the_run_outcome_and_result() {
    for (decision, exit, outcome, code) in [
        ("done", "done", "success", "completed"),
        ("blocked", "failed", "failure", "blocked"),
    ] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = RunStore::for_runs_dir(temp.path().join("runs"));
        let mut flow = visit_loop_flow(&[("done", "done"), ("blocked", "failed")]);
        flow.nodes.insert(
            "failed".to_string(),
            exit_node("blocked", ExitOutcome::Failure),
        );
        let (result, _) = run_visit_loop(
            &store,
            "run-exit-result",
            temp.path(),
            flow,
            move |_| Outcome {
                notes: "credentials are missing".to_string(),
                ..Outcome::completed(decision)
            },
            None,
            Default::default(),
        );
        assert_eq!(
            result.status, "completed",
            "{decision}: {}",
            result.failure_reason
        );
        assert_eq!(result.current_node, exit);
        assert_eq!(result.outcome.as_deref(), Some(outcome));
        assert_eq!(result.outcome_reason_code.as_deref(), Some(code));
        let message = format!("evaluate chose {decision}: credentials are missing");
        assert_eq!(
            result.outcome_reason_message.as_deref(),
            Some(message.as_str())
        );
        let record = store
            .read_run_meta("run-exit-result")
            .unwrap()
            .unwrap()
            .record
            .unwrap();
        assert_eq!(record.status, "completed");
        assert_eq!(record.outcome.as_deref(), Some(outcome));
        assert_eq!(record.outcome_reason_code.as_deref(), Some(code));
        assert_eq!(
            record.outcome_reason_message.as_deref(),
            Some(message.as_str())
        );
        let completed = events_of(&store, "run-exit-result")
            .into_iter()
            .find(|event| event.event_type == "PipelineCompleted")
            .expect("PipelineCompleted");
        assert_eq!(completed.payload["outcome"], outcome);
        assert_eq!(completed.payload["outcome_reason_code"], code);
    }
}

/// A decision into an exhausted intermediate target fails with that target's
/// visit limit, naming the selecting node, not some later node.
#[test]
fn exhausted_intermediate_target_reports_the_visit_limit() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let mut flow = visit_loop_flow(&[("recheck", "check")]);
    flow.nodes.insert(
        "check".to_string(),
        FlowNode {
            runtime: Some(NodeRuntimeConfig {
                max_visits: Some(1),
                ..NodeRuntimeConfig::default()
            }),
            ..agent("check", &[("done", "implement")])
        },
    );
    let (result, calls) = run_visit_loop(
        &store,
        "run-visit-intermediate",
        temp.path(),
        flow,
        |_| Outcome::completed("recheck"),
        None,
        Default::default(),
    );
    let reason = visit_limit("evaluate", "recheck", "check", 1, 1);
    assert_eq!(result.status, "failed");
    assert_eq!(result.failure_reason, reason);
    assert_eq!(implement_runs(&calls), 2);
    assert_durably_failed(&store, "run-visit-intermediate", &reason);
}

/// The first selection of an exhausted target fails at once.
#[test]
fn exhausted_target_on_first_selection_reports_the_visit_limit() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let mut flow = visit_loop_flow(&[("again", "implement")]);
    flow.nodes.get_mut("implement").unwrap().runtime = Some(NodeRuntimeConfig {
        max_visits: Some(1),
        ..NodeRuntimeConfig::default()
    });
    let (result, calls) = run_visit_loop(
        &store,
        "run-visit-first",
        temp.path(),
        flow,
        again,
        None,
        Default::default(),
    );
    assert_eq!(result.status, "failed");
    assert_eq!(
        result.failure_reason,
        visit_limit("evaluate", "again", "implement", 1, 1)
    );
    assert_eq!(implement_runs(&calls), 1);
}

/// With an exhausted loop target beside a route to another agent or to an
/// exit, the exhausted decision still fails and the other target never runs.
#[test]
fn exhausted_decision_beside_another_target_never_reaches_it() {
    for (target, kind) in [("polish", NodeKind::AgentTask), ("done", NodeKind::Exit)] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = RunStore::for_runs_dir(temp.path().join("runs"));
        let mut flow = visit_loop_flow(&[("again", "implement"), ("finish", target)]);
        flow.nodes.get_mut("implement").unwrap().runtime = Some(NodeRuntimeConfig {
            max_visits: Some(1),
            ..NodeRuntimeConfig::default()
        });
        if kind == NodeKind::AgentTask {
            flow.nodes
                .insert(target.to_string(), agent(target, &[("done", "done")]));
        }
        let (result, calls) = run_visit_loop(
            &store,
            "run-visit-beside",
            temp.path(),
            flow,
            again,
            None,
            Default::default(),
        );
        assert_eq!(result.status, "failed", "{target}");
        assert_eq!(
            result.failure_reason,
            visit_limit("evaluate", "again", "implement", 1, 1)
        );
        assert_eq!(implement_runs(&calls), 1);
        assert!(!calls.iter().any(|node| node == target), "{target}");
        assert!(
            !result.route_trace.contains(&target.to_string()),
            "{target}"
        );
    }
}

/// Runs `flow` with `evaluate` answering `evaluate(n)` and pauses right
/// after evaluate's third execution completes; returns the resume start.
fn pause_after_third_evaluate(
    store: &RunStore,
    run_id: &str,
    workdir: &std::path::Path,
    flow: FlowDefinition,
    evaluate: impl Fn(usize) -> Outcome + 'static,
) -> attractor_runtime::ExecutionStart {
    let calls = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = calls.clone();
    let polled = calls.clone();
    let mut executor = PipelineExecutor::with_control(
        move |request: NodeExecutionRequest| {
            let mut seen = seen.lock().unwrap();
            seen.push(request.node_id.clone());
            if request.node_id == "evaluate" {
                let evaluations = seen.iter().filter(|node| *node == "evaluate").count();
                return Ok(evaluate(evaluations));
            }
            Ok(step(&request.node_id))
        },
        move || {
            let calls = polled.lock().unwrap();
            (calls.iter().filter(|node| *node == "evaluate").count() == 3)
                .then_some(attractor_runtime::ExecutionControlAction::Pause)
        },
    );
    let paused = executor
        .execute(execute_request(
            store,
            run_id,
            workdir,
            flow,
            None,
            Default::default(),
        ))
        .expect("execute");
    assert_eq!(paused.status, "paused");
    let meta = store.read_run_meta(run_id).unwrap().unwrap();
    let checkpoint = meta.checkpoint.unwrap();
    assert_eq!(checkpoint.current_node, "evaluate");
    assert_eq!(checkpoint.visit_counts["implement"], 3);
    attractor_runtime::ExecutionStart::Resume {
        paths: meta.paths,
        checkpoint,
    }
}

/// Recovery from a checkpoint taken after the current visit completed routes
/// onward without re-running it, and fails when that route is exhausted.
#[test]
fn resume_routing_into_an_exhausted_target_fails() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let flow = || visit_loop_flow(&[("again", "implement")]);
    let start = pause_after_third_evaluate(&store, "run-visit-resume", temp.path(), flow(), again);

    let (resumed, calls) = run_visit_loop(
        &store,
        "run-visit-resume",
        temp.path(),
        flow(),
        again,
        None,
        start,
    );
    let reason = visit_limit("evaluate", "again", "implement", 3, 3);
    assert_eq!(resumed.status, "failed");
    assert_eq!(resumed.failure_reason, reason);
    assert!(calls.is_empty(), "{calls:?}");
    let record = store
        .read_run_meta("run-visit-resume")
        .unwrap()
        .unwrap()
        .record
        .unwrap();
    assert_eq!(record.status, "failed");
    assert_eq!(record.last_error, resumed.failure_reason);
}

/// Recovery of a completed visit whose stored decision selects an exhausted
/// target fails like uninterrupted execution: it neither re-asks the node
/// (which would now choose `done`) nor takes the other route.
#[test]
fn resume_of_a_completed_visit_keeps_its_exhausted_decision() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let flow = || visit_loop_flow(&[("again", "implement"), ("done", "done")]);
    let start =
        pause_after_third_evaluate(&store, "run-visit-resume-keep", temp.path(), flow(), again);

    let (resumed, calls) = run_visit_loop(
        &store,
        "run-visit-resume-keep",
        temp.path(),
        flow(),
        |_| Outcome::completed("done"),
        None,
        start,
    );
    let reason = visit_limit("evaluate", "again", "implement", 3, 3);
    assert_eq!(resumed.status, "failed");
    assert_eq!(resumed.failure_reason, reason);
    assert!(calls.is_empty(), "{calls:?}");
    assert!(!resumed.route_trace.contains(&"done".to_string()));
    assert_durably_failed(&store, "run-visit-resume-keep", &reason);
}

/// Recovery of a completed visit takes its stored transition exactly once,
/// without executing the node again.
#[test]
fn resume_of_a_completed_visit_takes_its_stored_transition_once() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let flow = || visit_loop_flow(&[("again", "implement"), ("done", "done")]);
    let start = pause_after_third_evaluate(
        &store,
        "run-visit-resume-done",
        temp.path(),
        flow(),
        |call| Outcome::completed(if call == 3 { "done" } else { "again" }),
    );

    let (resumed, calls) = run_visit_loop(
        &store,
        "run-visit-resume-done",
        temp.path(),
        flow(),
        again,
        None,
        start,
    );
    assert_eq!(resumed.status, "completed", "{}", resumed.failure_reason);
    assert_eq!(resumed.current_node, "done");
    assert!(calls.is_empty(), "{calls:?}");
    let transitions = events_of(&store, "run-visit-resume-done")
        .into_iter()
        .filter(|event| event.event_type == "StageTransition" && event.payload["target"] == "done")
        .count();
    assert_eq!(transitions, 1);
}

/// Recovery from a checkpoint taken after routing into a node runs that
/// pending visit, even though an earlier visit to the node completed.
#[test]
fn resume_runs_a_pending_visit_to_a_previously_completed_node() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let flow = || visit_loop_flow(&[("again", "implement")]);
    // Six steps end routed into evaluate's third visit, not yet executed.
    let (paused, _) = run_visit_loop(
        &store,
        "run-visit-pending",
        temp.path(),
        flow(),
        again,
        Some(6),
        Default::default(),
    );
    assert_eq!(paused.status, "paused");
    let meta = store.read_run_meta("run-visit-pending").unwrap().unwrap();
    let checkpoint = meta.checkpoint.unwrap();
    assert_eq!(checkpoint.current_node, "evaluate");
    assert!(checkpoint.completed_nodes.contains(&"evaluate".to_string()));

    let (resumed, calls) = run_visit_loop(
        &store,
        "run-visit-pending",
        temp.path(),
        flow(),
        again,
        None,
        attractor_runtime::ExecutionStart::Resume {
            paths: meta.paths,
            checkpoint,
        },
    );
    assert_eq!(resumed.status, "failed");
    assert_eq!(
        resumed.failure_reason,
        visit_limit("evaluate", "again", "implement", 3, 3)
    );
    assert_eq!(calls, vec!["evaluate"]);
}

/// Visit counts are run-wide: neither resuming a paused or crashed run nor
/// continuing it as a new run grants the loop target fresh visits.
#[test]
fn visit_counts_survive_resume_and_continue() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let flow = || visit_loop_flow(&[("again", "implement")]);
    let read_checkpoint = |run_id: &str| {
        let meta = store.read_run_meta(run_id).unwrap().unwrap();
        (meta.paths, meta.checkpoint.unwrap())
    };
    let exhausted = visit_limit("evaluate", "again", "implement", 3, 3);

    // Pause after routing back into implement: two of its three visits used.
    // Resume, as crash recovery does, and continue both start from here.
    let (paused, calls) = run_visit_loop(
        &store,
        "run-visits",
        temp.path(),
        flow(),
        again,
        Some(3),
        Default::default(),
    );
    assert_eq!(paused.status, "paused");
    assert_eq!(implement_runs(&calls), 1);
    let (paths, checkpoint) = read_checkpoint("run-visits");
    assert_eq!(checkpoint.visit_counts["implement"], 2);

    let (resumed, calls) = run_visit_loop(
        &store,
        "run-visits",
        temp.path(),
        flow(),
        again,
        None,
        attractor_runtime::ExecutionStart::Resume {
            paths,
            checkpoint: checkpoint.clone(),
        },
    );
    assert_eq!(resumed.status, "failed");
    assert_eq!(resumed.failure_reason, exhausted);
    // The checkpointed second visit is pending: it runs, then the third.
    assert_eq!(
        calls,
        vec!["implement", "evaluate", "implement", "evaluate"]
    );

    // Continue from implement: its third visit is the continuation's start.
    let (continued, calls) = run_visit_loop(
        &store,
        "run-visits-continued",
        temp.path(),
        flow(),
        again,
        None,
        attractor_runtime::ExecutionStart::FromCheckpoint {
            start_node: "implement".to_string(),
            checkpoint,
            flow_files: None,
        },
    );
    assert_eq!(continued.status, "failed");
    assert_eq!(continued.failure_reason, exhausted);
    assert_eq!(calls, vec!["implement", "evaluate"]);
}

#[test]
fn max_entries_is_rejected_by_name() {
    let error = serde_json::from_value::<NodeRuntimeConfig>(json!({"max_entries": 3}))
        .expect_err("max_entries is not a field");
    assert!(error.to_string().contains("max_entries"), "{error}");
    let runtime = serde_json::from_value::<NodeRuntimeConfig>(json!({"max_visits": 3})).unwrap();
    assert_eq!(runtime.max_visits, Some(3));
}

#[test]
fn persisted_cancellation_while_final_node_is_blocked_wins_over_completion() {
    // Exercise every finalization caller: an undeclared decision, a reached
    // exit, and an execution error.
    for (with_exit, task_errors) in [(false, false), (true, false), (true, true)] {
        let temp = tempfile::tempdir().unwrap();
        let store = RunStore::for_runs_dir(temp.path());
        let mut flow = linear_flow(agent("blocked", &[]));
        if !with_exit {
            flow.nodes.remove("done");
            flow.nodes.get_mut("task").unwrap().routes.clear();
        }
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let executor_store = store.clone();
        let record = RunRecord::new("cancel-final-node", temp.path().to_string_lossy());
        let worker = std::thread::spawn(move || {
            // No control probe: completion must honor the persisted control channel itself.
            PipelineExecutor::new(move |request: NodeExecutionRequest| {
                if request.node_id == "task" {
                    entered_tx.send(()).unwrap();
                    release_rx
                        .recv_timeout(std::time::Duration::from_secs(30))
                        .unwrap();
                    if task_errors {
                        return Ok(Outcome::error(
                            FailureKind::Runtime,
                            "task failed",
                            Some(false),
                        ));
                    }
                }
                Ok(step(&request.node_id))
            })
            .execute(ExecuteRunRequest {
                store: executor_store,
                record,
                flow,
                flow_definition_json: None,
                launch_context: LaunchContext::empty(),
                runtime_context: ContextMap::new(),
                max_steps: None,
                start: Default::default(),
            })
        });
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(30))
            .unwrap();
        let accepted = attractor_runtime::RuntimeControls::new(store.clone())
            .request_cancel("cancel-final-node")
            .unwrap();
        assert_eq!(accepted.status, "cancel_requested");
        release_tx.send(()).unwrap();
        let result = worker.join().unwrap().unwrap();
        assert_eq!(
            result.status, "canceled",
            "with_exit={with_exit}, task_errors={task_errors}"
        );
        assert_eq!(result.failure_reason, "aborted_by_user");
        let paths = store.find_run_root("cancel-final-node").unwrap().unwrap();
        let record = store.read_run_record(&paths).unwrap().unwrap();
        assert_eq!(record.status, "canceled");
        assert!(record.ended_at.is_some());
        assert_eq!(
            store.read_result(&paths).unwrap().unwrap().status,
            "canceled"
        );
        let events = store.read_raw_events(&paths).unwrap();
        let terminal = events
            .iter()
            .filter(|event| event.event_type == "runtime")
            .last()
            .unwrap();
        assert_eq!(terminal.payload["status"], "canceled");
        assert!(events
            .iter()
            .any(|event| event.event_type == "PipelineFailed"
                && event.payload["error"] == "aborted_by_user"));
        assert!(!events
            .iter()
            .any(|event| event.event_type == "PipelineCompleted"));
    }
}

#[test]
fn subflow_output_map_copies_and_clears_parent_keys_when_the_child_ends() {
    for status in ["completed", "failed"] {
        let temp = tempfile::tempdir().expect("tempdir");
        write_child_flow(&temp.path().join("child"), child_flow_source());
        let mut node = subflow_node(
            BTreeMap::new(),
            ManagerLoopConfig {
                max_cycles: Some(1),
                child_autostart: Some(true),
                ..ManagerLoopConfig::default()
            },
        );
        if let Some(NodeConfig::Subflow { output_map, .. }) = node.config.as_mut() {
            *output_map = BTreeMap::from([
                ("context.child.a".to_string(), "context.a".to_string()),
                ("context.child.b".to_string(), "context.b".to_string()),
            ]);
        }
        node.contracts = Some(NodeContracts {
            writes_context: vec!["context.a".to_string(), "context.b".to_string()],
            ..NodeContracts::default()
        });
        let flow = flow_of("output-map", vec![("run_child", node.clone())]);
        let mut runner = RuntimeHandlerRunner::new();
        runner.set_child_run_launcher(move |request| ChildRunResult {
            context: ContextMap::from([("context.child.a".to_string(), json!("A"))]),
            failure_reason: "child broke".to_string(),
            ..if status == "completed" {
                completed_child(&request.child_run_id)
            } else {
                ChildRunResult {
                    run_id: request.child_run_id,
                    status: status.to_string(),
                    ..ChildRunResult::default()
                }
            }
        });
        let parent_context = ContextMap::from([
            ("context.b".to_string(), json!("stale")),
            (
                "internal.bundled_root".to_string(),
                json!(temp.path().to_string_lossy()),
            ),
        ]);
        let node_attrs =
            attractor_runtime::flow_runtime::node_attrs_for_handler("run_child", &node);
        let outcome = runner
            .execute(node_request(
                &flow,
                "run_child",
                parent_context.clone(),
                None,
                temp.path(),
                "parent-run",
            ))
            .expect("subflow outcome");
        assert_eq!(outcome.context_updates["context.a"], json!("A"), "{status}");
        assert_eq!(
            outcome.context_updates["context.b"],
            json!(null),
            "{status}"
        );
        // A completed child maps its result to a decision; a failed child is
        // an execution error, never a business result.
        if status == "completed" {
            assert_eq!(outcome.status, OutcomeStatus::Completed, "{outcome:?}");
            assert_eq!(outcome.decision, "done");
        } else {
            assert_eq!(outcome.status, OutcomeStatus::Error, "{outcome:?}");
            assert!(
                outcome.failure_reason.contains("child broke"),
                "{outcome:?}"
            );
        }

        // The parent's write contract admits them, and a null clears the key.
        let mut context = attractor_core::AttractorContext::from_map(parent_context).unwrap();
        let applied = attractor_runtime::apply_outcome_context_updates(
            "run_child",
            &node_attrs,
            &mut context,
            &outcome,
        )
        .expect("apply");
        assert_ne!(applied.failure_kind, Some(FailureKind::Contract));
        assert_eq!(context.get("context.a"), Some(&json!("A")));
        assert_eq!(context.get("context.b"), None, "{status}");
    }
}

// ---------------------------------------------------------------------------
// Execution errors: bounded retries, then failure, never a route
// ---------------------------------------------------------------------------

/// Resumes `visit_loop_flow` paused while routed into evaluate's first visit
/// (no durable outcome). Evaluate raises a retryable runtime failure on its
/// first `runtime_failures` calls, then decides `done`. Returns the result
/// and the node ids executed after resume.
fn resume_into_runtime_failure(
    store: &RunStore,
    workdir: &std::path::Path,
    run_id: &str,
    max_retries: Option<u64>,
    runtime_failures: usize,
) -> (attractor_runtime::PipelineExecutionResult, Vec<String>) {
    let mut flow = visit_loop_flow(&[("again", "implement"), ("done", "done")]);
    flow.nodes.get_mut("evaluate").unwrap().retry = Some(attractor_core::RetryConfig {
        policy: None,
        max_retries,
    });
    let (paused, _) = run_visit_loop(
        store,
        run_id,
        workdir,
        flow.clone(),
        |_| Outcome::completed("done"),
        Some(2),
        Default::default(),
    );
    assert_eq!(paused.status, "paused");
    let meta = store.read_run_meta(run_id).unwrap().unwrap();
    let checkpoint = meta.checkpoint.unwrap();
    assert_eq!(checkpoint.current_node, "evaluate");

    let calls = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = calls.clone();
    let mut executor = PipelineExecutor::new(move |request: NodeExecutionRequest| {
        let mut seen = seen.lock().unwrap();
        seen.push(request.node_id.clone());
        let evaluations = seen.iter().filter(|node| *node == "evaluate").count();
        if request.node_id == "evaluate" && evaluations <= runtime_failures {
            return Err(attractor_runtime::RuntimeNodeError::runtime(
                "failed to read activity transcript",
            ));
        }
        Ok(step(&request.node_id))
    });
    let result = executor
        .execute(execute_request(
            store,
            run_id,
            workdir,
            flow,
            None,
            attractor_runtime::ExecutionStart::Resume {
                paths: meta.paths,
                checkpoint,
            },
        ))
        .expect("resume");
    let calls = calls.lock().unwrap().clone();
    (result, calls)
}

#[test]
fn resumed_runtime_failure_is_retried_once_without_configured_retries() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let (result, calls) = resume_into_runtime_failure(&store, temp.path(), "run-rt-once", None, 1);
    assert_eq!(result.status, "completed", "{}", result.failure_reason);
    assert_eq!(result.current_node, "done");
    assert_eq!(calls, vec!["evaluate", "evaluate"]);
}

#[test]
fn exhausted_runtime_failure_fails_the_run_instead_of_following_any_route() {
    for (max_retries, expected_attempts) in [(None, 2), (Some(0), 2), (Some(3), 4)] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = RunStore::for_runs_dir(temp.path().join("runs"));
        let run_id = "run-rt-exhausted";
        let (result, calls) =
            resume_into_runtime_failure(&store, temp.path(), run_id, max_retries, usize::MAX);
        let case = format!("max_retries={max_retries:?}");
        assert_eq!(result.status, "failed", "{case}");
        assert_eq!(
            result.failure_reason, "failed to read activity transcript",
            "{case}"
        );
        assert_eq!(calls, vec!["evaluate"; expected_attempts], "{case}");

        let meta = store.read_run_meta(run_id).unwrap().unwrap();
        let checkpoint = meta.checkpoint.unwrap();
        assert_eq!(checkpoint.current_node, "evaluate", "{case}");
        assert!(
            !checkpoint.completed_nodes.contains(&"evaluate".to_string()),
            "{case}"
        );
        assert_eq!(meta.record.unwrap().status, "failed", "{case}");

        let status = latest_status(&store, run_id, "evaluate");
        assert_eq!(status["status"], "error", "{case}");
        assert_eq!(status["decision"], "", "{case}");
        assert_eq!(status["failure_kind"], "runtime", "{case}");
        assert_eq!(status["tolerable"], true, "{case}");
        assert_eq!(status["retryable"], true, "{case}");
        assert_eq!(
            status["failure_reason"], "failed to read activity transcript",
            "{case}"
        );
        assert!(status.get("outcome").is_none(), "{case}: {status}");
        let retries = events_of(&store, run_id)
            .into_iter()
            .filter(|event| event.event_type == "StageRetrying")
            .count();
        assert_eq!(retries, expected_attempts - 1, "{case}");
    }
}

/// A negative judgment is a completed decision: it is never retried and
/// follows its route. A contract error is never retried either, even with
/// retries configured, and fails the run instead of being accepted.
#[test]
fn negative_decisions_route_without_retry_and_contract_errors_never_retry() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let mut flow = visit_loop_flow(&[("again", "implement"), ("done", "done")]);
    flow.nodes.get_mut("implement").unwrap().runtime = None;
    flow.nodes.get_mut("evaluate").unwrap().retry = Some(attractor_core::RetryConfig {
        policy: None,
        max_retries: Some(2),
    });
    let (result, calls) = run_visit_loop(
        &store,
        "run-verdict-again",
        temp.path(),
        flow.clone(),
        again,
        Some(4),
        Default::default(),
    );
    assert_eq!(result.status, "paused");
    assert_eq!(calls, vec!["start", "implement", "evaluate", "implement"]);

    let (result, calls) = run_visit_loop(
        &store,
        "run-verdict-contract",
        temp.path(),
        flow,
        |_| {
            Outcome::error(
                FailureKind::Contract,
                "invalid decision envelope: decision \"partial\" is not declared",
                None,
            )
        },
        None,
        Default::default(),
    );
    assert_eq!(result.status, "failed");
    assert_eq!(
        result.failure_reason,
        "invalid decision envelope: decision \"partial\" is not declared"
    );
    assert_eq!(calls, vec!["start", "implement", "evaluate"]);
    assert!(!events_of(&store, "run-verdict-contract")
        .iter()
        .any(|event| event.event_type == "StageRetrying"));
}

#[test]
fn parallel_branch_runtime_failure_fails_the_branch_after_its_retry() {
    let flow = looping_branch_flow(None, true);
    let calls = Arc::new(AtomicUsize::new(0));
    let mut runner = RuntimeHandlerRunner::new();
    let counter = calls.clone();
    runner.register_handler_fn(HANDLER_CODERGEN, move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
        Err(attractor_runtime::RuntimeNodeError::runtime(
            "failed to read activity transcript",
        ))
    });
    let result = execute_looping_branch(&mut runner, &calls, &flow);
    assert_eq!(result["status"], "error", "{result}");
    // The branch applies the root's retry rule: one automatic retry.
    assert_eq!(result["work_calls"], 2, "{result}");
    assert_eq!(
        result["error"]["reason"],
        "failed to read activity transcript"
    );
    assert_eq!(result["error"]["kind"], "runtime");
    assert_eq!(result["error"]["attempts"], 2);
    assert_eq!(result["decisions"], json!([]));

    // A negative decision loops, then the branch reaches the join.
    let counter = calls.clone();
    runner.register_handler_fn(HANDLER_CODERGEN, move |_| {
        if counter.fetch_add(1, Ordering::SeqCst) == 0 {
            return Ok(Outcome::completed("again"));
        }
        Ok(Outcome::completed("done"))
    });
    let result = execute_looping_branch(&mut runner, &calls, &flow);
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(result["work_calls"], 2, "{result}");
    assert_eq!(
        result["decisions"],
        json!([
            {"node": "work", "decision": "again", "target": "work"},
            {"node": "work", "decision": "done", "target": "join"}
        ])
    );
}

/// The failed run's durable record, result and events all carry `reason`,
/// and nothing records a completion.
fn assert_durably_failed(store: &RunStore, run_id: &str, reason: &str) {
    let paths = store.find_run_root(run_id).unwrap().unwrap();
    let record = store.read_run_record(&paths).unwrap().unwrap();
    assert_eq!(record.status, "failed");
    assert_eq!(record.last_error, reason);
    assert!(record.ended_at.is_some());
    assert_eq!(store.read_result(&paths).unwrap().unwrap().status, "failed");
    let events = store.read_raw_events(&paths).unwrap();
    let terminal = events
        .iter()
        .filter(|event| event.event_type == "runtime")
        .last()
        .unwrap();
    assert_eq!(terminal.payload["status"], "failed");
    assert!(events
        .iter()
        .any(|event| event.event_type == "PipelineFailed" && event.payload["error"] == reason));
    assert!(!events
        .iter()
        .any(|event| event.event_type == "PipelineCompleted"));
}

// ---------------------------------------------------------------------------
// Undeclared decisions never succeed
// ---------------------------------------------------------------------------

/// A decision the node does not declare fails the run with the node, the
/// decision and every declared route, keeps the recorded decision, and never
/// reaches a later node; a declared decision still continues.
#[test]
fn undeclared_decision_fails_the_run_with_a_route_diagnostic() {
    for decision in ["partial", "", "Done"] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = RunStore::for_runs_dir(temp.path().join("runs"));
        let (result, calls) = run_visit_loop(
            &store,
            "run-unmatched",
            temp.path(),
            visit_loop_flow(&[("done", "done")]),
            move |_| Outcome::completed(decision),
            None,
            Default::default(),
        );
        let reason = no_matching_route("evaluate", decision, &[("done", "done")]);
        assert_eq!(result.status, "failed", "{decision:?}");
        assert_eq!(result.failure_reason, reason);
        assert_eq!(result.current_node, "evaluate");
        assert_eq!(result.node_outcomes["evaluate"].decision, decision);
        assert_eq!(calls, vec!["start", "implement", "evaluate"]);
        assert_durably_failed(&store, "run-unmatched", &reason);
        let checkpoint = store
            .read_run_meta("run-unmatched")
            .unwrap()
            .unwrap()
            .checkpoint
            .unwrap();
        assert_eq!(
            checkpoint.context["_attractor.node_outcomes"]["evaluate"],
            json!({"status": "completed", "decision": decision})
        );

        let (result, _) = run_visit_loop(
            &store,
            "run-matched",
            temp.path(),
            visit_loop_flow(&[("done", "done")]),
            |_| Outcome::completed("done"),
            None,
            Default::default(),
        );
        assert_eq!(result.status, "completed", "{}", result.failure_reason);
        assert_eq!(result.current_node, "done");
    }
}

/// Execution errors keep their own reason and never consult the route map,
/// whatever their kind; an error without a reason fails as stage_failed.
#[test]
fn execution_errors_keep_their_reason_without_route_details() {
    for (outcome, reason) in [
        (
            Outcome::error(FailureKind::Runtime, "not good enough", Some(false)),
            "not good enough",
        ),
        (
            Outcome::error(FailureKind::Contract, "malformed envelope", None),
            "malformed envelope",
        ),
        (
            Outcome::error(FailureKind::Runtime, "", Some(false)),
            "stage_failed",
        ),
    ] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = RunStore::for_runs_dir(temp.path().join("runs"));
        let (result, calls) = run_visit_loop(
            &store,
            "run-error-reason",
            temp.path(),
            visit_loop_flow(&[("done", "done")]),
            move |_| outcome.clone(),
            None,
            Default::default(),
        );
        assert_eq!(result.status, "failed", "{reason}");
        assert_eq!(result.failure_reason, reason);
        assert!(!result.failure_reason.contains("no_matching_route"));
        assert_eq!(calls, vec!["start", "implement", "evaluate"]);
        assert_durably_failed(&store, "run-error-reason", reason);
    }
}

/// Only an exit ends a run: a route-free non-exit leaf fails with the
/// unmatched decision, and an explicit exit completes with its result.
#[test]
fn non_exit_leaf_fails_while_explicit_exit_succeeds() {
    for with_exit in [false, true] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = RunStore::for_runs_dir(temp.path().join("runs"));
        let mut flow = linear_flow(agent("task", &[]));
        if !with_exit {
            flow.nodes.get_mut("task").unwrap().routes.clear();
        }
        let result =
            PipelineExecutor::new(|request: NodeExecutionRequest| Ok(step(&request.node_id)))
                .execute(execute_request(
                    &store,
                    "run-leaf",
                    temp.path(),
                    flow,
                    None,
                    Default::default(),
                ))
                .expect("execute");
        if with_exit {
            assert_eq!(result.status, "completed", "{}", result.failure_reason);
            assert_eq!(result.current_node, "done");
            assert_eq!(result.outcome.as_deref(), Some("success"));
            assert_eq!(result.outcome_reason_code.as_deref(), Some("completed"));
            continue;
        }
        let reason = no_matching_route("task", "done", &[]);
        assert_eq!(result.status, "failed");
        assert_eq!(result.failure_reason, reason);
        assert_durably_failed(&store, "run-leaf", &reason);
    }
}

/// Dispatch is exact: a route's display label or its target's node ID is
/// not a decision, so neither selects the route.
#[test]
fn labels_and_target_ids_are_not_decisions() {
    for decision in ["Again", "implement", " again", "AGAIN"] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = RunStore::for_runs_dir(temp.path().join("runs"));
        let mut flow = visit_loop_flow(&[("again", "implement"), ("done", "done")]);
        flow.nodes
            .get_mut("evaluate")
            .unwrap()
            .routes
            .get_mut("again")
            .unwrap()
            .label = "Again".to_string();
        let (result, calls) = run_visit_loop(
            &store,
            "run-label",
            temp.path(),
            flow,
            move |_| Outcome::completed(decision),
            None,
            Default::default(),
        );
        let reason = no_matching_route(
            "evaluate",
            decision,
            &[("again", "implement"), ("done", "done")],
        );
        assert_eq!(result.status, "failed", "{decision:?}");
        assert_eq!(result.failure_reason, reason);
        assert_eq!(implement_runs(&calls), 1, "{decision:?}");
        assert_durably_failed(&store, "run-label", &reason);
    }
}

/// Runs `flow` with `outcome_for` answering each node, pauses right after
/// `evaluate` completes, then resumes; returns the resumed result and the
/// nodes the resume executed.
fn pause_after_evaluate_then_resume(
    store: &RunStore,
    run_id: &str,
    workdir: &std::path::Path,
    flow: impl Fn() -> FlowDefinition,
    outcome_for: fn(&str) -> Outcome,
) -> (attractor_runtime::PipelineExecutionResult, Vec<String>) {
    let calls = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = calls.clone();
    let polled = calls.clone();
    let paused = PipelineExecutor::with_control(
        move |request: NodeExecutionRequest| {
            seen.lock().unwrap().push(request.node_id.clone());
            Ok(outcome_for(&request.node_id))
        },
        move || {
            polled
                .lock()
                .unwrap()
                .contains(&"evaluate".to_string())
                .then_some(attractor_runtime::ExecutionControlAction::Pause)
        },
    )
    .execute(execute_request(
        store,
        run_id,
        workdir,
        flow(),
        None,
        Default::default(),
    ))
    .expect("execute");
    assert_eq!(paused.status, "paused");
    let meta = store.read_run_meta(run_id).unwrap().unwrap();
    let checkpoint = meta.checkpoint.unwrap();
    assert_eq!(checkpoint.current_node, "evaluate");

    let resumed_calls = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = resumed_calls.clone();
    let resumed = PipelineExecutor::new(move |request: NodeExecutionRequest| {
        seen.lock().unwrap().push(request.node_id.clone());
        Ok(outcome_for(&request.node_id))
    })
    .execute(execute_request(
        store,
        run_id,
        workdir,
        flow(),
        None,
        attractor_runtime::ExecutionStart::Resume {
            paths: meta.paths,
            checkpoint,
        },
    ))
    .expect("resume");
    let resumed_calls = resumed_calls.lock().unwrap().clone();
    (resumed, resumed_calls)
}

/// Recovery of a completed visit whose decision is undeclared fails the run
/// from the stored visit, without executing the node again, with the same
/// reason uninterrupted execution gives.
#[test]
fn resume_of_an_undeclared_completed_visit_fails_without_reexecution() {
    let cases: [(fn(&str) -> Outcome, &str); 2] = [
        (
            |node| match node {
                "evaluate" => Outcome::completed("partial"),
                other => step(other),
            },
            "partial",
        ),
        (
            |node| match node {
                "evaluate" => Outcome::completed(""),
                other => step(other),
            },
            "",
        ),
    ];
    for (outcome_for, decision) in cases {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = RunStore::for_runs_dir(temp.path().join("runs"));
        let (resumed, calls) = pause_after_evaluate_then_resume(
            &store,
            "run-unmatched-resume",
            temp.path(),
            || visit_loop_flow(&[("done", "done")]),
            outcome_for,
        );
        let reason = no_matching_route("evaluate", decision, &[("done", "done")]);
        assert_eq!(resumed.status, "failed");
        assert_eq!(resumed.failure_reason, reason);
        assert!(calls.is_empty(), "{calls:?}");
        assert_durably_failed(&store, "run-unmatched-resume", &reason);
    }
}

// ---------------------------------------------------------------------------
// Forks: wait_all, strict or tolerant of branch execution errors
// ---------------------------------------------------------------------------

/// start -> fan -> {good, stray} -> join -> after -> done; `stray` may also
/// loop on itself (one visit) or choose `blocked` to the shared failure exit.
fn branch_flow(tolerate: bool) -> FlowDefinition {
    flow_of(
        "branch-policy",
        vec![
            ("start", start_node("fan")),
            ("fan", fork(&["good", "stray"], "join", tolerate)),
            ("good", agent("good", &[("done", "join")])),
            (
                "stray",
                FlowNode {
                    runtime: Some(NodeRuntimeConfig {
                        max_visits: Some(1),
                        ..NodeRuntimeConfig::default()
                    }),
                    ..agent(
                        "stray",
                        &[("done", "join"), ("again", "stray"), ("blocked", "failed")],
                    )
                },
            ),
            ("join", fan_in("after")),
            ("after", agent("after", &[("done", "done")])),
            ("done", success_exit()),
            ("failed", exit_node("blocked", ExitOutcome::Failure)),
        ],
    )
}

const STRAY_ROUTES: [(&str, &str); 3] =
    [("again", "stray"), ("blocked", "failed"), ("done", "join")];

/// Runs `flow` with `stray` answering `stray` and every other agent
/// completing `done`; returns the result and the agent nodes executed.
fn run_branch_flow(
    store: &RunStore,
    workdir: &std::path::Path,
    flow: FlowDefinition,
    stray: Outcome,
) -> (attractor_runtime::PipelineExecutionResult, Vec<String>) {
    let calls = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = calls.clone();
    let mut runner = RuntimeHandlerRunner::new();
    runner.register_handler_fn(HANDLER_CODERGEN, move |runtime| {
        seen.lock().unwrap().push(runtime.node_id.clone());
        if runtime.node_id == "stray" {
            return Ok(stray.clone());
        }
        Ok(Outcome {
            context_updates: ContextMap::new(),
            ..Outcome::completed("done")
        })
    });
    let result = PipelineExecutor::new(runner)
        .execute(execute_request(
            store,
            "run-branch",
            workdir,
            flow,
            None,
            Default::default(),
        ))
        .expect("execute");
    let calls = calls.lock().unwrap().clone();
    (result, calls)
}

fn stage_starts(store: &RunStore, node_id: &str) -> usize {
    events_of(store, "run-branch")
        .iter()
        .filter(|event| event.event_type == "StageStarted" && event.payload["node_id"] == node_id)
        .count()
}

/// An undeclared branch decision and an exhausted branch target are routing
/// errors: they fail the owning run whether or not the fork tolerates branch
/// execution errors, before the join; a sibling reaching the join does not
/// rescue it.
#[test]
fn branch_routing_errors_fail_the_parent_strict_or_tolerant() {
    for (decision, details) in [
        (
            "partial",
            no_matching_route("stray", "partial", &STRAY_ROUTES),
        ),
        ("again", visit_limit("stray", "again", "stray", 1, 1)),
    ] {
        let reason = format!("branch stray (index 1) failed at stray: {details}");
        for tolerate in [false, true] {
            let temp = tempfile::tempdir().expect("tempdir");
            let store = RunStore::for_runs_dir(temp.path().join("runs"));
            let (result, calls) = run_branch_flow(
                &store,
                temp.path(),
                branch_flow(tolerate),
                Outcome::completed(decision),
            );
            let case = format!("{decision} tolerate={tolerate}");
            assert_eq!(result.status, "failed", "{case}");
            assert_eq!(result.failure_reason, reason, "{case}");
            assert_eq!(result.current_node, "fan", "{case}");
            assert!(!calls.contains(&"after".to_string()), "{case}: {calls:?}");
            assert_eq!(
                calls.iter().filter(|node| *node == "stray").count(),
                1,
                "{case}: the fork is not retried"
            );
            assert_eq!(stage_starts(&store, "join"), 0, "{case}");
            assert_durably_failed(&store, "run-branch", &reason);
            let status = latest_status(&store, "run-branch", "fan");
            assert_eq!(status["failure_kind"], "routing", "{case}");
            assert_eq!(status["tolerable"], false, "{case}");
        }
    }
}

/// Runs `flow` with `gate` looping back twice and every other agent completing
/// `done`; returns the result and each agent's node and `parallel.results`.
fn run_forks(
    flow: FlowDefinition,
) -> (
    attractor_runtime::PipelineExecutionResult,
    Vec<(String, Option<Value>)>,
) {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = calls.clone();
    let mut runner = RuntimeHandlerRunner::new();
    runner.register_handler_fn(HANDLER_CODERGEN, move |runtime| {
        let mut calls = seen.lock().unwrap();
        calls.push((
            runtime.node_id.clone(),
            runtime.context.get("parallel.results").cloned(),
        ));
        let gates = calls.iter().filter(|(node, _)| node == "gate").count();
        let loop_back = runtime.node_id == "gate" && gates < 3;
        Ok(Outcome::completed(if loop_back { "again" } else { "done" }))
    });
    let mut request = execute_request(
        &store,
        "run-forks",
        temp.path(),
        flow,
        None,
        Default::default(),
    );
    request.runtime_context = ContextMap::from([(
        "context.fixture.padding".to_string(),
        json!("x".repeat(4096)),
    )]);
    let result = PipelineExecutor::new(runner)
        .execute(request)
        .expect("execute");
    let calls = calls.lock().unwrap().clone();
    (result, calls)
}

/// Each entry keeps its branch's inherited values but no earlier results.
fn assert_entries_hold_no_results(results: &Value, ids: &[&str]) {
    let entries = results.as_array().expect("results");
    assert_eq!(
        entries.iter().map(|entry| &entry["id"]).collect::<Vec<_>>(),
        ids
    );
    for entry in entries {
        assert_eq!(entry["status"], "completed");
        assert_eq!(
            entry["context"]["context.fixture.padding"],
            "x".repeat(4096)
        );
        assert!(entry["context"].get("parallel.results").is_none());
    }
}

/// Nesting each round's results in the next grows them by the branch count per
/// round, which took a prompt past a provider's limit by round three.
#[test]
fn a_repeated_fork_keeps_parallel_results_the_same_size() {
    let (result, calls) = run_forks(flow_of(
        "repeated-fork",
        vec![
            ("start", start_node("fan")),
            ("fan", fork(&["a", "b"], "join", false)),
            ("a", agent("a", &[("done", "join")])),
            ("b", agent("b", &[("done", "join")])),
            ("join", fan_in("gate")),
            ("gate", agent("gate", &[("again", "fan"), ("done", "done")])),
            ("done", success_exit()),
        ],
    ));
    assert_eq!(result.status, "completed", "{}", result.failure_reason);

    let rounds = calls
        .iter()
        .filter(|(node, _)| node == "gate")
        .map(|(_, results)| results.clone().expect("gate reads the round's results"))
        .collect::<Vec<_>>();
    assert_eq!(rounds.len(), 3);
    for round in &rounds {
        assert_entries_hold_no_results(round, &["a", "b"]);
    }
    let sizes = rounds
        .iter()
        .map(|round| round.to_string().len())
        .collect::<Vec<_>>();
    assert!(sizes[2] < sizes[0] * 2, "{sizes:?}");
    assert!(
        calls
            .iter()
            .all(|(node, results)| node == "gate" || results.is_none()),
        "branches start without earlier results"
    );
}

#[test]
fn a_later_fork_does_not_nest_an_earlier_forks_results() {
    let (result, calls) = run_forks(flow_of(
        "successive-forks",
        vec![
            ("start", start_node("first")),
            ("first", fork(&["a", "b"], "first_join", false)),
            ("a", agent("a", &[("done", "first_join")])),
            ("b", agent("b", &[("done", "first_join")])),
            ("first_join", fan_in("second")),
            ("second", fork(&["c", "d"], "second_join", false)),
            ("c", agent("c", &[("done", "second_join")])),
            ("d", agent("d", &[("done", "second_join")])),
            ("second_join", fan_in("done")),
            ("done", success_exit()),
        ],
    ));
    assert_eq!(result.status, "completed", "{}", result.failure_reason);
    assert_entries_hold_no_results(&result.context["parallel.results"], &["c", "d"]);
    assert_eq!(calls.len(), 4);
    assert!(calls.iter().all(|(_, results)| results.is_none()));
}

/// A branch execution error fails a strict fork; a tolerant fork keeps it
/// as evidence in parallel.results and enters the join exactly once.
#[test]
fn branch_execution_errors_fail_strict_forks_and_enter_the_join_of_tolerant_ones() {
    let stray = Outcome::error(FailureKind::Runtime, "stray failed", Some(false));
    for tolerate in [false, true] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = RunStore::for_runs_dir(temp.path().join("runs"));
        let (result, calls) =
            run_branch_flow(&store, temp.path(), branch_flow(tolerate), stray.clone());
        if !tolerate {
            let reason = "branch stray (index 1) failed at stray: stray failed";
            assert_eq!(result.status, "failed");
            assert_eq!(result.failure_reason, reason);
            assert!(!calls.contains(&"after".to_string()), "{calls:?}");
            assert_eq!(stage_starts(&store, "join"), 0);
            assert_durably_failed(&store, "run-branch", reason);
            continue;
        }
        assert_eq!(result.status, "completed", "{}", result.failure_reason);
        assert_eq!(result.current_node, "done");
        assert_eq!(calls.iter().filter(|node| *node == "after").count(), 1);
        assert_eq!(stage_starts(&store, "join"), 1);
        let results = result.context["parallel.results"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(results[0]["id"], "good");
        assert_eq!(results[0]["status"], "completed");
        assert_eq!(results[1]["id"], "stray");
        assert_eq!(results[1]["status"], "error");
        assert_eq!(results[1]["error"]["kind"], "runtime");
        assert_eq!(results[1]["error"]["tolerable"], true);
        assert_eq!(results[1]["error"]["reason"], "stray failed");
    }
}

/// A branch choosing a failure exit finalizes the owner with that exit's
/// result: completed/failure, the join and parent continuation never run.
#[test]
fn branch_failure_exit_finalizes_the_owner_with_its_result() {
    for tolerate in [false, true] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = RunStore::for_runs_dir(temp.path().join("runs"));
        let (result, calls) = run_branch_flow(
            &store,
            temp.path(),
            branch_flow(tolerate),
            Outcome {
                notes: "no credentials".to_string(),
                ..Outcome::completed("blocked")
            },
        );
        assert_eq!(result.status, "completed", "{}", result.failure_reason);
        assert_eq!(result.current_node, "failed");
        assert_eq!(result.outcome.as_deref(), Some("failure"));
        assert_eq!(result.outcome_reason_code.as_deref(), Some("blocked"));
        assert_eq!(
            result.outcome_reason_message.as_deref(),
            Some("branch stray (index 1): stray chose blocked: no credentials")
        );
        assert!(!calls.contains(&"after".to_string()), "{calls:?}");
        assert_eq!(stage_starts(&store, "join"), 0);
        let record = store
            .read_run_meta("run-branch")
            .unwrap()
            .unwrap()
            .record
            .unwrap();
        assert_eq!(record.status, "completed");
        assert_eq!(record.outcome.as_deref(), Some("failure"));
        assert_eq!(record.outcome_reason_code.as_deref(), Some("blocked"));
        let status = latest_status(&store, "run-branch", "fan");
        assert_eq!(status["terminal"]["exit_id"], "failed");
        assert_eq!(status["terminal"]["decision"], "blocked");
    }
}

/// Recovery that finds the fork's durable routing-error status written but
/// the run not yet failed fails the run from that status: no branch runs
/// again, the retryable fork is not retried, and nothing continues.
#[test]
fn resume_after_a_branch_routing_error_status_write_fails_without_continuing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let flow = || {
        let mut flow = branch_flow(true);
        flow.nodes.get_mut("fan").unwrap().retry = Some(attractor_core::RetryConfig {
            max_retries: Some(2),
            ..Default::default()
        });
        flow
    };
    let (failed, _) = run_branch_flow(&store, temp.path(), flow(), Outcome::completed("partial"));
    let reason = format!(
        "branch stray (index 1) failed at stray: {}",
        no_matching_route("stray", "partial", &STRAY_ROUTES)
    );
    assert_eq!(failed.status, "failed");
    assert_eq!(failed.failure_reason, reason);

    // Rewind to the crash window: fan's status is durable, the run is not
    // failed, and the checkpoint still names fan's pending visit.
    let meta = store.read_run_meta("run-branch").unwrap().unwrap();
    let checkpoint = meta.checkpoint.unwrap();
    assert_eq!(checkpoint.current_node, "fan");
    store
        .update_run_record("run-branch", |record| {
            record.status = "running".to_string();
            record.last_error.clear();
            record.ended_at = None;
        })
        .unwrap();
    let failures_before = store
        .read_raw_events(&meta.paths)
        .unwrap()
        .iter()
        .filter(|event| event.event_type == "PipelineFailed")
        .count();

    let calls = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = calls.clone();
    let mut runner = RuntimeHandlerRunner::new();
    runner.register_handler_fn(HANDLER_CODERGEN, move |runtime| {
        seen.lock().unwrap().push(runtime.node_id.clone());
        Ok(Outcome::completed("done"))
    });
    let resumed = PipelineExecutor::new(runner)
        .execute(execute_request(
            &store,
            "run-branch",
            temp.path(),
            flow(),
            None,
            attractor_runtime::ExecutionStart::Resume {
                paths: meta.paths.clone(),
                checkpoint,
            },
        ))
        .expect("resume");
    assert_eq!(resumed.status, "failed");
    assert_eq!(resumed.failure_reason, reason);
    assert_eq!(resumed.current_node, "fan");
    assert!(
        calls.lock().unwrap().is_empty(),
        "{:?}",
        calls.lock().unwrap()
    );
    assert_durably_failed(&store, "run-branch", &reason);
    let failures_after = store
        .read_raw_events(&meta.paths)
        .unwrap()
        .iter()
        .filter(|event| event.event_type == "PipelineFailed")
        .count();
    assert_eq!(
        failures_after,
        failures_before + 1,
        "recovery records its own failure"
    );
}
