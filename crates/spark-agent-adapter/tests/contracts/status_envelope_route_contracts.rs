use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use attractor_core::{DotAttribute, DotValue, DotValueType, FailureKind, OutcomeStatus};
use serde_json::{json, Value};
use spark_agent_adapter::codergen::DECISION_ENVELOPE_SCHEMA_METADATA_KEY;
use spark_agent_adapter::{
    CodergenBackend, CodergenBackendOutput, CodergenBackendRequest, CodergenError,
    CodergenExecution, CodergenHandler, CodergenRequest,
};

/// Answers with the queued texts in order and records each request.
struct Scripted {
    answers: VecDeque<String>,
    requests: Arc<Mutex<Vec<CodergenBackendRequest>>>,
}

impl CodergenBackend for Scripted {
    fn run(
        &mut self,
        request: CodergenBackendRequest,
    ) -> Result<CodergenBackendOutput, CodergenError> {
        self.requests.lock().unwrap().push(request);
        let answer = self.answers.pop_front().expect("unexpected backend call");
        Ok(CodergenBackendOutput::text(answer))
    }
}

fn set_attr(graph: &mut attractor_core::DotGraph, key: &str, value: &str) {
    graph.nodes.get_mut("work").unwrap().attrs.insert(
        key.to_string(),
        DotAttribute {
            key: key.to_string(),
            value: DotValue::String(value.to_string()),
            value_type: DotValueType::String,
            line: 0,
        },
    );
}

fn run(
    response_contract: Option<&str>,
    context: Value,
    answers: &[&str],
) -> (CodergenExecution, Vec<CodergenBackendRequest>) {
    let mut graph = attractor_dsl::apply_graph_transforms(&flow().to_runtime_dot_graph());
    // The runtime maps a node's `contracts` onto these attributes.
    if let Some(contract) = response_contract {
        set_attr(&mut graph, "codergen.response_contract", contract);
    }
    set_attr(
        &mut graph,
        "spark.writes_context",
        r#"["context.work.summary"]"#,
    );
    let node_id = "work";
    let request: CodergenRequest = serde_json::from_value(
        json!({"node_id": node_id, "node": graph.nodes[node_id], "graph": graph, "context": context}),
    )
    .unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let execution = CodergenHandler::with_backend(Scripted {
        answers: answers.iter().map(|a| a.to_string()).collect(),
        requests: requests.clone(),
    })
    .execute(request)
    .unwrap();
    let requests = requests.lock().unwrap().clone();
    (execution, requests)
}

