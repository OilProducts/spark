use attractor_core::{parse_context_write_contract, DotGraph};
use serde_json::{json, Value};
use spark_agent_adapter::{
    coerce_structured_text_outcome, status_envelope_json_schema, StructuredTextOutcome,
};

fn graph() -> DotGraph {
    let source = json!({"schema_version":"1", "id":"flow",
        "nodes":{
            "start":{"kind":"start"},
            "review":{"kind":"agent_task", "config":{"kind":"agent_task", "prompt":"review"}},
            "lone":{"kind":"agent_task", "config":{"kind":"agent_task", "prompt":"lone"}},
            "fix":{"kind":"agent_task", "config":{"kind":"agent_task", "prompt":"fix"}},
            "end":{"kind":"exit"}},
        "edges":[
            {"from":"start","to":"review"},
            {"from":"review","to":"end","label":"Approve"},
            {"from":"review","to":"fix","label":"Fix Evidence","condition":"outcome=fail && preferred_label=Rework"},
            {"from":"fix","to":"lone"},
            {"from":"lone","to":"end"}]
    });
    attractor_dsl::parse_flow_definition(&source.to_string())
        .unwrap()
        .to_runtime_dot_graph()
}

fn schema(graph: &DotGraph, node_id: &str) -> Value {
    let writes = (node_id == "review").then_some(r#"["context.review.notes"]"#);
    let contract = parse_context_write_contract(writes);
    status_envelope_json_schema(graph, node_id, &contract)
}

fn parses(text: &str) -> bool {
    matches!(
        coerce_structured_text_outcome(text, "status_envelope"),
        StructuredTextOutcome::ModeledOutcome(_)
    )
}

#[test]
fn status_envelope_schema_comes_from_the_node_edges_and_write_contract() {
    let graph = graph();
    let schema = schema(&graph, "review");
    assert_eq!(
        schema,
        json!({
            "type": "object",
            "properties": {
                "outcome": {"type": "string", "enum": ["success", "fail", "partial_success", "retry"]},
                "preferred_label": {"type": "string", "enum": ["Approve", "Fix Evidence", "Rework"]},
                "suggested_next_ids": {"type": "array", "items": {"type": "string", "enum": ["end", "fix"]}},
                "context_updates": {
                    "type": "object",
                    "properties": {"context.review.notes": {}},
                    "additionalProperties": false,
                },
                "notes": {"type": "string"},
                "failure_reason": {"type": "string"},
                "retryable": {"type": "boolean"},
            },
            "required": ["outcome"],
            "additionalProperties": false,
        })
    );
    let validator = jsonschema::validator_for(&schema).expect("valid schema");

    for valid in [
        r#"{"outcome":"success","preferred_label":"Approve","suggested_next_ids":["end"],"notes":"ok","retryable":false}"#,
        r#"{"outcome":"fail","preferred_label":"Rework","failure_reason":"no","context_updates":{"context.review.notes":null}}"#,
    ] {
        assert!(
            validator.is_valid(&serde_json::from_str(valid).unwrap()),
            "{valid}"
        );
        assert!(parses(valid), "{valid}");
    }
    // The parser takes any label string; the schema keeps the model to the
    // labels routing knows.
    for invalid in [
        r#"{"outcome":"success","preferred_label":"Aprove"}"#,
        r#"{"outcome":"success","suggested_next_ids":["elsewhere"]}"#,
        r#"{"outcome":"success","context_updates":{"context.other":1}}"#,
        r#"{"outcome":"done"}"#,
        r#"{"outcome":"success","extra":1}"#,
        r#"{"notes":"no outcome"}"#,
    ] {
        assert!(
            !validator.is_valid(&serde_json::from_str(invalid).unwrap()),
            "{invalid}"
        );
    }
    assert!(parses(
        r#"{"outcome":"success","preferred_label":"Aprove"}"#
    ));
}

#[test]
fn status_envelope_schema_omits_routing_and_context_updates_a_node_lacks() {
    let graph = graph();
    let schema = schema(&graph, "lone");
    let properties = schema["properties"].as_object().unwrap();
    assert_eq!(
        properties.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "failure_reason",
            "notes",
            "outcome",
            "retryable",
            "suggested_next_ids"
        ]
    );
    assert_eq!(
        properties["suggested_next_ids"]["items"]["enum"],
        json!(["end"])
    );
    let validator = jsonschema::validator_for(&schema).expect("valid schema");
    assert!(validator.is_valid(&json!({"outcome": "success"})));
    assert!(parses(r#"{"outcome":"success"}"#));
    assert!(!validator.is_valid(&json!({"outcome": "success", "preferred_label": "Approve"})));
    assert!(!validator.is_valid(&json!({"outcome": "success", "context_updates": {"x": 1}})));

    let mut exit_only = graph.clone();
    exit_only.edges.retain(|edge| edge.source != "lone");
    let properties = schema_props(&exit_only, "lone");
    assert!(!properties.contains_key("suggested_next_ids"));
}

fn schema_props(graph: &DotGraph, node_id: &str) -> serde_json::Map<String, Value> {
    schema(graph, node_id)["properties"]
        .as_object()
        .unwrap()
        .clone()
}
