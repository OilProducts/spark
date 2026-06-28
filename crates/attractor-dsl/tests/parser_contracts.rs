use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use attractor_dsl::{
    normalize_graph, parse_dot, DotAttribute, DotGraph, DotNode, DotSubgraphScope, DotValue,
    DotValueType, DurationLiteral,
};
use serde_json::{json, Value};

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/compat/fixtures")
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
fn parses_typed_defaults_fixture_payload() {
    let fixture = fixture_json("dsl/parse-valid-typed-defaults.json");
    let source = fixture["input"]["dot"].as_str().expect("fixture input");
    let graph = parse_dot(source).expect("parse fixture");

    assert_eq!(graph_payload(&graph), fixture["observation"]["graph"]);
}

#[test]
fn parses_spark_extension_attrs_fixture_payload() {
    let fixture = fixture_json("dsl/parse-valid-spark-extension-attrs.json");
    let source = fixture["input"]["dot"].as_str().expect("fixture input");
    let graph = parse_dot(source).expect("parse fixture");

    assert_eq!(graph_payload(&graph), fixture["observation"]["graph"]);
}

#[test]
fn rejects_unsupported_constructs_fixture_payload() {
    let fixture = fixture_json("dsl/parse-reject-unsupported-constructs.json");
    let cases = fixture["input"]["cases"]
        .as_object()
        .expect("fixture cases");
    let mut observed = Vec::new();
    for (case_id, source) in cases.iter().collect::<BTreeMap<_, _>>() {
        match parse_dot(source.as_str().expect("source")) {
            Ok(_) => observed.push(json!({
                "case_id": case_id,
                "result": "accepted",
            })),
            Err(error) => observed.push(json!({
                "case_id": case_id,
                "result": "rejected",
                "error_type": "DotParseError",
                "message": error.to_string(),
                "line": error.line(),
            })),
        }
    }

    assert_eq!(Value::Array(observed), fixture["observation"]["cases"]);
}

#[test]
fn parses_comments_typed_values_defaults_subgraphs_and_chained_edges() {
    let dot = r#"
    // graph comment
    DIGRAPH Demo {
        graph [goal="Ship", default_max_retries=5]
        node [shape=box, timeout=+0900s]
        edge [weight=2]

        start [shape=Mdiamond, label="Start"]
        plan [prompt="Plan for $goal", threshold=.5, enabled=false]
        done [shape=Msquare]

        subgraph cluster_loop {
            graph [label="Loop - A! #1"]
            node [thread_id="loop-a"]
            review [prompt="Review", class="critical, critical"]
        }

        start -> plan -> done [label="next"]
    }
    "#;

    let graph = parse_dot(dot).expect("graph");

    assert_eq!(graph.graph_id, "Demo");
    assert_eq!(graph.goal(), "Ship");
    assert_eq!(
        graph.nodes["plan"].attrs["timeout"].value.to_string(),
        "900s"
    );
    assert_eq!(
        graph.nodes["plan"].attrs["threshold"].value,
        DotValue::Float(0.5)
    );
    assert_eq!(
        graph.nodes["review"].attrs["class"].value,
        DotValue::String("critical,loop-a-1".to_string())
    );
    assert_eq!(
        graph
            .edges
            .iter()
            .map(|edge| (edge.source.as_str(), edge.target.as_str()))
            .collect::<Vec<_>>(),
        [("start", "plan"), ("plan", "done")]
    );
    assert_eq!(graph.edges[0].attrs["weight"].value, DotValue::Integer(2));
    assert_eq!(graph.subgraphs[0].node_ids, ["review"]);
}

#[test]
fn normalizes_chained_edges_to_expanded_edges() {
    let chained = r#"
    digraph G {
        edge [weight=2]
        a -> b -> c [label="next", timeout=5s]
    }
    "#;
    let expanded = r#"
    digraph G {
        edge [weight=2]
        a -> b [label="next", timeout=5s]
        b -> c [label="next", timeout=5s]
    }
    "#;

    assert_eq!(
        normalize_graph(&parse_dot(chained).expect("chained")),
        normalize_graph(&parse_dot(expanded).expect("expanded"))
    );
}

#[test]
fn parses_scientific_float_literals_emitted_by_formatter() {
    let graph = parse_dot(
        r#"
    digraph Floats {
        start [shape=Mdiamond, small=1e-06, negative=-1e-06, large=1e+16]
        done [shape=Msquare]
        start -> done [weight=1.2e-05]
    }
    "#,
    )
    .expect("scientific floats parse");

    assert_eq!(
        graph.nodes["start"].attrs["small"].value,
        DotValue::Float(0.000001)
    );
    assert_eq!(
        graph.nodes["start"].attrs["negative"].value,
        DotValue::Float(-0.000001)
    );
    assert_eq!(
        graph.nodes["start"].attrs["large"].value,
        DotValue::Float(10000000000000000.0)
    );
    assert_eq!(
        graph.edges[0].attrs["weight"].value,
        DotValue::Float(0.000012)
    );
}

#[test]
fn reports_compatibility_errors_for_rejected_syntax() {
    let cases = [
        (
            "strict digraph G { a -> b }",
            "strict modifier is not supported",
        ),
        (
            "graph G { a -> b }",
            "undirected graph declarations are not supported",
        ),
        (
            "digraph G { a -- b }",
            "undirected edges ('--') are not supported",
        ),
        (
            "digraph G { a [label=<b>Bold</b>] }",
            "HTML-like labels are not supported",
        ),
        (
            "digraph G { a:out -> b }",
            "port and compass point syntax is not supported",
        ),
        (
            "digraph G { a [shape=box,] }",
            "trailing comma is not allowed in attribute blocks",
        ),
        (
            "digraph G { a [model..provider=\"openai\"] }",
            "invalid attribute key",
        ),
    ];

    for (source, expected) in cases {
        let error = parse_dot(source).expect_err("source should be rejected");
        assert!(
            error.to_string().contains(expected),
            "expected {expected:?} in {error}"
        );
    }
}
