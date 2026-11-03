//! Step-2 routing contract: declared decisions, exact dispatch, visit
//! limits, execution errors, tool adapters, switches, declared exits and
//! structured forks, through root and branch runners.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use attractor_core::{ContextMap, FailureKind, FlowDefinition, LaunchContext, Outcome, RunRecord};
use attractor_runtime::{
    ExecuteRunRequest, ExecutionStart, HandlerRuntime, PipelineExecutionResult, PipelineExecutor,
    RunStore, RuntimeHandlerRunner, HANDLER_CODERGEN, HANDLER_TOOL,
};
use serde_json::{json, Value};

fn route(to: &str) -> Value {
    json!({"to": to, "description": format!("Continue at {to}.")})
}

fn agent(routes: Value) -> Value {
    json!({"kind": "agent_task", "contracts": {"response": "decision_envelope"}, "routes": routes})
}

fn exit(name: &str, outcome: &str) -> Value {
    json!({"kind": "exit", "config": {"kind": "exit", "result": {"name": name, "outcome": outcome}}})
}

fn parse_flow(nodes: Value) -> FlowDefinition {
    let flow: FlowDefinition =
        serde_json::from_value(json!({"schema_version": "2", "id": "contract", "nodes": nodes}))
            .expect("flow JSON");
    flow.validate().expect("valid flow");
    flow
}

/// Node id -> answers consumed in order (the last one repeats).
type Script = BTreeMap<String, VecDeque<Outcome>>;

fn script(entries: &[(&str, Vec<Outcome>)]) -> Arc<Mutex<Script>> {
    Arc::new(Mutex::new(
        entries
            .iter()
            .map(|(node, answers)| (node.to_string(), answers.iter().cloned().collect()))
            .collect(),
    ))
}

fn answer(script: &Mutex<Script>, calls: &Mutex<Vec<String>>, runtime: &HandlerRuntime) -> Outcome {
    calls.lock().unwrap().push(runtime.node_id.clone());
    let mut script = script.lock().unwrap();
    let answers = script
        .get_mut(&runtime.node_id)
        .unwrap_or_else(|| panic!("no scripted answer for {}", runtime.node_id));
    if answers.len() > 1 {
        answers.pop_front().unwrap()
    } else {
        answers.front().cloned().expect("answer")
    }
}

/// A runner whose agent and tool nodes answer from `script`.
fn scripted_runner(
    script: Arc<Mutex<Script>>,
    script_tools: bool,
) -> (RuntimeHandlerRunner, Arc<Mutex<Vec<String>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut runner = RuntimeHandlerRunner::new();
    let mut handler_types = vec![HANDLER_CODERGEN];
    if script_tools {
        handler_types.push(HANDLER_TOOL);
    }
    for handler_type in handler_types {
        let (script, calls) = (script.clone(), calls.clone());
        runner.register_thread_safe_handler_fn(handler_type, move |runtime| {
            Ok(answer(&script, &calls, &runtime))
        });
    }
    (runner, calls)
}

struct Run {
    result: PipelineExecutionResult,
    store: RunStore,
    _temp: tempfile::TempDir,
    workdir: PathBuf,
}

fn execute(
    flow: FlowDefinition,
    runner: RuntimeHandlerRunner,
    start: Option<ExecutionStart>,
) -> Run {
    let temp = tempfile::tempdir().expect("tempdir");
    execute_in(temp, flow, runner, start)
}

fn execute_in(
    temp: tempfile::TempDir,
    flow: FlowDefinition,
    runner: RuntimeHandlerRunner,
    start: Option<ExecutionStart>,
) -> Run {
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let workdir = temp.path().join("work");
    std::fs::create_dir_all(&workdir).unwrap();
    let result = PipelineExecutor::new(runner)
        .execute(ExecuteRunRequest {
            store: store.clone(),
            record: RunRecord::new("run-contract", workdir.to_string_lossy()),
            flow,
            flow_definition_json: None,
            launch_context: LaunchContext::empty(),
            runtime_context: ContextMap::new(),
            max_steps: None,
            start: start.unwrap_or_default(),
        })
        .expect("execute");
    Run {
        result,
        store,
        _temp: temp,
        workdir,
    }
}

impl Run {
    fn record(&self) -> RunRecord {
        let paths = self.store.find_run_root("run-contract").unwrap().unwrap();
        self.store.read_run_record(&paths).unwrap().unwrap()
    }

    fn status_json(&self, node: &str, stage: u64, attempt: u64) -> Value {
        let paths = self.store.find_run_root("run-contract").unwrap().unwrap();
        let root = self
            .store
            .node_execution_root(&paths, node, stage, attempt)
            .unwrap();
        serde_json::from_str(&std::fs::read_to_string(root.join("status.json")).unwrap()).unwrap()
    }

    fn events(&self, event_type: &str) -> Vec<Value> {
        let paths = self.store.find_run_root("run-contract").unwrap().unwrap();
        self.store
            .read_raw_events(&paths)
            .unwrap()
            .into_iter()
            .filter(|event| event.event_type == event_type)
            .map(|event| Value::Object(event.payload.into_iter().collect()))
            .collect()
    }
}

fn count(calls: &Mutex<Vec<String>>, node: &str) -> usize {
    calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| *call == node)
        .count()
}

fn completed(decision: &str) -> Outcome {
    Outcome::completed(decision)
}

fn review_loop(implement_visits: u64) -> FlowDefinition {
    parse_flow(json!({
        "start": {"kind": "start", "routes": {"next": route("implement")}},
        "implement": {
            "kind": "agent_task", "contracts": {"response": "decision_envelope"},
            "runtime": {"max_visits": implement_visits},
            "routes": {"ready_for_review": route("review"), "unfinished": route("implement"), "blocked": route("failed")}
        },
        "review": agent(json!({"ready": route("done"), "fix": route("implement"), "blocked": route("failed")})),
        "done": exit("completed", "success"),
        "failed": exit("blocked", "failure"),
    }))
}

#[test]
fn decisions_dispatch_by_exact_lookup_and_unfinished_work_cannot_skip_review() {
    let answers = script(&[
        (
            "implement",
            vec![completed("unfinished"), completed("ready_for_review")],
        ),
        ("review", vec![completed("fix"), completed("ready")]),
    ]);
    // implement: unfinished, ready_for_review, then (after fix) ready_for_review.
    answers
        .lock()
        .unwrap()
        .get_mut("implement")
        .unwrap()
        .push_back(completed("ready_for_review"));
    let (runner, calls) = scripted_runner(answers, false);
    let run = execute(review_loop(5), runner, None);
    assert_eq!(run.result.status, "completed");
    assert_eq!(
        calls.lock().unwrap().clone(),
        ["implement", "implement", "review", "implement", "review"]
    );
    let record = run.record();
    assert_eq!(record.outcome.as_deref(), Some("success"));
    assert_eq!(record.outcome_reason_code.as_deref(), Some("completed"));
    // The durable trace keeps each decision and its transition (stage 0 is
    // the start node).
    assert_eq!(run.status_json("review", 3, 0)["decision"], json!("fix"));
    let transitions = run.events("StageTransition");
    assert!(transitions.iter().any(|event| event["node_id"] == "review"
        && event["decision"] == "fix"
        && event["target"] == "implement"
        && event["visit"] == 3));
}

