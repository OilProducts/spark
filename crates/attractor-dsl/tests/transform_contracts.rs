use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use attractor_core::{AttractorContext, ContextMap};
use attractor_dsl::{
    apply_graph_transforms_with_extra, format_dot, graph_attr_context_seed, parse_dot,
    AttributeDefaultsTransform, DotAttribute, DotGraph, DotNode, DotSubgraphScope, DotValue,
    DotValueType, DurationLiteral, GoalVariableTransform, GraphTransform, ModelStylesheetTransform,
    RuntimePreambleTransform,
};
use serde_json::{json, Value};

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("crates/test-fixtures/compat")
        .join(name)
}

fn fixture_json(name: &str) -> Value {
    let path = fixture_path(name);
    serde_json::from_str(&fs::read_to_string(&path).expect("fixture readable"))
        .expect("fixture json")
}

fn attr_payload(attr: &DotAttribute) -> Value {
    json!({
        "value": dot_value_payload(&attr.value),
        "value_type": value_type_payload(attr.value_type),
    })
}

fn attrs_payload(attrs: &BTreeMap<String, DotAttribute>) -> Value {
    Value::Object(
        attrs
            .iter()
            .map(|(key, attr)| (key.clone(), attr_payload(attr)))
            .collect(),
    )
}

fn dot_value_payload(value: &DotValue) -> Value {
    match value {
        DotValue::Null => Value::Null,
        DotValue::String(value) => json!(value),
        DotValue::Integer(value) => json!(value),
        DotValue::Float(value) => json!(value),
        DotValue::Boolean(value) => json!(value),
        DotValue::Duration(DurationLiteral { raw, unit, value }) => {
            json!({ "raw": raw, "unit": unit, "value": value })
        }
    }
}

fn value_type_payload(value_type: DotValueType) -> &'static str {
    match value_type {
        DotValueType::String => "string",
        DotValueType::Integer => "integer",
        DotValueType::Float => "float",
        DotValueType::Boolean => "boolean",
        DotValueType::Duration => "duration",
    }
}

fn node_payload(node: &DotNode) -> Value {
    json!({
        "attrs": attrs_payload(&node.attrs),
        "explicit_attr_keys": node.explicit_attr_keys.iter().cloned().collect::<Vec<_>>(),
    })
}

fn subgraph_payload(subgraph: &DotSubgraphScope) -> Value {
    json!({
        "id": subgraph.id,
        "attrs": attrs_payload(&subgraph.attrs),
        "node_ids": subgraph.node_ids,
        "defaults": {
            "node": attrs_payload(&subgraph.defaults.node),
            "edge": attrs_payload(&subgraph.defaults.edge),
        },
        "subgraphs": subgraph.subgraphs.iter().map(subgraph_payload).collect::<Vec<_>>(),
    })
}

fn graph_payload(graph: &DotGraph) -> Value {
    json!({
        "graph_id": graph.graph_id,
        "graph_attrs": attrs_payload(&graph.graph_attrs),
        "defaults": {
            "node": attrs_payload(&graph.defaults.node),
            "edge": attrs_payload(&graph.defaults.edge),
        },
        "nodes": Value::Object(
            graph
                .nodes
                .iter()
                .map(|(node_id, node)| (node_id.clone(), node_payload(node)))
                .collect(),
        ),
        "edges": graph
            .edges
            .iter()
            .map(|edge| {
                json!({
                    "source": edge.source,
                    "target": edge.target,
                    "attrs": attrs_payload(&edge.attrs),
                })
            })
            .collect::<Vec<_>>(),
        "subgraphs": graph.subgraphs.iter().map(subgraph_payload).collect::<Vec<_>>(),
    })
}

#[test]
fn attribute_defaults_match_fixture_observations() {
    let fixture = fixture_json("dsl/transform-attribute-defaults.json");
    let source = fixture["input"]["dot"].as_str().expect("fixture input");
    let mut graph = parse_dot(source).expect("parse fixture");

    AttributeDefaultsTransform::with_child_workdir("__WORKTREE__").apply(&mut graph);
    let payload = graph_payload(&graph);

    assert_eq!(
        payload["graph_attrs"],
        fixture["observation"]["graph_attrs"]
    );
    assert_eq!(
        payload["nodes"]["task"]["attrs"],
        fixture["observation"]["task_attrs"]
    );
    assert_eq!(
        payload["edges"][0]["attrs"],
        fixture["observation"]["edge_attrs"]
    );
    assert!(graph.nodes["task"].explicit_attr_keys.is_empty());
}

