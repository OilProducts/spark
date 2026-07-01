use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use attractor_dsl::{
    graph_payload, normalize_flow_name, preview_dot_source, preview_response_payload_with_options,
    DotGraph, DotScopeDefaults, DotSubgraphScope, DotValue, DotValueType, PreviewOptions,
};
use serde_json::{json, Number, Value};

#[test]
fn preview_status_and_parse_errors_match_dsl_fixture() {
    let fixture = fixture_json("dsl/preview-status-and-errors.json");
    let accepted_dot = fixture["input"]["accepted_dot"]
        .as_str()
        .expect("accepted dot");
    let parse_error_dot = fixture["input"]["parse_error_dot"]
        .as_str()
        .expect("parse error dot");

    let accepted = preview_dot_source(accepted_dot);
    assert_eq!(
        accepted.payload,
        fixture["observation"]["accepted"]["payload"]
    );
    assert_eq!(
        graph_contract_payload(accepted.graph.as_ref().expect("accepted graph")),
        fixture["observation"]["accepted"]["graph"]
    );

    let parse_error = preview_dot_source(parse_error_dot);
    assert!(parse_error.graph.is_none());
    assert_eq!(
        parse_error.payload,
        fixture["observation"]["parse_error"]["payload"]
    );
}

#[test]
fn flow_name_normalization_matches_path_safety_fixture() {
    let fixture = fixture_json("dsl/flow-name-path-safety.json");
    let accepted = fixture["observation"]["accepted"]
        .as_array()
        .expect("accepted cases");
    for case in accepted {
        let input = case["input"].as_str().expect("input");
        let normalized = case["normalized"].as_str().expect("normalized");
        assert_eq!(normalize_flow_name(input).expect("normalized"), normalized);
    }

    let rejected = fixture["observation"]["rejected"]
        .as_array()
        .expect("rejected cases");
    for case in rejected {
        let input = case["input"].as_str().expect("input");
        if case["accepted"].as_bool().unwrap_or(false) {
            assert!(normalize_flow_name(input).is_ok());
            continue;
        }
        let error = normalize_flow_name(input).expect_err("flow source error");
        assert_eq!(
            error.status_code(),
            case["status_code"].as_u64().unwrap() as u16
        );
        assert_eq!(error.detail(), case["detail"].as_str().unwrap());
    }
}

#[test]
fn preview_route_graph_payload_matches_http_fixture() {
    let fixture = fixture_json("http/attractor-preview-success.json");
    let flow_content = fixture["request"]["body"]["json"]["flow_content"]
        .as_str()
        .expect("flow content");
    let payload = preview_response_payload_with_options(flow_content, PreviewOptions::default());

    assert_eq!(payload, fixture["response"]["body"]["json"]);
}

#[test]
fn preview_route_parse_error_payload_matches_http_fixture() {
    let fixture = fixture_json("http/attractor-preview-parse-error.json");
    let flow_content = fixture["request"]["body"]["json"]["flow_content"]
        .as_str()
        .expect("flow content");
    let payload = preview_response_payload_with_options(flow_content, PreviewOptions::default());

    assert_eq!(payload, fixture["response"]["body"]["json"]);
}

#[test]
fn preview_payload_exposes_full_manager_loop_authoring_surface() {
    let flow = r#"
    digraph ManagerLoop {
      manager [
        shape=house,
        type=stack.manager_loop,
        manager.poll_interval=25ms,
        manager.max_cycles=4,
        manager.stop_condition="context.stack.child.ready=true",
        manager.actions="observe,steer",
        manager.steer_cooldown=2s,
        stack.child_autostart=false
      ];
    }
    "#;

    let payload = preview_response_payload_with_options(flow, PreviewOptions::default());
    let manager = payload["graph"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == "manager")
        .unwrap();

    assert_eq!(manager["manager.poll_interval"], "25ms");
    assert_eq!(manager["manager.max_cycles"], 4);
    assert_eq!(
        manager["manager.stop_condition"],
        "context.stack.child.ready=true"
    );
    assert_eq!(manager["manager.actions"], "observe,steer");
    assert_eq!(manager["manager.steer_cooldown"], "2s");
    assert_eq!(manager["stack.child_autostart"], false);
}