#[test]
fn an_undeclared_decision_fails_the_run_with_the_declared_set() {
    for decision in ["Ready", "approve", ""] {
        let answers = script(&[
            ("implement", vec![completed("ready_for_review")]),
            ("review", vec![completed(decision)]),
        ]);
        let (runner, calls) = scripted_runner(answers, false);
        let run = execute(review_loop(5), runner, None);
        assert_eq!(run.result.status, "failed", "{decision}");
        let reason = &run.record().last_error;
        assert!(reason.starts_with("no_matching_route"), "{reason}");
        assert!(
            reason.contains(&format!("returned decision {decision:?}")),
            "{reason}"
        );
        assert!(reason.contains(r#""blocked" -> "failed""#), "{reason}");
        assert!(reason.contains(r#""fix" -> "implement""#), "{reason}");
        assert!(reason.contains(r#""ready" -> "done""#), "{reason}");
        assert_eq!(count(&calls, "implement"), 1);
        // The completed invocation is recorded as it was returned.
        let status = run.status_json("review", 2, 0);
        assert_eq!(status["status"], json!("completed"));
        assert_eq!(status["decision"], json!(decision));
    }
}

/// Again-to-exhaustion: a decision whose target used all its visits fails
/// the run; no other route (not even `ready`) is ever taken instead.
#[test]
fn a_decision_selecting_an_exhausted_target_fails_without_another_route() {
    let answers = script(&[
        ("implement", vec![completed("ready_for_review")]),
        ("review", vec![completed("fix")]),
    ]);
    let (runner, calls) = scripted_runner(answers, false);
    let run = execute(review_loop(2), runner, None);
    assert_eq!(run.result.status, "failed");
    let reason = run.record().last_error;
    assert_eq!(
        reason,
        r#"visit_limit_reached: node "review" decision "fix" selects "implement", which used 2 of its 2 visits"#
    );
    assert_eq!(count(&calls, "implement"), 2);
    assert_eq!(count(&calls, "review"), 2);
    assert!(!run.result.route_trace.contains(&"done".to_string()));
    assert_eq!(run.status_json("review", 4, 0)["decision"], json!("fix"));
}

#[test]
fn execution_errors_retry_within_one_visit_then_fail_without_a_decision() {
    // A retryable runtime error is retried once by default, then succeeds.
    let answers = script(&[
        (
            "implement",
            vec![
                Outcome::error(FailureKind::Runtime, "provider timeout", Some(true)),
                completed("ready_for_review"),
            ],
        ),
        ("review", vec![completed("ready")]),
    ]);
    let (runner, calls) = scripted_runner(answers, false);
    let run = execute(review_loop(1), runner, None);
    assert_eq!(run.result.status, "completed");
    // Both attempts belong to one visit, so max_visits 1 still holds.
    assert_eq!(count(&calls, "implement"), 2);
    assert_eq!(
        run.status_json("implement", 1, 0)["failure_kind"],
        json!("runtime")
    );
    assert_eq!(
        run.status_json("implement", 1, 1)["decision"],
        json!("ready_for_review")
    );

    // Exhausted retries and nonretryable errors fail the run; the blocked
    // route is never taken for them.
    for error in [
        Outcome::error(FailureKind::Runtime, "provider down", Some(true)),
        Outcome::error(
            FailureKind::Contract,
            "invalid decision envelope",
            Some(false),
        ),
    ] {
        let answers = script(&[("implement", vec![error.clone()])]);
        let (runner, calls) = scripted_runner(answers, false);
        let run = execute(review_loop(3), runner, None);
        assert_eq!(run.result.status, "failed");
        assert_eq!(run.record().last_error, error.failure_reason);
        let expected = if error.failure_kind == Some(FailureKind::Runtime) {
            2
        } else {
            1
        };
        assert_eq!(count(&calls, "implement"), expected);
        assert!(!run.result.route_trace.contains(&"failed".to_string()));
    }
}

#[test]
fn reaching_a_failure_exit_completes_the_run_with_its_declared_result() {
    let answers = script(&[(
        "implement",
        vec![Outcome {
            notes: "the credentials are unavailable".to_string(),
            ..completed("blocked")
        }],
    )]);
    let (runner, _calls) = scripted_runner(answers, false);
    let run = execute(review_loop(5), runner, None);
    assert_eq!(run.result.status, "completed");
    assert_eq!(run.result.current_node, "failed");
    let record = run.record();
    assert_eq!(record.status, "completed");
    assert_eq!(record.outcome.as_deref(), Some("failure"));
    assert_eq!(record.outcome_reason_code.as_deref(), Some("blocked"));
    assert_eq!(
        record.outcome_reason_message.as_deref(),
        Some("implement chose blocked: the credentials are unavailable")
    );
}

fn tool_flow(config: Value, routes: Value) -> FlowDefinition {
    let mut config = config;
    config["kind"] = json!("tool");
    parse_flow(json!({
        "start": {"kind": "start", "routes": {"next": route("check")}},
        "check": {"kind": "tool", "config": config, "routes": routes,
            "contracts": {"writes_context": ["context.result.count"]}},
        "after": agent(json!({"done": route("done")})),
        "repair": agent(json!({"done": route("done")})),
        "done": exit("completed", "success"),
    }))
}

fn run_tool(config: Value) -> (Run, Arc<Mutex<Vec<String>>>) {
    let flow = tool_flow(
        config,
        json!({"passed": route("after"), "rejected": route("repair")}),
    );
    let answers = script(&[
        ("after", vec![completed("done")]),
        ("repair", vec![completed("done")]),
    ]);
    let (runner, calls) = scripted_runner(answers, false);
    (execute(flow, runner, None), calls)
}

#[test]
fn exit_code_tools_select_mapped_decisions_and_fail_on_unmapped_codes() {
    let codes =
        |command: &str| json!({"command": command, "exit_codes": {"0": "passed", "1": "rejected"}});
    let (run, calls) = run_tool(codes("exit 1"));
    assert_eq!(run.result.status, "completed");
    assert_eq!(calls.lock().unwrap().clone(), ["repair"]);
    assert_eq!(run.result.context["context.tool.exit_code"], json!(1));

    let (run, calls) = run_tool(codes("echo setup broke >&2; exit 2"));
    assert_eq!(run.result.status, "failed");
    assert_eq!(
        run.record().last_error,
        "tool exited with unmapped code 2; stderr: setup broke"
    );
    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(run.status_json("check", 1, 0)["status"], json!("error"));
}

#[test]
fn json_tools_require_exit_zero_and_one_declared_decision() {
    let json_mode = |command: &str| {
        json!({"command": command, "decision_field": "decision",
        "output_map": {"context.result.count": "count"}})
    };
    let (run, calls) = run_tool(json_mode(r#"printf '{"decision":"rejected","count":3}'"#));
    assert_eq!(run.result.status, "completed");
    assert_eq!(calls.lock().unwrap().clone(), ["repair"]);
    assert_eq!(run.result.context["context.result.count"], json!(3));

    for (command, reason) in [
        (
            r#"printf '{"decision":"passed","count":1}'; exit 1"#,
            "tool exited with code 1 instead of 0",
        ),
        ("echo not json", "tool stdout is not one JSON object"),
        (
            r#"printf '{"count":1}'"#,
            "tool JSON output has no decision field",
        ),
        (
            r#"printf '{"decision":"maybe","count":1}'"#,
            "undeclared decision \"maybe\"",
        ),
        // The decision is only accepted with its output_map values.
        (
            r#"printf '{"decision":"passed"}'"#,
            "tool.output_map path 'count' not found",
        ),
    ] {
        let (run, calls) = run_tool(json_mode(command));
        assert_eq!(run.result.status, "failed", "{command}");
        assert!(
            run.record().last_error.contains(reason),
            "{command}: {}",
            run.record().last_error
        );
        assert!(calls.lock().unwrap().is_empty(), "{command}");
        assert!(
            run.result.context.get("context.result.count").is_none(),
            "{command}"
        );
    }
}

#[test]
fn tool_timeouts_and_launch_failures_are_execution_errors() {
    let flow = {
        let mut flow = tool_flow(
            json!({"command": "sleep 5", "exit_codes": {"0": "passed", "1": "rejected"}}),
            json!({"passed": route("after"), "rejected": route("repair")}),
        );
        flow.nodes.get_mut("check").unwrap().runtime = Some(attractor_core::NodeRuntimeConfig {
            timeout: Some("200ms".to_string()),
            ..Default::default()
        });
        flow
    };
    let (runner, calls) = scripted_runner(script(&[]), false);
    let run = execute(flow, runner, None);
    assert_eq!(run.result.status, "failed");
    assert!(
        run.record().last_error.contains("timed out"),
        "{}",
        run.record().last_error
    );
    assert!(calls.lock().unwrap().is_empty());
}

fn switch_flow(count_json: &str) -> FlowDefinition {
    parse_flow(json!({
        "start": {"kind": "start", "routes": {"next": route("prepare")}},
        "prepare": {"kind": "tool",
            "config": {"kind": "tool", "command": format!("printf '{{\"conflict_count\":{count_json}}}'"),
                "exit_codes": {"0": "next"}, "output_map": {"context.integration.conflict_count": "conflict_count"}},
            "contracts": {"writes_context": ["context.integration.conflict_count"]},
            "routes": {"next": route("check_conflicts")}},
        "check_conflicts": {"kind": "switch",
            "contracts": {"reads_context": ["context.integration.conflict_count"]},
            "config": {"kind": "switch", "on": "context.integration.conflict_count",
                "cases": {"0": "clean"}, "otherwise": "conflicts"},
            "routes": {"clean": route("finish"), "conflicts": route("resolve")}},
        "finish": agent(json!({"done": route("done")})),
        "resolve": agent(json!({"done": route("done")})),
        "done": exit("completed", "success"),
    }))
}

#[test]
fn switches_compare_exact_scalars_and_fail_on_unsupported_values() {
    for (count, expected) in [
        ("0", "finish"),
        ("0.0", "finish"),
        ("2", "resolve"),
        (r#""0""#, "resolve"),
    ] {
        let answers = script(&[
            ("finish", vec![completed("done")]),
            ("resolve", vec![completed("done")]),
        ]);
        let (runner, calls) = scripted_runner(answers, false);
        let run = execute(switch_flow(count), runner, None);
        assert_eq!(run.result.status, "completed", "{count}");
        assert_eq!(calls.lock().unwrap().clone(), [expected], "{count}");
        assert_eq!(
            run.status_json("check_conflicts", 2, 0)["decision"],
            json!(if expected == "finish" {
                "clean"
            } else {
                "conflicts"
            })
        );
    }
    for count in ["null", "[0]", "{}"] {
        let (runner, calls) = scripted_runner(script(&[]), false);
        let run = execute(switch_flow(count), runner, None);
        assert_eq!(run.result.status, "failed", "{count}");
        assert!(
            run.record()
                .last_error
                .contains("switch key context.integration.conflict_count"),
            "{}",
            run.record().last_error
        );
        assert!(calls.lock().unwrap().is_empty());
    }
}

/// A fork with branches `a` and `b` joining at `join`; `synth` follows the
/// join. Every branch agent may block to the shared failure exit.
fn fork_flow(tolerate: bool) -> FlowDefinition {
    let lens = json!({"kind": "agent_task", "contracts": {"response": "decision_envelope",
            "writes_context": ["context.review.lens_findings"]},
        "routes": {"done": route("join"), "unfinished": route("a"), "blocked": route("failed")}});
    let mut b = lens.clone();
    b["routes"]["unfinished"] = route("b");
    parse_flow(json!({
        "start": {"kind": "start", "routes": {"next": route("fork")}},
        "fork": {"kind": "parallel", "config": {"kind": "parallel", "branches": ["a", "b"], "join": "join",
            "max_parallel": 2, "tolerate_branch_execution_errors": tolerate}},
        "a": lens,
        "b": b,
        "join": {"kind": "fan_in", "routes": {"next": route("synth")}},
        "synth": {"kind": "agent_task", "contracts": {"response": "decision_envelope", "reads_context": ["parallel.results"]},
            "routes": {"done": route("done"), "blocked": route("failed")}},
        "done": exit("completed", "success"),
        "failed": exit("blocked", "failure"),
    }))
}

fn finding(value: &str) -> Outcome {
    Outcome {
        context_updates: ContextMap::from([(
            "context.review.lens_findings".to_string(),
            json!(value),
        )]),
        notes: format!("findings {value}"),
        ..completed("done")
    }
}

fn fork_results(run: &Run) -> Vec<Value> {
    run.status_json("fork", 1, 0)["context_updates"]["parallel.results"]
        .as_array()
        .cloned()
        .expect("parallel.results")
}

#[test]
fn completed_branches_join_once_with_isolated_contexts() {
    let answers = script(&[
        ("a", vec![finding("from a")]),
        ("b", vec![finding("from b")]),
        ("synth", vec![completed("done")]),
    ]);
    let (runner, calls) = scripted_runner(answers, false);
    let run = execute(fork_flow(false), runner, None);
    assert_eq!(run.result.status, "completed");
    assert_eq!(count(&calls, "a"), 1);
    assert_eq!(count(&calls, "b"), 1);
    assert_eq!(count(&calls, "synth"), 1);
    // The join ran once in the parent and never inside a branch.
    assert_eq!(
        run.result.route_trace,
        ["start", "fork", "join", "synth", "done"]
    );
    let results = fork_results(&run);
    assert_eq!(results.len(), 2);
    for (index, (id, value)) in [("a", "from a"), ("b", "from b")].into_iter().enumerate() {
        assert_eq!(results[index]["id"], json!(id));
        assert_eq!(results[index]["index"], json!(index));
        assert_eq!(results[index]["status"], json!("completed"));
        assert_eq!(
            results[index]["context"]["context.review.lens_findings"],
            json!(value)
        );
        assert_eq!(results[index]["notes"], json!(format!("findings {value}")));
        assert_eq!(results[index]["decisions"][0]["target"], json!("join"));
    }
    // Branch context keys are never promoted into the parent.
    assert!(run
        .result
        .context
        .get("context.review.lens_findings")
        .is_none());
}

#[test]
fn strict_forks_fail_on_a_branch_execution_error_and_tolerant_forks_keep_it() {
    let broken = Outcome::error(FailureKind::Runtime, "provider crashed", Some(false));
    let answers = || {
        script(&[
            ("a", vec![finding("from a")]),
            ("b", vec![broken.clone()]),
            ("synth", vec![completed("done")]),
        ])
    };

    let (runner, calls) = scripted_runner(answers(), false);
    let run = execute(fork_flow(false), runner, None);
    assert_eq!(run.result.status, "failed");
    assert_eq!(
        run.record().last_error,
        "branch b (index 1) failed at b: provider crashed"
    );
    assert_eq!(count(&calls, "synth"), 0);
    assert!(!run.result.route_trace.contains(&"join".to_string()));

    let (runner, calls) = scripted_runner(answers(), false);
    let run = execute(fork_flow(true), runner, None);
    assert_eq!(run.result.status, "completed");
    assert_eq!(count(&calls, "synth"), 1);
    let results = fork_results(&run);
    assert_eq!(results[0]["status"], json!("completed"));
    assert_eq!(results[1]["status"], json!("error"));
    assert_eq!(results[1]["error"]["kind"], json!("runtime"));
    assert_eq!(results[1]["error"]["tolerable"], json!(true));
    assert_eq!(results[1]["error"]["reason"], json!("provider crashed"));
    assert_eq!(results[1]["error"]["node"], json!("b"));

    // Even with every branch failing, a tolerant fork runs the join once
    // with zero completed entries.
    let answers = script(&[
        (
            "a",
            vec![Outcome::error(
                FailureKind::Contract,
                "bad envelope",
                Some(false),
            )],
        ),
        ("b", vec![broken.clone()]),
        ("synth", vec![completed("blocked")]),
    ]);
    let (runner, calls) = scripted_runner(answers, false);
    let run = execute(fork_flow(true), runner, None);
    assert_eq!(run.result.status, "completed");
    assert_eq!(count(&calls, "synth"), 1);
    assert_eq!(run.record().outcome.as_deref(), Some("failure"));
    assert!(fork_results(&run)
        .iter()
        .all(|record| record["status"] == "error"));
}

#[test]
fn routing_errors_in_a_branch_are_fatal_even_when_tolerated() {
    for decision in ["approve", "unfinished"] {
        // `unfinished` loops `a` onto itself past its branch visit limit.
        let answers = script(&[
            ("a", vec![completed(decision)]),
            ("b", vec![finding("from b")]),
            ("synth", vec![completed("done")]),
        ]);
        let (runner, calls) = scripted_runner(answers, false);
        let mut flow = fork_flow(true);
        flow.nodes.get_mut("a").unwrap().runtime = Some(attractor_core::NodeRuntimeConfig {
            max_visits: Some(2),
            ..Default::default()
        });
        let run = execute(flow, runner, None);
        assert_eq!(run.result.status, "failed", "{decision}");
        let reason = run.record().last_error;
        let expected = if decision == "approve" {
            "no_matching_route"
        } else {
            "visit_limit_reached"
        };
        assert!(reason.contains(expected), "{reason}");
        assert_eq!(count(&calls, "synth"), 0);
        assert_eq!(fork_results(&run)[0]["error"]["kind"], json!("routing"));
        assert_eq!(fork_results(&run)[0]["error"]["tolerable"], json!(false));
    }
}

#[test]
fn a_blocked_branch_ends_the_owner_at_its_failure_exit() {
    for (tolerate, b_answer, expected_status) in [
        // Alone, or beside a tolerable error, blocked is the owner's result.
        (false, finding("from b"), "completed"),
        (
            true,
            Outcome::error(FailureKind::Runtime, "crash", Some(false)),
            "completed",
        ),
        // A fatal sibling error wins over the domain result.
        (true, completed("nonsense"), "failed"),
        // Both branches blocked: the lowest index is primary.
        (
            false,
            Outcome {
                notes: "b blocked".into(),
                ..completed("blocked")
            },
            "completed",
        ),
    ] {
        let answers = script(&[
            (
                "a",
                vec![Outcome {
                    notes: "a cannot review".into(),
                    ..completed("blocked")
                }],
            ),
            ("b", vec![b_answer.clone()]),
            ("synth", vec![completed("done")]),
        ]);
        let (runner, calls) = scripted_runner(answers, false);
        let run = execute(fork_flow(tolerate), runner, None);
        assert_eq!(run.result.status, expected_status, "{b_answer:?}");
        assert_eq!(count(&calls, "synth"), 0);
        assert!(!run.result.route_trace.contains(&"join".to_string()));
        let results = fork_results(&run);
        assert_eq!(results[0]["status"], json!("terminal_result"));
        assert_eq!(results[0]["terminal"]["exit_id"], json!("failed"));
        assert_eq!(results[0]["terminal"]["node"], json!("a"));
        assert_eq!(results[0]["terminal"]["decision"], json!("blocked"));
        assert_eq!(results[0]["terminal"]["notes"], json!("a cannot review"));
        if expected_status == "completed" {
            let record = run.record();
            assert_eq!(record.outcome.as_deref(), Some("failure"));
            assert_eq!(record.outcome_reason_code.as_deref(), Some("blocked"));
            assert_eq!(
                record.outcome_reason_message.as_deref(),
                Some("branch a (index 0): a chose blocked: a cannot review")
            );
            assert_eq!(run.result.current_node, "failed");
            assert_eq!(run.result.route_trace, ["start", "fork", "failed"]);
        } else {
            assert!(run.record().last_error.contains("no_matching_route"));
        }
    }
}

#[test]
fn a_blocked_branch_cancels_and_settles_running_siblings() {
    // `b` keeps working until the fork cancels it; it is recorded as
    // canceled by its owner, not as an execution error.
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut runner = RuntimeHandlerRunner::new();
    let seen = calls.clone();
    runner.register_thread_safe_handler_fn(HANDLER_CODERGEN, move |runtime: HandlerRuntime| {
        seen.lock().unwrap().push(runtime.node_id.clone());
        match runtime.node_id.as_str() {
            "a" => {
                std::thread::sleep(std::time::Duration::from_millis(50));
                Ok(completed("blocked"))
            }
            _ => Ok(completed("unfinished")),
        }
    });
    let mut flow = fork_flow(false);
    flow.nodes.get_mut("b").unwrap().runtime = Some(attractor_core::NodeRuntimeConfig {
        max_visits: Some(100_000),
        ..Default::default()
    });
    let run = execute(flow, runner, None);
    assert_eq!(run.result.status, "completed");
    assert_eq!(run.record().outcome_reason_code.as_deref(), Some("blocked"));
    let results = fork_results(&run);
    assert_eq!(results[1]["status"], json!("canceled"));
    assert_eq!(results[1]["started"], json!(true));
}

#[test]
fn a_resumed_fork_never_relaunches_settled_branches() {
    let temp = tempfile::tempdir().unwrap();
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let workdir = temp.path().join("work");
    std::fs::create_dir_all(&workdir).unwrap();
    let flow = fork_flow(false);
    let paths = attractor_runtime::prepare_fresh_run(
        &store,
        &RunRecord::new("run-contract", workdir.to_string_lossy()),
        &flow,
        None,
        None,
        &LaunchContext::empty(),
        &ContextMap::new(),
    )
    .unwrap();
    // A terminal result persisted before the crash: the fork (stage 1,
    // attempt 0) finalizes it and launches nothing.
    let branches = paths.logs_dir().join("fork/executions/1-0/branches");
    std::fs::create_dir_all(&branches).unwrap();
    std::fs::write(
        branches.join("0.json"),
        serde_json::to_vec(&json!({
            "id": "a", "index": 0, "status": "terminal_result", "started": true, "context": {},
            "terminal": {"exit_id": "failed", "result": {"name": "blocked", "outcome": "failure"},
                "branch": "a", "branch_index": 0, "node": "a", "decision": "blocked", "notes": "saved"}
        }))
        .unwrap(),
    )
    .unwrap();
    let (runner, calls) = scripted_runner(script(&[]), false);
    let run = execute_in(temp, flow, runner, Some(ExecutionStart::Prepared { paths }));
    assert_eq!(run.result.status, "completed");
    assert!(
        calls.lock().unwrap().is_empty(),
        "{:?}",
        calls.lock().unwrap()
    );
    assert_eq!(run.record().outcome_reason_code.as_deref(), Some("blocked"));
    let results = fork_results(&run);
    assert_eq!(results[1]["status"], json!("canceled"));
    assert_eq!(results[1]["started"], json!(false));
    let _ = &run.workdir;
}

fn bundled_flow(name: &str) -> FlowDefinition {
    let bundled = Path::new(env!("CARGO_MANIFEST_DIR")).join("../spark-assets/assets");
    attractor_dsl::load_flow(&attractor_dsl::FlowRoots::new(bundled, None), name)
        .unwrap_or_else(|error| panic!("{name}: {error}"))
        .flow
}

#[test]
fn bundled_audit_sweep_runs_each_lens_once_and_joins_once() {
    let lenses = [
        "lens_correctness",
        "lens_minimality",
        "lens_safety",
        "lens_test_gaps",
    ];
    let mut entries = lenses
        .iter()
        .map(|lens| (*lens, vec![completed("done")]))
        .collect::<Vec<_>>();
    entries.extend([
        ("verify_findings", vec![completed("done")]),
        ("report", vec![completed("done")]),
    ]);
    let (runner, calls) = scripted_runner(script(&entries), false);
    let run = execute(
        bundled_flow("software-development/audit-codebase"),
        runner,
        None,
    );
    assert_eq!(run.result.status, "completed");
    assert_eq!(run.record().outcome.as_deref(), Some("success"));
    for lens in lenses {
        assert_eq!(count(&calls, lens), 1, "{lens}");
    }
    assert_eq!(count(&calls, "verify_findings"), 1);
    assert_eq!(
        run.result
            .route_trace
            .iter()
            .filter(|node| *node == "fan_in")
            .count(),
        1
    );

    // A lens that cannot review ends the audit blocked; nothing after the
    // join runs.
    entries[1] = ("lens_minimality", vec![completed("blocked")]);
    let (runner, calls) = scripted_runner(script(&entries), false);
    let run = execute(
        bundled_flow("software-development/audit-codebase"),
        runner,
        None,
    );
    assert_eq!(run.result.status, "completed");
    assert_eq!(run.record().outcome.as_deref(), Some("failure"));
    assert_eq!(run.record().outcome_reason_code.as_deref(), Some("blocked"));
    assert_eq!(count(&calls, "verify_findings"), 0);
}

#[test]
fn bundled_design_review_allows_three_rounds_then_fails_the_fourth() {
    let lenses = [
        "codebase_check",
        "critic",
        "naive_reader",
        "readiness",
        "zoom_out",
    ];
    let mut entries = lenses
        .iter()
        .map(|lens| (*lens, vec![completed("done")]))
        .collect::<Vec<_>>();
    entries.extend([
        ("draft", vec![completed("done")]),
        ("synthesize", vec![completed("done")]),
        ("resolve", vec![completed("rereview")]),
    ]);
    let (runner, calls) = scripted_runner(script(&entries), false);
    let run = execute(
        bundled_flow("software-development/design-change"),
        runner,
        None,
    );
    assert_eq!(run.result.status, "failed");
    assert_eq!(count(&calls, "resolve"), 3);
    for lens in lenses {
        assert_eq!(count(&calls, lens), 3, "{lens}");
    }
    assert!(run.record().last_error.starts_with(
        r#"visit_limit_reached: node "resolve" decision "rereview" selects "review", which used 3 of its 3 visits"#
    ), "{}", run.record().last_error);
}

fn implement_change(entries: &[(&str, Vec<Outcome>)]) -> (Run, Arc<Mutex<Vec<String>>>) {
    let mut all = vec![
        ("normalize_task", vec![completed("done")]),
        ("prepare_workspace", vec![completed("done")]),
        ("validate", vec![completed("passed")]),
        ("diagnose_validation", vec![completed("done")]),
        ("finalize_commit", vec![completed("done")]),
        ("cleanup_workspace", vec![completed("done")]),
        // The done exit's result summary is one synthetic agent call.
        (
            "result_summary",
            vec![Outcome {
                notes: "summary".to_string(),
                ..completed("")
            }],
        ),
    ];
    for (node, answers) in entries {
        all.retain(|(existing, _)| existing != node);
        all.push((node, answers.clone()));
    }
    let (runner, calls) = scripted_runner(script(&all), true);
    (
        execute(
            bundled_flow("software-development/implement-change"),
            runner,
            None,
        ),
        calls,
    )
}

#[test]
fn bundled_implement_change_reviews_every_pass_before_validating_and_committing() {
    let (run, calls) = implement_change(&[
        (
            "implement",
            vec![
                completed("unfinished"),
                completed("ready_for_review"),
                completed("ready_for_review"),
            ],
        ),
        ("evaluate", vec![completed("fix"), completed("ready")]),
    ]);
    assert_eq!(run.result.status, "completed");
    assert_eq!(run.record().outcome.as_deref(), Some("success"));
    assert_eq!(
        calls.lock().unwrap().clone(),
        [
            "normalize_task",
            "prepare_workspace",
            "implement",
            "implement",
            "evaluate",
            "implement",
            "evaluate",
            "validate",
            "finalize_commit",
            "cleanup_workspace",
            "result_summary",
        ]
    );

    // A fix at the exhausted implement budget fails; nothing validates.
    let (run, calls) = implement_change(&[
        ("implement", vec![completed("ready_for_review")]),
        ("evaluate", vec![completed("fix")]),
    ]);
    assert_eq!(run.result.status, "failed");
    assert!(
        run.record().last_error.starts_with("visit_limit_reached"),
        "{}",
        run.record().last_error
    );
    assert_eq!(count(&calls, "implement"), 5);
    assert_eq!(count(&calls, "validate"), 0);
    assert_eq!(count(&calls, "finalize_commit"), 0);

    // A failed commit never runs cleanup or reaches a success exit.
    let (run, calls) = implement_change(&[
        ("implement", vec![completed("ready_for_review")]),
        ("evaluate", vec![completed("ready")]),
        (
            "finalize_commit",
            vec![Outcome::error(
                FailureKind::Runtime,
                "git commit failed",
                Some(false),
            )],
        ),
    ]);
    assert_eq!(run.result.status, "failed");
    assert_eq!(run.record().last_error, "git commit failed");
    assert_eq!(count(&calls, "cleanup_workspace"), 0);
}

#[test]
fn bundled_parallel_review_cancel_ends_with_declared_failure() {
    let answers = script(&[
        ("plan", vec![completed("done")]),
        ("implement", vec![completed("done")]),
    ]);
    let (mut runner, calls) = scripted_runner(answers, false);
    let interviewer =
        attractor_runtime::QueueInterviewer::new([attractor_runtime::HumanAnswer::selected(
            "cancel",
        )]);
    let gate_runner = RuntimeHandlerRunner::with_interviewer(interviewer);
    // Keep the scripted agents, answer the gate from the queue.
    let _ = &mut runner;
    let mut runner = gate_runner;
    let (script_ref, calls_ref) = (
        script(&[
            ("plan", vec![completed("done")]),
            ("implement", vec![completed("done")]),
        ]),
        calls.clone(),
    );
    runner.register_thread_safe_handler_fn(HANDLER_CODERGEN, move |runtime| {
        Ok(answer(&script_ref, &calls_ref, &runtime))
    });
    let run = execute(bundled_flow("examples/parallel-review"), runner, None);
    assert_eq!(run.result.status, "completed");
    let record = run.record();
    assert_eq!(record.outcome.as_deref(), Some("failure"));
    assert_eq!(record.outcome_reason_code.as_deref(), Some("blocked"));
    assert_eq!(run.result.context["human.gate.selected"], json!("cancel"));
    assert_eq!(count(&calls, "branch_docs"), 0);
}

#[test]
fn human_gates_take_only_declared_choices() {
    let flow = parse_flow(json!({
        "start": {"kind": "start", "routes": {"next": route("gate")}},
        "gate": {"kind": "human_gate", "routes": {
            "approve": {"to": "done", "label": "Approve", "description": "Approved."},
            "stop": {"to": "failed", "label": "Stop Run", "description": "Stopped."}
        }},
        "done": exit("completed", "success"),
        "failed": exit("blocked", "failure"),
    }));
    for (answer, status, result) in [
        ("approve", "completed", Some("completed")),
        ("stop", "completed", Some("blocked")),
        // Labels, accelerators and undeclared answers are never decisions.
        ("Stop Run", "failed", None),
        ("S", "failed", None),
        ("maybe", "failed", None),
    ] {
        let runner =
            RuntimeHandlerRunner::with_interviewer(attractor_runtime::QueueInterviewer::new([
                attractor_runtime::HumanAnswer::selected(answer),
            ]));
        let run = execute(flow.clone(), runner, None);
        assert_eq!(run.result.status, status, "{answer}");
        assert_eq!(
            run.record().outcome_reason_code.as_deref(),
            result,
            "{answer}"
        );
    }
}

fn write_flow(root: &Path, name: &str, files: &[(&str, &str)]) {
    let dir = root.join("flows").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    for (file, text) in files {
        std::fs::write(dir.join(file), text).unwrap();
    }
}

const FLOW_MD: &str = "---\ntype: Flow\nid: tiny\nschema_version: '2'\n---\n";
const START_MD: &str =
    "---\ntype: start\nroutes:\n  next:\n    to: done\n    description: Enter.\n---\n";
const DONE_MD: &str =
    "---\ntype: exit\nconfig:\n  result:\n    name: completed\n    outcome: success\n---\n";

#[test]
fn markdown_flows_reject_duplicate_decisions_and_old_versions_by_path() {
    let temp = tempfile::tempdir().unwrap();
    let duplicate = "---\ntype: start\nroutes:\n  next:\n    to: done\n    description: Enter.\n  next:\n    to: done\n    description: Again.\n---\n";
    write_flow(
        temp.path(),
        "dup",
        &[
            ("flow.md", FLOW_MD),
            ("start.md", duplicate),
            ("done.md", DONE_MD),
        ],
    );
    let error = attractor_dsl::read_flow_dir(&temp.path().join("flows/dup")).unwrap_err();
    assert!(error.detail().contains("start.md"), "{}", error.detail());
    assert!(
        error.detail().contains("duplicate entry with key \"next\""),
        "{}",
        error.detail()
    );

    for header in [
        "---\ntype: Flow\nid: tiny\nschema_version: '1'\n---\n",
        "---\ntype: Flow\nid: tiny\n---\n",
    ] {
        write_flow(
            temp.path(),
            "old",
            &[
                ("flow.md", header),
                ("start.md", START_MD),
                ("done.md", DONE_MD),
            ],
        );
        let dir = temp.path().join("flows/old");
        let error = attractor_dsl::read_flow_dir(&dir).unwrap_err();
        assert!(
            error.detail().contains(&dir.display().to_string()),
            "{}",
            error.detail()
        );
        assert!(
            error.detail().contains("schema_version must be '2'"),
            "{}",
            error.detail()
        );
    }

    // Legacy route lists are not schema 2.
    let legacy = "---\ntype: start\nroutes:\n- to: done\n---\n";
    write_flow(
        temp.path(),
        "legacy",
        &[
            ("flow.md", FLOW_MD),
            ("start.md", legacy),
            ("done.md", DONE_MD),
        ],
    );
    assert!(attractor_dsl::read_flow_dir(&temp.path().join("flows/legacy")).is_err());

    // A valid flow round-trips through the writer unchanged.
    write_flow(
        temp.path(),
        "tiny",
        &[
            ("flow.md", FLOW_MD),
            ("start.md", START_MD),
            ("done.md", DONE_MD),
        ],
    );
    let flow = attractor_dsl::read_flow_dir(&temp.path().join("flows/tiny")).unwrap();
    let rendered = attractor_dsl::render_flow_files(&flow).unwrap();
    assert_eq!(attractor_dsl::assemble_flow_files(&rendered).unwrap(), flow);
}

#[test]
fn an_invalid_project_override_is_rejected_without_falling_back_to_bundled() {
    let temp = tempfile::tempdir().unwrap();
    let bundled = temp.path().join("bundled");
    write_flow(
        &bundled,
        "tiny",
        &[
            ("flow.md", FLOW_MD),
            ("start.md", START_MD),
            ("done.md", DONE_MD),
        ],
    );
    let project = temp.path().join("project");
    let definitions = spark_common::project::definitions_dir(&project);
    let old_start = "---\ntype: start\nroutes:\n- to: done\n---\n";
    write_flow(
        &definitions,
        "tiny",
        &[
            (
                "flow.md",
                "---\ntype: Flow\nid: tiny\nschema_version: '1'\n---\n",
            ),
            ("start.md", old_start),
            ("done.md", DONE_MD),
        ],
    );
    let roots = attractor_dsl::FlowRoots::new(&bundled, Some(&project));
    let error = attractor_dsl::load_flow(&roots, "tiny").unwrap_err();
    assert!(
        error
            .detail()
            .contains(&definitions.join("flows/tiny").display().to_string()),
        "{}",
        error.detail()
    );
    assert!(
        attractor_dsl::load_flow(&attractor_dsl::FlowRoots::new(&bundled, None), "tiny").is_ok()
    );
}

#[test]
fn subflow_results_must_match_the_effective_child_exactly() {
    let temp = tempfile::tempdir().unwrap();
    let bundled = temp.path().join("bundled");
    let failed =
        "---\ntype: exit\nconfig:\n  result:\n    name: blocked\n    outcome: failure\n---\n";
    let child_start =
        "---\ntype: start\nroutes:\n  next:\n    to: work\n    description: Enter.\n---\n";
    let work = "---\ntype: agent_task\ncontracts:\n  response: decision_envelope\nroutes:\n  done:\n    to: done\n    description: Done.\n  blocked:\n    to: failed\n    description: Blocked.\n---\n\nWork.\n";
    write_flow(
        &bundled,
        "team/child",
        &[
            ("flow.md", FLOW_MD),
            ("start.md", child_start),
            ("work.md", work),
            ("done.md", DONE_MD),
            ("failed.md", failed),
        ],
    );
    let parent = |results: &str, flow_ref: &str| {
        format!(
            "---\ntype: subflow\nconfig:\n  flow_ref: {flow_ref}\n  results:\n{results}routes:\n  child_completed:\n    to: done\n    description: Completed.\n  child_blocked:\n    to: failed\n    description: Blocked.\n---\n"
        )
    };
    let parent_start =
        "---\ntype: start\nroutes:\n  next:\n    to: manager\n    description: Enter.\n---\n";
    let roots = attractor_dsl::FlowRoots::new(&bundled, None);
    for (results, flow_ref, expected) in [
        (
            "    completed: child_completed\n    blocked: child_blocked\n",
            "child",
            None,
        ),
        (
            "    completed: child_completed\n    gone: child_blocked\n",
            "child",
            Some("does not map child result 'blocked'"),
        ),
        (
            "    completed: child_completed\n    blocked: child_blocked\n",
            "missing",
            Some("Flow not found"),
        ),
        (
            "    completed: child_completed\n    blocked: child_blocked\n",
            "parent",
            Some("recursive reference"),
        ),
    ] {
        write_flow(
            &bundled,
            "team/parent",
            &[
                ("flow.md", FLOW_MD),
                ("start.md", parent_start),
                ("manager.md", &parent(results, flow_ref)),
                ("done.md", DONE_MD),
                ("failed.md", failed),
            ],
        );
        match (attractor_dsl::load_flow(&roots, "team/parent"), expected) {
            (Ok(_), None) => {}
            (Err(error), Some(text)) => {
                assert!(error.detail().contains(text), "{}", error.detail())
            }
            (result, expected) => panic!("{expected:?}: {:?}", result.map(|_| ())),
        }
    }
}

#[test]
fn external_cancellation_of_a_fork_aborts_the_run_without_a_join() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut runner = RuntimeHandlerRunner::new();
    let seen = calls.clone();
    runner.register_thread_safe_handler_fn(HANDLER_CODERGEN, move |runtime: HandlerRuntime| {
        seen.lock().unwrap().push(runtime.node_id.clone());
        if runtime.node_id == "a" {
            let paths = runtime.run_paths.clone().expect("run paths");
            let mut record = attractor_runtime::read_run_record(&paths).unwrap().unwrap();
            attractor_runtime::mark_record_cancel_requested(&mut record);
            attractor_runtime::write_run_record(&paths, &record).unwrap();
            return Ok(completed("done"));
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        Ok(completed("unfinished"))
    });
    let mut flow = fork_flow(false);
    flow.nodes.get_mut("b").unwrap().runtime = Some(attractor_core::NodeRuntimeConfig {
        max_visits: Some(100_000),
        ..Default::default()
    });
    let temp = tempfile::tempdir().unwrap();
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let control_store = store.clone();
    let result = PipelineExecutor::with_control(runner, move || {
        let paths = control_store.find_run_root("run-contract").ok().flatten()?;
        let record = control_store.read_run_record(&paths).ok().flatten()?;
        (record.status == "cancel_requested")
            .then_some(attractor_runtime::ExecutionControlAction::Cancel)
    })
    .execute(ExecuteRunRequest {
        store: store.clone(),
        record: RunRecord::new("run-contract", temp.path().to_string_lossy()),
        flow,
        flow_definition_json: None,
        launch_context: LaunchContext::empty(),
        runtime_context: ContextMap::new(),
        max_steps: None,
        start: Default::default(),
    })
    .unwrap();
    assert_eq!(result.status, "canceled");
    assert_eq!(count(&calls, "synth"), 0);
    assert!(!result.route_trace.contains(&"join".to_string()));
}

#[test]
fn branches_retry_execution_errors_like_the_root() {
    let answers = script(&[
        (
            "a",
            vec![
                Outcome::error(FailureKind::Runtime, "transport reset", Some(true)),
                finding("from a"),
            ],
        ),
        (
            "b",
            vec![Outcome::error(
                FailureKind::Runtime,
                "provider down",
                Some(true),
            )],
        ),
        ("synth", vec![completed("done")]),
    ]);
    let (runner, calls) = scripted_runner(answers, false);
    let run = execute(fork_flow(true), runner, None);
    assert_eq!(run.result.status, "completed");
    // One retry each, as at the root; `b` stays an error after it.
    assert_eq!(count(&calls, "a"), 2);
    assert_eq!(count(&calls, "b"), 2);
    let results = fork_results(&run);
    assert_eq!(results[0]["status"], json!("completed"));
    assert_eq!(results[1]["status"], json!("error"));
    assert_eq!(results[1]["error"]["attempts"], json!(2));
}

/// `b` returns only after `a` blocked and the fork raised cancellation, so
/// its error lands while the cancellation flag is set.
fn after_cancellation(error: Outcome) -> RuntimeHandlerRunner {
    let mut runner = RuntimeHandlerRunner::new();
    runner.register_thread_safe_handler_fn(HANDLER_CODERGEN, move |runtime: HandlerRuntime| {
        if runtime.node_id == "a" {
            return Ok(Outcome {
                notes: "a cannot review".into(),
                ..completed("blocked")
            });
        }
        let cancel = runtime.cancel.clone().expect("branch cancellation flag");
        while !cancel.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        Ok(error.clone())
    });
    runner
}

#[test]
fn an_independent_fatal_sibling_error_after_blocked_fails_the_owner() {
    for tolerate in [false, true] {
        for (error, kind) in [
            (
                Outcome::error(FailureKind::Routing, "child result unmapped", Some(false)),
                "routing",
            ),
            (
                Outcome::error(FailureKind::Contract, "bad envelope", Some(false)),
                "contract",
            ),
            // A runtime failure the cancellation did not cause is the
            // branch's own error, not a cancellation.
            (
                Outcome::error(FailureKind::Runtime, "provider crashed", Some(false)),
                "runtime",
            ),
        ] {
            let run = execute(fork_flow(tolerate), after_cancellation(error.clone()), None);
            let results = fork_results(&run);
            assert_eq!(results[0]["status"], json!("terminal_result"));
            // The sibling's own error keeps its kind and reason.
            assert_eq!(results[1]["status"], json!("error"), "{tolerate} {kind}");
            assert_eq!(results[1]["error"]["kind"], json!(kind));
            assert_eq!(results[1]["error"]["reason"], json!(error.failure_reason));
            let fatal = kind == "routing" || !tolerate;
            if fatal {
                assert_eq!(run.result.status, "failed", "{tolerate} {kind}");
                assert_eq!(
                    run.record().last_error,
                    format!("branch b (index 1) failed at b: {}", error.failure_reason)
                );
            } else {
                // A tolerated error does not override the failure exit.
                assert_eq!(run.result.status, "completed", "{tolerate} {kind}");
                assert_eq!(run.record().outcome_reason_code.as_deref(), Some("blocked"));
            }
        }
    }
}

const INTERRUPT_CHILD_ENV: &str = "SPARK_BRANCH_INTERRUPT_CHILD_DIR";
const INTERRUPT_PAUSE_ENV: &str = "SPARK_BRANCH_INTERRUPT_PAUSE";

fn flow_pauses(flow: &FlowDefinition) -> bool {
    flow.nodes["t1"]
        .runtime
        .as_ref()
        .is_some_and(|runtime| runtime.recovery_policy == attractor_core::RecoveryPolicy::Pause)
}

/// `t1` (a side-effecting tool) then `a2` in branch one; `b` in branch two.
fn interrupted_fork_flow(log: &Path, tool_tail: &str, pause: bool) -> FlowDefinition {
    let mut flow = parse_flow(json!({
        "start": {"kind": "start", "routes": {"next": route("fork")}},
        "fork": {"kind": "parallel", "config": {"kind": "parallel", "branches": ["t1", "b"], "join": "join"}},
        "t1": {"kind": "tool", "config": {"kind": "tool",
            "command": format!("echo ran >> '{}'{tool_tail}", log.display()), "exit_codes": {"0": "done"}},
            "routes": {"done": route("a2")}},
        "a2": agent(json!({"done": route("join")})),
        "b": agent(json!({"done": route("join")})),
        "join": {"kind": "fan_in", "routes": {"next": route("synth")}},
        "synth": agent(json!({"done": route("done")})),
        "done": exit("completed", "success"),
    }));
    if pause {
        flow.nodes.get_mut("t1").unwrap().runtime = Some(attractor_core::NodeRuntimeConfig {
            recovery_policy: attractor_core::RecoveryPolicy::Pause,
            ..Default::default()
        });
    }
    flow
}

/// Runs `flow` in `dir` until the process dies inside it.
fn run_until_interrupted(dir: &Path, flow: &FlowDefinition, test: &str) {
    if let Ok(child_dir) = std::env::var(INTERRUPT_CHILD_ENV) {
        let child_dir = PathBuf::from(child_dir);
        let mut runner = RuntimeHandlerRunner::new();
        runner.register_thread_safe_handler_fn(HANDLER_CODERGEN, |runtime: HandlerRuntime| {
            if runtime.node_id == "a2" {
                // The process dies after t1 was accepted, before a2 finishes.
                std::process::exit(7);
            }
            if runtime.node_id == "b" {
                std::thread::sleep(std::time::Duration::from_secs(30));
            }
            Ok(completed("done"))
        });
        let store = RunStore::for_runs_dir(child_dir.join("runs"));
        let _ = PipelineExecutor::new(runner).execute(ExecuteRunRequest {
            store,
            record: RunRecord::new("run-contract", child_dir.to_string_lossy()),
            flow: flow.clone(),
            flow_definition_json: None,
            launch_context: LaunchContext::empty(),
            runtime_context: ContextMap::new(),
            max_steps: None,
            start: Default::default(),
        });
        panic!("the child run should have been interrupted");
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("routing_model_contracts::{test}"),
            "--nocapture",
        ])
        .env(INTERRUPT_CHILD_ENV, dir)
        .env(INTERRUPT_PAUSE_ENV, flow_pauses(flow).to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success(), "the child must die mid-run");
}

fn resume(
    dir: &Path,
    flow: FlowDefinition,
    runner: RuntimeHandlerRunner,
) -> PipelineExecutionResult {
    let store = RunStore::for_runs_dir(dir.join("runs"));
    let bundle = store.read_run_bundle("run-contract").unwrap().unwrap();
    let checkpoint = bundle.checkpoint.unwrap();
    assert_eq!(checkpoint.current_node, "fork");
    PipelineExecutor::new(runner)
        .execute(ExecuteRunRequest {
            store,
            record: bundle.record.unwrap(),
            flow,
            flow_definition_json: None,
            launch_context: LaunchContext::empty(),
            runtime_context: ContextMap::new(),
            max_steps: None,
            start: ExecutionStart::Resume {
                paths: bundle.paths,
                checkpoint,
            },
        })
        .unwrap()
}

#[test]
fn a_resumed_branch_never_reruns_an_accepted_side_effect() {
    let temp = tempfile::tempdir().unwrap();
    let dir = std::env::var(INTERRUPT_CHILD_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| temp.path().to_path_buf());
    let log = dir.join("side-effect.log");
    let flow = interrupted_fork_flow(&log, "", false);
    run_until_interrupted(
        &dir,
        &flow,
        "a_resumed_branch_never_reruns_an_accepted_side_effect",
    );
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "ran\n");

    let answers = script(&[
        ("a2", vec![completed("done")]),
        ("b", vec![completed("done")]),
        ("synth", vec![completed("done")]),
    ]);
    let (runner, calls) = scripted_runner(answers, false);
    let result = resume(&dir, flow, runner);
    assert_eq!(result.status, "completed");
    // t1 ran once in total; the branch continued at a2 with t1's transition.
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "ran\n");
    assert_eq!(count(&calls, "a2"), 1);
    let store = RunStore::for_runs_dir(dir.join("runs"));
    let paths = store.find_run_root("run-contract").unwrap().unwrap();
    let root = store.node_execution_root(&paths, "fork", 1, 0).unwrap();
    let status: Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("status.json")).unwrap()).unwrap();
    let branch = &status["context_updates"]["parallel.results"][0];
    assert_eq!(branch["status"], json!("completed"));
    assert_eq!(branch["route_trace"], json!(["t1", "a2", "join"]));
    // The visit counts were restored, not reset.
    assert_eq!(branch["visits"]["t1"], json!(1));
    assert_eq!(branch["visits"]["a2"], json!(1));
}

#[test]
fn an_interrupted_pause_node_in_a_branch_requires_a_recovery_decision() {
    let temp = tempfile::tempdir().unwrap();
    let dir = std::env::var(INTERRUPT_CHILD_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| temp.path().to_path_buf());
    let log = dir.join("side-effect.log");
    // The tool kills its own runner while it runs. A child process runs the
    // variant its parent names.
    let variants = match std::env::var(INTERRUPT_PAUSE_ENV) {
        Ok(pause) => vec![pause == "true"],
        Err(_) => vec![true, false],
    };
    for pause in variants {
        let flow = interrupted_fork_flow(&log, "; kill -9 $PPID; sleep 5", pause);
        if std::env::var(INTERRUPT_CHILD_ENV).is_err() {
            let _ = std::fs::remove_dir_all(dir.join("runs"));
        }
        run_until_interrupted(
            &dir,
            &flow,
            "an_interrupted_pause_node_in_a_branch_requires_a_recovery_decision",
        );
        let store = RunStore::for_runs_dir(dir.join("runs"));
        let paths = store.find_run_root("run-contract").unwrap().unwrap();
        let pending =
            attractor_runtime::fork_branch_requiring_recovery_decision(&paths, &flow, "fork", 1, 0);
        assert_eq!(pending.as_deref(), pause.then_some("t1"), "pause={pause}");
    }
}

/// Branches `t1` and `t2` are tools running `commands`; both join.
fn tool_fork_flow(first: &str, second: &str) -> FlowDefinition {
    let tool = |command: &str| {
        json!({"kind": "tool", "config": {"kind": "tool", "command": command, "exit_codes": {"0": "done"}},
            "routes": {"done": route("join")}})
    };
    parse_flow(json!({
        "start": {"kind": "start", "routes": {"next": route("fork")}},
        "fork": {"kind": "parallel", "config": {"kind": "parallel", "branches": ["t1", "t2"], "join": "join"}},
        "t1": tool(first),
        "t2": tool(second),
        "join": {"kind": "fan_in", "routes": {"next": route("synth")}},
        "synth": agent(json!({"done": route("done")})),
        "done": exit("completed", "success"),
    }))
}

#[test]
fn branch_tools_drain_output_beyond_pipe_capacity_without_a_timeout() {
    let chatty =
        "head -c 1048576 /dev/zero | tr '\\000' a; head -c 1048576 /dev/zero | tr '\\000' b >&2";
    let (runner, calls) = scripted_runner(script(&[("synth", vec![completed("done")])]), false);
    let run = execute(tool_fork_flow(chatty, chatty), runner, None);
    assert_eq!(
        run.result.status,
        "completed",
        "{}",
        run.record().last_error
    );
    assert_eq!(count(&calls, "synth"), 1);
    assert!(fork_results(&run)
        .iter()
        .all(|record| record["status"] == "completed"));
}

#[test]
fn external_cancellation_stops_branch_tools_that_never_return_on_their_own() {
    let temp = tempfile::tempdir().unwrap();
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let canceler = store.clone();
    // Cancel once both branches are inside their tools.
    std::thread::spawn(move || {
        let paths = loop {
            if let Some(paths) = canceler.find_run_root("run-contract").ok().flatten() {
                let branches = paths.logs_dir().join("fork/executions/1-0/branches");
                if (0..2).all(|index| branches.join(format!("{index}.progress.json")).exists()) {
                    break paths;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        std::thread::sleep(std::time::Duration::from_millis(200));
        let mut record = attractor_runtime::read_run_record(&paths).unwrap().unwrap();
        attractor_runtime::mark_record_cancel_requested(&mut record);
        attractor_runtime::write_run_record(&paths, &record).unwrap();
    });
    let control_store = store.clone();
    let (runner, calls) = scripted_runner(script(&[("synth", vec![completed("done")])]), false);
    let started = std::time::Instant::now();
    let result = PipelineExecutor::with_control(runner, move || {
        let paths = control_store.find_run_root("run-contract").ok().flatten()?;
        let record = control_store.read_run_record(&paths).ok().flatten()?;
        (record.status == "cancel_requested")
            .then_some(attractor_runtime::ExecutionControlAction::Cancel)
    })
    .execute(ExecuteRunRequest {
        store: store.clone(),
        record: RunRecord::new("run-contract", temp.path().to_string_lossy()),
        flow: tool_fork_flow("sleep 60", "sleep 60"),
        flow_definition_json: None,
        launch_context: LaunchContext::empty(),
        runtime_context: ContextMap::new(),
        max_steps: None,
        start: Default::default(),
    })
    .unwrap();
    assert_eq!(result.status, "canceled");
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    assert_eq!(count(&calls, "synth"), 0);
    let paths = store.find_run_root("run-contract").unwrap().unwrap();
    let branches = paths.logs_dir().join("fork/executions/1-0/branches");
    for index in 0..2 {
        let record: Value = serde_json::from_str(
            &std::fs::read_to_string(branches.join(format!("{index}.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(record["status"], json!("canceled"), "{record}");
        assert_eq!(
            record["interruption"],
            json!("tool command stopped by cancellation")
        );
    }
}

#[test]
fn a_tool_interrupted_by_its_fork_is_canceled_not_failed() {
    // t1 blocks immediately; t2's running tool is stopped by the fork's
    // cancellation and recorded as canceled with that evidence.
    let mut flow = tool_fork_flow("true", "sleep 60");
    flow.nodes.get_mut("t1").unwrap().routes =
        serde_json::from_value(json!({"done": route("failed")})).unwrap();
    flow.nodes.insert(
        "failed".to_string(),
        serde_json::from_value(exit("blocked", "failure")).unwrap(),
    );
    let (runner, _calls) = scripted_runner(script(&[]), false);
    let run = execute(flow, runner, None);
    assert_eq!(run.result.status, "completed");
    assert_eq!(run.record().outcome_reason_code.as_deref(), Some("blocked"));
    let results = fork_results(&run);
    assert_eq!(results[0]["status"], json!("terminal_result"));
    assert_eq!(results[1]["status"], json!("canceled"));
    assert_eq!(
        results[1]["interruption"],
        json!("tool command stopped by cancellation")
    );
}

#[test]
fn an_unvalidated_success_exit_branch_cannot_end_the_owner_successfully() {
    // Built without validation, which rejects it, to exercise the runtime boundary.
    let flow: FlowDefinition = serde_json::from_value(json!({
        "schema_version": "2", "id": "contract",
        "nodes": {
            "start": {"kind": "start", "routes": {"next": route("fork")}},
            "fork": {"kind": "parallel", "config": {"kind": "parallel", "branches": ["done"], "join": "join"}},
            "join": {"kind": "fan_in", "routes": {"next": route("done")}},
            "done": exit("completed", "success"),
        }
    }))
    .unwrap();
    assert!(flow.validate().is_err());
    let (runner, _calls) = scripted_runner(script(&[]), false);
    let run = execute(flow, runner, None);
    assert_eq!(run.result.status, "failed");
    assert!(
        run.record()
            .last_error
            .contains("success exit done before its join"),
        "{}",
        run.record().last_error
    );
    assert!(!run.result.route_trace.contains(&"join".to_string()));
}

fn child_result(status: &str, result: Option<&str>) -> attractor_runtime::ChildRunResult {
    serde_json::from_value(json!({
        "run_id": "child-1",
        "status": status,
        "outcome": result.map(|_| "success"),
        "outcome_reason_code": result,
        "failure_reason": if status == "failed" { "child crashed" } else { "" },
    }))
    .unwrap()
}

/// A subflow watching a linked child that is still running for `running`
/// polls, then ends with `last`.
fn run_bounded_manager(
    manager: Value,
    running: usize,
    last: attractor_runtime::ChildRunResult,
) -> (Run, usize) {
    let flow = parse_flow(json!({
        "start": {"kind": "start", "routes": {"next": route("manager")}},
        "manager": {"kind": "subflow", "manager": manager,
            "config": {"kind": "subflow", "flow_ref": "child",
                "results": {"completed": "child_completed", "blocked": "child_blocked"}},
            "routes": {"child_completed": route("done"), "child_blocked": route("failed")}},
        "done": exit("completed", "success"),
        "failed": exit("blocked", "failure"),
    }));
    let polls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = polls.clone();
    let runner = RuntimeHandlerRunner::new().with_child_status_resolver(move |child_id| {
        assert_eq!(child_id, "child-1");
        let poll = seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Some(if poll < running {
            child_result("running", None)
        } else {
            last.clone()
        })
    });
    let temp = tempfile::tempdir().unwrap();
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let result = PipelineExecutor::new(runner)
        .execute(ExecuteRunRequest {
            store: store.clone(),
            record: RunRecord::new("run-contract", temp.path().to_string_lossy()),
            flow,
            flow_definition_json: None,
            launch_context: LaunchContext::empty(),
            runtime_context: ContextMap::from([(
                "context.stack.child.run_id".to_string(),
                json!("child-1"),
            )]),
            max_steps: None,
            start: Default::default(),
        })
        .unwrap();
    let polls = polls.load(std::sync::atomic::Ordering::SeqCst);
    (
        Run {
            result,
            store,
            _temp: temp,
            workdir: PathBuf::new(),
        },
        polls,
    )
}

#[test]
fn a_subflow_keeps_waiting_on_the_same_child_after_its_observation_bounds() {
    for manager in [
        json!({"child_autostart": false, "max_cycles": 1, "poll_interval": "0s", "actions": ["observe", "wait"]}),
        json!({"child_autostart": false, "poll_interval": "0s", "actions": ["observe", "wait"],
            "stop_condition": "context.stack.child.status=running"}),
    ] {
        // The child finishes well after the bound ended observation.
        let (run, polls) = run_bounded_manager(
            manager.clone(),
            4,
            child_result("completed", Some("completed")),
        );
        assert_eq!(run.result.status, "completed", "{manager}");
        assert_eq!(run.result.current_node, "done", "{manager}");
        assert!(polls >= 5, "{manager}: {polls}");

        let (run, _) = run_bounded_manager(
            manager.clone(),
            4,
            child_result("completed", Some("blocked")),
        );
        assert_eq!(
            run.record().outcome_reason_code.as_deref(),
            Some("blocked"),
            "{manager}"
        );

        // A child failure after the bound fails the parent with its identity.
        let (run, _) = run_bounded_manager(manager.clone(), 4, child_result("failed", None));
        assert_eq!(run.result.status, "failed", "{manager}");
        assert_eq!(
            run.record().last_error,
            "child child-1 failed: child crashed"
        );
    }
}

/// Answers every agent node through the real codergen path, so the
/// adapter writes each invocation's prompt, response and status.
struct RoundBackend {
    answers: Arc<Mutex<Script>>,
}

impl spark_agent_adapter::CodergenBackend for RoundBackend {
    fn run(
        &mut self,
        request: spark_agent_adapter::CodergenBackendRequest,
    ) -> Result<spark_agent_adapter::CodergenBackendOutput, spark_agent_adapter::CodergenError>
    {
        let mut answers = self.answers.lock().unwrap();
        let queue = answers.get_mut(&request.node_id).expect("scripted node");
        let answer = queue.pop_front().expect("answer");
        Ok(spark_agent_adapter::CodergenBackendOutput::text(
            json!({"decision": answer.decision, "notes": answer.notes}).to_string(),
        ))
    }
}

#[test]
fn every_fork_round_keeps_its_own_branch_execution_evidence() {
    let mut flow = fork_flow(false);
    flow.nodes.get_mut("synth").unwrap().routes = serde_json::from_value(json!({
        "again": route("fork"), "done": route("done"), "blocked": route("failed")
    }))
    .unwrap();
    flow.validate().unwrap();
    let note = |decision: &str, notes: &str| Outcome {
        notes: notes.to_string(),
        ..completed(decision)
    };
    let answers = script(&[
        (
            "a",
            vec![note("done", "a round one"), note("done", "a round two")],
        ),
        (
            "b",
            vec![note("done", "b round one"), note("done", "b round two")],
        ),
        (
            "synth",
            vec![note("again", "again"), note("done", "finished")],
        ),
    ]);
    let runner = RuntimeHandlerRunner::new().with_codergen_backend_factory(move || {
        Box::new(RoundBackend {
            answers: answers.clone(),
        })
    });
    let run = execute(flow, runner, None);
    assert_eq!(
        run.result.status,
        "completed",
        "{}",
        run.record().last_error
    );
    let paths = run.store.find_run_root("run-contract").unwrap().unwrap();
    for node in ["a", "b"] {
        let executions = paths.logs_dir().join(node).join("executions");
        let mut responses = std::fs::read_dir(&executions)
            .unwrap()
            .map(|entry| {
                std::fs::read_to_string(entry.unwrap().path().join("response.md")).unwrap()
            })
            .collect::<Vec<_>>();
        responses.sort();
        assert_eq!(responses.len(), 2, "{node}: {responses:?}");
        assert!(
            responses[0].contains(&format!("{node} round one")),
            "{responses:?}"
        );
        assert!(
            responses[1].contains(&format!("{node} round two")),
            "{responses:?}"
        );
    }
}

/// A tool whose descendant would write `effect` after a delay.
fn nested_effect_command(effect: &Path) -> String {
    format!(
        "sh -c 'sleep 8; echo effect > \"{}\"'; echo done",
        effect.display()
    )
}

#[test]
fn canceling_a_branch_tool_stops_its_descendants_promptly() {
    let temp = tempfile::tempdir().unwrap();
    let effect = temp.path().join("effect");
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let canceler = store.clone();
    std::thread::spawn(move || {
        let paths = loop {
            if let Some(paths) = canceler.find_run_root("run-contract").ok().flatten() {
                let branches = paths.logs_dir().join("fork/executions/1-0/branches");
                if (0..2).all(|index| branches.join(format!("{index}.progress.json")).exists()) {
                    break paths;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        std::thread::sleep(std::time::Duration::from_millis(200));
        let mut record = attractor_runtime::read_run_record(&paths).unwrap().unwrap();
        attractor_runtime::mark_record_cancel_requested(&mut record);
        attractor_runtime::write_run_record(&paths, &record).unwrap();
    });
    let control_store = store.clone();
    let (runner, _calls) = scripted_runner(script(&[("synth", vec![completed("done")])]), false);
    let command = nested_effect_command(&effect);
    let started = std::time::Instant::now();
    let result = PipelineExecutor::with_control(runner, move || {
        let paths = control_store.find_run_root("run-contract").ok().flatten()?;
        let record = control_store.read_run_record(&paths).ok().flatten()?;
        (record.status == "cancel_requested")
            .then_some(attractor_runtime::ExecutionControlAction::Cancel)
    })
    .execute(ExecuteRunRequest {
        store: store.clone(),
        record: RunRecord::new("run-contract", temp.path().to_string_lossy()),
        flow: tool_fork_flow(&command, &command),
        flow_definition_json: None,
        launch_context: LaunchContext::empty(),
        runtime_context: ContextMap::new(),
        max_steps: None,
        start: Default::default(),
    })
    .unwrap();
    assert_eq!(result.status, "canceled");
    // Settled well before the descendant's own 8 s sleep would end.
    assert!(
        started.elapsed() < std::time::Duration::from_secs(6),
        "{:?}",
        started.elapsed()
    );
    std::thread::sleep(std::time::Duration::from_secs(9).saturating_sub(started.elapsed()));
    assert!(!effect.exists(), "a canceled descendant wrote its effect");
}

#[test]
fn a_tool_timeout_stops_its_descendants() {
    let temp = tempfile::tempdir().unwrap();
    let effect = temp.path().join("effect");
    let mut flow = tool_flow(
        json!({"command": nested_effect_command(&effect), "exit_codes": {"0": "passed", "1": "rejected"}}),
        json!({"passed": route("after"), "rejected": route("repair")}),
    );
    flow.nodes.get_mut("check").unwrap().runtime = Some(attractor_core::NodeRuntimeConfig {
        timeout: Some("300ms".to_string()),
        ..Default::default()
    });
    let (runner, _calls) = scripted_runner(script(&[]), false);
    let started = std::time::Instant::now();
    let run = execute_in(temp, flow, runner, None);
    assert_eq!(run.result.status, "failed");
    assert!(run.record().last_error.contains("timed out"));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(6),
        "{:?}",
        started.elapsed()
    );
    std::thread::sleep(std::time::Duration::from_secs(9).saturating_sub(started.elapsed()));
    assert!(!effect.exists(), "a timed-out descendant wrote its effect");
}

/// A child whose `work` agent loops onto itself past its single visit
/// (a real routing failure), or returns `work_error` as its result.
fn write_looping_child(root: &Path) {
    let flow = parse_flow(json!({
        "start": {"kind": "start", "routes": {"next": route("work")}},
        "work": {"kind": "agent_task", "contracts": {"response": "decision_envelope"},
            "runtime": {"max_visits": 1},
            "routes": {"again": route("work"), "done": route("done")}},
        "done": exit("completed", "success"),
    }));
    attractor_dsl::write_flow_dir(&root.join("flows/child"), &flow).unwrap();
}

/// A tolerant fork whose first branch runs the child as a subflow.
fn subflow_fork_flow(autostart: bool) -> FlowDefinition {
    parse_flow(json!({
        "start": {"kind": "start", "routes": {"next": route("fork")}},
        "fork": {"kind": "parallel", "config": {"kind": "parallel", "branches": ["sub", "b"],
            "join": "join", "tolerate_branch_execution_errors": true}},
        "sub": {"kind": "subflow",
            "manager": {"child_autostart": autostart, "poll_interval": "0s"},
            "config": {"kind": "subflow", "flow_ref": "child", "results": {"completed": "child_completed"}},
            "routes": {"child_completed": route("join")}},
        "b": agent(json!({"done": route("join")})),
        "join": {"kind": "fan_in", "routes": {"next": route("synth")}},
        "synth": agent(json!({"done": route("done")})),
        "done": exit("completed", "success"),
    }))
}

fn child_answers(work: Outcome) -> (RuntimeHandlerRunner, Arc<Mutex<Vec<String>>>) {
    scripted_runner(
        script(&[
            ("work", vec![work]),
            ("b", vec![completed("done")]),
            ("synth", vec![completed("done")]),
        ]),
        false,
    )
}

fn run_parent(
    temp: &Path,
    flow: FlowDefinition,
    runner: RuntimeHandlerRunner,
    context: ContextMap,
) -> PipelineExecutionResult {
    let mut runtime_context = ContextMap::from([(
        "internal.bundled_root".to_string(),
        json!(temp.to_string_lossy()),
    )]);
    runtime_context.extend(context);
    PipelineExecutor::new(runner)
        .execute(ExecuteRunRequest {
            store: RunStore::for_runs_dir(temp.join("runs")),
            record: RunRecord::new("run-parent", temp.to_string_lossy()),
            flow,
            flow_definition_json: None,
            launch_context: LaunchContext::empty(),
            runtime_context,
            max_steps: None,
            start: Default::default(),
        })
        .unwrap()
}

#[test]
fn a_child_routing_failure_stays_fatal_inside_a_tolerant_fork() {
    let temp = tempfile::tempdir().unwrap();
    write_looping_child(temp.path());
    let (runner, calls) = child_answers(completed("again"));
    let result = run_parent(
        temp.path(),
        subflow_fork_flow(true),
        runner,
        ContextMap::new(),
    );
    assert_eq!(result.status, "failed");
    assert_eq!(result.failure_kind, Some(FailureKind::Routing));
    assert!(
        result.failure_reason.contains("visit_limit_reached"),
        "{}",
        result.failure_reason
    );
    assert!(
        result.failure_reason.contains("child "),
        "{}",
        result.failure_reason
    );
    assert!(!result.route_trace.contains(&"join".to_string()));
    assert_eq!(count(&calls, "synth"), 0);

    // The child's durable record carries the classification.
    let store = RunStore::for_runs_dir(temp.path().join("runs"));
    let child = store
        .list_run_records()
        .unwrap()
        .into_iter()
        .find(|record| record.parent_run_id.as_deref() == Some("run-parent"))
        .expect("child run");
    assert_eq!(child.status, "failed");
    assert_eq!(child.failure_kind, Some(FailureKind::Routing));
}

#[test]
fn a_recovered_parent_keeps_its_failed_childs_routing_class() {
    let temp = tempfile::tempdir().unwrap();
    write_looping_child(temp.path());
    // The child failed on its own run before the parent observes it.
    let child_flow = attractor_dsl::read_flow_dir(&temp.path().join("flows/child")).unwrap();
    let (runner, _) = child_answers(completed("again"));
    let child = PipelineExecutor::new(runner)
        .execute(ExecuteRunRequest {
            store: RunStore::for_runs_dir(temp.path().join("runs")),
            record: RunRecord::new("child-1", temp.path().to_string_lossy()),
            flow: child_flow,
            flow_definition_json: None,
            launch_context: LaunchContext::empty(),
            runtime_context: ContextMap::new(),
            max_steps: None,
            start: Default::default(),
        })
        .unwrap();
    assert_eq!(child.status, "failed");
    // The parent re-observes that linked child from its durable record.
    let (runner, calls) = child_answers(completed("done"));
    let result = run_parent(
        temp.path(),
        subflow_fork_flow(false),
        runner,
        ContextMap::from([("context.stack.child.run_id".to_string(), json!("child-1"))]),
    );
    assert_eq!(result.status, "failed");
    assert_eq!(result.failure_kind, Some(FailureKind::Routing));
    assert!(
        result
            .failure_reason
            .contains("child child-1 failed: visit_limit_reached"),
        "{}",
        result.failure_reason
    );
    assert!(!result.route_trace.contains(&"join".to_string()));
    assert_eq!(count(&calls, "synth"), 0);
}

#[test]
fn a_child_execution_error_stays_tolerated_inside_a_tolerant_fork() {
    let temp = tempfile::tempdir().unwrap();
    write_looping_child(temp.path());
    let (runner, calls) = child_answers(Outcome::error(
        FailureKind::Runtime,
        "provider down",
        Some(false),
    ));
    let result = run_parent(
        temp.path(),
        subflow_fork_flow(true),
        runner,
        ContextMap::new(),
    );
    assert_eq!(result.status, "completed", "{}", result.failure_reason);
    assert_eq!(count(&calls, "synth"), 1);
    let results = result.context["parallel.results"].as_array().unwrap();
    assert_eq!(results[0]["status"], json!("error"));
    assert_eq!(results[0]["error"]["kind"], json!("runtime"));
    assert!(results[0]["error"]["reason"]
        .as_str()
        .unwrap()
        .contains("provider down"));
}

/// A child flow that waits on an unanswered human gate.
fn write_gate_child(root: &Path) {
    let flow = parse_flow(json!({
        "start": {"kind": "start", "routes": {"next": route("gate")}},
        "gate": {"kind": "human_gate", "config": {"kind": "human_gate", "prompt": "Approve?"},
            "routes": {"approve": route("done")}},
        "done": exit("completed", "success"),
    }));
    attractor_dsl::write_flow_dir(&root.join("flows/gated"), &flow).unwrap();
}

/// Branch `sub` runs the gated child by default launch; branch `b` answers
/// with `b_answer` once that child waits on its gate.
fn gated_child_fork(tolerate: bool) -> FlowDefinition {
    parse_flow(json!({
        "start": {"kind": "start", "routes": {"next": route("fork")}},
        "fork": {"kind": "parallel", "config": {"kind": "parallel", "branches": ["sub", "b"],
            "join": "join", "tolerate_branch_execution_errors": tolerate}},
        "sub": {"kind": "subflow", "manager": {"poll_interval": "0s"},
            "config": {"kind": "subflow", "flow_ref": "gated", "results": {"completed": "child_completed"}},
            "routes": {"child_completed": route("join")}},
        "b": agent(json!({"done": route("join"), "blocked": route("failed")})),
        "join": {"kind": "fan_in", "routes": {"next": route("synth")}},
        "synth": agent(json!({"done": route("done")})),
        "done": exit("completed", "success"),
        "failed": exit("blocked", "failure"),
    }))
}

fn gated_runner(
    runs: PathBuf,
    b_answer: Outcome,
    exit_after_wait: bool,
) -> (RuntimeHandlerRunner, Arc<Mutex<Vec<String>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = calls.clone();
    let mut runner = RuntimeHandlerRunner::new().with_blocking_human_gates();
    runner.register_thread_safe_handler_fn(HANDLER_CODERGEN, move |runtime: HandlerRuntime| {
        seen.lock().unwrap().push(runtime.node_id.clone());
        if runtime.node_id == "b" {
            // Wait until the child run sits on its unanswered gate.
            let store = RunStore::for_runs_dir(runs.clone());
            while !store
                .list_run_records()
                .unwrap_or_default()
                .iter()
                .any(|record| {
                    record.parent_node_id.as_deref() == Some("sub") && record.status == "waiting"
                })
            {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            if exit_after_wait {
                std::process::exit(7);
            }
            return Ok(b_answer.clone());
        }
        Ok(completed("done"))
    });
    (runner, calls)
}

fn gated_child_record(runs: &Path) -> RunRecord {
    RunStore::for_runs_dir(runs.to_path_buf())
        .list_run_records()
        .unwrap()
        .into_iter()
        .find(|record| record.parent_node_id.as_deref() == Some("sub"))
        .expect("child run")
}

#[test]
fn a_blocked_sibling_cancels_a_waiting_child_and_settles_it() {
    let temp = tempfile::tempdir().unwrap();
    write_gate_child(temp.path());
    let runs = temp.path().join("runs");
    let (runner, calls) = gated_runner(runs.clone(), completed("blocked"), false);
    let result = run_parent(
        temp.path(),
        gated_child_fork(false),
        runner,
        ContextMap::new(),
    );
    assert_eq!(result.status, "completed", "{}", result.failure_reason);
    assert_eq!(result.outcome.as_deref(), Some("failure"));
    assert_eq!(result.outcome_reason_code.as_deref(), Some("blocked"));
    assert!(!result.route_trace.contains(&"join".to_string()));
    assert_eq!(count(&calls, "synth"), 0);
    assert_eq!(gated_child_record(&runs).status, "canceled");
}

#[test]
fn a_fatal_sibling_cancels_a_waiting_child_and_fails_the_owner() {
    let temp = tempfile::tempdir().unwrap();
    write_gate_child(temp.path());
    let runs = temp.path().join("runs");
    let fatal = Outcome::error(FailureKind::Routing, "child result unmapped", Some(false));
    let (runner, calls) = gated_runner(runs.clone(), fatal, false);
    let result = run_parent(
        temp.path(),
        gated_child_fork(true),
        runner,
        ContextMap::new(),
    );
    assert_eq!(result.status, "failed");
    assert_eq!(result.failure_kind, Some(FailureKind::Routing));
    assert!(
        result.failure_reason.contains("child result unmapped"),
        "{}",
        result.failure_reason
    );
    assert!(!result.route_trace.contains(&"join".to_string()));
    assert_eq!(count(&calls, "synth"), 0);
    assert_eq!(gated_child_record(&runs).status, "canceled");
}

const GATED_CHILD_ENV: &str = "SPARK_GATED_CHILD_INTERRUPT_DIR";

#[test]
fn a_recovered_fork_cancels_its_waiting_child_when_a_sibling_blocks() {
    let temp = tempfile::tempdir().unwrap();
    let dir = std::env::var(GATED_CHILD_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| temp.path().to_path_buf());
    let runs = dir.join("runs");
    if std::env::var(GATED_CHILD_ENV).is_ok() {
        // The process dies while the child waits on its gate.
        let (runner, _) = gated_runner(runs, completed("blocked"), true);
        run_parent(&dir, gated_child_fork(false), runner, ContextMap::new());
        panic!("the parent should have been interrupted");
    }
    write_gate_child(&dir);
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "routing_model_contracts::a_recovered_fork_cancels_its_waiting_child_when_a_sibling_blocks",
            "--nocapture",
        ])
        .env(GATED_CHILD_ENV, &dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success());
    assert_eq!(gated_child_record(&runs).status, "waiting");

    let store = RunStore::for_runs_dir(runs.clone());
    let bundle = store.read_run_bundle("run-parent").unwrap().unwrap();
    let checkpoint = bundle.checkpoint.unwrap();
    assert_eq!(checkpoint.current_node, "fork");
    let (runner, calls) = gated_runner(runs.clone(), completed("blocked"), false);
    let result = PipelineExecutor::new(runner)
        .execute(ExecuteRunRequest {
            store,
            record: bundle.record.unwrap(),
            flow: gated_child_fork(false),
            flow_definition_json: None,
            launch_context: LaunchContext::empty(),
            runtime_context: ContextMap::new(),
            max_steps: None,
            start: ExecutionStart::Resume {
                paths: bundle.paths,
                checkpoint,
            },
        })
        .unwrap();
    assert_eq!(result.status, "completed", "{}", result.failure_reason);
    assert_eq!(result.outcome_reason_code.as_deref(), Some("blocked"));
    assert!(!result.route_trace.contains(&"join".to_string()));
    assert_eq!(count(&calls, "synth"), 0);
    // The recovered invocation adopted the existing child and settled it.
    let children = RunStore::for_runs_dir(runs)
        .list_run_records()
        .unwrap()
        .into_iter()
        .filter(|record| record.parent_node_id.as_deref() == Some("sub"))
        .collect::<Vec<_>>();
    assert_eq!(children.len(), 1, "{children:?}");
    assert_eq!(children[0].status, "canceled");
}
