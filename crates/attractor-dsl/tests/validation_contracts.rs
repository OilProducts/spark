use std::fs;
use std::path::PathBuf;

use attractor_dsl::{
    apply_graph_transforms, diagnostic_payload, parse_dot, preview_payload_for_graph,
    validate_graph, validate_launch_contract_declarations, validate_or_raise, Diagnostic,
    DiagnosticSeverity,
};
use serde_json::{json, Value};

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".spark/rust-rewrite/current/compat-fixtures")
        .join(name)
}

fn fixture_json(name: &str) -> Value {
    let path = fixture_path(name);
    serde_json::from_str(&fs::read_to_string(&path).expect("fixture readable"))
        .expect("fixture json")
}

fn compact_diagnostic_payload(diagnostic: &Diagnostic) -> Value {
    let mut payload = serde_json::Map::from_iter([
        ("rule".to_string(), json!(diagnostic.rule_id)),
        ("severity".to_string(), json!(diagnostic.severity.as_str())),
        ("message".to_string(), json!(diagnostic.message)),
        ("line".to_string(), json!(diagnostic.line)),
        (
            "node".to_string(),
            diagnostic
                .node_id
                .as_ref()
                .map(|node_id| json!(node_id))
                .unwrap_or(Value::Null),
        ),
    ]);
    if let Some((source, target)) = &diagnostic.edge {
        payload.insert("edge".to_string(), json!([source, target]));
    }
    if let Some(fix) = &diagnostic.fix {
        payload.insert("fix".to_string(), json!(fix));
    }
    Value::Object(payload)
}

fn compact_diagnostics_payload(diagnostics: &[Diagnostic]) -> Value {
    Value::Array(diagnostics.iter().map(compact_diagnostic_payload).collect())
}

fn parse_and_validate(dot: &str) -> Vec<Diagnostic> {
    let graph = parse_dot(dot).expect("valid dot");
    validate_graph(&graph)
}

fn rules(diagnostics: &[Diagnostic]) -> Vec<&str> {
    diagnostics
        .iter()
        .map(|diagnostic| diagnostic.rule_id.as_str())
        .collect()
}

#[test]
fn validate_structure_diagnostics_match_fixture() {
    let fixture = fixture_json("dsl/validate-structure-diagnostics.json");
    let source = fixture["input"]["dot"].as_str().expect("fixture input");
    let graph = parse_dot(source).expect("parse fixture");
    let diagnostics = validate_graph(&graph);

    assert_eq!(
        compact_diagnostics_payload(&diagnostics),
        fixture["observation"]["diagnostics"]
    );
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
            .map(|diagnostic| diagnostic.rule_id.clone())
            .collect::<Vec<_>>(),
        serde_json::from_value::<Vec<String>>(fixture["observation"]["error_rules"].clone())
            .expect("error rules")
    );
}

#[test]
fn validate_launch_context_contracts_preserves_fixture_behavior() {
    let fixture = fixture_json("dsl/validate-launch-context-contracts.json");
    let source = fixture["input"]["dot"].as_str().expect("fixture input");
    let graph = parse_dot(source).expect("parse fixture");
    let diagnostics = validate_graph(&graph);

    assert_eq!(
        compact_diagnostics_payload(&diagnostics),
        fixture["observation"]["diagnostics"]
    );
}

#[test]
fn preview_payload_for_graph_matches_status_fixture_observations() {
    let fixture = fixture_json("dsl/preview-status-and-errors.json");
    let source = fixture["input"]["accepted_dot"]
        .as_str()
        .expect("fixture input");
    let graph = parse_dot(source).expect("parse fixture");
    let transformed = apply_graph_transforms(&graph);

    assert_eq!(
        preview_payload_for_graph(&transformed),
        fixture["observation"]["accepted"]["payload"]
    );
}

#[test]
fn diagnostic_payload_preserves_public_aliases() {
    let diagnostic = Diagnostic {
        rule_id: "edge_target_exists".to_string(),
        severity: DiagnosticSeverity::Error,
        message: "edge target does not exist".to_string(),
        line: 4,
        node_id: None,
        edge: Some(("start".to_string(), "missing".to_string())),
        fix: Some("define node 'missing' or update edge".to_string()),
    };

    assert_eq!(
        diagnostic_payload(&diagnostic),
        json!({
            "rule": "edge_target_exists",
            "rule_id": "edge_target_exists",
            "severity": "error",
            "message": "edge target does not exist",
            "line": 4,
            "node": null,
            "node_id": null,
            "edge": ["start", "missing"],
            "fix": "define node 'missing' or update edge",
        })
    );
}

#[test]
fn validate_or_raise_returns_warnings_but_raises_errors() {
    let warning_graph = parse_dot(
        r#"
digraph WarningsOnly {
  start [shape=Mdiamond]
  work [shape=box, prompt="Work", fidelity="warp"]
  done [shape=Msquare]
  start -> work -> done
}
"#,
    )
    .expect("parse warning graph");
    let diagnostics = validate_or_raise(&warning_graph).expect("warnings only");
    assert_eq!(rules(&diagnostics), ["fidelity_valid"]);

    let error_graph = parse_dot(
        r#"
digraph Errors {
  start [shape=Mdiamond]
  done [shape=Msquare]
  start -> missing
}
"#,
    )
    .expect("parse error graph");
    let error = validate_or_raise(&error_graph).expect_err("validation error");
    assert!(error.to_string().contains("[edge_target_exists] line 5"));
}