#[test]
fn graph_payload_preserves_spark_extension_attrs_and_excludes_unsupported_human_default_choice() {
    let flow = r#"
    digraph Extensions {
      graph [goal="Keep attrs", spark.title="Title", ui_default_llm_model="fast"];
      start [shape=Mdiamond];
      task [shape=box, prompt="Do", spark.writes_context="[\"context.done\"]", human.default_choice="skip"];
      done [shape=Msquare];
      start -> task [spark.edge_flag="yes"];
      task -> done;
    }
    "#;
    let payload = preview_response_payload_with_options(flow, PreviewOptions::default());
    let graph = &payload["graph"];

    assert_eq!(graph["graph_attrs"]["spark.title"], "Title");
    assert_eq!(graph["graph_attrs"]["ui_default_llm_model"], "fast");
    let task = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == "task")
        .unwrap();
    assert_eq!(task["spark.writes_context"], "[\"context.done\"]");
    assert!(task.get("human.default_choice").is_none());
    assert_eq!(graph["edges"][0]["spark.edge_flag"], "yes");
}

#[test]
fn child_preview_expands_only_for_manager_loop_nodes_and_ignores_missing_children() {
    let temp = tempfile::tempdir().expect("tempdir");
    let child_path = temp.path().join("child.dot");
    fs::write(
        &child_path,
        r#"
        digraph Child {
          graph [label="Child label"];
          start [shape=Mdiamond];
          done [shape=Msquare];
          start -> done;
        }
        "#,
    )
    .expect("write child");
    let parent = r#"
    digraph Parent {
      graph [stack.child_dotfile="child.dot"];
      start [shape=Mdiamond];
      manager [shape=house, prompt="Watch"];
      worker [shape=box, prompt="Do"];
      done [shape=Msquare];
      start -> manager -> worker -> done;
    }
    "#;

    let payload = preview_response_payload_with_options(
        parent,
        PreviewOptions {
            expand_children: true,
            flow_source_dir: Some(temp.path().to_path_buf()),
            run_workdir: None,
        },
    );
    let child_previews = payload["graph"]["child_previews"].as_object().unwrap();
    assert_eq!(
        child_previews.keys().cloned().collect::<Vec<_>>(),
        vec!["manager".to_string()]
    );
    assert_eq!(child_previews["manager"]["flow_name"], "child.dot");
    assert_eq!(child_previews["manager"]["flow_label"], "Child label");
    assert_eq!(child_previews["manager"]["parent_node_id"], "manager");
    assert_eq!(child_previews["manager"]["read_only"], true);
    assert_eq!(
        child_previews["manager"]["provenance"],
        "derived_child_preview"
    );
    assert_eq!(
        child_previews["manager"]["flow_path"],
        child_path.to_string_lossy().to_string()
    );

    let missing = parent.replace("child.dot", "missing.dot");
    let missing_payload = preview_response_payload_with_options(
        &missing,
        PreviewOptions {
            expand_children: true,
            flow_source_dir: Some(temp.path().to_path_buf()),
            run_workdir: None,
        },
    );
    assert!(missing_payload["graph"].get("child_previews").is_none());
}

