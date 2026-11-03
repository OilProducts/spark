use attractor_core::{parse_context_write_contract, DotGraph, OutcomeStatus};
use serde_json::{json, Value};
use spark_agent_adapter::{
    coerce_structured_text_outcome, decision_envelope_json_schema, declared_decisions,
    StructuredTextOutcome,
};

const CONTRACT: &str = "decision_envelope";

fn agent(prompt: &str, routes: Value) -> Value {
    json!({"kind":"agent_task", "config":{"kind":"agent_task", "prompt":prompt},
        "contracts":{"response":CONTRACT}, "routes":routes})
}

fn graph() -> DotGraph {
    let source = json!({"schema_version":"2", "id":"flow",
        "nodes":{
            "start":{"kind":"start", "routes":{"next":{"to":"review", "description":"Enter review."}}},
            "review":agent("review", json!({
                "approve":{"to":"done", "description":"The change is ready."},
                "rework":{"to":"fix", "description":"The change needs fixes.", "label":"Fix Evidence"},
                "blocked":{"to":"failed", "description":"Review cannot proceed."}})),
            "fix":agent("fix", json!({
                "fixed":{"to":"review", "description":"The fixes are in."},
                "blocked":{"to":"failed", "description":"Fixing cannot proceed."}})),
            "done":{"kind":"exit", "config":{"kind":"exit", "result":{"name":"completed", "outcome":"success"}}},
            "failed":{"kind":"exit", "config":{"kind":"exit", "result":{"name":"blocked", "outcome":"failure"}}}}
    });
    attractor_dsl::validated_flow(serde_json::from_value(source).unwrap())
        .unwrap()
        .to_runtime_dot_graph()
}

