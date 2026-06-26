use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;

use attractor_core::{
    ContextMap, DotGraph, FailureKind, LaunchContext, Outcome, OutcomeStatus, RunRecord,
};
use attractor_dsl::parse_dot;
use attractor_runtime::{
    check_goal_gates, coerce_retry_exhausted_outcome, read_checkpoint, read_run_record,
    retry_policy_for_node, should_retry_outcome, BackoffConfig, ExecuteRunRequest,
    NodeExecutionRequest, PipelineExecutor, RunStore,
};
use serde_json::{json, Value};

fn parse_graph(dot: &str) -> DotGraph {
    parse_dot(dot).expect("dot parses")
}

fn temp_store(temp: &tempfile::TempDir) -> RunStore {
    RunStore::for_runs_dir(temp.path().join("spark-home/attractor/runs"))
}

fn record(run_id: &str, project_path: &Path) -> RunRecord {
    let mut record = RunRecord::new(run_id, project_path.to_string_lossy());
    record.flow_name = "runtime-contract".to_string();
    record.model = "compat-model".to_string();
    record.started_at = "2026-06-23T10:00:00Z".to_string();
    record
}

fn execute_with<F>(
    graph: DotGraph,
    temp: &tempfile::TempDir,
    run_id: &str,
    node_executor: F,
) -> attractor_runtime::PipelineExecutionResult
where
    F: FnMut(
        NodeExecutionRequest,
    ) -> std::result::Result<Outcome, attractor_runtime::RuntimeNodeError>,
{
    let project_path = temp.path().join("Project Runtime");
    std::fs::create_dir_all(&project_path).expect("project dir");
    let store = temp_store(temp);
    let mut executor = PipelineExecutor::new(node_executor);
    executor
        .execute(ExecuteRunRequest {
            store,
            record: record(run_id, &project_path),
            graph,
            graph_source: None,
            graph_dot: None,
            launch_context: LaunchContext::empty(),
            runtime_context: Default::default(),
            max_steps: None,
            start: Default::default(),
        })
        .expect("execute pipeline")
}

fn success() -> Outcome {
    Outcome::new(OutcomeStatus::Success)
}