#[test]
fn preview_nodes_follow_declaration_order_not_map_key_order() {
    let flow = r#"
    digraph Ordered {
      b [shape=Mdiamond];
      a [shape=box, prompt="A"];
      c [shape=Msquare];
      b -> a -> c;
    }
    "#;

    let payload = preview_response_payload_with_options(flow, PreviewOptions::default());
    let ids = payload["graph"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();

    assert_eq!(ids, vec!["b", "a", "c"]);
}

fn fixture_json(relative: &str) -> Value {
    let path = workspace_root()
        .join("crates")
        .join("test-fixtures")
        .join("compat")
        .join(relative);
    serde_json::from_str(
        &fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("unable to read fixture {}: {error}", path.display())),
    )
    .expect("fixture json")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn graph_contract_payload(graph: &DotGraph) -> Value {
    json!({
        "graph_id": graph.graph_id,
        "graph_attrs": attrs_contract_payload(&graph.graph_attrs),
        "defaults": {
            "node": attrs_contract_payload(&graph.defaults.node),
            "edge": attrs_contract_payload(&graph.defaults.edge),
        },
        "nodes": graph.nodes.iter().map(|(node_id, node)| {
            (
                node_id.clone(),
                json!({
                    "attrs": attrs_contract_payload(&node.attrs),
                    "explicit_attr_keys": node.explicit_attr_keys.iter().cloned().collect::<Vec<_>>(),
                }),
            )
        }).collect::<serde_json::Map<_, _>>(),
        "edges": graph.edges.iter().map(|edge| {
            json!({
                "source": edge.source,
                "target": edge.target,
                "attrs": attrs_contract_payload(&edge.attrs),
            })
        }).collect::<Vec<_>>(),
        "subgraphs": graph.subgraphs.iter().map(subgraph_contract_payload).collect::<Vec<_>>(),
    })
}

fn attrs_contract_payload(attrs: &BTreeMap<String, attractor_dsl::DotAttribute>) -> Value {
    Value::Object(
        attrs
            .iter()
            .map(|(key, attr)| {
                (
                    key.clone(),
                    json!({
                        "value": dot_value_contract_payload(&attr.value),
                        "value_type": dot_value_type_text(attr.value_type),
                    }),
                )
            })
            .collect(),
    )
}

fn dot_value_contract_payload(value: &DotValue) -> Value {
    match value {
        DotValue::Null => Value::Null,
        DotValue::String(value) => Value::String(value.clone()),
        DotValue::Integer(value) => Value::Number(Number::from(*value)),
        DotValue::Float(value) => Number::from_f64(*value)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(value.to_string())),
        DotValue::Boolean(value) => Value::Bool(*value),
        DotValue::Duration(value) => json!({
            "raw": value.raw,
            "value": value.value,
            "unit": value.unit,
        }),
    }
}

fn dot_value_type_text(value_type: DotValueType) -> &'static str {
    match value_type {
        DotValueType::String => "string",
        DotValueType::Integer => "integer",
        DotValueType::Float => "float",
        DotValueType::Boolean => "boolean",
        DotValueType::Duration => "duration",
    }
}

fn subgraph_contract_payload(subgraph: &DotSubgraphScope) -> Value {
    json!({
        "id": subgraph.id,
        "attrs": attrs_contract_payload(&subgraph.attrs),
        "node_ids": subgraph.node_ids,
        "defaults": defaults_contract_payload(&subgraph.defaults),
        "subgraphs": subgraph.subgraphs.iter().map(subgraph_contract_payload).collect::<Vec<_>>(),
    })
}

fn defaults_contract_payload(defaults: &DotScopeDefaults) -> Value {
    json!({
        "node": attrs_contract_payload(&defaults.node),
        "edge": attrs_contract_payload(&defaults.edge),
    })
}

#[test]
fn direct_graph_payload_helper_matches_preview_graph_shape() {
    let flow = r#"
    digraph Direct {
      start [shape=Mdiamond];
      done [shape=Msquare];
      start -> done;
    }
    "#;
    let preview = preview_dot_source(flow);
    let graph = preview.graph.as_ref().unwrap();
    let payload = graph_payload(graph);
    assert_eq!(payload["nodes"][0]["id"], "start");
    assert_eq!(payload["edges"][0]["from"], "start");
    assert_eq!(payload["edges"][0]["to"], "done");
}
