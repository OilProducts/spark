//! The decision envelope agents answer with:
//! `{"decision": "<declared id>", "context_updates": {...}, "notes": "..."}`.
//! The prompt appendix, the provider schema enum and the membership check
//! all come from the node's declared routes, read from one place:
//! [`declared_decisions`].

use std::collections::{BTreeMap, BTreeSet};

use attractor_core::{
    attr_string, validate_context_updates_against_contract, ContextMap, ContextWriteContract,
    DotGraph, FailureKind, Outcome, RUNTIME_UNAVAILABLE_ROUTES_KEY,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const DECISION_ENVELOPE_KEYS: &[&str] = &["decision", "context_updates", "notes"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlainTextParseResult {
    pub raw_text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModeledOutcomeParseResult {
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredContractViolation {
    pub response_contract: String,
    pub raw_text: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_contract: Option<ContextWriteContract>,
    /// The node's declared decisions, restated in repair prompts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StructuredTextOutcome {
    /// Text from a node without a response contract.
    PlainText(PlainTextParseResult),
    ModeledOutcome(ModeledOutcomeParseResult),
    ContractViolation(StructuredContractViolation),
}

/// One decision a node declares: its ID, meaning and target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredDecision {
    pub decision: String,
    pub description: String,
    pub target: String,
}

/// The node's declared decisions, in decision order, from the route edges
/// of the graph the flow's route map projects.
pub fn declared_decisions(graph: &DotGraph, node_id: &str) -> Vec<DeclaredDecision> {
    let mut decisions = graph
        .edges
        .iter()
        .filter(|edge| edge.source == node_id)
        .filter_map(|edge| {
            let decision = attr_string(&edge.attrs, "decision");
            (!decision.is_empty()).then(|| DeclaredDecision {
                decision,
                description: attr_string(&edge.attrs, "description"),
                target: edge.target.clone(),
            })
        })
        .collect::<Vec<_>>();
    decisions.sort_by(|left, right| left.decision.cmp(&right.decision));
    decisions
}

pub fn build_decision_envelope_prompt_appendix(
    write_contract: Option<&ContextWriteContract>,
    graph: &DotGraph,
    node_id: &str,
    context: &ContextMap,
) -> String {
    let mut sections = vec![[
        "Structured response contract:".to_string(),
        "- Return ONLY a JSON object: {\"decision\": \"<decision>\", \"context_updates\": {...}, \"notes\": \"...\"}.".to_string(),
        "- \"decision\" is required and must be exactly one of the decision IDs listed below.".to_string(),
        "- \"context_updates\" (a JSON object) and \"notes\" (a string) are optional. Do not emit any other top-level keys.".to_string(),
        "- Put the evidence for your decision, including any blocker, in \"notes\".".to_string(),
        "- Finish long-running work inside this invocation: wait for and poll the commands you start rather than detaching them and answering early.".to_string(),
        build_decision_envelope_context_updates_contract_text(write_contract),
    ]
    .join("\n")];
    sections.extend(build_decisions_text(graph, node_id, context));
    sections.join("\n")
}

/// Lists each declared decision with its target and description. A target
/// that used all its visits stays listed, with that fact stated.
fn build_decisions_text(graph: &DotGraph, node_id: &str, context: &ContextMap) -> Option<String> {
    let unavailable = context
        .get(RUNTIME_UNAVAILABLE_ROUTES_KEY)
        .and_then(Value::as_array)
        .map(|targets| {
            targets
                .iter()
                .filter_map(Value::as_str)
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let decisions = declared_decisions(graph, node_id);
    if decisions.is_empty() {
        return None;
    }
    let mut lines = vec!["Decisions (choose exactly one):".to_string()];
    for DeclaredDecision {
        decision,
        description,
        target,
    } in decisions
    {
        let target_label = graph
            .nodes
            .get(&target)
            .map(|node| attr_string(&node.attrs, "label"))
            .filter(|label| !label.trim().is_empty())
            .unwrap_or_else(|| target.clone());
        let budget = if unavailable.contains(target.as_str()) {
            " (its target has used all its visits: choosing it ends the run with visit_limit_reached)"
        } else {
            ""
        };
        lines.push(format!(
            "- {}: goes to {}{budget}. {description}",
            json!(decision),
            json!(target_label)
        ));
    }
    Some(lines.join("\n"))
}

/// JSON Schema for a node's decision envelope: the decision enum is the
/// node's declared decisions, `context_updates` keys its write contract.
pub fn decision_envelope_json_schema(
    graph: &DotGraph,
    node_id: &str,
    write_contract: &ContextWriteContract,
) -> Value {
    let decisions = declared_decisions(graph, node_id)
        .into_iter()
        .map(|decision| decision.decision)
        .collect::<Vec<_>>();
    let mut properties = serde_json::Map::new();
    properties.insert(
        "decision".into(),
        json!({"type": "string", "enum": decisions}),
    );
    let keys = write_contract
        .allowed_keys
        .iter()
        .map(|key| (key.clone(), json!({})))
        .collect::<serde_json::Map<_, _>>();
    properties.insert(
        "context_updates".into(),
        json!({"type": "object", "properties": keys, "additionalProperties": false}),
    );
    properties.insert("notes".into(), json!({"type": "string"}));
    json!({
        "type": "object",
        "properties": properties,
        "required": ["decision"],
        "additionalProperties": false,
    })
}

pub fn build_decision_envelope_context_updates_contract_text(
    write_contract: Option<&ContextWriteContract>,
) -> String {
    let allowed_keys = write_contract
        .map(|contract| contract.allowed_keys.as_slice())
        .unwrap_or(&[]);
    let mut lines = vec!["Node-specific \"context_updates\" rules:".to_string()];
    if allowed_keys.is_empty() {
        lines.push("- This node must not emit \"context_updates\".".to_string());
        return lines.join("\n");
    }

    lines.extend([
        "- This node may include \"context_updates\" only when needed.".to_string(),
        format!(
            "- Allowed \"context_updates\" keys for this node, and no others: {}.",
            format_allowed_keys(allowed_keys)
        ),
        "- Inside \"context_updates\", emit a flat key/value map using the literal keys above."
            .to_string(),
        "- Use JSON null to clear a previously set context key when this node needs to end or reset live state.".to_string(),
    ]);
    if let Some(dotted_key) = allowed_keys.iter().find(|key| key.contains('.')) {
        lines.extend([
            format!("- Keys with dots stay literal keys, for example \"{dotted_key}\"."),
            format!(
                "- Do not nest objects inside \"context_updates\" for dotted keys. Use {} not {}.",
                flat_context_updates_example(dotted_key),
                nested_context_updates_example(dotted_key),
            ),
        ]);
    }
    lines.join("\n")
}

pub fn decision_envelope_allowed_keys(write_contract: Option<&ContextWriteContract>) -> String {
    format_allowed_keys(
        write_contract
            .map(|contract| contract.allowed_keys.as_slice())
            .unwrap_or(&[]),
    )
}

/// Parses a final answer. Without a response contract the text is plain
/// output. With one, the answer must be a decision envelope whose decision
/// is in `decisions`; anything else is a contract violation, never a
/// default decision.
pub fn coerce_structured_text_outcome(
    text: &str,
    response_contract: &str,
    decisions: &[String],
) -> StructuredTextOutcome {
    let raw_text = text.trim().to_string();
    if !has_response_contract(response_contract) {
        return StructuredTextOutcome::PlainText(PlainTextParseResult { raw_text });
    }
    let violation = |reason: String| {
        StructuredTextOutcome::ContractViolation(StructuredContractViolation {
            response_contract: response_contract.to_string(),
            raw_text: raw_text.clone(),
            reason,
            write_contract: None,
            decisions: decisions.to_vec(),
        })
    };
    let candidate = match extract_decision_envelope(text) {
        Ok(candidate) => candidate,
        Err(reason) => return violation(reason),
    };
    let decision = match candidate.get("decision") {
        Some(Value::String(decision)) => decision.clone(),
        Some(_) => return violation("invalid decision envelope: decision must be a string".into()),
        None => {
            return violation(
                "invalid decision envelope: missing required top-level key \"decision\"".into(),
            )
        }
    };
    if !decisions.contains(&decision) {
        return violation(format!(
            "invalid decision envelope: decision {} is not declared; choose one of {}",
            json!(decision),
            format_allowed_keys(decisions)
        ));
    }
    let context_updates = match context_updates(&candidate) {
        Ok(value) => value,
        Err(reason) => return violation(reason),
    };
    let notes = match candidate.get("notes") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(notes)) => notes.clone(),
        Some(_) => return violation("invalid decision envelope: notes must be a string".into()),
    };
    StructuredTextOutcome::ModeledOutcome(ModeledOutcomeParseResult {
        outcome: Outcome {
            context_updates,
            notes,
            raw_response_text: raw_text.clone(),
            ..Outcome::completed(decision)
        },
    })
}

/// A completed outcome a backend returned directly must still carry a
/// declared decision and keep its write contract.
pub fn validate_write_contract_violation(
    outcome: &Outcome,
    write_contract: Option<&ContextWriteContract>,
    response_contract: &str,
    raw_text: &str,
) -> Option<StructuredContractViolation> {
    if !has_response_contract(response_contract) {
        return None;
    }
    let write_contract = write_contract?;
    let violation =
        validate_context_updates_against_contract(&outcome.context_updates, write_contract)?;
    Some(StructuredContractViolation {
        response_contract: response_contract.to_string(),
        raw_text: raw_text.trim().to_string(),
        reason: violation.format_reason(None),
        write_contract: Some(write_contract.clone()),
        decisions: Vec::new(),
    })
}

pub fn build_contract_repair_prompt(violation: &StructuredContractViolation) -> String {
    let mut lines = vec![
        format!(
            "Your previous final answer violated the {} response contract.",
            violation.response_contract
        ),
        format!("Validation error: {}", violation.reason),
        String::new(),
        "Re-emit only a corrected final answer for the same judgment.".to_string(),
        "Do not do new repository work.".to_string(),
        "Do not run commands.".to_string(),
        "Do not change the substantive decision or context updates except as required to satisfy the response contract.".to_string(),
    ];
    lines.extend(decision_reminder(violation));
    let allowed_keys = violation
        .write_contract
        .as_ref()
        .map(|contract| contract.allowed_keys.as_slice())
        .unwrap_or(&[]);
    if allowed_keys.is_empty() {
        lines.push("Re-emit the same decision with no \"context_updates\".".to_string());
    } else {
        lines.push(format!(
            "Re-emit the same decision using only these \"context_updates\" keys when needed: {}.",
            format_allowed_keys(allowed_keys),
        ));
    }
    lines.extend([
        build_decision_envelope_context_updates_contract_text(violation.write_contract.as_ref()),
        String::new(),
        "Previous invalid final answer:".to_string(),
        violation.raw_text.clone(),
    ]);
    lines.join("\n")
}

/// Repair for a session that still holds the task and the invalid answer.
pub fn build_in_session_contract_repair_prompt(violation: &StructuredContractViolation) -> String {
    let mut lines = vec![
        format!(
            "Your final answer violated the {} response contract.",
            violation.response_contract
        ),
        format!("Validation error: {}", violation.reason),
        "Re-emit only a corrected final answer for the same judgment. Do not do new repository work."
            .to_string(),
    ];
    lines.extend(decision_reminder(violation));
    lines.push(build_decision_envelope_context_updates_contract_text(
        violation.write_contract.as_ref(),
    ));
    lines.join("\n")
}

fn decision_reminder(violation: &StructuredContractViolation) -> Option<String> {
    (!violation.decisions.is_empty()).then(|| {
        format!(
            "Answer with a JSON object whose \"decision\" is one of {}.",
            format_allowed_keys(&violation.decisions)
        )
    })
}

/// Exhausted repair: a nonretryable contract error with its evidence.
pub fn contract_failure_outcome(violation: &StructuredContractViolation) -> Outcome {
    Outcome {
        notes: violation.raw_text.clone(),
        raw_response_text: violation.raw_text.clone(),
        ..Outcome::error(FailureKind::Contract, violation.reason.clone(), Some(false))
    }
}

/// The single JSON object of a final answer, optionally inside one fenced
/// block, with only decision-envelope keys.
pub fn extract_decision_envelope(text: &str) -> Result<BTreeMap<String, Value>, String> {
    let stripped = text.trim();
    if stripped.is_empty() {
        return Err("invalid decision envelope: empty response".to_string());
    }
    let mut candidates = vec![stripped.to_string()];
    if stripped.starts_with("```") && stripped.ends_with("```") {
        let lines = stripped.lines().collect::<Vec<_>>();
        if lines.len() >= 3 {
            let inner = lines[1..lines.len() - 1].join("\n").trim().to_string();
            if !inner.is_empty() && !candidates.contains(&inner) {
                candidates.push(inner);
            }
        }
    }
    let mut error = String::new();
    for candidate in candidates {
        let payload: Value = match serde_json::from_str(&candidate) {
            Ok(value) => value,
            Err(source) => {
                error = format!("invalid decision envelope: invalid JSON: {source}");
                continue;
            }
        };
        let Some(object) = payload.as_object() else {
            error = "invalid decision envelope: expected a JSON object".to_string();
            continue;
        };
        let unexpected = object
            .keys()
            .filter(|key| !DECISION_ENVELOPE_KEYS.contains(&key.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !unexpected.is_empty() {
            return Err(format!(
                "invalid decision envelope: unexpected top-level keys {}",
                unexpected.join(", ")
            ));
        }
        return Ok(object
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect());
    }
    Err(error)
}

fn has_response_contract(response_contract: &str) -> bool {
    !response_contract.trim().is_empty()
}

fn context_updates(payload: &BTreeMap<String, Value>) -> Result<ContextMap, String> {
    match payload.get("context_updates") {
        None | Some(Value::Null) => Ok(ContextMap::new()),
        // Session handles are runtime-owned; an agent never writes them.
        Some(Value::Object(items)) => Ok(items
            .iter()
            .filter(|(key, _)| key.as_str() != attractor_core::RUNTIME_SESSIONS_KEY)
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()),
        _ => Err("invalid decision envelope: context_updates must be an object".to_string()),
    }
}

fn format_allowed_keys(allowed_keys: &[String]) -> String {
    if allowed_keys.is_empty() {
        "<none>".to_string()
    } else {
        allowed_keys
            .iter()
            .map(|key| format!("\"{key}\""))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn flat_context_updates_example(key: &str) -> String {
    json!({"context_updates": {key: "..."}}).to_string()
}

fn nested_context_updates_example(key: &str) -> String {
    let mut nested = json!("...");
    for segment in key.split('.').rev() {
        nested = json!({segment: nested});
    }
    json!({"context_updates": nested}).to_string()
}