fn context_updates(entries: impl IntoIterator<Item = (&'static str, Value)>) -> ContextMap {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

#[test]
fn retry_policy_presets_and_attempt_math_match_contract() {
    let graph = parse_graph(
        r#"
        digraph G {
          graph [default_max_retries=2]
          start [shape=Mdiamond]
          defaulted [shape=box]
          explicit [shape=box, max_retries=4]
          linear [shape=box, retry_policy="linear", max_retries=9]
          none [shape=box, retry_policy="none"]
          done [shape=Msquare]
          start -> defaulted -> explicit -> linear -> none -> done
        }
        "#,
    );

    assert_eq!(retry_policy_for_node(&graph, "defaulted").max_attempts, 3);
    assert_eq!(retry_policy_for_node(&graph, "explicit").max_attempts, 5);
    let linear = retry_policy_for_node(&graph, "linear");
    assert_eq!(linear.max_attempts, 3);
    assert_eq!(linear.backoff.initial_delay_ms, 500);
    assert_eq!(linear.backoff.backoff_factor, 1.0);
    assert!(!linear.backoff.jitter);
    assert_eq!(retry_policy_for_node(&graph, "none").max_attempts, 1);

    let no_jitter = BackoffConfig {
        initial_delay_ms: 200,
        backoff_factor: 2.0,
        max_delay_ms: 700,
        jitter: false,
    };
    assert_eq!(no_jitter.delay_for_attempt(1), 200.0);
    assert_eq!(no_jitter.delay_for_attempt(2), 400.0);
    assert_eq!(no_jitter.delay_for_attempt(3), 700.0);
    assert_eq!(no_jitter.delay_for_attempt(4), 700.0);

    let jittered = BackoffConfig {
        initial_delay_ms: 500,
        backoff_factor: 3.0,
        max_delay_ms: 1_000,
        jitter: true,
    };
    assert_eq!(
        jittered.delay_for_attempt_with_jitter_factor(2, 1.5),
        1_500.0
    );
    let sampled_delay = jittered.delay_for_attempt(2);
    assert!(
        (500.0..=1_500.0).contains(&sampled_delay),
        "jittered retry delay {sampled_delay} should stay within Python-compatible uniform range",
    );

    assert!(should_retry_outcome(&Outcome::new(OutcomeStatus::Retry)));
    assert!(should_retry_outcome(&Outcome {
        status: OutcomeStatus::Fail,
        failure_kind: Some(FailureKind::Runtime),
        ..Outcome::new(OutcomeStatus::Fail)
    }));
    assert!(!should_retry_outcome(&Outcome {
        status: OutcomeStatus::Fail,
        retryable: Some(false),
        ..Outcome::new(OutcomeStatus::Fail)
    }));
    assert!(!should_retry_outcome(&Outcome {
        status: OutcomeStatus::Fail,
        failure_kind: Some(FailureKind::Contract),
        ..Outcome::new(OutcomeStatus::Fail)
    }));
}

#[test]
fn failure_routes_through_conditional_target_using_prior_stage_outcome() {
    let graph = parse_graph(
        r#"
        digraph G {
          start [shape=Mdiamond]
          plan [shape=box]
          gate [shape=diamond]
          fix [shape=box]
          done [shape=Msquare]

          start -> plan
          plan -> gate
          gate -> done [condition="outcome=success"]
          gate -> fix [condition="outcome=fail"]
          fix -> done
        }
        "#,
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let result = execute_with(graph, &temp, "run-conditional-prior-fail", move |request| {
        if request.node_id == "plan" {
            return Ok(Outcome {
                status: OutcomeStatus::Fail,
                failure_reason: "needs fix".to_string(),
                retryable: Some(false),
                ..Outcome::new(OutcomeStatus::Fail)
            });
        }
        Ok(success())
    });

    assert_eq!(result.status, "completed");
    assert_eq!(result.route_trace, ["start", "plan", "gate", "fix", "done"]);
    assert_eq!(result.node_outcomes["gate"].status, OutcomeStatus::Success);
}

#[test]
fn executor_retries_retryable_failures_and_persists_terminal_success() {
    let graph = parse_graph(
        r#"
        digraph RetryGoalFixture {
          graph [retry_target="implement"];
          start [shape=Mdiamond];
          implement [shape=box, max_retries=1, goal_gate=true, spark.writes_context="[\"context.fixed\"]"];
          done [shape=Msquare];
          start -> implement;
          implement -> done;
        }
        "#,
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let calls = Rc::new(RefCell::new(BTreeMap::<String, u64>::new()));
    let calls_for_runner = calls.clone();
    let result = execute_with(graph, &temp, "run-retry-success", move |request| {
        let mut calls = calls_for_runner.borrow_mut();
        let count = calls.entry(request.node_id.clone()).or_insert(0);
        *count += 1;
        if request.node_id == "implement" && *count == 1 {
            return Ok(Outcome {
                status: OutcomeStatus::Fail,
                failure_reason: "try again".to_string(),
                retryable: Some(true),
                ..Outcome::new(OutcomeStatus::Fail)
            });
        }
        if request.node_id == "implement" {
            return Ok(Outcome {
                context_updates: context_updates([("context.fixed", json!(true))]),
                raw_response_text: "fixed response".to_string(),
                ..success()
            });
        }
        Ok(success())
    });

    assert_eq!(result.status, "completed");
    assert_eq!(result.outcome.as_deref(), Some("success"));
    assert_eq!(result.route_trace, ["start", "implement", "done"]);
    assert_eq!(calls.borrow().get("implement"), Some(&2));
    assert_eq!(result.context.get("context.fixed"), Some(&json!(true)));

    let project_path = temp.path().join("Project Runtime");
    let store = temp_store(&temp);
    let paths = store
        .run_root(&project_path.to_string_lossy(), "run-retry-success")
        .expect("paths");
    let record = read_run_record(&paths)
        .expect("read record")
        .expect("run record");
    assert_eq!(record.status, "completed");
    assert_eq!(record.outcome.as_deref(), Some("success"));
    assert!(record.last_error.is_empty());

    let checkpoint = read_checkpoint(&paths)
        .expect("read checkpoint")
        .expect("checkpoint");
    assert_eq!(checkpoint.current_node, "done");
    assert_eq!(checkpoint.completed_nodes, ["start", "implement"]);
    assert!(checkpoint.retry_counts.is_empty());

    let events = store.read_raw_events(&paths).expect("events");
    assert!(events
        .iter()
        .any(|event| event.event_type == "StageRetrying"
            && event.payload.get("node_id") == Some(&json!("implement"))
            && event.payload.get("attempt") == Some(&json!(1))));
    assert!(events
        .iter()
        .any(|event| event.event_type == "PipelineCompleted"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "PipelineCompleted"
            && event.payload.get("artifact_count") == Some(&json!(3))));
    let journal = store.read_journal(&paths).expect("journal");
    assert!(journal
        .iter()
        .any(|entry| entry.summary == "Stage implement retrying (attempt 1)"));
    let status: Value = serde_json::from_str(
        &std::fs::read_to_string(paths.logs_dir().join("implement/status.json"))
            .expect("status json"),
    )
    .expect("status payload");
    assert_eq!(status["outcome"], json!("success"));
    assert_eq!(status["status_transitions"], json!(["fail", "success"]));
    assert_eq!(
        std::fs::read_to_string(paths.logs_dir().join("implement/response.md")).expect("response"),
        "fixed response\n"
    );
    assert_eq!(
        store
            .read_result(&paths)
            .expect("read result")
            .expect("result")
            .body_markdown,
        "fixed response\n"
    );
}

#[test]
fn retry_exhaustion_and_allow_partial_preserve_routing_outcomes() {
    let graph = parse_graph(
        r#"
        digraph G {
          start [shape=Mdiamond]
          task [shape=box, max_retries=1, allow_partial=true]
          done [shape=Msquare]
          start -> task
          task -> done [condition="outcome=partial_success"]
        }
        "#,
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let result = execute_with(graph.clone(), &temp, "run-partial", move |request| {
        if request.node_id == "task" {
            Ok(Outcome {
                status: OutcomeStatus::Retry,
                failure_reason: "stuck".to_string(),
                ..Outcome::new(OutcomeStatus::Retry)
            })
        } else {
            Ok(success())
        }
    });

    assert_eq!(result.status, "completed");
    assert_eq!(result.route_trace, ["start", "task", "done"]);
    assert_eq!(
        result.node_outcomes["task"].status,
        OutcomeStatus::PartialSuccess
    );
    assert_eq!(
        result.node_outcomes["task"].notes,
        "retries exhausted, partial accepted"
    );

    let node = &graph.nodes["task"];
    let non_retryable = Outcome {
        status: OutcomeStatus::Fail,
        retryable: Some(false),
        failure_reason: "do not retry".to_string(),
        ..Outcome::new(OutcomeStatus::Fail)
    };
    let coerced = coerce_retry_exhausted_outcome(&graph, &node.node_id, &non_retryable, 1, 1);
    assert_eq!(coerced.status, OutcomeStatus::Fail);
    assert_eq!(coerced.failure_reason, "do not retry");
}

#[test]
fn failure_routing_prefers_fail_edges_then_conditions_then_node_targets() {
    let exact_graph = parse_graph(
        r#"
        digraph G {
          start [shape=Mdiamond]
          task [shape=box, max_retries=0, retry_target="fix"]
          review [shape=box]
          fix [shape=box]
          done [shape=Msquare]
          start -> task
          task -> done [condition=" outcome = fail "]
          task -> review [condition="context.force_route=true", weight=10]
          review -> done
          fix -> done
        }
        "#,
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let result = execute_with(exact_graph, &temp, "run-exact-fail", move |request| {
        if request.node_id == "task" {
            return Ok(Outcome {
                status: OutcomeStatus::Fail,
                failure_reason: "permanent".to_string(),
                retryable: Some(false),
                context_updates: context_updates([("context.force_route", json!(true))]),
                ..Outcome::new(OutcomeStatus::Fail)
            });
        }
        Ok(success())
    });
    assert_eq!(result.status, "completed");
    assert_eq!(result.route_trace, ["start", "task", "done"]);

    let target_graph = parse_graph(
        r#"
        digraph G {
          graph [retry_target="graph_fix"]
          start [shape=Mdiamond]
          task [shape=box, max_retries=0, retry_target="node_fix"]
          node_fix [shape=box]
          graph_fix [shape=box]
          done [shape=Msquare]
          start -> task
          task -> done
          node_fix -> done
          graph_fix -> done
        }
        "#,
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let result = execute_with(target_graph, &temp, "run-node-target", move |request| {
        if request.node_id == "task" {
            return Ok(Outcome {
                status: OutcomeStatus::Fail,
                failure_reason: "permanent".to_string(),
                retryable: Some(false),
                ..Outcome::new(OutcomeStatus::Fail)
            });
        }
        Ok(success())
    });
    assert_eq!(result.status, "completed");
    assert_eq!(result.route_trace, ["start", "task", "node_fix", "done"]);
}

#[test]
fn non_goal_failures_without_failure_routes_become_runtime_failed_runs() {
    let graph = parse_graph(
        r#"
        digraph G {
          graph [retry_target="graph_fix"]
          start [shape=Mdiamond]
          task [shape=box, max_retries=0]
          graph_fix [shape=box]
          done [shape=Msquare]
          start -> task
          task -> done
          graph_fix -> done
        }
        "#,
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let result = execute_with(graph, &temp, "run-runtime-fail", move |request| {
        if request.node_id == "task" {
            return Ok(Outcome {
                status: OutcomeStatus::Fail,
                failure_reason: "permanent".to_string(),
                retryable: Some(false),
                ..Outcome::new(OutcomeStatus::Fail)
            });
        }
        Ok(success())
    });

    assert_eq!(result.status, "failed");
    assert_eq!(result.current_node, "task");
    assert_eq!(result.failure_reason, "permanent");
    assert_eq!(result.route_trace, ["start", "task"]);

    let project_path = temp.path().join("Project Runtime");
    let paths = temp_store(&temp)
        .run_root(&project_path.to_string_lossy(), "run-runtime-fail")
        .expect("paths");
    let record = read_run_record(&paths)
        .expect("record")
        .expect("run record");
    assert_eq!(record.status, "failed");
    assert_eq!(record.outcome, None);
    assert_eq!(record.last_error, "permanent");
}

#[test]
fn goal_gate_failures_route_to_recovery_or_complete_as_workflow_failure() {
    let graph = parse_graph(
        r#"
        digraph G {
          graph [retry_target="fix"]
          start [shape=Mdiamond]
          implement [shape=box, goal_gate=true, max_retries=0]
          fix [shape=box]
          done [shape=Msquare]
          start -> implement
          implement -> done
          fix -> implement
        }
        "#,
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let calls = Rc::new(RefCell::new(0u64));
    let calls_for_runner = calls.clone();
    let result = execute_with(graph, &temp, "run-goal-retry", move |request| {
        if request.node_id == "implement" {
            let mut calls = calls_for_runner.borrow_mut();
            *calls += 1;
            if *calls == 1 {
                return Ok(Outcome {
                    status: OutcomeStatus::Fail,
                    failure_reason: "needs fix".to_string(),
                    retryable: Some(false),
                    ..Outcome::new(OutcomeStatus::Fail)
                });
            }
        }
        Ok(success())
    });
    assert_eq!(result.status, "completed");
    assert_eq!(result.outcome.as_deref(), Some("success"));
    assert_eq!(
        result.route_trace,
        ["start", "implement", "done", "fix", "implement", "done"]
    );

    let graph = parse_graph(
        r#"
        digraph G {
          start [shape=Mdiamond]
          implement [shape=box, goal_gate=true, max_retries=0]
          done [shape=Msquare]
          start -> implement
          implement -> done
        }
        "#,
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let result = execute_with(graph.clone(), &temp, "run-goal-failure", move |request| {
        if request.node_id == "implement" {
            return Ok(Outcome {
                status: OutcomeStatus::Fail,
                failure_reason: "needs fix".to_string(),
                retryable: Some(false),
                ..Outcome::new(OutcomeStatus::Fail)
            });
        }
        Ok(success())
    });
    assert_eq!(result.status, "completed");
    assert_eq!(result.outcome.as_deref(), Some("failure"));
    assert_eq!(
        result.outcome_reason_code.as_deref(),
        Some("goal_gate_unsatisfied")
    );
    assert_eq!(
        result.outcome_reason_message.as_deref(),
        Some("Goal gate unsatisfied and no retry target")
    );
    assert_eq!(result.route_trace, ["start", "implement", "done"]);

    let gate_check = check_goal_gates(
        &graph,
        &attractor_core::AttractorContext::from_map(BTreeMap::from([(
            "_attractor.node_outcomes".to_string(),
            json!({"unvisited": "fail"}),
        )]))
        .expect("context"),
        &[],
    );
    assert!(gate_check.satisfied);

    let invalid_graph = parse_graph(
        r#"
        digraph G {
          start [shape=Mdiamond]
          implement [shape=box, goal_gate=true, max_retries=0, spark.writes_context="[\"context.workflow_outcome\"]"]
          done [shape=Msquare]
          start -> implement
          implement -> done
        }
        "#,
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let result = execute_with(invalid_graph, &temp, "run-goal-invalid", move |request| {
        if request.node_id == "implement" {
            return Ok(Outcome {
                status: OutcomeStatus::Fail,
                failure_reason: "needs fix".to_string(),
                retryable: Some(false),
                context_updates: context_updates([("context.workflow_outcome", json!("maybe"))]),
                ..Outcome::new(OutcomeStatus::Fail)
            });
        }
        Ok(success())
    });
    assert_eq!(result.status, "failed");
    assert_eq!(
        result.failure_reason,
        "invalid context.workflow_outcome: maybe"
    );
}

#[test]
fn terminal_workflow_context_distinguishes_completed_failure_from_runtime_failure() {
    let failure_graph = parse_graph(
        r#"
        digraph G {
          start [shape=Mdiamond]
          decide [shape=box, spark.writes_context="[\"context.workflow_outcome\", \"context.workflow_outcome_reason_code\", \"context.workflow_outcome_reason_message\"]"]
          done [shape=Msquare]
          start -> decide -> done
        }
        "#,
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let result = execute_with(
        failure_graph,
        &temp,
        "run-workflow-failure",
        move |request| {
            if request.node_id == "decide" {
                return Ok(Outcome {
                    context_updates: context_updates([
                        ("context.workflow_outcome", json!("failure")),
                        (
                            "context.workflow_outcome_reason_code",
                            json!("business_rule"),
                        ),
                        (
                            "context.workflow_outcome_reason_message",
                            json!("not ready"),
                        ),
                    ]),
                    ..success()
                });
            }
            Ok(success())
        },
    );
    assert_eq!(result.status, "completed");
    assert_eq!(result.outcome.as_deref(), Some("failure"));
    assert_eq!(result.outcome_reason_code.as_deref(), Some("business_rule"));

    let project_path = temp.path().join("Project Runtime");
    let paths = temp_store(&temp)
        .run_root(&project_path.to_string_lossy(), "run-workflow-failure")
        .expect("paths");
    let record = read_run_record(&paths)
        .expect("record")
        .expect("run record");
    assert_eq!(record.status, "completed");
    assert_eq!(record.outcome.as_deref(), Some("failure"));
    assert!(record.last_error.is_empty());

    let invalid_graph = parse_graph(
        r#"
        digraph G {
          start [shape=Mdiamond]
          decide [shape=box, spark.writes_context="[\"context.workflow_outcome\"]"]
          done [shape=Msquare]
          start -> decide -> done
        }
        "#,
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let result = execute_with(
        invalid_graph,
        &temp,
        "run-invalid-workflow",
        move |request| {
            if request.node_id == "decide" {
                return Ok(Outcome {
                    context_updates: context_updates([(
                        "context.workflow_outcome",
                        json!("maybe"),
                    )]),
                    ..success()
                });
            }
            Ok(success())
        },
    );
    assert_eq!(result.status, "failed");
    assert_eq!(
        result.failure_reason,
        "invalid context.workflow_outcome: maybe"
    );
}