#[test]
fn condition_validation_rejects_unsupported_and_unknown_clauses() {
    let diagnostics = parse_and_validate(
        r#"
digraph Conditions {
  start [shape=Mdiamond]
  route [shape=diamond]
  done [shape=Msquare]
  start -> route [condition="context.topic contains docs"]
  route -> done [condition="request.topic && outcome = \">\""]
}
"#,
    );
    let condition_messages = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.rule_id == "condition_syntax")
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        condition_messages,
        [
            "unsupported operator 'contains' in condition clause 'context.topic contains docs'",
            "invalid condition variable 'request.topic'",
        ]
    );
}

#[test]
fn stylesheet_syntax_validation_reports_invalid_rules() {
    let diagnostics = parse_and_validate(
        r#"
digraph Styles {
  graph [model_stylesheet="box { bad: value; reasoning_effort: turbo; }"]
  start [shape=Mdiamond]
  work [shape=box, prompt="Work"]
  done [shape=Msquare]
  start -> work -> done
}
"#,
    );
    let stylesheet_messages = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.rule_id == "stylesheet_syntax")
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        stylesheet_messages,
        [
            "unsupported stylesheet property 'bad', expected one of llm_model, llm_provider, llm_profile, reasoning_effort",
            "reasoning_effort must be one of: low, medium, high, xhigh",
        ]
    );
}

#[test]
fn tool_and_parallel_join_rules_validate_handler_contracts() {
    let diagnostics = parse_and_validate(
        r#"
digraph Handlers {
  start [shape=Mdiamond]
  tool [shape=parallelogram]
  fan [shape=component, join_policy=k_of_n, join_k=3]
  left [shape=box, prompt="Left"]
  right [shape=box, prompt="Right"]
  done [shape=Msquare]
  start -> tool -> fan
  fan -> left
  fan -> right
  left -> done
  right -> done
}
"#,
    );

    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.rule_id == "tool_command_required"
            && diagnostic.node_id.as_deref() == Some("tool")));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.rule_id == "parallel_join_threshold"
            && diagnostic.message == "node 'fan' join_k must be <= outgoing branch count (2)"));
}

#[test]
fn retry_fidelity_type_and_prompt_warnings_match_rule_contracts() {
    let diagnostics = parse_and_validate(
        r#"
digraph Warnings {
  graph [retry_target=missing, default_fidelity="warp"]
  start [shape=Mdiamond]
  node [label="Inherited"]
  work [shape=box, type="alien", goal_gate=true, fidelity="warp"]
  done [shape=Msquare]
  start -> work [fidelity="warp"]
  work -> done
}
"#,
    );

    let warning_rules = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning)
        .map(|diagnostic| diagnostic.rule_id.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        warning_rules,
        [
            "retry_target_exists",
            "goal_gate_has_retry",
            "fidelity_valid",
            "fidelity_valid",
            "fidelity_valid",
            "type_known",
            "prompt_on_llm_nodes",
        ]
    );
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.rule_id == "prompt_on_llm_nodes"
            && diagnostic.node_id.as_deref() == Some("work")));
}

#[test]
fn authored_labels_satisfy_codergen_prompt_rule_but_inherited_labels_do_not() {
    let diagnostics = parse_and_validate(
        r#"
digraph PromptAuthorship {
  start [shape=Mdiamond]
  node [label="Inherited"]
  inherited [shape=box]
  authored [shape=box, label="Authored"]
  done [shape=Msquare]
  start -> inherited -> authored -> done
}
"#,
    );
    let prompt_nodes = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.rule_id == "prompt_on_llm_nodes")
        .map(|diagnostic| diagnostic.node_id.as_deref().unwrap_or(""))
        .collect::<Vec<_>>();

    assert_eq!(prompt_nodes, ["inherited"]);
}

#[test]
fn explicit_launch_contract_validation_reports_malformed_declarations() {
    let source = r#"
digraph ContractIssues {
  graph [spark.launch_inputs="[{\"key\":\"topic\",\"type\":\"weird\",\"label\":\"\"}]"];
  start [shape=Mdiamond];
  write [shape=box, prompt="Write", spark.reads_context="[\"bad key\"]", spark.writes_context="[\"_attractor.runtime.execution_mode\"]"];
  done [shape=Msquare];
  start -> write -> done;
}
"#;
    let graph = parse_dot(source).expect("parse fixture");
    let default_diagnostics = validate_graph(&graph);
    let launch_diagnostics = validate_launch_contract_declarations(&graph);

    assert_eq!(
        rules(&default_diagnostics),
        ["execution_placement_context_non_authoritative"]
    );
    assert_eq!(
        rules(&launch_diagnostics),
        [
            "launch_inputs_valid",
            "launch_inputs_valid",
            "launch_inputs_valid",
            "reads_context_valid",
        ]
    );
}