fn envelope_prompt(context: Value) -> (String, Value) {
    let (_, requests) = run(
        Some("decision_envelope"),
        context,
        &[r#"{"decision":"done"}"#],
    );
    let request = &requests[0];
    (
        request.prompt.clone(),
        request.metadata[DECISION_ENVELOPE_SCHEMA_METADATA_KEY]["properties"]["decision"]["enum"]
            .clone(),
    )
}

fn agent(label: &str, prompt: &str, routes: Value) -> Value {
    json!({"kind":"agent_task", "label":label, "config":{"kind":"agent_task", "prompt":prompt},
        "contracts":{"response":"decision_envelope"}, "routes":routes})
}

fn flow() -> attractor_core::FlowDefinition {
    let source = json!({"schema_version":"2", "id":"flow",
        "nodes":{
            "start":{"kind":"start", "routes":{"next":{"to":"work", "description":"Begin."}}},
            "work":agent("Work", "Do the work.", json!({
                "done":{"to":"end", "description":"The work is complete."},
                "changes_needed":{"to":"fix", "description":"The work needs fixes."},
                "escalate":{"to":"review", "description":"A human must look.", "label":"Escalate"},
                "blocked":{"to":"failed", "description":"The work cannot proceed."}})),
            "fix":agent("Fix It", "fix", json!({
                "fixed":{"to":"work", "description":"Fixed."},
                "blocked":{"to":"failed", "description":"Cannot fix."}})),
            "review":agent("Human Review", "review", json!({
                "approved":{"to":"end", "description":"Approved."},
                "blocked":{"to":"failed", "description":"Cannot review."}})),
            "end":{"kind":"exit", "label":"Done", "config":{"kind":"exit", "result":{"name":"completed", "outcome":"success"}}},
            "failed":{"kind":"exit", "label":"Failed", "config":{"kind":"exit", "result":{"name":"blocked", "outcome":"failure"}}}
        }
    });
    attractor_dsl::validated_flow(serde_json::from_value(source).unwrap()).unwrap()
}

const DECISION_LINES: [&str; 4] = [
    "- \"blocked\": goes to \"Failed\". The work cannot proceed.",
    "- \"changes_needed\": goes to \"Fix It\". The work needs fixes.",
    "- \"done\": goes to \"Done\". The work is complete.",
    "- \"escalate\": goes to \"Human Review\". A human must look.",
];

#[test]
fn appendix_and_provider_schema_enumerate_the_same_declared_decisions() {
    let (prompt, enum_values) = envelope_prompt(json!({}));
    let expected = std::iter::once("Decisions (choose exactly one):")
        .chain(DECISION_LINES)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(prompt.contains(&expected), "{prompt}");
    assert!(prompt.contains("Structured response contract:"), "{prompt}");
    assert!(prompt.contains("\"context.work.summary\""), "{prompt}");
    assert_eq!(
        enum_values,
        json!(["blocked", "changes_needed", "done", "escalate"])
    );
    for legacy in ["preferred_label", "suggested_next_ids", "\"outcome\""] {
        assert!(!prompt.contains(legacy), "{legacy}\n{prompt}");
    }
}

#[test]
fn an_exhausted_target_stays_a_choice_with_its_budget_stated() {
    let (prompt, enum_values) =
        envelope_prompt(json!({"_attractor.runtime.unavailable_routes": ["review"]}));
    assert!(
        prompt.contains(
            "- \"escalate\": goes to \"Human Review\" (its target has used all its visits: choosing it ends the run with visit_limit_reached). A human must look."
        ),
        "{prompt}"
    );
    for line in &DECISION_LINES[..3] {
        assert!(prompt.contains(line), "{line}\n{prompt}");
    }
    assert_eq!(
        enum_values,
        json!(["blocked", "changes_needed", "done", "escalate"])
    );
    assert!(
        !prompt.contains("_attractor.runtime.unavailable_routes"),
        "{prompt}"
    );
}

#[test]
fn non_envelope_node_prompt_is_unchanged_by_routes() {
    let (_, requests) = run(None, json!({}), &["plain answer"]);
    assert_eq!(requests[0].prompt, "Do the work.");
    assert!(!requests[0]
        .metadata
        .contains_key(DECISION_ENVELOPE_SCHEMA_METADATA_KEY));
}

#[test]
fn a_valid_decision_keeps_allowed_context_writes() {
    let (execution, requests) = run(
        Some("decision_envelope"),
        json!({}),
        &[
            r#"{"decision":"changes_needed","context_updates":{"context.work.summary":"x"},"notes":"n"}"#,
        ],
    );
    assert_eq!(requests.len(), 1);
    let outcome = execution.outcome;
    assert_eq!(outcome.status, OutcomeStatus::Completed);
    assert_eq!(outcome.decision, "changes_needed");
    assert_eq!(outcome.notes, "n");
    assert_eq!(outcome.context_updates["context.work.summary"], json!("x"));
    assert_eq!(execution.repair_attempts, 0);
}

#[test]
fn a_repaired_answer_is_accepted_after_one_bounded_repair() {
    let (execution, requests) = run(
        Some("decision_envelope"),
        json!({}),
        &[r#"{"outcome":"success"}"#, r#"{"decision":"done"}"#],
    );
    assert_eq!(execution.outcome.decision, "done");
    assert_eq!(execution.repair_attempts, 1);
    assert_eq!(execution.contract_violations.len(), 1);
    let repair = &requests[1].prompt;
    assert!(
        repair.contains("unexpected top-level keys outcome"),
        "{repair}"
    );
    assert!(
        repair.contains(r#""decision" is one of "blocked", "changes_needed", "done", "escalate""#),
        "{repair}"
    );
}

#[test]
fn exhausted_repair_is_a_contract_error_never_a_default_decision() {
    for (label, first, second) in [
        ("plain text", "Looks good to me.", "Still looks good."),
        (
            "unknown decision",
            r#"{"decision":"ship"}"#,
            r#"{"decision":"ship"}"#,
        ),
        (
            "missing decision",
            r#"{"notes":"done"}"#,
            r#"{"notes":"done"}"#,
        ),
        (
            "unauthorized write",
            r#"{"decision":"done","context_updates":{"context.other":1}}"#,
            r#"{"decision":"done","context_updates":{"context.other":1}}"#,
        ),
    ] {
        let (execution, requests) = run(Some("decision_envelope"), json!({}), &[first, second]);
        assert_eq!(requests.len(), 2, "{label}");
        let outcome = &execution.outcome;
        assert_eq!(outcome.status, OutcomeStatus::Error, "{label}");
        assert_eq!(outcome.failure_kind, Some(FailureKind::Contract), "{label}");
        assert_eq!(outcome.retryable, Some(false), "{label}");
        assert!(outcome.decision.is_empty(), "{label}");
        assert!(
            !outcome.context_updates.contains_key("context.other"),
            "{label}"
        );
        assert_eq!(execution.repair_attempts, 1, "{label}");
        assert_eq!(execution.contract_violations.len(), 2, "{label}");
    }
}

#[test]
fn agent_written_runtime_sessions_are_contract_violations_not_dropped_keys() {
    let forged = r#"{"decision":"done","context_updates":{"_attractor.runtime.sessions":{"named":{"forged":true}}}}"#;
    // One bounded repair: the corrected answer is accepted without the forged key.
    let (execution, requests) = run(
        Some("decision_envelope"),
        json!({}),
        &[forged, r#"{"decision":"done"}"#],
    );
    assert_eq!(requests.len(), 2);
    assert_eq!(execution.outcome.status, OutcomeStatus::Completed);
    assert!(!execution
        .outcome
        .context_updates
        .contains_key("_attractor.runtime.sessions"));
    assert_eq!(execution.contract_violations.len(), 1);
    assert!(
        execution.contract_violations[0]
            .reason
            .contains("_attractor.runtime.sessions"),
        "{}",
        execution.contract_violations[0].reason
    );
    // Exhausted repair is a contract error, never a completed decision.
    let (execution, requests) = run(Some("decision_envelope"), json!({}), &[forged, forged]);
    assert_eq!(requests.len(), 2);
    assert_eq!(execution.outcome.status, OutcomeStatus::Error);
    assert_eq!(execution.outcome.failure_kind, Some(FailureKind::Contract));
    assert!(execution.outcome.decision.is_empty());
    assert!(!execution
        .outcome
        .context_updates
        .contains_key("_attractor.runtime.sessions"));
}
