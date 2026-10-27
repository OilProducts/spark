use std::collections::BTreeMap;
use std::fs;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use attractor_core::{
    attr_bool, attr_text, ContextMap, FlowDefinition, FlowEdge, FlowNode, LaunchContext,
    ManagerLoopConfig, NodeConfig, NodeContracts, NodeKind, NodeRuntimeConfig, Outcome,
    OutcomeStatus, RunRecord,
};
use attractor_runtime::{
    ChildRunResult, CreateRunRequest, ExecuteRunRequest, NodeExecutionRequest, NodeExecutor,
    PipelineExecutor, RunStore, RuntimeHandlerRunner, HANDLER_CODERGEN,
};
use serde_json::json;

fn linear_flow(task: FlowNode) -> FlowDefinition {
    FlowDefinition {
        schema_version: "1".to_string(),
        id: "typed-runtime".to_string(),
        title: "Typed Runtime".to_string(),
        nodes: [
            (
                "start".to_string(),
                FlowNode {
                    kind: NodeKind::Start,
                    config: Some(NodeConfig::Start {}),
                    ..FlowNode::default()
                },
            ),
            ("task".to_string(), task),
            (
                "done".to_string(),
                FlowNode {
                    kind: NodeKind::Exit,
                    config: Some(NodeConfig::Exit {
                        result_summary: false,
                        result_summary_prompt: None,
                    }),
                    ..FlowNode::default()
                },
            ),
        ]
        .into_iter()
        .collect(),
        edges: vec![
            FlowEdge {
                from: "start".to_string(),
                to: "task".to_string(),
                ..FlowEdge::default()
            },
            FlowEdge {
                from: "task".to_string(),
                to: "done".to_string(),
                ..FlowEdge::default()
            },
        ],
        ..FlowDefinition::default()
    }
}

#[test]
fn executor_enforces_typed_write_contract_without_extension_duplicate() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let flow = linear_flow(FlowNode {
        kind: NodeKind::AgentTask,
        config: Some(NodeConfig::AgentTask {
            prompt: "write allowed context".to_string(),
        }),
        contracts: Some(NodeContracts {
            writes_context: vec!["context.allowed".to_string()],
            ..NodeContracts::default()
        }),
        ..FlowNode::default()
    });
    let mut executor = PipelineExecutor::new(|request: NodeExecutionRequest| {
        if request.node_id == "task" {
            return Ok(Outcome {
                context_updates: ContextMap::from([("context.denied".to_string(), json!(true))]),
                ..Outcome::new(OutcomeStatus::Success)
            });
        }
        Ok(Outcome::new(OutcomeStatus::Success))
    });
    let record = RunRecord::new("run-typed-contract", temp.path().to_string_lossy());

    let result = executor
        .execute(ExecuteRunRequest {
            store,
            record,
            flow,
            flow_definition_json: None,
            launch_context: LaunchContext::empty(),
            runtime_context: ContextMap::new(),
            max_steps: None,
            start: Default::default(),
        })
        .expect("execute typed contract flow");

    assert_eq!(result.status, "failed");
    assert!(
        result.failure_reason.contains("context.denied"),
        "failure reason should name the denied typed contract key: {}",
        result.failure_reason
    );
}

#[test]
fn parallel_handler_uses_typed_parallel_config_without_extension_duplicate() {
    let flow = FlowDefinition {
        schema_version: "1".to_string(),
        id: "typed-parallel".to_string(),
        nodes: [
            (
                "fan".to_string(),
                FlowNode {
                    kind: NodeKind::Parallel,
                    config: Some(NodeConfig::Parallel {
                        join_policy: Some("first_success".to_string()),
                        max_parallel: Some(1),
                        join_k: None,
                        join_quorum: None,
                    }),
                    runtime: Some(NodeRuntimeConfig {
                        error_policy: Some("ignore".to_string()),
                        ..NodeRuntimeConfig::default()
                    }),
                    ..FlowNode::default()
                },
            ),
            (
                "good".to_string(),
                FlowNode {
                    kind: NodeKind::AgentTask,
                    extensions: [("type".to_string(), json!("custom.success"))]
                        .into_iter()
                        .collect(),
                    ..FlowNode::default()
                },
            ),
            (
                "bad".to_string(),
                FlowNode {
                    kind: NodeKind::AgentTask,
                    extensions: [("type".to_string(), json!("custom.fail"))]
                        .into_iter()
                        .collect(),
                    ..FlowNode::default()
                },
            ),
        ]
        .into_iter()
        .collect(),
        edges: vec![
            FlowEdge {
                from: "fan".to_string(),
                to: "bad".to_string(),
                ..FlowEdge::default()
            },
            FlowEdge {
                from: "fan".to_string(),
                to: "good".to_string(),
                ..FlowEdge::default()
            },
        ],
        ..FlowDefinition::default()
    };
    let mut runner = RuntimeHandlerRunner::new();
    runner.register_handler_fn(HANDLER_CODERGEN, |runtime| {
        assert_eq!(
            runtime.attempt, 3,
            "branches inherit the prepared parallel attempt"
        );
        Ok(Outcome::new(OutcomeStatus::Success))
    });
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = RunStore::for_runs_dir(temp.path().join("runs"))
        .create_run(CreateRunRequest {
            record: RunRecord::new("run-typed-parallel", temp.path().to_string_lossy()),
            checkpoint: None,
            manifest: None,
            flow_definition_json: None,
            flow_files: None,
        })
        .expect("create run");

    let outcome = runner
        .execute(attractor_runtime::NodeExecutionRequest {
            node_id: "fan".to_string(),
            stage_index: 0,
            attempt: 3,
            context: ContextMap::new(),
            prompt: String::new(),
            node: flow.nodes["fan"].clone(),
            node_attrs: attractor_runtime::flow_runtime::node_attrs_for_handler(
                "fan",
                &flow.nodes["fan"],
            ),
            flow: flow.clone(),
            outgoing_edges: attractor_runtime::outgoing_routing_edges(&flow, "fan")
                .expect("outgoing"),
            run_paths: Some(paths),
            run_workdir: temp.path().to_path_buf(),
            run_id: "run-typed-parallel".to_string(),
            fallback_model: None,
            fallback_provider: None,
            fallback_profile: None,
            fallback_reasoning_effort: None,
        })
        .expect("parallel outcome");

    assert_eq!(outcome.status, OutcomeStatus::Success);
}