#[test]
fn goal_variable_and_runtime_preamble_match_fixture_observations() {
    let fixture = fixture_json("dsl/transform-goal-runtime-preamble.json");
    let source = fixture["input"]["dot"].as_str().expect("fixture input");
    let mut graph = parse_dot(source).expect("parse fixture");

    GoalVariableTransform.apply(&mut graph);

    let mut values = ContextMap::new();
    values.insert("graph.goal".to_string(), json!("Ship transform parity"));
    values.insert("internal.run_id".to_string(), json!("run-transform"));
    values.insert("context.release".to_string(), json!("v1"));
    values.insert(
        "_attractor.node_outcomes".to_string(),
        json!({"start": "success"}),
    );
    let context = AttractorContext::from_map(values).expect("valid context");
    let completed_nodes = vec!["start".to_string()];
    let preamble =
        RuntimePreambleTransform::new().apply("summary:high", &context, &completed_nodes);

    assert_eq!(graph_payload(&graph), fixture["observation"]["graph"]);
    assert_eq!(preamble, fixture["observation"]["preamble"]);
}

#[test]
fn model_stylesheet_precedence_matches_fixture_observations() {
    let fixture = fixture_json("dsl/transform-stylesheet-precedence.json");
    let source = fixture["input"]["dot"].as_str().expect("fixture input");
    let mut graph = parse_dot(source).expect("parse fixture");

    ModelStylesheetTransform.apply(&mut graph);
    let payload = graph_payload(&graph);

    assert_eq!(
        payload["nodes"]["plan"]["attrs"],
        fixture["observation"]["plan_attrs"]
    );
    assert_eq!(
        payload["nodes"]["review"]["attrs"],
        fixture["observation"]["review_attrs"]
    );
}

#[test]
fn graph_attr_context_seed_matches_fixture_observations() {
    let fixture = fixture_json("dsl/transform-graph-attrs-context-mirror.json");
    let source = fixture["input"]["dot"].as_str().expect("fixture input");
    let graph = parse_dot(source).expect("parse fixture");
    let seed = graph_attr_context_seed(&graph);

    let mut merged = seed.clone();
    for (key, value) in fixture["input"]["launch_context"]
        .as_object()
        .expect("launch context")
    {
        merged.insert(key.clone(), value.clone());
    }

    assert_eq!(
        graph_payload(&graph)["graph_attrs"],
        fixture["observation"]["graph_attrs"]
    );
    assert_eq!(json!(seed), fixture["observation"]["context_seed"]);
    assert_eq!(json!(merged), fixture["observation"]["merged_context"]);
}

#[derive(Debug)]
struct AssertPromptExpandedTransform;

impl GraphTransform for AssertPromptExpandedTransform {
    fn apply(&self, graph: &mut DotGraph) {
        assert_eq!(
            graph.nodes["task"].attrs["prompt"].value,
            DotValue::String("Plan for Ship it".to_string())
        );
        graph.graph_attrs.insert(
            "extra_seen".to_string(),
            DotAttribute {
                key: "extra_seen".to_string(),
                value: DotValue::Boolean(true),
                value_type: DotValueType::Boolean,
                line: 0,
            },
        );
    }
}

#[test]
fn transform_pipeline_clones_input_and_runs_extra_transforms_after_builtins() {
    let graph = parse_dot(
        r#"
digraph Pipeline {
  graph [goal="Ship it"];
  task [shape=box, prompt="Plan for $goal"];
}
"#,
    )
    .expect("parse graph");

    let transformed = apply_graph_transforms_with_extra(
        &graph,
        vec![Box::new(AssertPromptExpandedTransform) as Box<dyn GraphTransform>],
    );

    assert_eq!(
        graph.nodes["task"].attrs["prompt"].value,
        DotValue::String("Plan for $goal".to_string())
    );
    assert_eq!(
        transformed.nodes["task"].attrs["prompt"].value,
        DotValue::String("Plan for Ship it".to_string())
    );
    assert_eq!(
        transformed.graph_attrs["extra_seen"].value,
        DotValue::Boolean(true)
    );
}

#[test]
fn stylesheet_respects_explicit_attrs_and_uses_graph_defaults_as_fallbacks() {
    let mut graph = parse_dot(
        r#"
digraph Styles {
  graph [llm_profile="balanced", model_stylesheet="* { llm_model: styled; llm_provider: styled-provider; reasoning_effort: low; }"];
  node [llm_model="inherited"];
  inherited [shape=box];
  explicit [shape=box, llm_model="explicit"];
}
"#,
    )
    .expect("parse graph");

    ModelStylesheetTransform.apply(&mut graph);

    assert_eq!(
        graph.nodes["inherited"].attrs["llm_model"].value,
        DotValue::String("styled".to_string())
    );
    assert_eq!(
        graph.nodes["inherited"].attrs["llm_profile"].value,
        DotValue::String("balanced".to_string())
    );
    assert_eq!(
        graph.nodes["explicit"].attrs["llm_model"].value,
        DotValue::String("explicit".to_string())
    );
    assert_eq!(
        graph.nodes["explicit"].attrs["llm_provider"].value,
        DotValue::String("styled-provider".to_string())
    );
}

