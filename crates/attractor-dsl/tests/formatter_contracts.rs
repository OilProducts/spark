use std::fs;
use std::path::PathBuf;

use attractor_dsl::{
    canonicalize_dot, canonicalize_readable_dot, format_dot, format_readable_dot, parse_dot,
    semantic_equivalent, DotValue,
};
use serde_json::Value;

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

fn line_index(lines: &[&str], text: &str) -> usize {
    lines
        .iter()
        .position(|line| *line == text)
        .unwrap_or_else(|| panic!("missing line: {text}"))
}

#[test]
fn format_readable_dot_matches_branching_fixture() {
    let fixture = fixture_json("dsl/format-canonical-branching.json");
    let source = fixture["input"]["dot"].as_str().expect("fixture input");
    let expected = fixture["observation"]["canonical_dot"]
        .as_str()
        .expect("canonical dot");

    let formatted = format_readable_dot(&parse_dot(source).expect("parse fixture"));

    assert_eq!(formatted, expected);
    assert_eq!(
        canonicalize_readable_dot(&formatted).expect("reparse"),
        expected
    );
}

#[test]
fn format_readable_dot_places_start_before_downstream_nodes() {
    let graph = parse_dot(
        r#"
digraph Workflow {
  done [shape=Msquare];
  task [shape=box];
  start [shape=Mdiamond];
  task -> done;
  start -> task;
}
"#,
    )
    .expect("parse graph");

    let formatted = format_readable_dot(&graph);
    let lines = formatted.lines().collect::<Vec<_>>();

    assert!(
        line_index(&lines, r#"  start [shape="Mdiamond"];"#)
            < line_index(&lines, r#"  task [shape="box"];"#)
    );
    assert!(
        line_index(&lines, r#"  task [shape="box"];"#)
            < line_index(&lines, r#"  done [shape="Msquare"];"#)
    );
}

#[test]
fn format_readable_dot_precedes_each_node_with_blank_line() {
    let graph = parse_dot(
        r#"
digraph Workflow {
  start [shape=Mdiamond];
  task [shape=box];
  done [shape=Msquare];
  start -> task;
  task -> done;
}
"#,
    )
    .expect("parse graph");

    let formatted = format_readable_dot(&graph);
    let lines = formatted.lines().collect::<Vec<_>>();

    for node_line in [
        r#"  start [shape="Mdiamond"];"#,
        r#"  task [shape="box"];"#,
        r#"  done [shape="Msquare"];"#,
    ] {
        assert_eq!(lines[line_index(&lines, node_line) - 1], "");
    }
}

#[test]
fn format_readable_dot_emits_outgoing_edges_after_source_node() {
    let graph = parse_dot(
        r#"
digraph Workflow {
  start [shape=Mdiamond];
  task [shape=box];
  done [shape=Msquare];
  start -> task;
  task -> done;
}
"#,
    )
    .expect("parse graph");

    let formatted = format_readable_dot(&graph);
    let lines = formatted.lines().collect::<Vec<_>>();

    assert_eq!(
        lines[line_index(&lines, r#"  start [shape="Mdiamond"];"#) + 1],
        "  start -> task;"
    );
    assert_eq!(
        lines[line_index(&lines, r#"  task [shape="box"];"#) + 1],
        "  task -> done;"
    );
}

#[test]
fn format_readable_dot_preserves_branch_edge_order() {
    let graph = parse_dot(
        r#"
digraph Workflow {
  start [shape=Mdiamond];
  first [shape=box];
  second [shape=box];
  done [shape=Msquare];
  start -> second [label="Second"];
  start -> first [label="First"];
  second -> done;
  first -> done;
}
"#,
    )
    .expect("parse graph");

    let formatted = format_readable_dot(&graph);
    let lines = formatted.lines().collect::<Vec<_>>();

    assert!(
        line_index(&lines, r#"  start -> second [label="Second"];"#)
            < line_index(&lines, r#"  start -> first [label="First"];"#)
    );
    assert!(
        line_index(&lines, r#"  second [shape="box"];"#)
            < line_index(&lines, r#"  first [shape="box"];"#)
    );
}

#[test]
fn format_readable_dot_emits_loop_edge_without_duplicating_node() {
    let graph = parse_dot(
        r#"
digraph Workflow {
  start [shape=Mdiamond];
  task [shape=box];
  done [shape=Msquare];
  start -> task;
  task -> task [label="Retry"];
  task -> done;
}
"#,
    )
    .expect("parse graph");

    let formatted = format_readable_dot(&graph);
    let lines = formatted.lines().collect::<Vec<_>>();

    assert_eq!(
        lines
            .iter()
            .filter(|line| **line == r#"  task [shape="box"];"#)
            .count(),
        1
    );
    assert!(lines.contains(&r#"  task -> task [label="Retry"];"#));
}

#[test]
fn format_readable_dot_appends_unreachable_nodes_in_parse_order() {
    let graph = parse_dot(
        r#"
digraph Workflow {
  orphan_b [shape=box];
  start [shape=Mdiamond];
  done [shape=Msquare];
  orphan_a [shape=box];
  start -> done;
  orphan_b -> orphan_a;
}
"#,
    )
    .expect("parse graph");

    let formatted = format_readable_dot(&graph);
    let lines = formatted.lines().collect::<Vec<_>>();

    assert!(
        line_index(&lines, r#"  done [shape="Msquare"];"#)
            < line_index(&lines, r#"  orphan_b [shape="box"];"#)
    );
    assert!(
        line_index(&lines, r#"  orphan_b [shape="box"];"#)
            < line_index(&lines, r#"  orphan_a [shape="box"];"#)
    );
    assert_eq!(
        lines[line_index(&lines, r#"  orphan_b [shape="box"];"#) + 1],
        "  orphan_b -> orphan_a;"
    );
}

#[test]
fn format_readable_dot_preserves_same_line_node_declaration_order() {
    let graph =
        parse_dot("digraph Workflow { b [shape=box]; a [shape=box]; }\n").expect("parse graph");

    assert_eq!(
        format_readable_dot(&graph),
        "digraph Workflow {\n\n  b [shape=\"box\"];\n\n  a [shape=\"box\"];\n}\n"
    );
}

#[test]
fn format_dot_remains_canonical_and_stable() {
    let dot = r#"
digraph Workflow {
  b [z=1, a=2];
  graph [label="Label", goal="Goal"];
  b -> a [weight=0, label="next"];
  a [shape=box];
  a -> b;
}
"#;

    assert_eq!(
        format_dot(&parse_dot(dot).expect("parse graph")),
        "digraph Workflow {\n  graph [goal=\"Goal\", label=\"Label\"];\n  a [shape=\"box\"];\n  b [a=2, z=1];\n  a -> b;\n  b -> a [label=\"next\", weight=0];\n}\n"
    );

    let canonical = canonicalize_dot(dot).expect("canonicalize");
    assert_eq!(canonicalize_dot(&canonical).expect("idempotent"), canonical);
}

#[test]
fn format_dot_renders_typed_values_and_spark_extension_attrs() {
    let graph = parse_dot(
        r#"
digraph Typed {
  graph [spark.title="Title", score=1.0, enabled=true];
  node [shape=box, timeout=250ms];
  start [shape=Mdiamond, prompt="Line\nTab\tQuote\"Backslash\\"];
  start -> done [weight=2, fidelity=0.5, auto_status=false];
  done [shape=Msquare];
}
"#,
    )
    .expect("parse graph");

    let formatted = format_readable_dot(&graph);

    assert!(formatted.contains("spark.title=\"Title\""));
    assert!(formatted.contains("score=1.0"));
    assert!(formatted.contains("enabled=true"));
    assert!(formatted.contains("timeout=250ms"));
    assert!(formatted.contains("fidelity=0.5"));
    assert!(formatted.contains("auto_status=false"));
    assert!(formatted.contains(r#"prompt="Line\nTab\tQuote\"Backslash\\""#));
    assert_eq!(
        parse_dot(&formatted).expect("reparse").nodes["start"].attrs["prompt"].value,
        DotValue::String("Line\nTab\tQuote\"Backslash\\".to_string())
    );
}

#[test]
fn format_dot_uses_python_repr_float_thresholds() {
    let graph = parse_dot(
        r#"
digraph Floats {
  start [shape=Mdiamond, exact=1.0, small=0.000001, negative=-0.000001, threshold=0.0001, mid=1000000000000000.0, large=10000000000000000.0];
  done [shape=Msquare];
  start -> done [weight=0.000012];
}
"#,
    )
    .expect("parse graph");

    let canonical = format_dot(&graph);
    assert_eq!(
        canonical,
        "digraph Floats {\n  done [shape=\"Msquare\"];\n  start [exact=1.0, large=1e+16, mid=1000000000000000.0, negative=-1e-06, shape=\"Mdiamond\", small=1e-06, threshold=0.0001];\n  start -> done [weight=1.2e-05];\n}\n"
    );
    assert_eq!(
        canonicalize_dot(&canonical).expect("reparse canonical"),
        canonical
    );
    assert_eq!(
        parse_dot(&canonical).expect("reparse canonical").nodes["start"].attrs["small"].value,
        DotValue::Float(0.000001)
    );

    let readable = format_readable_dot(&graph);
    assert!(readable.contains("small=1e-06"));
    assert!(readable.contains("large=1e+16"));
    assert!(readable.contains("weight=1.2e-05"));
    assert_eq!(
        canonicalize_readable_dot(&readable).expect("reparse readable"),
        readable
    );
}

#[test]
fn semantic_equivalence_ignores_order_and_graph_id_but_rejects_meaning_changes() {
    let left = parse_dot(
        r#"
digraph Original {
  start [shape=Mdiamond, prompt="Go"];
  done [shape=Msquare];
  start -> done [label="ship"];
}
"#,
    )
    .expect("parse left");
    let reordered = parse_dot(
        r#"
digraph Reordered {
  done [shape=Msquare];
  start -> done [label="ship"];
  start [prompt="Go", shape=Mdiamond];
}
"#,
    )
    .expect("parse reordered");
    let changed = parse_dot(
        r#"
digraph Changed {
  done [shape=Msquare];
  start [prompt="Stop", shape=Mdiamond];
  start -> done [label="ship"];
}
"#,
    )
    .expect("parse changed");

    assert!(semantic_equivalent(&left, &reordered));
    assert!(!semantic_equivalent(&left, &changed));
}