#[test]
fn handler_dispatch_ignores_legacy_extension_type_override() {
    let flow = linear_flow(FlowNode {
        kind: NodeKind::AgentTask,
        extensions: [("type".to_string(), json!("custom.fail"))]
            .into_iter()
            .collect(),
        ..FlowNode::default()
    });
    let mut runner = RuntimeHandlerRunner::new();
    runner.register_static_handler("custom.fail", Outcome::new(OutcomeStatus::Fail));
    runner.register_static_handler(HANDLER_CODERGEN, Outcome::new(OutcomeStatus::Success));

    let outcome = runner
        .execute(attractor_runtime::NodeExecutionRequest {
            node_id: "task".to_string(),
            stage_index: 0,
            attempt: 0,
            context: ContextMap::new(),
            prompt: String::new(),
            node: flow.nodes["task"].clone(),
            node_attrs: attractor_runtime::flow_runtime::node_attrs_for_handler(
                "task",
                &flow.nodes["task"],
            ),
            flow: flow.clone(),
            outgoing_edges: attractor_runtime::outgoing_routing_edges(&flow, "task")
                .expect("outgoing"),
            run_paths: None,
            run_workdir: std::env::current_dir().expect("current dir"),
            run_id: "run-typed-dispatch".to_string(),
            fallback_model: None,
            fallback_provider: None,
            fallback_profile: None,
            fallback_reasoning_effort: None,
        })
        .expect("typed handler outcome");

    assert_eq!(outcome.status, OutcomeStatus::Success);
}

#[test]
fn extension_only_core_runtime_keys_do_not_drive_terminal_or_retry_behavior() {
    let node = FlowNode {
        kind: NodeKind::AgentTask,
        extensions: [
            ("allow_partial".to_string(), json!(true)),
            ("goal_gate".to_string(), json!(true)),
            ("retry_target".to_string(), json!("retry")),
            ("fallback_retry_target".to_string(), json!("retry")),
        ]
        .into_iter()
        .collect(),
        ..FlowNode::default()
    };
    let flow = FlowDefinition {
        schema_version: "1".to_string(),
        id: "extension-core-ignored".to_string(),
        nodes: [
            ("task".to_string(), node.clone()),
            (
                "retry".to_string(),
                FlowNode {
                    kind: NodeKind::AgentTask,
                    ..FlowNode::default()
                },
            ),
        ]
        .into_iter()
        .collect(),
        ..FlowDefinition::default()
    };

    assert!(!attractor_runtime::flow_runtime::node_attr_bool(
        &node,
        "allow_partial",
        false
    ));
    assert!(!attractor_runtime::terminal::is_goal_gate_node(
        &flow, "task"
    ));
    assert_eq!(
        attractor_runtime::terminal::resolve_failure_retry_target(&flow, "task"),
        None
    );

    let exhausted = attractor_runtime::retry::coerce_retry_exhausted_outcome(
        &flow,
        "task",
        &Outcome::new(OutcomeStatus::Retry),
        1,
        1,
    );
    assert_eq!(exhausted.status, OutcomeStatus::Fail);
}

#[test]
fn typed_core_runtime_keys_still_drive_terminal_and_retry_behavior() {
    let node = FlowNode {
        kind: NodeKind::AgentTask,
        runtime: Some(NodeRuntimeConfig {
            allow_partial: true,
            goal_gate: true,
            retry_target: Some("retry".to_string()),
            ..NodeRuntimeConfig::default()
        }),
        ..FlowNode::default()
    };
    let flow = FlowDefinition {
        schema_version: "1".to_string(),
        id: "typed-core-used".to_string(),
        nodes: [
            ("task".to_string(), node.clone()),
            (
                "retry".to_string(),
                FlowNode {
                    kind: NodeKind::AgentTask,
                    ..FlowNode::default()
                },
            ),
        ]
        .into_iter()
        .collect(),
        ..FlowDefinition::default()
    };

    assert!(attractor_runtime::flow_runtime::node_attr_bool(
        &node,
        "allow_partial",
        false
    ));
    assert!(attractor_runtime::terminal::is_goal_gate_node(
        &flow, "task"
    ));
    assert_eq!(
        attractor_runtime::terminal::resolve_failure_retry_target(&flow, "task"),
        Some("retry".to_string())
    );

    let exhausted = attractor_runtime::retry::coerce_retry_exhausted_outcome(
        &flow,
        "task",
        &Outcome::new(OutcomeStatus::Retry),
        1,
        1,
    );
    assert_eq!(exhausted.status, OutcomeStatus::PartialSuccess);
}

