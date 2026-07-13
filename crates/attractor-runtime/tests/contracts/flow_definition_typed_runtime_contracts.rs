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
                    config: Some(NodeConfig::Exit {}),
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
            flow_source: None,
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
    runner.register_static_handler(HANDLER_CODERGEN, Outcome::new(OutcomeStatus::Success));
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = RunStore::for_runs_dir(temp.path().join("runs"))
        .create_run(CreateRunRequest {
            record: RunRecord::new("run-typed-parallel", temp.path().to_string_lossy()),
            checkpoint: None,
            manifest: None,
            flow_source: None,
            flow_definition_json: None,
        })
        .expect("create run");

    let outcome = runner
        .execute(attractor_runtime::NodeExecutionRequest {
            node_id: "fan".to_string(),
            stage_index: 0,
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
            flow_ref: "typed-child.yaml".to_string(),
            input_map: BTreeMap::new(),
        }),
        manager: Some(ManagerLoopConfig {
            child_autostart: Some(false),
            ..ManagerLoopConfig::default()
        }),
        context: [
            (
                "stack.child_flow_ref".to_string(),
                json!("legacy-child.yaml"),
            ),
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
        Some("typed-child.yaml")
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
                extensions: [("stack.child_flow_ref".to_string(), json!("child.yaml"))]
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
    fs::write(
        temp.path().join("child.yaml"),
        r#"
schema_version: "1"
id: child
title: Child
nodes:
  start:
    kind: start
    config:
      kind: start
  done:
    kind: exit
    config:
      kind: exit
edges:
  - from: start
    to: done
"#,
    )
    .expect("write child flow");
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
    let child_source = r#"
schema_version: "1"
id: child_input_map
title: Child Input Map
nodes:
  start:
    kind: start
    config:
      kind: start
  check:
    kind: agent_task
    config:
      kind: agent_task
      prompt: Check mapped context
  done:
    kind: exit
    config:
      kind: exit
edges:
  - from: start
    to: check
  - from: check
    to: done
"#;
    fs::write(temp.path().join("child.yaml"), child_source).expect("write child flow");

    let parent_flow = FlowDefinition {
        schema_version: "1".to_string(),
        id: "parent_input_map".to_string(),
        title: "Parent Input Map".to_string(),
        nodes: [(
            "run_child".to_string(),
            FlowNode {
                kind: NodeKind::Subflow,
                config: Some(NodeConfig::Subflow {
                    flow_ref: "child.yaml".to_string(),
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
            flow_source: None,
            flow_definition_json: None,
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
            context: ContextMap::from([
                ("context.parent_ticket".to_string(), json!("TICKET-7")),
                ("context.count".to_string(), json!(42)),
                (
                    "context.payload".to_string(),
                    json!({ "ticket": "nested-7" }),
                ),
                (
                    "internal.flow_source_dir".to_string(),
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