#[test]
fn invalid_stylesheet_rules_are_ignored_without_diagnostics() {
    let mut graph = parse_dot(
        r#"
digraph InvalidStyles {
  graph [model_stylesheet="* { llm_model: ; } .bad class { llm_provider: openai; }"];
  task [shape=box];
}
"#,
    )
    .expect("parse graph");

    ModelStylesheetTransform.apply(&mut graph);

    assert_eq!(
        graph.nodes["task"].attrs["llm_model"].value,
        DotValue::String(String::new())
    );
    assert_eq!(
        graph.nodes["task"].attrs["llm_provider"].value,
        DotValue::String(String::new())
    );
    assert_eq!(
        graph.nodes["task"].attrs["reasoning_effort"].value,
        DotValue::String("high".to_string())
    );
}

#[test]
fn runtime_preamble_includes_retry_metadata_and_filters_internal_context() {
    let mut values = ContextMap::new();
    values.insert("graph.goal".to_string(), json!("Retry goal"));
    values.insert("internal.run_id".to_string(), json!("run-retry"));
    values.insert("context.visible".to_string(), json!({"b": 2, "a": 1}));
    values.insert("internal.hidden".to_string(), json!("nope"));
    values.insert("_attractor.hidden".to_string(), json!("nope"));
    values.insert(
        "_attractor.node_outcomes".to_string(),
        json!({"task": "failed"}),
    );
    values.insert(
        "_attractor.runtime.retry.node_id".to_string(),
        json!("task"),
    );
    values.insert("_attractor.runtime.retry.attempt".to_string(), json!("2"));
    values.insert(
        "_attractor.runtime.retry.max_attempts".to_string(),
        json!(3),
    );
    values.insert(
        "_attractor.runtime.retry.failure_reason".to_string(),
        json!("boom"),
    );
    let context = AttractorContext::from_map(values).expect("valid context");
    let completed_nodes = vec!["task".to_string()];

    let preamble = RuntimePreambleTransform::new().apply("compact", &context, &completed_nodes);

    assert_eq!(
        preamble,
        "carryover:compact\ngoal=Retry goal\nrun_id=run-retry\ncompleted=task:failed\nretry.node_id=task\nretry.attempt=2\nretry.max_attempts=3\nretry.failure_reason=boom\n- context.visible={\"a\":1,\"b\":2}"
    );
}

#[test]
fn spark_extension_attrs_survive_transform_pipeline() {
    let fixture = fixture_json("dsl/parse-valid-spark-extension-attrs.json");
    let source = fixture["input"]["dot"].as_str().expect("fixture input");
    let graph = parse_dot(source).expect("parse fixture");
    let transformed = attractor_dsl::apply_graph_transforms(&graph);
    let payload = graph_payload(&transformed);

    for key in [
        "spark.title",
        "spark.description",
        "spark.launch_inputs",
        "spark.result_node",
        "spark.result_summary_body",
        "spark.result_summary_title",
        "ui_default_goal",
    ] {
        assert_eq!(
            payload["graph_attrs"][key],
            fixture["observation"]["graph"]["graph_attrs"][key]
        );
    }
    assert_eq!(
        payload["nodes"]["summarize"]["attrs"]["spark.reads_context"],
        fixture["observation"]["graph"]["nodes"]["summarize"]["attrs"]["spark.reads_context"]
    );
    assert_eq!(
        payload["nodes"]["summarize"]["attrs"]["spark.writes_context"],
        fixture["observation"]["graph"]["nodes"]["summarize"]["attrs"]["spark.writes_context"]
    );
}

#[test]
fn formatter_renders_transform_null_values_as_empty_quoted_values() {
    let mut graph = parse_dot("digraph Nulls { task [shape=box]; }\n").expect("parse graph");
    graph.nodes.get_mut("task").expect("task").attrs.insert(
        "timeout".to_string(),
        DotAttribute {
            key: "timeout".to_string(),
            value: DotValue::Null,
            value_type: DotValueType::Duration,
            line: 0,
        },
    );

    assert!(format_dot(&graph).contains(r#"timeout="""#));
}
