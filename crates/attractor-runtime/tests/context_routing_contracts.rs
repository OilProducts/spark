use std::path::PathBuf;

use attractor_core::{
    AttractorContext, ContextMap, DotGraph, LaunchContext, Outcome, OutcomeStatus,
};
use attractor_dsl::parse_dot;
use attractor_runtime::{
    apply_outcome_context_updates, checkpoint_from_context, initialize_runtime_context,
    resolve_start_node, select_next_node,
};
use serde_json::{json, Value};

fn context_map(entries: impl IntoIterator<Item = (&'static str, Value)>) -> ContextMap {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn fixture_graph(name: &str) -> DotGraph {
    let fixture_path = repo_root()
        .join(".spark/rust-rewrite/current/compat-fixtures/runtime")
        .join(format!("{name}.json"));
    let fixture: Value = serde_json::from_str(
        &std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", fixture_path.display())),
    )
    .expect("fixture json");
    let dot = fixture["input"]["dot"].as_str().expect("fixture input dot");
    parse_dot(dot).expect("fixture dot parses")
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("repo root")
        .to_path_buf()
}

fn success() -> Outcome {
    Outcome::new(OutcomeStatus::Success)
}

fn apply_node(
    graph: &DotGraph,
    context: &mut AttractorContext,
    node_id: &str,
    outcome: &Outcome,
) -> Outcome {
    apply_outcome_context_updates(node_id, &graph.nodes[node_id].attrs, context, outcome)
        .expect("apply outcome")
}

#[test]
fn launch_initialization_mirrors_graph_attrs_applies_launch_and_snapshots() {
    let graph = fixture_graph("executor-context-write-contracts");
    let start = resolve_start_node(&graph).expect("start node");
    let launch = LaunchContext::new(context_map([("context.request.id", json!("REQ-1"))]))
        .expect("launch context");
    let mut context = initialize_runtime_context(&graph, &start, &launch).expect("runtime context");

    assert_eq!(context.get("graph.goal"), Some(&json!("Context contract")));
    assert_eq!(context.get("graph.release"), Some(&json!("canary")));
    assert_eq!(context.get("graph.default_max_retries"), Some(&json!(0)));
    assert_eq!(context.get("context.request.id"), Some(&json!("REQ-1")));
    assert_eq!(context.get("current_node"), Some(&json!("start")));
    assert_eq!(context.get("outcome"), Some(&json!("")));
    assert_eq!(context.get("preferred_label"), Some(&json!("")));
    assert_eq!(context.get("_attractor.node_outcomes"), Some(&json!({})));

    context
        .set("context.request.id", Value::Null)
        .expect("delete launch key");
    let checkpoint = checkpoint_from_context(
        "start",
        Vec::<String>::new(),
        &context,
        Vec::<(String, u64)>::new(),
    );
    assert_eq!(checkpoint.current_node, "start");
    assert!(!checkpoint.context.contains_key("context.request.id"));
    assert_eq!(checkpoint.context["graph.goal"], json!("Context contract"));
}

#[test]
fn context_updates_normalize_delete_and_enforce_write_contracts() {
    let graph = fixture_graph("executor-context-write-contracts");
    let mut context = initialize_runtime_context(&graph, "start", &LaunchContext::empty())
        .expect("runtime context");
    context
        .set("context.remove", json!("old value"))
        .expect("seed removable key");

    let allowed = Outcome {
        context_updates: context_map([
            ("context.keep", json!("kept")),
            ("context.remove", Value::Null),
        ]),
        ..success()
    };
    let allowed = apply_node(&graph, &mut context, "allowed", &allowed);
    assert_eq!(allowed.status, OutcomeStatus::Success);
    assert_eq!(allowed.context_updates["context.keep"], json!("kept"));
    assert_eq!(context.get("context.keep"), Some(&json!("kept")));
    assert_eq!(context.get("context.remove"), None);
    assert_eq!(context.get("outcome"), Some(&json!("success")));

    let denied = Outcome {
        context_updates: context_map([("artifact_path", json!("logs/denied/response.md"))]),
        ..success()
    };
    let denied = apply_node(&graph, &mut context, "denied", &denied);
    assert_eq!(denied.status, OutcomeStatus::Fail);
    assert_eq!(denied.retryable, Some(false));
    assert_eq!(
        denied.failure_reason,
        "undeclared context_updates keys for node 'denied': artifact_path; declared spark.writes_context allowlist: <none>"
    );
    assert_eq!(context.get("artifact_path"), None);
    assert_eq!(context.get("outcome"), Some(&json!("fail")));
    assert_eq!(
        context.get("_attractor.node_outcomes"),
        Some(&json!({"allowed": "success", "denied": "fail"}))
    );
}

#[test]
fn codergen_builtin_context_updates_do_not_require_authored_write_contracts() {
    let graph = parse_dot(
        r#"
        digraph G {
          task [shape=box, prompt="Plan"];
          explicit [type="codergen", prompt="Review"];
          toolish [type="tool", shape=box];
        }
        "#,
    )
    .expect("dot parses");
    let mut context = initialize_runtime_context(&graph, "task", &LaunchContext::empty())
        .expect("runtime context");

    let task_outcome = Outcome {
        context_updates: context_map([
            ("last_stage", json!("task")),
            ("last_response", json!("backend text")),
        ]),
        ..success()
    };
    let applied_task = apply_node(&graph, &mut context, "task", &task_outcome);
    assert_eq!(applied_task.status, OutcomeStatus::Success);
    assert_eq!(context.get("last_stage"), Some(&json!("task")));
    assert_eq!(context.get("last_response"), Some(&json!("backend text")));

    let explicit_outcome = Outcome {
        context_updates: context_map([
            ("last_stage", json!("explicit")),
            ("last_response", json!("explicit text")),
        ]),
        ..success()
    };
    let applied_explicit = apply_node(&graph, &mut context, "explicit", &explicit_outcome);
    assert_eq!(applied_explicit.status, OutcomeStatus::Success);
    assert_eq!(context.get("last_stage"), Some(&json!("explicit")));

    let tool_outcome = Outcome {
        context_updates: context_map([("last_response", json!("not exempt for tool"))]),
        ..success()
    };
    let applied_tool = apply_node(&graph, &mut context, "toolish", &tool_outcome);
    assert_eq!(applied_tool.status, OutcomeStatus::Fail);
    assert_eq!(
        applied_tool.failure_reason,
        "undeclared context_updates keys for node 'toolish': last_response; declared spark.writes_context allowlist: <none>"
    );
}

#[test]
fn route_success_partial_fixture_routes_condition_before_unconditional() {
    let graph = fixture_graph("executor-route-success-partial");
    let mut context = initialize_runtime_context(
        &graph,
        &resolve_start_node(&graph).expect("start"),
        &LaunchContext::empty(),
    )
    .expect("runtime context");
    let mut trace = vec!["start".to_string()];

    let start_outcome = apply_node(&graph, &mut context, "start", &success());
    let selected =
        select_next_node(&graph, "start", &start_outcome, &context).expect("start route");
    assert_eq!(selected.selected_node.as_deref(), Some("plan"));
    trace.push("plan".to_string());

    let plan = Outcome {
        status: OutcomeStatus::PartialSuccess,
        preferred_label: "needs_review".to_string(),
        context_updates: context_map([("context.plan", json!("partial"))]),
        ..Outcome::new(OutcomeStatus::PartialSuccess)
    };
    let plan = apply_node(&graph, &mut context, "plan", &plan);
    let selected = select_next_node(&graph, "plan", &plan, &context).expect("plan route");
    assert_eq!(selected.selected_node.as_deref(), Some("review"));
    assert_eq!(selected.reason, "condition");
    trace.push("review".to_string());

    let review = Outcome {
        context_updates: context_map([("context.reviewed", json!(true))]),
        ..success()
    };
    let review = apply_node(&graph, &mut context, "review", &review);
    let selected = select_next_node(&graph, "review", &review, &context).expect("review route");
    assert_eq!(selected.selected_node.as_deref(), Some("done"));
    trace.push("done".to_string());

    assert_eq!(trace, ["start", "plan", "review", "done"]);
    assert_eq!(context.get("context.plan"), Some(&json!("partial")));
    assert_eq!(context.get("context.reviewed"), Some(&json!(true)));
}

#[test]
fn condition_routing_fixture_does_not_fallback_unqualified_dotted_keys_to_context_namespace() {
    let graph = fixture_graph("executor-condition-routing");
    let mut context = initialize_runtime_context(
        &graph,
        &resolve_start_node(&graph).expect("start"),
        &LaunchContext::empty(),
    )
    .expect("runtime context");
    let mut trace = vec!["start".to_string()];

    let start_outcome = apply_node(&graph, &mut context, "start", &success());
    let selected =
        select_next_node(&graph, "start", &start_outcome, &context).expect("start route");
    assert_eq!(selected.selected_node.as_deref(), Some("decide"));
    trace.push("decide".to_string());

    let decide = Outcome {
        preferred_label: "ship".to_string(),
        context_updates: context_map([("context.flag", json!(true)), ("flat.key", json!("flat"))]),
        ..success()
    };
    let decide = apply_node(&graph, &mut context, "decide", &decide);
    assert_eq!(context.get("context.flat.key"), Some(&json!("flat")));
    assert_eq!(context.get("flat.key"), None);
    let selected = select_next_node(&graph, "decide", &decide, &context).expect("decide route");
    assert_eq!(selected.selected_node.as_deref(), Some("fallback"));
    assert_eq!(selected.reason, "unconditional");
    trace.push("fallback".to_string());

    let fallback = apply_node(&graph, &mut context, "fallback", &success());
    let selected =
        select_next_node(&graph, "fallback", &fallback, &context).expect("fallback route");
    assert_eq!(selected.selected_node.as_deref(), Some("done"));
    trace.push("done".to_string());

    assert_eq!(trace, ["start", "decide", "fallback", "done"]);
}