fn schema(graph: &DotGraph, node_id: &str) -> Value {
    let writes = (node_id == "review").then_some(r#"["context.review.notes"]"#);
    decision_envelope_json_schema(graph, node_id, &parse_context_write_contract(writes))
}

fn decisions(graph: &DotGraph, node_id: &str) -> Vec<String> {
    declared_decisions(graph, node_id)
        .into_iter()
        .map(|decision| decision.decision)
        .collect()
}

fn coerce(text: &str) -> StructuredTextOutcome {
    coerce_structured_text_outcome(text, CONTRACT, &decisions(&graph(), "review"))
}

fn violation_reason(text: &str) -> String {
    match coerce(text) {
        StructuredTextOutcome::ContractViolation(violation) => violation.reason,
        other => panic!("{text} must be a contract violation, got {other:?}"),
    }
}

#[test]
fn declared_decisions_come_from_the_node_routes_with_descriptions_and_targets() {
    let declared = declared_decisions(&graph(), "review")
        .into_iter()
        .map(|d| (d.decision, d.description, d.target))
        .collect::<Vec<_>>();
    let expected = [
        ("approve", "The change is ready.", "done"),
        ("blocked", "Review cannot proceed.", "failed"),
        ("rework", "The change needs fixes.", "fix"),
    ]
    .map(|(d, desc, t)| (d.to_string(), desc.to_string(), t.to_string()));
    assert_eq!(declared, expected);
}

#[test]
fn decision_envelope_schema_enumerates_the_declared_decisions_and_write_contract() {
    let graph = graph();
    let fix_schema = schema(&graph, "fix");
    let schema = schema(&graph, "review");
    assert_eq!(
        schema,
        json!({
            "type": "object",
            "properties": {
                "decision": {"type": "string", "enum": ["approve", "blocked", "rework"]},
                "context_updates": {
                    "type": "object",
                    "properties": {"context.review.notes": {}},
                    "additionalProperties": false,
                },
                "notes": {"type": "string"},
            },
            "required": ["decision"],
            "additionalProperties": false,
        })
    );
    // The provider enum and the parser's membership check are one set.
    assert_eq!(
        schema["properties"]["decision"]["enum"],
        json!(decisions(&graph, "review"))
    );
    assert_eq!(
        fix_schema["properties"]["decision"]["enum"],
        json!(["blocked", "fixed"])
    );

    let validator = jsonschema::validator_for(&schema).expect("valid schema");
    for valid in [
        r#"{"decision":"approve"}"#,
        r#"{"decision":"rework","notes":"tests fail","context_updates":{"context.review.notes":null}}"#,
    ] {
        assert!(
            validator.is_valid(&serde_json::from_str(valid).unwrap()),
            "{valid}"
        );
        assert!(
            matches!(coerce(valid), StructuredTextOutcome::ModeledOutcome(_)),
            "{valid}"
        );
    }
    for invalid in [
        r#"{"decision":"aprove"}"#,
        r#"{"notes":"no decision"}"#,
        r#"{"decision":"approve","context_updates":{"context.other":1}}"#,
        r#"{"decision":"approve","outcome":"success"}"#,
        r#"{"decision":"approve","preferred_label":"Approve"}"#,
    ] {
        assert!(
            !validator.is_valid(&serde_json::from_str(invalid).unwrap()),
            "{invalid}"
        );
    }
}

#[test]
fn a_node_without_writes_gets_a_schema_without_context_update_keys() {
    let schema = schema(&graph(), "fix");
    let validator = jsonschema::validator_for(&schema).expect("valid schema");
    assert!(validator.is_valid(&json!({"decision": "fixed"})));
    assert!(!validator.is_valid(&json!({"decision": "fixed", "context_updates": {"x": 1}})));
}

#[test]
fn a_valid_decision_keeps_its_context_updates_and_notes() {
    let StructuredTextOutcome::ModeledOutcome(parsed) = coerce(
        "```json\n{\"decision\":\"rework\",\"context_updates\":{\"context.review.notes\":\"fix tests\"},\"notes\":\"two failures\"}\n```",
    ) else {
        panic!("fenced envelope must parse");
    };
    let outcome = parsed.outcome;
    assert_eq!(outcome.status, OutcomeStatus::Completed);
    assert_eq!(outcome.decision, "rework");
    assert_eq!(outcome.notes, "two failures");
    assert_eq!(
        outcome.context_updates["context.review.notes"],
        json!("fix tests")
    );
    assert!(outcome.failure_reason.is_empty());
}

#[test]
fn invalid_answers_are_contract_violations_never_a_default_decision() {
    assert!(violation_reason(r#"{"notes":"done"}"#)
        .contains("missing required top-level key \"decision\""));
    assert!(violation_reason(r#"{"decision":"ship"}"#).contains("\"ship\" is not declared"));
    assert!(violation_reason(r#"{"decision":true}"#).contains("decision must be a string"));
    assert!(violation_reason("All done, looks good.").contains("invalid JSON"));
    assert!(violation_reason(r#"{"decision":"approve""#).contains("invalid JSON"));
    assert!(violation_reason("").contains("empty response"));
    assert!(violation_reason(r#"["approve"]"#).contains("expected a JSON object"));
    assert!(
        violation_reason(r#"{"decision":"approve","context_updates":["x"]}"#)
            .contains("context_updates must be an object")
    );
    assert!(
        violation_reason(r#"{"decision":"approve","notes":1}"#).contains("notes must be a string")
    );
    for legacy in [
        "outcome",
        "preferred_label",
        "suggested_next_ids",
        "failure_reason",
        "retryable",
    ] {
        let text = json!({"decision": "approve", legacy: "x"}).to_string();
        assert!(
            violation_reason(&text).contains(&format!("unexpected top-level keys {legacy}")),
            "{legacy}"
        );
    }
    // The old status envelope alone is rejected too.
    assert!(matches!(
        coerce(r#"{"outcome":"success","preferred_label":"Approve"}"#),
        StructuredTextOutcome::ContractViolation(_)
    ));
    // The violation restates the declared decisions for repair.
    let StructuredTextOutcome::ContractViolation(violation) = coerce("ok") else {
        unreachable!()
    };
    assert_eq!(violation.decisions, ["approve", "blocked", "rework"]);
}

#[test]
fn text_without_a_response_contract_is_plain_output() {
    assert!(matches!(
        coerce_structured_text_outcome("hello", "", &[]),
        StructuredTextOutcome::PlainText(_)
    ));
}