#[test]
fn node_context_core_keys_do_not_override_typed_handler_attrs() {
    let node = FlowNode {
        kind: NodeKind::Subflow,
        config: Some(NodeConfig::Subflow {
            flow_ref: "typed-child".to_string(),
            output_map: BTreeMap::new(),
            input_map: BTreeMap::new(),
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
    let flow = FlowDefinition {
        schema_version: "1".to_string(),
        id: "extension-child-ref-ignored".to_string(),
        title: "Extension Child Ref Ignored".to_string(),
        nodes: [(
            "run_child".to_string(),
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
                ..FlowNode::default()
            },
        )]
        .into_iter()
        .collect(),
        ..FlowDefinition::default()
    };
    let temp = tempfile::tempdir().expect("tempdir");
    write_child_flow(
        &temp.path().join("child"),
        json!({
            "schema_version": "1",
            "id": "child",
            "title": "Child",
            "nodes": {
                "start": { "kind": "start", "config": { "kind": "start" } },
                "done": { "kind": "exit", "config": { "kind": "exit" } }
            },
            "edges": [{ "from": "start", "to": "done" }]
        }),
    );
    let launched = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&launched);
    let mut runner = RuntimeHandlerRunner::new();
    runner.set_child_run_launcher(move |_request| {
        observed.store(true, Ordering::SeqCst);
        ChildRunResult {
            status: "completed".to_string(),
            outcome: Some("success".to_string()),
            ..ChildRunResult::default()
        }
    });

    let outcome = runner
        .execute(attractor_runtime::NodeExecutionRequest {
            node_id: "run_child".to_string(),
            stage_index: 0,
            attempt: 0,
            context: ContextMap::from([(
                "internal.run_workdir".to_string(),
                json!(temp.path().to_string_lossy().to_string()),
            )]),
            prompt: String::new(),
            node: flow.nodes["run_child"].clone(),
            node_attrs: attractor_runtime::flow_runtime::node_attrs_for_handler(
                "run_child",
                &flow.nodes["run_child"],
            ),
            flow: flow.clone(),
            outgoing_edges: Vec::new(),
            run_paths: None,
            run_workdir: temp.path().to_path_buf(),
            run_id: "parent-run".to_string(),
            fallback_model: None,
            fallback_provider: None,
            fallback_profile: None,
            fallback_reasoning_effort: None,
        })
        .expect("subflow outcome");

    assert_eq!(outcome.status, OutcomeStatus::Fail);
    assert!(!launched.load(Ordering::SeqCst));
}

#[test]
fn default_subflow_launch_applies_typed_input_map_to_child_context() {
    let temp = tempfile::tempdir().expect("tempdir");
    let child_source = json!({
        "schema_version": "1",
        "id": "child_input_map",
        "title": "Child Input Map",
        "nodes": {
            "start": { "kind": "start", "config": { "kind": "start" } },
            "check": {
                "kind": "agent_task",
                "config": {
                    "kind": "agent_task",
                    "prompt": "Check mapped context"
                }
            },
            "done": { "kind": "exit", "config": { "kind": "exit" } }
        },
        "edges": [{ "from": "start", "to": "check" }, { "from": "check", "to": "done" }]
    });
    write_child_flow(&temp.path().join("child"), child_source);

    let parent_flow = FlowDefinition {
        schema_version: "1".to_string(),
        id: "parent_input_map".to_string(),
        title: "Parent Input Map".to_string(),
        nodes: [(
            "run_child".to_string(),
            FlowNode {
                kind: NodeKind::Subflow,
                config: Some(NodeConfig::Subflow {
                    flow_ref: "child".to_string(),
                    output_map: BTreeMap::new(),
                    input_map: [
                        (
                            "context.child_ticket".to_string(),
                            "context.parent_ticket".to_string(),
                        ),
                        (
                            "context.child_count".to_string(),
                            "context.count".to_string(),
                        ),
                        (
                            "context.child_nested".to_string(),
                            "context.payload.ticket".to_string(),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                }),
                manager: Some(ManagerLoopConfig {
                    max_cycles: Some(1),
                    child_autostart: Some(true),
                    ..ManagerLoopConfig::default()
                }),
                ..FlowNode::default()
            },
        )]
        .into_iter()
        .collect(),
        ..FlowDefinition::default()
    };
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let paths = store
        .create_run(CreateRunRequest {
            record: RunRecord::new("parent-run", temp.path().to_string_lossy()),
            checkpoint: None,
            manifest: None,
            flow_definition_json: None,
            flow_files: None,
        })
        .expect("create parent run");
    let child_handler_observed_map = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&child_handler_observed_map);
    let mut runner = RuntimeHandlerRunner::new();
    runner.register_thread_safe_handler_fn(HANDLER_CODERGEN, move |runtime| {
        let has_map = runtime.context.get("context.child_ticket") == Some(&json!("TICKET-7"))
            && runtime.context.get("context.child_count") == Some(&json!(42))
            && runtime.context.get("context.child_nested") == Some(&json!("nested-7"));
        observed.store(has_map, Ordering::SeqCst);
        if has_map {
            Ok(Outcome::new(OutcomeStatus::Success))
        } else {
            Ok(Outcome {
                status: OutcomeStatus::Fail,
                failure_reason: format!(
                    "child context did not include mapped input values: {:?}",
                    runtime.context
                ),
                ..Outcome::new(OutcomeStatus::Fail)
            })
        }
    });

    let outcome = runner
        .execute(attractor_runtime::NodeExecutionRequest {
            node_id: "run_child".to_string(),
            stage_index: 0,
            attempt: 0,
            context: ContextMap::from([
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
            prompt: String::new(),
            node: parent_flow.nodes["run_child"].clone(),
            node_attrs: attractor_runtime::flow_runtime::node_attrs_for_handler(
                "run_child",
                &parent_flow.nodes["run_child"],
            ),
            flow: parent_flow.clone(),
            outgoing_edges: Vec::new(),
            run_paths: Some(paths),
            run_workdir: temp.path().to_path_buf(),
            run_id: "parent-run".to_string(),
            fallback_model: None,
            fallback_provider: None,
            fallback_profile: None,
            fallback_reasoning_effort: None,
        })
        .expect("subflow outcome");

    assert_eq!(outcome.status, OutcomeStatus::Success);
    assert!(child_handler_observed_map.load(Ordering::SeqCst));
}

/// Writes a child flow as a bundled flow: `dir` is `<root>/<name>`, and the
/// flow lands in `<root>/flows/<name>`, where `internal.bundled_root` =
/// `<root>` finds it.
fn write_child_flow(dir: &std::path::Path, source: serde_json::Value) {
    let root = dir.parent().expect("bundled root");
    attractor_dsl::write_flow_dir(
        &root.join("flows").join(dir.file_name().expect("flow name")),
        &attractor_dsl::validated_flow(serde_json::from_value(source).expect("flow json"))
            .expect("valid flow"),
    )
    .expect("write flow");
}

fn child_flow_source() -> serde_json::Value {
    json!({
        "schema_version": "1",
        "id": "child",
        "title": "Child",
        "nodes": {
            "start": { "kind": "start", "config": { "kind": "start" } },
            "done": { "kind": "exit", "config": { "kind": "exit" } }
        },
        "edges": [{ "from": "start", "to": "done" }]
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
    FlowNode {
        kind: NodeKind::Subflow,
        config: Some(NodeConfig::Subflow {
            flow_ref: "child".to_string(),
            output_map: BTreeMap::new(),
            input_map: BTreeMap::new(),
        }),
        manager: Some(ManagerLoopConfig {
            max_cycles: Some(1),
            child_autostart: Some(true),
            child_workdir_from: Some(child_workdir_from.to_string()),
            ..ManagerLoopConfig::default()
        }),
        ..FlowNode::default()
    }
}

fn execute_dynamic_workdir_subflow(
    node: FlowNode,
    run_workdir: &std::path::Path,
    bundled_root: &std::path::Path,
    context_entries: Vec<(String, serde_json::Value)>,
) -> (Outcome, Option<std::path::PathBuf>) {
    let flow = FlowDefinition {
        schema_version: "1".to_string(),
        id: "dynamic-workdir".to_string(),
        title: "Dynamic Workdir".to_string(),
        nodes: [("run_child".to_string(), node)].into_iter().collect(),
        ..FlowDefinition::default()
    };
    let observed_workdir = Arc::new(std::sync::Mutex::new(None::<std::path::PathBuf>));
    let recorded = Arc::clone(&observed_workdir);
    let mut runner = RuntimeHandlerRunner::new();
    runner.set_child_run_launcher(move |request| {
        *recorded.lock().expect("record child workdir") = Some(request.child_workdir.clone());
        ChildRunResult {
            run_id: request.child_run_id,
            status: "completed".to_string(),
            outcome: Some("success".to_string()),
            ..ChildRunResult::default()
        }
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
        .execute(attractor_runtime::NodeExecutionRequest {
            node_id: "run_child".to_string(),
            stage_index: 0,
            attempt: 0,
            context,
            prompt: String::new(),
            node: flow.nodes["run_child"].clone(),
            node_attrs: attractor_runtime::flow_runtime::node_attrs_for_handler(
                "run_child",
                &flow.nodes["run_child"],
            ),
            flow: flow.clone(),
            outgoing_edges: Vec::new(),
            run_paths: None,
            run_workdir: run_workdir.to_path_buf(),
            run_id: "parent-run".to_string(),
            fallback_model: None,
            fallback_provider: None,
            fallback_profile: None,
            fallback_reasoning_effort: None,
        })
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

    assert_eq!(outcome.status, OutcomeStatus::Success);
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
    let node = FlowNode {
        kind: NodeKind::Subflow,
        config: Some(NodeConfig::Subflow {
            flow_ref: "child".to_string(),
            output_map: BTreeMap::new(),
            input_map: BTreeMap::new(),
        }),
        manager: Some(ManagerLoopConfig {
            max_cycles: Some(1),
            child_autostart: Some(true),
            child_workdir: Some("worktree".to_string()),
            ..ManagerLoopConfig::default()
        }),
        ..FlowNode::default()
    };
    let flow = FlowDefinition {
        schema_version: "1".to_string(),
        id: "parent".to_string(),
        title: "Parent".to_string(),
        nodes: [("run_child".to_string(), node.clone())]
            .into_iter()
            .collect(),
        ..FlowDefinition::default()
    };
    let outcome = RuntimeHandlerRunner::new()
        .execute(NodeExecutionRequest {
            node_id: "run_child".to_string(),
            stage_index: 0,
            attempt: 0,
            context: ContextMap::from([
                ("internal.run_id".to_string(), json!("parent-run")),
                ("internal.root_run_id".to_string(), json!("parent-run")),
                ("internal.run_workdir".to_string(), json!(project)),
                ("internal.bundled_root".to_string(), json!(project)),
            ]),
            prompt: String::new(),
            node: node.clone(),
            node_attrs: attractor_runtime::flow_runtime::node_attrs_for_handler("run_child", &node),
            flow,
            outgoing_edges: Vec::new(),
            run_paths: Some(parent_paths),
            run_workdir: project.clone(),
            run_id: "parent-run".to_string(),
            fallback_model: None,
            fallback_provider: None,
            fallback_profile: None,
            fallback_reasoning_effort: None,
        })
        .expect("launch child");
    assert_eq!(outcome.status, OutcomeStatus::Success, "{outcome:?}");

    let child = store
        .list_run_records()
        .expect("list runs")
        .into_iter()
        .find(|record| record.parent_run_id.as_deref() == Some("parent-run"))
        .expect("child record");
    assert_eq!(child.project_path, project.to_string_lossy());
    assert_eq!(child.working_directory, worktree.to_string_lossy());
    assert_eq!(child.flow_name, "Child");
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

    assert_eq!(outcome.status, OutcomeStatus::Success, "{outcome:?}");
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

    assert_eq!(outcome.status, OutcomeStatus::Fail);
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

    assert_eq!(outcome.status, OutcomeStatus::Fail);
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

    assert_eq!(outcome.status, OutcomeStatus::Fail);
    assert!(
        outcome
            .failure_reason
            .contains("both manager.child_workdir and manager.child_workdir_from"),
        "{}",
        outcome.failure_reason
    );
    assert!(observed.is_none(), "child must not launch");
}

fn tool_node(
    command: &str,
    env_map: BTreeMap<String, String>,
    output_map: BTreeMap<String, String>,
) -> FlowNode {
    FlowNode {
        kind: NodeKind::Tool,
        config: Some(NodeConfig::Tool {
            command: command.to_string(),
            env_map,
            output_map,
        }),
        ..FlowNode::default()
    }
}

fn execute_tool_node(
    node: FlowNode,
    run_workdir: &std::path::Path,
    context_entries: Vec<(String, serde_json::Value)>,
) -> Outcome {
    let flow = FlowDefinition {
        schema_version: "1".to_string(),
        id: "tool-bindings".to_string(),
        title: "Tool Bindings".to_string(),
        nodes: [("run_tool".to_string(), node)].into_iter().collect(),
        ..FlowDefinition::default()
    };
    let mut context = ContextMap::from([(
        "internal.run_workdir".to_string(),
        json!(run_workdir.to_string_lossy().to_string()),
    )]);
    context.extend(context_entries);
    RuntimeHandlerRunner::new()
        .execute(attractor_runtime::NodeExecutionRequest {
            node_id: "run_tool".to_string(),
            stage_index: 0,
            attempt: 0,
            context,
            prompt: String::new(),
            node: flow.nodes["run_tool"].clone(),
            node_attrs: attractor_runtime::flow_runtime::node_attrs_for_handler(
                "run_tool",
                &flow.nodes["run_tool"],
            ),
            flow: flow.clone(),
            outgoing_edges: Vec::new(),
            run_paths: None,
            run_workdir: run_workdir.to_path_buf(),
            run_id: "tool-run".to_string(),
            fallback_model: None,
            fallback_provider: None,
            fallback_profile: None,
            fallback_reasoning_effort: None,
        })
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

    assert_eq!(outcome.status, OutcomeStatus::Success, "{outcome:?}");
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

    assert_eq!(outcome.status, OutcomeStatus::Fail);
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

    assert_eq!(outcome.status, OutcomeStatus::Fail);
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

    assert_eq!(outcome.status, OutcomeStatus::Success, "{outcome:?}");
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

    assert_eq!(outcome.status, OutcomeStatus::Fail);
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

    assert_eq!(outcome.status, OutcomeStatus::Fail);
    assert!(
        outcome.failure_reason.contains("worktree_path"),
        "{}",
        outcome.failure_reason
    );
}

// ---------------------------------------------------------------------------
// Manager-loop child invocation reset (CR-2026-0102)
// ---------------------------------------------------------------------------

fn reentry_parent_flow() -> FlowDefinition {
    FlowDefinition {
        schema_version: "1".to_string(),
        id: "parent_reentry".to_string(),
        title: "Parent Reentry".to_string(),
        nodes: [(
            "run_child".to_string(),
            FlowNode {
                kind: NodeKind::Subflow,
                config: Some(NodeConfig::Subflow {
                    flow_ref: "child".to_string(),
                    output_map: BTreeMap::new(),
                    input_map: Default::default(),
                }),
                manager: Some(ManagerLoopConfig {
                    max_cycles: Some(1),
                    child_autostart: Some(true),
                    ..ManagerLoopConfig::default()
                }),
                ..FlowNode::default()
            },
        )]
        .into_iter()
        .collect(),
        ..FlowDefinition::default()
    }
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
    paths: &attractor_runtime::RunRootPaths,
    temp: &std::path::Path,
    context: ContextMap,
) -> Outcome {
    runner
        .execute(NodeExecutionRequest {
            node_id: "run_child".to_string(),
            stage_index: 0,
            attempt: 0,
            context,
            prompt: String::new(),
            node: flow.nodes["run_child"].clone(),
            node_attrs: attractor_runtime::flow_runtime::node_attrs_for_handler(
                "run_child",
                &flow.nodes["run_child"],
            ),
            flow: flow.clone(),
            outgoing_edges: Vec::new(),
            run_paths: Some(paths.clone()),
            run_workdir: temp.to_path_buf(),
            run_id: run_id.to_string(),
            fallback_model: None,
            fallback_provider: None,
            fallback_profile: None,
            fallback_reasoning_effort: None,
        })
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

fn create_parent_run(
    store: &RunStore,
    run_id: &str,
    temp: &std::path::Path,
) -> attractor_runtime::RunRootPaths {
    store
        .create_run(CreateRunRequest {
            record: RunRecord::new(run_id, temp.to_string_lossy()),
            checkpoint: None,
            manifest: None,
            flow_definition_json: None,
            flow_files: None,
        })
        .expect("create parent run")
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
    let paths = create_parent_run(&store, "parent-run", temp.path());
    let mut runner = RuntimeHandlerRunner::new();

    let first = execute_reentry_node(
        &mut runner,
        &flow,
        "parent-run",
        &paths,
        temp.path(),
        reentry_context(temp.path(), "parent-run"),
    );
    assert_eq!(first.status, OutcomeStatus::Success);
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
    assert_eq!(second.status, OutcomeStatus::Success);
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
    let paths = create_parent_run(&store, "parent-run", temp.path());
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
    assert_eq!(second.status, OutcomeStatus::Success);
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
    let source_paths = create_parent_run(&store, "source-run", temp.path());
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

    let continuation_paths = create_parent_run(&store, "continuation-run", temp.path());
    let continued = execute_reentry_node(
        &mut runner,
        &flow,
        "continuation-run",
        &continuation_paths,
        temp.path(),
        with_outcome_updates(reentry_context(temp.path(), "continuation-run"), &source),
    );
    assert_eq!(continued.status, OutcomeStatus::Success);
    assert_ne!(linked_child_id(&continued), source_child);
    assert_eq!(
        store
            .list_child_run_bundles("continuation-run")
            .expect("continuation children")
            .len(),
        1
    );
}

/// `start → implement → evaluate`, with `implement` limited to three visits
/// and `evaluate`'s routes given by the caller.
fn visit_loop_flow(
    evaluate_edges: Vec<FlowEdge>,
    evaluate_retry_target: Option<&str>,
) -> FlowDefinition {
    let agent = |prompt: &str, runtime: Option<NodeRuntimeConfig>| FlowNode {
        kind: NodeKind::AgentTask,
        config: Some(NodeConfig::AgentTask {
            prompt: prompt.to_string(),
        }),
        runtime,
        ..FlowNode::default()
    };
    let mut edges = vec![
        FlowEdge {
            from: "start".to_string(),
            to: "implement".to_string(),
            ..FlowEdge::default()
        },
        FlowEdge {
            from: "implement".to_string(),
            to: "evaluate".to_string(),
            ..FlowEdge::default()
        },
    ];
    edges.extend(evaluate_edges);
    FlowDefinition {
        schema_version: "1".to_string(),
        id: "visit-loop".to_string(),
        title: "Visit Loop".to_string(),
        nodes: [
            (
                "start".to_string(),
                FlowNode {
                    kind: NodeKind::Start,
                    config: Some(NodeConfig::Start {}),
                    ..FlowNode::default()
                },
            ),
            (
                "implement".to_string(),
                agent(
                    "implement",
                    Some(NodeRuntimeConfig {
                        max_visits: Some(3),
                        ..NodeRuntimeConfig::default()
                    }),
                ),
            ),
            (
                "evaluate".to_string(),
                agent(
                    "evaluate",
                    evaluate_retry_target.map(|target| NodeRuntimeConfig {
                        retry_target: Some(target.to_string()),
                        ..NodeRuntimeConfig::default()
                    }),
                ),
            ),
            (
                "done".to_string(),
                FlowNode {
                    kind: NodeKind::Exit,
                    config: Some(NodeConfig::Exit {
                        result_summary: false,
                        result_summary_prompt: None,
                    }),
                    ..FlowNode::default()
                },
            ),
        ]
        .into_iter()
        .collect(),
        edges,
        ..FlowDefinition::default()
    }
}

fn edge(to: &str, label: &str, condition: &str) -> FlowEdge {
    FlowEdge {
        from: "evaluate".to_string(),
        to: to.to_string(),
        label: label.to_string(),
        condition: condition.to_string(),
        ..FlowEdge::default()
    }
}

/// Runs `flow` with `evaluate` answering `evaluate_status` and the label
/// `Again`; returns the result and the node ids executed, in order.
fn run_visit_loop(
    store: &RunStore,
    run_id: &str,
    workdir: &std::path::Path,
    flow: FlowDefinition,
    evaluate_status: OutcomeStatus,
    max_steps: Option<usize>,
    start: attractor_runtime::ExecutionStart,
) -> (attractor_runtime::PipelineExecutionResult, Vec<String>) {
    let calls = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = calls.clone();
    let mut executor = PipelineExecutor::new(move |request: NodeExecutionRequest| {
        seen.lock().unwrap().push(request.node_id.clone());
        if request.node_id == "evaluate" {
            return Ok(Outcome {
                status: evaluate_status,
                preferred_label: "Again".to_string(),
                failure_reason: "not good enough".to_string(),
                ..Outcome::new(evaluate_status)
            });
        }
        Ok(Outcome::new(OutcomeStatus::Success))
    });
    let result = executor
        .execute(ExecuteRunRequest {
            store: store.clone(),
            record: RunRecord::new(run_id, workdir.to_string_lossy()),
            flow,
            flow_definition_json: None,
            launch_context: LaunchContext::empty(),
            runtime_context: ContextMap::new(),
            max_steps,
            start,
        })
        .expect("execute visit loop");
    let calls = calls.lock().unwrap().clone();
    (result, calls)
}

fn implement_runs(calls: &[String]) -> usize {
    calls
        .iter()
        .filter(|node| node.as_str() == "implement")
        .count()
}

/// A loop with no other route fails at the target's visit limit, whatever
/// the outcome and whether it loops by edge or by retry target.
#[test]
fn loop_without_another_route_fails_when_its_target_used_all_visits() {
    for (evaluate_status, edges, retry_target) in [
        (
            OutcomeStatus::Fail,
            vec![edge("implement", "Fix", "outcome=fail")],
            None,
        ),
        (OutcomeStatus::Fail, Vec::new(), Some("implement")),
        (
            OutcomeStatus::Success,
            vec![edge("implement", "", "")],
            None,
        ),
    ] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = RunStore::for_runs_dir(temp.path().join("runs"));
        let (result, calls) = run_visit_loop(
            &store,
            "run-visit-loop",
            temp.path(),
            visit_loop_flow(edges, retry_target),
            evaluate_status,
            None,
            Default::default(),
        );
        assert_eq!(result.status, "failed", "{evaluate_status:?}");
        assert_eq!(
            result.failure_reason,
            "visit_limit_reached: implement used all 3 visits; no other route from evaluate"
        );
        assert_eq!(implement_runs(&calls), 3);
        assert_eq!(calls.iter().filter(|node| *node == "evaluate").count(), 3);
    }
}

/// A node with a "loop again" route and a "done" route takes "done" once the
/// loop target is exhausted, and its request names the unavailable target.
#[test]
fn loop_with_another_route_takes_it_when_its_target_used_all_visits() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let flow = visit_loop_flow(
        vec![edge("implement", "Again", ""), edge("done", "Done", "")],
        None,
    );
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
        }
        Ok(Outcome {
            preferred_label: "Again".to_string(),
            ..Outcome::new(OutcomeStatus::Success)
        })
    });
    let result = executor
        .execute(ExecuteRunRequest {
            store,
            record: RunRecord::new("run-visit-done", temp.path().to_string_lossy()),
            flow,
            flow_definition_json: None,
            launch_context: LaunchContext::empty(),
            runtime_context: ContextMap::new(),
            max_steps: None,
            start: Default::default(),
        })
        .expect("execute");
    assert_eq!(result.status, "completed", "{}", result.failure_reason);
    assert_eq!(result.current_node, "done");
    assert_eq!(
        *unavailable.lock().unwrap(),
        vec![None, None, Some(json!(["implement"]))]
    );
    assert!(!result
        .context
        .contains_key(attractor_core::RUNTIME_UNAVAILABLE_ROUTES_KEY));
}

/// A failed node whose conditional route is exhausted still takes its
/// retry target.
#[test]
fn exhausted_conditional_route_falls_back_to_retry_target() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let (result, calls) = run_visit_loop(
        &store,
        "run-visit-fallback",
        temp.path(),
        visit_loop_flow(vec![edge("implement", "Fix", "outcome=fail")], Some("done")),
        OutcomeStatus::Fail,
        None,
        Default::default(),
    );
    assert_eq!(result.status, "completed", "{}", result.failure_reason);
    assert_eq!(result.current_node, "done");
    assert_eq!(implement_runs(&calls), 3);
}

/// A failed node whose unconditional route into a conditional node is
/// exhausted fails with the visit-limit reason, not its own failure.
#[test]
fn exhausted_unconditional_fail_route_reports_the_visit_limit() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let mut flow = visit_loop_flow(vec![edge("check", "", "")], None);
    flow.nodes.insert(
        "check".to_string(),
        FlowNode {
            kind: NodeKind::Conditional,
            config: Some(NodeConfig::Conditional {}),
            runtime: Some(NodeRuntimeConfig {
                max_visits: Some(1),
                ..NodeRuntimeConfig::default()
            }),
            ..FlowNode::default()
        },
    );
    flow.edges.push(FlowEdge {
        from: "check".to_string(),
        to: "implement".to_string(),
        condition: "outcome=fail".to_string(),
        ..FlowEdge::default()
    });
    let (result, calls) = run_visit_loop(
        &store,
        "run-visit-unconditional",
        temp.path(),
        flow,
        OutcomeStatus::Fail,
        None,
        Default::default(),
    );
    assert_eq!(result.status, "failed");
    assert_eq!(
        result.failure_reason,
        "visit_limit_reached: check used all 1 visits; no other route from evaluate"
    );
    assert_eq!(implement_runs(&calls), 2);
}

/// A failed node with no retry target whose unconditional route is exhausted
/// fails with the visit-limit reason, not its own failure.
#[test]
fn exhausted_unconditional_route_from_a_failed_node_reports_the_visit_limit() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let mut flow = visit_loop_flow(vec![edge("implement", "", "")], None);
    flow.nodes.get_mut("implement").unwrap().runtime = Some(NodeRuntimeConfig {
        max_visits: Some(1),
        ..NodeRuntimeConfig::default()
    });
    let (result, calls) = run_visit_loop(
        &store,
        "run-visit-fail-unconditional",
        temp.path(),
        flow,
        OutcomeStatus::Fail,
        None,
        Default::default(),
    );
    assert_eq!(result.status, "failed");
    assert_eq!(
        result.failure_reason,
        "visit_limit_reached: implement used all 1 visits; no other route from evaluate"
    );
    assert_eq!(implement_runs(&calls), 1);
}

/// Recovery that routes past a completed node fails when its only route is
/// exhausted, instead of inferring completion from the prior outcome.
#[test]
fn resume_routing_into_an_exhausted_target_fails() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let flow = || visit_loop_flow(vec![edge("implement", "", "")], None);
    let (paused, _) = run_visit_loop(
        &store,
        "run-visit-resume",
        temp.path(),
        flow(),
        OutcomeStatus::Success,
        Some(6),
        Default::default(),
    );
    assert_eq!(paused.status, "paused");
    let meta = store.read_run_meta("run-visit-resume").unwrap().unwrap();
    let checkpoint = meta.checkpoint.unwrap();
    assert_eq!(checkpoint.current_node, "evaluate");
    assert_eq!(checkpoint.visit_counts["implement"], 3);

    let (resumed, calls) = run_visit_loop(
        &store,
        "run-visit-resume",
        temp.path(),
        flow(),
        OutcomeStatus::Success,
        None,
        attractor_runtime::ExecutionStart::Resume {
            paths: meta.paths,
            checkpoint,
        },
    );
    assert_eq!(resumed.status, "failed");
    assert_eq!(
        resumed.failure_reason,
        "visit_limit_reached: implement used all 3 visits; no other route from evaluate"
    );
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

/// Visit counts are run-wide: neither resuming a paused or crashed run nor
/// continuing it as a new run grants the loop target fresh visits.
#[test]
fn visit_counts_survive_resume_and_continue() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let flow = || visit_loop_flow(vec![edge("implement", "Fix", "outcome=fail")], None);
    let read_checkpoint = |run_id: &str| {
        let meta = store.read_run_meta(run_id).unwrap().unwrap();
        (meta.paths, meta.checkpoint.unwrap())
    };

    // Pause after routing back into implement: two of its three visits used.
    // Resume, as crash recovery does, and continue both start from here.
    let (paused, calls) = run_visit_loop(
        &store,
        "run-visits",
        temp.path(),
        flow(),
        OutcomeStatus::Fail,
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
        OutcomeStatus::Fail,
        None,
        attractor_runtime::ExecutionStart::Resume {
            paths,
            checkpoint: checkpoint.clone(),
        },
    );
    assert_eq!(resumed.status, "failed");
    assert!(resumed
        .failure_reason
        .starts_with("visit_limit_reached: implement"));
    // Resume routes past the checkpointed implement (it is in completion
    // history), so only its third visit runs before the limit.
    assert_eq!(calls, vec!["evaluate", "implement", "evaluate"]);

    // Continue from implement: its third visit is the continuation's start.
    let (continued, calls) = run_visit_loop(
        &store,
        "run-visits-continued",
        temp.path(),
        flow(),
        OutcomeStatus::Fail,
        None,
        attractor_runtime::ExecutionStart::FromCheckpoint {
            start_node: "implement".to_string(),
            checkpoint,
            flow_files: None,
        },
    );
    assert_eq!(continued.status, "failed");
    assert!(continued
        .failure_reason
        .starts_with("visit_limit_reached: implement"));
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
    // Exercise all completion callers: dead end, successful exit, and failed goal gate.
    for (with_exit, failed_goal) in [(false, false), (true, false), (true, true)] {
        let temp = tempfile::tempdir().unwrap();
        let store = RunStore::for_runs_dir(temp.path());
        let mut flow = linear_flow(FlowNode {
            kind: NodeKind::AgentTask,
            config: Some(NodeConfig::AgentTask {
                prompt: "blocked".into(),
            }),
            runtime: Some(NodeRuntimeConfig {
                goal_gate: failed_goal,
                ..NodeRuntimeConfig::default()
            }),
            ..FlowNode::default()
        });
        if !with_exit {
            flow.nodes.remove("done");
            flow.edges.retain(|edge| edge.from != "task");
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
                    if failed_goal {
                        return Ok(Outcome::new(OutcomeStatus::Fail));
                    }
                }
                Ok(Outcome::new(OutcomeStatus::Success))
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
            "with_exit={with_exit}, failed_goal={failed_goal}"
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
        let node = FlowNode {
            kind: NodeKind::Subflow,
            config: Some(NodeConfig::Subflow {
                flow_ref: "child".to_string(),
                input_map: BTreeMap::new(),
                output_map: BTreeMap::from([
                    ("context.child.a".to_string(), "context.a".to_string()),
                    ("context.child.b".to_string(), "context.b".to_string()),
                ]),
            }),
            contracts: Some(NodeContracts {
                writes_context: vec!["context.a".to_string(), "context.b".to_string()],
                ..NodeContracts::default()
            }),
            manager: Some(ManagerLoopConfig {
                max_cycles: Some(1),
                child_autostart: Some(true),
                ..ManagerLoopConfig::default()
            }),
            ..FlowNode::default()
        };
        let flow = FlowDefinition {
            id: "output-map".to_string(),
            nodes: [("run_child".to_string(), node.clone())]
                .into_iter()
                .collect(),
            ..FlowDefinition::default()
        };
        let mut runner = RuntimeHandlerRunner::new();
        runner.set_child_run_launcher(move |request| ChildRunResult {
            run_id: request.child_run_id,
            status: status.to_string(),
            context: ContextMap::from([("context.child.a".to_string(), json!("A"))]),
            ..ChildRunResult::default()
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
            .execute(NodeExecutionRequest {
                node_id: "run_child".to_string(),
                stage_index: 0,
                attempt: 0,
                context: parent_context.clone(),
                prompt: String::new(),
                node,
                node_attrs: node_attrs.clone(),
                flow,
                outgoing_edges: Vec::new(),
                run_paths: None,
                run_workdir: temp.path().to_path_buf(),
                run_id: "parent-run".to_string(),
                fallback_model: None,
                fallback_provider: None,
                fallback_profile: None,
                fallback_reasoning_effort: None,
            })
            .expect("subflow outcome");
        assert_eq!(outcome.context_updates["context.a"], json!("A"), "{status}");
        assert_eq!(
            outcome.context_updates["context.b"],
            json!(null),
            "{status}"
        );

        // The parent's write contract admits them, and a null clears the key.
        let mut context = attractor_core::AttractorContext::from_map(parent_context).unwrap();
        let applied = attractor_runtime::apply_outcome_context_updates(
            "run_child",
            &node_attrs,
            &mut context,
            &outcome,
        )
        .expect("apply");
        assert_ne!(
            applied.failure_kind,
            Some(attractor_core::FailureKind::Contract)
        );
        assert_eq!(context.get("context.a"), Some(&json!("A")));
        assert_eq!(context.get("context.b"), None, "{status}");
    }
}
